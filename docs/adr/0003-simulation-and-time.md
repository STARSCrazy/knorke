# ADR 0003 – Simulationszeit, Determinismus und Seeds

**Status:** angenommen
**Datum:** 2026-11-23

## Entscheidung

1. **Fixed Tick** für Simulation (KI, Navigation, Gameplay). Typ: `knorke_types::Tick` (monoton, u64). Die Tickrate gehört dem Host/der Runtime, nicht dem Modul.
2. **Keine eigenen Uhren in Modulen.** Kein `Instant::now()`, kein `SystemTime` in KI, Navigation, Perception, Animation oder Beleuchtungslogik. Zeit kommt als Argument (`Tick`, Delta).
3. **Kein globaler Zufall.** Deterministische Systeme nehmen `knorke_types::Seed` und leiten Unterströme über `Seed::derive(stream)` ab: zwei Runden des SplitMix64-Mischers, `mix(mix(seed) ^ stream)`. Der Seed wird gemischt, bevor die Stream-Nummer dazukommt; bei der früheren einrundigen Form `mix(seed ^ stream·φ)` hoben sich Seed und Stream auf (`Seed(φ).derive(0) == Seed(0).derive(1)`). Ein Test fixiert Referenzwerte. Der Mischer selbst ist `sagax_core::mix`; `Seed(s).derive(0)` ist aber **nicht mehr** gleich `sagax_core::mix(s)`. Entscheidung 2026-11-23: In Phase 0 ist Kompatibilität zu Sagax-Ergebnissen nicht nötig. **Mit dem Sagax-Umbau wird die Ableitung eingefroren**; Sagax übernimmt dann `Seed::derive`, danach ist jede Änderung ein Bruch aller Replays.
4. **Determinismus wird pro Pfad deklariert**, nicht pauschal: CPU-Simulation mit gleichem Seed, gleicher Eingabe und gleicher Version → bitgleiches Ergebnis (Replay-Test). GPU-Rendering ist ausgenommen (Toleranzvergleich, Plan 8.4).
5. **Frame ≠ Tick.** Rendering interpoliert zwischen Ticks; Änderungen aus Authoring/Agenten werden an Tick-Grenzen angewendet.

## Konsequenzen

- **Keine Hash-Maps in Simulationscode** (Entscheidung 2026-11-23). `clippy.toml` verbietet `HashMap` und `HashSet` als Typen im Workspace. Gründe: Der Standard-Hasher ist zufällig geseedet, und das lässt sich nicht pro Methode abfangen (`HashMap::default()`, `collect()`, `HashMap::from` rutschten durch – belegt). Auch mit festem Hasher ist die Iterationsreihenfolge zwischen Plattformen und Rust-Versionen nicht festgelegt. Ersatz: `HandleMap` für Daten pro Entity (2,8–5,0 ns pro Lookup), `BitSet` für Mengen von Indizes, `BTreeMap`/`BTreeSet` oder sortierte `Vec` für alles andere, dichte Raster für Zellen. Preis, gemessen (Release, 2026-11-23, Windows, 16 logische Kerne, drei Läufe, zufällige Zugriffe): `BTreeMap::get` 31–39 ns bei 1000 Einträgen, 236–430 ns bei 1 Mio. Einträgen; `HashMap` mit `StableState` 21–37 ns bei 1 Mio. Große Schlüssel-Tabellen in Hot Paths brauchen also eine eigene Struktur (Raum-Index, Plan 17.1), keine `BTreeMap`. Host-Werkzeuge und Darstellungs-Crates dürfen die Regel lokal mit Begründung aufheben; für sie bleibt `StableState`.
- Floats in deterministischen Pfaden: nur mit dokumentierter Reihenfolge; Sagax rechnet weitgehend in Ganzzahlen – das bleibt der Default für Navigation/Entscheidung.
- Transzendente Funktionen nur über `knorke_math::det`; `clippy.toml` verbietet die Plattform-Varianten, `f32::min`/`max`, Uhren und Hash-Maps (ADR 0011).
