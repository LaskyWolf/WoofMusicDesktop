# WoofMusic Desktop

Desktop companion of [WoofMusic](https://github.com/LaskyWolf/WoofMusic) for Linux and Windows, written in Rust. It plays YouTube Music with the same Google account, library and playlists as WoofMusic on iPhone.

Status: rewrite in progress, nothing to install yet. See [docs/MILESTONES.md](docs/MILESTONES.md).

## Build from source

1. Install Rust (stable): <https://rustup.rs>
2. Install libmpv 0.35 or newer (the audio player):
   - Arch: `sudo pacman -S mpv`
   - Debian 12 / Ubuntu 24.04: `sudo apt install libmpv-dev`
   - Fedora: `sudo dnf install mpv-libs-devel`
3. Optional: install `ffmpeg` (Arch `sudo pacman -S ffmpeg`, Debian/Ubuntu `sudo apt install ffmpeg`, Fedora `sudo dnf install ffmpeg-free`).
   AutoMix uses it to find each song's beat; without it, AutoMix falls back to a Smart Crossfade.
4. Run:

```bash
cargo run -p woofmusic --release   # the app
cargo test --workspace             # the tests
```

Windows builds are not set up yet (milestone R8).

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
