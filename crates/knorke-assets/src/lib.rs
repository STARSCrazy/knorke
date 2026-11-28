//! Small asset system (ADR 0023): images and sounds by id from a folder.
//!
//! An [`AssetId`] is a relative path in one fixed spelling (`sprites/guard`, no extension).
//! [`Folder`] finds the file with exactly that spelling on every system, so a file that
//! differs only in case fails on Windows too instead of only on Linux. Loading is
//! synchronous and happens outside the simulation; nothing here knows GPU, audio device or
//! simulation state.

mod id;
mod image;
mod wav;

use knorke_types::{Diagnostic, DiagnosticCode};
use std::path::{Path, PathBuf};

pub use id::AssetId;
pub use image::Image;
pub use wav::Sound;

/// No file belongs to the id.
pub const MISSING: DiagnosticCode = DiagnosticCode::new("knorke.assets.missing");
/// The id breaks the spelling rules.
pub const BAD_ID: DiagnosticCode = DiagnosticCode::new("knorke.assets.bad_id");
/// The file is broken or its format variant is not supported.
pub const DECODE: DiagnosticCode = DiagnosticCode::new("knorke.assets.decode");
/// The file exists only in another spelling; the message names the found name.
pub const CASE: DiagnosticCode = DiagnosticCode::new("knorke.assets.case");

/// A folder of assets, looked up by id.
#[derive(Clone, Debug)]
pub struct Folder {
    root: PathBuf,
}

impl Folder {
    /// Uses `root` as the asset folder. Nothing is read until an asset is requested.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The asset folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Loads `<id>.png` as RGBA8 (sRGB bytes).
    pub fn image(&self, id: &str) -> Result<Image, Diagnostic> {
        let (id, bytes) = self.read(id, "png")?;
        image::decode(&bytes).map_err(|e| decode_error(&id, &e))
    }

    /// Loads `<id>.wav` as `f32` samples.
    pub fn sound(&self, id: &str) -> Result<Sound, Diagnostic> {
        let (id, bytes) = self.read(id, "wav")?;
        wav::decode(&bytes).map_err(|e| decode_error(&id, &e))
    }

    /// Path of the file for `id` with extension `ext`, checked for exact spelling.
    pub fn locate(&self, id: &AssetId, ext: &str) -> Result<PathBuf, Diagnostic> {
        let file = format!("{}.{ext}", id.as_str());
        let mut path = self.root.clone();
        // Walk segment by segment: `Path::exists` would accept any case on Windows.
        for segment in file.split('/') {
            let entries = std::fs::read_dir(&path).map_err(|_| missing(id, &file))?;
            let mut other = None;
            let mut exact = false;
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if name == segment {
                    exact = true;
                    break;
                }
                if name.eq_ignore_ascii_case(segment) {
                    other = Some(name.to_owned());
                }
            }
            if !exact {
                return Err(match other {
                    Some(found) => Diagnostic::error(
                        CASE,
                        format!(
                            "asset `{id}`: found `{found}` instead of `{segment}`; file names must match the id exactly"
                        ),
                    ),
                    None => missing(id, &file),
                });
            }
            path.push(segment);
        }
        if path.is_file() {
            Ok(path)
        } else {
            Err(missing(id, &file))
        }
    }

    fn read(&self, id: &str, ext: &str) -> Result<(AssetId, Vec<u8>), Diagnostic> {
        let id = AssetId::parse(id)?;
        let path = self.locate(&id, ext)?;
        let bytes = std::fs::read(&path).map_err(|e| {
            Diagnostic::error(
                MISSING,
                format!("asset `{id}`: cannot read {}: {e}", path.display()),
            )
        })?;
        Ok((id, bytes))
    }
}

fn missing(id: &AssetId, file: &str) -> Diagnostic {
    Diagnostic::error(MISSING, format!("asset `{id}`: no file `{file}`"))
}

fn decode_error(id: &AssetId, reason: &str) -> Diagnostic {
    Diagnostic::error(DECODE, format!("asset `{id}`: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh folder per test under the system temp dir.
    fn folder(name: &str) -> Folder {
        let root =
            std::env::temp_dir().join(format!("knorke-assets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sprites")).unwrap();
        Folder::new(root)
    }

    fn write(f: &Folder, rel: &str, bytes: &[u8]) {
        std::fs::write(f.root().join(rel), bytes).unwrap();
    }

    fn png(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    #[test]
    fn image_size_and_pixels() {
        let f = folder("image");
        write(
            &f,
            "sprites/guard.png",
            &png(2, 1, png::ColorType::Rgba, &[1, 2, 3, 4, 5, 6, 7, 8]),
        );
        write(
            &f,
            "sprites/gray.png",
            &png(1, 1, png::ColorType::Grayscale, &[9]),
        );
        let img = f.image("sprites/guard").unwrap();
        assert_eq!(
            (img.width, img.height, img.rgba),
            (2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8])
        );
        assert_eq!(f.image("sprites/gray").unwrap().rgba, vec![9, 9, 9, 255]);
    }

    #[test]
    fn broken_png_is_decode() {
        let f = folder("broken");
        write(&f, "sprites/bad.png", b"\x89PNG not really");
        assert_eq!(f.image("sprites/bad").unwrap_err().code, DECODE);
    }

    #[test]
    fn sound_by_id() {
        let f = folder("sound");
        write(&f, "step.wav", &wav::tests::wav(1, 1, 8_000, 8, &[128]));
        let s = f.sound("step").unwrap();
        assert_eq!((s.samples, s.rate, s.stereo), (vec![0.0], 8_000, false));
        write(&f, "odd.wav", &wav::tests::wav(1, 1, 8_000, 32, &[0; 4]));
        assert_eq!(f.sound("odd").unwrap_err().code, DECODE);
    }

    #[test]
    fn missing_bad_id_and_case() {
        let f = folder("lookup");
        write(
            &f,
            "sprites/Guard.png",
            &png(1, 1, png::ColorType::Grayscale, &[0]),
        );
        std::fs::create_dir_all(f.root().join("Sounds")).unwrap();
        write(
            &f,
            "Sounds/step.wav",
            &wav::tests::wav(1, 1, 8_000, 8, &[128]),
        );

        assert_eq!(f.image("sprites/wall").unwrap_err().code, MISSING);
        assert_eq!(f.image("nothing/wall").unwrap_err().code, MISSING);
        assert_eq!(f.image("Sprites/guard").unwrap_err().code, BAD_ID);
        // Wrong case in the file name and in a folder name; also on Windows.
        let e = f.image("sprites/guard").unwrap_err();
        assert_eq!(e.code, CASE);
        assert!(e.message.contains("Guard.png"), "{}", e.message);
        assert_eq!(f.sound("sounds/step").unwrap_err().code, CASE);
        // A directory is not an asset.
        assert_eq!(
            f.locate(&AssetId::parse("sprites").unwrap(), "")
                .unwrap_err()
                .code,
            MISSING
        );
    }
}
