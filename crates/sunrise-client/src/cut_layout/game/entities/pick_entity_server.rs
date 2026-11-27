#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_gameplay_common::game::creatures::SpawnSite;
    use sunrise_gameplay_common::game::dragon_common::dragon_part_id;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::pick_entity_common::PickEntityCommonGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::motion::PlayerMotion;

    #[test]
    #[ignore = "daily"]
    fn an_entitys_pick_result_reads_through_its_handle() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::pick_entity_server::tests::an_entitys_pick_result_reads_through_its_handle",
            || {
                with_game(|game| {
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap());
                    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(
                        game, 20, 123,
                    );
                    // The summoned entities resolve in a loaded chunk.
                    game.world.chunks.insert(
                        (0, 0),
                        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                    );
                    game.block_ticks
                        .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                        .unwrap();
                    let mut summon = |kind: &str, nbt: &str| match game
                        .summon_with_nbt(kind, [2.5, 65.0, 2.5], nbt)
                        .unwrap()
                    {
                        Some(
                            Target::Mob(id)
                            | Target::Stand(id)
                            | Target::Boat(id)
                            | Target::Frame(id)
                            | Target::Projectile(id),
                        ) => id,
                        other => panic!("{kind} summoned as {other:?}"),
                    };
                    let cases = [
                        (
                            summon("minecraft:zombie", "{}"),
                            Some("minecraft:zombie_spawn_egg"),
                        ),
                        (
                            summon("minecraft:armor_stand", "{}"),
                            Some("minecraft:armor_stand"),
                        ),
                        (
                            summon("minecraft:oak_boat", "{}"),
                            Some("minecraft:oak_boat"),
                        ),
                        (
                            summon(
                                "minecraft:item_frame",
                                r#"{Item:{id:"minecraft:diamond",count:1}}"#,
                            ),
                            Some("minecraft:diamond"),
                        ),
                        (summon("minecraft:arrow", "{}"), None),
                        (20, None),
                    ];
                    for (id, want) in cases {
                        let entity = game
                            .entity(id)
                            .unwrap_or_else(|| panic!("no handle for {id}"));
                        let picked = entity.pick_result(game).map(|stack| {
                            mod_api::__private::item_key_id(stack.item())
                                .unwrap()
                                .to_owned()
                        });
                        assert_eq!(picked.as_deref(), want, "entity {id}");
                    }
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn server_entity_pick_returns_lead_and_delegates_dragon_parts() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::pick_entity_server::tests::server_entity_pick_returns_lead_and_delegates_dragon_parts",
            || {
                with_game(|game| {
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap());
                    let knot = game.tie_knot([0, 65, 2]);
                    let far_knot = game.tie_knot([0, 65, 10]);
                    let Some(Target::Other(explicit_knot)) = game
                        .summon_with_nbt("minecraft:leash_knot", [1.5, 65.0, 2.5], "{}")
                        .unwrap()
                    else {
                        panic!("expected an explicit leash knot");
                    };
                    for id in [knot, explicit_knot] {
                        assert!(
                            game.within_entity_range(id as i32),
                            "knot {id} is reachable"
                        );
                        assert_eq!(
                            game.pick_result_of(id as i32),
                            Some(ItemStack::new("minecraft:lead", 1))
                        );
                    }
                    assert!(
                        !game.within_entity_range(far_knot as i32),
                        "far knot outside reach+3 must fail server reach"
                    );
                    // Cross the signed i32 boundary so later dragon parts carry negative wire IDs.
                    game.server.next_entity = (i32::MAX as u32) - 3;
                    let dragon = game
                        .spawn_dragon_facing([0.5, 65.0, 3.5], SpawnSite::Plain, Some(0.0))
                        .unwrap();
                    let parent_result = game.pick_result_of(dragon as i32);
                    assert_eq!(
                        parent_result,
                        Some(ItemStack::new("minecraft:ender_dragon_spawn_egg", 1))
                    );
                    let body_part_id = dragon_part_id(dragon, 2) as i32;
                    assert!(body_part_id < 0, "expected negative signed part wire ID");
                    assert!(game.within_entity_range(body_part_id));
                    for part in 0..8 {
                        assert_eq!(
                            game.pick_result_of(dragon_part_id(dragon, part) as i32),
                            parent_result
                        );
                    }

                    // Exercise the real PickItemFromEntity server packet handler through a creative guest.
                    let (mut hub, _rx, _other) =
                        sunrise_gameplay::game::server_side::lan_particles::tests::test_hub(
                            "PickEntity",
                            Vec::new(),
                            None,
                        );
                    let gi = hub.guests.connection_index(1).unwrap();
                    hub.guests.update(gi, |g| {
                        g.game_mode = 1;
                        g.position = [0.5, 65.0, 0.5];
                        g.placed = true;
                        g.dimension = "minecraft:overworld".into();
                        g.inventory[0] = None;
                        g.selected = 0;
                    });
                    game.server.lan = Some(
                        sunrise_gameplay::game::server_side::lan::LanHost::for_test(hub),
                    );

                    for (id, expected) in [
                        (far_knot as i32, None),
                        (knot as i32, Some("minecraft:lead")),
                        (explicit_knot as i32, Some("minecraft:lead")),
                        (body_part_id, Some("minecraft:ender_dragon_spawn_egg")),
                    ] {
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(gi, |g| {
                                g.inventory[0] = None;
                                g.selected = 0;
                            });
                        game.lan_guest_packet(
                            1,
                            sunrise_protocol::packets::play::Serverbound::PickItemFromEntity(
                                sunrise_protocol::packets::play::PickItemFromEntity {
                                    id,
                                    include_data: false,
                                },
                            ),
                        );
                        let held = game.server.lan.as_ref().unwrap().hub.guests[gi].inventory[0]
                            .as_ref()
                            .map(|s| &*s.item);
                        assert_eq!(held, expected, "PickItemFromEntity({id})");
                    }
                });
            },
        );
    }
}
