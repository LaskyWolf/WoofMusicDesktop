//! Port of the Dart `test/swiftytube/cipher_test.dart`, plus the QuickJS engine
//! on a mocked player.js and the BotGuard parsers.

use std::sync::Arc;

use swiftytube::cipher::cipher_html_builder::CipherHTMLBuilder;
use swiftytube::cipher::cipher_js_engine::CipherJsEngine;
use swiftytube::cipher::function_name_extractor::FunctionNameExtractor;
use swiftytube::cipher::player_config::PlayerConfig;
use swiftytube::cipher::player_config_store::{BUNDLED_PLAYER_CONFIGS, PlayerConfigStore};
use swiftytube::cipher::player_js_fetcher::PlayerJsFetcher;
use swiftytube::networking::http_client::{HttpResponse, Session};
use swiftytube::po_token::bot_guard_service::BotGuardService;

/// Shaped like the inside of player.js: a decoy, a sig function, an n function, the URL class use.
/// (The real file's outer IIFE is ~3 MB, over the 3000-char scan limit, so it never matches.)
const BODY: &str = r#"
var decoy=function(a){return a+"}"};
Xy=function(a){a=a.split("");Ab.reverse(a);return a.join("")};
var nf=function(b){var c=b.charCodeAt(0);return String.fromCharCode(c+1)+'{'};
g.Kq=function(u,t){this.u=u};
function use(x){return (new g.Kq(x,!0)).get("n")}
var cfg={signatureTimestamp:20770};
"#;

fn synthetic_js() -> String {
    format!("var _yt_player={{}};(function(g){{var window=this;\n{BODY}}})(_yt_player);")
}

#[test]
fn finds_the_sig_function_by_split_join() {
    let extracted = FunctionNameExtractor::extract_heuristic(BODY);
    assert_eq!(
        extracted.sig_js,
        r#"(function(a){a=a.split("");Ab.reverse(a);return a.join("")})"#
    );
}

#[test]
fn finds_the_n_function_skipping_quoted_braces() {
    let extracted = FunctionNameExtractor::extract_heuristic(BODY);
    assert_eq!(
        extracted.n_js.as_deref(),
        Some("(function(b){var c=b.charCodeAt(0);return String.fromCharCode(c+1)+'{'})")
    );
}

#[test]
fn finds_the_url_builder_class() {
    let extracted = FunctionNameExtractor::extract_heuristic(BODY);
    assert_eq!(extracted.n_class.as_deref(), Some("Kq"));
    assert_eq!(
        FunctionNameExtractor::extract_n_class_heuristic("x=new g.Zz(a, !0);").as_deref(),
        Some("Zz")
    );
}

#[test]
fn extracts_the_signature_timestamp() {
    assert_eq!(
        FunctionNameExtractor::extract_signature_timestamp(BODY),
        Some(20770)
    );
    assert_eq!(
        PlayerJsFetcher::extract_signature_timestamp("\"signatureTimestamp\":12345"),
        Ok(12345)
    );
}

#[test]
fn no_match_yields_an_empty_sig() {
    let none = FunctionNameExtractor::extract_heuristic("var a=1;");
    assert_eq!(none.sig_js, "");
    assert!(none.n_js.is_none());
    assert!(none.n_class.is_none());
}

#[test]
fn config_parse_keeps_valid_entries_and_skips_invalid_ones() {
    let configs = PlayerConfigStore::parse(
        r#"{"schemaVersion":1,"players":{
 "9c249f6f":{"sig":"Tl(48,5831,INPUT)","nClass":"W_","sts":20602,"aliases":["a6fc27c5"]},
 "bad00000":{"sig":"alert(1)","nClass":"W_","sts":1},
 "0000000z":{"sig":"Tl(1,2,INPUT)","nClass":"W_","sts":1}
}}"#,
    )
    .unwrap();
    assert_eq!(configs.keys().collect::<Vec<_>>(), ["9c249f6f"]);
    assert_eq!(
        configs["9c249f6f"].aliases.as_deref(),
        Some(&["a6fc27c5".to_string()][..])
    );
}

#[test]
fn config_parse_rejects_the_whole_table_on_a_duplicate_key_or_bad_schema() {
    assert!(PlayerConfigStore::parse(r#"{"schemaVersion":"1","players":{}}"#).is_none());
    assert!(
        PlayerConfigStore::parse(
            r#"{"schemaVersion":1,"players":{
"aaaaaaaa":{"sig":"a(1,2,INPUT)","nClass":"b","sts":1},
"bbbbbbbb":{"sig":"a(1,2,INPUT)","nClass":"b","sts":1,"aliases":["aaaaaaaa"]}}}"#
        )
        .is_none()
    );
}

#[test]
fn bundled_table_parses() {
    let configs = PlayerConfigStore::parse(BUNDLED_PLAYER_CONFIGS).unwrap();
    assert_eq!(configs["9c249f6f"].n_class.as_deref(), Some("W_"));
}

#[test]
fn patch_player_js_exports_before_the_iife_close() {
    let n_expression = PlayerConfig {
        n_class: Some("W_".into()),
        ..PlayerConfig::default()
    }
    .n_js_expression();
    let patched = CipherHTMLBuilder::patch_player_js(
        &synthetic_js(),
        Some("Tl(48,5831,INPUT)"),
        true,
        Some("W_"),
        n_expression.as_deref(),
        None,
    );
    assert!(patched.contains("window._cipherSigFunc=function(sig){try{return Tl(48,5831,sig)}"));
    assert!(patched.contains("new g.W_('https://x.googlevideo.com/videoplayback?n='+n,true)"));
    assert!(patched.find("window._nTransformFunc") < patched.find("})(_yt_player);"));
}

/// A tiny player.js: `Tl` reverses the sig, `g.Kq` is the URL class and its
/// `get("n")` returns the transformed n (reversed + "Z").
const MOCK_PLAYER_JS: &str = r#"var _yt_player={};(function(g){var window=this;
var Tl=function(a,b,c){return c.split("").reverse().join("")};
g.Kq=function(u,t){var q=u.split("?");this.base=q[0];this.keys=[];this.vals={};
 var parts=q[1]?q[1].split("&"):[];for(var i=0;i<parts.length;i++){var kv=parts[i].split("=");this.set(kv[0],kv[1])}};
g.Kq.prototype.set=function(k,v){if(!(k in this.vals))this.keys.push(k);this.vals[k]=v};
g.Kq.prototype.get=function(k){var v=this.vals[k];return k==="n"?v.split("").reverse().join("")+"Z":v};
g.Kq.prototype.toString=function(){var s=[];for(var i=0;i<this.keys.length;i++)s.push(this.keys[i]+"="+this.vals[this.keys[i]]);return this.base+"?"+s.join("&")};
var cfg={signatureTimestamp:20770};
})(_yt_player);"#;

#[tokio::test]
async fn quickjs_engine_signs_and_transforms_n_on_a_mocked_player_js() {
    let dir = std::env::temp_dir().join(format!("swiftytube-cipher-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let session = Session::mock(|request| {
        let path = request.url().path().to_string();
        let (status, data) = if path == "/iframe_api" {
            (
                200,
                br#"var u="https:\/\/www.youtube.com\/s\/player\/abcd1234\/www-widgetapi.vflset";"#
                    .to_vec(),
            )
        } else if path.ends_with("/base.js") {
            (200, MOCK_PLAYER_JS.as_bytes().to_vec())
        } else {
            (404, Vec::new()) // the remote config table
        };
        HttpResponse {
            status,
            data,
            ..HttpResponse::default()
        }
    });
    let table = r#"{"schemaVersion":1,"players":{"abcd1234":{"sig":"Tl(48,5831,INPUT)","nClass":"Kq","sts":20770}}}"#;
    let store = Arc::new(PlayerConfigStore::new(
        Some(table.into()),
        session.clone(),
        Some(dir.clone()),
    ));
    let fetcher = Arc::new(PlayerJsFetcher::new(
        Some(dir.clone()),
        session,
        store.clone(),
    ));
    let engine = CipherJsEngine::new(
        fetcher.clone(),
        store.clone(),
        Arc::new(FunctionNameExtractor::new(store)),
    );

    assert_eq!(fetcher.get_signature_timestamp().await, Ok(20770));
    engine.load().await.unwrap();
    let url = engine
        .resolve_cipher_url(
            "s=CBA&sp=sig&url=https%3A%2F%2Fr1.googlevideo.com%2Fvideoplayback%3Fn%3Dabcdef%26x%3D1",
        )
        .await
        .unwrap();
    assert_eq!(
        url,
        "https://r1.googlevideo.com/videoplayback?n=fedcbaZ&x=1&alr=yes&sig=ABC"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bot_guard_parses_integrity_tokens_and_descrambles() {
    let (token, expires_in) = BotGuardService::parse_integrity_token(r#"["AQI=",43200]"#).unwrap();
    assert_eq!(token, "new Uint8Array([1,2])");
    assert_eq!(expires_in, 43200);
    assert!(BotGuardService::parse_integrity_token(r#"[null,43200]"#).is_err());
    // "AQI=" decodes to [1, 2]; +97 gives "bc".
    assert_eq!(BotGuardService::descramble("AQI."), "bc");
}
