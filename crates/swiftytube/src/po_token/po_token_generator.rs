// Replaces PoTokenGenerator.swift (WKWebView + po_token.html). Not ported to
// QuickJS: BotGuard does not attest outside a real browser.
//
// What was tried with the Dart translation (2026-11, anonymous, BotGuardService
// requests as in Swift):
// 1. `Create` works: the challenge descrambles and the interpreter (~64 KB) loads.
// 2. In a JS VM with no DOM, the interpreter throws `window is not defined`.
// 3. With the DOM stub from `cipher_js_engine.rs`, `runBotGuard` returns a
//    ~2.3 KB botguardResponse, but `GenerateIT` answers
//    `[null, 43200, null, "<websafeFallbackToken>"]`: integrityToken is null, so
//    attestation was rejected. `webPoSignalOutput[0]` (getMinter) is never set,
//    so `createPoTokenMinter` fails with "PMD:Undefined".
// QuickJS has no timers, which `loadBotGuard` polls with, so it gets no further.
//
// `generate` therefore fails without network calls. StreamFallback's PoToken
// provider gets None and the chain uses clients that do not need a token.
// To revisit with the R5 webview (`wry`).

use std::sync::LazyLock;

use super::po_token_result::PoTokenResult;

#[derive(Debug, thiserror::Error)]
#[error("PoToken minting needs a browser: BotGuard rejects attestation in QuickJS")]
pub struct PoTokenUnsupported;

#[derive(Default)]
pub struct PoTokenGenerator;

impl PoTokenGenerator {
    pub fn shared() -> &'static PoTokenGenerator {
        static SHARED: LazyLock<PoTokenGenerator> = LazyLock::new(PoTokenGenerator::default);
        &SHARED
    }

    /// Swift mints `(sessionId ?? "SESSION")` and `videoId` tokens. Always fails here.
    pub async fn generate(
        &self,
        _video_id: &str,
        _session_id: Option<&str>,
    ) -> Result<PoTokenResult, PoTokenUnsupported> {
        Err(PoTokenUnsupported)
    }
}
