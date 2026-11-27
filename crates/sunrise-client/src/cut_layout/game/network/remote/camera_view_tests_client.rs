use crate::game::remote::camera_view_tests::BOAT;
use crate::game::remote::camera_view_tests::FRAME_CASE;
use crate::game::remote::camera_view_tests::FRAME_NAME;
use crate::game::remote::camera_view_tests::OWN;
use crate::game::remote::camera_view_tests::PLAYER_VIEW;
use crate::game::remote::camera_view_tests::ZOMBIE;
use crate::game::remote::camera_view_tests::framed;
use crate::game::remote::camera_view_tests::kept;
use crate::game::remote::camera_view_tests::remove;
use crate::game::remote::camera_view_tests::set_camera;
use crate::game::remote::camera_view_tests::set_light;
use crate::game::remote::camera_view_tests::update;
use crate::game::remote::camera_view_tests::view;
use crate::game::remote::camera_view_tests::with_spectator;
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_client::{gfx::pack_uniforms::ViewEntity, options_file::V};
use sunrise_level::camera_math::CameraType;
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// An entity that isn't living has no effects to read and looks nowhere, and a living one with
/// none reads none; neither has the player's mood or blindness.
#[test]
#[ignore = "daily"]
fn a_view_on_another_entity_has_none_of_the_players_values() {
    with_spectator(|game| {
        game.client.ambience.moodiness = 0.375;
        update(game, OWN, "minecraft:blindness", 100, 0);
        // Not kept: a boat is not a LivingEntity.
        update(game, BOAT, "minecraft:blindness", 100, 0);
        assert_eq!(kept(game, BOAT), None);
        set_camera(game, BOAT);
        let none = ViewEntity {
            living: false,
            blindness: 0.0,
            night_vision: 0.0,
            darkness: 0.0,
            mood: 0.0,
            eye_light: [0; 2],
        };
        assert_eq!(view(game), none);
        set_camera(game, ZOMBIE);
        assert_eq!(
            view(game),
            ViewEntity {
                living: true,
                ..none
            }
        );
        // The boat leaves while it is the camera: the view stays on it (`Minecraft.cameraEntity`
        // keeps the removed entity), with none of the player's values.
        set_camera(game, BOAT);
        game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![BOAT] }))
            .unwrap();
        assert_eq!(view(game), none);
        // So does a removed living camera: the zombie's blindness as it was.
        update(game, ZOMBIE, "minecraft:blindness", 100, 0);
        set_camera(game, ZOMBIE);
        game.handle_packet(C::RemoveEntities(play::RemoveEntities {
            ids: vec![ZOMBIE],
        }))
        .unwrap();
        assert_eq!(
            view(game),
            ViewEntity {
                living: true,
                blindness: 1.0,
                ..none
            }
        );
        set_camera(game, OWN);
        assert_eq!(view(game), PLAYER_VIEW);
    });
}

/// Through a game's frame: `eyeBrightness` reads the light at the camera entity's eye in first
/// person and in third person, where the camera backs off to another block with other light.
#[test]
#[ignore = "daily"]
fn the_frame_reads_the_eye_light_in_first_and_third_person() {
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    if run_fresh_cases(FRAME_NAME, FRAME_CASE, &["frame"]) {
        return;
    }
    with_client_game(|game| {
        let _event_loop =
            sunrise_client_gameplay::game::native_hooks::particle_count_tests::frame_scene(game);
        game.client.screen = sunrise_gameplay_common::game::Screen::Playing;
        // The view's uniforms are worked out while a pack is picked: a `shaders/` pack with
        // one program. The first frame turns the resource packs' material maps on and reloads
        // them, and the next loads the pack.
        let packs = tempfile::tempdir().unwrap();
        let pack = packs.path().join("eye_light");
        let shaders = pack.join("shaders");
        std::fs::create_dir_all(&shaders).unwrap();
        std::fs::write(
            shaders.join("gbuffers_terrain.vsh"),
            "#version 120\nvoid main() {\n    gl_Position = ftransform();\n}\n",
        )
        .unwrap();
        std::fs::write(
            shaders.join("gbuffers_terrain.fsh"),
            "#version 120\nvoid main() {\n    gl_FragData[0] = vec4(1.0);\n}\n",
        )
        .unwrap();
        game.client.settings.sunrise.shader_pack = pack.to_string_lossy().into_owned();
        framed(game);
        framed(game);
        for camera_type in [CameraType::FirstPerson, CameraType::ThirdPersonBack] {
            game.apply_camera_type(camera_type);
            let (camera, eye, _) = framed(game);
            let (at_eye, at_camera) = (eye.floor().as_ivec3(), camera.floor().as_ivec3());
            assert_eq!(
                at_eye == at_camera,
                camera_type == CameraType::FirstPerson,
                "{camera_type:?}"
            );
            // Sky 5, block 12 at the eye; sky 3, block 0 at a third-person camera.
            if at_eye != at_camera {
                set_light(game, at_camera, 0x30);
            }
            set_light(game, at_eye, 0x5c);
            assert_eq!(framed(game).2, [12, 5], "{camera_type:?}");
            set_light(game, at_eye, 0);
        }
    });
}

/// `eyeBrightness` reads the light at the camera entity's eye, where a third-person camera
/// backs off from: not at the camera's own block.
#[test]
#[ignore = "daily"]
fn the_view_light_is_at_the_camera_entitys_eye() {
    with_spectator(|game| {
        set_camera(game, ZOMBIE);
        game.apply_camera_type(CameraType::ThirdPersonBack);
        let (camera, _, _) = game.camera_view(0.0).unwrap();
        let eye = glam::DVec3::from_array(game.client.view.eye);
        let (at_eye, at_camera) = (eye.floor().as_ivec3(), camera.floor().as_ivec3());
        assert_ne!(at_eye, at_camera);
        // Sky 5, block 12 at the eye; sky 3, block 0 at the camera.
        set_light(game, at_eye, 0x5c);
        set_light(game, at_camera, 0x30);
        assert_eq!(game.pack_view_entity(eye, 0.0).eye_light, [12, 5]);
        assert_eq!(game.pack_view_entity(camera, 0.0).eye_light, [0, 3]);
    });
}

/// While the view is on a spectated zombie, `blindness`, `nightVision` and `darknessFactor`
/// read the effects the server sent for it, counting down and fading on its ticks, and the mood
/// is 0; on the player's own view they are the player's, and the lightmap keeps the player's
/// all along.
#[test]
#[ignore = "daily"]
fn the_view_reads_the_spectated_entitys_effects() {
    with_spectator(|game| {
        game.client.ambience.moodiness = 0.375;
        game.client
            .settings
            .file
            .set("darknessEffectScale", V::Double(0.5));
        update(game, OWN, "minecraft:blindness", 100, 0);
        // Not blended (no flag 8): darkness is at its target at once.
        update(game, ZOMBIE, "minecraft:night_vision", 201, 0);
        update(game, ZOMBIE, "minecraft:darkness", 600, 0);
        assert_eq!(view(game), PLAYER_VIEW, "the player is the camera");

        set_camera(game, ZOMBIE);
        let zombie = ViewEntity {
            living: true,
            blindness: 0.0,
            night_vision: 1.0,
            darkness: 0.5,
            mood: 0.0,
            eye_light: [0; 2],
        };
        assert_eq!(view(game), zombie);
        // The lightmap's night vision and darkness stay the player's (none).
        assert_eq!(game.night_vision_intensity(0.25), 0.0);
        assert_eq!(game.darkness_lighting(0.25).0, 0.0);

        // GameRenderer.nightVisionScale once 200 ticks are left: it flickers, on Mth.sin's
        // table (Java's float expression evaluated outside Sunrise: 0.6530526; f32 `sin` gives
        // 0.6530707).
        game.tick_remote_entities(true);
        assert_eq!(view(game).night_vision, 0.6530526);
        // A frozen level ticks no mob: nothing counts down.
        game.tick_remote_entities(false);
        assert_eq!(view(game).night_vision, 0.6530526);

        update(game, ZOMBIE, "minecraft:blindness", 40, 0);
        assert_eq!(view(game).blindness, 1.0);
        remove(game, ZOMBIE, "minecraft:blindness");
        assert_eq!(view(game).blindness, 0.0);
        // getEffectBlendFactor: the fade is the instance's, so it goes with it, before any tick.
        remove(game, ZOMBIE, "minecraft:darkness");
        assert_eq!(view(game).darkness, 0.0);
        game.tick_remote_entities(true);
        assert_eq!(view(game).darkness, 0.0);

        set_camera(game, OWN);
        assert_eq!(view(game), PLAYER_VIEW, "back on the player");
    });
}
