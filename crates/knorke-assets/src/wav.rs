//! Own WAV reader (ADR 0023, decided question 1): PCM is simple, so no extra dependency.

/// Decoded sound, shaped like `knorke_audio_sdl::Clip`.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    /// Samples in `-1..=1`, interleaved when `stereo`.
    pub samples: Vec<f32>,
    /// Sample rate in Hz.
    pub rate: u32,
    /// Two interleaved channels instead of one.
    pub stereo: bool,
}

const PCM: u16 = 1;
const FLOAT: u16 = 3;
const EXTENSIBLE: u16 = 0xFFFE;

/// Decodes RIFF/WAVE with PCM 8/16/24 bit or float 32, mono or stereo.
pub(crate) fn decode(bytes: &[u8]) -> Result<Sound, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("WAV: not a RIFF/WAVE file".into());
    }
    let mut fmt = None;
    let mut data = None;
    let mut rest = &bytes[12..];
    while rest.len() >= 8 {
        let len = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        let body = rest.get(8..8 + len).ok_or("WAV: chunk runs past the end")?;
        match &rest[0..4] {
            b"fmt " => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        // Chunks are padded to even length.
        rest = rest.get(8 + len + (len & 1)..).unwrap_or(&[]);
    }
    let fmt = fmt.filter(|f| f.len() >= 16).ok_or("WAV: no fmt chunk")?;
    let data = data.ok_or("WAV: no data chunk")?;
    let u16_at = |i: usize| u16::from_le_bytes([fmt[i], fmt[i + 1]]);
    let mut tag = u16_at(0);
    if tag == EXTENSIBLE {
        // The sub-format GUID starts at byte 24; its first two bytes are the real tag.
        tag = if fmt.len() >= 26 { u16_at(24) } else { 0 };
    }
    let channels = u16_at(2);
    let rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
    let bits = u16_at(14);
    if channels != 1 && channels != 2 {
        return Err(format!(
            "WAV: {channels} channels, only mono and stereo are supported"
        ));
    }
    let samples = match (tag, bits) {
        (PCM, 8) => data
            .iter()
            .map(|&b| (f32::from(b) - 128.0) / 128.0)
            .collect(),
        (PCM, 16) => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0)
            .collect(),
        // Shift into the top of an i32 so the sign extends.
        (PCM, 24) => data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        (FLOAT, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
        _ => {
            return Err(format!(
                "WAV: format {tag} with {bits} bits is not supported"
            ));
        }
    };
    Ok(Sound {
        samples,
        rate,
        stereo: channels == 2,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a WAV file; tests elsewhere in the crate reuse it.
    pub(crate) fn wav(tag: u16, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend(tag.to_le_bytes());
        fmt.extend(channels.to_le_bytes());
        fmt.extend(rate.to_le_bytes());
        let block = channels * bits / 8;
        fmt.extend((rate * u32::from(block)).to_le_bytes());
        fmt.extend(block.to_le_bytes());
        fmt.extend(bits.to_le_bytes());
        let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
        // An odd-length unknown chunk checks padding.
        for (id, body) in [(b"LIST", &b"x"[..]), (b"fmt ", &fmt), (b"data", data)] {
            out.extend(id);
            out.extend((body.len() as u32).to_le_bytes());
            out.extend(body);
            if body.len() % 2 == 1 {
                out.push(0);
            }
        }
        let riff = (out.len() - 8) as u32;
        out[4..8].copy_from_slice(&riff.to_le_bytes());
        out
    }

    #[test]
    fn pcm16_mono() {
        let data: Vec<u8> = [0i16, 16_384, -32_768]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let s = decode(&wav(PCM, 1, 22_050, 16, &data)).unwrap();
        assert_eq!(
            s,
            Sound {
                samples: vec![0.0, 0.5, -1.0],
                rate: 22_050,
                stereo: false
            }
        );
    }

    #[test]
    fn float_stereo() {
        let data: Vec<u8> = [0.25f32, -0.75]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let s = decode(&wav(FLOAT, 2, 48_000, 32, &data)).unwrap();
        assert_eq!(
            s,
            Sound {
                samples: vec![0.25, -0.75],
                rate: 48_000,
                stereo: true
            }
        );
    }

    #[test]
    fn pcm8_and_pcm24() {
        let s = decode(&wav(PCM, 1, 8_000, 8, &[128, 0, 192])).unwrap();
        assert_eq!(s.samples, vec![0.0, -1.0, 0.5]);
        // -8388608 (0x800000) and 4194304 (0x400000).
        let s = decode(&wav(PCM, 1, 8_000, 24, &[0, 0, 0x80, 0, 0, 0x40])).unwrap();
        assert_eq!(s.samples, vec![-1.0, 0.5]);
    }

    #[test]
    fn unsupported_and_broken() {
        // 32-bit integer PCM, ADPCM (tag 2), five channels, truncated file.
        assert!(decode(&wav(PCM, 1, 8_000, 32, &[0; 4])).is_err());
        assert!(decode(&wav(2, 1, 8_000, 4, &[0; 4])).is_err());
        assert!(decode(&wav(PCM, 5, 8_000, 16, &[0; 10])).is_err());
        let full = wav(PCM, 1, 8_000, 16, &[0; 4]);
        assert!(decode(&full[..full.len() - 2]).is_err());
        assert!(decode(b"not a wav").is_err());
    }
}
