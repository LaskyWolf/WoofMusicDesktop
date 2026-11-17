use std::sync::{Arc, LazyLock};

use super::cipher_error::CipherError;
use super::cipher_js_engine::CipherJsEngine;
use super::player_config_store::PlayerConfigStore;
use super::player_js_fetcher::PlayerJsFetcher;
use crate::streaming::stream_resolver::StreamResolveProviders;

/// Cipher calls via the shared JS engine, plus one-line provider wiring.
pub struct CipherExecutor {
    engine: Arc<CipherJsEngine>,
    fetcher: Arc<PlayerJsFetcher>,
}

impl CipherExecutor {
    pub fn shared() -> Arc<CipherExecutor> {
        static SHARED: LazyLock<Arc<CipherExecutor>> = LazyLock::new(|| {
            Arc::new(CipherExecutor::new(
                CipherJsEngine::shared(),
                PlayerJsFetcher::shared(),
            ))
        });
        SHARED.clone()
    }

    pub fn new(engine: Arc<CipherJsEngine>, fetcher: Arc<PlayerJsFetcher>) -> Self {
        Self { engine, fetcher }
    }

    /// `player_js` and `player_hash` are unused, as in Swift: the engine loads its own copy.
    pub async fn resolve_cipher_url(
        &self,
        cipher_text: &str,
        _player_js: &str,
        _player_hash: Option<&str>,
    ) -> Result<String, CipherError> {
        self.engine.load().await?;
        self.engine.resolve_cipher_url(cipher_text).await
    }

    pub async fn get_signature_timestamp(&self) -> Result<i64, CipherError> {
        self.fetcher.get_signature_timestamp().await
    }
}

/// Swift `extension StreamResolveProviders { static var live }`.
impl StreamResolveProviders {
    /// Full cipher wiring for `StreamResolver::resolve`.
    pub fn live() -> Self {
        Self {
            signature_timestamp: Some(Arc::new(|| {
                Box::pin(async {
                    PlayerJsFetcher::shared()
                        .get_signature_timestamp()
                        .await
                        .ok()
                })
            })),
            cipher_url: Some(Arc::new(|cipher_text, player_js| {
                Box::pin(async move {
                    CipherExecutor::shared()
                        .resolve_cipher_url(&cipher_text, &player_js, None)
                        .await
                })
            })),
            player_js: Some(Arc::new(|| {
                Box::pin(async { PlayerJsFetcher::shared().get_player_js().await })
            })),
            on_stream_rejection: Some(Arc::new(|| {
                Box::pin(async { PlayerConfigStore::shared().notify_stream_rejection().await })
            })),
            ..Self::default()
        }
    }
}
