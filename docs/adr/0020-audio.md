# ADR 0020: Audio – `knorke-audio` als Mischpult ohne Gerät

Status: angenommen (Stufe 1 und 2 umgesetzt 2026-11-27) · Datum: 2026-11-27 · Bezug: ADR 0001, 0002, 0006, 0008, 0011, Plan 17.6

## Kontext

Die Engine soll alte Spielsounds (Wizardry 7, Syndicate: 8-Bit-Mono-PCM, AdLib/OPL-Musik) abspielen und aufbessern können: Raumwirkung, Entfernung, Verdeckung hinter Türen, Überblenden. Audio hat harte Fristen (wenige Millisekunden pro Rückruf): kein Sperren, keine Allokation im Audio-Rückruf. Die Wizardry-7-Neufassung (`wizardry7/revised/platform/sdl/audio_sdl.c`, C) hat dafür erprobte, billige Formeln: Pegel und Tiefpass aus Entfernung, Wänden und „hinter mir“, kleiner Zeitversatz zwischen den Ohren, kurze Raum-Echos, weiche Pegelgrenze.

## Entscheidung

1. **Zuschnitt wie GPU (ADR 0002).** Der Host besitzt Gerät, Thread und Takt (SDL3, `cpal`, …). `knorke-audio` ist ein reines Mischpult: `Mixer::render(&sounds, &mut out)` schreibt Stereo-Frames in einen geliehenen Puffer. Kein Thread, keine Queue, kein Gerät, keine weitere Abhängigkeit außer `knorke-math`.
2. **Steuerung nur über `Command`** (ADR 0008): `Copy`-Werte, die der Host über eine Queue seiner Wahl (z. B. `rtrb`) zum Audio-Thread bringt und dort mit `Mixer::apply` anwendet. Das Spiel vergibt die `VoiceId` selbst, damit kein Rückkanal nötig ist.
3. **Klangdaten leiht der Host** über das Trait `Sounds` (ADR 0006); `[Pcm]` ist die einfachste Umsetzung. Alte Formate dekodiert ein Lader im Spiel, die Engine kennt nur PCM (`f32`, mono oder stereo, beliebige Rate).
4. **Feste Kosten:** Alle Puffer werden in `Mixer::new` angelegt; `apply` und `render` allozieren nie (Vertragstest). Gerechnet wird in Blöcken von 256 Frames auf einem festen Raster ab Frame 0 – das Ergebnis hängt nicht von der Rückruf-Größe ab. Raumwerte entstehen einmal pro Block und werden innerhalb des Blocks linear angeglichen (kein Knacken). Unhörbare Stimmen laufen „virtuell“ weiter: nur die Position rückt vor.
5. **Raumklang Stufe 1** nach Wizardry-Modell: Hörer mit Position, Blickrichtung und Raum; Pegel `1 / ((1 + rolloff·d) · (1 + 0,6·Wände))`, Helligkeit (Koeffizient eines einpoligen Tiefpasses) `1 / (1 + Luft·d + 5·Wände + 0,55·hinten)`, Panorama mit gleicher Leistung, mit Kopfhörern zusätzlich Zeitversatz bis 0,65 ms und Kopfschatten am fernen Ohr. Wände zählt das Spiel (Raster, Türen) und setzt sie per `Command::Occlude`.
6. **Raum-Echos** als drei kurze Verzögerungen auf einer Ringleitung je Raumart (`Room::SMALL`, `STONE`, `CAVE`), wählbar am Hörer, sanft überblendet.
7. **Busse** (Anzahl in `Config`): Lautstärke mit Blende pro Bus, z. B. Musik/Effekte/Oberfläche. Ducking und Mix-Zustände sind Bus-Blenden, die das Spiel auslöst.
8. **Ausgang:** feste Pegelreserve und weiches Knie statt hartem Abschneiden.
9. **Stufe 2 – Zeit, Busse, Höchstzahl** (2026-11-27): Eine Stimme kann auf einem Ausgabe-Frame starten (`Play::start`, Zeitbasis `Mixer::clock` = Frames, die das Gerät bekam) und bei gesetzter Schallgeschwindigkeit um ihre Laufzeit später; bis dahin rechnet sie nichts, ein `Stop` beendet sie sofort. `Play::limit` begrenzt Stimmen desselben Klangs. Jeder Bus hat einen Filter (je zwei einpolige Tief- und Hochpässe, Koeffizienten mit Blende) und optional ein Ducking, das den Pegel eines anderen Busses vor dessen Bus-Lautstärke verfolgt. Alles pro Frame, damit das Ergebnis nicht von der Rückruf-Größe abhängt; ein offener Filter wird übersprungen (ADR 0011).
10. **Deterministisch** über `knorke_math::det`: Ein offline gerenderter Block ist bitgleich auf jeder Plattform, daher als Orakel für Tests (Prüfsumme) nutzbar. Audio bleibt Darstellung: kein `Snapshot`, keine Rückwirkung auf die Simulation; was Figuren hören, bleibt bei `knorke-perception`.

11. **Retro-Synthesizer und OPL** (2026-11-28): `knorke_audio::chip` rendert Noten im Stil alter Chips (NES, Game Boy, C64, AdLib, Amiga) offline in Host-Puffer; das Ergebnis ist PCM für den Mixer. Mit Feature `opl` läuft echte AdLib-/Sound-Blaster-Musik über den OPL2/OPL3-Emulator `oplon` (MIT, `no_std`, nur Ganzzahlen, keine Abhängigkeiten, codeberg.org/sbechet/oplon) – die einzige Ausnahme von „nur `knorke-math`“, optional und ohne Feature kostenlos (ADR 0011). Nuked OPL3 (LGPL) und `opl-emu` (GPL) bleiben ausgeschlossen.

## Konsequenzen

- Musik als Datenstrom (OPL-Emulator, Synthesizer) braucht später eine Quelle, die Frames erzeugt, statt PCM zu lesen (Plan 17.6, Stufe 2). OPL nicht als LGPL-Code fest einbauen (`ymfm`, BSD-3, prüfen).
- Lineare Interpolation beim Umrechnen der Rate: billig und für alte 8-Bit-Quellen ausreichend; bessere Filter erst nach Hörvergleich.
- Hall aus Raumgröße, Beugung um Ecken, HRTF: spätere Stufen, nur mit Messung.
- Abhängigkeitsregel in `xtask/src/deps.rs`: `knorke-audio` darf nur `knorke-math` (und dessen Liste) nutzen; Fachmodule ohne Darstellung kennen `knorke-audio` nicht.
- Host-Seite für SDL3: `knorke-audio-sdl` (2026-11-27). Rückruf mit `try_lock` auf einen festen Befehls-Eingang statt eigener Lock-free-Queue – ohne weitere Abhängigkeit; kommt das Spiel gerade dazwischen, wirken die Befehle einen Rückruf später (wenige ms). Darf außer SDL3 nur `knorke-audio`, `knorke-math`, `knorke-types` nutzen (`xtask/src/deps.rs`).
