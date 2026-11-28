# Knorke Engine – Architektur-, Migrations- und Implementierungsplan

**Stand:** 28. November 2026  
**Version:** 0.7 – Fachmodule, 2D-Licht auf der GPU und Audio stehen; Stand-Berichte in `docs/stand.md`  
**Status:** Zielarchitektur und ausführbarer Arbeitsplan; die Phasen 0 bis 3 sind weitgehend umgesetzt (Abschnitt 16): Basis-Crates, KI-Module, Software-Renderer, GPU-2D-Licht (`knorke-light2d`), Audio, Szene als Text und Agenten-Kommandozeile. Offen: Phase 4 (spielbarer Vertical Slice), 3D (17.14), Arbeitsliste in 17.15 
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

**Explizit noch nicht beginnen:** Großes generisches ECS, ~~3D-Renderer~~ (seit ADR 0021 Ausbau zur 3D-Engine beschlossen, Plan 17.14), kompletter GUI-Editor, Multiplayer, viele MCP-Tools, automatische Codegenerierung ohne Prüfsystem, Portierung aller Chiaro-Spezialeffekte auf einmal.

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

## 16. Umgesetzter Stand (2026-11-28)

Details in den ADRs unter `docs/adr/`, Inventuren unter `docs/inventory/`. **Ausführliche Berichte** (Messwerte, Zweige, Commits, Testnamen) zu den Zeilen mit Datum: [`docs/stand.md`](stand.md), gleiche Reihenfolge. Neue Berichte dorthin, hier genügt eine Zeile.

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
| Frame-Ablauf Stufe 1 (2026-11-25): Fester Tick aus Host-Zeit (`FixedStep`), `Runtime` mit Commands am Tick-Anfang, Systeme in fester Reihenfolge; erste Szene `guard_scene` mit Replay-Test. | `knorke-runtime`, `knorke-perception` | 0009 |
| Frame-Ablauf Stufe 2 (2026-11-25): Wachmann-Szene als Bild über die Laufzeit (Feature `gpu`), Pose zwischen zwei Ticks gemischt, Lampe als Spot mit Schatten. | `knorke-runtime` (Beispiel/Test), `knorke-integration` | 0009, 0002 |
| Szene als Text und Agenten-Kommandozeile, Stufe 1 (2026-11-25): Szene als Text (`knorke.scene/1`), Komponenten als Schema-Quelle, Änderungen als `ChangeSet` mit Undo; Kommandozeile `knorke` mit Antworten in JSON. | `knorke-scene`, `knorke-integration`, `knorke-agent` | 0018, 0004 |
| Agenten-Ebene Stufe 2 (2026-11-25): `knorke render` schreibt die Szene nach n Ticks als PNG; Test mit Lichtkegel und Farbänderung per `mutate`. | `knorke-agent`, `knorke-integration` | 0018, 0002 |
| Aufräum-Runde (2026-11-25): Verhaltensgleiche Aufräum-Runde über 59 Dateien (netto −981 Zeilen), Goldens unverändert. | mehrere | – |
| Software-Renderer beschleunigt (2026-11-25): Software-Renderer schneller bei bitgleichen Bildern (Kantenwerte, `visit` generisch, `Palette::nearest`, `Texture::sample`), neuer Benchmark. | `knorke-render`, `knorke-contract-tests` | – |
| Lichtzeilen Schritt L1 (2026-11-26): Erste Zeilen-Lichter (L1): Punkt- und Spotlichter nehmen ihren Schatten aus einer gemerkten Zeile pro Lampe statt aus einem Strahl-Marsch. | `knorke-light2d`, `knorke-integration` | 0019 |
| Lichtzeilen Schritte L2a–L4b (2026-11-26): Zeilen-Lichter mit Profil, hinter halb durchsichtigen Hindernissen, als Linien- und Flächenlicht und durch Muster-Hindernisse. | `knorke-light2d`, `knorke-integration` | 0019 |
| Lichtzeilen ohne Surface-Buffer (2026-11-26): Zeilen-Lichter in Bildern ohne Surface-Buffer (mit Material-Puffer bitgleich zu Chiaro, sonst flaches Licht). | `knorke-light2d`, `knorke-integration`, `knorke-runtime` | 0019 |
| Lichtzeilen mit Sprite-Maske (2026-11-26): Zeilen-Lichter in Bildern mit Sprite-Maske der Anwendung (E21), Zeitmessung gegen den alten Weg. | `knorke-light2d`, `knorke-integration` | 0019 |
| Lichtzeilen standardmäßig an (2026-11-26): `LightSettings::light_rows` ist jetzt Standard, nach neuer stabiler Zeitmessung (ruhende Lampen bis 2,5× schneller). | `knorke-light2d`, `knorke-integration` | 0019 |
| Lichtzeilen mit Masken (2026-11-26): Punkt- und Spotlichter mit Projektor- oder Fenster-Maske als Zeilen-Lichter; Demo komplett auf Zeilen. | `knorke-light2d`, `knorke-integration` | 0019 |
| Harte Sonne aus einer Sonnen-Höhenkarte (2026-11-26): Harte Sonne aus einer zur Sonne gedrehten Höhenkarte (L3), welt-fest und in Bändern erneuert. | `knorke-light2d`, `knorke-integration` | 0019 |
| Weiche Sonne aus der Sonnen-Höhenkarte (2026-11-26): Weiche Sonne (Richtungslicht mit Quellradius) aus derselben Karte mit zweitem Ziel für die Halbschatten. | `knorke-light2d`, `knorke-integration` | 0019 |
| Sonne hinter durchscheinenden und Muster-Hindernissen (2026-11-26): Sonnen-Karte hält bis zu 2 durchscheinende Läufe pro Texel, damit Sonne hinter durchscheinenden und Muster-Hindernissen stimmt. | `knorke-gpu`, `knorke-light2d`, `knorke-integration` | 0019 |
| Kachel-Raster mit Muster in Zeilen und Sonnen-Karte (2026-11-27): `PatternExit` kennt auch Kachel-Raster, Muster in Zeilen und Sonnen-Karte. | `knorke-light2d`, `knorke-integration` | 0019 |
| Zeilen-Licht und Sonnen-Karte durch das Medium (2026-11-27): Zeilen-Licht und Sonnen-Karte durch das Medium (Nebel, Wolken); `frame_suits` schließt solche Bilder nicht mehr aus. | `knorke-light2d`, `knorke-integration` | 0019 |
| Zeilen-Lampen scrollend nicht langsamer als heute (2026-11-27): Schwenk baut Zeilen nur noch bei echter Änderung neu; scrollende Lampen nicht langsamer als vorher. | `knorke-light2d`, `knorke-integration` | 0019 |
| Lücken der Zeilen geschlossen (2026-11-27): Lücken der Zeilen geschlossen: Schattenfilter, `inside_obstacle`, hohe Oberkanten und weitere Sonderfälle. | `knorke-light2d`, `knorke-integration` | 0019 |
| Zeilen ohne Chiaros Schatten-Weg (2026-11-27): Mit `light_rows` baut kein Bild mehr das Distanzfeld; weite Lampen mit mehr Winkeln, Messwerte. | `knorke-light2d`, `knorke-integration` | 0019 |
| Chiaros Schatten-Weg gelöscht (2026-11-27): Chiaros Schatten-Weg samt Distanzfeld und zugehörigen Programmen gelöscht, Zeilen sind das Licht. | `knorke-light2d`, `knorke-integration`, `knorke-gpu`, Vertragstests | 0019 |
| Muster-Schatten weich statt treppig (2026-11-27): Schatten von Muster-Hindernissen weich statt blockig und hart (Interpolation im Muster-Atlas). | `knorke-integration` (Shader), `knorke-light2d` | 0019 |
| Audio Stufe 1 (2026-11-27): `knorke-audio` als Mischpult ohne Gerät: Stimmen, Busse mit Ducking, Raumklang nach Wizardry-7-Modell, Messwerte. | `knorke-audio` | 0020 |
| Audio am Gerät (2026-11-27): `knorke-audio-sdl` öffnet das Standard-Ausgabegerät über SDL3, Rückruf wartet nie und alloziert nicht; am echten Gerät gelaufen. | `knorke-audio-sdl` | 0020 |
| Audio Stufe 2 (2026-11-27): Middleware-Lücken geschlossen: sample-genauer Start, Schall-Laufzeit, Höchstzahl je Klang, Filter pro Bus, Ducking; Messwerte. | `knorke-audio` | 0020 |
| Retro-Synthesizer Stufe 1 (2026-11-28): `knorke_audio::chip` rendert Noten im Stil von NES, Game Boy, C64, AdLib und Amiga in einen Host-Puffer (mono PCM für den Mixer), deterministisch über `det`; Tests `tests/chip.rs`, Beispiel `chiptune`. FM ist eine Nachahmung; echter OPL2-Klang mit Feature `opl` über `oplon` (MIT, `render_opl`, 9 Kanäle, Patches Blech/Orgel/Bass/Glocke). Stealth-Hof: Alarm als NES-Arpeggio, Fangen als C64-Sägezahn mit Rutsch. | `knorke-audio`, `knorke-host` | 0020 |
| Shader-Paket ohne Chiaro-Ordner (2026-11-27): Shader-Paket `knorke-chiaro` baut nur noch aus dem Repo, 48 genutzte Chiaro-Programme übernommen. | `knorke-integration`, `knorke-light2d`, `knorke-gpu`, `xtask` | 0019 |
| Letzte Chiaro-Lücken, Port abgeschlossen (2026-11-27): Port abgeschlossen: Farbtemperatur `kelvin` bitgleich und weitere letzte Chiaro-Lücken. | `knorke-post`, `knorke-light2d` | 0014 |
| Shader aufgeräumt (2026-11-27): Sieben doppelte Shader-Hilfsdateien liegen einmal in `shaders/shared`, toter Code entfernt. | `knorke-integration` (Shader), `xtask` | 0019 |
| Rust-Code aufgeräumt (2026-11-27): Toter und doppelter Code in `knorke-light2d` entfernt, Verhalten und Fingerprint unverändert. | `knorke-light2d` | 0019 |
| Plan-Abschnitt 16 gekürzt (2026-11-28): lange Berichte nach `docs/stand.md`, hier eine Zeile je Baustein (17.15). | `docs/plan.md`, `docs/stand.md` | – |
| Eingabe, ADR 0022 Schritte 1–2 (2026-11-28): Abhängigkeitsregeln für `knorke-host`/`knorke-input` samt Prüfung „`test-support` nur in Dev-Abhängigkeiten“; `knorke-input` (`no_std`) mit `InputEvent` für Tastatur, Maus, Gamepad, `InputState` mit Flanken pro Tick und `Bindings<A>`, Vertragstest ohne Allokation. | `knorke-input`, `xtask`, `knorke-contract-tests` | 0022 |
| Fenster-Host, ADR 0022 Schritte 3–4 (2026-11-28): `knorke-host` (Feature `sdl`) mit Fenster, Uhr `SDL_GetTicksNS` (Sprung gekappt auf 250 ms), SDL-Ereignisse → `InputEvent`, `begin_frame`/`end_frame` ohne Schleife und Rückrufe; Gerät, sRGB-Swapchain (`SDR_LINEAR`), Shader-Paket aus `KNORKE_SHADER_DIR` bzw. `shaders/` neben der Binary mit Prüfsumme und Diagnostics `knorke.host.shader_pack.missing`/`.stale`, `knorke.host.device.unavailable`. Beispiel `clear` zeigt die Löschfarbe. | `knorke-host` | 0022 |
| Stealth-Hof im Fenster, ADR 0022 Schritt 5 (2026-11-28): `stealth_view::View::draw_into` zeichnet in geliehenen Command Buffer und geliehenes Ziel (Upload in derselben Kopie-Pass, kein Rücklesen); der PNG-Weg `draw` nutzt denselben Ablauf. Stealth-Profil mit `Command::Step(Dir)` (Tag 1, `Goto` bleibt Tag 0). `Runtime::ticks_due` fragt ohne zu ticken, ob ein Tick fällig ist, damit das Spiel Eingabe nur dann abtastet. Beispiel `knorke-host/examples/stealth_yard.rs`: Pfeiltasten → `Bindings` → `Step` → `Runtime::frame` mit Host-Zeit, Shader-Paket über den Host (Prüfsumme aus den Quellen). | `knorke-host`, `knorke-integration`, `knorke-runtime` | 0022 |
| Sitzung aufzeichnen, ADR 0022 Schritt 6 (2026-11-28): Spielschleife ohne Fenster in `examples/stealth_yard/session.rs` (Treiber `Option<Recorder>`), Beispiel mit `--record <datei>` (am Ende `verify` im Speicher, Ergebnis auf stderr, Datei schreiben) und `--replay <datei>` (ohne Fenster lesen und prüfen). Replay-Kopf mit Szenen-Kennung: `stealth::replay_header` (Prüfsumme des kanonischen Szenentexts), `stealth::read_replay` lehnt fremde Szene (`knorke.stealth.replay_scene`) und unlesbare Datei (`knorke.stealth.replay_file`) ab. Test `knorke-host/tests/stealth_session.rs`: synthetische Eingabe mit wechselnden Bildzeiten besteht `verify`, 30 und 144 Bilder/s ergeben dasselbe Log, Datei besteht nach dem Lesen `verify`. | `knorke-host`, `knorke-integration` | 0022 |
| Ton im Stealth-Hof, ADR 0022 Schritt 7 (2026-11-28): `Host::open_audio` (Feature `audio`, Code `knorke.host.audio.unavailable`) öffnet das Gerät über `knorke-audio-sdl`. `examples/stealth_yard/sound.rs` liest Positionen und `Event`s der Simulation und schickt Mixer-Commands: Wind als Schleife, eigene Schritte, Schritte der Wache räumlich (Hörer = Eindringling), Alarm bei `Spotted`, Ton beim Fangen; Klänge im Code erzeugt. Ohne Feature baut `knorke-audio-sdl` nicht. Von Hand angehört. | `knorke-host`, `knorke-audio-sdl` | 0022 |
| Zweiter Aufräum-Durchgang (2026-11-27): Zweiter Aufräum-Durchgang: Maske immer mit Höhen, vier eingefrorene Chiaro-Programme entfernt. | `knorke-light2d`, `knorke-gpu` | 0019 |
| Snapshot-Vertrag (`Snapshot`, `SnapshotBuf`, `SnapshotValue`), `Checksum` pro Modul-Abschnitt | `knorke-types` | 0010 |
| Replay: `Recorder`/Flight Recorder, `verify` (Tick + Modul), `seek`, `minimize` (Delta-Debugging) | `knorke-replay` | 0010 |
| Fehlererkennung: `Invariants`, `ProgressWatch`, Bildheuristiken (`frame_diff`, `flicker_score`, `non_finite`) | `knorke-observe` | 0010 |
| Geometrie `Aabb2`, `Circle`, `Segment2`, `Capsule2`, `Ray2`; Snapshot-Unterstützung aller Mathe-Typen | `knorke-math` | 0010 |
| Projektregeln für Agenten und Menschen | `CLAUDE.md` | – |
| Plattformunabhängige Mathematik `det` (libm), sRGB-Tabellen, Clippy-Verbot plattformabhängiger Funktionen/Uhren/Zufalls-HashMaps | `knorke-math`, `clippy.toml` | 0011 |
| Determinismus-Fingerprint (eingefroren; CI: Windows, Linux, macOS/ARM, WASM) | `knorke-contract-tests` | 0011 |
| Abschaltbare Zusatzfeatures ohne Kosten: `TickDriver`/`NoRecord`, nullgroße No-op-Typen, Messung „aus vs. nicht vorhanden“ | `knorke-replay`, `knorke-contract-tests` | 0011 |
| Review der Grundlagen (2026-11-23): Review der Grundlagen mit je einem Test: `EventQueue`, `Slots`, `HandleMap`, Snapshot-Prüfungen, `Parallel::for_each_range*`. | `knorke-types` | 0006, 0008, 0010 |
| Review: `det::min`/`det::max` (std-Varianten sind nicht deterministisch, gemessen Debug ≠ Release), `det::powi` für alle Exponenten, Schnitt-Tests unabhängig vom Maßstab der Welt, definierte Antworten für entartete Formen, `Pod`-Impls aus Feldlisten geprüft, Clippy-Regeln erweitert | `knorke-math`, `clippy.toml` | 0006, 0011 |
| Review: `ReplayLog::read` prüft Tick-Folge und Keyframes, Index-Überlauf als Panic statt Umbruch, `seek` sucht binär; Tests für Fensterränder des Flight Recorders und 1-Minimalität | `knorke-replay` | 0010 |
| Fingerprint auf `wasm32-wasip1` lokal belegt (Node/V8, `xtask/wasi-run.mjs`) | `knorke-contract-tests`, `xtask` | 0011 |
| Sagax-Umbau: Sagax-Umbau (`fab00c0`): Raumtypen, Wegsuche, Wahrnehmung und KI-Entscheidungen auf `knorke-grid`, `-navigation`, `-perception`, `-ai`, `-integration` verteilt. | `knorke-grid`, `-navigation`, `-perception`, `-ai`, `-integration`, `knorke-observe`, `xtask` | 0001, 0010, 0012 |
| pixelraum migriert: pixelraum migriert: Software-Renderer, Licht und Schatten auf G-Buffer-Spalten über `Parallel`, Golden Images, Verbindung in `knorke-integration::pixel`. | `knorke-render`, `knorke-lighting`, `knorke-integration`, `knorke-contract-tests` | 0006, 0008, 0013 |
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
| **Task-Graph-Scheduler** (Systeme nach Lese-/Schreibzugriff parallel) | geplant | ADR 0009; die feste Tabelle (`run_systems`) reicht, bis ein Tick gemessen parallel lohnt |
| **Wachmann-Szene mit Bild**: `guard_scene` über die Laufzeit zeichnen (GPU-2D-Licht, Phasen „Render vorbereiten“/„Render“ aus ADR 0009) | erledigt 2026-11-25 (Abschnitt 16) | – |
| **Licht beeinflusst Wahrnehmung**: der Wachmann sieht in `guard_scene` unabhängig vom gezeichneten Licht; `SenseMap::light` aus den Lichtern der Szene speisen (CPU-Seite, deterministisch, nicht von der GPU zurücklesen) | offen | sobald Schleichen im Dunkeln gespielt werden soll |
| **Leistung der Medium-Lichtstreifen**: Szene `shafts` ist in Knorke in drei von drei Läufen langsamer als in Chiaro (Median etwa +34 %, bei ±25 % Streuung pro Seite; Messwerte in `docs/chiaro-port/ABLAUF.md`, 2026-11-25) | offen | Pässe und Zielwechsel im Polar-/Scatter-Pfad gegen Chiaro zählen, dann messen |
| **Chiaro-Port: Medium, Rauch, Ringlicht** (P12, P21, P29, P30, Medium-Draw, vier Medium-Szenen) | erledigt 2026-11-25, alle bitgleich (Protokoll in `docs/chiaro-port/ABLAUF.md`) | Abschluss „Ende“ erledigt 2026-11-27 (Kelvin, Wolken-Helfer, Muster-Warnungen nachgezogen; C++-Chiaro archiviert). Rest für später: D3D12/MSL, Damage-Grade/Arcade-CRT/CRT-Krümmung |
| **2D-Licht grundsätzlich neu denken** (Manuel, 2026-11-25): `knorke-light2d` ist mit ~29 000 Zeilen plus ~6 000 Zeilen HLSL zu groß. Die Funktionen sollen im Prinzip bleiben, aber nicht in dieser Breite und Tiefe. Kandidat für den Kern: Radiance Cascades (17.11 Nr. 6) – ein Verfahren für Schatten, weiches und indirektes Licht statt vieler Einzelpässe | geplant – Recherche und Entwurf fertig (2026-11-25): **ADR 0019 (vorgeschlagen)** empfiehlt „Lichtzeilen“ (Sicht einmal pro Lampe statt Strahl-Marsch pro Pixel) statt Radiance Cascades als Kern; Einzelheiten in 17.13. Bis Bild und Messung den Ersatz belegen, wird am Chiaro-Licht weder optimiert noch gekürzt noch gelöscht | Entscheidung Manuel über den Entwurf; danach der Versuch aus ADR 0019 (Grundlinie messen, drei Vergleichsbilder, Zeitmessung) |
| **Füllpfad des Software-Renderers**: nach der Beschleunigung bleiben ~11 ns pro geschriebenem Pixel (Vermutung des Agenten: Division `1/inv_z` und getrennte Speicherungen in mehrere `Vec` mit Grenzprüfungen); `begin`/`end` kosten bei 640×360 allein ~3 ms | offen | nur mit Profil und Messung (`--bench render`) |
| **Fenster und Echtzeit-Host**: die Szene läuft offscreen und schreibt PNGs; Gerät und Shader-Pack kommen noch aus `knorke_gpu::test_support` | erledigt 2026-11-28 (ADR 0022 angenommen, Schritte 1–8: Regeln, `knorke-input`, `knorke-host` mit Fenster, Uhr, Ereignissen, Gerät, Swapchain, Shader-Paket; Stealth-Hof im Fenster mit Pfeiltasten, Beispiel `stealth_yard`; `--record`/`--replay` mit `verify` und Szenen-Kennung; Ton mit Feature `audio`) | – |
| **`knorke-scene` und Agenten-CLI** (Phase 3): Szene als Daten statt als Rust-Code, `discover`/`inspect`/`mutate`/`validate`/`execute` | Stufe 1 erledigt 2026-11-25 (Abschnitt 16, ADR 0018) | – |
| **Agenten-Ebene Stufe 2**: `render` als Operation (Bild aus der Kommandozeile) | erledigt 2026-11-25 (Abschnitt 16, ADR 0018) | – |
| **Agenten-Ebene Stufe 3**: `query`/`observe`/`verify`, Replay-Operationen, Agenten-Skript als ein Batch, MCP-Adapter; Bild auf D3D12/Metal (heute nur Vulkan, weil das Paket SPIR-V ist) | geplant | nach dem Abschluss des Chiaro-Ports |
| **Profile melden sich an** (Capability-Manifest, ADR 0004): `knorke-agent` kennt fest das Profil `stealth` | offen | mit dem zweiten Profil |
| **Szenen-Kennung im Replay-Kopf**: ein Snapshot passt nur zu der Szene, aus der die Simulation gebaut wurde; heute prüft `restore` nur Zahl der Wachen und Positionen | erledigt für das Stealth-Profil (2026-11-28, ADR 0022 Schritt 6: `stealth::replay_header`/`read_replay`); andere Profile offen | mit `replay.verify` als Agenten-Operation |
| **Prefabs, Assets, Referenzen zwischen Entities, Schema-Migration** in `knorke-scene` | offen | sobald eine zweite Szene sie braucht |
| **Agent ändert laufende Simulation** (Hot-Reload am Sync-Punkt statt Neubau pro `execute`) | offen | mit einem interaktiven Host |
| **Pipelining** Simulation Tick N+1 ∥ Render Frame N | geplant | nach erstem Vertical Slice messen |
| **Handle-Reservierung aus parallelen Chunks** (atomarer Zähler in `Slots`) | offen | sobald ein System parallel spawnt |
| **Gemeinsamer Raum-Index** (Broadphase/Spatial Hash für Navigation, Wahrnehmung, Culling, Audio) | offen | wenn zwei echte Module ihn brauchen |
| **Frame-/Render-Graph** (Pässe mit deklarierten Ressourcen) | geplant | zuerst expliziter Pass-Plan in `knorke-gpu` (ADR 0002) |
| **GPU-getriebenes Rendering** (Culling/Sortierung per Compute) | offen | nach Identifikation der Grafikbibliothek; SDL3-GPU hat Compute |
| **GPU-Timings** | offen | SDL3-GPU-Möglichkeiten prüfen (ADR 0007) |
| **Hot-Reload** von Assets, Shadern, Parametern | geplant | mit `knorke-scene`, Anwendung an Sync-Punkten |
| **Entity-Relationen** („Kind von“, „zielt auf“) als eigene Struktur | offen | bei Bedarf in KI/Szenengraph |
| **Kleines Asset-System** (Einschätzung A2): PNG und WAV über Asset-IDs aus einem Ordner, genaue Schreibweise auf allen Systemen, synchron beim Start, Diagnosen `knorke.assets.*` | vorbereitet (ADR 0023, Vorschlag, 2026-11-28); entschieden: eigener WAV-Leser, IDs aus Dateinamen | nächster Schritt nach ADR 0022; Schritt 4 erst nach dem laufenden Ton-Chat |
| **Asset-System** (asynchrones Laden, Referenzzählung, Streaming) | geplant | Phase 3 mit `knorke-scene`, baut auf den IDs aus ADR 0023 auf |
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
| Zeit: Tick + Interpolationsfaktor pro Frame | vorbereitet (`FixedStep::alpha`, `Frame::alpha`) | ADR 0003/0009; Animation läuft in „Render vorbereiten“ |
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
| Ausgabe | erledigt 2026-11-27 (`knorke-audio-sdl`, Abschnitt 16) | SDL3-Audio-Streams; Mixer als eigenes Modul `knorke-audio` oder bestehende Crate (z. B. `kira`) prüfen |
| Räumlicher Klang (Position, Dämpfung, Verdeckung) | offen | Positionen aus `knorke-math`; Verdeckung/Ausbreitung kann das Hör-Modell von Sagax (`hear`, `SenseMap`) wiederverwenden |
| Abgrenzung | – | Audio ist Darstellung, nicht Simulation; was ein NPC hört, läuft über Wahrnehmung, nicht über den Mixer |

**Entwurf `knorke-audio` (2026-11-27; Stufe 1 umgesetzt, ADR 0020, siehe Abschnitt 16):**

| Baustein | Status | Bemerkung |
|---|---|---|
| Zuschnitt wie GPU (ADR 0002) | erledigt (Stufe 1) | Host besitzt Gerät und Takt (SDL3 oder `cpal`); `knorke-audio` rendert nur in einen geliehenen `&mut [f32]`-Puffer. Ohne Gerät testbar: N Frames rendern, Prüfsumme vergleichen (bitgleich, da eigene Mathematik) |
| Steuerung | erledigt: `Command` + `Mixer::apply`; die Queue wählt der Host (z. B. `rtrb`), Host-Anbindung an SDL3 offen | Spiel schickt semantische Befehle (`Play { sound, at, gain }`, `Stop`, `SetListener`) über eine SPSC-Queue an den Audio-Thread; Sim-Events → Klänge am Sync-Punkt. Replay erzeugt dieselben Klänge, kein eigener Snapshot nötig |
| Alte Formate | offen; nächster Schritt: Generator-Quelle für Datenströme (OPL-Musik, Synthesizer) statt nur PCM | Lader pro Spiel, nicht in der Engine: VOC (8-Bit unsigned PCM, 4–22 kHz), Wizardry `SOUND*.SND`, OPL/AdLib-Musik über Emulator (Lizenz prüfen: Nuked OPL3 ist LGPL, `ymfm` BSD). Engine kennt nur PCM |
| Aufbessern zur Laufzeit | teilweise: Varianten über `Play::pitch`/`gain` (Zufall wählt das Spiel), Hall je Raum, Filter je Bus (2026-11-27); offen: besseres Resampling (heute linear), EQ, Kompression, Bass-Schicht, Stereo-Verbreiterung | gutes Resampling (Sinc statt linear), Varianten (Tonhöhe/Lautstärke aus eigenem Zufall – Darstellung), EQ, Kompression/Sättigung für „wuchtiger“, Bass-Schicht, Stereo-Verbreiterung von Mono, Hall je Raum. Umschalter Original/Verbessert pro Spiel |
| Aufbessern offline | offen | Asset-Pipeline: Entrauschen, KI-Upsampling, neu eingespielte Ersatzklänge; Ergebnis als normales PCM-Asset |
| Raumklang Stufe 1 | erledigt (plus Wände, „hinter mir“, Ohr-Versatz) | Panning + Entfernungsdämpfung relativ zu Hörerposition/Blickrichtung (`knorke-math`) |
| Raumklang Stufe 2 | teilweise: Wände wirken (`Command::Occlude`), das Zählen auf dem Raster macht noch das Spiel | Verdeckung über `knorke-grid` (Wand dazwischen = Tiefpass), Weg um Ecken über `knorke-navigation`/Sagax-`hear`, Hall aus Raumgröße |
| Raumklang Stufe 3 | offen | HRTF (binaural für Kopfhörer), ggf. Ambisonics; Kandidaten Firewheel (17.11 Nr. 17) oder Steam Audio – erst nach Spike mit Messung |
| Aus = kostenlos (ADR 0011) | erledigt: ohne `Mixer` keine Kosten; Echo ohne Raum wird übersprungen, ferne Stimmen laufen virtuell | ohne Audio kein Thread, keine Queue; Effekte als Option pro Bus |

**Erfahrung aus Wizardry 7 `revised/` (Antwort der Codex-Session 2026-11-27, Code `wizardry7/revised/platform/sdl/audio_sdl.c`, gelesen):**
- Alte Mono-Samples bleiben unverändert; der Mixer gibt je nach Position und Blickrichtung Pegel, Dämpfung, eine kurze Laufzeitdifferenz links/rechts (bruchteilige Verzögerung bis ~29 Samples plus Kopfschatten) und einen dumpfen Eindruck für „hinter mir“.
- Leise Raumreflexionen je Raumart (kleiner Raum, Mauerwerk, Höhle) als kurze Stereo-Echo-Muster – billig, ohne Hallgerät.
- UI-Signale und OPL-Musik hängen nicht an der Weltposition (eigener Bus). OPL-Musik bekommt leichte Bass- und Raumanreicherung.
- Musikwechsel: zwei OPL-Chips, der alte klingt aus, während der neue startet (Überblenden).
- Pegelreserve vor dem Resampling des Geräts, wenn laute Klänge sich überlagern.
- Lizenz: dort Nuked OPL3 (LGPL-2.1). Für Knorke (MIT/Apache) nicht fest einbauen – `ymfm` (BSD-3) prüfen oder OPL nur im Spiel-Lader.
- Prüfvorschlag: wenige A/B-Szenen (vorne/hinten, links/rechts, nah/fern, mehrere Stimmen gleichzeitig), mit Kopfhörern und Lautsprechern anhören. Dazu bei uns: dieselben Szenen offline rendern und per Prüfsumme festhalten.

**Was moderne Engines können – und wohin es bei uns gehört (Recherche 2026-11-27):**

Quellen: Unreal 5 Doku (MetaSounds, Audio Modulation, Quartz, Sound Concurrency) – belegt; Rockstar GDC 2014 „Sound of GTA V“ (Distanz, Verdeckung, Reflexionen, Hall, Mix-Kategorien, Musik-Quantisierung, Stimmen-Virtualisierung) – belegt; GTA VI: keine offiziellen Audio-Angaben, alles darüber nur Sekundärquelle/Spekulation; Wwise Spatial Audio (Rooms & Portals, Beugung → Obstruction) – belegt; Steam Audio (HRTF, gebackene Reflexionen und Pfade, Rust-Binding `audionimbus`) – belegt.

| Fähigkeit | Was sie tut | Engine oder Spiel |
|---|---|---|
| Stimmen-Verwaltung (Concurrency, Priorität, Virtualisierung) | Höchstzahl gleichzeitiger Klänge je Gruppe; unwichtige leise Klänge laufen „virtuell“ weiter, ohne Rechenzeit | Engine |
| Busse, Mix-Zustände, Ducking | Gruppen (Musik, Sprache, Effekte); Zustand „Dialog“ senkt Musik ab; Seitenkette: Explosion drückt kurz alles andere | Engine (Mechanik), Spiel (Werte) |
| Parameter/Modulation (RTPC) | Spielwert (Tempo, Gesundheit, Regenstärke) steuert Lautstärke, Filter, Tonhöhe, Musikschichten | Engine (Mechanik), Spiel (welcher Wert) |
| Verdeckung und Hindernis | Wand/Tür zwischen Quelle und Hörer: leiser und dumpfer; Tür auf = Klang kommt durch die Öffnung | Engine, aus Raster/Szene |
| Räume und Portale | Hall je Raum; Klang aus Nachbarraum kommt aus Richtung der Tür, um Ecken gebeugt | Engine; mit Raster/Navigation |
| Physik der Luft | Entfernungsdämpfung, Höhenverlust über Entfernung, Schallverzögerung (Blitz vor Donner), Doppler | Engine |
| HRTF / 3D für Kopfhörer, Objekt-Audio (Atmos) | Oben/unten/hinten hörbar | Engine, spät |
| Ray-traced bzw. gebackene Akustik | Reflexionen aus echter Geometrie, vorab berechnet oder live | später, nur nach Messung; Steam Audio als Kandidat |
| Prozedurale Klänge (MetaSounds) | Regen, Wind, Motoren, Klicks aus Synthese statt Samples; Klanggraph aus Bausteinen | Engine (Graph + Bausteine), Spiel (Klänge) |
| Varianten | Zufällige Tonhöhe/Lautstärke/Sample-Auswahl gegen „Maschinengewehr-Effekt“ | Engine |
| Musik-Taktgeber (Quartz) | Klänge sample-genau auf Takt oder Schlag legen | Engine |
| Adaptive Musik | Überblenden ins nächste Stück auf dem nächsten Takt, Schichten zu- und abschalten (Kampf = Trommeln dazu), kurze Einwürfe (Stinger) | Engine (Überblenden, Takt, Schichten), Spiel (welches Stück wann) |
| Reaktion auf Spiel-Events | Event „Tür auf“, „Alarm“, „Treffer“ → Klang; Abbildung Event → Klang als Daten | Spiel/Host; Engine nimmt nur Commands |
| Sound verfälschen | Funk-, Telefon-, Unterwasser-, Betäubt-Filter; Zeitlupe senkt Tonhöhe | Engine (Effekte), Spiel (wann) |
| Debug | Anzeige aktiver Stimmen, Pegel je Bus, Verdeckungsstrahlen; Aufnahme des Mix für Tests | Engine (`Inspect`, `Probe`) |
| Retro-Synthesizer (8/16 Bit) | Stufe 1 erledigt 2026-11-28 (`knorke_audio::chip`: Puls mit PWM, Stufen-Dreieck, Säge, NES-Rauschen, 2-Operator-FM, ADSR, Arpeggio, Vibrato, Slide; Chip-Charakter NES/Game Boy/C64/AdLib/Amiga über Lautstärkestufen, Bits, Sample-and-Hold, Tiefpass; rendert Noten in PCM für den Mixer; Beispiel `chiptune` schreibt WAVs). Echter OPL2 über `oplon` (Feature `opl`) ebenfalls erledigt. Offen: SID-Emulation (Kandidat `sidera`, gleicher Autor wie `oplon`, prüfen), SID-Filter mit Resonanz, Ringmodulation/Sync, Tracker-Format, Abspielen als Datenstrom. Wunsch 2026-11-27: Chiptune-Klänge nachahmen: Rechteck mit Pulsbreite, Dreieck, Rauschen, Hüllkurven, Arpeggio, Bitcrusher/Sample-Rate-Reduktion; FM-Klang wie AdLib/OPL kommt über den OPL-Emulator (siehe „Alte Formate“). Sound-Blaster-Familie: FM-Teil = OPL2 (AdLib, SB 1/2), 2× OPL2 links/rechts (SB Pro 1), OPL3 (SB Pro 2, SB16) – ein OPL3-Emulator deckt alles ab; Digital-Teil (VOC) ist normales PCM, Klangcharakter der alten Karte optional über Bitcrusher + Tiefpass | Engine als optionaler Baustein im Klanggraph (eigene Crate erst bei Verbraucher), Spiel liefert Klänge/Noten |

**Umsetzungsstand dieser Liste (2026-11-27, Stufe 2, siehe Abschnitt 16):**

| Fähigkeit | Status | Bemerkung |
|---|---|---|
| Musik-Taktgeber: sample-genauer Start | erledigt (`Play::start`, `Mixer::clock`) | Wie Quartz: Befehl im Voraus schicken, Start fällt mitten in einen Rückruf. Tempo und Takt rechnet das Spiel; ein eigenes Metronom erst bei Verbraucher |
| Schall-Laufzeit (Blitz vor Donner) | erledigt (`Config::sound_speed`) | Verzögerung nur beim Start, aus der Entfernung zum Hörer |
| Doppler | offen | billig machbar (Tonhöhe aus Änderung der Entfernung je Block), aber bei Raster-Spielen springen Positionen pro Zug → Tonhöhe würde wackeln; erst mit Glättung und echtem Verbraucher (3D, ADR 0021) |
| Höchstzahl je Klang (Concurrency) | erledigt (`Play::limit`) | ersetzt die Stimme, die am weitesten durch ist; Gruppen über mehrere Klänge offen |
| Seitenketten-Ducking | erledigt (`Command::Duck`) | hört den Bus vor dessen Bus-Lautstärke ab: auch ein stumm gedrehter Sprach-Bus duckt die Musik |
| Sound verfälschen (Unterwasser, Funk, Betäubt) | teilweise: Filter je Bus (`Command::Filter`) | Verzerrung, Bitcrusher, Zeitlupe-Tonhöhe für alle Stimmen offen |
| Parameter/Modulation (RTPC) | offen | heute über `Gain`/`Pitch`/`Filter`-Befehle je Tick; Kurven als Daten erst mit Verbraucher |
| Pegelanzeige je Bus (Debug) | offen | `Stats` kennt nur den Gesamtpegel |
| Besseres Resampling | offen | heute linear; kubisch (4 Stützstellen) erst nach Hörvergleich (ADR 0020) |

Zusatzquelle: Unreal-Doku „Overview of Quartz“ (sample-genaue Planung auf dem Audio-Thread, Start mitten im Puffer) – belegt; Forenhinweis, dass die Blueprint-Seite nur bildrate-genau ist – Sekundärquelle.

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
| Fester Tick, Sub-Steps, Interpolation für die Darstellung | vorbereitet (fester Tick: `knorke-runtime`) | ADR 0003/0009; Physik läuft in der Simulationsphase, Rendering interpoliert |
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
| 6 | **Holographic Radiance Cascades** – 2D-Globalbeleuchtung ohne zeitliche Akkumulation und Entrauschen, konstante Kosten (1,9 ms bei 512², 7,7 ms bei 1024², RTX-3080-Laptop) | F, erste Implementierungen | Stärkster Kandidat für `knorke-lighting` neben den Chiaro-Verfahren: deterministisch pro Frame → kein Geisterbild, Screenshot-/Replay-Tests stabil. Reiner Compute → SDL3-GPU-Compute. Für schwache GPUs Kaskaden-Auflösung/Fallback. **Nachgeprüft 2026-11-25 (Paper gelesen, 17.13):** kennt keine Höhen, Schatten etwa 4 Probenabstände weicher, kleine Lichter fehlerhaft, alle gefundenen Umsetzungen brauchen Compute (hat `knorke-gpu` nicht) | verworfen als Kern des 2D-Lichts (ADR 0019); offen als spätere Zusatzstufe für leuchtende Flächen und indirektes Licht | arxiv.org/abs/2505.02041 |
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
| 8 | Feature-Matrix vollständig, C++-Chiaro archivieren | erledigt 2026-11-27 | Chiaro liegt unter `D:\Entwicklung\_Archiv\chiaro`; Liste „bewusst nicht portiert“ in `docs/chiaro-port/ABLAUF.md` (Protokoll 2026-11-27) |

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
- **Demo-Szenen aus P3** (10 Szenen, 1280×720) – **teils erledigt** (2026-11-25): `wet_street`, `iso_night_plain`, `iso_night_hard` bitgleich zu den Goldens (HUD-Rechteck ausgenommen), helfergezeichnete Surface-Layer bytegleich zu Chiaro; Farb-, Material- und Glow-Layer als Fixtures aus Referenzprogrammen (`tests/golden/chiaro/{wet_street,iso_night}/`), weil die Demos sie mit SDL_Renderer malen. Dafür P15/P28 (Muster) portiert (Entscheidung Manuel 2026-11-25). **Offen:** `iso_night_t5`, `iso_night_cutaway`, `iso_night_grade_crt`, `smoke_blast` brauchen Medium (P12, P29, P30), `smoke_blast` auch Ringlicht (P21) – Entscheidung Manuel, ob portieren; vier Szenen (`cpp_renderer_*`, `normal_map*`) sind nicht deterministisch (Uhr/Maus). Manifest: HUD-Rechteck von `iso_night_plain` ist 6 px zu schmal (Test gleicht per Overlay aus).
- **Leistung Pass-Plan gegen alten Spike** – **erledigt**: Messung 2026-11-25 abends (AMD Radeon RX 6700 XT, Vulkan, Release, Wand-Uhr pro Frame inkl. Submit+Warten, je Lauf 3000 Frames, CPU-Grundlast 5–19 %, kein Build nebenher; Rust über `KNORKE_SCENE_BENCH`, C++ über `gate_ref.c` gegen Chiaro 9bc5b9b, in 8 Runden abwechselnd): `minimal_normal_map` Median Rust 184–268 µs (17 Läufe, Mitte 207) gegen C++ 185–271 µs (8 Läufe, Mitte 223); `gate_sprite_shadow` Rust 443–705 µs (17 Läufe, Mitte 547) gegen C++ 454–624 µs (9 Läufe, Mitte 536). **Ergebnis:** kein Unterschied außerhalb der Streuung; die Läufe streuen je Seite um etwa ±20 %, ein Unterschied unter ~15 % ist damit nicht ausschließbar. Die Werte aus ADR 0015 (405–427 µs) erreicht heute auch C++ nicht – sie taugen nicht als Maßstab, nur der Vergleich in derselben Sitzung.
- **`check_invariants` des Pass-Plans** ist eine Methode mit `Sink`, kein `knorke_observe::Invariants` (Lebensdauer `'d`) – **erledigt (bleibt so)**, bei P32 angesehen: `Invariants<W>` ist eine Regelliste des Hosts über seine Welt; der Plan liefert die Prüfung, der Host kann sie dort als Regel eintragen.

Aus P32/P33 (Debug-Prüfung, Layers; 2026-11-25):
- **GPU-Debug-Prüfung** – **erledigt**: opt-in über `DebugChecks::LAYERS`, Readback nur dann (ADR 0017 Punkt 8), aus = nichts angelegt; Bild unverändert und 0 Allokationen belegt. **Medium-Prüfung im Plan** – **offen** bis P29 (Stufe nimmt das Medium schon, getestet).
- **Layers** in `knorke-integration` – **erledigt**: nur GPU-Layers. **Layer-Größe = Szenengröße** wird nicht geprüft (der Pass-Plan meldet falsche Puffergrößen selbst) – so gewollt.
- **Eigene Shader für die 2D-Helfer** (Sprites, Surface-Geometrie zeichnen; in Chiaro machte das SDL_Renderer) – **erledigt für Sprites (P34)**, Surface offen (P35) (Entscheidung Manuel 2026-11-25): Übersetzen mit DXC statt shadercross (`D:\Dev-Tools\dxc`, SPIR-V bytegleich zu Chiaros Blobs geprüft an `output.frag`), Ablage laut ADR 0014 §3. **MSL für eigene Shader** – **offen** (DXC kann kein MSL; shadercross oder SPIRV-Cross nötig).
- **Sprites** (P34) – **erledigt**: Karten-Texel werden exakt geschrieben statt geblendet (Abweichung von Chiaro, dessen Blend nasse Material-Einträge mit Alpha 224 mischte). **Stehende Sprites** – **offen** (P35, brauchen die Surface-Kodierung). **Streaming-Upload** – **vertagt**: kein Verbraucher; kommt mit der Brücke CPU-Bild → GPU-Licht.
- **`SV_VertexID` und erster Vertex** – nicht nachgeschlagen (Wissen des Agenten, keine Quelle geprüft): auf Vulkan zählt er den ersten Vertex mit, auf D3D12 nicht. Eigene Shader bekommen den Startindex deshalb als Parameter. **D3D12-Lauf der eigenen Shader** – **offen**: nur Vulkan getestet.
- **Surface-Geometrie** (P35) – **erledigt**; **stehende Sprites** – **erledigt** (damit ist der Punkt oben geschlossen). `Sprite::new` hat seitdem das Argument `height_step`. **Medium-Draw** – **offen** (P30).
- **Muster (P15/P28), Reste** – Frame-Warnungen für Muster **erledigt** 2026-11-27 (`PatternDestroyed`, `PatternNoAlpha`); **offen**: Atlas-Textur schrumpft mit dem Store (Chiaro wächst nur) – Wirkung auf das Bild nach Schrumpfen nicht gemessen; Rotation nur bei 30° und 17° gegen Chiaro gemessen.
- **Surface-Geometrie und stehende Sprites zusammen in einer Szene** (Golden-Bild) – **erledigt** (2026-11-25): `iso_night_hard`, Surface-Layer bytegleich zu Chiaro.
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

### 17.13 2D-Licht neu denken (Entwurf 2026-11-25)

Auftrag: `docs/prompts/licht-neu-denken.md`. Ergebnis: **ADR 0019 (vorgeschlagen)**. Status insgesamt: **geplant** – wartet auf Manuels Entscheidung, dann der Versuch. Am Chiaro-Licht wird bis zum Beleg nichts gelöscht oder gekürzt.

**Befund zum Bestand** (Code gelesen): Das Verfahren ist schlank (eine Maske mit Höhen in der Bodenebene, Distanzfeld, pro Pixel und Licht ein Strahl-Marsch mit höchstens 64 Schritten). Die Größe kommt aus der Breite: 92 Lichtprogramme als Permutation von 8 Merkmalen, sieben Stores, vier Qualitätsstufen, zwei Schatten-Modi, Caches, Importer, Prüfungen. **Das heutige Licht hat kein indirektes Licht.** Für die Demo-Szenen (`iso_night_*`, 640×360, 36 Lichter) gibt es keinen Zeit-Messwert.

**Recherche** (2026-11-25; „belegt“ = Primärquelle oder Code selbst gelesen):

| # | Ergebnis | Reife | Beleg | Bedeutung für Knorke | Status |
|---|---|---|---|---|---|
| L1 | **Holographic Radiance Cascades**: flache 2D-Ebene aus Emission und Extinktion; 512² in 1,85 ms, 1024² in 7,67 ms (RTX 3080 Laptop, CUDA); Schatten etwa 4 Probenabstände weicher, Muster-Fehler bei Lichtern unter 8 Probenabständen, Schachbrett-Filter nötig, viel Speicher; Ausgabe in 4 Richtungen | F | belegt (arxiv.org/abs/2505.02041) | Für harte Schatten, Spots, Profile und Höhen ungeeignet | verworfen als Kern; offen als Zusatzstufe |
| L2 | **Klassische Radiance Cascades**: Ringe und Lecks ohne „Bilinear Fix“ (etwa 4-fache Strahlzahl); kleine Lampen werden unscharf; üblich mit Distanzfeld | E | belegt (Paper-Quelltext github.com/Raikiri/RadianceCascadesPaper); Bilinear Fix nur über Zusammenfassung von arxiv.org/abs/2408.14425 | wie L1, zusätzlich Ring-Fehler | verworfen als Kern |
| L3 | **Kaskaden mit Höhen / 2.5D**: nichts gefunden (kein Paper, Repo, Shadertoy, Blogpost). Path of Exile 2 rechnet im Bildraum auf dem Tiefenpuffer | – | PoE2 belegt im Paper; „nichts gefunden“ ist das Ergebnis einer gezielten Suche | Höhen müssten selbst erfunden werden | offen (Forschung, nicht geplant) |
| L4 | **Fremde Umsetzungen** (4 Repos gelesen): Licht-Teil 300–600 Zeilen Shader, 250–1 200 Zeilen Host; alle mit Compute-Shadern und Distanzfeld; einzige HRC-Umsetzung unfertig und nicht von den Autoren | E | belegt (Code); Bildzeiten nur Sekundärquelle (8,7 ms bei 4K auf RX 7900 XTX) | `knorke-gpu` hat kein Compute; eine Fragment-Fassung wäre Eigenbau | offen |
| L5 | **Verfahren mit Höhen**: Strahl-Marsch über Höhenkarte (macht Chiaro schon), Horizont-Karten für Höhenfelder, Linien-Sweep für Gelände | P (Technik) | nur Sekundärquelle | Grundlage der Lichtzeilen-Idee | vorbereitet (ADR 0019) |

**Entwurf (ADR 0019):** „Lichtzeilen“ – pro Lampe und Winkel einmal über die Bodenkarte marschieren und bis zu vier Hindernis-Läufe speichern (nahe Kante, ferne Kante, gesperrtes Gefälle-Intervall; damit gehen auch schwebende Hindernisse); jedes Pixel vergleicht nur noch sein Gefälle zur Lampe mit der Zeile. Dieselbe Zeile dient Boden, Wänden und Nebel. Ein Lichtshader statt 92 Programmen, kein Distanzfeld, keine Qualitätsstufen. Geschätzt 3 000–4 500 Zeilen Rust und 600–800 Zeilen HLSL – **nicht belegt**. Tempo: der erste Überschlag war zu günstig; gezählt (Schritt −1) bringt nur die gemerkte Schattenhöhen-Karte ruhender Lampen einen Vorteil, wandernde Lampen kosten etwa wie heute – **Zeit nicht gemessen**. Varianten daneben: A „Chiaro-Verfahren schlank“ (Rückfall), C „Kaskaden als Kern“ (verworfen). Ein unabhängiges Review (Opus, 2026-11-25) hat die erste Fassung zurückgewiesen (ein Eintrag pro Texel statt pro Lauf hätte die Zeile gesprengt; schwebende Hindernisse fehlten, obwohl `iso_night` sie benutzt); die zweite Runde fand zwei Fehler in der Regel (steigende Strahlen, Empfänger unter einem schwebenden Dach). Alles ist eingearbeitet; die Regel in der Endfassung ist mit 59 197 Zufallsfällen gegen dichtes Abtasten nachgerechnet, ohne Abweichung (**belegt**, Wegwerf-Skript). **Nicht geprüft:** Verdrängungsregel, Überlauf bei mehr als vier Läufen, Winkel-Raster, weiche Schatten, Tempo.

**Nächster Schritt – der Versuch** (erst nach Zustimmung; Einzelheiten und Abbruchkriterien in ADR 0019):

| Schritt | Inhalt | Status |
|---|---|---|
| −1 | Zählen auf der CPU, ohne GPU-Code: Läufe pro Winkel, Anteil falsch entschiedener Pixel, Zugriffe für den Aufbau gegen den heutigen Strahl-Marsch (`iso_night_hard`). Abbruch von B, wenn > 1 % falsch oder Aufbau teurer als der heutige Marsch | **erledigt 2026-11-25** (Wegwerf-Skript, 13 Lampen, 54 205 Bodenpixel; Zahlen in ADR 0019): mehr als 4 Läufe in 0,11 % der Richtungen; 0,24 % der Bodenpixel falsch (Schattenkanten, Winkel-Raster); heutiger Marsch nur 6,6 Schritte pro Pixel und Lampe (angenommen waren 12). **Ruhende Lampe mit Schattenhöhen-Karte: etwa ein Viertel bis ein Achtel der heutigen Textur-Zugriffe. Wandernde Lampe: nicht billiger als heute** (zweites Abbruchkriterium knapp gerissen). Nur Zugriffe gezählt, keine Zeit gemessen |
| 0 | Grundlinie: heutiges Licht auf `iso_night` (640×360) und einer Last-Szene (Lichter vervierfacht; bei 640×360, nicht 1920×1080) messen | **erledigt 2026-11-25** (RX 6700 XT, Vulkan, Wand-Uhr, Median): 13 Lampen 380 µs, 52 Lampen 1 118 µs, Sockel ohne Lampen 187 bzw. 322 µs |
| 1 | Bild 1: Schrägansicht mit Höhen, harte Schatten (`iso_night_hard`, nur Punkt-/Spotlichter) – heute, neu, Differenz | **erledigt 2026-11-25**, Urteil Manuel offen: mittlere Differenz 0,93 von 255, 1,77 % der Pixel über 8; Gesamtbild gleich, aber dünne dunkle Linien an Fassaden (Winkel-Raster), Zaun anders, Kisten-Schatten weichen ab (Ursache offen). Bilder nur im Temp-Ordner. **Runde 2 (2026-11-26):** Wahrheitsbild als dritter Weg gebaut; Karte entfallen, Nachschauen direkt in der gepackten Zeile (2 Zugriffe, 32 KB pro Lampe). Gegen „wahr“: heute 1,74 % der Pixel über 8, **neu 0,66 %** – die auffälligen Unterschiede aus Runde 1 waren überwiegend Fehler des heutigen Verfahrens. Rest bei neu: Winkel-Raster (gezackte Schatten-Enden, Streifen an streifender Fassade). **Runde 3 (2026-11-26):** Schritt 3 (bewegte Lampe) **erledigt** – Flimmern neu/heute 0,64 und 0,99 (Kriterium ≤ 1,5); zwei Nachbarwinkel mischen **verworfen** (bringt nichts, Fehler sitzt am Umriss); Schritt 2 (weiche Schatten) **teilweise** – längs des Strahls weich ohne Zugriffe (gegen weiche Wahrheit: heute 5,47 %, neu 2,54 %), quer zum Strahl noch hart. **Runde 4 und 5 (2026-11-26):** Schritt 2 **erledigt** – Halbschatten auch quer über Randzahlen pro Lauf (Nachlauf-Pass); gegen weiche Wahrheit heute 5,45 %, neu 2,31 %; Zeit weich 52 Lampen: ruhend 549 statt 1 182 µs, alle neu gebaut 1 077 µs; 13 Lampen: ruhend 270 statt 400 µs, alle neu gebaut **450 µs (langsamer als heute)**. Offen: Lichtkeil auf einem Autodach, schwebende Hindernisse weich, Nachlauf-Pass verbilligen |
| 2 | Bild 2: weiche Schatten (`iso_night_plain`, `gate_sprite_shadow`) | **erledigt 2026-11-26** auf `iso_night` mit den Lampengrößen aus `plain.txt` (Zahlen bei Schritt 1 und in ADR 0019, Runden 3–5); `gate_sprite_shadow` nicht geprüft |
| 3 | Bild 3: Draufsicht mit dünnen Wänden und bewegter Lampe (Flimmern, Lecks) | **teilweise 2026-11-26**: bewegte Lampe in der Schrägansicht gemessen (Flimmern neu/heute 0,64–0,99); Draufsicht mit dünnen Wänden **offen** |
| 4 | Zeitmessung neu gegen heute, mit und ohne Cache | **erledigt 2026-11-25** (gleiche Hardware, heute/neu im Wechsel, 3 × 500 Frames ruhig + 2 × 5 × 2 000 Frames unruhig): Lampen ruhen – 13 Lampen 220 statt 380 µs, 52 Lampen 402 statt 1 118 µs; alle Lampen jedes Bild neu gebaut – 360 statt 380 µs (kein Gewinn) bzw. 773 statt 1 118 µs. **Runde 2 ohne Karte (2026-11-26):** ruhend 257 statt 401 µs (13 Lampen) und 417 statt 1 095 µs (52); alle neu gebaut 369 bzw. 690 µs. Wand-Uhr, keine reine GPU-Zeit; Code als Wegwerf-Probe im Worktree `.claude/worktrees/agent-a6a6d2cd427a8f7f6`, nicht committet |

**Bau als Modul** (Entscheidung Manuel 2026-11-26: Versuch sichern, dann das neue Licht als richtiges Modul neben dem heutigen bauen; Zweig `lightrows`, Probe-Commit 936c57b). Weg: das Zeilen-Licht kommt als zweiter Zeichenweg **in den bestehenden Pass-Plan** von `knorke-light2d`; Lichter, die es schon kann, laufen darüber, alle anderen im selben Bild weiter über den Chiaro-Weg. So bleibt jeder Schritt lauffähig und vergleichbar; der Chiaro-Weg schrumpft erst, wenn er leer ist.

| Schritt | Inhalt | Status |
|---|---|---|
| L1 | Stufe `rows` in `knorke-light2d`: Zeilen-Atlas mit Merken pro Lampe, Rand-Pass, ein Lichtshader für Punkt und Spot (hart und weich, Lambert, Glanz und Material wie heute); Übergangs-Einstellung im Pass-Plan; Tests gegen den Chiaro-Weg und das Wahrheitsbild | **erledigt 2026-11-26** (Zweig `lightrows`, vom Hauptagenten gelesen und nachgeprüft). Einstellung `LightSettings::light_rows`, Standard aus (**seit 2026-11-26 an**: Zeilen-Licht ist überall schneller, wo die Lampen ruhen, und näher an „wahr“; die Chiaro-Vergleichstests `gpu_iso_night`, `gpu_wet_street` und das „heute“-Bild der Probe schalten es ausdrücklich aus). **Grenzen** (`rows::frame_suits`, `rows::light_suits`): das Bild braucht kein Medium (**seit 2026-11-27 nicht mehr nötig**, Zweig `lightrows-medium`, siehe L5; Surface-Buffer: **seit 2026-11-26 nicht mehr nötig**, siehe Abschnitt 16; Sprite-Maske: **seit 2026-11-26 erlaubt**, siehe Abschnitt 16; durchscheinende und Muster-Hindernisse: siehe L1b), Hindernis-Oberkanten unter 40,9 Welt-Einheiten (die Zeile speichert Höhen in Hundertsteln mit 12 Bit); die Lampe ist Punkt oder Spot ohne Profil, Maske, Caster-Filter und `inside_obstacle`, Reichweite höchstens 600 Masken-Texel, höchstens 256 Zeilen. Alles andere läuft im selben Bild über den Chiaro-Weg. Stand nach L1 allein: die Demo `iso_night` zeichnete in keiner Variante ein Zeilen-Licht, weil jede Variante zwei Zäune mit Muster hat und ein solches Hindernis das ganze Bild ausschloss (behoben mit L1b; die frühere Angabe „`plain` hat keinen Surface-Buffer“ war falsch – in Knorke ist die Ebene immer da, nur leer). **Bild** (nachgemessen, gleiche Zahlen wie der Versuch): gegen „wahr“ hart 0,66 % / 1,60 % der Pixel über 8 (heute 1,74 % / 4,22 %), weich 2,31 % / 3,03 % (heute 5,44 % / 5,24 %); ohne Hindernisse bitgleich zum Chiaro-Licht in allen drei Material-Modi. **Merken:** Lampen ruhen – 0 Zeilen gebaut; eine Lampe bewegt – 1; ein bewegtes Hindernis – nur die Lampen in Reichweite; Kamera um ganze Masken-Texel – nur Lampen, die über den Maskenrand reichen; das Bild ist dabei bitgleich zu einem Plan, der alles neu baut. **Zeit: am 2026-11-26 nicht verlässlich messbar** (RX 6700 XT, Vulkan, Release, Wand-Uhr; vier Läufe, Mediane schwanken um Faktor 2 bis 4, auch bei Bildern ganz ohne Lampen – also Rechner, nicht Verfahren; Ursache nicht gefunden, CPU und GPU waren sonst frei). Ein ruhiger Abschnitt (hart, 3 × 2 000 Frames, Median µs heute / ruhend / alle neu gebaut / ohne Lampen): 13 Lampen 378 / 318 / 371 / 249, 52 Lampen 959 / 377 / 705 / 241 – in der Größe des Versuchs. Für weiche Schatten gibt es aus dem Modul keine verlässliche Zeit. Shader-Paket mit `cargo xtask shaders --compile` neu gebaut. **Zeit nachgemessen 2026-11-26 (verlässlich):** Ursache der Schwankung war die Messung – sie wartete nach jedem Bild auf die GPU, die dazwischen heruntertaktete. Jetzt in Paketen zu 100 Bildern, gewartet wird am Paketende (`gpu_lightrow_probe.rs`, `BATCH`); drei Läufe liegen 1–2 % auseinander. RX 6700 XT, Vulkan, Release, Wand-Uhr pro Bild, Median in µs – heute / Zeilen ruhend / Zeilen jedes Bild alle neu gebaut (Bild ohne Lampen: 84): hart 13 Lampen 285 / 164 / 279, hart 52 Lampen 884 / 356 / 637; weich 13 Lampen 306 / 203 / **387**, weich 52 Lampen 944 / 503 / **976**; halb durchsichtige Zäune 300 / 163 / 293; Muster-Zäune hart 418 / 166 / 305, weich 447 / 203 / 408; 4 Flächenlichter 135 / 132 / **211**; Linie mit Pfosten 109 / 104 / **162**. Ruhende Lampen sind immer schneller (bis 2,5×), alles jedes Bild neu bauen ist bei weichen Lampen und Flächenlichtern langsamer als heute (bis +60 % bei Flächenlichtern: vier Zeilen pro Lampe). **Offen:** das Distanzfeld wird weiter gebaut, auch wenn nur Zeilen-Lichter Schatten werfen (überflüssige Arbeit, spätestens mit L7 weg); sechs Chiaro-Shaderdateien (rund 1 270 Zeilen) liegen unverändert als Kopie bei den eigenen Shadern, damit die Zeilen-Programme gleich beleuchten (schrumpft mit L7) |
| L1b | Die Demo benutzt das Zeilen-Licht: durchscheinende und Muster-Hindernisse schließen nicht mehr das ganze Bild aus, sondern nur die Schatten werfenden Lampen, deren Reichweite ein solches Hindernis berührt (`rows::RowBlockers`, vorsichtig gerechnet als Quadrat in Masken-Texeln). Das ist genau: eine Zeile beschreibt nur die Maske in Reichweite der Lampe | **erledigt 2026-11-26** (Commit b5f8827, vom Hauptagenten gelesen und nachgeprüft). `iso_night_hard` und `iso_night_plain` zeichnen je 13 von 34 Lichtern als Zeilen-Lichter (mit undurchsichtigen Zäunen 17; die übrigen 17 sind Profile, Masken, Flächenlichter, Sonne – L2 und L3). Gegen „Zeilen aus“: `hard` 0,55 % der Pixel verschieden (Mittel 0,0018 von 255, größte Abweichung 34), `plain` 1,58 % (0,0067, 43). Dabei behoben: flüchtige Lichter teilten sich den Zeilen-Schlüssel mit festen Lichtern gleicher Nummer (`rows::row_key`). **Offen:** kein GPU-Test schiebt einen Zaun in die Reichweite einer Lampe hinein oder heraus (trägt die Regel „Lampe hat ein Bild ausgesetzt → Zeile neu“); **Bilder ganz ohne Surface-Buffer** (flaches Licht) kann das Zeilen-Licht weiter nicht – kommt in der Demo nicht vor; Bau hieße rund 60 Zeilen Lichtkörper aus Chiaros `light_sdf.frag.hlsl` kopieren oder dort einen Einhängepunkt schaffen, dazu ein Ersatz für die Höhen-Textur bei nur unendlich hohen Hindernissen – Entscheidung Manuel, sobald ein Spiel ohne Surface-Buffer das Licht braucht |
| L2 | Profil- und Masken-Atlas, Animation/Flackern (ändert nur Helligkeit, Zeile bleibt gültig) | **teilweise 2026-11-26.** Animation und Flackern: gehen seit L1. **Profile: erledigt** (Commit 5521cc6, vom Hauptagenten gelesen und nachgeprüft): ein Datenzweig im selben Lichtshader, keine neue Variante; ohne Hindernisse bitgleich zum Chiaro-Licht in allen drei Material-Modi. `iso_night_hard` und `_plain`: 15 von 34 Lichtern sind Zeilen-Lichter (gegen „Zeilen aus“ 0,65 % bzw. 2,58 % der Pixel verschieden), mit undurchsichtigen Zäunen 25 von 34. **Zählung der übrigen Lichter der Demo:** 10 erreichen einen Zaun (4 ohne, 6 mit Profil – L4), 8 Flächenlichter (6 Rechteck, 2 Linie – L3), 1 Punktlicht mit Fenster-Maske. **Masken: erledigt 2026-11-26** (Weg (a)). Eigene `_mask`-Programme wie bei Chiaro (Punkt und Spot, Masken-Atlas nach den Muster-Texturen); das Licht wird mit der Maske eingefärbt. Fenster-Masken: der Bau der Zeile beginnt dort, wo der Strahl das Fenster am Boden trifft (aufrechtes Fenster: eine Linie; schräges: die nahe Kante seines Rechtecks), plus Chiaros 1,5 Texel; das Licht schneidet zusätzlich pro Pixel am eigenen Durchgang durch das Fenster ab. Die Lage des Fensters gehört zur Zeile (bewegt sich das Fenster, wird sie neu gebaut). Ohne Hindernisse bitgleich zum Chiaro-Licht in allen drei Material-Modi. Nur das Fenster-Licht der Demo, gegen Chiaro: `hard` größte Abweichung 1 von 255; `plain` (unendlich hohe Wände) 0,02 % der Pixel verschieden, größte Abweichung 46. Ein erster Stand ohne Bau-Beginn am Fenster verlor in `plain` den Schatten der Schild-Pfosten (der Bau hörte an der unendlich hohen Hallenwand auf). Demo `hard`/`plain`: **34 von 34** instanzierte Lichter sind Zeilen-Lichter. Nachgezogen: GPU-Test für ein schräg aus der Wand gekipptes Fenster und für eine Projektor-Maske (Lampe vor der Halle). Er fand beim schrägen Fenster mit unendlich hohen Wänden wieder den fehlenden Pfosten-Schatten: ein Lauf, der innerhalb der Fenster-Strecke am Boden beginnt, darf weder fernere Läufe verdecken noch den Bau beenden (das Licht schneidet ihn pro Pixel ab). Nur das jeweilige Licht gegen Chiaro: schräg `hard` 0,03 %, `plain` 0,15 % → 0,05 % der Pixel verschieden (Reste: einzelne Pixel an gezackten Schattenkanten); Projektor `hard` bitgleich, `plain` 0,09 % (ein weicher statt harter Schattenrand an der Hallenecke – die bekannte Abweichung weicher Schatten, kein Masken-Fehler). **Offen:** das Wahrheitsbild der Probe kennt keine Masken |
| L3 | Linien- und Flächenlichter (mehrere Stützpunkte oder eigene Zeilen), Sonne/Mond | **teilweise 2026-11-26.** **Linien- und Flächenlichter: erledigt** (Commit 13a669e, vom Hauptagenten gelesen und nachgeprüft). Vier harte Zeilen an den Mitten von vier gleichen Stücken des Leuchtkörpers, die Sicht ist ihr Mittel (Mittelpunktregel); Helligkeit unverändert Chiaros. Chiaro zieht nur einen Strahl zum nächsten Punkt des Leuchtkörpers. Ohne Hindernisse bitgleich in allen drei Material-Modi. Gegen „wahr“ (Mittel über Punkte des Leuchtkörpers) heute 0,11 %, **neu 0,01 %** der Pixel über 8 – in der Demo werfen Flächenlichter aber kaum Schatten, der Nachweis ist schwach. Demo: `hard`/`plain` 21 von 34 Zeilen-Lichter, mit undurchsichtigen Zäunen **33 von 34** (alle außer dem Fenster-Licht). Je schattenwerfendem Flächenlicht 4 Zeilen und 8 Zugriffe pro Pixel. **Schwächen:** Halbschatten in 4 Stufen statt glatt; Quellradius des Flächenlichts wird nicht beachtet; Punkte gleich gewichtet; Chiaros 1,5-Texel-Abstand vor dem Ziel fehlt. Test mit Pfosten direkt vor einer Linie (Commit ff2156e): gegen „wahr“ heute 0,19 %, **neu 0,01 %**; heute ein schmaler, harter, seitlich versetzter Streifen, neu derselbe weiche Keil wie „wahr“. **Offen:** zweiseitige und liegende Rechtecke nur im Unit-Test. **Harte Sonne: erledigt 2026-11-26 (Zweig `lightrows-sun`, siehe Abschnitt 16).** Eingebaut wie unten geplant (gedrehtes Raster 1 / 0,25, 2 Intervalle + Hülle, Neubau nur bei Änderung). Mond gegen „wahr“: A heute 0,32 %, **neu 0,18 %**; B heute 0,29 %, **neu 0,12 %** der Pixel über 8. Ruhend 100 µs statt 180 µs pro Bild; scrollende Kamera 162–166 µs statt 238 µs (nur die Bänder hinter den Randstreifen werden neu gebaut, siehe Abschnitt 16); jedes Bild ganz neu gebaut 1077 µs (6× langsamer als heute – nur noch bei sich ständig drehender Sonne). Speicher 22 MB pro Karte (Maske 480×340). Das Distanzfeld wird weiter gebaut (L7 offen). **Weiche Sonne: erledigt 2026-11-26 (Zweig `lightrows-sunsoft`, siehe Abschnitt 16)**: gedrehtes Raster mit bilinearer Mischung und Aufweitung der Intervall-Enden; A heute 0,70 %, **neu 0,21 %**, B heute 0,66 %, **neu 0,19 %** der Pixel über 8; scrollend 175–184 µs statt 264 µs. Schwäche: die Strahlen der Scheibe streuen auch quer zum Licht, das hält die Karte nicht (seitliche Halbschatten-Ränder bleiben härter). **Demo:** `iso_night_hard` hat ein Richtungslicht (den Mond, hart, schattenwerfend); es bleibt beim Chiaro-Weg, weil die beiden Muster-Zäune im Bild liegen (bis heute schloss schon jedes durchscheinende oder Muster-Hindernis irgendwo in der Szene die Sonne aus; jetzt nur noch eins in der Maske – ein Sonnenstrahl kreuzt die ganze Maske, eine engere Grenze bringt kaum etwas). Mit undurchsichtigen Zäunen nimmt der Mond die Karte. `iso_night_plain`: der Mond wirft keinen Schatten, braucht also keine Karte. **Sonne hinter durchscheinenden und Muster-Hindernissen: erledigt 2026-11-26 (Zweig `lightrows-sunpattern`, siehe Abschnitt 16).** Gebaut als durchscheinende Läufe pro Karten-Texel (Durchlass pro Lauf wie die Zeilen ihr Durchlass-Byte, Muster an der Strahlhöhe im Licht), kein Ausweich-Merker. Demo `iso_night_hard`: der Mond nimmt jetzt die Karte (1 Sonne aus der Karte). Jeder Fall näher an „wahr“ als heute (Zaun hart 0,17 % statt 0,33 %, weich 0,42 % statt 0,89 % der Pixel über 8), scrollend schneller als heute. Kachel-Raster mit Muster: erledigt 2026-11-27 (Zweig `lightrows-tilegrid`, Abschnitt 16). **Offen:** ein dritter Lauf wird mit dem zweiten verbunden; weiche Enden eines Laufs nehmen beide den Abstand seines Beginns; zwei aneinanderstoßende Läufe verschiedener Intervalle zählen als zwei Kreuzungen (Chiaro: eine). Alte Schätzung zum Nachlesen: durchscheinend – die Karte hält nur undurchsichtige Höhenintervalle; ein Durchlass-Profil über der Höhe bräuchte pro Texel mehrere Höhenstufen mit Durchlass (weitere Ziele, Bau ~2× teurer); billiger wäre ein Merker pro Karten-Texel „Strahl trifft durchscheinendes Hindernis“, mit dem nur diese Pixel Chiaros Marsch nehmen (ein Kanal mehr, eine Verzweigung im Licht). Muster – wie bei den Zeilen ein Kreuzungs-Ziel: pro Karten-Texel die ersten Kreuzungen mit Muster-Hindernissen (q, Hindernis), das Licht liest das Muster in der Höhe seines Strahls; ein zweiter Bau-Durchgang mit dem Muster-Pass (`lightrow_pattern.frag` als Vorlage), ein zusätzliches Ziel, rund 1–2 Tage; Genauigkeit des Welt-Rasters fällt bei sehr großen Welt-Koordinaten (ab ~10⁶ Masken-Texeln vom Ursprung, f32); der Neubau-Anteil eines Schwenks wächst mit der Reichweite (höchste endliche Oberkante / Sonnensteigung) – ein einzelnes sehr hohes Hindernis macht ihn für die ganze Karte groß. **Sonne/Mond: entschieden 2026-11-26 (Manuel) – bleibt vorerst beim Chiaro-Weg;** eine andere Speicherform (verschmolzene Höhenintervalle pro Strahl) wird später versucht, wenn der Rest steht. Versuch „parallele Zeilen“ (Zweig `lightrows-sun-wip`, Commit 1ff9921, nicht übernommen): ohne Hindernisse bitgleich, aber gegen „wahr“ (Mond aus `iso_night_hard`) **schlechter als heute** – heute 0,32 %, Zeilen 0,84 % der Pixel über 8 (falscher dunkler Streifen an einer Fassade, weil der vierte Platz die Hülle weiterer Läufe nimmt); ohne Hülle 0,52 %, dann liegen Kanten um einen Texel daneben (Strahlen fest 1 Texel auseinander). Chiaros Sonne ist hier schon nah an „wahr“. **Versuch „Sonnen-Höhenkarte“ 2026-11-26 (Zweig `lightrows-sunmap`, Commit 195c957, Testwerkzeug, nicht übernommen): trägt.** Pro Karten-Texel die verschmolzenen beschatteten Höhenintervalle (2 reichen in der Demo), gebaut durch einen Marsch zur Sonne über die endlichen Hindernisse; das Licht liest die Karte am Strahlanfang und verschiebt z exakt entlang der Sonnenrichtung. Mond allein, gegen „wahr“ (Mittel / % Pixel über 8), Kamera A und um 0,37 px verschobene Kamera B: heute hart 0,076 / 0,32 und 0,064 / 0,29; Karte im Maskenraster 0,115 / 0,49 (schlechter); 2× Raster 0,020 / 0,09 aber 0,050 / 0,23 (hängt an der Kamera); **zur Sonne gedrehtes Raster (1 längs / 0,25 quer) 0,035 / 0,16 und 0,027 / 0,11**. Weich: heute 0,147 / 0,70 und 0,149 / 0,66; **2× Raster bilinear mit Aufweitung der Intervall-Enden 0,073 / 0,25 und 0,068 / 0,13**. Zeit (RX 6700 XT, Vulkan, Licht 640×360, Maske 480×340): Chiaros Sonnen-Pass hart 94,5 µs, weich 114,5 µs; Karten-Licht 11–19 µs; Karten-Bau 257 µs (Maskenraster) bis 968 µs (gedreht 1/0,25) – nur neu bauen, wenn Sonne oder endliche Hindernisse sich ändern (lohnt ab ~12 Bildern ohne Neubau). Grenzen: eine Szene, eine Sonnenrichtung; weich nur senkrecht aufgeweitet; halb durchsichtige und Muster-Hindernisse nicht enthalten. **Geplant:** einbauen (hart: gedrehtes Raster; weich: 2× bilinear), Neubau nur bei Änderung. Möglichkeiten (alt): andere Speicherform (verschmolzene Höhenintervalle pro Strahl), mehr Abschnitte, zwei Nachbarstrahlen mischen, oder Sonne bleibt beim Chiaro-Weg (dann bleibt das Distanzfeld für die Sonne, L7 schrumpft) |
| L4 | Durchscheinende Hindernisse und Muster (Zaun, Gitter) in der Zeile | **teilweise 2026-11-26.** **Durchscheinende Hindernisse (Deckkraft unter 1): erledigt** (Commit a0ea520, vom Hauptagenten gelesen und nachgeprüft). Ein Lauf trägt seinen Durchlass (1 − Deckkraft des ersten Texels, wie Chiaro beim Eintritt); das Licht wird mit dem Produkt der gekreuzten Läufe multipliziert. Kein dritter Zugriff: das Vorzeichen des ersten Werts markiert den Winkel, der vierte Lauf-Platz hält dann drei Durchlass-Bytes (ein solcher Winkel hat nur drei Läufe). Weiche Lampen: Durchlass hart am Strahl zur Lampenmitte, dann das Kleinere aus Durchlass und weicher Sicht (Chiaros dünner Saum außerhalb entfällt). Bild: `iso_night_hard` mit Zäunen als 50-%-Wänden – 25 von 34 Zeilen-Lichter; gegen „wahr“ heute 2,56 % der Pixel über 8, **neu 1,25 %**; im Schatten der Wände lässt Chiaro mehr durch (0,62 statt 0,55 des Wegs von dicht zu frei – dünne Wand, 1-Texel-Schritte). Offen: kein GPU-Test für weiche Lampe mit durchscheinender Wand und für Kachel-Zellen mit Deckkraft. **Muster-Hindernisse: erledigt 2026-11-26 (L4b, Commits 32575df und 6686748, vom Hauptagenten gelesen und nachgeprüft).** Ein eigener Muster-Pass schreibt pro Winkel bis zu 2 Kreuzungen (Hindernis-Id, Lage u am Umriss); harte Lampen suchen den Eintritt auf dem eigenen Strahl, wenn er oben aus dem Hindernis steigt (sonst fehlte z. B. der Schatten eines Gitterturms auf einer Fassade). Demo `hard` und `plain`: **33 von 34** Zeilen-Lichter (nur der Mond nicht). Gegen „wahr“: Zaun hart heute 2,10 %, **neu 1,53 %**; Zaun weich heute 8,49 %, **neu 5,70 %** der Pixel über 8. Etwa 390 neue Zeilen ohne Kopien und Wahrheits-Shader. Fehlende senkrechte Streifen am rechten Fassadenrand behoben (Commit 9f13215: bei vollen Plätzen werden zwei Stücke eines geteilten Hindernisses zu einem Lauf verbunden statt die Hülle zu nehmen; Fassade 1,95 → 1,32 gegen „wahr“, „hart halb“ 1,25 % → 0,88 %). **Ein verbundener Muster-Lauf liest das Muster nur einmal – geprüft 2026-11-26, bleibt so (verworfen):** Versuch „die Zeile zählt die verbundenen Stücke, das Licht liest das Muster so oft“. Mit den Zäunen der Demo (Muster nur ganz dicht oder ganz offen) ändert er kein Pixel. Mit halb durchsichtigen Muster-Zäunen (Deckkraft 0,5, nur als Versuch): Fassade 1,006 → 0,828 gegen „wahr“, aber das ganze Bild hart 0,314 → 0,320 und weich 0,661 / 2,96 % → 0,695 / 3,12 % – das zweite Stück wird oft gar nicht vom Strahl dieses Pixels gekreuzt. Kein Gewinn, also kein Code. **Weiche Lampen lesen das Muster an der gespeicherten Kreuzung** – bleibt so: der schärfere Eintritt pro Pixel war für weiche Lampen schlechter gemessen (Probe „zaun weich“). **Offen:** Polygon-Fall im Lichtpass ungetestet (Kachel-Raster mit Muster: erledigt 2026-11-27, Zweig `lightrows-tilegrid`, Abschnitt 16, `RowBlockers` gelöscht); Lichtshader von 63 auf 84 KB SPIR-V. Entscheidung war: Mittelweg – die Zeile merkt sich pro Winkel, welches Muster-Hindernis wo getroffen wird (Eintrittsstelle entlang des Hindernisses); das Licht liest pro Pixel mit der Höhe, die es ohnehin rechnet, genau einen Musterpunkt nach. Genau wie heute (höhenabhängig), Mehrkosten nur auf Winkeln mit Muster. Die folgenden Möglichkeiten (1)–(4) sind damit erledigt. Die Zäune der Demo sind Muster (Deckkraft 1), deshalb bleibt die Demo bei 15 von 34. Chiaro liest das Muster am Eintrittspunkt des Strahls, abhängig von der Strahlhöhe, über die Welt-Geometrie des Hindernisses (`pattern.hlsli`, rund 660 Zeilen, Id-Textur, Atlas, 3 Puffer). Möglichkeiten: (1) dasselbe im Lichtshader der Zeilen – genau, aber groß; (2) Muster in die Zeile backen (Durchlass pro Winkel bei fester Höhe) – klein, bei schrägem Strahl durch ein gestrecktes Muster sichtbar falsch; (3) Muster bleiben beim Chiaro-Weg (heutiger Stand); (4) wie im Entwurf (ADR 0019 §3): der Host malt Zaun und Gitter als Durchlass in die Bodenkarte, Muster-Hindernisse als eigene Sorte entfallen – Feinheit = Auflösung der Karte |
| L5 | Nebel/Medium aus der Zeile statt Polar-Atlas und Marsch | **teilweise 2026-11-27.** **Licht am Boden durch das Medium: gebaut und angenommen 2026-11-27** (Zweig `lightrows-medium`, Abschnitt 16). Abgewogen: ADR 0019 (§3, „Nebel/Rauch, Lichtstreifen“) meint den Marsch entlang des Sehstrahls in der Medium-Stufe (Lichtstreifen, Polar-Atlas, Sonnen-Höhen-Atlas), nicht die Dämpfung des Bodenlichts; die Zeile hält Hindernis-Läufe, keine Dichte – eine Dämpfung „aus der Zeile“ bräuchte eine zweite Zeile mit dem Integral der Dichte pro Winkel und Entfernung, die sich mit jedem bewegten Nebel ändert. Deshalb der einfache Weg (Standard, schlank): `MediumTransmit` entlang des Lichtstrahls wie Chiaros `*_medium`-Programme, 12 Define-Hüllen, keine eigene Datenstruktur. Näher an „wahr“ als heute in allen Fällen (Lampen hart 0,51 % statt 1,43 %, weich 1,81 % statt 3,67 % der Pixel über 8; Mond ebenso), ruhend und Mond scrollend schneller; **Lampen scrollend langsamer als heute** (hart 498 statt 484 µs, weich 581 statt 496 µs) – das Kriterium „scrollend nicht langsamer“ ist nicht erfüllt; der Abstand ist der schon ohne Nebel gemessene Neubau beim Scrollen (Abschnitt 16, `lightrows-tilegrid`). Angenommen, weil der Abstand nicht vom Medium kommt; mit L6 scrollen die Lampen im Nebel schneller als heute (hart 334 statt 481, weich 374 statt 498 µs). **Offen:** Lichtstreifen der Medium-Stufe aus der Zeile statt Polar-Atlas (die eigentliche Idee des ADR) |
| L6 | Feste und bewegte Zeile getrennt; Grundlicht plus Korrektur (Vorschlag Manuel) | **teilweise 2026-11-27** (Zweig `lightrows-scroll`, Abschnitt 16). Ziel war: Lampen scrollend nicht langsamer als heute. Gemessen und gewählt: Zeile behalten, solange in den Rand-Streifen des Schwenks kein Hindernis in ihrer Reichweite liegt (statt nur die Winkel in den neuen Streifen neu zu bauen oder eine feste und eine bewegte Zeile zu trennen: in der Probe liegt nie eines dort, es wird gar nichts gebaut), dazu kein Distanzfeld ohne Chiaro-Schatten. Danach scrollend in allen Probe-Fällen schneller als heute (18 bis 70 %). **Offen:** feste/bewegte Zeile für Lampen, an denen bewegliche Hindernisse vorbeilaufen (eine Figur im Lichtkreis baut die Zeile heute jedes Bild neu); Grundlicht plus Korrektur; Neubau nur der betroffenen Winkel, wenn ein Hindernis durch den Rand-Streifen in die Reichweite kommt |
| L7 | Chiaro-Weg, Distanzfeld und Permutationen entfernen, sobald kein Licht sie mehr braucht; neue Referenzbilder | **Teil 2 erledigt 2026-11-27** (Zweig `lightrows-delete`, Abschnitt 16): alles von der Liste unten gelöscht, neue Referenzbilder unter `tests/golden/rows/` (Probe ohne „heute“). **Teil 3 erledigt 2026-11-27** (Abschnitt 16): Chiaros Programme aus dem Shader-Paket genommen (103 Programme), die übrigen 48 liegen im Repo, das Paket baut ohne `D:\Entwicklung\chiaro`. **Offen:** Polar- und Sonnen-Höhen-Atlas samt Umrechnung, bis `medium_light`/`medium_directional` Zeile und Karte direkt lesen; 512 Winkel für kleine Lampen. **Teil 1b erledigt 2026-11-27** (Zweig `lightrows-gaps2`, Abschnitt 16): die drei Entwurfsfragen nach den Empfehlungen – (a) 2048 Winkel bis 2047 Texel, darüber und über 16 384 Plätzen ohne Schatten mit Warnung; (b) jede Sonne eine Karte, gröberes Raster nur für große Masken (Sonnen ab der dritten gröber: gemessen schlechter als heute, verworfen); (c) Umrechnungs-Passes für Lichtstreifen und Sonnen-Höhen. Mit Zeilen baut kein Bild das Feld, kein Licht nimmt ein Chiaro-Schattenprogramm (Test über die Demo-Varianten und die Probe). **Teil 2 kann löschen (geplant, erledigt 2026-11-27):** Chiaros Schatten-Lichtprogramme mit Feld-Marsch (Punkt, Spot, Linie, Rechteck, Richtung, je `_mask`/`_medium`) und ihren Zweig im Licht-Pass, sobald `light_rows` fest an ist; das Distanzfeld (JFA-Programme `jfa_seed`/`jfa_step`/`jfa_distance` in Voll- und Rechteck-Fassung, Feld-Ziele, Klassen-Felder, `set_field_needed`, Teil-Updates); Chiaros `medium_polar.frag` und `medium_sun_height.frag` mit `MediumProgram::Polar`/`SunHeight`, `PolarRowParams`, `SunHeightParams` und dem Feld-Zweig in `prepare_shafts`/`prepare_suns`; die Einstellung `light_rows` und `frame_suits`; dann neue Referenzbilder für `gpu_iso_night`, `gpu_wet_street` und das „heute“ der Probe. Polar- und Sonnen-Höhen-Atlas samt Umrechnung erst, wenn `medium_light`/`medium_directional` Zeile und Karte direkt lesen (offen). 512 Winkel für kleine Lampen: nicht umgesetzt (ADR 0019). **Teil 1 erledigt 2026-11-27** (Zweig `lightrows-gaps`, Abschnitt 16): Ziel war, dass jedes Licht und jedes Bild den Zeilen-Weg nimmt, damit Teil 2 Chiaros Schatten-Weg löschen kann. **In die Zeilen übernommen:** Schattenfilter (E53) für Lampen und Sonnen (Zeile und Karte aus Maske und Höhen der Klasse; ein Bau- und ein Licht-Draw pro Klasse; Klassen-Felder nur noch, wenn Chiaros Programme lesen); `inside_obstacle` (E147, war schon richtig: jede Zeile überspringt das Hindernis, in dem die Lampenhöhe liegt; Chiaro nur bis 64 Texel); Oberkanten ab 40,9 (Höhen-Einheit pro Bild verdoppelt, bis die höchste Oberkante unter 4095 Einheiten liegt, `rows::height_code`; ein Wechsel baut alle Zeilen neu); mehr als 256 Zeilen (`MAX_ROWS` 16 384, der Atlas wächst nach Bedarf, Platzsuche mit Merker statt Suche von vorn); Sonne von unter dem Horizont und waagrecht (Enden des Intervalls nach Vorzeichen der Steigung, Reichweite die ganze Maske); Sonne senkrecht von oben und Richtungslichter ohne Schatten (`rows::sun_plain`: das Sonnen-Programm zeichnet sie ohne Karte, Sicht 1 wie Chiaro). **Weggelassen:** Ringlichter der Schockwellen – eigener Pass ohne Schatten und ohne Feld, Teil 2 berührt ihn nicht. **Offen, Entwurfsfragen (Bericht an Manuel):** (a) Reichweite über `MAX_REACH` 600 Masken-Texel – A: Winkelzahl nach Reichweite (512/1024/2048, ADR 0019) mit 2048 Winkeln und halben Texel-Abständen für 600–2047 (empfohlen); B: über 600 ohne Schatten mit Warnung. (b) mehr als `MAX_SUNS` 2 Sonnen und Karten über `MAX_SUN_MAP` 8192 – eine Karte bei Maske 1024² braucht rund 134 MB; A: `MAX_SUNS` anheben (Karten werden erst bei Bedarf angelegt); B: gröberes Quer-Raster (`SUN_ACROSS`) für große Masken und überzählige Sonnen (empfohlen). (c) Lichtstreifen und Sonnen-Höhen der Medium-Stufe lesen das Distanzfeld (`medium_polar`, `medium_sun_height`) – zu groß für Teil 1, das Feld bleibt nur für sie; A: Knorke-Fassungen, die die Maske per DDA ablaufen; B: kleine Umrechnungs-Passes aus dem Zeilen-Atlas bzw. der Sonnen-Karte in die Polar- und Sonnen-Höhen-Texturen (empfohlen). Über 16 384 Zeilen bleibt ein Licht bei Chiaro (Teil 2: ohne Schatten mit Warnung). **Stand vorher (2026-11-27, nach `lightrows-medium`):** **Was noch an den Chiaro-Weg fällt (Stand 2026-11-27, nach `lightrows-medium`; kein Hindernis-Typ und kein Medium mehr):** ganzes Bild (`rows::frame_suits`): `light_rows` aus; ein endlicher Hindernis-Deckel ab `MAX_TOP` 40,9 (Zeile speichert Höhen in Hundertsteln, 12 Bit); Lampe (`rows::light_suits`, schattenwerfend): Schattenfilter außer `All` (E53, die Zeile kennt nur die Maske aller Werfer); `inside_obstacle` (E147); Reichweite über `MAX_REACH` 600 Masken-Texel; mehr als `MAX_ROWS` 256 Zeilen im Bild (Linien- und Flächenlichter je 4); Sonne (`rows::sun_suits`, `prepare_suns`): Schattenfilter außer `All`; senkrecht von oben (x = y = 0); von unter dem Horizont (Steigung ≤ 0); mehr als `MAX_SUNS` 2; Karte länger als `MAX_SUN_MAP` 8192 Texel (sehr große Masken); Richtungslichter ohne Schatten laufen weiter über Chiaros Richtungsprogramm (brauchen keine Karte, aber ein eigenes Programm). Unabhängig davon: Ringlichter der Schockwellen sind ein eigener Pass ohne Schatten. |

**Offene Punkte / Ideen aus dem Entwurf:**
- **Schwebende Hindernisse** (Unterkante > 0) – **vorbereitet**: `iso_night_hard` benutzt vier (Dach über einer Lampe, drei Balken); der Entwurf führt sie als obere Grenze des Gefälle-Intervalls. **Schräge Dächer/Rampen** – **offen**: gehen nur als Stufen.
- **Schatten in die Lichtkarte malen statt marschieren** (ein Instanced Draw über alle Paare Lampe × Hindernis, Max-Blend; exakte Kanten, kein Marsch; braucht Hindernisse als Formen und eine Karte Winkel × Entfernung) – **offen**: Idee, in Schritt −1 mitzählen.
- **Feste und bewegte Zeile pro Lampe** (Manuel, 2026-11-25): ruhende Hindernisse einmal merken, nur bei wandernder Lampe oder zerstörter Umgebung neu bauen; bewegliche Hindernisse in einer zweiten, kleinen Zeile pro Frame – **vorbereitet** (in ADR 0019 aufgenommen), im Versuch zu belegen.
- **Tricks für wenig Arbeit pro Pixel** (Nachfrage Manuel, 2026-11-25; Einzelheiten ADR 0019 Abschnitt 2a): (1) **Schattenhöhe statt Liste** – pro Lampe eine Karte Winkel × Entfernung mit der Höhe, unter der Schatten ist; ein Textur-Zugriff und ein Vergleich pro Pixel und Lampe; Formel mit 29 185 Zufallsfällen nachgerechnet (**belegt**), Nutzen gegen Speicher und Umrechnung **offen** (Schritt −1). (2) **Enge Formen** statt Rechtecke – **offen**. (3) **Fertiges Licht ruhender Lampen merken**, nur geänderte Bereiche neu rechnen (exakt, kein Mischen über Frames; hilft auch dem heutigen Verfahren) – **offen**, zweite Stufe. (4) **Lampen-Schleife pro Kachel** statt Rechteck pro Lampe – **offen**, zweite Stufe. (5) Halbe Auflösung – **vertagt**. Zu (3) die genauere Fassung von Manuel (2026-11-25): **Grundlicht** (fertiges Licht ruhender Lampen, bleibt stehen) plus **Bildlicht** pro Frame (Kopie, dazu wandernde Lampen, abgezogene Schattenkeile beweglicher Hindernisse, frisch beleuchtete Figuren); schlechtester Fall kostet wie ohne den Trick – **offen**, zweite Stufe nach dem Kern. (6) **Wandernde Lampen billig halten**: ohne Schatten keine Karte, Spot baut nur seinen Kegel, gröbere Karte – **offen**, im GPU-Versuch messen. Praxis großer Engines (Unreal statisch/ortsfest/beweglich, Unity Subtractive/Shadowmask, gemerkte Schatten-Karten) nur aus Vorwissen, **nicht nachgeschlagen**.
- **Eine Lampensorte mit Parametern** statt fünf Lichtarten mit eigenen Wegen (Manuel, 2026-11-25): Kegel, Ausdehnung des Leuchtkörpers, Lampengröße, „unendlich weit weg“ – **vorbereitet** (ADR 0019 Abschnitt 2); im GPU-Versuch laufen Punkt und Spot schon über einen Weg.
- **Mehr als vier Läufe pro Richtung** – **gezählt 2026-11-26, kein Handlungsbedarf**: Versuch in der Probe (vorübergehende Markierung im Bau, nicht committet; Richtungen ohne Schatten gezeichnet, Anteil der Pixel über 8 verschieden). Mehr als vier Läufe *vor* dem Verbinden (`Squeeze`): hart 0,01 %, hart Last 0,14 %, weich 0,01 % / 0,15 %, halb durchsichtige Zäune 0,79 % der Pixel. Hülle trotz Verbinden: in keinem Fall ein Pixel (alle neun Probe-Fälle, auch Zäune und Flächenlichter). Ein dritter Zeilen-Texel lohnt sich für diese Szenen nicht; in dichteren Szenen neu zählen.
- **Kaskaden als abschaltbare Zusatzstufe** für leuchtende Flächen und indirektes Licht (neue Funktion, nicht Teil des Solls) – **offen**, erst nach dem Kern.
- **Lichtpuffer unbewegter Lampen in Bodenkoordinaten zwischenspeichern** (Kosten pro Frame nur für bewegte Lampen) – **offen**; Wandflächen bräuchten eine zweite Höhenschicht.
- **Höhen in Kaskaden** („obere Hülle des Höhenprofils pro Strahlabschnitt“) – **verworfen** für jetzt: Forschungsprojekt ohne Vorarbeit.
- **Aufräumen, weitere Kandidaten** (2026-11-27) – **erledigt** 2026-11-27 (zweiter Durchgang, siehe 16), bis auf `LightDesc::inside_obstacle` (Snapshot-Format). Ursprüngliche Liste: Masken-Pfad ohne Höhen (`height_mode == false`) in `occlusion`/`occluder_raster` (Tests und Shader hängen dran); `GpuLight::shadow[0]` (Quellradius aus dem alten Strahlengang, von den Zeilen-Shadern vermutlich nicht gelesen, erst prüfen); doppelte Kleinigkeiten (`DEG_TO_RAD` 5×, `smoothstep` 2×, projiziertes Hüllrechteck 3×); doppelte Test-Helfer (`with_vulkan` ~5×, Bildvergleich-Helfer der Integrationstests); nur von Tests genutzte API (`StoredZone::hull`, `LightList::auto_height_lights` u. a.). Bewusst behalten: CPU-Modelle der Shader-Mathematik als Test-Orakel, allgemeine `knorke-gpu`-Hüllen, Typ-Kopien zwischen `knorke-light2d` und `knorke-post` (Abhängigkeitsregel, ADR 0014).

### 17.14 3D-Engine aus pixelraum (Entwurf 2026-11-27)

Ziel (Manuel, 2026-11-27): pixelraum (`knorke-render` + `knorke-lighting`) wird Schritt für Schritt eine vollwertige 3D-Engine für verschiedene Spiele: zuerst Wizardry 7 (Gitter-Dungeon, Pixel-Look), später ein Ego-Shooter. Beschlossen in **ADR 0021** (angenommen 2026-11-27): eine gemeinsame Szene, zwei Renderwege (CPU für Pixel-Look und als Referenz, GPU als Hauptweg), Schatten über Schattenkarten, `knorke-post` für beide, Einsatzprofile in `knorke-integration`.

| Schritt | Inhalt | Status |
|---|---|---|
| 3D-1 | Szenenmodell aus `knorke-render` herauslösen, Meshes gleichberechtigt | läuft (Entwurf) |
| 3D-2 | GPU-Raster derselben Szene, danach `knorke-post`, Vergleich gegen CPU-Goldens | offen |
| 3D-3 | GPU-Licht mit Schattenkarten, Materialien, Nebel, Pixel-Look auf der GPU | offen |
| 3D-4 | Wizardry 7 auf Knorke (C-ABI `knorke-abi`, 17.1) | offen |
| 3D-5 | Shooter-Grundlagen (Kollision 17.8, Sichtbarkeit, Animation 17.4, Maus), Demo | offen |
| – | CPU-Bild an `knorke-post` geben | verworfen 2026-11-27 – Post braucht ohnehin die GPU, also gleich alles auf der GPU (ADR 0021) |
| – | Lichtzeilen (ADR 0019) als Sonderweg für reine Gitter-Welten | offen – nur nach Messung |

### 17.15 Einschätzung und Arbeitsliste (2026-11-28)

Gesamtblick auf den Stand mit einer geordneten Liste der nächsten Schritte: `docs/einschaetzung-2026-11-28.md`. Status: **geplant** – entschieden (Manuel, 2026-11-28): Reihenfolge C1, dann A1–A3, dann B1 (Kollision), dann das erste Spiel (A4), danach Wizardry 7. **Erstes Spiel:** kleines Schleich- und Schießspiel im Hof der Wachmann-Szene – Figur steuern, Wachen sehen im Licht besser, Ziel erreichen oder erwischt werden, Neustart; Schießen schaltet eine Wache aus, ist aber laut und lockt andere an. Zweck: die typischen Grundmechaniken (Bewegen, Sehen, Hören, Treffer, Sieg/Niederlage) einmal ganz durchspielen. Schießen braucht Kollision, deshalb B1 vor A4. Neu gegenüber dem bisherigen Backlog (standen nirgends): **Schrift und einfache Oberfläche** (offen), **Spielstände als Dateiformat mit Version und Szenen-Kennung** (offen), **Dauer der Prüfkette messen** (offen), **Plan-Abschnitt 16 kürzen, Berichte auslagern** (erledigt 2026-11-28: Berichte in `docs/stand.md`, Abschnitt 16 hat eine Zeile je Baustein).
