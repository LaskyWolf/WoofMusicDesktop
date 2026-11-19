//! `YouTube/CommentModels.swift`.

use std::collections::HashMap;

/// Sort order for a YouTube comments section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CommentSort {
    #[default]
    Top,
    Newest,
}

impl CommentSort {
    /// Swift `CaseIterable.allCases`.
    pub const ALL_CASES: [CommentSort; 2] = [CommentSort::Top, CommentSort::Newest];

    pub fn title(self) -> &'static str {
        match self {
            CommentSort::Top => "Top",
            CommentSort::Newest => "Newest",
        }
    }
}

/// A run of text inside a comment body.
#[derive(Clone, Debug, PartialEq)]
pub enum CommentTextRun {
    Text(String),
    /// Swift keeps a `URL`; the string is already absolute.
    Link {
        text: String,
        url: String,
    },
    Timestamp {
        text: String,
        seconds: f64,
        video_id: Option<String>,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommentAuthor {
    pub name: String,
    pub avatar_url: Option<String>,
    pub channel_url: Option<String>,
    pub is_creator: bool,
    pub is_verified: bool,
    pub is_artist: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommentVoteState {
    #[default]
    Indifferent,
    Liked,
    Disliked,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommentItem {
    pub id: String,
    pub author: CommentAuthor,
    pub runs: Vec<CommentTextRun>,
    pub published_time: String,
    pub like_count_text: String,
    pub is_hearted: bool,
    pub is_pinned: bool,
    pub pinned_text: Option<String>,
    pub reply_count: i64,
    pub reply_count_text: String,
    pub reply_continuation: Option<String>,
    /// Viewer's current vote. Only meaningful together with the action tokens.
    pub vote_state: CommentVoteState,
    /// Server-issued `performCommentActionEndpoint` tokens for the viewer's
    /// toolbar surface. All None when signed out: the viewer must sign in
    /// before any vote can be performed.
    pub like_action: Option<String>,
    pub unlike_action: Option<String>,
    pub dislike_action: Option<String>,
    pub undislike_action: Option<String>,
    pub delete_action: Option<String>,
    pub edit_params: Option<String>,
    // ponytail: `reportParams` and `CommentsParser+Report.swift` are not
    // ported; WoofMusic has no Report item. Port both when a report menu lands.
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommentsPage {
    pub items: Vec<CommentItem>,
    pub next_page_token: Option<String>,
    pub sort_tokens: HashMap<CommentSort, String>,
    pub total_count_text: Option<String>,
    pub create_comment_params: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommentRepliesPage {
    pub items: Vec<CommentItem>,
    pub next_page_token: Option<String>,
}
