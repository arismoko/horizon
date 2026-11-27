#[cfg(feature = "client")]
#[cfg(test)]
#[path = "claimed_item_use_tests_client.rs"]
pub(crate) mod claimed_item_use_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "gamemode_actions_tests_client.rs"]
pub(crate) mod gamemode_actions_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "chat_audit_tests_client.rs"]
pub(crate) mod chat_audit_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "interaction_event_tests_client.rs"]
pub(crate) mod interaction_event_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "native_entity_tests_client.rs"]
pub(crate) mod native_entity_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "camera_view_tests_client.rs"]
pub(crate) mod camera_view_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "client_camera_tests_client.rs"]
pub(crate) mod client_camera_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "riptide_tests_client.rs"]
pub(crate) mod riptide_tests;

#[cfg(all(test, feature = "client"))]
#[path = "recipe_properties/client_logging_out_tests.rs"]
mod client_logging_out_tests;

#[cfg(all(test, feature = "client"))]
#[path = "attack_target_tests_client.rs"]
mod attack_target_tests;

#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) mod tests {
    pub(crate) use crate::game::remote::tests::connect_menu_test;
    pub(crate) use crate::game::remote::tests::connect_play_test;
    pub(crate) use crate::game::remote::tests::spawn_menu_test;
    use sunrise_gameplay_common::game::GameGameExt1;

    pub(crate) mod client_explosion_tests {
        include!("client_explosion_tests_client.rs");
    }

    pub(crate) mod chat_window_and_input_tests {
        include!("chat_window_and_input_tests_client.rs");
    }

    pub(crate) mod play_bundle_tests {
        include!("play_bundle_tests_client.rs");
    }

    pub(crate) mod camera_draw_tests {
        include!("camera_draw_tests_client.rs");
    }

    pub(crate) mod camera_parity_red_tests {
        include!("camera_parity_red_tests_client.rs");
    }

    pub(crate) mod camera_death_tests {
        include!("camera_death_tests_client.rs");
    }

    pub(crate) mod client_hud_camera_tests {
        include!("client_hud_camera_tests_client.rs");
    }

    pub(crate) mod client_day_time_tests {
        include!("client_day_time_tests_client.rs");
    }

    pub(crate) mod client_level_environment_tests {
        include!("client_level_environment_tests_client.rs");
    }

    pub(crate) mod client_level_lifecycle_tests {
        use sunrise_gameplay_common::game::tables::WorldTablesExt;
        include!("client_level_lifecycle_tests_client.rs");
    }

    pub(crate) mod client_menu_tests {
        include!("client_menu_tests_client.rs");
    }

    pub(crate) mod client_vanilla_reads_tests {
        include!("client_vanilla_reads_tests_client.rs");
    }

    pub(crate) mod container_drop_tests {
        include!("container_drop_tests_client.rs");
    }

    pub(crate) mod dimension_transition_tests {
        include!("dimension_transition_tests_client.rs");
    }

    pub(crate) mod menu_packets_tests {
        include!("menu_packets_tests_client.rs");
    }

    pub(crate) mod client_close_tests {
        include!("client_close_tests_client.rs");
    }

    pub(crate) mod particle_group_tests {
        include!("particle_group_tests_client.rs");
    }

    pub(crate) mod spear_jab_tests {
        include!("spear_jab_tests_client.rs");
    }

    pub(crate) mod client_admission_tests {
        include!("client_admission_tests_client.rs");
    }

    pub(crate) mod client_emitters_tests {
        include!("client_emitters_tests_client.rs");
    }

    pub(crate) mod tracking_state_tests {
        include!("tracking_state_tests_client.rs");
    }

    #[test]
    #[ignore = "daily"]
    fn declared_gate_uses_its_class_for_blasts_and_predicted_use() {
        use std::sync::Arc;
        use sunrise_core::position::BlockPos;
        use sunrise_gameplay::game::native_guest_turn_tests::with_guests_at;
        use sunrise_level::chunk::Chunk;
        use sunrise_mod_package::{
            content::{Block, Content, GateSound},
            enabled::EnabledMods,
        };
        use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
        use sunrise_sim::{
            lan::{Out, PlayerSender},
            mod_content::{ContentMod, resource_root},
        };

        let base = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
        );
        let scratch = tempfile::tempdir().unwrap();
        let mut content = Content::declared("gates");
        let use_sound = "gates:use.open";
        let blast_sound = "gates:blast.close";
        let wire_sound = "minecraft:entity.zombie.ambient";
        content
            .declare(
                &[Block {
                    id: "gates:sites".into(),
                    template: "minecraft:oak_fence_gate".into(),
                    item: false,
                    open_sound: Some(GateSound::Sites {
                        on_use: Some(use_sound.into()),
                        on_explosion: None,
                        on_redstone: Some(wire_sound.into()),
                    }),
                    close_sound: Some(GateSound::Sites {
                        on_use: None,
                        on_explosion: Some(blast_sound.into()),
                        on_redstone: None,
                    }),
                    ..Default::default()
                }],
                &[],
            )
            .unwrap();
        let root = resource_root(
            &base,
            scratch.path(),
            &[ContentMod {
                id: "gates".into(),
                content,
                jars: Vec::new(),
            }],
            scratch.path(),
            &EnabledMods::new([("gates".into(), mod_api::Side::Both)]),
        )
        .unwrap();
        let pos = [3, 64, 4];
        let [x, y, z] = pos;
        let hit = sunrise_gameplay_common::game::Hit {
            pos,
            face: [0, 1, 0],
            point: [3.5, 64.875, 4.5],
            inside: false,
        };
        fn open(
            game: &sunrise_gameplay_common::game::CommonGame<'_>,
            x: i32,
            y: i32,
            z: i32,
        ) -> bool {
            let b = game.res.behaviours();
            b.table.bool(game.world.block(x, y, z), b.props.open)
        }
        let blocks = mod_api::SoundSource::Blocks.index();

        crate::game::server_side::native_entity_refs::tests::with_game_role_client(
            root.clone(),
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            |client| {
                client
                    .world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                with_guests_at(root, |game| {
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.sync_tick_chunks();
                    let gate = game
                        .pack
                        .assets
                        .registry
                        .default_state("gates:sites")
                        .unwrap();

                    let [actor, other] = [12, 13].map(|conn| {
                        let (tx, rx) = std::sync::mpsc::channel();
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .update_guest(conn, |guest| {
                                guest.sender = PlayerSender::new(tx, Arc::default());
                            })
                            .unwrap();
                        rx
                    });
                    let sounds =
                        |game: &sunrise_gameplay::game::ServerGame<'_>,
                         rx: &std::sync::mpsc::Receiver<Out>| {
                            let sync = game.server.lan.as_ref().unwrap().hub.sync_data();
                            rx.try_iter()
                                .flat_map(|out| match out {
                                    Out::Packet(body) => vec![body],
                                    Out::Packets(bodies) => bodies,
                                    Out::Disconnect(_) => Vec::new(),
                                })
                                .filter_map(|body| {
                                    let mut reader = Reader::new(&body);
                                    let id = reader.varint().unwrap();
                                    match Clientbound::decode(id, reader.rest()).unwrap() {
                                        Clientbound::Sound(packet) => {
                                            let name =
                                                packet.sound.1.map(|(id, _)| id).unwrap_or_else(
                                                    || {
                                                        sync.builtin
                                                            .get("minecraft:sound_event")
                                                            .unwrap()
                                                            .name(packet.sound.0)
                                                            .unwrap()
                                                            .to_owned()
                                                    },
                                                );
                                            Some((name, packet.source, packet.volume))
                                        }
                                        _ => None,
                                    }
                                })
                                .collect::<Vec<_>>()
                        };
                    let vanilla_gate = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:oak_fence_gate")
                        .unwrap();
                    game.options.dedicated = Some(Vec::new());
                    for (state, block, name) in [
                        (
                            vanilla_gate,
                            "minecraft:oak_fence_gate",
                            "minecraft:block.fence_gate.open",
                        ),
                        (gate, "gates:sites", use_sound),
                    ] {
                        let client_state =
                            client.pack.assets.registry.default_state(block).unwrap();
                        client.world.set(x, y, z, client_state).unwrap();
                        client.client.audio.0.capture = Some(Vec::new());
                        let (answer, dry) = client.remote_block_reacts(&hit, None, true);
                        assert!(answer.consumes_action());
                        client.predict_use(&dry, 1);
                        assert!(
                            open(client, x, y, z),
                            "the gate opens without a server round trip"
                        );
                        let capture = client.client.audio.0.capture.as_mut().unwrap();
                        assert_eq!(capture.len(), 1);
                        let predicted = capture.pop().unwrap();
                        assert_eq!((predicted.event.as_str(), predicted.source), (name, blocks));

                        game.world.set(x, y, z, state).unwrap();
                        game.world.block_log = Some(Vec::new());
                        assert!(
                            game.guest_use_item_on(
                                12,
                                game.guest_index(12).unwrap(),
                                hit,
                                sunrise_sim::first_person_sim::Hand::Main,
                                None,
                            )
                            .unwrap()
                        );
                        assert!(open(game, x, y, z));
                        game.lan_broadcast_effects();
                        assert!(sounds(game, &actor).is_empty(), "no echo to the clicker");
                        assert_eq!(sounds(game, &other), [(name.into(), blocks as i32, 1.0)]);
                    }

                    game.explosion_trigger_blocks(&mut [pos]).unwrap();
                    assert!(!open(game, x, y, z));
                    game.lan_broadcast_effects();
                    for listener in [&actor, &other] {
                        assert_eq!(
                            sounds(game, listener),
                            [(blast_sound.into(), blocks as i32, 1.0)]
                        );
                    }

                    let power = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:redstone_block")
                        .unwrap();
                    assert!(game.with_level(|b, level, ticks| {
                        b.set_block(level, ticks, BlockPos::new(x + 1, y, z), power, 3)
                    }));
                    game.server_process_level_events().unwrap();
                    assert!(open(game, x, y, z));
                    game.lan_broadcast_effects();
                    for listener in [&actor, &other] {
                        assert_eq!(
                            sounds(game, listener),
                            [(wire_sound.into(), blocks as i32, 1.0)]
                        );
                    }
                    assert_eq!(game.test_positional_sound_count(blast_sound), Some(&1));
                    let powered = game
                        .with_props(gate, &[("open", "true"), ("powered", "true")])
                        .unwrap();
                    game.world.set(x, y, z, powered).unwrap();
                    game.explosion_trigger_blocks(&mut [pos]).unwrap();
                    assert_eq!(game.world.block(x, y, z), powered);
                    assert_eq!(game.test_positional_sound_count(blast_sound), Some(&1));
                });
            },
        );
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "movement_input_tests_client.rs"]
pub(crate) mod movement_input_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "player_tick_tests_client.rs"]
pub(crate) mod player_tick_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "rider_tick_tests_client.rs"]
pub(crate) mod rider_tick_tests;
#[path = "sync.rs"]
mod sync_reads;

#[path = "raw_ingress_client.rs"]
pub(crate) mod raw_ingress;

#[path = "raw_ingress.rs"]
pub(crate) mod ingress_state;

#[path = "item_use_client.rs"]
pub(crate) mod item_use;

#[path = "fall_sound_client.rs"]
pub(crate) mod fall_sound;

#[path = "client_level_tick_client.rs"]
pub(crate) mod client_level_tick;

#[path = "abilities_client.rs"]
pub(crate) mod abilities;
