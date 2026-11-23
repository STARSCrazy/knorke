# ADR 0008 – Events, Commands und Parallelität ohne Broker

**Status:** angenommen
**Datum:** 2026-11-23

## Kontext

Module müssen sich gegenseitig etwas mitteilen („NPC angekommen“), die Welt verändern wollen, die ihnen nicht gehört („Projektil spawnen“), und die CPU-Kerne nutzen. Die klassische Antwort – zentraler Event-Bus/Message-Broker, Callbacks, jedes Modul mit eigenem Thread-Pool – kostet Determinismus, Nachvollziehbarkeit und Leistung: unklare Reihenfolge, Reentranz, dynamischer Dispatch und Allokation pro Nachricht, konkurrierende Pools.

## Entscheidung

### Kommunikation

| Bedarf | Mittel | Typ |
|---|---|---|
| A erzeugt Daten, B verarbeitet sie | Datenfluss in fester Reihenfolge über Puffer | `Column`/`ColumnMut` (ADR 0006) |
| „Etwas ist passiert“, beliebig viele Leser | typisierte Event-Queue, eigener Cursor pro Leser | `EventQueue<E>`, `EventCursor`, `Events` |
| „Hat sich etwas geändert?“ | Change Detection | `Revision`, `BitSet` (pro Entity) |
| Modul will fremden Zustand ändern | Command Buffer, angewendet am Sync-Punkt | `CommandBuffer<C>` |
| Seltene Ereignisse (Asset geladen) | ebenfalls Queue, Verarbeitung im Schedule | `EventQueue<E>` |
| Außenwelt (Agent/MCP/Editor/Netz) | Broker/RPC am Rand, nicht im Kern | Development Plane |

- **Kein globaler Event-Bus, keine Subscription, keine Callbacks im Hot Path.** Wann ein Leser läuft, bestimmt der Schedule (ADR 0009), also ist die Reihenfolge explizit und deterministisch.
- `EventQueue`: Struct-of-Arrays, Leser bekommen `&[E]`. `keep_ticks(now, n)` mustert alte Events aus (Bevy-artiges Double-Buffering = `n = 2`); zu langsame Leser bekommen `Events::Resync` statt still Events zu verlieren. Kapazität bleibt erhalten → keine Allokation im Dauerbetrieb.
- `CommandBuffer`: Reihenfolge der Aufzeichnung = Reihenfolge der Anwendung; `pending()` erlaubt Validierung vor dem Anwenden (Agenten-Transaktionen, Plan 8.2).

### Parallelität

- **Module starten keine Threads.** Wer parallele Arbeit hat, nimmt `&impl Parallel` (Fork-Join): `for_each_chunk_mut`, `for_each_chunk`, `for_each_chunk_with` (eigener Zustand pro Chunk), `for_each_range`/`for_each_range_with` (Index-Bereiche für Eingaben, die keine Slices sind – `impl Column<T>`, `project`), `join`.
- **Der Host wählt den Executor** (`knorke-exec`): `Rayon` (Work-Stealing, Standard), `ScopedThreads` (ohne Abhängigkeit, Threads pro Aufruf – nur für grobe Arbeit), `Serial` (Tests, Referenz).
- **Determinismus:** Chunk-Grenzen hängen nur von Datenlänge und `chunk_len` ab, nie von der Thread-Anzahl. Ergebnisse pro Chunk (Commands, Events, Teilsummen) werden in Chunk-Reihenfolge zusammengeführt. Vertragstest: Serial, 1/7 Scoped-Threads, Rayon global und mit 3 Threads liefern **bitgleiche** Ergebnisse.
- `BitSet::words_mut` erlaubt parallel disjunkte Wort-Bereiche zu schreiben (Chunks von 64·k Indizes) – ohne Locks oder Atomics.
- Keine Hash-Maps in Simulationscode (ADR 0003): `HandleMap`, `BitSet`, `BTreeMap`/`BTreeSet`, sortierte `Vec`. `StableHasher` (FxHash) ist die Grundlage von `Checksum`; `StableState` bleibt für Host-Werkzeuge (Reihenfolge nur von Lauf zu Lauf desselben Builds gleich, nicht plattformübergreifend; nicht DoS-fest).

## Messwerte

`cargo bench -p knorke-contract-tests`, Release, 2026-11-23, Windows 11, 16 logische Kerne. Bestwert aus mehreren Durchläufen je Messung; Spannen stammen aus zwei Bench-Läufen am selben Tag (vor und nach dem Review). Nur als Größenordnung:

| Messung | Ergebnis |
|---|---|
| `project()` über Host-Structs vs. direkter Feldzugriff | 1,50 vs. 1,42 ns/Element (im Messrauschen) |
| `Column` über `&slice[..]` vs. direkte Schleife | 1,25 vs. 1,29 ns/Element (kein Overhead; vor und nach dem Weiterreichen von `iter` durch `&C` gleich) |
| `HandleMap::get` / `BTreeMap::get` / HashMap+`StableState` / HashMap std | 2,8–5,0 / 236–430 / 21–37 / 68–78 ns pro Lookup (zufällige Reihenfolge, 1 Mio. Einträge; mehrere Läufe, Rechner zeitweise unter Last) |
| `BTreeMap::get` bei 1000 Einträgen | 31–39 ns |
| `Seed::derive` (zwei SplitMix64-Runden) | 1,4–1,8 ns |
| `EventQueue` push+read+retire | 1,9–2,1 ns/Event |
| `CommandBuffer` push+apply | 1,2–1,7 ns/Command |
| `Probe::scope` mit `NoProbe` / mit `Recorder` | 0 / 65–69 ns pro Scope |
| Steering-Frame 1 Mio. Agenten: Serial / Rayon / ScopedThreads | 17,7–19,0 / 2,4 / 5,3–5,5 ms (7,5–7,7× / 3,3–3,5×) |
| Fork-Join-Overhead, 64 leere Chunks: Rayon / ScopedThreads | 10–11 µs / 790–950 µs |

Folgerungen:
- Instrumentierung pro System oder Chunk, nicht pro Entity (68 ns pro Scope durch zwei Uhrabfragen).
- `ScopedThreads` nur für Arbeit im Millisekundenbereich; im Frame-Loop `Rayon`.
- Chunk-Länge so wählen, dass ein Chunk ≥ ~10 µs Arbeit enthält (Rayon-Overhead pro Aufruf ~10 µs).

## Nachweis

- `tests/contracts/tests/parallel.rs`: bitgleiche Ergebnisse über alle Executoren, Randfälle, `join`, geordnete Maps unabhängig von der Einfügereihenfolge, paralleles `BitSet`.
- `tests/contracts/tests/no_alloc_parallel.rs`: 100 000 Agenten mit Recorder, Commands und Events. **Serial: strikt 0 Allokationen.** Rayon: unser Code alloziert nichts; Rayons Scheduler legt für Aufträge von außerhalb des Pools pro 63 Aufträgen einen Block in crossbeams Injector an (gemessen: 15 Allokationen pro 1000 Aufrufe). Der Test erlaubt genau diese Obergrenze.
- Empfehlung daraus: Den ganzen Frame mit `Rayon::install` im Pool laufen lassen – dann nutzen alle Parallel-Aufrufe die lokale Queue des Threads, und es gibt nur einen externen Auftrag pro Frame.

## Bewusst noch nicht

- **Handle-Reservierung aus parallelen Chunks** (Spawn-Command, der sofort ein Handle braucht): erfordert einen atomaren Reservierungszähler in `Slots`. Kommt, wenn ein Modul es braucht; bis dahin referenzieren Commands neue Objekte über chunk-lokale Indizes.
- Task-Graph über mehrere Systeme (Systeme parallel statt nur Daten parallel): Teil des Schedulers, ADR 0009.
