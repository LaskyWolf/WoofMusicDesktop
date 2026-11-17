//! Talks to the real YouTube Music. Run with `cargo test -p swiftytube -- --ignored`.

use swiftytube::auth::auth_state::AuthState;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use swiftytube::networking::inner_tube_decode::InnerTubeDecode;

#[tokio::test]
#[ignore = "needs network"]
async fn anonymous_browse_home_returns_sections() {
    let client = InnerTubeClient::new(SwiftyTubeConfig::music(), AuthState::guest());
    let home = client.browse(BrowseArgs::id("FEmusic_home")).await.unwrap();
    let tabs = InnerTubeDecode::array(
        &["contents", "singleColumnBrowseResultsRenderer", "tabs"],
        &home,
    )
    .unwrap();
    let sections = InnerTubeDecode::array(
        &["tabRenderer", "content", "sectionListRenderer", "contents"],
        tabs[0],
    )
    .unwrap();
    assert!(!sections.is_empty());
    assert!(client.current_auth().visitor_data.is_some());
    println!("sections={}", sections.len());
}
