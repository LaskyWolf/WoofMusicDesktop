//! The pages: Home, Explore, album, artist, playlist, podcast, mood, Library, Liked Songs, My Top 100.

use std::sync::Arc;

use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Gradient, Length, Radians};
use trop::detail::detail_models::{
    AlbumDetailInfo, ArtistDetailInfo, PlaylistDetailInfo, PodcastDetailInfo,
};
use trop::explore::mood_card::{MoodFamily, MoodFamilyEntry};
use trop::networking::models::home_page::{
    ExploreSection, ExploreSectionKind, HomeSection, MoodItem,
};
use trop::networking::models::yt_item::{SongItem, YTItem};
use trop::playback::episode_playback_store::EpisodePlaybackStore;

use crate::collections::{self, Caps, Ctx};
use crate::playlist_edit;
use crate::services::Library;
use crate::settings;
use crate::ui::{self, Covers};
use crate::{Load, Message, Route, theme};
use trop::settings::settings_store::LibraryLayout;

pub fn home<'a>(
    covers: &'a Covers,
    home: &'a Load<Vec<HomeSection>>,
    playing: Option<&str>,
    recap_card: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let sections = match home {
        Load::Loaded(sections) => sections,
        Load::Loading => return ui::status(None, Message::Retry),
        Load::Failed(error) => return ui::status(Some(error), Message::Retry),
    };
    // Swift shows the Recap card in Keep Listening's place.
    let has_card = recap_card.is_some();
    ui::scroll_page(
        recap_card
            .into_iter()
            .chain(
                sections
                    .iter()
                    .filter(|section| !section.items().is_empty())
                    .filter(|section| {
                        !(has_card && matches!(section, HomeSection::KeepListening(_)))
                    })
                    .map(|section| {
                        let limit = if section.is_songs_only() { 12 } else { 10 };
                        ui::section(
                            covers,
                            section.display_title(),
                            section.items(),
                            limit,
                            playing,
                        )
                    }),
            )
            .collect(),
    )
}

/// Explore feed, or a mood's page: shelves of rows and cards, mood chips.
pub fn explore<'a>(
    covers: &'a Covers,
    sections: &'a [ExploreSection],
    playing: Option<&str>,
    family: Option<&str>,
    lead: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let shelves = sections
        .iter()
        .filter(|section| !section.items.is_empty() || !section.moods.is_empty())
        .map(|section| match section.kind {
            ExploreSectionKind::Moods => family_chips(&section.title, &section.moods, family),
            _ if section.title.to_lowercase().contains("new albums") => {
                new_releases(covers, section)
            }
            ExploreSectionKind::Rows => {
                ui::section(covers, &section.title, &section.items, 12, playing)
            }
            _ => ui::section(covers, &section.title, &section.items, 10, playing),
        });
    ui::scroll_page(lead.into_iter().chain(shelves).collect())
}

/// Swift `FilterChipBar` of `MoodFamily.entries`: a family with several genres
/// is one chip, and the chosen family's genres show as cards under the chips.
fn family_chips<'a>(
    title: &'a str,
    all: &'a [MoodItem],
    family: Option<&str>,
) -> Element<'a, Message> {
    let chips = MoodFamily::entries(all)
        .into_iter()
        .map(|entry| match entry {
            MoodFamilyEntry::Group { family: name, .. } => {
                let chosen = family == Some(name.as_str());
                let message = Message::ExploreFamily((!chosen).then(|| name.clone()));
                settings::chip(name, chosen, message)
            }
            MoodFamilyEntry::Mood(mood) => settings::chip(
                mood.title.clone(),
                false,
                Message::Navigate(Route::Mood(mood)),
            ),
        });
    let mut page = column![
        ui::section_title(title),
        iced::widget::Row::with_children(chips)
            .spacing(8)
            .wrap()
            .vertical_spacing(8),
    ]
    .spacing(14);
    if let Some(family) = family {
        let genres: Vec<MoodItem> = all
            .iter()
            .filter(|mood| MoodFamily::family(&mood.title) == Some(family))
            .cloned()
            .collect();
        page = page.push(moods(format!("{family} sub-genres"), genres));
    }
    page.into()
}

/// Swift `NewReleasesSection`: the shelf on a rounded backdrop, the cards in one row.
// ponytail: Swift tints each card with its cover's colours, spins a vinyl disc and adds
// Play and Save to the card; the cards here are the plain tiles.
fn new_releases<'a>(covers: &'a Covers, section: &'a ExploreSection) -> Element<'a, Message> {
    let songs = ui::songs_of(&section.items);
    let cards = iced::widget::Row::with_children(
        section
            .items
            .iter()
            .map(|item| ui::tile(covers, item, ui::subtitle(item), &songs, false)),
    )
    .spacing(10);
    container(
        column![
            ui::section_title(section.title.as_str()),
            iced::widget::scrollable(cards).direction(
                iced::widget::scrollable::Direction::Horizontal(
                    iced::widget::scrollable::Scrollbar::new()
                        .width(4)
                        .scroller_width(4),
                )
            ),
        ]
        .spacing(12),
    )
    .padding(14)
    .width(Length::Fill)
    .style(|_| container::Style {
        background: Some(Background::Color(theme::PLACEHOLDER)),
        border: Border::default().rounded(22),
        ..container::Style::default()
    })
    .into()
}

/// "Pop › Brazilian Pop ✕" on a sub-genre's page: the family chip and the ✕ both
/// go back to all of Pop. `None` for a genre that is not in a family, or is the family.
pub fn mood_path<'a>(mood: &MoodItem) -> Option<Element<'a, Message>> {
    let family =
        MoodFamily::family(&mood.title).filter(|f| !f.eq_ignore_ascii_case(&mood.title))?;
    let pill = |label: String, shade: f32| {
        button(text(label).size(14).font(theme::weight(Weight::Medium)))
            .padding([6, 16])
            .on_press(Message::ExplorePath(family.to_owned()))
            .style(move |_, _| button::Style {
                background: Some(Background::Color(Color {
                    r: theme::accent().r * shade,
                    g: theme::accent().g * shade,
                    b: theme::accent().b * shade,
                    a: 1.0,
                })),
                border: Border::default().rounded(18),
                text_color: Color::WHITE,
                ..button::Style::default()
            })
    };
    Some(
        row![
            pill(family.to_owned(), 1.0),
            pill(format!("›  {}  ✕", mood.title), 0.7),
        ]
        .spacing(8)
        .into(),
    )
}

/// Mood chips: one line each, so every chip in a row has the same height.
fn moods<'a>(title: impl text::IntoFragment<'a>, moods: Vec<MoodItem>) -> Element<'a, Message> {
    const CHIP: f32 = 228.0;
    // Chip width minus padding, swatch and gap.
    const LABEL: f32 = CHIP - 2.0 * 14.0 - 4.0 - 12.0;
    let chips = moods.into_iter().map(|mood| {
        let color = mood
            .color
            .map(|rgb| Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
            .unwrap_or(theme::accent());
        let swatch = container(Space::new())
            .width(4)
            .height(22)
            .style(move |_| container::Style {
                background: Some(Background::Color(color)),
                border: Border::default().rounded(2),
                ..container::Style::default()
            });
        button(
            row![
                swatch,
                text(ui::fit(&mood.title, LABEL, 14.0))
                    .size(14)
                    .font(theme::weight(Weight::Medium))
                    .wrapping(text::Wrapping::None)
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([10, 14])
        .width(CHIP)
        .on_press(Message::Navigate(Route::Mood(mood)))
        .style(|_, status| button::Style {
            background: Some(Background::Color(if status == button::Status::Hovered {
                theme::PRESSED
            } else {
                theme::HOVER
            })),
            border: Border::default().rounded(12),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
    });
    column![
        ui::section_title(title),
        iced::widget::Row::with_children(chips)
            .spacing(10)
            .wrap()
            .vertical_spacing(10),
    ]
    .spacing(14)
    .into()
}

pub fn album<'a>(
    covers: &'a Covers,
    album: &'a AlbumDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    playing: Option<&str>,
    cx: Ctx<'a>,
) -> Element<'a, Message> {
    let line = [
        ui::artists(&album.artists),
        album.year.map(|y| y.to_string()).unwrap_or_default(),
        count(album.song_count.max(album.songs.len() as i64), "song"),
        ui::total_length(album.duration),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" • ");
    let link = collections::share_url(
        &Route::Album(album.browse_id.clone()),
        album.playlist_id.as_deref(),
    );
    let mut page = vec![ui::header_with(
        ui::cover(covers, album.thumbnail_url.as_deref(), 220.0, 14.0),
        &album.title,
        vec![line],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
        Some(collections::actions(
            cx,
            songs,
            link,
            ("Save", "Saved"),
            true,
            true,
        )),
    )];
    page.push(crate::downloads::collection_actions(
        &album.browse_id,
        songs,
    ));
    page.extend(selectable(
        covers,
        songs,
        true,
        playing,
        cx,
        Caps::default(),
    ));
    ui::scroll_page(page)
}

pub fn playlist<'a>(
    covers: &'a Covers,
    playlist: &'a PlaylistDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    playing: Option<&str>,
    edit: &'a playlist_edit::State,
    in_library: bool,
    cx: Ctx<'a>,
) -> Element<'a, Message> {
    let line = [
        playlist.author_name.clone().unwrap_or_default(),
        count(playlist.song_count.max(playlist.songs.len() as i64), "song"),
        ui::total_length(playlist.duration),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" • ");
    let description = playlist
        .description_text
        .as_deref()
        .map(|d| ui::fit(d.lines().next().unwrap_or_default(), 560.0, 14.0))
        .unwrap_or_default();
    let link = collections::share_url(&Route::Playlist(playlist.playlist_id.clone()), None);
    // No cover of its own: the first four song covers.
    let art = match playlist.thumbnail_url {
        Some(_) => ui::cover(covers, playlist.thumbnail_url.as_deref(), 220.0, 14.0),
        None => collections::mosaic(covers, songs, 220.0),
    };
    let mut page = vec![ui::header_with(
        art,
        &playlist.title,
        vec![line, description],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
        Some(collections::actions(cx, songs, link, ("", ""), true, true)),
    )];
    page.push(crate::downloads::collection_actions(
        &playlist.playlist_id,
        songs,
    ));
    page.push(playlist_edit::actions(covers, edit, playlist, in_library));
    if edit.editing(&playlist.playlist_id) {
        page.push(playlist_edit::edit_tracks(
            covers, edit, playlist, songs, playing,
        ));
    } else {
        let caps = Caps {
            remove: playlist.is_editable,
            delete: false,
        };
        page.extend(selectable(covers, songs, true, playing, cx, caps));
    }
    ui::scroll_page(page)
}

pub fn artist<'a>(
    covers: &'a Covers,
    artist: &'a ArtistDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    related: &'a [YTItem],
    playing: Option<&str>,
    cx: Ctx<'a>,
) -> Element<'a, Message> {
    let link = collections::share_url(&Route::Artist(artist.browse_id.clone()), None);
    let mut page = vec![ui::header_with(
        ui::cover(covers, artist.thumbnail_url.as_deref(), 220.0, 110.0),
        &artist.name,
        vec![
            artist
                .subscriber_count_text
                .as_deref()
                .map(subscriber_label)
                .unwrap_or_default(),
        ],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
        Some(collections::actions(
            cx,
            songs,
            link,
            ("Subscribe", "Subscribed"),
            true,
            false,
        )),
    )];
    if !songs.is_empty() {
        page.push(
            column![
                ui::section_title("Songs"),
                tracks(covers, songs, true, playing)
            ]
            .spacing(14)
            .into(),
        );
    }
    // Shelves of albums, singles, playlists and related artists, kept as YTItems.
    for (title, items) in shelves(related) {
        page.push(ui::section(covers, title, items, 10, playing));
    }
    if let Some(about) = artist.description_text.as_deref().filter(|d| !d.is_empty()) {
        page.push(
            column![
                ui::section_title("About"),
                container(text(about).size(14).color(theme::SECONDARY)).max_width(720),
            ]
            .spacing(14)
            .into(),
        );
    }
    ui::scroll_page(page)
}

/// A podcast: header, description, then its episodes, newest first as YouTube sends them.
pub fn podcast<'a>(
    covers: &'a Covers,
    podcast: &'a PodcastDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    playing: Option<&str>,
    cx: Ctx<'a>,
    positions: &EpisodePlaybackStore,
) -> Element<'a, Message> {
    let link = collections::share_url(&Route::Podcast(podcast.browse_id.clone()), None);
    let mut page = vec![ui::header_with(
        ui::cover(covers, podcast.thumbnail_url.as_deref(), 220.0, 14.0),
        &podcast.title,
        vec![
            podcast.author.clone().unwrap_or_default(),
            count(podcast.episodes.len() as i64, "episode"),
        ],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
        Some(collections::actions(
            cx,
            songs,
            link,
            ("Subscribe", "Subscribed"),
            false,
            false,
        )),
    )];
    if let Some((index, saved)) = collections::latest_in_progress(&podcast.episodes, positions) {
        page.push(collections::resume_card(covers, songs, index, saved));
    }
    if let Some(about) = podcast
        .description_text
        .as_deref()
        .filter(|d| !d.is_empty())
    {
        page.push(collections::description(cx.state, about));
    }
    // `songs` holds the episodes in the same order (`Detail::podcast`).
    let rows = podcast.episodes.iter().enumerate().map(|(index, episode)| {
        let subtitle = [
            episode.publish_date.clone().unwrap_or_default(),
            episode.views_text.clone().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" • ");
        let is_playing = playing == Some(episode.video_id.as_str());
        let track = ui::track_row(covers, songs, index, true, is_playing, subtitle, None);
        match collections::episode_tag(positions.position(&episode.video_id), is_playing) {
            Some(label) => row![
                container(track).width(Length::Fill),
                collections::tag(label)
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
            None => track,
        }
    });
    page.push(column(rows).spacing(2).into());
    ui::scroll_page(page)
}

/// The artist page's item shelves, as (title, items) slices of one list built
/// by `artist_items`: albums, singles, playlists, related artists.
fn shelves(items: &[YTItem]) -> Vec<(&'static str, &[YTItem])> {
    let mut out = Vec::new();
    let mut start = 0;
    for (title, is) in [
        ("Albums", is_album as fn(&YTItem) -> bool),
        ("Singles & EPs", is_single),
        ("Playlists", |i| matches!(i, YTItem::Playlist(_))),
        ("Fans also like", |i| matches!(i, YTItem::Artist(_))),
    ] {
        let len = items[start..].iter().take_while(|item| is(item)).count();
        if len > 0 {
            out.push((title, &items[start..start + len]));
        }
        start += len;
    }
    out
}

fn is_album(item: &YTItem) -> bool {
    matches!(item, YTItem::Album(album) if !album.browse_id.is_empty() && album.playlist_id.as_deref() != Some("single"))
}

fn is_single(item: &YTItem) -> bool {
    matches!(item, YTItem::Album(album) if album.playlist_id.as_deref() == Some("single"))
}

/// The artist's shelves flattened in `shelves` order. Singles are marked with
/// a `playlist_id` of "single" so the two album shelves stay apart.
pub fn artist_items(artist: &ArtistDetailInfo) -> Vec<YTItem> {
    let mut items: Vec<YTItem> = artist.albums.iter().cloned().map(YTItem::Album).collect();
    items.extend(artist.singles.iter().cloned().map(|mut single| {
        single.playlist_id = Some("single".to_owned());
        YTItem::Album(single)
    }));
    items.extend(artist.playlists.iter().cloned().map(YTItem::Playlist));
    items.extend(artist.related_artists.iter().cloned().map(YTItem::Artist));
    items
}

fn tracks<'a>(
    covers: &'a Covers,
    songs: &Arc<Vec<SongItem>>,
    show_cover: bool,
    playing: Option<&str>,
) -> Element<'a, Message> {
    // ponytail: builds every row; a 5,000-song playlist should move to
    // virtual_list once the header can scroll with it.
    column((0..songs.len()).map(|index| {
        let is_playing = playing == Some(songs[index].video_id.as_str());
        let subtitle = ui::artists(&songs[index].artists);
        ui::track_row(covers, songs, index, show_cover, is_playing, subtitle, None)
    }))
    .spacing(2)
    .into()
}

/// The song list; in select mode a bar of bulk actions and rows that tick.
fn selectable<'a>(
    covers: &'a Covers,
    songs: &Arc<Vec<SongItem>>,
    show_cover: bool,
    playing: Option<&str>,
    cx: Ctx<'a>,
    caps: Caps,
) -> Vec<Element<'a, Message>> {
    if cx.state.selecting {
        vec![
            collections::bulk_bar(cx, songs.len(), caps),
            collections::pick_rows(covers, songs, cx.state),
        ]
    } else {
        vec![tracks(covers, songs, show_cover, playing)]
    }
}

/// Swift `ArtistDetailView.subscriberLabel`: "63.5M" becomes "63.5M subscribers".
fn subscriber_label(count: &str) -> String {
    if count.is_empty() || count.to_lowercase().contains("subscriber") {
        count.to_owned()
    } else {
        format!("{count} subscribers")
    }
}

pub(crate) fn count(n: i64, noun: &str) -> String {
    match n {
        0 => String::new(),
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

/// Library while signed out: Google sign-in, and the cookie paste under it.
pub fn sign_in<'a>(
    signing_in: bool,
    cookie_header: &'a str,
    error: Option<&'a str>,
) -> Element<'a, Message> {
    let mut prompt = column![
        text("Sign in to see your library")
            .size(28)
            .font(theme::weight(Weight::Bold)),
        text(
            "Your playlists, albums, artists and liked songs come from your YouTube Music account."
        )
        .size(15)
        .color(theme::SECONDARY)
        .width(Length::Fill)
        .align_x(Alignment::Center),
        Space::new().height(12),
        ui::accent_button(
            if signing_in {
                "Finish in the Google window"
            } else {
                "Sign in with Google"
            },
            None,
            (!signing_in).then_some(Message::SignIn),
        ),
    ]
    .spacing(8)
    .align_x(Alignment::Center)
    .max_width(520);
    if let Some(error) = error {
        prompt = prompt.push(text(error).size(13).color(theme::CLOSE));
    }
    let paste = row![
        text_input("Cookie header", cookie_header)
            .secure(true)
            .on_input(Message::CookieInput)
            .on_submit(Message::ImportCookies)
            .size(14)
            .padding([10, 14])
            .style(ui::field_style),
        ui::pill_button("Use cookies", Message::ImportCookies),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    ui::centered(
        prompt
            .push(Space::new().height(28))
            .push(
                text("Or paste the Cookie header of a signed-in music.youtube.com tab.")
                    .size(13)
                    .color(theme::SECONDARY),
            )
            .push(paste),
    )
}

/// Library's filter chips: one section alone instead of all four.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryFilter {
    Playlists,
    Albums,
    Artists,
    Podcasts,
}

impl LibraryFilter {
    const ALL: [(LibraryFilter, &'static str); 4] = [
        (LibraryFilter::Playlists, "Playlists"),
        (LibraryFilter::Albums, "Albums"),
        (LibraryFilter::Artists, "Artists"),
        (LibraryFilter::Podcasts, "Podcasts"),
    ];
}

/// Swift `LibraryView`: filter chips, the Liked Songs, My Top 100 and Downloads cards,
/// then playlists, albums, artists and podcasts.
pub fn library<'a>(
    covers: &'a Covers,
    library: &'a Library,
    filter: Option<LibraryFilter>,
    syncing: bool,
    edit: &'a playlist_edit::State,
    layout: LibraryLayout,
) -> Element<'a, Message> {
    let chips = row(LibraryFilter::ALL.map(|(chip, label)| {
        let selected = filter == Some(chip);
        button(text(label).size(14).font(theme::weight(Weight::Medium)))
            .padding([7, 16])
            .on_press(Message::LibraryFilter(chip))
            .style(move |_, status| button::Style {
                background: Some(Background::Color(match (selected, status) {
                    (true, _) => theme::TEXT,
                    (false, button::Status::Hovered) => theme::PRESSED,
                    (false, _) => theme::HOVER,
                })),
                border: Border::default().rounded(18),
                text_color: if selected {
                    theme::BACKGROUND
                } else {
                    theme::TEXT
                },
                ..button::Style::default()
            })
            .into()
    }))
    .spacing(8)
    .push(Space::new().width(Length::Fill))
    .push(playlist_edit::new_playlist(edit));
    let shortcuts = row![
        shortcut(
            covers,
            "Liked Songs",
            count(library.liked_count, "song"),
            &library.liked_covers,
            crate::icons::HEART,
            Color::from_rgb8(0xFF, 0x2D, 0x55),
            Route::LikedSongs,
        ),
        shortcut(
            covers,
            "My Top 100",
            "Most played".to_owned(),
            &library.top_covers,
            crate::icons::TROPHY,
            Color::from_rgb8(0xFF, 0x95, 0x00),
            Route::TopSongs,
        ),
        shortcut(
            covers,
            "Downloads",
            match library.downloaded_count {
                0 => "Offline".to_owned(),
                n => count(n, "song"),
            },
            &library.downloaded_covers,
            crate::downloads::icons::DOWNLOAD,
            Color::from_rgb8(0x2F, 0x6F, 0xDB),
            Route::Downloads,
        ),
    ]
    .spacing(12);
    let mut head = column![chips, shortcuts].spacing(20);
    if let Some(card) = crate::downloads::library_card() {
        head = head.push(card);
    }
    let mut page = vec![head.into()];
    let sections = [
        (LibraryFilter::Playlists, "Playlists", &library.playlists),
        (LibraryFilter::Albums, "Albums", &library.albums),
        (LibraryFilter::Artists, "Artists", &library.artists),
        (LibraryFilter::Podcasts, "Podcasts", &library.podcasts),
    ];
    for (kind, title, items) in sections {
        if filter.is_none_or(|filter| filter == kind) && !items.is_empty() {
            page.push(library_section(covers, title, items, layout));
        }
    }
    if page.len() == 1 {
        page.push(
            text(if syncing {
                "Syncing your library…"
            } else {
                "Nothing here yet. Save playlists, albums and artists in YouTube Music and they show up here."
            })
                .size(15)
                .color(theme::SECONDARY)
                .into(),
        );
    }
    ui::scroll_page(page)
}

fn library_section<'a>(
    covers: &'a Covers,
    title: &'a str,
    items: &'a [(YTItem, String)],
    layout: LibraryLayout,
) -> Element<'a, Message> {
    let no_songs = Arc::new(Vec::new());
    let tiles = items.iter().map(|(item, subtitle)| match layout {
        LibraryLayout::Grid => ui::tile(covers, item, subtitle.clone(), &no_songs, false),
        LibraryLayout::List => list_row(covers, item, subtitle, &no_songs),
    });
    let (spacing, vertical) = match layout {
        LibraryLayout::Grid => (20, 24),
        LibraryLayout::List => (12, 4),
    };
    column![
        row![
            ui::section_title(title),
            Space::new().width(Length::Fill),
            text(items.len()).size(15).color(theme::SECONDARY),
        ]
        .align_y(Alignment::Center),
        iced::widget::Row::with_children(tiles)
            .spacing(spacing)
            .wrap()
            .vertical_spacing(vertical),
    ]
    .spacing(14)
    .into()
}

/// Swift `MediaListRow` in the Library's List layout: a small cover, the
/// title and its subtitle. Rows wrap into columns on a wide window.
fn list_row<'a>(
    covers: &'a Covers,
    item: &'a YTItem,
    subtitle: &str,
    songs: &Arc<Vec<SongItem>>,
) -> Element<'a, Message> {
    const WIDTH: f32 = 340.0;
    const ART: f32 = 52.0;
    let round = matches!(item, YTItem::Artist(_));
    let text_width = WIDTH - ART - 14.0 - 16.0;
    // Swift names the kind when the library stores nothing else.
    let subtitle = match (subtitle, item) {
        ("", YTItem::Playlist(_)) => "Playlist",
        ("", YTItem::Album(_)) => "Album",
        ("", YTItem::Artist(_)) => "Artist",
        ("", YTItem::Podcast(_)) => "Podcast",
        (subtitle, _) => subtitle,
    };
    let content = row![
        ui::cover(
            covers,
            item.thumbnail_url(),
            ART,
            if round { ART / 2.0 } else { 8.0 }
        ),
        column![
            text(ui::fit(item.title(), text_width, 15.0))
                .size(15)
                .font(theme::weight(Weight::Medium))
                .wrapping(text::Wrapping::None),
            text(ui::fit(subtitle, text_width, 13.0))
                .size(13)
                .color(theme::SECONDARY)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(2),
    ]
    .spacing(14)
    .align_y(Alignment::Center)
    .width(WIDTH - 16.0);
    ui::clickable(content, ui::open(item, songs), 8.0)
}

/// Swift `shortcutTile`: a card in the list's colour, its first two covers
/// overlapping at the top and a large faint symbol in the corner.
fn shortcut<'a>(
    covers: &'a Covers,
    title: &'a str,
    subtitle: String,
    urls: &'a [String],
    symbol: &'static [u8],
    tint: Color,
    route: Route,
) -> Element<'a, Message> {
    const COVER: f32 = 42.0;
    const RADIUS: f32 = 14.0;
    let darker = Color::from_rgb(tint.r * 0.7, tint.g * 0.7, tint.b * 0.7);
    let fill = Linear::new(Radians(2.4))
        .add_stop(0.0, tint)
        .add_stop(1.0, darker);
    // The second cover sits over the first, ringed in the card's colour.
    let mut fan = iced::widget::Stack::new();
    for (index, url) in urls.iter().take(2).enumerate() {
        let ringed = container(ui::cover(covers, Some(url), COVER, 7.0))
            .padding(2)
            .style(move |_| container::Style {
                background: Some(Background::Color(tint)),
                border: Border::default().rounded(9),
                ..container::Style::default()
            });
        fan =
            fan.push(container(ringed).padding(iced::Padding::ZERO.left(index as f32 * 24.0_f32)));
    }
    let content = column![
        fan,
        Space::new().height(Length::Fill),
        text(title).size(15).font(theme::weight(Weight::Bold)),
        text(subtitle)
            .size(12)
            .font(theme::weight(Weight::Medium))
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.85)),
    ]
    .spacing(1);
    let mark = container(ui::icon(symbol, 64.0).opacity(0.3_f32))
        .align_right(Length::Fill)
        .align_bottom(Length::Fill);
    let card = container(iced::widget::stack![mark, content])
        .padding(12)
        .width(240)
        .height(116)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(fill))),
            border: Border::default().rounded(RADIUS),
            ..container::Style::default()
        });
    button(card)
        .padding(0)
        .on_press(Message::Navigate(route))
        .style(|_, status| button::Style {
            border: Border::default().rounded(RADIUS).width(2).color(
                if status == button::Status::Hovered {
                    Color::from_rgba(1.0, 1.0, 1.0, 0.35)
                } else {
                    Color::TRANSPARENT
                },
            ),
            ..button::Style::default()
        })
        .into()
}

/// Liked Songs or My Top 100, from the database.
pub fn songs<'a>(
    covers: &'a Covers,
    route: &Route,
    songs: &Arc<Vec<SongItem>>,
    playing: Option<&str>,
) -> Element<'a, Message> {
    let (title, empty) = match route {
        Route::LikedSongs => ("Liked Songs", "Songs you like show up here."),
        Route::Similar(_) => ("Similar Songs", "No similar tracks. Try again later."),
        _ => ("My Top 100", "The songs you play most show up here."),
    };
    let length: i64 = songs.iter().map(|song| song.duration).sum();
    let line = [count(songs.len() as i64, "song"), ui::total_length(length)]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" • ");
    let mut page = vec![ui::header(
        covers,
        songs.first().and_then(|song| song.thumbnail_url.as_deref()),
        false,
        title,
        vec![line],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
    )];
    page.push(if songs.is_empty() {
        text(empty).size(15).color(theme::SECONDARY).into()
    } else {
        tracks(covers, songs, true, playing)
    });
    ui::scroll_page(page)
}

/// Swift `HistoryScreenView`: plays by day, newest first, with a search field.
/// Plays from this app can be deleted; YouTube Music's own rows only play.
pub fn history<'a>(
    covers: &'a Covers,
    feeds: &'a crate::feeds::Feeds,
    playing: Option<&str>,
    cx: Ctx<'a>,
) -> Element<'a, Message> {
    use crate::downloads::{hint, icon_button, icons};
    use crate::feeds::Msg;

    let field = text_input("Search your history", &feeds.query)
        .on_input(|query| Message::Feeds(Msg::Query(query)))
        .size(16)
        .padding([12, 16])
        .style(ui::field_style);
    let mut page = vec![
        page_title("History"),
        container(field).max_width(640).into(),
    ];
    let selecting = cx.state.selecting;
    let history = &feeds.history;
    let days = history.filtered(&feeds.query);
    if history.is_loading {
        page.push(text("Loading…").color(theme::SECONDARY).into());
    } else if days.is_empty() {
        page.push(empty_note(
            if feeds.query.trim().is_empty() {
                "No History"
            } else {
                "No Results"
            },
            if feeds.query.trim().is_empty() {
                "Songs you play will appear here."
            } else {
                "Check the spelling or try another search."
            },
        ));
    }
    // One queue for the whole list, so a play goes on through the days below it.
    // ponytail: filtered and cloned on every redraw; cache it in `Feeds` if a long history stutters.
    let songs: Arc<Vec<SongItem>> = Arc::new(
        days.iter()
            .flat_map(|day| day.rows.iter().map(|row| row.song.clone()))
            .collect(),
    );
    if !songs.is_empty() {
        page.push(
            iced::widget::row![ui::pill_button(
                if selecting { "Done" } else { "Select" },
                Message::Pages(collections::Msg::Select(!selecting)),
            )]
            .into(),
        );
    }
    if selecting {
        let caps = Caps {
            remove: false,
            delete: true,
        };
        page.push(collections::bulk_bar(cx, songs.len(), caps));
    }
    let mut index = 0;
    for day in &days {
        let rows = day.rows.iter().map(|row| {
            if selecting {
                let pick = collections::pick_row(
                    covers,
                    &row.song,
                    index,
                    cx.state.picked.contains(&index),
                );
                index += 1;
                return pick;
            }
            let is_playing = playing == Some(row.song.video_id.as_str());
            let subtitle = ui::artists(&row.song.artists);
            let track = ui::track_row(covers, &songs, index, true, is_playing, subtitle, None);
            index += 1;
            let delete: Element<'a, Message> = match row.event.as_ref().and_then(|e| e.id) {
                Some(id) => hint(
                    icon_button(icons::TRASH, Message::Feeds(Msg::Delete(vec![id]))),
                    "Delete from History".to_owned(),
                ),
                // From YouTube Music only: nothing here to delete.
                None => Space::new().width(34).into(),
            };
            row![container(track).width(Length::Fill), delete]
                .spacing(4)
                .align_y(Alignment::Center)
                .into()
        });
        page.push(
            column![
                text(day.title.clone())
                    .size(22)
                    .font(theme::weight(Weight::Bold)),
                column(rows.collect::<Vec<_>>()).spacing(2)
            ]
            .spacing(10)
            .into(),
        );
    }
    // YouTube Music's own history has no next page: only this app's plays load more.
    if history.has_more_local && feeds.query.trim().is_empty() {
        page.push(ui::pill_button(
            "Show older plays",
            Message::Feeds(Msg::More),
        ));
    }
    ui::scroll_page(page)
}

/// Swift `ActivityView`: new releases from followed artists, newest first.
/// A dot marks the ones not seen yet; `None` while the first load runs.
pub fn activity<'a>(
    covers: &'a Covers,
    items: Option<&'a [trop::activity::activity_store::ActivityItem]>,
) -> Element<'a, Message> {
    let mut page = vec![page_title("New releases")];
    match items {
        None => page.push(
            text("Checking the artists you follow…")
                .color(theme::SECONDARY)
                .into(),
        ),
        Some([]) => page.push(empty_note(
            "No New Releases",
            "Albums and singles from artists you follow, out in the last 60 days, appear here.",
        )),
        Some(items) => page.push(
            column(items.iter().map(|item| {
                let mut lines = column![
                    text(&item.message)
                        .size(14)
                        .font(theme::weight(Weight::Medium))
                ]
                .spacing(2)
                .width(Length::Fill);
                if let Some(time) = &item.time {
                    lines = lines.push(text(time).size(13).color(theme::SECONDARY));
                }
                let dot: Element<'a, Message> = if item.is_read {
                    Space::new().width(10).into()
                } else {
                    container(Space::new())
                        .width(10)
                        .height(10)
                        .style(|_| container::Style {
                            background: Some(Background::Color(theme::accent())),
                            border: Border::default().rounded(5),
                            ..container::Style::default()
                        })
                        .into()
                };
                let content = row![
                    ui::cover(covers, item.thumbnail.as_deref(), ui::ROW_COVER, 8.0),
                    lines,
                    dot
                ]
                .spacing(14)
                .align_y(Alignment::Center);
                let open = item
                    .album_id
                    .clone()
                    .map(|album| Message::Feeds(crate::feeds::Msg::Open(item.id.clone(), album)));
                ui::clickable(container(content).padding([6, 8]), open, 0.0)
            }))
            .spacing(2)
            .into(),
        ),
    }
    ui::scroll_page(page)
}

fn page_title(title: &str) -> Element<'_, Message> {
    text(title)
        .size(28)
        .font(theme::weight(Weight::Bold))
        .into()
}

/// What an empty list says: a title and one line under it.
fn empty_note<'a>(title: &'a str, line: &'a str) -> Element<'a, Message> {
    column![
        text(title).size(18).font(theme::weight(Weight::Bold)),
        text(line).size(14).color(theme::SECONDARY),
    ]
    .spacing(6)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_sub_genre_has_a_path() {
        let mood = |title: &str| MoodItem::new(title, None, None);
        assert!(mood_path(&mood("Brazilian Pop")).is_some());
        assert!(mood_path(&mood("Pop")).is_none());
        assert!(mood_path(&mood("Chill")).is_none());
    }
}
