//! Port of the in-scope cases of the Dart `test/swiftytube/auth_test.dart`.
//! `CookieHarvest` and `LoginWebviewHarvest` cases come with the webview login (R5).

use std::collections::BTreeMap;

use swiftytube::auth::cookie_parser::CookieParser;
use swiftytube::auth::sapisid_auth::SAPISIDAuth;
use swiftytube::auth::session_importer::{SessionImportError, SessionImporter};

#[test]
fn sapisidhash_is_timestamp_and_sha1_of_ts_sapisid_origin() {
    // python3: hashlib.sha1(b'1700000000 SAPISID_VALUE https://music.youtube.com').hexdigest()
    assert_eq!(
        SAPISIDAuth::authorization_header_at(
            "SAPISID_VALUE",
            "https://music.youtube.com",
            1_700_000_000
        ),
        "SAPISIDHASH 1700000000_f6e5594a37222bac438723f36abaf1d27f31d7d4"
    );
}

#[test]
fn cookie_parser_splits_pairs_and_drops_empty_names_and_values() {
    let expected: BTreeMap<String, String> = [("SID", "a"), ("HSID", "b=="), ("PREF", "f6=1&tz=X")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    assert_eq!(
        CookieParser::parse(" SID=a; HSID=b==; bad; =x; y=; PREF=f6=1&tz=X "),
        expected
    );
}

#[test]
fn session_importer_prefers_secure_3psapisid_and_rejects_empty_input() {
    let state = SessionImporter::import_session(
        "SAPISID=old; __Secure-3PSAPISID=new; visitor_data=V",
        None,
    )
    .unwrap();
    assert_eq!(state.sapisid.as_deref(), Some("new"));
    assert_eq!(state.visitor_data.as_deref(), Some("V"));
    assert_eq!(
        SessionImporter::import_session("", None),
        Err(SessionImportError::InvalidCookie)
    );
}
