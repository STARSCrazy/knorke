# ADR 0000 – Produktname „Knorke Engine“ und Namensschema

**Status:** angenommen (Name) · offen (Marken-/Crate-/Domain-Prüfung)
**Datum:** 2026-11-23

## Kontext

Sagax (KI/Navigation, Rust) und Chiaro (Beleuchtung, C++) werden in eine gemeinsame Engine überführt. Ohne festes Namensschema entstehen Mischformen (`knorke-sagax`, `chiaro-rs`), die alte Grenzen konservieren.

## Entscheidung

- Produkt: **Knorke Engine**, Kurzname **Knorke**.
- Rust-Pakete `knorke-<fachfunktion>`, Imports `knorke_<fachfunktion>`. Der Paketname benennt die Fachfunktion, nie die Herkunft (`knorke-lighting`, nicht `knorke-chiaro`).
- Schema-/Komponenten-IDs: `knorke.<modul>.<name>/<major>`, z. B. `knorke.lighting.point/1`.
- Diagnosecodes: `knorke.<modul>.<...>` gemäß `knorke_types::DiagnosticCode` (Kleinbuchstaben, Ziffern, `_`, punktgetrennt, ≥ 2 Segmente).
- Agenten-Operationen: `<bereich>.<objekt>.<verb>`, z. B. `scene.entity.create`.
- Sagax und Chiaro bleiben als Herkunft in Migrationsdoku und Git-Historie nachvollziehbar.

## Offen (vor erster Veröffentlichung)

- [ ] crates.io: `knorke`, `knorke-types`, `knorke-render`, `knorke-lighting`, `knorke-navigation`, `knorke-ai`, `knorke-perception` frei?
- [ ] Marke „Knorke“ in Klasse 9/41 (DPMA/EUIPO) – Kollisionsrecherche
- [ ] Domain(s)

Bis dahin `publish = false` im gesamten Workspace.
