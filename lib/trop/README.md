# trop (Dart mirror)

Dart translation of the WoofMusic app logic (`Trop/` in the WoofMusic repo): parsers, services, database. Screens are not translated; desktop screens live in `lib/ui/`.

Same rules as `lib/swiftytube/README.md`:

- One Swift file = one Dart file, same folder, snake_case name: `Trop/Networking/Models/HomePageParser.swift` → `networking/models/home_page_parser.dart`.
- Database tables keep the names and columns of `Trop/Database/DatabaseMigrations.swift`.
- `/TROP_COMMIT` holds the WoofMusic commit translated so far. Update it after each sync.
