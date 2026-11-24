//! Contract: building the uniform blocks of the bloom and output shaders of `knorke-post` (P20)
//! allocates nothing: the blocks are fixed-size byte arrays written field by field
//! (ADR 0006, ADR 0011). The GPU passes themselves are not part of it (they need a device).
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests.

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_math::Vec2;
use knorke_post::bloom::plan_bloom;
use knorke_post::output_flags::{crt_flags, grade_flags, shockwave_flags};
use knorke_post::params::{
    BloomInput, OutputFrame, OverlayBlend, down_params, output_params, prefilter_params, up_params,
};
use knorke_post::settings::{CrtPreset, CrtSettings, Filter, PostSettings, Tonemap};
use knorke_post::shockwave::{
    Projection, Shockwave, ShockwaveFrame, ShockwaveList, collect_shockwaves,
};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn post_parameter_blocks_allocate_nothing() {
    let projection = Projection {
        origin: Vec2::new(0.0, 0.0),
        x_axis: Vec2::new(1.0, 0.0),
        y_axis: Vec2::new(0.0, 1.0),
        z_axis: Vec2::new(0.0, 0.0),
    };
    let mut list = ShockwaveList::default();
    list.set(&[Shockwave {
        x: 96.0,
        y: 64.0,
        radius: 30.0,
        width: 24.0,
        strength: 5.0,
        ..Shockwave::default()
    }])
    .unwrap();
    let mut waves = ShockwaveFrame::default();
    let mut settings = PostSettings::default();
    settings.bloom.enabled = true;
    settings.output.tonemap = Tonemap::Aces;
    settings.crt = CrtSettings::preset(CrtPreset::Arcade);

    let mut sum = 0u64;
    let mut step = |i: u32| {
        settings.output.filter = if i.is_multiple_of(2) {
            Filter::Nearest
        } else {
            Filter::Linear
        };
        collect_shockwaves(&list, &projection, 192, 128, &mut waves);
        let plan = plan_bloom(settings.bloom.radius, 192, 128);
        let bloom = BloomInput::new(settings.bloom.intensity, plan, (96, 64));
        let frame = OutputFrame {
            scene: (192, 128),
            target: (400, 300),
            encode_srgb: true,
            dither_target: true,
            overlay: Some(OverlayBlend::Srgb),
            bloom: Some(bloom),
            shockwaves: Some((&projection, &waves)),
        };
        let out = output_params(&settings, &frame);
        let pre = prefilter_params((192, 128), (96, 64), 0.5, &settings.bloom);
        let down = down_params((96, 64), (48, 32));
        let up = up_params((48, 32), (96, 64), 0.5);
        let flags = grade_flags(&settings.output)
            | crt_flags(&settings.crt, 1.0, true)
            | shockwave_flags(1, settings.output.filter);
        sum += u64::from(out[28])
            + u64::from(pre[0])
            + u64::from(down[0])
            + u64::from(up[0])
            + u64::from(flags);
    };
    step(0); // warm up
    let before = allocations();
    for i in 0..1000 {
        step(i);
    }
    let after = allocations();
    assert_eq!(after - before, 0, "the post parameter blocks allocated");
    assert!(sum > 0);
}
