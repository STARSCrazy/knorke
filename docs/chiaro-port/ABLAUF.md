# Chiaro-Port – Ablaufplan und Übergabe

**Diese Datei ist der aktuelle Stand.** Wer am Port arbeitet (Mensch oder Agent), liest sie zuerst und aktualisiert sie nach jedem Schritt: Status in der Tabelle, Eintrag im Protokoll, „Nächster Schritt“ neu setzen. Ein neuer Agent muss allein mit dieser Datei weitermachen können.

## Einstieg für einen neuen Agenten

1. Lesen: `CLAUDE.md` (bindende Regeln, „Fertig heißt“), `docs/adr/0014-chiaro-port-split.md` (Schnitt), `docs/inventory/chiaro-aufteilung.md` §3 (Pakete im Detail: Quelldateien, Tests, Abhängigkeiten), bei Bedarf `docs/inventory/chiaro.md`.
2. Quelle: `D:\Entwicklung\chiaro` @ `9bc5b9b` (nur lesen, nichts ändern). Lizenz zlib: Herkunft (Repo + Revision + Pfad) und zlib-Hinweis im Kommentar jeder übernommenen Datei.
3. Abschnitt „Nächster Schritt“ unten ausführen. Pakete mit Status `offen`, deren Abhängigkeiten `fertig` sind, dürfen parallel laufen.
4. Pro Paket: eigener Branch oder Worktree (`port/pNN-kurzname`), zuerst die Chiaro-Unit-Tests nach Rust portieren (rot), dann Code (grün), dann alle Prüfungen aus `CLAUDE.md`. Merge nach `main` nur, wenn alles grün ist.
5. Danach diese Datei aktualisieren (Status, Protokoll, Nächster Schritt) und mit dem Paket committen.

## Nächster Schritt

- **Läuft:** Gate-Spike (Opus). Abgebrochen? `git branch` / `git worktree list` prüfen, fertige Branches mergen, sonst neu starten.
- Danach **Gate-Spike** (Opus): ein Punktlicht + ein Schattenfall + Offscreen-Capture in Rust gegen die Golden Scenes `minimal_normal_map` und `wet_street` oder `iso_night_hard` (Schwelle STRICT aus `manifest.toml`). Bringt einen PNG-Decoder für den Pixelvergleich mit (nur dev-dependency). Passzeiten + Bilddiff messen; Ergebnis als ADR. Bei „nein“ stoppen und ADR 0014 ändern.

## Umgebung (wichtig für die Prüfungen)

- `cargo test --all-features` baut SDL3 statisch (`sdl-build-static`) und braucht **CMake im PATH**: `D:\Dev-Tools\cmake\bin` (4.4.5) voranstellen. Ohne CMake schlägt das Build-Skript von `sdl3-sys` fehl.
- GPU dieses Rechners: AMD Radeon RX 6700 XT (die Intel UHD aus der Inventur ist nicht mehr verbaut). Chiaro-Szenen sind hier pro Backend lauf-zu-lauf bitgleich; Vulkan vs. D3D12 weichen an wenigen Kantenpixeln ab.
- C++-Chiaro bauen (für Vergleiche): `D:\Dev-Tools\cmake\bin\cmake.exe --preset msvc -B <build>` in `D:\Entwicklung\chiaro` (MSVC 14.29 aus VS-2019-BuildTools), dann `--build <build> --config Release`. Build-Verzeichnis außerhalb des Repos; der Build aus P3 lag im Scratchpad und ist ggf. weg.
- GPU-Tests überspringen sauber ohne GPU (`SDL_GPU_DRIVER=nonexistent` zum Ausprobieren).

## Status der Pakete

Status: `offen` · `läuft` (mit Branch) · `fertig` (mit Commit) · `blockiert` (mit Grund) · `gestrichen` (mit Grund). Modell: H = Haiku, S = Sonnet, O = Opus. Ziel-Crate laut ADR 0014.

| Paket | Inhalt | Ziel | Modell | Abhängig von | Status |
|---|---|---|---|---|---|
| P1 | GPU-Grundlagen: Handles, RenderTarget, Sampler, UploadBuffer | `knorke-gpu` | O | – | fertig (Merge 2026-11-24): `knorke-gpu` auf `sdl3-sys` 0.7.3 (SDL 3.4.20, zlib, alle 95 GPU-Funktionen); Features `sdl` (installiertes SDL3) / `sdl-build-static` (gebündelt, braucht CMake im PATH) |
| P2 | ShaderLibrary, ProgramBinding, PipelineCache, Shader-Pakete pro Format laden (`.kpak`; Vulkan/Metal, D3D12 optional) | `knorke-gpu` | O | P1 | fertig (Merge 2026-11-24): `.kpak`-Format (`kpak.rs`, ohne Features), `ShaderLibrary`, `PipelineCache`, `bind_program`, `formats`; `cargo xtask shaders` (aus Chiaro-Blobs, ohne shadercross); volle Packs nach `target/shaders/` (nicht im Repo), Test-Packs in `crates/knorke-gpu/tests/data/` |
| P3 | Golden Scenes aus C++-Demos + Paritätsmetrik | Tests | S | – | fertig (Merge 2026-11-24): 12 Szenen `tests/golden/chiaro/` (1,9 MB) + `manifest.toml`, Paritätsmetrik `tests/contracts/src/parity.rs` |
| P4 | Farbe, Formate, Letterbox | `knorke-post` (CPU) | H | – | fertig (Merge 2026-11-24): ColorMath, Grading, Dither, OutputRect in `knorke-post`; `Formats.*` (SDL-Formattabellen) verschoben nach P2/`knorke-gpu` |
| P5 | Projektion + Triangulierung | `knorke-light2d` (CPU) | S (mit P6, P7) | – | fertig (Merge 2026-11-24): `projection.rs`, `triangulate.rs` in `knorke-light2d` |
| P6 | Pixel-/Normal-/Material-Codec | `knorke-light2d` (CPU) | S | – | fertig (Merge 2026-11-24): `codec.rs`, `material.rs`; 8-Bit-Rundung bitgleich zur C++-Referenz |
| P7 | Licht: Store + Math | `knorke-light2d` (CPU) | S | – | fertig (Merge 2026-11-24): `light.rs`, `light_math.rs`, `light_store.rs` auf `Slots`, Snapshot `knorke.light2d.lights` v1; Ressourcen-Handles (Profil/Animation/Maske) sind Marker bis P8–P10 |
| Gate | Spike Rust vs. C++ (Licht + Schatten + Capture) | – | O | P1, P2, P3 | läuft (Worktree; Merge durch Koordinator) |
| P8 | Licht: Culling, Fläche, Animation | `knorke-light2d` | S | P7, Gate | offen |
| P9 | Profile + IES/EULUMDAT-Import | `knorke-light2d` | H | Gate | offen |
| P10 | Masken (Gobo/Fenster) | `knorke-light2d` | H | P7, Gate | offen |
| P11 | Ambient-Zonen | `knorke-light2d` | H | Gate | offen |
| P12 | Medium-CPU | `knorke-light2d` | H | Gate | offen (exotisch, nur bei Verbraucher) |
| P13 | Hindernisse: Store, Desc, Stored | `knorke-light2d` | S | P5, Gate | offen |
| P14 | Tile-Gitter + Muster an Hindernissen | `knorke-light2d` | S | P13 | offen |
| P15 | Muster-Store + Geometrie | `knorke-light2d` | S | P13 | offen (exotisch) |
| P16 | Schatten-Cache-Plan | `knorke-light2d` | O | P13 | offen |
| P17 | Post-Mathe: Shockwave, CRT, Bloom-Parameter | `knorke-post` (CPU) | H | – | fertig (Merge 2026-11-24): CRT, Bloom-Plan, Shockwave in `knorke-post`; Farbtemperatur (Kelvin→RGB) fehlt noch, Aufrufer liefert die Farbe |
| P18 | FrameWarnings → `Diagnostic` | je Crate | S | Gate | offen |
| P19 | Settings + Qualitätspresets (Redesign) | je Crate | S | Gate | offen |
| P20 | Post-Pass: Output + Bloom + Tonemap | `knorke-post` | S | P2, P4, P17, P19 | offen |
| P21 | Druckwellen-Ringlicht | offen | H | P20 | offen (exotisch) |
| P22 | Hindernis-Raster | `knorke-light2d` | S | P2, P13, P14 | offen |
| P23 | Okklusionsmaske, Distanzfeld, Cache | `knorke-light2d` | O | P22, P16 | offen |
| P24 | Licht-Programmtabelle (92 Programme) | `knorke-light2d` | O | P2 | offen |
| P25 | Lichtstufe | `knorke-light2d` | O | P24, P7, P8, P23 | offen |
| P26 | Atlanten + Ambient-Pass | `knorke-light2d` | S | P25, P9, P10, P11 | offen |
| P27 | Composite + Layout | `knorke-light2d` | S | P25 | offen |
| P28 | Muster-GPU | `knorke-light2d` | S/O | P23, P15 | offen (exotisch) |
| P29 | Medium-GPU 1 | `knorke-light2d` | S | P25, P12 | offen (exotisch) |
| P30 | Medium-GPU 2 | `knorke-light2d` | S | P29 | offen (exotisch) |
| P31 | Pass-Plan + Einbindung `knorke-integration` (Feature `gpu`), Snapshot/Inspect | Integration | O | P20, P25, P27 | offen |
| P32 | Debug-Prüfung (opt-in) | `knorke-light2d` | S | P31 | offen |
| P33 | 2D-Helfer: Layers | offen (Verbraucher) | S | P2, P31 | offen |
| P34 | 2D-Helfer: Sprite + Streaming | offen (Verbraucher) | S | P33 | offen |
| P35 | 2D-Helfer: Surface + Medium-Draw | offen (Verbraucher) | S | P33, P30 | offen |
| Ende | Feature-Matrix vollständig, C++-Chiaro archivieren | – | S | alle nötigen | offen |

## Offene Fragen

- Reife von `sdl3-sys` für die SDL3-GPU-API (P1). Fällt sie durch: ADR 0014 ändern.
- Ziel-Crate der exotischen Effekte (P12, P15, P21, P28–P30) – erst mit Verbraucher.

## Protokoll

Neueste oben. Datum · wer · was · Ergebnis (auch Fehlschläge, Messwerte).

- 2026-11-24 · Opus-Agent + Claude · P2 gemerged. Packs Chiaro voll (151 Programme): SPIR-V 4,4 MB, MSL 10,2 MB, DXIL 2,2 MB. Noch ein gemeinsames Pack `knorke-chiaro`; Aufteilung light2d/post per `--only`, sobald die GPU-Stufen da sind. Chiaros feste Programmtabellen bleiben für P24. Prüfungen: 1126 Tests grün (all-features, auch `--release`), 1109 (no-default), 20 Regeln, standalone ok. Gate-Spike gestartet.
- 2026-11-24 · Sonnet-Agent + Claude · P5–P7 gemerged: Crate `knorke-light2d` (80 Tests, nur `knorke-types`/`knorke-math`), Vertragstest `no_alloc_light2d` (0 Allokationen), typisierte Fehler `knorke.light2d.*`, keine C-ABI-Flags. Gesamt nach Merge: 1103 Tests grün (all-features), 1097 (no-default), 20 Abhängigkeitsregeln.
- 2026-11-24 · Opus-Agent + Claude · P1 gemerged: `knorke-gpu` (Device geliehen, `GpuHandle` mit Lebendzähler, RenderTarget, Samplers, UploadBuffer, Staging), Diagnosecodes `knorke.gpu.*`, 9 Tests (4 echte GPU-Tests, überspringen ohne GPU), compile_fail-Test für Lebensdauer. `Device` bewusst weder `Send` noch `Sync`. P2 gestartet.
- 2026-11-24 · Sonnet-Agent + Claude · P3 gemerged: 12 Golden Scenes (8 deterministisch), Rauschen gemessen (Vulkan↔D3D12 ≤ 0,098 % Pixel), Schwellen STRICT (Toleranz 2, ≤ 100 ppm, Grenze 128) / LOOSE; 4 Szenen hängen an Uhr/Maus.
- 2026-11-24 · Haiku-Agent + Claude · P4+P17 gemerged: Crate `knorke-post` (31 Tests, nur `knorke-math`, 16 Abhängigkeitsregeln). Nicht portiert: C-API-Struktur-/Flag-Tests, SDL-Fehlertexte, MediumCloud-Tests (eigenes Paket), Kelvin-Farbe.

- 2026-11-23 · Claude · Aufteilung analysiert (`docs/inventory/chiaro-aufteilung.md`), ADR 0014 angenommen, diese Datei angelegt. Noch kein Code.
- 2026-11-23 · Claude · Welle 0/1 gestartet: P1, P3, P4+P17, P5–P7 (vier Agenten parallel).
- 2026-11-23 · Claude · Entscheidung: Shader nicht in der Bibliothek, sondern als getrennte Pakete pro Format (`.kpak`), ADR 0014 §3.
- 2026-11-23 · Claude · Entscheidung: Windows standardmäßig nur Vulkan (SPIR-V), D3D12 optionales Feature (ADR 0014 §3).
