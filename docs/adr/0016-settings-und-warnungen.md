# ADR 0016: Settings und Warnungen im Chiaro-Port (P18, P19)

Status: angenommen · Datum: 2026-11-24 · Bezug: ADR 0010, 0011, 0014, Plan 17.11, `docs/inventory/chiaro-aufteilung.md` (Abschnitt 4)

## Kontext

Chiaro hält alle Einstellungen in einem 204-Byte-Struct `CHI_Settings` (C-ABI, Flag-Bits, `0` bedeutet „abgeleitet“, Preset-Werte werden in die Felder zurückgeschrieben, Prüfung liefert einen SDL-Fehlertext). Warnungen sind `CHI_WARN_*`-Bits in `CHI_FrameStats`, geloggt über `SDL_LogWarn`. Beides passt nicht zu ADR 0010 (stabile Diagnosecodes) und ADR 0011 (Zusatzfeatures abschaltbar).

## Entscheidung

1. **Settings werden nach Besitzer geschnitten**, nicht als ein Struct: `knorke_light2d::settings::LightSettings` (Ambient, Qualität, Oberfläche, Material, Glow, Dunst, Debug-Prüfungen) und `knorke_post::settings::PostSettings` (Ausgabe, Bloom, CRT). Der Host kombiniert sie; `knorke-light2d` und `knorke-post` kennen sich weiter nicht (ADR 0014).
2. **Typen statt Flag-Bits und Sentinelwerte.**
   - Aufzählungen und `bool` statt `CHI_*`-Bits (`Quality`, `Filter`, `Tonemap::Soft { knee }`, `shafts`, `dither`, `overlay`).
   - „0 = abgeleitet“ wird `Option<f32>` (`None` = abgeleitet). Folge: ein ausdrückliches `Some(0.0)` für `wet_breakup` bedeutet jetzt „kein Aufbrechen“, nicht mehr „abgeleitet“; für Felder, bei denen 0 unzulässig ist, wird `Some(0.0)` abgelehnt.
   - `Quality` ist eine Aufzählung mit Daten (`Custom(Resolutions)`). Es gibt keine doppelt gehaltenen Felder, die ein Preset überschreibt: `resolutions()` liefert immer die gültigen Werte.
3. **Prüfung** durch `validate(&self) -> Result<(), SettingsError>` (erste Verletzung, ohne Allokation); `replace` ersetzt atomar. Fehler tragen stabile Codes `knorke.light2d.settings.{not_finite,out_of_range}` bzw. `knorke.post.settings.*` und werden mit `to_diagnostic()` zu `Diagnostic`. Werte werden auch geprüft, wo sie wirkungslos sind (Bloom aus). Die „Struct ist nullinitialisiert“-Heuristik entfällt: `Default` ist der einzige Ausgangspunkt.
4. **Settings sind Host-Konfiguration, kein Simulationszustand**: kein `Snapshot`, kein Teil des Determinismus-Fingerprints. Wer sie in einem Replay braucht, schreibt sie als Command (ADR 0008) und reicht sie an `replace`.
5. **Warnungen (P18)**: `FrameWarnings` in `knorke-light2d` wertet die Befunde eines Frames aus (`FrameFindings`, reine Zähler, keine Speicherzugriffe auf Stores) und liefert ein `WarningSet` (Bitmenge, typisiert). Jede `Warning` hat einen stabilen Code `knorke.light2d.warning.<name>` und Schweregrad Warnung. Welche Art unter welchem Schalter (`DebugChecks::{LAYERS, LIGHTS, OBSTACLES}`) läuft, bleibt wie in Chiaro. **Aus-Zustand** (ADR 0011): `DebugChecks::NONE` ist der Default; die Auswertung kostet dann einen Sprung, nichts wird gezählt oder gebaut. Das „einmal loggen“ ist Sache des Hosts: `newly_reported()` nennt die Arten, die in diesem Frame erstmals auftraten. Diagnostics werden nur auf Anforderung in einen Host-`Vec` geschrieben.
6. Die Muster-Warnungen (`PATTERN_DESTROYED`, `PATTERN_NO_ALPHA`) kommen mit dem Muster-Store (P15); `Warning` ist `#[non_exhaustive]`.

## Konsequenzen

- `CullSettings` (P8) entfällt; `collect_lights` liest `LightSettings`.
- P20 (Post-Pass) und die GPU-Stufen lesen `LightSettings`/`PostSettings` statt eines C-Structs; die Kosten der Dunststufe liefert `Quality::medium_cost()`.
- Abgeleitete Werte (Dunsthöhe, Mediumschritt, Nässe) sind Methoden, nicht gespeicherte Felder.
