//! The pages: Home, Explore, Search, album, artist, playlist, podcast, mood, Library, Liked Songs, My Top 100.

use std::sync::Arc;

use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Gradient, Length, Radians};
use trop::detail::detail_models::{
    AlbumDetailInfo, ArtistDetailInfo, PlaylistDetailInfo, PodcastDetailInfo,
};
use trop::networking::models::home_page::{
    ExploreSection, ExploreSectionKind, HomeSection, MoodItem,
};
use trop::networking::models::yt_item::{SongItem, YTItem};
use trop::search::search_parser::SearchSection;

use crate::playlist_edit;
use crate::services::Library;
use crate::ui::{self, Covers};
use crate::{Load, Message, Route, theme};

pub const SEARCH_INPUT: &str = "search";

pub fn home<'a>(
    covers: &'a Covers,
    home: &'a Load<Vec<HomeSection>>,
    playing: Option<&str>,
) -> Element<'a, Message> {
    let sections = match home {
        Load::Loaded(sections) => sections,
        Load::Loading => return ui::status(None, Message::Retry),
        Load::Failed(error) => return ui::status(Some(error), Message::Retry),
    };
    ui::scroll_page(
        sections
            .iter()
            .filter(|section| !section.items().is_empty())
            .map(|section| {
                let limit = if section.is_songs_only() { 12 } else { 10 };
                ui::section(
                    covers,
                    section.display_title(),
                    section.items(),
                    limit,
                    playing,
                )
            })
            .collect(),
    )
}

/// Explore feed, or a mood's page: shelves of rows and cards, mood chips.
pub fn explore<'a>(
    covers: &'a Covers,
    sections: &'a [ExploreSection],
    playing: Option<&str>,
) -> Element<'a, Message> {
    ui::scroll_page(
        sections
            .iter()
            .filter(|section| !section.items.is_empty() || !section.moods.is_empty())
            .map(|section| match section.kind {
                ExploreSectionKind::Moods => moods(&section.title, &section.moods),
                ExploreSectionKind::Rows => {
                    ui::section(covers, &section.title, &section.items, 12, playing)
                }
                ExploreSectionKind::Cards => {
                    ui::section(covers, &section.title, &section.items, 10, playing)
                }
            })
            .collect(),
    )
}

/// Mood chips: one line each, so every chip in a row has the same height.
fn moods<'a>(title: &'a str, moods: &'a [MoodItem]) -> Element<'a, Message> {
    const CHIP: f32 = 228.0;
    // Chip width minus padding, swatch and gap.
    const LABEL: f32 = CHIP - 2.0 * 14.0 - 4.0 - 12.0;
    let chips = moods.iter().map(|mood| {
        let color = mood
            .color
            .map(|rgb| Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
            .unwrap_or(theme::ACCENT);
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
        .on_press(Message::Navigate(Route::Mood(mood.clone())))
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

/// Search field and results. Results are `None` before the first search.
pub fn search<'a>(
    covers: &'a Covers,
    query: &'a str,
    results: Option<&'a Load<Vec<SearchSection>>>,
    playing: Option<&str>,
) -> Element<'a, Message> {
    let field = text_input("Songs, albums, artists, playlists, podcasts", query)
        .id(SEARCH_INPUT)
        .on_input(Message::SearchInput)
        .on_submit(Message::SearchSubmit)
        .size(16)
        .padding([12, 16])
        .style(ui::field_style);
    let mut page = vec![container(field).max_width(640).into()];
    match results {
        None => {}
        Some(Load::Loading) => page.push(text("Searching…").color(theme::SECONDARY).into()),
        Some(Load::Failed(error)) => page.push(
            text(format!("Search failed: {error}. Press Enter to try again."))
                .color(theme::SECONDARY)
                .into(),
        ),
        Some(Load::Loaded(sections)) if sections.is_empty() => {
            page.push(text("No results").color(theme::SECONDARY).into());
        }
        Some(Load::Loaded(sections)) => page.extend(
            sections
                .iter()
                .filter(|section| !section.items.is_empty())
                .map(|section| ui::section(covers, &section.title, &section.items, 12, playing)),
        ),
    }
    ui::scroll_page(page)
}

pub fn album<'a>(
    covers: &'a Covers,
    album: &'a AlbumDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    playing: Option<&str>,
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
    let mut page = vec![ui::header(
        covers,
        album.thumbnail_url.as_deref(),
        false,
        &album.title,
        vec![line],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
    )];
    page.push(tracks(covers, songs, false, playing));
    ui::scroll_page(page)
}

pub fn playlist<'a>(
    covers: &'a Covers,
    playlist: &'a PlaylistDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    playing: Option<&str>,
    edit: &'a playlist_edit::State,
    in_library: bool,
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
    let mut page = vec![ui::header(
        covers,
        playlist.thumbnail_url.as_deref(),
        false,
        &playlist.title,
        vec![line, description],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
    )];
    page.push(playlist_edit::actions(covers, edit, playlist, in_library));
    page.push(if edit.editing(&playlist.playlist_id) {
        playlist_edit::edit_tracks(covers, edit, playlist, songs, playing)
    } else {
        tracks(covers, songs, true, playing)
    });
    ui::scroll_page(page)
}

pub fn artist<'a>(
    covers: &'a Covers,
    artist: &'a ArtistDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    related: &'a [YTItem],
    playing: Option<&str>,
) -> Element<'a, Message> {
    let mut page = vec![ui::header(
        covers,
        artist.thumbnail_url.as_deref(),
        true,
        &artist.name,
        vec![
            artist
                .subscriber_count_text
                .as_deref()
                .map(subscriber_label)
                .unwrap_or_default(),
        ],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
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
) -> Element<'a, Message> {
    let mut page = vec![ui::header(
        covers,
        podcast.thumbnail_url.as_deref(),
        false,
        &podcast.title,
        vec![
            podcast.author.clone().unwrap_or_default(),
            count(podcast.episodes.len() as i64, "episode"),
        ],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
    )];
    if let Some(about) = podcast
        .description_text
        .as_deref()
        .filter(|d| !d.is_empty())
    {
        page.push(
            container(text(about).size(14).color(theme::SECONDARY))
                .max_width(720)
                .into(),
        );
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
        ui::track_row(covers, songs, index, true, is_playing, subtitle, None)
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
    let mut page = vec![column![chips, shortcuts].spacing(20).into()];
    let sections = [
        (LibraryFilter::Playlists, "Playlists", &library.playlists),
        (LibraryFilter::Albums, "Albums", &library.albums),
        (LibraryFilter::Artists, "Artists", &library.artists),
        (LibraryFilter::Podcasts, "Podcasts", &library.podcasts),
    ];
    for (kind, title, items) in sections {
        if filter.is_none_or(|filter| filter == kind) && !items.is_empty() {
            page.push(library_section(covers, title, items));
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
) -> Element<'a, Message> {
    let no_songs = Arc::new(Vec::new());
    let tiles = items
        .iter()
        .map(|(item, subtitle)| ui::tile(covers, item, subtitle.clone(), &no_songs, false));
    column![
        row![
            ui::section_title(title),
            Space::new().width(Length::Fill),
            text(items.len()).size(15).color(theme::SECONDARY),
        ]
        .align_y(Alignment::Center),
        iced::widget::Row::with_children(tiles)
            .spacing(20)
            .wrap()
            .vertical_spacing(24),
    ]
    .spacing(14)
    .into()
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
