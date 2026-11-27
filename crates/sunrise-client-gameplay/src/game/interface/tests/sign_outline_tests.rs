use super::{BlockEntity, CacheOptions, Vertex, in_own_process};
use glam::DVec3;
use sunrise_client::gfx::{TEXT_NORMAL_SOLID_OVERLAY, TEXT_POLYGON_OFFSET_SOLID_OVERLAY};
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::{Chunk, Light};
use sunrise_level::{camera_math::CameraType, signs_data::SignData};
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C},
};
use sunrise_sim::{entities::ArmorStand, motion::PlayerMotion};

#[test]
#[ignore = "daily"]
fn sign_outline_follows_scope_and_camera_subject_in_both_draw_paths() {
    in_own_process(
        "game::client_side::sign_outline_tests::sign_outline_follows_scope_and_camera_subject_in_both_draw_paths",
        || super::native_entity_refs::tests::with_client_game(check_signs),
    );
}

fn check_signs(game: &mut crate::game::ClientGame<'_>) {
    const POS: [i32; 3] = [0, 0, 0];
    const NEAR: DVec3 = DVec3::new(0.5, 0.5, 0.5);
    const FAR: DVec3 = DVec3::new(18.5, 0.5, 0.5);
    const EDGE: DVec3 = DVec3::new(16.5, 0.5, 0.5);
    const PARTIAL: f32 = 0.25;
    const SUBJECT: i32 = 31;
    let (client, _server) = Pipe::pair();
    game.connect_local(client, "SignOutline").unwrap();
    let player_id = game.client.remote.as_ref().unwrap().entity_id;
    let motion = game.motion;
    let player = || PlayerMotion::new(motion, Vec3::new(FAR.x, FAR.y, FAR.z)).unwrap();
    let mut chunk = Chunk::empty(0, 0);
    let section = chunk.section_index(POS[1]).unwrap();
    std::sync::Arc::make_mut(&mut chunk.sections[section]).light = Light::Uniform(0x83);
    game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
    let set_camera = |game: &mut crate::game::ClientGame<'_>, camera_id| {
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id }))
            .unwrap();
    };
    let remove = |game: &mut crate::game::ClientGame<'_>, id| {
        game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![id] }))
            .unwrap();
    };

    let mut observed = Vec::new();
    for (block, hanging) in [
        ("minecraft:oak_sign", false),
        ("minecraft:oak_hanging_sign", true),
    ] {
        let state = game.pack.assets.registry.default_state(block).unwrap();
        let sign = |color, glowing| {
            let mut sign = SignData {
                hanging,
                ..Default::default()
            };
            sign.front.lines[0] = "A".into();
            sign.front.color = color;
            sign.front.glowing = glowing;
            BlockEntity::Sign(sign)
        };
        // One red glyph. Holding the sign data, position and light fixed across the
        // series exercises cached vertices on both sides of an outline transition.
        game.world.block_entities.insert((0, 0, 0), sign(14, true));
        let cases = [
            (
                "first_person_active_scope",
                CameraType::FirstPerson,
                true,
                Some(FAR),
                FAR,
                9,
            ),
            (
                "third_person_active_scope",
                CameraType::ThirdPersonBack,
                true,
                Some(FAR),
                NEAR,
                1,
            ),
            (
                "passive_spyglass",
                CameraType::FirstPerson,
                false,
                Some(FAR),
                NEAR,
                1,
            ),
            (
                "just_inside",
                CameraType::FirstPerson,
                false,
                Some(DVec3::new(EDGE.x.next_down(), EDGE.y, EDGE.z)),
                FAR,
                9,
            ),
            (
                "exact_distance",
                CameraType::FirstPerson,
                false,
                Some(EDGE),
                NEAR,
                1,
            ),
            (
                "eye_height",
                CameraType::FirstPerson,
                false,
                Some(DVec3::new(0.5, 16.25, 0.5)),
                FAR,
                9,
            ),
            ("no_subject", CameraType::FirstPerson, false, None, NEAR, 1),
            (
                "scoping_without_subject",
                CameraType::FirstPerson,
                true,
                None,
                FAR,
                9,
            ),
            (
                "local_camera",
                CameraType::ThirdPersonFront,
                false,
                None,
                FAR,
                9,
            ),
        ];
        for cached in [true, false] {
            for (name, camera_type, scoping, subject, visual, expected) in cases {
                game.player = Some(player());
                game.client.camera_type = camera_type;
                game.using = None;
                game.inventory[0] = Some(ItemStack::new("minecraft:spyglass", 1));
                if scoping {
                    assert!(game.start_using_item());
                }
                set_camera(game, player_id);
                game.armor_stands.clear();
                if let Some(subject) = subject {
                    let at = Vec3::new(subject.x, subject.y, subject.z);
                    let mut stand = ArmorStand::new(SUBJECT as u32, at, 0.0);
                    // The previous position deliberately disagrees for a threshold case.
                    if name == "exact_distance" {
                        stand.previous = Vec3::new(NEAR.x, NEAR.y, NEAR.z);
                    }
                    game.armor_stands.push(stand);
                    set_camera(game, SUBJECT);
                }
                let visual = match name {
                    "eye_height" => {
                        game.client.view.eye_height = 1.7775;
                        game.client.view.eye_height_old = game.client.view.eye_height;
                        let (camera, _, _) = game.camera_view(PARTIAL).unwrap();
                        assert!((camera - NEAR).length_squared() > 16.0 * 16.0);
                        camera
                    }
                    "exact_distance" => {
                        let (camera, _, _) = game.camera_view(PARTIAL).unwrap();
                        assert!((camera - NEAR).length_squared() < 16.0 * 16.0);
                        camera
                    }
                    "no_subject" => {
                        game.player = None;
                        visual
                    }
                    "scoping_without_subject" => {
                        // Loss of the held object before a new camera subject is available.
                        game.armor_stands
                            .push(ArmorStand::new(SUBJECT as u32, Vec3::ZERO, 0.0));
                        set_camera(game, SUBJECT);
                        remove(game, SUBJECT);
                        visual
                    }
                    "local_camera" => {
                        game.player.as_mut().unwrap().position = Vec3::new(NEAR.x, NEAR.y, NEAR.z);
                        visual
                    }
                    _ => visual,
                };
                let quads = render_glyphs(game, (POS, state.0), visual, PARTIAL, cached, 'A').len();
                println!("{block} {cached} {name}: {quads}");
                observed.push((block, cached, name, quads, expected));
            }
        }
        // Without the distance or scope predicate, black glowing still has its eight
        // outline copies. Plain text never has them, even nearby.
        game.player = Some(player());
        game.using = None;
        for (name, color, glowing, at, expected) in [
            ("black_glowing_far", 15, true, FAR, 9),
            ("plain_near", 14, false, NEAR, 1),
        ] {
            game.player.as_mut().unwrap().position = Vec3::new(at.x, at.y, at.z);
            game.world
                .block_entities
                .insert((0, 0, 0), sign(color, glowing));
            for cached in [true, false] {
                let quads = render_glyphs(game, (POS, state.0), at, PARTIAL, cached, 'A').len();
                println!("{block} {cached} {name}: {quads}");
                observed.push((block, cached, name, quads, expected));
            }
        }

        // The front and back are independent. Keep their data and light fixed while
        // the back gains and loses its outline, in both standing and hanging paths.
        game.world.block_entities.insert(
            (0, 0, 0),
            BlockEntity::Sign(SignData {
                hanging,
                front: sunrise_level::signs_data::SignText {
                    color: 13,
                    lines: ["A".into(), String::new(), String::new(), String::new()],
                    ..Default::default()
                },
                back: sunrise_level::signs_data::SignText {
                    color: 14,
                    glowing: true,
                    lines: ["B".into(), String::new(), String::new(), String::new()],
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
        game.client.camera_type = CameraType::FirstPerson;
        game.player = Some(player());
        let cart_kind = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:minecart")
            .unwrap();
        for cached in [true, false] {
            let check = |game: &crate::game::ClientGame<'_>, outlined| {
                let glyphs = |character| {
                    render_glyphs(game, (POS, state.0), FAR, PARTIAL, cached, character)
                };
                assert_eq!(
                    glyphs('A'),
                    [(0xFF00_6600, 0x3080, TEXT_POLYGON_OFFSET_SOLID_OVERLAY)],
                    "{block} front, cached={cached}"
                );
                let mut expected_back = Vec::new();
                if outlined {
                    expected_back.extend([(0xFF00_0066, 0xF0F0, TEXT_NORMAL_SOLID_OVERLAY); 8]);
                }
                expected_back.push((0xFF00_00FF, 0xF0F0, TEXT_POLYGON_OFFSET_SOLID_OVERLAY));
                assert_eq!(glyphs('B'), expected_back, "{block} back, cached={cached}");
            };
            set_camera(game, player_id);
            check(game, false);

            // Replace the stack before the first use tick. The active item remains
            // the captured one until updatingUsingItem stops the use (in either direction).
            for (start, replacement, starts_scoped) in [
                ("minecraft:apple", "minecraft:spyglass", false),
                ("minecraft:spyglass", "minecraft:apple", true),
            ] {
                game.inventory[0] = Some(ItemStack::new(start, 1));
                assert!(game.start_using_item());
                check(game, starts_scoped);
                game.inventory[0] = Some(ItemStack::new(replacement, 1));
                check(game, starts_scoped);
                game.remote_tick_using();
                assert!(game.using.is_none());
                check(game, false);
            }

            // Switching slots while holding use keeps the current hand active
            // for the same item, even though the original stack is still in slot 0.
            game.inventory[0] = Some(ItemStack::new("minecraft:spyglass", 1));
            game.set_mapping_held("key.use", true);
            for (replacement, stays_scoped) in
                [("minecraft:spyglass", true), ("minecraft:apple", false)]
            {
                game.inventory[1] = Some(ItemStack::new(replacement, 1));
                assert!(game.start_using_item());
                game.selected = 1;
                check(game, true);
                game.remote_tick_using();
                assert_eq!(game.using, stays_scoped.then_some((1, 1)));
                check(game, stays_scoped);
                game.selected = 0;
                game.using = None;
            }
            game.set_mapping_held("key.use", false);

            const CART: i32 = 32;
            game.handle_packet(C::AddEntity(play::AddEntity {
                id: CART,
                kind: cart_kind,
                uuid: CART as _,
                position: [2.5, 0.5, 0.5],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
            set_camera(game, CART);
            check(game, true);
            remove(game, CART);

            game.armor_stands.push(ArmorStand::new(
                SUBJECT as u32,
                Vec3::new(2.5, 0.5, 0.5),
                0.0,
            ));
            set_camera(game, SUBJECT);
            check(game, true);
            // Move after selection: removal must keep the last base position, not the first.
            game.armor_stands.last_mut().unwrap().position.x = FAR.x;
            check(game, false);
            game.armor_stands.last_mut().unwrap().position.x = 4.5;
            remove(game, SUBJECT);
            assert!(game.armor_stands.is_empty());
            assert_eq!(
                game.camera_subject().unwrap().camera.position,
                [4.5, 0.5, 0.5]
            );
            check(game, true);
            set_camera(game, SUBJECT); // Removed id is not a valid switch.
            check(game, true);
            set_camera(game, player_id);
            check(game, false);
        }
    }
    for (block, cached, name, actual, expected) in observed {
        assert_eq!(actual, expected, "{block} {name}, cached={cached}");
    }
}

fn render_glyphs(
    game: &crate::game::ClientGame<'_>,
    block: ([i32; 3], u32),
    visual: DVec3,
    partial: f32,
    cached: bool,
    character: char,
) -> Vec<(u32, u32, u32)> {
    let renderer = game.client_res().block_entity_render();
    let blocks = [block];
    let mut out: Vec<Vertex> = Vec::new();
    game.with_block_entity_scene(visual, partial, &|_, _, _, _| true, |scene| {
        if cached {
            renderer.render_with_gpu_cache_options(
                scene,
                blocks.iter(),
                std::iter::empty(),
                CacheOptions {
                    signs: true,
                    ..Default::default()
                },
                0,
                |_| None,
                &mut out,
            );
        } else {
            renderer.render_with_cache_options_and_callbacks(
                scene,
                blocks.iter(),
                CacheOptions::default(),
                0,
                None,
                &mut out,
                |_, _, _| {},
                |_, _, _, _| {},
            );
        }
    });
    let glyph = game
        .client
        .gui
        .atlas
        .font
        .glyph_styled(character, &Default::default());
    let cell = glyph.rect.expect("the prepared font has a glyph");
    out.as_chunks::<4>()
        .0
        .iter()
        .filter(|q| q[0].normal == [0; 4] && q[0].uv == [cell.u0, cell.v1])
        .map(|q| {
            let v = &q[0];
            let draw = (v.color, v.light, v.layer & 0xFFFF);
            assert!(
                q.iter()
                    .all(|v| (v.color, v.light, v.layer & 0xFFFF) == draw)
            );
            draw
        })
        .collect()
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
