#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_client_gameplay::game::client_side::item_use_client::tests::stand_in_first_person;

    #[test]
    #[ignore = "daily"]
    fn default_first_person_arm_obeys_its_own_sleeve_option() {
        use sunrise_client::options_file::V;
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        };

        if run_fresh_cases(
            "cut_layout::game::client_side::item_use_client::tests::default_first_person_arm_obeys_its_own_sleeve_option",
            "SUNRISE_HAND_SLEEVE_CASE",
            &["left-hidden"],
        ) {
            return;
        }
        with_client_game(|game| {
            stand_in_first_person(game);
            game.client.settings.main_hand_left = true;
            let left = game.hand_geometry(0.0).0;
            game.client.settings.main_hand_left = false;
            let right = game.hand_geometry(0.0).0;

            game.client
                .settings
                .file
                .set("modelPart_left_sleeve", V::Bool(false));
            game.tick_player_model();
            let right_hidden_left = game.hand_geometry(0.0).0;
            game.client.settings.main_hand_left = true;
            let left_hidden_left = game.hand_geometry(0.0).0;

            assert_eq!(
                (left.len(), right.len(), left_hidden_left.len()),
                (48, 48, 24)
            );
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&left[..24]),
                bytemuck::cast_slice::<_, u8>(&left_hidden_left)
            );
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&right),
                bytemuck::cast_slice::<_, u8>(&right_hidden_left)
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn render_hand_submits_both_skinned_arms_in_draw_order() {
        use glam::{Mat4, Quat, Vec3};
        use mod_api::{
            ClientHost, Flow, Hand, Hooks, MainHand, ModEntry, RenderArmEvent, RenderHandEvent,
        };
        use std::sync::{
            Arc,
            atomic::{AtomicU8, Ordering::Relaxed},
        };
        use sunrise_client::hud::Rect;
        use sunrise_items::items::ItemStack;
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        };

        if run_fresh_cases(
            "cut_layout::game::client_side::item_use_client::tests::render_hand_submits_both_skinned_arms_in_draw_order",
            "SUNRISE_HAND_ARM_CASE",
            &["wide", "slim"],
        ) {
            return;
        }
        let slim = std::env::var("SUNRISE_HAND_ARM_CASE").unwrap() == "slim";
        const MOVES: [[f32; 3]; 2] = [[0.75, -0.5, -1.0], [-0.25, 0.5, -2.0]];
        const LIGHT: [u32; 2] = [0x30a0, 0x60b0];
        const SCALE: [f32; 3] = [5e-7, 1e-8, 1e-8];
        #[derive(Default)]
        struct State {
            mode: AtomicU8,
            default_arms: AtomicU8,
        }
        struct Mod(Arc<State>);
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn render_arm(&self, _: &dyn ClientHost, _: &mut RenderArmEvent<'_>) -> Flow {
                self.0.default_arms.fetch_add(1, Relaxed);
                Flow::Continue
            }
            fn render_hand(&self, _: &dyn ClientHost, event: &mut RenderHandEvent<'_, '_>) -> Flow {
                let mode = self.0.mode.load(Relaxed);
                if event.render.hand != Hand::Main || mode == 0 {
                    return Flow::Continue;
                }
                let original_light = event.draw.light();
                for (i, arm) in [MainHand::Left, MainHand::Right].into_iter().enumerate() {
                    event.draw.push();
                    event.draw.translate(MOVES[i]);
                    if mode == 3 && i == 0 {
                        event.draw.scale(SCALE);
                        event.draw.rotate_about([0.0, 0.0, 1.0], 90.0, [0.0; 3]);
                    }
                    event.draw.set_light(LIGHT[i]);
                    event.draw.render_arm(arm);
                    event.draw.pop();
                }
                event.draw.set_light(original_light);
                if mode == 2 {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        let state = Arc::new(State::default());
        mod_platform::testing::with_mods(
            vec![ModEntry::new("arms", Box::new(Mod(state.clone())))].into(),
            || {
                with_client_game(|game| {
                    stand_in_first_person(game);
                    for (i, (slot, item)) in [
                        (game.selected, "minecraft:iron_sword"),
                        (
                            sunrise_gameplay_common::game::held_items::OFFHAND_SLOT,
                            "minecraft:stick",
                        ),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let stack = ItemStack::new(item, 1);
                        *game.stack_in_mut(slot) = Some(stack.clone());
                        game.client.visible_hands[i] = (slot, Some(stack));
                    }
                    let atlas = Rect {
                        u0: 0.25,
                        v0: 0.125,
                        u1: 0.75,
                        v1: 0.375,
                        w: 64,
                        h: 64,
                    };
                    game.client
                        .gui
                        .atlas
                        .sprites
                        .insert("arms:fixture_skin".into(), atlas);
                    let avatar = game.client.player_model.as_mut().unwrap();
                    avatar.skin = Some("arms:fixture_skin".into());
                    avatar.slim = slim;
                    let (vanilla, runs) = game.hand_geometry(0.0);
                    let [_, (offhand_at, offhand_id)] = runs[..] else {
                        panic!("expected both item runs: {runs:?}")
                    };
                    assert!(offhand_at > 0 && (offhand_at as usize) < vanilla.len());
                    state.mode.store(1, Relaxed);
                    let (added, added_runs) = game.hand_geometry(0.0);
                    state.mode.store(2, Relaxed);
                    let (replaced, replaced_runs) = game.hand_geometry(0.0);
                    assert_eq!(state.default_arms.load(Relaxed), 0);

                    // Six quads for the arm and six for the sleeve. Check each layer's first
                    // corner from PlayerModel's cube origins/texture offsets, including 3px arms.
                    let width = if slim { 3.0 } else { 4.0 };
                    let quads = 6 * 4;
                    let arm_vertices = quads * 2;
                    let submitted = arm_vertices * 2;
                    let view = game.client.sway.to_world(0.0, &game.client.view);
                    for (i, (shoulder, roll, x, base_u, sleeve_u, v)) in [
                        (
                            5.0,
                            -0.1,
                            width - 1.0,
                            32.0 + 4.0 + width,
                            48.0 + 4.0 + width,
                            48.0,
                        ),
                        (-5.0, 0.1, 1.0, 40.0 + 4.0 + width, 40.0 + 4.0 + width, 16.0),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let world = view
                            * Mat4::from_translation(Vec3::from_array(MOVES[i]))
                            * Mat4::from_rotation_translation(
                                Quat::from_rotation_z(roll),
                                Vec3::new(shoulder / 16.0, 2.0 / 16.0, 0.0),
                            );
                        let arm = &added[i * arm_vertices..(i + 1) * arm_vertices];
                        for (vertex, inflate, u, v) in [
                            (arm[0], 0.0, base_u, v),
                            (
                                arm[quads],
                                0.25,
                                sleeve_u,
                                if i == 0 { v } else { v + 16.0 },
                            ),
                        ] {
                            let p = Vec3::new(x + inflate, -2.0 - inflate, 2.0 + inflate) / 16.0;
                            assert_eq!(
                                vertex.pos.map(f32::to_bits),
                                world.transform_point3(p).to_array().map(f32::to_bits)
                            );
                            assert_eq!(vertex.uv, [0.25 + u / 128.0, 0.125 + v / 256.0]);
                        }
                        assert!(
                            arm.iter()
                                .all(|vertex| vertex.light == LIGHT[i] && vertex.layer == 0)
                        );
                    }
                    assert_eq!(
                        &added_runs[..],
                        &[
                            (0, 0),
                            (submitted as u32, 0),
                            (submitted as u32 + offhand_at, offhand_id)
                        ]
                    );
                    assert_eq!(
                        &replaced_runs[..],
                        &[(0, 0), (submitted as u32, offhand_id)]
                    );
                    assert_eq!(
                        bytemuck::cast_slice::<_, u8>(&added[submitted..]),
                        bytemuck::cast_slice::<_, u8>(&vanilla)
                    );
                    assert_eq!(
                        bytemuck::cast_slice::<_, u8>(&replaced[..submitted]),
                        bytemuck::cast_slice::<_, u8>(&added[..submitted])
                    );
                    assert_eq!(
                        bytemuck::cast_slice::<_, u8>(&replaced[submitted..]),
                        bytemuck::cast_slice::<_, u8>(&vanilla[offhand_at as usize..])
                    );

                    state.mode.store(3, Relaxed);
                    let (scaled, _) = game.hand_geometry(0.0);
                    // ModelPart.Cube's first polygon faces DOWN. The left arm's
                    // -0.1 roll followed by the listener's quarter turn sends -Y
                    // toward +X and -Y. Scale normals by the reciprocal axes.
                    let normal = view
                        .transform_vector3(Vec3::new(
                            sunrise_core::jmath::cos(0.1) as f32 / SCALE[0],
                            -(sunrise_core::jmath::sin(0.1) as f32) / SCALE[1],
                            0.0,
                        ))
                        .normalize();
                    let packed = normal.to_array().map(|n| (n * 127.0) as i8);
                    assert!(scaled[..4].iter().all(|v| v.normal[..3] == packed));
                    assert_eq!(
                        bytemuck::cast_slice::<_, u8>(&scaled[arm_vertices..submitted]),
                        bytemuck::cast_slice::<_, u8>(&added[arm_vertices..submitted]),
                        "pop restores the unscaled normal pose for the second arm"
                    );

                    game.client.gui.atlas.sprites.remove("arms:fixture_skin");
                    for missing_avatar in [false, true] {
                        if missing_avatar {
                            game.client.player_model = None;
                        }
                        state.mode.store(0, Relaxed);
                        let (items, item_runs) = game.hand_geometry(0.0);
                        state.mode.store(1, Relaxed);
                        let (drawn, drawn_runs) = game.hand_geometry(0.0);
                        assert_eq!(drawn_runs, item_runs);
                        assert_eq!(
                            bytemuck::cast_slice::<_, u8>(&drawn),
                            bytemuck::cast_slice::<_, u8>(&items),
                            "no arm without the selected sprite or the avatar state"
                        );
                    }
                });
            },
        );
    }
}
