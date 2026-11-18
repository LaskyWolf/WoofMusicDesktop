//! Swift `DatabaseMigrations`. GRDB's `create(table:)` and `alter(table:)` calls
//! are written out as the SQL GRDB generates, with the same names, types and defaults.

use std::time::SystemTime;

use rusqlite::Connection;

use super::date_text;

type Result = rusqlite::Result<()>;

/// Wraps a migration body with start/complete debug logs.
fn logged(name: &str, db: &Connection, block: impl FnOnce(&Connection) -> Result) -> Result {
    log::debug!("Running {name} migration");
    block(db)?;
    log::debug!("{name} migration complete");
    Ok(())
}

pub fn table_exists(db: &Connection, name: &str) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
        [name],
        |row| row.get(0),
    )
}

/// Column names of `table`, in table order (GRDB `db.columns(in:)`).
pub fn columns(db: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = db.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    statement.query_map([], |row| row.get("name"))?.collect()
}

/// The canonical downloaded_track DDL, shared by v5 (fresh create) and
/// v7 (rebuild target).
pub fn create_downloaded_track_table(db: &Connection, name: &str) -> Result {
    db.execute_batch(&format!(
        r#"CREATE TABLE "{name}" ("id" TEXT PRIMARY KEY, "title" TEXT NOT NULL, "artist" TEXT NOT NULL, "duration" INTEGER NOT NULL, "thumbnail_url" TEXT, "local_path" TEXT NOT NULL, "downloaded_at" TEXT NOT NULL)"#
    ))
}

pub fn v1(db: &Connection) -> Result {
    db.execute_batch(
        r#"
        CREATE TABLE "song" ("id" TEXT PRIMARY KEY, "title" TEXT NOT NULL, "duration" INTEGER NOT NULL, "thumbnail_url" TEXT, "liked" INTEGER NOT NULL DEFAULT 0, "total_play_time" INTEGER NOT NULL DEFAULT 0, "in_library" TEXT, "library_add_token" TEXT NOT NULL DEFAULT '', "library_remove_token" TEXT NOT NULL DEFAULT '', "is_episode" INTEGER NOT NULL DEFAULT 0, "is_uploaded" INTEGER NOT NULL DEFAULT 0, "is_video" INTEGER NOT NULL DEFAULT 0, "create_date" TEXT NOT NULL, "modify_date" TEXT NOT NULL);
        CREATE TABLE "artist" ("id" TEXT PRIMARY KEY, "name" TEXT NOT NULL, "thumbnail_url" TEXT, "bookmarked_at" TEXT, "is_podcast_channel" INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE "album" ("id" TEXT PRIMARY KEY, "title" TEXT NOT NULL, "playlist_id" TEXT, "thumbnail_url" TEXT, "song_count" INTEGER NOT NULL DEFAULT 0, "duration" INTEGER NOT NULL DEFAULT 0, "bookmarked_at" TEXT);
        CREATE TABLE "playlist" ("id" TEXT PRIMARY KEY, "browse_id" TEXT, "name" TEXT NOT NULL, "is_editable" INTEGER NOT NULL DEFAULT 0, "bookmarked_at" TEXT, "remote_song_count" INTEGER);
        CREATE TABLE "playlist_song_map" ("id" INTEGER PRIMARY KEY AUTOINCREMENT, "playlist_id" TEXT NOT NULL, "song_id" TEXT NOT NULL, "position" INTEGER NOT NULL, "set_video_id" TEXT, UNIQUE ("playlist_id", "song_id"));
        CREATE TABLE "event" ("id" INTEGER PRIMARY KEY AUTOINCREMENT, "song_id" TEXT NOT NULL, "timestamp" TEXT NOT NULL, "play_time" INTEGER NOT NULL, UNIQUE ("song_id", "timestamp"));
        CREATE TABLE "play_count" ("song_id" TEXT NOT NULL, "year" INTEGER NOT NULL, "month" INTEGER NOT NULL, "count" INTEGER NOT NULL DEFAULT 0, PRIMARY KEY ("song_id", "year", "month"));
        CREATE TABLE "format" ("id" TEXT PRIMARY KEY, "itag" INTEGER NOT NULL, "mime_type" TEXT NOT NULL, "codecs" TEXT NOT NULL, "bitrate" INTEGER NOT NULL, "sample_rate" INTEGER NOT NULL, "content_length" INTEGER NOT NULL, "loudness_db" DOUBLE, "perceptual_loudness_db" DOUBLE, "playback_url" TEXT);
        "#,
    )
}

pub fn v2(db: &Connection) -> Result {
    db.execute_batch(
        r#"
        ALTER TABLE "song" ADD COLUMN "artist_name" TEXT;
        ALTER TABLE "song" ADD COLUMN "album_name" TEXT;
        "#,
    )
}

pub fn v3(db: &Connection) -> Result {
    logged("v3", db, |db| {
        db.execute_batch(
            r#"
            ALTER TABLE "artist" ADD COLUMN "channel_id" TEXT;
            ALTER TABLE "album" ADD COLUMN "is_uploaded" INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE "playlist" ADD COLUMN "is_auto_sync" INTEGER NOT NULL DEFAULT 0;
            CREATE TABLE "episode" ("id" TEXT PRIMARY KEY, "title" TEXT NOT NULL, "duration" INTEGER NOT NULL, "thumbnail_url" TEXT, "podcast_id" TEXT, "podcast_name" TEXT, "is_played" INTEGER NOT NULL DEFAULT 0, "saved_at" TEXT);
            CREATE TABLE "podcast" ("id" TEXT PRIMARY KEY, "name" TEXT NOT NULL, "thumbnail_url" TEXT, "subscribed_at" TEXT);
            CREATE TABLE "podcast_episode_map" ("id" INTEGER PRIMARY KEY AUTOINCREMENT, "podcast_id" TEXT NOT NULL, "episode_id" TEXT NOT NULL, "position" INTEGER NOT NULL, UNIQUE ("podcast_id", "episode_id"));
            "#,
        )
    })
}

pub fn v4(db: &Connection) -> Result {
    logged("v4", db, |db| {
        db.execute_batch(r#"ALTER TABLE "playlist" ADD COLUMN "thumbnail_url" TEXT"#)
    })
}

pub fn v5(db: &Connection) -> Result {
    logged("v5", db, |db| {
        if !table_exists(db, "downloaded_track")? {
            create_downloaded_track_table(db, "downloaded_track")?;
        } else if !columns(db, "downloaded_track")?
            .iter()
            .any(|c| c == "artist")
        {
            // Table existed from an earlier schema without the artist column.
            db.execute_batch(
                r#"ALTER TABLE "downloaded_track" ADD COLUMN "artist" TEXT NOT NULL DEFAULT ''"#,
            )?;
        }
        Ok(())
    })
}

pub fn v6(db: &Connection) -> Result {
    logged("v6", db, |db| {
        // Earlier schemas may have created downloaded_track with fewer columns
        // than DownloadedTrackEntity expects. Add any missing columns so inserts
        // from the app succeed on existing databases.
        if table_exists(db, "downloaded_track")? {
            let existing = columns(db, "downloaded_track")?;
            // (name, type, not null)
            let required = [
                ("artist", "TEXT", true),
                ("thumbnail_url", "TEXT", false),
                ("local_path", "TEXT", true),
                ("downloaded_at", "TEXT", true),
                ("duration", "INTEGER", true),
                ("title", "TEXT", true),
            ];
            for (name, kind, not_null) in required {
                if existing.iter().any(|c| c == name) {
                    continue;
                }
                // Swift gives every NOT NULL column a '' default, the INTEGER one too.
                let constraint = if not_null { " NOT NULL DEFAULT ''" } else { "" };
                db.execute_batch(&format!(
                    r#"ALTER TABLE "downloaded_track" ADD COLUMN "{name}" {kind}{constraint}"#
                ))?;
            }
        }
        Ok(())
    })
}

pub fn v7(db: &Connection) -> Result {
    logged("v7", db, |db| {
        // The existing downloaded_track table may have been created by an older
        // schema (e.g. with a file_size column) that doesn't match
        // DownloadedTrackEntity. Rebuild it to exactly match the entity,
        // preserving any rows we can carry over by id.
        if table_exists(db, "downloaded_track")? {
            create_downloaded_track_table(db, "downloaded_track_new")?;
            // Carry over rows that have the columns we need (id + local_path).
            if columns(db, "downloaded_track")?
                .iter()
                .any(|c| c == "local_path")
            {
                // Swift's `ISO8601DateFormatter` text (`yyyy-MM-ddTHH:mm:ssZ`) for rows
                // missing downloaded_at; `DbDate` reads it back.
                let fallback_date = format!(
                    "{}Z",
                    date_text(SystemTime::now())[..19].replacen(' ', "T", 1)
                );
                db.execute(
                    "INSERT INTO downloaded_track_new (id, title, artist, duration, thumbnail_url, local_path, downloaded_at)
                     SELECT id,
                            COALESCE(title, ''),
                            COALESCE(artist, ''),
                            COALESCE(duration, 0),
                            thumbnail_url,
                            local_path,
                            COALESCE(NULLIF(downloaded_at, ''), ?)
                     FROM downloaded_track
                     WHERE local_path IS NOT NULL",
                    [fallback_date],
                )?;
            }
            db.execute_batch(
                r#"
                DROP TABLE "downloaded_track";
                ALTER TABLE "downloaded_track_new" RENAME TO "downloaded_track";
                "#,
            )?;
        }
        Ok(())
    })
}

pub fn v8(db: &Connection) -> Result {
    logged("v8", db, |db| {
        // Songs pinned to the homepage Speed Dial.
        if !table_exists(db, "speed_dial")? {
            db.execute_batch(
                r#"CREATE TABLE "speed_dial" ("video_id" TEXT PRIMARY KEY, "title" TEXT NOT NULL, "artist" TEXT NOT NULL, "album" TEXT, "thumbnail_url" TEXT, "duration" INTEGER NOT NULL DEFAULT 0, "pinned_at" DATETIME NOT NULL)"#,
            )?;
        }
        Ok(())
    })
}

pub fn v9(db: &Connection) -> Result {
    logged("v9", db, |db| {
        // History and recency queries filter and sort events by timestamp.
        db.execute_batch("CREATE INDEX IF NOT EXISTS event_on_timestamp ON event(timestamp)")
    })
}

pub fn v10(db: &Connection) -> Result {
    logged("v10", db, |db| {
        // Each album's songs, so its cards know when all of them are downloaded.
        db.execute_batch(
            r#"CREATE TABLE "album_song" ("album_id" TEXT NOT NULL, "song_id" TEXT NOT NULL, PRIMARY KEY ("album_id", "song_id"))"#,
        )
    })
}

pub fn v11(db: &Connection) -> Result {
    logged("v11", db, |db| {
        // Songs YouTube Music greys out, for pages built from the database.
        db.execute_batch(
            r#"ALTER TABLE "song" ADD COLUMN "is_unavailable" BOOLEAN NOT NULL DEFAULT 0"#,
        )
    })
}
