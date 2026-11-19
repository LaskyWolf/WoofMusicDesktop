# WoofMusic Desktop

Desktop companion of [WoofMusic](https://github.com/LaskyWolf/WoofMusic) for Linux and Windows, written in Rust. It plays YouTube Music with the same Google account, library and playlists as WoofMusic on iPhone.

Status: rewrite in progress. See [docs/MILESTONES.md](docs/MILESTONES.md).

## Install

Download the latest package from the repository's Releases page.

- **Linux** (x86_64, glibc 2.39 or newer: Ubuntu 24.04, Fedora 40, Arch and later): download `WoofMusic-x86_64.AppImage`, then
  ```bash
  chmod +x WoofMusic-x86_64.AppImage && ./WoofMusic-x86_64.AppImage
  ```
  Everything the app needs is inside the file.
- **Windows 10/11** (x64): run `WoofMusic-<version>-setup.exe`. It installs for your user only, without admin rights. The sign-in window uses Microsoft Edge WebView2, which Windows 11 includes; on Windows 10 install it from <https://developer.microsoft.com/microsoft-edge/webview2/> if sign-in does not open.

## Build from source

1. Install Rust (stable): <https://rustup.rs>
2. Linux: install libmpv 0.35 or newer (the audio player) and WebKitGTK 4.1 (the sign-in window):
   - Arch: `sudo pacman -S mpv webkit2gtk-4.1`
   - Debian 12 / Ubuntu 24.04: `sudo apt install libmpv-dev libwebkit2gtk-4.1-dev libdbus-1-dev`
   - Fedora: `sudo dnf install mpv-libs-devel webkit2gtk4.1-devel dbus-devel`
3. Optional: install `ffmpeg` (Arch `sudo pacman -S ffmpeg`, Debian/Ubuntu `sudo apt install ffmpeg`, Fedora `sudo dnf install ffmpeg-free`).
   AutoMix uses it to find each song's beat; without it, AutoMix falls back to a Smart Crossfade.
4. Run:

```bash
cargo run -p woofmusic --release   # the app
cargo test --workspace             # the tests
```

On Windows, build from a "Developer PowerShell for VS" (MSVC). The `packaging/windows/libmpv.ps1` script used by CI downloads libmpv and makes the `mpv.lib` the linker needs; it expects the GitHub CLI (`gh`) and 7-Zip on PATH.

Packages are built by CI (`.github/workflows/build.yml`) when a `v*` tag is pushed: `packaging/linux/appimage.sh` makes the AppImage, `packaging/windows/woofmusic.iss` the installer.

## Layout

| Folder | Content |
| --- | --- |
| `crates/swiftytube/` | Rust translation of SwiftyTube, file by file |
| `crates/trop/` | Rust translation of WoofMusic logic (`Trop/`), file by file |
| `crates/fixtures/` | Real YouTube Music responses used by the tests |
| `SWIFTYTUBE_COMMIT`, `TROP_COMMIT` | Last upstream commits translated |

[docs/STUDY.md](docs/STUDY.md) has the research behind the stack.

## License

GPLv3, like WoofMusic.
