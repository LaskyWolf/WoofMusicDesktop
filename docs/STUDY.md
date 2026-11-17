# WoofMusic Desktop: preliminary study

Desktop companion of WoofMusic (iOS) for Linux and Windows. This document covers three questions: how to get and parse the data, how to get real Liquid Glass, and which stack runs on both systems.

Paths starting with `Trop/` refer to the WoofMusic repository. Paths starting with `SwiftyTube/` refer to the SwiftyTube package, version 0.0.9.

## 1. Data

### Where the iOS app gets its data

Almost all YouTube logic lives in the external package **SwiftyTube 0.0.9** (`github.com/686udjie/SwiftyTube`). WoofMusic only wires it up (`Trop/Networking/SwiftyTubeShared.swift`) and adds its own parsers for Home, items and Search.

| Piece | How it works in WoofMusic | What the desktop needs |
| --- | --- | --- |
| API | InnerTube, the private JSON API of the YouTube website: `POST https://music.youtube.com/youtubei/v1/{browse,search,next,player}`, client `WEB_REMIX`, no API key, `visitorData` refreshed from every response | Same calls. They are plain HTTPS + JSON |
| Parsing | Manual walk of untyped JSON (`[String: Any]`). Renderers: `musicResponsiveListItemRenderer`, `musicTwoRowItemRenderer`, `musicShelfRenderer`, `musicCarouselShelfRenderer`, headers, `continuationItemRenderer`. Files: `Trop/Networking/Models/HomePageParser.swift`, `Trop/Networking/Models/YTItem.swift`, `Trop/Search/SearchParser.swift`, `SwiftyTube/Sync/LibraryBrowseParser.swift` | A port of those parsers, or a library that already has them |
| Audio stream | Tries clients in order (VISIONOS → ANDROID_VR → TV embedded → IOS → TV → ANDROID → WEB_REMIX) and keeps the first URL that works (`SwiftyTube/Streaming/ClientFallbackChain.swift`) | Same fallback chain |
| Signature and `n` param | Downloads YouTube's `base.js` and runs it in a hidden WKWebView (`SwiftyTube/Cipher/CipherWebView.swift`) | A JavaScript engine |
| PoToken | BotGuard challenge run in a WKWebView (`SwiftyTube/PoToken/PoTokenGenerator.swift`) | A JavaScript engine |
| Player | libmpv through MPVKit, audio only, gapless (`Trop/Audio/PlayerController/PlayerController.swift`) | libmpv exists on Linux and Windows |
| Login | Google login in a webview, cookies harvested, header `SAPISIDHASH <ts>_<sha1(ts SAPISID origin)>` (`Trop/Auth/LoginWebView.swift`, `SwiftyTube/Auth/SAPISIDAuth.swift`). No OAuth | A webview for login + secure storage (Secret Service on Linux, Credential Manager on Windows) |
| Local data | GRDB / SQLite, one `trop.sqlite` per profile. Tables `song`, `album`, `artist`, `playlist`, `event`, `play_count`, `format`, `downloaded_track`, podcasts (`Trop/Database/DatabaseMigrations.swift`) | SQLite: the same schema can be reused |

ELI5 on the signature: YouTube scrambles stream URLs with a JavaScript function that changes every few weeks. Instead of rewriting that function, the app downloads YouTube's own code and runs it. The desktop app must do the same, so it needs a JavaScript engine.

### What the phone and the desktop can share

- **Shared through YouTube**: library, playlists, likes, subscriptions, YouTube history. Logging in with the same Google account is enough.
- **Not shared**: local history (`event`, `play_count`), Recap, downloads, AutoMix analysis, settings. The iOS app has no iCloud, no export and no local network server.
- **Only live channel**: ListenTogether, a WebSocket with hand-written protobuf to `wss://metroserverx.meowery.eu/ws` (`Trop/ListenTogether/ListenTogether.swift:25`). It is Metrolist's public server, which we don't control. The desktop could join a room to follow or drive playback, but it depends on that server.

### Data options for the desktop

| Option | Pros | Cons |
| --- | --- | --- |
| **A. Port the SwiftyTube parsers** to the chosen language | Same behaviour as iOS, same bugs fixed in one place of thought | Fragile code to rewrite (types guessed from `pageType` and column positions); decipher and PoToken still need a JS engine |
| **B. Existing library**: `rustypipe` (Rust, YouTube Music + botguard CLI for PoToken) or `youtubei.js` (TypeScript, JS engine built in) | Someone else follows YouTube's changes | Different data model from iOS; must be mapped |
| **C. `yt-dlp` for the stream only** + own InnerTube parsers for browse/search | yt-dlp is the most maintained extractor | External Python binary; recent versions need an external JS runtime (Deno) for YouTube: verify before relying on it |

Decision: **A, as a file-by-file Dart mirror of SwiftyTube**. 91% of SwiftyTube (about 10,270 of 11,235 lines) uses only Foundation; the 964 Apple-only lines are WebKit (cipher, PoToken, login), Keychain and CryptoKit. SwiftyTube walks JSON as `[String: Any]`, and Dart does the same with `Map<String, dynamic>`, so translation is close to line by line and an upstream diff maps to the same Dart file. RustyPipe was rejected: it has no write actions, no Home, no Explore, no podcasts, and it adds a second upstream to follow.

## 2. Liquid Glass on desktop

### What "real" Liquid Glass means

On iOS, Liquid Glass bends (refracts) the **app's own content** under the control: the album cover behind the player bar, the list behind the tab bar. It also has a bright edge (specular highlight) and slight colour separation at the borders.

No desktop OS lets an app read the pixels **behind** its window (other windows, wallpaper). The OS only offers a blur:

| OS | System effect | Notes |
| --- | --- | --- |
| Windows 11 | Mica, Acrylic (DWM) | Blur + tint, no refraction. Acrylic also on Windows 10 1809+, slow while dragging |
| Linux Wayland | `ext-background-effect-v1` protocol | KDE Plasma 6.7, GNOME 51 (Mutter), niri 26.04. Blur only. Other compositors: nothing |
| Linux X11 | Depends on the compositor | Not reliable |

So "real" glass on desktop has two layers:

1. **Window background**: system blur (Mica / Wayland blur) with a fallback to a blurred, darkened album cover when the system has none.
2. **Controls**: our own refraction shader running on the app's content. This is the part that looks like iOS, and it works the same on every OS because the app draws it itself.

### Which renderers can run a shader on the content behind a control

| Renderer | Backdrop shader | Linux and Windows identical? |
| --- | --- | --- |
| Flutter (Impeller) | `BackdropFilter` + `ImageFilter.shader` with a GLSL fragment shader. Package `liquid_glass_renderer` does exactly this but declares Windows/Linux unsupported; Impeller became default on desktop in Flutter 3.47 | Yes, same renderer. Check `ImageFilter.isShaderFilterSupported` at runtime |
| Qt Quick (QML) | `ShaderEffect` / `MultiEffect` on a `layer` texture | Yes |
| Chromium (Electron) | CSS `backdrop-filter: url(#svg-filter)` with `feDisplacementMap` | Yes, Chromium on both |
| Tauri | WebView2 (Chromium) on Windows, WebKitGTK on Linux. SVG filters in `backdrop-filter` are reported as Chromium only | **No**: glass on Windows, plain blur on Linux |

## 3. Stack for Linux + Windows

| Stack | Glass | Player | YouTube data | Size / weight | Fit |
| --- | --- | --- | --- | --- | --- |
| **Flutter + media_kit** | Shader on Impeller, same on both OS | `media_kit` uses libmpv, the same engine as iOS | Dart mirror of SwiftyTube + `quickjs_engine` for cipher and PoToken | Medium | **Chosen** |
| Electron + youtubei.js | CSS/SVG, same on both OS | libmpv via native module or HTML audio | `youtubei.js` covers most of it | Heavy (~150 MB, high RAM) | Good, heavy |
| Tauri + rustypipe | Inconsistent (see above) | libmpv via Rust binding | `rustypipe` | Light | Glass fails on Linux |
| Qt/QML + libmpv | Good | libmpv (classic integration) | Own C++/Python port | Medium | Steeper learning curve |

Why Flutter: one renderer draws the same pixels on Linux and Windows, so the glass shader is written once. media_kit gives libmpv, the same player as iOS, so AutoMix and crossfade logic map one to one. Main risk: `liquid_glass_renderer` does not officially support desktop yet.

Why not Tauri, even though it is light: on Linux the WebKitGTK engine does not run the SVG refraction filter, so the main visual requirement fails on half the targets.

## 4. Open decisions

- **What "companion" means**: a standalone desktop player (own playback, same account), or a remote for the phone (controls WoofMusic on iPhone). The second needs a channel that does not exist today: ListenTogether through a third-party server, or a new local server in the iOS app.
- **Stack**: confirm Flutter after the spikes below.

## 5. Spikes before writing the app

Each spike answers one risk. Do them before any real screen.

1. **Glass**: Flutter app with an album cover and a glass bar on top, using `liquid_glass_renderer` or an own GLSL shader. Run it on Linux (KDE Wayland) and Windows 11. Pass = refraction visible on both, 60 fps while scrolling.
2. **Stream**: from a `videoId`, get a playable audio URL and play it with media_kit. Try yt-dlp first, then a Dart/JS decipher. Pass = 10 songs in a row play without 403 errors.
3. **Login**: Google login in a webview, harvest cookies, call `browse` with `FEmusic_liked_videos`. Pass = liked songs list matches the phone.
4. **Window blur**: Mica on Windows 11 and `ext-background-effect-v1` on KDE. Pass = blurred wallpaper behind a transparent window, fallback on other systems.

## 6. Switch to Rust

The first build used Flutter 3.50 (branch `dev/milestones`). Its new Wayland renderer keeps the buffer scale it was created with, so a window that moves to a monitor with another scale is drawn at the wrong size ([flutter#196476](https://github.com/flutter/flutter/issues/196476), open, no fix). Native Wayland is a requirement, so the app is rewritten in Rust.

- Logic: a Rust mirror of SwiftyTube and Trop, file by file, with the same rules as the Dart mirror. Swift enums, optionals and structs map to Rust ones.
- Window: winit, which supports native Wayland and fractional scaling.
- Glass: a wgpu shader that samples the content rendered behind the glass.
- Player: libmpv (`libmpv2` crate), the same engine as the Flutter build.
- The Dart code on `dev/milestones` is the reference for the port: it already maps every Swift file and has tests on real responses.

## Sources

- [ext-background-effect-v1 protocol](https://wayland.app/protocols/ext-background-effect-v1)
- [GNOME Mutter background blur (Phoronix)](https://phoronix.com/news/GNOME-Mutter-Background-Blur)
- [Niri 26.04 blur support](https://www.neowin.net/amp/niri-2604-lands-with-long-awaited-blur-support/)
- [window-vibrancy (Mica/Acrylic)](https://docs.rs/window-vibrancy/latest/src/window_vibrancy/lib.rs.html)
- [liquid_glass_renderer](https://pub.dev/packages/liquid_glass_renderer)
- [liquid_glass_widgets](https://pub.dev/packages/liquid_glass_widgets)
- [What's new in Flutter 3.47](https://flutter.dev/blog/whats-new-in-flutter-3-47)
- [Flutter Impeller docs](https://docs.flutter.dev/perf/impeller)
- [ImageFilter API](https://api.flutter.dev/flutter/dart-ui/ImageFilter-class.html)
- [Liquid glass in CSS and SVG](https://medium.com/ekino-france/liquid-glass-in-css-and-svg-839985fcb88d)
- [Liquid glass on the web, 6 ways](https://dev.to/devyatov/liquid-glass-on-the-web-6-ways-to-build-it-with-css-and-svg-3m07)
- [RustyPipe README](https://code.thetadev.de/ThetaDev/rustypipe/raw/branch/main/README.md)
- [KWin glass effect](https://github.com/4v3ngR/kwin-effects-glass)
