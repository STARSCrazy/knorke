//! The yard's files (ADR 0023): the scene names ids (`knorke.stealth.assets/1`), this loads
//! them once at start from `knorke-integration/assets`. A missing or misspelt file stops the
//! game with its diagnostic instead of running on without it.

use std::path::Path;

use knorke_assets::{Folder, Image, Sound};
use knorke_integration::stealth::Assets;
use knorke_scene::Scene;
use knorke_types::{Diagnostic, DiagnosticCode};

/// The scene names no assets.
pub const NO_ASSETS: DiagnosticCode = DiagnosticCode::new("knorke.stealth.assets.none");

/// The yard's asset folder in this checkout.
pub fn folder() -> Folder {
    Folder::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("../knorke-integration/assets"))
}

/// Everything the yard shows and plays, decoded.
pub struct Loaded {
    /// Light floor, dark floor, wall, guard, intruder.
    pub images: [Image; 5],
    /// Wind, step, alarm, catch.
    pub sounds: [Sound; 4],
}

/// Loads every asset the scene names from `folder`.
pub fn load(folder: &Folder, scene: &Scene) -> Result<Loaded, Diagnostic> {
    let Some((_, a)) = scene.each::<Assets>().next() else {
        return Err(Diagnostic::error(NO_ASSETS, "the scene names no assets"));
    };
    let image = |id: &String| folder.image(id);
    let sound = |id: &String| folder.sound(id);
    Ok(Loaded {
        images: [
            image(&a.floor[0])?,
            image(&a.floor[1])?,
            image(&a.wall)?,
            image(&a.guard)?,
            image(&a.intruder)?,
        ],
        sounds: [
            sound(&a.wind)?,
            sound(&a.step)?,
            sound(&a.alarm)?,
            sound(&a.catch)?,
        ],
    })
}
