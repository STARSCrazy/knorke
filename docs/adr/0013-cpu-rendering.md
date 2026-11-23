# ADR 0013: CPU-Rendering – pixelraum als `knorke-render` + `knorke-lighting`

Status: angenommen · Datum: 2026-11-23 · Bezug: ADR 0001, 0002, 0006, 0008, 0011, Inventur `docs/inventory/pixelraum.md`

## Kontext

Die in Plan §2.3 gesuchte Grafikbibliothek ist **pixelraum** (Rust, reiner Software-Renderer im Pixel-Look, gleicher Autor, MIT). Sie wird migriert. Plan §7.2 trennt Renderer und Beleuchtung in eigene Crates, die jeweils allein funktionieren; `knorke-integration` verbindet sie. In pixelraum sind Licht und Schatten mit dem Shading verwoben (ein `Renderer` mit eigenen Threads).

## Entscheidung

1. **Schnitt über G-Buffer-Spalten.** `knorke-render` rastert und liefert pro Sample Art, Position, Normale, Billboard-Achse und Pixel (`Renderer::light_samples`, Layout: erst die `w*h` Bildschirm-Samples, dann Fragment `k` bei `w*h + k`), dazu die Schattenwerfer des Frames. `knorke-lighting` berechnet daraus einen Lichtwert `[f32; 3]` pro Sample in eigenen, wiederverwendeten Zustand und gibt ihn als `impl Column<[f32; 3]>` heraus (ADR 0006). `Renderer::end_lit` nimmt ihn entgegen und shadet. `knorke-render` hängt nicht von `knorke-lighting` ab und umgekehrt (`xtask deps`); nur `knorke-integration::pixel` kennt beide. Texturen erreichen das Licht über den Trait `CasterTextures`, die Werfer werden in der Integration in die Typen des Lichts kopiert (kleine Listen, Kapazität bleibt).
2. **Der Host besitzt das Ziel.** Der Renderer schreibt RGBA, Palettenindex und Normalen in seine eigenen, bei `new` angelegten Puffer; der Host liest sie nach `end` und lädt sie in seine Textur. Kein Fenster, kein Present, kein Device (ADR 0002 gilt für den späteren GPU-Pfad).
3. **Keine eigenen Threads.** Parallelität nur über `knorke_types::Parallel` (ADR 0008): Shading in festen Bändern zu 16 Zeilen, Licht in Chunks zu 1024 Samples, Sonnen-Schattenkarte in Bändern, Würfelflächen der Punktlichter als Aufträge. Das Ergebnis ist bitgleich über `Serial` und jeden Executor. `knorke-render` und `knorke-lighting` hängen nicht von `knorke-exec`/`rayon` ab; die Integration wählt aus `Options::threads` `Serial` oder `ScopedThreads`.
4. **Keine Allokation im eingeschwungenen Frame** (seriell), belegt durch den Vertragstest `tests/contracts/tests/no_alloc_pixel.rs` (Licht, Schatten, Himmel, Konturen, Transparenz, Partikel). Dafür wurde gegenüber pixelraum das Clipping auf ein Stack-Polygon umgestellt und Backdrop-Reihenfolge, Fragmentreihenfolge, Konturmaske und Emitter-Durchlauf auf wiederverwendeten Zustand. Ausnahme: mit mehr als einem Thread alloziert `end_lit` eine Band-Liste pro Frame (Plan §17.1).
5. **Präsentations-Crates lockern Clippy über ein eigenes `clippy.toml`.** `knorke-render`, `knorke-lighting` und `knorke-integration` verbieten weiter Uhren (`Instant`, `SystemTime`) und `HashMap`/`HashSet`/`RandomState`, aber nicht die plattformabhängigen `f32`-Funktionen aus ADR 0011: ihr Ergebnis ist ein Bild, kein Simulationszustand, und fließt nicht in Prüfsummen oder Replays. Golden Images sind deshalb nur pro Plattform verbindlich.
6. **Licht linear, sRGB einmal am Ende – Ziel, noch nicht umgesetzt.** Richtig wäre: Texturen beim Laden nach linear, Licht und Nebel linear, Umrechnung nach sRGB einmal beim Schreiben der Ausgabe (Tabellen in `knorke-math`). pixelraum multipliziert Licht direkt auf die 0..255-Werte. Die Migration behält das bewusst bei, damit alle 55 Golden Images pixelgenau gleich bleiben und der Umbau nachprüfbar ist. Die Umstellung ist ein eigener Schritt mit neu erzeugten Golden Images (Plan §17.1).
7. **C-Schnittstelle bleibt in pixelraum.** `capi.rs` und `include/pixelraum.h` werden nicht migriert; Wizardry 7 nutzt weiter das Original-Repo. Eine Knorke-C-ABI kommt mit `knorke-abi` (Plan §17.1).
8. **Lizenz.** pixelraum steht unter MIT, Knorke unter `MIT OR Apache-2.0`. Gleicher Autor, daher neu lizenziert; Herkunft in den Crate-Dokumentationen (`pixelraum @ 81927d8`).
9. **Späterer GPU-Pfad.** Der Rust-Neubau von Chiaro (Plan §5) kommt als GPU-Pfad in `knorke-lighting` dazu, neben dem Software-Pfad; der Schnitt (Samples rein, Licht raus) ist so gewählt, dass ein GPU-G-Buffer dieselbe Rolle übernehmen kann.

## Warum der Schnitt nicht ganz sauber ist

- Sample-Arten (`UNLIT`, `BILLBOARD` …) und `SOLID_CASTER` sind in beiden Crates definiert, weil keine die andere kennt; `knorke-integration` prüft die Gleichheit zur Compile-Zeit. Ein gemeinsamer Ort wäre ein kleiner Vertragstyp in `knorke-types`, sobald ein zweiter Renderer ihn braucht.
- Das Licht des Renderers für Partikel (`ParticleLight`) und die Himmelsfärbung laufen über `FrameLight` zurück in den Renderer; das ist Teil des Vertrags, nicht des Lichtmoduls.
- `LitRenderer` in der Integration ahmt die pixelraum-Gesamt-API nach (Deref auf `Renderer`), damit die übernommenen Tests unverändert bleiben. Neue Hosts können `light_frame` direkt nutzen.

## Konsequenzen

- `render-only` (ohne Licht, `end`), `lighting-only` (`LightScene` auf Host-Spalten, `light_points`) und `render+lighting` (`knorke-integration::pixel`) funktionieren je allein (`xtask standalone`).
- Tests: siehe Inventur §3.1; Golden Images byte-gleich, alle Bildtests pixelgenau.
- Farbumstellung auf linear und Golden Images auf anderen Plattformen sind offen (Plan §17.1).
