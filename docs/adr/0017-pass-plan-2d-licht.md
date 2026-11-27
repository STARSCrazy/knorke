# ADR 0017: Pass-Plan des 2D-Lichts und GPU-Profil in `knorke-integration` (P31)

Status: vorgeschlagen · Datum: 2026-11-24 · Bezug: ADR 0001, 0002, 0008, 0010, 0011, 0014, 0016, `docs/inventory/chiaro-aufteilung.md` (P31)

## Kontext

Chiaro bündelt alles in `Context`: Stores, Settings, Shader-Pipelines, alle Stufen, Ausgabe und Statistik. `Context::Render` baut die Frame-Eingaben (`BuildFrame`), ruft erst alle `Prepare`, dann alle `Record` in fester Reihenfolge (`RunStages`) und wertet danach Warnungen aus. Nach dem Port liegen die Stufen einzeln vor (P22–P27), der Zusammenbau stand nur im Wegwerf-Spike `gpu_spike`. Licht und Ausgabe liegen in getrennten Crates, die sich nicht kennen (ADR 0014).

## Entscheidung

1. **`knorke_light2d::pass_plan::PassPlan`** ist die feste Stufenfolge eines Frames: Hindernis-Raster → Okklusionsmaske (mit Sprite-Maske) und Distanzfeld (seit ADR 0019, Plan 17.13 L7 Teil 2, ohne Distanzfeld: Höhen-Textur, Lichtzeilen und Sonnen-Karten) → Atlanten → Zonen- und Lichtpass → Composite. `prepare` (darf fehlschlagen, legt an, was der Frame braucht) und `record` (zeichnet in den geliehenen Command Buffer) wie bei jeder Stufe. Ergebnis ist eine Texturreferenz auf den HDR-Puffer. Kein Submit, kein Present, kein Readback (ADR 0002) – einzige Ausnahme ist die eingeschaltete Debug-Prüfung (Punkt 8). Kein Event-Bus, keine Callbacks: jede Stufe bekommt die Ergebnisse der vorigen als Wert (ADR 0008).
2. **Kein `Context`-Nachbau: Stores und Settings bleiben beim Host.** Der Plan bekommt sie pro Frame per Referenz (`SceneStores`, `LightSettings`) und besitzt nur GPU-Zustand und wiederverwendete Cull-Listen. Grund: Stores sind Simulationszustand mit Snapshot (ADR 0010) und werden auch von Werkzeugen, Agenten und anderen Systemen gelesen; ein Besitzer im Render-Pfad würde sie dort einsperren.
3. **GPU-Zustand ist Cache, kein Simulationszustand**: kein `Snapshot` für Plan und Stufen. Ein neuer Plan aus wiederhergestellten Stores zeichnet dasselbe Bild (Test); nach einem Restore in einen laufenden Plan ruft der Host `invalidate`.
4. **Beobachtbarkeit**: `Inspect` für den Plan (Stufen des letzten Frames, Maskenupdate, Materialmodus, Pipelines, Puffergrößen, Cache-Treffer), `check_invariants` meldet Verstöße als `Diagnostic` mit `knorke.light2d.pass_plan.invariant`; Fehler `knorke.light2d.pass_plan.{not_prepared,invalid_frame}`. Frame-Warnungen (ADR 0016) werden in `record` ausgewertet; Aus-Zustand `DebugChecks::NONE`. Dafür hängt `knorke-light2d` mit Feature `gpu` an `knorke-observe` (`default-features = false`, nur `knorke-types`).
5. **Ausgabe gehört nicht in den Plan.** `knorke_integration::light2d::Light2dRenderer` (Feature `gpu`, nicht in `default`) verbindet Pass-Plan und `knorke_post::pass::PostRenderer`. **Eine Host-Einstellung** speist den Farbraum beider Crates: `PostSettings::output.scene_color_space` ist zugleich der `InputColorSpace` des Composite.
6. **Abhängigkeitsregeln** (`xtask/src/deps.rs`) werden nur verschärft: `knorke-integration` erreicht SDL nur über `knorke-gpu`; `knorke-post` darf `knorke-integration` nicht kennen und parallelisiert nicht über `rayon`/`knorke-exec` (beides war bisher nur für die anderen Fachcrates geregelt).
7. **Nicht verbaut**: Medium (P12, P29, P30), Muster (P15, P28), Ringlichter (P21) kommen als weitere Felder in `PassFrame`/`SceneStores` und als Stufen zwischen Okklusion und Composite; die Stufen nehmen dafür heute schon `None`. Die Caster-Klassen-Masken (E53) brauchen keine eigene Eingabe und keine eigene Stufe: sie sind Teil der Okklusionsstufe (`OcclusionMask::prepare_casters`, gezeichnet in `record`), wie in Chiaro (E76); ohne sichtbares gefiltertes Licht wird nichts angelegt oder gezeichnet.
8. **GPU-Debug-Prüfung (P32) liest nur auf Anforderung zurück.** `DebugChecks::LAYERS` ist die Anforderung des Hosts im Sinn von ADR 0002. Dann zeichnet `debug_check::DebugCheckStage` nach dem Composite einen Pass in ein 1×1-Ziel und nimmt dessen Download in den Command Buffer des Hosts auf; gelesen wird der 4-Byte-Puffer vier aufgezeichnete Frames später, ohne eigenes Submit, ohne Fence, ohne Warten. Befunde kommen als `WarningSet` in die Frame-Warnungen (ADR 0016), also verspätet, nie undefiniert (Puffer starten genullt). Aus-Zustand (ADR 0011): nichts wird angelegt oder aufgezeichnet; das Bild hängt nie von der Prüfung ab.

## Konsequenzen

- `gpu_spike` (Gate-Spike) entfällt; seine zwei Szenen laufen über `Light2dRenderer` und bleiben bitgleich zu Chiaro.
- Die Sprite-Maske der Anwendung ist eine echte Zeichenstufe in `OcclusionMask` (`record_with_sprite`).
- Hosts, die nur die Stufen wollen, benutzen sie weiter einzeln; der Plan ist ein Angebot, kein Zwang (ADR 0001).
- Offen: die Szenen der C++-Demos (P3, 1280×720) brauchen die 2D-Helfer (Layers, Sprites, P33–P35) bzw. das Medium; erst dann sind sie über den Plan nachstellbar.
