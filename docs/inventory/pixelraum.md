# Inventur pixelraum (Phase 0)

Stand: 2026-11-23 · Quelle: `D:\Entwicklung\pixelraum` @ `81927d8559c05aacb56754e8894ecfae78f7ca85` (86 Commits, 2026-11-20 … 2026-11-23) · Lizenz MIT, gleicher Autor → in Knorke `MIT OR Apache-2.0`.

pixelraum ist die in Plan §2.3 gesuchte „bereits begonnene Grafikbibliothek“. Diese Inventur beantwortet die Auditfragen aus Plan §7.1 und hält fest, was wohin migriert wurde (ADR 0013).

---

## 0. Kurzfassung

- **Rust-Software-Renderer im Pixel-Look**, keine GPU: ein Dreiecks-Rasterizer schreibt in einen kleinen G-Buffer (Farbe, Tiefe, Normale, Licht, Owner), danach Licht, Schatten, Nebel, Palette, Dithering und Konturen auf der CPU. Gitter-Räume (Wände, Böden, Decken, Türen, Stockwerke, Schrägen), Sprites, Dreieckslisten, Partikel, Hintergrundebenen, Picking.
- **Keine Abhängigkeiten** (`Cargo.toml` hat keine `[dependencies]`), Edition 2021, `crate-type = rlib, staticlib, cdylib`, C-Header `include/pixelraum.h` (486 Zeilen) über `src/capi.rs` (1 380 Zeilen).
- Erster Nutzer: Wizardry 7 revised (über die C-Schnittstelle). Der bleibt vorerst beim Original-Repo.
- Umfang: `src/` 9 103 LOC (davon C-Schnittstelle 1 380), Tests 4 234 LOC, **55 Golden Images** (`tests/golden/*.ppm`).
- Baseline `cargo test --release` (45 s inkl. Build): **183 bestanden, 4 ignoriert** (Unit 56, davon 11 C-Schnittstelle; `images.rs` 91 + 3 ignoriert; `light_gaps.rs` 13; `pick.rs` 23; `lod_timing.rs` 1 ignoriert).

## 1. Auditfragen (Plan §7.1)

| Frage | Befund |
|---|---|
| SDL_Renderer, SDL_GPU oder andere API? Sprache/Toolchain? | **Keine Grafik-API.** Reines Rust (stable), reine CPU. Ausgabe sind RGBA-Bytes, ein Palettenindex-Puffer und Normalen; der Host lädt sie in seine Textur. |
| Wer erzeugt Device, Swapchain, Fenster, Command Buffer, Render Targets? | Nichts davon. Der Renderer besitzt nur seine Frame-Puffer (Breite × Höhe bei `Renderer::new`, wiederverwendet). Kein Fenster, kein Present. |
| Shader, Material, Sprite, Text, Tilemap, Kamera, Assets? | Kein Shader; „Material“ = Textur-ID + Flags. Texturen mit fernen Stufen (Mip-Ersatz, der dunkle Fugen behält) und optionaler Konturmaske. Sprites (Billboard, Flip, Schattenfrei), Gitterkarte als Tilemap-Ersatz, Kamera (Yaw/Pitch/FOV/Lens-Shift). **Kein Text, kein Asset-Import** (nur RGBA/indizierte Bytes und ein kleiner PNG-Encoder für Tests). |
| Farbraum, Alpha, Koordinaten, Auflösung? | Farben 0..255 im sRGB-Wertebereich, **Licht wird direkt darauf multipliziert (nicht linear)**. Alpha: Löcher unter 128, sonst Blend-Modi `BLEND_ALPHA`/`BLEND_ADD` in 1/8-Stufen. Welt rechtshändig, y oben, Blick bei Yaw 0 nach −z. Auflösung frei, gedacht für 160×100 … 640×400. |
| Unnötige Kopien, doppelte Caches, globale Zustände? | Keine globalen Zustände. Pro Frame mehrere kleine Allokationen (Clip-Polygon je Dreieck, Backdrop-Liste, Fragmentreihenfolge je Band, Konturmaske, Emitter-IDs, Band-Liste beim Threading) – in Knorke beseitigt (§3). Eigene Threads per `std::thread::scope` (`renderer/mod.rs:536`, `lights.rs:312/381`), Anzahl über `Options::threads` bzw. `available_parallelism().min(8)`. |
| Tests, Screenshots, Benchmarks, ABI-/Packaging-Verträge? | Golden-Image-Tests (pixelgenau, `PIXELRAUM_BLESS=1` schreibt neu), Pick-/Raycast-Tests, Licht-Lücken-Tests, eine ignorierte Release-Zeitmessung, Beispiel `examples/demo.rs`, C-Beispiel `examples/c/` + `tools/build-c-demo.sh`. ABI-Vertrag: `PR_VERSION` und `repr(C)`-Structs im Header. |

## 2. Aufbau des Originals

| Bereich | Dateien | Inhalt |
|---|---|---|
| Rasterizer | `raster.rs` | Clipping am Frustum, Festkomma-Kanten, Top-Left-Regel, perspektivische UV, Texturstufen |
| Kamera, Mathe | `camera.rs`, `math.rs` | eigener `Vec3` (in Knorke durch `knorke_math::Vec3` ersetzt) |
| Gitter | `grid.rs` | Zellen, Wände, Türen (Schiebe-/Drehtüren), Stockwerke, Schrägen, Raycast |
| Texturen | `texture.rs`, `renderer/textures.rs`, `anim.rs` | ferne Stufen, Konturmaske, Animation |
| Transparenz | `blend.rs` | Fragmentspeicher als verkettete Liste pro Pixel |
| Partikel | `particles.rs`, `renderer/particles.rs` | Funken, Rauch, Regen, Schnee; zeitbasiert, deterministisch |
| Hintergrund | `backdrop.rs`, `renderer/backdrop.rs` | Bänder und Spots, Himmel |
| Licht/Schatten | `light.rs`, `shadow.rs`, `renderer/lights.rs`, Teile von `renderer/shade.rs` | Sonne mit Schattenkarte, bis zu `MAX_LIGHTS` Punktlichter mit Würfel-Schattenkarten, weiche Schatten, Billboard-Schatten, Schattencache |
| Picking | `pick.rs` | Treffer pro Pixel: Gitterfläche, Sprite, Mesh |
| C-Schnittstelle | `capi.rs`, `include/pixelraum.h` | Handles, `PR_*`-Structs |

## 3. Migration nach Knorke

| Ziel | Inhalt | Abweichung zum Original |
|---|---|---|
| `knorke-render` | Raster, Kamera, Gitter, Texturen, Sprites, Picking, Palette, Blending, Animation, Partikel, Hintergrund, Nebel, Konturen | kein Licht, keine Threads; `Renderer::light_samples` gibt G-Buffer-Spalten und Schattenwerfer heraus, `Renderer::end_lit` nimmt einen Lichtwert pro Sample zurück. Clipping auf dem Stack, Puffer wiederverwendet. `HashMap` → `BTreeMap`. |
| `knorke-lighting` | Sonne, Punktlichter, Schattenkarten, weiche Schatten, Billboard-Schatten, Schattencache | arbeitet auf geliehenen Spalten (`Surfaces`), Texturen über den Trait `CasterTextures`, Parallelität über `knorke_types::Parallel` statt eigener Threads |
| `knorke-integration::pixel` | `LitRenderer` (pixelraum-ähnliche Gesamt-API), `light_frame`, kombinierte `Options`/`Stats` | `Options::threads` wählt `Serial` oder `knorke_exec::ScopedThreads` |
| nicht migriert | C-Schnittstelle, `examples/demo.rs`, `examples/c/`, `tools/` | C-Schnittstelle: Plan §17.1; Beispiele folgen mit dem ersten Knorke-Beispielprogramm |

### 3.1 Tests

| Suite | Original | Knorke |
|---|---|---|
| Unit | 56 (11 C-Schnittstelle) | 45 (`knorke-render` 29, `knorke-lighting` 16); `frame_caches_never_change_a_pixel` ist jetzt Integrationstest `caches.rs` |
| `images.rs` | 91 + 3 ignoriert | 91 + 3 ignoriert |
| `light_gaps.rs` | 13 | 13 |
| `pick.rs` | 23 | 23 (C-Blöcke in drei Tests entfernt; `default_wall_c_matches_rust` → `default_wall_matches_c_defaults` ohne C-Seite) |
| `lod_timing.rs` | 1 ignoriert | 1 ignoriert |
| `caches.rs` | – | 1 |
| Vertrag `no_alloc_pixel` | – | 1 (`knorke-contract-tests`) |
| **Summe** | **183 + 4 ignoriert** | **174 + 4 ignoriert** (Original ohne C-Tests: 172) |

**Golden Images:** alle 55 Dateien byte-gleich übernommen (`diff -rq` leer) und alle Bildtests bestehen pixelgenau in Debug und Release. `.gitattributes` setzt `*.ppm binary`.

## 4. Offene Punkte

- C-Schnittstelle für Wizardry 7 (Plan §17.1).
- Farben nicht linear (ADR 0013 §3): in Knorke am 2026-11-23 auf linear umgestellt (ADR 0013 §6), Golden Images neu.
- Golden Images bisher nur unter Windows/x86-64 belegt; `knorke-render` nutzt `sin`/`cos`/`tan`/`atan2`/`log2` aus `std` (Kamera, Türen, Partikel, Hintergrund, Texturstufen), die nach ADR 0011 plattformabhängig sein dürfen – plattformübergreifend unbelegt. Die Präsentations-Crates lockern dafür die Clippy-Regeln (ADR 0013 §5).
- Mit `threads > 1` alloziert `end_lit` eine Band-Liste pro Frame (Plan §17.1).
