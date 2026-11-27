# Schritt 3D-2: `knorke-render3d` – GPU-Raster ohne Licht

Stand: 2026-11-27 (Entwurf, wird nach 3D-1 umgesetzt) · Bezug: ADR 0021, `docs/3d/3d-1-szenenmodell.md`, Plan 17.14

## Befunde

- **`knorke-gpu` reicht noch nicht für 3D:**
  - Renderpässe kennen keine Tiefe (`record.rs:278-330`).
  - Pipelines haben fest `CULLMODE_NONE` und keinen Tiefenzustand (`pipeline_cache.rs:250-256`).
  - Texturen haben nur eine Ebene und keine Mips, Sampler nur Clamp.
  - Uploads schreiben ab Offset 0.
  - Gezeichnet wird mit Vertex-Pulling über StructuredBuffer.
- **Den CPU-Unlit-Modus gibt es schon:** `Options::lighting = false` mit `end()`, Farbe = `lin(texel) * clamp(light)`. Rund 20 Goldens sind direkt vergleichbar.
- **`knorke-post` erwartet** ein `RGBA16F`-HDR-Ziel. Für den Vergleich braucht es `scene_color_space = Linear`, `Filter::Nearest`, `Tonemap::None` und Bloom aus.

## Pipeline

Ein Renderpass pro Frame, davor ein Copy-Pass für die Uploads.

**Ziele:**
- Farbe: geliehenes `RGBA16F`.
- Tiefe: eigenes `D32_FLOAT`. Das Format wird vorher geprüft, sonst `Diagnostic`.

**Reihenfolge im Pass:**
1. Backdrop (Vollbild, kein Tiefentest)
2. Opak (Reverse-Z, `GREATER`, Tiefe schreiben)
3. Transparent (`GREATER`, Tiefe nicht schreiben)

**API wie `PostRenderer`:**
- `Renderer3d::new`
- `prepare(&Frame3d)`: alles, was fehlschlagen oder allozieren kann.
- `record(&mut CommandBuffer, …)`: submittet nie (ADR 0002).
- `Options3d` ist eigen (Hintergrund, Sichtweite, später LOD).

**Kamera:**
- `View` wird auf der CPU gerechnet, mit denselben Floats wie der CPU-Weg.
- Tiefe = near/z mit unendlichem Reverse-Z. Strenges `GREATER` entspricht der CPU-Regel „erster gewinnt bei Gleichstand“.
- Der Vertex-Shader rundet die Bildschirmposition auf 1/16 Pixel wie die CPU (`raster.rs:9`). Mit Top-Left-Regel und Pixelzentren ist die Abdeckung dann gleich.

**Geometrie:**
- **Vertex-Layout:** nur `float4`-Felder, weil `float3` in SPIR-V auf 16 Byte ausgelegt wird. Der erste Index kommt als Uniform, weil `SV_VertexID` je Backend anders zählt.
- **Gitter:**
  - Chunks aus 8×8 Säulen, jeder mit eigenem Puffer, bei Änderung neu geschrieben.
  - Gleiche Diagonale wie die CPU.
  - Animation und Scroll laufen pro Frame über eine Materialtabelle (`Textures::resolve`), ohne Neu-Upload.
  - Die Sichtweite wird pro Säule im Vertex-Shader geprüft.
- **Sprites:** als Instanzen; der Vertex-Shader baut die Ecken wie `draw_sprite_body`.
- **Partikel:** `emit_one` läuft auf der CPU, der Shader erzeugt ein pixelgenaues Rechteck wie die CPU.
- **Backdrop:** ein Vollbild-Shader über die Layer-Liste. Die Layer-Ableitung wird eine gemeinsame Funktion in scene3d.

**Opak und Transparenz:**
- Opak: `alpha < 128 → discard`, genau wie die CPU.
- Transparent: Die CPU sortiert die Einträge von fern nach nah (bei Gleichstand nach `blend_order`). Die CPU-Referenz sortiert dagegen pro Pixel.
- Abweichungen gibt es bei sich schneidenden transparenten Flächen, großen transparenten Meshes und bei Mischungen aus Alpha und Additiv.
- Kein OIT, weil SDL-GPU-Fragmentshader keine RW-Storage-Buffer haben.

**Texturen: ein Texel-Pool statt Atlas oder Arrays.**
- Ein Storage-Buffer mit RGBA8 enthält alle Texturen und ihre CPU-gebauten Stufen, dazu eine Tabelle `TexInfo`.
- Der Fragment-Shader liest wie `Texture::sample` (Wrap mit `rem_euclid` oder Clamp). Das ist bitgleich und braucht weder Sampler noch Packer.
- sRGB wird über eine 256er-Tabelle dekodiert.
- Die Fern-Stufen mit dunklen Fugen (ADR 0013) werden mit derselben Stufenwahl und demselben Bayer-Schwellwert getroffen. Die Footprint-Rechnung ist exakt nachgebaut (`noperspective` u/z, v/z, 1/z und deren Ableitungen).
- Der Pool wird nur neu geladen, wenn sich die `Textures`-Revision ändert.

**Ressourcen:**
- Alles liegt in `UploadBuffer`s, die nur wachsen. Im eingeschwungenen Zustand wird nichts alloziert.
- `GridMesh` bekommt eine Revision pro Säule. So merkt sich jeder Verbraucher selbst, was er zuletzt gesehen hat, und die CPU verbraucht kein Dirty-Bit, das die GPU noch braucht. Das wurde in 3D-1 mitgegeben.

## Shader

Ablage: `crates/knorke-render3d/shaders/{src,compiled}`, aufgenommen ins gemeinsame Host-Paket (`DEFAULT_SRC`), dazu ein kleines Testpaket unter `tests/data`.

Programme:
- `scene3d.hlsli`
- die Vertex-Shader `scene3d_grid`, `scene3d_mesh`, `scene3d_sprite` und `scene3d_particle`
- die Fragment-Shader `scene3d_surface`, `scene3d_blend`, `scene3d_particle` und `scene3d_backdrop`

Kein MSL, macOS bleibt in 3D-2 außen vor.

## Vergleich mit der CPU

**Testaufbau:**
- Der Test liegt in `knorke-integration/tests/gpu_scene3d_parity.rs`. `knorke-render3d` darf `knorke-render` nicht kennen.
- `LitRenderer::last_frame()` liefert Szene, `DrawList`, Kamera und Zeit der Golden-Szenen. Die GPU zeichnet damit genau dieselben Szenen.

**Vergleichbar (unlit):**
- plain_front, look_up_turn, anim_frame1, inside_room, scroll_t0/t1, ramp
- backdrop_layers*, backdrop_behind_geometry
- door_*, two_sided_wall, storey_hole_up
- später auch glass/additive (C6) und contour_floor_* (C7)

**Nicht vergleichbar:** alles mit Licht, Nebel, Palette oder Konturen.

**Zwei Stufen:**
1. Das HDR-Bild zurücklesen und auf der CPU kodieren. Das misst das reine Raster.
2. Einmal die volle Kette über `PostRenderer`.

**Metrik:** `ParityBudget` mit neuem Budget `RASTER3D`.
- Startwert: Toleranz 2, höchstens 0,3 % Ausreißer.
- Danach wird das Budget aus einer Messung festgelegt, mit Hardware und Datum.

**Erwartete Unterschiede:**
- ±1 im sRGB-Encode und durch fp16
- Dreiecke, die an der Nahebene geclippt werden
- UV-Rundung an Texelgrenzen
- Sortierung pro Eintrag statt pro Pixel
- Bandkanten im Backdrop
- Gleichstand bei koplanaren Flächen
- LOD-Dither nahe am Schwellwert

## Commits

| # | Inhalt |
|---|---|
| C0 | CPU, bitgleich: `View` und Backdrop-Layer als gemeinsame Funktionen, `LitRenderer::last_frame`, Säulen-Revision (falls nicht schon in 3D-1) |
| C1 | `knorke-gpu`: `DepthTarget`, `render_pass_with_depth`, Tiefe und Cull in `ProgramDesc`; GPU-Test mit Überdeckung und Cull |
| C2 | Crate-Gerüst `knorke-render3d` (Feature `gpu`, Abhängigkeitsregeln), Projektion, Texel-Pool, GPU-Typen; Property-Test „gleiche 1/16-Pixel wie CPU“ |
| C3 | Opak: Gitter und Meshes, Materialtabelle, `prepare`/`record`; Tür-Frame lädt nur einen Chunk neu |
| C4 | Sprites, Partikel, Backdrop |
| C5 | Parity-Harness, `RASTER3D` gemessen, Post-Kette, Doku |
| C6 | Transparenz |
| C7 | Textur-LOD (Konturstufen, `face_lod`, Dither) |

**Nicht in 3D-2:**
- Licht, Schatten, Nebel, Palette, Dither, Konturen (alles 3D-3)
- GPU-Picking
- BVH oder Portale
- MSAA
- Hardware-Mips und Arrays (kommen mit dem Shooter)
- OIT, Compute, MSL

**Risiken:**
- FFI-Erweiterung in `knorke-gpu` (`unsafe`, Unterstützung der Tiefenformate)
- die 3D-1-API ist noch im Fluss
- Fast-Math und FMA in Shadern verschieben Texelgrenzen
- Unterschiede zwischen Vulkan und D3D12

## Entscheidungen (Vorgabe, 2026-11-27)

1. **Texel-Pool in 3D-2.** Er ist exakt und erlaubt eigene Stufen. Hardware-Texturen mit Sampling kommen mit dem Shooter-Profil.
2. **Kein Nebel in 3D-2.** Er kommt mit Licht in 3D-3.
3. **Gemeinsames Shader-Paket** für Post und Render3d, dazu ein kleines Testpaket.
