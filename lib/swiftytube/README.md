# swiftytube (Dart mirror)

Dart translation of [SwiftyTube](https://github.com/686udjie/SwiftyTube), the YouTube Music client used by WoofMusic.

Rules:

- One Swift file = one Dart file, same folder, name in snake_case: `Sync/LibraryBrowseParser.swift` → `sync/library_browse_parser.dart`.
- Keep function and field names, so a Swift diff maps to the Dart file line by line.
- No desktop-only code here. It goes in `lib/ui/`.
- Apple-only files get a Dart replacement in the same place:
  - `Cipher/CipherWebView.swift`, `PoToken/PoTokenGenerator.swift` → JavaScript engine (`quickjs_engine`)
  - `Auth/SecureStore.swift` → `flutter_secure_storage`
  - `Auth/LoginWebviewHarvest.swift`, `Auth/SessionTokenRefresher.swift` → cookie paste first, webview later
  - `CryptoKit` SHA1 → `package:crypto`

## Updating

`/SWIFTYTUBE_COMMIT` holds the SwiftyTube commit translated so far.

1. In the SwiftyTube repo: `git diff $(cat /path/to/SWIFTYTUBE_COMMIT)..<new> -- SwiftyTube/`
2. Translate each changed hunk into the matching Dart file.
3. Write the new commit hash into `SWIFTYTUBE_COMMIT`.
