use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;

/// A client names the level it shows by the server's key, even where `Dim` has
/// no name for it.
#[test]
#[ignore = "daily"]
fn client_handles_reach_a_custom_level() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{DimensionKey, LevelRef};
    use std::sync::Arc;
    use sunrise_client::level_loading::Reason;
    use sunrise_protocol::{connection::Pipe, packets::play::SpawnInfo};

    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        use sunrise_world::dimension_type::DimensionFacts;
        let stale_key = sunrise_core::LevelKey::parse("test:stale_guest_facts").unwrap();
        let mut stale = DimensionFacts::overworld().clone();
        stale.coordinate_scale = 19.0;
        DimensionFacts::set_levels(vec![(stale_key, stale)]);
        game.connect_local(client, "CustomLevelTest").unwrap();
        assert_eq!(game.dimension_facts_for(stale_key).coordinate_scale, 1.0);
        let mut registries = (*game.client.remote.as_ref().unwrap().registries).clone();
        let json = serde_json::json!({"min_y": 0, "height": 256,
                "coordinate_scale": 4.0, "ambient_light": 0.3,
                "attributes": {"minecraft:gameplay/fast_lava": true}});
        registries.registries.insert(
            "minecraft:dimension_type".into(),
            vec![(
                "test:type".into(),
                Some(sunrise_game_data::scoreboard::json_to_tag(&json)),
            )],
        );
        game.set_registries(Arc::new(registries)).unwrap();
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "test:pocket".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        game.client.remote.as_mut().unwrap().logged_in = true;

        let level = game.level("test:pocket").unwrap();
        assert_ne!(level.dimension(), DimensionKey::OVERWORLD);
        assert_eq!(level.dimension().id(game), Some("test:pocket"));
        assert!(game.level("minecraft:overworld").is_none());
        assert!(level.height(game).is_some());
        let facts: mod_api::LevelDimensionType<'_> = level.dimension_type(game).unwrap();
        assert_eq!(facts.id, "test:type");
        assert_eq!(facts.coordinate_scale, 4.0);
        assert_eq!(facts.ambient_light, 0.3);
        assert!(facts.fast_lava);
        assert_eq!(game.dimension_type().coordinate_scale, 4.0);
        let overworld = LevelRef::new(level.epoch(), DimensionKey::OVERWORLD);
        assert_eq!(overworld.height(game), None);
        // An equal JSON body with a different type name must not hit the old cache.
        let mut renamed = (*game.client.remote.as_ref().unwrap().registries).clone();
        renamed
            .registries
            .get_mut("minecraft:dimension_type")
            .unwrap()[0]
            .0 = "test:renamed".into();
        game.set_registries(Arc::new(renamed)).unwrap();
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        assert_eq!(game.dimension_type().id, "test:renamed");
        // Reject invalid server numbers and enums without failing the join.
        for (field, value) in [
            ("coordinate_scale", serde_json::json!(0.0)),
            ("coordinate_scale", serde_json::json!("NaN")),
            ("coordinate_scale", serde_json::Value::Null),
            ("ambient_light", serde_json::json!("NaN")),
            ("ambient_light", serde_json::Value::Null),
            ("monster_spawn_block_light_limit", serde_json::json!(16)),
            ("monster_spawn_block_light_limit", serde_json::json!(-1)),
            ("skybox", serde_json::json!("invalid")),
        ] {
            let mut invalid = json.clone();
            invalid[field] = value;
            let mut registries = (*game.client.remote.as_ref().unwrap().registries).clone();
            registries.registries.insert(
                "minecraft:dimension_type".into(),
                vec![(
                    "minecraft:the_nether".into(),
                    Some(sunrise_game_data::scoreboard::json_to_tag(&invalid)),
                )],
            );
            game.set_registries(Arc::new(registries)).unwrap();
            game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
            assert_eq!(game.dimension_type().coordinate_scale, 8.0, "{field}");
            assert!(game.dimension_type().fast_lava);
        }
        let min = f64::from(1.0E-5_f32);
        let max = 30_000_000.0_f64;
        for (value, valid) in [
            (min.next_down(), false),
            (min, true),
            (min.next_up(), true),
            (max.next_down(), true),
            (max, true),
            (max.next_up(), false),
            (f64::from_bits(1), false),
        ] {
            let mut boundary = json.clone();
            boundary["coordinate_scale"] = serde_json::json!(value);
            // Vanilla uses Codec.FLOAT, not a unit-interval ambient-light codec.
            boundary["ambient_light"] = serde_json::json!(1.1);
            let mut registries = (*game.client.remote.as_ref().unwrap().registries).clone();
            registries.registries.insert(
                "minecraft:dimension_type".into(),
                vec![(
                    "minecraft:the_nether".into(),
                    Some(sunrise_game_data::scoreboard::json_to_tag(&boundary)),
                )],
            );
            game.set_registries(Arc::new(registries)).unwrap();
            game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
            assert_eq!(
                game.dimension_type().coordinate_scale,
                if valid { value } else { 8.0 }
            );
            if valid {
                assert_eq!(game.dimension_type().ambient_light, 1.1);
            }
        }
        for ambient in [1.5_f32, -0.5_f32] {
            let mut legal = json.clone();
            legal["ambient_light"] = serde_json::json!(ambient);
            let block_limit = if ambient > 0.0 { 15 } else { 0 };
            legal["monster_spawn_block_light_limit"] = serde_json::json!(block_limit);
            let mut registries = (*game.client.remote.as_ref().unwrap().registries).clone();
            registries.registries.insert(
                "minecraft:dimension_type".into(),
                vec![(
                    "test:type".into(),
                    Some(sunrise_game_data::scoreboard::json_to_tag(&legal)),
                )],
            );
            game.set_registries(Arc::new(registries)).unwrap();
            game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
            assert_eq!(game.dimension_type().coordinate_scale, 4.0);
            assert_eq!(game.dimension_type().ambient_light, ambient);
            assert_eq!(
                game.dimension_type().monster_spawn_block_light_limit,
                block_limit
            );
            let level = game.level("test:pocket").unwrap();
            assert_eq!(level.dimension_type(game).unwrap().ambient_light, ambient);
        }
        // Disconnect drops the connection-owned table; the server table is untouched.
        let lifetime = Arc::downgrade(
            game.client
                .remote
                .as_ref()
                .unwrap()
                .level_facts
                .as_ref()
                .unwrap(),
        );
        game.leave_server(None).unwrap();
        assert!(
            lifetime.upgrade().is_none(),
            "connection facts must be released"
        );
        assert_eq!(DimensionFacts::of_level(stale_key).coordinate_scale, 19.0);
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "SecondFactsSession").unwrap();
        let key = sunrise_core::LevelKey::parse("test:pocket").unwrap();
        assert_eq!(game.dimension_facts_for(key).coordinate_scale, 1.0);
        DimensionFacts::set_levels(Vec::new());
    });
}

/// The `both` handle reads answer on a client from its own copy: the local
/// player's health, effects, equipment and last death, the blocks and time
/// of day of the level it shows and the entities it tracks.
#[test]
#[ignore = "daily"]
fn client_handle_reads_answer_from_the_clients_copy() {
    use mod_api::{EntityReadError, EntityRef, EquipmentSlot, GlobalPos};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C, SpawnInfo},
    };
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
    };
    use {
        sunrise_client::level_loading::Reason, sunrise_level::chunk::Chunk,
        sunrise_sim::first_person_sim::Hand, sunrise_sim::motion::PlayerMotion,
    };

    let mut untracked_out = None;
    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "HandleReadTest").unwrap();
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: Some(play::GlobalPos {
                dimension: "minecraft:the_nether".into(),
                pos: [4, 70, -9],
            }),
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(1.5, 65.0, -2.5)).unwrap());
        game.yaw = 90.0;
        game.pitch = 10.0;

        // The local player and the level it shows.
        let player = game.local_player().unwrap();
        let me = player.entity();
        let living = game.living(me).unwrap();
        let level = game.level("minecraft:overworld").unwrap();
        assert!(game.level("minecraft:the_nether").is_none());
        assert_eq!(me.position(game), Some([1.5, 65.0, -2.5]));
        assert_eq!(me.rotation(game), Some([90.0, 10.0]));
        game.player.as_mut().unwrap().velocity = Vec3::new(0.0, -0.5, 0.25);
        assert_eq!(me.velocity(game), Some([0.0, -0.5, 0.25]));

        // Last death: the login/respawn packet's, as a GlobalPos.
        let died = player.last_death_location(game).unwrap();
        assert_eq!(died.dimension().to_string(), "minecraft:the_nether");
        let pos = died.pos();
        assert_eq!([pos.x, pos.y, pos.z], [4, 70, -9]);
        let _: GlobalPos = died;

        // Health, maximum health, effects and equipment come from the packets.
        game.handle_packet(C::SetHealth(play::SetHealth {
            health: 14.0,
            food: 20,
            saturation: 5.0,
        }))
        .unwrap();
        assert_eq!(living.health(game), Some(14.0));
        assert_eq!(living.max_health(game), Some(20.0));
        let speed = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:mob_effect")
            .unwrap()
            .id("minecraft:speed")
            .unwrap();
        use std::ops::ControlFlow;
        assert_eq!(
            living.visit_effects(game, &mut |_| panic!("effect before sync")),
            Ok(ControlFlow::Continue(()))
        );
        assert!(
            living
                .effect(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed")
                )
                .is_none()
        );
        game.handle_packet(C::UpdateMobEffect(play::UpdateMobEffect {
            id: 7,
            effect: speed,
            amplifier: 1,
            duration: 600,
            flags: 2,
        }))
        .unwrap();
        let effect = living
            .effect(
                game,
                mod_platform::__private::part_tests::effect_key("minecraft:speed"),
            )
            .unwrap();
        assert_eq!((effect.duration, effect.amplifier), (600, 1));
        let mut effects = Vec::new();
        assert_eq!(
            living.visit_effects(game, &mut |effect| {
                effects.push((effect.duration, effect.amplifier));
                ControlFlow::Continue(())
            }),
            Ok(ControlFlow::Continue(()))
        );
        assert_eq!(effects, [(600, 1)]);
        let stone = Some(sunrise_protocol::item::Stack::new(
            game.item_id("minecraft:stone"),
            3,
        ));
        game.handle_packet(C::SetPlayerInventory(play::SetPlayerInventory {
            slot: 0,
            item: stone,
        }))
        .unwrap();
        assert_eq!(
            living.with_hand_item(game, Hand::Main, |held| (
                mod_api::__private::item_key_id(held.item()).unwrap(),
                held.count(),
            )),
            Some(("minecraft:stone", 3))
        );
        assert_eq!(
            living.with_equipment(game, EquipmentSlot::Head, |_| ()),
            None
        );
        assert_eq!(living.with_hand_item(game, Hand::Off, |_| ()), None);

        // Blocks of the chunks the client holds; nothing loads another.
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let chunks = game.world.chunks.len();
        let at = [3, 64, 5];
        assert!(level.is_loaded(game, at));
        assert!(!level.is_loaded(game, [160, 64, 0]));
        game.handle_packet(C::BlockUpdate(play::BlockUpdate { pos: at, state: 1 }))
            .unwrap();
        assert_eq!(
            level
                .block(game, at)
                .and_then(mod_api::__private::block_key_id),
            Some("minecraft:stone")
        );
        assert!(level.block_state(game, at).is_some());
        assert_eq!(level.block(game, [160, 64, 0]), None);
        assert_eq!(level.block_property(game, [160, 64, 0], "facing"), None);
        assert_eq!(game.world.chunks.len(), chunks);

        // The time of day, as the server's clock last set it.
        game.set_registries({
            let mut registries = (*game.client.remote.as_ref().unwrap().registries).clone();
            registries.registries.insert(
                "minecraft:world_clock".into(),
                vec![("minecraft:overworld".into(), None)],
            );
            Arc::new(registries)
        })
        .unwrap();
        game.handle_packet(C::SetTime(play::SetTime {
            game_time: 100,
            clocks: vec![play::ClockState {
                clock: 0,
                total_ticks: 30_000,
                partial_tick: 0.0,
                rate: 1.0,
            }],
        }))
        .unwrap();
        assert_eq!(level.day_time(game), Some(30_000));

        // An entity the server told the client of; others read None.
        let pig = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:pig")
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 31,
            uuid: 9,
            kind: pig,
            position: [10.0, 64.0, 12.0],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 64,
            head_yaw: 64,
            data: 0,
        }))
        .unwrap();
        let other = game.entity(31).unwrap();
        assert_eq!(other.position(game), Some([10.0, 64.0, 12.0]));
        assert!(other.velocity(game).is_some());
        assert!(other.rotation(game).is_some());
        assert!(game.living(other).is_none(), "only the local player");
        assert!(game.entity(99).is_none());
        let untracked = EntityRef::new(me.epoch(), 99);
        assert_eq!(untracked.position(game), None);
        assert_eq!(untracked.velocity(game), None);

        // The spawn tracks an item before its item-stack metadata supplies a body.
        game.spawn_entity(32, "minecraft:item", [10.0, 64.0, 12.0], [0.0; 3]);
        let pending = game.entity(32).unwrap();
        assert_eq!(
            pending.visit_parts(game, &mut |_| panic!("visited without a projected body")),
            Err(EntityReadError::Unavailable)
        );
        assert_eq!(
            pending.visit_passengers(game, &mut |_| panic!("visited an empty seat list")),
            Ok(ControlFlow::Continue(()))
        );
        assert_eq!(
            untracked.visit_parts(game, &mut |_| panic!("visited an untracked entity")),
            Err(EntityReadError::Stale)
        );
        game.client.remote.as_mut().unwrap().reconfiguring = Some(0);
        assert_eq!(
            other.visit_parts(game, &mut |_| panic!("visited with no live client world")),
            Err(EntityReadError::Unavailable)
        );
        game.client.remote.as_mut().unwrap().reconfiguring = None;

        // A replaced level retires the handles.
        let mut other_level = spawn.clone();
        other_level.dimension = "test:other_overworld".into();
        game.enter_remote_dimension(&other_level, RemoteDimensionEntry::Respawn, Reason::Other);
        assert_eq!(me.position(game), None);
        assert_eq!(living.health(game), None);
        assert_eq!(level.block(game, at), None);
        assert!(!level.is_loaded(game, at));
        assert_eq!(level.day_time(game), None);
        assert_eq!(player.last_death_location(game), None);
        untracked_out = Some(untracked);
    });
    // The server's role keeps its own reads: a client handle names nothing there.
    with_game(|server| {
        assert_eq!(server.native_entity_target(untracked_out.unwrap()), None);
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
