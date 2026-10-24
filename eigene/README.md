# Eigene Programme

Jedes Unterverzeichnis ist ein eigenes Rust-Programm (egui/eframe, nativ und Browser), das
der Ablauf «Eigene-Release» für Windows, Linux und den Browser baut und als Release
`<name>-v<version>` veröffentlicht. Die Version steht in der `Cargo.toml` des Programms.

Damit es im Launcher erscheint: ein Eintrag in `launcher/apps.toml` mit `tag = "<name>-v"`.

Vorgaben für ein Programm `<name>/`:
- `Cargo.toml` mit `name = "<name>"`; die Programmdatei heisst dann `<name>` bzw. `<name>.exe`.
- `index.html` mit `<link data-trunk rel="rust">` für die Browser-Version.
- Optional `werk-prebuild.sh` (läuft vor dem Bauen, `KIND` = windows | linux | macos | web).
  Legt es Dateien nach `vendor/bundle/`, kommen diese ins Paket neben die Programmdatei
  (Windows: neben die .exe, Linux: nach `bin/`, macOS: nach `Contents/MacOS/`, dort ad hoc
  signiert) – so bringt **abspielwerk** libVLC mit.

Veröffentlichen: Reiter «Actions» → «Eigene-Release» → «Run workflow», Programm wählen.
