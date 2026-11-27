use super::clouds_reload_tests::{INNER, a_reload_fails_and_drops_the_selection, write_pack};
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_client_gameplay::game::client_side::native_entity_refs::tests::{
    run_fresh_cases, test_window,
};
use sunrise_client_gameplay::game::{
    ClientGame, client_side::model_baking_completed_tests::reload,
};

const NAME: &str = "cut_layout::game::client_side::packs::core_shaders_reload_tests::a_pack_whose_core_shader_a_required_pipeline_refuses_fails_the_load";
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
            game.client.settings.resource_packs = vec!["file/inner".into(), "file/broken".into()];
            reload(game);
            assert_eq!(
                game.client.settings.resource_packs,
                ["file/inner", "file/broken"]
            );
            assert!(game.client.gui.atlas.sky_shader.is_some());
            assert!(game.client.gui.atlas.text_shaders.gui.is_some());
        });
    } else {
        a_reload_fails_and_drops_the_selection("file/broken");
    }
}

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
        let atlas = |game: &ClientGame<'_>| game.client.gui.atlas.sky_shader.is_some();
        game.client.settings.resource_packs = vec!["file/inner".into(), "file/taken".into()];
        reload(game);
        assert_eq!(
            game.client.settings.resource_packs,
            ["file/inner", "file/taken"]
        );
        assert!(atlas(game));
        game.client.settings.resource_packs = vec!["file/inner".into(), "file/vec3".into()];
        reload(game);
        assert_eq!(
            game.client.settings.resource_packs,
            ["file/inner", "file/vec3"]
        );
        assert!(atlas(game), "the atlas keeps the pack's sky program");
        game.client.settings.resource_packs = vec!["file/inner".into(), "file/refused".into()];
        reload(game);
        assert!(game.client.settings.resource_packs.is_empty());
        assert!(!atlas(game), "the working pack's sky went too");
        assert!(
            game.client
                .toast_manager
                .system_mut(
                    sunrise_client_gameplay::game::toast_manager::SystemToastId::PackLoadFailure
                )
                .is_some(),
            "resourcePack.load_fail"
        );
        assert!(game.test_renderer().is_some(), "the game's own renderer");
    });
}

/// A hidden window with the game's renderer on it, as the reload remakes it.
fn window(game: &mut ClientGame<'_>) -> winit::event_loop::EventLoop<()> {
    let (event_loop, window) = test_window([320, 240]);
    game.options.offscreen = true;
    let renderer = game.new_renderer(window.clone()).unwrap();
    game.test_set_renderer(renderer);
    *game.test_window_mut() = Some(window);
    event_loop
}
