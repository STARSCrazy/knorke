# ADR 0001 – Modularität: eigenständige Crates, erzwungene Abhängigkeitsgrenzen

**Status:** angenommen
**Datum:** 2026-11-23

## Kontext

Plan-Prinzip 1 (Standalone) und 2 (Komposition) widersprechen sich, sobald Bequemlichkeit gewinnt: ein kleiner Import von `knorke-render` in `knorke-lighting` „nur für den Texturtyp“ macht Lighting unbrauchbar ohne Renderer. Solche Kanten entstehen leise und werden erst bemerkt, wenn ein Fremdnutzer scheitert.

## Entscheidung

1. **Ein Cargo-Monorepo**, jede fachliche Crate separat nutzbar. Repo-Grenze ≠ Modulgrenze.
2. **Verbotene Kanten sind Code, nicht Doku:** `cargo xtask deps` prüft die transitive Hülle der normalen und Build-Abhängigkeiten jeder Workspace-Crate gegen `xtask/src/deps.rs::RULES`. Dev-Abhängigkeiten sind frei (Tests dürfen kombinieren). CI bricht bei Verstoß ab.
3. Die Prüfung läuft mit `--all-features`, optionale Abhängigkeiten zählen also mit.
4. **Standalone-Build:** `cargo xtask standalone` prüft jede Bibliothek einzeln mit `--no-default-features`, damit keine Crate nur dank Feature-Unification aus Nachbarn kompiliert.
5. **`knorke-types` bleibt winzig und abhängigkeitsfrei** (ebenfalls als Regel erzwungen). Aufnahmekriterium: Mindestens zwei unabhängige Fachmodule brauchen den Typ, um ohne Konvertierung zu komponieren. Aktueller Inhalt: Identität (`Handle`, `Slots`, `HandleMap`), geliehene Massendaten (`Column`, `project`), Kommunikation (`EventQueue`, `CommandBuffer`, `Revision`), Parallelität (`Parallel`, `Serial`), Flags (`BitSet`), Determinismus (`Tick`, `Seed`, `StableHasher`), Zustand sichern (`Snapshot`, `SnapshotBuf`, `SnapshotValue`, `Checksum`), Fehler (`Diagnostic`). Begründung je Typ: ADR 0006, 0008 und 0010. Der Snapshot-Vertrag bleibt hier: Jedes Modul mit Simulationszustand braucht ihn, und `Slots`/`HandleMap`/`EventQueue` sichern ihren privaten Zustand (Freiliste, Generationen) selbst – eine eigene Crate müsste diese Interna öffentlich machen oder `knorke-types` eine Abhängigkeit geben.
6. **Keine Koordinaten, Einheiten, Mathe- oder GPU-Typen in `knorke-types`.** Gemeinsame Werttypen stehen in `knorke-math`, Koordinatenräume dort als eigene Typen (`At<Space, V>`); Umrechnung explizit über `Transform2<A, B>` (ADR 0006).
7. **Authoring-ID ≠ Runtime-Handle.** `Handle<T>` ist generationsgesichert und nur für die ausstellende Storage gültig; persistente Identität bekommt `knorke-scene` (eigener Typ, eigene ADR in Phase 3).
8. Neue Crates werden erst angelegt, wenn es Code und einen Verbraucher gibt. Leere Platzhalter-Crates sind verboten.

## Regeln (Stand)

| Von | Darf nicht (transitiv) nach | Grund |
|---|---|---|
| `knorke-ai`, `-navigation`, `-perception` | render, lighting, gpu, runtime, scene, integration, agent, cli, mcp, `sdl3*`, `wgpu*` | headless |
| `knorke-render` | `knorke-lighting` | unabhängig; nur Integration verbindet |
| `knorke-lighting` | render, runtime, scene | arbeitet auf Host-Targets |
| `knorke-runtime` | agent, cli, mcp | Release-Spiel ohne Development Plane |
| alle Fachmodule + types/math/observe/gpu | `knorke-integration` | Adapter zeigen nach innen, nie zurück |
| `knorke-types` | `*` | bleibt Vertrag, wird kein Core |
| `knorke-math` | `*` außer `bytemuck`, `mint`, `knorke-types`, `libm` | Basis-Crate, geschlossene Abhängigkeitsliste (ADR 0006) |
| `knorke-observe` | `*` außer `knorke-types` | Basis-Crate, geschlossene Abhängigkeitsliste (ADR 0007) |
| `knorke-replay` | `*` außer `knorke-types` | Replay darf als Flight Recorder in Release-Builds mitlaufen (ADR 0010) |
| `knorke-exec` | `*` außer `knorke-types`, `rayon`, `rayon-core`, `crossbeam-*`, `either` | Executor des Hosts (ADR 0008) |
| Basis- und Fachmodule | `knorke-exec`, `rayon*` | Module parallelisieren über `Parallel`; der Host wählt den Executor (ADR 0008) |

## Konsequenzen

- Gemeinsame Typen, die nur zwei Module betreffen, wandern in das Integrationsprofil oder werden in beiden Modulen als Trait-Anforderung formuliert statt in `knorke-types` gezogen.
- Regeländerungen brauchen eine Änderung dieser ADR.
