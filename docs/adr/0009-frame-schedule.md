# ADR 0009 – Frame-Ablauf und Scheduler

**Status:** vorgeschlagen – Umsetzung in `knorke-runtime` (Plan Phase 3), sobald es echte Systeme zum Anordnen gibt
**Datum:** 2026-11-23

## Kontext

Ohne ECS-Framework (Plan-Prinzip 9) muss Knorke selbst festlegen, wann welches System läuft, was es liest und schreibt und wo Änderungen sichtbar werden. ADR 0003 (Tick ≠ Frame), 0006 (geliehene Daten) und 0008 (Events, Commands, Parallelität) setzen das voraus.

## Entscheidung (Rahmen)

### Phasen pro Frame

```text
Frame:
  1. Input & Außenwelt        Eingaben, Agenten-Transaktionen, fertige Asset-Loads → Queues
  2. Simulation (0..n Ticks)  Fixed Tick, so oft wie die Echtzeit verlangt:
       a. Systeme lesen Welt + Events, schreiben eigene Ausgaben, Commands, Events
       b. Sync-Punkt: Commands in fester Reihenfolge anwenden, Revisionen erhöhen,
          alte Events ausmustern (`keep_ticks`)
  3. Render vorbereiten        Interpolation zwischen den letzten zwei Ticks, Sichtbarkeit,
                               GPU-Uploads aufbereiten (Pod-Slices)
  4. Render                    Pässe auf dem gemeinsamen Device (ADR 0002), Submit/Present durch den Host
```

- **Simulation und Darstellung sind entkoppelt:** Rendering liest einen konsistenten Stand (letzter Tick + Interpolationsfaktor) und blockiert die Simulation nicht. Ziel ist Pipelining: Frame N rendern, während Tick N+1 simuliert (später, wenn gemessen nötig).
- **Sync-Punkte sind die einzigen Stellen, an denen sich die Welt strukturell ändert** (Spawn/Despawn, Agenten-Änderungen). Dazwischen sind alle Daten stabil → Systeme können sie ohne Locks leihen.

### Systeme

- Ein System deklariert, was es **liest** und **schreibt** (Ressourcen/Spalten, Event-Typen, Command-Typen). Daraus leitet der Scheduler ab, welche Systeme **parallel** laufen dürfen (keine Schreib-Überschneidung) – Task-Graph statt fester Thread-Zuordnung.
- Reihenfolge-Abhängigkeiten werden explizit angegeben („nach `navigation`“), nicht aus der Registrierungsreihenfolge erraten.
- Innerhalb eines Systems parallelisiert das Modul selbst über `Parallel` (ADR 0008); über Systeme hinweg der Scheduler mit demselben Executor (ein Pool für alles).
- Der Schedule ist Daten: inspizierbar (`Inspect`), als Graph exportierbar, mit Profil-Scopes pro System (`Probe`). Agenten können ihn abfragen (Plan 8.1 `inspect`).

## Warum jetzt noch kein Code

Ein Scheduler ohne echte Systeme bekommt die falsche Form (ADR 0001 §8). Die Bausteine, auf denen er aufsetzt – `Parallel`, `CommandBuffer`, `EventQueue`, `Revision`, `Probe` – sind fertig und getestet. Der Scheduler entsteht mit den ersten zwei, drei realen Systemen (Navigation, Wahrnehmung, Lighting-Vorbereitung) im Vertical Slice.

## Offene Fragen

- Deklaration von Lese-/Schreibzugriffen: statisch über Typen (wie Bevy) oder als Daten (agentenfreundlicher, aber Laufzeitprüfung)?
- Snapshot/Rollback des Simulationszustands pro Tick (für Replays, Rollback-Netcode, „Time-Travel“-Debugging): Pflicht für deterministische Systeme oder optional?
