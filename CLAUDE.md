# Knorke Engine – Projektregeln

Gilt für Menschen und Agenten (Claude Code, Codex, andere). `AGENTS.md` verweist hierher.

## Quellen der Wahrheit

- **Plan:** `docs/plan.md`. Abschnitt 16 = umgesetzter Stand (eine Zeile je Baustein), Abschnitt 17 = Backlog (Ideen, Konzepte, vertagte Entscheidungen, Recherche).
- **Entscheidungen:** `docs/adr/` ist bindend. Abweichen heißt: zuerst die ADR ändern.
- **Bestand:** `docs/inventory/` (Sagax, Chiaro). `reference/` ist unversionierter Lesestoff – dort nichts ändern.

## Pflege

- **Jede neue Idee, jedes Konzept, jede vertagte Entscheidung** kommt in Plan-Abschnitt 17 mit Status (offen / geplant / vorbereitet / erledigt / verworfen + Grund). Erledigtes zusätzlich in Abschnitt 16 (eine Zeile; lange Berichte mit Messwerten in `docs/stand.md`).
- Recherche-Ergebnisse mit Quelle, Reifegrad und „belegt / nur Sekundärquelle“ in 17.11.
- Neue Grundsatzentscheidung → neue ADR (`docs/adr/NNNN-titel.md`, Status, Datum, Kontext, Entscheidung, Konsequenzen).

## Fertig heißt

Vor jedem Abschluss alle grün, Ergebnis ehrlich berichten (auch Fehlschläge und Messwerte):

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features   # 0 Warnungen
cargo test --workspace --all-features            # zusätzlich einmal mit --release
cargo test --workspace --no-default-features
cargo xtask deps          # Abhängigkeitsregeln
cargo xtask standalone    # jede Bibliothek einzeln baubar
```

Leistungsaussagen nur mit Messung (`cargo bench -p knorke-contract-tests`), Hardware und Datum. Nicht-deterministische Testergebnisse untersuchen, nicht wegwiederholen.

## Architektur-Regeln

### Modularität (ADR 0001)
- Jede Fachcrate ist **standalone nutzbar**. Verbotene Abhängigkeiten stehen als Code in `xtask/src/deps.rs` – Änderung nur mit ADR-Änderung.
- Basis-Crates haben **geschlossene Abhängigkeitslisten**: `knorke-types` keine; `knorke-math` nur `bytemuck`, `mint`, `knorke-types`, `libm`; `knorke-observe` und `knorke-replay` nur `knorke-types`; `knorke-exec` nur `rayon`.
- `knorke-types` bleibt klein: nur Typen, die mindestens zwei unabhängige Module zum Komponieren brauchen. Keine Koordinaten-, Mathe- oder GPU-Typen.
- **Keine leeren Platzhalter-Crates.** Neue Crate erst mit Code und Verbraucher; dann in `Cargo.toml` (`members`, `workspace.dependencies`), `[lints] workspace = true`, Regeln in `xtask` ergänzen.

### Datenaustausch ohne Kopien (ADR 0006)
- Massendaten als `impl Column<T>` / `impl ColumnMut<T>` **per Wert** annehmen, nie `Vec<T>` verlangen. Felder in Host-Structs über `project`.
- Werttypen und Geometrie aus `knorke-math` (festes Layout, `Pod`); Koordinatenräume als Typen (`At<World2, Vec2>`), Umrechnung nur über `Transform2<A, B>`.
- Ergebnisse in Puffer schreiben, die der Host bereitstellt, oder in eigenen, wiederverwendeten Zustand.
- **Steady-State-Pfade allozieren nicht** – belegt durch Vertragstests in `tests/contracts`.

### Kommunikation (ADR 0008)
- Datenfluss in fester Reihenfolge, `EventQueue`, `CommandBuffer`, `Revision`/`BitSet`. **Kein Event-Bus, kein Broker, keine Callbacks im Hot Path.**
- Änderungen an fremdem Zustand nur als Command, angewendet am Sync-Punkt.

### Parallelität (ADR 0008)
- Module parallelisieren nur über `&impl Parallel` mit **fester Chunk-Länge** (`for_each_chunk*` für Slices, `for_each_range*` für `impl Column<T>`); sie starten keine Threads und hängen nicht an `rayon`/`knorke-exec`.
- Ergebnisse pro Chunk **in Chunk-Reihenfolge** zusammenführen → bitgleich auf jeder Thread-Anzahl.
- Ein Chunk ≥ ~10 µs Arbeit. Im Frame-Loop `Rayon` mit `Rayon::install` um den ganzen Frame.

### Determinismus, Snapshots, Replay (ADR 0003, 0010)
- Keine eigenen Uhren, kein globaler Zufall in Modulen. Zeit als `Tick`, Zufall über `Seed::derive`. **Keine `HashMap`/`HashSet` in Simulationscode** (Clippy verbietet die Typen; Iterationsreihenfolge ist nicht plattformübergreifend festgelegt): `HandleMap` pro Entity, `BitSet`, `BTreeMap`/`BTreeSet`, sortierte `Vec`, dichte Raster. `BTreeMap` ist bei großen Tabellen ~10× langsamer als eine Hash-Map (ADR 0003) – in Hot Paths eigene Struktur. Host-Werkzeuge dürfen lokal mit `#[allow(clippy::disallowed_types)]` + Begründung `StableState` nutzen.
- **Simulationsmathematik plattformunabhängig:** `knorke_math::det::{sin, cos, atan2, exp, pow, min, max, …}` statt `f32::sin` usw.; `sqrt`, Grundrechenarten, `clamp`, `mul_add` sind erlaubt (`mul_add` ohne FMA-Befehl ~2× langsamer). **`f32::min`/`max` sind verboten** (Ergebnis für `-0.0`/`+0.0` unterscheidet sich schon zwischen Debug und Release). `clippy.toml` erzwingt das; Darstellungs-Crates dürfen mit eigener `clippy.toml` lockern, Host-Werkzeuge lokal mit `#[allow]` + Begründung. `det::pow` ist ~4× langsamer – in Hot Paths Tabellen oder `powi` (ADR 0011).
- Der Determinismus-Fingerprint (`tests/contracts/tests/determinism.rs`) muss auf allen Plattformen gleich bleiben. Bewusste Änderung = alle Replays ungültig → in der Commit-Message nennen.
- **Eingabegrenze:** Alles Nicht-Deterministische (Spieler, Netzwerk, Agenten, LLM-Antworten, fertige Asset-Loads, Echtzeit) kommt **nur als Command eines Ticks** in die Simulation. Commands semantisch, nicht als rohe Tastendrücke.
- **Jedes Modul mit Simulationszustand implementiert `Snapshot`** (eigener Abschnitt, Versionsnummer, Restore ohne Allokation über `get_into`). Zustand bleibt kopierbar: Handles statt Zeiger. `EventCursor` eines Lesers gehört zu seinem Zustand und wird mitgesichert. `restore` prüft, was später zu Panics führen würde, und liefert `SnapshotError::Invalid`.
- Neue Module bekommen einen Replay-Test: aufnehmen → `verify` → `seek`.

### Debugging und Fehlererkennung (ADR 0007, 0010)
- Instrumentierung als `impl Probe` (pro System/Chunk, nicht pro Entity) und `impl Sink<E>` für modul-eigene Traces; Zustand über `Inspect`.
- Fehler und Befunde als `knorke_types::Diagnostic` mit **stabilem Code** (`knorke.<modul>.<…>`). Codes umbenennen = Breaking Change.
- Module bringen ihre **Invarianten** mit (`Invariants`), festhängende Objekte über `ProgressWatch`.
- Orakel nach Kosten: Invarianten → Navigations-/Erreichbarkeitsorakel → Bilddiff gegen Referenz → Bildmodell nur zur Einordnung.

### GPU (ADR 0002)
- Host besitzt Device, Swapchain und Frame-Takt. Module zeichnen in geliehene Command Buffer, **submitten und präsentieren nie selbst**, lesen nicht ungefragt von der GPU zurück.
- Licht linear, Umrechnung nach sRGB genau einmal am Ende.

### Agenten-Schnittstelle (Plan 8, 17.11)
- **Eine Schema-Quelle**: Rust-Typen erzeugen CLI, Skript-API, MCP-Adapter und Mod-Schnittstellen. CLI zuerst, MCP als dünner Adapter.
- Agenten-Änderungen sind Command-Batches: validierbar (`CommandBuffer::pending`), replaybar, rückgängig zu machen.

### `unsafe` (ADR 0005)
- Workspace-Default `forbid`. Nur an FFI-/GPU-Grenzen, in `knorke-math/src/pod.rs` und in Test-Allokatoren; jeweils `// SAFETY:` und eigene Lint-Tabelle mit `deny` + lokalem `allow`.

### Zusatzfeatures abschaltbar, aus = kostenlos (ADR 0011)
- Jedes Zusatzfeature (Instrumentierung, Traces, Aufzeichnung, Prüfregeln, Prüfsummen, Debug-Prüfungen) hat einen **Aus-Zustand ohne Kosten**:
  - Compile-Zeit: generischer Parameter mit nullgroßem No-op-Typ (`NoProbe`, `NoopSink`, `NoRecord`, `Serial`);
  - Laufzeit: `Option<…>` oder leere Sammlung – höchstens ein vorhersagbarer Sprung pro Tick/System.
- Kein Zusatzfeature darf Fachmodule zu `std`, Allokation oder zusätzlichen Abhängigkeiten zwingen (`knorke-observe` mit `default-features = false`).
- Nachweis per Messung „aus vs. nicht vorhanden“ in `cargo bench -p knorke-contract-tests` und nullgroße Typen im Layout-Vertragstest.

## Konventionen

- Dokumentation, ADRs, Plan: **Deutsch**. Code, Bezeichner, Rustdoc, Commit-Messages: **Englisch**.
- Lizenz: `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`). Übernommener Code aus Sagax (gleiche Lizenz) und Chiaro (zlib): Herkunft (Repo + Revision + Pfad) im Kommentar; bei Chiaro-Code den zlib-Hinweis erhalten.
- Sagax und Chiaro werden erst nach den Grundlagen umgebaut; Chiaro wird in Rust neu gebaut (`knorke-lighting`).
- Committen nur auf Anweisung, Dateien einzeln stagen.
