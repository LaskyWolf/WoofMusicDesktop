# Milestones

Each milestone ends with something that runs. The riskiest pieces come first: if one fails, the plan changes before much code exists.

Swift paths refer to SwiftyTube (`SwiftyTube/…`) or WoofMusic (`Trop/…`). Dart files go in the mirror folder described in `lib/swiftytube/README.md`.

## M0: Skeleton

- [x] Flutter project for Linux and Windows
- [x] Mirror folders `lib/swiftytube`, `lib/trop`, `lib/ui` with their rules
- [x] Baseline commits in `SWIFTYTUBE_COMMIT` and `TROP_COMMIT`
- [x] CI that builds Linux and Windows and attaches them to `v*` releases
- [ ] Push to GitHub and check the first CI run

Done when: CI produces a Linux `.tar.gz` and a Windows `.zip` that open an empty window.

## M1: Glass test

Risk answered: can Flutter draw real Liquid Glass on Linux and Windows?

- [ ] Write `shaders/glass.frag`: refraction at the edges, bright rim, slight colour split
- [ ] Show an album cover with a scrolling list under a glass player bar (`BackdropFilter` + `ImageFilter.shader`)
- [ ] Fall back to a plain blur when `ImageFilter.isShaderFilterSupported` is false
- [ ] Try `liquid_glass_renderer` and compare with the custom shader; keep the simpler one

Done when: refraction is visible on KDE Wayland and Windows 11, and scrolling stays at 60 fps.

## M2: Talk to YouTube

Risk answered: does the Dart translation of SwiftyTube reach YouTube with your account?

- [ ] Translate `SwiftyTube/Core/` and `SwiftyTube/Networking/` (client context, headers, `browse`, `search`, `next`, `player`, retries, `visitorData`)
- [ ] Translate `SwiftyTube/Formatting/`
- [ ] Translate `SwiftyTube/Auth/SAPISIDAuth.swift` and the cookie store; login by pasting a cookie string
- [ ] Store the cookie with `flutter_secure_storage`
- [ ] Save real responses as JSON fixtures under `test/fixtures/` and test the translated helpers on them

Done when: `browse("FEmusic_home")` returns your own Home JSON.

## M3: Play a song

Risk answered: can the desktop get a playable stream without WKWebView?

- [ ] Translate `SwiftyTube/Streaming/` (client fallback chain, format selection)
- [ ] Translate `SwiftyTube/Cipher/` with `quickjs_engine` in place of `CipherWebView.swift`
- [ ] Translate `SwiftyTube/PoToken/` with `quickjs_engine` in place of `PoTokenGenerator.swift`
- [ ] Play the stream with `media_kit` (libmpv), audio only

Done when: 10 songs in a row play without a 403 error.

If `quickjs_engine` cannot run YouTube's code: try yt-dlp as an external tool before changing anything else.

## M4: Browse

- [ ] Translate `Trop/Networking/Models/HomePageParser.swift`, `YTItem.swift`, `Trop/Search/SearchParser.swift`
- [ ] Home screen with sections, chips and continuations
- [ ] Search with suggestions
- [ ] Album, artist, playlist and podcast pages (`Trop/Detail/`)
- [ ] Explore: new releases, moods and genres

Done when: every page of the iPhone app above shows the same items on desktop for the same account.

## M5: Library and local data

- [ ] Translate `Trop/Database/DatabaseMigrations.swift` to SQLite with the same tables
- [ ] Translate `SwiftyTube/Sync/` (library sync, incremental sync, mutations)
- [ ] Like, save album, subscribe, create/rename/edit playlists
- [ ] Listening history (`event`, `play_count`) and History screen

Done when: a like or playlist edit made on desktop shows up on the iPhone, and the other way round.

## M6: Player

- [ ] Queue, Play Next, radio
- [ ] Crossfade and AutoMix (`Trop/Audio/`)
- [ ] Lyrics providers (`Trop/Audio/Lyrics/`)
- [ ] SponsorBlock
- [ ] Media keys and system controls (MPRIS on Linux, SMTC on Windows)
- [ ] Downloads for offline play

Done when: a full listening session runs with media keys and transitions like on the iPhone.

## M7: Integrations

- [ ] Last.fm and ListenBrainz scrobbling
- [ ] Discord Rich Presence
- [ ] Odesli share links
- [ ] ListenTogether (same server and protocol as `Trop/ListenTogether/`)
- [ ] Recap

## M8: Release

- [ ] Google login in a webview (replaces cookie paste)
- [ ] System blur behind the window: Mica on Windows 11, `ext-background-effect-v1` on Wayland, blurred cover elsewhere
- [ ] App icon and name in both systems
- [ ] Packages: AppImage or Flatpak for Linux, installer for Windows
- [ ] First `v0.1.0` tag
