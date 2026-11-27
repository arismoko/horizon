//! Sunrise's HDR extension served to a `shaders/` pack through the game's own loader
//! (`shader_pack_inputs`) and frames (#5827 step 2), on a real offscreen renderer. Each case runs
//! in a process of its own (one event loop, its own resources and environment).
use super::native_entity_refs::tests::with_client_game;
use super::native_shaders::outcome;
use crate::game::native_hooks::particle_count_tests::frame_scene;
use sunrise_client::gfx::ScreenshotRequest;
use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases_with_env;
use sunrise_shaders::Tonemap;

pub(super) const CASE: &str = "SUNRISE_SHADER_PACK_HDR_CASE";
/// Sunrise's HDR under its switch, with HDR10 output offscreen.
pub(super) const SERVED: [(&str, &str); 2] =
    [("SUNRISE_HDR_FRAME", "float"), ("SUNRISE_HDR_OUTPUT", "pq")];
const VERTEX: &str = "#version 120\nvarying vec2 uv;\nvoid main() { gl_Position = ftransform(); uv = gl_MultiTexCoord0.xy; }\n";

/// A pack folder in `root` named `name` with `files` under `shaders/` (each `.fsh` also gets
/// `VERTEX` as its `.vsh`).
pub(super) fn pack(
    root: &std::path::Path,
    name: &str,
    files: &[(&str, &str)],
) -> std::path::PathBuf {
    let shaders = root.join(name).join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (file, text) in files {
        std::fs::write(shaders.join(file), text).unwrap();
        if let Some(program) = file.strip_suffix(".fsh") {
            std::fs::write(shaders.join(format!("{program}.vsh")), VERTEX).unwrap();
        }
    }
    root.join(name)
}

/// The game's frames up to the loader, as `render_with_hooks` runs them: the frame follows the
/// pick, then the loader runs.
pub(super) fn loader_frame(game: &mut crate::game::ClientGame<'_>) {
    let sky = sunrise_level::environment::Sky::default();
    let camera = glam::DVec3::new(0.5, 65.5, 0.5);
    game.follow_frame();
    game.shader_pack_inputs(camera, camera, 0.0, &sky);
}

/// Frames until the pick runs, installed for `wanted` (its dimension and served answer), with
/// nothing building: a finished build counts only once a frame has taken and installed it.
/// At most 60 s.
pub(super) fn settle(game: &mut crate::game::ClientGame<'_>, wanted: (i32, Option<Tonemap>)) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        loader_frame(game);
        let renderer = game.client.renderer.0.as_ref().unwrap();
        if !renderer.shader_pack_building()
            && renderer.shader_pack_kind() == Some("iris")
            && renderer.shader_pack_loaded_for() == Some(wanted)
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the pack never settled"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Waits for the build the loader started, without a frame (so without taking its result).
fn built(game: &crate::game::ClientGame<'_>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while game
        .client
        .renderer
        .0
        .as_ref()
        .unwrap()
        .shader_pack_building()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the build never ended"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// `composite` writes colortex0, which is an integer buffer when `refused_under` is defined:
/// with no `final` to draw it, that build is refused (`PackPost::new`); otherwise a float one
/// the colortex0 copy shows.
fn colortex0_pack(refused_under: &str) -> String {
    format!(
        "#version 330\n/* RENDERTARGETS:0 */\n#ifdef {refused_under}\n\
         /*\nconst int colortex0Format = RGBA32UI;\n*/\nout uvec4 c;\nvoid main() {{ c = uvec4(1u); }}\n\
         #else\nout vec4 c;\nvoid main() {{ c = vec4(0.5); }}\n#endif\n"
    )
}

/// A served answer that changes for the same pick and dimension while the build for the old
/// one, which ends refused, waits to be taken: the loader drops that build before it looks at
/// any result, loads the pick once for the new answer, and it keeps running. The stale refusal
/// is never counted: the pick stays picked and isn't remembered as refused.
#[test]
#[ignore = "daily"]
fn a_served_answer_change_drops_the_stale_build_and_loads_once() {
    const NAME: &str = "game::client_side::shader_pack_hdr_tests::a_served_answer_change_drops_the_stale_build_and_loads_once";
    if run_fresh_cases_with_env(NAME, CASE, &["on"], 1, &SERVED) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let path = pack(
        scratch.path(),
        "served",
        &[
            (
                "shaders.properties",
                "sunrise.hdr = true\nsunrise.hdr.tonemap = pack\n",
            ),
            ("composite.fsh", &colortex0_pack("SUNRISE_HDR_TONEMAP_PACK")),
        ],
    );
    let properties = path.join("shaders/shaders.properties");
    with_client_game(|game| {
        game.client.settings.sunrise.hdr = sunrise_client::graphics::HdrMode::On;
        game.client.settings.sunrise.shader_pack = path.to_str().unwrap().to_owned();
        // The material maps already on: no resource reload, whose overlay would cover the frame.
        sunrise_client::sprites::set_labpbr_maps(true);
        let _event_loop = frame_scene(game);
        // The first frames start the build for the pack's curve.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while game
            .client
            .renderer
            .0
            .as_ref()
            .unwrap()
            .shader_pack_loaded_for()
            != Some((0, Some(Tonemap::Pack)))
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the pack never loaded"
            );
            loader_frame(game);
        }
        built(game);
        let loads = game
            .client
            .renderer
            .0
            .as_ref()
            .unwrap()
            .test_hooks
            .pack_loads;
        // The pick now asks for Sunrise's curve; a scan of the packs reads it again.
        std::fs::write(
            &properties,
            "sunrise.hdr = true\nsunrise.hdr.tonemap = sunrise\n",
        )
        .unwrap();
        game.client.native_shaders.pick_hdr.get_mut().forget();
        settle(game, (0, Some(Tonemap::Sunrise)));
        let renderer = game.client.renderer.0.as_ref().unwrap();
        assert_eq!(renderer.pack_hdr, Some(Tonemap::Sunrise));
        assert_eq!(
            renderer.test_hooks.pack_loads,
            loads + 1,
            "one replacement build"
        );
        assert_eq!(
            renderer.shader_pack_loaded_for(),
            Some((0, Some(Tonemap::Sunrise)))
        );
        assert_eq!(renderer.shader_pack_kind(), Some("iris"), "the pack runs");
        assert_eq!(
            game.client.settings.sunrise.shader_pack,
            path.to_str().unwrap()
        );
        assert_ne!(outcome(&path), Some(false), "not remembered as refused");
        // Later frames leave it running, with no other build.
        for _ in 0..5 {
            loader_frame(game);
        }
        let renderer = game.client.renderer.0.as_ref().unwrap();
        assert_eq!(renderer.test_hooks.pack_loads, loads + 1);
        assert_eq!(renderer.shader_pack_kind(), Some("iris"));
    });
}

/// Not served (no switch): a build for the overworld that ends refused, still waiting to be
/// taken when the player changes dimension, is taken as before the extension: its refusal turns
/// the pick off and is remembered, and the loader goes on to start the new dimension's build.
#[test]
#[ignore = "daily"]
fn an_unserved_build_is_still_taken_across_a_dimension_change() {
    const NAME: &str = "game::client_side::shader_pack_hdr_tests::an_unserved_build_is_still_taken_across_a_dimension_change";
    // The switch off in the child's own environment, whatever the parent's.
    if run_fresh_cases_with_env(NAME, CASE, &["off"], 1, &[("SUNRISE_HDR_FRAME", "off")]) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    // Refused whatever the macros: `MC_VERSION` is always defined.
    let path = pack(
        scratch.path(),
        "refused",
        &[("composite.fsh", &colortex0_pack("MC_VERSION"))],
    );
    with_client_game(|game| {
        game.client.settings.sunrise.shader_pack = path.to_str().unwrap().to_owned();
        // The material maps already on: no resource reload, whose overlay would cover the frame.
        sunrise_client::sprites::set_labpbr_maps(true);
        let _event_loop = frame_scene(game);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while game
            .client
            .renderer
            .0
            .as_ref()
            .unwrap()
            .shader_pack_loaded_for()
            != Some((0, None))
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the pack never loaded"
            );
            loader_frame(game);
        }
        built(game);
        let loads = game
            .client
            .renderer
            .0
            .as_ref()
            .unwrap()
            .test_hooks
            .pack_loads;
        game.common.dim = super::Dim::Nether;
        loader_frame(game);
        assert_eq!(outcome(&path), Some(false), "the refusal is remembered");
        assert!(
            game.client.settings.sunrise.shader_pack.is_empty(),
            "the pick is turned off"
        );
        // Then, in the same frame, the loader starts the Nether's build, as it always has.
        assert_eq!(
            game.client
                .renderer
                .0
                .as_ref()
                .unwrap()
                .test_hooks
                .pack_loads,
            loads + 1
        );
    });
}

/// A frame kept for another pick (as a failed rebuild leaves it: `Renderer::pack_hdr` still
/// `Some(Pack)`, made for another pick) serves nothing to a pick that asks for the pack's curve:
/// the loader loads it unserved, and its load says the read's `Some(Pack)` is served `None`, not
/// the kept frame's answer.
#[test]
#[ignore = "daily"]
fn a_load_under_a_kept_frame_names_the_served_answer() {
    const NAME: &str = "game::client_side::shader_pack_hdr_tests::a_load_under_a_kept_frame_names_the_served_answer";
    if run_fresh_cases_with_env(NAME, CASE, &["on"], 1, &SERVED) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let path = pack(
        scratch.path(),
        "kept",
        &[
            ("shaders.properties", "sunrise.hdr = true\n"),
            (
                "final.fsh",
                "#version 120\nvoid main() { gl_FragColor = vec4(1.0); }\n",
            ),
        ],
    );
    with_client_game(|game| {
        game.client.settings.sunrise.hdr = sunrise_client::graphics::HdrMode::On;
        game.client.settings.sunrise.shader_pack = path.to_str().unwrap().to_owned();
        sunrise_client::sprites::set_labpbr_maps(true);
        let _event_loop = frame_scene(game);
        let renderer = game.client.renderer.0.as_mut().unwrap();
        assert_eq!(renderer.pack_hdr, Some(Tonemap::Pack));
        "another pick".clone_into(&mut renderer.pack_hdr_pick);
        // The loader alone: `follow_frame` would make the frame again for this pick.
        let sky = sunrise_level::environment::Sky::default();
        let camera = glam::DVec3::new(0.5, 65.5, 0.5);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while game
            .client
            .renderer
            .0
            .as_ref()
            .unwrap()
            .shader_pack_loaded_for()
            != Some((0, None))
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the pack never loaded"
            );
            game.shader_pack_inputs(camera, camera, 0.0, &sky);
        }
        assert_eq!(
            game.client
                .renderer
                .0
                .as_ref()
                .unwrap()
                .test_hooks
                .loaded_log,
            [
                "shaders.properties asks for Sunrise's HDR as Some(Pack) with the renderer's \
                 macros, but it is served None: the served answer counts"
            ]
        );
    });
}

/// The HDR10 copy's red code at a pixel left of the hotbar and the crosshair, and whether the
/// frame ran Sunrise's tone curve.
fn drawn(game: &mut crate::game::ClientGame<'_>) -> (u16, bool) {
    game.client.renderer.0.as_mut().unwrap().screenshot = Some(ScreenshotRequest {
        path: "unused.png".into(),
        size: None,
        scaled: false,
        background: false,
    });
    game.render_with(|renderer, frame| renderer.render(frame, None))
        .unwrap();
    let renderer = game.client.renderer.0.as_mut().unwrap();
    let curve = renderer.test_hooks.tone_curve_ran;
    let pending = renderer.screenshot_result.take().unwrap().unwrap();
    let ([width, _], pixels) = pending
        .hdr_pixels()
        .expect("an HDR10 copy: the layered frame");
    let at = (40 * width as usize + 20) * 6;
    (u16::from_be_bytes([pixels[at], pixels[at + 1]]), curve)
}

/// ST 2084's code for `nits`, in 10 bits, as the PNG's 16 bits repeat it.
pub(super) fn pq16(nits: f64) -> u16 {
    let code = (pq_nits_inverse(nits) * 1023.0).round() as u16;
    code << 6 | code >> 4
}

/// ST 2084's inverse: nits for the signal `e`.
fn pq_nits(e: f64) -> f64 {
    let p = e.clamp(0.0, 1.0).powf(1.0 / 78.84375);
    let y = (p - 0.8359375).max(0.0) / (18.8515625 - 18.6875 * p);
    10000.0 * y.powf(1.0 / 0.1593017578125)
}

/// Grey light `light` (× paper white 203 nits, on a 1000-nit display) after Sunrise's tone curve
/// runs `passes` times on the float frame and the last pass holds it at the peak, worked out
/// apart from the shader, in f64: ITU-R BT.2390's EETF in PQ, its source following the display
/// where Sunrise lit nothing (a source of 3 × the peak's signal less 2 × the knee's, the knee at
/// 60% of the headroom and at least white), the frame storing each pass's result in vanilla's
/// gamma as a half float. 5.0's own gamma, 1064/512, lies under half a half-float step above
/// 1064, so either rounding stores it alike.
pub(super) fn curved(light: f64, passes: u32) -> f64 {
    let (paper, headroom): (f64, f64) = (203.0, 1000.0 / 203.0);
    let signal = |light: f64| pq_nits_inverse(light * paper);
    let knee = (0.6 * headroom).max(1.0);
    // The nearest half float (10 bits of mantissa) to a normal `x`.
    let half = |x: f64| {
        let step = 2f64.powi(x.log2().floor() as i32 - 10);
        (x / step).round() * step
    };
    let mut stored = half(light.powf(1.0 / 2.2));
    for _ in 0..passes {
        let top = stored.powf(2.2);
        if top <= knee {
            break;
        }
        let peak = signal(headroom);
        let source = (3.0 * peak - 2.0 * signal(knee)).min(1.0);
        let e = signal(top) / source;
        let max_lum = peak / source;
        let ks = (1.5 * max_lum - 0.5).max(0.0);
        let scale = if e >= 1.0 {
            headroom / top
        } else {
            let t = (e - ks) / (1.0 - ks);
            let (t2, t3) = (t * t, t * t * t);
            let p = (2.0 * t3 - 3.0 * t2 + 1.0) * ks
                + (t3 - 2.0 * t2 + t) * (1.0 - ks)
                + (3.0 * t2 - 2.0 * t3) * max_lum;
            pq_nits(p * source) / paper / top
        };
        stored = half(stored * scale.powf(1.0 / 2.2));
    }
    stored.powf(2.2).min(headroom)
}

/// ST 2084's signal for `nits`.
fn pq_nits_inverse(nits: f64) -> f64 {
    let y = (nits / 10000.0).clamp(0.0, 1.0).powf(0.1593017578125);
    ((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y)).powf(78.84375)
}

/// The tone curve through real frames on the layered frame (HDR10 offscreen, paper white 203,
/// peak 1000): a pack served with its own curve writes 5.0, past the curve's knee, and the frame
/// shows it held at the peak with no curve; the same pack with Sunrise's curve runs it, and
/// shows less; a frame whose world was drawn without the pack's passes (no scaled target) runs
/// the curve though the installed pack owns it.
#[test]
#[ignore = "daily"]
fn the_tone_curve_follows_the_pack_that_drew_the_frame() {
    const NAME: &str = "game::client_side::shader_pack_hdr_tests::the_tone_curve_follows_the_pack_that_drew_the_frame";
    if run_fresh_cases_with_env(NAME, CASE, &["on"], 1, &SERVED) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let final_fsh = "#version 120\nvoid main() {\n#ifdef SUNRISE_HDR\n    gl_FragColor = vec4(5.0, 5.0, 5.0, 1.0);\n\
                     #else\n    gl_FragColor = vec4(1.0);\n#endif\n}\n";
    // No vignette: it eases in over frames and darkens the edges after the curve.
    let owned = |tonemap: &str| {
        format!("vignette = false\nsunrise.hdr = true\nsunrise.hdr.tonemap = {tonemap}\n")
    };
    let path = pack(
        scratch.path(),
        "curve",
        &[
            ("shaders.properties", &owned("pack")),
            ("final.fsh", final_fsh),
        ],
    );
    let properties = path.join("shaders/shaders.properties");
    with_client_game(|game| {
        game.client.settings.sunrise.hdr = sunrise_client::graphics::HdrMode::On;
        game.client.settings.sunrise.hdr_paper_white = 203;
        game.client.settings.sunrise.hdr_peak = 1000;
        game.client.settings.sunrise.shader_pack = path.to_str().unwrap().to_owned();
        // The material maps already on: no resource reload, whose overlay would cover the frame.
        sunrise_client::sprites::set_labpbr_maps(true);
        let _event_loop = frame_scene(game);
        // HDR output on the renderer, as the game's start applies its settings.
        game.apply_ui_settings();
        let owner = |game: &mut crate::game::ClientGame<'_>, tonemap: &str, answer| {
            std::fs::write(&properties, owned(tonemap)).unwrap();
            game.client.native_shaders.pick_hdr.get_mut().forget();
            settle(game, (0, Some(answer)));
            let renderer = game.client.renderer.0.as_ref().unwrap();
            assert_eq!(renderer.shader_pack_loaded_for(), Some((0, Some(answer))));
            assert_eq!(renderer.shader_pack_kind(), Some("iris"));
            drawn(game)
        };
        let (pack, pack_curve) = owner(game, "pack", Tonemap::Pack);
        let (sunrise, sunrise_curve) = owner(game, "sunrise", Tonemap::Sunrise);
        assert!(!pack_curve, "the pack's own curve: Sunrise's doesn't run");
        // 5.0 × 203 nits held at the 1000-nit peak: ST 2084's 10-bit code for 1000 nits is
        // 769 (769.12), 49264 as the PNG's 16 bits repeat it.
        assert_eq!(pack, 49264);
        assert_eq!(pq16(curved(5.0, 0) * 203.0), 49264);
        assert!(sunrise_curve, "Sunrise's curve runs for it");
        // The curve once: 753, 48239 in 16 bits (twice it would be 744).
        assert_eq!(sunrise, pq16(curved(5.0, 1) * 203.0));
        assert_ne!(pq16(curved(5.0, 2) * 203.0), sunrise);
        // Back to the pack's curve, then a frame drawn without its passes.
        assert_eq!(owner(game, "pack", Tonemap::Pack), (pack, false));
        game.client
            .renderer
            .0
            .as_mut()
            .unwrap()
            .test_hooks
            .fail_scaled_targets = true;
        let (_, fallback_curve) = drawn(game);
        assert!(
            fallback_curve,
            "drawn without the pack's passes: Sunrise's curve runs"
        );
    });
}
