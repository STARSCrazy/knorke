# Chiaro: Aufteilung des Ports

Stand: 2026-11-23 · Quelle: `D:\Entwicklung\chiaro` (Stand wie `docs/inventory/chiaro.md`) · Bezug: Plan §5, §7, §17.12, ADR 0001, 0002, 0013.
LOC mit `wc -l` gemessen (Quelle plus Header, ohne Tests). Alles unter „Einschätzung“ und alle mit **(Vermutung)** markierten Zahlen sind nicht belegt.

## 0. Antwort in Kürze

- Ja, 18 kLOC in einem Stück sind zu viel. Es ist aber auch kein einzelnes Stück.
- Von 23 580 LOC in `src/` sind nur etwa 1,2 kLOC generischer GPU-Unterbau und etwa 1,5 kLOC generische Nachbearbeitung. Gut 14,6 kLOC sind fest mit dem 2D-Licht verdrahtet. Etwa 2,3 kLOC sind 2D-Szenenhelfer. Etwa 4,1 kLOC entfallen (C-ABI) oder werden neu entworfen statt übersetzt.
- Vorschlag: vier Bausteine (`knorke-gpu`, Nachbearbeitung, Licht, 2D-Helfer) und rund 30 kleine Pakete.
- Etwa die Hälfte der Pakete ist reine CPU-Logik ohne GPU. Sie kann sofort und parallel laufen.

## 1. Tabelle je Quellgruppe

Zielspalte: `gpu` = `knorke-gpu`; `post` = Nachbearbeitung (Crate-Name offen, siehe Frage 1); `light` = `knorke-lighting` (Feature `gpu`, CPU-Teile ohne GPU-Abhängigkeit); `2d-help` = 2D-Szenenhelfer; `drop` = entfällt; `neu` = neu entwerfen.
Art: **G** = generische GPU, **P** = 2D+3D (Render/Post), **L** = nur 2D-Licht, **H** = nur 2D-Helfer.
Tests: Unit-Testfälle aus `tests/unit/*Tests.cpp` (zählt `TEST_CASE`).

### 1.1 `src/gpu` (1 540 LOC)

| Gruppe (Pfad) | LOC | Ziel | Art | Hängt ab von | Hinweis | Unit-Tests |
|---|---|---|---|---|---|---|
| `gpu/GpuHandle.hpp`, `RenderTarget.*`, `Samplers.*`, `UploadBuffer.*`, `StagingMath.hpp` | 453 | gpu | G | – | RAII mit Lebendzähler; Transfer-Cycling. Lebensdauer-Kern (`unsafe`). | StagingMath 3 |
| `gpu/ShaderLibrary.*`, `ProgramBinding.hpp` | 231 | gpu | G | – | Formatwahl DXIL > SPIR-V > MSL, Reflection-JSON, Slot-Regeln. Blobs per `include_bytes!`. | – |
| `gpu/PipelineCache.*` | 434 | gpu | G | ShaderLibrary | Cache je (Programm, Zielformat). | – |
| `gpu/LightProgram.hpp` | 274 | light | L | ProgramBinding | Basis × 8 Merkmalsbits, 92 Programme. Heikelste Tabelle. | LightProgramLayout 4 |
| `gpu/MediumLayout.hpp`, `CompositeLayout.hpp` | 168 | light | L | ProgramBinding | Uniform-Layouts als `#[repr(C)]`. | – |

### 1.2 `src/core` (2 927 LOC)

| Gruppe | LOC | Ziel | Art | Hängt ab von | Hinweis | Unit-Tests |
|---|---|---|---|---|---|---|
| `core/ColorMath.hpp`, `Formats.*`, `OutputRect.*` | 339 | post (Teile `knorke-math`) | P | – | sRGB, Zielformate, Letterbox. Spiegel von `common.hlsli`. | ColorMath 7, Color 5, Formats 5, OutputRect 5 |
| `core/Projection.*` | 86 | light (oder `knorke-math`) | L (2D/Iso) | – | Affin Welt→Szene. | Projection 4 |
| `core/Triangulate.*` | 212 | `knorke-math` oder light | 2D-Geometrie | – | Schatten-Polygone. | Triangulate 10 |
| `core/Shockwave.*`, `CrtMath.hpp`, `Bloom.hpp` | 522 | post | P (Bildschirmraum) | ColorMath | Reine Mathematik für Output/Bloom/CRT/Druckwelle. | Shockwave 9, Crt 10, Bloom 6 |
| `core/Settings.*` | 571 | neu | – | – | 204-Byte-Struct, 18 Kurz-Setter. Teilen in Licht-, Post-, Qualitäts-Konfiguration. Presets lohnen die Übernahme. | Settings 29, OutputGrade 10 |
| `core/Context.*`, `ContextInspect.hpp` | 832 | neu | – | alles | `RunStages` (`Context.cpp:420-442`), Prepare/Record. Wird zum expliziten Pass-Plan. | – |
| `core/FrameWarnings.*` | 161 | neu | – | – | Wird `knorke_types::Diagnostic` mit stabilen Codes. | FrameWarnings 11 |
| `core/SlotMap.hpp`, `Batch.hpp`, `Ids.hpp` | 204 | drop | – | – | Ersatz: `Slots`/`HandleMap`/`CommandBuffer` aus `knorke-types`. | SlotMap 5 |

### 1.3 `src/lighting` (5 329 LOC, CPU-Logik)

| Gruppe | LOC | Ziel | Art | Hängt ab von | Hinweis | Unit-Tests |
|---|---|---|---|---|---|---|
| `LightStore.*`, `LightMath.*` | 966 | light | L | – | Handles, Slots, Falloff. | LightStore 12, LightMath 7 |
| `LightCulling.*`, `AreaLight.*`, `Animation.*`, `AnimationStore.*` | 1 103 | light | L | LightStore, GpuLight (POD) | Culling inkl. Bezug auf `OcclusionDomain`. | LightCulling 8, AreaLight 8, Animation 18, FrameLightAlloc 1 |
| `LightMask.*`, `Mask.*`, `MaskStore.*` | 696 | light | L | LightStore | Gobo/Fenster. | Mask 15 |
| `Profile.*`, `ProfileStore.*`, `ProfileImport.*` | 1 048 | light | L | – | IES/EULUMDAT-Parser (658 LOC), Testdateien in `tests/unit/data/profiles`. | Profile 12, ProfileImport 14 |
| `AmbientZone*` | 759 | light | L | – | Zonen, Culling. | AmbientZone 9, Store 5 |
| `MaterialMath.*`, `PixelCodec.*` | 460 | light (Vertrag mit Render) | L, Codec auch 3D nutzbar **(Vermutung)** | – | Byte-Layouts von Normal/Material. Gehört evtl. als Vertrag nach `knorke-types`/`-math`. | MaterialMath 14, Material 8, PixelCodec 14, NormalCodec 4 |
| `MediumCulling.*`, `MediumTable.*` | 297 | light | L | – | | MediumCulling 4, MediumTable 5 |

Rückkante: `lighting/*` bindet `rendering/GpuLight.hpp` und `rendering/GpuZone.hpp` (Belege: `lighting/LightCulling.hpp:5`, `AmbientZoneCulling.hpp:3`). Das sind reine POD-Strukturen. In Rust gehören sie in ein GPU-freies CPU-Modul.

### 1.4 `src/shadows` (3 530 LOC, CPU-Logik)

| Gruppe | LOC | Ziel | Art | Hängt ab von | Hinweis | Unit-Tests |
|---|---|---|---|---|---|---|
| `ObstacleStore.*`, `ObstacleDesc.*`, `StoredObstacle.*` | 1 187 | light | L | Triangulate | Größtes CPU-Paket, 44 Tests. | ObstacleStore 44 |
| `TileGrid.*`, `ObstaclePatterns.*` | 604 | light | L | ObstacleStore | | – |
| `PatternStore.*`, `PatternGeometry.*` | 841 | light | L | ObstacleStore | Zaun/Gitter. | Pattern 13+12 |
| `MaskUpdatePlan.*`, `DirtyRects.*`, `OcclusionDomain.*` | 898 | light | L | LightMath | Inkrementeller Schatten-Cache (E21, E157). Viele Randfälle. | MaskUpdatePlan 20, DirtyRects 9, OcclusionDomain 7 |

### 1.5 `src/rendering` (6 143 LOC, GPU-Stufen)

| Gruppe | LOC | Ziel | Art | Hängt ab von | Hinweis |
|---|---|---|---|---|---|
| `BloomStage.*`, `OutputStage.*`, `OutputFlags.hpp` | 470 | post | P | gpu, ColorMath | Tonemap, Grade, Vignette, Dither, Skalierung, Overlay, CRT (Shader `output.frag`, `bloom_*`). |
| `ShockwaveLightPass.*` | 125 | post | P (Bildschirmraum) | Shockwave | Ringlicht-Pass. |
| `CompositeStage.*` | 173 | light | L | gpu, Licht-Puffer | Szene × Licht + Glow. 6 Shadervarianten. |
| `Pass.hpp` | 62 | gpu | G | – | Basis für den Pass-Plan. |
| `Frame.hpp` | 216 | light | L | – | Eingaben pro Frame. |
| `OccluderGeometry.*`, `TileOccluders.*` | 678 | light | L | ObstacleStore, gpu | Hindernisse in die Maske rastern. |
| `OcclusionMask.*`, `OcclusionStage.*`, `OcclusionCommon.hpp`, `SdfShadows.*` | 933 | light | L | MaskUpdatePlan, gpu | Maske, Jump-Flooding, Cache-Ausführung. |
| `PatternOccluders.*`, `PatternAtlas.*`, `GpuPatternObstacle.hpp` | 890 | light | L | PatternStore, gpu | Vulkan-Compiler-Workaround (Backlog Muster 7). |
| `LightingStage.*`, `GpuLight.hpp`, `GpuZone.hpp`, `LightAtlases.hpp` | 813 | light | L | LightProgram, Culling, gpu | Kern: Programmwahl und Bindung. |
| `ProfileAtlas.*`, `MaskAtlas.*`, `AmbientZonePass.*` | 554 | light | L | Profile, Mask, Ambient, gpu | |
| `MediumMapStage.*`, `MediumStage.*` | 1 042 | light | L | MediumCulling, Layout, gpu | Als „neu“ eingestuft, bekannte Grenzen (Inventur §3). |
| `DebugCheckStage.*` | 186 | neu | – | – | GPU→CPU-Readback verletzt ADR 0002. Nur auf Anforderung, als `Invariants`/`Diagnostic`. |

### 1.6 `src/integration` (2 073 LOC, 2D-Helfer)

| Gruppe | LOC | Ziel | Art | Hängt ab von | Hinweis |
|---|---|---|---|---|---|
| `Layers.*` | 625 | 2d-help | H | gpu | Chiaro-eigene Szenenpuffer. Eigene Submits → neu entwerfen. |
| `Sprite.*`, `StreamingTexture.*` | 623 | 2d-help | H | gpu, Layers | `StreamingTexture` (175) ist ein CPU-Framebuffer-Upload und gehört eher nach `knorke-gpu` **(Vermutung)**. Sprite hat eigenen Submit (`Sprite.cpp:274-297`). |
| `SurfaceGeometry.*` | 407 | 2d-help | H | Layers | 2.5D-Boxen, Wände, Böden in die Puffer. |
| `MediumDraw.*` | 269 | 2d-help | H | Layers, Medium | Eigener Submit (`MediumDraw.cpp:232-262`). |
| `RendererTexture*` | 98 | drop | – | – | `SDL_Renderer`-Interop, versteckter Submit (E62). |

### 1.7 `src/capi` und Header

| Gruppe | LOC | Ziel | Hinweis |
|---|---|---|---|
| `capi/CApi.cpp` | 2 038 | drop | C-ABI-Fassade. Knorke bekommt später `knorke-abi`, nicht diese. |
| `include/Chiaro/Chiaro.h`, `.hpp`, `_dynload.h`, `_bytes.h` | 5 207 | drop | Nur als Wörterbuch für Feldnamen und Standardwerte lesen. |

### 1.8 Shader (`shaders/src`, 6 028 LOC HLSL) und Build

| Gruppe | LOC | Ziel | Art | Hinweis |
|---|---|---|---|---|
| `common.hlsli`, `normal_codec.hlsli`, `fullscreen.vert`, `rect.vert` | 100 | gpu/post | G | |
| `bloom*`, `output.frag`, `composite.frag` | 532 | post (composite: light) | P / L | |
| `shockwave_light.*` | 76 | post | P | |
| `occluder`, `solid`, `tile*`, `occlusion*`, `clear`, `pattern_id`, `tile_pattern_id` | 333 | light | L | |
| `jfa_*` | 87 | light | L | |
| `light*`, `directional*`, `shadow_trace`, `profile`, `surface*` | 2 378 | light | L | |
| `pattern.hlsli` | 660 | light | L | |
| `ambient_zone*` | 185 | light | L | |
| `medium_*` | 1 597 | light | L | |
| `debug_check.*` | 80 | neu | – | |

Shader werden **nicht übersetzt**, sondern als Blobs (`shaders/compiled/`, 18 MB) plus Reflection übernommen (Inventur §4). Die Quelltext-Summe oben dient nur der Größenordnung. `cmake/*` (491 LOC) wird zu einem `build.rs` oder `xtask`; es ist kein Übersetzungsziel, sondern Spezifikation der Programmtabelle (`cmake/ChiaroLightPrograms.cmake`, 195 LOC).

### 1.9 Summen

| Block | LOC `src/` | Anteil |
|---|---|---|
| Generische GPU (G) | 1 160 | 5 % |
| 2D+3D Nachbearbeitung und Farbe (P) | 1 457 | 6 % |
| Nur 2D-Licht (L), inkl. Schatten, Medium, Layouts | 14 600 | 62 % |
| 2D-Helfer (H) plus 2D-Geometrie (Projection, Triangulate) | 2 273 | 10 % |
| Neu entwerfen (Settings, Context, Warnungen, Debug-Check) | 1 750 | 7 % |
| Entfällt (C-ABI, SlotMap/Batch/Ids, Renderer-Interop) | 2 340 | 10 % |
| **Summe** | **23 580** | |

Die Inventur §7.1 nennt „~17-19 kLOC fachlich“. Das passt: 23,6 kLOC minus 2,3 kLOC Drop minus Teile von „neu“ ≈ 19-21 kLOC. Davon ist aber nur der Block L die eigentliche Chiaro-Substanz.

## 2. Abhängigkeitsgraph

Belege aus `#include`-Zählung je Verzeichnis: `rendering` → `gpu`, `lighting`, `shadows`, `core`; `lighting` → `core`; `shadows` → `core`; `integration` → `gpu`, `core`, `lighting`; `core/Context` → fast alles.

```
Ebene 0 (frei, sofort startbar, alle parallel)
  A  gpu-Unterbau (Handle, Target, Sampler, Upload)      -> knorke-gpu
  B  Farbe/Formate/Skalierung, Projection, Triangulate    -> post / math
  C  Codecs (Pixel/Normal/Material)                       -> light
  D  Profile + Import      E  Masken      F  Ambient-Zonen
  G  Medium-CPU            H  Shockwave/CRT/Bloom-Mathe
  I  FrameWarnings -> Diagnostic

Ebene 1
  A -> ShaderLibrary + ProgramBinding -> PipelineCache      (gpu)
  LightStore + LightMath -> Culling/Area/Animation          (light-CPU)
  Triangulate -> ObstacleStore -> TileGrid, PatternStore
  LightMath -> OcclusionDomain/DirtyRects/MaskUpdatePlan

Ebene 2 (GPU-Stufen, brauchen PipelineCache)
  Output/Bloom/Shockwave-Pass        (B, H, gpu)
  Occluder-Raster + Occlusion/JFA    (ObstacleStore, MaskUpdatePlan, gpu)
  LightProgram-Tabelle -> LightingStage -> Composite
  Atlanten + AmbientZonePass          (D, E, F, gpu)
  Pattern-GPU                         (PatternStore, gpu)
  Medium-GPU                          (G, LightingStage, gpu)

Ebene 3
  Pass-Plan / Orchestrierung (ersetzt Context) -> knorke-integration
  2D-Helfer (Layers, Sprite, Surface, MediumDraw)
```

Harte Vorbedingungen: `PipelineCache` vor jeder GPU-Stufe. `LightProgram` vor `LightingStage`. `MaskUpdatePlan` vor `OcclusionMask`. Der Gate-Spike (Plan 5.1/2) braucht nur A, ShaderLibrary, PipelineCache, ein Licht-Shaderpaar und ein Okklusionsraster.

## 3. Arbeitspakete

Jedes Paket nennt Quelle (LOC) und Unit-Tests als Spezifikation. Größe ≤ ca. 1,5 kLOC C++. Modellstufe: **H** = Haiku (mechanisch, eng beschrieben, mit Tests), **S** = Sonnet (klare Umsetzung), **O** = Opus (knifflig). Parallel = Pakete derselben Spalte „Welle“ ohne Pfeil dazwischen.

Regeln für alle Pakete: `det::*` statt `f32::sin` usw. (nur CPU-Logik mit Simulationsbezug; reine Darstellungs-Mathe darf nach ADR 0013 lockern), kein `HashMap`, Handles statt Zeiger, Rust-Unit-Tests 1:1 aus den C++-Fällen, Diagnostics mit stabilen Codes.

### Welle 0 – Fundament und Referenz (sofort, parallel)

| Nr | Paket | Quelle | LOC | Tests | Modell | Parallel |
|---|---|---|---|---|---|---|
| P1 | `knorke-gpu`: Handle/Lebendzähler, RenderTarget, Samplers, UploadBuffer, StagingMath. Vorher `sdl3-sys`-Reife prüfen. | `gpu/GpuHandle.hpp`, `RenderTarget.*`, `Samplers.*`, `UploadBuffer.*`, `StagingMath.hpp` | 453 | StagingMath 3 + neue Lebensdauer-Tests | **O** (Lifetime/`unsafe`) | ja zu P3, P4-P12 |
| P2 | `knorke-gpu`: ShaderLibrary + ProgramBinding, `build.rs` bettet Blobs + Reflection-JSON ein; PipelineCache. | `gpu/ShaderLibrary.*`, `ProgramBinding.hpp`, `PipelineCache.*`, `cmake/EmbedShaders.cmake` | 665 | neu (Bindungsprüfung gegen JSON) | **O** (Bindungstabellen) | nach P1 |
| P3 | Golden Scenes und Paritätsmetrik mit Ausreißer-Budget; Harness für headless Offscreen. Kein Port, sondern C++-Demos mit `--screenshot --time` laufen lassen und PNGs ablegen. | Inventur §5, `examples/*`, `tests/gpu/FingerprintScene.hpp` | – | – | **S** | ja zu allem |

### Welle 1 – CPU-Logik ohne GPU (alle parallel zu Welle 0)

| Nr | Paket | Quelle | LOC | Tests | Modell |
|---|---|---|---|---|---|
| P4 | Farbe, Formate, Letterbox | `core/ColorMath.hpp`, `Formats.*`, `OutputRect.*` | 339 | 22 | **H** |
| P5 | Projektion (Ortho/Iso) + Triangulierung | `core/Projection.*`, `Triangulate.*` | 298 | 14 | **H** |
| P6 | Pixel-/Normal-/Material-Codec | `lighting/PixelCodec.*`, `MaterialMath.*` | 460 | 40 | **S** (Rundung der 8-Bit-Codecs) |
| P7 | Licht: Store + Math | `lighting/LightStore.*`, `LightMath.*` (SlotMap → `Slots`) | 966 | 19 | **S** |
| P8 | Licht: Culling, Fläche, Animation | `lighting/LightCulling.*`, `AreaLight.*`, `Animation*` | 1 103 | 35 | **S** (nach P7) |
| P9 | Profile + IES/EULUMDAT-Import | `lighting/Profile*`, `ProfileImport.*`, `tests/unit/data/profiles` | 1 048 | 26 | **H** (Parser, enge Tests) |
| P10 | Masken (Gobo/Fenster) | `lighting/LightMask.*`, `Mask.*`, `MaskStore.*` | 696 | 15 | **H** (nach P7) |
| P11 | Ambient-Zonen | `lighting/AmbientZone*` | 759 | 14 | **H** |
| P12 | Medium-CPU | `lighting/MediumCulling.*`, `MediumTable.*` | 297 | 9 | **H** |
| P13 | Hindernisse: Store, Desc, Stored | `shadows/ObstacleStore.*`, `ObstacleDesc.*`, `StoredObstacle.*` | 1 187 | 44 | **S** (nach P5) |
| P14 | Tile-Gitter + Muster an Hindernissen | `shadows/TileGrid.*`, `ObstaclePatterns.*` | 604 | – | **S** (nach P13) |
| P15 | Muster-Store + Geometrie | `shadows/PatternStore.*`, `PatternGeometry.*` | 841 | 25 | **S** (nach P13) |
| P16 | Schatten-Cache-Plan: MaskUpdatePlan, DirtyRects, OcclusionDomain | `shadows/MaskUpdatePlan.*`, `DirtyRects.*`, `OcclusionDomain.*` | 898 | 36 | **O** (Anker, Dirty-Rect-Faltung, Vollaufbau-Schwelle) |
| P17 | Post-Mathe: Shockwave, CRT, Bloom-Parameter | `core/Shockwave.*`, `CrtMath.hpp`, `Bloom.hpp` | 522 | 25 | **H** |
| P18 | Diagnose: FrameWarnings → `Diagnostic` | `core/FrameWarnings.*` | 161 | 11 | **S** (Redesign, stabile Codes) |
| P19 | Konfiguration: Settings + Qualitätspresets + Output-Grade | `core/Settings.*` | 571 | 39 | **S** (Redesign, siehe Abschnitt 4) |

Hinweis: Die GPU-POD-Strukturen `GpuLight` (55 LOC), `GpuZone` (26 LOC) werden in P7/P11 als GPU-freie `#[repr(C)]`-Typen mitgeführt (Rückkante, siehe 1.3).

### Welle 2 – GPU-Stufen (brauchen P2, je nach Paket weitere)

| Nr | Paket | Quelle | LOC | Braucht | Modell | Parallel |
|---|---|---|---|---|---|---|
| P20 | Post-Pass: Output + Bloom + Flags (Tonemap, Grade, Skalierung, Overlay) | `rendering/OutputStage.*`, `BloomStage.*`, `OutputFlags.hpp` | 470 | P2, P4, P17, P19 | **S** | ja zu P21-P24 |
| P21 | Druckwellen-Ringlicht-Pass | `rendering/ShockwaveLightPass.*` | 125 | P20 | **H** | |
| P22 | Hindernis-Raster: Geometrie + Tiles | `rendering/OccluderGeometry.*`, `TileOccluders.*` | 678 | P2, P13, P14 | **S** | ja zu P20 |
| P23 | Okklusionsmaske, Distanzfeld, Cache-Ausführung | `rendering/OcclusionMask.*`, `OcclusionStage.*`, `OcclusionCommon.hpp`, `SdfShadows.*` | 933 | P22, P16 | **O** (Schatten-Cache, Texturkopie beim Pan) | |
| P24 | Licht-Programmtabelle (92 Programme, Gültigkeitsregeln, Bindungsbelegung) | `gpu/LightProgram.hpp`, `cmake/ChiaroLightPrograms.cmake`, Test `LightProgramLayoutTests.cpp` | 469 | P2 | **O** | ja zu P20, P22 |
| P25 | Lichtstufe: Instanzen, Programmwahl, Bindung, Richtungslicht | `rendering/LightingStage.*`, `GpuLight.hpp`, `LightAtlases.hpp` | 813 | P24, P7, P8, P23 | **O** | |
| P26 | Atlanten + Ambient-Pass | `rendering/ProfileAtlas.*`, `MaskAtlas.*`, `AmbientZonePass.*` | 554 | P25, P9, P10, P11 | **S** | ja zu P27 |
| P27 | Composite (Szene × Licht + Glow) + Layout | `rendering/CompositeStage.*`, `gpu/CompositeLayout.hpp` | 250 | P25 | **S** | ja zu P26 |
| P28 | Muster-GPU: Atlas, ID-Textur, Occluder | `rendering/PatternOccluders.*`, `PatternAtlas.*`, `GpuPatternObstacle.hpp` | 890 | P23, P15 | **S** (Vermutung; Vulkan-Workaround spricht für **O**) | ja zu P29 |
| P29 | Medium-GPU, Teil 1: Karten + Layout | `rendering/MediumMapStage.*`, `gpu/MediumLayout.hpp` | 353 | P25, P12 | **S** | ja zu P28 |
| P30 | Medium-GPU, Teil 2: Streuung, Lichtstreifen | `rendering/MediumStage.*` | 780 | P29 | **S** | |

### Welle 3 – Zusammenführung und Helfer

| Nr | Paket | Quelle | LOC | Braucht | Modell |
|---|---|---|---|---|---|
| P31 | Pass-Plan statt `Context`: feste Stufenfolge, Prepare/Record, Ressourcen-Ein-/Ausgänge. Danach Einbindung in `knorke-integration` (Feature `gpu`), Snapshot/Inspect/Invariants. | `core/Context.*`, `ContextInspect.hpp`, `rendering/Pass.hpp`, `Frame.hpp` | 1 110 | P20, P25, P27 | **O** (Redesign) |
| P32 | Debug-Prüfung (opt-in, Readback nur auf Anforderung) | `rendering/DebugCheckStage.*` | 186 | P31 | **S** |
| P33 | 2D-Helfer: Layers | `integration/Layers.*` | 625 | P2, P31 | **S** (Redesign: kein eigener Submit) |
| P34 | 2D-Helfer: Sprite + Streaming-Upload | `integration/Sprite.*`, `StreamingTexture.*` | 623 | P33 | **S** |
| P35 | 2D-Helfer: Surface-Geometrie + Medium-Draw | `integration/SurfaceGeometry.*`, `MediumDraw.*` | 676 | P33, P30 | **S** |

### Gate (dazwischen, nicht Teil der Summe)

Der Gate-Spike (Plan §17.12 Schritt 2) nutzt P1, P2, P3 plus je einen schmalen Schnitt aus P22, P23, P24, P25 (ein Punktlicht, ein Schattenfall, Offscreen-Capture). Modell **O**. Sein Ergebnis entscheidet über alle Pakete ab Welle 2 (siehe Frage 2).

### Reihenfolge-Empfehlung

1. P1, P2, P3 und die freien CPU-Pakete P4, P5, P6, P7, P17 parallel. Das sind ~3 kLOC und sie nutzen auch bei Weg A (C-ABI) noch Teilen.
2. Gate-Spike.
3. Bei „Rust ja“: Rest von Welle 1 (P8-P16, P18, P19) parallel, dann Welle 2 in der Reihenfolge P20 → P22/P24 → P23 → P25 → P26/P27 → P28-P30, dann Welle 3.

Tests: pro Paket zuerst die portierten Unit-Tests (rot→grün). Analytische GPU-Tests (`tests/gpu/*`) nur für das jeweilige Feature mitnehmen, mit Toleranz.

## 4. Nicht übersetzen, sondern neu entwerfen oder weglassen

| Teil | LOC | Entscheidung | Grund |
|---|---|---|---|
| `capi/CApi.cpp`, `Chiaro.h`, `Chiaro.hpp`, `Chiaro_dynload.h`, `Chiaro_bytes.h` | 7 245 | weglassen | Plan §5.3: kein API-Bestandsschutz. Eine C-ABI kommt mit `knorke-abi`. Header nur als Feldwörterbuch. |
| `SlotMap`, `Batch`, `Ids` | 204 | ersetzen | `Slots`/`HandleMap`/`CommandBuffer`/`BitSet` aus `knorke-types` (ADR 0001/0008). |
| `RendererTexture*`, `CHI_FromRenderer`, `FlushOffscreenRenderers` (`Context.cpp:329-347`) | 98 + ~20 | weglassen | `SDL_Renderer`-Interop mit verstecktem Submit; ADR 0002 verbietet das. Pixelraum liefert eine CPU-Szene über Upload. |
| `CHI_RenderToWindow`, `Sprite`/`MediumDraw` mit eigenem Submit | ~60 | neu entwerfen | Zeichnen in den Command Buffer des Hosts (ADR 0002 Befund). |
| `DebugCheckStage` mit GPU-Readback | 186 | neu entwerfen | Nur auf ausdrückliche Anforderung; Befunde als `Diagnostic`. |
| `FrameWarnings` mit Warnbits | 161 | neu entwerfen | `Diagnostic` mit Codes `knorke.lighting.*`. |
| `Settings` als 204-Byte-Struct mit 18 Settern | 571 | neu entwerfen | Aufteilen in Licht-/Post-/Qualitätsdaten. Typen statt Flag-Bits. Presets und Standardwerte übernehmen. |
| `Context` (`RunStages`, Prepare/Record) | 832 | neu entwerfen | Wird expliziter Pass-Plan (Plan §7.2: erst kleiner Pass-Plan, kein Render-Graph). Die feste Stufenfolge bleibt. |
| Lichtprogramm-Kombinatorik (`cmake/ChiaroLightPrograms.cmake`, `gpu/LightProgram.hpp`) | 469 | neu entwerfen, aber Ergebnis exakt prüfen | Typisierte Merkmalsmenge statt Bit-Flags; Tabelle per `build.rs` aus einer Quelle erzeugen; Test, dass sie die 92 Programme und Slots 1:1 trifft (Inventur §7.3/1). |
| Bitgleichheits-Fingerprints (`FingerprintScene.hpp`, Fingerprint-Tests) | – | weglassen | Selbst die C++-Referenz ist auf der Intel UHD nicht lauf-zu-lauf-bitgleich (Inventur §1.3). Ersatz: Toleranzmetrik mit Ausreißer-Budget. |
| `tests/gpu` (24 174 LOC) | – | nur auswählen | Nur analytische Tests je portiertem Feature. Der Rest wäre eine zweite Portierung. |
| Beispiele, Benchmarks, C-/Consumer-/Docs-Tests (~7,6 kLOC) | – | weglassen | `shadow_bench` und zwei Demos nur als Messreferenz. |
| Bekannte Fehler (Grazing-Ray-Leck, gestapelte Puffs E95, `T_sun` E101) | – | pro Feature entscheiden | Erben oder reparieren; in Feature-Matrix §5.3 als `übernommen`/`neu entworfen` eintragen. |

## 5. Offene Fragen an den Besitzer (höchstens fünf)

1. **Wohin kommt die Nachbearbeitung (Bloom, Tonemap, Grade, CRT, Skalierung)?**
   Plan §7.2 schreibt sie `knorke-lighting` zu, aber sie ist nicht lichtspezifisch und soll auch ein 3D-Renderer nutzen. `knorke-render` ist heute der CPU-Renderer (ADR 0013). **Empfehlung:** eigene kleine Crate (Arbeitsname `knorke-post`, nur `knorke-gpu` als Abhängigkeit). `knorke-lighting` liefert einen linearen HDR-Zielpuffer, `knorke-integration` verbindet beide. Licht-CPU-Logik bleibt in `knorke-lighting` ohne GPU-Abhängigkeit, die GPU-Stufen hinter Feature `gpu`. Die 2D-Helfer (P33-P35) später als Feature in `knorke-render` oder eigene Crate. Entscheidung per ADR vor Welle 2 (so schon in Plan §17.12 vorgesehen).

2. **Wie viel vor dem Gate bauen?**
   Plan §5.1 verbietet ein blindes Komplett-Portieren. **Empfehlung:** vor dem Gate nur P1-P3 sowie die ohnehin nützlichen Pakete P4, P5, P6, P7, P17. Alles Weitere erst nach dem Gate-Ergebnis. Das kostet ~3 kLOC Risiko statt ~20.

3. **Welche Features kommen in den Port, welche bleiben in C++?**
   Medium/Volumetrik (P29, P30, ~1,4 kLOC plus 1,6 kLOC Shader), Muster (P28), Druckwellen (P21) und CRT sind als „neu“ eingestuft, haben bekannte Grenzen und Performanceziele wurden verfehlt (Inventur §3). **Empfehlung:** drei Wellen. Erst Kern (Licht, Schatten, Surface/Material, Post), dann Masken/Ambient/Profile, zuletzt Muster/Medium/Druckwelle/CRT, nur wenn ein Verbraucher sie braucht (Syndicate ist der heutige Verbraucher, Stand Inventur §6).

4. **Wohin mit den Shader-Blobs (18 MB, 604 Dateien) und dem Shader-Werkzeug?**
   Alles ins Knorke-Repo zu kopieren bläht es auf; nur Verweis auf `D:\Entwicklung\chiaro` macht den Build unreproduzierbar. **Empfehlung:** HLSL-Quellen plus die gebrauchte Teilmenge der Blobs in eine eigene Crate (`knorke-shaders`, Arbeitsname) oder `shaders/` mit Git-LFS; shadercross/DXC-Bootstrap nur im `xtask`, Blobs bleiben eingecheckt (Inventur §7.3/6). Reihenfolge der Blobs folgt den Paketen, nicht auf einmal.

5. **Namensraum von `knorke-lighting`: eine Crate oder zwei?**
   Das heutige `knorke-lighting` ist das CPU-Licht (ADR 0013). Chiaro ist GPU-Licht mit ~14,6 kLOC. **Empfehlung:** zwei Crates, `knorke-lighting` (CPU, unverändert) und `knorke-lighting-gpu` (Chiaro-Port, hängt an `knorke-gpu`), denn die Chiaro-CPU-Logik (Stores, Culling, Schatten-Cache) ist für den CPU-Pfad kaum nutzbar **(Vermutung)** und die Abhängigkeitsregeln lassen sich so einfacher erzwingen. Alternative laut Plan §17.12: eine Crate mit `cpu`/`gpu`-Teil. Entscheidung per ADR vor Plan-Schritt 3.
