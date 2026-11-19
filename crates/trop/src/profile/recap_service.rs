//! `Trop/Profile/RecapService.swift`: listening stats for one period and the
//! badges, read from the local `event` history plus YouTube Music's history.
//! Days and hours come from the local time zone, like Swift's `Calendar.current`.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use rusqlite::{Connection, OptionalExtension};

use crate::database::DbDate;
use crate::database::database_service::DatabaseService;
use crate::history::history_view_model::{RemoteSection, shelf_date};
use crate::networking::models::yt_item::SongItem;

// Badge rules. Shares need `RATIO_MINIMUM_MINUTES` of listening first, so a
// single late-night song can't make a Night Owl.
pub const NIGHT_OWL_SHARE: f64 = 0.4;
pub const EARLY_BIRD_SHARE: f64 = 0.3;
pub const RATIO_MINIMUM_MINUTES: i64 = 120;
pub const LOYAL_PUP_PLAYS: i64 = 50;
pub const STREAK_DAYS: i64 = 7;
pub const EXPLORER_ARTISTS: i64 = 50;
pub const PODCAST_SAGE_HOURS: i64 = 10;
pub const FIRST_STEPS_PLAYS: i64 = 10;
pub const THOUSAND_CLUB_PLAYS: i64 = 1000;
pub const HUNDRED_HOURS_HOURS: i64 = 100;
pub const MARATHON_MINUTES: i64 = 180;
pub const WEEKEND_SHARE: f64 = 0.4;
pub const LUNCH_SHARE: f64 = 0.25;
pub const INSOMNIAC_NIGHTS: i64 = 10;
pub const IRON_PAW_DAYS: i64 = 30;
pub const OLD_FRIEND_DAYS: i64 = 100;
pub const ON_REPEAT_PLAYS: i64 = 10;
pub const SUPERFAN_PLAYS: i64 = 300;
pub const DEEP_DIVER_SONGS: i64 = 25;
pub const COLLECTOR_SONGS: i64 = 500;
pub const VARIETY_ARTISTS: i64 = 15;
pub const PODCAST_PUP_MINUTES: i64 = 60;
pub const BINGE_EPISODES: i64 = 5;
/// Length counted for a YouTube history song that came without one.
pub const UNKNOWN_SONG_SECONDS: i64 = 180;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BadgeKind {
    NightOwl,
    EarlyBird,
    LoyalPup,
    Streak,
    Explorer,
    PodcastSage,
    TimeBender,
    FirstSteps,
    ThousandClub,
    HundredHours,
    Marathon,
    WeekendWarrior,
    LunchBreak,
    Insomniac,
    IronPaw,
    OldFriend,
    OnRepeat,
    Superfan,
    DeepDiver,
    Collector,
    VarietyPack,
    PodcastPup,
    BingeListener,
}

impl BadgeKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::NightOwl => "Night Owl",
            Self::EarlyBird => "Early Bird",
            Self::LoyalPup => "Loyal Pup",
            Self::Streak => "Streak Dragon",
            Self::Explorer => "Explorer",
            Self::PodcastSage => "Podcast Sage",
            Self::TimeBender => "Time Bender",
            Self::FirstSteps => "First Steps",
            Self::ThousandClub => "Thousand Club",
            Self::HundredHours => "Hundred Hours",
            Self::Marathon => "Marathon",
            Self::WeekendWarrior => "Weekend Warrior",
            Self::LunchBreak => "Lunch Break",
            Self::Insomniac => "Insomniac",
            Self::IronPaw => "Iron Paw",
            Self::OldFriend => "Old Friend",
            Self::OnRepeat => "On Repeat",
            Self::Superfan => "Superfan",
            Self::DeepDiver => "Deep Diver",
            Self::Collector => "Collector",
            Self::VarietyPack => "Variety Pack",
            Self::PodcastPup => "Podcast Pup",
            Self::BingeListener => "Binge Listener",
        }
    }

    pub fn emoji(self) -> &'static str {
        match self {
            Self::NightOwl => "🦉",
            Self::EarlyBird => "🌅",
            Self::LoyalPup => "🐶",
            Self::Streak => "🐉",
            Self::Explorer => "🧭",
            Self::PodcastSage => "📻",
            Self::TimeBender => "⏩",
            Self::FirstSteps => "🐣",
            Self::ThousandClub => "🎟️",
            Self::HundredHours => "⏳",
            Self::Marathon => "🏃",
            Self::WeekendWarrior => "🎉",
            Self::LunchBreak => "🥪",
            Self::Insomniac => "🌙",
            Self::IronPaw => "🐾",
            Self::OldFriend => "🦴",
            Self::OnRepeat => "🔁",
            Self::Superfan => "⭐",
            Self::DeepDiver => "🤿",
            Self::Collector => "📚",
            Self::VarietyPack => "🎨",
            Self::PodcastPup => "🎙️",
            Self::BingeListener => "📺",
        }
    }

    pub fn summary(self) -> String {
        let pct = |share: f64| (share * 100.0) as i64;
        match self {
            Self::NightOwl => format!(
                "{}% of your listening between midnight and 5 am",
                pct(NIGHT_OWL_SHARE)
            ),
            Self::EarlyBird => format!(
                "{}% of your listening between 5 and 9 am",
                pct(EARLY_BIRD_SHARE)
            ),
            Self::LoyalPup => format!("Played one song {LOYAL_PUP_PLAYS} times"),
            Self::Streak => format!("Listened {STREAK_DAYS} days in a row"),
            Self::Explorer => format!("Played songs by {EXPLORER_ARTISTS} different artists"),
            Self::PodcastSage => format!("Listened to {PODCAST_SAGE_HOURS} hours of episodes"),
            Self::TimeBender => "Played a show at a speed other than 1×".into(),
            Self::FirstSteps => format!("Played your first {FIRST_STEPS_PLAYS} songs"),
            Self::ThousandClub => "Played 1,000 songs".into(),
            Self::HundredHours => format!("Listened for {HUNDRED_HOURS_HOURS} hours"),
            Self::Marathon => format!("Listened {} hours in a single day", MARATHON_MINUTES / 60),
            Self::WeekendWarrior => {
                format!("{}% of your listening on weekends", pct(WEEKEND_SHARE))
            }
            Self::LunchBreak => format!(
                "{}% of your listening between noon and 2 pm",
                pct(LUNCH_SHARE)
            ),
            Self::Insomniac => {
                format!("Listened between 3 and 5 am on {INSOMNIAC_NIGHTS} nights")
            }
            Self::IronPaw => format!("Listened {IRON_PAW_DAYS} days in a row"),
            Self::OldFriend => format!("Listened on {OLD_FRIEND_DAYS} different days"),
            Self::OnRepeat => format!("Played one song {ON_REPEAT_PLAYS} times in a single day"),
            Self::Superfan => format!("Played one artist {SUPERFAN_PLAYS} times"),
            Self::DeepDiver => format!("Played {DEEP_DIVER_SONGS} different songs by one artist"),
            Self::Collector => format!("Played {COLLECTOR_SONGS} different songs"),
            Self::VarietyPack => {
                format!("Played {VARIETY_ARTISTS} different artists in a single day")
            }
            Self::PodcastPup => "Listened to your first hour of episodes".into(),
            Self::BingeListener => format!("Played {BINGE_EPISODES} episodes in a single day"),
        }
    }

    /// Swift `hexPair`: the gem's light and dark colours as 0xRRGGBB.
    pub fn colors(self) -> (u32, u32) {
        match self {
            Self::NightOwl => (0xB98CFF, 0x4B2A8A),
            Self::EarlyBird => (0xFFE08A, 0xE07B39),
            Self::LoyalPup => (0xFFD36E, 0xD9822B),
            Self::Streak => (0xFF8A8A, 0xA8324A),
            Self::Explorer => (0x7FE0D2, 0x2A7A70),
            Self::PodcastSage => (0xFFA98A, 0xB8463A),
            Self::TimeBender => (0x9FD0FF, 0x2B5FA8),
            Self::FirstSteps => (0xFFF3A8, 0xE0A92B),
            Self::ThousandClub => (0xFFB3D9, 0xB0306E),
            Self::HundredHours => (0xE8D3A8, 0x8A6235),
            Self::Marathon => (0xA8F0A0, 0x2F8A3A),
            Self::WeekendWarrior => (0xFFC2E8, 0x9A3AA8),
            Self::LunchBreak => (0xFFE3B0, 0xC9783A),
            Self::Insomniac => (0x8FA8FF, 0x2A2F7A),
            Self::IronPaw => (0xC9D2DE, 0x4A5566),
            Self::OldFriend => (0xF2E6D0, 0x9A7A52),
            Self::OnRepeat => (0x8AF0FF, 0x2A7AA8),
            Self::Superfan => (0xFFE98A, 0xC9952B),
            Self::DeepDiver => (0x8AD8FF, 0x1F4F8A),
            Self::Collector => (0xD9B38C, 0x7A4A2B),
            Self::VarietyPack => (0xFFB08A, 0xC93A5A),
            Self::PodcastPup => (0xFFC9A8, 0xB8603A),
            Self::BingeListener => (0xC8A8FF, 0x5A3AA8),
        }
    }
}

/// A badge computed from history, with no table of its own: it is unlocked
/// while the history meets its rule, so deleting history can lock it again.
#[derive(Clone, Debug, PartialEq)]
pub struct RecapBadge {
    pub kind: BadgeKind,
    pub is_unlocked: bool,
    /// 0...1 toward unlocking.
    pub progress: f64,
    /// "4/7 days".
    pub progress_text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecapSong {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub thumbnail_url: Option<String>,
    pub plays: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecapArtist {
    pub name: String,
    pub minutes: i64,
    pub plays: i64,
    /// The artist's picture when the artist table has it, else the cover of their most played song.
    pub thumbnail_url: Option<String>,
}

/// One period: `start` inclusive, `end` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub start: Timestamp,
    pub end: Timestamp,
}

impl Interval {
    fn contains(&self, time: Timestamp) -> bool {
        self.start <= time && time < self.end
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecapStats {
    pub interval: Interval,
    pub total_minutes: i64,
    pub plays: i64,
    /// Minutes per 7-day slice of the period, from its first day.
    pub weekly_minutes: Vec<i64>,
    pub top_song: Option<RecapSong>,
    /// Up to five, most listened first. Empty when no song has an artist name.
    pub top_artists: Vec<RecapArtist>,
    /// Every badge, as of the end of the period.
    pub badges: Vec<RecapBadge>,
    /// Badges locked at the start of the period and unlocked at its end.
    pub new_badges: Vec<RecapBadge>,
}

impl RecapStats {
    pub fn is_empty(&self) -> bool {
        self.plays == 0
    }

    pub fn unlocked_count(&self) -> usize {
        self.badges.iter().filter(|b| b.is_unlocked).count()
    }
}

// MARK: - Periods

pub fn month_containing(time: Timestamp, tz: &TimeZone) -> Interval {
    let date = time.to_zoned(tz.clone()).date().first_of_month();
    let start = date.to_zoned(tz.clone()).map(|z| z.timestamp());
    let end = date
        .checked_add(1.month())
        .and_then(|next| next.to_zoned(tz.clone()))
        .map(|z| z.timestamp());
    match (start, end) {
        (Ok(start), Ok(end)) => Interval { start, end },
        _ => Interval {
            start: time,
            end: time,
        },
    }
}

pub fn last_month(now: Timestamp, tz: &TimeZone) -> Interval {
    let start = month_containing(now, tz).start;
    month_containing(start - 1.nanosecond(), tz)
}

pub fn month_to_date(now: Timestamp, tz: &TimeZone) -> Interval {
    Interval {
        start: month_containing(now, tz).start,
        end: now,
    }
}

/// "October".
pub fn month_name(interval: Interval, tz: &TimeZone) -> String {
    interval
        .start
        .to_zoned(tz.clone())
        .strftime("%B")
        .to_string()
}

// MARK: - Reading

/// One play, from this app's events or from YouTube Music's history.
#[derive(Clone, Debug)]
pub struct Play {
    song_id: String,
    time: Timestamp,
    day: Date,
    hour: i8,
    ms: i64,
    artist: Option<String>,
    is_episode: bool,
}

/// A song from the account's YouTube Music history, dated by its shelf.
#[derive(Clone, Debug)]
pub struct RemotePlay {
    pub song: SongItem,
    pub time: Timestamp,
}

/// The YouTube Music history shelves inside `interval`.
pub fn remote_plays(
    sections: &[RemoteSection],
    interval: Interval,
    now: Timestamp,
    tz: &TimeZone,
) -> Vec<RemotePlay> {
    let today = now.to_zoned(tz.clone()).date();
    sections
        .iter()
        .filter_map(|(title, songs)| {
            let time = shelf_date(title, today)?
                .to_zoned(tz.clone())
                .ok()?
                .timestamp();
            interval.contains(time).then(|| {
                songs.iter().map(move |song| RemotePlay {
                    song: song.clone(),
                    time,
                })
            })
        })
        .flatten()
        .collect()
}

// ponytail: loads every event up to `end` to compute lifetime badges;
// move the badge counts into SQL if histories reach hundreds of thousands of rows.
/// Every play before `end` (all of them for `None`), oldest first.
pub fn fetch_plays(
    db: &Connection,
    end: Option<Timestamp>,
    tz: &TimeZone,
) -> rusqlite::Result<Vec<Play>> {
    let end = DbDate(end.map_or_else(
        || SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1 << 40),
        SystemTime::from,
    ));
    let mut statement = db.prepare(
        "SELECT event.song_id, event.timestamp, event.play_time, song.artist_name,
                (COALESCE(song.is_episode, 0) = 1 OR episode.id IS NOT NULL) AS is_ep
         FROM event
         LEFT JOIN song ON song.id = event.song_id
         LEFT JOIN episode ON episode.id = event.song_id
         WHERE event.timestamp < ?
         ORDER BY event.timestamp",
    )?;
    let rows = statement.query_map([end], |row| {
        let time: DbDate = row.get("timestamp")?;
        let artist: Option<String> = row.get("artist_name")?;
        Ok((
            row.get::<_, String>("song_id")?,
            time.0,
            row.get::<_, i64>("play_time")?,
            artist.filter(|a| !a.is_empty()),
            row.get::<_, bool>("is_ep")?,
        ))
    })?;
    let mut plays = Vec::new();
    for row in rows {
        let (song_id, time, ms, artist, is_episode) = row?;
        let Ok(time) = Timestamp::try_from(time) else {
            continue;
        };
        plays.push(play(song_id, time, ms, artist, is_episode, tz));
    }
    Ok(plays)
}

fn play(
    song_id: String,
    time: Timestamp,
    ms: i64,
    artist: Option<String>,
    is_episode: bool,
    tz: &TimeZone,
) -> Play {
    let zoned = time.to_zoned(tz.clone());
    Play {
        song_id,
        time,
        day: zoned.date(),
        hour: zoned.hour(),
        ms,
        artist,
        is_episode,
    }
}

/// Whether this app played anything in `interval`.
pub fn has_local_history(db: &DatabaseService, interval: Interval) -> bool {
    db.read(|db| {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM event WHERE timestamp >= ? AND timestamp < ?)",
            [DbDate(interval.start.into()), DbDate(interval.end.into())],
            |row| row.get(0),
        )
    })
    .unwrap_or(false)
}

/// Stats for `interval`. `time_bent` is whether a podcast speed other than 1× was picked.
pub fn compute(
    db: &DatabaseService,
    remote: &[RemotePlay],
    interval: Interval,
    time_bent: bool,
    tz: &TimeZone,
) -> rusqlite::Result<RecapStats> {
    db.read(|db| {
        let plays = fetch_plays(db, Some(interval.end), tz)?;
        stats(db, &plays, remote, interval, time_bent, tz)
    })
}

/// Every badge as of now, for the profile.
pub fn lifetime_badges(db: &DatabaseService, time_bent: bool, tz: &TimeZone) -> Vec<RecapBadge> {
    db.read(|db| fetch_plays(db, None, tz))
        .map(|plays| badges(&plays, time_bent))
        .unwrap_or_else(|error| {
            log::error!("Badges failed: {error}");
            badges(&[], time_bent)
        })
}

// MARK: - Aggregation

fn stats(
    db: &Connection,
    plays: &[Play],
    remote: &[RemotePlay],
    interval: Interval,
    time_bent: bool,
    tz: &TimeZone,
) -> rusqlite::Result<RecapStats> {
    let local: Vec<&Play> = plays.iter().filter(|p| p.time >= interval.start).collect();
    // A song this app played in the period is already counted: YouTube
    // history lists the app's own plays too.
    let local_ids: HashSet<&str> = local.iter().map(|p| p.song_id.as_str()).collect();
    let mut remote_songs: HashMap<&str, &SongItem> = HashMap::new();
    for entry in remote {
        remote_songs
            .entry(&entry.song.video_id)
            .or_insert(&entry.song);
    }
    // Badges read local plays only: a history song's hour is unknown.
    let remote_plays: Vec<Play> = remote
        .iter()
        .filter(|entry| !local_ids.contains(entry.song.video_id.as_str()))
        .map(|entry| {
            let seconds = if entry.song.duration > 0 {
                entry.song.duration
            } else {
                UNKNOWN_SONG_SECONDS
            };
            let mut remote = play(
                entry.song.video_id.clone(),
                entry.time,
                seconds * 1000,
                entry.song.artists.first().map(|a| a.name.clone()),
                entry.song.music_video_type.as_deref() == Some("MUSIC_VIDEO_TYPE_PODCAST_EPISODE"),
                tz,
            );
            remote.hour = 12;
            remote
        })
        .collect();
    let in_period: Vec<&Play> = local.into_iter().chain(remote_plays.iter()).collect();
    let total_ms: i64 = in_period.iter().map(|p| p.ms).sum();

    let start_day = interval.start.to_zoned(tz.clone()).date();
    let end_day = interval.end.to_zoned(tz.clone()).date();
    let days_between = |a: Date, b: Date| a.until(b).map_or(0, |span| span.get_days() as i64);
    let day_count = days_between(start_day, end_day).max(1);
    // Whole weeks; the last one takes the month's leftover days.
    let mut weekly = vec![0i64; (day_count / 7).max(1) as usize];
    for p in &in_period {
        let index = (days_between(start_day, p.day) / 7).clamp(0, weekly.len() as i64 - 1);
        weekly[index as usize] += p.ms;
    }

    let music: Vec<&&Play> = in_period.iter().filter(|p| !p.is_episode).collect();
    let mut song_plays: HashMap<&str, i64> = HashMap::new();
    for p in &music {
        *song_plays.entry(&p.song_id).or_default() += 1;
    }
    // Most plays; ties go to the smaller id, like Swift's comparator.
    let top_song_id = song_plays
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(id, _)| id.to_string());

    let mut artist_ms: HashMap<&str, i64> = HashMap::new();
    let mut artist_plays: HashMap<&str, i64> = HashMap::new();
    let mut artist_song_plays: HashMap<&str, HashMap<&str, i64>> = HashMap::new();
    for p in &music {
        let Some(artist) = p.artist.as_deref() else {
            continue;
        };
        *artist_ms.entry(artist).or_default() += p.ms;
        *artist_plays.entry(artist).or_default() += 1;
        *artist_song_plays
            .entry(artist)
            .or_default()
            .entry(&p.song_id)
            .or_default() += 1;
    }
    let mut ranked: Vec<(&str, i64)> = artist_ms.iter().map(|(k, v)| (*k, *v)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let top_artist_names: Vec<&str> = ranked.into_iter().take(5).map(|(name, _)| name).collect();
    let artist_top_song: HashMap<&str, String> = top_artist_names
        .iter()
        .filter_map(|name| {
            let songs = artist_song_plays.get(name)?;
            let (id, _) = songs
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))?;
            Some((*name, id.to_string()))
        })
        .collect();

    let mut ids: Vec<String> = artist_top_song.values().cloned().collect();
    ids.extend(top_song_id.clone());
    ids.sort();
    ids.dedup();
    let songs = DatabaseService::fetch_song_map_by_ids(&ids, db)?;
    let top_song = top_song_id.and_then(|id| {
        let plays = song_plays.get(id.as_str()).copied().unwrap_or(0);
        if let Some(song) = songs.get(&id) {
            return Some(RecapSong {
                title: song.title.clone(),
                artist: song.artist_name.clone(),
                thumbnail_url: song.thumbnail_url.clone(),
                plays,
                id,
            });
        }
        remote_songs.get(id.as_str()).map(|song| RecapSong {
            title: song.title.clone(),
            artist: song.artists.first().map(|a| a.name.clone()),
            thumbnail_url: song.thumbnail_url.clone(),
            plays,
            id,
        })
    });
    let mut top_artists = Vec::new();
    for name in top_artist_names {
        let picture: Option<String> = db
            .query_row(
                "SELECT thumbnail_url FROM artist WHERE name = ? AND thumbnail_url IS NOT NULL LIMIT 1",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        let song_cover = artist_top_song.get(name).and_then(|id| {
            songs
                .get(id)
                .and_then(|s| s.thumbnail_url.clone())
                .or_else(|| {
                    remote_songs
                        .get(id.as_str())
                        .and_then(|s| s.thumbnail_url.clone())
                })
        });
        top_artists.push(RecapArtist {
            name: name.to_owned(),
            minutes: artist_ms.get(name).copied().unwrap_or(0) / 60_000,
            plays: artist_plays.get(name).copied().unwrap_or(0),
            thumbnail_url: picture.or(song_cover),
        });
    }

    let earlier: Vec<Play> = plays
        .iter()
        .filter(|p| p.time < interval.start)
        .cloned()
        .collect();
    // Time Bender has no date, so it never counts as new.
    let before = badges(&earlier, time_bent);
    let after = badges(plays, time_bent);
    let locked_before: HashSet<BadgeKind> = before
        .iter()
        .filter(|b| !b.is_unlocked)
        .map(|b| b.kind)
        .collect();
    let new_badges = after
        .iter()
        .filter(|b| b.is_unlocked && locked_before.contains(&b.kind))
        .cloned()
        .collect();

    Ok(RecapStats {
        interval,
        total_minutes: total_ms / 60_000,
        plays: in_period.len() as i64,
        weekly_minutes: weekly.into_iter().map(|ms| ms / 60_000).collect(),
        top_song,
        top_artists,
        badges: after,
        new_badges,
    })
}

type Values = (bool, f64, String);

fn tally(value: i64, goal: i64, unit: &str) -> Values {
    (
        value >= goal,
        value as f64 / goal as f64,
        format!("{}/{goal} {unit}", value.min(goal)),
    )
}

fn count_by<'a, K: std::hash::Hash + Eq>(
    plays: impl Iterator<Item = &'a Play>,
    key: impl Fn(&'a Play) -> K,
) -> HashMap<K, Vec<&'a Play>> {
    let mut map: HashMap<K, Vec<&Play>> = HashMap::new();
    for p in plays {
        map.entry(key(p)).or_default().push(p);
    }
    map
}

fn distinct<'a, T: std::hash::Hash + Eq + 'a>(items: impl Iterator<Item = T>) -> i64 {
    items.collect::<HashSet<T>>().len() as i64
}

pub fn badges(plays: &[Play], time_bent: bool) -> Vec<RecapBadge> {
    let total_ms: i64 = plays.iter().map(|p| p.ms).sum();
    let total_minutes = total_ms / 60_000;

    let share = |needed: f64, label: &str, counts: &dyn Fn(&Play) -> bool| -> Values {
        let ms: i64 = plays.iter().filter(|p| counts(p)).map(|p| p.ms).sum();
        let ratio = if total_ms > 0 {
            ms as f64 / total_ms as f64
        } else {
            0.0
        };
        let has_enough = total_minutes >= RATIO_MINIMUM_MINUTES;
        let text = if has_enough {
            format!("{}% {label}", (ratio * 100.0) as i64)
        } else {
            format!("{total_minutes}/{RATIO_MINIMUM_MINUTES} min listened")
        };
        let progress =
            ratio / needed * (total_minutes as f64 / RATIO_MINIMUM_MINUTES as f64).min(1.0);
        (has_enough && ratio >= needed, progress, text)
    };

    let music: Vec<&Play> = plays.iter().filter(|p| !p.is_episode).collect();
    let episodes: Vec<&Play> = plays.iter().filter(|p| p.is_episode).collect();
    let by_artist = count_by(music.iter().copied().filter(|p| p.artist.is_some()), |p| {
        p.artist.as_deref().unwrap_or_default()
    });
    let music_days = count_by(music.iter().copied(), |p| p.day);
    let best_song_plays = count_by(music.iter().copied(), |p| p.song_id.as_str())
        .values()
        .map(|v| v.len() as i64)
        .max()
        .unwrap_or(0);
    let episode_ms: i64 = episodes.iter().map(|p| p.ms).sum();
    let best_day_minutes = count_by(plays.iter(), |p| p.day)
        .values()
        .map(|v| v.iter().map(|p| p.ms).sum::<i64>() / 60_000)
        .max()
        .unwrap_or(0);
    let late_nights = distinct(
        plays
            .iter()
            .filter(|p| (3..5).contains(&p.hour))
            .map(|p| p.day),
    );
    let best_day_song_repeats = music_days
        .values()
        .map(|day| {
            count_by(day.iter().copied(), |p| p.song_id.as_str())
                .values()
                .map(|v| v.len() as i64)
                .max()
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0);
    let best_artist_plays = by_artist
        .values()
        .map(|v| v.len() as i64)
        .max()
        .unwrap_or(0);
    let best_artist_songs = by_artist
        .values()
        .map(|v| distinct(v.iter().map(|p| &p.song_id)))
        .max()
        .unwrap_or(0);
    let best_day_artists = music_days
        .values()
        .map(|v| distinct(v.iter().filter_map(|p| p.artist.as_deref())))
        .max()
        .unwrap_or(0);
    let best_day_episodes = count_by(episodes.iter().copied(), |p| p.day)
        .values()
        .map(|v| distinct(v.iter().map(|p| &p.song_id)))
        .max()
        .unwrap_or(0);

    let mut days: Vec<Date> = plays
        .iter()
        .map(|p| p.day)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    days.sort();
    let (mut longest, mut run) = (0i64, 0i64);
    let mut previous: Option<Date> = None;
    for day in &days {
        run = if previous.and_then(|p| p.tomorrow().ok()) == Some(*day) {
            run + 1
        } else {
            1
        };
        longest = longest.max(run);
        previous = Some(*day);
    }
    let weekend = |p: &Play| matches!(p.day.weekday(), Weekday::Saturday | Weekday::Sunday);
    let time_bender: Values = if time_bent {
        (true, 1.0, "Speed changed".into())
    } else {
        (false, 0.0, "Try a show at 1.5×".into())
    };

    use BadgeKind::*;
    let values: Vec<(BadgeKind, Values)> = vec![
        (
            NightOwl,
            share(NIGHT_OWL_SHARE, "after midnight", &|p| {
                (0..5).contains(&p.hour)
            }),
        ),
        (
            EarlyBird,
            share(EARLY_BIRD_SHARE, "before 9 am", &|p| {
                (5..9).contains(&p.hour)
            }),
        ),
        (LoyalPup, tally(best_song_plays, LOYAL_PUP_PLAYS, "plays")),
        (Streak, tally(longest, STREAK_DAYS, "days")),
        (
            Explorer,
            tally(by_artist.len() as i64, EXPLORER_ARTISTS, "artists"),
        ),
        (
            PodcastSage,
            tally(episode_ms / 3_600_000, PODCAST_SAGE_HOURS, "h"),
        ),
        (TimeBender, time_bender),
        (
            FirstSteps,
            tally(plays.len() as i64, FIRST_STEPS_PLAYS, "plays"),
        ),
        (
            ThousandClub,
            tally(plays.len() as i64, THOUSAND_CLUB_PLAYS, "plays"),
        ),
        (
            HundredHours,
            tally(total_minutes / 60, HUNDRED_HOURS_HOURS, "h"),
        ),
        (
            Marathon,
            tally(best_day_minutes, MARATHON_MINUTES, "min in a day"),
        ),
        (
            WeekendWarrior,
            share(WEEKEND_SHARE, "on weekends", &weekend),
        ),
        (
            LunchBreak,
            share(LUNCH_SHARE, "at lunch", &|p| (12..14).contains(&p.hour)),
        ),
        (Insomniac, tally(late_nights, INSOMNIAC_NIGHTS, "nights")),
        (IronPaw, tally(longest, IRON_PAW_DAYS, "days")),
        (OldFriend, tally(days.len() as i64, OLD_FRIEND_DAYS, "days")),
        (
            OnRepeat,
            tally(best_day_song_repeats, ON_REPEAT_PLAYS, "plays in a day"),
        ),
        (Superfan, tally(best_artist_plays, SUPERFAN_PLAYS, "plays")),
        (
            DeepDiver,
            tally(best_artist_songs, DEEP_DIVER_SONGS, "songs"),
        ),
        (
            Collector,
            tally(
                distinct(music.iter().map(|p| &p.song_id)),
                COLLECTOR_SONGS,
                "songs",
            ),
        ),
        (
            VarietyPack,
            tally(best_day_artists, VARIETY_ARTISTS, "artists in a day"),
        ),
        (
            PodcastPup,
            tally(episode_ms / 60_000, PODCAST_PUP_MINUTES, "min"),
        ),
        (
            BingeListener,
            tally(best_day_episodes, BINGE_EPISODES, "episodes in a day"),
        ),
    ];
    values
        .into_iter()
        .map(
            |(kind, (is_unlocked, progress, progress_text))| RecapBadge {
                kind,
                is_unlocked,
                progress: progress.clamp(0.0, 1.0),
                progress_text,
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn song_play(id: &str, time: &str, minutes: i64, artist: &str) -> Play {
        play(
            id.into(),
            at(time),
            minutes * 60_000,
            Some(artist.into()),
            false,
            &TimeZone::UTC,
        )
    }

    #[test]
    fn a_week_of_plays_makes_a_streak_dragon() {
        let plays: Vec<Play> = (1..=7)
            .map(|day| song_play("a", &format!("2026-10-{day:02}T20:00:00Z"), 3, "Pup"))
            .collect();
        let badges = badges(&plays, false);
        let streak = badges.iter().find(|b| b.kind == BadgeKind::Streak).unwrap();
        assert!(streak.is_unlocked);
        let iron = badges
            .iter()
            .find(|b| b.kind == BadgeKind::IronPaw)
            .unwrap();
        assert_eq!(iron.progress_text, "7/30 days");
        // 21 minutes is under the 120 a share needs.
        let owl = badges
            .iter()
            .find(|b| b.kind == BadgeKind::NightOwl)
            .unwrap();
        assert_eq!(owl.progress_text, "21/120 min listened");
    }

    #[test]
    fn a_month_runs_from_its_first_midnight() {
        let tz = TimeZone::UTC;
        let month = last_month(at("2026-11-19T10:00:00Z"), &tz);
        assert_eq!(month.start, at("2026-10-01T00:00:00Z"));
        assert_eq!(month.end, at("2026-11-01T00:00:00Z"));
        assert_eq!(month_name(month, &tz), "October");
    }

    #[test]
    fn stats_rank_songs_and_artists_in_the_period() {
        let db = DatabaseService::open_in_memory();
        let plays = vec![
            song_play("old", "2026-09-30T12:00:00Z", 100, "Before"),
            song_play("a", "2026-10-02T12:00:00Z", 4, "Pup"),
            song_play("a", "2026-10-09T12:00:00Z", 4, "Pup"),
            song_play("b", "2026-10-20T12:00:00Z", 30, "Wolf"),
        ];
        let tz = TimeZone::UTC;
        let interval = month_containing(at("2026-10-15T00:00:00Z"), &tz);
        let stats = db
            .read(|db| stats(db, &plays, &[], interval, false, &tz))
            .unwrap();
        assert_eq!(stats.total_minutes, 38);
        assert_eq!(stats.plays, 3);
        assert_eq!(stats.weekly_minutes, vec![4, 4, 30, 0]);
        assert_eq!(stats.top_song.as_ref().map(|s| s.plays), None); // songs not in the db
        assert_eq!(stats.top_artists[0].name, "Wolf");
        assert_eq!(stats.top_artists[1].plays, 2);
        // 138 minutes, all at noon: Lunch Break unlocks once October passes 120.
        let new: Vec<BadgeKind> = stats.new_badges.iter().map(|b| b.kind).collect();
        assert!(new.contains(&BadgeKind::LunchBreak));
        assert!(!new.contains(&BadgeKind::FirstSteps));
    }
}
