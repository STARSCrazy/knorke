# Knorke Engine

Rust-first, agent-native, kompositionelle Game Engine. Fachmodule (Rendering, Beleuchtung, Navigation, KI, Wahrnehmung) sind einzeln nutzbar und werden über explizite Integrationsprofile verbunden. Herkunft: [Sagax](https://github.com/STARSCrazy/sagax) (KI/Navigation) und [Chiaro](https://github.com/STARSCrazy/chiaro) (Beleuchtung).

**Stand:** Phase 0 – Grundlagen für Datenaustausch und Analyse, Inventur. Noch keine Fachmodule.

- Plan: [`docs/plan.md`](docs/plan.md)
- Entscheidungen: [`docs/adr/`](docs/adr/README.md) (Index mit Status)
- Bestandsaufnahme: [`docs/inventory/`](docs/inventory/)

## Crates

| Crate | Inhalt |
|---|---|
| `knorke-types` | Datenverträge: `Handle`/`Slots`/`HandleMap`/`BitSet`, `Column`/`project`, `EventQueue`/`CommandBuffer`/`Revision`, `Parallel`/`Serial`, `Tick`/`Seed`/`StableHasher`, `Snapshot`/`Checksum`, `Diagnostic`. Ohne Abhängigkeiten, `no_std` + `alloc` |
| `knorke-math` | `Vec2/3/4`, `IVec2/3`, `Affine2`, `Mat4`, `LinearRgba`, `Srgba8`, Geometrie `Aabb2`/`Circle`/`Segment2`/`Capsule2`/`Ray2`, plattformunabhängige Mathematik `det` (inkl. `min`/`max`), Koordinatenräume `At<Space, V>` – festes Layout, ohne Kopie umdeutbar (`bytemuck`), optional `mint` |
| `knorke-observe` | Debugging-Verträge `Sink`, `Probe`, `Inspect`, Fehlererkennung `Invariants`, `ProgressWatch`, Bildheuristiken, begrenzter `Collector` (`no_std`); mit `std`: `Recorder`, Export nach Perfetto/Chrome-Trace und JSON |
| `knorke-grid` | Raumtypen der KI-Module: `Cell`, `Pos`, `HexLayout`, ganzzahlige Richtungstabellen `sine` (`no_std`) |
| `knorke-navigation` | Wegsuche: A*/Dijkstra (`Search`), D* Lite, Flow Fields, Glättung, Formationen, `Navigator` mit Budget; quadratisch und hex, mit Ebenen |
| `knorke-perception` | Wahrnehmung: Sichtkegel, exakte Sichtlinien mit Nebel und Licht, Hören, Aufmerksamkeit, `Awareness`, `Contagion` |
| `knorke-ai` | Entscheidungen: Utility-`Decider`, `StateMachine`, Gedächtnis (`Knowledge`, `Alertness`, `GoalGuess`), Missionen, Gruppen (`Crew`, `Squad`) |
| `knorke-render` | CPU-Software-Renderer im Pixel-Look (aus pixelraum): Raster, Kamera, Gitter, Texturen, Sprites, Picking, Palette, Blending; liefert G-Buffer-Samples ans Licht, sRGB-Ausgabe einmal am Ende |
| `knorke-lighting` | CPU-Licht und Schatten für `knorke-render` (Punktlichter, Sonne, Schattenkarten), linear gerechnet; eingefroren, neues Licht entsteht auf der GPU (ADR 0014) |
| `knorke-light2d` | 2D-Licht aus Chiaro (ADR 0014), bisher der GPU-freie Teil: Projektion, Triangulierung, Pixel-/Material-Codecs, Licht-Store mit Handles und Snapshots, Lichtmathematik; GPU-Stufen folgen hinter Feature `gpu` |
| `knorke-integration` | Verbindungscode zwischen den Modulen, Features je Profil: `ai` (Wahrnehmung → Gedächtnis → Navigation) und `pixel` (Renderer + Licht); Prelude, Szenario- und Bildtests |
| `knorke-replay` | Replays: Aufzeichnung, Verifikation mit abweichendem Tick und Modul, Sprung zu Tick, Flight Recorder, Minimierung fehlerauslösender Commands |
| `knorke-exec` | Executoren für `Parallel` auf Host-Seite: `Rayon` (Work-Stealing, Standard), `ScopedThreads` (ohne Abhängigkeit, nur für grobe Arbeit) |
| `knorke-contract-tests` | crate-übergreifende Vertragstests (kein Kopieren, 0 Allokationen, Layouts), nicht ausgeliefert |
| `xtask` | Repo-Automatisierung, nicht ausgeliefert |

Wie Module Daten austauschen: [ADR 0006](docs/adr/0006-data-exchange.md). Debugging und Analyse: [ADR 0007](docs/adr/0007-observability.md). Events, Commands, Parallelität: [ADR 0008](docs/adr/0008-events-commands-parallelism.md). Frame-Ablauf: [ADR 0009](docs/adr/0009-frame-schedule.md). Snapshots, Replay, Fehlererkennung: [ADR 0010](docs/adr/0010-snapshot-replay-bug-detection.md). Plattformübergreifender Determinismus und abschaltbare Zusatzfeatures: [ADR 0011](docs/adr/0011-cross-platform-determinism-and-optional-features.md). Sagax-Neuschnitt: [ADR 0012](docs/adr/0012-sagax-split.md).

Regeln für die Arbeit am Projekt: [`CLAUDE.md`](CLAUDE.md). Plan, Stand und Ideen-Backlog: Abschnitte 16 und 17 im Plan.

Weitere Crates entstehen erst mit Code und Verbraucher (ADR 0001 §8).

## Entwickeln

```sh
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features
cargo xtask deps         # verbotene Abhängigkeitskanten (ADR 0001)
cargo xtask standalone   # jede Bibliothek einzeln, ohne Default-Features
cargo bench -p knorke-contract-tests   # Zugriff, Lookups, Events, Instrumentierung, Parallel-Frame

# Determinismus-Fingerprint auf WASM ohne wasmtime (braucht Node ≥ 20 und `rustup target add wasm32-wasip1`):
cargo test -p knorke-contract-tests --test determinism --target wasm32-wasip1 --no-run
node xtask/wasi-run.mjs target/wasm32-wasip1/debug/deps/determinism-*.wasm
```

Referenz-Repos für Portierung/Vergleich liegen unversioniert unter `reference/`:

```sh
git clone https://github.com/STARSCrazy/sagax  reference/sagax   # fab00c0
git clone https://github.com/STARSCrazy/chiaro reference/chiaro  # 9bc5b9b
```

## Lizenz

Wahlweise [MIT](LICENSE-MIT) oder [Apache 2.0](LICENSE-APACHE), wie im Rust-Ökosystem üblich und wie Sagax. Beiträge stehen unter beiden Lizenzen, sofern nicht anders angegeben.
