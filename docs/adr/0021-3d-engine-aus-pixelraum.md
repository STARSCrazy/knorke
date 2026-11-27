# ADR 0021: 3D-Engine aus pixelraum – eine Szene, zwei Renderwege, Einsatzprofile

Status: angenommen 2026-11-27 · Datum: 2026-11-27 · Bezug: ADR 0001, 0002, 0011, 0013, 0014, Plan 17.14

## Kontext

Manuel will pixelraum (in Knorke `knorke-render` + `knorke-lighting`, ADR 0013) Schritt für Schritt zu einer vollwertigen 3D-Engine ausbauen. Sie soll verschiedene Spiele tragen: zuerst Wizardry 7 (Gitter-Dungeon, Schrittbewegung, Pixel-Look), später einen Ego-Shooter (freie Geometrie, freie Bewegung, hohe Bildrate).

Heute:
- `knorke-render` rastert auf der CPU. Es kennt Gitter-Räume, Sprites, Dreieckslisten, Partikel, Hintergrund und Picking und ist für 160×100 bis 640×400 gedacht.
- `knorke-lighting` hat eine Sonne mit Schattenkarte und Punktlichter mit Würfel-Schattenkarten. Laut ADR 0014 ist es eingefroren: Neue Lichtfunktionen entstehen nur auf der GPU.
- Für die GPU gibt es `knorke-gpu`, für die Nachbearbeitung `knorke-post` (für 2D und 3D gedacht), dazu das 2D-Licht `knorke-light2d`.
- Plan §11 sagt bisher „3D-Renderer explizit noch nicht beginnen“.

Ein Shooter ist mit CPU-Rastern in kleiner Auflösung nicht zu machen. Ein Dungeon im Pixel-Look braucht dagegen keine GPU und läuft auch headless.

## Entscheidung

1. **Eine Szene, zwei Renderwege.** Ein gemeinsames 3D-Szenenmodell enthält Kamera, Gitter-Karten und freie Meshes als zwei gleichberechtigte Geometriequellen, Sprites, Materialien und Lichter. Es zeichnen zwei Wege:
   - **CPU** (`knorke-render`): Pixel-Look, ohne Grafikkarte, headless, Prüfreferenz.
   - **GPU** (neue Crate, erst mit Code, z. B. `knorke-render3d` auf `knorke-gpu`): Hauptweg für alles Neue, auch für den Pixel-Look in höherer Bildrate.
2. **Schatten in 3D über Schattenkarten** (kaskadiert für die Sonne, Würfelkarten für Punktlichter), nicht über die Lichtzeilen von ADR 0019. Die Lichtzeilen setzen eine Welt aus senkrechten Höhenintervallen voraus. Ob sie als schneller Sonderweg für reine Gitter-Welten taugen, wird erst mit Messung geprüft.
3. **Geteilt statt doppelt:** `knorke-post` (Tonemap, Bloom, Grading, CRT, Dither) für beide Wege. Es läuft nur auf der GPU; der CPU-Weg gibt dafür sein lineares Bild als Textur ab. Eine CPU-Kopie der Post-Effekte entsteht nicht (wäre doppelter Code); ohne GPU bleibt die bisherige CPU-Ausgabe. Die Licht-Daten (Profile/IES, Animation/Flackern, Masken, Kelvin) liegen heute in `knorke-light2d` und `knorke-post`; was 2D und 3D brauchen, wandert in eine gemeinsame Crate, sobald der zweite Nutzer da ist (Regel „mindestens zwei Nutzer“).
4. **Einsatzprofile statt Weichen im Kern.** „Dungeon“ (Gitter, Schritt, Pixel-Look) und „Shooter“ (Meshes, freie Bewegung) sind Profile in `knorke-integration`, wie heute `pixel`. Die Fachcrates bleiben standalone (ADR 0001).
5. **Was ein Shooter zusätzlich braucht, entsteht als eigene Module nach Bedarf:** Kollision und Physik (Plan 17.8), Skelett-Animation (17.4), Level-Format und Sichtbarkeit (Portale oder BVH), Maus-Steuerung, Netzwerk (17.3), Asset-Import (glTF).

## Schritte (je einzeln abnehmbar)

| Schritt | Inhalt | Nutzen |
|---|---|---|
| 3D-1 | CPU-Raster liefert sein lineares Bild (vor sRGB und Palette) als Textur an `knorke-post` (GPU: Belichtung, Tonemap, Bloom, Grading, CRT, sRGB-Ausgabe) | Bloom, Grading, CRT für den CPU-Renderer; ohne GPU bleibt der reine CPU-Weg (Palette, Dither) unverändert |
| 3D-2 | Szenenmodell aus `knorke-render` herauslösen, Meshes gleichberechtigt zum Gitter | gemeinsame Eingabe für beide Wege |
| 3D-3 | GPU-Raster derselben Szene ohne Licht; Vergleich gegen die CPU-Goldens mit Toleranz | Grundgerüst des GPU-Wegs |
| 3D-4 | GPU-Licht mit Schattenkarten, physikalisch plausiblen Materialien und Nebel | Licht auf Höhe von `knorke-lighting` und darüber |
| 3D-5 | Wizardry 7 auf Knorke (C-ABI `knorke-abi`, Plan 17.1) | erster echter Nutzer |
| 3D-6 | Shooter-Grundlagen: Kollision, Sichtbarkeit, Animation, Maus; kleine Demo | zweiter Einsatzzweck |

## Konsequenzen

- Plan §11 „3D-Renderer noch nicht beginnen“ fällt mit der Annahme dieser ADR.
- ADR 0013 und 0014 bleiben gültig: Die CPU bekommt kein neues Licht, sie bleibt Pixel-Look-Weg und Referenz.
- `xtask deps` bekommt Regeln für die neue GPU-Crate (kennt `knorke-gpu`, nicht `knorke-light2d`).

## Offene Fragen

- Muss der GPU-Pixel-Look genau wie die CPU aussehen (bitgleich) oder nur ähnlich (Toleranz)?
- Welches Level-Format für den Shooter: eigenes Format, glTF-Szenen oder etwas Gitterähnliches?
- Reihenfolge 3D-5 vor 3D-3/3D-4, falls Wizardry früher umziehen soll.
