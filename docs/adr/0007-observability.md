# ADR 0007 – Gemeinsames Debugging und Analyse

**Status:** angenommen
**Datum:** 2026-11-23

## Kontext

Plan-Prinzip 6: Zustand, Abläufe und Fehler aller Module gezielt inspizierbar, Traces und Profile von Anfang an. Sagax hat ein eigenes Trace-Muster (`Sink<E>`), Chiaro eigene Statistiken/Debug-Checks. Ohne gemeinsamen Rahmen sieht man jedes Modul isoliert und nie auf einer Zeitachse.

## Entscheidung

`knorke-observe` definiert drei Kanäle als kleine Traits. Module **nehmen sie entgegen**, der Host **implementiert** sie. Mit den No-op-Implementierungen fällt die Instrumentierung beim Kompilieren weg.

| Kanal | Trait | Frage | Kosten aus |
|---|---|---|---|
| Trace | `Sink<E>` | *Warum* hat das Modul das getan? Modul-eigene Ereignistypen | `NoopSink`, `enabled() == false` → Ereignis wird gar nicht gebaut |
| Profil | `Probe` (+ `scope!`, `counter!`) | *Wo* geht Zeit hin, wie viel Arbeit? | `NoProbe` |
| Zustand | `Inspect` / `Visitor` | *Was* ist der aktuelle Zustand? | nur bei Aufruf (kalter Pfad, `dyn`) |

Fehler/Befunde sind `knorke_types::Diagnostic` über einen `Sink<Diagnostic>`.

### Regeln

1. **Module lesen keine Uhr und schreiben in keinen globalen Logger.** Zeitstempel setzt der Host (`Probe`-Implementierung). Konsistent mit ADR 0003.
2. **`Sink<E>` hat dieselbe Form wie `sagax_core::Sink`** (`enabled`, `emit`). Sagax-Traces lassen sich über `MapSink` ohne Umbau anschließen.
3. Modul-eigene Trace-Typen bleiben modul-eigen. Übersetzung in gemeinsame Analyseformate passiert an der Grenze (`MapSink`), nicht im Modul.
4. Scope-/Counter-Beschreibungen sind `&'static` (Makros), Modulname im Namensschema von ADR 0000 (`knorke.navigation`).
5. `Inspect` statt serde: keine schwere Abhängigkeit in Fachmodulen; der Host wählt das Format.
6. Fachmodule hängen mit `default-features = false` an `knorke-observe` (nur `no_std`-Verträge). Recorder und Exporter (`std`-Feature) nutzt nur der Host bzw. Werkzeuge.

### Host-Seite (`std`-Feature)

- `Recorder`: implementiert `Probe` und `Sink<Diagnostic>`, Ringpuffer mit fester Kapazität, nach dem Anlegen allokationsfrei (bis 64 Schachtelungsebenen). Markiert Spans/Diagnosen mit dem aktuellen `Tick`. Ein Recorder pro Thread, gemeinsame Epoche über `with_epoch`.
- Export:
  - `write_chrome_trace` → Chrome-Trace-Event-JSON. **Alle Module und Threads auf einer Zeitachse** in Perfetto (`ui.perfetto.dev`) oder `chrome://tracing`; Counter als Spuren, Diagnosen als Markierungen.
  - `stats()` / `summary()` → count, total, p50, p95, max pro Scope (Plan §11: p50/p95).
  - `write_diagnostics_jsonl` → eine Diagnose pro Zeile, für Agenten und CI.
  - `json::inspect_to_json` → Zustand als kompaktes, deterministisches JSON (Feldreihenfolge = Besuchsreihenfolge, NaN → `null`).

## Bewusst noch nicht

- GPU-Timings: SDL3-GPU bietet nach meinem Stand keine Timestamp-Queries, Chiaro misst ebenfalls nicht (siehe Inventur). Klären mit `knorke-gpu`.
- Replay/Snapshot-Format – braucht `knorke-scene` (Phase 3).
- Thread-übergreifender Sammel-Recorder mit Locks – erst bei Bedarf; heute ein Recorder pro Thread.
