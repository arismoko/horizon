//! A selected pack whose core shader one of the pipelines vanilla 26.3 requires can't be made
//! from (`ShaderManager.apply`'s "Failed to load required shader programs"): the reload fails as
//! vanilla's does (`Minecraft.clearResourcePacksOnError`), dropping every selected pack, the
//! working pack's own core shader included. That holds for the text, the block outline, the sky,
//! the stars and the lightmap, for pipelines Sunrise never draws with the pack's shader (the
//! grayscale text, Improved Transparency's stages) and for a pipeline only the device refuses,
//! when the renderer is made. Each case runs in a process of its own, in a folder whose
//! `resourcepacks` holds the packs.
use super::clouds_reload_tests::{INNER, a_reload_fails_and_drops_the_selection, write_pack};
use crate::game::model_baking_completed_tests::reload;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};

const NAME: &str = "game::packs::core_shaders_reload_tests::a_pack_whose_core_shader_a_required_pipeline_refuses_fails_the_load";
const CASE: &str = "SUNRISE_CORE_SHADERS_RELOAD_CASE";

/// `resourcepacks/<name>`: a pack of `files` (path under `assets/minecraft/shaders/core/`, text).
fn write_shader_pack(name: &str, files: &[(&str, String)]) {
    let dir = std::env::current_dir()
        .unwrap()
        .join("resourcepacks")
        .join(name);
    let file = |path: &str, bytes: &[u8]| {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    file(
        "pack.mcmeta",
        br#"{"pack": {"description": "core shader", "pack_format": 97, "min_format": [97, 0], "max_format": [97, 1]}}"#,
    );
    for (path, text) in files {
        file(
            &format!("assets/minecraft/shaders/core/{path}"),
            text.as_bytes(),
        );
    }
}

/// A fragment stage for vanilla's vertex stage of any of these shaders, using an undeclared
/// name, which no translator takes, where `condition` (a `#if` expression) holds.
fn fragment(condition: &str) -> String {
    format!(
        "#version 330\nlayout(location = 0) out vec4 fragColor;\nvoid main() {{\n    \
         fragColor = vec4(0.25, 0.5, 0.75, 1.0);\n#if {condition}\n    fragColor = undeclared;\n\
         #endif\n}}\n"
    )
}

#[test]
#[ignore = "daily"]
fn a_pack_whose_core_shader_a_required_pipeline_refuses_fails_the_load() {
    let cases = [
        ("control", "text.fsh", "0"),
        (
            "text",
            "text.fsh",
            "!defined(OIT) && !defined(IS_GUI) && !defined(IS_SEE_THROUGH)",
        ),
        (
            "text_grayscale",
            "text.fsh",
            "defined(IS_GRAYSCALE) && defined(IS_SEE_THROUGH)",
        ),
        ("text_oit", "text.fsh", "defined(OIT_ACCUMULATE)"),
        (
            "lines_oit",
            "rendertype_lines.fsh",
            "defined(OIT_TRANSMITTANCE)",
        ),
        ("sky", "sky.fsh", "1"),
        ("stars", "stars.fsh", "1"),
        ("lightmap", "lightmap.fsh", "1"),
    ];
    let mut names: Vec<&str> = cases.iter().map(|(name, ..)| *name).collect();
    names.push("device");
    if run_fresh_cases(NAME, CASE, &names) {
        return;
    }
    write_pack("inner", INNER);
    let case = std::env::var(CASE).unwrap();
    if case == "device" {
        return the_device_refusing_a_drawn_pipeline_fails_the_reload();
    }
    let (_, file, condition) = cases.iter().find(|(name, ..)| *name == case).unwrap();
    write_shader_pack("broken", &[(file, fragment(condition))]);
    if case == "control" {
        // The same pack where no pipeline keeps the broken line: both packs' shaders load.
        with_client_game(|game| {
            game.settings.resource_packs = vec!["file/inner".into(), "file/broken".into()];
            reload(game);
            assert_eq!(game.settings.resource_packs, ["file/inner", "file/broken"]);
            assert!(game.gui.atlas.sky_shader.is_some());
            assert!(game.gui.atlas.text_shaders.gui.is_some());
        });
    } else {
        a_reload_fails_and_drops_the_selection("file/broken");
    }
}

/// A `sky.fsh` sampling `Sampler0`, which `SKY` doesn't bind, translates, and only the device
/// refuses its pipeline when the reload makes the renderer, as vanilla's builder refuses a
/// descriptor missing from the pipeline's layout (`PipelineBuilder`): the reload fails as a
/// translation failure does. The same stage not sampling it is taken, and one writing a `vec3`
/// colour, which vanilla doesn't check and the device doesn't take, draws natively with the packs
/// kept.
fn the_device_refusing_a_drawn_pipeline_fails_the_reload() {
    let sky = |colour: &str| {
        format!(
            "#version 330\nuniform sampler2D Sampler0;\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {{\n    fragColor = {colour};\n}}\n"
        )
    };
    write_shader_pack("taken", &[("sky.fsh", sky("vec4(1.0)"))]);
    write_shader_pack(
        "refused",
        &[("sky.fsh", sky("texture(Sampler0, vec2(0.5))"))],
    );
    // A colour output the device doesn't take into the sky's target, which vanilla's builder
    // doesn't check: the sky draws natively, and the packs stay.
    write_shader_pack(
        "vec3",
        &[(
            "sky.fsh",
            "#version 330\nlayout(location = 0) out vec3 fragColor;\n\
             void main() {\n    fragColor = vec3(1.0);\n}\n"
                .to_owned(),
        )],
    );
    with_client_game(|game| {
        let _event_loop = window(game);
        let atlas = |game: &crate::game::Game<'_>| game.gui.atlas.sky_shader.is_some();
        game.settings.resource_packs = vec!["file/inner".into(), "file/taken".into()];
        reload(game);
        assert_eq!(game.settings.resource_packs, ["file/inner", "file/taken"]);
        assert!(atlas(game));
        game.settings.resource_packs = vec!["file/inner".into(), "file/vec3".into()];
        reload(game);
        assert_eq!(game.settings.resource_packs, ["file/inner", "file/vec3"]);
        assert!(atlas(game), "the atlas keeps the pack's sky program");
        game.settings.resource_packs = vec!["file/inner".into(), "file/refused".into()];
        reload(game);
        assert!(game.settings.resource_packs.is_empty());
        assert!(!atlas(game), "the working pack's sky went too");
        assert!(
            game.toast_manager
                .system_mut(crate::game::toast_manager::SystemToastId::PackLoadFailure)
                .is_some(),
            "resourcePack.load_fail"
        );
        assert!(game.renderer.is_some(), "the game's own renderer");
    });
}

/// A hidden window with the game's renderer on it, as the reload remakes it.
fn window(game: &mut crate::game::Game<'_>) -> winit::event_loop::EventLoop<()> {
    let (event_loop, window) = crate::game::native_hooks::particle_count_tests::hidden_window();
    game.options.offscreen = true;
    let renderer = game.new_renderer(window.clone()).unwrap();
    game.set_renderer(renderer);
    game.window = Some(window);
    event_loop
}
