#![cfg(test)]
#[cfg(test)]
#[cfg(feature = "client")]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1;

    #[test]
    #[ignore = "daily"]
    fn equipment_camera_overlays_emit_in_vanilla_slot_order() {
        use sunrise_client::hud::Rect;
        use sunrise_core::{geometry::Vec3, nbt::Tag};
        use sunrise_items::items::ItemStack;
        use sunrise_level::camera_math::CameraType;
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_gameplay_common::game::in_own_process,
        };

        in_own_process(
            "cut_layout::game::client_side::render_tick::tests::equipment_camera_overlays_emit_in_vanilla_slot_order",
            || {
                with_client_game(|game| {
                    game.player = Some(
                        sunrise_sim::motion::PlayerMotion::new(
                            game.motion,
                            Vec3::new(0.5, 64.0, 0.5),
                        )
                        .unwrap(),
                    );
                    game.client.camera_type = CameraType::FirstPerson;
                    game.client.chat_ui.hide_gui = false;
                    game.client.gui.atlas.sprites.clear();
                    let expected = [0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875];
                    let mut stack = |slot: &str, u0: f32| {
                        let overlay = format!("misc/test_{slot}");
                        game.client.gui.atlas.sprites.insert(
                            overlay.clone(),
                            Rect {
                                u0,
                                v0: 0.0,
                                u1: u0 + 0.125,
                                v1: 1.0,
                                w: 16,
                                h: 16,
                            },
                        );
                        let mut stack = ItemStack::new("minecraft:stone", 1);
                        stack.data_mut().set_other(
                            "minecraft:equippable",
                            Some(Tag::compound([
                                ("slot", Tag::String(slot.into())),
                                ("camera_overlay", Tag::String(overlay.as_str().into())),
                            ])),
                        );
                        Some(stack)
                    };
                    let mainhand = stack("mainhand", expected[0]);
                    let offhand = stack("offhand", expected[1]);
                    let feet = stack("feet", expected[2]);
                    let legs = stack("legs", expected[3]);
                    let chest = stack("chest", expected[4]);
                    let head = stack("head", expected[5]);
                    let body = stack("body", expected[6]);
                    let saddle = stack("saddle", expected[7]);
                    game.selected = 4;
                    game.inventory[4] = mainhand;
                    game.offhand = offhand;
                    game.armor = [head, chest, legs, feet];
                    game.equipment.set(sunrise_items::equipment::BODY, body);
                    game.equipment.set(sunrise_items::equipment::SADDLE, saddle);
                    let quads = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                        game.screen_quads(
                            0.0,
                            [427, 240],
                            1.0,
                            glam::Mat4::IDENTITY.to_cols_array(),
                        )
                    };
                    let drawn = quads(game);
                    assert_eq!(
                        drawn
                            .iter()
                            .map(|q| q.vertices[0].rect[0])
                            .collect::<Vec<_>>(),
                        expected
                    );
                    for q in drawn {
                        assert_eq!(q.vertices[0].pos[..2], [-1.0, 1.0]);
                        assert_eq!(q.vertices[2].pos[..2], [1.0, -1.0]);
                    }

                    game.client.camera_type = CameraType::ThirdPersonBack;
                    assert!(
                        quads(game).is_empty(),
                        "third-person suppresses equipment overlays"
                    );
                    game.client.camera_type = CameraType::FirstPerson;
                    game.inventory[4] = Some(ItemStack::new("minecraft:spyglass", 1));
                    assert_eq!(
                        quads(game).len(),
                        7,
                        "remaining seven slots still draw before scoping"
                    );
                    assert!(game.start_using_item(), "spyglass use starts");
                    assert!(
                        quads(game).is_empty(),
                        "scoping suppresses equipment overlays"
                    );

                    // SetPlayerInventory slots 41 and 42 sync body armor and saddle.
                    game.init_remote_for_replay().unwrap();
                    let item_id = |name: &str| {
                        game.client
                            .remote
                            .as_ref()
                            .unwrap()
                            .builtin
                            .get("minecraft:item")
                            .and_then(|i| i.id(name))
                            .unwrap()
                    };
                    let body_id = item_id("minecraft:wolf_armor");
                    let saddle_id = item_id("minecraft:saddle");
                    game.handle_packet(
                        sunrise_protocol::packets::play::Clientbound::SetPlayerInventory(
                            sunrise_protocol::packets::play::SetPlayerInventory {
                                slot: 41,
                                item: Some(sunrise_protocol::item::Stack::new(body_id, 1)),
                            },
                        ),
                    )
                    .unwrap();
                    game.handle_packet(
                        sunrise_protocol::packets::play::Clientbound::SetPlayerInventory(
                            sunrise_protocol::packets::play::SetPlayerInventory {
                                slot: 42,
                                item: Some(sunrise_protocol::item::Stack::new(saddle_id, 1)),
                            },
                        ),
                    )
                    .unwrap();
                    assert_eq!(
                        game.equipment
                            .get(sunrise_items::equipment::BODY)
                            .map(|s| s.item.as_ref()),
                        Some("minecraft:wolf_armor")
                    );
                    assert_eq!(
                        game.equipment
                            .get(sunrise_items::equipment::SADDLE)
                            .map(|s| s.item.as_ref()),
                        Some("minecraft:saddle")
                    );
                });
            },
        );
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "tests/ambient_particle_tests.rs"]
mod ambient_particle_tests;
