# WoofMusic Desktop

Desktop companion of [WoofMusic](https://github.com/LaskyWolf/WoofMusic) for Linux and Windows, written in Flutter.

Status: skeleton. See [docs/MILESTONES.md](docs/MILESTONES.md) for the plan and [docs/STUDY.md](docs/STUDY.md) for the research behind it.

## Run it

1. Install Flutter (stable, 3.50 or newer): <https://docs.flutter.dev/get-started/install>
2. Linux only, install the build tools and libmpv:
   - Arch: `sudo pacman -S clang cmake ninja pkgconf gtk3 mpv`
   - Debian/Ubuntu: `sudo apt install clang cmake ninja-build pkg-config libgtk-3-dev libmpv-dev`
3. Windows only, install Visual Studio with the "Desktop development with C++" workload.
4. Run:

```bash
flutter pub get
flutter run -d linux     # or: -d windows
```

`flutter doctor` lists anything still missing.

## Build a release

```bash
flutter build linux --release     # → build/linux/x64/release/bundle/
flutter build windows --release   # → build/windows/x64/runner/Release/ (Windows only)
```

A Windows build cannot be made on Linux. GitHub Actions ([.github/workflows/build.yml](.github/workflows/build.yml)) builds both on every push and attaches them to the release when a `v*` tag is pushed.

## Layout

| Folder | Content |
| --- | --- |
| `lib/swiftytube/` | Dart translation of SwiftyTube, file by file |
| `lib/trop/` | Dart translation of WoofMusic logic (`Trop/`), file by file |
| `lib/ui/` | Desktop screens, glass, window |
| `SWIFTYTUBE_COMMIT`, `TROP_COMMIT` | Last upstream commits translated |

## License

GPLv3, like WoofMusic.
