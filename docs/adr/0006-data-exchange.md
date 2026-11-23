# ADR 0006 – Datenaustausch ohne Kopien

**Status:** angenommen
**Datum:** 2026-11-23

## Kontext

Plan-Prinzip 7: keine Serialisierung, kein Event-Bus und keine Formatkonvertierung pro Element im Hot Path. Ohne gemeinsame Regeln entstehen sie trotzdem: Jedes Modul definiert eigene Vektortypen und verlangt `Vec<T>`, also baut der Host jedes Frame Arrays zusammen und rechnet sie um.

## Entscheidung

### 1. Module leihen sich Daten, sie besitzen sie nicht

- Massendaten werden als **`impl Column<T>`** (bzw. `impl ColumnMut<T>` zum Zurückschreiben) **per Wert** übergeben. Slices, `Vec`, Arrays und Referenzen darauf implementieren `Column`; Aufrufer schreiben `&positions[..]`.
- Liegen die Werte **in den Structs des Hosts** (`struct Npc { pos, hp, … }`), übergibt der Host `project(&npcs, |n| &n.pos)`. Es wird kein Zwischenarray gebaut; nach Monomorphisierung ist der Zugriff derselbe Maschinencode wie ein direkter Feldzugriff.
- Ergebnisse schreibt ein Modul in **vom Host bereitgestellte** Puffer (`impl ColumnMut<T>`) oder in eigenen, wiederverwendeten Zustand – nie in pro Frame neu allozierte `Vec`s.
- Was ein Modul pro Abfrage braucht (Kosten einer Zelle, Sichtbarkeit), holt es über **Query-Traits**, die das Modul definiert und der Host implementiert (Vorbild: Sagax `CellMap`, `SenseMap`). Generisch, also ohne dynamischen Dispatch.

### 2. Gemeinsame Werttypen mit festem Layout (`knorke-math`)

- `Vec2/3/4`, `IVec2/3`, `Affine2`, `Mat4`, `LinearRgba`, `Srgba8`: `#[repr(C)]`, nur `f32`/`i32`/`u8`, kein Padding, `bytemuck::Pod`. Größen und Offsets sind Vertrag (`tests/contracts/tests/layout.rs`).
- Dadurch sind `&[Vec2]` ↔ `&[f32]` ↔ `&[u8]` (GPU-Upload) ↔ gleich aufgebaute Fremdtypen (glam mit `bytemuck`-Feature, `[f32; 2]`) **ohne Kopie** umdeutbar.
- **Koordinatenräume** sind Typen: `At<World2, Vec2>`, `At<Screen, Vec2>`, `At<Tile, IVec2>`; eigene Räume über `trait Space`. `#[repr(transparent)]`, also kostenlos; ganze Slices werden mit `At::slice_raw` / `At::slice_from_raw` ohne Kopie umgedeutet. Übergänge zwischen Räumen sind `Transform2<A, B>` und werden beim Kompilieren geprüft.
- Farbe: Rechnen in `LinearRgba`; `Srgba8` nur an den Rändern (Texturen, Ausgabe), exakte sRGB-Kurve.
- `bytemuck` ohne Derive (keine proc-macro-Kette `syn`/`quote`); die `Pod`-Impls stehen von Hand in `pod.rs`. Jede Impl nennt die Felder mit Typ; daraus prüft der Compiler: die Liste ist vollständig, jedes Feld ist `Pod`, die Offsets liegen lückenlos hintereinander, die Struct ist genau so groß wie ihre Felder (kein Padding). Ein neues Feld ohne Eintrag oder mit Padding bricht den Build.
- `Srgba8` hat Alignment 1: `&[Srgba8]` → `&[u32]` geht nur an 4-ausgerichteten Adressen (`bytemuck` prüft das zur Laufzeit). Umgekehrt (`&[u32]` → `&[Srgba8]`) und als Bytes immer. `mint` optional für glam/nalgebra/cgmath.
- `knorke-math` ist **keine** allgemeine Lineare-Algebra-Bibliothek. Was nur ein Modul braucht, bleibt dort.

### 3. Identität und modul-eigener Zustand (`knorke-types`)

- `Slots<T>` vergibt `Handle<T>` (Index + Generation). `HandleMap<K, V>` hält modul-eigene Daten zu fremden Handles als dichtes Array: ein Lookup = Bounds-Check + Generationsvergleich, keine Hash-Map. Ein wiederverwendeter Slot erbt nie den Zustand seines Vorgängers, und ein `insert` über das Handle des Vorgängers überschreibt nie die Daten des Nachfolgers (Generationen eines Slots wachsen nur).
- `Handle::cast` nur an expliziten Grenzen (Integrationsadapter).

### 4. Inkrementelle Änderungen statt Neueinlesen

- `Revision`: Zähler pro Datenbestand; gleiche Revision → Konsument überspringt die Arbeit.
- `EventQueue<E>`: Besitzer hängt Änderungen mit `Tick` an; jeder Konsument hat einen eigenen `EventCursor` und bekommt ungelesene Einträge als geliehene Slices. Wurde schon ausgemustert, was ein Konsument noch nicht gelesen hat, bekommt er explizit `Events::Resync` – er verpasst also nie still eine Änderung. `Resync` bekommt auch, wessen Cursor hinter dem Ende der Queue steht (Queue per Snapshot zurückgesetzt, Cursor nicht). Der Cursor gehört zum Zustand des Lesers und ist `SnapshotValue`. Details und Command Buffer: ADR 0008.

### 5. GPU und FFI

- GPU: Texturen/Buffer werden zwischen Renderer und Lighting über das gemeinsame Device geteilt (ADR 0002), nicht über die CPU. Für Uploads sind `Pod`-Slices direkt als Bytes nutzbar – die einzige Kopie ist die unvermeidbare in den Transfer-Buffer.
- FFI (C/C#/WASM): Die `knorke-math`-Typen sind bereits ABI-stabil (`repr(C)`), Handles gehen als `u64` (`Handle::to_bits`). Eine `knorke-abi`-Crate mit `repr(C)`-Slice-/Strided-Views entsteht erst mit dem ersten FFI-Verbraucher (ADR 0001 §7).

## Nachweis

`tests/contracts` (eigene Crate, nicht ausgeliefert):
- `exchange.rs`: Host-Structs → Navigation-Stand-in → Ausgabepuffer, Event-Queue, Recorder. Zeigergleichheit beweist „keine Kopie“; ein zählender Allocator beweist **0 Allokationen** in 10 Steady-State-Frames mit 1000 Entities (mit Recorder und mit No-op-Instrumentierung).
- `layout.rs`: Größen, Alignments, Offsets.

## Abhängigkeitsgrenzen (in `cargo xtask deps` erzwungen)

| Crate | erlaubte Abhängigkeiten (transitiv) |
|---|---|
| `knorke-types` | keine |
| `knorke-math` | `bytemuck`, `mint` (optional), `knorke-types` (Snapshots, ADR 0010), `libm` (deterministische Mathematik, ADR 0011) |
| `knorke-observe` | `knorke-types` |

## Konsequenzen

- Modul-APIs werden generisch (`impl Column<…>`, `impl Probe`, `impl Sink<…>`). Das kostet Compile-Zeit und gelegentlich lange Signaturen; Abhilfe sind Kontext-Structs pro Modul, nicht `dyn` im Hot Path.
- Gleichzeitig lesen und schreiben in **denselben** Host-Structs geht nicht über zwei Columns (Rust-Aliasing). Muster: lesen per `project`, schreiben in einen Host-Puffer, oder das Modul definiert ein Query-Trait mit `&mut`-Zugriff pro Element.
