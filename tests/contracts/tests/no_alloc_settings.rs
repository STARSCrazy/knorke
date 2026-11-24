//! Contract: validating and replacing the settings of the 2D light and the post-processing, and
//! evaluating the frame warnings (also with every debug check on), allocate nothing in the
//! steady state (ADR 0011, ADR 0016). Own test binary with a single test, so the process-wide
//! allocation counter is not disturbed by other tests.

#![allow(unsafe_code)] // counting allocator

use knorke_light2d::Projection;
use knorke_light2d::frame_warnings::{
    DebugChecks, FrameFindings, FrameWarnings, Warning, WarningSet,
};
use knorke_light2d::settings::{LightSettings, Quality};
use knorke_post::settings::{CrtPreset, CrtSettings, PostSettings};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

struct Counting;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn settings_and_frame_warnings_allocate_nothing() {
    let mut light = LightSettings::default();
    let mut post = PostSettings::default();
    let mut warnings = FrameWarnings::new();
    let projection = Projection::IDENTITY;
    let mut findings = FrameFindings {
        checks: DebugChecks::ALL,
        gpu_findings: WarningSet::of(Warning::HalfAlpha),
        flat_lights: 2,
        unreachable_lights: 1,
        thin_obstacles: 3,
        ..FrameFindings::default()
    };
    let presets = [Quality::Exact, Quality::High, Quality::Medium, Quality::Low];
    let mut step = |i: u32| {
        let mut l = LightSettings::default();
        l.quality.quality = presets[(i % 4) as usize];
        l.debug_checks = DebugChecks::ALL;
        assert!(light.replace(l).is_ok());
        let _ = (light.resolutions(), light.quality.quality.medium_cost());
        let _ = (
            light.haze_falloff(&projection),
            light.wet_ripple_size(&projection),
            light.resolved_medium_height_step(),
        );
        let p = PostSettings {
            crt: CrtSettings::preset(CrtPreset::Arcade),
            ..PostSettings::default()
        };
        assert!(post.replace(p).is_ok());
        findings.checks = if i.is_multiple_of(2) {
            DebugChecks::ALL
        } else {
            DebugChecks::NONE
        };
        let w = warnings.update(&findings);
        let _ = (
            w.len(),
            warnings.newly_reported(),
            warnings.flat_lights_hint(),
        );
        warnings.add_pending(WarningSet::of(Warning::StaleLayers));
    };
    for i in 0..4 {
        step(i);
    }
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    for i in 4..100 {
        step(i);
    }
    let allocated = ALLOCATIONS.load(Ordering::Relaxed) - before;
    assert_eq!(allocated, 0, "allocations in 96 steady-state steps");
}
