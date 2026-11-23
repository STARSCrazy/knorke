# ADR 0005 – Rust-Rewrite-Policy

**Status:** angenommen
**Datum:** 2026-11-23

## Entscheidung

1. **Rust first.** Neue Kernsysteme in Rust. Bestehende C++-Implementierungen (Chiaro) dienen als Referenz und Vergleichs-Backend, nicht als Zielarchitektur.
2. **Gate vor jedem großen Port** (Plan 5.1): Ein Spike implementiert eine Referenzszene in beiden Varianten; verglichen werden GPU-Passzeiten, Ressourcenübergaben, Diagnose, Build-Komplexität und visuelle Abweichung. Ohne gemessenes Ergebnis kein Komplettport.
3. **Fähigkeiten statt APIs erhalten.** Jede portierte Bibliothek bekommt eine Feature-Matrix (`übernommen` / `vereinfacht` / `neu entworfen` / `bewusst gestrichen` / `offen`). Golden Scenes, Benchmarks und Regressionsfälle der alten Bibliothek werden zu Contract-Tests der neuen.
4. **Referenz-Repos** liegen lokal unter `reference/` (nicht versioniert) auf fixierten Revisionen, dokumentiert in `docs/inventory/`. Code wird übernommen, nicht verlinkt; Herkunft und Lizenz (Sagax: MIT OR Apache-2.0, Chiaro: siehe Inventur) im Commit und in der Datei vermerkt.
5. **`unsafe`** nur an FFI-/GPU-Grenzen und für die `bytemuck`-Marker in `knorke-math/src/pod.rs`, jeweils mit `// SAFETY:`-Begründung. Workspace-Default `forbid`; betroffene Crates setzen `deny` und erlauben es lokal pro Modul.
6. **Bindings (C-ABI, WASM)** sind Randpakete und werden nach Bedarf neu entworfen, nicht 1:1 übernommen.
