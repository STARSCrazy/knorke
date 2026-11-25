# Knorke Engine – Architektur-, Migrations- und Implementierungsplan

**Stand:** 23. November 2026  
**Version:** 0.6 – plattformunabhängiger Determinismus, abschaltbare Zusatzfeatures (ADR 0011)  
**Status:** Zielarchitektur und ausführbarer Arbeitsplan; Phase 0 läuft: Basis-Crates für Datenaustausch, Parallelität und Analyse stehen, noch keine Fachmodule  
**Projektname:** **Knorke Engine** ist der gewählte Produktname, **Knorke** das Präfix der Bibliotheksfamilie (`knorke-*`). Vor einer öffentlichen Veröffentlichung sind Marke, Domains und exakte Paketnamen zu überprüfen; diese Prüfung ist noch kein Nachweis der Verfügbarkeit.

> **Leitentscheidung:** Wir bauen eine **Rust-first, agent-native, kompositionelle Game Engine**. Bereits vorhandener Code aus **Sagax** (KI/Navigation) und **Chiaro** (Beleuchtung) ist wertvolle Ausgangsbasis, aber **kein architektonischer Zwang**. APIs, Crate-Grenzen, Datenmodelle, Namen, Repositories und sogar die Implementierungssprache von Chiaro dürfen geändert werden. Maßstab sind die langfristig beste Integration, eigenständige Nutzbarkeit, Performance, Qualität und agentische Steuerbarkeit – nicht die Erhaltung alter Strukturen.

## 1. Produktvision und unverhandelbare Anforderungen

Knorke ist ein **System aus separaten Spielentwicklungs-Bausteinen** mit einer gemeinsamen, optional zusammensetzbaren Laufzeit und einer **agentischen Development Plane**. KI-Agenten erstellen und ändern Spiele über versionierte, maschinenlesbare Operationen; sie können ausführen, untersuchen, testen, visuell prüfen, Fehler lokalisieren und Änderungen übernehmen oder verwerfen.

**Nicht das Ziel:** Ein klassischer grafischer Editor als Hauptentwicklungswerkzeug oder eine „LLM schreibt endlos C++/Rust und hofft, dass es funktioniert“-Engine.

**Verbindliche Prinzipien:**

1. **Standalone by design:** `render`, `lighting`, `ai`, `navigation` usw. sind unabhängig von `world`, `runtime`, `agent`, CLI und MCP nutzbar. Eine separat verwendete Bibliothek muss kein komplettes Knorke-Projekt initialisieren.
2. **Nahtlose Komposition:** Wenn Module zusammenarbeiten, verwenden sie gemeinsame, explizite Verträge, eine koordinierte Laufzeit und nach Möglichkeit gemeinsame Speicher-/GPU-Ressourcen. Keine dauernden Formatkonvertierungen oder State-Dumps.
3. **Integration vor Bestandsschutz:** Breaking Changes und Neuimplementierungen sind erlaubt. Die **vorherigen Fähigkeiten** und Qualitätsstandards sollen, wo fachlich sinnvoll, nachweisbar erhalten oder übertroffen werden – **nicht notwendigerweise alte APIs, Binary-Kompatibilität oder Repo-Strukturen**.
4. **Rust first, nicht Rust um jeden Preis:** Neue Kernsysteme werden in Rust gebaut. Chiaro ist Kandidat für einen vollständigen Rust-Neubau. Alte C++-Implementierungen können temporär als Referenz und Vergleichssystem dienen; native C/C++-Interop bleibt möglich.
5. **Agentisch statt GUI-zentriert:** Eine einzige typisierte Service-/Command-Schicht; CLI, MCP und SDK sind dünne Zugänge. Strukturierte Metadaten, Schema-Discovery und überprüfbare Operationen gehören zum Design.
6. **Observability und Verifikation von Anfang an:** Jeder wichtige Zustand, jeder Integration Flow und jede Fehlersituation muss gezielt inspizierbar sein. Tests, Traces, Replays und Bild-Captures sind keine nachträglichen Extras.
7. **Performance ohne Magie:** Keine verpflichtende Serialisierung, IPC, dynamische Reflexion, Event-Bus-Durchläufe oder FFI-Aufrufe pro Sprite/NPC im Hot Path.
8. **Explizite Grenzen:** Ownership, Lebensdauer, Koordinaten, Einheiten, Farbräume, Tick/Frame-Reihenfolge, Nebenwirkungen und Threading werden vertraglich festgelegt.
9. **Freiheit für Spiele:** Knorke darf nicht ein ECS, eine KI, einen Renderer, eine Physik-Engine oder eine einzelne Spielarchitektur als einzig richtigen Weg erzwingen.

## 2. Ausgangslage – gesichtet, noch zu verifizieren

### 2.1 Sagax – bestehende Rust-KI-Bibliothek

Repository: https://github.com/STARSCrazy/sagax

Bei der Bestandsaufnahme sichtbarer Stand: Cargo-Workspace Version `0.16.0`; unter anderem `sagax-core`, `sagax-nav`, `sagax-decide`, `sagax-perception`, `sagax-debug`, `sagax-ffi`, `sagax-wasm`, `sagax-abi` sowie Umbrella-Crate. Enthält A*/Dijkstra, Hex-/Grid-Navigation, D* Lite, Flow Fields, Utility AI, Missionen, Wahrnehmung, Gruppentaktiken, Tracing, deterministische Ansätze und C-/JS/WASM-Anbindung.

**Stand 2026-11-23:** umgebaut in `knorke-grid`, `knorke-navigation`, `knorke-perception`, `knorke-ai`, `knorke-integration` (ADR 0012, Abschnitt 16); Bindings (`sagax-abi/ffi/wasm`) nicht übernommen (17.5).

**Neuer Auftrag:** Nicht bloß eine `knorke-sagax`-Fassade darum schreiben. Prüfen und, wenn sinnvoll, **interne Crate-Grenzen, Begriffe, Zustands-/Query-Verträge, Ownership, allgemeine Datentypen, Navigations-/Entscheidungs-APIs und Paketstruktur grundlegend verändern**. Bisherige Algorithmen dürfen wiederverwendet werden; öffentliche Kompatibilität ist kein Selbstzweck.

### 2.2 Chiaro – bestehende C++-Beleuchtungsbibliothek

Repository: https://github.com/STARSCrazy/chiaro

Bei der Bestandsaufnahme sichtbarer Stand: C++20 + SDL3 GPU, C-ABI, 2D-/2.5D-Licht und Schatten, Normal-/Materialpuffer, Bloom, geometrische Hindernisse, Batch-Operationen, Shader-Builds, verschiedene Renderpfade, Demos und Tests. `CMakeLists.txt` meldet Version `0.5.1`; die README referenziert noch die API 0.4. Dokumentation beschreibt detaillierte Ownership-, Format- und Frame-Verträge.

**Neuer Auftrag:** **Rust-Neuentwicklung ausdrücklich als bevorzugt zu untersuchende Option**; nicht automatisch die C++-Bibliothek für immer einbinden. Besonders prüfen, welche GPU-/Shader-Komponenten für den gemeinsamen Renderer geeignet sind und welche rein zum Lichtsystem gehören. Gute Algorithmen, Testfälle, Shader und Szenenreferenzen übernehmen; die öffentliche Chiaro-API muss nicht erhalten bleiben.

### 2.3 Bereits begonnene Grafikbibliothek

**Identifiziert: pixelraum** (`D:\Entwicklung\pixelraum` @ `81927d8`, Rust, reiner CPU-Software-Renderer im Pixel-Look, keine GPU-API, C-Schnittstelle für Wizardry 7). Audit nach §7.1: `docs/inventory/pixelraum.md`. Migriert nach `knorke-render` + `knorke-lighting` + `knorke-integration::pixel` (ADR 0013); die C-Schnittstelle bleibt vorerst im Original (§17.1).

### 2.4 Aussagekraft der Bestandsaufnahme

Die bisherige Prüfung betrifft ausgewählte Repository-Dateien und Dokumentation, **keine vollständigen lokalen Builds, Codeaudits, GPU-Tests oder Benchmarks**. Erst Phase 0 erhebt belastbare Baselines.

## 3. Ein einheitliches Namens- und Paketkonzept

**Namensentscheidung:** Die Produktfamilie heißt **Knorke Engine** (Kurzname **Knorke**). Die öffentlichen Rust-Pakete, Namensräume und Schemas verwenden das Präfix **`knorke-`** bzw. **`knorke.`**. **Sagax** und **Chiaro** bleiben als Herkunft und technische Referenzen nachvollziehbar, können aber im Rahmen der Migration umbenannt und grundlegend verändert werden. Verfügbarkeit von Marken, Domains und exakten Crate-Namen ist vor dem öffentlichen Release noch zu prüfen.

### 3.1 Vorgeschlagene öffentliche Namen

| Fachfunktion | Öffentliches Zielpaket | Bisheriger Ursprung | Rolle |
|---|---|---|---|
| Gemeinsame Basisverträge | `knorke-types` | neu | Kleine Datentypen/IDs, keine Runtime |
| Mathematische Hilfen | `knorke-math` | neu / vorhandene Typen prüfen | Vektoren/Transformationen; sparsam einsetzen |
| 2D-/3D-Render-API | `knorke-render` | Grafikbibliothek | Unabhängiger Renderer |
| GPU-Abstraktion | `knorke-gpu` | Grafikbibliothek + Chiaro prüfen | Device, Resources, Command/Pass-Integration |
| 2D-/2.5D-Licht | `knorke-lighting` | Chiaro | Licht, Schatten, Materiallicht, Postprocessing-Anteile |
| Navigation | `knorke-navigation` | Sagax | Grid/Hex/Pathfinding/Flow Fields |
| Entscheidungen/Planung | `knorke-ai` | Sagax | Utility AI, Missions, Squad/Crew und Erweiterungen |
| Wahrnehmung | `knorke-perception` | Sagax | Sicht, Hören, Wahrnehmungs-/Knowledge-Modelle |
| Szenen/Authoring | `knorke-scene` | neu | Persistente Weltbeschreibung/Assets/Prefabs |
| Ausführung | `knorke-runtime` | neu | Simulation, Scheduler, Lifecycle |
| Modulverbindungen | `knorke-integration` | neu | Optionale Profile und konkrete Adapter |
| Agentenschnittstellen | `knorke-agent`, `knorke-cli`, `knorke-mcp` | neu | Development Plane, nicht im Release-Spiel nötig |
| Diagnostik/Tests | `knorke-observe`, `knorke-verify` | Sagax/Chiaro + neu | Gemeinsames Auswertungsmodell |
| Sprachintegration | `knorke-ffi` / `knorke-wasm` | Sagax/Chiaro + neu | Nur dort, wo tatsächlich benötigt |

Dies sind **angestrebte Paketgrenzen, keine Pflicht, sofort alle Crates anzulegen**. `knorke-gpu` wird nur dann ein eigenes Paket, wenn der Grafik-/Licht-Spike tatsächlich eine stabile gemeinsame Abstraktion ergibt. `knorke-types` darf **nicht** zum riesigen Core werden.

### 3.2 Benennungsregeln

- Rust-Pakete: `knorke-*`; Rust-Imports: `knorke_render`, `knorke_ai` usw.; Rust-Typnamen nach Rust-Konventionen.
- Komponenten-IDs: stabile Namespaces, z. B. `knorke.render.sprite/1`, `knorke.lighting.point/1`, `knorke.ai.utility/1`.
- Agenten-Operationen: sprechende stabile Namen, z. B. `scene.entity.create`, `lighting.light.set`, `simulation.step`.
- Intern durchgängig dieselben Begriffe: `EntityId`, `ResourceId`, `Transform`, `WorldSpace`, `FrameContext`, `TickContext`, `Diagnostic`, `Capability`.
- **Keine vorschnelle Vereinheitlichung per globalem Datentyp:** z. B. 2D-Pixel, 2.5D-Höhen, 3D-Meter und Hex-Koordinaten sind fachlich verschieden. Explizite Konvertierungen statt falscher Gleichsetzung.
- Öffentliche API und Agentenschemas werden unabhängig versioniert; historische Bezeichner werden bei Bedarf über **zeitlich begrenzte** Migrationswerkzeuge unterstützt.

### 3.3 Repository-Entscheidung

**Bevorzugt: ein Rust-Cargo-Monorepo mit mehreren tatsächlich eigenständig nutzbaren Crates.** Der Vorteil ist ein gemeinsamer Abhängigkeitsgraph, konsistente Versionierung, atomare Integrationsänderungen und CI über alle Module. **Repo-Grenzen sind keine Modulgrenzen:** Ein Crate muss auch aus dem Workspace heraus separat nutzbar und gegebenenfalls veröffentlichbar sein.

- Die bestehenden Repositories `sagax` und `chiaro` bleiben während der Migration als Referenzen erhalten; sie **müssen nicht** dauerhaft die offiziellen aktiven Produkt-Repositories bleiben.
- Git-Historie, Copyright- und Lizenzhinweise erhalten; Migration dokumentieren; alte Namen/Repos nach Abschluss mit deutlichen Weiterleitungen archivieren oder im Migrationsmodus belassen.
- Falls andere Nutzer dauerhaft eigenständige Releases brauchen, kann später aus demselben Monorepo einzeln veröffentlicht werden; eigenständige Repos bleiben eine Alternative, wenn die konkrete CI-/Release-Praxis dafür spricht.
- **Kein blindes Big-Bang-Umbenennen:** Der Produktname steht fest. Öffentliche Module erhalten das neue `knorke-*`-Schema im Zuge der technisch geprüften Migration; interne Umbenennungen richten sich nach den tatsächlich festgelegten Modulgrenzen.

## 4. Architektur – Trennung und Integration gleichzeitig

```text
                       ENTWICKLUNGSSEITE (optional)
  Agenten / Codex / Claude / CLI / MCP / SDK
                     | typisierte Commands
  knorke-agent + knorke-scene + knorke-observe + knorke-verify
                     |
           knorke-runtime + knorke-integration
                     | explizite Profile und Scheduler
        +------------+-------------+--------------+
        |            |             |              |
  knorke-render   knorke-lighting  knorke-ai    knorke-navigation
        |            |             |              |
        +--- knorke-gpu?            +-- knorke-perception
                      |                |
          SDL3-GPU-Backend         Daten-/Query-Verträge

  STANDALONE: Jeder fachliche Zweig ist auch ohne knorke-runtime,
              knorke-agent und MCP verwendbar.
```

**Abhängigkeitsregeln (automatisch in CI prüfen):**

1. `knorke-ai`, `knorke-navigation`, `knorke-perception` haben **keine** Grafik-, SDL3-, GPU-, Agenten- oder Runtime-Abhängigkeit.
2. `knorke-render` importiert **nicht** `knorke-lighting` und umgekehrt keine komplette Renderer-/World-Runtime. Beide dürfen eine **minimale** gemeinsame GPU-Schicht nutzen, sofern das die Integrationsmessung rechtfertigt.
3. `knorke-lighting` ist **auch ohne `knorke-render`** nutzbar: Der Host stellt erforderliche Texturen/Render-Targets und Device/Command-Kontext bereit.
4. `knorke-runtime` setzt Systeme zusammen, besitzt aber nicht die fachlichen Algorithmen von KI, Grafik oder Beleuchtung.
5. `knorke-agent`/CLI/MCP bleiben außerhalb der ausgelieferten Runtime-Abhängigkeiten.
6. Integrationsadapter dürfen auf mehrere Module zugreifen; **die fachlichen Module importieren ihre Adapter nicht zurück**.
7. Offene Schnittstellen werden nach realen Integrationsfällen definiert, nicht als abstraktes Framework ohne Verbraucher.

### 4.1 Gemeinsame Verträge

- **IDs:** stabile Authoring-IDs und generationgesicherte Runtime-Handles; keine Verwechslung von Persistenzidentität und Speicheradresse.
- **Raum und Maße:** Welt-/Bildschirm-/Tile-/Hex-/3D-Raum, Einheiten, Transformationen, Rotation, Höhe, Ursprung, Präzision.
- **Zeit:** Fixed Tick, Frame-Zeit, Delta, Seed; keine heimlich selbst laufenden Uhren in KI/Animation/Beleuchtung.
- **Ressourcen:** explizite Ownership, Leih-/Freigaberegeln, GPU-Device-Lebensdauer, Usage-/Format-Angaben, asynchrone Transfers.
- **Verhalten:** `Capabilities` mit Eingabe-/Ausgabeschemas, Voraussetzungen, Constraints, Nebenwirkungen, Fehlern und Tests.
- **Diagnose:** Module liefern maschinenlesbare Ereignisse/Statistiken mit stabilen Codes; Adapter übersetzen ins gemeinsame Format.
- **Threading:** deklarierte Thread-/GPU-Kontexte, Scheduler-Abhängigkeiten, Sichtbarkeit von Änderungen und Synchronisationspunkte.

### 4.2 Integration Profiles statt direkter Kopplung

Profile sind versionierte, testbare Definitionen, die Komponenten durch fachliche Verträge verbinden:

- `character.navigation.motion`: Navigationsabsicht → Bewegungssteuerung; Ist-Position → Pfad-/Replan-System.
- `character.motion.animation`: tatsächliche Bewegung → Animationsstatus.
- `world.obstacle.navigation`: relevante Änderungen an Hindernissen → inkrementelle Aktualisierung der Navigation.
- `scene.render.lighting`: Renderer-Buffers (Color/Normal/Height/Occlusion/Emissive) → Lighting-Passes → Output.
- `perception.world.light`: **Gameplay-relevante** Helligkeit/Wahrnehmung → KI, möglichst ohne GPU-Readback.

**Achtung:** Profile sind **keine Magie**. Sie nennen Quell-/Zieldatentypen, Koordinatentransforms, Reihenfolge, Fehlerbedingungen und Testfälle. Der Hot Path läuft nach Bindung über native Zugriffe, nicht pro Frame über ein JSON- oder Agentenprotokoll.

### 4.3 Render-Lifecycle: ein gemeinsames GPU-Gerät

- Eine Host-Anwendung/ein Knorke-Runtime-Backend besitzt `SDL_GPUDevice`, Swapchain und Frame-Scheduling.
- Renderer und Lighting dürfen eigene Ressourcen besitzen, aber arbeiten mit **demselben Device** und einem kompatiblen Pass-/Command-Modell.
- Wenn der Host einen Command Buffer liefert, darf kein Modul eigenmächtig die Swapchain präsentieren oder den Buffer submitten.
- Lichtsystem verarbeitet **vom Renderer unabhängige Eingaben** (`Color`, optional `Normal`, `Height`, `Material`, `Occlusion`, `Emissive`) – ein eigenständig erzeugtes Texture-Target ist zulässig.
- Erst die Integrationsschicht entscheidet über Pass-Reihenfolge und Ressourcennutzung. GPU-Transfer über die CPU wird nicht zum Standardpfad.
- Farbräume, Tonemapping, Alphamodus, Koordinaten und Auflösungen müssen einmal zentral und messbar geklärt werden.

## 5. Rust-Strategie und Chiaro-Neuimplementierung

**Richtung:** Rust ist die bevorzugte Implementierungssprache des neuen Systems. Für Chiaro werden **zwei tatsächlich funktionierende Varianten** verglichen, nicht nur subjektive Vorlieben:

| Weg | Idee | Vorteile | Nachteile |
|---|---|---|---|
| **A – vorhandenes Chiaro via C-ABI** | Bestehende C++-Engine an Rust binden | Schnelle Referenz, bestehende Features, geringer Erstaufwand | Zwei Build-Ökosysteme; GPU-/Ressourcenlogik möglicherweise doppelt; strukturelle Grenzen bleiben |
| **B – neues `knorke-lighting` in Rust** | Engine-native GPU-/Pass-Verträge; Algorithmen/Shader/Tests übernehmen | Einheitliches Rust-Ownership, gemeinsamer Render-/GPU-Layer, einfachere Agenten-/Diagnoseanbindung | Höherer Initialaufwand; Grafikregressionen, Portierungs- und Performance-Risiken |

**Vorläufige Präferenz: B (Rust), sofern der technische Spike das bestätigt.** Der Nutzer akzeptiert kompletten Umbau; Bestandsschutz ist **kein** Kriterium. Möglichkeit A wird **als Baseline und Übergang**, nicht als Default für die endgültige Architektur betrachtet.

### 5.1 Entscheidungsgate für den Rewrite

Vor dem vollständigen Port drei kleine, messbare Spikes:

1. **GPU-Interop-Prototyp (A):** Bestehende Chiaro-API aus Rust ansprechen; denselben GPU-Kontext wie die Grafikbibliothek verwenden; Render-Target ohne CPU-Readback verarbeiten.
2. **Rust-GPU-/Light-Spike (B):** `SDL_GPUDevice`-Wrapper, Shader-Pipeline, ein Punktlicht, mindestens ein Schattenfall und Offscreen-Capture nativ in Rust implementieren.
3. **Vergleich:** GPU-Passzeiten, Draw/Dispatch-Aufwand, Ressourcenübergaben, Fehlerdiagnose, Code-/Build-Komplexität, visuelle Differenzen und Erweiterbarkeit gegen dieselbe Referenzszene vergleichen.

**Entscheidung:** Wenn Rust-Integration architektonisch sauber gelingt und Performance/visuelle Korrektheit das Budget einhalten, wird **Rust die Zielimplementierung**. Wenn Blocker auftauchen, zuerst die Ursache beheben oder C++ als austauschbares Backend behalten. **Kein blindes Komplett-Portieren vor diesem Nachweis.**

### 5.2 Reihenfolge eines Rust-Neubaus

1. Kleiner sicherer Rust-Wrapper für SDL3-GPU-Handles und Command-Lifetimes; `unsafe` auf kontrollierte Grenzen beschränken.
2. Ressourcen- und Render-Pass-Verträge: Device, Texture, RenderTarget, Shader, Pipeline, CommandContext, Synchronisation.
3. Basisausgabe: Szene übernehmen → linearer Compositing-Pass → Zieltextur; Offscreen-Bilder.
4. Punkt-/Spot-/gerichtete Lichter, Schatten-Geometrie und Culling.
5. Normals/Height/Materialien, 2.5D/isometrische Projektion.
6. Weitere Lichtquellen, Schattenmasken/-Caches, Bloom, Glow, Tonemapping, Spezialeffekte.
7. Qualitätspresets, GPU-Profiler, Diagnose, Batching, integrierte Tests und visuelle Referenzen.
8. Erst wenn ausreichend gleichwertig: C++-Chiaro ablösen/archivieren und neue Knorke-Namen aktivieren.

Die HLSL-/shadercross-Pipeline und bestehende Shader können **zunächst** weiterverwendet werden, sofern das Rust-GPU-Backend kompatibel ist. Die Sprache des CPU-Codes zwingt nicht zu einer neuen Shadersprache.

### 5.3 Feature-Parität ohne API-Bestandsschutz

- Eine **Feature-Matrix** kennzeichnet `übernommen`, `vereinfacht`, `neu entworfen`, `bewusst gestrichen`, `noch offen`.
- Visuelle Golden Scenes, Benchmarks und Referenzfälle aus Chiaro werden zu **Contract-Tests**, nicht zu Verpflichtungen, denselben C-Funktionsnamen beizubehalten.
- Parität wird **pro Feature/Referenzszene** überprüft; keine Forderung, dass alle GPU-Treiber bitidentische Bilder erzeugen.
- Rust-Wrapper/Ports bekommen Unit-, GPU-, Render-Graph-, Fehler- und Lifecycle-Tests; Device-Loss und Ressourcenfreigabe gesondert testen.
- Das alte Repository ist während der Migration lesbare Referenz und Vergleichsbackend, keine festzementierte Produktgrenze.

## 6. Sagax-Neuordnung und Integrationsrefactoring

Sagax darf genauso grundsätzlich umgebaut werden. **Wiederverwendbare Algorithmen bewahren, Kopplungen und Namen frei überarbeiten.**

### 6.1 Fachlich sinnvolle Zielgrenzen

- `knorke-navigation`: Wegsuche, Grid/Hex, Flow Fields, Reparatur, Formationen und Navigationsanfragen.
- `knorke-perception`: Sicht/Hören/Knowledge-Eingaben und Wahrnehmungsmodelle, ohne Render- oder World-Pflicht.
- `knorke-ai`: Utility Decisions, State Machines, Missions, Verhalten, Taktik, Crew/Squad.
- Diagnose/Tracing: shared diagnostics-contract, aber keine runtimepflichtige zentrale Logger-Abhängigkeit.

**Nicht automatisch jede heutige Sagax-Crate 1:1 umbenennen.** Nach Dependency-Audit Module nach ihren tatsächlichen Verantwortungen neu schneiden. Falls `decide` und `perception` starke Kopplung besitzen, über kleine gemeinsame Verhaltensverträge oder explizite Adapter lösen.

### 6.2 Refactoring-Aufgaben

1. Feature-, Dependency- und API-Matrix aus vorhandenem Sagax-Code erfassen.
2. Ziel-Domänentypen und Eingabe-Queries bestimmen: Grid-/Hex-Koordinaten, Positions-/Bewegungskosten, Knowledge, Sichtbarkeit, Zeit, Seeds.
3. Abhängigkeit von externen World-/Entity-/ECS-Typen verhindern; **Pull-/Callback- oder Batch-Queries** statt voller Weltkopie.
4. Verschiedene KI-Teile in nachvollziehbare Entscheidungsphasen zerlegen: Wahrnehmen → Wissen aktualisieren → Ziel wählen → Planen → Intent ausgeben.
5. Diagnostik nach einheitlichem Ereignismodell normalisieren; Traces bei Bedarf abschaltbar und im Hot Path billig halten.
6. Standalone Rust-Tests für jedes Zielpaket; C-ABI und WASM als **optionale Randpakete** erneut gestalten, nicht blind 1:1 übernehmen.
7. Regressions-/Eigenschaftstests und Benchmarks gegen Sagax-Referenz; überprüfte deterministische Eigenschaften behalten.
8. Neue, benannte Integrationsprofile für World/Movement/Physics/Animation schaffen, aber in `knorke-integration`, nicht im KI-Kern.

**Exit:** `knorke-navigation`, `knorke-perception` und `knorke-ai` laufen jeweils ohne Grafik/Runtime, können aber gemeinsam einen NPC mit erklärbaren Entscheidungen steuern. Alte Sagax-APIs dürfen ersetzt werden; die Umstellung wird dokumentiert.

## 7. Grafikbibliothek und gemeinsamer GPU-Layer

Da der konkrete Grafikcode noch nicht vorliegt, **erst auditieren**, anschließend entscheiden. Ziel: **Renderer, Beleuchtung und Host müssen dieselben GPU-Ressourcen verwenden können**, während der Renderer und die Beleuchtung jeweils standalone funktionieren.

### 7.1 Auditfragen

- Nutzt sie SDL_Renderer, SDL_GPU oder eine andere API? Welche Sprache und Toolchain?
- Wer erzeugt Device, Swapchain, Fenster, Command Buffer und Render Targets?
- Wie sind Shader, Material, Sprite, Text, Tilemap, Kamera und Asset-Import organisiert?
- Welche Farbraum-/Alpha-/Koordinaten-/Auflösungsannahmen gelten?
- Gibt es unnötige CPU↔GPU-Kopien, doppelte Cache-/Resource-Verwaltung, globale Zustände?
- Welche Tests, Screenshots, Benchmarks, ABI-/Packaging-Verträge existieren?

### 7.2 Zielverantwortlichkeiten

| Paket | Verantwortlich für | Nicht verantwortlich für |
|---|---|---|
| `knorke-gpu` (falls gerechtfertigt) | Device-Ownership, Texture-/Buffer-Handles, Command-/Pass-Verträge, Synchronisationsregeln | Spielobjekte, KI, Beleuchtungsalgorithmen |
| `knorke-render` | Szenen-Zeichenbefehle, Sprites/Tilemaps, Kameras, Material-/Buffer-Ausgabe | KI-/Agentenlogik, Lichtspezialeffekte |
| `knorke-lighting` | Licht, Schatten, Belichtungs-/Postpasses, optionale Materiallicht-Verarbeitung | Spielregeln, Eingabe, Welthierarchie |
| `knorke-integration` | Pass-Reihenfolge, Target-Bindings, Szenen-/Gameplay-Verknüpfung | Niedrige Rendering-/Lichtalgorithmen |

**Render-Graph?** Nicht sofort einen universellen Graph bauen. Zuerst einen **kleinen expliziten Pass-Plan** mit Ressourcenabhängigkeiten, Debug-Inspektion und nachvollziehbarer Scheduling-Reihenfolge. Erst bei nachgewiesenem Bedarf auf einen größeren Render-Graph erweitern.

**Exit:** `render-only`, `lighting-only` und `render+lighting` funktionieren unabhängig/zusammen ohne GPU→CPU→GPU-Umweg und ohne versteckte Present-/Submit-Aufrufe.

## 8. Agentische Architektur: dieselbe Domäne, mehrere Zugänge

```
       CLI         MCP          Rust SDK
         \          |           /
          \         |          /
          Authoring & Execution Service
             |  typed operations
        Scene / Schema / Validation
             |  revisioned state
      Runtime / Modules / Diagnostics
```

### 8.1 Operationen v1

| Operation | Zweck |
|---|---|
| `discover` | Pakete, Fähigkeiten, Komponenten, Schemas, Einschränkungen entdecken |
| `inspect` | Szenen, Assets, Ressourcen, Integrationsprofile und Zustände untersuchen |
| `query` | Filter-, Aggregat- und Detailabfragen statt unkontrollierter State-Dumps |
| `mutate` | Typisierte Änderungen als versionierte Transaktion am **Authoring State** |
| `validate` | Schema-, Referenz-, Ressourcen-, Abhängigkeits- und Constraint-Prüfung |
| `execute` | Build, Simulation, Test, Render, Screenshot, Szenario |
| `observe` | Diagnosen, Traces, Snapshots, GPU-/CPU-Statistiken, Bildartefakte |
| `verify` | Explizite Akzeptanzkriterien und Qualitätsbudgets prüfen |
| `checkpoint` / `diff` / `restore` | Änderungen vergleichen, übernehmen oder rückgängig machen |

### 8.2 Authoring ≠ Runtime

- **Authoring State:** persistente Szenen-/Objekt-/Material-/Gameplay-Definitionen; revisioniert und transaktional änderbar.
- **Runtime State:** konkrete Simulationszustände, temporäre Handles, GPU-Resources, Entscheidungen, Animationen und Events.
- Authoring-Transaktionen werden **atomar an einer Version** veröffentlicht. Runtime-Änderungen werden kontrolliert an Tick-/Frame-Grenzen angewendet; GPU-Ausführung und externe Nebenwirkungen sind nicht global rückrollbar.
- Agenten sollen Änderungen zunächst im Branch/Checkpoint probieren, Tests ausführen und **erst danach** eine neue stabile Revision veröffentlichen.

### 8.3 Maschinenlesbares Capability-Manifest

Jedes öffentliche Modul liefert ein Manifest mit:

- Modulkennung, SemVer, Plattformen, Abhängigkeiten, verfügbare Features.
- Typisierten Komponenten/Schemas, Defaultwerten, Einheiten, zulässigen Bereichen.
- Operationsbeschreibungen, Vor-/Nachbedingungen, Fehlercodes, Nebenwirkungen.
- Integrationsprofilen und erforderlichen Capabilities; keine hardcodierten Sagax-/Chiaro-Namen.
- Test-/Beispielreferenzen sowie verfügbaren Debug-/Screenshot-/Profiling-Fähigkeiten.

**Beispiel eines konzeptionellen Agenten-ChangeSets** (noch keine implementierte API):

```json
{
  "operation": "mutate",
  "base_revision": "r42",
  "transaction_id": "guard-with-light-01",
  "changes": [
    {
      "kind": "entity.create",
      "id": "guard.01",
      "archetype": "knorke.character.guard/1"
    },
    {
      "kind": "component.set",
      "entity": "guard.01",
      "component": "knorke.ai.utility/1",
      "values": { "profile": "guard-patrol" }
    }
  ],
  "postconditions": [
    "scene.references.valid",
    "simulation.guard-patrol.passes"
  ]
}
```

Operationen werden mit kleinen, gezielten Rückgaben versehen: Fehlercode, betroffener Pfad, erwartete Werte, relevante Artefakt-IDs und mögliche Korrekturen. Keine Megabyte Dumps als Standard.

### 8.4 Prüfungs- und Korrekturloop

`Discover → Plan → Mutate → Validate → Simulate → Observe → Verify → Commit / Restore`

Drei Ebenen: **strukturell** (Schemas/Referenzen), **funktional** (Simulation/Invarianten/Replay), **visuell** (Screenshot, Bilddiff, gegebenenfalls Modellbewertung). Visuelle Heuristiken ersetzen keine exakten Funktionstests. Determinismus nur für entsprechend definierte CPU-/Simulationspfade versprechen, nicht bitidentisches GPU-Rendering über alle Treiber.

## 9. Ziel-Workspace (nach Umstrukturierung)

```text
knorke/
├── Cargo.toml                    # gemeinsamer Workspace; Crates separat nutzbar
├── crates/
│   ├── knorke-types/
│   ├── knorke-math/              # nur wenn fachlich gerechtfertigt
│   ├── knorke-gpu/               # Ergebnis Render-/Lighting-Spike
│   ├── knorke-render/
│   ├── knorke-lighting/
│   ├── knorke-navigation/
│   ├── knorke-perception/
│   ├── knorke-ai/
│   ├── knorke-scene/
│   ├── knorke-runtime/
│   ├── knorke-integration/
│   ├── knorke-observe/
│   ├── knorke-verify/
│   ├── knorke-agent/
│   ├── knorke-cli/
│   ├── knorke-mcp/
│   ├── knorke-ffi/               # nur benötigte Bindings
│   └── knorke-wasm/              # nur benötigte Bindings
├── examples/
│   ├── ai-only/
│   ├── navigation-only/
│   ├── lighting-only/
│   ├── renderer-only/
│   ├── renderer-plus-lighting/
│   └── agent-built-2d-game/
├── tests/
│   ├── contracts/
│   ├── integration/
│   ├── visual/
│   └── performance/
├── tools/
├── docs/
│   ├── adr/
│   ├── schemas/
│   ├── migration/
│   └── architecture/
└── reference/                  # nur Referenzartefakte, keine Runtime-Pflicht
```

**Build-Disziplin:** Jedes standalone Crate lässt sich mit minimalen Features bauen. Headless-KI erfordert **keinen** GPU-Treiber, SDL3-Build und keinen C++-Compiler. Renderer + Lighting nutzt das GPU-Backend; CLI/MCP wird nur bei Bedarf gebaut. `cargo test --workspace` testet den gesamten Verbund; zusätzlich pro Crate Minimal-Feature-Matrix.

## 10. Implementierungsphasen mit Entscheidungspunkten

### Phase 0 – Baselines, Benennung, Integrationsanforderungen

**Aufgaben:**

1. Sagax, Chiaro und Grafikbibliothek vollständig inventarisieren: Features, APIs, interne Module, Abhängigkeiten, Benchmarks, Tests, Lizenz-/Build-Fragen.
2. Bestehende Releases/Commits als Referenz taggen; Build- und Bildbaselines erzeugen (Windows; Linux sofern möglich).
3. Gemeinsame Problemstellen herausarbeiten: GPU-Ownership, World-/Positionsdaten, `unsafe`/FFI, Komponentenlebensdauer, Diagnose, Nomenklatur.
4. Gewählten Produktnamen **Knorke Engine** als ADR dokumentieren; Crate-/Domain-/Markencheck durchführen und die Namenskonvention `knorke-*` validieren.
5. Architektur-ADRs: `0001-modularity`, `0002-gpu-and-render-lifecycle`, `0003-simulation-and-time`, `0004-schema-and-capabilities`, `0005-rust-rewrite-policy`.

**Exit:** belastbare Bestands-/Feature-Matrix, benannte Modulgrenzen, CI-Baselines und offene Architekturfragen mit Entscheidungen/Spikes; kein erzwungener Bestandsschutz.

### Phase 1 – Zwei technische Integrationsspikes

**Spike A: Renderer + Lighting:**

- Vorhandene Grafiklib mit Chiaro über gemeinsames SDL3-GPU-Device integrieren, um tatsächliche Kopplungsprobleme zu messen.
- Parallel minimalen Rust-GPU-/Lighting-Pfad für eine identische Referenzszene bauen.
- Vergleichen: Frame-/Pass-Zeiten, CPU-/GPU-Kopien, Ressourcenverwaltung, Code-Komplexität, visuelle Differenzen, Fehlerfälle.
- **Gate:** Rust-Rewrite von Chiaro beauftragen oder C++ zeitweilig als austauschbares Backend beibehalten.

**Spike B: KI + Simulation:**

- Sagax in einem headless Rust-Host verwenden; Navigations-/Entscheidungsdaten über Query-/Batch-Verträge austauschen.
- Engpässe und unpassende Crate-/Typgrenzen identifizieren.
- **Gate:** konkrete Neuaufteilung von `knorke-navigation`, `knorke-perception`, `knorke-ai` festlegen.

**Exit:** eine funktionierende GPU-Referenzszene, ein headless KI-Szenario, Messungen und dokumentierte Entscheidungen.

### Phase 2 – Module refactoren / migrieren

- `knorke-render` aus Grafikbibliothek ableiten oder gezielt neu entwerfen.
- `knorke-lighting` entlang des gewählten Rust-/Backend-Wegs entwickeln; Chiaro als Feature-/Bildreferenz.
- Sagax in neue fachliche Pakete aufteilen; Algorithmen und Regressionsfälle übernehmen.
- Kleinste nötige gemeinsamen Verträge und optionale Integrationsprofile implementieren.
- Standalone-, ABI-/WASM- und kombinierte Tests aufbauen. Historische APIs dürfen nach dokumentiertem Migrationspfad entfallen.

**Exit:** `ai-only`, `navigation-only`, `render-only`, `lighting-only` und `render+lighting` funktionieren; kein zyklischer Paketgraph, keine Runtime-Pflicht.

### Phase 3 – Knorke Authoring, Schema und minimale Runtime

- `knorke-scene`: versionierte Entity-/Component-/Prefab-/Asset-Definitionen und typisierte Änderungen.
- `knorke-runtime`: Clock, Fixed Tick, geordnete Systems, Ressourcen-/Entity-Lifecycle und sichere Synchronisationspunkte.
- `knorke-agent` + **CLI zuerst**: `discover`, `inspect`, `mutate`, `validate`, `execute`, `observe`, `diff`, `restore`.
- `knorke-integration`: Grafik/Lighting und Movement/Navigation/AI verbinden, mit transparenten Datenfluss-/Scheduler-Regeln.

**Exit:** Ein Agent kann per CLI eine Scene anlegen/ändern, Fehler mit Feldbezug erhalten und eine Headless-/Render-Simulation starten.

### Phase 4 – Spielbarer End-to-End-Vertical-Slice

- Kleine isometrische 2D-/2.5D-Szene: Spielfigur, Wachmann, Hindernisse, Licht und Schatten, Navigation und Entscheidungen.
- Die Welt wird **deklarativ über CLI** aufgebaut, nicht durch händisches Editieren des Engine-Codes.
- NPC beobachtet, plant, bewegt sich; Lichtsystem rendert die Szene; Diagnosen erklären die Entscheidungen.
- Screenshots, feste Testinputs, Replay, State-Snapshots und Profiling integrieren.

**Exit:** Eine agentisch erstellte, spielbare Szene reproduzierbar testen und gezielt korrigieren; erste End-to-End-Performance-Messung.

### Phase 5 – MCP, Selbstprüfung und sichere Agenteniteration

- MCP als dünne Schicht über den bereits getesteten fachlichen Service.
- Capability Discovery einschließlich Schemaversionen, Constraints und Integrationsprofilen.
- Agent-Loop mit Änderungsbudgets, kleinen Transaktionen, Simulation, strukturierten Fehlern, Bild-/Zustandsartefakten und gezieltem Rollback.
- Verifikation: `unit`, `integration`, `deterministic replay`, `visual diff`, `performance budgets`, `compatibility`.

**Exit:** Ein Agent kann eine definierte Feature-Anforderung ohne manuelle Engine-Codeänderung durch Plan → Änderung → Test → Diagnose → Korrektur lösen.

### Phase 6 – Component Development Kit und Ökosystem

- Agent kann ein **neues Modul oder eine neue Komponente** anlegen, Test-/Capability-Verträge erstellen, kompilieren und registrieren.
- Plugin-/Feature-Versionierung, Kompatibilitätsprüfung, Sandbox/Isolation für nicht vertrauenswürdige Erweiterungen evaluieren.
- Weitere Module (Audio, Physik, UI, 3D, Networking) nach realem Bedarf und nach denselben Standalone-/Integrationsregeln ergänzen.

**Exit:** Eine neu entwickelte Capability funktioniert standalone und im Verbund ohne Änderungen an bestehenden Fachmodulen.

## 11. Test- und Qualitätsmatrix

| Prüfung | KI/Navigation | Rendering | Lighting | Komplettes Knorke |
|---|---|---|---|---|
| Unit-/Property-Tests | Pflicht | Pflicht | Pflicht | Pflicht |
| Standalone-Ausführung | Pflicht | Pflicht | Pflicht | CLI + Runtime |
| ABI-/WASM-Bindings | optional, wo angeboten | optional | optional | optional |
| CPU-Replay/Determinismus | für deklarierte Pfade | nicht pauschal | Zeitsteuerung prüfen | Pflicht für definierte Simulation |
| GPU-/Golden-Scene-Vergleich | – | Pflicht | Pflicht | Pflicht |
| Performance-Regression | Navigations-/KI-Kosten | Frame-/Draw-Pass | Light-/Shadow-Pass | End-to-End |
| Fehler-/Lifecycle-/Leak-Tests | Pflicht | Pflicht | Pflicht | Pflicht |
| Agent-Schema-/Contract-Tests | Pflicht für exponierte APIs | Pflicht | Pflicht | Pflicht |

**Messgrößen:** p50/p95-Framezeit, CPU-Tickdauer, GPU-Passzeiten, Speicherkosten, Allokationen in Hot Paths, Ressourcenanzahl, Compile-/Buildzeiten, Datentransfer und FFI-Kosten, Anzahl Validierungsfehler pro Testfall, reproduzierbare Bildabweichungen. **Budgets erst nach Baselines pro Szene/Hardware festlegen**, nicht frei erfinden.

**Technische Regeln:**

- No-copy soweit möglich; kein Versprechen allgemeiner Zero-Copy bei GPU-/Format-/Backend-Unterschieden.
- Keine heap allocations pro Tick/Frame in zeitkritischen Pfaden, soweit sinnvoll und testbar.
- Keine zyklischen Crate-/Modulabhängigkeiten; CI prüft verbotene Abhängigkeitskanten.
- Rendering-Bildeinheitlichkeit nur innerhalb definierter Geräte-/Backend-/Toleranzprofile prüfen.
- `unsafe`-Code, FFI und GPU-Lifetimes gezielt prüfen, dokumentieren und testen.
- Ein **neuer Component-Typ** erhält Schema, Validierung, Beispiele, Diagnostik und Tests zusammen mit dem Code.

## 12. Risiken und Gegenmaßnahmen

| Risiko | Gegenmaßnahme |
|---|---|
| **Rust-Rewrite verliert Chiaro-Fähigkeiten** | Feature-Matrix, Golden Scenes, per Feature Portierung, alte Implementierung als Vergleich |
| **Gemeinsame GPU-Abstraktion wird zu groß** | Minimaler SDL3-GPU-Layer; erst aus gemessenen Integrationsanforderungen extrahieren |
| **Stand-alone-Module werden wieder heimlich an Engine gekoppelt** | Dependency-Regeln und isolierte Crate-Tests in CI |
| **Sagax-Umbau beschädigt deterministisches Verhalten** | Referenzcases, Seeds, Property-/Differentialtests, Benchmark-Baselines |
| **Einheitliche Namen erzeugen schlechte Fachgrenzen** | Nach Verantwortlichkeiten schneiden, nicht mechanisch Crates umbenennen |
| **Migrationsaufwand explodiert** | Vertical Slices und Decision Gates statt parallelem Komplettumbau |
| **GPU-Lifecycle/FFI verursacht Abstürze** | Host-Ownership, klare Command-/Device-Regeln, kleine `unsafe`-Grenzen |
| **Agentenschicht wird Runtime-Pflicht** | Feature- und Package-Grenzen, CLI/MCP außerhalb der Runtime |
| **Schemas verschleiern Performance-Kosten** | Compile-/Bind-Zeit vs. Tick/Frame getrennt; native Batch-/Query-APIs |
| **Agenten erzeugen unprüfbare Änderungen** | Transaction/Checkpoint, Verifikation vor Commit, maschinenlesbare Fehler |
| **Umbenennung bricht alte Verbraucher** | Zielmigration explizit kommunizieren, Adapter nur zeitlich begrenzt sofern nötig |

## 13. Erstes ausführbares Arbeitspaket (priorisiert)

> **Stand 2026-11-23:** Punkte 1 (Sagax, Chiaro; Grafikbibliothek fehlt), 2 (Inventuren), 3 (ADR 0000, Markenprüfung offen), 4 (ADR 0002, vorläufig), 9 (Workspace mit Basis-Crates) und 10 (CI) sind begonnen bzw. erledigt – Details in Abschnitt 16. Die Punkte 5–8 (Spikes, Chiaro-Entscheidung, Sagax-Neuschnitt) folgen nach den Grundlagen.

1. **Alle drei Quellbasen identifizieren und sichern:** Git-Revisions, relevante Tests/Benchmarks, Bild-/Output-Referenzen; insbesondere das noch unbekannte Grafiklib-Repository ermitteln.
2. **Feature- und Dependency-Graph erstellen:** Was muss erhalten bleiben? Was ist nur internes Implementierungsdetail? Welche Funktionen sollten zwischen Rendering und Lighting aufgeteilt werden?
3. **Namens-ADR schreiben:** **Knorke Engine** als gewählten Produktnamen dokumentieren; Marken-, Domain- und Paketnamen prüfen und `knorke-render`, `knorke-lighting`, `knorke-navigation`, `knorke-ai`, `knorke-perception` als Zielbegriffe festlegen.
4. **Rust-/GPU-Ownership-ADR schreiben:** Wer besitzt `SDL_GPUDevice`, Texturen, Frame/Command Buffer, wer submittet, wer präsentiert?
5. **Renderer+Chiaro-Referenzspike erstellen:** Gemeinsames Device, Offscreen-Capture, Performance-/Bildbaseline; den bisherigen C++-Stand als Vergleich verwenden.
6. **Rust-Lighting-Minispike erstellen:** Eine identische Light-/Shadow-Szene nativ in Rust; Shader/Assets wo sinnvoll wiederverwenden; messen statt spekulieren.
7. **Entscheidung über Chiaro-Port festhalten:** Rust wird bei erfülltem Spike zur Zielimplementierung; andernfalls konkrete Blocker und Alternativplan dokumentieren.
8. **Sagax-Neuschnitt entwerfen:** `navigation`, `perception`, `ai` und deren minimale gemeinsame Verträge; headless Referenzvergleich aufsetzen.
9. **Einheitlichen Rust-Workspace für die Zielmodule aufbauen:** keine Vollruntime; zuerst standalone Crates und kombinierte Beispiele.
10. **CI-Kriterien festlegen:** Standalone-Builds, Dependency-Graph, GPU-/Headless-Regression, Daten-/Versionsschemavalidierung.

**Explizit noch nicht beginnen:** Großes generisches ECS, 3D-Renderer, kompletter GUI-Editor, Multiplayer, viele MCP-Tools, automatische Codegenerierung ohne Prüfsystem, Portierung aller Chiaro-Spezialeffekte auf einmal.

## 14. Definition of Done – Knorke 0.1

- **Konsistente neue Namensfamilie** für öffentliche Module und Schemas ist dokumentiert; historische Namen sind migriert oder eindeutig als Referenz gekennzeichnet.
- **Rust-first-Integration** ist implementiert; Entscheidung zu Chiaro-Rust ist mit Messungen belegt und der für v0.1 erforderliche Beleuchtungspfad funktioniert.
- **Standalone:** Render, Lighting, Navigation und KI können außerhalb der Knorke-Runtime verwendet werden.
- **Komposition:** Renderer + Lighting arbeiten auf einem gemeinsamen GPU-Gerät ohne Standard-CPU-Readback; KI + Navigation können den World State ohne Vollkopie abfragen.
- **Agentisch:** Ein Agent kann eine kleine 2D-Szene über typisierte CLI-Operationen anlegen, verändern, validieren und starten; MCP bietet denselben Service, sobald Phase 5 abgeschlossen ist.
- **Beobachtbar:** KI liefert Entscheidungs-/Navigationstraces, Grafik liefert Screenshots und Frame-Statistiken, Fehler sind strukturiert.
- **Prüfbar:** deterministischer CPU-Replay für definierte Pfade, GPU-Bildvergleich mit festgelegten Toleranzen, Integrations- und Performance-Regressionschecks.
- **Transaktional:** Ungültige Authoring-Changes publizieren keine Teiländerungen; Checkpoint/Restore und Versionskonflikte werden überprüft.
- **Sauber paketiert:** Release-Spiel benötigt weder MCP-Server noch Agentenruntime; keine verbotenen Abhängigkeitskanten.
- **Bewusst kein Bestandsschutz:** Keine Pflicht zur Aufrechterhaltung aller alten C++-/Sagax-APIs. Fachliche Qualität und dokumentierte Migration sind das Maß.

## 15. Quellen und nächste Architekturentscheidungen

**Gesichtete Bestandsquellen:**

- Sagax: https://github.com/STARSCrazy/sagax
- Sagax Workspace: https://github.com/STARSCrazy/sagax/blob/main/Cargo.toml
- Sagax Roadmap: https://github.com/STARSCrazy/sagax/blob/main/docs/ROADMAP.md
- Chiaro: https://github.com/STARSCrazy/chiaro
- Chiaro CMake: https://github.com/STARSCrazy/chiaro/blob/main/CMakeLists.txt
- Chiaro Integration: https://github.com/STARSCrazy/chiaro/blob/main/docs/INTEGRATION.md
- Chiaro Architektur: https://github.com/STARSCrazy/chiaro/blob/main/docs/ARCHITECTURE.md

**Offene, absichtlich nicht erfundene Fakten:** Exakte Grafikbibliothek und ihre Implementierung; tatsächliche GPU-/Rust-Profilingwerte; Resultat des Chiaro-Rust-Spikes; rechtliche und praktische Verfügbarkeit des gewählten Namens und der geplanten Paketnamen; Umfang der beizubehaltenden externen Verbraucher; Plattform-Support für v0.1.

**Kurzfassung der Strategie:** Ein **konsistentes Rust-Ökosystem** ist das Ziel, kein Netzwerk von dauerhaft zusammengeflickten Bestandsbibliotheken. **Portieren, refactoren, zusammenführen und umbenennen ist ausdrücklich erlaubt.** Dennoch bewerten wir jede große Änderung anhand eines funktionierenden Vertical Slice, Regressionstests und realer Messdaten statt eines unkontrollierten Big Bang.

## 16. Umgesetzter Stand (Phase 0, 2026-11-23)

Details in den ADRs unter `docs/adr/`, Inventuren unter `docs/inventory/`.

| Baustein | Crate | ADR |
|---|---|---|
| Handles, `Slots`, `HandleMap`, `BitSet` | `knorke-types` | 0001, 0006, 0008 |
| Geliehene Massendaten: `Column`/`ColumnMut`, `project` | `knorke-types` | 0006 |
| `EventQueue`, `CommandBuffer`, `Revision` – Kommunikation ohne Broker | `knorke-types` | 0008 |
| `Parallel`-Vertrag + `Serial`; Executoren `Rayon`, `ScopedThreads` | `knorke-types`, `knorke-exec` | 0008 |
| `Tick`, `Seed`, `StableHasher` – Determinismus | `knorke-types` | 0003, 0008 |
| `Diagnostic` mit stabilen Codes | `knorke-types` | 0004 |
| Vektoren/Matrizen/Farben mit festem Layout, Koordinatenräume als Typen | `knorke-math` | 0006 |
| `Sink`/`Probe`/`Inspect`, `Recorder`, Export Perfetto/JSON | `knorke-observe` | 0007 |
| Vertragstests (keine Kopie, 0 Allokationen auf allen Threads, bitgleich über Executoren), Benchmarks | `knorke-contract-tests` | 0006, 0008 |
| Abhängigkeitsregeln und Standalone-Builds in CI | `xtask` | 0001 |
| Frame-Ablauf/Scheduler (nur Entwurf) | – | 0009 |
| Snapshot-Vertrag (`Snapshot`, `SnapshotBuf`, `SnapshotValue`), `Checksum` pro Modul-Abschnitt | `knorke-types` | 0010 |
| Replay: `Recorder`/Flight Recorder, `verify` (Tick + Modul), `seek`, `minimize` (Delta-Debugging) | `knorke-replay` | 0010 |
| Fehlererkennung: `Invariants`, `ProgressWatch`, Bildheuristiken (`frame_diff`, `flicker_score`, `non_finite`) | `knorke-observe` | 0010 |
| Geometrie `Aabb2`, `Circle`, `Segment2`, `Capsule2`, `Ray2`; Snapshot-Unterstützung aller Mathe-Typen | `knorke-math` | 0010 |
| Projektregeln für Agenten und Menschen | `CLAUDE.md` | – |
| Plattformunabhängige Mathematik `det` (libm), sRGB-Tabellen, Clippy-Verbot plattformabhängiger Funktionen/Uhren/Zufalls-HashMaps | `knorke-math`, `clippy.toml` | 0011 |
| Determinismus-Fingerprint (eingefroren; CI: Windows, Linux, macOS/ARM, WASM) | `knorke-contract-tests` | 0011 |
| Abschaltbare Zusatzfeatures ohne Kosten: `TickDriver`/`NoRecord`, nullgroße No-op-Typen, Messung „aus vs. nicht vorhanden“ | `knorke-replay`, `knorke-contract-tests` | 0011 |
| **Review der Grundlagen (2026-11-23)**, jeweils mit Test: `EventQueue` (Cursor hinter dem Ende → `Resync`, `keep_ticks` an der Tick-Obergrenze, `EventCursor` als `SnapshotValue`, Restore prüft Tick-Reihenfolge); `Slots`-Restore prüft die Freiliste als Kette; `HandleMap::insert` über ein veraltetes Handle überschreibt den Nachfolger nicht, `HandleMap: Clone`; `Vec<Vec<T>>`-Restore behält innere Puffer; NaN kanonisch im Snapshot; `BitSet`-Gleichheit nach Inhalt; `Column::iter` wird durch `&C` weitergereicht; `Parallel::for_each_range*` für Columns | `knorke-types` | 0006, 0008, 0010 |
| Review: `det::min`/`det::max` (std-Varianten sind nicht deterministisch, gemessen Debug ≠ Release), `det::powi` für alle Exponenten, Schnitt-Tests unabhängig vom Maßstab der Welt, definierte Antworten für entartete Formen, `Pod`-Impls aus Feldlisten geprüft, Clippy-Regeln erweitert | `knorke-math`, `clippy.toml` | 0006, 0011 |
| Review: `ReplayLog::read` prüft Tick-Folge und Keyframes, Index-Überlauf als Panic statt Umbruch, `seek` sucht binär; Tests für Fensterränder des Flight Recorders und 1-Minimalität | `knorke-replay` | 0010 |
| Fingerprint auf `wasm32-wasip1` lokal belegt (Node/V8, `xtask/wasi-run.mjs`) | `knorke-contract-tests`, `xtask` | 0011 |
| **Sagax-Umbau** (Sagax `fab00c0`): Raumtypen `Cell`/`Pos`/`HexLayout`/`sine` → `knorke-grid`; Wegsuche (A*, Dijkstra, D* Lite, Flow Fields, Glättung, Formationen, `Navigator`) → `knorke-navigation`; Sicht, Hören, Awareness, Contagion → `knorke-perception`; Utility-Decider, State Machine, Gedächtnis, Missionen, Crew/Squad → `knorke-ai`; Verbindungscode der Fassade + 13 Szenario-Tests → `knorke-integration`; `Collector` → `knorke-observe`. Traces über `knorke_observe::Sink`, Zufall über `Seed::derive`. Snapshot: `Navigator` (Abschnitt `knorke.navigation`, auch mitten in einer Budget-Suche), `Search`, `FlowField`, `Awareness`, `Contagion`, `StateMachine`, `Knowledge`, `Alertness`; Replay-Test je Modul; Diagnosecodes `knorke.ai.*`; Abhängigkeitsregeln (die drei Fachmodule unabhängig, `knorke-grid` nur `knorke-types`). Alle 595 Tests der übernommenen Sagax-Crates laufen (Baseline Sagax: 785 bestanden, 1 ignoriert, inkl. FFI) | `knorke-grid`, `-navigation`, `-perception`, `-ai`, `-integration`, `knorke-observe`, `xtask` | 0001, 0010, 0012 |
| **pixelraum migriert** (ADR 0013, Inventur `pixelraum.md`): Software-Renderer (Raster, Kamera, Gitter, Texturen, Sprites, Picking, Palette, Blending, Animation, Partikel, Hintergrund) ohne Licht und Threads; Licht/Schatten auf geliehenen G-Buffer-Spalten über `Parallel`; Verbindung in `knorke-integration::pixel`. 55 Golden Images byte-gleich, alle Bildtests pixelgenau; Vertragstest „0 Allokationen im eingeschwungenen Frame“ (seriell) | `knorke-render`, `knorke-lighting`, `knorke-integration`, `knorke-contract-tests` | 0006, 0008, 0013 |
| **Lineares Licht** im Software-Renderer (sRGB-Tabellen, Licht/Nebel/Blending linear, sRGB einmal am Ende; 51 Golden Images neu) und **Snapshots aller KI-Zustandstypen** (2026-11-23) | `knorke-render`, `knorke-lighting`, `knorke-navigation`, `knorke-ai` | 0012, 0013 |
| `knorke-integration` mit Features je Profil: `ai` (Sagax-Verbindungscode, ADR 0012) und `pixel` (Renderer + Licht, ADR 0013), beide Standard; ein Host zieht nur die Module, die er verbindet. Tests und Beispiele hängen am jeweiligen Feature | `knorke-integration` | 0001, 0012, 0013 |
| Entscheidungen 2026-11-23: `HashMap`/`HashSet` in Simulationscode verboten (Clippy-Typverbot); `Seed::derive` mit zwei Mischrunden, Fingerprint neu `0x8E13_81FE_2691_7217`; Lizenz `MIT OR Apache-2.0` mit Lizenzdateien; kein Browser-Ziel | `clippy.toml`, `knorke-types` | 0003, 0011 |

Entscheidungen aus Gesprächen:
- Sagax und Chiaro werden **erst nach den Grundlagen** umgebaut.
- Chiaro wird **später in Rust neu gebaut** (`knorke-lighting`). Die Chiaro-Pfade mit eigenem Submit/Present/Readback werden nicht übernommen (ADR 0002). Ob der Vergleichs-Spike mit dem C++-Chiaro (Weg A, Abschnitt 5) noch nötig ist, ist offen.
- **Kein Browser als Plattformziel** (2026-11-23). WASM bleibt Prüfziel für den Determinismus und Kandidat für Mods (17.7), nicht Auslieferungsziel.
- Kompatibilität zu Sagax-Ergebnissen ist in Phase 0 nicht nötig; nach dem Sagax-Umbau sind Seed-Ableitung und Fingerprint eingefroren.
- Abhängigkeiten der Basis-Crates sind geschlossene Listen: `knorke-types` keine, `knorke-math` nur `bytemuck`/`mint`/`knorke-types`/`libm`, `knorke-observe` und `knorke-replay` nur `knorke-types`, `knorke-exec` nur `rayon`.
- Chiaros Rust-Neubau kommt als **eigene Crate `knorke-light2d`** (mit `knorke-gpu`, `knorke-post`); GPU ist der Hauptweg, das CPU-Licht wird eingefroren (ADR 0014, ersetzt ADR 0013 §9).
- Befund Rayon: Aufrufe von außerhalb des Pools allozieren in Rayons Scheduler (ein Block pro 63 Aufträge). Unser Code alloziert nicht. Frames deshalb mit `Rayon::install` im Pool laufen lassen (ADR 0008).

## 17. Backlog: Ideen, Konzepte und noch fehlende Grundlagen

**Pflegeregel:** Jede neue Idee, jedes Konzept und jede vertagte Entscheidung kommt hierher, mit Status. Status: **offen** (noch nicht entschieden), **geplant** (Phase/Auslöser bekannt), **vorbereitet** (Grundlage existiert), **verworfen** (mit Grund). Erledigtes wandert nach Abschnitt 16.

### 17.1 Querschnitt – aus der Architekturdiskussion

| Idee | Status | Auslöser / nächster Schritt |
|---|---|---|
| **Snapshot/Restore des Simulationszustands** (Replays, Rollback-Netcode, Time-Travel-Debugging) | erledigt (Abschnitt 16, ADR 0010) | beim Sagax-Umbau: jedes neue Modul implementiert `Snapshot` |
| **Task-Graph-Scheduler** (Systeme nach Lese-/Schreibzugriff parallel) | geplant | ADR 0009, mit den ersten 2–3 echten Systemen |
| **Pipelining** Simulation Tick N+1 ∥ Render Frame N | geplant | nach erstem Vertical Slice messen |
| **Handle-Reservierung aus parallelen Chunks** (atomarer Zähler in `Slots`) | offen | sobald ein System parallel spawnt |
| **Gemeinsamer Raum-Index** (Broadphase/Spatial Hash für Navigation, Wahrnehmung, Culling, Audio) | offen | wenn zwei echte Module ihn brauchen |
| **Frame-/Render-Graph** (Pässe mit deklarierten Ressourcen) | geplant | zuerst expliziter Pass-Plan in `knorke-gpu` (ADR 0002) |
| **GPU-getriebenes Rendering** (Culling/Sortierung per Compute) | offen | nach Identifikation der Grafikbibliothek; SDL3-GPU hat Compute |
| **GPU-Timings** | offen | SDL3-GPU-Möglichkeiten prüfen (ADR 0007) |
| **Hot-Reload** von Assets, Shadern, Parametern | geplant | mit `knorke-scene`, Anwendung an Sync-Punkten |
| **Entity-Relationen** („Kind von“, „zielt auf“) als eigene Struktur | offen | bei Bedarf in KI/Szenengraph |
| **Asset-System** (asynchrones Laden, Referenzzählung, Streaming) | geplant | Phase 3 mit `knorke-scene` |
| **Frame-Scratch-Allocator** | offen | nur wenn Messungen zeigen, dass Host-Puffer nicht reichen |
| **`knorke-abi`** (`repr(C)`-Slice-/Strided-Views für C/C#/WASM) | offen | erster FFI-Verbraucher |
| **Billigere Profiling-Uhr** (heute 68 ns pro Scope) | offen | wenn feinere Scopes gebraucht werden |
| **Lokale Verifikation Linux/WASM** | WASM erledigt (Node/V8 über `xtask/wasi-run.mjs`, gleicher Fingerprint); Linux/ARM offen | WSL-Ubuntu hat kein Rust; bis dahin prüft nur die CI |
| **Prüfsummen günstiger** (gemessen +10 % bei Prüfsumme jeden Tick in der Fingerprint-Welt; ein früherer Einzelwert von +22,6 % ließ sich nicht wiederholen) | offen | inkrementell pro Modul oder nur geänderte Abschnitte, falls `checksum_every = 1` im Alltag gebraucht wird |
| **Agenten-Transaktionen über `CommandBuffer::pending()`** validieren | vorbereitet | Phase 3/5 |
| **`RandomState` über `HashMap::default()`, `collect()`, `HashMap::from`** entging Clippy | erledigt (Abschnitt 16, ADR 0003) | `HashMap`/`HashSet` sind als Typen verboten; alle vier Formen werden gemeldet |
| **Hash-Map-Iterationsreihenfolge plattformübergreifend** nicht festgelegt | erledigt durch das Typverbot (ADR 0003) | – |
| **Schnelle Schlüssel-Tabelle für Simulationscode** (`BTreeMap::get` kostet bei 1 Mio. Einträgen 236–430 ns, eine Hash-Map 21–37 ns) | offen | erst bei gemessenem Bedarf: Lookup-Map ohne Iteration (Hash-Map hinter einer Schnittstelle, die keine Reihenfolge preisgibt) oder der gemeinsame Raum-Index |
| **`ColumnMut` parallel beschreiben** (`project_mut` lässt sich nicht in Chunks teilen; `for_each_chunk_mut` braucht `&mut [T]`) | offen | erstes Modul, das Ergebnisse parallel direkt in Host-Structs schreiben will; bis dahin Host-Puffer als Slice |
| **2D-Raster als geliehene Sicht** (`Grid<T>`: Breite, Höhe, Schrittweite) für Navigation, Wahrnehmung, Lighting | offen | sobald zwei Module dasselbe Raster lesen; Sagax `CellMap` ist ein Query-Trait, kein Massendaten-Vertrag |
| **Rechnen mit `At<S, V>`** (`At + At`, `map`) fehlt; Module greifen auf `.value`/`.raw()` zu | offen | mit dem ersten Fachmodul entscheiden, welche Operationen raumtreu sind (Punkt − Punkt = Vektor) |
| **`Seed::derive` vermischte Seed und Stream linear** | erledigt (Abschnitt 16, ADR 0003): zwei Mischrunden | eingefroren mit dem Sagax-Umbau; die KI-Module nutzen `Seed::derive` statt `sagax_core::mix` (ADR 0012) |
| **`normalize_or_zero` mit fester Schwelle** (`len > f32::EPSILON`): in Welten mit sehr kleinen Einheiten werden kurze Vektoren zu Null; ab ~1,8e19 läuft `length_squared` über | offen | Schnitt-Tests sind schon maßstabsunabhängig; hier erst mit einem Verbraucher ändern (ändert Simulationsergebnisse) |
| **Snapshot-Längen bei nullgroßen Elementen** (`Vec<()>` mit Länge 2³² kostet beim Lesen Milliarden Schleifendurchläufe ohne Daten) | offen | nur relevant für nicht vertrauenswürdige Replays; Obergrenze im `Reader` einführen, wenn Replays aus dem Netz kommen |
| **`minimize` kopiert den Start-Keyframe pro Kandidat** (`filtered` klont den Startzustand) | offen | bei großen Welten: Kandidat als Sicht auf das Original statt als eigenes Log |
| **`ProgressWatch` nach `seek`/Rollback** (gemerkter Tick liegt in der Zukunft, Meldungen bleiben bis dahin aus) | offen | Diagnosezustand beim Restore leeren oder mitsichern |
| **Snapshot-Vertrag in eigene Crate** | verworfen (vorerst) | Begründung ADR 0001 §5; neu bewerten, wenn Kompression oder Migration alter Versionen dazukommt |
| **Zu früh vorhanden** (kein Verbraucher): `Mat4`, `mint`-Feature, Bildheuristiken ohne Capture, `ScopedThreads` (0,8–0,95 ms Fork-Join-Overhead) | offen (Einschätzung) | behalten, solange sie nichts kosten; beim ersten Verbraucher prüfen, ob die Form passt |
| **Lizenz** | erledigt: `MIT OR Apache-2.0`, `LICENSE-MIT` + `LICENSE-APACHE` | gleiche Lizenz wie Sagax; Chiaro ist zlib (beim Übernehmen von Code Hinweis erhalten) |
| **C-Schnittstelle von pixelraum** (`capi.rs`, `include/pixelraum.h`) nicht migriert; Wizardry 7 nutzt weiter das Original-Repo | geplant | mit `knorke-abi` (C-ABI über `knorke-integration::pixel`); bis dahin Fehlerbehebungen ggf. in beiden Repos |
| **Lineares Licht im Software-Renderer** (Texturen → linear, Licht/Nebel linear, sRGB einmal am Ende) | erledigt (Abschnitt 16, ADR 0013 §6) | 51 Golden Images neu erzeugt; Tabellen liegen in `knorke-render/src/srgb.rs` statt in `knorke-math` (Verschieben offen); Frame ~10–20 % langsamer, auf belastetem Rechner gemessen – auf ruhigem Rechner nachmessen; Textur-Decode beim Shading statt beim Laden, weil der G-Buffer 8-Bit-sRGB speichert |
| **Golden Images plattformübergreifend** (heute nur Windows/x86-64 belegt; `sin`/`cos`/`tan`/`atan2`/`log2` aus `std` im Renderer) | offen | CI-Lauf auf Linux/ARM; bei Abweichung `det`-Varianten oder Golden Images pro Plattform (ADR 0013 §5) |
| **Band-Liste in `end_lit` mit `threads > 1`** (eine Allokation pro Frame; seriell 0) | offen | wiederverwendbare Bandstruktur, sobald `Parallel` geliehene Teilslices ohne `Vec` verteilen kann (vgl. „`ColumnMut` parallel beschreiben“) |
| **pixelraum-Beispiele** (`examples/demo.rs`, C-Beispiel) nicht migriert | offen | mit dem ersten Knorke-Beispielprogramm |
| **MSRV 1.88 nicht mit echter Toolchain geprüft** (lokal nur über Clippy `incompatible_msrv`, 0 Meldungen) | offen | erster CI-Lauf des `msrv`-Jobs |

### 17.2 Linux und weitere Plattformen

**Stand:** Alle Basis-Crates sind reines Rust ohne Plattformcode; `cargo check` für `x86_64-unknown-linux-gnu` läuft, CI baut Windows und Linux.

| Thema | Status | Was es braucht |
|---|---|---|
| Fenster, Eingabe, GPU, Audio auf Linux | vorbereitet | SDL3 abstrahiert das (Vulkan auf Linux, D3D12 auf Windows, Metal auf macOS); Shader pro Backend (SPIR-V/DXIL/MSL) – Chiaro erzeugt alle drei bereits |
| GPU-Tests auf Linux | offen | Chiaro hat keine Linux-Tests (Inventur); CI braucht einen Software-Vulkan-Treiber (lavapipe) für Headless-Bildtests |
| **Plattformübergreifender Determinismus** | **erledigt (ADR 0011)**, CI-Lauf auf Linux/macOS-ARM/WASM steht aus | Außer `sqrt` sind `f32::sin/cos/powf/exp` nicht garantiert bitgleich zwischen Plattformen und Bibliotheken. Für deterministische Pfade (Simulation, Netzwerk, Replay zwischen Rechnern): Ganzzahl/Fixpunkt (Sagax-Stil) oder eigene plattformunabhängige Implementierungen (z. B. `libm`). Entscheidung als ADR vor Netzwerk und Replay |
| Dateipfade/Assets | offen | Groß-/Kleinschreibung (Linux unterscheidet), `/` als Trenner, Asset-IDs statt Pfaden |
| Steam Deck, ARM (aarch64, macOS) | offen | CI-Matrix erweitern, sobald GPU-Code existiert |

### 17.3 Stabiles Netzwerkspiel

Zwei Modelle, beide brauchen dieselben Grundlagen:

- **Deterministischer Lockstep / Rollback** (Kampfspiele, RTS): nur Eingaben werden übertragen; jede Maschine simuliert identisch, bei Abweichung zurückspulen und neu simulieren.
- **Server-autoritativ mit Replikation und Vorhersage** (Action, Koop): Server simuliert, Clients sagen voraus und korrigieren.

| Grundlage | Status | Bemerkung |
|---|---|---|
| Fester Tick, Seeds, stabile Hash-Reihenfolge | vorhanden | ADR 0003, 0008 |
| Eingaben als Commands | vorbereitet | `CommandBuffer` – Netz-Eingaben sind Commands mit Tick |
| Bitgleich unabhängig von der Thread-Anzahl | vorhanden | Vertragstest |
| **Plattformübergreifend bitgleiche Mathematik** | erledigt (ADR 0011) | 17.2 – Voraussetzung für Lockstep/Rollback zwischen Windows und Linux |
| **Snapshot/Restore pro Modul** | erledigt (ADR 0010) | Netz-Rollback nutzt denselben Vertrag |
| **Zustands-Prüfsumme pro Tick** (Desync-Erkennung) | erledigt (ADR 0010) | pro Modul-Abschnitt; plattformübergreifend erst mit 17.2 |
| Netzwerk-IDs (über Maschinen stabile Entity-Identität) | offen | eigener Typ neben Authoring-ID und Runtime-Handle (ADR 0001 §7) |
| Delta-Kompression, Interessenbereiche (wer sieht was) | offen | baut auf `Revision`, `BitSet` und Raum-Index auf |
| Transport (UDP, Zuverlässigkeit, Verschlüsselung) | offen | eigenes Modul `knorke-net`, außerhalb der Simulation |

### 17.4 Animation

| Thema | Status | Bemerkung |
|---|---|---|
| Zeit: Tick + Interpolationsfaktor pro Frame | vorbereitet | ADR 0003/0009; Animation läuft in „Render vorbereiten“ |
| Sprite-/Frame-Animation, Kurven, Easing | offen | Modul `knorke-animation`; deterministisch, wenn Gameplay davon abhängt |
| Gameplay-Ereignisse aus Animationen (Treffer-Frame, Schritt) | vorbereitet | als `EventQueue`-Events |
| Skelett-Animation (2D-Bones), Blending | offen | nach Sprite-Animation |
| Integrationsprofil `character.motion.animation` | geplant | Abschnitt 4.2 |

### 17.5 Ingame-KI

| Thema | Status | Bemerkung |
|---|---|---|
| Weltabfragen über Query-Traits, Traces über `Sink` | vorbereitet | Sagax-Muster (`CellMap`, `SenseMap`) passt zu ADR 0006/0007 |
| Parallele KI über `Parallel`, Entscheidungen als Commands | vorbereitet | ADR 0008 |
| **Arbeitsbudget / Zeitscheiben** (KI über mehrere Ticks verteilen) | offen | Sagax hat `Budget`; als gemeinsamen Begriff prüfen |
| Gemeinsamer Raum-Index für Wahrnehmung | offen | 17.1 |
| Sagax-Neuschnitt (`navigation`, `perception`, `ai`) | erledigt (Abschnitt 16, ADR 0012) | `knorke-grid`, `-navigation`, `-perception`, `-ai`, `-integration` |
| **Bindings `sagax-abi`/`-ffi`/`-wasm`** (C-ABI 23,8 kLOC, WASM 4,8 kLOC) | verworfen (vorerst) | Plan 6.2 §6: nur für einen konkreten Verbraucher, dann neu gegen `knorke-*` entworfen (zusammen mit `knorke-abi`, 17.1); externe Verbraucher der Sagax-C-ABI noch nicht ermittelt |
| **Snapshot der übrigen KI-Zustandstypen**: `DStarLite`, `GoalGuess`, `Mission`, `Crew`, `Squad`, `GoalKeeper`, `Areas`, `Timers`, `Inbox`/`Outbox`, `Route`/`Progress`, `Spots`, dazu `Decider`, `Interval`, `Metric`, `GridRules` | erledigt (Abschnitt 16, ADR 0012) | alle als `SnapshotValue`, Restore ohne Allokation, ungültige Daten → `SnapshotError::Invalid`; Replay-Test erweitert |
| **Snapshot-Uneinheitlichkeit**: `Mission` muss in eine Mission mit gleicher `MissionConfig` zurück (`Mission::get` nur mit Standard-Config), `Squad`/`Crew` sichern ihre Kapazitäten mit | offen | vereinheitlichen, sobald ein Host Missionen mit eigener Config sichert; außerdem: Playbook-Plan als laufender Plan im Round-Trip-Test nicht abgedeckt |
| **`Parallel`-Vertrag in den KI-Modulen** | offen | Sagax rechnet pro Figur seriell; parallel heute nur der Host über Figuren. Kandidaten: `look` über viele Beobachter, Flow-Field-Abfragen, Decider über Figuren – mit Messung (Chunk ≥ 10 µs) |
| **`Column`-Eingaben statt Slices** (`&[Candidate]`, `&[Target]`, Decider-Eingabezeilen) | offen | ADR 0006; mit dem ersten Host, der diese Daten in eigenen Structs hält |
| **`Seed`/`Tick` in KI-Signaturen** (heute `u64`-Seeds, `u32`-Zeit wie in Sagax) | offen | API-Umbau ohne Ergebnisänderung möglich; zusammen mit `knorke-runtime` |
| **`Invariants`/`ProgressWatch` für KI-Module** (z. B. Squad hängt fest, Mission ohne Fortschritt) | offen | ADR 0010; mit dem ersten Vertical Slice |
| **Benchmarks gegen Sagax** | offen | Sagax hat keine Bench-Ziele; mit `cargo bench -p knorke-contract-tests` neu aufbauen |
| **`knorke-ai` weiter teilen** (`memory`, `mission`, `group` als eigene Crates) | offen | erst bei einem Verbraucher, der einen Teil ohne den Rest braucht (ADR 0012) |
| LLM-gestützte NPCs (Dialog, Absichten) | offen | nicht deterministisch und asynchron: nur außerhalb der Simulation; Ergebnisse kommen als Commands an einem Sync-Punkt herein und werden für Replays aufgezeichnet (17.10) |

### 17.6 Sound

| Thema | Status | Bemerkung |
|---|---|---|
| **Echtzeit-Audio-Thread** | offen – Grundlage fehlt | Audio hat harte Deadlines (wenige ms): kein Locking, keine Allokation im Audio-Callback. Braucht eine lock-freie Einzel-Produzent/Einzel-Konsument-Queue vom Spiel zum Audio-Thread (eigene oder `rtrb`) |
| Ausgabe | vorbereitet | SDL3-Audio-Streams; Mixer als eigenes Modul `knorke-audio` oder bestehende Crate (z. B. `kira`) prüfen |
| Räumlicher Klang (Position, Dämpfung, Verdeckung) | offen | Positionen aus `knorke-math`; Verdeckung/Ausbreitung kann das Hör-Modell von Sagax (`hear`, `SenseMap`) wiederverwenden |
| Abgrenzung | – | Audio ist Darstellung, nicht Simulation; was ein NPC hört, läuft über Wahrnehmung, nicht über den Mixer |

### 17.7 Erweiterbarkeit

| Weg | Status | Bewertung |
|---|---|---|
| Rust-Crates (statisch gelinkt, Traits) | vorhanden | schnellster Weg, Standard für eigene Module |
| **WASM-Module** (Sandbox, Ressourcenlimits) | offen | passt zur agentischen Idee: von Agenten erzeugte Spiellogik läuft isoliert (Plan Phase 6) |
| C-ABI-Plugins (DLL/SO) | offen | Rust hat kein stabiles ABI; nur über `knorke-abi` |
| Skriptsprache (Lua, Rhai) | offen | für Modder/Designer; Agenten brauchen sie nicht zwingend |
| **Modul-Lebenszyklus** (registrieren, Systeme anmelden, herunterfahren) | geplant | mit `knorke-runtime` |
| **Capability-Manifest, Schemas, stabile IDs** | vorbereitet | ADR 0004; Diagnosecodes sind schon stabil |

### 17.8 Physik

Plan-Prinzip 9: Knorke erzwingt keine Physik-Engine. Physik ist ein austauschbares Modul (eigene Implementierung oder z. B. `rapier2d`), angebunden über dieselben Verträge wie alle anderen Module.

| Thema | Status | Bemerkung |
|---|---|---|
| Fester Tick, Sub-Steps, Interpolation für die Darstellung | vorbereitet | ADR 0003/0009; Physik läuft in der Simulationsphase, Rendering interpoliert |
| Positionen/Geschwindigkeiten ohne Kopie austauschen | vorbereitet | `Column`/`project`, `At<World2, Vec2>`; Physik-Engines halten meist eigene gepackte Arrays, die direkt als `&[Vec2]` lesbar sind |
| Kollisionsereignisse (Beginn/Ende, Trigger) | vorbereitet | `EventQueue`, nie Callbacks während des Physik-Schritts |
| Kräfte, Impulse, Teleport von außen | vorbereitet | `CommandBuffer`, angewendet vor dem Physik-Schritt |
| **Gemeinsame Geometrie-Grundformen** (`Aabb2`, Kreis, Kapsel, Segment, Strahl + Schnitt-/Abstandstests) | erledigt (`knorke-math`, ADR 0010) | Nicht nur Physik: Navigation (Sichtlinien), Wahrnehmung (Sichtkegel, Verdeckung), Lighting (Schattenwerfer), Culling, Auswahl per Maus. Sonst definiert jedes Modul eigene AABBs und rechnet an den Grenzen um |
| **Abfrage-Vertrag „Physik-Welt“** (Raycast, Shape-Cast, Überlappung) | offen | Query-Trait, das KI und Wahrnehmung nutzen, ohne die Physik-Engine zu kennen; Rückgabe über Handles |
| Gemeinsamer Raum-Index (Broadphase) | offen | 17.1; Physik bringt meist einen eigenen mit – Abfragen über den Vertrag statt doppeltem Index |
| **Determinismus** | offen | Plattformübergreifend nur mit festgelegter Mathematik (17.2). Rapier bietet dafür ein eigenes Feature (`enhanced-determinism`) – bei der Auswahl prüfen. Parallele Physik nur mit fester Chunk-Aufteilung (ADR 0008) |
| Snapshot/Restore | vorbereitet | Physik-Modul implementiert `Snapshot` (ADR 0010); bei Fremd-Engines (Rapier) deren Serialisierung als Abschnitt einbetten |
| Durchtunneln schneller Objekte (CCD), Stabilität von Stapeln | offen | Aufgabe des Physik-Moduls; Invarianten (17.9) erkennen Objekte in Wänden |
| Integrationsprofil `world.obstacle.navigation` | geplant | Abschnitt 4.2: Änderungen an Hindernissen → inkrementelle Navigation über `EventQueue`/`Revision` |

### 17.9 Automatische Bug- und Glitch-Erkennung

Ziel: Fehler finden, bevor ein Mensch sie sieht, und jeden Fund **reproduzierbar** machen (17.10). Für eine agentische Engine zentral, weil Agenten nur prüfen können, was maschinell messbar ist.

| Konzept | Status | Was es braucht |
|---|---|---|
| **Invarianten pro Tick** (keine NaN-Positionen, Entities in Weltgrenzen, nicht in Wänden, Geschwindigkeit unter Maximum, Handles gültig) | erledigt (`Invariants`, ADR 0010) | Module registrieren ihre Prüfregeln, sobald es sie gibt |
| **Determinismus-Wächter** (gleiche Eingaben zweimal simulieren, Prüfsummen vergleichen) | erledigt (`verify`) | Prüfsumme pro Tick (17.10); findet versteckte Nichtdeterminismus-Bugs sofort mit Tick-Angabe |
| **„Stuck“-Erkennung** (NPC mit Ziel bewegt sich N Ticks nicht, Wegsuche schlägt dauernd fehl) | erledigt (`ProgressWatch`) | KI-Module füttern es mit der Restdistanz |
| **Telemetrie-Anomalien** (Frame-Spitzen, Zähler außerhalb des Üblichen) | vorbereitet | `Recorder::stats` (p50/p95); Schwellwerte oder Vergleich mit Baseline |
| **Visuelle Glitch-Heuristiken** (NaN-/Inf-Pixel, schwarze/leere Frames, Flackern zwischen Frames, fehlende Texturen) | Heuristiken erledigt (`image`), Capture offen | Offscreen-Capture in `knorke-render`; Bilddiff gegen Golden Images mit Toleranz (Chiaro-Erfahrung: GPU-Ausgabe schwankt leicht) |
| **Screenshot-Bewertung durch Bildmodelle** (Agent sieht Bild und Erwartung) | offen | ergänzt, ersetzt keine exakten Prüfungen (Plan 8.4) |
| **Automatisches Playtesting** (Zufalls-/„Monkey“-Eingaben, Bots, die Ziele verfolgen, Abdeckung neuer Zustände) | offen | Eingaben als Commands; Headless-Simulation schneller als Echtzeit |
| **Property-Based Testing der Simulation** (`proptest`: zufällige Szenen, Invarianten müssen halten) | offen | headless Module + Invarianten |
| **Fehlerfall minimieren** (Eingabefolge automatisch kürzen, bis der Fehler gerade noch auftritt) | erledigt (`minimize`) | deterministisches Replay; Delta-Debugging über die Eingabespur |
| **Automatisches Crash-Logging** (Wunsch Manuel 2026-11-25; nur wer die ganze Engine nutzt): bei Absturz eine Crash-Datei mit den wichtigen Infos, wenn möglich mit Screenshot | offen | Panic-Hook und OS-Absturzbehandlung im Host-Profil (`knorke-integration`), nicht in Fachcrates; Inhalt: Version, Plattform, GPU, letzter Tick, letzte `Diagnostic`s, `Inspect`-Auszug, Replay-Spur und letzter Snapshot (17.10), damit der Absturz nachspielbar ist; Screenshot = letzter fertiger Frame, den der Host bereithält (ADR 0002: kein Readback im Absturz); abschaltbar (ADR 0011) |
| **Fehlerbericht ohne Absturz** (Wunsch Manuel 2026-11-25): belegbare Taste → Screenshot, Spiel pausiert automatisch, kleines Textfenster für die Beschreibung, Markierwerkzeug für Bildschirmbereiche mit eigenem Text; Meta- und Speicherinfos werden mitgesammelt | offen | Auslöser als Command (Eingabegrenze); Pause als Tick-Stopp; Screenshot über Offscreen-Capture (Zeile „Visuelle Glitch-Heuristiken“); UI für Text und Markierungen (braucht UI-Schicht); Bericht = Ordner oder Archiv mit Bild, Markierungen, Text, denselben Daten wie die Crash-Datei (Snapshot, Replay-Spur, `Inspect`, Speicherverbrauch); gleiches Format wie Crash-Datei, damit Werkzeuge und Agenten beide lesen |

### 17.10 Replay, Snapshots und Time-Travel

**Antwort auf „brauchen wir noch etwas für Replays?“:** Ja. Tick, Seeds, stabile Hash-Reihenfolge und thread-unabhängige Ergebnisse sind da. Es fehlen vier Bausteine, die **vor dem Umbau von Sagax** festgelegt werden sollten, weil jedes Modul sie mittragen muss:

1. **Snapshot/Restore-Vertrag pro Modul:** Zustand in einen Byte-Puffer schreiben und daraus wiederherstellen, ohne Allokation im Dauerbetrieb. Module halten ihren Zustand dafür kopierbar (Pod-Typen, `HandleMap`, keine Zeiger).
2. **Prüfsumme pro Tick** (`StableHasher` über Snapshot-Bytes oder pro Modul): Replay und Netzwerk erkennen eine Abweichung im ersten abweichenden Tick und im betroffenen Modul.
3. **Grenze für alle nicht-deterministischen Eingaben:** Spieler-Eingaben, Agenten-Transaktionen, Netzwerk, LLM-Antworten, **Fertigstellung asynchroner Asset-Loads**, Echtzeit → alles kommt nur als aufgezeichneter Command an einem Sync-Punkt in die Simulation. Was diese Grenze umgeht, macht Replays kaputt.
4. **Aufzeichnungsformat:** versioniert, nur anhängend: Engine-/Content-Version, Startzustand (Szenen-Revision + Seed oder Snapshot), Commands pro Tick, Prüfsummen pro Tick, **Keyframe-Snapshots alle N Ticks** zum Springen (wie I-Frames im Video).

Darauf aufbauend:
- **Flight Recorder:** immer laufender Ringpuffer der letzten N Sekunden (Commands + Keyframes). Bei Absturz, Invariantenverletzung oder Agenten-Meldung wird er als reproduzierbarer Fehlerbericht gesichert.
- **Time-Travel-Debugging:** zu Tick T springen (nächster Keyframe + Commands nachspielen), Zustand per `Inspect` ansehen, KI-Traces dieses Ticks abrufen. Für Agenten eine Operation wie `replay.seek` / `replay.inspect`.
- **Replay als Regressionstest:** gespeicherte Replays laufen in CI; Prüfsummen müssen gleich bleiben oder bewusst neu festgeschrieben werden.
- Grenzen: GPU-Ausgabe ist nicht bitgleich (nur Toleranzvergleich); Replays zwischen Plattformen brauchen plattformunabhängige Mathematik (17.2).

**Stand:** Bausteine 1, 2 und 4 sowie Flight Recorder und Time-Travel (`seek`) sind umgesetzt (ADR 0010, Abschnitt 16). Baustein 3 (Eingabegrenze) ist als Regel festgeschrieben (`CLAUDE.md`) und wird mit dem Scheduler technisch durchgesetzt.

Offen:
- plattformübergreifend gleiche Mathematik: umgesetzt (ADR 0011); Bestätigung durch den ersten CI-Lauf auf Linux, macOS/ARM und WASM steht aus;
- Lebenszyklus von Golden Replays in CI: Vergleichsstufen (bitgenau / Event-Assertions / Bilddiff) und `replay rebase` nach bewusst akzeptierten Änderungen;
- Exploration über Snapshot-Fork (Go-Explore, Deterministic Simulation Testing), parallel über `Parallel`, Navigation als Orakel;
- Agenten-Operationen `replay.verify`, `replay.seek`, `replay.minimize`, `invariant.add`.

### 17.11 Aktuell diskutierte und erforschte Konzepte (Recherche 2026-11-23)

Web-Recherche durch einen Subagenten, Quellen 2025–2026. **Nicht von mir einzeln nachgeprüft**; „unbelegt“ heißt: nur Sekundärquellen. Reife: F = Forschung, E = frühe Anwendung, P = Produktion.

| # | Konzept | Reife | Bedeutung für Knorke | Status | Quelle |
|---|---|---|---|---|---|
| 1 | **Code Mode / programmatischer Tool-Aufruf:** Agent bekommt eine typisierte API und schreibt ein Skript, statt hunderte MCP-Tools aufzurufen; Kontextbedarf bleibt bei wachsender API fast konstant | E | Wichtigste Entscheidung für die Agenten-Schicht: **eine Schema-Quelle** (Rust-Typen) erzeugt CLI, skriptbare API und dünnen MCP-Adapter. Ein Agenten-Skript = ein Command-Batch (validierbar, replaybar, rückgängig zu machen) | offen – vor Phase 3 entscheiden | blog.cloudflare.com/code-mode-mcp |
| 2 | **Offizielle Engine-MCP-Server, Schwenk zur CLI** (Unreal 5.8 MCP-Plugin; Unity ersetzt MCP-Server durch CLI) | P/E | Bestätigt „CLI zuerst, MCP als Adapter“ (Plan 8). Lehre aus Godot: Hot-Reload darf die Agenten-Session nicht beenden → Session außerhalb des neu geladenen Prozesses | vorbereitet | unrealengine.com/news/unreal-engine-5-8-is-now-available (Unity-Angaben unbelegt) |
| 3 | **Benchmarks für Agenten-Spieleentwicklung** mit abgespielten Eingaben und Bild-Bewertung (GameCraft-Bench u. a.; bestes Ergebnis ~41 %, Schwäche: fehlendes visuelles Feedback) | F | Deterministisches Replay macht Knorke zum idealen Prüfer: `verify` = Eingabespur rein → Prüfsummen, Screenshots, Event-Log raus. Screenshot-/Frame-Capture als vollwertige Agenten-Operation | vorbereitet (`verify`, ADR 0010) | arxiv.org/abs/2606.17861 |
| 4 | **Laufzeit-Inspektion per Reflection-Protokoll** (Bevy Remote Protocol: Abfragen, Schema, zusammenfassende `inspect`/`summarize`-Aufrufe) | E | Typ-Registry/Reflection pro Modul erzeugt Schema, Inspektion und Mutation; zusammenfassende, token-sparsame Abfragen statt Roh-Dumps; Mutation nur über `CommandBuffer` | offen – baut auf `Inspect` (ADR 0007) und ADR 0004 auf | github.com/bevyengine/bevy/pull/25883 |
| 5 | **Automatisches Playtesting mit LLM-Agenten und abdeckungsgesteuertem RL** (TITAN: Bugs in MMORPG gefunden; SMART: Belohnung für neu erreichte Code-Pfade) | F/E | Headless, deterministisch, schneller als Echtzeit, parallel über viele Seeds; Funde kommen als Replay-Datei zurück; Engine liefert Abdeckung (erreichte Systeme/Events) als Telemetrie | offen (17.9) | arxiv.org/abs/2509.22170 |
| 6 | **Holographic Radiance Cascades** – 2D-Globalbeleuchtung ohne zeitliche Akkumulation und Entrauschen, konstante Kosten (1,9 ms bei 512², 7,7 ms bei 1024², RTX-3080-Laptop) | F, erste Implementierungen | Stärkster Kandidat für `knorke-lighting` neben den Chiaro-Verfahren: deterministisch pro Frame → kein Geisterbild, Screenshot-/Replay-Tests stabil. Reiner Compute → SDL3-GPU-Compute. Für schwache GPUs Kaskaden-Auflösung/Fallback | offen – Spike beim Lighting-Neubau | arxiv.org/abs/2505.02041 |
| 7 | **SDL 3.4:** `SDL_CreateGPURenderer` (2D-Renderer und GPU-API teilen ein Device); `SDL_shadercross` weiter Preview; kein Bindless, kein Mesh-Shading | P | Debug-Overlays/Tool-UI ohne zweiten Render-Pfad; Shader offline mit shadercross, Reflection-Daten ins eigene Typsystem (Agent sieht Shader-Bindings typisiert) | vorbereitet (ADR 0002) | discourse.libsdl.org/t/announcing-sdl-3-4-0/65438 |
| 8 | **WebGPU-Stand:** in Chrome, Safari 26, Firefox (Windows/macOS) ausgeliefert | P mit Lücken | SDL3-GPU hat **kein Web-Backend**. Ist ein Browser-Build Ziel, braucht es einen zweiten Pfad (wgpu) oder eine Abstraktion über dem Renderer – früh entscheiden | verworfen (vorerst) – kein Browser-Ziel (Entscheidung 2026-11-23); neu bewerten, falls sich das ändert | en.wikipedia.org/wiki/WebGPU |
| 9 | **Neural Texture Compression** (in einem AAA-Titel ausgeliefert, braucht Cooperative Vectors) | P (selektiv) | Für 2D auf SDL3 vorerst irrelevant | verworfen (vorerst) – nicht erreichbar über SDL3, kaum Nutzen für Pixel-Art | ubisoft.com/…/shipping-neural-texture-compression… |
| 10 | **Generative NPCs als Hybrid:** deterministischer Kern (FSM/GOAP/Behaviour Tree) entscheidet *was*, LLM formuliert *wie*; Aktionen als Tool-Calls, Kognition entkoppelt mit 1–2 Hz | E | Bestätigt 17.5: LLM-Ausgaben sind externe Eingaben → typisierte Commands mit Ziel-Tick, im Replay aufgezeichnet, im Netz nur der Command repliziert. KI validiert die LLM-Aktion gegen erlaubte Aktionen | vorbereitet | arxiv.org/pdf/2611.05427 |
| 11 | **Plattformübergreifender Float-Determinismus in Rust:** Physik-Engines leiten Transzendente über `libm`; einige `std`-Funktionen sind eigene Formeln und werden dabei übersehen | P | Konkret: Modul `detmath`; Clippy `disallowed-methods` für std-Transzendente in deterministischen Crates; CI vergleicht Prüfsummen über x86_64, aarch64, wasm32 → Determinismus-Verletzung wird maschinell prüfbarer Fehler | erledigt (ADR 0011) | github.com/rust-lang/rust/issues/150323 |
| 12 | **Code World Model:** Agent schreibt deterministische Weltlogik, ein Video-Modell rendert daraus die Optik | F | Sim/Render-Trennung hält die Tür offen für einen generativen Stil-Pass. Jetzt schon: ID-, Tiefen- und Normalen-Puffer als exportierbare Render-Ausgaben vorsehen | offen | arxiv.org/abs/2608.25927 |
| 13 | **Prozedurale Generierung: WFC + Evolution/RL** (globale Eigenschaften wie Erreichbarkeit bleiben schwer) | F | Deterministisches WFC-/Constraint-Modul mit Seed; Agent liefert Beispiel-Tiles und Regeln, Engine prüft Erreichbarkeit über Navigation und meldet strukturiert zurück | offen | arxiv.org/abs/2607.02082 |
| 14 | **Visuelle Regressionstests** (Pixel- und Wahrnehmungs-Diffs; Probleme: Antialiasing, Treiber, Timing) | P im Web | `capture`/`diff`-Operation mit Wahrnehmungsmetrik (z. B. NVIDIA FLIP) und Toleranz pro Plattform; Software-Rasterizer als Referenz-Backend für CI | offen (17.9) | argos-ci.com |
| 15 | **Rust-Hot-Patching mit Subsecond** (Funktionen zur Laufzeit austauschen; nicht bei geänderten Struct-Layouts, nur ein Crate) | E | Tick-Einstieg einmal über `subsecond::call`; bei Layout-Änderungen: Snapshot → neu laden → deterministisch nachsimulieren. Einschränkung kollidiert mit der Crate-Struktur (Spiellogik müsste in einem Crate liegen) | offen | dioxuslabs.com/blog/release-070 |
| 16 | **WASM Component Model für Mods** (wasmtime, WIT-Schnittstellen, Limits pro Mod; ~21 µs pro Aufruf mit frischer Instanz; Kopierkosten an der Grenze) | E | Bestätigt 17.7. WIT als dritte Ausgabe derselben Schema-Quelle. Mods bekommen Events rein, Commands raus (grobkörnig, also wenig Kopien). „Fuel“-Limits sind deterministisch → netz- und replaytauglich | offen | tartanllama.xyz/posts/wasm-plugins |
| 17 | **Firewheel** – Audio-Graph-Engine in Rust, unabhängig vom ECS, räumlicher Klang, WASM-Backend | E | Direkter Kandidat für `knorke-audio` (17.6); Parameter aus Sim-Events per Command | offen | github.com/BillyDM/firewheel |
| 18 | **Textuelle Szenen-Notation** (Bevy BSN) | E | Szenenformat von `knorke-scene` textuell, diff-bar, schema-validiert – für Agenten leichter als Binärformate | geplant (Phase 3) | bevy.org/news/bevys-sixth-birthday |
| 19 | **Netcode modularer:** Replikation/Interessenbereiche getrennt von Vorhersage/Rollback (lightyear + bevy_replicon) | E | Bestätigt die Aufteilung in 17.3 | vorbereitet | github.com/cBournhonesque/lightyear |
| 20 | **ML-Sprite-Pipelines** (Zwischenbilder, Sprite-Sheets per Diffusion) | F | Asset-Import bietet Paletten- und Raster-Prüfung als prüfbare Regeln | offen | arxiv.org/pdf/2610.27253 |
| 21 | **Gaussian Splatting** | E (3D) | Für 2D/2.5D nicht relevant | verworfen – kein Nutzen für 2D | github.com/ReconWorldLab/godot-gaussian-splatting |

**Nachtrag: Fehlererkennung und Replay** (gleiche Recherche, zweiter Teil)

| # | Konzept | Reife | Bedeutung für Knorke | Status | Quelle |
|---|---|---|---|---|---|
| 22 | **Property-based Testing der Simulation mit Shrinking:** zufällige Seeds und Command-Folgen, Invarianten nach jedem Tick, Fehlschläge auf minimale Folge schrumpfen. ddmin ist 1-minimal; Weighted DD braucht ~23 % weniger Läufe. Warnung: naives Schrumpfen erzeugt ungültige Command-Folgen | P (Technik) | Günstigste, wirksamste Maßnahme für den deterministischen Kern. Command-Generator und Shrinker zusammen entwerfen; Engine validiert Commands am Sync-Punkt | vorbereitet (`Invariants`, `minimize`) | arxiv.org/pdf/2411.19410 |
| 23 | **Deterministic Simulation Testing:** ganzes System deterministisch, Exploration verzweigt Zeitlinien, jeder Fund exakt reproduzierbar | P (verteilte Systeme), Demo (Spiele) | Knorke braucht keinen Hypervisor: Snapshot-Fork + `Parallel` ergibt `explore`/`fuzz` | offen | antithesis.com/docs/introduction/how_antithesis_works |
| 24 | **Go-Explore für Spieletests:** Checkpoints sichern, vielversprechende wählen, von dort erkunden; Navmesh als Heuristik und Orakel (Position außerhalb = Glitch, unerreichte Bereiche melden); schlägt Curiosity-RL ohne neuronales Training | F/P (große Studios) | Snapshots liefern das Zurückkehren; Navigation ist Orakel. Für ein kleines Team realistischer als RL | offen | arxiv.org/abs/2209.00570 |
| 25 | **Visuelle Glitch-Erkennung mit Bildmodellen:** referenzbasiert (Frame-Paar) schlägt referenzfrei deutlich; zeitliche Glitches aus Einzelbildern schwer erkennbar | F | Erst Pixel-/Wahrnehmungs-Diff gegen Referenz-Frame derselben Tick/Seed-Kombination, nur Auffälliges an ein Bildmodell. Zeitliche Glitches (Zittern, Teleport) über Invarianten statt Pixel | vorbereitet (`frame_diff`) | arxiv.org/abs/2604.11082 |
| 26 | **Anomalieerkennung auf Telemetrie** (Modelle auf fehlerfreien Event-Folgen; Baseline-Vergleich) | P (Live-Ökonomie), F (Gameplay) | Event-Histogramme pro Tick aus `EventQueue`/`Recorder` gegen Baseline der letzten grünen Version, als CI-Batch | offen | arxiv.org/pdf/2312.08418 |
| 27 | **Replay = Eingaben + Keyframes; Seek über nächsten Keyframe** (seit Jahrzehnten; Retro City Rampage nutzt dieselbe Infrastruktur für Tests und Replays) | P | Umgesetzt, mit Prüfsummen pro Modul zur Eingrenzung | erledigt (ADR 0010) | youtube.com/watch?v=W20t1zCZv8M |
| 28 | **Flight Recorder:** üblich als Video-Ringpuffer oder Prozess-Aufzeichnung (rr); als Zustands-/Eingabe-Ringpuffer kein Produkt gefunden | P (Video/Prozess) | Knorke macht es billiger und besser: Keyframes + Commands, bei Panic/Invariante/Hotkey Dump als Replay; Video bei Bedarf nachrendern | erledigt (`Recorder` mit `window`) | rr-project.org |
| 29 | **Golden Replays als Regressionstests** mit Vergleichsstufen und Neufestschreiben nach gewollten Änderungen | P | Siehe 17.10, offen | offen | – |

**Abgeleitete Leitlinien:**
1. **Eine Schema-Quelle für alles:** Rust-Typen + Reflection erzeugen CLI, Agenten-Skript-API, MCP-Adapter, WIT-Schnittstellen für Mods und Inspektions-Endpunkte (1, 4, 16). Ergänzt ADR 0004.
2. **Determinismus ist Prüf-Infrastruktur:** Eingabespuren, Prüfsummen, Frame-Captures und Bild-Diffs zusammen sind der Prüfer für Agenten-Builds und Playtest-Bots (3, 5, 11, 14). Stützt die Priorität von 17.10.
3. **LLM-Ausgaben sind externe Eingaben** wie Spielereingaben: aufgezeichnet, repliziert, die Simulation bleibt rein (10).
4. **Deterministische 2D-Beleuchtung** ohne zeitliche Akkumulation erleichtert Bildtests (6).
5. **Ein Artefakt für alles:** Das Replay ist Bug-Report, Regressionstest, Fuzzing-Fund, Flight-Recorder-Dump und Agenten-Eingabe (22–29).
6. **Orakel nach Kosten:** Invarianten → Navigations-/Erreichbarkeitsorakel → Bilddiff gegen Referenz → Bildmodell nur zur Einordnung (22–25).

**Recherchelücken:** Kein Spiele-Werkzeug gefunden, das Replay-Logs mit Delta-Debugging kombiniert, und kein Zustands-Ringpuffer-Recorder als Produkt; Prozentzahlen der Glitch-Benchmarks teils nur aus Sekundärquellen. Außerdem nicht belegt: Stand von wgpu 2026, spielespezifische Praxis visueller Regressionstests und Details zu SMART/CA2/GBQA sind nicht belegt. Bei Bedarf gezielt nachrecherchieren.

### 17.12 Chiaro-Port nach Rust und Integration (vorgesehen 2026-11-23)

Quelle: `D:\Entwicklung\chiaro` @ `9bc5b9b` (gleicher Stand wie die Inventur `docs/inventory/chiaro.md`). Umfang laut Inventur §7.1: ~17–19 kLOC C++ fachlich, 6 kLOC HLSL (151 Programme), ~37 kLOC Tests als Spezifikation. Rahmen: Abschnitt 5 (Weg B bevorzugt, Gate 5.1, Reihenfolge 5.2, Feature-Matrix 5.3).

**Entschieden in ADR 0014:** drei Crates `knorke-gpu`, `knorke-post`, `knorke-light2d`; GPU ist der Hauptweg, das CPU-`knorke-lighting` (ADR 0013) wird auf pixelraum-Stand eingefroren; Shader als getrennte Pakete pro Format (Vulkan Standard, Metal für macOS, D3D12 optional), nicht in der Bibliothek. **Aktueller Stand, Pakete P1–P35 und Übergabe: `docs/chiaro-port/ABLAUF.md`** (Aufteilung: `docs/inventory/chiaro-aufteilung.md`). Die Tabelle unten ist die Grobfassung.

| Schritt | Inhalt | Status | Bemerkung |
|---|---|---|---|
| 1 | **`knorke-gpu`** (erledigt 2026-11-24, P1/P2): sicherer Rust-Wrapper um SDL3-GPU (Device geliehen vom Host, Texture, RenderTarget, Shader aus Chiaro-Blobs + Reflection, PipelineCache, UploadBuffer, Debug-Gruppen); `unsafe` nur hier (ADR 0005) | geplant | vorher Reife von `sdl3-sys` prüfen (Inventur §7.3/4); Vertrag ADR 0002: kein Submit/Present/Readback im Modul |
| 2 | **Gate-Spike (5.1)**: ein Punktlicht + ein Schattenfall + Offscreen-Capture in Rust, gegen dieselbe Szene aus C++-Chiaro; Passzeiten und Bilddiff messen | erledigt 2026-11-24: bestanden, bitgleich (ADR 0015) | Ergebnis als ADR „Chiaro: Rust-Port ja/nein“ |
| 3 | Golden Scenes aus den Chiaro-Demos (`--screenshot --time`) erzeugen, Paritätsmetrik mit Ausreißer-Budget (Treiber streut schon in C++, Inventur §1.3) | geplant | ohne das keine Paritätsaussage |
| 4 | CPU-Logik portieren (Licht-, Schatten-, Medien-Mathematik, Atlanten) samt Unit-Tests als Spezifikation; `knorke-math`-Typen, `det::*`, kein `HashMap` | geplant | gut parallel in Teilen machbar |
| 5 | GPU-Stufen portieren (8-Stufen-Pipeline, Licht-Programmtabelle exakt aus `cmake/ChiaroLightPrograms.cmake`, Schatten-Cache) | geplant | größtes Risiko: falsch belegte Slots ergeben stille Bildfehler (Inventur §7.3/1–2) |
| 6 | Post-Stufe (Tonemap, Bloom, sRGB-Encode) – Zuordnung `knorke-render` vs. `knorke-lighting` klären | offen | Inventur §7.2 |
| 7 | **Integration** in `knorke-integration` als eigenes Feature (z. B. `gpu`), Snapshot/Inspect/Diagnostics (`knorke.lighting.*`), Vertragstests (keine Allokation im Steady State, nullgroße Aus-Zustände) | geplant | wie das `pixel`-Profil |
| 8 | Feature-Matrix vollständig, C++-Chiaro archivieren | geplant | erst bei ausreichender Parität (5.2/8) |

Aus P23 (Okklusionsmaske, Distanzfeld, Cache; 2026-11-24):
- **Dirty-`BitSet`s im `ObstacleStore`** – entschieden: `occlusion::OcclusionMask` verbraucht sie nicht (wie schon der Plan aus P16). Maske und Plan lesen nur `&ObstacleStore` und sind ohne die Bits exakt (eigene Fußabdrücke je dynamischem Hindernis, `static_version` für die statische Geometrie). Ein Verbraucher mit `clear_dirty` bräuchte `&mut ObstacleStore` im Render-Pfad, wäre der einzige Leser eines geteilten Stores (Lichtstufe, Caster-Klassen E53, Werkzeuge lesen mit) und verlöre bei einem fehlgeschlagenen Frame still Änderungen. Status: **offen**, ob die Bits bleiben (Host-Komfort) oder mit einer Snapshot-Version entfernt werden; Entscheidung, sobald ein echter Verbraucher „geändert seit meinem letzten Blick“ braucht.
- **Sprite-Maske der Anwendung** (`occlusion_mask.frag`, E21-Szenenrechteck) in der Maske – **offen** (der Plan aus P16 unterstützt sie schon; fehlt nur die Zeichenstufe).
- **Masken und Felder der Caster-Klassen** gefilterter Schatten (E53, `RecordCasters`) – **offen** (P25 hat sie nicht geliefert; die Lichtstufe nimmt sie über `ShadowMaps::casters` entgegen, `OcclusionMask` baut sie noch nicht).
- Belegt: volle Maske und Feld sowie eine Cache-Folge (wiederverwendet, ganzzahliger Schwenk mit bewegtem Hindernis, Bruchteil-Schwenk) texelgleich zu C++-Chiaro (`tests/golden/chiaro/field/`); inkrementell = voll exakt bei ganzzahligen Schwenks, auch mit Höhentextur. **erledigt**.

Aus P25 (Lichtstufe; 2026-11-24):
- **Uniform-Layouts der Lichtshader** (offen aus P24) – **erledigt**: `tests/light_uniform_layouts.rs` vergleicht die Rust-Structs Feld für Feld mit den SPIR-V-Offsets aller 99 Shader der 92 Lichtprogramme; keine Abweichung zu Chiaro.
- Belegt: Licht-, Specular- und Direction-Puffer in 5 Szenen (`tests/golden/chiaro/light/`: flat, shadow, height, exact, direction) bytegleich zu C++-Chiaro (Vulkan, RX 6700 XT, 2026-11-24); Spike-Szenen über die neue Stufe weiter bitgleich. **erledigt**.
- **Gefilterte Schatten (E53)** – **offen**: nicht renderbar und ungetestet, bis `OcclusionMask` die Caster-Masken baut; solche Frames brechen mit `knorke.light2d.lighting.casters_missing` ab.
- **Sprite-Maske** (`occlusion_mask.frag`, E21) – **offen**: lebt weiter im Spike, nicht als Stufe in `OcclusionMask`.
- **Ringlichter der Schockwellen** (Chiaro `ShockwavePass`) – **offen**, bisher keinem Paket zugeordnet (passt zu P21).
- **Ohne GPU-Test**: maskierte und profilierte Lichter (Atlanten erst mit P26), Surface-Puffer, andere Projektionen als Identität/Ortho – **offen**, mit P26/P31 nachziehen.
- **Absturz in GPU-Tests** (gefunden bei der Abnahme von P25, Ursache in P20-Test `knorke-post/tests/gpu_parity.rs`): Tests eines Programms riefen `SDL_Init` gleichzeitig aus mehreren Threads; Folge war ein still übersprungener Test oder ein Speicherzugriffsfehler (3 von 60 Läufen unter Last, einmal im normalen Release-Lauf). Behoben: Sperre in `knorke_gpu::init_video`, GPU-Tests in `gpu_parity` nacheinander; danach 160 Läufe unter Last ohne Absturz und ohne Skip. **erledigt**.
- **GPU-Tests nacheinander** in `recording.rs`, `light_program_pack.rs` – **erledigt** (Aufräumen 2026-11-24, eine Sperre in `knorke_gpu::test_support`). Früher: **offen**: dort fehlt die Sperre „ein Gerät zur Zeit“; nicht geprüft, ob sie abstürzen können.
- **Stilles Überspringen von GPU-Tests** – **offen**: auf einem Rechner mit GPU kann ein Skip einen echten Fehler verdecken. Idee: Umgebungsvariable (z. B. `KNORKE_REQUIRE_GPU=1`), mit der ein Skip zum Fehlschlag wird; für die Abnahme setzen **Vorbereitet** in `gpu_atlas_zones.rs` (P26); in den übrigen GPU-Tests noch nachziehen, am besten als gemeinsamer Helfer.

Aus P26/P27 (Atlanten, Ambient-Zonen-Pass, Composite; 2026-11-24):
- Belegt: Lichtpuffer mit Profil-Licht, Masken-Licht und Ambient-Zonen (5 Szenen, `tests/golden/chiaro/atlas/`) sowie Composite-HDR (7 Szenen, `tests/golden/chiaro/composite/`) bytegleich zu C++-Chiaro (Vulkan, RX 6700 XT, 2026-11-24). Schließt „maskierte und profilierte Lichter ohne GPU-Test“ aus P25. **erledigt**.
- **Profiltabellen nicht ulp-genau** – **erledigt** (entschieden 2026-11-24, Manuel: so lassen): `build_shape_profile` (P9) rechnet mit `knorke_math::det` und weicht in 879 von 3×13 032 Werten um höchstens 2 ulp von C++ `std` ab (belegt: `tests/golden/chiaro/atlas/table*.bin`, Test mit Grenze 4 ulp). Der Lichtpuffer bleibt bitgleich. `det` ist nach ADR 0011 Pflicht; ulp-genaue Tabellen hieße, Chiaros `std`-Ergebnisse als Tabelle zu übernehmen. Bleibt so, solange das Bild bitgleich ist; kippt ein Golden-Test deswegen, neu entscheiden.
- **Composite mit Zonen** (`*_zones`-Programme, `pure_ambient`) – **erledigt** mit P31 (5 Atlas-Szenen bis HDR bitgleich). **Composite mit Medium** – **offen** (P29): nur Layout und Pipelines geprüft.
- **`InputColorSpace` doppelt** (light2d-Enum neben dem Szenen-Farbraum in `knorke-post`, weil light2d `knorke-post` nicht kennen darf) – **erledigt** mit P31: `Light2dRenderer` speist beide aus `PostSettings::output.scene_color_space`.

Aus P31 (Pass-Plan, Integration; 2026-11-24):
- Belegt: `minimal_normal_map`, `gate_sprite_shadow` über Pass-Plan + Post STRICT max diff 0; Snapshot der Stores → neuer Plan → gleiches Bild; 0 Allokationen in 5 warmen Frames (`no_alloc_pass_plan.rs`). **erledigt**. Sprite-Maske (E21) ist Stufe in `OcclusionMask`. **erledigt**.
- **ADR 0017** (Pass-Plan, GPU-Profil in `knorke-integration`, verschärfte Abhängigkeitsregeln) – **offen**: Status „vorgeschlagen“, Entscheidung Manuel.
- **Sprite-Maske im Höhenmodus** (`occlusion_mask_height.frag`) – **erledigt** (2026-11-25): Chiaro-Golden `sprite_height` (Maske und Höhe texelgenau gleich; Alpha 128 verdeckt, 127 nicht).
- **Demo-Szenen aus P3** (10 Szenen, 1280×720) – **läuft**: `wet_street` erledigt (2026-11-25, bitgleich zum Golden, Surface-Layer bytegleich zu Chiaro; Farb- und Material-Layer als Fixtures aus `tests/golden/chiaro/wet_street/wet_street_ref.cpp`, weil Knorke keinen SDL_Renderer hat). `iso_night_hard` und `iso_night_plain` – **blockiert** (2026-11-25): beide brauchen Hindernis-Muster (Lattenzaun, Gitterturm; P15 + P28, nicht portiert). Gemessen mit der C++-Demo gegen das Golden, HUD ausgenommen: mit `--no-patterns` weicht `hard` in 9,93 % der Pixel ab (7,99 % über Toleranz 2, max 152), `plain` in 1,64 % (0,98 % über 2, max 31) – STRICT nicht erreichbar. Entscheidung Manuel 2026-11-25: P15/P28 jetzt portieren – beide erledigt (2026-11-25, bitgleich zu Chiaro in drei Referenzszenen); die zwei Szenen sind damit wieder offen. Rest: brauchen Layers/Sprites (P33–P35), teils Medium (P29/P30) und Ringlicht (P21); vier davon sind nicht deterministisch (Uhr/Maus).
- **Leistung Pass-Plan gegen alten Spike** – **erledigt**: Messung 2026-11-25 abends (AMD Radeon RX 6700 XT, Vulkan, Release, Wand-Uhr pro Frame inkl. Submit+Warten, je Lauf 3000 Frames, CPU-Grundlast 5–19 %, kein Build nebenher; Rust über `KNORKE_SCENE_BENCH`, C++ über `gate_ref.c` gegen Chiaro 9bc5b9b, in 8 Runden abwechselnd): `minimal_normal_map` Median Rust 184–268 µs (17 Läufe, Mitte 207) gegen C++ 185–271 µs (8 Läufe, Mitte 223); `gate_sprite_shadow` Rust 443–705 µs (17 Läufe, Mitte 547) gegen C++ 454–624 µs (9 Läufe, Mitte 536). **Ergebnis:** kein Unterschied außerhalb der Streuung; die Läufe streuen je Seite um etwa ±20 %, ein Unterschied unter ~15 % ist damit nicht ausschließbar. Die Werte aus ADR 0015 (405–427 µs) erreicht heute auch C++ nicht – sie taugen nicht als Maßstab, nur der Vergleich in derselben Sitzung.
- **`check_invariants` des Pass-Plans** ist eine Methode mit `Sink`, kein `knorke_observe::Invariants` (Lebensdauer `'d`) – **erledigt (bleibt so)**, bei P32 angesehen: `Invariants<W>` ist eine Regelliste des Hosts über seine Welt; der Plan liefert die Prüfung, der Host kann sie dort als Regel eintragen.

Aus P32/P33 (Debug-Prüfung, Layers; 2026-11-25):
- **GPU-Debug-Prüfung** – **erledigt**: opt-in über `DebugChecks::LAYERS`, Readback nur dann (ADR 0017 Punkt 8), aus = nichts angelegt; Bild unverändert und 0 Allokationen belegt. **Medium-Prüfung im Plan** – **offen** bis P29 (Stufe nimmt das Medium schon, getestet).
- **Layers** in `knorke-integration` – **erledigt**: nur GPU-Layers. **Layer-Größe = Szenengröße** wird nicht geprüft (der Pass-Plan meldet falsche Puffergrößen selbst) – so gewollt.
- **Eigene Shader für die 2D-Helfer** (Sprites, Surface-Geometrie zeichnen; in Chiaro machte das SDL_Renderer) – **erledigt für Sprites (P34)**, Surface offen (P35) (Entscheidung Manuel 2026-11-25): Übersetzen mit DXC statt shadercross (`D:\Dev-Tools\dxc`, SPIR-V bytegleich zu Chiaros Blobs geprüft an `output.frag`), Ablage laut ADR 0014 §3. **MSL für eigene Shader** – **offen** (DXC kann kein MSL; shadercross oder SPIRV-Cross nötig).
- **Sprites** (P34) – **erledigt**: Karten-Texel werden exakt geschrieben statt geblendet (Abweichung von Chiaro, dessen Blend nasse Material-Einträge mit Alpha 224 mischte). **Stehende Sprites** – **offen** (P35, brauchen die Surface-Kodierung). **Streaming-Upload** – **vertagt**: kein Verbraucher; kommt mit der Brücke CPU-Bild → GPU-Licht.
- **`SV_VertexID` und erster Vertex** – nicht nachgeschlagen (Wissen des Agenten, keine Quelle geprüft): auf Vulkan zählt er den ersten Vertex mit, auf D3D12 nicht. Eigene Shader bekommen den Startindex deshalb als Parameter. **D3D12-Lauf der eigenen Shader** – **offen**: nur Vulkan getestet.
- **Surface-Geometrie** (P35) – **erledigt**; **stehende Sprites** – **erledigt** (damit ist der Punkt oben geschlossen). `Sprite::new` hat seitdem das Argument `height_step`. **Medium-Draw** – **offen** (P30).
- **Muster (P15/P28), Reste** – **offen**: Frame-Warnungen für Muster (Sheet ohne Alpha, zerstörtes Muster in Benutzung; brauchen neue Warnungsarten und einen Scan); Atlas-Textur schrumpft mit dem Store (Chiaro wächst nur) – Wirkung auf das Bild nach Schrumpfen nicht gemessen; Rotation nur bei 30° und 17° gegen Chiaro gemessen.
- **Surface-Geometrie und stehende Sprites zusammen in einer Szene** (Golden-Bild) – **offen**: kommt mit den Demo-Szenen.
- **`encode_surface_sprite` parallel** (`Parallel`, feste Chunks) – **offen**: läuft einmal beim Laden über alle Pixel; erst messen, wenn große Atlanten da sind.
- **Gefilterte Schatten E53** (Caster-Klassen-Masken) – **erledigt** (2026-11-25): Teil der Okklusionsstufe, aus = nichts angelegt. Für E53 hat Chiaro keine Dumps; die Tests sind Verhaltens-Ports. **Inkrementelles Feld für die dynamische Klasse** – **offen**: heute wie in Chiaro jeden Frame voll neu (E76: erst nach Messung). **Klassen-Texturen in `Inspect`** – **offen** (weggelassen, damit bestehende Ausgaben gleich bleiben).
- **Ein Shader-Paket für alles** (`knorke-chiaro` aus Chiaros und Knorkes Blobs) statt eines Pakets pro Crate (ADR 0014 §3) – **offen**: so gewachsen, weil alle Tests ein Paket laden; Aufteilen erst, wenn Chiaros Shader ins Repo umziehen.

Aufräumen (Auftrag Manuel 2026-11-24; zwei Opus-Audits, nur gelesen; Ersparnis meist geschätzt):
- Leitlinie: wenig Code, nichts doppelt, Leser ist ein Agent; Kommentare sagen „warum“ und Nebeneffekte; Verhalten, Bitgleichheit, Snapshot-Formate und Diagnose-Codes bleiben.
- **GPU-Tests**: Helfer (Gerät, Sperre, Golden laden, Textur, Readback, `KNORKE_REQUIRE_GPU`) stehen in 8–14 Dateien als Kopie → ein gemeinsamer Helfer. **erledigt** (2026-11-24): `knorke_gpu::test_support`, netto −1 012 Zeilen, 117 Tests unverändert; Sperre und `KNORKE_REQUIRE_GPU` gelten für alle GPU-Tests.
- **Licht-Fachcode** (~950–1 000 Zeilen): `LIGHT_PROGRAMS` per Makro eine Zeile je Programm (~540, gezählt); ein `uniform_block`-Makro statt drei Byte-Schreibern und 17 `to_bytes` (~200); `SettingsError` einmal (~70); `validate` tabellengetrieben (~110); `AtlasTexture<S>` statt zwei Hüllen (~50). **erledigt** (2026-11-24, −852 Zeilen; `Block<N>` in `knorke-post` und `RectParams` bleiben von Hand, `input_missing` bleibt getrennt wegen verschiedener Texte). Zurückgestellt: `Projection` doppelt in light2d/post (API-Änderung).
- **KI-Crates**: `SnapshotValue` per Makro (`by_fields!` nach `knorke-types`, neu `by_tag!`; ~700–900, vorher Byte-Tests); Zähl-Allokator der Vertragstests einmal (~200); `SquadRules`-Weiterreichen per Makro (~70); Test-Helfer Gruppe/Golden/Karten (~250–350). **erledigt** (2026-11-24, netto −870; Kartenbauer verworfen, weil nicht dasselbe).
- **Zweite Runde** (`cleanup/light-programs`, `cleanup/snapshots-nav`): 92 Lichtprogramme per `const fn` aus Regeln (alte Tabelle als Test), Snapshot-Makros in navigation/perception/ai/grid, `assert_bytes!` in `knorke-types`, gemeinsames `open_list` für A*/D* Lite, Feature `test-support` in `knorke-gpu`. **erledigt** (2026-11-25, 1557 / 1426 Tests grün, Clippy 0, 21 Regeln, standalone ok).
- Geprüft, lohnt nicht: Stores (bauen schon auf `Slots`), `prepare`/`record`-Gerüste (Bausteine liegen in `knorke-gpu`), Szenario-Tests der KI (jeder prüft einen eigenen Fall). `knorke-integration/tests/images.rs` (4 310 Zeilen) ist die Bild-Suite des Software-Renderers – liegt am falschen Ort, Größe berechtigt. Nicht geprüft: `knorke-render` (`grid.rs`), toter öffentlicher Code.
- Lehre: Zeilenziele im Auftrag lenken auf „dichter schreiben“. Aufträge künftig als „dieselbe Logik an eine Stelle“ formulieren; was nur ähnlich aussieht, bleibt getrennt.
- **Rust 1.100.0** – **erledigt** (2026-11-25): `rust-toolchain.toml` auf `1.100.0`, `rust-version` und CI-Job `msrv` auf `1.100`, `[profile.dev] debug = "line-tables-only"`, `cargo fmt`, 3× `needless_range_loop`, `knorke-gpu` in `tests/contracts` nach `dev-dependencies` (`cargo::unused_dependencies`). Volle Prüfung grün (1557 / 1426, Fingerprint unverändert). Neue Features (`!`, `Vec::from_fn`, `bool::toggle`, `trim_prefix`, `DropGuard`) künftig nutzen, wo sie eigenen Code ersetzen. Idee, **offen**: stabile `Allocator`-API (`Vec::new_in`) für Arenen in Pfaden ohne Allokation.
- **DLL-Größen** (dylib, release, 2026-11-24 / 2026-11-25): Code (`.text`) je Crate 30–300 KB (ohne GPU), mit GPU ~3,3 MB statisches SDL3; Dateigröße größer wegen `.rustc`-Metadaten (nicht ausgeliefert). Nach Aufräumen Runde 1+2 und Rust 1.100.0 neu gemessen (2026-11-25): `.text` praktisch unverändert (±0 bis +10 KB je Crate; `knorke_ai` 300 KB [−5], `knorke_light2d` 307 KB [+7], `knorke_gpu` mit-gpu `sdl-build-static` 3320 KB [±0], `test-support` bleibt draußen). **erledigt**.
