// Logic half of Trop/LastFM/LastFMAuthWebView.swift. Swift shows the approval
// page in a web view; the desktop app opens it in the browser. Last.fm has no
// redirect back to the app, so both poll until the token is approved.

use std::time::Duration;

use swiftytube::auth::secure_store::SecureStoreError;

use super::last_fm_defaults::LastFMDefaults;
use super::last_fm_service::{LastFMError, LastFMService, LastFMSession};

impl LastFMService {
    const SIGN_IN_POLL_INTERVAL: Duration = Duration::from_secs(2);

    /// Swift `fetchToken`: a new token and the page where the user approves it.
    pub async fn start_sign_in(&self) -> Result<(String, String), LastFMError> {
        let token = self.get_token().await?;
        let url = LastFMDefaults::auth_url(&self.api_key, &token);
        log::info!("LastFM auth token issued, loading approval page");
        Ok((token, url))
    }

    /// Swift `startPolling`: asks for the session every 2 seconds until the user
    /// approves `token`, then stores it. Last.fm errors are retried; drop the
    /// future to cancel.
    pub async fn complete_sign_in(&self, token: &str) -> Result<LastFMSession, SecureStoreError> {
        loop {
            tokio::time::sleep(Self::SIGN_IN_POLL_INTERVAL).await;
            if let Ok(auth) = self.get_session(token).await {
                self.token_store
                    .store(&auth.session.key, &auth.session.name)?;
                return Ok(auth.session);
            }
        }
    }
}
