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

- [x] Choose the UI toolkit: iced 0.14
- [x] Window without system decorations; title bar drawn by the app, with drag, double-click to maximize, resize edges, minimize/maximize/close
- [x] Glass shader in wgpu: refraction at the edges, bright rim, slight colour split, on content scrolling behind it (`crates/app/src/glass.rs`; needs the one-line iced_wgpu patch in `vendor/`)
- [x] Check native Wayland on KDE with two monitors at different scales (125% and 100%): moving the window keeps the full size and sharp text
- [ ] Check Windows 11

Done when: the window works on KDE Wayland and Windows 11, and scrolling under the glass stays at 60 fps.

## R2: Talk to YouTube

- [x] Translate `SwiftyTube/Core/`, `Networking/`, `Formatting/` (client context, headers, `browse`, `search`, `next`, `player`, retries, `visitorData`)
- [x] Translate `SwiftyTube/Auth/` cookie parsing and SAPISID hash; store the cookie in the system keyring (`keyring`)
- [x] Tests on the fixtures, same cases as the Dart tests

Done when: `browse("FEmusic_home")` returns Home anonymously and with a cookie.

## R3: Play a song

- [x] Translate `SwiftyTube/Streaming/` (client fallback chain with VISIONOS, format selection)
- [x] Translate `SwiftyTube/Cipher/` with `rquickjs` in place of `CipherWebView.swift`
- [x] Play audio with libmpv (`libmpv2`)

Done when: 10 songs in a row play without a 403 error.

## R4: Browse

- [x] Translate the Trop parsers: Home, Explore, Search, album, artist, playlist, podcast
- [x] Sidebar, Home with real covers, player bar that plays a clicked song
- [x] Search, Explore with mood pages, album/artist/playlist pages, back button that restores the scroll position
- [x] Queue: a click plays its list from that song; next, previous, auto-advance at the song's end
- [x] Podcast page, Now Playing view tinted by the cover's colours, queue panel, seek slider
- [x] Shortcuts: Space plays or pauses, Escape closes the queue or Now Playing, Alt+Left goes back

Done when: every page opens from real data and starts playback.

## R5: Account and library

- [x] Translate the Trop database (SQLite, migrations v1–v11) with `rusqlite`
- [x] Google sign-in in a webview (`wry`) that harvests the cookies; cookie paste as fallback
- [x] Translate `SwiftyTube/Sync/` and the Trop library services: likes, playlists, albums, artists
- [x] Record plays (play time, history) so My Top 100 fills
- [x] Library screens

Done when: the library matches WoofMusic on the iPhone with the same account.

## R6: Player features

- [x] Two-deck player: Crossfade and AutoMix
- [x] Lyrics (LRCLIB, Musixmatch and the other Trop providers), romanization
- [x] Downloads for offline play
- [x] Media keys and system media panel: MPRIS on Linux, SMTC on Windows (`playwire`)
- [x] Discord presence (local IPC), Last.fm and ListenBrainz scrobbling, Odesli song links; Settings page
- [x] Everyday controls: volume (`PlayerVolumeSlider`), shuffle and repeat (`PlaybackQueue`), like button (`LikeStore`, `LikeFlavor`)
- [x] Song menu: play next, add to queue, add to playlist, go to album or artist, share, start radio (`SongActionsMenu`)
- [x] Radio options (style, length, explicit) and New Playlist in Add to Playlist (`RadioOptions`, `RadioViews`)
- [x] Restore the queue and the playing song at launch (`NowPlaying.restoreQueueIfNeeded`)
- [x] Queue editing: drag to reorder; remove is in the song menu (`QueueView`)
- [x] Playlist editing: create, rename, delete, edit songs, cover, collaborators (`PlaylistSongEdits`, `PlaylistUpload`, `PlaylistCoverUploader`, `PlaylistCollaborators`, `PlaylistRemoval`)
- [x] History page and play reports to YouTube Music history (`History/`, `YouTubeHistoryReport`)
- [x] Activity: new releases from followed artists (`Activity/`); a sidebar badge replaces the iOS notifications
- [x] Sleep timer, playback speed, equalizer (`SleepTimer`, `PlaybackSpeed`, `Equalizer`)
- [x] SponsorBlock skips the non-music parts of music videos (`SponsorBlock`)
- [x] Comments panel: read, sort, replies, like, write (`Comments/`)
- [x] Podcast episodes resume where they stopped (`EpisodePlaybackStore`)
- [x] Lyrics extras: karaoke countdown, share card, provider order (`KaraokeCountdown`, `LyricShareCard`, `LyricsProviderOrderView`)
- [x] Settings pages: Appearance, Content, Player, Lyrics, Diagnostics
- [x] Lyrics menu: Edit Lyrics and Search Lyrics sheets; karaoke fill inside the current word (`LyricsOptionsMenu`, `KaraokeLineText`)
- [x] Share card looks: other backgrounds, frames and text styles, stickers, QR code (`ShareCardStudio`); the video export is not ported
- [x] Player settings: persist queue, persistent play/pause, autoplay similar, audio normalization (`SettingsStore`, `NowPlaying`)
- [x] Accent colour, library layout and gapless playback settings (`SettingsStore`)
- [x] Profile and Recap (`Profile/`)
- [x] ListenTogether (`ListenTogether/`); not yet tried against a live metroserver

Not ported (iPhone only): CarPlay, widgets, App Intents and quick actions, haptics, swipe gestures, alternate app icons, in-app updater.

Done when: every WoofMusic iOS feature above works on desktop.

## R7: Window background

- [ ] Blur behind the window: `ext-background-effect-v1` / KWin blur on Wayland, Mica on Windows 11
- [ ] Opaque fallback elsewhere

## R8: Packages

- [ ] libmpv on Windows: download an mpv-dev build in CI and make `mpv.lib` for MSVC (the Windows CI job fails to link until then)
- [ ] AppImage for Linux, installer for Windows, built by CI on `v*` tags
- [ ] README install steps

Done when: a `v*` tag publishes both packages and they install on a clean system.
