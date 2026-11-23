# ADR 0002 – GPU-Ownership und Render-Lifecycle

**Status:** vorgeschlagen – Bestätigung nach Spike A (Plan Phase 1)
**Datum:** 2026-11-23

## Kontext

Renderer und Lighting sollen standalone funktionieren und im Verbund dasselbe `SDL_GPUDevice` ohne GPU→CPU→GPU-Umweg nutzen. Chiaro definiert heute eigene Ownership-/Frame-Verträge (siehe `docs/inventory/chiaro.md`); die Grafikbibliothek ist noch nicht identifiziert.

## Entscheidung (vorläufig)

1. **Der Host besitzt** Fenster, `SDL_GPUDevice`, Swapchain und Frame-Takt. Kein Fachmodul erzeugt implizit ein Device.
2. **Module bekommen pro Frame einen geliehenen Kontext** (Device + Command Buffer + Ziel-Targets). Sie zeichnen hinein, **submitten nicht und präsentieren nicht**.
3. Module dürfen eigene Ressourcen besitzen (Pipelines, Zwischen-Targets); deren Lebensdauer ist an ein vom Host geliehenes Device gebunden und wird explizit freigegeben, bevor das Device zerstört wird.
4. `knorke-lighting` nimmt **Eingaben als Texturen**, nicht als Renderer-Objekte: `color` (Pflicht), optional `normal`, `height`, `material`, `occlusion`, `emissive`. Formate und Farbraum stehen im Vertrag.
5. Farbraum: Lichtberechnung linear; Umrechnung nach sRGB genau einmal am Ende (Tonemap/Output-Pass). Wer das tut, entscheidet das Integrationsprofil `scene.render.lighting`.
6. `knorke-gpu` entsteht nur, wenn Spike A einen gemeinsamen minimalen Layer rechtfertigt (Handles, Pass-/Command-Vertrag). Bis dahin: SDL3-GPU-Typen direkt an der Grenze.
7. `unsafe` nur in einem dünnen SDL3-Wrapper; alle fachlichen Crates bleiben `unsafe_code = "forbid"`.

## Befund Chiaro (Inventur, `docs/inventory/chiaro.md` §2.4)

Chiaro hält den Vertrag weitgehend ein (`CHI_Render` zeichnet nur in den Command Buffer der App), mit vier Ausnahmen, die eigene Command Buffer submitten oder präsentieren: `CHI_RenderToWindow` (Komfortaufruf), Offscreen-Renderer-Texturen innerhalb `CHI_Render`, `CHI_CreateSprite` (Upload), `CHI_DrawMedium` auf GPU-Layers. Dazu GPU-Readback in Debug-Checks. Chiaro wird später in Rust neu gebaut (`knorke-lighting`). Diese Pfade sind deshalb keine Integrationsaufgabe, sondern eine Liste von Mustern, die der Neubau **nicht** übernimmt: Uploads und Medien-Draws zeichnen in den Command Buffer des Hosts, Offscreen-Ziele sind normale Texturen des gemeinsamen Devices, Debug-Checks lesen nur auf ausdrückliche Anforderung zurück.

## Offene Fragen für Spike A

- Rust-Binding: `sdl3-sys` direkt oder eigener schmaler Wrapper? Device-Loss-Verhalten?
- Shader: Chiaros HLSL/shadercross-Pipeline weiterverwenden (pro Backend SPIR-V/DXIL/MSL)?
- Reicht ein expliziter Pass-Plan (Liste mit Ressourcen-Ein-/Ausgängen), oder braucht es früh einen Render-Graph? (Plan: erst Pass-Plan.)
