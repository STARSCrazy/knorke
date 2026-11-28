//! Writes a short tune in NES, C64 and AdLib style (imitated and on the emulated OPL2 chip) to WAV files to listen to.
//! `cargo run -p knorke-audio --example chiptune [out-dir]`

use knorke_audio::chip::{self, Chip, Instrument, Note, beats};

const RATE: u32 = 48_000;
const BPM: f32 = 140.0;

/// Melody, bass and drums (key 0 = rest) per eighth note, two bars.
const LEAD: [u8; 16] = [76, 76, 0, 76, 0, 72, 76, 0, 79, 0, 0, 0, 67, 0, 0, 0];
const BASS: [u8; 16] = [50, 0, 50, 0, 50, 0, 50, 0, 55, 0, 55, 0, 43, 0, 43, 0];
const DRUM: [u8; 16] = [1, 0, 2, 0, 1, 0, 2, 1, 1, 0, 2, 0, 1, 1, 2, 2];

fn song(lead: u8, bass: u8, chord: u8, drums: u8) -> Vec<Note> {
    let e = beats(0.5, BPM, RATE);
    let mut notes = Vec::new();
    for rep in 0..2 {
        for i in 0..16 {
            let at = (rep * 16 + i) as u32 * e;
            if LEAD[i] != 0 {
                notes.push(Note {
                    at,
                    len: e * 9 / 10,
                    key: LEAD[i],
                    instrument: lead,
                    gain: 1.0,
                });
            }
            if BASS[i] != 0 {
                notes.push(Note {
                    at,
                    len: e * 8 / 10,
                    key: BASS[i],
                    instrument: bass,
                    gain: 1.0,
                });
            }
            match DRUM[i] {
                1 => notes.push(Note {
                    at,
                    len: e / 3,
                    key: 40,
                    instrument: drums,
                    gain: 1.0,
                }),
                2 => notes.push(Note {
                    at,
                    len: e / 6,
                    key: 100,
                    instrument: drums,
                    gain: 0.6,
                }),
                _ => {}
            }
            if i % 8 == 0 {
                notes.push(Note {
                    at,
                    len: e * 4,
                    key: 60,
                    instrument: chord,
                    gain: 0.5,
                });
            }
        }
    }
    notes
}

fn main() -> std::io::Result<()> {
    let dir = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/chiptune".into()),
    );
    std::fs::create_dir_all(&dir)?;
    let len = beats(16.5, BPM, RATE) as usize;
    let styles: [(&str, Chip, [Instrument; 4]); 3] = [
        (
            "nes",
            Chip::NES,
            [
                Instrument::pulse(0.25),
                Instrument::nes_triangle(),
                Instrument::arp_major(),
                Instrument::noise(false),
            ],
        ),
        (
            "c64",
            Chip::C64,
            [
                Instrument::sid_lead(),
                Instrument::sid_bass(),
                Instrument::arp_major(),
                Instrument::noise(false),
            ],
        ),
        (
            "adlib",
            Chip::ADLIB,
            [
                Instrument::fm_brass(),
                Instrument::fm_organ(),
                Instrument {
                    gain: 0.15,
                    ..Instrument::fm_organ()
                },
                Instrument::noise(false),
            ],
        ),
    ];
    let mut out = vec![0.0; len];
    for (name, c, insts) in styles {
        chip::render(&c, &insts, &song(0, 1, 2, 3), RATE, &mut out);
        let path = dir.join(format!("{name}.wav"));
        write_wav(&path, &out)?;
        println!("{}", path.display());
    }
    // The real chip: the same tune on the emulated OPL2 (AdLib / Sound Blaster).
    let patches = [
        chip::Patch::brass(),
        chip::Patch::bass(),
        chip::Patch::organ(),
        chip::Patch::bell(),
    ];
    let mut notes = song(0, 1, 2, 3);
    // FM has no noise drum; the bell plays the hi-hat line an octave down instead.
    notes.retain(|n| n.instrument != 3 || n.key == 100);
    for n in &mut notes {
        if n.instrument == 3 {
            n.key = 88;
        }
    }
    chip::render_opl(&Chip::CLEAN, &patches, &notes, RATE, &mut out);
    let path = dir.join("opl.wav");
    write_wav(&path, &out)?;
    println!("{}", path.display());
    Ok(())
}

/// 16-bit mono WAV.
fn write_wav(path: &std::path::Path, samples: &[f32]) -> std::io::Result<()> {
    let data = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 1 | 1 << 16, RATE, RATE * 2, 2 | 16 << 16] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, b)
}
