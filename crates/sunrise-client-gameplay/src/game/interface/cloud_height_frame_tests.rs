//! The cloud layer's height through a game's frames: `EnvironmentAttributes.CLOUD_HEIGHT` as the
//! server's synchronized dimension type sets it (`minecraft:visual/cloud_height`), probed at the
//! camera each frame as vanilla's `LevelExtractor` does, places the layer the renderer draws. A
//! pack's `core/clouds` shows it: its `CloudInfo.CloudOffset.y` is the layer's bottom less the
//! camera's height. Each case runs in a process of its own (one event loop, its own resources).
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use crate::game::native_hooks::particle_count_tests::frame_scene;
use sunrise_assets::resource_source::{Resource, ResourceSource};
use sunrise_client::gfx::ScreenshotRequest;

const NAME: &str =
    "game::client_side::cloud_height_frame_tests::the_cloud_layer_follows_the_dimensions_height";
const CASE: &str = "SUNRISE_CLOUD_HEIGHT_FRAME_CASE";

/// A pack whose clouds cover the whole frame (each face's quad placed by `gl_VertexIndex`,
/// nearest of all), white where the layer's bottom is more than 150 blocks above the camera
/// (`CloudOffset.y`), black where it isn't: the camera stands at y 64, 186 below a layer at 250
/// and 128 below one at 192.33.
fn pack() -> ResourceSource {
    let files = [
        (
            "assets/minecraft/shaders/core/clouds.vsh",
            "#version 330\nlayout(std140) uniform CloudInfo {\n    vec4 CloudColor;\n    \
             vec3 CloudOffset;\n    vec3 CellSize;\n};\nlayout(location = 0) out float above;\n\
             void main() {\n    int corner = gl_VertexIndex % 4;\n    \
             vec2 xy = vec2(corner == 1 || corner == 2 ? 1.0 : -1.0, corner >= 2 ? 1.0 : -1.0);\n    \
             above = CloudOffset.y > 150.0 ? 1.0 : 0.0;\n    \
             gl_Position = vec4(xy, 1.0, 1.0);\n}\n",
        ),
        (
            "assets/minecraft/shaders/core/clouds.fsh",
            "#version 330\nlayout(location = 0) in float above;\n\
             layout(location = 0) out vec4 fragColor;\n\
             void main() {\n    fragColor = vec4(vec3(above), 1.0);\n}\n",
        ),
    ];
    ResourceSource::indexed(
        files
            .into_iter()
            .map(|(path, text)| (path.to_owned(), Resource::bytes(text.as_bytes().to_vec())))
            .collect(),
    )
}

/// Joins a server whose Overworld's synchronized dimension type is the prepared resources' with
/// its `minecraft:visual/cloud_height` set to `height`, through the `Login` packet a server sends.
fn join_overworld(game: &mut crate::game::ClientGame<'_>, height: f64) {
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    let resources = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let path = "data/minecraft/dimension_type/overworld.json";
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(resources.join(path)).unwrap()).unwrap();
    json["attributes"]["minecraft:visual/cloud_height"] = serde_json::json!(height);
    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(client, "CloudHeightFrameTest").unwrap();
    let mut registries = (*game.client.remote.as_ref().unwrap().registries).clone();
    registries.registries.insert(
        "minecraft:dimension_type".into(),
        vec![(
            "minecraft:overworld".into(),
            Some(sunrise_game_data::scoreboard::json_to_tag(&json)),
        )],
    );
    game.set_registries(std::sync::Arc::new(registries))
        .unwrap();
    game.handle_packet(C::Login(play::Login {
        player_id: 7,
        hardcore: false,
        levels: vec!["minecraft:overworld".into()],
        max_players: 1,
        chunk_radius: 2,
        simulation_distance: 2,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: play::SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        },
        online_mode: false,
        enforces_secure_chat: false,
    }))
    .unwrap();
    // The level's terrain is in: the loading screen closes.
    game.client.remote.as_mut().unwrap().level_load.finish();
}

/// A pixel left of the hotbar and the crosshair.
fn drawn_pixel(game: &mut crate::game::ClientGame<'_>) -> [u8; 4] {
    game.client.renderer.0.as_mut().unwrap().screenshot = Some(ScreenshotRequest {
        path: "unused.png".into(),
        size: None,
        scaled: false,
        background: false,
    });
    game.render_with(|renderer, frame| renderer.render(frame, None))
        .unwrap();
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
    let at = (40 * image.width as usize + 20) * 4;
    image.rgba[at..at + 4].try_into().unwrap()
}

/// A joined Overworld whose type keeps vanilla's height (192.33) leaves the layer 128 above the
/// camera; one whose type sets 250 raises it to 186 above, in the first frame after the join.
#[test]
#[ignore = "daily"]
fn the_cloud_layer_follows_the_dimensions_height() {
    if run_fresh_cases(NAME, CASE, &["default", "raised"]) {
        return;
    }
    let raised = std::env::var(CASE).unwrap() == "raised";
    with_client_game(|game| {
        game.client.gui =
            crate::game::UiResources::load(&pack(), std::path::Path::new("")).unwrap();
        assert!(game.client.gui.atlas.clouds_shader.is_some());
        join_overworld(game, if raised { 250.0 } else { 192.33 });
        let _event_loop = frame_scene(game);
        game.client.screen = super::Screen::Playing;
        game.client.screen_fx.vignette_brightness = 0.0;
        let expected = if raised { 255 } else { 0 };
        assert_eq!(drawn_pixel(game), [expected, expected, expected, 255]);
    });
}
