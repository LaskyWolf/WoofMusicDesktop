# WoofMusic Desktop

Desktop companion of [WoofMusic](https://github.com/LaskyWolf/WoofMusic) for Linux and Windows, written in Rust. It plays YouTube Music with the same Google account, library and playlists as WoofMusic on iPhone.

Status: rewrite in progress, nothing to install yet. See [docs/MILESTONES.md](docs/MILESTONES.md).

## Build from source

1. Install Rust (stable): <https://rustup.rs>
2. Run:

```bash
cargo test --workspace
```

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
