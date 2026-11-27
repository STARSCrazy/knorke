# ADR 0019: 2D-Licht neu – eine Lichtzeile pro Lampe statt Strahl-Marsch pro Pixel

Status: angenommen 2026-11-27 (umgesetzt in Plan 17.13 L1–L7; Chiaros Schatten-Weg und das Distanzfeld sind gelöscht, siehe Plan Abschnitt 16) · Datum: 2026-11-25, Versuch bis 2026-11-26 · Bezug: ADR 0002, 0011, 0014, 0016, 0017, Plan 17.1 („2D-Licht grundsätzlich neu denken“), 17.11 Nr. 6, 17.13 · Auftrag: `docs/prompts/licht-neu-denken.md`

Kennzeichnung in diesem Dokument: **[belegt]** = im Code oder in der Primärquelle selbst gelesen, **[sekundär]** = nur aus zweiter Hand, **[Schätzung]** = überlegt, nicht gemessen.

## Kontext

`knorke-light2d` ist der bitgenaue Port von Chiaro: rund 29 000 Zeilen Rust (mit Tests) und rund 6 000 Zeilen HLSL. Manuel will ein kleines, schnelles Licht, das die Funktionen im Prinzip behält, ohne Festlegung auf Draufsicht oder Schrägansicht. Sichtbare Qualität darf minimal leiden, wenn es deutlich schneller wird; kein Qualitäts-Schalter.

### Wie das heutige Licht arbeitet [belegt, Code gelesen 2026-11-25]

- Alle Hindernisse werden einmal in eine **Maske in der Bodenebene** gezeichnet, dazu eine Höhen-Textur (Unterkante, Oberkante) und per Jump Flooding ein **Distanzfeld** (8 Vollbild-Pässe, über Frames zwischengespeichert).
- Jedes Licht ist ein Rechteck im Bild (Instanced Draw, höchstens 11 Draws für alle Lichter). **Pro Pixel und Licht** läuft im Fragment-Shader ein Strahl-Marsch durch das Distanzfeld vom Empfänger zur Lampe, höchstens 64 Schritte (`shadow_trace.hlsli`). Die Höhe des Strahls wächst linear; geblockt wird nur, wenn sie im Höhenintervall des Texels liegt. Daraus folgt die Schattenlänge. Weiche Schatten: engste Kegelöffnung entlang des Strahls (ein Strahl, kein Sampling).
- Wandflächen und Dächer liefert der Host als Surface-Puffer (Höhe + Normale pro Bildpixel); Lambert und Glanz werden pro Licht gerechnet.
- **Kein indirektes Licht.** Umgebungslicht ist eine Farbe, Ambient-Zonen sind Formen mit weicher Kante, die in den Lichtpuffer geblendet werden.
- Nebel: Strahl-Marsch pro Pixel und Licht (6–16 Proben), dazu pro Licht eine **polare Zeile** (512 Winkel), die die ersten zwei Hindernisse als (Entfernung, Oberkante) speichert (`medium_polar.frag.hlsl`).
- Die 151 Shader-Programme sind Permutationen: 92 Lichtprogramme = 3 Grundformen × 8 Merkmals-Bits (Höhe, Surface, Exact, Direction, Area, Maske, Muster, Medium).

**Befund:** Das Verfahren selbst ist schlank. Die Größe kommt aus der Breite: Permutationen, sieben Stores, vier Qualitätsstufen, zwei Schatten-Modi, inkrementelle Caches, Importer, Prüfungen und Warnungen für jede Kombination. Die Diagnose aus dem Auftrag stimmt.

### Was die Recherche ergab

- **Holographic Radiance Cascades** (Freeman, Sannikov, Margel, arXiv 2505.02041) [belegt, Paper gelesen]: eine flache 2D-Ebene mit Emission und Extinktion; Kosten nur von der Auflösung abhängig (512²: 1,85 ms, 1024²: 7,67 ms auf RTX 3080 Laptop, CUDA über LuisaCompute). Vier Durchläufe (einer pro Himmelsrichtung), pro Durchlauf log₂(N) Stufen hinauf und hinunter. Schwächen laut Paper: Schatten etwa 4 Probenabstände weicher, Muster-Fehler bei Lichtern kleiner als 8 Probenabstände, Schachbrett-Artefakte (Kreuz-Filter nötig), Lichter außerhalb des Gitters brauchen einen eigenen Weg, die Beschleunigungsstruktur belegt den meisten Speicher. Ausgabe: Lichtmenge in 4 Richtungen pro Punkt. **Höhen kommen nicht vor.**
- **Klassische Radiance Cascades** (Sannikov 2023; Osborne/Sannikov, arXiv 2408.14425) [belegt: Paper-Quelltext; Tutorials nur als Zusammenfassung]: üblich sind 4 Strahlen in Stufe 0, Faktor 4 im Winkel, 2 im Raum, Distanzfeld per Jump Flooding. Ringe um Lichter und Lecks behebt der „Bilinear Fix“ für etwa 4-fache Strahlzahl. Kleine Lampen werden unscharf („wie von einer Flächenlampe“). Veröffentlichte Zeiten: etwa 12–30 ms für Sannikovs 2D-Demo ohne Angabe der Auflösung; eine Bachelorarbeit misst 8,7 ms bei 4K auf RX 7900 XTX [sekundär].
- **Path of Exile 2** [belegt im Paper]: Kaskaden im Bildraum auf dem Tiefenpuffer, nicht in einer 2D-Ebene; ohne Lichter und Verdecker außerhalb des Bilds.
- **Höhen / 2.5D mit Kaskaden: nichts gefunden** – kein Paper, kein Repo, kein Shadertoy, kein Blogpost (gezielte Suche, 2026-11-25).
- **Fremde Umsetzungen** [belegt, vier Repos gelesen, nichts gebaut]: Licht-Teil jeweils rund 300–600 Zeilen Shader und 250–1 200 Zeilen Host. **Alle benutzen Compute-Shader**, alle ein Distanzfeld. Die einzige HRC-Umsetzung (Unity, als unfertig markiert) stammt nicht von den Autoren.
- **`knorke-gpu`** [belegt]: keine Compute-Pässe, keine Mips, kein Min-Blend. Vorhanden: Fragment-Pässe mit bis zu 4 Zielen, RGBA16F/R16F/R32F, Instancing, Storage-Buffer lesend, Readback.
- **Alternativen mit Höhen** [sekundär]: Strahl-Marsch über eine Höhenkarte pro Pixel und Licht (das macht Chiaro schon, mit Distanzfeld), Horizont-Karten für Höhenfelder (Microsoft Research), Linien-Sweep für Gelände. Schatten-Geometrie aus Kanten und polare Schatten-Karten (Godot, Unity) kennen keine Höhe.

### Messlage

Zeiten gibt es nur für winzige Szenen (64×64 bis 192×128: 0,2–0,7 ms Wand-Uhr inklusive Submit und Warten, Streuung ±20–25 %, RX 6700 XT, Vulkan, 2026-11-25). Für die Demo-Szenen (`iso_night_*`, intern 640×360, 36 Lichter) gibt es **keinen** Messwert. Ohne diese Grundlinie ist keine Aussage „schneller“ möglich.

## Varianten

### A – „Chiaro-Verfahren, schlank“

Verfahren bleibt (Maske, Distanzfeld, Strahl-Marsch pro Pixel und Licht). Neu geschrieben mit einem Lichtshader statt 92 Permutationen, ohne Qualitätsstufen, Funktionen als Daten.

- Kann: alles, was heute geht; Bilder nahe an den Referenzen.
- Kostet: etwa gleiche Bildzeit wie heute [Schätzung] – schneller wird es dadurch nicht. Umfang grob 6 000–8 000 Zeilen Rust, 1 500 HLSL [Schätzung].
- Scheitert kaum; bringt aber nur „kleiner“, nicht „schneller“.

### B – „Lichtzeilen“ (Empfehlung)

Die Sicht einer Lampe wird **einmal pro Lampe** berechnet und in einer Zeile gespeichert; jedes Pixel schaut nur noch nach. Einzelheiten unter „Entscheidung“.

- Kann: Punkt, Spot, Sonne, harte und weiche Schatten, Höhen (auch schwebende Hindernisse), Licht-Höhe, Nebel-Lichtstreifen, halbdurchsichtige Hindernisse – alles aus derselben Zeile.
- Kostet: siehe „Ergebnis von Schritt −1“ (gezählt, nicht mehr geschätzt). Kurz: **Der Gewinn kommt vom Merken.** Eine ruhende Lampe mit Schattenhöhen-Karte braucht etwa ein Achtel der heutigen Textur-Zugriffe; eine Lampe, die jedes Bild wandert, braucht etwa gleich viele oder mehr. Der erste Überschlag in diesem Entwurf (heutiges Verfahren mit 12 Schritten pro Pixel angenommen) war zu günstig für B – gezählt sind 6,6. Kein Distanzfeld, kein Jump Flooding. Umfang 3 000–4 500 Zeilen Rust (mit Tests), 600–800 Zeilen HLSL [Schätzung].
- Kann scheitern an: mehr Hindernissen pro Richtung, als die Zeile fasst; Treppchen und Flimmern durch die Winkel-Auflösung; weichen Schatten quer zur Lichtrichtung; Linien-/Flächenlichtern; schrägen Dächern. Siehe „Risiken“.

### C – „Kaskaden als Kern“ (HRC oder klassisch)

Ein Transport-Durchlauf für alles; Lichter sind gemalte Emission.

- Kann von selbst: Flächen- und Linienlicht, sehr viele Lichter, indirektes Licht, Nebel in der Draufsicht.
- Kann nicht: Höhen (keine Vorarbeit gefunden; die Idee „Horizont pro Strahlabschnitt“ braucht pro Abschnitt die obere Hülle des Höhenprofils und Lichter nach Höhe getrennt – ein Forschungsprojekt mit offenem Ausgang), harte Schatten kleiner Lampen, Lampen-Profile, Spots nur über Umwege.
- Kostet: bei 512² rund 2 ms auf vergleichbarer Hardware [belegt für HRC in CUDA; für eine Fragment-Fassung in `knorke-gpu` ungemessen]. Das ist vermutlich mehr als das heutige Licht in den Demo-Szenen braucht [Schätzung, Grundlinie fehlt]. Entweder `knorke-gpu` um Compute erweitern oder das Verfahren als erste Fragment-Fassung selbst herleiten.
- Urteil: Als Kern verfehlt C die Hälfte des Solls und müsste um einen Direkt-Weg wie B ergänzt werden. Indirektes Licht ist **nicht Teil des Solls** – das heutige Licht hat keines.

## Entscheidung

**Variante B als Kern. C nur als spätere, abschaltbare Zusatzstufe. A ist der Rückfall, wenn B im Versuch am Bild scheitert.**

### 1. Die Lichtzeile

Welt = Bodenebene mit Höhenfeld. Die **Bodenkarte** ist eine Textur in Bodenkoordinaten: Höhe pro Texel (frei, oder ein Hindernis bis „unendlich hoch“) und farbiger Durchlass für Hindernisse, die nicht ganz dicht sind (1 = Licht geht ganz durch, kleiner = Zaun, Gitter, farbiges Glas).

Die Bodenkarte trägt pro Texel ein Höhenintervall (Unterkante `base`, Oberkante `top`) wie heute; `base = 0` heißt „steht auf dem Boden“.

Ein Lichtstrahl von der Lampe (Höhe `zL`) zu einem Empfänger in Bodenentfernung `r` und Höhe `z` hat das **Gefälle** `m = (zL − z) / r`. Entlang einer Richtung liegt ein Hindernis als **Lauf** von der nahen Kante `rn` bis zur fernen Kante `rf` mit gleicher Höhe. Die Höhe des Strahls in Entfernung `q` ist `zL − m·q`.

**Regel:** Ein Lauf blockt genau dann, wenn die Höhen, die der Strahl über dem Stück des Laufs **vor dem Empfänger** durchläuft (von `rn` bis `min(rf, r)`), das Höhenintervall `[base, top]` treffen. Weil die Strahlhöhe linear ist, genügen die beiden Enden des Stücks:

- fallender Strahl (`m ≥ 0`, Empfänger tiefer als die Lampe): geblockt, wenn `(zL − top) / rf' ≤ m ≤ (zL − base) / rn`, mit `rf' = min(rf, r)`;
- steigender Strahl (`m < 0`, Empfänger höher als die Lampe, z. B. Fassade über einer niedrigen Laterne): die Kanten tauschen die Rollen, geblockt, wenn `(zL − top) / rn ≤ m ≤ (zL − base) / rf'`.

Jeder Lauf sperrt also ein **Gefälle-Intervall**. Für Hindernisse auf dem Boden ist die Grenze mit `base` ohne Wirkung. Ausgenommen ist nur die Fläche, die den Empfänger selbst trägt: Berührt der Strahl das Intervall erst am Empfänger (Dachpixel mit `z` an der Oberkante, mit kleiner Toleranz wie heute), blockt der Lauf nicht. Ein Empfänger am Boden **unter** einem schwebenden Dach wird dagegen von diesem Dach verschattet, wenn die Lampe darüber steht – deshalb zählt der Lauf bis zum Empfänger mit und wird nicht übersprungen. Die Regel deckt ab: Hindernisse niedriger als die Lampe (endlich langer Schatten ab der fernen Kante), höhere, schwebende (Dach über einer Lampe, Balken), Empfänger am Boden, an Wänden und auf Dächern.

Wichtig (Fund des Reviews): Gespeichert wird **ein Eintrag pro Lauf**, nicht pro Texel. Über einem flachen Dach fällt die Schwelle sonst mit jedem Texel weiter, und die Zeile liefe sofort voll.

- **Aufbau:** pro Lampe eine Zeile, ein Texel pro Winkel. Ein Fragment-Pass marschiert je Winkel einmal nach außen über die Bodenkarte bis zum Lichtradius und schreibt bis zu K = 4 Läufe als (`rn`, `rf`, `base`, `top`) in vier RGBA16F-Ziele; die Grenzen rechnet das Nachschauen daraus. Ein Lauf entfällt nur, wenn ein näherer, am Boden stehender Lauf für fallende **und** steigende Strahlen schon alles sperrt, was er sperren würde (zwei Vergleiche). Ein unendlich hoher Lauf beendet den Marsch. Laufen mehr als K Läufe auf, wird der letzte Platz so gefüllt, dass er eher zu viel verdeckt als Licht durchlässt (wie Chiaros Polar-Zeile). Halbdurchsichtige Läufe tragen ihren Durchlass statt voller Sperre (Kodierung legt der Versuch fest). Winkelzahl nach Lichtradius (512 / 1024 / 2048), sodass ein Winkelschritt am Rand höchstens etwa einen Bodentexel breit ist. **Umgesetzt 2026-11-27 (Plan 17.13 L7):** 1024 Winkel bis 600 Masken-Texel Reichweite, 2048 Winkel (Abstände in halben Texeln) bis 2047; 512 nicht – eine Zeile mit 1024 Winkeln ist schon ein halber Atlas-Streifen, die Ersparnis wäre klein, und weniger Winkel verlieren dünne Masten. Darüber zeichnet das Licht ohne Schatten mit der Warnung `knorke.light2d.warning.unshadowed_lights`. Lampe **in** einem Hindernis (ihre Höhe liegt im Intervall des eigenen Texels): dieser Lauf wird übersprungen (wie heute E147); eine Lampe unter einem schwebenden Dach steckt nicht darin, das Dach zählt. Lampe außerhalb der Karte: der Marsch beginnt, wo der Strahl die Karte betritt.
- **Nachschauen:** Der Lichtshader rechnet Winkel, `r` und `m` des Pixels, liest die Zeile an zwei Nachbarwinkeln und wendet die Regel oben auf jeden Lauf an, der vor dem Empfänger beginnt. Fassaden starten wie heute einen Texel vor ihrer Wand, damit sie sich nicht selbst verschatten.
- **Weiche Schatten:** in Strahlrichtung analytisch (Lampengröße ρ ergibt an jeder Intervallgrenze eine Rampe der Breite `ρ·(r − rk)/rk`, `rk` = die Kante der Grenze), quer dazu wenige feste Proben über Nachbarwinkel mit derselben Breite. Kein Rauschen.
- **Sonne/Mond:** dieselbe Zeile mit parallelen Strahlen (ein Texel pro Versatz quer zur Lichtrichtung; die Grenzen der Läufe sind dann Höhen statt Gefälle).
- **Cache:** Eine Zeile bleibt gültig, solange sich weder die Lampe noch die Bodenkarte in ihrem Radius ändert. Wiederverwenden liefert dieselben Werte wie Neuberechnen – Determinismus und Bildtests bleiben unberührt.
- **Ruhende und bewegte Hindernisse trennen** (Vorschlag Manuel, 2026-11-25): zwei Zeilen pro Lampe. Die **feste Zeile** entsteht aus den ruhenden Hindernissen und wird nur neu gebaut, wenn die Lampe wandert oder die Umgebung in ihrem Radius zerstört oder umgebaut wird. Die **bewegte Zeile** enthält nur bewegliche Hindernisse (Figuren, Türen, Kisten) und wird pro Frame neu gebaut; das sind wenige Formen, sie lassen sich direkt einzeichnen statt zu marschieren. Das Nachschauen prüft beide. Eine laufende Figur im Lichtkreis macht dann die feste Zeile nicht ungültig. Flackern ändert nur die Helligkeit und kostet keinen Neubau.
- **Warum K klein bleibt:** K ist die Zahl der Hindernisse **hintereinander in einer Richtung**, nicht die Zahl der Hindernisse um die Lampe (die ist unbegrenzt). Die Grenze kommt vom Speicherplatz pro Richtung und vor allem vom Nachschauen: jeder Platz kostet jedes Pixel einen Textur-Zugriff mehr, in jedem Frame. Der Aufbau der festen Zeile ist dagegen selten und dürfte teuer sein. K ist eine Stellgröße (4 oder 8), kein Naturgesetz; Schritt −1 zählt, was echte Szenen brauchen.

**Idee für einen noch billigeren Aufbau (ungeprüft): Schatten malen statt marschieren.** Hindernisse, die als Form vorliegen (Kasten, Kreis, zusammengefasste Kacheln), lassen sich pro Lampe direkt in deren Karte zeichnen: ein Instanced Draw über alle Paare (Lampe, Hindernis), jedes Paar füllt seinen Winkelbereich hinter der fernen Kante mit seiner Schwelle (Max-Blend). Kanten wären dann exakt, dünne Masten gingen nie verloren, und es gäbe keinen Marsch. Preis: eine Karte Winkel × Entfernung statt einer Zeile, schwebende Hindernisse nur genähert, und Hindernisse aus Sprite-Masken bräuchten weiter den Marsch. Schritt −1 des Versuchs zählt beide Wege.

Herkunft: Chiaros Nebel benutzt schon eine polare Zeile mit zwei Hindernissen; polare Schatten-Karten ohne Höhe sind üblich (Godot); Horizont-Karten sind seit Max 1988 bekannt. Neu ist die Verbindung: gesperrte Gefälle-Intervalle pro Lauf als einzige Sichtbarkeits-Struktur für Boden, Wände und Nebel, mit beiden Kanten und Unterkante statt nur „Entfernung und Oberkante“. Eine veröffentlichte Fassung genau dieser Form haben wir nicht gefunden [Schätzung: naheliegend, aber nicht belegt].

### 2. Funktionen als Daten, ein Weg

- **Eine** Bodenkarte, **ein** Zeilen-Aufbau, **ein** Lichtshader (Verzweigung über Daten der Lampe, keine Permutationen), **ein** Composite.
- **Eine Lampensorte mit Parametern** (Vorschlag Manuel, 2026-11-25) statt fünf Arten mit eigenen Wegen: Ort und Höhe, Reichweite und Abfall, Kegel (360° = Punktlicht, enger = Spot), Ausdehnung des Leuchtkörpers (0 = Punkt, Länge = Linie, Länge × Breite = Fläche), Lampengröße für weiche Schatten, „unendlich weit weg“ (= Sonne), dazu optional Profil und Maske. Die Karte wird für jede Lampe gleich gebaut; die Parameter wirken nur auf Helligkeit und Kegel, bei ausgedehnten Leuchtkörpern auf die Zahl der Stützpunkte.
- Keine Qualitätsstufen, kein zweiter Schatten-Modus, kein Distanzfeld, kein inkrementeller Masken-Cache.
- Lichtpass: ein Instanced Draw (ein Rechteck pro Lampe), additiv in einen RGBA16F-Lichtpuffer; Lambert und Glanz pro Licht wie heute (die Formeln aus `surface_light.hlsli` bleiben, Herkunft nach zlib kennzeichnen).
- Regeln aus ADR 0002, 0011, 0016 und 0017 gelten weiter: kein Submit im Modul, GPU-Zustand ist Cache ohne Snapshot, Stores beim Host, aus = kostenlos.

### 2a. Tricks, die die Arbeit pro Pixel klein halten (Nachfrage Manuel, 2026-11-25)

Nach Wirkung geordnet. Die ersten zwei gehören in den Versuch, die anderen in eine zweite Stufe.

1. **Schattenhöhe statt Liste – ein Blick statt Suche.** Aus der Zeile entsteht pro Lampe eine kleine Karte (Winkel × Entfernung) mit **einer Zahl pro Stelle: der Höhe `s`, unter der dort Schatten ist.** `s` ist das Maximum der Schattenlinien aller Hindernisse davor: über die ferne Oberkante, wenn das Hindernis niedriger ist als die Lampe (`zL + (top − zL)·r/rf`), über die nahe Oberkante, wenn es höher ist (`zL + (top − zL)·r/rn`). Ein Pixel ist beleuchtet, wenn seine Höhe über `s` liegt – für Boden, Wände, Dächer, fallende und steigende Strahlen gleich. Das ist **ein** Textur-Zugriff mit einem Vergleich statt 8 bis 16 Zugriffen in die Liste. Zwischen zwei Hindernis-Kanten ist `s` entlang der Entfernung eine Gerade, die lineare Filterung der Grafikkarte trifft sie dort genau; die Karte darf deshalb in der Entfernung grob sein. Schwebende Hindernisse bekommen zwei weitere Kanäle (Ober- und Unterseite ihres Schattens, als Hülle – bei zwei schwebenden Hindernissen hintereinander zu viel Schatten). Nachgerechnet [belegt, 2026-11-25]: 29 185 Zufallsfälle mit Hindernissen auf dem Boden gegen dichtes Abtasten, keine Abweichung (Wegwerf-Skript). Preis: Speicher (rund 0,5–1 MB pro Lampe bei 1 024 × 64 bis 128 Stellen) und ein Umrechnungs-Pass, wenn sich die Zeile ändert. Für eine wandernde Lampe mit kleinem Bildanteil kann die Umrechnung teurer sein als die gesparten Blicke – der Versuch zählt, ab wann sich die Karte lohnt.
2. **Enge Formen statt Rechtecke.** Spot als Keil, Punktlicht als Achteck, Abbruch am Radius: Pixel, die die Lampe nie erreicht, starten den Shader gar nicht.
3. **Fertiges Licht ruhender Lampen merken, nur Geändertes neu rechnen.** Der Lichtpuffer bleibt stehen. Neu gerechnet werden nur Bereiche, in denen sich etwas ändert: wo eine Figur steht oder stand, wo ihr Schatten fällt oder fiel, der Lichtkreis einer wandernden oder flackernden Lampe, der neu sichtbare Rand beim Schwenk um ganze Pixel. Das ist kein Mischen über Frames: ein neu gerechneter Bereich hat dieselben Werte wie ein voll gerechnetes Bild – prüfbar mit einem Test „schrittweise = voll, bitgleich“ (so macht es Chiaro schon für die Maske). Ruhende Lampen kosten dann fast nichts. Kein Gewinn, wenn sich alles bewegt oder die Kamera in Bruchteilen von Pixeln gleitet. Preis: Buchführung über geänderte Bereiche und das Risiko veralteter Pixel. Der Trick hilft **auch dem heutigen Verfahren** (Variante A).
   **Genauere Fassung (Vorschlag Manuel, 2026-11-25): Grundlicht plus Korrektur.** Zwei Puffer. Das **Grundlicht** ist das fertige Licht aller ruhenden Lampen ohne bewegliche Hindernisse; es bleibt stehen. Das **Bildlicht** entsteht pro Frame als Kopie davon plus Korrekturen: (a) wandernde und flackernde Lampen werden dazugezählt; (b) für jedes Paar aus ruhender Lampe und beweglichem Hindernis wird nur der Schattenkeil hinter dem Hindernis gezeichnet und dort der Anteil dieser einen Lampe wieder abgezogen (ihr Licht dort kostet dank Schattenhöhen-Karte einen Blick); (c) die Pixel der bewegten Figuren selbst werden frisch beleuchtet. Anders als Unitys „Subtractive“-Modus, der pauschal abdunkelt, ist das Abziehen hier exakt bis auf Rundung, weil der Anteil jeder Lampe einzeln nachrechenbar ist. Schlechtester Fall (Kamera gleitet in Bruchteilen von Pixeln, alles bewegt sich): das Grundlicht wird jedes Bild neu gerechnet – Kosten wie ohne diesen Trick, nicht schlechter. So arbeiten auch große Engines [Vorwissen, 2026-11-25 nicht nachgeschlagen]: Unreal trennt Lampen in statisch / ortsfest / beweglich und merkt sich Schatten ruhender Geometrie (bei den Virtual Shadow Maps seitenweise); Unity kennt gebackene Lichtkarten mit den Mischarten „Subtractive“ und „Shadowmask“; viele Engines lassen kleine bewegte Lichter (Mündungsfeuer, Geschosse) gar keine Schatten werfen.
4. **Wandernde Lampen billig halten.** (a) Lampen ohne Schatten (Leuchtgeschosse, Funken; das Feld `cast_shadows` gibt es schon) brauchen keine Zeile und keine Karte. (b) Ein Spot baut nur die Winkel seines Kegels: ein Scheinwerfer mit 60° kostet ein Sechstel. (c) Gröbere Karte für wandernde Lampen – ihre Schattenkanten bewegen sich ohnehin. Alles ungemessen.
5. **Pixeldaten einmal lesen, Lampen in einer Schleife.** Statt ein Rechteck pro Lampe zu zeichnen (jedes liest Höhe, Normale und Material neu und mischt in den Puffer), ein Durchlauf über das Bild mit einer Lampen-Liste pro Kachel. Spart pro Pixel und Lampe etwa drei Zugriffe und einen Schreibvorgang [Schätzung]. Preis: Kachel-Listen auf der CPU.
6. **Licht in halber Auflösung** – möglich (Chiaro hat das als Qualitätsstufe), kostet Schärfe an Schattenkanten. Erst, wenn der Rest nicht reicht.

### 3. Abgleich mit dem Soll

| Funktion | Entsteht im Entwurf B durch | Urteil |
|---|---|---|
| Punkt-, Spotlicht | Lichtzeile + Abfall/Kegel aus Lampendaten | bleibt |
| Richtungslicht | Parallel-Zeilen, sonst gleicher Weg | bleibt |
| Linien-, Flächenlicht | Helligkeit analytisch wie heute; Sicht aus 2–4 Zeilen an Stützpunkten des Emitters, dazwischen gemischt | einfacher; Schattenform nah an der Lampe anders – **Risiko** |
| Harte Schatten | Vergleich des Gefälles mit den gesperrten Intervallen | bleibt, Schärfe = Winkel-Auflösung |
| Weiche Schatten (Lampengröße) | Rampe + feste Proben quer | bleibt, anderes Verfahren – **Risiko** |
| Höhen, Schattenlänge, Licht-Höhe, Isometrie | stecken in der Formel; Projektion nur beim Umrechnen Pixel → Welt (Surface-Puffer wie heute) | bleibt |
| Schwebende Hindernisse (Unterkante > 0) | obere Grenze des Gefälle-Intervalls; nötig, weil die Demos sie benutzen (`iso_night_hard`: drei Balken mit Unterkante 2,5 [belegt: `tests/golden/chiaro/iso_night/hard.txt` Zeilen 90–92]; das Dach in Zeile 71 mit Unterkante 7,7 ist in dieser Variante abgeschaltet (Flag 2) – eine frühere Fassung dieses ADR nannte es fälschlich als aktiv) | bleibt; kostet Platz in der Zeile – **Risiko** |
| Schräge Dächer, Rampen (Höhe ändert sich stetig) | jeder Texel wäre ein eigener Lauf | **nur als Stufen**; echte Schrägen sprengen K |
| Normal-/Material-Maps, Glanz | pro Licht im Lichtshader | bleibt |
| Glow, Umgebungslicht | Composite | bleibt |
| Ambient-Zonen | Host malt eine Ambient-Ebene (Textur) | einfacher; Zonen-Formen als Code entfallen |
| Masken/Projektoren | Masken-Textur bleibt als Daten, ein Lookup im Lichtshader; farbiges Glas alternativ als Durchlass in der Bodenkarte | einfacher; Fenster-Sonderfall prüfen |
| Lampen-Profile (IES) | Tabelle bleibt als Daten, ein Lookup; der Importer (heute 965 Zeilen) wird Werkzeug außerhalb des Kerns | bleibt, Kern kleiner |
| Animation/Flackern | Host ändert die Helligkeit; eine kleine Hilfsfunktion aus `Seed` und `Tick` | einfacher |
| Halbdurchsichtige Hindernisse, Muster | Zaun/Gitter wird als Durchlass in die Bodenkarte gemalt; die Zeile trägt das Produkt | einfacher; Feinheit = Auflösung der Bodenkarte – **Risiko** bei feinen Gittern |
| Nebel/Rauch, Lichtstreifen | Marsch entlang des Sehstrahls wie heute, aber jede Probe ist ein Blick in dieselbe Lichtzeile; eigener Polar-Atlas und Sonnen-Höhen-Atlas entfallen | bleibt, weniger Proben |
| Druckwellen-Lichtring | Lampe mit Ring-Abfall (innerer Radius als Datum) | einfacher |
| Deterministisch, linear, Post danach | unverändert | bleibt |
| Indirektes Licht | – | gab es nie; siehe Punkt 4 |

### 4. Kaskaden als mögliche Zusatzstufe (nicht Teil dieses Entwurfs)

Wenn später leuchtende Flächen (die Glow-Ebene) die Szene wirklich beleuchten oder Licht von Wänden zurückgeworfen werden soll, kann ein flacher HRC-Durchlauf in halber Auflösung dazukommen: Emission = Glow-Ebene, Extinktion = Hindernisse höher als ein Schwellwert. Aus-Zustand ohne Kosten (ADR 0011). Entscheidung erst, wenn B steht und jemand die Funktion will.

### 5. Chiaro-Licht bleibt

Bis Bild und Messung den Ersatz belegen, wird an `knorke-light2d` nichts gelöscht und nichts gekürzt. Der Versuch ist Wegwerf-Code neben dem Bestand.

**Nachtrag 2026-11-27 (Plan 17.13 L7 Teil 2):** Belegt durch die Probe gegen das Wahrheitsbild in allen Fällen (L1–L7) – gelöscht. Chiaros Schatten-Lichtprogramme, das Distanzfeld (Jump Flooding, Feld-Ziele, Klassen-Felder, Feld-Rechtecke), Chiaros Medium-Programme `medium_polar`/`medium_sun_height` und die Einstellung `light_rows` gibt es nicht mehr: jedes Licht nimmt seinen Schatten aus einer Zeile, jede Sonne aus einer Karte. Wo das Bild von Chiaro abweicht, gelten neue Referenzbilder unter `tests/golden/rows/`; die Chiaro-Bilder bleiben als Vorher-Vergleich (Konsequenzen unten).

## Risiken

1. **Winkel-Raster:** dünne Pfeiler weit weg von der Lampe können zwischen zwei Strahlen fallen (die Laternenmasten in `iso_night` haben Radius 0,12 – wenige Texel); Schattenkanten können beim Bewegen der Lampe flimmern. Gegenmittel: Winkelzahl nach Radius, halbe Schritte, lineares Lesen der Höhe.
2. **Lecks an dünnen Wänden:** eine 1 Texel dicke schräge Wand kann beim Marschieren übersprungen werden.
3. **Mehr als 4 Läufe** entlang eines Strahls, die sich nicht gegenseitig überflüssig machen (schwebende Hindernisse zählen immer; schräge Dächer zerfallen in viele Läufe): dann wird zu viel verdeckt. Wie oft das in echten Szenen vorkommt, ist **nicht gezählt** – das ist die erste Frage des Versuchs. Es gibt keinen billigen Rückfall innerhalb von B: eine Karte Winkel × Entfernung kann Unterkanten nur genähert, eine echte Schatten-Karte (Winkel × Gefälle) kostet im Aufbau ein Vielfaches. Läuft die Zeile in echten Szenen oft über, endet B, und es gilt Variante A.
9. **Die Regel hat viele Sonderfälle** (steigende Strahlen, Empfänger unter/auf einem Dach, Lampe im Hindernis). Zwei davon waren in der ersten Fassung falsch und sind erst im Review aufgefallen. Die Regel in der jetzigen Form ist nachgerechnet [belegt, 2026-11-25]: 59 197 Zufallsfälle (ein bis fünf Läufe, teils schwebend; Empfänger am Boden, in Wandhöhe, auf und unter Dächern; fallende und steigende Strahlen) gegen dichtes Abtasten des Strahls – keine Abweichung (ein gemeldeter Fall war ein Streifschuss, den die grobe Abtastung verpasst hatte; mit feiner Abtastung stimmt er). Wegwerf-Skript, nicht im Repo. **Nicht** geprüft sind damit: die Verdrängungsregel, der Überlauf bei mehr als K Läufen, das Winkel-Raster und weiche Schatten. Schritt −1 prüft das auf der echten Szene, bevor ein Shader entsteht.
4. **Weiche Schatten quer zur Lichtrichtung** brauchen Proben; ob 4 reichen, zeigt das Bild.
5. **Linien-/Flächenlichter:** ein Punkt-Mittelpunkt passt nicht; Stützpunkt-Zeilen können Doppelschatten zeigen.
6. **Vier Ziele pro Winkel** sind die Obergrenze von `knorke-gpu`; farbiger Durchlass braucht dann einen zweiten Pass oder eine engere Kodierung.
7. **Tempo ist nur als Zahl der Textur-Zugriffe gezählt, nicht als Zeit gemessen.** Für wandernde Lampen ist B nach dieser Zählung nicht schneller als heute (siehe „Ergebnis von Schritt −1“). Ob Zugriffe und Zeit auf der Grafikkarte gleich laufen, zeigt erst die Messung.
8. Budget-Zahlen sind Schätzungen; das heutige Licht hat seine Größe auch erst durch viele Sonderfälle bekommen.

## Versuch (erst nach Manuels Zustimmung bauen)

Wegwerf-Code: ein GPU-Test `crates/knorke-integration/tests/gpu_lightrow_probe.rs` und zwei bis drei eigene Fragment-Shader unter `crates/knorke-integration/shaders/src/` (Ablauf „Eigene Shader“, DXC). Keine neue Crate. Er benutzt den Szenen-Parser `tests/common/demo_scene.rs`, die vorhandene Hindernis-Rasterung (Höhen-Textur) und das vorhandene Composite; ersetzt wird nur die Lichtstufe. Vergleichsbild ist das heutige Licht **mit derselben Szene**, eingeschränkt auf Punkt- und Spotlichter (ohne Profil, Maske, Muster), damit beide Seiten dasselbe zeigen. Ziel-Umfang: etwa 500 Zeilen Rust, 250 Zeilen HLSL.

Die Bodenkarte hat im Versuch die Auflösung der heutigen Maske (ein Texel = ein Szenenpixel am Boden), damit beide Seiten gleich fein rechnen.

Reihenfolge – das Riskanteste und Billigste zuerst:

−1. **Zählen auf der CPU, ohne GPU-Code** (Vorschlag des Reviews): ein kleines Programm baut für die Lampen von `iso_night_hard` die Zeilen auf der CPU und vergleicht sie mit einem exakten Strahl-Marsch. Ergebnis: Läufe pro Winkel (Verteilung, Anteil über 4), Anteil falsch entschiedener Bildpixel, Zugriffe für den Aufbau pro Lampe neben „Bildpixel der Lampe × gemessene Schritte“ des heutigen Verfahrens. **Abbruch von B schon hier**, wenn mehr als 1 % der beleuchteten Pixel falsch sind oder der Aufbau einer Lampe im Mittel mehr Zugriffe braucht als ihr heutiger Strahl-Marsch.
0. **Grundlinie messen:** heutiges Licht auf `iso_night` (640×360, eingeschränkte Lichter) und auf einer Last-Szene (dieselbe Szene bei 1920×1080 intern, Lichter vervierfacht). Wand-Uhr wie `KNORKE_SCENE_BENCH`, Median aus 2 000 Frames, mindestens 5 Läufe je Seite im Wechsel, Rechner ruhig (keine anderen Agenten am Bauen).
1. **Bild 1 – Höhen und Schrägansicht, harte Schatten:** `iso_night_hard` (eingeschränkt). Heute | neu | Differenz ×8 nebeneinander. Prüft Schattenlängen, Wandflächen, Selbstverschattung.
2. **Bild 2 – weiche Schatten:** `iso_night_plain` (eingeschränkt) und `gate_sprite_shadow`, gleiche Darstellung.
3. **Bild 3 – Draufsicht mit dünnen Wänden und bewegter Lampe:** die Wachmann-Szene (`stealth_view`), Lampe wandert 32 Frames um je ¼ Pixel. Messwert: größte Änderung eines Pixels von Frame zu Frame, heute gegen neu (Flimmern), und Lecks durch 1-Texel-Wände ja/nein.
4. **Messung:** dieselben Szenen wie in Schritt 0 mit der neuen Lichtstufe; zusätzlich einmal alle Lampen unbewegt (Cache greift) und einmal alle bewegt.

Entscheidung danach:

- **Weiter mit B**, wenn Manuel in Bild 1–3 keinen störenden Unterschied sieht, keine Lecks auftreten, das Flimmern höchstens das 1,5-Fache des heutigen Werts beträgt und die Last-Szene im Median mindestens 30 % schneller ist (Abstand größer als die Streuung beider Seiten), die kleine Szene nicht langsamer.
- **Nachbessern** (mehr Winkel, feinere Schritte, K anders verteilt), wenn nur Risiko 1–3 zuschlägt und das Tempo noch reicht.
- **Rückfall auf A**, wenn das Bild nicht zu retten oder B nicht schneller ist.

Nicht im ersten Versuch und deshalb auf beiden Seiten ausgeblendet: Masken (`window_hall`), Profile, Muster, Flächenlichter, Nebel. Sie fehlen damit im Vergleichsbild – das Bild zeigt nicht die ganze Demo.

Zweite Runde nur bei Erfolg: Nebel-Lichtstreifen über dieselbe Zeile (Szene `shafts`, dort ist Knorke heute etwa ein Drittel langsamer als Chiaro), Zaun als Durchlass, Sonne als Parallel-Zeilen, Linienlicht, Maske.

### Ergebnis von Schritt −1 (2026-11-25, gezählt auf der CPU)

Wegwerf-Skript in Python (nicht im Repo), eigene Rasterung der 33 Hindernisse von `iso_night_hard` in eine Bodenkarte 960×680 (Maske mit Rand wie heute), 13 Punkt- und Spotlichter mit Schatten, Empfänger = jeder dritte Bodentexel im Bild (54 205 Stück), Vergleich gegen einen exakten Marsch in halben Texel-Schritten. Das heutige Verfahren ist nachgebaut (Distanzfeld bis 32 Texel, Schrittregel aus `shadow_trace.hlsli`, harte Schatten) und nur gezählt.

| Frage | Ergebnis |
|---|---|
| Läufe pro Richtung nach Verdrängung (23 552 Richtungen) | 0: 19 %, 1: 69 %, 2: 10 %, 3: 1,5 %, 4: 0,3 %, mehr als 4: **0,11 %** (höchstens 7) |
| Falsch entschiedene Bodenpixel durch die Verdrängungsregel | 0 |
| … zusätzlich durch das Winkel-Raster (512/1 024/2 048 nach Radius, nächster Winkel) | **0,24 %** (Schattenkanten) |
| … zusätzlich durch K = 4 | keine weiteren |
| … mit der Schattenhöhen-Form (Hülle für schwebende Hindernisse) statt der Liste | ebenfalls 0,24 % – die Hülle kostet hier nichts |
| Schritte des heutigen Strahl-Marschs pro Pixel und Lampe | **6,6** im Mittel, dazu 0,9 Höhen-Abfragen |
| Textur-Zugriffe pro Bild, heutiges Verfahren (nur diese 13 Lampen, nur Bodenpixel) | rund 5,6 Mio. |
| Aufbau aller 13 Zeilen (halbe Schritte; mit ganzen Schritten die Hälfte) | rund 6,5 Mio. – nur für Lampen, die sich bewegt haben |
| Nachschauen in der Liste (8 Zugriffe pro Pixel und Lampe) | rund 5,2 Mio. |
| Nachschauen in der Schattenhöhen-Karte (1 Zugriff; 2 mit Mischen zweier Winkel) | rund 0,7 Mio. (1,3 Mio.) |

Was das heißt:

- **K = 4 reicht** in dieser Szene, und die Genauigkeit liegt unter der Abbruchgrenze von 1 %.
- **Die Liste allein bringt kein Tempo.** Ruhend ist sie etwa so teuer wie heute, bewegt etwa doppelt so teuer. Als Nachschau-Struktur scheidet sie aus; sie bleibt nur Zwischenschritt für die Karte.
- **Mit der Schattenhöhen-Karte** (Abschnitt 2a, Trick 1) braucht eine ruhende Lampe etwa ein Viertel bis ein Achtel der heutigen Zugriffe. Das gilt auch bei Kamerafahrt und laufenden Figuren, weil die Karte an der Lampe hängt, nicht am Bild.
- **Eine Lampe, die jedes Bild wandert, wird nicht billiger:** Aufbau (0,25–0,5 Mio. Zugriffe) plus Umrechnung in die Karte (rund 0,26 Mio.) liegen über dem heutigen Marsch dieser Lampe (rund 0,43 Mio.). Das zweite Abbruchkriterium von Schritt −1 ist damit **knapp gerissen** (mit halben Schritten) bzw. knapp gehalten (mit ganzen). Gegenmittel zum Messen: gröbere Karte für wandernde Lampen, ganze Schritte, Schatten malen statt marschieren.
- Nicht gezählt: Wand- und Dachpixel, weiche Schatten, Flächenlichter, die Kosten des Distanzfelds im heutigen Verfahren (Jump Flooding, wenn sich Hindernisse bewegen), und alles, was nur die Grafikkarte zeigt (Sprünge, Schleifen, Schreibvorgänge).

**Folge für die Empfehlung:** B bleibt die Empfehlung, aber nur in der Form „Zeile → Schattenhöhen-Karte, pro Lampe gemerkt“. Der Nutzen hängt am Anteil ruhender Lampen. Für Spiele mit überwiegend wandernden Lampen ist Variante A (heutiges Verfahren, schlank) gleich schnell und sicherer.

### Ergebnis des GPU-Versuchs (2026-11-25, Schritte 0, 1 und 4)

Wegwerf-Code in einem Worktree, nicht committet: `.claude/worktrees/agent-a6a6d2cd427a8f7f6` (Branch `worktree-agent-a6a6d2cd427a8f7f6`), Test `crates/knorke-integration/tests/gpu_lightrow_probe.rs` (713 Zeilen) und fünf Shader `lightrow*` (444 Zeilen HLSL). Am Bestand ist nur `shaders/compiled/sources.sum` geändert. Gebaut von einem Subagenten, vom Hauptagenten nachgeprüft (Bilder angesehen, Test selbst neu gelaufen).

Aufbau: `iso_night_hard`, 640×360, auf beiden Seiten gleich eingeschränkt auf die 13 Punkt- und Spotlichter mit Schatten, harte Schatten, ohne Profil, Maske, Muster, Flackern, Nebel, Flächenlichter, Sonne, Glanz. Heute = `Light2dRenderer` unverändert. Neu = vorhandene Hindernis-Rasterung → Zeile bauen → Schattenhöhen-Karte (1 024 Winkel × 128 Entfernungen) → ein Lichtshader für Punkt und Spot → vorhandenes Composite und Post. Last-Szene = dieselben Lichter vervierfacht (52).

**Zeit** (AMD Radeon RX 6700 XT, Vulkan, Release; Wand-Uhr pro Frame mit Submit und Warten; Median in µs, Spanne der Läufe in Klammern; heute und neu im Wechsel). Nachprüf-Lauf des Hauptagenten, 3 Läufe × 500 Frames, Rechner ruhig:

| Szene | heute | neu, Lampen ruhen | neu, alle Lampen jedes Bild neu gebaut | ohne Lampen (Sockel) |
|---|---|---|---|---|
| normal (13 Lampen) | 380 (372–386) | **220** (213–226) | 360 (359–369) | 187 |
| Last (52 Lampen) | 1 118 (1 018–1 121) | **402** (390–418) | 773 (754–780) | 322 |

Die zwei Sitzungen des Subagenten (5 Läufe × 2 000 Frames, Rechner unruhig, p95 beim 2- bis 6-Fachen des Medians): Last-Szene gleich (heute 1 009–1 093, ruhend 426–435, neu gebaut 773–778); normale Szene dort ohne belastbaren Unterschied (heute 513–535, ruhend 457–467, neu gebaut 500–527, Sockel 414–422, Läufe überlappen).

- **Ruhende Lampen:** das ganze Bild ist 42 % (normal, ruhiger Lauf) bis 64 % (Last) schneller. Zieht man den Sockel ab, kostet das Licht selbst etwa ein Sechstel bis ein Zehntel von heute – das passt zur Zählung aus Schritt −1.
- **Alle Lampen jedes Bild neu gebaut:** normale Szene gleich schnell wie heute (−5 %, im Rahmen der Streuung), Last-Szene 31 % schneller. Der Versuch baut dabei alle Winkel aller Lampen, ohne Kegel-Zuschnitt und ohne gröbere Karte.
- Mit Masken-Auflösung 1,0 statt der Demo-Einstellung 0,5 (eine Sitzung des Subagenten): Last-Szene heute 1 345, ruhend 461, neu gebaut 1 131.
- Einschränkungen: Wand-Uhr mit Warten auf die Grafikkarte, keine reine GPU-Zeit; die neue Seite benutzt Maske und Höhen des heutigen Renderers (dort im Messlauf ebenfalls gemerkt); Speicher der Karten 1 MB pro Lampe.

**Bild** (Endbild 1 280×720, deterministisch – zwei Läufe liefern dieselben Zahlen): mittlere absolute Differenz 0,93 von 255, 1,77 % der Pixel weichen um mehr als 8 ab (Last-Szene: 1,66 und 4,44 %). Angesehen:

- Gesamtbild, Wandflächen, Dächer, Abfall, Spot-Kegel, Schatten der Balken (schwebend): praktisch gleich.
- **Zaun** (dünner als ein Masken-Texel): heute fallen breite Lichtstreifen durch, neu nur dünne. Beide Seiten lassen Licht durch einen Zaun, der hier als dichtes Hindernis gilt; welche näher an der Wahrheit liegt, ist nicht belegt.
- **Dünne dunkle Linien** auf hellen Fassaden und an Hausecken (1 Pixel), ein etwa 2 Pixel breiterer Schattenstreifen an einer streifend beleuchteten Fassadenkante – Folge des Winkel-Rasters. Das sind sichtbare Fehler, die vor einer Übernahme weg müssen.
- **Niedrige Kisten neben niedrigen Lampen:** Schattenform und -länge weichen ab; Ursache nicht geklärt.
- Nicht prüfbar in dieser Szene: Dach über einer Lampe (abgeschaltet).

Abweichungen des Versuchs vom Entwurf: Zeile in 2 Zielen RGBA32F statt 4 × RGBA16F (der `PipelineCache` von `knorke-gpu` kann nur zwei Farbziele pro Programm); der Winkel rastet auf den Texel, nur die Entfernung wird linear gefiltert (voll gefiltert würde jeder Schatten um einen Winkel-Texel dünner); „kein Lauf“ ist −4 statt eines großen negativen Werts; ein Lauf zählt erst ab 0,75 Texel vor dem Empfänger; Chiaros Regel „Bodenpixel auf einem Hindernis-Texel gilt als beleuchtet“ ist übernommen.

**Bewertung gegen die Kriterien oben:** Tempo erfüllt für ruhende Lampen (Last-Szene mehr als 30 % schneller, kleine Szene nicht langsamer); für durchweg wandernde Lampen in der Last-Szene knapp erfüllt, in der kleinen Szene kein Gewinn. Bild: nah, aber mit sichtbaren Linien-Fehlern – die Entscheidung „kein störender Unterschied“ trifft Manuel am Bild. Noch offen: Schritt 2 (weiche Schatten), Schritt 3 (bewegte Lampe, Flimmern, Lecks an dünnen Wänden in der Draufsicht), die Ursache der Kisten-Schatten, D3D12.

### Runde 2 des GPU-Versuchs (2026-11-26): Wahrheitsbild, Nachschauen direkt in der Zeile

Drei Änderungen an der Probe (gleicher Worktree, weiter nicht committet; 723 Zeilen Rust, 479 Zeilen HLSL):

1. **Wahrheitsbild** als dritter Rechenweg: gleicher Lichtshader, Sichtbarkeit durch einen Marsch pro Pixel in Viertel-Texel-Schritten über die Höhen-Textur, ohne Distanzfeld und ohne Schrittgrenze, Regeln wie Chiaros `TraceVisibility` (erster Texel ab dem Empfänger zählt nicht).
2. **Die Schattenhöhen-Karte ist entfallen.** Die Zeile liegt gepackt in zwei Zielen RGBA32F (vier Läufe, je vier 12-Bit-Zahlen: Kanten in Viertel-Texeln, Höhen in Hundertsteln). Der Lichtshader holt beide Texel am gerasteten Winkel (**2 Zugriffe**) und prüft die vier Läufe genau nach der Lauf-Regel aus Abschnitt 1, mit Chiaros Abstand von einem Texel vor dem Empfänger. Kein Umrechen-Pass, **32 KB statt 1 MB pro Lampe**. Trick 1 aus Abschnitt 2a (ein Zugriff in eine Karte) ist damit überholt: zwei Zugriffe mit genauer Rechnung sind genauer und nicht messbar langsamer.
3. **Kanten genau:** die Kante eines Laufs liegt auf dem Eintritt des Strahls in den Texel, nicht auf dem Abtastpunkt.

**Bild gegen „wahr“** (Endbild 1 280×720; mittlere absolute Differenz von 255 / Anteil der Pixel über 8; vom Hauptagenten nachgelaufen, Zahlen gleich):

| Einstellung | Szene | heute – wahr | neu – wahr |
|---|---|---|---|
| Maske 0,5, 1 024 Winkel, Schritt 0,5 | normal | 0,92 / 1,74 % | **0,35 / 0,66 %** |
| | Last | 1,81 / 4,22 % | **0,64 / 1,60 %** |
| 2 048 Winkel | normal | | 0,31 / 0,56 % |
| Schritt 1,0 | normal | | 0,62 / 1,27 % (Mast-Schatten zerfallen sichtbar) |
| 4 096 Winkel, Schritt 0,25 | normal | | 0,17 / 0,34 % |
| Maske 1,0 | normal | 2,27 / 4,60 % | 0,41 / 0,69 % |

- **Das neue Verfahren liegt in jeder Einstellung näher an „wahr“ als das heutige.** Von den 0,66 % steht bei 0,48 % „wahr“ allein (heute und neu einig), nur bei 0,15 % weicht neu allein ab.
- Die auffälligen Unterschiede aus Runde 1 waren überwiegend Fehler des heutigen Verfahrens: helle Linie über eine Hausecke, Körnermuster am Mastfuß, Form der Kisten-Schatten, breite Lichtstreifen durch den Zaun (der Zaun ist dünner als ein Maskentexel; die Maske hat dort Löcher, auch „wahr“ lässt punktweise Licht durch).
- Verbleibende Fehler des neuen Verfahrens: gezackte Schatten-Enden und ein um einen Szenenpixel breiterer Streifen an einer streifend beleuchteten Fassade – beides **Winkel-Raster**; weg erst bei 4 096 Winkeln. Verdrängung, K = 4 und Packen zeigen sich nirgends (0,04 % bei 4 096 Winkeln und Schritt 0,25).
- Die Vermutung des Hauptagenten, der lineare Filter der Karte erzeuge dunkle Linien vor Wänden, hat sich gegen „wahr“ **nicht bestätigt**.
- Einschränkung: „wahr“ tastet punktweise ab und ist an Texelecken selbst nicht exakt (eine Pixelspalte, an der nur „wahr“ dunkel ist, ist ungeklärt).

**Zeit** (RX 6700 XT, Vulkan, Release, Wand-Uhr, Median in µs; Nachlauf des Hauptagenten 3 × 500 Frames, ruhiger; in Klammern der Lauf des Subagenten 5 × 2 000 Frames, unruhig):

| Szene | heute | neu, Lampen ruhen | neu, alle neu gebaut | ohne Lampen |
|---|---|---|---|---|
| normal (13) | 401 (478) | **257** (434) | 369 (479) | 200 (329) |
| Last (52) | 1 095 (1 115) | **417** (474) | 690 (706) | 327 (347) |

Ruhend 36 % (normal) bzw. 62 % (Last) schneller als heute; alle Lampen jedes Bild neu gebaut 8 % bzw. 37 % schneller. Mit Marsch-Schritt 1,0 wäre der Aufbau schneller (Last 577), das Bild aber sichtbar schlechter – verworfen.

### Runde 3 des GPU-Versuchs (2026-11-26): bewegte Lampe, Nachbarwinkel, weiche Schatten längs

Probe jetzt 889 Zeilen Rust, 608 Zeilen HLSL; Zahlen vom Hauptagenten nachgelaufen und gleich.

**Bewegte Lampe (Schritt 3 des Versuchs).** Eine Lampe wandert 64 Bilder lang um je 0,05 Welteinheiten, Zeile jedes Bild neu. Gemessen wird die Bildänderung von Bild zu Bild, die das Wahrheitsbild **nicht** mitmacht (in 1/255, Mittel über Pixel und Bildpaare):

| Lampe | heute | neu | neu / heute | schlimmstes Bildpaar heute / neu |
|---|---|---|---|---|
| 3 (zwischen zwei Häusern, Mast, Auto) | 0,0196 | 0,0126 | **0,64** | 0,071 / 0,031 |
| 7 (neben dem Zaun) | 0,0719 | 0,0715 | **0,99** | 0,109 / 0,120 |

Das Kriterium „Flimmern höchstens 1,5-mal so stark wie heute“ ist erfüllt; bei Lampe 3 flimmert das neue Verfahren weniger als das heutige. Bei Lampe 7 beherrscht der Zaun (dünner als ein Maskentexel) beide Verfahren. Einschränkung: nur zwei Lampen, eine Richtung, kein Video; Bildstreifen angesehen, keine springenden Kanten erkannt.

**Zwei Nachbarwinkel mischen: verworfen.** Gebaut (4 statt 2 Zugriffe, gepaarte Läufe gemischt), bringt nichts: 0,67 % statt 0,66 % gegen „wahr“. Grund: der Rest-Fehler sitzt am **Umriss** eines Hindernisses; dort hat nur einer der zwei Winkel den Lauf, und ohne Wissen, wo die Kante zwischen den Strahlen liegt, bleibt nur Einrasten. 512 Winkel reichen für die normale Szene (0,68 %), nicht für die Last-Szene (2,22 statt 1,60 %).

**Weiche Schatten, erster Teil (Schritt 2 des Versuchs).** Lampengrößen aus `plain.txt` (0,15 bis 0,3). Wahrheit weich = 4 × 4 Punkte auf der Lampenscheibe, je ein genauer Marsch. Neu weich bisher **nur längs des Strahls**: Für jeden Lauf ist die Menge der Lampenhöhen, bei denen er blockiert, ein Intervall; sein Anteil an der Lampenhöhe ±R ist die Abdeckung (Vereinigung über die vier Läufe). Das kostet keinen Zugriff und geht für R = 0 in die harte Regel über.

| Szene | heute – wahr | neu – wahr (nur längs weich) |
|---|---|---|
| normal | 1,66 / 5,47 % | **0,90 / 2,54 %** |
| Last | 1,77 / 5,35 % | 1,25 / 4,00 % |

Angesehen: Schatten-Enden laufen im neuen Verfahren weich aus wie in „wahr“. Das heutige Verfahren wirkt fast hart, sein Halbschatten ist deutlich schmaler als „wahr“. Im neuen Verfahren fehlt der Halbschatten **quer** zum Strahl (Seitenkanten, Masten, Zaunstrahlen bleiben hart) – dort liegt der Rest. Zeit (Nachlauf, 3 × 500 Frames): weich kostet neu nichts Messbares (13 Lampen 249 µs ruhend, 354 neu gebaut; 52 Lampen 438 / 691), heute wird weich etwas langsamer (421 / 1 136).

**Offen für Runde 4:** Halbschatten quer zum Strahl. Dafür braucht die Zeile pro Lauf den Abstand zum Rand des Hindernisses in Winkel-Texeln (Nachlauf-Pass über den Zeilen-Atlas). Dieselbe Angabe ist das, was dem Mischen gefehlt hat.

### Runde 4 des GPU-Versuchs (2026-11-26): Halbschatten quer zum Strahl

Probe jetzt 944 Zeilen Rust, 797 Zeilen HLSL; Zahlen vom Hauptagenten nachgelaufen und gleich.

**Aufbau.** Die harte Zeile bleibt (4 Läufe, 2 Ziele). Ein Nachlauf-Pass über den Zeilen-Atlas schreibt 2 weitere Ziele RGBA32F: pro eigenem Lauf den Abstand zum linken und rechten Rand des Hindernisses in Winkel-Texeln (je 6 Bit, Suchweite 32 Winkel je Seite), je einen fremden Lauf links und rechts (damit der Halbschatten auch außerhalb des harten Schattens ankommt) und pro Lauf und Seite, wo entlang des Laufs der Umriss steht (3 Bit). Weiche Lampen lesen 4 Texel, harte weiter 2. Im Lichtshader verdeckt jeder Lauf ein Rechteck der Lampenfläche (quer × senkrecht); gerechnet wird die Fläche der Vereinigung.

**Bild gegen die weiche Wahrheit** (jetzt 8 × 8 Punkte auf der Lampenscheibe):

| Szene | heute – wahr | neu, nur längs (Runde 3) | neu, längs + quer |
|---|---|---|---|
| normal | 1,66 / 5,45 % | 0,90 / 2,54 % | **0,61 / 2,22 %** |
| Last | 1,70 / 5,23 % | 1,25 / 4,00 % | **0,87 / 3,03 %** |

Angesehen: Seitenkanten und Enden der Schatten sind weich, die Breite passt, am Fuß eines Hindernisses hart und in der Ferne weich; das neue Bild ist glatter als „wahr“ (dort sieht man die 8 Stufen). Größte Abweichung am Zaun (Masken-Problem, weich nur verschmiert); ein Autodach ist zu hell (Ursache offen). Mit Lampengröße 0 ist das Bild zahlengleich zu Runde 2. Flimmern weich (Lampe 3): neu/heute 0,92.

**Zeit weich** (RX 6700 XT, Vulkan, Release, Wand-Uhr, Median in µs; Nachlauf des Hauptagenten 3 × 500 Frames):

| Szene | heute | neu, Lampen ruhen | neu, alle neu gebaut | ohne Lampen |
|---|---|---|---|---|
| normal (13) | 408 | 355 | **609** | 247 |
| Last (52) | 1 050 | 895 | **1 535** | 233 |

**Das ist ein Rückschritt beim Tempo:** ruhend nur noch 13–15 % schneller als heute, bei jedes Bild neu gebauten Lampen 46–49 % **langsamer**. Kostentreiber: die genaue Vereinigung von sechs Rechtecken pro Pixel und Lampe (zwei Sortierungen) und der Nachlauf-Pass mit bis zu 130 Zugriffen pro Winkel (52 Lampen: rund 360 µs bei Suchweite 32, rund 130 µs bei 8 – bei fast gleichem Bild). Runde 5 soll das beheben (früher Ausstieg für Pixel ohne Halbschatten, einfachere Vereinigung, kleinere Suchweite); ohne das ist die weiche Form in dieser Fassung nicht annehmbar.

### Runde 5 des GPU-Versuchs (2026-11-26): weiches Nachschauen verbilligt

Probe am Ende 953 Zeilen Rust, 822 Zeilen HLSL (davon rund 70 nur für das Wahrheitsbild); Zahlen vom Hauptagenten nachgelaufen.

| Hebel (Last-Szene weich, µs) | ruhend | alle neu gebaut | Bild normal / Last gegen weiche Wahrheit |
|---|---|---|---|
| Runde 4 | 934 | 1 633 | 2,22 % / 3,03 % |
| Früher Ausstieg für Pixel ohne Halbschatten | 870 | 1 545 | gleich |
| Vereinigung je Lampenhälfte (ein Durchlauf über 4 Reichweiten) statt Rechteck-Vereinigung | 579 | 1 144 | 2,32 % / 3,04 % |
| Suchweite des Nachlauf-Passes pro Lampe und Lauf statt fest 32 | 565 | 1 088 | 2,31 % / 3,03 % |
| verworfen: Summe gedeckelt | 519 | – | 2,98 % / 3,56 % |

Erkenntnis: Engpass sind nicht die Textur-Zugriffe, sondern die Rechnung pro Pixel und Lampe; der frühe Ausstieg half erst, als die Sortierschleifen ganz aus dem Shader waren.

**Endstand** (RX 6700 XT, Vulkan, Release, Wand-Uhr, Median in µs; Nachlauf des Hauptagenten 3 × 500 Frames, ruhiger Rechner; der Lauf des Subagenten mit 5 × 2 000 Frames liefert für die Last-Szene dieselben Werte, für die normale Szene höhere mit gleicher Reihenfolge):

| Schatten | Szene | heute | neu, Lampen ruhen | neu, alle neu gebaut | ohne Lampen | Bild heute / neu gegen „wahr“ |
|---|---|---|---|---|---|---|
| hart | normal (13) | 405 | **255** | 386 | 209 | 1,74 % / **0,66 %** |
| hart | Last (52) | 1 111 | **447** | 706 | 266 | 4,22 % / **1,60 %** |
| weich | normal (13) | 400 | **270** | 450 | 209 | 5,45 % / **2,31 %** |
| weich | Last (52) | 1 182 | **549** | 1 077 | 388 | 5,23 % / **3,03 %** |

### Bewertung des Versuchs gegen die Kriterien

- **Tempo, Lampen ruhen:** erfüllt. Hart 37 % (normal) und 60 % (Last) schneller als heute, weich 32 % und 54 %.
- **Tempo, alle Lampen jedes Bild neu gebaut:** hart 5 % und 36 % schneller; weich in der Last-Szene 9 % schneller, **in der normalen Szene 12 % langsamer** (450 statt 400 µs). Der Nachlauf-Pass für die Randzahlen ist hier der Rest.
- **Bild:** in allen vier Fällen näher am Wahrheitsbild als das heutige Verfahren, bei weichen Schatten deutlich.
- **Flimmern bei bewegter Lampe:** erfüllt (neu/heute 0,64 bis 0,99).
- **Umfang:** rund 750 Zeilen HLSL und 950 Zeilen Rust für die Probe samt Messgerüst – für Punkt- und Spotlichter mit harten und weichen Schatten.

**Nicht geprüft oder offen:** Flächen- und Linienlichter, Sonne und Mond, Nebel, Profile, Masken, Muster, Glanz, durchscheinende Hindernisse; schwebende Hindernisse bei weichen Schatten (Näherung); ein Lichtkeil auf einem Autodach bei weichen Schatten (Ursache vermutet: Lage des Umrisses entlang des Laufs falsch geschätzt); Zaun dünner als ein Maskentexel (Masken-Problem beider Verfahren); Lampe im Hindernis und Dach über der Lampe (kommen in der Szene nicht vor); Draufsicht mit dünnen Wänden; reine GPU-Zeit; D3D12; Grundlicht-Cache und Schatten malen (zweite Stufe).

## Konsequenzen (bei Annahme nach dem Versuch)

- Das neue Licht ersetzt den Inhalt von `knorke-light2d` schrittweise; der Chiaro-Port bleibt in der Git-Historie. ADR 0014 (Schnitt der Crates) bleibt, ADR 0017 (Pass-Plan) wird durch eine kürzere Stufenfolge ersetzt: Bodenkarte → Lichtzeilen → Lichtpass → (Nebel) → Composite.
- Die Bit-Gleichheit zu Chiaro endet. Neue Referenzbilder entstehen aus dem neuen Licht; die Chiaro-Bilder bleiben als Vorher-Vergleich.
- Shader: wenige eigene HLSL-Dateien im Repo statt Chiaros Paket; das MSL-Problem (DXC erzeugt kein MSL) bleibt offen wie bisher.
- `knorke-gpu` braucht für B keine Erweiterung. Für die Zusatzstufe aus Punkt 4 wäre zu entscheiden: Compute in `knorke-gpu` oder Fragment-Fassung.
- Host-Seite: Hindernisse und Zäune werden in die Bodenkarte gezeichnet, Ambient in eine Ebene; die Stores für Zonen, Muster und Animation entfallen oder schrumpfen zu Hilfsfunktionen.

## Quellen

- Freeman, Sannikov, Margel: Holographic Radiance Cascades for 2D Global Illumination, arxiv.org/abs/2505.02041 [belegt]
- Sannikov: Radiance Cascades, github.com/Raikiri/RadianceCascadesPaper (LaTeX-Quelle) [belegt]; Osborne, Sannikov: arxiv.org/abs/2408.14425 (Bilinear Fix, §2.5) [belegt über Zusammenfassung]
- Umsetzungen gelesen: github.com/Youssef-Afella/UnityURP-HolographicRadianceCascades2DGI, github.com/RP-content/radiance-cascades-opengl, github.com/IceKontroI/sparse_rc_2d, github.com/nixon-voxell/bevy_radiance_cascades [belegt: Aufbau und Zeilen; Bildzeiten sekundär]
- jason.today/rc, mini.gmshaders.com/p/radiance-cascades2 [sekundär]
- Chiaro @ `9bc5b9b`: `shaders/src/shadow_trace.hlsli`, `medium_polar.frag.hlsl`; Knorke `crates/knorke-light2d` [belegt]
- Horizont-Karten für Höhenfelder (Microsoft Research, „Fast Soft Self-Shadowing on Dynamic Height Fields“), Godot-2D-Schatten-Doku [sekundär]
