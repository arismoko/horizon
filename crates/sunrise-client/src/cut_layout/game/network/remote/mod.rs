#[cfg(test)]
pub(crate) mod block_entity_data_tests;
#[cfg(test)]
pub(crate) mod chat_audit_tests;
#[cfg(test)]
pub(crate) mod claimed_item_use_tests;
#[cfg(test)]
pub(crate) mod interaction_event_tests;

#[cfg(test)]
pub(crate) mod native_entity_tests;
#[cfg(test)]
pub(crate) mod no_gravity_data_tests;

#[cfg(test)]
pub(crate) mod camera_view_tests;
#[cfg(test)]
pub(crate) mod effect_blend_tests;
#[cfg(all(test, feature = "client"))]
pub(crate) mod level_sound_tests;

#[cfg(test)]
pub(crate) mod attachment_query_tests;
#[cfg(test)]
pub(crate) mod passage_cross_tests;
#[cfg(test)]
pub(crate) mod respawn_motion_tests;

#[cfg(all(test, feature = "client"))]
pub(crate) mod tests {
    use sunrise_gameplay_common::game::tables::GameItems;
    pub(crate) mod client_explosion_tests {
        include!("client_explosion_tests.rs");
    }

    use sunrise_gameplay_common::game::api_values::TryIntoApi;

    pub(crate) mod block_entity_state_tests {
        include!("block_entity_state_tests.rs");
    }
    pub(crate) mod chat_decoration_tests {
        include!("chat_decoration_tests.rs");
    }
    pub(crate) mod chat_event_tests {
        include!("chat_event_tests.rs");
    }
    pub(crate) mod chat_window_and_input_tests {
        include!("chat_window_and_input_tests.rs");
    }
    pub(crate) mod play_bundle_tests {
        include!("play_bundle_tests.rs");
    }
    pub(crate) mod chunk_storage_tests {
        include!("chunk_storage_tests.rs");
    }
    pub(crate) mod click_ending_tests {
        include!("click_ending_tests.rs");
    }
    pub(crate) mod client_attributes_tests {
        include!("client_attributes_tests.rs");
    }
    pub(crate) mod client_block_changed_tests {
        include!("client_block_changed_tests.rs");
    }
    pub(crate) mod camera_draw_tests {
        include!("camera_draw_tests.rs");
    }
    pub(crate) mod camera_parity_red_tests {
        include!("camera_parity_red_tests.rs");
    }
    pub(crate) mod set_camera_kinds_tests {
        include!("set_camera_kinds_tests.rs");
    }
    pub(crate) mod camera_death_tests {
        include!("camera_death_tests.rs");
    }
    pub(crate) mod client_hud_camera_tests {
        include!("client_hud_camera_tests.rs");
    }
    pub(crate) mod client_day_time_tests {
        include!("client_day_time_tests.rs");
    }
    pub(crate) mod client_epochs_tests {
        include!("client_epochs_tests.rs");
    }
    pub(crate) mod client_handles_tests {
        use sunrise_gameplay_common::game::GameGameExt1;
        include!("client_handles_tests.rs");
    }
    pub(crate) mod client_level_environment_tests {
        use sunrise_gameplay_common::game::weather::WeatherGameExt;
        include!("client_level_environment_tests.rs");
    }
    pub(crate) mod client_level_lifecycle_tests {
        include!("client_level_lifecycle_tests.rs");
    }
    pub(crate) mod client_menu_tests {
        include!("client_menu_tests.rs");
    }
    pub(crate) mod client_player_reads_tests {
        include!("client_player_reads_tests.rs");
    }
    pub(crate) mod client_vanilla_reads_tests {
        include!("client_vanilla_reads_tests.rs");
    }
    pub(crate) mod client_read_tests {
        include!("client_read_tests.rs");
    }
    pub(crate) mod container_drop_tests {
        include!("container_drop_tests.rs");
    }
    pub(crate) mod creative_sync_tests {
        include!("creative_sync_tests.rs");
    }
    pub(crate) mod damage_sound_tests {
        include!("damage_sound_tests.rs");
    }
    pub(crate) mod dig_abort_tests {
        include!("dig_abort_tests.rs");
    }
    pub(crate) mod dimension_transition_tests {
        include!("dimension_transition_tests.rs");
    }
    pub(crate) mod game_event_tests {
        include!("game_event_tests.rs");
    }
    pub(crate) mod hand_operated_tests {
        include!("hand_operated_tests.rs");
    }
    pub(crate) mod hand_swap_tests {
        include!("hand_swap_tests.rs");
    }
    pub(crate) mod hanging_entity_age_tests {
        include!("hanging_entity_age_tests.rs");
    }
    pub(crate) mod main_hand_tests {
        include!("main_hand_tests.rs");
    }
    pub(crate) mod menu_click_order_tests {
        include!("menu_click_order_tests.rs");
    }
    pub(crate) mod menu_generation_tests {
        include!("menu_generation_tests.rs");
    }
    pub(crate) mod menu_packets_tests {
        include!("menu_packets_tests.rs");
    }
    pub(crate) mod client_close_tests {
        include!("client_close_tests.rs");
    }
    pub(crate) mod menu_title_tests {
        include!("menu_title_tests.rs");
    }
    pub(crate) mod movement_snap_tests {
        include!("movement_snap_tests.rs");
    }
    pub(crate) mod object_interactions_tests {
        include!("object_interactions_tests.rs");
    }
    pub(crate) mod particle_group_tests {
        include!("particle_group_tests.rs");
    }
    pub(crate) mod placement_preview_tests {
        include!("placement_preview_tests.rs");
    }
    pub(crate) mod player_animation_tests {
        include!("player_animation_tests.rs");
    }
    pub(crate) mod player_sleep_tests {
        include!("player_sleep_tests.rs");
    }
    pub(crate) mod predicted_block_notice_tests {
        use sunrise_gameplay_common::game::GameGameExt1;
        include!("predicted_block_notice_tests.rs");
    }
    pub(crate) mod predicted_throw_tests {
        include!("predicted_throw_tests.rs");
    }
    pub(crate) mod remote_columns_tests {
        include!("remote_columns_tests.rs");
    }
    pub(crate) mod server_fire_flag_tests {
        include!("server_fire_flag_tests.rs");
    }
    pub(crate) mod slot_mapping_tests {
        include!("slot_mapping_tests.rs");
    }
    pub(crate) mod sound_entity_tests {
        include!("sound_entity_tests.rs");
    }

    #[test]
    #[ignore = "daily"]
    fn named_remote_players_are_told_one_by_one() {
        use mod_api::{EntityTickPostEvent, EntityTickPreEvent, Hooks, HostSide, ModEntry, Mods};
        use std::sync::{Arc, Mutex};
        type Calls = Arc<Mutex<Vec<(bool, Vec<u32>)>>>;
        struct Named(Calls);
        #[mod_api::__subscriptions]
        impl Hooks for Named {
            fn entity_tick_pre(
                &self,
                _: HostSide<'_>,
                event: &mut EntityTickPreEvent<'_>,
            ) -> mod_api::Flow {
                self.0.lock().unwrap().push((true, vec![event.entity]));
                mod_api::Flow::Continue
            }
            fn entity_tick_post(&self, _: HostSide<'_>, event: &mut EntityTickPostEvent<'_>) {
                self.0.lock().unwrap().push((false, vec![event.entity]));
            }
        }
        let heard: Calls = Arc::default();
        let mods: Mods = vec![ModEntry::new(
            "remote-player-ticks",
            Box::new(Named(heard.clone())),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(client, "avatar-tick-client").unwrap();
                let remote = game.client.remote.as_mut().unwrap();
                let kind = remote.player_type.as_ref().unwrap().clone();
                for id in [66, 67] {
                    remote.level_players.insert(id, id as u128);
                    game.common.mobs.push(sunrise_sim::mobs::Mob::new(
                        id as u32,
                        kind.clone(),
                        sunrise_core::geometry::Vec3::ZERO,
                        0.0,
                    ));
                }
                game.tick_remote_entities(false);
            });
        });
        assert_eq!(
            *heard.lock().unwrap(),
            [
                (true, vec![66]),
                (false, vec![66]),
                (true, vec![67]),
                (false, vec![67])
            ]
        );
    }

    pub(crate) mod spear_jab_tests {
        include!("spear_jab_tests.rs");
    }
    pub(crate) mod client_admission_tests {
        include!("client_admission_tests.rs");
    }
    pub(crate) mod tab_publication_tests {
        include!("tab_publication_tests.rs");
    }
    pub(crate) mod thrown_item_swing_tests {
        include!("thrown_item_swing_tests.rs");
    }
    pub(crate) mod client_emitters_tests {
        include!("client_emitters_tests.rs");
    }
    pub(crate) mod tracking_state_tests {
        include!("tracking_state_tests.rs");
    }
    pub(crate) mod use_flag_tests {
        include!("use_flag_tests.rs");
    }

    /// A client connecting through a local pipe and through configuration, as
    /// the client menu tests drive it: the server end's reader (what the
    /// client sends) and writer.
    pub(crate) fn connect_menu_test(
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    ) -> (
        sunrise_protocol::connection::ConnReader,
        sunrise_protocol::connection::ConnWriter,
    ) {
        connect_play_test(game, |_, _| {})
    }

    pub(crate) fn connect_play_test(
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
        configure: impl FnOnce(
            &mut sunrise_protocol::connection::ConnReader,
            &mut sunrise_protocol::connection::ConnWriter,
        ),
    ) -> (
        sunrise_protocol::connection::ConnReader,
        sunrise_protocol::connection::ConnWriter,
    ) {
        use sunrise_protocol::connection::{Connection, Pipe};
        game.options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        let (client, server) = Pipe::pair();
        game.connect_local(client, "MenuReadTest").unwrap();
        finish_play_test(game, Connection::local(server), configure)
    }

    pub(crate) fn finish_play_test(
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
        server: sunrise_protocol::connection::Connection,
        configure: impl FnOnce(
            &mut sunrise_protocol::connection::ConnReader,
            &mut sunrise_protocol::connection::ConnWriter,
        ),
    ) -> (
        sunrise_protocol::connection::ConnReader,
        sunrise_protocol::connection::ConnWriter,
    ) {
        use sunrise_protocol::packets::{Empty, configuration as cfg, login};
        let (mut reader, mut writer) = server.split();
        reader
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        reader.recv().unwrap();
        reader.recv().unwrap();
        writer
            .send(
                &login::Clientbound::LoginFinished(login::LoginFinished {
                    profile: login::GameProfile {
                        uuid: 42,
                        name: "MenuReadTest".into(),
                        properties: vec![],
                    },
                    session: 1,
                })
                .encode(),
            )
            .unwrap();
        let (id, body) = reader.recv().unwrap();
        assert!(matches!(
            login::Serverbound::decode(id, &body).unwrap(),
            login::Serverbound::LoginAcknowledged(_)
        ));
        reader.recv().unwrap();
        reader.recv().unwrap();
        configure(&mut reader, &mut writer);
        writer
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        game.acknowledge_test_configuration();
        let (id, body) = reader.recv().unwrap();
        assert!(matches!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::FinishConfiguration(_)
        ));
        (reader, writer)
    }

    /// The test server offers its payload manifest and reads the client's offer.
    pub(crate) fn offer_test_payloads(
        reader: &mut sunrise_protocol::connection::ConnReader,
        writer: &mut sunrise_protocol::connection::ConnWriter,
        manifest: &sunrise_protocol::payload_negotiation::Manifest,
    ) {
        use sunrise_protocol::{
            packets::configuration as cfg,
            payload_negotiation::{CHANNEL, Control},
        };
        writer
            .send(
                &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                    channel: CHANNEL.into(),
                    data: Control::Offer(manifest.clone()).encode().unwrap(),
                })
                .encode(),
            )
            .unwrap();
        let (id, body) = reader.recv().unwrap();
        assert!(
            matches!(cfg::Serverbound::decode(id, &body).unwrap(), cfg::Serverbound::CustomPayload(p) if p.channel == CHANNEL)
        );
    }

    /// Spawns the connected client in the overworld as entity 7, profile 42.
    pub(crate) fn spawn_menu_test(
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    ) -> sunrise_protocol::packets::play::SpawnInfo {
        use sunrise_client::level_loading::Reason;
        use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
        use sunrise_protocol::packets::play::SpawnInfo;
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        spawn
    }

    use {
        sunrise_gameplay_common::game::remote::boat_interact_passes,
        sunrise_gameplay_common::game::remote::item_frame_interact_passes,
        sunrise_gameplay_common::game::remote::remote_object_swings,
    };

    #[test]
    #[ignore = "daily"]
    fn block_alias_survival_consumption_and_client_prediction() {
        use std::{path::PathBuf, sync::Arc};
        use sunrise_items::items::ItemStack;
        use sunrise_level::chunk::Chunk;
        use sunrise_mod_package::content::{Block, Content, Item};
        use sunrise_sim::mod_content::{ContentMod, build};
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
            sunrise_gameplay_common::game::World,
        };
        let base = PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
        );
        let root = std::env::temp_dir().join(format!("sunrise-alias-game-{}", std::process::id()));
        let mut content = Content::declared("ruby");
        content
            .declare(
                &[Block {
                    id: "ruby:frame".into(),
                    template: "minecraft:stone".into(),
                    item: false,
                    ..Default::default()
                }],
                &["empty", "filled"].map(|name| Item {
                    id: format!("ruby:{name}"),
                    template: "minecraft:stick".into(),
                    block: Some("ruby:frame".into()),
                    ..Default::default()
                }),
            )
            .unwrap();
        let mods = [ContentMod {
            id: "ruby".into(),
            content,
            jars: Vec::new(),
        }];
        let inventory = sunrise_mod_package::enabled::EnabledMods::new([(
            "ruby".to_owned(),
            mod_api::Side::Both,
        )]);
        build(&base, std::path::Path::new(""), &mods, &root, &inventory).unwrap();
        let (registry, _) = sunrise_world::level::data::load_registry_with_flags(&root).unwrap();
        crate::game::api_values::register_game_keys((0..registry.len()).map(|id| {
            registry
                .state(sunrise_world::registry::BlockStateId(id as u32))
                .unwrap()
                .name
                .clone()
        }));
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.world = World::new(game.res.facts().air, 1);
        for x in -1..=1 {
            for z in -1..=1 {
                game.world
                    .chunks
                    .insert((x, z), Arc::new(Chunk::empty(x, z)));
            }
        }
        game.sync_tick_chunks();
        game.set_mode_id(0);
        game.player = None;
        game.selected = 0;
        let target = game
            .pack
            .assets
            .registry
            .default_state("ruby:frame")
            .unwrap();
        for (i, name) in ["ruby:empty", "ruby:filled"].iter().enumerate() {
            let pos = [i as i32 * 2, 70, 0];
            let hit = sunrise_gameplay_common::game::Hit {
                pos,
                face: [0, 1, 0],
                inside: false,
                point: [f64::from(pos[0]) + 0.5, 70.0, 0.5],
            };
            let stack = ItemStack::new(name, 2);
            assert_eq!(game.items().get(name).unwrap().block, Some(target));
            *game.held_mut() = Some(stack);
            assert!(game.use_hand(Some(&hit)).unwrap());
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), target);
            let held = game.held().unwrap();
            assert_eq!((&*held.item, held.count), (*name, 1));
        }
        let blocked = sunrise_gameplay_common::game::Hit {
            pos: [0, 70, 0],
            face: [0, 1, 0],
            inside: false,
            point: [0.5, 71.0, 0.5],
        };
        game.set_mode_id(2);
        let before = game.held().cloned();
        game.use_hand(Some(&blocked)).unwrap();
        assert_eq!(
            game.held(),
            before.as_ref(),
            "adventure refusal keeps the alias stack"
        );
        drop(game);

        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        let mut client = sunrise_client_gameplay::game::ClientGame::new(
            &loaded.resources,
            loaded.client_res.unwrap(),
            loaded.pack,
            loaded.client_pack.unwrap(),
            loaded.gui.unwrap(),
            options,
            settings,
            sunrise_client_lib::audio::Audio::silent(
                &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
            ),
            None,
            sunrise_gameplay::game::server_side::integrated::serve,
            sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
            crate::game::session_verifier(),
            crate::game::profile_resolver,
        )
        .unwrap();
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        client.connect_local(pipe, "AliasTest").unwrap();
        client.world = World::new(client.res.facts().air, 1);
        for x in -1..=1 {
            for z in -1..=1 {
                client
                    .world
                    .chunks
                    .insert((x, z), Arc::new(Chunk::empty(x, z)));
            }
        }
        client.set_local_mode(sunrise_command::args::GameMode::Survival);
        client.player = None;
        client.selected = 0;
        let target = client
            .pack
            .assets
            .registry
            .default_state("ruby:frame")
            .unwrap();
        for (i, name) in ["ruby:empty", "ruby:filled"].iter().enumerate() {
            let pos = [i as i32 * 2, 70, 0];
            let hit = sunrise_gameplay_common::game::Hit {
                pos,
                face: [0, 1, 0],
                inside: false,
                point: [f64::from(pos[0]) + 0.5, 70.0, 0.5],
            };
            let stack = ItemStack::new(name, 2);
            assert_eq!(client.items().get(name).unwrap().block, Some(target));
            let preview = client
                .remote_can_place(&hit, &stack)
                .expect("client predicts the alias");
            assert_eq!([preview.pos.x, preview.pos.y, preview.pos.z], pos);
            assert_eq!(
                client.world.block(pos[0], pos[1], pos[2]),
                client.res.facts().air,
                "prediction is dry"
            );
        }
        drop(client);
        std::fs::remove_dir_all(root).unwrap();
    }

    use sunrise_core::nbt::Tag;
    use sunrise_items::items::ItemStack;
    use sunrise_level::containers::{Beacon, BlockEntity};

    /// #12040: a server's client moves at the speed the server's attributes give it, never its
    /// effects' own. Vanilla's server removes a Speed that runs out in ServerPlayer.doTick, after
    /// the tick's sendDirtyEntityData: ClientboundRemoveMobEffectPacket goes out at once and the
    /// movement speed without the effect's modifier a tick later, so for that tick the client still
    /// runs fast (a potion drunk starts the same way). A client a tick off is what GrimAC's
    /// Simulation flagged.
    #[test]
    #[ignore = "daily"]
    fn a_servers_client_takes_its_speed_from_the_attributes_not_the_effect() {
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::play::{
            AttributeModifier, AttributeSnapshot, Clientbound as C, RemoveMobEffect,
            UpdateAttributes, UpdateMobEffect,
        };
        use sunrise_sim::motion::PlayerMotion;
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_client_lib::audio::Audio, sunrise_gameplay_common::game::Options,
            sunrise_gameplay_common::game::Progress,
        };
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        let mut game = sunrise_client_gameplay::game::ClientGame::new(
            &loaded.resources,
            loaded.client_res.unwrap(),
            loaded.pack,
            loaded.client_pack.unwrap(),
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
            sunrise_gameplay::game::server_side::integrated::serve,
            sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
            crate::game::session_verifier(),
            crate::game::profile_resolver,
        )
        .unwrap();
        game.init_remote_for_replay().unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        let me = game.client.remote.as_ref().unwrap().entity_id;
        let speed = game.remote_effect_network_id("speed").unwrap();
        let attribute = {
            let p = game.player.as_ref().unwrap();
            let key = p.attributes.key("minecraft:movement_speed").unwrap();
            key.attribute().registry_id()
        };
        // Player.createAttributes: 0.1F.
        let walking = f64::from(0.1f32);
        // Speed II: 0.2 (a float) a level, multiplying the total.
        let amount = f64::from(0.2f32) * 2.0;
        let attributes = |sped: bool| {
            C::UpdateAttributes(UpdateAttributes {
                id: me,
                values: vec![AttributeSnapshot {
                    attribute,
                    base: walking,
                    modifiers: sped
                        .then(|| AttributeModifier {
                            id: "minecraft:effect.speed".into(),
                            amount,
                            operation: 2,
                        })
                        .into_iter()
                        .collect(),
                }],
            })
        };
        // Each step: the packets of one server tick, then a client tick.
        let mut step = |packets: Vec<C>| {
            for packet in packets {
                game.handle_packet(packet).unwrap();
            }
            game.tick_client_effects();
            game.player
                .as_mut()
                .unwrap()
                .attribute("minecraft:movement_speed")
                .unwrap()
        };
        let effect = C::UpdateMobEffect(UpdateMobEffect {
            id: me,
            effect: speed,
            amplifier: 1,
            duration: 60,
            flags: 2 | 4,
        });
        let fast = walking * (1.0 + amount);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let seen = [
            step(vec![effect]),
            step(vec![attributes(true)]),
            step(Vec::new()),
            step(vec![C::RemoveMobEffect(RemoveMobEffect {
                id: me,
                effect: speed,
            })]),
            step(vec![attributes(false)]),
        ];
        let expected = [walking, fast, fast, fast, walking];
        assert!(
            seen.iter().zip(expected).all(|(&s, e)| close(s, e)),
            "speed on each tick: {seen:?}, vanilla's client: {expected:?}"
        );
    }
}

#[cfg(test)]
pub(crate) mod boat_push_order_tests;
#[cfg(test)]
pub(crate) mod item_use_tick_tests;
#[cfg(test)]
pub(crate) mod movement_input_tests;
#[cfg(test)]
pub(crate) mod placement_prediction_tests;
#[cfg(test)]
#[path = "player_tick_tests.rs"]
pub(crate) mod player_tick_tests;
#[cfg(test)]
pub(crate) mod smoke_source_tests;
#[cfg(test)]
pub(crate) mod swim_start_tests;

#[cfg(all(test, feature = "client"))]
pub(crate) mod tables;

#[cfg(all(test, feature = "client"))]
pub(crate) mod general;

#[cfg(all(test, feature = "client"))]
pub(crate) mod fall_sound;

#[cfg(all(test, feature = "client"))]
pub(crate) mod abilities;
