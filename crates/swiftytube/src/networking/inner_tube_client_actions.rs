// Account, library and queue endpoints. Parsing stays in apps.
// Swift `extension InnerTubeClient` → a second `impl InnerTubeClient` block.
// `verifySession` does not dump the menu JSON to disk (`dumpDiagnosticJSON` is not ported).

use std::collections::HashSet;

use serde_json::{Value, json};

use super::http_client::HttpClient;
use super::inner_tube_client::{BrowseArgs, InnerTubeClient, PlayerArgs, insert_some};
use super::inner_tube_decode::{InnerTubeDecode, JsonDict, as_dict, as_dict_list};
use super::inner_tube_error::InnerTubeError;
use super::inner_tube_json::InnerTubeJSON;
use super::request_builder::RequestBuilder;
use super::retry_policy::RetryPolicy;
use crate::auth::account_info::AccountInfo;
use crate::auth::auth_state::AuthState;
use crate::auth::auth_state_provider::AuthStateProvider;
use crate::core::swiftytube_log::SwiftyTubeLog;
use crate::core::youtube_client::YouTubeClient;
use crate::core::youtube_locale::YouTubeLocale;
use crate::core::youtube_service::YouTubeService;
use crate::streaming::duration_cache::DurationCache;
use crate::streaming::player_response::PlayerResponse;
use crate::sync::browse_lens::BrowseLens;

impl InnerTubeClient {
    // MARK: - Auth state

    /// Loads persisted session fields from the app's store.
    pub async fn load_state(&self, store: &impl AuthStateProvider) {
        let cookies = store.cookies().await;
        let sapisid = store.sapisid().await;
        let visitor_data = store.visitor_data().await;
        let data_sync_id = store.data_sync_id().await;
        let yes_no = |present: bool| if present { "yes" } else { "no" };
        SwiftyTubeLog::debug(&format!(
            "[Auth] loadState: cookies={} sapisid={} visitorData={} dataSyncId={}",
            cookies.len(),
            yes_no(sapisid.is_some()),
            yes_no(visitor_data.is_some()),
            yes_no(data_sync_id.is_some()),
        ));
        self.update_auth(AuthState {
            cookies,
            sapisid,
            visitor_data,
            data_sync_id,
        });
    }

    /// Ensures visitorData is set, fetching it via the home tab if needed.
    pub async fn ensure_visitor_data(
        &self,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<(), InnerTubeError> {
        if self.current_auth().visitor_data.is_some() {
            return Ok(());
        }
        let home_id = match self.config().service {
            YouTubeService::Music => "FEmusic_home",
            YouTubeService::Youtube => "FEwhat_to_watch",
        };
        self.browse(BrowseArgs {
            client,
            locale,
            ..BrowseArgs::id(home_id)
        })
        .await?;
        Ok(())
    }

    pub async fn fetch_duration(&self, video_id: &str) -> Result<i64, InnerTubeError> {
        let response: PlayerResponse = self
            .player_response(PlayerArgs {
                video_id: video_id.to_string(),
                client: Some(YouTubeClient::IOS),
                ..PlayerArgs::default()
            })
            .await?;
        let duration = response
            .video_details
            .and_then(|d| d.length_seconds)
            .and_then(|s| s.parse().ok())
            .ok_or(InnerTubeError::DecodingFailed)?;
        DurationCache::set(video_id, duration);
        Ok(duration)
    }

    /// Resolves a channel avatar when a feed/search renderer omits it.
    /// Search lockups can contain the channel name but neither channel ID nor
    /// avatar; `/player` provides the ID and the channel header provides the
    /// canonical avatar image.
    /// `avatar_url` stands in for `YouTubeChannelParser.avatarURL(in:)` until the YouTube/ port.
    pub async fn resolve_channel_avatar(
        &self,
        video_id: &str,
        avatar_url: impl Fn(&JsonDict) -> Option<String>,
    ) -> Option<String> {
        let player_json = match self
            .player(PlayerArgs {
                video_id: video_id.to_string(),
                client: Some(YouTubeClient::IOS),
                ..PlayerArgs::default()
            })
            .await
        {
            Ok(json) => json,
            Err(error) => {
                SwiftyTubeLog::debug(&format!(
                    "[AvatarResolve] failed videoId={video_id} error={error}"
                ));
                return None;
            }
        };
        let Some(channel_id) =
            InnerTubeDecode::string(&["videoDetails", "channelId"], &player_json)
                .filter(|id| id.starts_with("UC"))
        else {
            SwiftyTubeLog::debug(&format!("[AvatarResolve] no channelId videoId={video_id}"));
            return None;
        };
        match self
            .browse(BrowseArgs {
                client: Some(YouTubeClient::WEB),
                ..BrowseArgs::id(channel_id)
            })
            .await
        {
            Ok(channel_json) => avatar_url(&channel_json),
            Err(error) => {
                SwiftyTubeLog::debug(&format!(
                    "[AvatarResolve] failed videoId={video_id} error={error}"
                ));
                None
            }
        }
    }

    /// Cached duration, else fetch. None while another task fetches.
    pub async fn resolve_duration(&self, video_id: &str) -> Option<i64> {
        if let Some(cached) = DurationCache::get(video_id).filter(|d| *d > 0) {
            return Some(cached);
        }
        if DurationCache::is_pending(video_id) {
            return None;
        }
        DurationCache::mark_pending(video_id);
        match self.fetch_duration(video_id).await {
            Ok(duration) => Some(duration),
            Err(_) => {
                DurationCache::clear_pending(video_id);
                None
            }
        }
    }

    // MARK: - Pagination

    /// All pages via continuations. Fails on repeats/cap instead of
    /// returning partial data: callers sweep remote state against the result,
    /// so a truncated page must fail the section, not corrupt the library.
    /// `args` carries `browseId`, `params`, `client` and `locale` of the first page.
    pub async fn paginate<T>(
        &self,
        args: BrowseArgs,
        max_pages: usize,
        parse: impl Fn(&JsonDict) -> Vec<T>,
        continuation: impl Fn(&JsonDict) -> Option<String>,
    ) -> Result<Vec<T>, InnerTubeError> {
        let browse_id = args.browse_id.clone().unwrap_or_default();
        let mut all_items = Vec::new();
        let mut token: Option<String> = None;
        let mut seen_tokens = HashSet::new();
        let mut pages = 0;
        loop {
            let json = match token.take() {
                Some(token) => {
                    self.browse(BrowseArgs {
                        continuation: Some(token),
                        ..BrowseArgs::default()
                    })
                    .await?
                }
                None => {
                    self.browse(BrowseArgs {
                        browse_id: args.browse_id.clone(),
                        params: args.params.clone(),
                        client: args.client,
                        locale: args.locale.clone(),
                        ..BrowseArgs::default()
                    })
                    .await?
                }
            };
            all_items.extend(parse(&json));
            token = continuation(&json);
            pages += 1;
            if let Some(token) = &token
                && !seen_tokens.insert(token.clone())
            {
                return Err(InnerTubeError::TruncatedResponse(format!(
                    "paginate {browse_id}: repeated continuation token"
                )));
            }
            if pages >= max_pages {
                return Err(InnerTubeError::TruncatedResponse(format!(
                    "paginate {browse_id}: hit {max_pages}-page cap"
                )));
            }
            if token.is_none() {
                return Ok(all_items);
            }
        }
    }

    // MARK: - Account

    /// Account menu (verifies signed-in state).
    pub async fn account_menu(
        &self,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
        po_token: Option<String>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        if let Some(po_token) = po_token {
            body.insert(
                "serviceIntegrityDimensions".into(),
                json!({ "poToken": po_token }),
            );
        }
        self.post(
            "account/account_menu",
            body,
            client,
            RetryPolicy::INNER_TUBE,
        )
        .await
    }

    /// Probes `account/account_menu` to confirm the session is still valid.
    pub async fn verify_session(
        &self,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<bool, InnerTubeError> {
        let json = self.account_menu(client, locale, None).await?;
        if BrowseLens::account_name_runs(&json).is_some_and(|runs| !runs.is_empty()) {
            return Ok(true);
        }
        let Some(identity) = BrowseLens::signed_in_account(&json) else {
            if json.is_empty() {
                SwiftyTubeLog::error("verifySession: empty account menu response");
            } else {
                SwiftyTubeLog::notice("verifySession: no account header in menu");
            }
            return Ok(false);
        };
        SwiftyTubeLog::debug(&format!(
            "verifySession: signed in as {} email={} photo={}",
            identity.name.as_deref().unwrap_or("?"),
            if identity.email.is_some() {
                "yes"
            } else {
                "no"
            },
            if identity.has_photo { "yes" } else { "no" },
        ));
        Ok(true)
    }

    /// Fetches account info (name, email, profile picture).
    pub async fn account_info(
        &self,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<AccountInfo, InnerTubeError> {
        let json = self.account_menu(client, locale, None).await?;
        let info = Self::extract_account_info(&json);
        if info.name.is_empty() || info.name == "Guest" {
            SwiftyTubeLog::notice("accountInfo: guest-shaped menu — no active account header");
        } else {
            SwiftyTubeLog::notice(&format!(
                "accountInfo: name={} photo={} email={}",
                info.name,
                if info.thumbnail_url.is_some() {
                    "yes"
                } else {
                    "no"
                },
                if info.email.is_some() { "yes" } else { "no" },
            ));
        }
        Ok(info)
    }

    pub fn extract_account_info(json: &JsonDict) -> AccountInfo {
        let guest = AccountInfo {
            name: "Guest".into(),
            ..AccountInfo::default()
        };
        let first = as_dict_list(json.get("actions")).and_then(|a| a.into_iter().next());
        let popup = as_dict(first.and_then(|f| f.get("openPopupAction")))
            .and_then(|a| as_dict(a.get("popup")));
        let multi_page_menu = popup.and_then(|p| {
            as_dict(p.get("multiPageMenuRenderer"))
                .or_else(|| as_dict(p.get("musicMultiPageMenuRenderer")))
        });
        let Some(active_account) = multi_page_menu
            .and_then(|m| as_dict(m.get("header")))
            .and_then(|h| as_dict(h.get("activeAccountHeaderRenderer")))
        else {
            return guest;
        };
        let runs = |key: &str| InnerTubeJSON::runs_text(as_dict(active_account.get(key)));
        let thumbnails = InnerTubeDecode::array(&["accountPhoto", "thumbnails"], active_account);
        AccountInfo {
            name: runs("accountName").unwrap_or(guest.name),
            email: runs("email"),
            channel_handle: runs("channelHandle"),
            thumbnail_url: InnerTubeJSON::last_thumbnail_url(thumbnails.as_deref()),
            channel_id: Self::channel_account_id_in_dict(active_account),
        }
    }

    /// The account header links the viewer's own channel. Its nesting varies
    /// (browse endpoint vs. channel URL), so scan the header subtree for the
    /// first canonical "UC…" channel id.
    fn channel_account_id(node: Option<&Value>) -> Option<String> {
        match node? {
            Value::Object(dict) => Self::channel_account_id_in_dict(dict),
            Value::Array(list) => list
                .iter()
                .find_map(|value| Self::channel_account_id(Some(value))),
            _ => None,
        }
    }

    fn channel_account_id_in_dict(dict: &JsonDict) -> Option<String> {
        if let Some(browse_id) = dict
            .get("browseId")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("UC"))
        {
            return Some(browse_id.to_string());
        }
        if let Some(url) = dict.get("url").and_then(Value::as_str)
            && let Some((_, rest)) = url.split_once("/channel/")
        {
            let id = rest.split('/').next().unwrap_or(rest);
            if id.starts_with("UC") {
                return Some(id.to_string());
            }
        }
        dict.values()
            .find_map(|value| Self::channel_account_id(Some(value)))
    }

    // MARK: - Likes

    pub async fn like(
        &self,
        video_id: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        self.post_target("like/like", "videoId", video_id, client, locale)
            .await
    }

    pub async fn unlike(
        &self,
        video_id: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        self.post_target("like/removelike", "videoId", video_id, client, locale)
            .await
    }

    pub async fn dislike(
        &self,
        video_id: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        self.post_target("like/dislike", "videoId", video_id, client, locale)
            .await
    }

    /// Rates a playlist/album — this is what YouTube Music's
    /// "Save to library" / "Remove from library" actions call.
    /// `rating` is `LIKE` (save), `INDIFFERENT` (remove), or `DISLIKE`.
    pub async fn rate_playlist(
        &self,
        playlist_id: &str,
        rating: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let endpoint = match rating {
            "INDIFFERENT" => "like/removelike",
            "DISLIKE" => "like/dislike",
            _ => "like/like",
        };
        self.post_target(endpoint, "playlistId", playlist_id, client, locale)
            .await
    }

    // MARK: - Comments

    /// Posts a top-level comment. `params` is the server-issued
    /// `createCommentParams` parsed from the comments header's simplebox —
    /// it embeds the video id, so there is no separate target field. The
    /// response carries the created comment under `actions[0].createCommentAction`,
    /// or a `showErrorAction` on failure.
    pub async fn create_comment(
        &self,
        params: &str,
        text: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("createCommentParams".into(), params.into());
        body.insert("commentText".into(), text.into());
        self.post(
            "comment/create_comment",
            body,
            client,
            RetryPolicy::SINGLE_ATTEMPT,
        )
        .await
    }

    /// Votes on a comment (like/unlike/dislike/undislike) by replaying a
    /// server-issued action token parsed from the comments payload's
    /// toolbar surface (`performCommentActionEndpoint.action`). The response
    /// carries no framework updates, so callers update their state locally
    /// after checking `actionResults[0].status == "STATUS_SUCCEEDED"`.
    pub async fn perform_comment_action(
        &self,
        action: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("actions".into(), json!([action]));
        self.post(
            "comment/perform_comment_action",
            body,
            client,
            RetryPolicy::SINGLE_ATTEMPT,
        )
        .await
    }

    /// Edits an existing comment: replays the server-issued
    /// `updateCommentParams` from the comment's Edit menu item together
    /// with the new text. The response carries `actionResults` like
    /// `perform_comment_action`; check for `STATUS_SUCCEEDED`.
    pub async fn update_comment(
        &self,
        params: &str,
        text: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("updateCommentParams".into(), params.into());
        body.insert("commentText".into(), text.into());
        self.post(
            "comment/update_comment",
            body,
            client,
            RetryPolicy::SINGLE_ATTEMPT,
        )
        .await
    }

    /// Opens the comment abuse-report panel. `params` is the Report menu
    /// item's `showEngagementPanelEndpoint.globalConfiguration.params`.
    /// Works signed-out (YouTube gates the actual submission instead); the
    /// response carries the reason list plus a `flowStateEntity`.
    pub async fn abuse_report_panel(
        &self,
        params: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("panelId".into(), "PAabuse_report".into());
        body.insert("params".into(), params.into());
        self.post("get_panel", body, client, RetryPolicy::INNER_TUBE)
            .await
    }

    /// Advances the abuse-report flow one step (`reason → details →
    /// confirmation`). `flow_state` is the full entity from `get_panel` /
    /// the previous response, mutated with the chosen `reasonKey` and
    /// optional `userComments`; the server requires `reasonKey` (400
    /// INVALID_ARGUMENT without it).
    pub async fn abuse_report_flow(
        &self,
        flow_state: JsonDict,
        flow_state_entity_key: &str,
        flow_id: &str,
        target_id: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("flowId".into(), flow_id.into());
        body.insert("targetId".into(), target_id.into());
        body.insert("flowState".into(), flow_state.into());
        body.insert("flowStateEntityKey".into(), flow_state_entity_key.into());
        self.post("flow", body, client, RetryPolicy::INNER_TUBE)
            .await
    }

    // MARK: - Transcripts

    /// Transcript segments for a video. `params` is the transcript
    /// continuation token from the watch `next` response's transcript button.
    /// `po_token` is needed by recent YouTube builds (FAILED_PRECONDITION
    /// otherwise); mint one with `PoTokenGenerator`.
    pub async fn transcript(
        &self,
        params: &str,
        po_token: Option<String>,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("params".into(), params.into());
        if let Some(po_token) = po_token {
            body.insert(
                "serviceIntegrityDimensions".into(),
                json!({ "poToken": po_token }),
            );
        }
        self.post("get_transcript", body, client, RetryPolicy::INNER_TUBE)
            .await
    }

    // MARK: - Library mutations

    /// Library add/remove tokens.
    pub async fn feedback(
        &self,
        tokens: &[String],
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("feedbackTokens".into(), json!(tokens));
        self.post("feedback", body, client, RetryPolicy::INNER_TUBE)
            .await
    }

    /// Add, remove, or reorder playlist videos.
    pub async fn edit_playlist(
        &self,
        playlist_id: &str,
        actions: Vec<JsonDict>,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("playlistId".into(), playlist_id.into());
        body.insert(
            "actions".into(),
            Value::Array(actions.into_iter().map(Value::Object).collect()),
        );
        self.post(
            "browse/edit_playlist",
            body,
            client,
            RetryPolicy::INNER_TUBE,
        )
        .await
    }

    /// Create a new playlist.
    pub async fn create_playlist(
        &self,
        title: &str,
        description: Option<&str>,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("title".into(), title.into());
        insert_some(&mut body, "description", description);
        self.post("playlist/create", body, client, RetryPolicy::INNER_TUBE)
            .await
    }

    /// Delete a playlist.
    pub async fn delete_playlist(
        &self,
        playlist_id: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("playlistId".into(), playlist_id.into());
        self.post("playlist/delete", body, client, RetryPolicy::INNER_TUBE)
            .await
    }

    // MARK: - Subscriptions

    pub async fn subscribe(
        &self,
        channel_id: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("channelIds".into(), json!([channel_id]));
        self.post(
            "subscription/subscribe",
            body,
            client,
            RetryPolicy::INNER_TUBE,
        )
        .await
    }

    pub async fn unsubscribe(
        &self,
        channel_id: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("channelIds".into(), json!([channel_id]));
        self.post(
            "subscription/unsubscribe",
            body,
            client,
            RetryPolicy::INNER_TUBE,
        )
        .await
    }

    // MARK: - Playback tracking

    pub async fn register_playback(
        &self,
        tracking_url: &str,
        playlist_id: Option<&str>,
        client: Option<YouTubeClient>,
    ) -> Result<(), InnerTubeError> {
        let config = self.config();
        let client = client.unwrap_or(config.default_client);
        let Some(request) = RequestBuilder::build_playback_tracking_request(
            &config,
            tracking_url,
            &client,
            &self.current_auth(),
            playlist_id,
        ) else {
            SwiftyTubeLog::error("Failed to build playback tracking request");
            return Err(InnerTubeError::InvalidResponse);
        };
        let http = HttpClient::data(request, self.session()).await?;
        if !(200..=299).contains(&http.status) {
            SwiftyTubeLog::error(&format!("Playback tracking HTTP {}", http.status));
            return Err(InnerTubeError::HttpError {
                status_code: http.status,
                data: http.data,
            });
        }
        SwiftyTubeLog::debug(&format!(
            "Playback registered - status={} bytes={}",
            http.status,
            http.data.len()
        ));
        Ok(())
    }

    // MARK: - Helpers (Rust only: Swift writes each body literal inline)

    /// `["context": contextDict(client:locale:)]`.
    fn body(&self, client: Option<YouTubeClient>, locale: Option<&YouTubeLocale>) -> JsonDict {
        let mut body = JsonDict::new();
        body.insert("context".into(), self.context_dict(client.as_ref(), locale));
        body
    }

    /// Like endpoints: context + `target: {key: id}`.
    async fn post_target(
        &self,
        endpoint: &str,
        key: &str,
        id: &str,
        client: Option<YouTubeClient>,
        locale: Option<YouTubeLocale>,
    ) -> Result<JsonDict, InnerTubeError> {
        let mut body = self.body(client, locale.as_ref());
        body.insert("target".into(), json!({ key: id }));
        self.post(endpoint, body, client, RetryPolicy::INNER_TUBE)
            .await
    }
}
