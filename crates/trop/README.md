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
| `enum LyricsRomanizer` with `+<Script>` extension files | `struct LyricsRomanizer`; each extension file is an `impl LyricsRomanizer` block in `audio/lyrics/romanizer/<script>.rs` |
| Swift dictionaries of letter readings (`[Character: String]`, `[UInt32: String]`) | `match` functions returning `Option<&'static str>` |
| `CFStringTransform(..., kCFStringTransformMandarinLatin, ...)` in `LyricsRomanizer+Chinese` | `pinyin` crate (`with_tone`), one reading per character, adjacent syllables space-separated |

## Status

Translated: `Networking/Models/` (`YTItem`, `HomePage`, `HomePageParser`), `Home/HomeViewModel` (data half), `Search/` (`SearchParser`, `SearchService` network half), `Explore/` (`ExploreViewModel` data half, `MoodCard` families), `Detail/` (`DetailModels`, album, artist, playlist and podcast parsers), `Utilities/UIImage+ColorExtraction`, `Database/` (migrations v1–v11, `DatabaseService`, entities), `Services/` (`TropLibraryStore`, `LibraryPages`, `SyncBridge`), `Audio/Lyrics/Romanizer/` (`LyricsRomanizer` and all 11 script files).

Not yet: personalization sections on Home, `SearchService.localSearch`, `MoodGenreIndex`, `PlaylistHeaderCache`, saving and downloads from detail pages (all R5), GRDB `ValueObservation` live queries and their publishers, the other `Services/` files, `SyncBridge.cancelRunning` and its `AccountState.isChangingSession` guard (no profile switching yet), `LikeStore` refresh after a sync.

## Tests

`cargo test -p trop` runs offline against `crates/fixtures/trop/` and a mock session. Each test file mirrors a Dart file in `test/trop/` with the same cases. `database_test.rs` uses in-memory and temporary-file databases. `sync_services_test.rs` runs the library store on an in-memory database and the sync services on `crates/fixtures/sync/`. `lyrics_romanizer_test.rs` has no Swift or Dart counterpart: its expected outputs are traced from the Swift rules, and standard Hanyu Pinyin for Chinese.
