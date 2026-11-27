//! A pack's Polytone expression uniforms on its core shaders (`shader_modifiers`) through a
//! game's frames, from the pack's resources to what is drawn: the menu bundle's atlas loads each
//! shader and resolves its uniforms from the pack's files, the renderer the game builds from it
//! takes both, and each frame evaluates the programs into the blocks the shaders read. F3's
//! lightmap chart reads the drawn light texture back; a screenshot shows the sky. The expression
//! context the frames build is rewritten in place, and copied into a pack's particles' own
//! frame the same way. Each case runs in a process of its own (one event loop, its own game
//! resources).
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use crate::game::native_hooks::particle_count_tests::frame_scene;
use sunrise_assets::resource_source::{Resource, ResourceSource};

const NAME: &str =
    "game::client_side::lightmap_expression_tests::a_packs_lightmap_draws_its_expression_uniform";
const CASE: &str = "SUNRISE_LIGHTMAP_EXPRESSION_CASE";
/// The biome of `frame_scene`'s empty chunk (its biome 0), the camera's.
const BIOME: &str = "minecraft:badlands";

/// A pack whose lightmap writes its `Tint` expression uniform (0.2, a value a unorm byte holds
/// exactly: 51) in red and nothing else, with the file that gives it.
fn pack() -> ResourceSource {
    let files = [
        (
            "assets/minecraft/shaders/core/screenquad.vsh",
            "#version 330\nlayout(location = 0) out vec2 texCoord;\nvoid main() {\n    \
             vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n    \
             gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);\n    texCoord = uv;\n}\n",
        ),
        (
            "assets/minecraft/shaders/core/lightmap.fsh",
            "#version 330\nlayout(std140) uniform Tint {\n    float value;\n};\n\
             layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {\n    fragColor = vec4(value, 0.0, 0.0, 1.0);\n}\n",
        ),
        (
            "assets/minecraft/polytone/shader_modifiers/core/lightmap.json",
            r#"{"Tint": "0.1 * 2"}"#,
        ),
    ];
    ResourceSource::indexed(
        files
            .into_iter()
            .map(|(path, text)| (path.to_owned(), Resource::bytes(text.as_bytes().to_vec())))
            .collect(),
    )
}

/// The sky's vertex stage: the disc over the frame, its rim past the screen's edges.
const SKY_VSH: &str = "#version 330\nlayout(location = 0) in vec3 Position;\nvoid main() {\n    \
    gl_Position = vec4(Position.x / 300.0, Position.z / 300.0, 0.0, 1.0);\n}\n";

/// A pack whose sky writes its `Tint` (0.2: 51) in red, from its own file, and whose stars and
/// block outline read a `Tint` of their own files (0.5, 0.75): those of `shaders` only.
fn sky_pack(shaders: &[&str]) -> ResourceSource {
    let vsh = SKY_VSH;
    let fsh = "#version 330\nlayout(std140) uniform Tint {\n    float value;\n};\n\
               layout(location = 0) out vec4 fragColor;\n\
               void main() {\n    fragColor = vec4(value, 0.0, 0.0, 1.0);\n}\n";
    let mut files: Vec<(String, String)> = Vec::new();
    let all = [
        ("sky", "0.2"),
        ("stars", "0.5"),
        ("rendertype_lines", "0.75"),
    ];
    for (shader, value) in all.into_iter().filter(|(s, _)| shaders.contains(s)) {
        let core = format!("assets/minecraft/shaders/core/{shader}");
        files.push((format!("{core}.vsh"), vsh.to_owned()));
        files.push((format!("{core}.fsh"), fsh.to_owned()));
        let modifier = format!("assets/minecraft/polytone/shader_modifiers/core/{shader}.json");
        files.push((modifier, format!(r#"{{"Tint": "{value}"}}"#)));
    }
    ResourceSource::indexed(
        files
            .into_iter()
            .map(|(path, text)| (path, Resource::bytes(text.into_bytes())))
            .collect(),
    )
}

/// The light texture F3's lightmap chart reads back once frames have drawn it, with the chart
/// open (the game asks the renderer for the readback each frame): a frame copies it, the device
/// maps the copy once the frame is done, and a later frame takes it in.
fn drawn_texture(game: &mut crate::game::ClientGame<'_>) -> [u8; 16 * 16 * 4] {
    game.client.screen = super::Screen::Playing;
    game.client.debug = true;
    game.client.debug_charts.lightmap = true;
    for _ in 0..32 {
        game.render_with(|renderer, frame| renderer.render(frame, None))
            .unwrap();
        let renderer = game.client.renderer.0.as_ref().unwrap();
        renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        if let Some(pixels) = renderer.lightmap_sample() {
            return *pixels;
        }
    }
    panic!("no light texture read back in 32 frames");
}

#[test]
#[ignore = "daily"]
fn a_packs_lightmap_draws_its_expression_uniform() {
    if run_fresh_cases(
        NAME,
        CASE,
        &[
            "pack",
            "fullbright",
            "sky",
            "outline",
            "globals",
            "post",
            "particles",
        ],
    ) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    if case == "sky" {
        return the_sky_draws_its_expression_uniform();
    }
    if case == "globals" {
        return the_sky_draws_poly_globals();
    }
    if case == "post" {
        return a_post_effect_reads_poly_globals();
    }
    if case == "particles" {
        return the_particles_frame_keeps_its_strings();
    }
    if case == "outline" {
        // An outline-only modifier is enough to enable expression evaluation.
        return with_client_game(|game| {
            let pack = sky_pack(&["rendertype_lines"]);
            game.client.gui =
                crate::game::UiResources::load(&pack, std::path::Path::new("")).unwrap();
            let _event_loop = frame_scene(game);
            assert!(game.reads_pack_expressions());
        });
    }
    let fullbright = case == "fullbright";
    with_client_game(|game| {
        // The pack's resources as the game loads them: the atlas resolves the uniform, and the
        // renderer the game builds over them takes the shader with it.
        game.client.gui =
            crate::game::UiResources::load(&pack(), std::path::Path::new("")).unwrap();
        assert!(game.client.gui.atlas.lightmap_shader.is_some());
        assert_eq!(game.client.gui.atlas.lightmap_expressions.len(), 1);
        let _event_loop = frame_scene(game);
        game.client.renderer.0.as_mut().unwrap().fullbright = fullbright;
        // The expression context's strings, given room up front: every frame rewrites them in
        // place, so their buffers (address and capacity) are the same after the frames.
        let context = &mut game.client.expression_frame;
        context.dimension = String::with_capacity(64);
        context.biome = String::with_capacity(64);
        let buffers = |game: &crate::game::ClientGame<'_>| {
            let context = &game.client.expression_frame;
            [&context.dimension, &context.biome].map(|s| (s.as_ptr(), s.capacity()))
        };
        let before = buffers(game);
        let texture = drawn_texture(game);
        if fullbright {
            // Fullbright keeps the texture white, and nothing is evaluated: no context is
            // written.
            assert!(!game.reads_pack_expressions());
            assert!(texture.iter().all(|&b| b == 255));
            assert_eq!(game.client.expression_frame.dimension, "");
            assert_eq!(game.client.expression_frame.biome, "");
        } else {
            assert!(game.reads_pack_expressions());
            for texel in texture.chunks(4) {
                assert_eq!(texel, [51, 0, 0, 255]);
            }
            for _ in 0..3 {
                game.render_with(|renderer, frame| renderer.render(frame, None))
                    .unwrap();
            }
            assert_eq!(buffers(game), before);
            let context = &game.client.expression_frame;
            assert_eq!(context.dimension, "minecraft:overworld");
            assert_eq!(context.biome, BIOME);
        }
    });
}

/// The particles case: a pack's particles read the same context, which each frame copies into
/// their registry's own frame, into the strings that frame already has.
fn the_particles_frame_keeps_its_strings() {
    with_client_game(|game| {
        let registry = sunrise_client::particles::polytone::Registry::read(
            &ResourceSource::indexed(Default::default()),
            Default::default(),
            game.pack.assets.registry,
        );
        {
            let mut held = registry.frame.write().unwrap();
            held.dimension = String::with_capacity(64);
            held.biome = String::with_capacity(64);
        }
        let registry = std::sync::Arc::new(registry);
        game.client.client_pack.polytone_particles = Some(registry.clone());
        let _event_loop = frame_scene(game);
        game.client.screen = super::Screen::Playing;
        let buffers = || {
            let held = registry.frame.read().unwrap();
            [&held.dimension, &held.biome].map(|s| (s.as_ptr(), s.capacity()))
        };
        let before = buffers();
        for _ in 0..3 {
            game.render_with(|renderer, frame| renderer.render(frame, None))
                .unwrap();
        }
        assert_eq!(buffers(), before);
        let held = registry.frame.read().unwrap();
        assert_eq!(held.dimension, "minecraft:overworld");
        assert_eq!(held.biome, BIOME);
    });
}

/// The sky case: the pack's sky, stars and outline take their uniforms from their own files
/// (the same name in each), and the sky disc, which covers the frame (its rim past the screen's
/// edges) draws its value.
fn the_sky_draws_its_expression_uniform() {
    with_client_game(|game| {
        game.client.gui = crate::game::UiResources::load(
            &sky_pack(&["sky", "stars", "rendertype_lines"]),
            std::path::Path::new(""),
        )
        .unwrap();
        let atlas = &game.client.gui.atlas;
        let values = |expressions: &[sunrise_client::polytone_post::ExpressionUniform]| {
            let frame = sunrise_level::environment::Frame::default();
            expressions
                .iter()
                .map(|e| e.evaluate(&frame))
                .collect::<Vec<_>>()
        };
        assert_eq!(values(&atlas.sky_expressions), [0.2]);
        assert_eq!(values(&atlas.stars_expressions), [0.5]);
        assert_eq!(values(&atlas.lines_expressions), [0.75]);
        let _event_loop = frame_scene(game);
        assert!(game.reads_pack_expressions());
        // No shader imports `PolyGlobals`: the frame makes none for them, though in a world it
        // requests a post effect (vanilla's `end_of_frame`).
        assert!(!game.reads_poly_globals());
        assert_eq!(corner(game), [51, 0, 0]);
        assert_eq!(game.requested_post_effects(), ["minecraft:end_of_frame"]);
        assert!(
            !game
                .client
                .renderer
                .0
                .as_ref()
                .unwrap()
                .post_effects_read_poly_globals()
        );
    });
}

/// A frame's screenshot: the colour of the pixel two in from the top left corner.
fn corner(game: &mut crate::game::ClientGame<'_>) -> [u8; 3] {
    corner_of(game, frame)
}

/// A game frame.
pub(in crate::game::client_side) fn frame(
    game: &mut crate::game::ClientGame<'_>,
) -> sunrise_game_data::Result<bool> {
    game.render_with(|renderer, frame| renderer.render(frame, None))
}

/// The colour of the pixel two in from the top left corner of the frame `draw` draws (a game
/// frame, a loading screen's), from a screenshot of it.
pub(in crate::game::client_side) fn corner_of(
    game: &mut crate::game::ClientGame<'_>,
    draw: impl FnOnce(&mut crate::game::ClientGame<'_>) -> sunrise_game_data::Result<bool>,
) -> [u8; 3] {
    game.client.renderer.0.as_mut().unwrap().screenshot =
        Some(sunrise_client::gfx::ScreenshotRequest {
            path: "unused.png".into(),
            size: None,
            scaled: false,
            background: false,
        });
    draw(game).unwrap();
    let result = game
        .client
        .renderer
        .0
        .as_mut()
        .unwrap()
        .screenshot_result
        .take();
    let mut pending = result.unwrap().unwrap();
    let image = pending.image_mut();
    let width = image.width as usize;
    image.rgba[(2 * width + 2) * 4..(2 * width + 2) * 4 + 3]
        .try_into()
        .unwrap()
}

/// The globals case: a pack's sky and lightmap importing Polytone's `polytone_globals.glsl`,
/// which the pack doesn't ship, write the player's block height (64, where the scene stands it)
/// in red beside their `Tint` (0.2: 51) in green: the frame gives the shaders the game's
/// `PolyGlobals`.
fn the_sky_draws_poly_globals() {
    with_client_game(|game| {
        let fsh = "#version 330\n#moj_import <polytone_globals.glsl>\n\
                   layout(std140) uniform Tint {\n    float value;\n};\n\
                   layout(location = 0) out vec4 fragColor;\n\
                   void main() {\n    fragColor = vec4(float(PolyPlayerBlockPos.y) / 255.0, \
                   value, 0.0, 1.0);\n}\n";
        let screenquad = "#version 330\nlayout(location = 0) out vec2 texCoord;\nvoid main() {\n    \
             vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n    \
             gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);\n    texCoord = uv;\n}\n";
        let tint = r#"{"Tint": "0.2"}"#;
        let files = [
            ("assets/minecraft/shaders/core/sky.vsh", SKY_VSH),
            ("assets/minecraft/shaders/core/sky.fsh", fsh),
            (
                "assets/minecraft/polytone/shader_modifiers/core/sky.json",
                tint,
            ),
            ("assets/minecraft/shaders/core/screenquad.vsh", screenquad),
            ("assets/minecraft/shaders/core/lightmap.fsh", fsh),
            (
                "assets/minecraft/polytone/shader_modifiers/core/lightmap.json",
                tint,
            ),
        ];
        let pack = ResourceSource::indexed(
            files
                .into_iter()
                .map(|(path, text)| (path.to_owned(), Resource::bytes(text.as_bytes().to_vec())))
                .collect(),
        );
        game.client.gui = crate::game::UiResources::load(&pack, std::path::Path::new("")).unwrap();
        assert!(game.client.gui.atlas.sky_shader.is_some());
        assert!(game.client.gui.atlas.lightmap_shader.is_some());
        let _event_loop = frame_scene(game);
        assert!(game.reads_poly_globals());
        assert_eq!(corner(game), [64, 51, 0]);
        for texel in drawn_texture(game).chunks(4) {
            assert_eq!(texel, [64, 51, 0, 255]);
        }
    });
}

/// The post case: a pack's own post effect, which the server has the screen run (no Polytone
/// chain), importing the built-in `polytone_globals.glsl`: its pass reads the game's
/// `PolyGlobals` too, the player's block height (64) in red over the whole frame.
fn a_post_effect_reads_poly_globals() {
    with_client_game(|game| {
        let effect = r#"{"passes": [{"vertex_shader": "minecraft:post/globals",
            "fragment_shader": "minecraft:post/globals", "inputs": [],
            "output": "minecraft:main"}]}"#;
        let fsh = "#version 330\n#moj_import <polytone_globals.glsl>\n\
                   layout(location = 0) out vec4 fragColor;\n\
                   void main() {\n    fragColor = vec4(float(PolyPlayerBlockPos.y) / 255.0, \
                   0.2, 0.0, 1.0);\n}\n";
        let vsh = "#version 330\nvoid main() {\n    \
                   vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n    \
                   gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);\n}\n";
        let files = [
            ("assets/minecraft/post_effect/globals.json", effect),
            ("assets/minecraft/shaders/post/globals.vsh", vsh),
            ("assets/minecraft/shaders/post/globals.fsh", fsh),
        ];
        let pack = ResourceSource::indexed(
            files
                .into_iter()
                .map(|(path, text)| (path.to_owned(), Resource::bytes(text.as_bytes().to_vec())))
                .collect(),
        );
        game.client.gui = crate::game::UiResources::load(&pack, std::path::Path::new("")).unwrap();
        game.pack.source = pack;
        let _event_loop = frame_scene(game);
        assert!(!game.reads_poly_globals());
        // Only `end_of_frame` requested: nothing reads them, nothing builds them.
        corner(game);
        assert!(
            !game
                .client
                .renderer
                .0
                .as_ref()
                .unwrap()
                .post_effects_read_poly_globals()
        );
        game.client.post_effects = vec!["minecraft:globals".to_owned()];
        assert_eq!(corner(game), [64, 51, 0]);
        assert!(
            game.client
                .renderer
                .0
                .as_ref()
                .unwrap()
                .post_effects_read_poly_globals()
        );
    });
}
