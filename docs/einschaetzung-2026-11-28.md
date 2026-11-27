# Einschätzung Knorke Engine und nächste Schritte

**Datum:** 2026-11-28
**Von:** Claude (Fable 5.2), im Auftrag von Manuel
**Zweck:** Arbeitsliste. Jeder Punkt ist so geschrieben, dass ein anderes Modell ihn einzeln abarbeiten kann.

**Grundlage:** Plan (Abschnitte 1, 13, 14, 16, 17), README, ADR-Liste, Ordnerstruktur, Zeilenzahlen, CI-Datei.
**Nicht gemacht:** Code nicht im Detail gelesen, keine Tests, kein Clippy, keine Benchmarks gelaufen. Urteile über Codequalität sind deshalb Vermutung und so markiert.

---

## 1. Kurzurteil

Das Fundament ist sehr gut. Was fehlt, ist alles, was aus guten Bausteinen ein Spiel macht.

Heute kann Knorke eine Szene rechnen und als PNG schreiben. Es kann kein Fenster öffnen, keine Taste lesen, keine Datei als Asset laden und keinen Text zeichnen. Es gibt noch kein Spiel, das auf Knorke läuft.

## 2. Was gut ist

- **Determinismus, Snapshots, Replay.** Gleiches Ergebnis auf Windows, Linux, macOS/ARM und WASM, per Fingerprint in der CI geprüft. Das haben wenige Engines.
- **Vertragstests.** „0 Allokationen im Dauerbetrieb“ und „bitgleich über Thread-Zahlen“ sind gemessen, nicht behauptet.
- **Klare Regeln.** 22 ADRs, Abhängigkeitsregeln als Code (`xtask`), Clippy erzwingt die Determinismus-Regeln.
- **2D-Licht.** Chiaro ist portiert, die Lichtzeilen sind schneller und genauer als der alte Weg.
- **KI.** Wegsuche, Wahrnehmung, Entscheider, Gruppen – aus Sagax übernommen und geteilt.
- **Audio.** Mischpult, Raumklang, SDL-Gerät, sample-genauer Start.
- **Agenten-Ebene.** `knorke discover / inspect / validate / mutate / execute / render` läuft für ein Profil.

## 3. Was auffällt

| Befund | Beleg |
|---|---|
| Der Code ist ungleich verteilt. Licht ist riesig, der Engine-Kern winzig. | `knorke-light2d` 46 350 Zeilen (davon 16 563 in Tests), `knorke-runtime` 717, `knorke-agent` 635, `knorke-scene` 552 |
| `knorke-light2d` sollte schrumpfen (Plan 17.1: „mit ~29 000 Zeilen zu groß“). Es ist nach meiner Zählung nicht kleiner geworden. | Zählung `*.rs` am 2026-11-28; Zählweise von damals unbekannt, Vergleich daher unsicher |
| Kein Fenster, keine Eingabe, kein Echtzeit-Host. | Plan 17.1 „Fenster und Echtzeit-Host: offen“; GPU-Gerät kommt aus `knorke_gpu::test_support` |
| Schrift, Text, HUD, Menüs, Spielstände, Lokalisierung stehen nicht im Plan. | Suche in `docs/plan.md`: 0 Treffer für Schrift, Font, Spielstand, Lokalisierung |
| Kein Asset-System, keine Kollision, keine Animation. | Plan 17.1, 17.4, 17.8: offen bzw. geplant |
| Nur Vulkan ist getestet. | Plan 17.12: D3D12 und MSL offen |
| 3D wurde angefangen, bevor ein 2D-Spiel läuft. | ADR 0021, Plan 17.14, Schritt 3D-1 „läuft (Entwurf)“ |
| README und Plan-Kopf sind veraltet. | Beide sagen „Phase 0, noch keine Fachmodule“ |
| Der Plan ist für Agenten zu teuer zu lesen. | 1068 Zeilen, aber rund 85 000 Tokens; Abschnitt 16 hat Tabellenzeilen mit mehreren tausend Zeichen |
| Sehr hohes Tempo, wenig menschliche Kontrolle. | 318 Commits seit 2026-11-23, rund 140 000 Zeilen Rust |

Der letzte Punkt ist kein Vorwurf. Er heißt nur: die Tests tragen die ganze Last. Wo Tests fehlen oder still übersprungen werden, weiß niemand, ob es stimmt.

## 4. Die Liste

Reihenfolge = meine Empfehlung. Jeder Punkt nennt, woran man „fertig“ erkennt.

### A. Zuerst: aus Bausteinen eine Engine machen

**A1. Fenster, Echtzeit-Schleife und Eingabe**
- Neue Crate für den Host (Name z. B. `knorke-host`), auf SDL3.
- Fenster öffnen, Bild zeigen, `Runtime::frame` mit echter Zeit füttern.
- Tastatur, Maus, Gamepad lesen und als semantische Commands in die Simulation geben (ADR 0010, Eingabegrenze).
- Gerät und Shader-Paket nicht mehr aus `test_support` holen.
- Fertig, wenn: die Wachmann-Szene in einem Fenster läuft und der Eindringling sich mit den Pfeiltasten bewegen lässt. Ein Replay der Sitzung besteht `verify`.
- Braucht vermutlich eine ADR (wem gehört die Schleife, wie werden Tasten zu Commands).

**A2. Kleines Asset-System**
- Assets über IDs ansprechen, nicht über Pfade (Plan 17.2).
- Texturen (PNG) und Klänge (WAV) laden. Laden passiert außerhalb der Simulation, das fertige Ergebnis kommt als Command.
- Noch kein Streaming, keine Referenzzählung, kein Hot-Reload.
- Fertig, wenn: eine Szene ihre Bilder und Klänge über IDs aus einem Ordner bekommt und ein fehlendes Asset eine `Diagnostic` mit stabilem Code liefert.

**A3. Schrift und einfache Oberfläche**
- Steht bisher nicht im Plan. Zuerst in Plan 17 eintragen.
- Bitmap-Schrift reicht für den Anfang (passt zum Pixel-Look). Text, Rechtecke, Bilder zeichnen.
- Kein UI-Framework. Nur genug für Anzeige, Menü und Debug-Text.
- Fertig, wenn: das Fenster aus A1 einen Zähler, einen Pausentext und ein Menü mit zwei Einträgen zeigt.

**A4. Ein kleines Spiel bis zum Ende bauen**
- Der Stealth-Hof als echtes Spiel: Start, Ziel erreichen oder erwischt werden, Neustart. Mit Licht, Klang und Text.
- Dazu gehört „Licht beeinflusst Wahrnehmung“ (Plan 17.1, offen): `SenseMap::light` aus den Lichtern der Szene speisen, auf der CPU, deterministisch.
- Zweck: alles, was beim Zusammenstecken klemmt, zeigt sich hier und nicht erst bei Wizardry 7.
- Fertig, wenn: man es fünf Minuten spielen kann, ohne dass etwas fehlt, und ein Agent dasselbe Spiel über die CLI starten, verändern und prüfen kann.

### B. Danach: was jedes Spiel braucht

**B1. Kollision und Abfrage-Vertrag**
- Query-Trait „Physik-Welt“ (Raycast, Shape-Cast, Überlappung), Plan 17.8.
- Erst eine eigene, einfache Umsetzung auf Raster und den Formen aus `knorke-math`. Rapier erst, wenn ein Spiel echte Physik braucht.
- Fertig, wenn: KI und Spielfigur dieselbe Abfrage benutzen und ein Invarianten-Test „niemand steckt in einer Wand“ grün ist.

**B2. Sprite-Animation**
- `knorke-animation`: Frame-Folgen, Kurven, Ereignisse (Schritt, Treffer) als `EventQueue`-Events (Plan 17.4).
- Fertig, wenn: eine Figur im Spiel aus A4 läuft und der Schritt-Klang vom Animations-Ereignis kommt.

**B3. Spielstände**
- Steht nicht im Plan. Snapshots gibt es, aber kein Dateiformat mit Version und Szenen-Kennung.
- Hängt zusammen mit „Szenen-Kennung im Replay-Kopf“ (Plan 17.1, offen).
- Fertig, wenn: Speichern, Programm beenden, Laden ergibt denselben Zustand (Prüfsumme gleich), und ein Stand aus einer anderen Szene wird sauber abgelehnt.

**B4. Agenten-Ebene Stufe 3**
- `query`, `observe`, `verify`, Replay-Operationen, Skript als ein Batch, dann der MCP-Adapter (Plan 17.1, geplant).
- Profile melden sich selbst an (Capability-Manifest, ADR 0004). Heute kennt `knorke-agent` nur `stealth` fest.
- Fertig, wenn: ein zweites Profil ohne Änderung an `knorke-agent` nutzbar ist.

**B5. 3D weiterführen (Plan 17.14, Schritte 3D-1 bis 3D-4)**
- Mit Wizardry 7 als echtem Verbraucher. Dazu die C-Schnittstelle `knorke-abi`.
- Erst nach A1 bis A3, weil 3D dieselben Dinge braucht (Fenster, Eingabe, Assets, Text).

### C. Aufräumen und Absichern

**C1. Dokumente für Agenten billig machen**
- README auf den heutigen Stand bringen (Stand-Zeile, Beschreibung `knorke-light2d`, fehlende Crates `knorke-gpu`, `knorke-post`, `knorke-audio`, `knorke-audio-sdl`).
- Plan-Kopf (Zeile 5) korrigieren.
- Abschnitt 16 teilen: im Plan nur eine Zeile pro Baustein, die langen Berichte in eine eigene Datei (z. B. `docs/stand.md`).
- Fertig, wenn: ein Agent Abschnitt 16 und 17 in einem Stück lesen kann.
- Klein und schnell. Kann sofort gemacht werden.

**C2. GPU-Tests dürfen nicht still ausfallen**
- `KNORKE_REQUIRE_GPU=1` in allen GPU-Tests (Plan 17.12, heute nur in einem).
- CI mit Software-Vulkan (lavapipe) für Bildtests auf Linux (Plan 17.2).
- Fertig, wenn: ein GPU-Test ohne GPU auf dem Abnahme-Rechner rot wird statt grün.

**C3. D3D12 prüfen**
- Eigene Shader laufen nur auf Vulkan (Plan 17.12). Auf Windows ist D3D12 der übliche Weg.
- Fertig, wenn: die Bildtests auf D3D12 laufen oder die Abweichungen notiert sind.

**C4. `knorke-light2d` durchsehen**
- Ein Drittel des ganzen Codes. Nach dem Löschen von Chiaros Schatten-Weg lohnt eine Prüfung: was ist noch nötig, was doppelt?
- Offener Messpunkt: Szene `shafts` ist rund 34 % langsamer als in Chiaro (Plan 17.1).
- Fertig, wenn: Zeilenzahl vorher/nachher und Messwerte im Plan stehen, Bilder unverändert.

**C5. Dauer der Prüfkette messen**
- Die sechs Befehle aus „Fertig heißt“ laufen vor jedem Abschluss. Wie lange dauert das? Nicht gemessen.
- Wenn es lang ist: bremst jeden Agenten bei jedem Schritt. Dann lohnt Beschleunigen mehr als jede Einzeloptimierung.
- Fertig, wenn: Zeiten mit Hardware und Datum im Plan stehen.

**C6. KI-Schnittstellen angleichen**
- `Seed`/`Tick` statt `u64`/`u32`, `Column` statt Slices, `Invariants` für KI-Module (Plan 17.5).
- Erst wenn A4 zeigt, wo es wirklich stört.

### D. Bewusst später

Nicht anfangen, bevor A und B stehen: Netzwerk (17.3), Mods und WASM (17.7), Skriptsprache, Task-Graph-Scheduler, Pipelining, HRTF-Audio, GPU-getriebenes Rendering, Skelett-Animation, weitere Licht-Ideen (17.13).

## 5. Eine Entscheidung für Manuel

**Was ist das erste echte Spiel auf Knorke?**

- **Möglichkeit 1 (meine Empfehlung):** A1 bis A3 bauen, dann der Stealth-Hof als kleines Testspiel (A4), danach Wizardry 7. Das Testspiel ist klein, zeigt Fehler früh und kostet wenig.
- **Möglichkeit 2:** A1 bis A3 bauen, A4 überspringen und direkt Wizardry 7. Schneller am echten Ziel, aber die ersten Fehler tauchen dann zusammen mit dem 3D-Umbau auf und sind schwerer zu finden.

In beiden Fällen gilt: A1 bis A3 zuerst. Beide Wege brauchen sie.
