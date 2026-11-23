# ADR 0010 – Snapshots, Replay und automatische Fehlererkennung

**Status:** angenommen
**Datum:** 2026-11-23

## Kontext

Replays, Rollback-Netcode, Time-Travel-Debugging, automatisches Playtesting und Agenten, die Fehler selbst reproduzieren und eingrenzen, brauchen dieselbe Grundlage: kopierbaren Simulationszustand, eine Prüfsumme pro Tick und eine harte Grenze für alles Nicht-Deterministische. Nachträglich ist das kaum einzubauen, weil jedes Modul es mittragen muss. Deshalb vor dem Sagax-Umbau.

Recherche (Plan 17.11, Nachtrag E/F): Ein Replay aus Seed, Commands, Keyframes und Prüfsummen dient zugleich als Bug-Report, Regressionstest, Fuzzing-Fund, Flight-Recorder-Dump und Agenten-Eingabe. Orakel nach Kosten: Invarianten → Navigations-/Erreichbarkeitsorakel → Bilddiff gegen Referenz → Bildmodell nur zur Einordnung.

## Entscheidung

### 1. Snapshot-Vertrag (`knorke-types`)

- Jedes Modul mit Simulationszustand implementiert `Snapshot` (`ID`, `VERSION`, `save`, `restore`) und schreibt **einen Abschnitt** in einen `SnapshotBuf`.
- Format: Little-Endian, kein Padding, keine Zeiger; Abschnitt = `id, version, len, payload`. Plattformunabhängig.
- `SnapshotValue` für Grundtypen, `Option`, Tupel, Arrays, `Vec`, `Tick`, `Seed`, `Revision`, `Handle`, `Slots`, `HandleMap`, `BitSet`, `EventQueue`, `EventCursor`, `CommandBuffer` und alle `knorke-math`-Typen. `get_into` verwendet vorhandene Kapazität → **Speichern und Wiederherstellen im Dauerbetrieb ohne Allokation**. Das gilt auch verschachtelt: `Vec<Vec<T>>` behält beim Restore die inneren Puffer (Elemente werden über `get_into` überschrieben, nicht verworfen).
- **NaN wird kanonisch gesichert** (`0x7FC0_0000` bzw. `0x7FF8_0000_0000_0000`): Vorzeichen und Payload eines berechneten NaN unterscheiden sich zwischen x86, ARM und WASM; gleicher Zustand muss gleiche Bytes und Prüfsummen ergeben. Alle anderen Werte bleiben bitgenau (auch `-0.0`, Unendlich, Subnormale).
- Restore prüft Strukturen, deren Verletzung später zu Panics oder falschem Verhalten führen würde: Die Freiliste von `Slots` muss eine einfache Kette durch genau die wiederverwendbaren freien Slots sein (kein Zyklus, kein vergessener Slot); die Ticks einer `EventQueue` müssen aufsteigend sein.
- `Slots` wird **exakt** gesichert, inklusive Freiliste und Generationen: Nach einem Restore vergibt die Storage dieselben Handles wie das Original.
- Fehler sind konkret (`UnexpectedEnd`, `WrongSection`, `UnsupportedVersion`, `TrailingBytes`, `Invalid`) und als `Diagnostic` mit stabilem Code abrufbar. `TrailingBytes` deckt auf, dass `save` und `restore` eines Moduls nicht zusammenpassen.
- `Checksum` (FxHash) über Bytes; `SnapshotBuf::section_checksums()` liefert **eine Prüfsumme pro Modul**.

### 2. Eingabegrenze

Alles Nicht-Deterministische kommt **nur als Command eines Ticks** in die Simulation: Spielereingaben, Agenten-Transaktionen, Netzwerk, LLM-Antworten, fertig geladene Assets, Echtzeit. Wer diese Grenze umgeht, macht Replays, Rollback und Netzwerk kaputt. Commands werden semantisch aufgezeichnet („gehe zu Zelle X“), nicht als rohe Tastendrücke – robuster gegen Content-Änderungen.

### 3. Replay (`knorke-replay`, nur abhängig von `knorke-types`)

- `Simulation`-Trait: `save`, `restore`, `step(tick, commands)`.
- `ReplayLog`: Header (App-/Content-Version, Seed), Startzustand, Commands pro Tick, Prüfsummen pro Tick und Modul, Keyframes. Versioniertes Binärformat (`KNRP`, Version 1), robust gegen abgeschnittene oder falsche Daten: `read` weist Bereiche außerhalb der Daten, nicht lückenlos aufsteigende Ticks und Keyframes außerhalb des aufgezeichneten Bereichs oder in falscher Reihenfolge ab. Mehr als `u32::MAX` Commands oder Prüfsummen in einem Log führen zu einem Panic mit klarer Meldung statt zu umgebrochenen Indizes (der Flight Recorder erreicht die Grenze nie).
- `Recorder` mit `RecordOptions { checksum_every, keyframe_every, window }`. Mit `window` ist er ein **Flight Recorder**: Er behält mindestens N Ticks, verwirft ältere Geschichte an Keyframe-Grenzen und verwendet Keyframe-Puffer wieder → **0 Allokationen im Dauerbetrieb** (Vertragstest).
- `verify` meldet den **ersten abweichenden Tick und die abweichenden Module**.
- `seek` springt über den nächsten Keyframe zu einem beliebigen Tick (Time-Travel). Vertragstest: gleicher Zustand wie ein Durchlauf von vorne.
- `minimize`: Delta-Debugging (ddmin) auf Command-Ebene; Ergebnis ist 1-minimal. Test: 60 Commands → genau die 2 auslösenden; ein zweiter Test prüft 1-Minimalität direkt (jedes einzelne Entfernen lässt den Fehler verschwinden).

### 4. Automatische Fehlererkennung (`knorke-observe`)

- `Invariants<W>`: Prüfregeln über die Welt, pro Regel alle N Ticks, am Sync-Punkt. Verletzungen sind `Diagnostic`s; pro Regel und Prüfung höchstens N Meldungen plus eine Sammelmeldung. Zähler pro Regel.
- `ProgressWatch`: meldet Objekte, deren „Restarbeit“ (Distanz zum Ziel, offene Schritte) sich N Ticks lang nicht verbessert – festhängende NPCs, hängende Quests.
- `image`: `frame_stats` (leere/einfarbige/transparente Frames), `frame_diff` mit Toleranz, `flicker_score` (einzelner Frame weicht von beiden Nachbarn ab), `non_finite` (NaN/Inf in Float-Targets).

### 5. Geometrie-Grundformen (`knorke-math`)

`Aabb2`, `Circle`, `Segment2`, `Capsule2`, `Ray2` mit Enthaltensein, Überlappung, nächstem Punkt, Abstand und Raycasts. Festes Layout, `Pod`, raumtypisierbar (`At<World2, Aabb2>`). Gemeinsame Sprache für Physik, Sichtlinien, Schattenwerfer, Culling und Picking.

## Abhängigkeiten (in `cargo xtask deps` erzwungen)

- `knorke-replay` → nur `knorke-types` (darf in Release-Builds mitlaufen, z. B. als Flight Recorder).
- `knorke-math` → `bytemuck`, `mint` (optional), **neu** `knorke-types` (für `SnapshotValue`).

## Noch offen (Plan 17)

- Plattformübergreifende Mathematik: erledigt mit ADR 0011 (`knorke_math::det`, Clippy-Regeln, Fingerprint-Test).
- Replay-Lebenszyklus: Golden Replays in CI, `replay rebase` nach bewusst akzeptierten Änderungen, Vergleichsstufen (bitgenau / Event-Assertions / Bilddiff).
- Exploration (Go-Explore/Deterministic Simulation Testing) über Snapshot-Fork, parallel über `Parallel`; Navigation als Orakel.
- Agenten-Operationen `replay.verify`, `replay.seek`, `replay.minimize`, `invariant.add` (Phase 3/5).
