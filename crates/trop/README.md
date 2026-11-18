# trop (Rust mirror)

Rust translation of the WoofMusic app logic (`Trop/` in the WoofMusic repo): parsers, view-model data, later services and database. Screens are not translated; desktop screens live in `crates/app`.

## Rules

Same rules as `crates/swiftytube/README.md`, plus:

- One Swift file = one Rust module, same folder in snake_case: `Trop/Networking/Models/HomePageParser.swift` → `src/networking/models/home_page_parser.rs`.
- A Swift file that mixes a SwiftUI view with logic keeps only the logic, under the Dart translation's name: `Detail/AlbumDetailView.swift` → `src/detail/album_detail_parser.rs`, `Home/HomeViewModel.swift` → `src/home/home_view_model.rs` (data half only).
- Database tables keep the names and columns of `Trop/Database/DatabaseMigrations.swift`.
- `/TROP_COMMIT` holds the WoofMusic commit translated so far. Update it after each sync.

## Replacements for Swift-only code

| Swift | Rust |
| --- | --- |
| `enum YTItem { case song(SongItem) ... }` | `enum YTItem { Song(SongItem), ... }`; shared getters (`id()`, `title()`...) are methods on the enum |
| `UUID()` ids (`ExploreSection`, `MoodItem`, `SearchSection`) | `u64` from a process-wide counter (`next_id` in `home_page.rs`) |
| `@MainActor` view models with generation counters | structs with `&mut self` async methods; calls cannot interleave, so the counters are gone |
| `InnerTubeClient.tropShared` | `Arc<InnerTubeClient>` passed to each view model |
| `Task` cancellation of an older search | the caller drops the older future |
| `UIImage` drawn into a `CGContext` | 8-bit RGBA byte slices; the caller decodes and scales the image |
| In-memory caches (`ArtistDetailCache`, `MoodDetailCache`) | `static Mutex<HashMap>` with `Instant` timestamps |
| GRDB `DatabasePool` in `actor DatabaseService` | `Mutex<rusqlite::Connection>` (bundled SQLite, WAL); synchronous methods, callers run them off the UI thread |
| GRDB `DatabaseMigrator` | `database_service::migrate`: one transaction per migration, applied ids in `grdb_migrations` like GRDB |
| GRDB `create(table:)` / `alter(table:)` | the SQL GRDB generates, same names, types and defaults |
| `FetchableRecord & PersistableRecord` entities | `Record` trait (`TABLE`, `COLUMNS`, `PRIMARY_KEY`, `from_row`, `values`) in `database/mod.rs` |
| GRDB `Date` columns | `SystemTime`, stored as GRDB's UTC text `yyyy-MM-dd HH:mm:ss.SSS` (`date_text`, `DbDate`) |
| `ProfileStorage.databaseURL` | `DatabaseService::default_path()`: `$XDG_DATA_HOME` (or `~/.local/share`) / `%APPDATA%`, then `WoofMusic/trop.sqlite`; `reopen(path)` for another profile |
| Fallback temp file, then `:memory:` | `:memory:` only; `uses_temporary_storage()` reports it |
| Swift `throws` from GRDB | `rusqlite::Error`; `update` of a missing row is `QueryReturnedNoRows` (GRDB `recordNotFound`) |
| `TropLibraryStore` (`async throws` over GRDB) | `TropLibraryStore` over `Arc<DatabaseService>`, implementing SwiftyTube's synchronous `LibrarySyncStore` |
| `LibraryPages` called inside `applyArtistSync` / `applyAlbumSync` / `applyPlaylistSync` | not called there: SwiftyTube's `LibrarySyncService` already fetches every page and fails the section on a failed page, so each list is complete; `LibraryPages` stays for other callers |
| `SignedInnerTube.post("browse", ...)` in `LibraryPages` | `InnerTubeClient::browse` on the `&InnerTubeClient` passed in |
| `enum SyncBridge` with static members on `InnerTubeClient.tropShared` | `SyncBridge` struct built with `SyncBridge::new(client, db, defaults_path)` |
| `SyncBridge.running` (`Task` shared by callers) | a `tokio::sync::Mutex<()>`: a caller that finds it held waits for that sync and does not start another |
| `SettingsStore` sync flags | `SyncBridge::options()`: every section on, the Swift defaults |
| `Notification.Name.libraryDidChange` | not ported yet (no screen listens) |
| `Crossfade` singleton driving mpv with `Task`s and timers | `crossfade::Crossfade` state machine: `tick` returns an `Action`, the player owns decks, timers and tasks and reports back (`plan_ready`, `deck_loaded`, `outgoing_ended`, ...) |
| `planToken: UUID` | `u64` token in `Action::Plan`, bumped by every `reset` |
| `runFade` / `runBeatMix` / `applyStyle` loops | `fade_gains` and `BeatMix::step`, one call per 50 ms step; they return gains and changed `af-command` values (`Effect`) |
| `SettingsStore` crossfade/AutoMix fields | `TransitionSettings` with the Swift defaults (both off, 6 s, effects 1.0) |
| `FFmpegKit` decode in `Crossfade.decode` | `crossfade::ffmpeg_args` for an `ffmpeg` process, bytes through `samples_from_f32le` into `track_analyzer::analyze` |
| `TrackAnalyzer.Result` | `TrackAnalysis` (`Result` would shadow Rust's) |
| `Crossfade.history` (Diagnostics) and the plan summary line | not ported (no Diagnostics screen); `plan_ready` logs a short summary |
| `UserDefaults` in `LastFMDefaults`, `ListenBrainzDefaults` | `LastFMSettings`, `ListenBrainzSettings`: serde structs with the Swift keys, each in its own JSON file next to `sync_defaults.json` (`default_path()`) |
| `LASTFM_API_KEY` / `LASTFM_SECRET` from the environment or Info.plist | the environment at run time, else `option_env!` at build time |
| Keychain `SecureStore` in `LastFMTokenStore`, `ListenBrainz`, `Odesli` | SwiftyTube's keyring `SecureStore`, same service and account names (first profile: unscoped) |
| `CryptoKit` `Insecure.MD5` (Last.fm `api_sig`) | `md-5` crate |
| `NowPlaying.shared` read by `LastFMIntegration` and `ListenBrainz` | a `PlayingSong` snapshot the app passes to `LastFMIntegration::sync_state` (every second and on each change) and `ListenBrainz::tick` (4 times a second) |
| `LastFMIntegration` scrobble `Task.sleep` timer | a deadline checked by `sync_state`; a scrobble goes out up to one call late |
| Detached `Task` sends in `LastFMIntegration`, `ListenBrainz` | awaited inside `sync_state` / `tick` |
| `SettingsStore.isIncognito` | `incognito` field on `LastFMIntegration` and `ListenBrainz` |
| `LastFMAuthWebView` (WKWebView + polling) | `LastFMService::start_sign_in` (token + approval URL to open in the browser) and `complete_sign_in` (polls `auth.getSession` every 2 s, stores the session) |
| `MusicLink(odesliResponse:)`, `MusicLink.isConvertible` | `Odesli::youtube_link` (the URL, not a parsed `MusicLink`), `Odesli::is_convertible` |
| `enum LyricsRomanizer` with `+<Script>` extension files | `struct LyricsRomanizer`; each extension file is an `impl LyricsRomanizer` block in `audio/lyrics/romanizer/<script>.rs` |
| Swift dictionaries of letter readings (`[Character: String]`, `[UInt32: String]`) | `match` functions returning `Option<&'static str>` |
| `CFStringTransform(..., kCFStringTransformMandarinLatin, ...)` in `LyricsRomanizer+Chinese` | `pinyin` crate (`with_tone`), one reading per character, adjacent syllables space-separated |
| `protocol LyricsProvider` (`async throws`) | trait `LyricsProvider`; `fetch(&query, &session)` returns a boxed `Send` future (`LyricsFuture`) |
| `URLSession.shared` in lyrics providers | SwiftyTube's `Session`, passed to `LyricsManager::new`; tests use `Session::mock` |
| `LyricsError` vs other thrown errors (`URLError`) | one `LyricsError` enum; `Transport` and `TimedOut` are network errors, `is_answer()` is Swift's `error is LyricsError` |
| `withTaskGroup` race in `LyricsManager` | `tokio::task::JoinSet`; dropping it aborts the providers still running |
| `Decodable` responses (Kugou, Genius) and `as? Bool` | untyped JSON; `lyrics_http::as_bool` also reads 0 / 1, as `NSNumber` bridging does |
| `NSData.decompressed(using: .zlib)` (KRC) | `flate2::read::DeflateDecoder` |
| `LyricsService` reading `NowPlaying` / the queue | the caller passes a `LyricsSong` |
| `UserDefaults` in `LyricsService` | a JSON object file (`LyricsService::default_defaults_path()`: `WoofMusic/lyrics_defaults.json`), same keys |
| `LyricsState.shared` (`@MainActor @Observable`) | `LyricsService::state()` snapshot; provider names keyed by video id |
| `LyricsSettings` + `SettingsStore.disabledLyricsProviders` | `LyricsManager::settings` (`Mutex<LyricsSettings>`), not persisted yet |
| `LyricsView.updateActiveLine` / `detectGaps` | `lyrics_view.rs`: `active_line_index`, `detect_gaps`, `display_lines` |
| `String.decodingHTMLEntities()` | `StringApp` trait on `str` (`utilities/string_app.rs`) |

## Status

Translated: `Networking/Models/` (`YTItem`, `HomePage`, `HomePageParser`), `Home/HomeViewModel` (data half), `Search/` (`SearchParser`, `SearchService` network half), `Explore/` (`ExploreViewModel` data half, `MoodCard` families), `Detail/` (`DetailModels`, album, artist, playlist and podcast parsers), `Utilities/UIImage+ColorExtraction`, `Database/` (migrations v1–v11, `DatabaseService`, entities), `Services/` (`TropLibraryStore`, `LibraryPages`, `SyncBridge`), `Audio/` (`TrackAnalyzer`, `MixPlanner`, the decision logic of `Crossfade`; the player wiring waits for R6), `Audio/Lyrics/Romanizer/` (`LyricsRomanizer` and all 11 script files), `Audio/Lyrics/` (`Providers/`, `LyricsService`, `LyricsManager`, the line lookup of `LyricsView`), `Utilities/String+App` (`decodingHTMLEntities`).

Not yet: personalization sections on Home, `SearchService.localSearch`, `MoodGenreIndex`, `PlaylistHeaderCache`, saving and downloads from detail pages (all R5), GRDB `ValueObservation` live queries and their publishers, the other `Services/` files, `SyncBridge.cancelRunning` and its `AccountState.isChangingSession` guard (no profile switching yet), `LikeStore` refresh after a sync, the lyrics views (`LyricsView`, `LyricsOptionsMenu`, `LyricShareCard`, `ShareCardStudio`, `KaraokeCountdown`) and lyrics translation.

## Wiring crossfade and AutoMix

The player needs a spare `MpvDeck` and `filterChain` (R6) first. Then: call `Crossfade::tick` every 0.25 s with a `NowPlayingState` and the live `TransitionSettings`, and act on the `Action`. `Plan`: resolve the next song's URL and loudness, run `ffmpeg` with `ffmpeg_args` for `TAIL_SECONDS` from the end of the playing file and `HEAD_SECONDS` from the start of the next one, `analyze` both, then call `plan_ready` (or `plan_failed`). `LoadIncoming`: set `incoming_properties` and the `af` chain (`plan.incoming_bass_gain()`, `plan.echo_delay`) on the spare deck, `loadfile`, then `deck_loaded`/`deck_failed` from its events. `StartMix`: sleep `wait`, `begin`, unpause the spare deck, then every 50 ms either `fade_gains(fade_progress(..))` or `BeatMix::step`, send volumes through `mpv_volume` and effects with `af-command`, call `promote` at progress 0.5 (then `promoteSpare` and advance NowPlaying), and on done send `tail_cut_effects` and wait `BeatMix::tail` before `complete`. A new song or stop calls `reset`; pause, seek and speed changes call `finish_now`; the outgoing deck's EOF during a mix calls `outgoing_ended`. The analysis needs only mono 32-bit float samples at 11 025 Hz; no Apple framework or network service is involved.
Integrations translated: `LastFM/` (service, integration, defaults, token store, the logic of `LastFMAuthWebView`), `ListenBrainz/` (all but the settings view), `Odesli/` (all but the settings view). Not ported: Last.fm's legacy migrations (session key in `UserDefaults`, keychain `username` and `subscriber`), the unused `lastFMLikeChanged` observer, an offline queue for failed sends.

## Tests

`cargo test -p trop` runs offline against `crates/fixtures/trop/` and a mock session. Each test file mirrors a Dart file in `test/trop/` with the same cases. `database_test.rs` uses in-memory and temporary-file databases. `sync_services_test.rs` runs the library store on an in-memory database and the sync services on `crates/fixtures/sync/`. `scrobbling_test.rs` and `odesli_test.rs` run Last.fm, ListenBrainz and Odesli on `crates/fixtures/{last_fm,listen_brainz,odesli}/` with keyring-core's mock store. `lyrics_romanizer_test.rs` has no Swift or Dart counterpart: its expected outputs are traced from the Swift rules, and standard Hanyu Pinyin for Chinese. `lyrics_test.rs` ports `LyricsParsing.selfCheck` and runs each provider on `crates/fixtures/lyrics/` through a mock `Session`.
| `DownloadManager` (`@MainActor ObservableObject`, `@Published` state) | `DownloadManager` behind a `Mutex`; the app reads `downloads()` on a timer while a download runs |
| `DownloadManager` ffmpeg download, AAC remux, Opus→AAC transcode, embedded cover | one `reqwest` GET per 10 MB `Range` piece; the stream is saved as sent (`.m4a` or `.webm`), mpv plays both |
| `DownloadManager.reload` orphan adoption, Wi-Fi Only, batch progress | not ported; `tracks()` drops rows whose file is gone |
| `DownloadedCollections` (album and playlist download badges) | not ported (no screen shows them) |
