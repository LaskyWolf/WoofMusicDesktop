//! The comments panel: `Trop/Audio/Comments/CommentsViewModel.swift` (state)
//! and `CommentsPanel.swift` (view). On desktop it sits at the right edge
//! like the queue instead of in place of the artwork.

use std::collections::HashMap;
use std::sync::Arc;

use iced::font::Weight;
use iced::widget::scrollable::Viewport;
use iced::widget::{
    Column, button, column, container, rich_text, row, scrollable, svg, text, text_input,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Task};
use swiftytube::youtube::comment_models::{
    CommentItem, CommentRepliesPage, CommentSort, CommentTextRun, CommentVoteState, CommentsPage,
};
use trop::services::comments_service::{CommentsError, CommentsService};

use crate::ui::{self, Covers};
use crate::{Message, theme, title_bar};

const WIDTH: f32 = 380.0;
const SCROLL: &str = "comments";
const AVATAR: f32 = 36.0;
/// Replies sit under the comment's text, past its avatar.
const REPLY_INDENT: f32 = AVATAR + 12.0;

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Loading,
    Loaded,
    Disabled,
    Failed(String),
}

/// The replies of one comment, opened under it.
#[derive(Default)]
struct Replies {
    items: Vec<CommentItem>,
    next: Option<String>,
    is_loading: bool,
    is_open: bool,
}

pub struct Comments {
    pub open: bool,
    /// Set when a like or a post did not go through; the app shows it as a notice.
    pub alert: Option<String>,
    phase: Phase,
    items: Vec<CommentItem>,
    total_count_text: Option<String>,
    sort: CommentSort,
    replies: HashMap<String, Replies>,
    is_posting: bool,
    draft: String,
    video_id: String,
    next_token: Option<String>,
    loading_more: bool,
    sort_tokens: HashMap<CommentSort, String>,
    create_params: Option<String>,
    /// Counts loads, so an answer for the previous song or order is dropped
    /// (Swift cancels the `.task`).
    load: u64,
}

impl Default for Comments {
    fn default() -> Self {
        Self {
            open: false,
            alert: None,
            phase: Phase::Loading,
            items: Vec::new(),
            total_count_text: None,
            sort: CommentSort::Top,
            replies: HashMap::new(),
            is_posting: false,
            draft: String::new(),
            video_id: String::new(),
            next_token: None,
            loading_more: false,
            sort_tokens: HashMap::new(),
            create_params: None,
            load: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded(u64, Result<CommentsPage, CommentsError>),
    Sort(CommentSort),
    Retry,
    Scrolled(Viewport),
    MoreLoaded(u64, String, Result<CommentsPage, CommentsError>),
    ToggleReplies(String),
    MoreReplies(String),
    RepliesLoaded(u64, String, Result<CommentRepliesPage, CommentsError>),
    Like(String),
    Voted(String, bool, Result<bool, CommentsError>),
    Draft(String),
    Send,
    Posted(Result<(), CommentsError>),
    OpenLink(String),
    Seek(f64),
}

fn msg(message: Msg) -> Message {
    Message::Comments(message)
}

impl Comments {
    /// Loads the comments of `video_id` when the panel shows another song's
    /// (Swift `.task(id: np.videoId)`).
    pub fn follow(
        &mut self,
        service: &Arc<CommentsService>,
        video_id: Option<&str>,
    ) -> Task<Message> {
        match video_id {
            Some(id) if self.open && id != self.video_id => self.load(service, id.to_owned()),
            _ => Task::none(),
        }
    }

    fn load(&mut self, service: &Arc<CommentsService>, video_id: String) -> Task<Message> {
        // Left over, they would post to the previous song or load its comments.
        *self = Comments {
            open: self.open,
            draft: std::mem::take(&mut self.draft),
            load: self.load + 1,
            video_id: video_id.clone(),
            ..Comments::default()
        };
        let load = self.load;
        let service = service.clone();
        Task::perform(
            async move { service.load_initial(&video_id).await },
            move |result| msg(Msg::Loaded(load, result)),
        )
    }

    fn load_sort(&mut self, service: &Arc<CommentsService>, sort: CommentSort) -> Task<Message> {
        let Some(token) = self.sort_tokens.get(&sort).cloned() else {
            return Task::none();
        };
        self.sort = sort;
        self.phase = Phase::Loading;
        self.items.clear();
        self.replies.clear();
        self.next_token = None;
        self.load += 1;
        let load = self.load;
        let service = service.clone();
        Task::perform(
            async move { service.load_page(&token).await },
            move |result| msg(Msg::Loaded(load, result)),
        )
    }

    pub fn update(&mut self, message: Msg, service: &Arc<CommentsService>) -> Task<Message> {
        match message {
            Msg::Loaded(load, result) => {
                if load != self.load {
                    return Task::none();
                }
                match result {
                    Ok(page) => self.apply(page),
                    Err(CommentsError::Disabled) => self.phase = Phase::Disabled,
                    Err(error) => self.phase = Phase::Failed(error.to_string()),
                }
                Task::none()
            }
            Msg::Sort(sort) if sort != self.sort => self.load_sort(service, sort),
            Msg::Sort(_) => Task::none(),
            Msg::Retry => {
                let video_id = self.video_id.clone();
                self.load(service, video_id)
            }
            // Swift loads the next page when the last comment appears.
            Msg::Scrolled(viewport) => {
                let near_end = viewport.relative_offset().y > 0.85
                    || viewport.content_bounds().height <= viewport.bounds().height;
                match self.next_token.clone() {
                    Some(token) if near_end && !self.loading_more => {
                        self.loading_more = true;
                        let load = self.load;
                        let service = service.clone();
                        Task::perform(
                            {
                                let token = token.clone();
                                async move { service.load_page(&token).await }
                            },
                            move |result| msg(Msg::MoreLoaded(load, token.clone(), result)),
                        )
                    }
                    _ => Task::none(),
                }
            }
            Msg::MoreLoaded(load, token, result) => {
                if load != self.load {
                    return Task::none();
                }
                self.loading_more = false;
                match result {
                    Ok(page) => {
                        let new: Vec<CommentItem> = page
                            .items
                            .into_iter()
                            .filter(|item| !self.items.iter().any(|old| old.id == item.id))
                            .collect();
                        self.items.extend(new);
                        self.next_token = page.next_page_token;
                    }
                    // Kept, so scrolling tries again.
                    Err(_) => self.next_token = Some(token),
                }
                Task::none()
            }
            Msg::ToggleReplies(id) => {
                let state = self.replies.entry(id.clone()).or_default();
                state.is_open = !state.is_open;
                let token = self
                    .items
                    .iter()
                    .find(|item| item.id == id)
                    .and_then(|item| item.reply_continuation.clone());
                match token {
                    Some(token) if state.is_open && state.items.is_empty() => {
                        self.load_replies(service, id, token)
                    }
                    _ => Task::none(),
                }
            }
            Msg::MoreReplies(id) => match self.replies.get(&id).and_then(|r| r.next.clone()) {
                Some(token) => self.load_replies(service, id, token),
                None => Task::none(),
            },
            Msg::RepliesLoaded(load, id, result) => {
                if load != self.load {
                    return Task::none();
                }
                let state = self.replies.entry(id).or_default();
                state.is_loading = false;
                match result {
                    Ok(page) => {
                        state.items.extend(page.items);
                        state.next = page.next_page_token;
                    }
                    Err(error) => self.alert = Some(error.to_string()),
                }
                Task::none()
            }
            Msg::Like(id) => {
                let Some(item) = self.find(&id) else {
                    return Task::none();
                };
                let liking = item.vote_state != CommentVoteState::Liked;
                let action = if liking {
                    item.like_action.clone()
                } else {
                    item.unlike_action.clone()
                };
                let Some(action) = action else {
                    self.alert = Some("Sign in to like comments.".into());
                    return Task::none();
                };
                self.change(&id, |item| set_liked(item, liking));
                let service = service.clone();
                Task::perform(async move { service.vote(&action).await }, move |result| {
                    msg(Msg::Voted(id.clone(), liking, result))
                })
            }
            Msg::Voted(id, liking, result) => {
                let error = match result {
                    Ok(true) => return Task::none(),
                    Ok(false) => "YouTube didn't accept the like.".to_owned(),
                    Err(error) => error.to_string(),
                };
                self.change(&id, |item| set_liked(item, !liking));
                self.alert = Some(error);
                Task::none()
            }
            Msg::Draft(draft) => {
                self.draft = draft;
                Task::none()
            }
            Msg::Send => {
                let text = self.draft.trim().to_owned();
                let Some(params) = self.create_params.clone() else {
                    return Task::none();
                };
                if text.is_empty() || self.is_posting {
                    return Task::none();
                }
                self.is_posting = true;
                let service = service.clone();
                Task::perform(
                    async move { service.post(&params, &text).await },
                    |result| msg(Msg::Posted(result)),
                )
            }
            Msg::Posted(result) => {
                self.is_posting = false;
                if let Err(error) = result {
                    // The draft stays, so nothing typed is lost.
                    self.alert = Some(error.to_string());
                    return Task::none();
                }
                self.draft.clear();
                // A new comment is the newest, so it shows at the top of that order.
                if self.sort_tokens.contains_key(&CommentSort::Newest) {
                    self.sort = CommentSort::Top;
                    self.load_sort(service, CommentSort::Newest)
                } else {
                    let video_id = self.video_id.clone();
                    self.load(service, video_id)
                }
            }
            Msg::OpenLink(url) => {
                open_link(&url);
                Task::none()
            }
            Msg::Seek(seconds) => Task::done(Message::SeekTo(seconds)),
        }
    }

    fn apply(&mut self, page: CommentsPage) {
        self.items = page.items;
        self.next_token = page.next_page_token;
        if !page.sort_tokens.is_empty() {
            self.sort_tokens = page.sort_tokens;
        }
        self.total_count_text = page.total_count_text.or(self.total_count_text.take());
        if page.create_comment_params.is_some() {
            self.create_params = page.create_comment_params;
        }
        self.phase = Phase::Loaded;
    }

    fn can_post(&self) -> bool {
        self.create_params.is_some()
    }

    fn load_replies(
        &mut self,
        service: &Arc<CommentsService>,
        id: String,
        token: String,
    ) -> Task<Message> {
        self.replies.entry(id.clone()).or_default().is_loading = true;
        let load = self.load;
        let service = service.clone();
        Task::perform(
            async move { service.load_replies(&token).await },
            move |result| msg(Msg::RepliesLoaded(load, id.clone(), result)),
        )
    }

    fn find(&self, id: &str) -> Option<&CommentItem> {
        self.items
            .iter()
            .chain(self.replies.values().flat_map(|r| r.items.iter()))
            .find(|item| item.id == id)
    }

    /// A comment or a reply, wherever it is.
    fn change(&mut self, id: &str, edit: impl FnOnce(&mut CommentItem)) {
        let item = self
            .items
            .iter_mut()
            .chain(self.replies.values_mut().flat_map(|r| r.items.iter_mut()))
            .find(|item| item.id == id);
        if let Some(item) = item {
            edit(item);
        }
    }

    /// The avatars to download.
    pub fn avatar_urls(&self) -> Vec<String> {
        self.items
            .iter()
            .chain(self.replies.values().flat_map(|r| r.items.iter()))
            .filter_map(|item| item.author.avatar_url.clone())
            .collect()
    }

    pub fn view<'a>(&'a self, covers: &'a Covers, signed_in: bool) -> Element<'a, Message> {
        let mut header = row![
            column![text("Comments").size(18).font(theme::weight(Weight::Bold))]
                .push(
                    self.total_count_text
                        .as_deref()
                        .map(|count| text(count).size(12).color(theme::SECONDARY)),
                )
                .width(Length::Fill),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        if self.phase == Phase::Loaded && !self.sort_tokens.is_empty() {
            for sort in CommentSort::ALL_CASES {
                header = header.push(chip(sort.title(), self.sort == sort, msg(Msg::Sort(sort))));
            }
        }
        let header = header
            .push(icon_button(
                title_bar::icons::CLOSE,
                16.0,
                Message::ToggleComments,
            ))
            .padding(Padding {
                top: 14.0,
                right: 10.0,
                bottom: 8.0,
                left: 18.0,
            });

        let content: Element<'a, Message> = match &self.phase {
            Phase::Loading => message("Loading…", None),
            Phase::Disabled => message(
                "Comments Are Off",
                Some("YouTube has no comments for this song.".into()),
            ),
            Phase::Failed(error) => container(
                column![
                    text("Couldn't Load")
                        .size(16)
                        .font(theme::weight(Weight::Bold)),
                    text(error.as_str()).size(13).color(theme::SECONDARY),
                    ui::pill_button("Try Again", msg(Msg::Retry)),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .padding(20)
            .into(),
            Phase::Loaded if self.items.is_empty() => message(
                "No Comments Yet",
                self.can_post().then(|| "Be the first to write one.".into()),
            ),
            Phase::Loaded => self.list(covers),
        };

        let mut panel = column![header, content];
        if self.can_post() && signed_in {
            panel = panel.push(self.composer());
        }
        container(ui::glass_panel(
            5,
            22.0,
            Length::Fixed(WIDTH),
            Length::Fill,
            panel,
        ))
        .padding(Padding {
            top: title_bar::HEIGHT,
            right: theme::INSET,
            bottom: theme::PLAYER_HEIGHT + 2.0 * theme::INSET,
            left: 0.0,
        })
        .align_right(Length::Fill)
        .into()
    }

    fn list<'a>(&'a self, covers: &'a Covers) -> Element<'a, Message> {
        let mut list = Column::new().spacing(18).padding(Padding {
            top: 6.0,
            right: 18.0,
            bottom: 18.0,
            left: 18.0,
        });
        for item in &self.items {
            list = list.push(comment_row(item, covers, self.replies.get(&item.id)));
            let Some(replies) = self.replies.get(&item.id).filter(|r| r.is_open) else {
                continue;
            };
            let mut thread = Column::new().spacing(14);
            for reply in &replies.items {
                thread = thread.push(comment_row(reply, covers, None));
            }
            if replies.is_loading {
                thread = thread.push(text("Loading…").size(12).color(theme::SECONDARY));
            } else if replies.next.is_some() {
                thread = thread.push(link_button(
                    "Show More Replies",
                    msg(Msg::MoreReplies(item.id.clone())),
                ));
            }
            list = list.push(container(thread).padding(Padding::default().left(REPLY_INDENT)));
        }
        if self.loading_more {
            list = list.push(
                container(text("Loading…").size(12).color(theme::SECONDARY)).center_x(Length::Fill),
            );
        }
        scrollable(list)
            .id(SCROLL)
            .on_scroll(|viewport| msg(Msg::Scrolled(viewport)))
            .direction(theme::scrollbar())
            .height(Length::Fill)
            .into()
    }

    fn composer(&self) -> Element<'_, Message> {
        let ready = !self.draft.trim().is_empty() && !self.is_posting;
        let field = text_input("Add a comment", &self.draft)
            .on_input(|draft| msg(Msg::Draft(draft)))
            .on_submit_maybe(ready.then(|| msg(Msg::Send)))
            .padding([10, 14])
            .size(14)
            .style(|_, _| text_input::Style {
                background: Background::Color(theme::HOVER),
                border: Border::default().rounded(18),
                icon: theme::SECONDARY,
                placeholder: theme::SECONDARY,
                value: theme::TEXT,
                selection: theme::accent(),
            });
        let send = button(ui::icon(icons::SEND, 18.0).opacity(if ready { 1.0_f32 } else { 0.4 }))
            .padding(9)
            .on_press_maybe(ready.then(|| msg(Msg::Send)))
            .style(|_, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered => theme::PRESSED,
                    _ => theme::HOVER,
                })),
                border: Border::default().rounded(18),
                ..button::Style::default()
            });
        row![field, send]
            .spacing(8)
            .align_y(Alignment::Center)
            .padding([10, 14])
            .into()
    }
}

/// The vote and a count that follows it. A count in "1.2K" form stays as it is.
fn set_liked(item: &mut CommentItem, liked: bool) {
    item.vote_state = if liked {
        CommentVoteState::Liked
    } else {
        CommentVoteState::Indifferent
    };
    if let Ok(count) = item.like_count_text.parse::<i64>() {
        item.like_count_text = (count + if liked { 1 } else { -1 }).max(0).to_string();
    } else if item.like_count_text.is_empty() && liked {
        item.like_count_text = "1".into();
    }
}

fn comment_row<'a>(
    item: &'a CommentItem,
    covers: &'a Covers,
    replies: Option<&Replies>,
) -> Element<'a, Message> {
    let mut body = Column::new().spacing(5).width(Length::Fill);
    if item.is_pinned {
        body = body.push(
            row![
                small_icon(icons::PIN, 12.0, theme::SECONDARY),
                text(item.pinned_text.as_deref().unwrap_or("Pinned"))
                    .size(11)
                    .font(theme::weight(Weight::Semibold))
                    .color(theme::SECONDARY),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        );
    }
    let mut byline = row![
        text(item.author.name.as_str())
            .size(12)
            .font(theme::weight(Weight::Medium))
            .color(theme::SECONDARY)
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    if item.author.is_creator || item.author.is_artist {
        byline = byline.push(small_icon(
            crate::song_menu::icons::CHECK,
            12.0,
            theme::SECONDARY,
        ));
    }
    if !item.published_time.is_empty() {
        byline = byline.push(
            text(format!("· {}", item.published_time))
                .size(12)
                .color(theme::SECONDARY),
        );
    }
    body = body.push(byline).push(words(item));

    let liked = item.vote_state == CommentVoteState::Liked;
    let mut actions = row![
        button(
            row![
                small_icon(
                    if liked {
                        icons::THUMB_FILL
                    } else {
                        icons::THUMB
                    },
                    15.0,
                    if liked {
                        theme::accent()
                    } else {
                        theme::SECONDARY
                    },
                ),
                text(item.like_count_text.as_str())
                    .size(12)
                    .color(if liked {
                        theme::accent()
                    } else {
                        theme::SECONDARY
                    }),
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .padding([4, 6])
        .on_press(msg(Msg::Like(item.id.clone())))
        .style(ghost),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    if item.reply_continuation.is_some() {
        let open = replies.is_some_and(|r| r.is_open);
        let label = if item.reply_count_text.is_empty() {
            "Replies".to_owned()
        } else if item.reply_count == 1 {
            "1 reply".to_owned()
        } else {
            format!("{} replies", item.reply_count_text)
        };
        actions = actions.push(
            button(
                row![
                    text(label)
                        .size(12)
                        .font(theme::weight(Weight::Medium))
                        .color(theme::accent()),
                    small_icon(
                        if open {
                            icons::CHEVRON_UP
                        } else {
                            icons::CHEVRON_DOWN
                        },
                        12.0,
                        theme::accent(),
                    ),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            )
            .padding([4, 6])
            .on_press(msg(Msg::ToggleReplies(item.id.clone())))
            .style(ghost),
        );
    }
    if item.is_hearted {
        actions = actions.push(small_icon(
            crate::song_menu::icons::HEART_FILL,
            14.0,
            // Red like Swift's heart.
            theme::CLOSE,
        ));
    }
    body = body.push(actions);

    row![
        ui::cover(
            covers,
            item.author.avatar_url.as_deref(),
            AVATAR,
            AVATAR / 2.0
        ),
        body,
    ]
    .spacing(12)
    .into()
}

/// The comment's words, with links and timestamps that can be clicked.
fn words(item: &CommentItem) -> Element<'_, Message> {
    let spans: Vec<text::Span<'_, Msg>> = item
        .runs
        .iter()
        .map(|run| match run {
            CommentTextRun::Text(text) => text::Span::new(text.as_str()),
            CommentTextRun::Link { text, url } => text::Span::new(text.as_str())
                .color(theme::accent())
                .link(Msg::OpenLink(url.clone())),
            CommentTextRun::Timestamp { text, seconds, .. } => text::Span::new(text.as_str())
                .color(theme::accent())
                .link(Msg::Seek(*seconds)),
        })
        .collect();
    rich_text(spans)
        .on_link_click(msg)
        .size(14)
        .line_height(1.35)
        .width(Length::Fill)
        .into()
}

fn message<'a>(title: &'a str, detail: Option<String>) -> Element<'a, Message> {
    container(
        column![text(title).size(16).font(theme::weight(Weight::Bold))]
            .push(detail.map(|detail| text(detail).size(13).color(theme::SECONDARY)))
            .spacing(6)
            .align_x(Alignment::Center),
    )
    .center(Length::Fill)
    .padding(20)
    .into()
}

fn chip<'a>(label: &'a str, selected: bool, on_press: Message) -> Element<'a, Message> {
    button(text(label).size(12).font(theme::weight(Weight::Medium)))
        .padding([5, 10])
        .on_press(on_press)
        .style(move |_, status| button::Style {
            background: Some(Background::Color(if selected {
                theme::TEXT
            } else if status == button::Status::Hovered {
                theme::PRESSED
            } else {
                theme::HOVER
            })),
            text_color: if selected {
                theme::BACKGROUND
            } else {
                theme::TEXT
            },
            border: Border::default().rounded(14),
            ..button::Style::default()
        })
        .into()
}

fn icon_button<'a>(glyph: &'static [u8], size: f32, on_press: Message) -> Element<'a, Message> {
    button(ui::icon(glyph, size))
        .padding(10)
        .on_press(on_press)
        .style(|_, status| button::Style {
            background: (status == button::Status::Hovered)
                .then_some(Background::Color(theme::HOVER)),
            border: Border::default().rounded(20),
            ..button::Style::default()
        })
        .into()
}

fn link_button<'a>(label: &'a str, on_press: Message) -> Element<'a, Message> {
    button(
        text(label)
            .size(12)
            .font(theme::weight(Weight::Medium))
            .color(theme::accent()),
    )
    .padding([4, 6])
    .on_press(on_press)
    .style(ghost)
    .into()
}

fn ghost(_: &iced::Theme, status: button::Status) -> button::Style {
    button::Style {
        background: (status == button::Status::Hovered).then_some(Background::Color(theme::HOVER)),
        border: Border::default().rounded(8),
        ..button::Style::default()
    }
}

fn small_icon<'a>(glyph: &'static [u8], size: f32, color: Color) -> Element<'a, Message> {
    ui::icon(glyph, size)
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

/// Opens a web link from a comment in the browser. Only http(s): a comment
/// is anyone's text.
pub(crate) fn open_link(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(error) = result {
        log::warn!("Could not open {url}: {error}");
    }
}

pub mod icons {
    /// `text.bubble`: a speech bubble with lines of text.
    pub const COMMENTS: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 4.5h12A2.5 2.5 0 0 1 20.5 7v8a2.5 2.5 0 0 1-2.5 2.5h-6.5l-4.6 3.2a.5.5 0 0 1-.8-.4v-2.8H6A2.5 2.5 0 0 1 3.5 15V7A2.5 2.5 0 0 1 6 4.5z"/><path d="M7.5 8.6h9M7.5 11.2h9M7.5 13.8h5.5"/></g></svg>"##;
    pub const THUMB: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M7.5 10.5v9h-3a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1zm0 0 3.8-6.3a1.6 1.6 0 0 1 2.9 1.2l-.8 3.6h5a2 2 0 0 1 2 2.4l-1.3 6.4a2 2 0 0 1-2 1.7H7.5" fill="none" stroke="#fff" stroke-width="1.9" stroke-linejoin="round"/></svg>"##;
    pub const THUMB_FILL: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M7.5 10.5v9h-3a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1zm0 0 3.8-6.3a1.6 1.6 0 0 1 2.9 1.2l-.8 3.6h5a2 2 0 0 1 2 2.4l-1.3 6.4a2 2 0 0 1-2 1.7H7.5" fill="#fff" stroke="#fff" stroke-width="1.9" stroke-linejoin="round"/></svg>"##;
    pub const PIN: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M9 3.5h6l-1 6 3.5 3.5v1.5h-11V13L10 9.5zM12 14.5v6" fill="#fff" stroke="#fff" stroke-width="1.8" stroke-linejoin="round" stroke-linecap="round"/></svg>"##;
    pub const CHEVRON_DOWN: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 9.5l6 6 6-6" fill="none" stroke="#fff" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
    pub const CHEVRON_UP: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 14.5l6-6 6 6" fill="none" stroke="#fff" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
    pub const SEND: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 19V5M6 11l6-6 6 6" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
}
