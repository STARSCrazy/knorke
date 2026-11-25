# ADR 0009 – Frame-Ablauf und Scheduler

**Status:** angenommen (2026-11-25) – Stufe 1 umgesetzt in `knorke-runtime`; der Task-Graph-Scheduler bleibt offen (siehe „Umsetzung“)
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

## Umsetzung

### Stufe 1 (2026-11-25): `knorke-runtime`

Mit den ersten vier echten Systemen (Wachmann-Szene: Laufen, Sehen, Entscheiden, Laufen) ist der kleinste Teil entstanden:

- **`FixedStep`** rechnet die vom Host gemessene Zeit in ganze Ticks um und liefert den Interpolationsfaktor. Keine eigene Uhr. Fällt der Host zurück, wird die Zahl der Ticks pro Frame gedeckelt und der Rest verworfen (das Spiel läuft langsamer, statt Ticks aufzustauen).
- **`Runtime<S, D>`** ist der Frame-Ablauf um eine `Simulation` (ADR 0010): Commands des Hosts gehen in den nächsten Tick, Ticks laufen über einen `TickDriver` (Aufnahme an, aus oder zur Laufzeit schaltbar, ADR 0011).
- **`System<W>` + `run_systems`**: der Schedule eines Ticks ist eine Tabelle aus Name (`Scope`) und Funktion, ausgeführt in Tabellenreihenfolge, jedes System in seinem `Probe`-Scope. Die Tabelle ist Daten und damit inspizierbar.
- **Sync-Punkt** ist `Simulation::step`: dort wendet das Spiel die Commands an und mustert alte Events aus, bevor die Systeme laufen.
- **Abhängigkeiten** von `knorke-runtime` sind eine geschlossene Liste: `knorke-types`, `knorke-observe`, `knorke-replay` (`xtask/src/deps.rs`, ADR 0001). Die Laufzeit ist generisch über die Simulation und kennt kein Fachmodul.

Belegt durch: `crates/knorke-runtime/tests/guard_scene.rs` (beide Ausgänge der Szene, Tabellenreihenfolge, Frames aus Host-Zeit, aufnehmen → `verify` → `seek`) und den Vertragstest `no_alloc_runtime` (0 Allokationen im Dauerbetrieb, Flight Recorder an).

### Noch nicht umgesetzt

- **Task-Graph-Scheduler** (Systeme nach Lese-/Schreibzugriff parallel) und explizite Reihenfolge-Abhängigkeiten („nach `navigation`“). Vier Systeme mit je wenigen Mikrosekunden Arbeit geben dafür keine Form her; die Reihenfolge ist die Tabelle. Auslöser: ein Tick, dessen Systeme gemessen parallel lohnen.
- **Phasen 3 und 4** (Render vorbereiten, Render) und Pipelining: mit dem ersten Bild aus der Laufzeit.

## Entschiedene Fragen

- **Deklaration von Lese-/Schreibzugriffen:** vertagt, bis der Task-Graph-Scheduler gebaut wird. Bis dahin gibt es nichts, was die Deklaration auswerten würde.
- **Snapshot des Simulationszustands:** Pflicht für jedes Modul mit Simulationszustand (ADR 0010, `CLAUDE.md`). Die Laufzeit verlangt es über den `Simulation`-Vertrag (`save`/`restore`).
