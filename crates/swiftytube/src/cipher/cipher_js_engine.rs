// Replaces CipherWebView.swift: QuickJS (`rquickjs`) runs the patched
// player.js instead of a hidden WKWebView. QuickJS has no DOM, so
// `BROWSER_SHIM` stubs the browser globals player.js touches while it loads.

use std::sync::{Arc, LazyLock};

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use regex::Regex;
use rquickjs::context::EvalOptions;
use rquickjs::{CaughtError, Context, Runtime};

use super::cipher_error::CipherError;
use super::cipher_html_builder::CipherHTMLBuilder;
use super::function_name_extractor::FunctionNameExtractor;
use super::player_config::PlayerConfig;
use super::player_config_store::PlayerConfigStore;
use super::player_js_fetcher::PlayerJsFetcher;
use crate::core::swiftytube_log::SwiftyTubeLog;
use crate::po_token::js_escaping::JSEscaping;

/// Foundation's `.urlQueryAllowed`: everything printable but these stays as is.
const URL_QUERY_ALLOWED: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

static N_PARAM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[?&]n=([^&]+)").unwrap());

#[derive(Default)]
struct State {
    is_ready: bool,
    player_hash: Option<String>,
    built_config_epoch: u64,
    /// The context keeps its runtime alive.
    context: Option<Context>,
}

/// QuickJS runtime running patched player.js for sig/n-transform calls.
pub struct CipherJsEngine {
    /// Swift `@MainActor` class with a `loadTask`: one tokio `Mutex` serializes
    /// loads and calls instead.
    state: tokio::sync::Mutex<State>,
    fetcher: Arc<PlayerJsFetcher>,
    config_store: Arc<PlayerConfigStore>,
    extractor: Arc<FunctionNameExtractor>,
}

impl CipherJsEngine {
    pub fn shared() -> Arc<CipherJsEngine> {
        static SHARED: LazyLock<Arc<CipherJsEngine>> = LazyLock::new(|| {
            Arc::new(CipherJsEngine::new(
                PlayerJsFetcher::shared(),
                PlayerConfigStore::shared(),
                FunctionNameExtractor::shared(),
            ))
        });
        SHARED.clone()
    }

    pub fn new(
        fetcher: Arc<PlayerJsFetcher>,
        config_store: Arc<PlayerConfigStore>,
        extractor: Arc<FunctionNameExtractor>,
    ) -> Self {
        Self {
            state: tokio::sync::Mutex::default(),
            fetcher,
            config_store,
            extractor,
        }
    }

    pub async fn load(&self) -> Result<(), CipherError> {
        let hash = self.fetcher.get_player_hash().await?;
        let epoch = self.config_store.epoch().await;
        let mut state = self.state.lock().await;
        if state.is_ready
            && state.player_hash.as_deref() == Some(hash.as_str())
            && state.built_config_epoch == epoch
        {
            return Ok(());
        }
        self.perform_load(&mut state, hash).await
    }

    fn strip_parens(js: &str) -> &str {
        js.strip_prefix('(')
            .and_then(|js| js.strip_suffix(')'))
            .unwrap_or(js)
    }

    async fn perform_load(&self, state: &mut State, hash: String) -> Result<(), CipherError> {
        state.is_ready = false;
        state.player_hash = Some(hash.clone());
        state.built_config_epoch = self.config_store.epoch().await;

        let player_js = self.fetcher.get_player_js().await?;

        let mut sig_config = None;
        let mut n_class = None;
        let mut n_js_expression = None;
        let mut raw_n_func_body = None;
        let is_expression;
        let mut config = self.config_store.config(&hash).await;
        if config.is_none() {
            self.config_store.force_refresh(&hash).await;
            config = self.config_store.config(&hash).await;
        }
        if let Some(config) = config {
            sig_config = config.sig_function().body;
            n_class = config.n_function().var_name;
            n_js_expression = config.n_js_expression();
            is_expression = true;
            SwiftyTubeLog::debug(&format!(
                "Config found: sig={}, nClass={}",
                sig_config.as_deref().unwrap_or("?"),
                n_class.as_deref().unwrap_or("?")
            ));
        } else {
            SwiftyTubeLog::debug(&format!(
                "No config for hash {hash}, trying heuristic extraction"
            ));
            let extracted = self.extractor.extract(&player_js, Some(&hash)).await;
            is_expression = false;
            if extracted.sig_js.is_empty() {
                SwiftyTubeLog::notice("Heuristic extraction failed, using fallback");
            } else {
                sig_config = Some(Self::strip_parens(&extracted.sig_js).to_string());
                SwiftyTubeLog::debug("Heuristic extraction succeeded");
            }
            if let Some(n_class_heuristic) = extracted.n_class {
                SwiftyTubeLog::debug(&format!(
                    "Extracted nClass={n_class_heuristic} heuristically"
                ));
                n_js_expression = PlayerConfig {
                    n_class: Some(n_class_heuristic.clone()),
                    ..PlayerConfig::default()
                }
                .n_js_expression();
                n_class = Some(n_class_heuristic);
            } else if let Some(n_js) = extracted.n_js {
                raw_n_func_body = Some(Self::strip_parens(&n_js).to_string());
                SwiftyTubeLog::debug("Extracted n-transform function heuristically");
            } else {
                SwiftyTubeLog::debug("No n-transform found via heuristics");
            }
        }

        let patched_js = CipherHTMLBuilder::patch_player_js(
            &player_js,
            sig_config.as_deref(),
            is_expression,
            n_class.as_deref(),
            n_js_expression.as_deref(),
            raw_n_func_body.as_deref(),
        );

        state.context = None;
        // ponytail: evaluating ~3 MB of player.js blocks this tokio worker for
        // about a second, once per player hash. Move it to `spawn_blocking` if
        // the UI stutters on the first cipher resolve.
        let context = Self::build_context(&patched_js)?;
        let discovery: Result<String, String> =
            context.with(|ctx| eval(&ctx, CipherHTMLBuilder::BRIDGE_SCRIPT));
        let discovery = discovery.map_err(CipherError::JsExecutionFailed)?;
        if let Ok(info) = serde_json::from_str::<serde_json::Value>(&discovery) {
            SwiftyTubeLog::info(&format!(
                "Cipher discovery: sig={} n={} ({})",
                info["sigFuncName"], info["nFuncName"], info["info"]
            ));
        }
        state.context = Some(context);
        state.is_ready = true;
        SwiftyTubeLog::debug(&format!("Ready (QuickJS, hash={hash})"));
        Ok(())
    }

    fn build_context(patched_js: &str) -> Result<Context, CipherError> {
        let failed = |e: rquickjs::Error| CipherError::JsExecutionFailed(e.to_string());
        let runtime = Runtime::new().map_err(failed)?;
        let context = Context::full(&runtime).map_err(failed)?;
        for (name, script) in [("shim", Self::BROWSER_SHIM), ("player", patched_js)] {
            context
                .with(|ctx| eval::<rquickjs::Value>(&ctx, script).map(|_| ()))
                .map_err(|e| CipherError::JsExecutionFailed(format!("{name}: {e}")))?;
        }
        Ok(context)
    }

    pub async fn resolve_cipher_url(&self, cipher_text: &str) -> Result<String, CipherError> {
        let state = self.state.lock().await;
        let context = state
            .context
            .as_ref()
            .ok_or_else(|| CipherError::JsExecutionFailed("runtime is nil".into()))?;
        let evaluate = |script: String| Self::evaluate_js(context, &script);

        let params = Self::parse_query_string(cipher_text);
        let url_param = params
            .iter()
            .find(|(k, _)| *k == "url")
            .map(|(_, v)| *v)
            .ok_or_else(|| CipherError::InvalidResponse("No url in cipher text".into()))?;
        let param = |name: &str| params.iter().find(|(k, _)| *k == name).map(|(_, v)| *v);
        let sig_encoded = param("s");
        let sp_param = param("sp").unwrap_or("signature");

        let decoded_url = percent_decode_str(url_param)
            .decode_utf8()
            .map_err(|_| CipherError::InvalidResponse("Could not decode URL".into()))?
            .into_owned();

        let mut url = decoded_url.clone();
        if let Some(sig) = sig_encoded {
            let built_url = evaluate(format!(
                "buildSignedUrl({}, {}, {})",
                escape_js(&decoded_url),
                escape_js(sp_param),
                escape_js(sig)
            ))?;
            if let Some(built_url) = built_url.filter(|u| u.starts_with("http")) {
                url = built_url;
            } else {
                let Some(deobfuscated) =
                    evaluate(format!("deobfuscateSig(null,null,{})", escape_js(sig)))?
                else {
                    return Err(CipherError::DeobfuscationFailed(
                        "sig deobfuscation returned nil (no cipher function exported)".into(),
                    ));
                };
                let separator = if url.contains('?') { '&' } else { '?' };
                let encoded_sig = utf8_percent_encode(&deobfuscated, URL_QUERY_ALLOWED);
                url = format!("{url}{separator}{sp_param}={encoded_sig}");
            }
        }

        if let Some(n_value) = N_PARAM.captures(&url).map(|c| c[1].to_string()) {
            let transformed = evaluate(format!("transformN({})", escape_js(&n_value)))
                .ok()
                .flatten()
                .filter(|t| !t.is_empty() && *t != n_value);
            let n = regex::escape(&n_value);
            // Rust's regex has no lookaround: the `[?&]` before and the `&`/end
            // after are captured and put back instead.
            if let Some(transformed) = transformed {
                let pattern = Regex::new(&format!("([?&])n={n}(&|$)")).expect("escaped pattern");
                url = pattern
                    .replace_all(&url, |c: &regex::Captures| {
                        format!("{}n={transformed}{}", &c[1], &c[2])
                    })
                    .into_owned();
            } else {
                let pattern = Regex::new(&format!("&?n={n}(&|$)")).expect("escaped pattern");
                url = pattern.replace_all(&url, "$1").replace("?&", "?");
                if url.ends_with('?') {
                    url.pop();
                }
            }
        }

        Ok(url)
    }

    // MARK: - Private

    /// Evaluates a JS expression; None when it returns null/undefined.
    /// WebView returned `result as? String`; here non-strings come back as their `String()` text.
    fn evaluate_js(context: &Context, script: &str) -> Result<Option<String>, CipherError> {
        let wrapped = format!("(function(){{var r=({script});return r==null?null:String(r)}})()");
        context
            .with(|ctx| eval(&ctx, &wrapped))
            .map_err(CipherError::JsExecutionFailed)
    }

    /// Raw `key=value` pairs, values still percent-encoded.
    fn parse_query_string(text: &str) -> Vec<(&str, &str)> {
        text.split('&')
            .filter_map(|pair| pair.split_once('='))
            .filter(|(key, _)| !key.is_empty())
            .collect()
    }

    /// Browser globals player.js reads at load time. Only load-time access
    /// matters: the cipher functions themselves are pure string code.
    pub const BROWSER_SHIM: &str = r#"
var window = globalThis; var self = globalThis;
globalThis.location = {href:'https://www.youtube.com/watch?v=dQw4w9WgXcQ',protocol:'https:',host:'www.youtube.com',hostname:'www.youtube.com',origin:'https://www.youtube.com',pathname:'/watch',search:'?v=dQw4w9WgXcQ',hash:'',port:''};
globalThis.navigator = {userAgent:'Mozilla/5.0',language:'en-US',languages:['en-US'],platform:'Win32',cookieEnabled:true,onLine:true};
function _el(){return {style:{},setAttribute:function(){},getAttribute:function(){return null},appendChild:function(c){return c},removeChild:function(){},addEventListener:function(){},removeEventListener:function(){},getElementsByTagName:function(){return []},querySelector:function(){return null},querySelectorAll:function(){return []},classList:{add:function(){},remove:function(){},contains:function(){return false}},childNodes:[],children:[],innerHTML:'',textContent:'',canPlayType:function(){return ''}};}
globalThis.document = {location:globalThis.location,cookie:'',referrer:'',title:'',readyState:'complete',documentElement:_el(),body:_el(),head:_el(),createElement:_el,createTextNode:_el,getElementById:function(){return null},getElementsByTagName:function(){return []},getElementsByClassName:function(){return []},querySelector:function(){return null},querySelectorAll:function(){return []},addEventListener:function(){},removeEventListener:function(){},createEvent:function(){return {initEvent:function(){}}}};
globalThis.addEventListener=function(){};globalThis.removeEventListener=function(){};
globalThis.setTimeout=globalThis.setTimeout||function(){return 0};globalThis.clearTimeout=globalThis.clearTimeout||function(){};
globalThis.setInterval=globalThis.setInterval||function(){return 0};globalThis.clearInterval=globalThis.clearInterval||function(){};
globalThis.console = globalThis.console || {log:function(){},warn:function(){},error:function(){},info:function(){},debug:function(){}};
globalThis.XMLHttpRequest=function(){};globalThis.Image=function(){return _el()};
globalThis.localStorage=globalThis.sessionStorage={getItem:function(){return null},setItem:function(){},removeItem:function(){}};
globalThis.performance=globalThis.performance||{now:function(){return Date.now()},timing:{}};
globalThis.screen={width:1920,height:1080};
globalThis.history={};globalThis.top=globalThis;globalThis.parent=globalThis;
"#;
}

/// Sloppy-mode global eval (player.js relies on `this` being the global object),
/// with the JS exception as the error text.
fn eval<'js, V: rquickjs::FromJs<'js>>(
    ctx: &rquickjs::Ctx<'js>,
    script: &str,
) -> Result<V, String> {
    let mut options = EvalOptions::default();
    options.strict = false;
    ctx.eval_with_options(script, options)
        .map_err(|e| CaughtError::from_error(ctx, e).to_string())
}

fn escape_js(string: &str) -> String {
    format!("\"{}\"", JSEscaping::escape(string))
}
