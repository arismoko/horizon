#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::{self, BlockPart, Flow, Hooks, RandomTickEvent, Registrar};
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::LevelDimension;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_world::level::LevelEvent;

    /// A stone part whose random tick plays level events through the write host.
    struct Events;
    #[mod_api::sunrise_part]
    impl BlockPart for Events {
        fn random_tick(
            &self,
            level: &mut dyn mod_api::LevelWriteHost,
            event: &mut RandomTickEvent,
        ) -> Flow {
            // 2001 carries a block state: `data` -1 is none, so it is refused.
            assert!(!level.level_event(2001, [1, 64, 1], -1));
            assert!(level.level_event(1500, [1, 64, 1], 0));
            assert!(level.level_event(2005, [2, 64, 2], 7));
            let _ = event;
            Flow::Continue
        }
    }
    struct EventsMod;
    #[mod_api::__subscriptions]
    impl Hooks for EventsMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let block_key = r.block_key("minecraft:stone");
            r.block_part(block_key, Events);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_queued_block_event_runs_the_blocks_trigger_once() {
        use mod_api::{LevelRef, WorldEpoch};
        use mod_platform::__private::part_tests::block_key;
        use sunrise_core::position::BlockPos;
        super::super::native_entity_refs::tests::with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let note = game
                .pack
                .assets
                .registry
                .default_state("minecraft:note_block")
                .unwrap();
            game.world.set(0, 64, 0, note).unwrap();
            let (note_key, stone_key) = (block_key("minecraft:note_block"), block_key("stone"));

            // Queued twice, it runs once; one queued for a block `pos` no longer holds is
            // dropped when it runs, as `runBlockEvents` checks the block.
            assert!(level.block_event(game, [0, 64, 0], note_key, 0, 0));
            assert!(level.block_event(game, [0, 64, 0], note_key, 0, 0));
            assert!(level.block_event(game, [0, 64, 0], stone_key, 0, 0));
            assert_eq!(game.block_ticks.block_events.len(), 2);
            let (bottom, height) = level.height(game).unwrap();
            assert!(!level.block_event(game, [0, bottom - 1, 0], note_key, 0, 0));
            assert!(
                !level.block_event(game, [0, bottom + height, 0], note_key, 0, 0),
                "above the build height"
            );
            assert!(!stale.block_event(game, [0, 64, 0], note_key, 0, 0));
            assert!(
                !nether.block_event(game, [0, 64, 0], note_key, 0, 0),
                "a level not in place"
            );
            assert_eq!(
                game.block_ticks.block_events.len(),
                2,
                "refusals queue nothing"
            );

            game.level_events.clear();
            game.with_level(|b, level, ticks| b.run_block_events(level, ticks));
            assert!(game.block_ticks.block_events.is_empty());
            let handled: Vec<_> = game
                .level_events
                .iter()
                .filter(|e| matches!(e, LevelEvent::BlockEntityEvent { .. }))
                .collect();
            assert_eq!(
                handled,
                [&LevelEvent::BlockEntityEvent {
                    pos: BlockPos::new(0, 64, 0),
                    state: note,
                    a: 0,
                    b: 0,
                }],
                "the note block's triggerEvent handled it once, for its players"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_level_event_in_a_write_hook_joins_the_events_in_order() {
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("level-events", Box::new(EventsMod))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                let before = LevelEvent::LevelEvent {
                    id: 1000,
                    pos: sunrise_core::position::BlockPos::new(0, 64, 0),
                    data: 0,
                    source: None,
                };
                game.level_events.push(before.clone());
                let state = game.world.block(0, 64, 0);
                let mut handled = false;
                game.as_guest(12, |game| {
                    game.with_level(|_, level, ticks| {
                        level.native_random_tick(
                            ticks,
                            LevelDimension::Overworld,
                            sunrise_core::position::BlockPos::new(0, 64, 0),
                            state,
                            &mut handled,
                        );
                    });
                    let events = game.level_events.clone();
                    game.lan_record_events(&events);
                    assert_eq!(
                        game.server.lan_events[1],
                        sunrise_gameplay_common::game::QueuedLevelEvent {
                            kind: 1500,
                            pos: [1, 64, 1],
                            data: 0,
                            source: None,
                        }
                    );
                    assert_eq!(
                        game.server.lan_events[2],
                        sunrise_gameplay_common::game::QueuedLevelEvent {
                            kind: 2005,
                            pos: [2, 64, 2],
                            data: 7,
                            source: None,
                        }
                    );
                })
                .unwrap();
                let at = |id, x, z, data| LevelEvent::LevelEvent {
                    id,
                    pos: sunrise_core::position::BlockPos::new(x, 64, z),
                    data,
                    source: None,
                };
                assert_eq!(
                    game.level_events,
                    [before, at(1500, 1, 1, 0), at(2005, 2, 2, 7)]
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_global_level_event_reaches_every_player_or_falls_back_to_a_plain_one() {
        use mod_api::{LevelRef, WorldEpoch};
        use sunrise_game_data::gamerules::Value;
        super::super::native_entity_refs::tests::with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            game.start_audio_capture();
            game.server.lan_global_events.clear();
            game.server.lan_events.clear();

            // An End portal opening: one packet with the global flag for every player.
            assert!(level.global_level_event(game, 1038, [10, 64, -3], 0));
            assert_eq!(
                game.server.lan_global_events,
                [("minecraft:overworld", 1038, [10, 64, -3], 0)]
            );
            assert!(
                game.server.lan_events.is_empty(),
                "no plain event while the rule is on"
            );
            assert!(
                !level.global_level_event(game, 2001, [0, 64, 0], -1),
                "2001 carries a block state"
            );
            assert!(!stale.global_level_event(game, 1038, [0, 64, 0], 0));
            assert_eq!(
                game.server.lan_global_events.len(),
                1,
                "refusals send nothing"
            );

            // ServerLevel.globalLevelEvent without global_sound_events: levelEvent(null, ...).
            assert!(game.ui.rules.set("global_sound_events", Value::Bool(false)));
            game.server.lan_global_events.clear();
            assert!(level.global_level_event(game, 1038, [10, 64, -3], 0));
            assert!(game.server.lan_global_events.is_empty());
            assert_eq!(
                game.server.lan_events,
                [sunrise_gameplay_common::game::QueuedLevelEvent {
                    kind: 1038,
                    pos: [10, 64, -3],
                    data: 0,
                    source: None,
                }]
            );
        });
    }

    /// `ServerLevel.globalLevelEvent` with `global_sound_events` off is
    /// `levelEvent(null, ...)`: a player within 64 blocks gets the plain packet
    /// at the event's position, and one further away gets nothing.
    #[test]
    #[ignore = "daily"]
    fn with_the_rule_off_a_global_level_event_reaches_a_player_in_range_as_a_plain_one() {
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_game_data::gamerules::Value;
        use sunrise_gameplay::game::server_side::lan::LanSettings;
        use sunrise_protocol::packets::{
            configuration::ClientInformation,
            play::{Clientbound, LevelEvent},
        };
        use sunrise_sim::lan::{
            Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig,
        };
        super::super::native_entity_refs::tests::with_game(|game| {
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.server.level_shapes.clone(),
                level_seeds: game.server.levels.0.runtime.seeds(),
            };
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            game.lan_host(settings, net, config).unwrap();
            let mut guest_at = |conn: u32, at: [f64; 3]| {
                let (tx, rx) = std::sync::mpsc::channel();
                let mut guest = Guest::new(
                    Joined {
                        conn,
                        uuid: u128::from(conn),
                        name: format!("guest{conn}"),
                        properties: Vec::new(),
                        info: ClientInformation::new(8),
                        brand: String::new(),
                        sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                        latency: Arc::new(AtomicI32::new(0)),
                        address: "127.0.0.1:1".parse().unwrap(),
                        expected: false,
                    },
                    conn as i32,
                    at,
                    10,
                );
                guest.placed = true;
                guest.dimension = game.dim.id().to_owned();
                game.server.lan.as_mut().unwrap().hub.guests.push(guest);
                rx
            };
            let near = guest_at(12, [30.5, 64.0, -3.5]);
            let far = guest_at(13, [110.5, 64.0, -3.5]);
            let events = |rx: &std::sync::mpsc::Receiver<Out>| {
                rx.try_iter()
                    .flat_map(|out| match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(_) => panic!("unexpected disconnect"),
                    })
                    .filter_map(|body| {
                        let mut reader = sunrise_protocol::buf::Reader::new(&body);
                        let id = reader.varint().unwrap();
                        match Clientbound::decode(id, reader.rest()) {
                            Ok(Clientbound::LevelEvent(p)) => Some(p),
                            _ => None,
                        }
                    })
                    .collect::<Vec<_>>()
            };
            game.lan_flush_effects();
            events(&near);
            events(&far);
            assert!(game.ui.rules.set("global_sound_events", Value::Bool(false)));
            let level = game.level(game.dim.id()).unwrap();
            assert!(level.global_level_event(game, 1038, [10, 64, -3], 0));
            game.lan_flush_effects();
            assert_eq!(
                events(&near),
                [LevelEvent {
                    kind: 1038,
                    pos: [10, 64, -3],
                    data: 0,
                    global: false,
                }]
            );
            assert!(events(&far).is_empty(), "100 blocks away is out of range");
        });
    }
}
