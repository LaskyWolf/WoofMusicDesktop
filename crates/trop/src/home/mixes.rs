//! The data half of `Trop/Home/MixesRow.swift` (`MixBuilder`): queues built
//! from what you play and like. A mix starts with its songs and goes on with
//! songs like the first one, so it does not run dry (the app appends a radio).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

use crate::database::database_service::DatabaseService;
use crate::database::entities::song_entity::SongEntity;
use crate::networking::models::yt_item::SongItem;
use crate::services::personalization_service::song_item;

/// Fewest songs a mix needs to be worth showing.
pub const MINIMUM_SONGS: usize = 5;
const DAY: Duration = Duration::from_secs(86_400);

/// Which mix it is; the app picks its icon from this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixKind {
    Flow,
    OnRepeat,
    Rewind,
    Artist,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mix {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub kind: MixKind,
    /// The card's colour, RGB in 0...1.
    pub color: [f32; 3],
    pub songs: Vec<SongItem>,
}

const INDIGO: [f32; 3] = [0.36, 0.35, 0.84];
const RED: [f32; 3] = [0.89, 0.35, 0.29];
const TEAL: [f32; 3] = [0.16, 0.55, 0.55];
const ARTIST_COLORS: [[f32; 3]; 2] = [[0.85, 0.55, 0.15], [0.55, 0.32, 0.7]];

/// Flow from your favorites, On Repeat and Rewind from your history, and a
/// mix for each of your two most played artists. Mixes without enough songs
/// are left out.
pub fn build(db: &DatabaseService, now: SystemTime) -> Vec<Mix> {
    let top = db.fetch_top_songs(60).unwrap_or_default();
    let liked = db.fetch_liked_songs().unwrap_or_default();
    let month_ago = now - 30 * DAY;
    let year_ago = now - 365 * DAY;
    let recent = db
        .fetch_top_songs_between(30, month_ago, now)
        .unwrap_or_default();
    let earlier = db
        .fetch_top_songs_between(30, year_ago, month_ago)
        .unwrap_or_default();

    let mut seen = HashSet::new();
    let favorites: Vec<SongEntity> = top
        .iter()
        .chain(&liked)
        .filter(|song| seen.insert(song.id.clone()) && is_music(song))
        .cloned()
        .collect();

    let mut mixes: Vec<Mix> = [
        mix(
            "flow",
            "Flow",
            "Your favorites, endlessly",
            MixKind::Flow,
            INDIGO,
            &favorites,
            true,
        ),
        mix(
            "on-repeat",
            "On Repeat",
            "Last 30 days",
            MixKind::OnRepeat,
            RED,
            &recent,
            false,
        ),
        mix(
            "rewind",
            "Rewind",
            "From earlier",
            MixKind::Rewind,
            TEAL,
            &earlier,
            false,
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    for (index, artist) in top_artists(&top).into_iter().enumerate() {
        let songs: Vec<SongEntity> = favorites
            .iter()
            .filter(|song| song.artist_name.as_deref() == Some(artist.as_str()))
            .cloned()
            .collect();
        mixes.extend(mix(
            &format!("artist-{artist}"),
            &format!("{artist} Mix"),
            "Songs you play",
            MixKind::Artist,
            ARTIST_COLORS[index % ARTIST_COLORS.len()],
            &songs,
            true,
        ));
    }
    mixes
}

fn mix(
    id: &str,
    title: &str,
    subtitle: &str,
    kind: MixKind,
    color: [f32; 3],
    songs: &[SongEntity],
    shuffled: bool,
) -> Option<Mix> {
    let mut usable: Vec<&SongEntity> = songs.iter().filter(|song| is_music(song)).collect();
    if usable.len() < MINIMUM_SONGS {
        return None;
    }
    if shuffled {
        fastrand::shuffle(&mut usable);
    }
    Some(Mix {
        id: id.to_owned(),
        title: title.to_owned(),
        subtitle: subtitle.to_owned(),
        kind,
        color,
        songs: usable.into_iter().take(30).map(song_item).collect(),
    })
}

fn is_music(song: &SongEntity) -> bool {
    !song.title.is_empty() && !song.is_episode && !song.is_unavailable
}

/// The two artists with the most songs among your most played, at least
/// `MINIMUM_SONGS` each; ties go to the name first in order.
fn top_artists(top: &[SongEntity]) -> Vec<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for song in top.iter().filter(|song| is_music(song)) {
        if let Some(name) = song.artist_name.as_deref().filter(|name| !name.is_empty()) {
            *counts.entry(name).or_default() += 1;
        }
    }
    let mut counts: Vec<(&str, usize)> = counts
        .into_iter()
        .filter(|(_, count)| *count >= MINIMUM_SONGS)
        .collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    counts
        .into_iter()
        .take(2)
        .map(|(name, _)| name.to_owned())
        .collect()
}
