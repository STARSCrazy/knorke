# Chiaro-Port – Ablaufplan und Übergabe

**Diese Datei ist der aktuelle Stand.** Wer am Port arbeitet (Mensch oder Agent), liest sie zuerst und aktualisiert sie nach jedem Schritt: Status in der Tabelle, Eintrag im Protokoll, „Nächster Schritt“ neu setzen. Ein neuer Agent muss allein mit dieser Datei weitermachen können.

## Einstieg für einen neuen Agenten

1. Lesen: `CLAUDE.md` (bindende Regeln, „Fertig heißt“), `docs/adr/0014-chiaro-port-split.md` (Schnitt), `docs/inventory/chiaro-aufteilung.md` §3 (Pakete im Detail: Quelldateien, Tests, Abhängigkeiten), bei Bedarf `docs/inventory/chiaro.md`.
2. Quelle: `D:\Entwicklung\chiaro` @ `9bc5b9b` (nur lesen, nichts ändern). Lizenz zlib: Herkunft (Repo + Revision + Pfad) und zlib-Hinweis im Kommentar jeder übernommenen Datei.
3. Abschnitt „Nächster Schritt“ unten ausführen. Pakete mit Status `offen`, deren Abhängigkeiten `fertig` sind, dürfen parallel laufen.
4. Pro Paket: eigener Branch oder Worktree (`port/pNN-kurzname`), zuerst die Chiaro-Unit-Tests nach Rust portieren (rot), dann Code (grün), dann alle Prüfungen aus `CLAUDE.md`. Merge nach `main` nur, wenn alles grün ist.
5. Danach diese Datei aktualisieren (Status, Protokoll, Nächster Schritt) und mit dem Paket committen.

## Nächster Schritt

- **Läuft seit 2026-11-23:** P1 (Opus, inkl. Entscheidung zur SDL3-GPU-Anbindung), P3 (Sonnet), P4 + P17 (Haiku, legen `knorke-post` an), P5–P7 (Sonnet, legen `knorke-light2d` an). Jeder Agent arbeitet in einem eigenen Worktree unter `.claude/worktrees/` und committet dort; der Koordinator prüft, merged nach `main`, trägt hier ein und pusht.
- Falls diese Arbeiten abgebrochen sind: `git branch` und `git worktree list` prüfen; fertige Branches mergen, sonst das Paket neu starten (Auftrag: Zeile in der Tabelle + Aufteilung §3).
- Danach: P2 (Opus, nach P1), dann **Gate-Spike** (Opus): ein Punktlicht, ein Schattenfall, Offscreen-Capture in Rust gegen dieselbe Szene aus C++-Chiaro (Golden Scenes aus P3); Passzeiten + Bilddiff messen; Ergebnis als ADR. Bei „nein“ stoppen und ADR 0014 ändern.

## Status der Pakete

Status: `offen` · `läuft` (mit Branch) · `fertig` (mit Commit) · `blockiert` (mit Grund) · `gestrichen` (mit Grund). Modell: H = Haiku, S = Sonnet, O = Opus. Ziel-Crate laut ADR 0014.

| Paket | Inhalt | Ziel | Modell | Abhängig von | Status |
|---|---|---|---|---|---|
| P1 | GPU-Grundlagen: Handles, RenderTarget, Sampler, UploadBuffer | `knorke-gpu` | O | – | läuft (Worktree; Merge durch Koordinator) |
| P2 | ShaderLibrary, ProgramBinding, PipelineCache, Shader-Pakete pro Format laden (`.kpak`; Vulkan/Metal, D3D12 optional) | `knorke-gpu` | O | P1 | offen |
| P3 | Golden Scenes aus C++-Demos + Paritätsmetrik | Tests | S | – | läuft (Worktree; Merge durch Koordinator) |
| P4 | Farbe, Formate, Letterbox | `knorke-post` (CPU) | H | – | läuft (Worktree; Merge durch Koordinator) |
| P5 | Projektion + Triangulierung | `knorke-light2d` (CPU) | S (mit P6, P7) | – | läuft (Worktree; Merge durch Koordinator) |
| P6 | Pixel-/Normal-/Material-Codec | `knorke-light2d` (CPU) | S | – | läuft (Worktree; Merge durch Koordinator) |
| P7 | Licht: Store + Math | `knorke-light2d` (CPU) | S | – | läuft (Worktree; Merge durch Koordinator) |
| Gate | Spike Rust vs. C++ (Licht + Schatten + Capture) | – | O | P1, P2, P3 | offen |
| P8 | Licht: Culling, Fläche, Animation | `knorke-light2d` | S | P7, Gate | offen |
| P9 | Profile + IES/EULUMDAT-Import | `knorke-light2d` | H | Gate | offen |
| P10 | Masken (Gobo/Fenster) | `knorke-light2d` | H | P7, Gate | offen |
| P11 | Ambient-Zonen | `knorke-light2d` | H | Gate | offen |
| P12 | Medium-CPU | `knorke-light2d` | H | Gate | offen (exotisch, nur bei Verbraucher) |
| P13 | Hindernisse: Store, Desc, Stored | `knorke-light2d` | S | P5, Gate | offen |
| P14 | Tile-Gitter + Muster an Hindernissen | `knorke-light2d` | S | P13 | offen |
| P15 | Muster-Store + Geometrie | `knorke-light2d` | S | P13 | offen (exotisch) |
| P16 | Schatten-Cache-Plan | `knorke-light2d` | O | P13 | offen |
| P17 | Post-Mathe: Shockwave, CRT, Bloom-Parameter | `knorke-post` (CPU) | H | – | läuft (Worktree; Merge durch Koordinator) |
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

- 2026-11-23 · Claude · Aufteilung analysiert (`docs/inventory/chiaro-aufteilung.md`), ADR 0014 angenommen, diese Datei angelegt. Noch kein Code.
- 2026-11-23 · Claude · Welle 0/1 gestartet: P1, P3, P4+P17, P5–P7 (vier Agenten parallel).
- 2026-11-23 · Claude · Entscheidung: Shader nicht in der Bibliothek, sondern als getrennte Pakete pro Format (`.kpak`), ADR 0014 §3.
- 2026-11-23 · Claude · Entscheidung: Windows standardmäßig nur Vulkan (SPIR-V), D3D12 optionales Feature (ADR 0014 §3).
