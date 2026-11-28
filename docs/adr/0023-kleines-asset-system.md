# ADR 0023: Kleines Asset-System – Bilder und Klänge über IDs aus einem Ordner

Status: angenommen, umgesetzt (2026-11-28) · Datum: 2026-11-28 · Bezug: ADR 0001, 0003, 0010, 0020, 0022, Plan 17.2 (Dateipfade/Assets), Plan 17 (Asset-System), Einschätzung 2026-11-28 Punkt A2

## Kontext

Seit ADR 0022 läuft der Stealth-Hof in einem Fenster mit Ton. Alles, was er zeigt und abspielt, entsteht im Code: Die Klänge in `knorke-host/examples/stealth_yard/sound.rs` werden aus Sinus und Rauschen gerechnet, das Bild baut `stealth_view` aus der Szene. Dateien lädt nur Testcode: `png` (0.17) steckt in `knorke-integration` und `knorke-agent`, um Referenzbilder zu schreiben und zu vergleichen. Einen Weg „Szene nennt ein Bild, das Spiel bekommt es“ gibt es nicht.

Plan 17.2 verlangt Asset-IDs statt Pfaden, weil Linux Groß- und Kleinschreibung unterscheidet und Windows nicht. Plan 17 führt das große Asset-System (asynchron, Referenzzählung, Streaming, Hot-Reload) als „geplant“. A2 ist bewusst die kleine Stufe davor.

Hinweis: Parallel überarbeitet ein anderer Chat den Ton (`knorke-audio`, Chip-Klänge in `sound.rs`). Schritt 4 unten berührt dieselbe Datei und wird erst danach gebaut.

## Entscheidung (Vorschlag)

1. **Neue Crate `knorke-assets`** (`std`). Sie liest einen Ordner und dekodiert. Sie kennt weder GPU noch Audiogerät noch Simulation. Abhängigkeiten: `knorke-types` und `png`. Keine Fachcrate hängt von ihr ab, nur Spiele, Beispiele, Tests und Werkzeuge (neue Regel in `xtask/src/deps.rs`).
2. **Asset-ID = relativer Pfad in fester Schreibweise.** Kleinbuchstaben, Ziffern, `_`, `-`, `.`, Trenner `/`, ohne Endung: `sprites/guard`, `sounds/step`. Die Datei wird anhand der ID gesucht, auf allen Systemen mit genauer Schreibweise. Eine Datei, die nur in anderer Schreibweise existiert, gilt als fehlend. So fällt ein Fehler, der sonst nur unter Linux auftritt, schon unter Windows auf.
3. **Laden außerhalb der Simulation.** Das Spiel lädt beim Start synchron, was die Szene nennt, und reicht die fertigen Daten an Bild und Ton weiter. Die Simulation sieht Assets nur, wenn sie davon abhängt (später z. B. eine Kollisionskarte). Dann kommt das fertige Ergebnis als Command eines Ticks (Eingabegrenze, ADR 0010). In A2 hängt die Simulation von keinem Asset ab.
4. **Formate:** PNG → `Image { width, height, rgba: Vec<u8> }` (sRGB-Bytes, Umrechnung nach linear macht der Zeichner, ADR 0002). WAV (PCM 8/16/24 Bit, Float 32, Mono/Stereo) → `Sound { samples: Vec<f32>, rate, stereo }`, passend zu `knorke_audio_sdl::Clip`.
5. **Diagnosen mit stabilen Codes:** `knorke.assets.missing` (keine Datei zur ID), `knorke.assets.bad_id` (ID verletzt die Schreibweise), `knorke.assets.decode` (Datei kaputt oder Format nicht unterstützt), `knorke.assets.case` (Datei existiert nur in anderer Schreibweise; die Meldung nennt den gefundenen Namen).
6. **Szene nennt IDs.** Komponenten in `knorke-scene` speichern Asset-IDs als Text. Die Prüfregel eines Profils kann melden, wenn eine ID im Ordner fehlt. Das Laden selbst bleibt aber beim Spiel.
7. **Bewusst nicht in A2:** asynchrones Laden, Streaming, Referenzzählung, Hot-Reload, gepackte Archive, weitere Formate (OGG, Schrift). Das bleibt in Plan 17 „geplant“.

## Konsequenzen

- Ein fremdes Projekt kann `knorke-assets` allein nutzen (nur `knorke-types` und `png`).
- Die Schreibweise der IDs ist ab dem ersten Asset Teil des Vertrags. Spätere Archive (Paket statt Ordner) behalten dieselben IDs.
- Synchrones Laden beim Start reicht für den Stealth-Hof. Für Wizardry 7 wird asynchrones Laden nötig. Es baut auf derselben ID-Tabelle auf.

## Umsetzungsplan

1. **ADR annehmen, Regeln zuerst.** Crate `knorke-assets` mit `AssetId` (Prüfung der Schreibweise) und `Folder` (Suche nach genauer Schreibweise). Regel in `xtask/src/deps.rs`, Eintrag in `standalone`.
   Fertig, wenn: `cargo xtask deps` grün ist, ein absichtlich falscher Eintrag (z. B. `knorke-ai → knorke-assets`) rot wird und Tests `missing`, `bad_id` und `case` belegen (`case` auch unter Windows).
2. **PNG laden.** `Folder::image(id)`. Tests mit einer kleinen Datei im Testordner: Größe und Pixel stimmen, eine kaputte Datei liefert `decode`.
3. **WAV laden.** `Folder::sound(id)`. Tests für 16 Bit Mono, Float-Stereo und eine nicht unterstützte Variante (`decode`).
4. **Stealth-Hof nutzt Dateien.** Ein Ordner `knorke-integration/assets/` mit mindestens einem Bild und den Klängen des Hofs; das Beispiel lädt sie über IDs statt sie im Code zu rechnen. Erst nach dem Ton-Chat, weil dieselbe Datei betroffen ist.
   Fertig, wenn: die Szene ihre Bilder und Klänge über IDs aus dem Ordner bekommt und das Beispiel bei einem umbenannten Asset mit `knorke.assets.missing` abbricht statt still weiterzulaufen.
5. **Abschluss.** Volle Prüfliste aus `CLAUDE.md`, Plan 16 und 17 nachtragen, ADR auf „angenommen“.

## Umsetzung (2026-11-28)

- Schritte 1–3: `knorke-assets` mit `AssetId`, `Folder`, PNG und eigenem WAV-Leser; Regeln in `xtask/src/deps.rs`.
- Schritt 4: Komponente `knorke.stealth.assets/1` (am Entity `yard`) nennt fünf Bilder und vier Klänge. `stealth_view::Look` hält die Kacheln; `View::with_look` nimmt sie, `View::new` nimmt `Look::plain()` (Werkzeuge ohne Asset-Ordner, z. B. das Bild der Agenten-Kommandozeile). Die Dateien in `knorke-integration/assets/` wurden einmal aus dem bisherigen Code erzeugt: Bild bitgleich (alle Referenzbilder unverändert), Klang als 16-Bit-PCM. Das Beispiel `stealth_yard` lädt sie vor dem Öffnen des Fensters; Test `knorke-host/tests/yard_assets.rs` (umbenannte Datei → `missing`), von Hand geprüft: `Wall.png` → `knorke.assets.case`, Abbruch.
- Nicht gemacht: Prüfregel im Profil für fehlende IDs (Entscheidung 6 sagt „kann“); die Simulation bleibt ohne Assets.

## Entschiedene Fragen (Manuel, 2026-11-28)

1. **WAV wird selbst gelesen**, ohne die Crate `hound`. Das sind etwa 100 Zeilen, PCM ist einfach, und es kommt keine Abhängigkeit dazu.
2. **IDs kommen aus den Dateinamen** (Entscheidung 2), es gibt keine Liste (`assets.json`). Das ist einfacher, und nichts kann veralten. Eine Liste lohnt erst, wenn Assets Zusatzdaten brauchen (z. B. Sprite-Raster, Lautstärke).
