//! ADR 0023 step 4: the yard's scene names its pictures and sounds by id, and the files in
//! `knorke-integration/assets` match. A renamed file stops the load with
//! `knorke.assets.missing` instead of the game running on without it.

#[allow(dead_code, unreachable_pub)]
#[path = "../examples/stealth_yard/assets.rs"]
mod assets;

use knorke_assets::{Folder, MISSING};
use knorke_scene::Scene;

const YARD: &str = include_str!("../../knorke-integration/scenes/guard_yard.json");

#[test]
fn every_named_asset_loads() {
    let scene = Scene::from_text(YARD).unwrap();
    let files = assets::load(&assets::folder(), &scene).unwrap();
    for img in &files.images {
        assert_eq!((img.width, img.height), (16, 16));
    }
    for s in &files.sounds {
        assert_eq!((s.rate, s.stereo), (48_000, false));
        assert!(!s.samples.is_empty());
    }
}

#[test]
fn renamed_asset_is_missing() {
    // A copy of the folder with one sound renamed.
    let root = std::env::temp_dir().join(format!("knorke-yard-assets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let src = assets::folder();
    for dir in ["tiles", "sprites", "sounds"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
        for entry in std::fs::read_dir(src.root().join(dir)).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), root.join(dir).join(entry.file_name())).unwrap();
        }
    }
    std::fs::rename(root.join("sounds/step.wav"), root.join("sounds/steps.wav")).unwrap();
    let scene = Scene::from_text(YARD).unwrap();
    let err = assets::load(&Folder::new(&root), &scene).err().unwrap();
    assert_eq!(err.code, MISSING);
    assert!(err.message.contains("sounds/step"), "{}", err.message);
}
