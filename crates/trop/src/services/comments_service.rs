//! `Trop/Services/CommentsService.swift`: reads and writes a song's YouTube
//! comments through InnerTube `/next` as the web client. The music client
//! (WEB_REMIX) does not return the comments section. The music host answers
//! first and www.youtube.com is the fallback, as for every comment call.

use std::sync::Arc;

use serde_json::Value;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::core::youtube_client::YouTubeClient;
use swiftytube::networking::inner_tube_client::{InnerTubeClient, NextArgs};
use swiftytube::networking::inner_tube_decode::{JsonDict, as_dict, as_dict_list, as_string};
use swiftytube::networking::inner_tube_error::InnerTubeError;
use swiftytube::youtube::comment_models::{CommentRepliesPage, CommentsPage};
use swiftytube::youtube::comments_parser::CommentsParser;

use crate::activity::activity_store::ActivityJson;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum CommentsError {
    #[error("Comments are off for this song")]
    Disabled,
    #[error("{0}")]
    Rejected(String),
    /// Swift rethrows the request error as is.
    #[error("{0}")]
    Request(String),
}

impl From<InnerTubeError> for CommentsError {
    fn from(error: InnerTubeError) -> Self {
        CommentsError::Request(error.to_string())
    }
}

pub struct CommentsService {
    music: Arc<InnerTubeClient>,
    youtube: InnerTubeClient,
}

impl CommentsService {
    /// `music` is the app's client: it holds the signed-in session.
    pub fn new(music: Arc<InnerTubeClient>) -> Self {
        let youtube = InnerTubeClient::new(SwiftyTubeConfig::youtube(), music.current_auth());
        Self { music, youtube }
    }

    pub async fn load_initial(&self, video_id: &str) -> Result<CommentsPage, CommentsError> {
        let token = self
            .initial_token(video_id)
            .await
            .ok_or(CommentsError::Disabled)?;
        let page = self.load_page(&token).await?;
        if page.items.is_empty() && page.create_comment_params.is_none() {
            return Err(CommentsError::Disabled);
        }
        Ok(page)
    }

    pub async fn load_page(&self, continuation: &str) -> Result<CommentsPage, CommentsError> {
        Ok(CommentsParser::parse_comments_page(
            &self.next(continuation).await?,
        ))
    }

    pub async fn load_replies(
        &self,
        continuation: &str,
    ) -> Result<CommentRepliesPage, CommentsError> {
        Ok(CommentsParser::parse_replies_page(
            &self.next(continuation).await?,
        ))
    }

    /// Replays a like or unlike token from the comment. True when YouTube accepted it.
    pub async fn vote(&self, action: &str) -> Result<bool, CommentsError> {
        let json = self.with_fallback(Call::Vote(action)).await?;
        let first = as_dict_list(json.get("actionResults")).and_then(|r| r.into_iter().next());
        Ok(first.and_then(|r| as_string(r.get("status"))) == Some("STATUS_SUCCEEDED"))
    }

    pub async fn post(&self, params: &str, text: &str) -> Result<(), CommentsError> {
        let json = self.with_fallback(Call::Post(params, text)).await?;
        match failure_message(&json) {
            Some(message) => Err(CommentsError::Rejected(message)),
            None => Ok(()),
        }
    }

    // MARK: - Private

    async fn initial_token(&self, video_id: &str) -> Option<String> {
        self.sync_youtube();
        for client in [&*self.music, &self.youtube] {
            let args = NextArgs {
                video_id: Some(video_id.to_owned()),
                client: Some(YouTubeClient::WEB),
                ..NextArgs::default()
            };
            if let Ok(json) = client.next(args).await
                && let Some(token) = CommentsParser::find_initial_comments_token(&json)
            {
                return Some(token);
            }
        }
        None
    }

    async fn next(&self, continuation: &str) -> Result<JsonDict, CommentsError> {
        self.with_fallback(Call::Next(continuation)).await
    }

    async fn with_fallback(&self, call: Call<'_>) -> Result<JsonDict, CommentsError> {
        match call.run(&self.music).await {
            Ok(json) => Ok(json),
            // After a timeout the request may have arrived: retrying could post a comment twice.
            Err(InnerTubeError::Transport(error)) if error.is_timeout() => {
                Err(CommentsError::Request(error.to_string()))
            }
            Err(_) => {
                self.sync_youtube();
                Ok(call.run(&self.youtube).await?)
            }
        }
    }

    /// Swift loads the cookies and locale into each client before a call; the
    /// app keeps the music client's current, so the fallback copies them.
    fn sync_youtube(&self) {
        self.youtube.update_auth(self.music.current_auth());
        self.youtube
            .update_locale(self.music.config().default_locale);
    }
}

/// The calls that fall back to www.youtube.com. Swift passes a closure; an
/// enum keeps the future `Send` for the app's tasks.
#[derive(Clone, Copy)]
enum Call<'a> {
    Next(&'a str),
    Vote(&'a str),
    Post(&'a str, &'a str),
}

impl Call<'_> {
    async fn run(self, client: &InnerTubeClient) -> Result<JsonDict, InnerTubeError> {
        let web = Some(YouTubeClient::WEB);
        match self {
            Call::Next(continuation) => {
                client
                    .next(NextArgs {
                        continuation: Some(continuation.to_owned()),
                        client: web,
                        ..NextArgs::default()
                    })
                    .await
            }
            Call::Vote(action) => client.perform_comment_action(action, web, None).await,
            Call::Post(params, text) => client.create_comment(params, text, web, None).await,
        }
    }
}

/// The text of the error YouTube answers with a 200, such as a refused comment.
fn failure_message(json: &JsonDict) -> Option<String> {
    if let Some(message) = as_dict(json.get("error")).and_then(|e| as_string(e.get("message"))) {
        return Some(message.to_owned());
    }
    let action = as_dict_list(json.get("actions"))
        .and_then(|actions| actions.into_iter().next())
        .and_then(|first| as_dict(first.get("showErrorAction")));
    let text: Option<&Value> = action
        .and_then(|a| as_dict(a.get("errorMessage")))
        .and_then(|m| as_dict(m.get("messageRenderer")))
        .and_then(|r| r.get("text"));
    ActivityJson::text(text)
}
