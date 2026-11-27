# ADR 0022: Fenster, Echtzeit-Schleife und Eingabe auf SDL3 – der Host als oberste, abnehmbare Schicht

Status: Vorschlag · Datum: 2026-11-28 · Bezug: ADR 0001, 0002, 0003, 0008, 0009, 0010, 0014, 0020, 0021, Plan 17.1, Einschätzung 2026-11-28 Punkt A1

## Kontext

Heute läuft keine Knorke-Szene in einem Fenster. Die Wachmann-Szene (`knorke-runtime/examples/guard_scene_view.rs`) zeichnet offscreen über `knorke_integration::stealth_view`, liest die Pixel zurück und schreibt PNGs. Gerät und Shader-Paket holt sie aus `knorke_gpu::test_support`. Das ist Testcode: er sperrt das Gerät prozessweit und überspringt Tests ohne GPU still. Die Eingabe ist ein festes `Script`, das vor bestimmten Ticks `Command::Goto(cell)` abgibt.

Was schon da ist und bleibt:
- `knorke-runtime`: `FixedStep` macht aus gemessener Zeit ganze Ticks plus `alpha`, `Runtime::submit` nimmt Commands für den nächsten Tick an, `Runtime::frame(elapsed_nanos)` treibt die Ticks über einen `TickDriver` (`NoRecord` oder `Recorder`). Die Crate hat keine Uhr und kein `std` (ADR 0003, 0009).
- `knorke-replay`: `Recorder`, `ReplayLog`, `verify`, `seek` (ADR 0010).
- `knorke-gpu`: `init_video`, `Device::create`, Shader-Paket `kpak` mit `expect_source_checksum`; SDL3 über `sdl3-sys` (ADR 0002, 0014).
- `knorke-audio-sdl`: das Audiogerät von SDL3 als Host-Seite von `knorke-audio` (ADR 0020).

Manuels harte Vorgabe: Die Engine bleibt aufteilbar. Ein fremdes Projekt (z. B. Wizardry) muss einzelne Teile übernehmen können, etwa nur die KI, nur Audio oder nur das Licht, ohne Host, Fenster oder SDL mitzunehmen. Der Host ist eine optionale oberste Schicht.

## Entscheidung

### 1. Zwei neue Crates, beide ganz oben und abnehmbar

- **`knorke-input`** (`no_std`, nur `knorke-types`): geräteneutrale Eingabetypen und die Zuordnung Eingabe→Aktion. Darin `InputEvent` (Taste als physischer Scancode, Maustaste, Mausposition, Gamepad-Knopf und -Achse; jeweils gedrückt/losgelassen/Wert), `InputState` (was gerade gehalten wird, Flanken seit dem letzten Tick) und `Bindings<A>` (Tabelle Eingabe→Aktion `A`, die das Spiel definiert). Kein SDL, keine Engine-Fachcrate. Damit kann ein fremdes Projekt die Zuordnung mit eigener Fensterbibliothek nutzen oder ganz weglassen. Zuordnungen headless testen geht auch.
- **`knorke-host`** (`std`, Feature `sdl`): Fenster, Ereignisschleife, Uhr, Swapchain und optional das Audiogerät. Er übersetzt SDL-Ereignisse in `knorke_input::InputEvent`. Den Simulationstyp, die Commands und die Fachmodule kennt er nicht.

Keine Fachcrate, kein Profil in `knorke-integration`, nicht `knorke-runtime` und nicht `knorke-scene` hängen von `knorke-host` oder `knorke-input` ab. Abhängen dürfen nur Spiele, Beispiele und Tests. `knorke-host` hängt ausschließlich ab von `knorke-types`, `knorke-input`, `knorke-gpu` (Gerät und Swapchain) und optional, hinter dem Feature `audio`, von `knorke-audio-sdl`. Er hängt **nicht** an `knorke-runtime`: die Zeit liefert er als Zahl, die das Spiel an `Runtime::frame` weitergibt.

### 2. Wem gehört die Schleife: dem Spiel

Das Spiel besitzt `main` und die Schleife. `knorke-host` ist eine Werkzeugkiste, kein Rahmenwerk: kein `run(app)`, kein `App`-Trait mit Rückrufen, keine eigenen Threads außer dem Audio-Callback von SDL (ADR 0020). Eine Runde sieht so aus:

```text
let frame = host.begin_frame()?;          // SDL-Ereignisse leeren, Zeit messen
for ev in frame.input() { input.apply(ev) }   // knorke-input::InputState
if frame.quit() { break }
for action in bindings.actions(&input) { rt.submit(game.command(action)) }  // Spiel
input.end_tick_edges_if(rt.frame(frame.elapsed_nanos()).ticks > 0)          // Spiel
view.draw_into(frame.target(), blend(alpha))? // Modul zeichnet in geliehenen Command Buffer
host.end_frame(frame)?;                   // Host submittet und präsentiert (ADR 0002)
```

Begründung: Ein Rahmenwerk mit Rückrufen würde Spiele in eine Form zwingen. Wizardry (rundenbasiert, `Runtime::tick`) und der Stealth-Hof (Echtzeit, `Runtime::frame`) brauchen verschiedene Schleifen. Rückrufe im Hot Path verbietet ADR 0008 ohnehin.

### 3. Echte Zeit und feste Ticks

- Die einzige Uhr ist im Host: `SDL_GetTicksNS` (monoton). `begin_frame` liefert `elapsed_nanos` seit dem letzten Frame.
- Der Host kappt einen einzelnen Sprung (Standard 250 ms; Fenster verschoben, Debugger-Halt), danach greift die vorhandene Regel von `FixedStep` (`max_ticks`: wer zurückfällt, spielt langsamer, statt Ticks aufzustauen).
- Pause heißt `elapsed_nanos = 0` an `Runtime::frame`. Die Simulation erfährt nichts davon.
- Echte Zeit, `alpha` und die Bildrate gelangen nie in die Simulation (ADR 0003). `alpha` dient nur dem Zeichnen.
- Bildtakt: VSync über die Swapchain (Standard an). Ein Frame-Limiter ohne VSync ist Sache des Hosts und berührt die Simulation nicht.

### 4. Tasten werden Commands

Drei Stufen, jede gehört einem anderen Besitzer:

1. **Gerät → `InputEvent`** (`knorke-host`): reine Übersetzung der SDL-Ereignisse, ohne Bedeutung.
2. **`InputEvent` → Aktion** (`knorke-input::Bindings<A>`, Tabelle vom Spiel bzw. Profil): z. B. `Scancode::Up → Action::Move(North)`. Die Tabelle ist Spiel- oder Nutzerdaten. Fachmodule kennen weder Tasten noch Aktionen.
3. **Aktion → `S::Command`** (Spielcode): die semantische Absicht, z. B. `Command::Step(Dir::North)` im Stealth-Profil. Für Fachmodule entstehen Commands nur so.

Die Abtastung passiert an der Tick-Grenze. Flanken (gedrückt seit dem letzten Tick) und Haltezustand werden zwischen Ticks gesammelt. Was daraus folgt, gibt das Spiel einmal pro anstehendem Tick als Commands ab. Ein kurzer Tastendruck zwischen zwei Ticks geht nicht verloren, ein gehaltener erzeugt nicht pro Frame ein Command. `Runtime::submit` legt die Commands in den nächsten Tick. Ist in einem Frame kein Tick fällig, warten sie (vorhandenes Verhalten).

### 5. Sitzung als Replay

- Aufgezeichnet werden die **Commands pro Tick**, nicht Tasten, nicht Zeitstempel und nicht `elapsed_nanos`. Das Spiel startet `Runtime` mit einem `Recorder` als Treiber, auf demselben Start-Tick wie die Simulation. Abgeschaltet wird mit `NoRecord` bzw. `Option<Recorder>` (ADR 0011, aus = kostenlos).
- Deshalb hängt die Wiedergabe nicht von Bildrate, VSync oder Rechnerlast ab. `verify` spielt das `ReplayLog` ohne Fenster und ohne GPU nach und muss gleiche Prüfsummen liefern.
- Erster Schritt: Am Sitzungsende prüft das Spiel das Log im Speicher mit `verify`, auf Wunsch per Kommandozeilenschalter. Die Datei kommt im selben Schritt dazu (entschiedene Frage 2). Das Dateiformat gehört nicht in `knorke-replay`, weil dessen Abhängigkeitsliste geschlossen ist (nur `knorke-types`).

### 6. Gerät und Shader-Paket aus dem Host, nicht aus `test_support`

- `knorke-host` ruft `init_video`, erzeugt das Fenster, `Device::create` mit den Shaderformaten des Pakets und bindet die Swapchain an das Fenster. Gerät, Swapchain und Frame-Takt gehören dem Host (ADR 0002). Module zeichnen in den geliehenen Command Buffer und das Swapchain-Ziel, submitten und präsentieren nie.
- Das Shader-Paket lädt der Host als Datei: Standard ist `shaders/` neben der ausführbaren Datei, überschreibbar mit `KNORKE_SHADER_DIR`. Er prüft es mit `expect_source_checksum`. Fehlt es oder ist es veraltet, gibt es eine `Diagnostic` mit stabilem Code (`knorke.host.shader_pack.missing`, `knorke.host.shader_pack.stale`, `knorke.host.device.unavailable`) und keinen stillen Fallback.
- `knorke_gpu::test_support` bleibt unverändert und **nur** für Tests (`test-support` nur in Dev-Abhängigkeiten). Eine Regel in `xtask` verhindert, dass eine normale Abhängigkeit es einschaltet.
- Zeichnen ins Fenster liest nicht zurück (ADR 0021: alles auf der GPU). `stealth_view` bekommt dafür einen Weg, in ein geliehenes Ziel zu zeichnen. Der PNG-Weg mit Rücklesen bleibt für Tests.

### 7. Neue Abhängigkeitsregeln in `xtask/src/deps.rs`

Konkret, als neue oder erweiterte `Rule`:

1. **Niemand hängt am Host.** Neue Regel: `from: ["knorke-*"]`, `to: ["knorke-host"]`, `except: ["knorke-host"]`. Grund: Der Host ist die oberste, abnehmbare Schicht. Beispiele, Tests und Spiele binden ihn als Dev-Abhängigkeit oder als eigene Binary ein, und Dev-Abhängigkeiten ignoriert die Prüfung.
2. **Eingabe nur oben.** Neue Regel: `from:` alle Fach- und Basis-Crates (`knorke-types`, `knorke-math`, `knorke-observe`, `knorke-replay`, `knorke-runtime`, `knorke-exec`, `knorke-grid`, `knorke-ai`, `knorke-navigation`, `knorke-perception`, `knorke-render`, `knorke-lighting`, `knorke-light2d`, `knorke-post`, `knorke-gpu`, `knorke-audio`, `knorke-audio-sdl`, `knorke-scene`, `knorke-integration`), `to: ["knorke-input"]`. Grund: Module sehen Commands, keine Tasten (ADR 0010).
3. **`knorke-input` geschlossen.** Neue Regel: `from: ["knorke-input"]`, `to: ["*"]`, `except: ["knorke-input", "knorke-types"]`. Grund: Die Zuordnung soll ohne SDL und ohne Engine-Fachcrates in fremde Projekte passen.
4. **`knorke-host` geschlossen.** Neue Regel: `from: ["knorke-host"]`, `to: ["knorke-*"]`, `except: ["knorke-host", "knorke-types", "knorke-input", "knorke-gpu", "knorke-audio-sdl", "knorke-audio", "knorke-math"]`. Grund: Der Host kennt keine KI, keine Navigation, kein Licht, keine Szene, kein Profil und auch nicht `knorke-runtime`. `knorke-audio`/`knorke-math` erreicht er nur, weil `knorke-audio-sdl` sie mitbringt. Fremde Crates sind frei, außer:
5. **Kein Grafik-API am Host vorbei.** Neue Regel: `from: ["knorke-host"]`, `to: ["wgpu*"]`. SDL3 ist hier ausdrücklich erlaubt, weil Fenster und Ereignisse direkt über `sdl3-sys` laufen. GPU-Aufrufe laufen nur über `knorke-gpu`. Das prüft das Review, ein Graph kann es nicht ausdrücken.
6. **Bestehende Listen erweitern:** `knorke-host` und `knorke-input` zur `to`-Liste von `DOMAIN_HEADLESS` und der `knorke-light2d`-Regel ergänzen. Das ist doppelt zu Regel 1 und 2, macht die Gründe aber an der Stelle lesbar, an der man sucht. `knorke-host` außerdem in die Regel „domain modules never import their adapters back“ als `to` aufnehmen.
7. **`test_support` nur in Tests.** Neue Prüfung in `deps.rs` (Feature-Kante statt Paketkante): Keine normale Abhängigkeit eines Workspace-Pakets schaltet `knorke-gpu/test-support` ein. Grund: Gerät und Paket kommen in Programmen aus dem Host (Entscheidung 6).

Dazu `cargo xtask standalone`: `knorke-input` (ohne Features) und `knorke-host` (ohne Features, mit `sdl`, mit `sdl,audio`) einzeln bauen.

## Konsequenzen

- Wizardry oder ein anderes Projekt kann `knorke-ai`, `knorke-audio` oder `knorke-light2d` (mit `knorke-gpu`) übernehmen, ohne `knorke-host`, `knorke-input`, ein Fenster oder SDL-Ereignisse mitzunehmen. `knorke-light2d` braucht wie bisher nur ein Gerät von `knorke-gpu`, das der fremde Host selbst erzeugen darf.
- Weil das Spiel die Schleife besitzt, steht in jedem Spiel etwas Schleifencode. Das ist gewollt: Er ist kurz, und er ist genau die Stelle, an der sich Spiele unterscheiden.
- Replays bleiben unabhängig von der Bildrate. Der Preis: Alles, was ein Spieler beeinflusst, muss ein Command sein. Kamera und Menü ohne Wirkung auf die Simulation bleiben Host- bzw. Spielzustand außerhalb.
- Zwei Crates mehr. `knorke-input` ist klein, hat aber von Anfang an Code und einen Verbraucher (das Stealth-Beispiel), also kein Platzhalter.
- Im Fenster wird nicht zurückgelesen. `stealth_view` braucht dafür ein Zeichenziel von außen, das ist eine Änderung in `knorke-integration`.
- SDL3 wird in drei Crates eingebunden (`knorke-gpu`, `knorke-audio-sdl`, `knorke-host`), alle über dieselbe `sdl3-sys`-Workspace-Version und dieselben Features `sdl`/`sdl-build-static`. `SDL_Init` für Video und Audio ruft nur der Host auf. Die Bibliotheken setzen voraus, dass das geschehen ist, oder bieten wie heute `init_video` an.

## Umsetzungsplan

1. **ADR annehmen, Regeln zuerst.** Regeln 1–7 in `xtask/src/deps.rs`, Crates in `members`/`workspace.dependencies` und `standalone`, jeweils zusammen mit dem ersten Code (keine leeren Crates).
   Fertig, wenn: `cargo xtask deps` grün ist und ein absichtlich falscher Eintrag (z. B. `knorke-ai → knorke-input`) rot wird.
2. **`knorke-input`.** `InputEvent`, `InputState` mit Flanken pro Tick, `Bindings<A>`. Tests: kurzer Druck zwischen zwei Ticks ergibt genau eine Aktion, gehaltene Taste ergibt keine Wiederholung pro Frame, Reihenfolge bleibt fest.
   Fertig, wenn: die Tests headless grün sind, die Crate `no_std` baut und `standalone` sie allein baut.
3. **`knorke-host`: Fenster, Uhr, Ereignisse.** `Host::new(config)`, `begin_frame`/`end_frame`, `elapsed_nanos` mit Kappung, SDL-Ereignisse → `InputEvent`, `quit`. Noch ohne GPU: leeres Fenster.
   Fertig, wenn: ein Beispiel ein Fenster öffnet, Tasten als `InputEvent` protokolliert und sich sauber schließt. Ein Test ohne Video-Treiber (`SDL_VIDEO_DRIVER=dummy`) prüft die Übersetzung.
4. **Gerät, Swapchain, Shader-Paket.** Gerät am Fenster, Paket aus `KNORKE_SHADER_DIR` bzw. neben der Binary, Prüfsumme, Diagnostics mit den Codes aus Entscheidung 6. Präsentieren nur im Host.
   Fertig, wenn: das Fenster eine Löschfarbe zeigt (linear, sRGB einmal am Ende), ein fehlendes oder veraltetes Paket die passende `Diagnostic` liefert und kein Programm `test_support` einschaltet (Regel 7 grün).
5. **Stealth-Hof im Fenster.** `stealth_view` zeichnet in ein geliehenes Ziel. Das Stealth-Profil bekommt ein semantisches `Command::Step(Dir)`. Das Beispiel `knorke-host/examples/stealth_yard.rs` (Dev-Abhängigkeiten auf `knorke-integration`, `knorke-runtime`) besitzt die Schleife: Pfeiltasten → `Bindings` → `Step` → `Runtime::frame`.
   Fertig, wenn: die Wachmann-Szene im Fenster läuft und sich der Eindringling mit den Pfeiltasten bewegen lässt. Der PNG-Weg und seine Tests bleiben grün.
6. **Sitzung aufzeichnen und prüfen.** `Recorder` als Treiber (Schalter `--record`). Am Ende `verify` im Speicher, Ergebnis auf stderr. Ein Test spielt eine synthetische Eingabefolge mit wechselnden `elapsed_nanos` durch Host-Logik ohne Fenster und prüft mit `verify`.
   Fertig, wenn: eine von Hand gespielte Sitzung `verify` besteht und derselbe Command-Strom bei 30 und 144 Bildern pro Sekunde dieselben Prüfsummen ergibt.
7. **Audio (optional, Feature `audio`).** Der Host öffnet über `knorke-audio-sdl` das Audiogerät. Das Spiel gibt die Klänge der Szene als Mixer-Commands.
   Fertig, wenn: das Beispiel mit `--features audio` Klang abspielt und ohne das Feature weder `knorke-audio-sdl` baut noch das Audiogerät öffnet.
8. **Abschluss.** Volle Prüfliste aus `CLAUDE.md`, Plan 16 und 17.1 auf „erledigt“, ADR auf „angenommen“.

## Entschiedene Fragen (Manuel, 2026-11-28)

1. **`knorke-input` ist eine eigene Crate** und deckt Tastatur, Maus und Gamepad ab. So bekommt ein fremdes Projekt Eingabe und Zuordnung ohne SDL, und Zuordnungen lassen sich headless testen.
2. **Replay-Datei gleich in Schritt 6.** Aufnahme ist abschaltbar (aus = `NoRecord`, kostenlos, ADR 0011) und läuft nur auf Wunsch (`--record <datei>`); `--replay <datei>` spielt sie ohne Fenster ab und prüft mit `verify`. Zweck: Fehler aus einer gespielten Sitzung als Test nachstellen. Kodierung im Spiel bzw. Profil (`Command` → Bytes, z. B. über das vorhandene serde in `knorke-scene`), Kopf mit Formatversion und Szenen-Kennung (Plan 17.1); `knorke-replay` bleibt bei `knorke-types`. Schritt 6 ist fertig, wenn zusätzlich eine gespeicherte Datei nach Programmneustart `verify` besteht und eine Datei einer anderen Szene mit `Diagnostic` abgelehnt wird.
