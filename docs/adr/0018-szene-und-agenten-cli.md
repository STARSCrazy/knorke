# ADR 0018 – Szene als Text, Profil `stealth`, Agenten-Kommandozeile

**Status:** angenommen (2026-11-25) – Stufe 1 umgesetzt. Auftrag von Manuel: „Szene als Datei plus Kommandozeile für Agenten“. Die Einzelentscheidungen unten hat Claude getroffen; Manuel hat sie noch nicht einzeln bestätigt.
**Datum:** 2026-11-25
**Bezug:** Plan 8 (Agenten-Architektur), Plan Phase 3, ADR 0001, 0004, 0009, 0010

## Kontext

Bis hierher war die Wachmann-Szene Rust-Code in einem Beispiel: Hofgröße, Mauer, Posten und Lampe standen als Konstanten im Quelltext. Ein Agent konnte nichts daran ändern, ohne Engine-Code anzufassen. Plan 8 verlangt das Gegenteil: Szenen als Daten, Änderungen als geprüfte Transaktionen, eine Schema-Quelle für alle Zugänge, CLI zuerst.

## Entscheidung

### 1. Drei Schichten, drei Crates

| Crate | Aufgabe | kennt |
|---|---|---|
| `knorke-scene` | Authoring State: Szene als Text, Komponenten-Registry, Change-Sets mit Undo | nur `knorke-types` (+ `serde`, `serde_json`, `schemars`) |
| `knorke-integration`, Feature `stealth` | Profil: Komponententypen und Regeln der Wachmann-Welt; baut aus einer Szene eine `Simulation` | Szene, Laufzeit, Navigation, Wahrnehmung, KI |
| `knorke-agent` | Operationen `discover`, `inspect`, `validate`, `mutate`, `execute` als Funktionen und als Kommandozeile `knorke` | Szene, Profil, Laufzeit |

Authoring ist nicht Runtime (Plan 8.2): Die Szene wird genau einmal gelesen, in `stealth::build`. Ein Tick fasst sie nie an; im Hot Path läuft kein JSON.

### 2. Szenenformat

- JSON, Kennung `knorke.scene/1`. Eine Szene ist `revision` + Entities; eine Entity ist eine Menge von Komponenten; eine Komponente ist ein JSON-Wert unter ihrer versionierten Id (`knorke.stealth.guard/1`).
- Sortierte Schlüssel (`BTreeMap`), eingerückt, Zeilenende am Schluss: der Text ist stabil und diff-bar.
- JSON statt einer eigenen Notation (Plan 17.11 Nr. 18): Agenten schreiben es fehlerfrei, es braucht keinen eigenen Parser, und JSON Schema beschreibt es. Eine kompaktere Notation bleibt möglich, solange sie auf dasselbe Modell abbildet.

### 3. Eine Schema-Quelle: der Rust-Typ

Eine Komponente ist ein Rust-Typ, der `knorke_scene::Component` implementiert. Aus ihm entstehen:

- Lesen und Schreiben (`serde`),
- das JSON Schema für `discover` (`schemars`, wie in ADR 0004 vorgeschlagen),
- die Prüfungen, die der Typ nicht ausdrücken kann (`Component::check`), und Regeln über die ganze Szene (`Registry::rule`), jeweils als `Diagnostic` mit stabilem Code und Pfad.

Nichts davon wird von Hand doppelt gepflegt. Damit ist der in ADR 0004 vorgeschlagene Teil „Schemaformat“ angenommen; das Capability-Manifest pro Modul (ADR 0004, Plan 8.3) bleibt offen.

### 4. Änderungen nur als Change-Set

- Ein `ChangeSet` nennt die Revision, für die es geschrieben wurde (`base_revision`), und eine Folge von Schritten: `entity.create`, `entity.remove`, `component.set`, `component.remove`.
- `Scene::apply` wendet es **ganz oder gar nicht** an: erst auf eine Kopie, dann wird die ganze Szene geprüft; bei einem Fehler bleibt die Szene unverändert.
- Passt `base_revision` nicht zur Szene, wird abgelehnt (`knorke.scene.revision_mismatch`). So überschreibt kein Agent unbemerkt die Arbeit eines anderen.
- Die Antwort ist das Change-Set, das die Änderung zurücknimmt. Undo ist damit eine ganz normale Änderung (die Revision zählt weiter), kein Sonderweg.

### 5. Kommandozeile `knorke`

- Jeder Aufruf antwortet mit **einem JSON-Objekt** auf stdout: `{"ok": true, …}` oder `{"ok": false, "diagnostics": […]}`. Exit-Code 0, 1 (Fehler mit Diagnosen) oder 2 (Aufruf nicht verstanden).
- Antworten sind klein: `inspect` nennt ohne Entity-Namen nur Entities und ihre Komponenten-Ids; `execute` liefert Ereignisse, Endpositionen und eine Prüfsumme des Endzustands, keinen State-Dump.
- `execute` läuft headless und deterministisch; gleiche Szene und gleiche Eingaben geben dieselbe Prüfsumme.
- Die Kommandozeile ist nur eine Hülle um `knorke_agent::run` und die Funktionen dahinter. Ein MCP-Adapter ruft dieselben Funktionen.

### 6. Abhängigkeiten

- `serde`, `serde_json`, `schemars` dürfen nur in `knorke-scene`, `knorke-integration` (Feature `stealth`) und der Agenten-Ebene vorkommen. Fachmodule (Navigation, Wahrnehmung, KI, Grid, Render, Licht, Post) und die Laufzeit bleiben frei davon; `xtask/src/deps.rs` prüft das.
- `knorke-scene` kennt außer `knorke-types` keine Knorke-Crate.
- `knorke-integration` und `knorke-scene` hängen nie an der Agenten-Ebene.

## Konsequenzen

- Die Wachmann-Logik liegt jetzt als Bibliothek in `knorke_integration::stealth` (vorher Beispielcode in `knorke-runtime`); das Beispiel ist nur noch der Host. Mehrere Wachen sind möglich, Ereignisse nennen die Wache.
- Ein Snapshot passt nur zu der Szene, aus der die Simulation gebaut wurde: Was die Szene festlegt (Mauern, Posten, Namen), steht nicht im Snapshot. `restore` lehnt eine andere Zahl von Wachen und Positionen außerhalb des Hofs ab. Eine Kennung der Szene im Replay-Kopf fehlt noch.
- Diagnose-Codes dieser Stufe sind Vertrag: `knorke.scene.*` (malformed, unknown_schema, unknown_component, invalid_component, revision_mismatch, unknown_entity, duplicate_entity, missing_component), `knorke.stealth.*` (yard_count, yard_size, intruder_count, off_grid, in_wall, lamp_range), `knorke.agent.*` (io, malformed, usage, unknown_entity).

## Bewusst nicht in Stufe 1

- **Mehrere Profile / Registrierung durch Module.** `knorke-agent` kennt fest das Profil `stealth`. Sobald es ein zweites gibt, melden Profile sich an (Capability-Manifest, ADR 0004).
- **`render` als Operation.** Das Bild gibt es über das Beispiel `guard_scene_view`, das jetzt eine Szenen-Datei annimmt. Die Kommandozeile bleibt ohne GPU.
- **`query`, `observe`, `verify`, `checkpoint`/`diff`** (Plan 8.1), Replay-Operationen (`replay.verify`, `replay.seek`), Agenten-Skripte als ein Batch.
- **Prefabs, Assets, Referenzen zwischen Entities**, Migration zwischen Schema-Versionen.
- **Änderungen an einer laufenden Simulation** (Hot-Reload): heute baut `execute` die Simulation jedes Mal neu aus der Szene.
