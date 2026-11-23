# Inventur Sagax

**Revision:** `fab00c059351111c7baf28567787057998ad453e` (2026-11-21), Workspace-Version 0.16.0, Edition 2024, MSRV 1.88, Lizenz MIT OR Apache-2.0
**Lokal:** `reference/sagax` (nicht versioniert)
**Methode:** statische Sichtung (Cargo-Manifeste, Modul-/Typlisten, Test-Zählung). **Kein Build, kein Testlauf, keine Benchmarks** – das Bauen von Fremdcode wurde in dieser Session nicht freigegeben. Baseline-Build/Test ist offen (siehe unten).

## Crates

| Crate | LOC (src) | `#[test]` | interne Deps | `unsafe` | Rolle |
|---|---:|---:|---|---:|---|
| `sagax-core` | 420 | 8 | – | 0 | `Pos=[i32;3]`, `Cell`, `HexLayout` (Offset↔Axial), `Sink<E>`-Traces, `mix(seed)` (SplitMix64), `sine` |
| `sagax-nav` | 5 261 | 77 (+3 Testdateien) | core | 0 | A*/Dijkstra (`search`), D* Lite, Flow Fields, Grid/Hex, Smoothing, Formation, `Navigator` mit `Budget` |
| `sagax-perception` | 6 860 | 97 | core | 0 | Sichtkegel, Sichtlinien, Hören, Attention, Contagion, Awareness, `SenseMap` |
| `sagax-decide` | 24 399 | 346 (+1 Testdatei) | core | 0 | Utility/Decider, `memory` (Knowledge, Alertness, GoalGuess, Route), `mission` (Planner, Replan, Detour), `group` (Crew, Squad, Orders, Tactic, Skill) |
| `sagax-debug` | 146 | 4 | core | 0 | Trace-Hilfen |
| `sagax` (Fassade) | 1 018 | 67 (13 Szenario-Testdateien) | core, nav, decide, perception, debug | 0 | Prelude + Verbindungen Perception↔Nav↔Memory |
| `sagax-abi` | 4 473 | 30 | core, nav, decide, perception | 0 | gemeinsame Binding-Schicht |
| `sagax-ffi` | 23 784 | 144 | abi, core, nav, decide, perception, sagax | 1 689 Treffer | C-ABI |
| `sagax-wasm` | 4 837 | 0 | abi, core, nav, decide | 485 Treffer | JS/WASM |

`unsafe`-Treffer sind `grep`-Zählungen (inkl. `unsafe extern`/Doku), keine Audit-Zahl. Workspace-Lint: `unsafe_code = "forbid"`, in ffi/wasm überschrieben.

## Weltzugriff (wichtig für den Neuschnitt)

Sagax kopiert keine Welt, sondern fragt sie über Traits ab – das passt direkt zu Plan 6.2 Punkt 3:

- Navigation: `trait CellMap { fn step(&mut self, from: Cell, dx, dy) -> Option<i32>; … }` (`crates/sagax-nav/src/grid.rs:18`), Graph-Abstraktion `trait Graph` / `trait Goal<N>` (`crates/sagax-nav/src/search.rs:18,28`).
- Wahrnehmung: `trait SenseMap` mit Default-Implementierungen pro Methode (`crates/sagax-perception/src/map.rs:22`); `Lit<H>` legt Helligkeit darüber (`map.rs:102`) – Anknüpfungspunkt für das Profil `perception.world.light`.
- Traces: generisch über `Sink<E>` (`NoopSink`, `CallbackSink`, `MapSink`), im Hot Path abschaltbar ohne dynamische Dispatch-Pflicht.

## Beobachtungen für den Zielschnitt

1. **`sagax-core` ist bereits klein und abhängigkeitsfrei.** `Cell`/`HexLayout`/`Pos` sind *Navigations-Raum-Typen* und gehören nicht in `knorke-types` (ADR 0001 §5), sondern in `knorke-navigation` (oder eine kleine `knorke-grid`, falls Perception sie ohne Navigation braucht – Perception nutzt `Cell`).
2. **`mix(seed)` ist identisch zu `knorke_types::Seed(s).derive(0)`** (gleicher SplitMix64-Finalizer, `crates/sagax-core/src/turn.rs:70`). Migration ohne Änderung deterministischer Ergebnisse möglich.
3. **`sagax-decide` ist mit 24 kLOC der Schwerpunkt** und bündelt vier Verantwortungen (Entscheidung, Gedächtnis, Missionen, Gruppen). Kandidat für die Aufteilung innerhalb von `knorke-ai` in Module statt Crates; eigene Crates nur, wenn ein Verbraucher Teile ohne den Rest braucht.
4. **Die Fassade `sagax` verbindet Perception, Nav und Memory** – genau das ist Integrationslogik. Ziel: in `knorke-integration` (Profil `character.navigation.motion` u. a.) oder als schmale Adapter-Module, nicht in den Fachcrates.
5. **`sagax-ffi` (24 kLOC) ist so groß wie `decide`.** Plan 6.2 §6: Bindings neu nach Bedarf. Vor Übernahme klären, welche externen Verbraucher die C-ABI tatsächlich nutzen.
6. Keine Abhängigkeit auf Grafik/SDL/Runtime in den Fachcrates – die Headless-Regel aus ADR 0001 ist heute schon erfüllt.

## Vorschlag Zielschnitt (vorläufig, vor Spike B)

| Ziel | Quelle | Bemerkung |
|---|---|---|
| `knorke-navigation` | `sagax-nav` + Raumteile aus `sagax-core` | `CellMap`/`Graph` bleiben Query-Traits |
| `knorke-perception` | `sagax-perception` | hängt an `knorke-navigation` nur wegen `Cell` – klären, ob Grid-Typen eigene Crate brauchen |
| `knorke-ai` | `sagax-decide` | interne Module `decide`, `memory`, `mission`, `group` |
| `knorke-integration` (Teil) | `sagax`-Fassade | Verbindungscode Perception→Memory→Nav |
| `knorke-ffi` / `knorke-wasm` | `sagax-abi/ffi/wasm` | erst bei konkretem Verbraucher |
| `knorke-types` | `mix` → `Seed::derive`, `Sink` → offen | `Sink` evtl. Kandidat für gemeinsamen Diagnose-Vertrag (ADR 0004) |

## Offen

- [ ] Baseline: `cargo build --workspace`, `cargo test --workspace`, Benchmark-Liste und -Werte (Windows) – braucht Freigabe zum Bauen des Referenz-Repos.
- [ ] Externe Verbraucher der C-ABI/WASM ermitteln.
- [ ] Abhängigkeit Perception → Nav-Typen genau prüfen (`cell_of`, `hex_heading` in `perception/src/world.rs`).
