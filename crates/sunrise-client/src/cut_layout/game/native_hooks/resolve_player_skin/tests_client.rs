use mod_api::PlayerModel;
use mod_api::PlayerSkinOverride;
use mod_api::{Identifier, ModEntry};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};
use sunrise_client::hud::Rect;
use sunrise_client_gameplay::game::ClientGame;
use sunrise_gameplay_common::game::native_hooks::resolve_player_skin::tests::Choice;
use sunrise_gameplay_common::game::native_hooks::resolve_player_skin::tests::SkinPicker;
use sunrise_level::camera_math::CameraType;
use sunrise_level::crash::{ModCall, enter_native_mod};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_client_gameplay::game::native_hooks::particle_count_tests::frame_scene,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay_common::game::Screen,
};

fn skin_slot_name(game: &mut ClientGame<'_>, texture: &Identifier) -> String {
    game.test_skin_slot_name(texture)
}

#[test]
#[ignore = "daily"]
fn live_skin_selection_reaches_the_hand_body_and_preview_and_restores_vanilla() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::resolve_player_skin::tests::live_skin_selection_reaches_the_hand_body_and_preview_and_restores_vanilla",
        "SUNRISE_SKIN_SELECTION_CHILD",
        &["live"],
    ) {
        return;
    }
    let choice = Arc::new(Mutex::new(Choice::default()));
    with_mods(
        vec![ModEntry::new(
            "picker",
            Box::new(SkinPicker(choice.clone())),
        )]
        .into(),
        || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                game.tick_player_model();
                let avatar = game.client.player_model.as_mut().unwrap();
                let vanilla = "entity/player/wide/steve";
                avatar.skin = Some(vanilla.into());
                avatar.slim = false;

                let wide = Identifier::parse("picker:wide").unwrap();
                let slim = Identifier::parse("picker:slim").unwrap();
                let wrong_size = Identifier::parse("picker:wrong_size").unwrap();
                let edit_texture = || {
                    enter_native_mod(ModCall {
                        id: "picker",
                        hook: "client_tick_post",
                        part: None,
                    })
                };
                {
                    let _call = edit_texture();
                    game.create_dynamic_texture(
                        &wide,
                        64,
                        64,
                        1,
                        &[255, 0, 0, 255].repeat(64 * 64),
                    )
                    .unwrap();
                    game.create_dynamic_texture(
                        &slim,
                        64,
                        64,
                        1,
                        &[0, 0, 255, 255].repeat(64 * 64),
                    )
                    .unwrap();
                    game.create_dynamic_texture(&wrong_size, 32, 64, 1, &[255; 32 * 64 * 4])
                        .unwrap();
                }

                // A complete frame must sample the chosen sprite, not merely expose the answer
                // from the hook. Parts outside the sprite do not count (armor, items, GUI chrome).
                let draw = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, key: &str| {
                    let Rect { u0, v0, u1, v1, .. } = game.client.gui.atlas.sprites[key];
                    let inside = |[u, v]: [f32; 2]| u0 <= u && u < u1 && v0 <= v && v < v1;
                    let mut hand = Vec::new();
                    let mut body = false;
                    let mut preview = false;
                    let before_calls = choice.lock().unwrap().calls;
                    let rendered = game
                        .render_with(|_, frame| {
                            hand.extend(frame.hand.iter().filter(|v| inside(v.uv)).map(|v| v.pos));
                            body = frame.entities.iter().any(|v| inside(v.uv));
                            preview = frame.hud.vertices.iter().any(|v| inside(v.uv));
                            Ok(true)
                        })
                        .unwrap();
                    assert!(rendered);
                    assert_eq!(choice.lock().unwrap().calls, before_calls + 1);
                    let avatar = game.client.player_model.as_ref().unwrap();
                    assert_eq!(avatar.skin.as_deref(), Some(vanilla));
                    assert!(!avatar.slim, "the selected width must not escape the frame");
                    (hand, body, preview)
                };

                let vanilla_hand = draw(game, vanilla).0;
                assert!(!vanilla_hand.is_empty(), "the vanilla arm draws");
                choice.lock().unwrap().selection =
                    Some(PlayerSkinOverride::new(wide.clone(), PlayerModel::Wide));
                let wide_name = skin_slot_name(game, &wide);
                assert_eq!(draw(game, &wide_name).0, vanilla_hand);

                choice.lock().unwrap().selection =
                    Some(PlayerSkinOverride::new(slim.clone(), PlayerModel::Slim));
                let slim_name = skin_slot_name(game, &slim);
                let slim_hand = draw(game, &slim_name).0;
                assert!(!slim_hand.is_empty(), "the new texture draws on the arm");
                assert_ne!(
                    slim_hand, vanilla_hand,
                    "the slim arm has different geometry"
                );

                game.apply_camera_type(CameraType::ThirdPersonBack);
                game.client.screen = Screen::Inventory;
                let (_, body, preview) = draw(game, &slim_name);
                assert!(body, "the world avatar uses the selection");
                assert!(preview, "the inventory avatar uses that same selection");

                game.apply_camera_type(CameraType::FirstPerson);
                game.client.screen = Screen::Playing;
                choice.lock().unwrap().selection = None;
                assert_eq!(
                    draw(game, vanilla).0,
                    vanilla_hand,
                    "clearing restores vanilla"
                );
                choice.lock().unwrap().selection =
                    Some(PlayerSkinOverride::new(wrong_size, PlayerModel::Slim));
                assert_eq!(
                    draw(game, vanilla).0,
                    vanilla_hand,
                    "wrong size keeps both values"
                );
                choice.lock().unwrap().selection =
                    Some(PlayerSkinOverride::new(slim.clone(), PlayerModel::Slim));
                {
                    let _call = edit_texture();
                    game.release_dynamic_texture(&slim).unwrap();
                }
                assert_eq!(
                    draw(game, vanilla).0,
                    vanilla_hand,
                    "freed id keeps both values"
                );
            });
        },
    );
}
