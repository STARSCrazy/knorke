# Chiaro-Port – Ablaufplan und Übergabe

**Diese Datei ist der aktuelle Stand.** Wer am Port arbeitet (Mensch oder Agent), liest sie zuerst und aktualisiert sie nach jedem Schritt: Status in der Tabelle, Eintrag im Protokoll, „Nächster Schritt“ neu setzen. Ein neuer Agent muss allein mit dieser Datei weitermachen können.

## Einstieg für einen neuen Agenten

1. Lesen: `CLAUDE.md` (bindende Regeln, „Fertig heißt“), `docs/adr/0014-chiaro-port-split.md` (Schnitt), `docs/inventory/chiaro-aufteilung.md` §3 (Pakete im Detail: Quelldateien, Tests, Abhängigkeiten), bei Bedarf `docs/inventory/chiaro.md`.
2. Quelle: `D:\Entwicklung\chiaro` @ `9bc5b9b` (nur lesen, nichts ändern). Lizenz zlib: Herkunft (Repo + Revision + Pfad) und zlib-Hinweis im Kommentar jeder übernommenen Datei.
3. Abschnitt „Nächster Schritt“ unten ausführen. Pakete mit Status `offen`, deren Abhängigkeiten `fertig` sind, dürfen parallel laufen.
4. Pro Paket: eigener Branch oder Worktree (`port/pNN-kurzname`), zuerst die Chiaro-Unit-Tests nach Rust portieren (rot), dann Code (grün), dann alle Prüfungen aus `CLAUDE.md`. Merge nach `main` nur, wenn alles grün ist.
5. Danach diese Datei aktualisieren (Status, Protokoll, Nächster Schritt) und mit dem Paket committen.

## Nächster Schritt

- Jetzt: **Aufräumen, zweite Runde** (Leitlinie Manuel: Logik zusammenlegen, nicht Zeilen sparen). Gemergt: `cleanup/gpu-tests`, `cleanup/light-src`, `cleanup/ai`. Fertig gemeldet, noch zu mergen und zu prüfen: `cleanup/light-programs` (Lichtprogramm-Tabelle entsteht aus den Regeln) und `cleanup/snapshots-nav` (Snapshot-Makros in navigation/perception, `open_list` gemeinsam, Feature `test-support` in `knorke-gpu`).
- Dann: **Rust 1.100.0** (Auftrag Manuel 2026-11-24): `rust-toolchain.toml` und `rust-version` anheben, CI-Job `msrv` anpassen, `cargo fmt` (5 Stellen), 3 Clippy-Meldungen `needless_range_loop` (render, replay), volle Prüfung samt Fingerprint; neue Features nur dort, wo sie eigenen Code ersetzen (Never-Typ `!`, `Vec::from_fn`, `DropGuard`, `cargo::unused_dependencies`).
- Danach: P32 (Debug-Prüfung, hängt sich über `FrameFindings.gpu_findings` in den Pass-Plan), dann P33–P35 (2D-Helfer; erst damit sind die Demo-Szenen aus P3 nachstellbar).
- ADR 0017 (Pass-Plan) steht auf „vorgeschlagen“ – Manuel entscheidet.
- Abnahme von GPU-Paketen mit `KNORKE_REQUIRE_GPU=1`: gilt seit dem Aufräumen für **alle** GPU-Tests (Skip wird zum Fehler). Neue GPU-Tests nutzen `knorke_gpu::test_support` (Gerät, eine Sperre, Pack, Golden, Readback); light2d-Tests binden `tests/common/` ein.
- DLL-Größen zum Vergleich: `target/dll-out/groessen.txt` (nicht im Repo; Bau siehe Protokoll).
- P12, P15 (exotisch) erst bei Verbraucher.
- Abgebrochen? `git branch` / `git worktree list` prüfen, fertige Branches mergen, sonst neu starten. Parallele Agenten in `knorke-light2d` erzeugen kleine Konflikte in `lib.rs` und `tests/contracts/tests/no_alloc_light2d.rs` – beide Seiten behalten.

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
| Gate | Spike Rust vs. C++ (Licht + Schatten + Capture) | – | O | P1, P2, P3 | fertig (2026-11-24): **bestanden**, bitgleich, Zeit gleich → ADR 0015 |
| P1b | Sichere Aufnahme-API: geliehener Command Buffer, RenderPass/CopyPass, Draw, Bindung über Handles | `knorke-gpu` | O | Gate | fertig (Merge 2026-11-24): `CommandBuffer`, `RenderPass`/`CopyPass` als Guards, `TextureRef`/`SamplerRef`/`BufferRef`/`PipelineRef`, sichere Bindung + Uniforms (`Pod`); `knorke-light2d` wieder `forbid(unsafe)`; Spike weiter bitgleich |
| P8 | Licht: Culling, Fläche, Animation | `knorke-light2d` | S | P7, Gate | fertig (Merge 2026-11-24): `animation`, `area_light`, `light_culling` (+ `gpu_light`), Trait `ProfileAtlas`; `CullSettings` steht für P19, `OcclusionDomain`/`MaskAnchor` sind Kopien für P16 |
| P9 | Profile + IES/EULUMDAT-Import | `knorke-light2d` | H | Gate | fertig (Merge 2026-11-24): `profile`, `profile_store`, `profile_import` (IES, EULUMDAT); Anbindung an `LightStore`/`StoreResources`/`ProfileAtlas` offen → P9b |
| P10 | Masken (Gobo/Fenster) | `knorke-light2d` | H | P7, Gate | fertig (Merge 2026-11-24): `mask`, `mask_store`, `light_mask`, `resources` (`StoreResources`); `destroy` lehnt benutzte Masken/Animationen ab |
| P11 | Ambient-Zonen | `knorke-light2d` | H | Gate | fertig (Merge 2026-11-24): `ambient_zone`, `ambient_zone_store`, `ambient_zone_culling` |
| P12 | Medium-CPU | `knorke-light2d` | H | Gate | offen (exotisch, nur bei Verbraucher) |
| P13 | Hindernisse: Store, Desc, Stored | `knorke-light2d` | S | P5, Gate | fertig (Merge 2026-11-24): `obstacle/` (Store auf `Slots`, Versionen + Dirty-`BitSet`s für P16, Snapshot `knorke.light2d.obstacles`); **Restore alloziert noch** → P13b; Zell-/Muster-Teile → P14/P15 |
| P9b | Profile anbinden: `StoreResources` um `ProfileStore`, `ProfileAtlas` implementieren, private `profile_reference`-Kopie in `light.rs` ersetzen, Nutzerzähler pro Profil + „benutzt → nicht löschen“ | `knorke-light2d` | S | P8, P9 | fertig (Merge 2026-11-24): `StoreResources::with_profiles`, `ProfileStore` implementiert `ProfileAtlas`, `destroy` → `InUse` |
| P13b | Hindernis-Restore ohne Allokation (Geometrie in Store-Arenen statt `Vec` pro Hindernis); Regel aus CLAUDE.md | `knorke-light2d` | S | P13 | fertig (Merge 2026-11-24): Geometrie in Arenen (`Span`), Vertragstest `no_alloc_obstacle_restore` |
| P14 | Tile-Gitter + Muster an Hindernissen | `knorke-light2d` | S | P13 | fertig (Merge 2026-11-24): Zell-Editing, Muster (`PatternSource` für P15), `CellLog`; Snapshot v3; Batch-Compare offen |
| P15 | Muster-Store + Geometrie | `knorke-light2d` | S | P13 | offen (exotisch) |
| P16 | Schatten-Cache-Plan | `knorke-light2d` | O | P13 | fertig (Merge 2026-11-24): `shadow/` (`OcclusionDomain`, `DirtyRectList`, `MaskUpdatePlan`); Einbau → P23 |
| P17 | Post-Mathe: Shockwave, CRT, Bloom-Parameter | `knorke-post` (CPU) | H | – | fertig (Merge 2026-11-24): CRT, Bloom-Plan, Shockwave in `knorke-post`; Farbtemperatur (Kelvin→RGB) fehlt noch, Aufrufer liefert die Farbe |
| P18 | FrameWarnings → `Diagnostic` | je Crate | S | Gate | fertig (Merge 2026-11-24): `frame_warnings`; Muster-Warnungen → P15 |
| P19 | Settings + Qualitätspresets (Redesign) | je Crate | S | Gate | fertig (Merge 2026-11-24): `LightSettings`, `PostSettings`, ADR 0016; ersetzt `CullSettings`; Anbindung → P20 |
| P20 | Post-Pass: Output + Bloom + Tonemap | `knorke-post` | S | P2, P4, P17, P19, P1b | fertig (Merge 2026-11-24): `PostRenderer` (Bloom, Tonemap, Grade, CRT, Shockwave), bitgleich zu Chiaro (3 Szenen, RX 6700 XT); Anbindung → P31 |
| P21 | Druckwellen-Ringlicht | offen | H | P20 | offen (exotisch) |
| P22 | Hindernis-Raster | `knorke-light2d` | S | P2, P13, P14, P1b | fertig (Merge 2026-11-24): `occluder_raster/`, Teil-Upload in knorke-gpu; Maske/Höhe texelgleich zu Chiaro; Muster → P15 |
| P23 | Okklusionsmaske, Distanzfeld, Cache | `knorke-light2d` | O | P22, P16 | fertig (Merge 2026-11-24): `occlusion/` (`OcclusionMask`: Maske, JFA-Distanzfeld, Cache, Schwenk), `CopyPass::copy_texture_region`; texelgleich zu Chiaro; Sprite-Maske, Caster-Klassen → P25 |
| P24 | Licht-Programmtabelle (92 Programme) | `knorke-light2d` | O | P2, P1b | fertig (Merge 2026-11-24): `light_program` (92 Varianten), `LightPipelines` lazy; Uniform-Layouts → P25 |
| P25 | Lichtstufe | `knorke-light2d` | O | P24, P7, P8, P23 | fertig (Merge 2026-11-24): `lighting/` (`LightingStage`, `LightDraws`, Uniform-Blöcke, `program_for`); Licht-/Specular-/Direction-Puffer in 5 Szenen bytegleich zu Chiaro; Spike-Lichtpass läuft über die Stufe; Uniform-Layouts gegen SPIR-V geprüft; E53-Caster-Masken und Sprite-Maske offen (Plan 17.12) |
| P26 | Atlanten + Ambient-Pass | `knorke-light2d` | S | P25, P9, P10, P11 | fertig (Merge 2026-11-24): `atlas/` (Profil-/Masken-Atlas, `LightAtlases`), `zone_pass/` (`AmbientZonePass`), `LightingStage::prepare_with_zones` (Lichtpass lädt bei aktiven Zonen); 5 Szenen bitgleich zu Chiaro (Profil, Maske, Zonen); Profiltabellen aus P9 weichen ≤ 2 ulp ab (Plan 17.12) |
| P27 | Composite + Layout | `knorke-light2d` | S | P25 | fertig (Merge 2026-11-24): `composite/` (`CompositeStage`, `composite_layout_of`, `CompositeParams`, 6 Programme); 7 HDR-Szenen bytegleich zu Chiaro; Uniform-Layouts gegen SPIR-V geprüft; Spike-Composite läuft über die Stufe; Medium- und Zonen-Pfad nur über Layout/Pipelines geprüft |
| P28 | Muster-GPU | `knorke-light2d` | S/O | P23, P15 | offen (exotisch) |
| P29 | Medium-GPU 1 | `knorke-light2d` | S | P25, P12 | offen (exotisch) |
| P30 | Medium-GPU 2 | `knorke-light2d` | S | P29 | offen (exotisch) |
| P31 | Pass-Plan + Einbindung `knorke-integration` (Feature `gpu`), Snapshot/Inspect | Integration | O | P20, P25, P27 | fertig (Merge 2026-11-24): `pass_plan::PassPlan` (feste Stufenfolge, `prepare`/`record`, `Inspect`), Sprite-Maske als Stufe (`record_with_sprite`), reines Ambient im Composite, `knorke-integration` Feature `gpu` (`light2d::Light2dRenderer` = Pass-Plan + Post); Spike entfernt; 2 Spike-Szenen + 5 Atlas-Szenen bis HDR bitgleich; ADR 0017 (vorgeschlagen); Demo-Szenen aus P3 warten auf P33–P35 |
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

- 2026-11-24 · Agenten + Claude · **Aufräumen** gemerged. `cleanup/gpu-tests`: gemeinsamer GPU-Test-Helfer, netto −1 012 Zeilen Testcode, 117 GPU-Tests unverändert; voll geprüft (1538 / 1407). `cleanup/light-src`: −852 Zeilen Fachcode (davon ~600 nur kompakter geschrieben: Programmtabelle, `validate`; echt zusammengelegt: `uniform_block!`, `settings_error!`, `AtlasTexture<S>`). `cleanup/ai`: netto −870 (`by_fields!`/`by_tag!` in `knorke-types` für 43 Typen, vorher Bytes mit 127 Werten festgehalten; Zähl-Allokator und Test-Kits einmal). Alle drei zusammen voll geprüft: 1547 Tests grün (all-features, auch `--release`, mit `KNORKE_REQUIRE_GPU=1`), 1416 (no-default), Clippy 0, 21 Regeln, standalone ok. Alle Bildvergleiche bitgleich, Fingerprint unverändert.
- 2026-11-24 · Opus-Agent + Claude · Gemerged: P31 (Pass-Plan, Integration Feature `gpu`, Spike entfernt). Der Agent konnte seine Abschlussprüfung wegen Speichermangel (Jest-Läufe einer anderen Session) nicht beenden; Koordinator hat voll geprüft: 1538 Tests grün (all-features, auch `--release`, mit `KNORKE_REQUIRE_GPU=1`), 1407 (no-default), Clippy 0, 21 Regeln, standalone ok. Nicht geschafft: weitere Demo-Szene (braucht P33–P35). Zwei Opus-Audits (nur lesen) zur Codemenge: Fachcode im Kern knapp, ~1 700–1 950 Zeilen kürzbar; größter Hebel im Testcode. DLL-Messung (dylib, release): Code je Crate 30–300 KB, Rest der Dateigröße sind Rust-Metadaten (`.rustc`).
- 2026-11-24 · Sonnet-Agenten + Claude · Gemerged: P27 (Composite, 7 Szenen bytegleich) und P26 (Atlanten + Ambient-Zonen-Pass, 5 Szenen bitgleich; Profiltabellen ≤ 2 ulp neben Chiaro, Ursache `det`-Mathe in P9, Bild bleibt bitgleich). Prüfungen nach P27 allein: 1488 / 1399 grün. Nach beiden: 1528 Tests grün (all-features, auch `--release`, mit `KNORKE_REQUIRE_GPU=1`), 1402 (no-default), Clippy 0, 20 Regeln, standalone ok. Gestartet: P31 (Opus).
- 2026-11-24 · Opus-Agent + Claude · Gemerged: P25 (Lichtstufe, 5 Szenen bytegleich zu Chiaro, Uniform-Layouts ohne Abweichung). Bei der Abnahme stürzte der Release-Lauf einmal ab (Speicherzugriffsfehler in `knorke-post` `gpu_parity`, aus P20): `SDL_Init` gleichzeitig aus mehreren Test-Threads. Nachgestellt (3 von 60 Läufen unter Last), behoben (Sperre in `init_video`, Tests nacheinander), danach 160 Läufe ohne Absturz. Prüfungen: 1466 Tests grün (all-features, auch `--release`), 1390 (no-default), Clippy 0, 20 Regeln, standalone ok. Gestartet: P26, P27 (Sonnet).
- 2026-11-24 · Claude · P25 neu gestartet (Opus, eigener Worktree). Auftrag: Lichtstufe, Uniform-Layouts gegen Chiaro prüfen, Lichtteil des Spikes ersetzen, Golden-Dumper; E53 und Sprite-Maske, wenn es passt.
- 2026-11-24 · Agent + Claude · Gemerged: P23 (Maske + Distanzfeld texelgleich zu Chiaro, inkrementell = voll). Prüfungen: 1442 Tests grün (all-features, auch `--release`), 1380 (no-default), 20 Regeln, standalone ok. Gestartet: P25 (Opus).
- 2026-11-24 · Agenten + Claude · Gemerged: P20, P22, P24 (Post-Pass und Hindernis-Raster bitgleich zu Chiaro). Prüfungen: 1426 Tests grün (all-features, auch `--release`), 1374 (no-default), 20 Regeln, standalone ok. Gestartet: P23 (Opus).
- 2026-11-24 · Agenten + Claude · Gemerged: P9b, P13b, P14, P16, P18, P19. Prüfungen: 1353 Tests grün (all-features, auch `--release`), 1327 (no-default), 20 Regeln, standalone ok. Hindernis-Snapshot jetzt Version 3.
- 2026-11-24 · Claude · Gestartet: P16 (Opus), P14 (Sonnet), P18+P19 (Sonnet), P9b+P13b (Sonnet).
- 2026-11-24 · Agenten + Claude · Gemerged: P1b (sichere Pass-API, Spike bitgleich, light2d ohne `unsafe`), P13 (Hindernisse), P8+P10 (Culling, Animation, Masken), P9+P11 (Profile, Ambient-Zonen). Prüfungen: 1267 Tests grün (all-features, auch `--release`), 1241 (no-default), 20 Regeln, standalone ok. Neu als Pakete: P9b (Profil-Anbindung), P13b (Restore ohne Allokation).
- 2026-11-24 · Claude · Gestartet: P1b (Opus), P8+P10 (Sonnet), P9+P11 (Haiku), P13 (Sonnet).
- 2026-11-24 · Opus-Agent + Claude · **Gate-Spike gemerged und bestanden** (ADR 0015): `minimal_normal_map` und neue Referenz `gate_sprite_shadow` (C++ über C-API, `tests/golden/chiaro/gate/gate_ref.c`) bitgleich; Frame-Zeit Rust = C++ im Rauschen. Lücken: keine sichere Pass-API (→ P1b), keine Uniform-Layout-Prüfung (→ P24), SDL-GPU ohne Timestamps. Spike-Test: `cargo xtask shaders --src D:\Entwicklung\chiaro\shaders\compiled --formats spirv`, dann `KNORKE_SPIKE_OUT=<dir>`/`KNORKE_SPIKE_BENCH=<frames>`.
- 2026-11-24 · Opus-Agent + Claude · P2 gemerged. Packs Chiaro voll (151 Programme): SPIR-V 4,4 MB, MSL 10,2 MB, DXIL 2,2 MB. Noch ein gemeinsames Pack `knorke-chiaro`; Aufteilung light2d/post per `--only`, sobald die GPU-Stufen da sind. Chiaros feste Programmtabellen bleiben für P24. Prüfungen: 1126 Tests grün (all-features, auch `--release`), 1109 (no-default), 20 Regeln, standalone ok. Gate-Spike gestartet.
- 2026-11-24 · Sonnet-Agent + Claude · P5–P7 gemerged: Crate `knorke-light2d` (80 Tests, nur `knorke-types`/`knorke-math`), Vertragstest `no_alloc_light2d` (0 Allokationen), typisierte Fehler `knorke.light2d.*`, keine C-ABI-Flags. Gesamt nach Merge: 1103 Tests grün (all-features), 1097 (no-default), 20 Abhängigkeitsregeln.
- 2026-11-24 · Opus-Agent + Claude · P1 gemerged: `knorke-gpu` (Device geliehen, `GpuHandle` mit Lebendzähler, RenderTarget, Samplers, UploadBuffer, Staging), Diagnosecodes `knorke.gpu.*`, 9 Tests (4 echte GPU-Tests, überspringen ohne GPU), compile_fail-Test für Lebensdauer. `Device` bewusst weder `Send` noch `Sync`. P2 gestartet.
- 2026-11-24 · Sonnet-Agent + Claude · P3 gemerged: 12 Golden Scenes (8 deterministisch), Rauschen gemessen (Vulkan↔D3D12 ≤ 0,098 % Pixel), Schwellen STRICT (Toleranz 2, ≤ 100 ppm, Grenze 128) / LOOSE; 4 Szenen hängen an Uhr/Maus.
- 2026-11-24 · Haiku-Agent + Claude · P4+P17 gemerged: Crate `knorke-post` (31 Tests, nur `knorke-math`, 16 Abhängigkeitsregeln). Nicht portiert: C-API-Struktur-/Flag-Tests, SDL-Fehlertexte, MediumCloud-Tests (eigenes Paket), Kelvin-Farbe.

- 2026-11-23 · Claude · Aufteilung analysiert (`docs/inventory/chiaro-aufteilung.md`), ADR 0014 angenommen, diese Datei angelegt. Noch kein Code.
- 2026-11-23 · Claude · Welle 0/1 gestartet: P1, P3, P4+P17, P5–P7 (vier Agenten parallel).
- 2026-11-23 · Claude · Entscheidung: Shader nicht in der Bibliothek, sondern als getrennte Pakete pro Format (`.kpak`), ADR 0014 §3.
- 2026-11-23 · Claude · Entscheidung: Windows standardmäßig nur Vulkan (SPIR-V), D3D12 optionales Feature (ADR 0014 §3).
