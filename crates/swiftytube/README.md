# swiftytube (Rust mirror)

Rust translation of [SwiftyTube](https://github.com/686udjie/SwiftyTube), the YouTube Music client used by WoofMusic.

## Rules

- One Swift file = one Rust module, same folder in snake_case: `Networking/InnerTubeClient.swift` → `src/networking/inner_tube_client.rs`. `InnerTubeClient+Actions.swift` → `inner_tube_client_actions.rs` (a second `impl InnerTubeClient` block).
- Keep type names as in Swift (`InnerTubeJSON`, `SAPISIDAuth`) and function/field names in snake_case (`runsText` → `runs_text`), so a Swift diff maps to the Rust file line by line.
- Swift `static let` constants become associated consts in SCREAMING_SNAKE_CASE: `YouTubeClient.webRemix` → `YouTubeClient::WEB_REMIX`, `androidVr1_65_10` → `ANDROID_VR_1_65_10`.
- Swift `enum` namespaces (`enum CountFormat`) become unit structs with associated functions. Swift `extension Int` members become traits on `i64` (`CountFormatInt`, `DurationFormatInt`).
- JSON stays untyped, like Swift's `[String: Any]`: `JsonDict` is `serde_json::Map<String, Value>`. `as_dict`, `as_dict_list` and `as_string` in `inner_tube_decode.rs` stand in for `as?` casts.
- Swift default parameters: endpoints with many of them take an args struct with `Default` (`BrowseArgs`, `SearchArgs`, `NextArgs`, `PlayerArgs`); the others take `Option` arguments in Swift order.
- Swift `throws` becomes `Result<_, SomeError>` with `thiserror`.
- No desktop UI code here. It goes in `crates/app`.

## Replacements for Apple-only code

| Swift | Rust |
| --- | --- |
| `actor InnerTubeClient` | `&self` methods; config and auth behind `Mutex`es that are never held across `.await` |
| `URLSession` | `Session` in `http_client.rs`: `reqwest::Client` (rustls, gzip), or a mock closure for tests |
| `URLError` thrown as is | `InnerTubeError::Transport` / `HttpError::Transport` wrap `reqwest::Error` |
| `os.Logger` | `log` crate, target `SwiftyTube.InnerTube`; `notice` maps to `warn` |
| `CryptoKit` SHA1 | `sha1` crate |
| `NSRegularExpression` | `regex` crate; the lookbehind `(?<=[sh]\d+)-c` in `ArtworkURLs` becomes a capture group |
| `Auth/SecureStore.swift` (Keychain) | `keyring` (Secret Service on Linux, Credential Manager on Windows) through `keyring-core`'s default store |
| `[String: String]` cookies | `BTreeMap`: the `Cookie` header is sorted by name (Swift order is random) |
| `Int.random`, `UUID()` | `random_u64()` in `retry_policy.rs` (std `RandomState`, not cryptographic) |
| `Task.sleep` | `tokio::time::sleep` |
| `Codable` (`AuthState`) | `serde` derive, same JSON keys |
| `Decodable` in `playerResponse` | `serde::de::DeserializeOwned` |

`SecureAuthStore` (in `secure_store.rs`) is not in Swift: it stores one JSON `AuthState` and implements `AuthStateProvider`, as the Dart translation did.

## Status

Translated: `Core/`, `Networking/`, `Formatting/`, and from `Auth/`: `AccountInfo`, `AuthState`, `AuthStateProvider`, `CookieParser`, `SAPISIDAuth`, `SecureStore`, `SessionImporter`. `Streaming/PlayerRequestOptions` is here because `InnerTubeClient::player` takes it.

Not yet, each in its own folder when its milestone starts:

| Folder / file | Milestone |
| --- | --- |
| `Streaming/`, `Cipher/` (`rquickjs` in place of `CipherWebView.swift`), `PoToken/` | R3 |
| `InnerTubeClient.fetchDuration` / `resolveDuration` (need `Streaming/`) | R3 |
| `Sync/`, `InnerTubeClient.verifySession` (needs `Sync/BrowseLens`) | R5 |
| `Auth/CookieHarvest`, `LoginWebviewHarvest`, `SessionTokenRefresher` (webview login) | R5 |
| `YouTube/` (main-YouTube parsers, comments) | not planned |

## Tests

`cargo test -p swiftytube` runs offline against `crates/fixtures/` and a mock session. `cargo test -p swiftytube -- --ignored` also calls YouTube Music anonymously.

## Updating

`/SWIFTYTUBE_COMMIT` holds the SwiftyTube commit translated so far.

1. In the SwiftyTube repo: `git diff $(cat /path/to/SWIFTYTUBE_COMMIT)..<new> -- SwiftyTube/`
2. Translate each changed hunk into the matching Rust file.
3. Write the new commit hash into `SWIFTYTUBE_COMMIT`.
