# Architekturentscheidungen (ADR)

Bindend für alle Änderungen (`CLAUDE.md`). Neue Grundsatzentscheidung → neue Datei `NNNN-titel.md` mit Status, Datum, Kontext, Entscheidung, Konsequenzen; hier eintragen.

| Nr. | Titel | Status |
|---|---|---|
| [0000](0000-name-knorke.md) | Produktname „Knorke Engine“ und Namensschema | angenommen; Marken-/Crate-Prüfung offen |
| [0001](0001-modularity.md) | Modularität: eigenständige Crates, erzwungene Abhängigkeitsgrenzen | angenommen |
| [0002](0002-gpu-and-render-lifecycle.md) | GPU-Ownership und Render-Lifecycle | vorgeschlagen (bis Spike A) |
| [0003](0003-simulation-and-time.md) | Simulationszeit, Determinismus und Seeds | angenommen |
| [0004](0004-schema-and-capabilities.md) | Schemas, Capabilities und Diagnosen | teilweise angenommen |
| [0005](0005-rust-rewrite-policy.md) | Rust-Rewrite-Policy | angenommen |
| [0006](0006-data-exchange.md) | Datenaustausch ohne Kopien | angenommen |
| [0007](0007-observability.md) | Gemeinsames Debugging und Analyse | angenommen |
| [0008](0008-events-commands-parallelism.md) | Events, Commands und Parallelität ohne Broker | angenommen |
| [0009](0009-frame-schedule.md) | Frame-Ablauf und Scheduler | vorgeschlagen (mit ersten Systemen) |
| [0010](0010-snapshot-replay-bug-detection.md) | Snapshots, Replay und automatische Fehlererkennung | angenommen |
| [0011](0011-cross-platform-determinism-and-optional-features.md) | Plattformübergreifender Determinismus und abschaltbare Zusatzfeatures | angenommen (Windows und WASM lokal belegt; CI-Bestätigung Linux/ARM offen) |
| [0012](0012-sagax-split.md) | Sagax-Neuschnitt: Grid, Navigation, Perception, AI, Integration | angenommen |
| [0013](0013-cpu-rendering.md) | CPU-Rendering: pixelraum als `knorke-render` + `knorke-lighting` | angenommen (lineares Licht umgesetzt) |
| [0014](0014-chiaro-port-split.md) | Chiaro-Port: Schnitt in `knorke-gpu`/`-post`/`-light2d`, Shader als getrennte Pakete pro Format (Vulkan Standard, D3D12 optional), GPU als Hauptweg | angenommen (`sdl3-sys`-Prüfung offen) |
| [0015](0015-chiaro-gate-rust.md) | Chiaro-Gate bestanden: Rust wird Zielimplementierung, sichere Pass-API vor Welle 2 | angenommen |
