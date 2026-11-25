# Auftrag für eine eigene Session: das 2D-Licht von Knorke neu denken

Stand: 2026-11-25. Geschrieben von Claude am Ende einer langen Session, als Übergabe an eine frische.
Antworte Manuel in einfacher Sprache und kurz (Output-Style „Klartext“).

## Worum es geht

Knorke hat ein vollständiges 2D-Licht, portiert aus der C++-Bibliothek Chiaro: `crates/knorke-light2d`, rund 29 000 Zeilen Rust (mit Tests) und rund 6 000 Zeilen HLSL in 151 Shader-Programmen. Es ist bitgenau gegen Chiaro geprüft (sieben Demo-Szenen, Stand in `docs/chiaro-port/ABLAUF.md`).

Manuel findet das viel zu groß. Seine Worte, sinngemäß:

- „29 000 Zeilen nur für Licht ist krank viel. Wir dürfen es nicht komplett übertreiben.“
- Er will die Funktionen **nicht grundsätzlich wegwerfen** („ich finde die Features cool“), aber das Ganze soll **grundlegend neu überlegt** werden.
- Er will sich **nicht auf eine Ansicht festlegen** (Draufsicht oder schräg von oben mit hohen Wänden): „Wir brauchen einfach ein Beleuchtungssystem.“
- Wichtig ist ihm **Geschwindigkeit**: „schnelle Lichter vor allem“. Gern mit mathematischen Tricks, klugem Umgang mit Speicher, Vereinfachungen. Vorhandene Ansätze darf und soll man **weiterdenken**.
- Qualität darf minimal leiden, wenn es dafür deutlich schneller wird (Richtwert von ihm: 20–30 % schneller für etwa 1 % Verlust, den man normalerweise nicht sieht). Kein Qualitäts-Schalter nötig. Das gilt für das, was man sieht – die Simulation bleibt exakt.

**Dein Auftrag:** Denk das gründlich durch und schlag einen Entwurf vor, wie Knorke zu einem kleinen, schnellen Licht kommt, das die Funktionen im Prinzip abdeckt. Ergebnis ist ein Vorschlag (ADR-Entwurf plus Plan für einen kleinen Versuch), kein Umbau. **Lösch und kürz am Chiaro-Licht nichts**, bevor ein Ersatz mit Bild und Messung belegt ist.

## Was das heutige Licht kann (das Soll)

- Punkt-, Spot-, Richtungs- (Sonne/Mond), Linien- und Flächenlichter; harte und weiche Schatten (Lampengröße).
- 2.5D: isometrische Projektion, Hindernisse mit Höhe, Schattenlänge aus der Höhe, Licht hat eine Höhe über dem Boden.
- Normal-/Material-Maps (Glanzlichter), Leucht-Ebene (Glow), Umgebungslicht, Ambient-Zonen (innen dunkler als außen).
- Lichtmasken/Projektoren (Fenster, Gobos), Lichtprofile realer Leuchten (IES), Animation/Flackern.
- Halbdurchsichtige Hindernisse über Muster (Zäune, Gitter).
- Nebel/Rauch mit Streuung und sichtbaren Lichtstreifen, Lichtring von Druckwellen.
- Deterministisch pro Frame, linear gerechnet, danach Post (`knorke-post`: Bloom, Tonemap, Grade, CRT).

Wo die Zeilen stecken (grob, `src` mit eingebetteten Tests): Hindernisse und ihre Schatten ~7 000 (`obstacle`, `occluder_raster`, `shadow`, `occlusion`), Nebel/Rauch ~3 000 (`medium_*`), Muster ~2 300 (`pattern_*`), Lampen-Profile ~1 600 (`profile*`), Lichtstufe ~2 000 (`lighting`), Pass-Plan ~1 300, dazu Masken, Zonen, Animation, Warnungen, Prüf-Werkzeuge. Chiaro dokumentiert 160 Einzelentscheidungen; die sind alle mit übernommen.

**Meine Diagnose:** Die Größe kommt weniger von einem aufgeblähten Verfahren als von der Breite – jede Funktion hat ihren eigenen Pfad, eigene Shader-Varianten und eigene Sonderfälle.

## Was die Recherche ergeben hat (Radiance Cascades)

Ein Agent hat am 2026-11-25 recherchiert. Kennzeichnung: **[belegt]** = Primärquelle gelesen, **[sekundär]**, **[Einschätzung]**.

- **[belegt]** Holographic Radiance Cascades (Freeman, Sannikov, Margel, arXiv 2505.02041, Mai 2025): Licht in einer 2D-Ebene aus zwei Eingaben pro Zelle, **Emission und Extinktion**. Feste Wände sind einfach sehr dichtes Medium; Nebel, Rauch und halbdurchsichtige Dinge sind dasselbe mit weniger Dichte. Kosten hängen nur von der Auflösung ab, nicht von der Zahl der Lichter. Messwerte der Autoren (RTX 3080 Laptop): 256² 0,55 ms · 512² 1,85 ms · 1024² 7,67 ms · 2048² 33,9 ms. Das Paper liegt als PDF und Text unter `…\scratchpad\dl\hrc.pdf` / `hrc.txt` der alten Session (kann weg sein; dann neu laden).
- **[belegt]** Schwächen laut Paper: Lichter kleiner als etwa das 8-Fache des Probenabstands zeigen Muster-Fehler; Schatten werden um etwa 4 px weicher; Glanz ist schwach; indirektes Licht über mehrere Sprünge braucht mehrere Durchläufe; Lichter außerhalb des Bilds brauchen eine Sonderbehandlung; die Beschleunigungsstruktur braucht viel Speicher. Die Autoren empfehlen, in halber bis viertel Auflösung zu rechnen und hochzurechnen.
- **[sekundär]** Klassische Radiance Cascades (Sannikov 2023) gehen ganz in Fragment-Shadern mit Ping-Pong-Zielen, meist mit einem Distanzfeld aus Jump Flooding (Tutorial mit Code: jason.today/rc). Bekannte Fallen: Ringe um Lichter ohne „Bilinear Fix“, Lichtlecks beim Zusammenführen, falsches Rechnen in sRGB.
- **[belegt im Paper, sonst unbestätigt]** Im Einsatz in Path of Exile 2. Eine gepflegte Bibliothek gibt es nicht, nur Demos und Hobby-Repos (u. a. mehrere in Rust/Bevy/wgpu, eines in C++/SDL3). Zeilenzahlen und Pflegezustand hat der Agent nicht geprüft.
- **[Einschätzung, keine Quelle gefunden]** 2.5D mit Höhen und isometrischer Projektion: Das Verfahren kennt nur eine flache Ebene. **Das ist die größte Lücke.**
- Abgleich mit dem Soll: von selbst gehen Punkt-, Linien-, Flächenlicht, weiche Schatten, indirektes Licht, Glow, Flackern, halbdurchsichtige Hindernisse, Nebel mit Lichtstreifen (in der Draufsicht), Lichtring. Nur mit Zusatz: Spot, Richtungslicht, Normal-Maps, Masken, Ambient-Zonen, Isometrie. Passt nicht: harte Schatten kleiner Lampen, Höhen, IES-Profile.

Andere kleine Verfahren (alles **[Einschätzung/Vorwissen]**, nicht nachgeprüft): Schatten-Karte pro Licht (1D/polar; Godot 2D), Schatten-Geometrie aus Kanten (Unity URP 2D; üblich für Höhen, weil die Schattenlänge direkt in die Geometrie eingeht), Distanzfeld + Strahl-Marsch pro Pixel (rauscht oder ist langsam), Licht-Ausbreitung auf dem Raster (blockig).

## Erste Ideen von mir (ungeprüft – bitte kritisch durchdenken, nicht übernehmen)

1. **Funktionen als Daten statt als Code-Pfade.** Der wichtigste Hebel. Alles wird in wenige Eingabe-Texturen der Bodenebene gemalt: Emission, Extinktion (farbig), Höhe. Zaun-Muster = Extinktions-Textur. Fenster/Gobo = farbige Extinktion vor der Lampe. Nebel und Rauch = geringe Extinktion plus Streuung. Glow, Flackern, Druckwellen-Ring = Emission, die der Host ändert. Dann gibt es **einen** Transport-Durchlauf statt 151 Programmen.
2. **Höhen in die Kaskaden nehmen („Horizont statt Durchlässigkeit“).** Idee: Pro Strahlabschnitt nicht nur speichern, wie viel Licht durchkommt, sondern auch den höchsten Horizontwinkel, den die Höhen entlang des Abschnitts erzeugen. Horizonte lassen sich wie Durchlässigkeit aus Teilstücken zusammensetzen (Maximum mit Versatz). Ein Licht in Höhe h ist von einem Bodenpunkt sichtbar, wenn sein Höhenwinkel über dem Horizont liegt. Damit kämen lange Schatten hoher Wände aus demselben Durchlauf. Offen: Wie viele Höhenstufen braucht man, damit Licht verschiedener Höhen stimmt? Reichen zwei bis drei Schichten?
3. **Zwei Wege statt vieler.** Ein Kaskaden-Durchlauf für „alles Viele und Weiche“ (viele Lampen, Flächen, indirektes Licht, Nebel) plus ein winziger Direkt-Weg für die wenigen wichtigen Lampen (z. B. höchstens 8–16), wo es scharf sein muss: Schatten-Karte pro Licht (1D polar), Spot-Kegel, Lampen-Profil als 1D-Winkel-Tabelle, Maske. Das deckt harte Schatten, Spots und IES mit wenig Code ab.
4. **Normal-Maps billig bedienen.** Aus der untersten Kaskade nicht nur die Lichtmenge, sondern auch eine mittlere Lichtrichtung behalten (Menge + 2D-Richtungsvektor, drei Kanäle mehr). Damit rechnet der Composite-Schritt Lambert und Glanz pro Pixel, ohne pro Lampe zu zeichnen.
5. **Schnell und sparsam.** In halber oder viertel Auflösung in der Bodenebene rechnen und kantenbewusst hochrechnen. Obere Kaskaden nur neu rechnen, wenn sich etwas in ihrem Einzugsbereich geändert hat (statische Szene und bewegte Lichter trennen). Kompakte Formate (RGBA16F oder R11G11B10). Kein Rauschen und keine Anhäufung über Frames – sonst leiden Determinismus und Bildtests.
6. **Budget als Ziel.** Vorschlag: Kern unter etwa 3 000 Zeilen Rust und unter etwa 600 Zeilen HLSL, Bildzeit unter der des heutigen Lichts auf denselben Szenen. Ob das realistisch ist, sollst du beurteilen.

## Was du liefern sollst

1. **Recherche gegenprüfen**, wo sie trägt oder wackelt: Lies die Primärquellen selbst (HRC-Paper, Sannikovs RC-Paper `github.com/Raikiri/RadianceCascadesPaper`, den „Bilinear Fix“), sieh dir ein bis zwei der Repos wirklich an (Umfang, Aufbau). Such gezielt nach Arbeiten zu Höhenfeldern/2.5D mit Kaskaden und nach Alternativen, die der erste Durchgang übersehen haben könnte.
2. **Entwurf:** Zwei bis höchstens drei durchdachte Varianten mit ehrlicher Abwägung (was jede von selbst kann, was sie kostet, wo sie scheitern kann). Dazu deine Empfehlung. Für jede Funktion aus dem Soll: wie sie im neuen Entwurf entsteht, oder dass und warum sie entfällt oder einfacher wird.
3. **Plan für einen kleinen Versuch**, der die riskanten Annahmen zuerst prüft – vor allem Höhen/Isometrie und harte Schatten. Mit den zwei bis drei Bildern und Messungen, die entscheiden. Vergleichsmaßstab sind die vorhandenen Chiaro-Referenzbilder (`tests/golden/chiaro/`); es muss nicht bitgleich sein, aber Manuel soll Vorher/Nachher nebeneinander sehen.
4. **ADR-Entwurf** (`docs/adr/0019-…`, Status „vorgeschlagen“) und ein Eintrag in `docs/plan.md` Abschnitt 17 (dort steht die Frage schon als „2D-Licht grundsätzlich neu denken“).

Den Versuch selbst baust du erst, wenn Manuel den Entwurf gesehen hat.

## Was du über das Projekt wissen musst

- Regeln: `CLAUDE.md` im Wurzelverzeichnis ist bindend (Determinismus, kein `unsafe` außerhalb von `knorke-gpu`, Module submitten nie selbst, „aus = kostenlos“, Leistungsaussagen nur mit Messung, Hardware und Datum).
- GPU: `crates/knorke-gpu` ist ein sicherer Wrapper um SDL3-GPU. Prüf zuerst, ob er Compute-Pässe anbietet; wenn nicht, plane mit Fragment-Shadern. Eigene Shader: HLSL über DXC (`D:/Dev-Tools/dxc/…`), Ablauf in `docs/chiaro-port/ABLAUF.md` unter „Eigene Shader“ und `cargo xtask shaders`.
- Ein kleiner Host zum Ausprobieren ist da: `knorke_integration::stealth_view` zeichnet die Wachmann-Szene (Draufsicht, Mauer, Lampe) und `knorke render` schreibt sie als PNG. Für Schräg-Ansicht mit Höhen sind die Chiaro-Szenen `iso_night_*` der Maßstab (`crates/knorke-integration/tests/gpu_iso_night.rs`, gemeinsamer Parser `tests/common/demo_scene.rs`).
- Messen: Vorbild sind die Zeitmessungen in `gpu_medium.rs` (`KNORKE_SCENE_BENCH`). Rechner: AMD Radeon RX 6700 XT, Vulkan. Ein bekannter wunder Punkt des heutigen Lichts: Lichtstreifen im Nebel sind in Knorke etwa ein Drittel langsamer als in Chiaro (Messwerte im Protokoll, 2026-11-25).
- Vorhandene Entscheidungen zum Licht: ADR 0002 (GPU-Lebenszyklus), 0014 (Schnitt des Chiaro-Ports), 0016, 0017 (Pass-Plan, noch „vorgeschlagen“).
- Ob das heutige Licht indirektes Licht hat, ist nicht geprüft – klär das, es entscheidet mit, was ein neues Verfahren zusätzlich bringt.

## Womit du Manuel am Ende antwortest

Kurz und in einfachen Worten: Was du empfiehlst, warum, was unsicher ist, und die höchstens zwei Wege, zwischen denen er wählen soll – mit deiner Empfehlung.
