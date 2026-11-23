# Inventur Chiaro (Phase 0)

Stand: 2026-11-23 · Quelle: `reference/chiaro` @ `9bc5b9b30cff57c784a3d675ed932700dec28a22` (451 Commits, 2026-11-09 … 2026-11-16) · Lizenz zlib (`LICENSE.txt`, GOALS.md „Offen“).
Alle Pfade relativ zu `reference/chiaro/`, sofern nicht anders angegeben. Belege als `Pfad:Zeile`.

Zweck: Entscheidungsgrundlage Weg A (C-ABI einbinden) vs. Weg B (Rust-Neubau `knorke-lighting`), Plan §5/§5.1. Bestandsaufnahme; Bewertungen nur in §7 und dort als Einschätzung markiert.

---

## 0. Kurzfassung

- **Build:** läuft auf diesem Rechner (MSVC 14.29 aus VS 2019 BuildTools + pip-CMake 4.4.4, SDL 3.4.20 per FetchContent, out-of-tree). Release: 0 Warnungen bei `/W4 /WX`. CTest 14/16 grün; die zwei roten sind die GPU-Suiten (Vulkan 5, D3D12 9 von 539 Fällen, je 1 davon erwartet) – fast alles Bitgleichheits-Vergleiche, die auf der Intel UHD **schon von Lauf zu Lauf** schwanken (§1.3).
- Version: `CMakeLists.txt:4` → 0.5.1, Header `include/Chiaro/Chiaro.h:46-48` → **0.5.3** (Micro = ABI-Revision, E138). README nennt noch API 0.4 (`README.md:3`) – Doku-Drift, wie im Plan §2.2 vermutet.
- Umfang: C++-Kern 23 580 LOC (`src/`), öffentliche Header 5 207 LOC, HLSL 6 028 LOC (64 Dateien → **151 Shaderprogramme**, je 4 Artefakte = 604 Dateien in `shaders/compiled/`, 18 MB), Tests 37 054 LOC (**405 Unit- + 539 GPU-Testfälle**), Beispiele 5 744, Benchmarks 1 894.
- Architektur: feste 8-Stufen-Pipeline ohne Render-Graph (E9, `src/core/Context.cpp:420-442`), nur Render-Passes (**keine Compute-Shader, keine Vertex-Buffer**), alle Zwischenpuffer RGBA16F/R16F/RG32F/R8G8.
- Frame-Vertrag passt weitgehend zu Plan §4.3 (App besitzt Device/CmdBuf/Swapchain, `CHI_Render` zeichnet nur auf) – mit **fünf dokumentierten Ausnahmen** (eigene Submits/Present/Readback, §2.4).
- Shader sind HLSL SM6, offline via shadercross nach SPIR-V/DXIL/MSL + JSON-Reflection; die Blobs sind ohne C++ aus Rust über `SDL_CreateGPUShader` nutzbar (§4).
- Keine Golden-Images im Testbaum; Tests prüfen analytisch mit Toleranz, plus FNV-Fingerprints für Build-gegen-Build-Bitvergleich. Demos haben deterministische `--screenshot --time`-Modi → beste Quelle für Contract-Szenen (§5).

---

## 1. Build-Fähigkeit (Windows, dieser Rechner)

### 1.1 Toolchain-Befund

| Komponente | Befund |
|---|---|
| `cmake` im PATH | fehlt |
| VS 2026 (18.12) / VS 2022 Professional | vorhanden, **ohne C++-Workload** (kein `VC/Tools/MSVC`, kein CMake unter `CommonExtensions/Microsoft/CMake`) |
| VS 2019 BuildTools | MSVC 14.29.30133 (cl 19.29.30159), CMake **3.20** + Ninja 1.10.2 unter `CommonExtensions/Microsoft/CMake` – CMake zu alt (Projekt verlangt 3.24, Presets v6 → 3.25) |
| Windows SDK | 10.0.19041 |
| pip-CMake | `C:/Users/<user>/AppData/Roaming/Python/Python315/Scripts/cmake.exe` **4.4.4** + ninja 1.13.2 (User-Install, nicht im PATH). Exakt die Kombination, die Chiaro selbst dokumentiert (`docs/ARCHITECTURE.md` E15: „MSVC 14.29 … mit CMake 4.4.4 per `pip install --user cmake`, Generator `Visual Studio 16 2019`“) |
| SDL3 | nicht installiert → `cmake/ChiaroDependencies.cmake:9-31` holt `release-3.4.20` per FetchContent ins Build-Verzeichnis (kein Systemeingriff) |
| doctest | FetchContent v2.5.3 (`cmake/ChiaroDependencies.cmake:34-42`) |
| shadercross | fehlt (`tools/bin/` leer) → „using precompiled shaders only“; für den Build nicht nötig (E3) |
| Vulkan SDK | nicht geprüft/nicht nötig; Vulkan-Treiber über GPU-Treiber |
| GPU | Intel UHD Graphics – dieselbe Referenz-GPU wie in Chiaros Benchmarks (E19, E21) |
| Rust | rustc 1.99.1 vorhanden; keine `sdl3`-Crates im lokalen Cargo-Cache |

### 1.2 Durchgeführt

Out-of-tree unter `reference/_build/chiaro/` (Repo danach `git status` sauber):

```
cmake --preset msvc -B reference/_build/chiaro/msvc     # Preset "msvc": VS 16 2019, x64, shared
cmake --build msvc --config Release --parallel
ctest --test-dir msvc -C Release --output-on-failure
```

Logs: `reference/_build/chiaro/configure.log`, `build-release.log`, `ctest-release.log`.

Ergebnis Configure: ok (99,8 s, davon SDL-Fetch), SDL-GPU-Treiber `d3d12 vulkan`, shadercross fehlt → vorkompilierte Shader. Build Release: Exit 0, keine `warning C`/`error`-Zeilen. Gebaut: `Chiaro.dll`, `SDL3.dll`, Unit-/GPU-/C-Tests, 8 Demos, 3 Benchmarks. Debug-Konfiguration und Static-Preset nicht gebaut. Linux nicht geprüft.

### 1.3 Testlauf (`ctest -C Release`, Intel UHD Graphics)

| Test | Ergebnis |
|---|---|
| `chiaro_consumer_install`, `chiaro_consumer_build` (find_package aus C/C++) | bestanden |
| `chiaro_unit_tests` (405 Fälle) | bestanden |
| C-ABI: `c_header_compile`, `c11_header_compile`, `c_link_test`, `c_dynload_test`, `c_dynload_version_test` | bestanden |
| `hello_light`, `minimal_normal_map`, `smoke_demo` `--smoke` (je Vulkan + D3D12) | bestanden |
| `chiaro_gpu_tests_vulkan` | **rot**: 534/539, 5 rot (davon 1 erwartet: Grazing-Ray, `tests/gpu/FieldTests.cpp:1160-1165`) |
| `chiaro_gpu_tests_d3d12` | **rot**: 530/539, 9 rot (davon 1 erwartet) |

Rote Fälle (Log `reference/_build/chiaro/ctest-release.log`):

| Testfall | Vulkan | D3D12 | Art der Prüfung |
|---|---|---|---|
| animated light renders like the unanimated light (`AnimationTests.cpp:118-133`) | rot | rot | Bitgleichheit zweier Wege |
| dither: slow gradient … sRGB targets (`DitherTests.cpp:161,169`) | rot | rot | Toleranz: Fehler 0,1106 statt < 0,08 (Format 52 = sRGB), worst 1,13 statt ≤ 1,1 |
| exact: specular 0 has no highlight (`MaterialTests.cpp:576`) | rot | rot | 21 statt ≤ 12 abweichende Pixel |
| frame lights come after the persistent ones (`FrameLightTests.cpp:98`) | rot | rot | Bitgleichheit |
| CHI_CreateTileGrids/SetTileGrids equal single calls (`BatchTests.cpp:266`) | rot | rot | Bitgleichheit |
| CHI_CreateLights equals single calls (`BatchTests.cpp:95`) | – | rot | Bitgleichheit |
| animation/mask/profile fingerprint: unused resource changes nothing (`AnimationTests.cpp:87`, `MaskTests.cpp:290`, Profile) | – | rot | Bitgleichheit |

**Nicht deterministisch:** Zwei Wiederholungen der betroffenen Fälle je Treiber liefern unterschiedliche Rot-Mengen (D3D12 4 bzw. 5, Vulkan 4 bzw. 3; z. B. `CHI_CreateShapes equals single calls` mal rot, mal grün). Stabil rot sind nur „animated light“ und „dither sRGB“. Gegenprobe Demo: `iso_night_demo --time 5 --screenshot` (1280×720) zweimal je Treiber → nicht bitgleich: D3D12 390 Bytes (0,011 %), Vulkan 282 Bytes (0,008 %) Differenz, Max-Abweichung 229/255 (Ausreißerpixel, keine Rauschfläche); D3D12 vs. Vulkan 2 251 Bytes (0,061 %). Bilder: `reference/_build/chiaro/shots/`. Chiaros Messungen der letzten Pakete liefen auf einer RX 6700 XT (GOALS Backlog Medien 1, E137); die Bitgleichheits-Tests sind offenbar dort grün und auf der Intel UHD nicht belastbar. Ursache nicht untersucht.

Folge für Contract-Tests (§5): Paritätsvergleich nur mit Toleranzmetrik und Ausreißer-Budget, nicht per Hash – auch innerhalb eines Treibers.

---

## 2. Öffentliche C-ABI (`include/Chiaro/`)

| Datei | LOC | Inhalt |
|---|---|---|
| `Chiaro.h` | 3 047 | einzige exportierte ABI: **154 exportierte Funktionen** (`extern CHI_DECLSPEC`), ~60 Structs/Enums, Inline-Helfer (`CHI_RGB`, `CHI_Kelvin`, Pixel-Encoder) |
| `Chiaro.hpp` | 2 011 | header-only C++20-RAII-Wrapper (move-only, `std::optional`, exception-frei), 15 Klassen |
| `Chiaro_bytes.h` | 30 | Byte-Makros (`CHI_MATERIAL_B` …) |
| `Chiaro_dynload.h` | 119 | `CHI_LoadLibrary`: `SDL_LoadObject` + ein Symbol `CHI_GetApi` → Funktionstabelle `CHI_Api` (`Chiaro.h:2867`, `:3040`) |

Konventionen: Präfix `CHI_`, Fehler = `false`/`NULL`/`id==0` + `SDL_SetError` (E5), jede Einstiegsfunktion in `Guard()` mit `catch(...)` (`src/capi/CApi.cpp:33-48`); keine `bool` in Structs (seit 0.4), Handles = `{Uint32 id}` mit 20 Bit Index + 12 Bit Generation (E18); Versionsprüfung einmalig über `header_version` statt `size`-Feldern (E6, E138). Struct-Layouts x64/x86 tabelliert in `docs/INTEGRATION.md` §19.

### 2.1 Funktionen nach Gruppen

| Gruppe | Funktionen | Header |
|---|---|---|
| Version/Laden | `GetVersion`, `GetApi` | `Chiaro.h:53`, `:3040` |
| Texturdeskriptoren | `FromRenderer`, `FromGPU`, `FromSwapchain`, `FromStream` | `:224-230`, `:2079` |
| Kontext | `CreateContext(device, CHI_VERSION)`, `DestroyContext` | `:1005`, `:1017` |
| Settings (ganz) | `GetDefaultSettings`, `GetSettings`, `SetSettings` | `:1263-1279` |
| Kurz-Setter | `SetAmbient`, `SetExposure`, `SetQuality`, `SetTonemap`, `SetBloom`, `SetHeightStep`, `SetWetness`, `SetHaze`, `SetDebugChecks`, `SetSpecularScale`, `SetGlowScale`, `SetScaling`, `SetSaturation`, `SetContrast`, `SetTint`, `SetVignette`, `SetOutputFlags`, `SetLightResolution` | `:1286-1307`, `:1522` |
| CRT | `InitCrtSettings`, `Crt(preset)`, `SetCrt`, `GetCrt`, `SetCrtCurvature` | `:1358-1389` |
| Druckwellen (pro Frame) | `InitShockwaveDesc`, `ExplosionShockwave`, `Shockwave`, `SetFrameShockwaves` | `:1432-1493` |
| Medien-Tabelle | `Medium(preset)`, `SetMedium`, `GetMedium` | `:1501-1517` |
| **Rendern** | `Render(ctx, cmdbuf, frame)`, `RenderToWindow`, `GetFrameStats` | `:1538`, `:1558`, `:1567` |
| Lichter | `InitLightDesc`, Konstruktoren `PointLight`/`SpotLight`/`LineLight`/`RectLight`/`SunLight`/`MoonLight`, `CreateLight`, `SetLight`, `GetLight`, `MoveLight`, `DestroyLight` | `:1626-1701` |
| Licht-Batches / Frame-Lichter | `CreateLights`, `SetLights`, `MoveLights`, `DestroyLights`, `SetFrameLights` | `:2265-2283` |
| Lichtprofile | `InitProfileShape`, `CreateProfile`, `CreateProfileFromIES`, `CreateProfileFromEULUMDAT`, `DestroyProfile`, `SampleProfile` | `:1708-1761` |
| Lichtmasken (Gobo/Fenster) | `InitMaskShape`, `CreateMask`, `CreateMaskFromPixels`, `DestroyMask`, `SampleMask` | `:1770-1815` |
| Ambient-Zonen | `InitAmbientZoneDesc`, `AmbientBox/Circle/Polygon`, `Create/Set/DestroyAmbientZone(s)`, `SampleAmbient` | `:1827-1884` |
| Muster-Hindernisse | `InitPatternDesc`, `CreatePatterns`, `DestroyPatterns`, `SamplePattern`, `SetObstaclePattern(s)`, `SetTilePatternTable`, `SetTilePatternCells` | `:1895-1961` |
| Lichtanimation | `InitAnimationDesc`, `Create/Set/Get/DestroyAnimation`, `SampleLightAnimation` | `:1970-2014` |
| Projektion | `OrthoProjection`, `IsoProjection` | `:2022`, `:2031` |
| Hindernisse | `InitShapeDesc`, `InitTileGridDesc`, `BoxShape`, `CircleShape`, `PoleShape`, `PolygonShape`, `CreateShape`, `CreateTileGrid`, `SetShape`, `SetTileGrid`, `SetTileCells`, `SetTileCellLevels`, `MoveObstacle`, `DestroyObstacle` | `:1576-1609`, `:2172-2255` |
| Hindernis-Batches | `CreateShapes`, `SetShapes`, `CreateTileGrids`, `SetTileGrids`, `MoveObstacles`, `DestroyObstacles` | `:2288-2303` |
| Streaming-Texturen (CPU-Framebuffer) | `CreateStreamingTexture`, `UploadStreamingTexture`, `DestroyStreamingTexture` | `:2053-2086` |
| Layers (chiaro-eigene Szenenpuffer) | `CreateRendererLayers`, `CreateGPULayers`, `DestroyLayers`, `ResizeLayers`, `BeginLayers`, `EndLayers`, `SetLayerTarget`, `GetLayerRendererTexture`, `GetLayerTexture`, `GetLayerColorTargets` | `:2122-2162` |
| Normalen/Material-Codec | `EncodeNormal`, `DecodeNormal`, `GetMaterialPreset`, `InitNormalMapOptions`, `ConvertNormalMap` | `:2320-2583` |
| Sprites | `InitSpriteDesc`, `CreateSprite`, `DestroySprite`, `DrawSprite`, `DrawSprites`, `GetSpriteTexture` | `:2627-2667` |
| Surface-Helfer (2.5D) | `SetLayerProjection`, `DrawSurfaceBox(es)`, `DrawSurfaceWall`, `DrawSurfaceFloor`, `DrawSurfaceQuad`, `BuildSurfaceGeometry`, `EncodeSurfaceSprite` | `:2689-2731`, `:2858` |
| Medium-Ebene | `DrawMedium`, `DrawMediums`, `MediumCloud` | `:2777-2802` |

Pro Frame: `CHI_Frame` (`Chiaro.h:321-352`) = `scene`, `target` (Pflicht), `layers`, `buffers` (`CHI_SceneBuffers`, `:253-312`: `overlay`, `occlusion`, `surface`, `material`, `glow`, `medium`), `projection`, `time` (Bibliothek liest nie eine Uhr → deterministisch), `skip_layers`. Selten Wechselndes in `CHI_Settings` (204 Byte, `:1238`).

### 2.2 Ownership und Lebensdauer

| Objekt | Besitzer | Regel | Beleg |
|---|---|---|---|
| `SDL_GPUDevice` | App | muss jeden Kontext überleben | `Chiaro.h:990-1005`, INTEGRATION §17 |
| Command Buffer | App | `CHI_Render` zeichnet nur auf, außerhalb jedes Passes aufrufen; nie Submit | `Chiaro.h:1524-1538`, E7 |
| Swapchain | App | nie acquired/präsentiert – außer `CHI_RenderToWindow` | `Chiaro.h:1541-1558` |
| Szene/Ziel/Puffer-Texturen | App | nur während `CHI_Render` gelesen; Format/Größe gibt die App an (`CHI_Texture`, SDL_GPU kann sie nicht abfragen) | `Chiaro.h:199-221` |
| `CHI_Context` | App | `DestroyContext` gibt alle eigenen GPU-Objekte frei; alle Handles werden ungültig | `Chiaro.h:1007-1017` |
| Lichter, Hindernisse, Profile, Masken, Zonen, Muster, Animationen | Kontext | Generations-Handles, veraltete werden abgewiesen | `Chiaro.h:348-358`, E18 |
| `CHI_Layers` | App | Renderer-Layers vor ihrem Renderer zerstören | INTEGRATION §17 |
| `CHI_Sprite` | seine Layers | | ebd. |
| `CHI_StreamingTexture` | App | Zerstörreihenfolge zum Kontext beliebig (verliert GPU-Ressourcen) | E7, `Chiaro.h:1009-1011` |
| Pipelines, Render-Targets, Sampler, Atlanten | Kontext | Pipeline-Cache je (Programm, Zielformat); Targets persistent, nur bei Größen-/Formatwechsel neu; `live_gpu_objects` als Leckzähler | E10, `src/gpu/PipelineCache.*`, `src/gpu/GpuHandle.hpp` |

Threading: Kontext nicht thread-sicher, alles auf dem Device-Thread; mehrere Kontexte pro Device erlaubt; Konstruktoren/Encoder thread-frei (E13, INTEGRATION §17).

### 2.3 Integrationswege (Szene-Quellen)

`SDL_GPUTexture` der App · Render-Target-Texturen eines `SDL_Renderer` mit GPU-Treiber auf **demselben Device** (`CHI_FromRenderer`, aufgelöst in `CHI_Render`, E62) · CPU-Framebuffer per `CHI_StreamingTexture` (Upload in den App-CmdBuf) · chiaro-eigene Layers (Renderer- oder GPU-Variante, `CHI_GetLayerColorTargets` liefert MRT-Infos für den eigenen Pass). Andere Renderer-Treiber (d3d11/opengl/software) nicht unterstützt (E8).

### 2.4 Abweichungen vom Plan-Vertrag §4.3 („kein Modul submittet/präsentiert eigenmächtig“)

| # | Stelle | Was passiert | Wann |
|---|---|---|---|
| 1 | `src/capi/CApi.cpp:512-534` | `CHI_RenderToWindow`: acquire CmdBuf, `WaitAndAcquireSwapchain`, Submit | nur dieser Komfortaufruf (explizit) |
| 2 | `src/core/Context.cpp:329-347` | `FlushOffscreenRenderers`: `SDL_RenderPresent` auf Offscreen-Renderern = Submit von deren CmdBuf **innerhalb `CHI_Render`**; bei Renderer-VSync blockiert das (E82, E89) | sobald ein `renderer_texture` im Frame steckt |
| 3 | `src/integration/Sprite.cpp:274-297` | `CHI_CreateSprite` (GPU-Layers): eigener CmdBuf mit Copy-Pass + Submit | Asset-Upload |
| 4 | `src/integration/MediumDraw.cpp:232-262` | `CHI_DrawMedium(s)` auf GPU-Layers: eigener CmdBuf, Render-Pass, Submit | pro Aufruf |
| 5 | `src/rendering/DebugCheckStage.cpp:67-130` | GPU→CPU-Readback (`SDL_DownloadFromGPUTexture`, 1×1-Ziel), 3 Frames verzögert gemappt | nur mit `CHI_DEBUG_LAYERS` |

Uploads im App-CmdBuf (konform): Copy-Passes in `LightingStage.cpp:431`, `MediumStage.cpp:518`, `OcclusionMask.cpp:219`, `PatternOccluders.cpp:397`, `StreamingTexture.cpp:100`; Transfer-Buffer mit Cycling (`src/gpu/UploadBuffer.cpp:51`). Kein CPU-Readback im Normalpfad.

---

## 3. Feature-Matrix

Reifegrad: **stabil** = seit ≥ 2 Paketen unverändert, getestet, in Demos; **neu** = 0.5-Paket, getestet, Messung nur auf einer GPU; **Grenze** = dokumentierte bekannte Schwäche. Tests = Anzahl `TEST_CASE` (U = Unit ohne GPU, G = GPU-Readback).

| Feature | Umsetzung | Belege (API / Kern / Shader) | Tests | Reifegrad |
|---|---|---|---|---|
| Punkt-/Spotlicht | additives instanziertes Quad je Licht in RGBA16F-Lichtpuffer; `saturate(1-(d/r)²)^falloff`, Spot `smoothstep` (E18) | `Chiaro.h:362-370`, `:588`, `:1635-1640` / `src/rendering/LightingStage.cpp:183,416`, `src/lighting/LightStore.cpp`, `LightCulling.cpp` / `light.vert.hlsl`, `light_common.hlsli:204-262` | G Lighting 14, LightApi 8, LightConstructor 8, HeightAuto 4; U LightMath 7, LightStore 12, LightCulling 8 | stabil (Phase 2) |
| Richtungslicht (Sonne/Mond) | Konstante in Ambient-Summe oder Vollbildpass je Licht mit Schatten/Normalen (E24) | `Chiaro.h:366`, `:1657-1661` / `directional*.frag.hlsl` | in Lighting/Shadow/Surface | stabil |
| Linien-/Rechtecklicht | im Punktlichtpfad, nächster Strahlerpunkt, Lambert-Fläche, eigene Schattenprogramme (E33, E51) | `Chiaro.h:367-369`, `:1645-1650` / `src/lighting/AreaLight.cpp` / `light_common.hlsli:138-228` | G AreaLight 11; U 8 | stabil |
| Frame-Lichter (transient) | `SetFrameLights`, ein Render lang (E44, E61) | `Chiaro.h:2283` | G FrameLight 5; U FrameLightAlloc 1 | stabil |
| Lichtprofile (IES/EULUMDAT) | Tabelle je Profil im R32F-Atlas (E31) | `Chiaro.h:1708-1761` / `src/lighting/ProfileImport.cpp` (658 LOC), `src/rendering/ProfileAtlas.cpp` / `profile.hlsli` | G Profile 9; U Profile 12, ProfileImport 14 (+6 Testdateien `tests/unit/data/profiles`) | stabil; Atlas wächst unbegrenzt bei Slot-Churn (GOALS Backlog Lichtmasken 7) |
| Lichtanimation | CPU je Frame aus `time` (Puls, Blitz, Rundum, Flackern) (E32) | `Chiaro.h:499-535`, `:1970-2014` / `src/lighting/Animation.cpp` | G 5; U 18 | stabil |
| Flackern | deterministisches Value-Noise aus `time` + Slot (E18) | `CHI_LightDesc::flicker` | in Animation/Lighting | stabil |
| Lichtmasken (Gobo, Fensterkreuz) | sRGB-Atlas, Projektor/Fenster-Modus (E115-E120) | `Chiaro.h:435-469`, `:1770-1815` / `src/lighting/Mask*.cpp`, `src/rendering/MaskAtlas.cpp` / `light_mask.hlsli` | G Mask 13; U Mask 15 | neu (0.5) |
| Ambient global + Zonen | Zonenpass vor Lichtpass, Priorität, weicher Rand, Höhenspanne (E121-E126) | `Chiaro.h:748-802`, `:1827-1884` / `src/lighting/AmbientZone*.cpp`, `src/rendering/AmbientZonePass.cpp` / `ambient_zone*.hlsl` | G 22+8; U 9+5 | neu |
| **Schatten: Verfahren** | Okklusionsmaske (R8G8) → Jump-Flooding-Distanzfeld (Seed RG32F, 6 Schritte, Distanz R16F, `kFieldBound`=32) → Sphere Tracing im Lichtshader, max. 64 Schritte, weich über Kegelterm (E19, E20) | `src/rendering/OcclusionMask.cpp`, `SdfShadows.cpp:116-169` / `jfa_seed/step/distance.frag.hlsl`, `light_sdf*.frag.hlsl`, `shadow_trace.hlsli:98` | G Shadow 43, Occlusion 23, Field 24, ShadowDefault 2, ShadowFilter 5 | stabil; **Grenze:** Grazing-Ray-Leck (erwarteter Fehlschlag `tests/gpu/FieldTests.cpp:1160-1162`), diagonale 1-Texel-Wände undicht, Halbschatten nur außen (E19) |
| Schatten-Cache inkrementell | Maske/Feld an Welttexeln verankert, Pan = Texturkopie, Dirty-Rects (≤ 64), Vollaufbau ab 50 % (E21, E157) | `src/shadows/MaskUpdatePlan.cpp:64`, `DirtyRects.cpp`, `OcclusionDomain.cpp` | U MaskUpdatePlan 20, DirtyRects 9, OcclusionDomain 7; G Field | stabil (3b), CPU-Logik GPU-frei testbar |
| Hindernisse | Polygon/Box/Kreis/Mast, Tile-Gitter (R8-Zelltextur), Sprite-Maske, statisch/dynamisch, Deckkraft, Höhe/Unterkante, Fußabdruck-Schatten, Kantenstreifen, Muster (E22, E25, E29, E59) | `Chiaro.h:665-885`, `:2172-2303` / `src/shadows/*` (3 530 LOC), `src/rendering/TileOccluders.cpp`, `OccluderGeometry.cpp` / `occluder.vert`, `tile.*`, `solid.frag`, `tile_strip.hlsli` | U ObstacleStore 44, Triangulate 10; G ObstacleApi 10 | stabil |
| Muster-Hindernisse (Zaun/Gitter) | R8-Atlas 4096², ID-Textur R16_UNORM, analytischer Eintritt (E127-E134) | `Chiaro.h:802-846`, `:1895-1961` / `src/shadows/Pattern*.cpp`, `src/rendering/PatternOccluders.cpp` / `pattern.hlsli` (660 LOC) | G Pattern 22, PatternApi 11; U 12+13 | neu; Vulkan-Compiler-Workaround (Backlog Muster 7) |
| Schattenwerfer-Filter je Licht | dynamic-only / static-only → zusätzliche Distanzfelder (E53, E76) | `Chiaro.h:555-558` | G ShadowFilter 5 | stabil |
| Normal-/Höhenpuffer („surface“) | RGBA8: R Höhe, GB Normale (2-Byte-Codec), A Flag; Bodenpunkt über invertierte 2×2-Projektion (E23) | `Chiaro.h:266-275` / `surface.hlsli` (436), `normal_codec.hlsli`, `light_surface*.hlsl` | G Surface 13; U NormalCodec 4, PixelCodec 14 | stabil |
| Materialpuffer | RGBA8: Detailnormale + Spec/Metal/Gloss-Byte, Blinn-Phong energienormiert, Toksvig, Nässe (E30, E48, E49, E72) | `Chiaro.h:276-296`, `:2337-2372` / `src/lighting/MaterialMath.cpp` / `surface.hlsli:196-347`, `surface_light.hlsli` | G Material 36, NearFieldWet 8; U MaterialMath 14, MaterialHelper 8 | stabil; `light_resolution<1` → Näherungsmodus `DIRECTION` |
| Glow/Emissive | RGBA8 additiv als HDR-Licht (E26) | `Chiaro.h:297-303`, `:1148` | G Glow 13, GlowDump 2 | stabil |
| Medien/Volumetrik (Dunst, Rauch, Nebel) | Medium-Ebene RGBA8, Streupuffer in reduzierter Auflösung, Polar-Sichtkarte für Lichtstreifen, Weltkarte für Abschirmung, Sonnen-Höhenkarte, HG-Phase (E92-E109) | `Chiaro.h:304-311`, `:1183-1229`, `:2745-2802` / `src/rendering/MediumStage.cpp` (607), `MediumMapStage.cpp`, `src/lighting/MediumCulling.cpp` / 13 `medium_*`-Shader, `medium_common.hlsli` (289), `medium_transmit.hlsli` | G Medium 60, MediumLayer 9; U MediumCulling 4, MediumTable 5 | neu; mehrere bekannte Grenzen (E95, E101, GOALS Backlog Medien 1-7), Performanceziele auf Referenz-GPU verfehlt (E105) |
| Bloom | 13-Tap-Downsample (Jimenez 2014), Tent-Upsample, RGBA16F, Addition im Output (E28) | `Chiaro.h:1163` / `src/rendering/BloomStage.cpp:96`, `src/core/Bloom.hpp` / `bloom_*.frag.hlsl` | G 10; U 6 | stabil |
| Tonemapping | NONE (Clip), SOFT (Knie), ACES (Narkowicz) im Output-Pass (E27) | `Chiaro.h:90-99` / `output.frag.hlsl:86-104` | in OutputFingerprint, Glow | stabil |
| Farbraum | Szene sRGB (Shader-Decode) oder linear, `*_SRGB` per Hardware; Zwischenpuffer linear RGBA16F; Ziel sRGB-Encode außer `*_SRGB`; Ziele R8G8B8A8/B8G8R8A8(_SRGB), R10G10B10A2, RGBA16F; **alles SDR, kein HDR10** (E11) | `Chiaro.h:80-84`, `:323-326` / `common.hlsli:5-12`, `src/core/ColorMath.hpp` | U ColorMath 7, Color 5, Formats 5 | stabil |
| Farbkorrektur/Vignette/Dither | Sättigung, Kontrast, Tint, Vignette, Bayer 8×8 nur 8-Bit-Ziele (E110-E114) | `Chiaro.h:1020-1079` / `output.frag.hlsl:105` | G GradeVignette 7, Dither 6; U OutputGrade 10 | neu, „neutral bitgleich“ per Fingerprint |
| CRT | Scanlines, Aperture Grille, Wölbung im Output-Pass (E136, E137) | `Chiaro.h:1312-1389` / `src/core/CrtMath.hpp`, `output.frag.hlsl:116-135` | G 9; U 10 | neu (nur RX 6700 XT gemessen) |
| Druckwellen | Verzerrung im Output + Ringlicht-Pass (E140-E144) | `Chiaro.h:1412-1493` / `src/core/Shockwave.cpp`, `src/rendering/ShockwaveLightPass.cpp` / `shockwave_light.*`, `output.frag.hlsl:136` | G 7; U 9 | neu |
| Skalierung/Pixel-Art | STRETCH/FIT/INTEGER, NEAREST/LINEAR, Letterbox, Overlay (HUD, unbeleuchtet, vormultipliziert) (E8, E12) | `Chiaro.h:58-73` / `src/core/OutputRect.cpp`, `src/rendering/OutputStage.cpp:75` | U OutputRect 5; G Render 14 | stabil |
| **2.5D/Iso** | `CHI_Projection` affin Welt→Szene, `CHI_IsoProjection` 2:1-Dimetrie; Licht rechnet in Weltkoordinaten (E17) | `Chiaro.h:242-249`, `:2022-2031` / `src/core/Projection.cpp` | U Projection 4; G Surface/Shadow | stabil |
| Batch-Ops | „erst alles prüfen, dann anwenden“ für Lichter, Hindernisse, Zonen, Muster (E45, E60, E77) | `Chiaro.h:2265-2303`, `:1867-1873`, `:1947` / `src/core/Batch.hpp` | G Batch 7 | stabil |
| Renderpfade | intern fest: Occlusion → MediumMap → Lighting → Medium → Composite → Bloom → Output → DebugChecks; Prepare (fehlbar) / Record (nicht fehlbar) (E9, E154). Extern: eigenes SDL_GPU, SDL_Renderer-Offscreen, CPU-Streaming, Layers, `RenderToWindow` | `src/core/Context.cpp:420-442` | G Frame 4, Layer 9, RendererInterop 4, StreamingTexture 4 | stabil |
| Lichtprogramm-Familie | Basis (`Light`, `LightSdf`, `Directional`) × 8 Merkmalsbits (Height, Surface, Exact, Direction, Area, Mask, Pattern, Medium) → 92 Fragment-Programme aus einer CMake-Regel (E152, E153) | `cmake/ChiaroLightPrograms.cmake`, `src/gpu/LightProgram.hpp` | U LightProgramLayout 4; G ShaderLibrary 14 | stabil, aber Kombinatorik |
| Qualitätspresets | EXACT/HIGH/MEDIUM/LOW/CUSTOM = Licht-, Schatten-, Medium-Auflösung (E63) | `Chiaro.h:1081-1108` / `src/core/Settings.cpp` | U Settings 29; G SettingsSetter 3 | stabil |
| Diagnostik | `CHI_FrameStats` (21 Zähler: Passes, Draws, Allokationen, live GPU-Objekte, Schattenfeldtexel …), Debug-Checks CPU+GPU mit Warnbits, Debug-Gruppen je Pass, Fehlertext nennt Feld (E14, E43, E69, E83) | `Chiaro.h:901-985` / `src/core/FrameWarnings.cpp`, `src/rendering/DebugCheckStage.cpp` | G DebugCheck 14, UnsupportedCombo 4; U FrameWarnings 11 | stabil |
| Profiling | nur CPU-Aufzeichnungszeit; **keine GPU-Timestamps** (SDL_GPU 3.4 hat keine) → RenderDoc/PIX; Bench-Programme messen Wall-Time über Fences (E14) | `Chiaro.h:903`, `benchmarks/*` | – | stabil, begrenzt |
| Layers/Sprites/Surface-Helfer | App-seitige Hilfen zum Befüllen der Puffer (Renderer oder GPU), Normal-Map-Konvertierung | `Chiaro.h:2091-2858` / `src/integration/*` (2 073 LOC) | G Layer 9, Sprite 6, SurfaceHelper 6; U SurfaceHelper 7 | stabil – gehört fachlich eher zum Renderer |

Nicht vorhanden: Compute-Pfad, HDR10-Ausgabe, GPU-Zeitmessung, Linux/macOS-Tests (Presets vorhanden, nicht gelaufen: GOALS Ziel 8), Modell B (eigener Sprite-Renderer, GOALS Ziel 3), Mehrthreading.

---

## 4. Shader-Pipeline

| Aspekt | Befund | Beleg |
|---|---|---|
| Sprache | HLSL Shader Model 6, Einstiegspunkt `main` | E3, `cmake/ChiaroShaders.cmake:122-124` |
| Übersetzung | `shadercross <src>.hlsl -s HLSL -d {SPIRV,DXIL,MSL,JSON} -t {vertex,fragment} -e main -D… -I shaders/src` (Entwickler-Target `chiaro_compile_shaders`, nicht in ALL) | `cmake/ChiaroShaders.cmake:110-140` |
| Toolchain-Beschaffung | `tools/bootstrap_shadercross.cmake`: shadercross @ `1ff05be`, DXC v1.9.2609 (Release-ZIP, SHA256 gepinnt), SPIRV-Cross, SDL 3.4.20 | `tools/bootstrap_shadercross.cmake:13-16` |
| Artefakte | `shaders/compiled/<name>.{spv,dxil,msl,json}` **eingecheckt**, 151 Programme, 18 MB | `shaders/compiled/` |
| Varianten | über `-D`-Defines aus einer Quelle (`CHI_HEIGHT`, `CHI_POINT_LIGHTS`, `CHI_LIGHT_MASK`, `CHI_MEDIUM`, `CHI_AMBIENT_ZONES` …) | `cmake/ChiaroShaders.cmake:23-50`, `cmake/ChiaroLightPrograms.cmake` |
| Einbettung | CMake erzeugt `ShaderBlobs.cpp` (Byte-Arrays + Reflection) und `ShaderIds.hpp` | `cmake/EmbedShaders.cmake`, `cmake/ChiaroShaders.cmake:146-186` |
| Formatwahl | zur Laufzeit DXIL > SPIR-V > MSL nach `SDL_GetGPUShaderFormats`; MSL-Entry `main0` (SPIRV-Cross-Umbenennung) | `src/gpu/ShaderLibrary.cpp:11-31`, `:57-70` |
| Reflection | JSON: `samplers`, `storage_textures`, `storage_buffers`, `uniform_buffers`, Ein-/Ausgänge → direkt die Felder von `SDL_GPUShaderCreateInfo` | z. B. `shaders/compiled/output.frag.json` |
| Register | SDL-Konvention: Vertex `t/s` space0, `b` space1; Fragment `t/s` space2, `b` space3; lückenlos ab 0 | E3 |
| Geometrie | **keine Vertex-/Index-Buffer**: Fullscreen-Dreieck per `SV_VertexID`, Instanzen aus Storage-Buffern (50 Programme mit Storage-Buffer, u. a. `light*.vert`, `ambient_zone.vert`, `*_pattern*.frag`) | grep `SDL_BindGPUVertexBuffers` in `src/` = 0 Treffer |
| Uniforms | nur `SDL_PushGPU{Vertex,Fragment}UniformData`, keine per-Frame-Buffer-Allokation | E10 |

Shader-Übersicht (64 Quelldateien, gruppiert):

| Gruppe | Dateien | Aufgabe |
|---|---|---|
| Gemeinsam | `common.hlsli`, `normal_codec.hlsli`, `fullscreen.vert`, `rect.vert` | sRGB↔linear (identisch zu `src/core/ColorMath.hpp`), Normalen-Codec, Vollbild, Rechteck-Instanzen für lokale Updates |
| Okklusion | `occluder.vert`, `solid.frag`, `tile.vert/.frag`, `tile_strip.hlsli`, `occlusion_mask.frag`, `occlusion_height.hlsli`, `clear.frag`, `pattern_id.frag`, `tile_pattern_id.frag` (+ `_height`-Varianten) | Hindernisse/Tiles/Sprite-Maske in die Maske rastern |
| Distanzfeld | `jfa_seed`, `jfa_step`, `jfa_distance` | Jump Flooding |
| Licht | `light.vert/.frag`, `light_surface.vert/.frag`, `light_sdf*.frag`, `directional*.frag`, `light_common.hlsli` (422), `surface_light.hlsli`, `surface.hlsli`, `shadow_trace.hlsli`, `profile.hlsli`, `light_mask.hlsli`, `pattern.hlsli` (660) | Lichtakkumulation, Schatten, Normalen/Material |
| Ambient | `ambient_zone.vert/.frag`, `ambient_zone.hlsli` | Zonen |
| Medium | `medium_*` (13 Dateien + 4 hlsli) | Dunst/Rauch, Streuung, Lichtstreifen, Weltkarte |
| Komposition/Post | `composite.frag` (150, 6 Varianten), `bloom_prefilter/down/up`, `bloom.hlsli`, `output.frag` (255), `shockwave_light.*` | Szene×Licht, Bloom, Output-Kette |
| Diagnose | `debug_check.vert/.frag` | GPU-Debug-Checks |

**Wiederverwendbarkeit aus Rust:** SDL3-GPU erwartet pro Backend Bytecode + `SDL_GPUShaderFormat` (D3D12: DXIL; Vulkan: SPIR-V; Metal: MSL-Quelltext oder Metallib) und die Ressourcenzahlen. Beides liegt fertig vor; ein Rust-Crate kann `shaders/compiled/*` per `include_bytes!` + JSON (build.rs) einbetten und dieselbe Formatwahl treffen – ohne C++-Code. Bindungslayouts (welche Textur in welchem Slot, Uniform-Struct-Layouts) stehen aber in C++ (`src/gpu/ProgramBinding.hpp`, `LightProgram.hpp`, `CompositeLayout.hpp`, `MediumLayout.hpp`, `src/rendering/GpuLight.hpp` u. a.) und müssen als `#[repr(C)]`-Structs nachgezogen werden. Für Shader-Änderungen braucht der Rust-Build weiterhin shadercross/DXC (oder Wechsel auf Slang/naga – nicht geprüft).

---

## 5. Tests, Benchmarks, Golden Images

| Art | Bestand | Beleg |
|---|---|---|
| Unit (ohne GPU) | 43 Dateien, 405 Testfälle, doctest; Mathematik, Stores, Culling, MaskUpdatePlan, Triangulierung, Profile-Import | `tests/unit/` |
| GPU | 49 Dateien, 539 Testfälle (+55 Subcases); headless Device, Offscreen-Render, Readback, Vergleich mit Toleranz/analytischem Erwartungswert; je Treiber registriert (`chiaro_gpu_tests_vulkan`, `_d3d12`), fehlender Treiber = Exit 77 = skipped | `tests/gpu/CMakeLists.txt:54-66`, E16 |
| C-ABI | Header mit C-Compiler, Link-Test, Dynload mit allen Tabelleneinträgen, Versions-Mismatch gegen Fake-Alt-DLL | `tests/capi/*` |
| Konsument | Install in privates Prefix + `find_package` aus C und C++ | `tests/CMakeLists.txt:6-24`, `tests/consumer/` |
| Doku | README-Beispiele C/C++ werden kompiliert | `tests/docs/` |
| Demo-Smoke | `hello_light`, `minimal_normal_map`, `smoke_demo` mit `--smoke` je Treiber als CTest | `examples/*/CMakeLists.txt` |
| Fingerprints | FNV-64 über Licht-/Output-Readback, mit `CHIARO_FINGERPRINT_OUT` als Datei → Bitvergleich zweier Builds (nicht gegen eingecheckte Referenz) | `tests/gpu/FingerprintScene.hpp:1-80`, `tests/gpu/OutputFingerprintTests.cpp:1-41`, Beispiel `docs/benchmarks/2026-11-14-lichtmasken/fingerprint-main-*.txt` |
| Golden Images | **keine im Testbaum**. PNGs nur als Sichtprüfungs-Belege in `docs/benchmarks/` (Medien vorher/nachher je Preset, CRT, Druckwelle, Lichtmasken, Ambient, Muster, Post) | `docs/benchmarks/2026-11-13-medien/perf/*.png` u. a. |
| Benchmarks | `shadow_bench` (Szenen boxes, mixed, static-cache, pan, threshold-*, doors; Messpunkt M = 1920×1080, Skala 0.5, 32 Lichter), `obstacle_bench` (statisch/dynamisch), `collect_bench` (CPU-Sammeln); Ergebnisse mit Rohlogs in `docs/benchmarks/` | `benchmarks/shadow_bench/main.cpp:1-40` |

**Kandidaten für Contract-Tests eines Rust-Ports** (deterministisch, weil `time` vorgegeben und keine Uhr gelesen wird):

| Szene | Quelle | prüft |
|---|---|---|
| Fingerprint-Szene (96×64, Punkt+Spot durch alle Programmzweige: plain, Schatten, Höhen, Surface, Material exact/direction) | `tests/gpu/FingerprintScene.hpp` | Lichtmodell, Schatten, Normalen, Material – klein, schnell, als Bild-Referenz exportierbar |
| Output-Fingerprint (64×48: alle Tonemaps, Bloom an/aus, Linear/Nearest mit Letterbox, Overlay, 3 Zielformate) | `tests/gpu/OutputFingerprintTests.cpp` | Farbraum, Tonemap, Bloom, Skalierung, Overlay |
| `shadow_bench` mixed / static-cache / pan bei M | `benchmarks/shadow_bench/main.cpp` | Schattenkosten, Cache-Verhalten, Pan-Stabilität; vorhandene Messwerte als Baseline (E21-Tabelle) |
| `iso_night_demo --screenshot --time t` (+ Schalter `--no-heights`, `--no-fog`, `--crt`, `--quality` …) | `examples/iso_night_demo` | 2.5D/Iso-Gesamtszene, Zonen, Muster, Medien |
| `smoke_demo --screenshot --time` (Explosion t2.5/t8, Säule t14) | `examples/smoke_demo` | Medien, Druckwelle; vorhandene PNGs in `docs/benchmarks/2026-11-13-medien/` |
| `wet_street_demo`, `normal_map_demo`, `cpp_renderer_demo --screenshot` | `examples/*` | Material/Nässe, Normal-Maps, Renderer-Interop |
| Analytische GPU-Tests (Pixelwerte mit Toleranz) | `tests/gpu/*Tests.cpp` | 1:1 als Rust-Integrationstests portierbar, wo nicht über `ContextInspect` Interna gelesen werden |

Einschränkung: Treiber unterscheiden sich (Plan §5.3 verlangt kein Bitidentität) – Fingerprints sind pro Treiber/GPU, für den Port braucht es Bilder + Toleranzmetrik, nicht Hashes.

---

## 6. Codestruktur (für die Zuordnung)

| Modul | LOC | Inhalt |
|---|---|---|
| `src/capi` | 2 038 | C-ABI-Fassade, Validierung, Guard |
| `src/core` | 2 927 | Context (RunStages), Settings, Projektion, Triangulierung, SlotMap, FrameWarnings, Shockwave/CRT-Mathe |
| `src/gpu` | 1 540 | GpuHandle (RAII + Lebendzähler), PipelineCache, RenderTarget, Samplers, ShaderLibrary, UploadBuffer, Programm-/Bindungslayouts |
| `src/rendering` | 6 143 | die Stufen (Occlusion, SdfShadows, Lighting, AmbientZonePass, MediumMap, Medium, Composite, Bloom, Output, DebugCheck, ShockwaveLight), Atlanten |
| `src/lighting` | 5 329 | LightStore, Culling, AreaLight, Animation, Profile(+Import), Mask, AmbientZone, Material-/Pixel-Codec, MediumTable |
| `src/shadows` | 3 530 | ObstacleStore, TileGrid, Pattern*, MaskUpdatePlan, DirtyRects, OcclusionDomain |
| `src/integration` | 2 073 | Layers, Sprite, StreamingTexture, RendererTexture, SurfaceGeometry, MediumDraw |

Dokumentation: `docs/ARCHITECTURE.md` (160 Entscheidungen E1-E160 mit verworfenen Alternativen und Messwerten), `docs/INTEGRATION.md` (1 339 Zeilen), `docs/API-PRINCIPLES.md`, `docs/GOALS.md` (Phasen, Backlogs), `docs/superpowers/{specs,plans,handoff}` (19 Specs, 14 Pläne). Externer Konsument: Syndicate (dynamisch über `Chiaro_dynload.h`, `CLAUDE.md`).

---

## 7. Einschätzung (nicht verifiziert, Bewertung des Inventurerstellers)

### 7.1 Umfang eines Rust-Ports

| Teil | C++/HLSL heute | Port-Aufwand |
|---|---|---|
| Shader (HLSL) | 6 028 LOC, 151 Programme | **übernehmbar** wie sie sind (Blobs + Reflection); nur Bindungs-/Uniform-Layouts nachziehen |
| GPU-Core | 1 540 | neu in Rust (Wrapper um SDL3-GPU), klein |
| Stufen + Atlanten | 6 143 | Kernarbeit: Pass-Aufbau, Target-Verwaltung, Draw-Logik |
| Licht-/Schatten-/Medien-Logik (CPU) | 5 329 + 3 530 + ~1 500 aus `core` | großteils reine Mathematik/Datenstrukturen, gut testbar portierbar (Unit-Tests als Spezifikation vorhanden) |
| Integration (Layers, Sprites, Streaming) | 2 073 | gehört größtenteils zu `knorke-render`, nicht zu `knorke-lighting` |
| C-ABI + Header + C++-Wrapper | 2 038 + 5 207 | entfällt (Plan: kein API-Bestandsschutz) |

Grob: **~17-19 k LOC C++ fachlich zu portieren**, davon ~8-9 k reine CPU-Logik und ~7-8 k GPU-Ablauf; Shader bleiben. Testbestand (~37 k LOC) ist die eigentliche Spezifikation und muss mitwandern, sonst gibt es keine Paritätsaussage. Der Repo-Stand entstand in einer Woche mit 451 Commits – Dichte und Detailtiefe (E-Einträge mit Messregeln) sind hoch, Feinheiten stecken in den Zeilen, nicht in der Architektur.

### 7.2 `knorke-gpu` vs. lichtspezifisch

| Gemeinsamer Layer (`knorke-gpu`/`knorke-render`) | Lichtspezifisch (`knorke-lighting`) |
|---|---|
| `GpuHandle` + Lebendzähler, `PipelineCache` (Programm × Zielformat), `RenderTarget` (persistent, Neuanlage-Zähler), `Samplers`, `ShaderLibrary` (Formatwahl, eingebettete Blobs, Reflection), `UploadBuffer` (Transfer-Cycling), Prepare/Record-Stufenmodell, Debug-Gruppen | Lichtpuffer, Lichtprogramm-Familie, Okklusionsmaske, JFA-Distanzfeld + inkrementeller Cache, Sphere Tracing, Profile/Masken/Muster-Atlanten, Ambient-Zonen, Medien-Stufe, Composite (Szene × Licht + Glow + Glanz) |
| Farb-Helfer (`common.hlsli`, `ColorMath.hpp`), Normalen-Codec, `CHI_Texture`-Idee (Format+Größe mitführen) | Material-/Surface-Codec-Semantik (Byte-Layouts) – Vertrag zwischen Render und Lighting, gehört in `knorke-core`/Verträge |
| Output-Stufe: Exposure, Tonemap, Grading, Vignette, Dither, sRGB-Encode, Skalierung/Letterbox, Overlay, CRT, Bloom | – (Post ist nicht lichtspezifisch; Plan §7.2 ordnet „Belichtungs-/Postpasses“ allerdings `knorke-lighting` zu – offen) |
| Layers, Sprites, Surface-/Medium-Zeichenhelfer, Streaming-Upload, SDL_Renderer-Interop | – |

### 7.3 Größte Portierungsrisiken

1. **Kombinatorik der Programme:** 92 Licht-Fragmentprogramme aus Basis × 8 Merkmalsbits mit Gültigkeitsregeln in CMake (`cmake/ChiaroLightPrograms.cmake`) und Bindungsbelegung daraus (E152/E153). Ein Port muss diese Tabelle exakt reproduzieren, sonst binden Slots falsch – stille Bildfehler statt Abstürze.
2. **Inkrementeller Schatten-Cache** (E21, E157): Maskenanker mit Subtexel-Rest, Dirty-Rect-Faltung, Vollaufbau-Schwelle, „verschwundener Inhalt“ – viele Randfälle, die nur Bildvergleiche nach Kamerafahrten aufdecken.
3. **Zahlengenauigkeit/Toleranzen:** Tests vergleichen Readbacks mit engen Toleranzen; sRGB-Decode-Ort, Rundung bei 8-Bit-Codecs und Float-Reihenfolge entscheiden über Bitgleichheit. Ohne Golden-Bilder fehlt eine treiberübergreifende Referenz – muss erst erzeugt werden (Phase-0-Aufgabe 2). Auf der Intel UHD ist schon die C++-Referenz nicht lauf-zu-lauf-bitgleich (§1.3); eine Paritätsmetrik braucht deshalb ein Ausreißer-Budget, sonst ist „Port weicht ab“ von „Treiber streut“ nicht zu trennen.
4. **SDL3-GPU aus Rust:** Reife der Rust-Bindings (`sdl3`/`sdl3-sys`) für die GPU-API nicht geprüft; Lebensdauer von CmdBuf/Pass/Transfer-Mapping in sicheres Rust zu gießen ist der Kern von Plan §5.2 Schritt 1.
5. **Bekannte, akzeptierte Fehler** (Grazing-Ray-Leck, gestapelte Puffs E95, T_sun E101): ein Port erbt sie oder „repariert“ sie und bricht damit Paritätsvergleiche – pro Feature in der Matrix §5.3 als `übernommen`/`neu entworfen` markieren.
6. **Shader-Toolchain:** Änderungen an HLSL brauchen shadercross+DXC (selbst gebaut, Bootstrap-Skript); in einer Rust-CI ist das ein Fremdkörper. Kurzfristig unkritisch, weil Blobs eingecheckt sind.
7. **Versteckte Submits (§2.4):** für Weg A müssen Ausnahmen 2-4 in der Integration gemieden werden (keine Renderer-Texturen, keine GPU-Layer-Helfer), sonst verletzt Chiaro den Vertrag §4.3. In Weg B entfallen sie per Design.

Für Weg A spricht: die C-ABI ist sauber (nur C-Typen, opaque Pointer, Funktionstabelle, Versionsprüfung, `bindgen`-tauglich laut `docs/INTEGRATION.md` §19) und `CHI_Render(ctx, cmdbuf, frame)` nimmt bereits fremdes Device und fremden CmdBuf – der Interop-Spike (Plan §5.1/1) ist mit geringem Aufwand machbar.
