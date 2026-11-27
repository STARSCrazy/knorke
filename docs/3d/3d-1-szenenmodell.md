# Schritt 3D-1: Szenenmodell aus `knorke-render` herauslösen

Stand: 2026-11-27 · Bezug: ADR 0021 (Abschnitt „Schnitt 3D-1“), Plan 17.14

## Ist-Stand

`knorke-render` arbeitet im Immediate Mode: `begin(&Camera)` (`renderer/mod.rs:431`) → `draw_grid`, `draw_sprite`, `draw_triangles_ex`, `draw_particles` und `draw_emitters` → `prepare_light_samples` und `light_samples` → Lichtpass in `knorke-integration/src/pixel.rs` → `end_lit`.

Gerastert wird schon im Draw-Aufruf. Die Reihenfolge der Aufrufe bestimmt deshalb das Bild mit: Gleichstand im Tiefentest, `blend_order` und Pick-IDs. Für Bitgleichheit muss die Szene genau diese Reihenfolge erhalten.

| Daten | Besitzer heute | künftig |
|---|---|---|
| Texturen, Opak-Flags, `tex_gen`, Animation/Scroll (`TexEffects`) | `Renderer` | Szene |
| Legacy-Himmel, Backdrops, Emitter | `Renderer` | Szene |
| `time` | `Renderer` und `LightScene` doppelt | Frame-Eingabe |
| GridMap, Sprites, Dreieckslisten, Partikel, Kamera | Host, pro Frame | Szene |
| Sonne, Ambient, Punktlichter, Sky-Tint | `LightScene` | Szene, erst mit 3D-3 |
| G-Buffer, `View`, Ausgabepuffer, Frame-Aufzeichnungen, Scratch, `Options`, Palette | `Renderer` | bleibt Renderer-Zustand |

Doppelte Arbeit heute:
- `draw_grid` baut jeden Frame für jede sichtbare Säule die Quads neu (`draw.rs:118-162`).
- Mesh-Normalen werden pro Dreieck und Frame neu berechnet (`draw.rs:309`).

## Form

**Neue Crate `knorke-scene3d`**. Sie ist Render-Eingabe, kein Authoring (das ist `knorke-scene`, ADR 0018) und kein Simulationszustand. Sie hängt nur von `knorke-types` und `knorke-math` ab und nutzt die gelockerte `clippy.toml` der Darstellungs-Crates.

**Unverändert verschoben:**
- `Camera` (ohne `View`)
- `GridMap`, `Cell`, `Wall`, `Span`, `GridHit`, `Quad`, mit `cell_quads`, `shadow_quads`, `raycast` und `surface_at`
- `Vertex` (`repr(C)`, `Pod`)
- `Sprite`
- `Particle`, `Emitter`, `emit_one`
- die Backdrop-Daten
- `Texture`

**Neu:**
- **`Textures`:** Texturen, Opak-Flags, `TexEffects`, dazu `Revision` statt `tex_gen`. `resolve(id, time)` ist eine gemeinsame Funktion für beide Wege.
- **`GridMesh`:**
  - Ein Cache pro Säule, in derselben Reihenfolge wie die Draw-Schleife (z → x → Stockwerk).
  - `GridMap` bekommt `revision` und `dirty: BitSet`; `cells` wird privat.
  - `GridMesh::refresh(&GridMap)` baut die 3×3-Nachbarschaft jeder schmutzigen Säule neu.
  - Animation und Scroll bleiben zur Draw-Zeit, damit der Cache nicht von der Zeit abhängt.
- **`Mesh`:** Dreiecke mit beim Einfügen berechneten Normalen, über `Handle<Mesh>` angesprochen. Dazu `MeshDraw { mesh, texture, light, cull, cast, tag, alpha, blend }`.
- **`Scene3d`:** Texturen, Gitter, Meshes, Emitter und Backdrops (nach ID geordnet) und der Himmel. `prepare()` ist der Sync-Punkt (ADR 0008). Danach lesen beide Renderer nur noch `&Scene3d`.
- **`DrawList`:** pro Frame, wiederverwendet. Die Reihenfolge der Einträge ist verbindlich.
- **Material:** In 3D-1 gibt es keine eigene Tabelle; der Schlüssel ist (Textur, Blend, Alpha, Wrap, Cull).

**Kein Snapshot:** Die Szene ist Render-Eingabe. Nach einem Replay-Seek baut der Host sie neu auf.
- Optional gibt es eine Invariante „Cache = frischer Bau“, die abschaltbar ist (ADR 0011).
- Im eingeschwungenen Frame wird nichts alloziert. `no_alloc_pixel.rs` bekommt einen Szenen-Frame mit animierter Tür.

**Abhängigkeitsregeln:** siehe ADR 0021, Abschnitt „Schnitt 3D-1“.

## Nutzung

**CPU (bitgleich per Konstruktion):**
- `draw_item(&Scene3d, &DrawItem)` ruft dieselben Kernel wie bisher. Der Schleifenrumpf von `draw_grid` wird zu `draw_grid_column`.
- `LitRenderer` besitzt `Scene3d`, `Renderer` und `LightScene`. Er behält die pixelraum-API: Jeder Aufruf erzeugt ein `DrawItem` und führt es sofort aus. Sofort ausführen ist Pflicht, weil Tests die Optionen zwischen zwei Draws ändern.
- Damit laufen alle Goldens über den Szenenweg.

**GPU (3D-2, hier nur als Passprobe):**
- Gitter: `GridMesh` wird pro Chunk in einen Vertex-Puffer geladen, neu hochgeladen bei gesetztem `dirty`-Bit, mit derselben Diagonale.
- Meshes: ein Puffer je Revision.
- Sprites und Partikel: als Instanzen.
- Materialien: eine Tabelle pro Frame aus `Textures::resolve`.
- Backdrop: als Vollbild-Pass.
- Kamera: Off-Center-Frustum aus `shift_x/y`.
- Transparenz und Sortierung weichen ab, deshalb wird mit Toleranz verglichen.

## Migration

| # | Inhalt | Prüfung |
|---|---|---|
| C1 | Crate anlegen, Datentypen, Gitter, Kamera, Vertex, Sprite, Partikel, Emitter und Backdrop-Daten verschieben; `knorke-render` exportiert auf alten Pfaden weiter; Workspace-, deps- und standalone-Regeln | alle Tests und Goldens unverändert |
| C2 | `Textures`-Store nach scene3d | Goldens, `caches.rs` |
| C3 | `GridMesh`, `revision`/`dirty`, `cells` privat, `draw_grid_column` geteilt | Property-Test „Cache = frischer Bau“, Goldens |
| C4 | `Mesh` mit Handles, Normalen-Cache | Mesh-Goldens |
| C5a | Renderer besitzt keine Texturen mehr (`&Textures` als Parameter) | Goldens |
| C5b | `Scene3d`, `DrawList`, `draw_item`, `LitRenderer` auf dem Szenenweg | alle Goldens Debug + Release, `no_alloc_pixel` mit Tür |

**Risiken:**
- Reihenfolge von Draws, Blend- und Pick-IDs
- die öffentliche Umgehung über `cells`
- `shadow_quads` hängt über den Animations-Frame von der Zeit ab und wird deshalb nicht gecacht
- einige `pub(crate)`-Konstanten werden öffentlich

**Nicht in 3D-1:**
- Instanz-Transformationen, Szenengraph, glTF
- Materialtabelle
- Sichtbarkeit
- Caches für Schatten und Werfer
- GPU-Code, Snapshot
- Änderungen an `knorke-lighting`
- Entfernen der Immediate-API
- Licht-Daten in der Szene (kommen mit 3D-3)

## Entscheidungen (Vorgabe, 2026-11-27)

1. **`GridMap` ist nur Render-Eingabe.** Das Spiel überträgt seine Karte hinein; die Simulation hat ein eigenes Modell (Türen rechnen mit `std`-Trigonometrie, kein `det`).
2. **Licht-Daten kommen erst mit 3D-3 in die Szene.** Vorher gibt es keinen zweiten Nutzer.
3. **Die Immediate-API bleibt in 3D-1** als dünne Hülle. Ob sie dauerhaft bleibt, wird mit `knorke-abi` in 3D-4 entschieden.
