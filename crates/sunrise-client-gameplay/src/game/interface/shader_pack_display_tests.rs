//! Sunrise's display uniforms (#5827 step 3) through the game's own loader and frames, on a
//! real offscreen renderer: the renderer fills them from the output each frame is drawn for.
//! Each case runs in a process of its own (one event loop, its own resources and environment).
use super::native_entity_refs::tests::with_client_game;
use super::shader_pack_hdr_tests::{CASE, SERVED, curved, pack, pq16, settle};
use crate::game::native_hooks::particle_count_tests::frame_scene;
use sunrise_client::gfx::ScreenshotRequest;
use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases_with_env;
use sunrise_shaders::Tonemap;

/// A `final` that writes, across four columns of the screen, grey light of `sunriseHdr` × 0.5,
/// `sunriseHeadroom` × 0.5, `sunrisePaperWhite` / 390 and `sunrisePeak` / 1980, the same with
/// or without `SUNRISE_HDR`.
const FINAL: &str = "#version 120\n\
    uniform int sunriseHdr;\n\
    uniform float sunriseHeadroom;\n\
    uniform float sunrisePaperWhite;\n\
    uniform float sunrisePeak;\n\
    varying vec2 uv;\n\
    void main() {\n\
        float v = uv.x < 0.25 ? float(sunriseHdr) * 0.5 : uv.x < 0.5 ? sunriseHeadroom * 0.5\n\
            : uv.x < 0.75 ? sunrisePaperWhite / 390.0 : sunrisePeak / 1980.0;\n\
        gl_FragColor = vec4(v, v, v, 1.0);\n\
    }\n";

/// The pack asks for Sunrise's HDR; no vignette, which eases in over frames.
const PROPERTIES: &str = "vignette = false\nsunrise.hdr = true\n";

/// A frame of the opted-in pack, with HDR output `hdr` and the display at paper white 203 and
/// peak 1000 nits, once it runs for `served`; the screenshot of it.
fn shot(hdr: bool, served: Option<Tonemap>) -> sunrise_client::gfx::PendingScreenshot {
    let scratch = tempfile::tempdir().unwrap();
    let path = pack(
        scratch.path(),
        "display",
        &[("shaders.properties", PROPERTIES), ("final.fsh", FINAL)],
    );
    let mut taken = None;
    with_client_game(|game| {
        game.client.settings.sunrise.hdr = if hdr {
            sunrise_client::graphics::HdrMode::On
        } else {
            sunrise_client::graphics::HdrMode::Off
        };
        game.client.settings.sunrise.hdr_paper_white = 203;
        game.client.settings.sunrise.hdr_peak = 1000;
        game.client.settings.sunrise.shader_pack = path.to_str().unwrap().to_owned();
        // The material maps already on: no resource reload, whose overlay would cover the frame.
        sunrise_client::sprites::set_labpbr_maps(true);
        let _event_loop = frame_scene(game);
        game.apply_ui_settings();
        settle(game, (0, served));
        game.client.renderer.0.as_mut().unwrap().screenshot = Some(ScreenshotRequest {
            path: "unused.png".into(),
            size: None,
            scaled: false,
            background: false,
        });
        game.render_with(|renderer, frame| renderer.render(frame, None))
            .unwrap();
        let renderer = game.client.renderer.0.as_mut().unwrap();
        taken = Some(renderer.screenshot_result.take().unwrap().unwrap());
    });
    taken.unwrap()
}

/// The four columns' pixels on row 40 (above the hotbar, beside nothing of the HUD), of a
/// picture `width` wide.
fn columns(width: u32) -> [usize; 4] {
    std::array::from_fn(|i| 40 * width as usize + width as usize * (2 * i + 1) / 8)
}

/// The red of each column in the 8-bit screenshot.
fn sdr_columns(shot: &mut sunrise_client::gfx::PendingScreenshot) -> [u8; 4] {
    let image = shot.image_mut();
    columns(image.width).map(|at| image.rgba[at * 4])
}

/// Served on the layered frame (HDR10 offscreen at paper white 203 and peak 1000): the columns
/// read 1, 1000/203, 203 and 1000, each as its grey light shows in the HDR10 copy's ST 2084
/// code (no curve: the pack owns it).
#[test]
#[ignore = "daily"]
fn a_served_pack_reads_the_layered_frames_display() {
    const NAME: &str = "game::client_side::shader_pack_display_tests::a_served_pack_reads_the_layered_frames_display";
    if run_fresh_cases_with_env(NAME, CASE, &["on"], 1, &SERVED) {
        return;
    }
    let shot = shot(true, Some(Tonemap::Pack));
    let ([width, _], pixels) = shot.hdr_pixels().expect("an HDR10 copy: the layered frame");
    let got = columns(width).map(|at| u16::from_be_bytes([pixels[at * 6], pixels[at * 6 + 1]]));
    let light = [
        1.0 * 0.5,
        1000.0 / 203.0 * 0.5,
        203.0 / 390.0,
        // `sunrisePeak` is paper white times the headroom, in f32.
        f64::from(203.0f32 * (1000.0f32 / 203.0)) / 1980.0,
    ];
    assert_eq!(got, light.map(|v| pq16(curved(v, 0) * 203.0)));
}

/// Served on an SDR frame (HDR output off): the columns read SDR's 0, 1, 80 and 80, the light
/// encoded in vanilla's 2.2 power into 8 bits: 0, 0.5 as 186, 80/390 as 124 and 80/1980 as 59.
#[test]
#[ignore = "daily"]
fn a_served_pack_reads_sdrs_display() {
    const NAME: &str =
        "game::client_side::shader_pack_display_tests::a_served_pack_reads_sdrs_display";
    if run_fresh_cases_with_env(NAME, CASE, &["on"], 1, &SERVED) {
        return;
    }
    let mut shot = shot(false, Some(Tonemap::Pack));
    assert_eq!(sdr_columns(&mut shot), [0, 186, 124, 59]);
}

/// The same pick without the switch isn't served: its program reads none of the display
/// uniforms, and every column is black.
#[test]
#[ignore = "daily"]
fn a_pack_not_served_reads_no_display() {
    const NAME: &str =
        "game::client_side::shader_pack_display_tests::a_pack_not_served_reads_no_display";
    if run_fresh_cases_with_env(NAME, CASE, &["off"], 1, &[("SUNRISE_HDR_FRAME", "off")]) {
        return;
    }
    let mut shot = shot(false, None);
    assert_eq!(sdr_columns(&mut shot), [0; 4]);
}
