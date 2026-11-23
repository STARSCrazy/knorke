//! The Chiaro golden scenes are complete and consistent with their manifest (package P3).
//!
//! No PNG decoder is linked (no consumer yet); the check reads each file's signature and
//! IHDR and compares the size with `manifest.toml`. Pixel comparison against the Rust
//! renderer comes with the gate spike, using `knorke_contract_tests::parity`.

use std::fs;
use std::path::PathBuf;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../golden/chiaro")
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}

#[test]
fn every_png_has_a_manifest_entry_with_matching_resolution() {
    let dir = golden_dir();
    let manifest = fs::read_to_string(dir.join("manifest.toml")).expect("manifest.toml");
    let mut scenes = 0;
    let mut total = 0;
    for entry in fs::read_dir(&dir).expect("golden dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let bytes = fs::read(&path).expect("png");
        total += bytes.len();
        let (w, h) = png_size(&bytes).unwrap_or_else(|| panic!("{path:?} is not a PNG"));
        let stem = path.file_stem().and_then(|s| s.to_str()).expect("name");
        let block = manifest
            .split("[[scene]]")
            .find(|b| b.contains(&format!("name = \"{stem}\"")))
            .unwrap_or_else(|| panic!("{stem} missing in manifest"));
        assert!(
            block.contains(&format!("resolution = [{w}, {h}]")),
            "{stem}: manifest resolution differs from {w}x{h}"
        );
        scenes += 1;
    }
    assert_eq!(manifest.matches("[[scene]]").count(), scenes, "scene count");
    assert!(scenes >= 8, "need at least 8 golden scenes");
    assert!(total < 4 << 20, "golden images grew to {total} bytes");
}
