# ADR 0004 – Schemas, Capabilities und Diagnosen

**Status:** teilweise angenommen (Diagnosen; Schemaformat seit ADR 0018) · Capability-Manifest vorgeschlagen
**Datum:** 2026-11-23

## Angenommen

- **Diagnosen** sind ab sofort einheitlich `knorke_types::Diagnostic { severity, code, message, path }`.
  - `code` ist stabiler Vertrag (Umbenennung = Breaking Change), `message` nicht.
  - `path` adressiert das betroffene Element (`entities/guard.01/components/knorke.ai.utility/1`), damit Agenten ohne State-Dump korrigieren können.
  - Konstante Codes werden zur Compile-Zeit validiert (`DiagnosticCode::new` in `const`).
- Module, die heute eigene Trace-/Fehlertypen haben (Sagax `Sink<E>`-Traces, Chiaro-Fehlercodes), behalten diese intern; Adapter übersetzen an der Grenze. Keine Pflicht-Abhängigkeit der Fachmodule auf ein Logging-Framework.

## Vorgeschlagen (Entscheidung in Phase 3)

- Capability-Manifest pro Modul (Plan 8.3): Modul-ID, SemVer, Komponenten-Schemas mit Einheiten/Bereichen, Operationen mit Vor-/Nachbedingungen und Fehlercodes.
- Schemaformat: JSON Schema (Draft 2020-12) als Austauschformat für Agenten; in Rust aus Typen generiert, nicht von Hand gepflegt. Kandidat `schemars` – Abhängigkeit nur in Dev-Plane-/Manifest-Crates, nie in Fachmodulen. **Angenommen und umgesetzt mit ADR 0018** (`knorke_scene::Component`, `knorke discover`); `schemars` zusätzlich in `knorke-scene` und im Profil `stealth`, weiterhin nie in Fachmodulen.
- Schema-Versionierung `…/<major>`; Minor-Änderungen nur additiv.
