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
| In-memory caches (`ArtistDetailCache`, `MoodDetailCache`) | `static Mutex<HashMap>` with `Instant` timestamps |

## Status

Translated: `Networking/Models/` (`YTItem`, `HomePage`, `HomePageParser`), `Home/HomeViewModel` (data half), `Search/` (`SearchParser`, `SearchService` network half), `Explore/` (`ExploreViewModel` data half, `MoodCard` families), `Detail/` (`DetailModels`, album, artist, playlist and podcast parsers).

Not yet: personalization sections on Home, `SearchService.localSearch`, `MoodGenreIndex`, `PlaylistHeaderCache`, saving and downloads from detail pages (all R5).

## Tests

`cargo test -p trop` runs offline against `crates/fixtures/trop/` and a mock session. Each test file mirrors a Dart file in `test/trop/` with the same cases.
