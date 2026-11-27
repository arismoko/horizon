//! A pack's text shader reading Polytone expression uniforms (`shader_modifiers/core/text.json`)
//! and `PolyGlobals` through a game's frames: the atlas loads the GUI's, the world's and the
//! see-through text's programs with their uniforms, the renderer the game builds takes them, and
//! each drawn frame shows the values the frame evaluated and gave (fullbright changes nothing),
//! or zeros where no world runs (the title screen, a loading screen). Each case runs in a process
//! of its own (one event loop, its own game resources).
use super::lightmap_expression_tests::{corner_of as corner, frame};
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use crate::game::native_hooks::particle_count_tests::frame_scene;
use sunrise_assets::resource_source::{Resource, ResourceSource};

const NAME: &str =
    "game::client_side::text_expression_tests::a_packs_text_draws_its_expression_uniforms";
const CASE: &str = "SUNRISE_TEXT_EXPRESSION_CASE";

/// A pack whose text covers the whole frame with one triangle of each glyph's (a vertex's index
/// picks its corner) in red: the GUI text's `Gui` (0.1 + 0.1: 51 in a byte), the world text's
/// `World` (0.3 * 2: 153), the see-through text's `Fade` (0.25 * 4: 255), each read by that
/// program only, by the shader's own defines. Every program writes 128 plus the player's block
/// height from Polytone's `PolyGlobals` (the built-in `polytone_globals.glsl`) in green: 192 where
/// the scene stands the player (64), 128 where the block is zeros, which still shows the text
/// drew. The world text's also writes vanilla's `ColorModulator` in blue: white (255) in the
/// blocks the frame shares with the GPU particles.
fn pack() -> ResourceSource {
    let vsh = "#version 330\nlayout(location = 0) in vec3 Position;\nvoid main() {\n    \
               int corner = gl_VertexID % 3;\n    \
               vec2 uv = vec2((corner << 1) & 2, corner & 2);\n    \
               gl_Position = vec4(uv * 2.0 - 1.0 + 0.0 * Position.xy, 0.5, 1.0);\n}\n";
    let fsh = "#version 330\n#moj_import <polytone_globals.glsl>\n\
               #ifdef IS_GUI\nlayout(std140) uniform Gui {\n    float value;\n};\n\
               #else\n#ifdef IS_SEE_THROUGH\nlayout(std140) uniform Fade {\n    float value;\n};\n\
               #else\nlayout(std140) uniform World {\n    float value;\n};\n\
               layout(std140) uniform DynamicTransforms {\n    mat4 ModelViewMat;\n    \
               mat4 TextureMat;\n    vec4 ColorModulator;\n    vec3 ModelOffset;\n};\n\
               #define BLUE ColorModulator.b\n#endif\n#endif\n#ifndef BLUE\n#define BLUE 0.0\n#endif\n\
               layout(location = 0) out vec4 fragColor;\n\
               void main() {\n    \
               float green = (128.0 + float(PolyPlayerBlockPos.y)) / 255.0;\n    \
               fragColor = vec4(value, green, BLUE, 1.0);\n}\n";
    let files = [
        ("assets/minecraft/shaders/core/text.vsh", vsh),
        ("assets/minecraft/shaders/core/text.fsh", fsh),
        (
            "assets/minecraft/polytone/shader_modifiers/core/text.json",
            r#"{"Gui": "0.1 + 0.1", "World": "0.3 * 2", "Fade": "0.25 * 4"}"#,
        ),
    ];
    ResourceSource::indexed(
        files
            .into_iter()
            .map(|(path, text)| (path.to_owned(), Resource::bytes(text.as_bytes().to_vec())))
            .collect(),
    )
}

#[test]
#[ignore = "daily"]
fn a_packs_text_draws_its_expression_uniforms() {
    if run_fresh_cases(NAME, CASE, &["world", "fullbright", "loading"]) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    let (loading, fullbright) = (case == "loading", case == "fullbright");
    with_client_game(|game| {
        game.client.gui =
            crate::game::UiResources::load(&pack(), std::path::Path::new("")).unwrap();
        let frame_context = sunrise_level::environment::Frame::default();
        let text = &game.client.gui.atlas.text_shaders;
        for (expressions, value) in [
            (&text.expressions.gui, 0.2),
            (&text.expressions.world, 0.6),
            (&text.expressions.see_through, 1.0),
        ] {
            let values: Vec<f32> = expressions
                .iter()
                .map(|e| e.evaluate(&frame_context))
                .collect();
            assert_eq!(values, [value]);
        }
        let _event_loop = frame_scene(game);
        assert!(game.reads_pack_expressions() && game.reads_poly_globals());
        if loading {
            return the_loading_screen_draws_zeros(game);
        }
        // A text display in front of the camera, as a server's would arrive.
        let sy = game.world.shape.column().section_index(65).unwrap() as i32;
        game.client.settings.chunk_fade = 0.0;
        game.client
            .renderer
            .0
            .as_mut()
            .unwrap()
            .upload((0, sy, 0), &sunrise_client::mesher::SectionMesh::default());
        let mut display = super::text_displays::RemoteTextDisplay::new(
            super::text_displays::Shows::Text,
            [0.5, 65.5, 3.5],
            0.0,
        );
        display.metadata(&[(
            23,
            0,
            sunrise_protocol::entity_data::Value::Text(sunrise_core::nbt::Tag::String("W".into())),
        )]);
        game.client.text_displays.0.insert(1, display);
        game.tick_text_displays(game.clock.runs_normally());
        // Fullbright, in its own process, is on from the first frame: it changes nothing.
        game.client.renderer.0.as_mut().unwrap().fullbright = fullbright;
        // The world text over the frame with the GUI hidden (F1), then the GUI's (F3's text)
        // over it.
        game.client.chat_ui.hide_gui = true;
        assert_eq!(
            corner(game, frame),
            [153, 192, 255],
            "fullbright {fullbright}"
        );
        game.client.chat_ui.hide_gui = false;
        game.client.debug = true;
        assert_eq!(corner(game, frame), [51, 192, 0], "fullbright {fullbright}");
        // The title screen: no world runs, the GUI text's value and `PolyGlobals` are 0.
        game.client.title.0 = Some(game.new_title(std::path::PathBuf::from("saves")));
        assert!(!game.reads_pack_expressions(), "none to evaluate");
        assert!(!game.reads_poly_globals(), "none to give");
        assert_eq!(corner(game, frame), [0, 128, 0]);
    });
}

/// A loading screen's frame (`loading_frame`, as the level loading and file fixer screens draw)
/// after a world frame evaluated the GUI text's value and gave its `PolyGlobals`: its text draws
/// 0 for both.
fn the_loading_screen_draws_zeros(game: &mut crate::game::ClientGame<'_>) {
    game.client.debug = true;
    assert_eq!(corner(game, frame), [51, 192, 0]);
    let renderer = game.client.renderer.0.as_ref().unwrap();
    let size = [renderer.config.width, renderer.config.height];
    let mut hud = sunrise_client::hud::HudBatch::new(size, game.client.settings.gui_scale_inputs());
    sunrise_client::level_loading::draw_progress(&mut hud, &game.client.gui.atlas, "Loading", 0.5);
    let loading = |game: &mut crate::game::ClientGame<'_>| {
        let frame = super::loading_frame(&hud);
        game.client
            .renderer
            .0
            .as_mut()
            .unwrap()
            .render(&frame, None)
    };
    assert_eq!(corner(game, loading), [0, 128, 0]);
}
