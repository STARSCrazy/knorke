# Review-Auftrag: Knorke-Grundlagen (Phase 0)

Du reviewst und verbesserst die Grundlagen der Knorke Engine im Repo `C:\Dev\Knorke` (Rust-Workspace, noch kein Commit). Ziel: Fehler, Soundness-Probleme, Determinismus-Lecks, falsche Leistungsbehauptungen und Inkonsistenzen zwischen Code und Dokumentation finden und **behoben mit Tests** zurückgeben. Arbeite eigenständig; frag nur bei echten Produktentscheidungen.

## Zuerst lesen

1. `CLAUDE.md` – Projektregeln. Bindend, auch für dich.
2. `docs/adr/README.md` und alle ADRs 0000–0011 – die Entscheidungen, gegen die du prüfst.
3. `docs/plan.md` Abschnitte 16 (Stand) und 17 (Backlog). Abschnitte 1–15 sind die ursprüngliche Zielarchitektur.
4. Code: `crates/knorke-{types,math,observe,exec,replay}`, `tests/contracts` (Vertragstests, Benchmarks, Fingerprint), `xtask` (Abhängigkeitsregeln), `clippy.toml`, `.github/workflows/ci.yml`.

`reference/` (Sagax, Chiaro) ist unversionierter Lesestoff – nicht ändern, nicht bauen.

## Was das System behauptet – prüfe jede Behauptung selbst

- Datenaustausch ohne Kopien (`Column`, `project`, `bytemuck`-Casts) und **0 Allokationen** in Steady-State-Pfaden (Serial strikt; Rayon nur Injector-Blöcke, ADR 0008).
- Parallele Ergebnisse **bitgleich** unabhängig von Executor und Thread-Anzahl.
- Snapshots stellen den Zustand **exakt** wieder her (inkl. `Slots`-Freiliste); Replays verifizieren, `seek` = Durchlauf von vorne, `minimize` ist 1-minimal.
- **Plattformübergreifend bitgleiche** Simulation über `knorke_math::det` + `clippy.toml`; Fingerprint `0x8E13_81FE_2691_7217` in `tests/contracts/tests/determinism.rs` (lokal nur auf Windows x86_64 belegt).
- Abgeschaltete Zusatzfeatures (`NoProbe`, `NoopSink`, `NoRecord`, `Serial`, `Option<Recorder>`, leere `Invariants`) kosten nichts.
- Abhängigkeitsgrenzen der Basis-Crates sind geschlossen und in `cargo xtask deps` erzwungen.

## Prüfschwerpunkte

1. **Soundness:** jedes `unsafe` (`knorke-math/src/pod.rs`, Zähl-Allokatoren in `tests/contracts/tests/no_alloc_*.rs`). Stimmen die `SAFETY`-Begründungen, die Layout-Asserts, die `Pod`-Bedingungen (auch für die generischen `At<S, V>`/`Transform2`)?
2. **Korrektheit an Rändern:** leere Eingaben, `u32`-Überläufe (Slot-Indizes, Längen in Snapshots), Generationsüberlauf, `EventQueue`-Cursor nach `retire`/`clear`, `Recorder`-Fenster beim Kürzen, `ReplayLog::read` mit manipulierten Daten (Bereiche, Keyframe-Reihenfolge, riesige Längen), `det::powi(_, i32::MIN)`, Geometrie bei entarteten Formen (Radius 0, Segment der Länge 0, Strahl mit Richtung 0).
3. **Determinismus-Lecks:** Was entgeht `clippy.toml`? (Beispiele zum Prüfen: `Instant::elapsed`, `f32::mul_add` ohne Hardware-FMA, `HashMap::with_capacity`, `Default` für `HashMap` mit `RandomState`, Iterationsreihenfolge, NaN-Payloads auf WASM, `as`-Casts von Float zu Int.) Ist der Fingerprint-Szenario-Code selbst plattformsicher?
4. **API-Design:** Sind die Verträge für die kommenden Fachmodule (Navigation, KI, Wahrnehmung, Lighting, Physik) tragfähig? Fehlt Wesentliches, ist etwas überflüssig oder zu früh? Ist `knorke-types` noch „klein“ im Sinne von ADR 0001 oder sollte z. B. der Snapshot-Teil eine eigene Crate werden? Begründe mit konkreten Nutzungsfällen, nicht mit Geschmack.
5. **Leistung:** Stimmen die Zahlen in ADR 0008/0011 (`cargo bench -p knorke-contract-tests`, Release)? Gibt es offensichtliche Engstellen (z. B. `Column::iter` über `get`, `EventQueue::retire` per `drain`, Prüfsumme über den ganzen Snapshot, 68 ns pro Profiling-Scope)?
6. **Tests:** Prüfen die Tests wirklich, was sie behaupten, oder bestehen sie zufällig? Gibt es flackernde Tests? Fehlen Property-Tests (z. B. Snapshot-Roundtrip, `Slots`/`HandleMap` gegen ein Referenzmodell)? Neue Abhängigkeiten nur als Dev-Abhängigkeit und nur, wenn klar nützlich.
7. **Konsistenz:** Widersprechen sich Code, ADRs, `CLAUDE.md`, README und Plan? Stimmen Status und Zahlen?
8. **Check-in-Reife:** CI-Workflow (läuft er so auf GitHub Actions? MSRV 1.88 tatsächlich eingehalten?), `.gitignore`/`.gitattributes`, fehlende Lizenzdateien (Cargo deklariert `MIT OR Apache-2.0`, es gibt keine Lizenzdateien – nicht selbst entscheiden, nur melden).

Mir bekannte, aber nicht geprüfte Verdachtsmomente stehen in Punkt 2, 3 und 5. Behandle sie als Hypothesen, nicht als Befunde – bestätige oder widerlege sie mit Tests.

## Regeln für Änderungen

- Jede Korrektur bekommt einen Test, der vorher fehlschlägt.
- Keine neuen Laufzeit-Abhängigkeiten ohne ADR-Änderung; die Grenzen in `xtask/src/deps.rs` nicht aufweichen.
- Öffentliche APIs dürfen sich ändern (noch kein Verbraucher), aber dann ADRs, README und Plan nachziehen.
- Ändert sich der Determinismus-Fingerprint, begründe es ausdrücklich.
- Neue Ideen und vertagte Punkte in `docs/plan.md` Abschnitt 17 eintragen; Erledigtes in Abschnitt 16.
- **Nicht committen, nicht pushen.** Kein Bauen von `reference/`. Nichts systemweit installieren, ohne zu fragen.
- Am Ende alle grün: `cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features` (0 Warnungen), `cargo test --workspace --all-features` (Debug und `--release`), `cargo test --workspace --no-default-features`, `cargo xtask deps`, `cargo xtask standalone`, `cargo check --workspace --all-targets --all-features --target x86_64-unknown-linux-gnu`.

## Ergebnis

1. **Befunde**, schwerste zuerst: Datei:Zeile, Problem, konkretes Fehlerszenario, Status (behoben / offen + Grund). Trenne belegt (Test/Messung) von Einschätzung.
2. **Änderungen** in zwei, drei Sätzen pro Bereich – nicht jede Datei nacherzählen.
3. **Messwerte** vorher/nachher, wo du Leistung angefasst hast (Hardware, Datum).
4. **Offene Fragen**, die eine Produktentscheidung brauchen (z. B. Lizenz, Repo-Ort, Plattformziele inkl. Browser).
5. Ausgabe der Verifikationsbefehle (Anzahl Tests, Warnungen).
