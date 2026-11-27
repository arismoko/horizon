#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {

    use super::super::native_entity_refs::tests::{record_key, with_game};
    use mod_api::{
        BlockPart, EntityRef, Hooks, LivingRef, LoopContext, MakesOpenTrapdoorAboveClimbableEvent,
        Registrar, WorldEpoch,
    };
    use sunrise_core::geometry::Vec3;
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay_common::game::block_props::BlockPropsGameExt1;
    use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6;
    use sunrise_gameplay_common::game::native_living_body::NativeLivingBodyGameExt4;
    use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
    use sunrise_gameplay_common::game_side;
    use sunrise_sim::entities::ArmorStand;
    use sunrise_sim::mobs::{Mob, villager};
    use sunrise_sim::motion::{MotionWorld, PlayerMotion};

    /// A live cow `id` at `(5.5, 64, 5.5)`, with its handle.
    fn cow(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32) -> LivingRef {
        fixture_resident_column(&mut game.world, (0, 0));
        let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
        game.mobs
            .push(Mob::new(id, kind, Vec3::new(5.5, 64.0, 5.5), 0.0));
        game.living(game.entity(id).unwrap()).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn start_gliding_sets_the_live_flag_and_refuses_other_kinds() {
        with_game(|game| {
            let mob = cow(game, 12);
            let game = &mut *game;
            assert_eq!(game.living_is_gliding(mob), Some(false));
            assert!(game.living_start_gliding(mob));
            assert!(game.mobs[0].extra.fall_flying);
            assert_eq!(game.living_is_gliding(mob), Some(true));
            // An armor stand has no glide state, so the write changes nothing.
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let living = game.living(game.entity(13).unwrap()).unwrap();
            assert!(!game.living_start_gliding(living));
            // The player in place and a guest start their own body's flag.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 20);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            assert!(game.living_start_gliding(player));
            assert!(game.player.as_ref().unwrap().fall_flying);
            guest_server(game, false, true);
            let (guest, _rx) = guest(game, 21);
            assert!(game.living_start_gliding(guest));
            assert!(game.command_guest(21).unwrap().fall_flying);
            assert!(game.living_stop_gliding(guest));
            assert!(!game.command_guest(21).unwrap().fall_flying);
        })
    }

    #[test]
    #[ignore = "daily"]
    fn mob_flags_write_the_stored_state_and_survive_the_save() {
        with_game(|game| {
            let mob = cow(game, 12);
            let game = &mut *game;
            // The mob flags: off, on, and what each write leaves behind.
            // They are MobRef's own methods (Mob.isNoAi and its kin).
            let mob_ref = game.mob(game.entity(12).unwrap()).unwrap();
            assert_eq!(game.mob_no_ai(mob_ref), Some(false));
            assert!(game.mob_set_no_ai(mob_ref, true));
            assert!(game.mobs[0].no_ai);
            assert_eq!(game.mob_no_ai(mob_ref), Some(true));
            assert_eq!(game.mob_persistence_required(mob_ref), Some(false));
            assert_eq!(game.mob_requires_custom_persistence(mob_ref), Some(false));
            assert!(game.mob_set_persistence_required(mob_ref));
            assert!(game.mobs[0].persistent);
            assert_eq!(game.mob_persistence_required(mob_ref), Some(true));
            // The baby age: a full day of growing up, and grown on the spot.
            assert_eq!(game.living_baby(mob), Some(false));
            assert!(game.mob_set_baby(mob_ref, true));
            assert_eq!(game.mobs[0].extra.age, -24000);
            assert_eq!(game.living_baby(mob), Some(true));
            let _ = &mob_ref;
            // The glide flag clears the way the game's own glide end does.
            game.mobs[0].extra.fall_flying = true;
            game.mobs[0].extra.fall_fly_ticks = 9;
            assert_eq!(game.living_is_gliding(mob), Some(true));
            assert_eq!(game.living_gliding_ticks(mob), Some(9));
            assert!(game.living_stop_gliding(mob));
            assert!(!game.mobs[0].extra.fall_flying);
            // A villager's bed helpers answer for an awake cow.
            assert!(!villager::sleeping(&game.mobs[0]));
            assert_eq!(game.living_is_sleeping(mob), Some(false));
            assert_eq!(game.living_sleeping_pos(mob), None);
            assert!(!game.living_start_sleeping(mob, [5, 64, 5]));
            assert!(!game.living_stop_sleeping(mob));
            // NoAI, PersistenceRequired and Age are the save's own tags.
            let tag = sunrise_sim::entity_nbt::mob_to_nbt(
                &game.mobs[0],
                [1, 2, 3, 4],
                None,
                Some(game.pack.assets.registry),
                |_| None,
                &Default::default(),
            );
            let restored = sunrise_sim::entity_nbt::mob_from_nbt(
                &tag,
                game.res.mob_types(),
                7,
                Some(game.pack.assets.registry),
            )
            .unwrap();
            assert!(restored.no_ai && restored.persistent);
            assert_eq!(restored.extra.age, -24000);
            // A zombie keeps its flags through the same cycle (its age
            // never saves, as vanilla's zombie baby never grows).
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(13, kind, Vec3::new(5.5, 64.0, 5.5), 0.0));
            let zombie_ref = game.mob(game.entity(13).unwrap()).unwrap();
            assert!(game.mob_set_persistence_required(zombie_ref));
            assert!(game.mob_set_no_ai(zombie_ref, true));
            let tag = sunrise_sim::entity_nbt::mob_to_nbt(
                &game.mobs[1],
                [5, 6, 7, 8],
                None,
                Some(game.pack.assets.registry),
                |_| None,
                &Default::default(),
            );
            let restored = sunrise_sim::entity_nbt::mob_from_nbt(
                &tag,
                game.res.mob_types(),
                8,
                Some(game.pack.assets.registry),
            )
            .unwrap();
            assert!(restored.persistent && restored.no_ai);
            // A grown-up cow reads back as one, too.
            assert!(game.mob_set_baby(mob_ref, false));
            assert_eq!(game.mobs[0].extra.age, 0);
            assert_eq!(game.living_baby(mob), Some(false));
            // A stale handle reads None and writes false.
            let stale = LivingRef::new(EntityRef::new(
                WorldEpoch::new(mob.entity().epoch().get() + 1),
                12,
            ));
            assert!(game.mob(stale.entity()).is_none());
            assert_eq!(game.living_is_sleeping(stale), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_summons_join_write_survives_the_spawn() {
        use mod_api::{EntityJoinLevelEvent, Flow, Hooks, ModEntry, ServerHost};
        struct Joiner;
        #[mod_api::__subscriptions]
        impl Hooks for Joiner {
            fn entity_join_level(
                &self,
                host: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                if event.kind == "minecraft:zombie"
                    && let Some(er) =
                        mod_api::prelude::EntitiesHost::entity(&*host, event.subject.id())
                    && let Some(m) = mod_api::prelude::MobsHost::mob(&*host, er)
                {
                    let _ = m.set_persistence_required(&mut *host);
                    let _ = m.set_no_ai(&mut *host, true);
                    let _ = m.set_baby(&mut *host, true);
                }
                Flow::Continue
            }
        }
        mod_platform::testing::with_mods(
            vec![ModEntry::new("join_persist", Box::new(Joiner))].into(),
            || {
                with_game(|game| {
                    fixture_resident_column(&mut game.world, (0, 0));
                    let before = game.mobs.len();
                    game.spawn_mob_spawned(
                        "zombie",
                        [5.0, 64.0, 5.0],
                        sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                            sunrise_sim::mobs::spawn_rules::Reason::Command,
                        ),
                    )
                    .unwrap();
                    let m = &game.mobs[before];
                    assert!(m.persistent, "the join hook's write survives");
                    assert!(m.no_ai && m.is_baby());
                });
            },
        );
        // Without a mod the summoned mob stays plain vanilla: not
        // persistent, as SummonCommand never calls setPersistenceRequired.
        with_game(|game| {
            let before = game.mobs.len();
            game.spawn_mob_spawned(
                "zombie",
                [5.0, 64.0, 5.0],
                sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                    sunrise_sim::mobs::spawn_rules::Reason::Command,
                ),
            )
            .unwrap();
            assert!(!game.mobs[before].persistent, "a summon stays vanilla");
        });
    }

    pub(crate) fn guest_server(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        dedicated: bool,
        owner: bool,
    ) {
        use sunrise_sim::lan::{NetConfig, hub::HubConfig};
        use {
            sunrise_gameplay::game::server_side::lan::LanSettings,
            sunrise_gameplay_common::game::execution_role::ExecutionRole,
        };
        game.options.execution_role = if dedicated {
            ExecutionRole::DedicatedServer
        } else {
            ExecutionRole::IntegratedServer
        };
        game.options.dedicated = dedicated.then(Vec::new);
        let settings = LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
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
            integrated: !dedicated,
            levels: game.server.level_shapes.clone(),
            level_seeds: game.server.levels.0.runtime.seeds(),
        };
        game.lan_host(settings, net, config).unwrap();
        game.server.lan.as_mut().unwrap().hub.singleplayer_owner = owner.then_some(12);
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
    }

    pub(crate) fn guest(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        conn: u32,
    ) -> (LivingRef, std::sync::mpsc::Receiver<sunrise_sim::lan::Out>) {
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{Joined, PlayerSender, guest::Guest};
        let (tx, rx) = std::sync::mpsc::channel();
        let mut g = Guest::new(
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
            [3.0, 64.0, 5.0],
            10,
        );
        g.placed = true;
        g.loaded = true;
        g.loading = false;
        g.load_timeout = 0;
        g.dimension = game.dim.id().to_owned();
        // The test client's initial column has already arrived, so the
        // real tracker can pair the other guest and publish dirty flags.
        g.sent_chunks.insert((0, 0));
        game.server.lan.as_mut().unwrap().hub.guests.push(g);
        let living = game.living(game.entity(conn).unwrap()).unwrap();
        (living, rx)
    }

    fn bed(game: &mut sunrise_gameplay::game::ServerGame<'_>, name: &str, pos: [i32; 3]) {
        let head = game
            .pack
            .assets
            .registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: name.into(),
                properties: [("facing", "north"), ("occupied", "false"), ("part", "head")]
                    .into_iter()
                    .map(|(k, v)| (k.into(), v.into()))
                    .collect(),
            })
            .unwrap();
        game.world.set(pos[0], pos[1], pos[2], head).unwrap();
        if name != "minecraft:straw_bed" {
            let mut foot = game.pack.assets.registry.state(head).unwrap().clone();
            foot.properties.insert("part".into(), "foot".into());
            let foot = game.pack.assets.registry.id(&foot).unwrap();
            game.world.set(pos[0], pos[1], pos[2] + 1, foot).unwrap();
        }
    }

    pub(crate) fn sync_guests(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        use sunrise_sim::lan::hub::TickCtx;
        let mut ctx = TickCtx {
            world: &mut game.common.world,
            entities: &[],
            next_entity: &mut game.server.next_entity,
            spawn: [0.0, 64.0, 0.0],
            spawn_angle: [0.0; 2],
            spawn_dimension: "minecraft:overworld",
            sea_level: 63,
            difficulty: 1,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: Some(game.common.collision),
            border: &game.common.border,
            streaming: None,
        };
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .tick_level(&mut ctx, None);
    }

    pub(crate) fn packets(
        rx: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
    ) -> Vec<sunrise_protocol::packets::play::Clientbound> {
        use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
        use sunrise_sim::lan::Out;
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                _ => Vec::new(),
            })
            .map(|body| {
                let mut r = Reader::new(&body);
                let id = r.varint().unwrap();
                Clientbound::decode(id, r.rest()).unwrap()
            })
            .collect()
    }

    fn has_bed_packet(
        packets: &[sunrise_protocol::packets::play::Clientbound],
        bed: Option<[i32; 3]>,
    ) -> bool {
        use sunrise_protocol::{entity_data::Value, packets::play::Clientbound as C};
        packets.iter().any(|p| matches!(p, C::SetEntityData(p) if p.id == 12
            && p.values.iter().any(|(index, _, v)| *index == 6 && matches!(v, Value::Id(pose) if *pose == if bed.is_some() { 2 } else { 0 }))
            && p.values.iter().any(|(index, _, v)| *index == 14 && matches!(v, Value::OptPos(pos) if *pos == bed))))
    }

    #[test]
    #[ignore = "daily"]
    fn living_flags_normal_guest_sleep_writes_survive_turns_and_sync() {
        use sunrise_protocol::packets::play::Clientbound as C;
        use sunrise_sim::pose::Pose;
        // The integrated owner, a LAN guest, and a dedicated guest all act
        // through the real enter/leave path, not a standalone window body.
        for (dedicated, owner) in [(false, true), (false, false), (true, false)] {
            with_game(|game| {
                guest_server(game, dedicated, owner);
                let (living, rx) = guest(game, 12);
                // Outside the turn the client owns jump input. Only the
                // active body exposes and accepts the controller's state.
                assert_eq!(game.living_is_jumping(living), Some(false));
                assert!(!game.living_set_is_jumping(living, true));
                game.as_guest(12, |game| {
                    assert_eq!(game.living_is_jumping(living), Some(false));
                    assert!(game.living_set_is_jumping(living, true));
                    assert!(game.player.as_ref().unwrap().jumping);
                    assert_eq!(game.living_is_jumping(living), Some(true));
                    assert!(game.living_set_is_jumping(living, false));
                    assert!(!game.player.as_ref().unwrap().jumping);
                    assert_eq!(game.living_is_jumping(living), Some(false));
                })
                .unwrap();
                assert!(
                    !game
                        .command_guest(12)
                        .unwrap()
                        .motion
                        .as_ref()
                        .unwrap()
                        .jumping
                );
                assert!(!game.living_set_is_jumping(living, false));
                bed(game, "minecraft:red_bed", [5, 64, 5]);
                let at = game.sleeping_position([5, 64, 5]).unwrap();
                assert!(game.living_start_sleeping(living, [5, 64, 5]));
                let g = game.command_guest(12).unwrap();
                assert_eq!(g.position, [at.x, at.y, at.z]);
                assert_eq!(g.motion.as_ref().unwrap().velocity, Vec3::ZERO);
                assert_eq!(g.pose(), Pose::Sleeping);
                assert_eq!(g.sleep_timer, 0);
                assert_eq!(game.living_sleeping_pos(living), Some([5, 64, 5]));
                assert_eq!(
                    game.pack
                        .assets
                        .registry
                        .state(game.world.block(5, 64, 5))
                        .unwrap()
                        .properties
                        .get("occupied")
                        .map(String::as_str),
                    Some("true")
                );
                sync_guests(game);
                let sent = packets(&rx);
                assert!(has_bed_packet(&sent, Some([5, 64, 5])));
                assert!(sent.iter().any(|p| matches!(p, C::PlayerPosition(_))));
                game.as_guest(12, |game| {
                    assert_eq!(game.living_is_sleeping(living), Some(true));
                    assert!(game.living_stop_sleeping(living));
                    assert_eq!(game.living_sleeping_pos(living), None);
                    assert!(!game.player.as_ref().unwrap().sleeping);
                })
                .unwrap();
                let g = game.command_guest(12).unwrap();
                assert!(!g.sleeping && g.bed.is_none());
                // Generic LivingEntity.stopSleeping uses stopSleepInBed(true, true).
                assert_eq!(g.sleep_timer, 0);
                assert_eq!(g.pose(), Pose::Standing);
                assert_ne!(g.position, [at.x, at.y, at.z]);
                sync_guests(game);
                let sent = packets(&rx);
                assert!(has_bed_packet(&sent, None));
                assert!(
                    sent.iter()
                        .any(|p| matches!(p, C::Animate(p) if p.id == 12 && p.action == 0))
                );
                assert!(sent.iter().any(|p| matches!(p, C::PlayerPosition(_))));
                game.free_left_beds();
                assert_eq!(
                    game.pack
                        .assets
                        .registry
                        .state(game.world.block(5, 64, 5))
                        .unwrap()
                        .properties
                        .get("occupied")
                        .map(String::as_str),
                    Some("false")
                );

                let before = game.command_guest(12).unwrap().position;
                // Raw set/clear change only the sleeping data/pose, not the
                // position or occupancy. Exercise own-turn and outside writes.
                game.as_guest(12, |game| {
                    assert!(game.living_set_sleeping_pos(living, [6, 64, 6]));
                })
                .unwrap();
                assert_eq!(game.command_guest(12).unwrap().position, before);
                assert_eq!(game.command_guest(12).unwrap().pose(), Pose::Sleeping);
                sync_guests(game);
                assert!(has_bed_packet(&packets(&rx), Some([6, 64, 6])));
                assert!(game.living_clear_sleeping_pos(living));
                game.as_guest(12, |game| {
                    assert_eq!(game.living_is_sleeping(living), Some(false));
                    assert!(!game.player.as_ref().unwrap().sleeping);
                })
                .unwrap();
                assert_eq!(game.command_guest(12).unwrap().position, before);
                sync_guests(game);
                let sent = packets(&rx);
                assert!(has_bed_packet(&sent, None));
                assert!(!sent.iter().any(|p| matches!(p, C::Animate(_))));
                // The inverse entry points must have the same writeback:
                // start/clear in the own turn and stop/set from outside.
                game.as_guest(12, |game| {
                    assert!(game.living_start_sleeping(living, [5, 64, 5]));
                })
                .unwrap();
                assert!(game.living_stop_sleeping(living));
                assert!(game.living_set_sleeping_pos(living, [6, 64, 6]));
                game.as_guest(12, |game| {
                    assert!(game.living_clear_sleeping_pos(living));
                })
                .unwrap();
                game.wake_woken_guests();
                assert_eq!(game.living_is_sleeping(living), Some(false));
                assert_eq!(game.command_guest(12).unwrap().pose(), Pose::Standing);
                game.free_left_beds();
                assert_eq!(
                    game.pack
                        .assets
                        .registry
                        .state(game.world.block(5, 64, 5))
                        .unwrap()
                        .properties
                        .get("occupied")
                        .map(String::as_str),
                    Some("false")
                );
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn living_flags_foreign_level_guest_sleep_writes_refuse_without_mutation() {
        for sleeping in [false, true] {
            for acting in [false, true] {
                with_game(|game| {
                    guest_server(game, true, false);
                    let (outer, outer_rx) = guest(game, 12);
                    let (foreign, foreign_rx) = guest(game, 13);
                    bed(game, "minecraft:red_bed", [5, 64, 5]);
                    bed(game, "minecraft:blue_bed", [7, 64, 7]);
                    // Initialize a real body in its own turn before moving its
                    // hub record to another level. Its bed coordinates also
                    // name a real bed here, so a wrong-world write is visible.
                    game.as_guest(13, |game| {
                        if sleeping {
                            assert!(game.living_set_sleeping_pos(foreign, [5, 64, 5]));
                        }
                        let p = game.player.as_mut().unwrap();
                        p.velocity = Vec3::new(0.25, 0.5, -0.25);
                        p.jumping = true;
                        game.sleep_timer = 37;
                    })
                    .unwrap();
                    // This world's bed is independent of the foreign sleeper;
                    // no player-bed cleanup is pending from the fixture setup.
                    game.set_bed_occupied([5, 64, 5], sleeping);
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(13, |guest| guest.dimension = "minecraft:the_nether".into())
                        .unwrap();
                    // Entity/living lookup intentionally still includes all
                    // server players; the sleep write must own the refusal.
                    assert_eq!(game.entity(13), Some(foreign.entity()));
                    let _ = packets(&outer_rx);
                    let _ = packets(&foreign_rx);
                    let refuse = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                        let foreign_before = SleepState::guest(game, 13);
                        let outer_before = SleepState::acting(game);
                        let outer_guest_before = SleepState::guest(game, 12);
                        let foreign_bed = game.world.block(5, 64, 5);
                        let outer_bed = game.world.block(7, 64, 7);
                        let player_beds = game.server.player_beds.clone();
                        for mutation in 0..4 {
                            let changed = match mutation {
                                0 => game.living_start_sleeping(foreign, [5, 64, 5]),
                                1 => game.living_stop_sleeping(foreign),
                                2 => game.living_set_sleeping_pos(foreign, [6, 64, 6]),
                                3 => game.living_clear_sleeping_pos(foreign),
                                _ => unreachable!(),
                            };
                            assert!(
                                !changed,
                                "mutation {mutation}, sleeping {sleeping}, acting {acting}"
                            );
                            // No wake marker or bed cleanup may be queued for
                            // the current world by the refused foreign write.
                            assert_eq!(game.server.player_beds, player_beds);
                            assert_eq!(SleepState::guest(game, 13), foreign_before);
                            assert_eq!(SleepState::acting(game), outer_before);
                            assert_eq!(SleepState::guest(game, 12), outer_guest_before);
                            assert_eq!(game.world.block(5, 64, 5), foreign_bed);
                            assert_eq!(game.world.block(7, 64, 7), outer_bed);
                            assert!(packets(&foreign_rx).is_empty());
                            assert!(packets(&outer_rx).is_empty());
                        }
                    };
                    if acting {
                        game.as_guest(12, |game| {
                            assert!(game.living_start_sleeping(outer, [7, 64, 7]));
                            let _ = packets(&outer_rx);
                            let _ = packets(&foreign_rx);
                            refuse(game);
                            assert_eq!(game.server.lan_acting, Some(12));
                            assert!(!game.server.lan_acting_fake);
                        })
                        .unwrap();
                    } else {
                        refuse(game);
                    }
                    // Level cleanup runs after the actor's turn, not in it.
                    let foreign_before = SleepState::guest(game, 13);
                    let foreign_bed = game.world.block(5, 64, 5);
                    let outer_bed = game.world.block(7, 64, 7);
                    game.wake_woken_guests();
                    game.free_left_beds();
                    assert_eq!(SleepState::guest(game, 13), foreign_before);
                    assert_eq!(game.world.block(5, 64, 5), foreign_bed);
                    assert_eq!(game.world.block(7, 64, 7), outer_bed);
                    assert!(game.server.lan_guest_swaps.is_empty());
                    assert_eq!(game.dim, sunrise_gameplay_common::game::Dim::Overworld);
                });
            }
        }
    }

    /// Observable body, bed and pending wake/teleport state of a sleep write.
    #[derive(Debug, PartialEq)]
    struct SleepState {
        position: Option<Vec3>,
        velocity: Option<Vec3>,
        sleeping: bool,
        jumping: Option<bool>,
        bed: Option<[i32; 3]>,
        timer: i32,
        spawn: Option<Vec3>,
        woke_from: Option<[i32; 3]>,
        awaiting_teleport: Option<(i32, [f64; 3])>,
        teleport_counter: i32,
    }

    impl SleepState {
        fn acting(game: &sunrise_gameplay::game::ServerGame<'_>) -> Self {
            Self {
                position: game.player.as_ref().map(|p| p.position),
                velocity: game.player.as_ref().map(|p| p.velocity),
                sleeping: game.player.as_ref().is_some_and(|p| p.sleeping),
                jumping: game.player.as_ref().map(|p| p.jumping),
                bed: game.sleeping,
                timer: game.sleep_timer,
                spawn: game.spawn,
                woke_from: None,
                awaiting_teleport: None,
                teleport_counter: 0,
            }
        }

        fn guest(game: &sunrise_gameplay::game::ServerGame<'_>, conn: u32) -> Self {
            let g = game.command_guest(conn).unwrap();
            Self {
                position: Some(Vec3::new(g.position[0], g.position[1], g.position[2])),
                velocity: g.motion.as_ref().map(|p| p.velocity),
                sleeping: g.sleeping,
                jumping: g.motion.as_ref().map(|p| p.jumping),
                bed: g.bed,
                timer: g.sleep_timer,
                spawn: g.spawn.map(|p| Vec3::new(p[0], p[1], p[2])),
                woke_from: g.woke_from,
                awaiting_teleport: g.awaiting_teleport,
                teleport_counter: g.teleport_counter,
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn living_flags_guest_glide_stop_survives_own_turn_and_vines() {
        use sunrise_sim::pose::Pose;
        with_game(|game| {
            guest_server(game, false, true);
            let (living, _rx) = guest(game, 12);
            let (_observer, observer_rx) = guest(game, 13);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.fall_flying = true)
                .unwrap();
            let vine = game
                .pack
                .assets
                .registry
                .default_state("minecraft:vine")
                .unwrap();
            game.world.set(3, 64, 5, vine).unwrap();
            game.as_guest(12, |game| {
                game.player.as_mut().unwrap().fall_fly_ticks = 9;
                assert_eq!(game.living_is_climbing(living), Some(false));
            })
            .unwrap();
            assert_eq!(game.living_gliding_ticks(living), Some(9));
            assert_eq!(game.living_is_climbing(living), Some(false));
            sync_guests(game);
            let sent = packets(&observer_rx);
            assert!(sent.iter().any(|p| matches!(p,
                sunrise_protocol::packets::play::Clientbound::SetEntityData(p)
                if p.id == 12 && p.values.iter().any(|(i, _, v)| *i == 0
                    && matches!(v, sunrise_protocol::entity_data::Value::Byte(flags) if *flags & -128 != 0)))));
            game.as_guest(12, |game| {
                assert_eq!(game.living_gliding_ticks(living), Some(9));
                assert!(game.living_stop_gliding(living));
                assert_eq!(game.living_is_climbing(living), Some(true));
            })
            .unwrap();
            let g = game.command_guest(12).unwrap();
            assert!(!g.fall_flying);
            assert_ne!(g.pose(), Pose::FallFlying);
            assert_eq!(game.living_is_gliding(living), Some(false));
            assert_eq!(game.living_is_climbing(living), Some(true));
            sync_guests(game);
            let sent = packets(&observer_rx);
            assert!(sent.iter().any(|p| matches!(p,
                sunrise_protocol::packets::play::Clientbound::SetEntityData(p)
                if p.id == 12 && p.values.iter().any(|(i, _, v)| *i == 0
                    && matches!(v, sunrise_protocol::entity_data::Value::Byte(flags) if *flags & -128 == 0)))));
            game.as_guest(12, |game| {
                assert_eq!(game.living_is_gliding(living), Some(false));
                assert_eq!(game.living_gliding_ticks(living), Some(9));
            })
            .unwrap();
            game.tick_guests();
            assert_eq!(game.living_gliding_ticks(living), Some(0));
            // A mob uses the same adapter with its own, not the player's flag.
            let mob = cow(game, 20);
            game.mobs[0].position = Vec3::new(3.0, 64.0, 5.0);
            assert_eq!(game.living_is_climbing(mob), Some(true));
            game.mobs[0].extra.fall_flying = true;
            assert_eq!(game.living_is_climbing(mob), Some(false));
            let ladder = game
                .pack
                .assets
                .registry
                .default_state("minecraft:ladder")
                .unwrap();
            game.world.set(3, 64, 5, ladder).unwrap();
            assert_eq!(game.living_is_climbing(mob), Some(true));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn living_flags_nested_guest_flags_restore_outer_and_refuse_fake_sleep() {
        use sunrise_sim::{lan::guest::Guest, motion::PlayerMotion};
        with_game(|game| {
            guest_server(game, false, true);
            let (outer, _rx) = guest(game, 12);
            let (inner, _rx2) = guest(game, 13);
            bed(game, "minecraft:red_bed", [5, 64, 5]);
            let motion = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            let mut fake = Guest::connectionless(14, 14, "fake14".into(), 14, [0.0; 3], motion);
            fake.dimension = game.dim.id().to_owned();
            game.server.lan.as_mut().unwrap().hub.guests.push(fake);
            let fake = game.living(game.entity(14).unwrap()).unwrap();
            for conn in [12, 13, 14] {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |guest| guest.fall_flying = true)
                    .unwrap();
                game.as_guest(conn, |game| {
                    game.player.as_mut().unwrap().fall_fly_ticks = conn as i32
                })
                .unwrap();
            }
            assert!(!game.living_start_sleeping(fake, [5, 64, 5]));
            assert!(!game.living_stop_sleeping(fake));
            assert!(!game.living_set_sleeping_pos(fake, [5, 64, 5]));
            assert!(!game.living_clear_sleeping_pos(fake));
            game.as_guest(12, |game| {
                let outer_pos = game.player.as_ref().unwrap().position;
                assert!(game.living_set_sleeping_pos(outer, [7, 64, 7]));
                game.as_guest(14, |game| {
                    assert!(game.server.lan_acting_fake);
                    assert_eq!(game.living_gliding_ticks(outer), None);
                    assert!(!game.living_stop_gliding(outer));
                    assert!(!game.living_clear_sleeping_pos(outer));
                    assert!(!game.living_start_sleeping(fake, [5, 64, 5]));
                    assert!(!game.living_set_sleeping_pos(fake, [5, 64, 5]));
                    assert!(!game.living_clear_sleeping_pos(fake));
                    assert!(!game.living_stop_sleeping(fake));
                    game.as_guest(13, |game| {
                        assert!(!game.server.lan_acting_fake);
                        assert!(game.living_stop_gliding(inner));
                        assert!(game.living_start_sleeping(inner, [5, 64, 5]));
                    })
                    .unwrap();
                    assert!(game.server.lan_acting_fake);
                    assert_eq!(game.living_is_gliding(fake), Some(true));
                    assert_eq!(game.living_gliding_ticks(fake), Some(14));
                    assert!(game.living_stop_gliding(fake));
                })
                .unwrap();
                assert!(!game.server.lan_acting_fake);
                assert_eq!(game.living_is_gliding(outer), Some(true));
                assert_eq!(game.living_gliding_ticks(outer), Some(12));
                assert_eq!(game.living_sleeping_pos(outer), Some([7, 64, 7]));
                assert_eq!(game.player.as_ref().unwrap().position, outer_pos);
            })
            .unwrap();
            assert!(game.command_guest(12).unwrap().fall_flying);
            assert!(!game.command_guest(13).unwrap().fall_flying);
            assert!(!game.command_guest(14).unwrap().fall_flying);
            assert_eq!(game.living_sleeping_pos(inner), Some([5, 64, 5]));
            assert_eq!(game.living_is_sleeping(fake), Some(false));
            assert!(game.server.lan_guest_swaps.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn living_flags_villager_sleep_rejects_invalid_beds_without_mutation() {
        crate::game::server_side::entity_server::taken_pass_tests::in_both_dimensions(
            |game, balance| {
                guest_server(game, true, false);
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:villager")
                    .unwrap()
                    .clone();
                game.mobs
                    .push(Mob::new(20, kind, Vec3::new(3.0, 64.0, 5.0), 0.0));
                let living = game.living(game.entity(20).unwrap()).unwrap();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(4, 64, 4, stone).unwrap();
                bed(game, "minecraft:straw_bed", [6, 64, 6]); // no foot -> no sleep shape
                balance.drain_balanced();
                for pos in [[5, 64, 5], [4, 64, 4], [1600, 64, 1600], [6, 64, 6]] {
                    let before = sunrise_sim::entity_nbt::mob_to_nbt(
                        &game.mobs[0],
                        [1, 2, 3, 4],
                        None,
                        Some(game.pack.assets.registry),
                        |_| None,
                        &Default::default(),
                    );
                    assert!(!game.living_start_sleeping(living, pos), "{pos:?}");
                    assert_eq!(
                        sunrise_sim::entity_nbt::mob_to_nbt(
                            &game.mobs[0],
                            [1, 2, 3, 4],
                            None,
                            Some(game.pack.assets.registry),
                            |_| None,
                            &Default::default()
                        ),
                        before
                    );
                    assert_eq!(game.mobs[0].position, Vec3::new(3.0, 64.0, 5.0));
                    assert_eq!(game.living_sleeping_pos(living), None);
                    assert_eq!(balance.drain_balanced(), 0, "rejected bed opened a pass");
                }
                bed(game, "minecraft:red_bed", [5, 64, 5]);
                crate::game::server_side::entity_server::taken_pass_tests::assert_mob_spawn_survives(
                game,
                balance,
                |game| {
                    assert!(game.living_start_sleeping(living, [5, 64, 5]));
                },
            );
                assert_eq!(game.living_sleeping_pos(living), Some([5, 64, 5]));
                assert_eq!(game.mobs[0].position, Vec3::new(5.5, 64.6875, 5.5));
                assert_eq!(
                    game.pack
                        .assets
                        .registry
                        .state(game.world.block(5, 64, 5))
                        .unwrap()
                        .properties
                        .get("occupied")
                        .map(String::as_str),
                    Some("true")
                );
                crate::game::server_side::entity_server::taken_pass_tests::assert_mob_spawn_survives(
                game,
                balance,
                |game| {
                    assert!(game.living_stop_sleeping(living));
                },
            );
                assert_eq!(game.living_sleeping_pos(living), None);
                assert_eq!(
                    game.pack
                        .assets
                        .registry
                        .state(game.world.block(5, 64, 5))
                        .unwrap()
                        .properties
                        .get("occupied")
                        .map(String::as_str),
                    Some("false")
                );
            },
        );
    }
    #[test]
    #[ignore = "daily"]
    fn a_block_part_decides_what_a_living_entity_climbs() {
        // The fixture's parts: glass is a ladder to even entity ids, a vine to none.
        mod_platform::__private::part_tests::ladder_game_thread();
        // The client library asks the mods through the executable's callbacks.
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let (glass, vine, ladder, stone) = (
                state("minecraft:glass"),
                state("minecraft:vine"),
                state("minecraft:ladder"),
                state("minecraft:stone"),
            );
            let at = |game: &sunrise_gameplay::game::ServerGame<'_>, id: u32| {
                let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
                mob.on_climbable(&game.mob_world_base(&[]))
            };
            let even = cow(game, 20);
            let odd = cow(game, 21);
            // The cows stand in air: no tag, no part, so neither climbs.
            for mob in [even, odd] {
                assert_eq!(game.living_is_climbing(mob), Some(false));
            }
            assert!(!at(game, 20) && !at(game, 21));

            // Allow: the part makes glass climbable for the even id only, as the flag a mod reads
            // and as the mob's own climbing decide.
            game.world.set(5, 64, 5, glass).unwrap();
            assert_eq!(game.living_is_climbing(even), Some(true));
            assert_eq!(game.living_is_climbing(odd), Some(false));
            assert!(at(game, 20) && !at(game, 21));

            // Deny: the vine is tagged `climbable`, and its part says no.
            game.world.set(5, 64, 5, vine).unwrap();
            for mob in [even, odd] {
                assert_eq!(game.living_is_climbing(mob), Some(false));
            }
            assert!(!at(game, 20) && !at(game, 21));

            // Default: a block with no part keeps vanilla's answer, either way.
            game.world.set(5, 64, 5, ladder).unwrap();
            for mob in [even, odd] {
                assert_eq!(game.living_is_climbing(mob), Some(true));
            }
            assert!(at(game, 20) && at(game, 21));
            game.world.set(5, 64, 5, stone).unwrap();
            for mob in [even, odd] {
                assert_eq!(game.living_is_climbing(mob), Some(false));
            }

            // The player's own motion asks the same parts, with the player's entity id.
            game.world.set(5, 64, 5, glass).unwrap();
            assert_eq!(
                player_climbs(game, 30),
                (true, Some(BlockPos::new(5, 64, 5)))
            );
            assert_eq!(player_climbs(game, 31), (false, None));
            game.world.set(5, 64, 5, vine).unwrap();
            assert_eq!(player_climbs(game, 30), (false, None));
            game.world.set(5, 64, 5, ladder).unwrap();
            assert_eq!(
                player_climbs(game, 31),
                (true, Some(BlockPos::new(5, 64, 5)))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_part_decides_whether_a_block_is_scaffolding_to_a_sneaking_player() {
        // The fixture's parts: a ladder is scaffolding, and scaffolding is not.
        mod_platform::__private::part_tests::scaffolding_game_thread();
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let (ladder, vine, scaffolding) = (
                state("minecraft:ladder"),
                state("minecraft:vine"),
                state("minecraft:scaffolding"),
            );
            // A sneaking player moving down at -0.3 in the block at (5, 64, 5): the climb
            // limits it to -0.15, and the slide stops (0.0) unless the block is scaffolding.
            // A ladder with the part keeps sliding, as in real scaffolding.
            game.world.set(5, 64, 5, ladder).unwrap();
            assert_eq!(sneaking_slide(game), -0.15f32 as f64);
            // A vine with no part stops the slide as before.
            game.world.set(5, 64, 5, vine).unwrap();
            assert_eq!(sneaking_slide(game), 0.0);
            // Scaffolding whose part answers false stops it too.
            game.world.set(5, 64, 5, scaffolding).unwrap();
            assert_eq!(sneaking_slide(game), 0.0);
        });
    }

    /// The vertical velocity left to a sneaking player at `(5.5, 64, 5.5)` falling at -0.3
    /// after `handle_on_climbable`.
    fn sneaking_slide(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> f64 {
        let mut player = PlayerMotion::new(game.motion, Vec3::new(5.5, 64.0, 5.5)).unwrap();
        player.shift_key_down = true;
        use sunrise_gameplay_common::game::side_ops::world::WorldSideGameExt as _;
        let meta = game.side_game_side_meta();
        let side = game_side!(
            game.common,
            meta,
            game.common.level.enabled_features.as_arc(),
            meta.2.then(|| game.common.res.items())
        );
        let mut world = MotionWorld {
            side: &side,
            entities: &[],
            player: &mut player,
            scene: &mut game.common.world,
            assets: &game.common.pack.assets,
            collision_binding: game.common.collision,
            resources: game.common.motion,
            health: &game.common.health_resources,
            tag_items: Some(game.common.res.items()),
            loop_context: &game.common.loop_context,
            damage_hooks: &game.common.damage_hooks,
            block_registries: Some(&game.common.ui.registries),
        };
        sunrise_world::living_air::handle_on_climbable(&mut world, Vec3::new(0.0, -0.3, 0.0))
            .unwrap()
            .y
    }

    /// Whether the player, as entity `id` at `(5.5, 64, 5.5)`, climbs by its own motion, and the
    /// block it then remembers climbing.
    fn player_climbs(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        id: i32,
    ) -> (bool, Option<BlockPos>) {
        let mut player = PlayerMotion::new(game.motion, Vec3::new(5.5, 64.0, 5.5)).unwrap();
        player.entity_id = id;
        use sunrise_gameplay_common::game::side_ops::world::WorldSideGameExt as _;
        let meta = game.side_game_side_meta();
        let side = game_side!(
            game.common,
            meta,
            game.common.level.enabled_features.as_arc(),
            meta.2.then(|| game.common.res.items())
        );
        let mut world = MotionWorld {
            side: &side,
            entities: &[],
            player: &mut player,
            scene: &mut game.common.world,
            assets: &game.common.pack.assets,
            collision_binding: game.common.collision,
            resources: game.common.motion,
            health: &game.common.health_resources,
            tag_items: Some(game.common.res.items()),
            loop_context: &game.common.loop_context,
            damage_hooks: &game.common.damage_hooks,
            block_registries: Some(&game.common.ui.registries),
        };
        let climbing = world.climbable().unwrap();
        (climbing, player.last_climbable)
    }

    /// The spruce trapdoor's key, kept across the fixture's repeat registration.
    static SPRUCE_TRAPDOOR: std::sync::OnceLock<mod_api::BlockKey> = std::sync::OnceLock::new();

    /// Makes an open trapdoor above oak planks climb.
    struct Planks;
    #[mod_api::sunrise_part]
    impl BlockPart for Planks {
        fn makes_open_trapdoor_above_climbable(
            &self,
            _: &dyn LoopContext,
            event: &mut MakesOpenTrapdoorAboveClimbableEvent,
        ) {
            event.climbable = true;
        }
    }

    /// Refuses a spruce trapdoor above a ladder, and leaves the others to vanilla.
    struct Ladder;
    #[mod_api::sunrise_part]
    impl BlockPart for Ladder {
        fn makes_open_trapdoor_above_climbable(
            &self,
            _: &dyn LoopContext,
            event: &mut MakesOpenTrapdoorAboveClimbableEvent,
        ) {
            if SPRUCE_TRAPDOOR.get() == Some(&event.trapdoor.block()) {
                event.climbable = false;
            }
        }
    }

    struct TrapdoorLadderMod;
    #[mod_api::__subscriptions]
    impl Hooks for TrapdoorLadderMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            record_key(&SPRUCE_TRAPDOOR, r.block_key("minecraft:spruce_trapdoor"));
            let planks = r.block_key("minecraft:oak_planks");
            r.block_part(planks, Planks);
            let ladder = r.block_key("minecraft:ladder");
            r.block_part(ladder, Ladder);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_part_decides_whether_an_open_trapdoor_above_it_climbs() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "trapdoor_ladder",
            Box::new(TrapdoorLadderMod),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        // The client library asks the mods through the executable's callbacks.
        crate::entry::install_client_hooks();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let registry = game.pack.assets.registry;
                let state = |name: &str, properties: &[(&str, &str)]| {
                    registry
                        .resolve(&sunrise_world::chunk::BlockState {
                            name: name.into(),
                            properties: properties
                                .iter()
                                .map(|(key, value)| ((*key).into(), (*value).into()))
                                .collect(),
                        })
                        .unwrap()
                };
                let open = |name: &str| state(name, &[("facing", "north"), ("open", "true")]);
                let ladder = |facing: &str| state("minecraft:ladder", &[("facing", facing)]);
                let living = cow(game, 40);
                for (trapdoor, below, climbs) in [
                    // Vanilla: a ladder facing the trapdoor's way, and the part leaves it.
                    (open("minecraft:oak_trapdoor"), ladder("north"), true),
                    (open("minecraft:oak_trapdoor"), ladder("south"), false),
                    // Allow: planks are no ladder, and their part says yes.
                    (
                        open("minecraft:oak_trapdoor"),
                        state("minecraft:oak_planks", &[]),
                        true,
                    ),
                    // Deny: the ladder's part refuses the spruce trapdoor.
                    (open("minecraft:spruce_trapdoor"), ladder("north"), false),
                ] {
                    game.world.set(5, 63, 5, below).unwrap();
                    game.world.set(5, 64, 5, trapdoor).unwrap();
                    // The flag a mod reads, the cow's own climbing and the player's motion agree.
                    assert_eq!(game.living_is_climbing(living), Some(climbs));
                    let mob = game.mobs.iter().find(|m| m.id == 40).unwrap();
                    assert_eq!(mob.on_climbable(&game.mob_world_base(&[])), climbs);
                    let remembered = climbs.then_some(BlockPos::new(5, 64, 5));
                    assert_eq!(player_climbs(game, 41), (climbs, remembered));
                }
            })
        });
    }
}
