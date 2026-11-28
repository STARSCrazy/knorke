# ADR 0025: Schrift und einfache Oberfläche – Bitmap- und TrueType-Schrift, ein Weg zum Bild

Status: Vorschlag · Datum: 2026-11-28 · Bezug: ADR 0001, 0002, 0006, 0008, 0010, 0011, 0022, 0023, Einschätzung 2026-11-28 Punkt A3, Plan 17.15

## Kontext

Der Stealth-Hof (ADR 0022, 0023) zeigt kein einziges Wort. Für das erste Spiel (A4) braucht er mindestens „Erwischt!“, „Ziel erreicht“, eine Zeile Hilfe und ein Menü „Neustart / Beenden“. Wizardry 7 braucht später viel mehr Text: Namen, Werte, Listen, Dialoge.

Manuel hat entschieden (2026-11-28): **beide Schriftarten werden unterstützt.** Bitmap-Schrift passt zu Pixelgrafik und braucht nichts Neues; TrueType ist in jeder Größe scharf und nötig für viel Text.

Bild entsteht heute auf zwei Wegen: `stealth_view` malt den Hof auf der CPU in eine RGBA-Leinwand und lädt sie hoch; `knorke-render3d` (ADR 0021) zeichnet auf der GPU. Text muss in beide passen.

## Entscheidung (Vorschlag)

1. **Neue Crate `knorke-text`** (`no_std` + `alloc`, Abhängigkeit nur `knorke-types`). Sie kennt weder Dateien noch GPU noch Fenster.
2. **Beide Schriftarten enden in derselben Form: `Font`** = ein Atlas (ein Graustufen-Bild mit allen Zeichen, 1 Byte Deckung je Pixel) plus Maße je Zeichen (Ausschnitt im Atlas, Versatz, Vorschub) und Zeilenhöhe. Alles nach dem Laden arbeitet nur mit `Font`. So gibt es Layout und Zeichnen nur einmal.
   - **Bitmap:** `Font::from_grid(atlas, cols, rows, first_char)` – ein PNG mit festem Raster, Zeichen in Unicode-Reihenfolge ab `first_char` (Standard: ASCII 32–126, 16 × 6). Kein Beschreibungsfile nötig, passend zu ADR 0023 (IDs aus Dateinamen, keine Liste). Feste Breite; Proportionalschrift per Bitmap erst bei Bedarf.
   - **TrueType:** Feature `ttf` mit einer reinen Rust-Bibliothek zum Rastern. `Font::from_ttf(bytes, px, chars)` rastert die verlangten Zeichen **einmal beim Laden** in einen Atlas fester Größe. Kein Rastern pro Frame.
3. **Laden über `knorke-assets`:** Bitmap über `Folder::image(id)`. Für TrueType kommt `Folder::bytes(id, ext)` dazu (rohe Bytes, gleiche Schreibweisen-Prüfung). `knorke-assets` bekommt **keine** Schrift-Abhängigkeit; die Liste aus ADR 0023 bleibt geschlossen.
4. **Layout ohne Allokation im Steady State (ADR 0006):** `layout(&Font, text, &Style, out: &mut Vec<Glyph>)` schreibt Positionen in einen Puffer des Aufrufers (geleert, nicht neu angelegt). Zeilenumbruch an `\n` und optional an einer Breite (Wortgrenzen). Ausrichtung links/Mitte/rechts. Kein Kerning, keine Bidi-Texte, kein Shaping (Arabisch, Ligaturen) – bewusst nicht in A3.
5. **Zeichnen auf zwei Wegen:**
   - **CPU:** `draw_rgba8(canvas, w, h, &Font, &[Glyph], color)` mischt die Deckung in eine sRGB-RGBA-Leinwand. Das nutzt `stealth_view` sofort.
   - **GPU:** Der Atlas ist eine Textur, jedes `Glyph` ein Rechteck. Die GPU-Anbindung (Shader, Vertex-Puffer) kommt erst mit dem ersten GPU-Verbraucher (3D-Ansicht, Wizardry 7) und lebt dann im zeichnenden Modul, nicht in `knorke-text`.
6. **Oberfläche (UI) klein und ohne Callbacks (ADR 0008):** `knorke-text` bekommt im Modul `ui` nur Bausteine mit Zustand als Daten:
   - `Label` (Text, Position, Ausrichtung), `Panel` (Rechteck mit Farbe hinter Text),
   - `Menu` (Einträge, gewählter Index). `Menu::apply(action)` nimmt `Up`/`Down`/`Confirm`/`Back` und liefert `Option<usize>` für die Wahl. Die Aktionen kommen aus `knorke-input::Bindings` im Spiel; `knorke-text` hängt nicht an `knorke-input`.
   - Kein Layout-System (Flexbox), keine Maus-Treffer, keine Fenster-im-Fenster. Das bleibt in Plan 17 „offen“.
7. **Eingabegrenze:** Menüs gehören zum Spiel, nicht zur Simulation. Was ein Menü auslöst („Neustart“), geht als Command in die Simulation bzw. baut sie neu (ADR 0010). Text liest Simulationszustand nur zum Anzeigen.
8. **Diagnosen:** `knorke.text.bad_grid` (Bildgröße passt nicht zum Raster), `knorke.text.ttf` (Schriftdatei nicht lesbar), `knorke.text.atlas_full` (verlangte Zeichen passen nicht in den Atlas). Fehlende Zeichen beim Layout werden durch `?` ersetzt und gezählt, keine Panik.

## Konsequenzen

- Ein fremdes Projekt kann `knorke-text` ohne Feature `ttf` mit null Fremdabhängigkeiten nutzen.
- Bitmap und TrueType sind nach dem Laden gleich; Tests für Layout und Zeichnen laufen einmal für beide.
- TrueType bringt eine Abhängigkeit (nur mit Feature `ttf`). Welche, wird in Schritt 3 nach Lizenz, Größe und `no_std`-Fähigkeit gewählt und hier nachgetragen.
- Rastern beim Laden heißt: eine neue Schriftgröße braucht einen neuen `Font`. Für A4 reicht das; dynamisches Nachrastern ist Plan 17.

## Umsetzungsplan

1. **Regeln zuerst.** Crate `knorke-text` mit `Font`, `Glyph`, `layout`; Regeln in `xtask/src/deps.rs` (`knorke-text` nur `knorke-types` + TrueType-Bibliothek hinter `ttf`; kein Fachmodul hängt an ihr außer Darstellung/Integration). Fertig, wenn `cargo xtask deps` grün ist und ein falscher Eintrag rot wird.
2. **Bitmap-Schrift.** `Font::from_grid`, CPU-Zeichnen, eine kleine Pixel-Schrift (eigene Zeichnung oder freie Lizenz mit Herkunft) in `knorke-integration/assets/fonts/`. Tests: Layout (Umbruch, Ausrichtung, fehlendes Zeichen), Referenzbild, Vertragstest „Layout alloziert nicht“.
3. **TrueType.** Feature `ttf`, `Folder::bytes`, `Font::from_ttf`; eine freie Schrift (z. B. OFL-Lizenz) im Asset-Ordner. Tests: dieselben Layout-Tests, Referenzbild bei zwei Größen.
4. **Oberfläche.** `ui::{Label, Panel, Menu}`; Tests für `Menu::apply` (Umlauf, Bestätigen, leeres Menü).
5. **Stealth-Hof zeigt Text.** Hilfe-Zeile, „Erwischt!“ mit Menü „Neustart / Beenden“ (Neustart baut die Simulation neu, Aufnahme beginnt neu). Fertig, wenn ein Spieldurchgang erwischt werden → Neustart wählen → weiterspielen ohne Neustart des Programms geht und Replay-Test grün bleibt.
6. **Abschluss.** Volle Prüfliste, Plan 16/17, ADR auf „angenommen“.

## Offene Fragen

1. **Bitmap-Schrift selbst zeichnen oder eine freie übernehmen?** Vorschlag: eine freie 8×8- oder 6×10-Pixelschrift mit klarer Lizenz (Public Domain/CC0) übernehmen und Herkunft vermerken. Selbst zeichnen geht auch, kostet aber Zeit.
2. **Welche TrueType-Bibliothek?** Kandidaten (Angaben aus dem Gedächtnis, noch nicht geprüft): `fontdue` (reines Rust, `no_std`, MIT/Apache/Zlib) und `ab_glyph` (reines Rust, Apache-2.0). Vorschlag: in Schritt 3 beide kurz messen und die kleinere nehmen.
