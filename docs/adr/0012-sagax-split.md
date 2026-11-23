# ADR 0012 – Sagax-Neuschnitt: Grid, Navigation, Perception, AI, Integration

**Status:** angenommen
**Datum:** 2026-11-23

## Kontext

Sagax (`https://github.com/STARSCrazy/sagax`, Revision `fab00c0`, lokal `D:\Entwicklung\SmartLib`) ist die bestehende Rust-KI-Bibliothek (Inventur `docs/inventory/sagax.md`). Plan 6.1 gibt die fachlichen Ziele vor (`knorke-navigation`, `knorke-perception`, `knorke-ai`), Plan 6.2 die Regeln (Weltzugriff über Queries, Bindings nur bei Bedarf, Integration außerhalb des KI-Kerns). Kompatibilität zu Sagax-Ergebnissen ist nicht verlangt (Plan 16).

Abhängigkeits-Audit des Quellstands (`use sagax_*` in allen Dateien):

| Sagax-Crate | braucht aus `sagax-core` | braucht andere Fach-Crates |
|---|---|---|
| `sagax-nav` | `Cell`, `HexLayout`, `axial_step`, `Sink` | – |
| `sagax-perception` | `Cell`, `Pos`, `axial_step`, `Sine`/`sine`, `mix`, `Sink` | – |
| `sagax-decide` | `Cell`, `Pos`, `HexLayout`, `Sine`/`sine`, `mix`, `Sink` | – |
| `sagax` (Fassade) | alles | nav, perception, decide |

Die Raumtypen braucht **jede** Fachcrate, aber keine Fachcrate eine andere.

Baseline des Originals (`cargo test --workspace` in `SmartLib`, eigenes Target-Verzeichnis, Windows, 2026-11-23): **785 bestanden, 0 fehlgeschlagen, 1 ignoriert**, 1 min 10 s inklusive kaltem Build. Davon in den übernommenen Crates: `sagax-core` 8, `sagax-nav` 77, `sagax-perception` 97, `sagax-decide` 346, Fassade `sagax` 67 (13 Szenario-Testdateien) = 595; Rest `sagax-ffi` 143 (+1 ignoriert), `sagax-abi` 30, `sagax-debug` 4, Doc-Tests.

## Entscheidung

### Crate-Grenzen

| Knorke-Crate | Quelle | Abhängigkeiten |
|---|---|---|
| `knorke-grid` | `sagax-core`: `cell.rs`, `hex.rs`, `turn.rs` (nur `Sine`/`sine`) | `knorke-types` (nur `SnapshotValue`) – geschlossene Liste |
| `knorke-navigation` | `sagax-nav` | `knorke-grid`, `knorke-observe` (ohne `std`), `knorke-types` |
| `knorke-perception` | `sagax-perception` | dito |
| `knorke-ai` | `sagax-decide` (Module `decide` = Crate-Wurzel, `state`, `memory`, `mission`, `group`) | dito |
| `knorke-integration` | Fassade `sagax`: `perception`, `goal_guess`, `walk`, `rules`, `prelude`, `debug` | alle oben |
| `knorke-observe` (erweitert) | `sagax-debug`: `Collector`, `Overflow`, `Record` | unverändert nur `knorke-types` |

1. **`knorke-grid` als eigene Crate**, weil drei unabhängige Module (Navigation, Wahrnehmung, KI) dieselben Raumtypen brauchen. In `knorke-navigation` gelegt, müssten Perception und AI die Navigation mitziehen; in `knorke-types` verbietet sie ADR 0001 §6 (keine Koordinatentypen). Inhalt: `Pos`, `Cell`, `HexLayout`, `axial_step`, `Sine`, `sine`, dazu `SnapshotValue` für `Cell`, `HexLayout`, `Sine`. `no_std`.
2. **Navigation, Perception und AI kennen sich nicht.** Neue Regeln in `xtask/src/deps.rs`: keine dieser drei darf (transitiv) von einer der anderen abhängen; `knorke-grid` nur von `knorke-types`; `knorke-grid` zählt zu den Headless-Modulen, darf `knorke-integration`, `knorke-exec` und `rayon` nicht nutzen. Nur `knorke-integration` verbindet sie (ADR 0001 entsprechend ergänzt).
3. **`sagax-decide` bleibt eine Crate** (`knorke-ai`) mit internen Modulen statt vier Crates: Es gibt keinen Verbraucher, der z. B. `mission` ohne `decide` braucht (Inventur Beobachtung 3). Aufteilen erst bei einem solchen Verbraucher.
4. **Die Fassade wird Integrationslogik.** `knorke-integration` trägt den Verbindungscode (Percepts → Knowledge, Flow Field → GoalGuess/Gehdistanzen, Navigationskosten → Squad-Regeln) samt Prelude und allen Szenario-Tests und -Beispielen. Die Fassade „Sagax in einer Crate“ entfällt als Begriff. Der Abschnitt Pixelraum-Integration einer parallel entstehenden Fassung von `knorke-integration` (anderer Arbeitszweig) wird beim Zusammenführen als weiteres Modul daneben gestellt.

### Ersetzt durch Knorke-Grundlagen

- **Traces:** `sagax_core::{Sink, NoopSink, CallbackSink, MapSink}` → `knorke_observe::*` (gleiche Form, ADR 0007), mit `default-features = false`. Alle `_traced`-Varianten bleiben; ohne Sink kostet nichts (ADR 0011).
- **`sagax-debug::Collector`** (begrenzte Sammlung) → `knorke_observe::Collector`, auf `alloc` statt `std`. Abweichung vom Auftrag „`sagax-debug` nicht migrieren“: Die portierten Tests brauchen den Collector, und `knorke-observe` fehlte ein begrenzter Sink für lange Sitzungen (der eigene `Vec`-Sink wächst unbegrenzt). 146 Zeilen, keine neue Crate.
- **Zufall:** `sagax_core::mix(x)` (ein SplitMix64-Schritt) → `knorke_types::Seed::derive`. Wo Sagax `mix(mix(a) ^ b)` rechnete (Squad-Ziehung), ist `Seed(a).derive(b)` **bitgleich**. Sonst ändern sich die Zufallsfolgen: `choose` zieht aus `Seed(seed).derive(0)`, `hear` aus `Seed(seed).derive(ear << 32 ^ stimulus)` mit Neuziehung `Seed(h).derive(0)`, Replan aus `Seed(config.id).derive(now)`. Kein Szenario-Test der Fassade hing an den alten Werten. Ein Abdeckungstest der Sichtlinien (`line_matches_brute_force_hex`) erreichte mit den neuen Folgen nur 95 statt > 100 freie Linien und prüft jetzt 250 statt 200 Zufallskarten; die Schranke blieb. Der Test, der `mix(0)`/`mix(42)` festnagelte, prüft jetzt, dass `choose` aus `Seed::derive` zieht. Seed-Parameter bleiben vorerst `u64` (Plan 17.5).
- **Determinismus-Regeln:** Simulationscode war bereits ganzzahlig. Einzige Treffer: `HashMap`/`HashSet` in einem Referenztest der Wegsuche → `BTreeMap`/`BTreeSet`; `f64::exp` in einem Toleranztest von `goal_guess` → lokal erlaubt mit Begründung (Referenzwerte, der geprüfte Code ist ganzzahlig).
- **Diagnosen:** alle öffentlichen Fehler von `knorke-ai` (`DecideError`, `UnknownState`, `ZeroCapacity`, `StepError`, `CrewError`, `PlanError`, `MissionError`) haben `to_diagnostic()` mit Codes `knorke.ai.<modul>.<fall>`. Navigation und Perception haben keine Fehlertypen.

### Zustand sichern (ADR 0010)

- `knorke-navigation`: `Navigator` implementiert `Snapshot` (Abschnitt `knorke.navigation`, Version 1) **einschließlich einer durch das Budget angehaltenen Suche**; `Topology` und Kapazität sind Konstruktionsparameter und werden nicht gesichert. `Search<N>` und `FlowField<N>` sind `SnapshotValue`: Knotenindex wird aus den Records neu aufgebaut, die Open-Liste in Heap-Reihenfolge geschrieben und unverändert gelesen (gleiche Pop-Reihenfolge, gleiche Bytes). Restore lehnt Indizes außerhalb der Records und doppelte Knoten ab.
- `knorke-perception`: `Awareness` (Stufe ≤ 2 geprüft) und `Contagion` (mit Konfiguration; gleiche Kapazität ohne Allokation) sind `SnapshotValue`.
- `knorke-ai`: `StateMachine` (Zustände außerhalb der Maschine abgelehnt), `Knowledge` (mehr Fakten als Kapazität abgelehnt) und `Alertness` sind `SnapshotValue`.
- Wahrnehmungs- und KI-Zustand gehört **pro Figur** zum Host; deshalb `SnapshotValue` statt eines eigenen Abschnitts pro Figur. Der Host sichert sie in seinem Abschnitt (wie `HandleMap<K, V>`). Ein modul-eigener Abschnitt entsteht, sobald ein Modul eine eigene Sammlung pro Welt besitzt.
- Replay-Test je Modul mit Zustand (`tests/replay.rs` in navigation, perception, ai): aufnehmen → `verify` → `seek` gegen geraden Lauf, bytegleich; Keyframes fallen absichtlich mitten in laufende Suchen.
- **Nachgezogen 2026-11-23** (Plan 17.5), ebenfalls als `SnapshotValue`: `DStarLite`, `GoalGuess`, `Mission`, `Crew`, `Squad`, `GoalKeeper`, `Areas`, `Timers`, `Inbox`/`Outbox`, `Route`/`Progress`, `Spots`.

### Nicht übernommen

- `sagax-abi`, `sagax-ffi` (C-ABI, 23,8 kLOC), `sagax-wasm`: Plan 6.2 §6 – Bindings erst für einen konkreten Verbraucher, dann neu entworfen gegen `knorke-*`. Kein Browser-Ziel (Plan 16).
- `sagax-debug` als Crate (Inhalt wanderte nach `knorke-observe`, siehe oben).
- `sagax_core::mix`, `sagax_core::Sink` & Co. (ersetzt) sowie die drei Sink-Tests von `sagax-core` (durch die Tests in `knorke-observe` abgedeckt).

### API-Änderungen (Migrationspfad)

| Sagax | Knorke |
|---|---|
| `sagax_core::{Cell, Pos, HexLayout, axial_step, Sine, sine}` | `knorke_grid::…` (`knorke_navigation::grid::Cell`, `::hex::HexLayout` re-exportieren weiter) |
| `sagax_core::{Sink, NoopSink, CallbackSink, MapSink}` | `knorke_observe::…` |
| `sagax_core::mix`, `sagax_decide::mix` | `knorke_types::Seed(x).derive(stream)` – andere Werte |
| `sagax_debug::{Collector, Overflow, Record}` | `knorke_observe::…` |
| `sagax_nav` / `sagax_perception` / `sagax_decide` | `knorke_navigation` / `knorke_perception` / `knorke_ai` |
| `sagax::{prelude, debug, perception, goal_guess, walk, rules, nav, decide}` | `knorke_integration::…` (gleiche Modulnamen) |
| C-ABI `sx_*`, WASM-Klassen | entfallen bis zu einem Verbraucher |

## Konsequenzen

- Herkunft jeder übernommenen Datei steht in ihrer ersten Zeile (Repo, Revision, Pfad).
- Tests: alle 595 Tests der übernommenen Sagax-Crates laufen in Knorke (3 Sink-Tests ersetzt, 1 Festwert-Test umgestellt), dazu Snapshot-, Replay- und Diagnose-Tests.
- Noch nicht umgesetzt und in Plan 17.5 geführt: `Parallel`-Vertrag (Sagax rechnet pro Figur seriell; Parallelität heute nur beim Host über Figuren), `Column`-Eingaben statt Slices (`&[Candidate]`, `&[Target]`, Eingabezeilen), `Seed` statt `u64` in Signaturen, `Tick` statt `u32`-Zeit, Snapshot der restlichen Zustandstypen, `Invariants`/`ProgressWatch` der Module, Benchmarks gegen Sagax.
- Der Determinismus-Fingerprint (`tests/contracts`) ist unverändert; die neuen Crates sind nicht Teil davon.
