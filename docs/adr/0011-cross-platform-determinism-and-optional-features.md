# ADR 0011 – Plattformübergreifender Determinismus und abschaltbare Zusatzfeatures

**Status:** angenommen (lokal belegt: Windows x86_64 und `wasm32-wasip1` unter Node/V8; Linux und macOS/ARM prüft die CI)
**Datum:** 2026-11-23

## Kontext

1. Replays zwischen Rechnern und Lockstep/Rollback-Netzwerk verlangen bitgleiche Simulation auf Windows, Linux, macOS (ARM) und WASM. `f32::sin/cos/exp/powf/…` rufen die Mathe-Bibliothek der Plattform auf und dürfen sich im letzten Bit unterscheiden. HashMaps mit Zufalls-Seed und Uhren in Modulen zerstören Determinismus ebenfalls.
2. Instrumentierung, Aufzeichnung, Prüfregeln und Prüfsummen sind Zusatzfeatures. Sie müssen abschaltbar sein und abgeschaltet nichts kosten.

## Entscheidung

### Deterministische Mathematik

- `knorke_math::det`: `sin`, `cos`, `sin_cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `exp`, `exp2`, `ln`, `log2`, `log10`, `pow`, `powi`, `cbrt`, `tanh`, `hypot` – reines Rust über `libm` (musl-Port), überall dieselben Bits. `powi` mit fester Multiplikationsreihenfolge, definiert für jeden Exponenten einschließlich `i32::MIN`. Dazu `det::min`/`det::max` (siehe unten).
- Bereits exakt nach IEEE-754 und damit erlaubt: `+ - * /`, `sqrt`, `abs`, `clamp`, `floor`/`ceil`/`round`, `mul_add`, Float→Int-Casts mit `as` (sättigend, NaN → 0). `mul_add` ist nur mit FMA-Befehl schnell; das Standardziel x86_64 hat keinen und ruft eine Software-Routine auf (gemessen 2,0× gegenüber `a * b + c`) – nur verwenden, wenn die Genauigkeit gebraucht wird.
- **Nicht erlaubt: `f32::min`/`f32::max`** (und `f64`). Für `-0.0` gegen `+0.0` ist das Ergebnis nicht festgelegt. Gemessen auf demselben Rechner: `(-0.0f32).max(0.0)` ergibt im Debug-Build `0.0`, im Release-Build `-0.0`. Das Vorzeichen der Null wirkt weiter (`atan2`, Division). Ersatz: `det::min`/`det::max` mit festgelegtem Ergebnis (`+0.0 > -0.0`, NaN wird ignoriert, außer beide sind NaN) – gleich schnell wie `std` (1,01×).
- `knorke-math` nutzt selbst nur noch deterministische Funktionen: `Vec2::from_angle`/`angle` über `det`; sRGB-Umrechnung über exakte Tabellen (256 Werte + 255 Schwellen als Bitmuster) statt `powf` – deterministisch **und** schneller.
- **Mechanisch erzwungen** über `clippy.toml` im Workspace: verboten sind die plattformabhängigen `f32`/`f64`-Funktionen einschließlich `min`/`max`, `Instant::now`/`elapsed`, `SystemTime::now`/`elapsed`, die Typen `HashMap`, `HashSet` und `RandomState`. Darstellungs-Crates (Render, Audio, UI) dürfen mit einer eigenen `clippy.toml` lockern; Host-Werkzeuge erlauben es lokal mit Begründung (z. B. `Recorder`).

- **Hash-Maps sind als Typ verboten** (ADR 0003), nicht nur ihre Konstruktoren: `HashMap::default()`, `.collect::<HashMap<_, _>>()` und `HashMap::from(..)` mit `RandomState` entgingen dem Methodenverbot (belegt mit einem Clippy-Lauf). Das Typverbot erfasst alle vier Formen (ebenfalls belegt).
- **NaN:** Vorzeichen und Payload berechneter NaN sind plattformabhängig. Snapshots schreiben NaN deshalb kanonisch (ADR 0010); Simulationscode darf NaN-Bits nicht auswerten (`to_bits` auf möglichem NaN).
- **Hash-Map-Iteration** ist auch mit festem Hasher nicht plattformübergreifend festgelegt (Tabellenaufbau hängt von SIMD-Breite und `std`-Version ab; Einschätzung aus dem hashbrown-Quelltext, nicht auf ARM gemessen). Durch das Typverbot gegenstandslos.

Messung (Release, 1 Mio. Aufrufe, 2026-11-23, Windows, 16 logische Kerne; zwei Läufe):

| Funktion | `det` gegenüber `std` |
|---|---|
| `sin_cos` | 0,70–0,84× (schneller) |
| `atan2` | 0,88–0,99× |
| `exp` | 1,10–1,18× |
| `pow` | 4,15–4,65× – deshalb in Hot Paths meiden; Farbumrechnung nutzt Tabellen |

### Fingerprint-Test

- `tests/contracts/src/fingerprint.rs`: Simulation mit Trigonometrie, `exp`, `pow`, `sqrt`, Geometrie, Seed-Rauschen, parallelen Chunks und Slot-Wiederverwendung; 600 Ticks; Prüfsummen-Kette über alle Ticks.
- `tests/contracts/tests/determinism.rs`: eingefrorener Wert `0x8E13_81FE_2691_7217`. Lokal belegt: gleich in Debug und Release, gleich mit Serial, Rayon (3 Threads) und ScopedThreads, gleich auf `wasm32-wasip1` (Node 26 / V8 über `node xtask/wasi-run.mjs`, 2026-11-23). Geändert am 2026-11-23 von `0x9F89_7647_8771_1CC5`: `Seed::derive` mischt jetzt in zwei Runden (ADR 0003), das Rauschen der Fingerprint-Welt ist dadurch ein anderes. Gespeicherte Replays gab es noch keine. Alle übrigen Korrekturen des Reviews ließen den Wert unverändert.
- CI: Windows, Linux (x86_64), macOS 14 (ARM) und `wasm32-wasip1` unter wasmtime müssen denselben Wert liefern. Schlägt nur eine Plattform fehl, nutzt Simulationscode eine plattformabhängige Operation. Wird der Wert bewusst geändert, sind alle gespeicherten Replays ungültig – das gehört in die Commit-Message.

### Abschaltbare Zusatzfeatures ohne Kosten

Regel: **Jedes Zusatzfeature hat einen Aus-Zustand, der nichts kostet.** Zwei Wege:

| Weg | Wann | Kosten aus |
|---|---|---|
| **Compile-Zeit:** generischer Parameter mit No-op-Typ (`NoProbe`, `NoopSink`, `NoRecord`, `Serial`) | Release-Builds, Hot Paths | null – die Typen sind nullgroß (Vertragstest) und werden vollständig weginlined |
| **Laufzeit:** `Option<…>` bzw. leere Sammlung (`Option<Recorder>`, leere `Invariants`) | Entwickler-Builds, Umschalten im laufenden Spiel | ein vorhersagbarer Sprung pro Tick |

- `knorke_replay::TickDriver` vereinheitlicht die Spielschleife: dieselbe Schleife läuft mit `NoRecord`, `Recorder` oder `Option<Recorder>`.
- Fachmodule hängen an `knorke-observe` mit `default-features = false` (nur Verträge, kein `std`, kein Recorder).

Messung (Fingerprint-Welt, 300 Ticks, Release, 2026-11-23, drei Läufe; Abweichungen ± ~8 % sind Messrauschen):

| Variante | gegenüber direktem `sim.step` |
|---|---|
| `NoRecord` (aus, Compile-Zeit) | im Rauschen (−6 % … −0,1 %) |
| `Option<Recorder> = None` (aus, Laufzeit) | im Rauschen (−8 % … −1,4 %) |
| leere `Invariants::run` | im Rauschen (−7 % … +0,8 %) |
| `Recorder` an, Prüfsumme alle 60 Ticks | im Rauschen (−0,7 % … +0,6 %) |
| `Recorder` an, Prüfsumme jeden Tick | +10 % bis +18 % (ruhiger Rechner: +10,4 %, +10,0 %; spätere Läufe +15,0 %, +16,1 %, +18,3 %; unter Last Ausreißer von −5 % bis +39 %) |

Folgerung: Als dauerhaft laufender Flight Recorder Prüfsummen alle ~60 Ticks; jeden Tick nur in Tests und bei der Fehlersuche (`verify` grenzt dann auf den Tick genau ein).

## Konsequenzen

- Simulationscode schreibt `det::sin(x)` statt `x.sin()`. Clippy meldet Verstöße mit Begründung.
- `pow` ist teuer; in Hot Paths Tabellen oder `powi` verwenden.
- Neue Zusatzfeatures brauchen einen No-op-Typ oder `Option`-Pfad und eine Messung „aus vs. nicht vorhanden“ in `cargo bench -p knorke-contract-tests`.
- Simulationscode schreibt `det::max(a, b)` statt `a.max(b)`.
- Offen: GPU-Ergebnisse bleiben ausgenommen (nur Toleranzvergleich). Lokale Verifikation von Linux und ARM steht aus, bis die CI läuft (lokal: WSL-Ubuntu ohne Rust). WASM ist lokal über Node belegt; die CI nutzt wasmtime.
