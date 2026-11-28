# ADR 0024: C-Schnittstelle für den Pixel-Renderer (Ersatz für `pixelraum.h`)

Status: angenommen 2026-11-28 · Datum: 2026-11-28 · Bezug: ADR 0001, 0005, 0013, 0021 (Schritt 3D-4), Plan 17.1, 17.14

## Kontext

Wizardry 7 (`wizardry7/revised/view3d/view3d.c`) zeichnet seine 3D-Ansicht noch mit dem alten Repo pixelraum. Der Code ist längst nach Knorke umgezogen (`knorke-render`, `knorke-lighting`, `knorke-integration::pixel`, ADR 0013). Nur die C-Schnittstelle (`pixelraum/src/capi.rs`, `include/pixelraum.h`) fehlt noch. Solange sie fehlt, gibt es zwei Kopien desselben Renderers.

Wizardry ruft rund 30 `pr_*`-Funktionen auf, alle in einer einzigen Datei.

ADR 0005 Punkt 6 sagt: Bindings sind Randpakete, sie werden neu entworfen und nicht 1:1 übernommen.

## Entscheidung

1. **Neues Randpaket `knorke-pixel-c`** unter `bindings/knorke-pixel-c/`, also nicht unter `crates/`. Es ist ein Workspace-Mitglied mit `crate-type = ["staticlib", "rlib"]` und baut auf `knorke-integration` (Feature `pixel`) auf.
2. **Erst kompatibel, dann neu entwerfen.** Die erste Fassung bietet dieselbe C-API wie pixelraum: gleiche Namen (`pr_*`), gleiche Strukturen, gleicher Header `include/pixelraum.h`, gleiche Versionsprüfung. Wizardry stellt damit nur den Pfad im Build um. Das ist eine bewusste Ausnahme von ADR 0005 Punkt 6, weil es mit Wizardry einen echten Nutzer gibt. Ein Neuentwurf (`knorke-abi`, Plan 17.1) folgt erst, wenn ein zweiter Nutzer da ist.
3. **Nachweis:** Die Tests aus `capi.rs` werden mit übernommen. Zusätzlich gibt es einen Vergleich: Wizardry-Szenen sollen mit Knorke dieselben Pixel liefern wie mit pixelraum.
4. **`unsafe`** ist nur in diesem Paket erlaubt (FFI-Grenze, ADR 0005): Lint-Tabelle mit `deny`, lokales `allow` am FFI-Modul, jede Stelle mit `// SAFETY:`.
5. **Abhängigkeitsregel:** Kein Workspace-Paket hängt von `knorke-pixel-c` ab (`xtask deps`).

## Konsequenzen

- Nach der Umstellung von Wizardry wird pixelraum nach `D:\Entwicklung\_Archiv` verschoben.
- Die C-API ist eingefroren, bis `knorke-abi` kommt. Neue Funktionen bekommt sie nur, wenn Wizardry sie braucht.
- **Linken unter Windows (MinGW):** Neben der Bibliothek muss der C-Host auch `synchronization` linken, denn die Rust-Standardbibliothek braucht `WaitOnAddress`/`WakeByAddress*`. Pixelraum brauchte das nicht. Festgestellt bei der Umstellung von Wizardry am 2026-11-28.
- Herkunft im Code: pixelraum @ `81927d8`, `src/capi.rs`, `include/pixelraum.h` (MIT).
