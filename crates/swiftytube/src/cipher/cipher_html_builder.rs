// The WebView HTML wrappers become plain scripts for QuickJS:
// `buildFileBasedHTML` → `BRIDGE_SCRIPT`. The legacy inline `buildHTML` is not ported (unused).

pub struct CipherHTMLBuilder;

impl CipherHTMLBuilder {
    /// Patched player.js with window exports for the JS engine.
    /// Swift defaults: `sigIsExpression: true`, `rawNFuncBody: nil`. Swift's
    /// `playerHash` argument is dropped: the QuickJS build uses no file name.
    pub fn patch_player_js(
        player_js: &str,
        sig_config: Option<&str>,
        sig_is_expression: bool,
        n_class: Option<&str>,
        n_js_expression: Option<&str>,
        raw_n_func_body: Option<&str>,
    ) -> String {
        let mut exports = Vec::new();

        if let Some(sig_config) = sig_config {
            if sig_is_expression {
                let sig_expr = sig_config.replace("INPUT", "sig");
                exports.push(format!(
                    "window._cipherSigFunc=function(sig){{try{{return {sig_expr}}}catch(e){{return null}}}}"
                ));
            } else {
                // config is a raw function declaration - export directly
                exports.push(format!("window._cipherSigFunc={sig_config}"));
            }
        }

        // URL builder mimicking YouTube's `s2` (tries classes, alr=yes, sig, then yq/toString/url/clone).
        let nc = format!("\"{}\"", n_class.unwrap_or("lq"));
        let known_classes = match n_class {
            Some("lq") => vec![nc, "\"WM\"".into()],
            Some("WM") => vec![nc, "\"lq\"".into()],
            _ => vec![nc, "\"WM\"".into(), "\"lq\"".into()],
        };
        let url_builder_sig = match sig_config {
            Some(sig_config) if sig_is_expression => sig_config.replace("INPUT", "d"),
            Some(_) => "(window._cipherSigFunc?window._cipherSigFunc(d):d)".into(),
            None => "d".into(),
        };
        exports.push(format!(
            "window._buildSignedUrl=function(url,sp,sig){{try{{\
             var classes=[{}];\
             for(var i=0;i<classes.length;i++){{try{{\
             var u=new g[classes[i]](url,true);u.set(\"alr\",\"yes\");\
             if(sig){{try{{var d;try{{d=decodeURIComponent(sig)}}catch(e){{d=sig}};\
             u.set(sp,{url_builder_sig})}}catch(e){{}}}}\
             var s;\
             if(typeof u.yq==='function'){{s=u.yq()}}\
             else if(typeof u.toString==='function'){{s=u.toString();if(s==='[object Object]')s=null}}\
             if(!s&&u.url!==undefined){{s=u.url}}\
             if(!s&&typeof u.clone==='function'){{try{{s=u.clone()}}catch(e){{}}}}\
             if(s)return s}}catch(e){{}}}}\
             return null}}catch(e){{return null}}}}",
            known_classes.join(",")
        ));

        // n-transform via a dummy ?n= URL read back through the builder class.
        if let Some(n_js_expression) = n_js_expression {
            let expr = n_js_expression.replace("INPUT", "n");
            exports.push(format!(
                "window._nTransformFunc=function(n){{try{{return {expr}}}catch(e){{return n}}}}"
            ));
        } else if let Some(raw_n_func_body) = raw_n_func_body {
            exports.push(format!("window._nTransformFunc={raw_n_func_body}"));
        }

        // Expose metadata for discovery
        exports.push(format!(
            "window._exportedCipher={{sigFuncName:'{}',nFuncClass:'{}'}}",
            sig_config.unwrap_or("?"),
            n_class.unwrap_or("?")
        ));

        const MARKER: &str = "})(_yt_player);";
        let export_code = format!("; {}", exports.join("; "));

        if player_js.contains(MARKER) {
            return player_js.replace(MARKER, &format!("{export_code} {MARKER}"));
        }

        format!("{player_js}\n{export_code}")
    }

    /// Sig/n/url bridges, run after the patched player.js. Swift `buildFileBasedHTML`.
    pub const BRIDGE_SCRIPT: &str = r#"
// ============================================================
// SIGNATURE DEOBFUSCATION
// ============================================================
function deobfuscateSig(funcName, constantArg, obfuscatedSig) {
    var func = window._cipherSigFunc;
    if (typeof func !== 'function') return null;
    try {
        var r = func(decodeURIComponent(obfuscatedSig));
        return (r !== undefined && r !== null && r !== '') ? String(r) : null;
    } catch(e) {
        return null;
    }
}

// ============================================================
// N-PARAMETER TRANSFORM
// ============================================================
function transformN(nValue) {
    var func = window._nTransformFunc;
    if (typeof func !== 'function') return nValue;
    try {
        var result = func(nValue);
        if (result === undefined || result === null || result === '') return nValue;
        var resultStr = String(result);
        return (resultStr.length > 0) ? resultStr : nValue;
    } catch(e) {
        return nValue;
    }
}

// ============================================================
// FULL URL BUILDER - uses player's own URL builder class to
// construct URL identical to YouTube's s2 function.
// Pass baseUrl (decoded), sp param name, and URL-encoded sig.
// Returns the fully constructed URL string.
// ============================================================
function buildSignedUrl(baseUrl, sp, encodedSig) {
    if (typeof window._buildSignedUrl === 'function') {
        return window._buildSignedUrl(baseUrl, sp, encodedSig);
    }
    return null;
}

// ============================================================
// READY SIGNAL - validate the exported n-transform first
// (ported from zemer-cipher): a transform that throws or returns
// same-value/garbage output must be disabled so the native side
// strips the untransformed n-param instead of shipping a bad one.
// ============================================================
var hasTransformN = typeof window._nTransformFunc === 'function';
var discoveryInfo = 'n_transform=' + hasTransformN;
if (hasTransformN) {
    var testInput = "KdrqFlzJXl9EcCwlmEy";
    try {
        var testResult = window._nTransformFunc(testInput);
        var valid = typeof testResult === 'string' &&
            testResult !== testInput &&
            testResult.length >= 5 &&
            /^[a-zA-Z0-9_-]+$/.test(testResult);
        if (!valid) {
            window._nTransformFunc = null;
            hasTransformN = false;
            discoveryInfo = 'n_transform_invalid:' + String(testResult).substring(0, 20);
        }
    } catch(e) {
        window._nTransformFunc = null;
        hasTransformN = false;
        discoveryInfo = 'n_transform_threw:' + e;
    }
}

var info = {
    type: 'discovery',
    sigFuncName: typeof _cipherSigFunc === 'function' ? 'exported_sig' : 'NOT_FOUND',
    nFuncName: hasTransformN ? 'exported_n' : 'noop',
    info: discoveryInfo
};
// QuickJS: the discovery JSON is the script's completion value
// (WebView posted it, then a separate 'ready' message).
JSON.stringify(info);
"#;
}
