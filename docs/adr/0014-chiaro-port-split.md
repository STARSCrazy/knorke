# ADR 0014: Chiaro-Port – Schnitt, Shader-Ablage, GPU als Hauptweg

Status: angenommen · Datum: 2026-11-23 · Bezug: ADR 0001, 0002, 0005, 0013, Plan §5 und §17.12, Inventur `docs/inventory/chiaro.md`, Aufteilung `docs/inventory/chiaro-aufteilung.md`, Ablauf `docs/chiaro-port/ABLAUF.md`

## Kontext

Chiaro (C++, SDL3-GPU, HLSL) soll nach Rust. Fachlich sind ~18 kLOC zu portieren. Die Aufteilung zeigt: nur ~1,2 kLOC sind allgemeiner GPU-Unterbau und ~1,5 kLOC allgemeine Nachbearbeitung; ~14,6 kLOC sind 2D-Licht, ~2,3 kLOC 2D-Helfer, ~2,3 kLOC entfallen. Ein Port in einem Stück ist nicht prüfbar. Gleichzeitig existiert mit ADR 0013 ein CPU-Licht (pixelraum) für Wizardry 7.

## Entscheidung

1. **Drei neue Crates statt einer.**
   - `knorke-gpu`: sicherer Wrapper um SDL3-GPU (Handles, RenderTarget, Sampler, UploadBuffer, ShaderLibrary, PipelineCache). Für 2D und ein späteres 3D. Einziger Ort mit `unsafe` für GPU (ADR 0005). Device, Swapchain und Submit gehören dem Host (ADR 0002).
   - `knorke-post`: Nachbearbeitung für 2D und 3D (Belichtung, Tonemap, Grading, Bloom, sRGB-Ausgabe, Skalierung/Letterbox, Dither). Abweichung von Plan §7.2, der Post `knorke-lighting` zuordnete: Post ist nicht lichtspezifisch.
   - `knorke-light2d`: 2D-Licht aus Chiaro (Lichter, Schatten, Hindernisse, Masken, Profile, Ambient-Zonen, Medium, Composite). CPU-Logik als GPU-freies Modul (ohne Feature `gpu` baubar und testbar), GPU-Stufen hinter Feature `gpu`.
   - 2D-Helfer (Layers, Sprites, Surface) kommen erst mit einem Verbraucher, bevorzugt nach `knorke-render` oder in die Integration. Keine leeren Crates.
2. **GPU ist der Hauptweg. Das CPU-Licht wird eingefroren, nicht verdoppelt.** Neue Lichtfunktionen entstehen nur auf der GPU. `knorke-lighting` (CPU, ADR 0013) bleibt auf dem pixelraum-Funktionsstand und wird nur gepflegt. Gründe, es nicht zu löschen: (a) es ist Teil eines kompletten CPU-Renderers (Pseudo-3D für Wizardry 7), den Chiaro nicht ersetzt – Chiaro ist 2D-Licht; (b) es läuft ohne Grafikkarte (CI, Server, headless) und ist bitgleich, also eine gute Prüfreferenz. Kein gemeinsamer Code zwischen CPU- und GPU-Licht erzwungen; sie teilen nur Begriffe. Wenn ein GPU-Renderer Wizardry 7 abdeckt, kommt `knorke-lighting` ins Archiv (neue ADR). Damit ersetzt dieser Punkt ADR 0013 §9 („GPU-Pfad in `knorke-lighting`“).
3. **Shader-Ablage pro Crate und pro Plattform.**
   - Jede Crate mit Shadern (`knorke-post`, `knorke-light2d`) hat `shaders/src/*.hlsl` und `shaders/compiled/<format>/` (eingecheckt). Übersetzen nur über `cargo xtask shaders` (shadercross); CI prüft per Prüfsumme, dass Blobs zu den Quellen passen. Kein shadercross im normalen Build.
   - Formate: **DXIL** (D3D12, Windows), **SPIR-V** (Vulkan, Windows/Linux/Android), **MSL** (Metal, macOS/iOS), jeweils mit Reflection-JSON.
   - **Shader liegen nicht in der Programm-Bibliothek, sondern in getrennten Shader-Paketen**, eins pro Crate und Format, z. B. `knorke-light2d.spirv.kpak`, `knorke-light2d.msl.kpak`, `knorke-light2d.dxil.kpak`. Ein Paket enthält die Blobs, die Reflection und einen Kopf mit Paketversion und Prüfsumme der Quellen. Bewusst Datendateien statt DLLs: eine DLL nur mit Daten bringt Lade- und Plattformaufwand ohne Nutzen. Ein Spiel liefert nur das Paket für sein Backend aus. `cargo xtask shaders` erzeugt alle Pakete.
   - Backends: Windows und Linux **standardmäßig Vulkan (SPIR-V)**, macOS Metal (MSL). Vulkan läuft auf praktisch allen Windows-PCs mit NVIDIA-, AMD- oder Intel-Treiber; eine Windows-Variante weniger zu testen. **D3D12 (DXIL) ist optional**, nur als eigenes Paket, für Xbox, ARM-Notebooks (Snapdragon), alte Intel-Grafik oder VMs ohne Vulkan.
   - Der Host wählt beim Start das Paket zum Backend und übergibt es (Pfad oder Bytes) an `ShaderLibrary`. Falsche Version, falsche Prüfsumme oder fehlendes Programm → `Diagnostic` (`knorke.gpu.shader_pack.*`), kein Panic.
   - Optional Feature `embed-shaders` (Standard aus): bettet ein Paket ein, für Tests und Ein-Datei-Programme. Ein Verzeichnis-Laden mit Hot Reload (Modding) ist dasselbe Format.
4. **Port in kleinen Paketen** (P1–P35, je ≤ ~1,2 kLOC, mit den Chiaro-Unit-Tests als Spezifikation), Reihenfolge und Modellstufe laut Aufteilung §3. Vor dem Gate-Spike nur P1–P3, P4–P7, P17. Exotische Effekte (Druckwelle, CRT, Medium, Muster) zuletzt und nur bei Verbraucher; ihre Crate wird dann entschieden.
5. **Nicht portiert:** C-ABI und C++-Wrapper, SDL_Renderer-Interop, Fingerprint-Bitgleichheitstests, alle Pfade mit eigenem Submit/Present/Readback. Neu entworfen statt übersetzt: Settings, Context (→ Pass-Plan), FrameWarnings (→ `Diagnostic`, Codes `knorke.light2d.*`, `knorke.post.*`, `knorke.gpu.*`), DebugCheck.

## Konsequenzen

- `xtask deps` bekommt Regeln: `knorke-light2d` und `knorke-post` kennen sich nicht; beide dürfen `knorke-gpu` nur mit Feature `gpu`; `knorke-gpu` kennt keine Fachcrate.
- Fortschritt und Übergabe stehen in `docs/chiaro-port/ABLAUF.md`; jede Änderung am Port aktualisiert diese Datei.
- Offene Prüfung: Reife von `sdl3-sys` für die GPU-API (P1). Fällt sie durch, ADR ändern (eigene Bindings oder `wgpu` prüfen).
