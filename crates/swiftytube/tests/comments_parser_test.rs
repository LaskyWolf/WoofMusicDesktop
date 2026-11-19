//! `Tests/SwiftyTubeTests/YouTube/CommentsParserTests.swift` without the Report flow tests.

use swiftytube::networking::inner_tube_decode::JsonDict;
use swiftytube::youtube::comment_models::{CommentTextRun, CommentVoteState};
use swiftytube::youtube::comments_parser::CommentsParser;

fn json(text: &str) -> JsonDict {
    serde_json::from_str(text).expect("valid fixture")
}

const SIGNED_IN_JSON: &str = r#"
{
  "onResponseReceivedEndpoints": [
    { "appendContinuationItemsAction": { "continuationItems": [
      { "commentThreadRenderer": {
          "commentViewModel": { "commentViewModel": {
            "commentId": "UgwIN0000000000000001",
            "toolbarStateKey": "state-key-1",
            "toolbarSurfaceKey": "surface-key-1"
          }}
      }},
      { "continuationItemRenderer": { "continuationEndpoint": {
          "continuationCommand": { "token": "Eg0SC3BhZ2VUb2tlbl8xMjM0NTY3ODkw" }
      }}}
    ]}}
  ],
  "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
    { "payload": { "commentEntityPayload": {
        "key": "comment-key-1",
        "author": { "displayName": "Somebody", "isVerified": true },
        "properties": {
          "commentId": "UgwIN0000000000000001",
          "content": { "content": "Great song!" },
          "publishedTime": "2 days ago",
          "toolbarStateKey": "state-key-1"
        },
        "toolbar": {
          "likeCountNotliked": "11",
          "likeCountLiked": "12",
          "replyCount": "3"
        }
    }}},
    { "payload": { "engagementToolbarStateEntityPayload": {
        "key": "state-key-1",
        "likeState": "TOOLBAR_LIKE_STATE_INDIFFERENT",
        "heartState": "TOOLBAR_HEART_STATE_UNHEARTED"
    }}},
    { "payload": { "engagementToolbarSurfaceEntityPayload": {
        "key": "surface-key-1",
        "isEngagementToolbar": true,
        "likeCommand": { "innertubeCommand": { "performCommentActionEndpoint": {
            "action": "LIKE_TOKEN_1"
        }}},
        "unlikeCommand": { "innertubeCommand": { "performCommentActionEndpoint": {
            "action": "UNLIKE_TOKEN_1"
        }}},
        "dislikeCommand": { "innertubeCommand": { "performCommentActionEndpoint": {
            "action": "DISLIKE_TOKEN_1"
        }}},
        "undislikeCommand": { "innertubeCommand": { "performCommentActionEndpoint": {
            "action": "UNDISLIKE_TOKEN_1"
        }}}
    }}}
  ]}}
}
"#;

const SIGNED_OUT_JSON: &str = r#"
{
  "onResponseReceivedEndpoints": [
    { "appendContinuationItemsAction": { "continuationItems": [
      { "commentThreadRenderer": {
          "commentViewModel": { "commentViewModel": {
            "commentId": "UgwOUT000000000000001",
            "toolbarStateKey": "state-key-out",
            "toolbarSurfaceKey": "surface-key-out"
          }}
      }}
    ]}}
  ],
  "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
    { "payload": { "commentEntityPayload": {
        "properties": {
          "commentId": "UgwOUT000000000000001",
          "content": { "content": "First!" },
          "publishedTime": "1 day ago",
          "toolbarStateKey": "state-key-out"
        },
        "toolbar": { "likeCountNotliked": "5", "likeCountLiked": "6" }
    }}},
    { "payload": { "engagementToolbarStateEntityPayload": {
        "key": "state-key-out",
        "likeState": "TOOLBAR_LIKE_STATE_INDIFFERENT",
        "heartState": "TOOLBAR_HEART_STATE_UNHEARTED"
    }}},
    { "payload": { "engagementToolbarSurfaceEntityPayload": {
        "key": "surface-key-out",
        "isEngagementToolbar": true,
        "prepareAccountCommand": { "innertubeCommand": { "modalEndpoint": {} } },
        "likeCommand": { "innertubeCommand": { "clickTrackingParams": "CAAQ" } },
        "unlikeCommand": { "innertubeCommand": { "clickTrackingParams": "CAAQ" } },
        "dislikeCommand": { "innertubeCommand": { "clickTrackingParams": "CAAQ" } },
        "undislikeCommand": { "innertubeCommand": { "clickTrackingParams": "CAAQ" } }
    }}}
  ]}}
}
"#;

const SIGNED_IN_REPLIES_JSON: &str = r#"
{
  "onResponseReceivedEndpoints": [
    { "appendContinuationItemsAction": { "continuationItems": [
      { "commentViewModel": { "commentViewModel": {
          "commentId": "UgwREP000000000000001",
          "toolbarStateKey": "state-key-rep",
          "toolbarSurfaceKey": "surface-key-rep"
      }}},
      { "continuationItemRenderer": { "continuationEndpoint": {
          "continuationCommand": { "token": "Eg0SC3JlcGx5VG9rZW5fOTg3NjU0MzIxMA" }
      }}}
    ]}}
  ],
  "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
    { "payload": { "commentEntityPayload": {
        "properties": {
          "commentId": "UgwREP000000000000001",
          "content": { "content": "Reply body" },
          "publishedTime": "3 hours ago",
          "replyLevel": 1,
          "toolbarStateKey": "state-key-rep"
        },
        "toolbar": { "likeCountNotliked": "11", "likeCountLiked": "12" }
    }}},
    { "payload": { "engagementToolbarStateEntityPayload": {
        "key": "state-key-rep",
        "likeState": "TOOLBAR_LIKE_STATE_LIKED",
        "heartState": "TOOLBAR_HEART_STATE_UNHEARTED"
    }}},
    { "payload": { "engagementToolbarSurfaceEntityPayload": {
        "key": "surface-key-rep",
        "isEngagementToolbar": true,
        "likeCommand": { "innertubeCommand": { "performCommentActionEndpoint": {
            "action": "LIKE_TOKEN_REP"
        }}},
        "unlikeCommand": { "innertubeCommand": { "performCommentActionEndpoint": {
            "action": "UNLIKE_TOKEN_REP"
        }}},
        "dislikeCommand": { "innertubeCommand": { "performCommentActionEndpoint": {
            "action": "DISLIKE_TOKEN_REP"
        }}},
        "undislikeCommand": { "innertubeCommand": { "performCommentActionEndpoint": {
            "action": "UNDISLIKE_TOKEN_REP"
        }}}
    }}}
  ]}}
}
"#;

const HEADER_JSON: &str = r#"
{
  "onResponseReceivedEndpoints": [
    { "reloadContinuationItemsCommand": { "continuationItems": [
      { "commentsHeaderRenderer": {
          "countText": { "runs": [{ "text": "2,661" }, { "text": " Comments" }] },
          "createRenderer": { "commentSimpleboxRenderer": {
              "submitButton": { "buttonRenderer": {
                  "serviceEndpoint": { "createCommentEndpoint": {
                      "createCommentParams": "EgtPMURhYmVNUno1SSoCCABQB-gCAagDAA%3D%3D"
                  }}
              }}
          }}
      }}
    ]}}
  ]
}
"#;

const SIGNED_OUT_HEADER_JSON: &str = r#"
{
  "onResponseReceivedEndpoints": [
    { "reloadContinuationItemsCommand": { "continuationItems": [
      { "commentsHeaderRenderer": {
          "countText": { "runs": [{ "text": "42" }, { "text": " Comments" }] }
      }}
    ]}}
  ]
}
"#;

const OWN_DELETE_MENU: &str = r#"
"likeCommand": { "innertubeCommand": { "performCommentActionEndpoint": { "action": "LIKE_TOKEN_9" } }},
"menuCommand": { "innertubeCommand": { "menuEndpoint": { "menu": { "menuRenderer": { "items": [
  { "menuServiceItemRenderer": {
      "text": { "runs": [{ "text": "Delete" }] },
      "icon": { "iconType": "DELETE" },
      "serviceEndpoint": { "performCommentActionEndpoint": { "action": "DELETE_TOKEN_9" } }
  }}
]}}}}}
"#;

const CONFIRM_DIALOG_DELETE_MENU: &str = r#"
"menuCommand": { "innertubeCommand": { "menuEndpoint": { "menu": { "menuRenderer": { "items": [
  { "menuNavigationItemRenderer": {
      "icon": { "iconType": "EDIT" },
      "text": { "runs": [{ "text": "Edit" }] },
      "navigationEndpoint": { "updateCommentDialogEndpoint": { "dialog": { "commentDialogRenderer": {
          "editableText": { "runs": [{ "text": "My own comment" }] },
          "submitButton": { "buttonRenderer": { "serviceEndpoint": {
              "commandMetadata": { "webCommandMetadata": { "apiUrl": "/youtubei/v1/comment/update_comment" }},
              "updateCommentEndpoint": { "updateCommentParams": "EDIT_PARAMS_CD" }
          }}}
      }}}}
  }},
  { "menuNavigationItemRenderer": {
      "icon": { "iconType": "DELETE" },
      "text": { "runs": [{ "text": "Delete" }] },
      "navigationEndpoint": { "confirmDialogEndpoint": { "content": { "confirmDialogRenderer": {
          "confirmButton": { "buttonRenderer": { "serviceEndpoint": {
              "performCommentActionEndpoint": { "action": "DELETE_TOKEN_CD" }
          }}}
      }}}}
  }}
]}}}}}
"#;

const REPORT_ONLY_MENU: &str = r#"
"menuCommand": { "innertubeCommand": { "menuEndpoint": { "menu": { "menuRenderer": { "items": [
  { "menuServiceItemRenderer": {
      "text": { "runs": [{ "text": "Report" }] },
      "icon": { "iconType": "FLAG" },
      "serviceEndpoint": { "showEngagementPanelEndpoint": { "identifier": { "tag": "PAabuse_report" } } }
  }}
]}}}}}
"#;

/// One signed-in comment whose toolbar surface carries `surface`.
fn page(surface: &str, author: &str, entity_extra: &str) -> JsonDict {
    json(&format!(
        r#"
{{
  "onResponseReceivedEndpoints": [
    {{ "appendContinuationItemsAction": {{ "continuationItems": [
      {{ "commentThreadRenderer": {{ "commentViewModel": {{ "commentViewModel": {{
          "commentId": "UgwIN0000000000000009",
          "toolbarStateKey": "state-key-9",
          "toolbarSurfaceKey": "surface-key-9"
      }}}}}}}}
    ]}}}}
  ],
  "frameworkUpdates": {{ "entityBatchUpdate": {{ "mutations": [
    {{ "payload": {{ "commentEntityPayload": {{
        "key": "comment-key-9",
        "author": {{ {author} }},
        "properties": {{
          "commentId": "UgwIN0000000000000009",
          "content": {{ "content": "My own comment" }},
          "publishedTime": "1 day ago",
          "toolbarStateKey": "state-key-9"
        }},
        {entity_extra}
        "toolbar": {{ "likeCountNotliked": "0" }}
    }}}}}},
    {{ "payload": {{ "engagementToolbarSurfaceEntityPayload": {{
        "key": "surface-key-9",
        {surface}
    }}}}}}
  ]}}}}
}}
"#
    ))
}

const ME: &str = r#""displayName": "Me""#;
#[test]
fn comments_header_exposes_create_comment_params_when_signed_in() {
    let page = CommentsParser::parse_comments_page(&json(HEADER_JSON));
    assert_eq!(
        page.create_comment_params.as_deref(),
        Some("EgtPMURhYmVNUno1SSoCCABQB-gCAagDAA%3D%3D")
    );
    assert_eq!(page.total_count_text.as_deref(), Some("2,661"));
    assert!(page.items.is_empty());
}

#[test]
fn signed_out_header_carries_no_create_comment_params() {
    let page = CommentsParser::parse_comments_page(&json(SIGNED_OUT_HEADER_JSON));
    assert_eq!(page.create_comment_params, None);
}

#[test]
fn own_comment_exposes_a_delete_token_from_its_menu() {
    let parsed = CommentsParser::parse_comments_page(&page(OWN_DELETE_MENU, ME, ""));
    assert_eq!(parsed.items.len(), 1);
    assert_eq!(
        parsed.items[0].delete_action.as_deref(),
        Some("DELETE_TOKEN_9")
    );
    assert_eq!(parsed.items[0].like_action.as_deref(), Some("LIKE_TOKEN_9"));
}

#[test]
fn delete_behind_a_confirm_dialog_still_yields_the_token_and_edit_params() {
    let parsed = CommentsParser::parse_comments_page(&page(CONFIRM_DIALOG_DELETE_MENU, ME, ""));
    assert_eq!(
        parsed.items[0].delete_action.as_deref(),
        Some("DELETE_TOKEN_CD")
    );
    assert_eq!(
        parsed.items[0].edit_params.as_deref(),
        Some("EDIT_PARAMS_CD")
    );
}

#[test]
fn report_only_menus_carry_no_edit_params_or_delete_token() {
    let parsed = CommentsParser::parse_comments_page(&page(REPORT_ONLY_MENU, ME, ""));
    assert_eq!(parsed.items[0].edit_params, None);
    assert_eq!(parsed.items[0].delete_action, None);
}

#[test]
fn unlabeled_action_menu_falls_back_to_its_token() {
    let surface = r#"
    "menuCommand": { "innertubeCommand": { "menuEndpoint": { "menu": { "menuRenderer": { "items": [
      { "menuServiceItemRenderer": {
          "text": { "runs": [{ "text": "Supprimer" }] },
          "serviceEndpoint": { "performCommentActionEndpoint": { "action": "DELETE_TOKEN_U" } }
      }}
    ]}}}}}
    "#;
    let parsed = CommentsParser::parse_comments_page(&page(surface, ME, ""));
    assert_eq!(
        parsed.items[0].delete_action.as_deref(),
        Some("DELETE_TOKEN_U")
    );
}

#[test]
fn menu_on_the_comment_entity_itself_yields_the_delete_token() {
    let entity_extra = r#"
    "actionMenu": { "menuRenderer": { "items": [
      { "menuServiceItemRenderer": {
          "text": { "runs": [{ "text": "Delete" }] },
          "serviceEndpoint": { "performCommentActionEndpoint": { "action": "DELETE_TOKEN_E" } }
      }}
    ]}},
    "#;
    let parsed = CommentsParser::parse_comments_page(&page(
        r#""isEngagementToolbar": true"#,
        ME,
        entity_extra,
    ));
    assert_eq!(
        parsed.items[0].delete_action.as_deref(),
        Some("DELETE_TOKEN_E")
    );
}

#[test]
fn author_channel_url_prefers_the_channel_id_over_a_handle() {
    let author = r#"
    "displayName": "Me",
    "channelId": "UCfakes0123456789abcdef",
    "channelCommand": { "innertubeCommand": { "commandMetadata": { "webCommandMetadata": {
        "url": "/@somehandle"
    }}}}
    "#;
    let parsed = CommentsParser::parse_comments_page(&page(OWN_DELETE_MENU, author, ""));
    let url = parsed.items[0].author.channel_url.as_deref().unwrap();
    assert!(url.ends_with("/channel/UCfakes0123456789abcdef"), "{url}");
}

#[test]
fn signed_in_thread_exposes_all_four_action_tokens() {
    let page = CommentsParser::parse_comments_page(&json(SIGNED_IN_JSON));
    assert_eq!(page.items.len(), 1);
    let comment = &page.items[0];
    assert_eq!(comment.id, "UgwIN0000000000000001");
    assert_eq!(comment.like_action.as_deref(), Some("LIKE_TOKEN_1"));
    assert_eq!(comment.unlike_action.as_deref(), Some("UNLIKE_TOKEN_1"));
    assert_eq!(comment.dislike_action.as_deref(), Some("DISLIKE_TOKEN_1"));
    assert_eq!(
        comment.undislike_action.as_deref(),
        Some("UNDISLIKE_TOKEN_1")
    );
    assert_eq!(comment.vote_state, CommentVoteState::Indifferent);
    assert_eq!(comment.like_count_text, "11");
    assert_eq!(comment.delete_action, None);
    assert_eq!(comment.reply_count, 3);
    assert_eq!(comment.reply_count_text, "3");
    assert_eq!(comment.author.name, "Somebody");
    assert!(comment.author.is_verified);
    assert_eq!(comment.runs, [CommentTextRun::Text("Great song!".into())]);
    assert_eq!(
        page.next_page_token.as_deref(),
        Some("Eg0SC3BhZ2VUb2tlbl8xMjM0NTY3ODkw")
    );
}

#[test]
fn signed_out_toolbar_carries_no_action_tokens() {
    let page = CommentsParser::parse_comments_page(&json(SIGNED_OUT_JSON));
    assert_eq!(page.items.len(), 1);
    let comment = &page.items[0];
    assert_eq!(comment.like_action, None);
    assert_eq!(comment.unlike_action, None);
    assert_eq!(comment.dislike_action, None);
    assert_eq!(comment.undislike_action, None);
    assert_eq!(comment.vote_state, CommentVoteState::Indifferent);
    assert_eq!(comment.like_count_text, "5");
}

#[test]
fn liked_reply_reports_liked_state_count_and_tokens() {
    let page = CommentsParser::parse_replies_page(&json(SIGNED_IN_REPLIES_JSON));
    assert_eq!(page.items.len(), 1);
    let reply = &page.items[0];
    assert_eq!(reply.id, "UgwREP000000000000001");
    assert_eq!(reply.vote_state, CommentVoteState::Liked);
    assert_eq!(reply.like_count_text, "12");
    assert_eq!(reply.like_action.as_deref(), Some("LIKE_TOKEN_REP"));
    assert_eq!(reply.unlike_action.as_deref(), Some("UNLIKE_TOKEN_REP"));
    assert_eq!(reply.dislike_action.as_deref(), Some("DISLIKE_TOKEN_REP"));
    assert_eq!(
        reply.undislike_action.as_deref(),
        Some("UNDISLIKE_TOKEN_REP")
    );
    assert_eq!(reply.reply_count, 0);
    assert_eq!(reply.reply_count_text, "");
    assert_eq!(
        page.next_page_token.as_deref(),
        Some("Eg0SC3JlcGx5VG9rZW5fOTg3NjU0MzIxMA")
    );
}

/// Not in Swift's tests: the run splitting, which the panel draws as links.
#[test]
fn text_runs_split_links_and_timestamps_by_utf16_index() {
    let body = r#"{
      "onResponseReceivedEndpoints": [{ "appendContinuationItemsAction": { "continuationItems": [
        { "commentThreadRenderer": { "commentViewModel": { "commentViewModel": { "commentId": "C1" }}}}
      ]}}],
      "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
        { "payload": { "commentEntityPayload": {
          "properties": { "commentId": "C1", "content": {
            "content": "🎵 at 1:23 see https://x.example",
            "commandRuns": [
              { "startIndex": 6, "length": 4, "onTap": { "innertubeCommand": {
                  "watchEndpoint": { "videoId": "abc", "startTimeSeconds": 83 } } } },
              { "startIndex": 15, "length": 17, "onTap": { "innertubeCommand": {
                  "urlEndpoint": { "url": "https://www.youtube.com/redirect?q=https%3A%2F%2Fx.example&v=1" } } } }
            ]
          }},
          "toolbar": { "replyCount": "1.2K" }
        }}}
      ]}}
    }"#;
    let page = CommentsParser::parse_comments_page(&json(body));
    let comment = &page.items[0];
    assert_eq!(
        comment.runs,
        [
            CommentTextRun::Text("🎵 at ".into()),
            CommentTextRun::Timestamp {
                text: "1:23".into(),
                seconds: 83.0,
                video_id: Some("abc".into()),
            },
            CommentTextRun::Text(" see ".into()),
            CommentTextRun::Link {
                text: "https://x.example".into(),
                url: "https://x.example".into(),
            },
        ]
    );
    assert_eq!(comment.reply_count, 1200);
    assert_eq!(
        CommentsParser::plain_text(&comment.runs),
        "🎵 at 1:23 see https://x.example"
    );
}
