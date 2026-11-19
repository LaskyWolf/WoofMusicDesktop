//! Swift `DatabaseService`. GRDB's `DatabasePool` actor becomes one connection
//! behind a `Mutex`; methods are synchronous, so callers run them off the UI thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use rusqlite::{
    Connection, OptionalExtension, Params, ToSql, Transaction, params, params_from_iter,
};

use super::entities::album_entity::AlbumEntity;
use super::entities::artist_entity::ArtistEntity;
use super::entities::event::Event;
use super::entities::playlist_entity::PlaylistEntity;
use super::entities::podcast_entity::PodcastEntity;
use super::entities::song_entity::SongEntity;
use super::entities::speed_dial_entry::SpeedDialEntry;
use super::{DbDate, Record, database_migrations as m};
use crate::networking::models::yt_item::SongItem;

type Result<T> = rusqlite::Result<T>;
type Migration = fn(&Connection) -> Result<()>;

/// Swift `registerMigrations`, in GRDB `DatabaseMigrator` order.
pub const MIGRATIONS: [(&str, Migration); 11] = [
    ("v1", m::v1),
    ("v2", m::v2),
    ("v3", m::v3),
    ("v4", m::v4),
    ("v5", m::v5),
    ("v6", m::v6),
    ("v7", m::v7),
    ("v8", m::v8),
    ("v9", m::v9),
    ("v10", m::v10),
    ("v11", m::v11),
];

/// GRDB `DatabaseMigrator.migrate`: runs each missing migration in its own
/// transaction and records its identifier in `grdb_migrations`, as GRDB does.
pub fn migrate(db: &mut Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS grdb_migrations (identifier TEXT NOT NULL PRIMARY KEY)",
    )?;
    for (identifier, migration) in MIGRATIONS {
        let tx = db.transaction()?;
        let applied: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM grdb_migrations WHERE identifier = ?)",
            [identifier],
            |row| row.get(0),
        )?;
        if !applied {
            migration(&tx)?;
            tx.execute(
                "INSERT INTO grdb_migrations (identifier) VALUES (?)",
                [identifier],
            )?;
        }
        tx.commit()?;
    }
    Ok(())
}

/// GRDB `Database.ConflictResolution`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictResolution {
    Rollback,
    Abort,
    Fail,
    Ignore,
    Replace,
}

impl ConflictResolution {
    fn sql(self) -> &'static str {
        match self {
            Self::Rollback => "INSERT OR ROLLBACK",
            Self::Abort => "INSERT",
            Self::Fail => "INSERT OR FAIL",
            Self::Ignore => "INSERT OR IGNORE",
            Self::Replace => "INSERT OR REPLACE",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LibrarySongSort {
    #[default]
    RecentlyAdded,
    Title,
    Artist,
    MostPlayed,
}

impl LibrarySongSort {
    pub const ALL: [Self; 4] = [
        Self::RecentlyAdded,
        Self::Title,
        Self::Artist,
        Self::MostPlayed,
    ];

    /// Swift `rawValue`, also its `id` and `displayName`.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::RecentlyAdded => "Recently Added",
            Self::Title => "Title",
            Self::Artist => "Artist",
            Self::MostPlayed => "Most Played",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HistoryEntry {
    pub event: Event,
    pub song: Option<SongEntity>,
}

pub struct DatabaseService {
    db: Mutex<Connection>,
    uses_temporary_storage: AtomicBool,
}

impl DatabaseService {
    /// Opens and migrates the database at `path`. When that fails, logs the error
    /// and falls back to a temporary database (`uses_temporary_storage()`), like Swift.
    pub fn open(path: &Path) -> Self {
        let (db, temporary) = Self::make_pool(path);
        Self {
            db: Mutex::new(db),
            uses_temporary_storage: AtomicBool::new(temporary),
        }
    }

    /// An empty migrated database in memory (tests, fallback).
    pub fn open_in_memory() -> Self {
        Self {
            db: Mutex::new(Self::memory_pool()),
            uses_temporary_storage: AtomicBool::new(false),
        }
    }

    /// Swift `ProfileStorage.databaseURL`: `trop.sqlite` in the active
    /// profile's folder (`$XDG_DATA_HOME/WoofMusic` or `%APPDATA%\WoofMusic`
    /// for the first profile). Downloads sit next to it.
    pub fn default_path() -> Option<PathBuf> {
        Some(swiftytube::auth::profile_storage::dir()?.join("trop.sqlite"))
    }

    /// The disk database failed to open, so nothing is saved until the next launch.
    pub fn uses_temporary_storage(&self) -> bool {
        self.uses_temporary_storage.load(Ordering::Relaxed)
    }

    /// Swift `reopenForActiveProfile`: switches to another database file.
    pub fn reopen(&self, path: &Path) {
        let (db, temporary) = Self::make_pool(path);
        *self.conn() = db;
        self.uses_temporary_storage
            .store(temporary, Ordering::Relaxed);
    }

    fn make_pool(path: &Path) -> (Connection, bool) {
        let open = || -> std::result::Result<Connection, Box<dyn std::error::Error>> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut db = Connection::open(path)?;
            // GRDB's DatabasePool runs in WAL mode.
            db.pragma_update(None, "journal_mode", "WAL")?;
            migrate(&mut db)?;
            Ok(db)
        };
        match open() {
            Ok(db) => (db, false),
            Err(error) => {
                log::error!(
                    "Database setup failed: {error}. Falling back to a temporary database."
                );
                // ponytail: Swift tries a temp file before memory; both lose data at exit.
                (Self::memory_pool(), true)
            }
        }
    }

    fn memory_pool() -> Connection {
        let mut db = Connection::open_in_memory().expect("Unable to create any database pool");
        migrate(&mut db).expect("migrations failed on an empty in-memory database");
        db
    }

    /// The connection. A panic inside an earlier call leaves it usable, so poison is ignored.
    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

// MARK: Generic CRUD

impl DatabaseService {
    /// Returns the new rowid (the id of `event` and `playlist_song_map` rows).
    /// Swift returns the record unchanged.
    pub fn insert<T: Record>(&self, record: &T) -> Result<i64> {
        self.insert_on_conflict(record, ConflictResolution::Abort)
    }

    pub fn insert_on_conflict<T: Record>(
        &self,
        record: &T,
        on_conflict: ConflictResolution,
    ) -> Result<i64> {
        let db = self.conn();
        Self::insert_in(&db, record, on_conflict)?;
        Ok(db.last_insert_rowid())
    }

    /// Fails with `QueryReturnedNoRows` when no row has the record's key (GRDB `recordNotFound`).
    pub fn update<T: Record>(&self, record: &T) -> Result<()> {
        match Self::update_in(&self.conn(), record)? {
            0 => Err(rusqlite::Error::QueryReturnedNoRows),
            _ => Ok(()),
        }
    }

    /// GRDB `save`: updates the row with the same key, or inserts it.
    pub fn save<T: Record>(&self, record: &T) -> Result<()> {
        self.write(|tx| Self::save_in(tx, record))
    }

    /// `save` inside a `write` block.
    pub fn save_in<T: Record>(db: &Connection, record: &T) -> Result<()> {
        if Self::update_in(db, record)? == 0 {
            Self::insert_in(db, record, ConflictResolution::Abort)?;
        }
        Ok(())
    }

    pub fn insert_or_replace<T: Record>(&self, record: &T) -> Result<i64> {
        self.insert_on_conflict(record, ConflictResolution::Replace)
    }

    /// True when a row was deleted.
    pub fn delete<T: Record>(&self, record: &T) -> Result<bool> {
        let sql = format!(
            "DELETE FROM \"{}\" WHERE \"{}\" = ?",
            T::TABLE,
            T::PRIMARY_KEY
        );
        Ok(self.conn().execute(&sql, [Self::primary_key(record)])? > 0)
    }

    pub fn fetch_one<T: Record>(&self, key: impl ToSql) -> Result<Option<T>> {
        Self::fetch_one_in(&self.conn(), key)
    }

    /// `fetch_one` inside a `write` or `read` block.
    pub fn fetch_one_in<T: Record>(db: &Connection, key: impl ToSql) -> Result<Option<T>> {
        let sql = format!(
            "SELECT * FROM \"{}\" WHERE \"{}\" = ?",
            T::TABLE,
            T::PRIMARY_KEY
        );
        db.query_row(&sql, [key], T::from_row).optional()
    }

    pub fn fetch_all<T: Record>(&self) -> Result<Vec<T>> {
        self.fetch_all_sql(&format!("SELECT * FROM \"{}\"", T::TABLE), [])
    }

    pub fn fetch_all_sql<T: Record>(&self, sql: &str, arguments: impl Params) -> Result<Vec<T>> {
        Self::fetch_all_in(&self.conn(), sql, arguments)
    }

    /// Runs `block` in a transaction, committed when it returns `Ok`.
    pub fn write<T>(&self, block: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        let mut db = self.conn();
        let tx = db.transaction()?;
        let value = block(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    pub fn read<T>(&self, block: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        block(&self.conn())
    }

    /// `?` placeholders for `IN (...)` clauses. Callers must guard against
    /// empty input first (`IN ()` is invalid SQL).
    pub fn placeholders(count: usize) -> String {
        vec!["?"; count].join(",")
    }

    pub fn chunked(ids: &[String], size: usize) -> Vec<Vec<String>> {
        ids.chunks(size).map(<[String]>::to_vec).collect()
    }

    /// Songs keyed by id. Empty input → empty map.
    pub fn fetch_song_map_by_ids(
        ids: &[String],
        db: &Connection,
    ) -> Result<HashMap<String, SongEntity>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let sql = format!(
            "SELECT * FROM song WHERE id IN ({})",
            Self::placeholders(ids.len())
        );
        let fetched: Vec<SongEntity> = Self::fetch_all_in(db, &sql, params_from_iter(ids))?;
        Ok(fetched
            .into_iter()
            .map(|song| (song.id.clone(), song))
            .collect())
    }

    fn fetch_all_in<T: Record>(
        db: &Connection,
        sql: &str,
        arguments: impl Params,
    ) -> Result<Vec<T>> {
        db.prepare(sql)?
            .query_map(arguments, T::from_row)?
            .collect()
    }

    fn insert_in<T: Record>(
        db: &Connection,
        record: &T,
        on_conflict: ConflictResolution,
    ) -> Result<()> {
        let columns: Vec<_> = T::COLUMNS.iter().map(|c| format!("\"{c}\"")).collect();
        let sql = format!(
            "{} INTO \"{}\" ({}) VALUES ({})",
            on_conflict.sql(),
            T::TABLE,
            columns.join(", "),
            Self::placeholders(columns.len())
        );
        db.execute(&sql, params_from_iter(record.values()))?;
        Ok(())
    }

    /// Changed row count.
    fn update_in<T: Record>(db: &Connection, record: &T) -> Result<usize> {
        let assignments: Vec<_> = T::COLUMNS
            .iter()
            .filter(|c| **c != T::PRIMARY_KEY)
            .map(|c| format!("\"{c}\" = ?"))
            .collect();
        let sql = format!(
            "UPDATE \"{}\" SET {} WHERE \"{}\" = ?",
            T::TABLE,
            assignments.join(", "),
            T::PRIMARY_KEY
        );
        let mut values: Vec<_> = T::COLUMNS
            .iter()
            .zip(record.values())
            .filter(|(c, _)| **c != T::PRIMARY_KEY)
            .map(|(_, v)| v)
            .collect();
        values.push(Self::primary_key(record));
        db.execute(&sql, params_from_iter(values))
    }

    fn primary_key<T: Record>(record: &T) -> rusqlite::types::Value {
        let index = T::COLUMNS
            .iter()
            .position(|c| *c == T::PRIMARY_KEY)
            .expect("PRIMARY_KEY is one of COLUMNS");
        record.values().swap_remove(index)
    }
}

// MARK: Playback Transactions

impl DatabaseService {
    pub fn increment_total_play_time(&self, song_id: &str, play_time_ms: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE song SET total_play_time = total_play_time + ? WHERE id = ?",
            params![play_time_ms, song_id],
        )?;
        Ok(())
    }

    pub fn increment_play_count(&self, song_id: &str, year: i64, month: i64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO play_count (song_id, year, month, count)
             VALUES (?, ?, ?, 1)
             ON CONFLICT(song_id, year, month) DO UPDATE SET
                 count = count + 1",
            params![song_id, year, month],
        )?;
        Ok(())
    }
}

/// `days` before now.
// ponytail: whole 24 h days; Swift's local calendar shifts by an hour across DST.
fn days_ago(days: i64) -> SystemTime {
    let span = Duration::from_secs(days.unsigned_abs() * 86_400);
    let now = SystemTime::now();
    if days >= 0 { now - span } else { now + span }
}

// MARK: Personalization Queries

impl DatabaseService {
    /// Liked songs ordered by play count (for QuickPicks)
    pub fn fetch_liked_songs(&self) -> Result<Vec<SongEntity>> {
        self.fetch_all_sql(
            "SELECT * FROM song WHERE liked = 1 ORDER BY total_play_time DESC LIMIT 25",
            [],
        )
    }

    /// Recently played songs (for KeepListening). Defaults: 30 days, 20 songs.
    pub fn fetch_recent_songs(
        &self,
        days: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<SongEntity>> {
        let cutoff = DbDate(days_ago(days.unwrap_or(30)));
        let db = self.conn();
        let recent_ids: Vec<String> = db
            .prepare(
                "SELECT song_id FROM event
                 WHERE timestamp > ?
                 GROUP BY song_id
                 ORDER BY MAX(timestamp) DESC
                 LIMIT ?",
            )?
            .query_map(params![cutoff, limit.unwrap_or(20)], |row| row.get(0))?
            .collect::<Result<_>>()?;
        let mut by_id = Self::fetch_song_map_by_ids(&recent_ids, &db)?;
        // Preserve recency order from recent_ids (most recent play first)
        Ok(recent_ids
            .iter()
            .filter_map(|id| by_id.remove(id))
            .collect())
    }

    /// Forgotten favorites: liked songs not played in N days. Defaults: 60 days, 15 songs.
    pub fn fetch_forgotten_favorites(
        &self,
        days: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<SongEntity>> {
        let cutoff = DbDate(days_ago(days.unwrap_or(60)));
        self.fetch_all_sql(
            "SELECT * FROM song
             WHERE liked = 1
             AND id NOT IN (SELECT DISTINCT song_id FROM event WHERE timestamp > ?)
             ORDER BY total_play_time ASC
             LIMIT ?",
            params![cutoff, limit.unwrap_or(15)],
        )
    }

    /// Most played albums (from liked/bookmarked albums). Default limit: 10.
    pub fn fetch_albums(&self, limit: Option<i64>) -> Result<Vec<AlbumEntity>> {
        self.fetch_all_sql(
            "SELECT * FROM album WHERE bookmarked_at IS NOT NULL ORDER BY bookmarked_at DESC LIMIT ?",
            [limit.unwrap_or(10)],
        )
    }

    /// Most played artists (from subscribed artists). Default limit: 10.
    pub fn fetch_artists(&self, limit: Option<i64>) -> Result<Vec<ArtistEntity>> {
        self.fetch_all_sql(
            "SELECT * FROM artist WHERE bookmarked_at IS NOT NULL ORDER BY name ASC LIMIT ?",
            [limit.unwrap_or(10)],
        )
    }

    /// User's playlists. Default limit: 10, applied before hiding `HIDDEN_IDS`, as in Swift.
    pub fn fetch_playlists(&self, limit: Option<i64>) -> Result<Vec<PlaylistEntity>> {
        let mut playlists: Vec<PlaylistEntity> = self.fetch_all_sql(
            "SELECT * FROM playlist WHERE bookmarked_at IS NOT NULL ORDER BY bookmarked_at DESC LIMIT ?",
            [limit.unwrap_or(10)],
        )?;
        playlists.retain(|p| !PlaylistEntity::HIDDEN_IDS.contains(&p.id.as_str()));
        Ok(playlists)
    }

    /// Default sort: recently added.
    pub fn fetch_all_liked_songs(&self, sort: Option<LibrarySongSort>) -> Result<Vec<SongEntity>> {
        let order = match sort.unwrap_or_default() {
            LibrarySongSort::Title => "title ASC",
            LibrarySongSort::Artist => "artist_name ASC, title ASC",
            LibrarySongSort::RecentlyAdded => "create_date DESC",
            LibrarySongSort::MostPlayed => "total_play_time DESC",
        };
        self.fetch_all_sql(
            &format!("SELECT * FROM song WHERE liked = 1 ORDER BY {order}"),
            [],
        )
    }

    pub fn fetch_all_albums(&self) -> Result<Vec<AlbumEntity>> {
        self.fetch_all_sql(
            "SELECT * FROM album WHERE bookmarked_at IS NOT NULL ORDER BY title ASC",
            [],
        )
    }

    pub fn fetch_all_podcasts(&self) -> Result<Vec<PodcastEntity>> {
        self.fetch_all_sql(
            "SELECT * FROM podcast WHERE subscribed_at IS NOT NULL ORDER BY name ASC",
            [],
        )
    }

    pub fn fetch_liked_song_ids(&self) -> Result<Vec<String>> {
        Self::strings(&self.conn(), "SELECT id FROM song WHERE liked = 1")
    }

    pub fn fetch_all_liked_song_count(&self) -> Result<i64> {
        self.conn()
            .query_row("SELECT COUNT(*) FROM song WHERE liked = 1", [], |row| {
                row.get(0)
            })
    }

    pub fn fetch_top_songs(&self, limit: i64) -> Result<Vec<SongEntity>> {
        self.fetch_all_sql(
            "SELECT * FROM song
             WHERE total_play_time > 0
             ORDER BY total_play_time DESC
             LIMIT ?",
            [limit],
        )
    }

    /// Swift `fetchTopSongs(limit:from:to:)`: most played time in `(from, to]`.
    pub fn fetch_top_songs_between(
        &self,
        limit: i64,
        from: SystemTime,
        to: SystemTime,
    ) -> Result<Vec<SongEntity>> {
        self.fetch_all_sql(
            "SELECT song.* FROM song
             JOIN (
                 SELECT song_id, SUM(play_time) AS played FROM event
                 WHERE timestamp > ? AND timestamp <= ?
                 GROUP BY song_id
                 ORDER BY played DESC
                 LIMIT ?
             ) AS top ON song.id = top.song_id
             ORDER BY top.played DESC",
            params![DbDate(from), DbDate(to), limit],
        )
    }

    pub fn orphan_song_ids(&self) -> Result<Vec<String>> {
        Self::strings(
            &self.conn(),
            "SELECT DISTINCT song_id FROM event WHERE song_id NOT IN (SELECT id FROM song)",
        )
    }

    fn strings(db: &Connection, sql: &str) -> Result<Vec<String>> {
        db.prepare(sql)?.query_map([], |row| row.get(0))?.collect()
    }
}

// MARK: History

impl DatabaseService {
    /// Default limit: 50.
    pub fn fetch_history(&self, limit: Option<i64>) -> Result<Vec<HistoryEntry>> {
        let db = self.conn();
        let events: Vec<Event> = Self::fetch_all_in(
            &db,
            "SELECT * FROM event ORDER BY timestamp DESC LIMIT ?",
            [limit.unwrap_or(50)],
        )?;
        let song_ids: Vec<String> = events.iter().map(|e| e.song_id.clone()).collect();
        let songs = Self::fetch_song_map_by_ids(&song_ids, &db)?;
        Ok(events
            .into_iter()
            .map(|event| HistoryEntry {
                song: songs.get(&event.song_id).cloned(),
                event,
            })
            .collect())
    }
}

// MARK: - Speed Dial (homepage pinning)

impl DatabaseService {
    /// Default limit: 20.
    pub fn fetch_speed_dial(&self, limit: Option<i64>) -> Result<Vec<SpeedDialEntry>> {
        self.fetch_all_sql(
            "SELECT * FROM speed_dial ORDER BY pinned_at DESC LIMIT ?",
            [limit.unwrap_or(20)],
        )
    }

    pub fn is_pinned_to_speed_dial(&self, video_id: &str) -> Result<bool> {
        Ok(self.fetch_one::<SpeedDialEntry>(video_id)?.is_some())
    }

    pub fn pin_to_speed_dial(&self, song: &SongItem) -> Result<()> {
        self.insert_or_replace(&SpeedDialEntry::new(song))?;
        Ok(())
    }

    pub fn remove_from_speed_dial(&self, video_id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM speed_dial WHERE video_id = ?", [video_id])?;
        Ok(())
    }
}
