# Milestones

Each milestone ends with something that runs. The riskiest pieces come first: if one fails, the plan changes before much code exists.

Swift paths refer to SwiftyTube (`SwiftyTube/…`) or WoofMusic (`Trop/…`). The Dart translation on branch `dev/milestones` (`lib/swiftytube`, `lib/trop`) is the reference for each Rust file: same names, same tests.

## R0: Workspace

- [x] Cargo workspace: `crates/swiftytube`, `crates/trop`, later `crates/app`
- [x] Real responses under `crates/fixtures/` (from the Dart tests)
- [x] CI: `cargo fmt --check`, `cargo clippy`, `cargo test` on Linux and Windows

Done when: CI is green on an empty workspace.

## R1: Window, title bar and glass

Risk answered: does the Rust UI toolkit give native Wayland, its own title bar and real Liquid Glass?

- [ ] Choose the UI toolkit (iced is the candidate)
- [ ] Window without system decorations; title bar drawn by the app, with drag, double-click to maximize, resize edges, minimize/maximize/close
- [ ] Glass shader in wgpu: refraction at the edges, bright rim, slight colour split, on content scrolling behind it
- [ ] Check native Wayland on KDE with two monitors at different scales (125% and 100%): moving the window keeps the full size and sharp text
- [ ] Check Windows 11

Done when: the window works on KDE Wayland and Windows 11, and scrolling under the glass stays at 60 fps.

## R2: Talk to YouTube

- [ ] Translate `SwiftyTube/Core/`, `Networking/`, `Formatting/` (client context, headers, `browse`, `search`, `next`, `player`, retries, `visitorData`)
- [ ] Translate `SwiftyTube/Auth/` cookie parsing and SAPISID hash; store the cookie in the system keyring (`keyring`)
- [ ] Tests on the fixtures, same cases as the Dart tests

Done when: `browse("FEmusic_home")` returns Home anonymously and with a cookie.

## R3: Play a song

- [ ] Translate `SwiftyTube/Streaming/` (client fallback chain with VISIONOS, format selection)
- [ ] Translate `SwiftyTube/Cipher/` with `rquickjs` in place of `CipherWebView.swift`
- [ ] Play audio with libmpv (`libmpv2`)

Done when: 10 songs in a row play without a 403 error.

## R4: Browse

- [ ] Translate the Trop parsers: Home, Explore, Search, album, artist, playlist, podcast
- [ ] Screens: sidebar, Home, Search, detail pages, player bar, now playing, with the R1 glass

Done when: every page opens from real data and starts playback.

## R5: Account and library

- [ ] Translate the Trop database (SQLite, migrations v1–v11) with `rusqlite`
- [ ] Google sign-in in a webview (`wry`) that harvests the cookies; cookie paste as fallback
- [ ] Translate `SwiftyTube/Sync/` and the Trop library services: likes, playlists, albums, artists, history
- [ ] Library screens

Done when: the library matches WoofMusic on the iPhone with the same account.

## R6: Player features

- [ ] Two-deck player: Crossfade and AutoMix
- [ ] Lyrics (LRCLIB, Musixmatch and the other Trop providers), romanization
- [ ] Downloads for offline play
- [ ] Media keys and system media panel: MPRIS on Linux, SMTC on Windows (`souvlaki`)
- [ ] Discord presence, Last.fm, ListenBrainz, ListenTogether, Odesli, Recap

Done when: every WoofMusic iOS feature in `PRODUCT.md` works on desktop.

## R7: Window background

- [ ] Blur behind the window: `ext-background-effect-v1` / KWin blur on Wayland, Mica on Windows 11
- [ ] Opaque fallback elsewhere

## R8: Packages

- [ ] AppImage for Linux, installer for Windows, built by CI on `v*` tags
- [ ] README install steps

Done when: a `v*` tag publishes both packages and they install on a clean system.
