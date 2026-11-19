//! Calls YouTube anonymously: `cargo test -p trop --test comments_live_test -- --ignored`.

use std::sync::Arc;

use swiftytube::auth::auth_state::AuthState;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::networking::inner_tube_client::InnerTubeClient;
use trop::services::comments_service::CommentsService;

#[tokio::test]
#[ignore]
async fn loads_comments_replies_and_the_newest_order() {
    let music = Arc::new(InnerTubeClient::new(
        SwiftyTubeConfig::music(),
        AuthState::guest(),
    ));
    let service = CommentsService::new(music);
    let page = service
        .load_initial(&std::env::var("WOOF_VIDEO").unwrap_or_else(|_| "dQw4w9WgXcQ".into()))
        .await
        .expect("comments load");
    assert!(!page.items.is_empty());
    println!(
        "{} comments, total {:?}, sorts {:?}, next {}",
        page.items.len(),
        page.total_count_text,
        page.sort_tokens.keys().collect::<Vec<_>>(),
        page.next_page_token.is_some()
    );
    let first = &page.items[0];
    println!(
        "{} · {} · {} likes",
        first.author.name, first.published_time, first.like_count_text
    );
    let thread = page
        .items
        .iter()
        .find(|item| item.reply_continuation.is_some())
        .expect("a thread with replies");
    let replies = service
        .load_replies(thread.reply_continuation.as_deref().unwrap())
        .await
        .expect("replies load");
    assert!(!replies.items.is_empty());
    let more = service
        .load_page(page.next_page_token.as_deref().unwrap())
        .await
        .expect("next page");
    assert!(!more.items.is_empty());
}
