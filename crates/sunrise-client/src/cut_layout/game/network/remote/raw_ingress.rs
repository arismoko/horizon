#![cfg(test)]
use std::time::Duration;
use sunrise_protocol::packets::play;
use sunrise_protocol::packets::play::Clientbound as C;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
#[path = "bundle_parity_tests.rs"]
pub(crate) mod bundle_parity_tests;

#[cfg(test)]
#[path = "post_effects_parity_tests.rs"]
pub(crate) mod post_effects_parity_tests;

#[cfg(test)]
pub(crate) mod tests {
    #[cfg(feature = "client")]
    use super::super::tests::{connect_play_test, spawn_menu_test};
    use super::*;
    #[cfg(feature = "client")]
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{ClientHost, ClientPacketEvent, EntityRef, Hooks, ModEntry};
    use std::sync::mpsc;
    #[cfg(feature = "client")]
    use sunrise_client_gameplay::game::client_side::remote_client::ingress_state::ListenerTurn;
    #[cfg(feature = "client")]
    use sunrise_client_gameplay::remote::chunks::Received;
    use sunrise_client_lib::net::Event;

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn executor_sees_entity_removal_published_during_packet_dispatch() {
        struct Listener {
            publish: Box<dyn Fn(C) + Send + Sync>,
            entity: EntityRef,
            record: mpsc::Sender<Option<[f64; 3]>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn client_packet(&self, host: &mut dyn ClientHost, event: &mut ClientPacketEvent<'_>) {
                if event.packet.id.path() != "ping" {
                    return;
                }
                let entity = self.entity;
                let record = self.record.clone();
                host.client_executor()
                    .unwrap()
                    .execute(Box::new(move |host| {
                        record.send(entity.position(host)).unwrap()
                    }));
                // Publish behind the callback while the first packet is still being handled.
                (self.publish)(C::RemoveEntities(play::RemoveEntities {
                    ids: vec![entity.id() as i32],
                }));
            }
        }

        with_client_game(|game| {
            let (_reader, _writer) = connect_play_test(game, |_, _| {});
            spawn_menu_test(game);
            let publisher = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .session
                .received_event_probe();
            let publish = move |packet| publisher(Event::Packet(Box::new(packet)));
            game.spawn_entity(20, "minecraft:pig", [2.0, 64.0, 0.0], [0.0; 3]);
            let entity = game.entity(20).unwrap();
            assert_eq!(entity.position(game), Some([2.0, 64.0, 0.0]));

            let (record, calls) = mpsc::channel();
            publish(C::Ping(sunrise_protocol::packets::configuration::Ping {
                id: 1,
            }));
            let mods = vec![ModEntry::new(
                "during_packet",
                Box::new(Listener {
                    publish: Box::new(publish),
                    entity,
                    record,
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || game.poll_remote().unwrap());
            assert_eq!(calls.try_recv(), Ok(None));
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_bundled_long_debug_sample_applies_only_the_prefix_before_packet_error() {
        use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
        if run_fresh_cases(
            "cut_layout::game::client_side::remote_client::ingress_state::tests::a_bundled_long_debug_sample_applies_only_the_prefix_before_packet_error",
            "SUNRISE_DEBUG_SAMPLE_BUNDLE_CHILD",
            &["debug-sample"],
        ) {
            return;
        }
        use std::sync::{Arc, Mutex};
        use sunrise_client::net::{PacketRecorder, RecorderStatus};
        use sunrise_protocol::ids::play::clientbound::{BUNDLE_DELIMITER, SET_HELD_SLOT};

        struct RecordIds(Arc<Mutex<Vec<i32>>>);
        impl PacketRecorder for RecordIds {
            fn is_active(&self) -> bool {
                true
            }
            fn received(&self, _: &C, id: i32, _: &[u8]) {
                self.0.lock().unwrap().push(id);
            }
            fn record_packet(&self, _: i32, _: &[u8]) {}
            fn status(&self) -> RecorderStatus {
                RecorderStatus {
                    active: true,
                    path: Default::default(),
                    elapsed: Duration::ZERO,
                    packet_count: self.0.lock().unwrap().len() as u64,
                    bytes_written: 0,
                    error: None,
                }
            }
            fn stop(self: Box<Self>) -> std::io::Result<std::path::PathBuf> {
                Ok(Default::default())
            }
        }

        with_client_game(|game| {
            let (_reader, mut writer) = connect_play_test(game, |_, _| {});
            spawn_menu_test(game);
            let recorded = Arc::new(Mutex::new(Vec::new()));
            game.client
                .remote
                .as_ref()
                .unwrap()
                .session
                .start_recording(RecordIds(recorded.clone()), |_| {});
            let delimiter = C::BundleDelimiter(sunrise_protocol::packets::Empty);
            game.selected = 0;
            for packet in [
                delimiter.clone(),
                C::SetHeldSlot(play::SetHeldSlot { slot: 1 }),
                C::DebugSample(play::DebugSample {
                    sample: vec![0; 5],
                    kind: play::DEBUG_SAMPLE_TICK_TIME,
                }),
                C::SetHeldSlot(play::SetHeldSlot { slot: 2 }),
                delimiter,
            ] {
                writer.send(&packet.encode()).unwrap();
            }
            let outcome =
                |event: &Event| matches!(event, Event::Bundle(_) | Event::Disconnected { .. });
            game.wait_for_test_remote_event(outcome);
            let backlog = &mut game.client.remote.as_mut().unwrap().backlog;
            let pos = backlog
                .iter()
                .position(|item| matches!(item, Received::Event(event, _) if outcome(event)))
                .unwrap();
            let Some(Received::Event(Event::Bundle(bundle), _credit)) = backlog.remove(pos) else {
                panic!("the receiver closed before handing the completed bundle to main");
            };
            assert_eq!(
                *recorded.lock().unwrap(),
                [
                    BUNDLE_DELIMITER,
                    SET_HELD_SLOT,
                    SET_HELD_SLOT,
                    BUNDLE_DELIMITER
                ],
                "only the real bundle members surrounding the fault reach the recorder",
            );
            use sunrise_client::net::BundleMember;
            assert!(matches!(
                bundle.packets.as_slice(),
                [
                    BundleMember::Packet(C::SetHeldSlot(_)),
                    BundleMember::OversizedDebugSample,
                    BundleMember::Packet(C::SetHeldSlot(_)),
                ]
            ));
            assert_eq!(game.selected, 0, "no member applied on the reader");
            let mut turn = ListenerTurn::default();
            game.handle_play_bundle(*bundle, &mut turn).unwrap();
            assert_eq!(
                game.selected, 1,
                "the prefix applied but the suffix did not"
            );
            assert!(matches!(
                turn.end,
                Some(Event::Disconnected {
                    key: Some(key),
                    from_server: false,
                    after_login: true,
                    ..
                }) if key == "disconnect.packetError"
            ));
        });
    }

    #[cfg(feature = "client")]
    fn push_events(
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
        events: impl IntoIterator<Item = Event>,
    ) {
        let publish = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .session
            .received_event_probe();
        for event in events {
            publish(event);
        }
        game.wait_for_test_remote_event(
            |event| matches!(event, Event::Packet(packet) if matches!(&**packet, C::Login(_))),
        );
    }

    fn login_packet() -> play::Clientbound {
        C::Login(play::Login {
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
        })
    }

    fn registries_with_biome(
        base: &sunrise_protocol::registry::Dynamic,
        biome_id: &str,
    ) -> std::sync::Arc<sunrise_protocol::registry::Dynamic> {
        let mut registries = base.clone();
        registries.registries.insert(
            "minecraft:dimension_type".into(),
            vec![("minecraft:overworld".into(), None)],
        );
        registries.registries.insert(
            "minecraft:worldgen/biome".into(),
            vec![
                ("minecraft:plains".into(), None),
                (
                    biome_id.into(),
                    Some(sunrise_core::nbt::Tag::Compound(
                        sunrise_core::nbt::Compound::new(),
                    )),
                ),
            ],
        );
        std::sync::Arc::new(registries)
    }

    #[cfg(feature = "client")]
    fn assert_disconnected_with_generic_reason(
        game: &sunrise_client_gameplay::game::ClientGame<'_>,
    ) {
        assert!(
            game.client.remote.is_none(),
            "client must disconnect on invalid registry"
        );
        assert!(
            !game.client.connection_open,
            "connection must close on invalid registry"
        );
        let mode = &game
            .client
            .title
            .0
            .as_ref()
            .expect("the disconnect screen")
            .mode;
        assert!(matches!(
            mode,
            sunrise_client_lib::title::Mode::Multiplayer(
                sunrise_client_lib::multiplayer::Screen::Disconnected { reason, .. }
            ) if reason.translation_key() == Some("disconnect.genericReason")
        ));
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn invalid_biome_registry_disconnects_before_join_or_reconfigured_play() {
        with_client_game(|game| {
            // Joined with an invalid biome identifier (e.g. `..:bad` with supplied data) must
            // disconnect before marking the join complete or processing queued play packets.
            game.client.title.0 = None;
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "InvalidBiomeJoin").unwrap();
            game.connection_starts("example.invalid:25565", "InvalidBiomeJoin", false);
            let invalid =
                registries_with_biome(&game.client.remote.as_ref().unwrap().registries, "..:bad");
            push_events(
                game,
                [
                    Event::Joined {
                        registries: invalid,
                        datapacks: Default::default(),
                        uuid: 1,
                        name: "Alex".into(),
                    },
                    Event::Packet(Box::new(login_packet())),
                ],
            );
            game.poll_remote_events(true).unwrap();
            assert_disconnected_with_generic_reason(game);

            // Reconfigured with an invalid biome identifier after a valid play session must
            // disconnect rather than allowing play to resume with the previous registries.
            game.common.world_biomes = None;
            game.common.world_biomes_base = None;
            game.client.title.0 = None;
            let (_reader, mut writer) = connect_play_test(game, |_, _| {});
            if !game.client.remote.as_ref().unwrap().joined {
                game.wait_for_test_remote_event(|event| matches!(event, Event::Joined { .. }));
                game.poll_remote_events(true).unwrap();
            }
            game.connection_starts("example.invalid:25565", "InvalidBiomeReconfigure", false);
            let valid = registries_with_biome(
                &game.client.remote.as_ref().unwrap().registries,
                "test:marsh",
            );
            let invalid =
                registries_with_biome(&game.client.remote.as_ref().unwrap().registries, "..:bad");
            push_events(
                game,
                [
                    Event::Joined {
                        registries: valid,
                        datapacks: Default::default(),
                        uuid: 2,
                        name: "Steve".into(),
                    },
                    Event::Packet(Box::new(login_packet())),
                ],
            );
            game.poll_remote_events(true).unwrap();
            assert!(
                game.client
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.joined && r.logged_in && r.reconfiguring.is_none())
            );
            let biomes = game
                .common
                .world_biomes
                .as_deref()
                .unwrap_or_else(|| game.common.res.biomes());
            assert!(biomes.contains("test:marsh"));

            writer
                .send(&C::StartConfiguration(sunrise_protocol::packets::Empty).encode())
                .unwrap();
            game.wait_for_test_remote_event(|event| matches!(event, Event::Reconfiguring(_)));
            game.poll_remote_events(true).unwrap();
            assert!(game.client.remote.as_ref().unwrap().reconfiguring.is_some());
            push_events(
                game,
                [
                    Event::Reconfigured {
                        registries: invalid,
                        datapacks: Default::default(),
                    },
                    Event::Packet(Box::new(login_packet())),
                ],
            );
            game.poll_remote_events(true).unwrap();
            assert_disconnected_with_generic_reason(game);
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn malformed_trim_material_registry_disconnects_before_join() {
        with_client_game(|game| {
            // A malformed synced trim_material entry must fail the registry load and disconnect
            // instead of falling back to the client's own file for minecraft:gold.
            game.client.title.0 = None;
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "MalformedTrimJoin").unwrap();
            game.connection_starts("example.invalid:25565", "MalformedTrimJoin", false);
            let mut registries = (*game.client.remote.as_ref().unwrap().registries).clone();
            registries.registries.insert(
                "minecraft:dimension_type".into(),
                vec![("minecraft:overworld".into(), None)],
            );
            registries.registries.insert(
                "minecraft:trim_material".into(),
                vec![(
                    "minecraft:gold".into(),
                    Some(sunrise_core::nbt::Tag::Compound(
                        sunrise_core::nbt::Compound::new(),
                    )),
                )],
            );
            push_events(
                game,
                [
                    Event::Joined {
                        registries: std::sync::Arc::new(registries),
                        datapacks: Default::default(),
                        uuid: 1,
                        name: "Alex".into(),
                    },
                    Event::Packet(Box::new(login_packet())),
                ],
            );
            game.poll_remote_events(true).unwrap();
            assert_disconnected_with_generic_reason(game);
        });
    }
}
