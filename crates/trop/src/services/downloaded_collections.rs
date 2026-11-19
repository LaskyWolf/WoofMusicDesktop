//! Trop/Services/DownloadedCollections.swift: which playlists and albums have
//! their songs saved. The Swift class refreshes itself from change
//! notifications; here the app calls `read` after downloads or the library change.

use std::collections::{HashMap, HashSet};

use super::playlist_song_edits::PlaylistSongEdits;
use crate::database::database_service::DatabaseService;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DownloadedCollections {
    /// Bare playlist ids and album browse ids with every song saved.
    complete: HashSet<String>,
    /// The same ids, with at least one song saved.
    with_downloads: HashSet<String>,
}

impl DownloadedCollections {
    /// Whether the album or playlist has every song saved.
    pub fn is_complete(&self, id: &str) -> bool {
        self.complete.contains(PlaylistSongEdits::bare_id(id))
    }

    pub fn has_downloads(&self, id: &str) -> bool {
        self.with_downloads.contains(PlaylistSongEdits::bare_id(id))
    }

    /// Swift `refresh`. Search opens a playlist by its bare id, the library by
    /// its VL id, so both fold into the bare one.
    pub fn read(db: &DatabaseService) -> rusqlite::Result<Self> {
        let rows: Vec<(String, bool, i64)> = db.read(|conn| {
            conn.prepare(
                "SELECT m.playlist_id AS id, COUNT(d.id) = COUNT(*), COUNT(*)
                 FROM playlist_song_map m
                 JOIN song s ON s.id = m.song_id
                 LEFT JOIN downloaded_track d ON d.id = m.song_id
                 WHERE s.is_unavailable = 0
                 GROUP BY m.playlist_id HAVING COUNT(d.id) > 0
                 UNION ALL
                 SELECT a.album_id, COUNT(d.id) = COUNT(*), COUNT(*)
                 FROM album_song a
                 LEFT JOIN downloaded_track d ON d.id = a.song_id
                 GROUP BY a.album_id HAVING COUNT(d.id) > 0",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect()
        })?;
        let mut newest: HashMap<String, (bool, i64)> = HashMap::new();
        for (id, is_complete, total) in rows {
            let id = PlaylistSongEdits::bare_id(&id).to_owned();
            // A playlist can have two snapshots, under its bare and VL ids. An old one mustn't win.
            match newest.get(&id) {
                Some(&(_, kept)) if kept >= total => {}
                _ => {
                    newest.insert(id, (is_complete, total));
                }
            }
        }
        Ok(Self {
            complete: newest
                .iter()
                .filter(|(_, (is_complete, _))| *is_complete)
                .map(|(id, _)| id.clone())
                .collect(),
            with_downloads: newest.into_keys().collect(),
        })
    }

    /// Swift `saveSongs(_:ofAlbum:)`: remembers an album's available songs,
    /// because nothing else in the database links them to it.
    pub fn save_album_songs(
        db: &DatabaseService,
        browse_id: &str,
        song_ids: &[String],
    ) -> rusqlite::Result<()> {
        db.write(|tx| {
            tx.execute("DELETE FROM album_song WHERE album_id = ?", [browse_id])?;
            for song_id in song_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO album_song (album_id, song_id) VALUES (?, ?)",
                    [browse_id, song_id],
                )?;
            }
            Ok(())
        })
    }
}
