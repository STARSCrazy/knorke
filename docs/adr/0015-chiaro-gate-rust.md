# ADR 0015: Chiaro-Gate – Rust wird die Zielimplementierung

Status: angenommen · Datum: 2026-11-24 · Bezug: Plan §5.1 und §17.12, ADR 0002, 0005, 0014, `docs/chiaro-port/ABLAUF.md`

## Kontext

Plan §5.1 verlangt vor dem vollständigen Port einen Nachweis: Kann ein nativer Rust-Pfad auf dem eigenen GPU-Layer Chiaro gleichwertig nachbilden? Gebaut wurden `knorke-gpu` (P1, P2), die Golden Scenes (P3) und ein schmaler Spike in `knorke-light2d` (Feature `gpu`, Modul `gpu_spike`).

## Befund (gemessen 2026-11-24, AMD Radeon RX 6700 XT, Treiber 32.0.21045.16010, Windows 11, Vulkan)

- **Bildgleichheit:** bitgleich. `minimal_normal_map` (Punktlicht + Normal Map, 64×64) und `gate_sprite_shadow` (weiches Punktlicht, Okklusionsmaske, Jump-Flooding-Distanzfeld, SDF-Schatten; 192×128, 12 Passes wie in C++): Max-Abweichung 0, 0 Pixel über Toleranz. (Nachtrag 2026-11-27: mit ADR 0019 nimmt der Schatten den Weg der Lichtzeilen, das Distanzfeld ist gelöscht; `gate_sprite_shadow` hat ein neues Referenzbild unter `tests/golden/rows/`, Mittel 0,40, 1,75 % Pixel über 8 gegen Chiaro.)
- Abweichung vom Auftrag: Statt `wet_street`/`iso_night_hard` eine eigene C++-Referenz `tests/golden/chiaro/gate/gate_ref.c` über die Chiaro-C-API. Die beiden Demo-Szenen brauchen Hindernis-Store, Tile-Gitter und Höhen-/Oberflächenpuffer (P13, P14, P22) – mehrere kLOC, kein Spike. Der Schattenkern ist trotzdem abgedeckt.
- **Zeit:** gleich innerhalb des Rauschens (Wand-Uhr pro Frame inkl. Submit+Warten, 3 × 3000 Frames: Rust 189–212 µs Median vs. C++ 187–208 µs; Schattenszene 405–427 µs vs. 412–510 µs). Reine GPU-Passzeiten nicht messbar: SDL-GPU hat keine Timestamp-Queries.
- **Aufwand:** Spike ~630 LOC, 8 `unsafe`-Blöcke + 2 `unsafe fn`; inkrementeller Build von `knorke-light2d` 0,55 s → 0,93 s mit `gpu`.

## Entscheidung

1. **Rust ist die Zielimplementierung** für Chiaro (Plan §5.1: Gate bestanden). Weg A (C++ über C-ABI) wird nicht weiter verfolgt; C++-Chiaro bleibt nur Referenz für Paritätstests.
2. **Vor Welle 2 kommt Paket P1b:** sichere Aufnahme-API in `knorke-gpu` (geliehener Command Buffer, `RenderPass`/`CopyPass`, Draw, Bindung mit geliehenen Textur-Handles statt Rohzeigern). Danach haben Fachcrates wieder `forbid(unsafe_code)`; der Spike verstößt heute gegen ADR 0014 §1 („`unsafe` nur in `knorke-gpu`“) und wird mit P20–P27 ersetzt.
3. **Uniform-Layouts prüfen:** P24 bringt Tests der Byte-Größen und Offsets gegen die C++-`static_assert`s oder Reflection der Puffergrößen. Falsche Layouts würden sonst nur als falsches Bild auffallen.
4. GPU-Zeiten pro Pass werden bis auf Weiteres mit externem Profiler gemessen (RenderDoc, Radeon GPU Profiler).

## Konsequenzen

- Welle 1 (CPU-Pakete P8–P16, P18, P19) und P1b dürfen starten; Welle 2 nach P1b.
- `xtask deps` kennt `through`-Regeln: `knorke-light2d` erreicht SDL nur über `knorke-gpu`.
