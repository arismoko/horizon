#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_client_gameplay::game::remote::general::tests::row_ids;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1;

    use sunrise_client_gameplay::game::remote::general::tests::rows;

    use sunrise_gameplay_common::game::remote::general::tests::FUEL_CODEC;

    use sunrise_gameplay_common::game::remote::general::tests::Declare;

    use sunrise_gameplay_common::game::remote::general::tests::attachment_message;

    use sunrise_gameplay_common::game::remote::general::tests::spawn;

    use sunrise_gameplay_common::game::remote::general::tests::spawn_facing;

    use sunrise_mod_runtime::attachments::{Attachments, Holder};

    use sunrise_protocol::packets::play::{self, Clientbound as C};

    #[test]
    #[ignore = "daily"]
    fn riding_reads_follow_the_server_passenger_lists() {
        use mod_api::{ClientHost, EntityReadError, EntityRef, WorldEpoch};
        use std::ops::ControlFlow;
        use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
        use sunrise_gameplay_common::game::test_fixtures::collect_entity_visit;

        if run_fresh_cases(
            "cut_layout::game::remote::general::tests::riding_reads_follow_the_server_passenger_lists",
            "SUNRISE_CLIENT_RIDING_TEST_CHILD",
            &["riding"],
        ) {
            return;
        }
        mod_platform::__private::initialize().unwrap();
        let _phase = mod_platform::__private::RegistrationGuard::open();
        let declare = || vec![mod_api::ModEntry::new("mymod", Box::new(Declare))];
        mod_platform::testing::with_mods(declare().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&declare()).unwrap();
            crate::game::client_side::item_entities_client::thrown_items_tests::with_remote_game(
                |game| {
                    let level = || play::SpawnInfo {
                        dimension_type: 0,
                        dimension: "minecraft:overworld".into(),
                        seed: 1,
                        game_type: 0,
                        previous_game_type: None,
                        debug: false,
                        flat: false,
                        last_death: None,
                        portal_cooldown: 0,
                        sea_level: 63,
                    };
                    let position = |teleport| {
                        C::PlayerPosition(play::PlayerPosition {
                            teleport,
                            position: [0.5, 70.0, 0.5],
                            delta: [0.0; 3],
                            yaw: 0.0,
                            pitch: 0.0,
                            relatives: 0,
                        })
                    };
                    game.handle_packet(C::Login(play::Login {
                        player_id: 7,
                        hardcore: false,
                        levels: vec!["minecraft:overworld".into()],
                        max_players: 20,
                        chunk_radius: 8,
                        simulation_distance: 8,
                        reduced_debug_info: false,
                        show_death_screen: true,
                        limited_crafting: false,
                        spawn: level(),
                        online_mode: false,
                        enforces_secure_chat: false,
                    }))
                    .unwrap();
                    game.handle_packet(position(1)).unwrap();
                    game.handle_packet(spawn_facing(9, "mymod:missile", [2.5, 70.0, 0.5], 90.0))
                        .unwrap();
                    game.handle_packet(spawn(12, "mymod:missile", [0.5, 70.0, 0.5]))
                        .unwrap();
                    game.spawn_entity(4, "minecraft:area_effect_cloud", [0.5, 70.0, 0.5], [0.0; 3]);
                    game.spawn_entity(5, "minecraft:marker", [0.5, 70.0, 0.5], [0.0; 3]);
                    game.spawn_entity(
                        6,
                        "minecraft:ominous_item_spawner",
                        [0.5, 70.0, 0.5],
                        [0.0; 3],
                    );
                    let set = |vehicle, passengers| {
                        C::SetPassengers(play::SetPassengers {
                            vehicle,
                            passengers,
                        })
                    };
                    let vehicle = game.entity(9).unwrap();
                    let row = game.entity(12).unwrap();
                    let cloud = game.entity(4).unwrap();
                    let me = game.entity(7).unwrap();
                    let unknown = EntityRef::new(vehicle.epoch(), 99);
                    let stale = EntityRef::new(
                        WorldEpoch::new(vehicle.epoch().get().wrapping_add(1)),
                        vehicle.id(),
                    );
                    game.handle_packet(set(9, vec![99])).unwrap();
                    let host: &dyn ClientHost = game;
                    assert_eq!(
                        vehicle.visit_passengers(host, &mut |_| panic!("empty seat list visited")),
                        Ok(ControlFlow::Continue(()))
                    );
                    assert_eq!(vehicle.is_vehicle(host), Some(false));

                    game.handle_packet(set(9, vec![12, 99, 7])).unwrap();
                    game.handle_packet(set(12, vec![4])).unwrap();
                    game.position_riders();
                    assert_eq!(row.position(game), Some([2.5, 71.25, 1.5]));
                    let host: &dyn ClientHost = game;
                    assert_eq!(
                        collect_entity_visit(|visit| vehicle.visit_passengers(host, visit)),
                        Ok(vec![row, me])
                    );
                    let mut visited = None;
                    assert_eq!(
                        vehicle.visit_passengers(host, &mut |rider| {
                            assert!(visited.replace(rider).is_none());
                            ControlFlow::Break(())
                        }),
                        Ok(ControlFlow::Break(()))
                    );
                    assert_eq!(visited, Some(row));
                    assert_eq!(
                        vehicle.visit_indirect_passengers(host, &mut |_| {
                            panic!("client walked the tree")
                        }),
                        Err(EntityReadError::Unavailable)
                    );
                    assert_eq!(vehicle.is_vehicle(host), Some(true));
                    assert_eq!(row.vehicle(host), Some(vehicle));
                    assert_eq!(me.vehicle(host), Some(vehicle));
                    assert_eq!(cloud.is_passenger(host), Some(true));
                    assert_eq!(cloud.root_vehicle(host), Some(vehicle));
                    assert_eq!(unknown.vehicle(host), None);
                    assert_eq!(
                        unknown.visit_passengers(host, &mut |_| {
                            panic!("untracked vehicle visited")
                        }),
                        Err(EntityReadError::Stale)
                    );
                    assert_eq!(
                        stale.visit_passengers(host, &mut |_| panic!("stale vehicle visited")),
                        Err(EntityReadError::Stale)
                    );
                    assert_eq!(stale.is_vehicle(host), None);

                    // An unknown vehicle can't displace a real relationship. Later tracking
                    // doesn't apply either of its old packets or its mention as a passenger.
                    game.handle_packet(set(99, vec![12])).unwrap();
                    assert_eq!(row.vehicle(game), Some(vehicle));
                    game.spawn_entity(99, "minecraft:marker", [0.5, 70.0, 0.5], [0.0; 3]);
                    let late = game.entity(99).unwrap();
                    assert_eq!(late.is_passenger(game), Some(false));
                    assert_eq!(late.is_vehicle(game), Some(false));
                    assert_eq!(
                        collect_entity_visit(|visit| vehicle.visit_passengers(game, visit)),
                        Ok(vec![row, me])
                    );

                    game.handle_packet(set(4, vec![9])).unwrap();
                    assert_eq!(vehicle.vehicle(game), None);
                    assert_eq!(cloud.is_vehicle(game), Some(false));
                    assert_eq!(cloud.root_vehicle(game), Some(vehicle));

                    // Known entities that cannot accept a passenger do not displace
                    // an existing ride either, even on a forced client mount.
                    for refused in [5, 6] {
                        let cannot_carry = game.entity(refused).unwrap();
                        game.handle_packet(set(refused as i32, vec![12])).unwrap();
                        assert_eq!(
                            collect_entity_visit(|visit| cannot_carry.visit_passengers(game, visit)),
                            Ok(vec![])
                        );
                        assert_eq!(row.vehicle(game), Some(vehicle));
                    }

                    game.spawn_entity(20, "minecraft:happy_ghast", [2.5, 70.0, 0.5], [0.0; 3]);
                    let ghast = game.entity(20).unwrap();
                    game.handle_packet(set(20, vec![7, 12])).unwrap();
                    game.position_riders();
                    assert_eq!(row.position(game), Some([2.5 - 1.7, 74.0, 0.5]));
                    game.handle_packet(set(9, vec![12])).unwrap();
                    assert_eq!(me.vehicle(game), Some(ghast));
                    game.handle_packet(set(7, vec![99])).unwrap();
                    assert_eq!(late.vehicle(game), Some(me));
                    // Replace the local player in the same level while it is both a rider and
                    // a vehicle. A death never plays the ghast's voluntary dismount.
                    game.handle_packet(C::Respawn(play::Respawn {
                        spawn: level(),
                        keep: 0,
                    }))
                    .unwrap();
                    assert_eq!(game.client.remote.as_ref().unwrap().ghast_dismount, None);
                    game.handle_packet(position(2)).unwrap();
                    assert_eq!(game.entity(7), Some(me));
                    assert_eq!(ghast.position(game), Some([2.5, 70.0, 0.5]));
                    assert_eq!(
                        game.mobs
                            .iter()
                            .find(|mob| mob.id == 20)
                            .unwrap()
                            .extra
                            .counter,
                        0
                    );
                    let host: &dyn ClientHost = game;
                    assert_eq!(me.is_passenger(host), Some(false));
                    assert_eq!(
                        collect_entity_visit(|visit| me.visit_passengers(host, visit)),
                        Ok(vec![])
                    );
                    assert_eq!(late.is_passenger(host), Some(false));
                    assert_eq!(
                        collect_entity_visit(|visit| vehicle.visit_passengers(host, visit)),
                        Ok(vec![row])
                    );
                    assert_eq!(cloud.root_vehicle(host), Some(vehicle));

                    game.handle_packet(set(9, vec![])).unwrap();
                    assert_eq!(
                        collect_entity_visit(|visit| vehicle.visit_passengers(game, visit)),
                        Ok(vec![])
                    );
                    assert_eq!(cloud.root_vehicle(game), Some(row));
                },
            )
        });
    }
    /// The window's side of a general row: a spawn adds it immediately in the client's catalog
    /// slot with empty `EntityRenderAttachments`, an unknown key and a second spawn of one id
    /// change nothing, attachment syncs for unknown entities are ignored while updates for known
    /// rows publish immediately regardless of `replace`, a position sync is reached over three
    /// ticks with nothing of the mod's run here, and a removal drops it with its synced
    /// attachments.
    #[test]
    #[ignore = "daily"]
    fn the_window_keeps_moves_and_drops_the_rows_it_is_shown() {
        mod_platform::__private::initialize().unwrap();
        let _phase = mod_platform::__private::RegistrationGuard::open();
        let declare = || vec![mod_api::ModEntry::new("mymod", Box::new(Declare))];
        mod_platform::testing::with_mods(declare().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&declare()).unwrap();
            crate::game::client_side::item_entities_client::thrown_items_tests::with_remote_game(
                |game| {
                    let fuel_key = mod_api::testing::attachment_key::<i32>("mymod", "fuel");
                    // An attachment sync for an unknown entity is ignored without storing anything.
                    game.remote_attachment_sync(attachment_message(5, false, Some(99)));
                    assert!(
                        !game
                            .client
                            .remote
                            .as_ref()
                            .unwrap()
                            .attachments
                            .contains_key(&Holder::Entity(5))
                    );
                    game.handle_packet(spawn(5, "mymod:missile", [0.5, 70.0, 0.5]))
                        .unwrap();
                    game.handle_packet(spawn(6, "other:unknown", [0.5, 70.0, 0.5]))
                        .unwrap();
                    game.handle_packet(spawn(5, "mymod:missile", [9.5, 70.0, 0.5]))
                        .unwrap();
                    let empty_capture = mod_api::__private::game::capture_entity_render_attachments(
                        &Attachments::default(),
                    );
                    let held = rows(game);
                    assert_eq!(held.keys().collect::<Vec<_>>(), [&5]);
                    assert_eq!(held[&5].position, [0.5, 70.0, 0.5]);
                    assert_eq!(held[&5].uuid, 0x77);
                    // Immediately visible with empty render attachments even without attachment_sync.
                    assert_eq!(held[&5].render_attachments, empty_capture);
                    assert_eq!(held[&5].render_attachments.get(&fuel_key), Ok(None));
                    // Both deltas and whole snapshots publish to render_attachments for a known row.
                    game.remote_attachment_sync(attachment_message(5, false, Some(12)));
                    assert_eq!(
                        rows(game)[&5].render_attachments.get(&fuel_key),
                        Ok(Some(&12))
                    );
                    game.remote_attachment_sync(attachment_message(5, true, Some(9)));
                    assert_eq!(row_ids(game), [5]);
                    assert_eq!(
                        rows(game)[&5].render_attachments.get(&fuel_key),
                        Ok(Some(&9))
                    );
                    let fuel = |game: &sunrise_client_gameplay::game::ClientGame<'_>, id: u32| {
                        let held = game.client.remote.as_ref().unwrap();
                        let map = held.attachments.get(&Holder::Entity(id))?;
                        let value = map
                            .synced(sunrise_sim::lan::entity::project_attachment_sync(
                                Holder::Entity(id),
                            ))
                            .into_iter()
                            .find(|entry| {
                                entry.namespace == "mymod".into() && entry.name == "fuel".into()
                            })?
                            .value?;
                        (FUEL_CODEC.decode)(&value).ok()
                    };
                    assert_eq!(fuel(game, 5), Some(9));
                    // A second row spawned with no attachment_sync at all is immediately visible with
                    // empty render attachments.
                    game.handle_packet(spawn(8, "mymod:missile", [0.5, 70.0, 0.5]))
                        .unwrap();
                    assert_eq!(row_ids(game), [5, 8]);
                    assert_eq!(rows(game)[&8].render_attachments, empty_capture);
                    game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
                        id: 5,
                        path: play::PositionPath::Linear([3.5, 70.0, 0.5]),
                        yaw: 0.0,
                        pitch: 0.0,
                        on_ground: false,
                    }))
                    .unwrap();
                    let mut xs = Vec::new();
                    for _ in 0..3 {
                        game.tick_remote_entities(true);
                        xs.push(rows(game)[&5].position[0]);
                    }
                    assert_eq!(xs, [1.5, 2.5, 3.5]);
                    assert_eq!(rows(game)[&5].previous[0], 2.5);
                    // A client holds rows only here: none is a resident row, ticked by mod code.
                    assert!(game.resident_entities.general.rows.is_empty());
                    let holder = Holder::Entity(5);
                    game.client
                        .remote
                        .as_mut()
                        .unwrap()
                        .attachments
                        .insert(holder, Default::default());
                    game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![5, 8] }))
                        .unwrap();
                    assert!(rows(game).is_empty());
                    assert!(
                        !game
                            .client
                            .remote
                            .as_ref()
                            .unwrap()
                            .attachments
                            .contains_key(&holder)
                    );
                },
            )
        });
    }
    /// Matching vanilla `ServerEntity.addPairing` + `ClientPacketListener.handleBundlePacket` and
    /// NeoForge `AttachmentSync`: an atomic pairing bundle `[general_entity::Spawn, attachment_sync]`
    /// dispatches both packets together so the spawned row immediately holds its synced attachments.
    #[test]
    #[ignore = "daily"]
    fn atomic_pairing_bundle_spawns_general_row_with_synced_attachments() {
        use super::super::tests::{connect_play_test, offer_test_payloads, spawn_menu_test};
        use sunrise_client::net::Event;
        use sunrise_protocol::{
            attachment_sync,
            packets::{Empty, configuration::CustomPayload},
        };
        use sunrise_sim::raw_payloads;
        mod_platform::__private::initialize().unwrap();
        let _phase = mod_platform::__private::RegistrationGuard::open();
        let declare = || vec![mod_api::ModEntry::new("mymod", Box::new(Declare))];
        mod_platform::testing::with_mods(declare().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&declare()).unwrap();
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                let fuel_key = mod_api::testing::attachment_key::<i32>("mymod", "fuel");
                let local = raw_payloads::Local::capture(raw_payloads::Side::Server).unwrap();
                let (_reader, mut writer) = connect_play_test(game, |reader, writer| {
                    offer_test_payloads(reader, writer, &local.manifest);
                });
                spawn_menu_test(game);
                let sync = C::CustomPayload(CustomPayload {
                    channel: attachment_sync::CHANNEL.into(),
                    data: attachment_sync::encode(&attachment_message(15, true, Some(42))).unwrap(),
                });
                for packet in [
                    C::BundleDelimiter(Empty),
                    spawn(15, "mymod:missile", [1.5, 70.0, 2.5]),
                    sync,
                    C::BundleDelimiter(Empty),
                ] {
                    writer.send(&packet.encode()).unwrap();
                }
                game.wait_for_test_remote_event(|event| matches!(event, Event::Bundle(_)));
                game.poll_remote_events(true).unwrap();

                let held = rows(game);
                assert_eq!(held.keys().collect::<Vec<_>>(), [&15]);
                assert_eq!(held[&15].position, [1.5, 70.0, 2.5]);
                assert_eq!(held[&15].render_attachments.get(&fuel_key), Ok(Some(&42)));
            })
        });
    }

    /// `Entity.DATA_CUSTOM_NAME` (2) and `DATA_CUSTOM_NAME_VISIBLE` (3) reach a general row
    /// through vanilla's `SetEntityData`, after its spawn and again when they change, and the
    /// client's `EntityRef` reads them (`name`, the styled `custom_name` and
    /// `custom_name_visible`); a name taken away reads as none and false.
    #[test]
    #[ignore = "daily"]
    fn a_general_rows_synced_name_and_visibility_read_through_its_handle() {
        use sunrise_core::nbt::Tag;
        use sunrise_protocol::entity_data::{Value, serializer};

        mod_platform::__private::initialize().unwrap();
        let _phase = mod_platform::__private::RegistrationGuard::open();
        let declare = || vec![mod_api::ModEntry::new("mymod", Box::new(Declare))];
        mod_platform::testing::with_mods(declare().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&declare()).unwrap();
            crate::game::client_side::item_entities_client::thrown_items_tests::with_remote_game(
                |game| {
                    game.handle_packet(C::Login(play::Login {
                        player_id: 7,
                        hardcore: false,
                        levels: vec!["minecraft:overworld".into()],
                        max_players: 20,
                        chunk_radius: 8,
                        simulation_distance: 8,
                        reduced_debug_info: false,
                        show_death_screen: true,
                        limited_crafting: false,
                        spawn: play::SpawnInfo {
                            dimension_type: 0,
                            dimension: "minecraft:overworld".into(),
                            seed: 1,
                            game_type: 0,
                            previous_game_type: None,
                            debug: false,
                            flat: false,
                            last_death: None,
                            portal_cooldown: 0,
                            sea_level: 63,
                        },
                        online_mode: false,
                        enforces_secure_chat: false,
                    }))
                    .unwrap();
                    game.handle_packet(C::PlayerPosition(play::PlayerPosition {
                        teleport: 1,
                        position: [0.5, 70.0, 0.5],
                        delta: [0.0; 3],
                        yaw: 0.0,
                        pitch: 0.0,
                        relatives: 0,
                    }))
                    .unwrap();
                    game.handle_packet(spawn(12, "mymod:missile", [0.5, 70.0, 0.5]))
                        .unwrap();
                    let row = game.entity(12).unwrap();
                    let data = |values: Vec<(u8, i32, Value)>| {
                        C::SetEntityData(play::SetEntityData { id: 12, values })
                    };
                    let named = |name: Option<&str>, visible: bool| {
                        data(vec![
                            (
                                2,
                                serializer::OPTIONAL_COMPONENT,
                                Value::OptText(name.map(|n| Tag::String(n.into()))),
                            ),
                            (3, serializer::BOOLEAN, Value::Bool(visible)),
                        ])
                    };

                    // Before any data: no name, not visible.
                    assert_eq!(row.custom_name(game).unwrap(), None);
                    assert_eq!(row.custom_name_visible(game), Some(false));
                    assert_ne!(row.name(game).as_deref(), Some("Grumbles"));

                    game.handle_packet(named(Some("Grumbles"), true)).unwrap();
                    assert_eq!(row.name(game).as_deref(), Some("Grumbles"));
                    assert_eq!(
                        row.custom_name(game).unwrap(),
                        Some(mod_api::Text::literal("Grumbles"))
                    );
                    assert_eq!(row.custom_name_visible(game), Some(true));

                    game.handle_packet(named(Some("Rumbles"), false)).unwrap();
                    assert_eq!(row.name(game).as_deref(), Some("Rumbles"));
                    assert_eq!(
                        row.custom_name(game).unwrap(),
                        Some(mod_api::Text::literal("Rumbles"))
                    );
                    assert_eq!(row.custom_name_visible(game), Some(false));

                    game.handle_packet(named(None, false)).unwrap();
                    assert_ne!(row.name(game).as_deref(), Some("Rumbles"));
                    assert_eq!(rows(game)[&12].custom_name, None);
                    assert_eq!(row.custom_name(game).unwrap(), None);
                    assert_eq!(row.custom_name_visible(game), Some(false));
                },
            )
        });
    }

    /// `Entity.DATA_NO_GRAVITY` (5) reaches a general row through vanilla's `SetEntityData`
    /// after its spawn and again when it changes: the client's row holds what was last set.
    #[test]
    #[ignore = "daily"]
    fn a_general_rows_synced_no_gravity_reaches_its_client_row() {
        use sunrise_protocol::entity_data::{Value, serializer};

        mod_platform::__private::initialize().unwrap();
        let _phase = mod_platform::__private::RegistrationGuard::open();
        let declare = || vec![mod_api::ModEntry::new("mymod", Box::new(Declare))];
        mod_platform::testing::with_mods(declare().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&declare()).unwrap();
            crate::game::client_side::item_entities_client::thrown_items_tests::with_remote_game(
                |game| {
                    game.handle_packet(C::Login(play::Login {
                        player_id: 7,
                        hardcore: false,
                        levels: vec!["minecraft:overworld".into()],
                        max_players: 20,
                        chunk_radius: 8,
                        simulation_distance: 8,
                        reduced_debug_info: false,
                        show_death_screen: true,
                        limited_crafting: false,
                        spawn: play::SpawnInfo {
                            dimension_type: 0,
                            dimension: "minecraft:overworld".into(),
                            seed: 1,
                            game_type: 0,
                            previous_game_type: None,
                            debug: false,
                            flat: false,
                            last_death: None,
                            portal_cooldown: 0,
                            sea_level: 63,
                        },
                        online_mode: false,
                        enforces_secure_chat: false,
                    }))
                    .unwrap();
                    game.handle_packet(C::PlayerPosition(play::PlayerPosition {
                        teleport: 1,
                        position: [0.5, 70.0, 0.5],
                        delta: [0.0; 3],
                        yaw: 0.0,
                        pitch: 0.0,
                        relatives: 0,
                    }))
                    .unwrap();
                    game.handle_packet(spawn(12, "mymod:missile", [0.5, 70.0, 0.5]))
                        .unwrap();
                    let set = |no_gravity: bool| {
                        C::SetEntityData(play::SetEntityData {
                            id: 12,
                            values: vec![(5, serializer::BOOLEAN, Value::Bool(no_gravity))],
                        })
                    };
                    assert!(!rows(game)[&12].no_gravity);
                    game.handle_packet(set(true)).unwrap();
                    assert!(rows(game)[&12].no_gravity);
                    game.handle_packet(set(false)).unwrap();
                    assert!(!rows(game)[&12].no_gravity);
                },
            )
        });
    }

    #[test]
    #[ignore = "daily"]
    fn general_spawn_captures_existing_entity_attachments_or_empty() {
        mod_platform::__private::initialize().unwrap();
        let _phase = mod_platform::__private::RegistrationGuard::open();
        let declare = || vec![mod_api::ModEntry::new("mymod", Box::new(Declare))];
        mod_platform::testing::with_mods(declare().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&declare()).unwrap();
            crate::game::client_side::item_entities_client::thrown_items_tests::with_remote_game(
                |game| {
                    let fuel_key = mod_api::testing::attachment_key::<i32>("mymod", "fuel");
                    // Path 1: no prior Holder::Entity entry -> empty initial snapshot.
                    game.handle_packet(spawn(21, "mymod:missile", [0.0, 70.0, 0.0]))
                        .unwrap();
                    assert_eq!(rows(game)[&21].render_attachments.get(&fuel_key), Ok(None));

                    // Path 2: existing Holder::Entity map present before spawn -> captured on spawn.
                    let mut existing =
                        sunrise_mod_runtime::attachments::ResidentAttachments::default();
                    existing.apply_sync(sunrise_mod_runtime::attachments::AttachmentSync {
                        namespace: "mymod".into(),
                        name: "fuel".into(),
                        value: Some([("n".into(), sunrise_core::nbt::Tag::Int(77))].into()),
                    });
                    game.client
                        .remote
                        .as_mut()
                        .unwrap()
                        .attachments
                        .insert(Holder::Entity(22), existing);
                    game.handle_packet(spawn(22, "mymod:missile", [1.0, 70.0, 0.0]))
                        .unwrap();
                    assert_eq!(
                        rows(game)[&22].render_attachments.get(&fuel_key),
                        Ok(Some(&77))
                    );
                },
            )
        });
    }
}
