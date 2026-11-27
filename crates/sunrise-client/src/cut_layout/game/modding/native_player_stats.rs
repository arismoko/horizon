#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::{
        native_entity_refs::tests::{fixture_root, run_fresh_cases, with_game},
        native_guest_turn_tests::with_guests,
    };
    use core::ops::ControlFlow;
    use mod_api::{
        AdvancementError, AdvancementProgress, EntityReadError, Flow, Hooks, Identifier,
        PlayerStatAwardEvent, ServerHost, StatType,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;

    struct ObserveAwards(Arc<AtomicUsize>);

    #[mod_api::__subscriptions]
    impl Hooks for ObserveAwards {
        fn player_stat_award(
            &self,
            _: &mut dyn ServerHost,
            _: &mut PlayerStatAwardEvent<'_>,
        ) -> Flow {
            self.0.fetch_add(1, Ordering::Relaxed);
            Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn statistics_restore_updates_live_scores_and_next_request_without_awarding() {
        use serde_json::{Value, json};
        use sunrise_game_data::scoreboard::{RenderType, literal};
        use sunrise_protocol::{
            buf::Reader,
            ids::play::clientbound::AWARD_STATS,
            packets::play::{ClientCommand, Clientbound, Serverbound},
        };
        use sunrise_sim::lan::{Out, PlayerSender, playerdata::uuid_ints};

        if run_fresh_cases(
            "cut_layout::game::server_side::native_player_stats::tests::statistics_restore_updates_live_scores_and_next_request_without_awarding",
            "SUNRISE_STATISTICS_SNAPSHOT_CHILD",
            &["restore"],
        ) {
            return;
        }
        sunrise_world::datafix::set_resources(&fixture_root());
        let awards = Arc::new(AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "observe_awards",
            Box::new(ObserveAwards(awards.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let world = tempfile::tempdir().unwrap();
                game.server.world_root = world.path().to_path_buf();
                game.load_scoreboard();
                assert!(mod_api::__private::listened(
                    mod_api::HookId::PlayerStatAward
                ));

                let version = sunrise_world::DATA_VERSION;
                let empty = json!({"DataVersion": version, "stats": {}});
                let player = game.player(12).unwrap();
                let other = game.player(13).unwrap();
                let stale = mod_api::__private::game::player_ref(player.entity(), 999);
                assert!(stale.statistics(game).is_none());
                assert!(!stale.set_statistics(game, &empty.to_string()));
                let before = player.statistics(game).unwrap();
                assert_eq!(serde_json::from_str::<Value>(&before).unwrap(), empty);

                // No DataVersion: vanilla's StatsCounterFix upgrades this flat counter.
                assert!(player.set_statistics(game, r#"{"stat.jump":6}"#));
                assert_eq!(player.stat(game, StatType::Custom, "jump"), Some(6));
                assert_eq!(serde_json::from_str::<Value>(&before).unwrap(), empty);

                let host = game.server.lan.as_mut().unwrap();
                let stats = &mut host.play.get_mut(&12).unwrap().stats;
                stats.add("mined", "minecraft:stone", 6);
                stats.custom("jump", 3);
                stats.custom("deaths", 2);
                stats.events.clear();
                stats.take_dirty();
                host.play.entry(13).or_default().stats.custom("jump", 17);
                let other_before = other.statistics(game).unwrap();
                let board = &mut game.score.board;
                for (name, criteria, score) in [
                    ("jump", "minecraft.custom:minecraft.jump", 50),
                    ("mined", "minecraft.mined:minecraft.stone", 40),
                    ("deaths", "minecraft.custom:minecraft.deaths", 11),
                    ("dummy", "dummy", 80),
                ] {
                    assert!(board.add_objective(
                        name,
                        criteria,
                        literal(name),
                        RenderType::Integer,
                        false,
                        None
                    ));
                    board.set("guest12", name, score);
                }
                board.set("guest13", "jump", 17);

                let before = player.statistics(game).unwrap();
                assert!(!player.set_statistics(game, r#"{"stats":{}"#));
                assert_eq!(player.statistics(game).unwrap(), before);
                let restored = json!({
                    "DataVersion": version,
                    "stats": {"minecraft:custom": {"minecraft:jump": -7, "minecraft:deaths": 2}}
                });
                assert!(player.set_statistics(game, &restored.to_string()));
                assert_eq!(player.stat(game, StatType::Mined, "stone"), Some(0));
                assert_eq!(other.statistics(game).unwrap(), other_before);
                let board = &game.score.board;
                for (holder, name, value) in [
                    ("guest12", "jump", -7),
                    ("guest12", "mined", 0),
                    ("guest12", "deaths", 11),
                    ("guest12", "dummy", 80),
                    ("guest13", "jump", 17),
                ] {
                    assert_eq!(
                        board.score(holder, name).unwrap().value,
                        value,
                        "{holder}/{name}"
                    );
                }

                let (tx, rx) = std::sync::mpsc::channel();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.sender = PlayerSender::new(tx, Arc::default());
                    })
                    .unwrap();
                game.lan_guest_packet(12, Serverbound::ClientCommand(ClientCommand { action: 1 }));
                let host = game.server.lan.as_ref().unwrap();
                let hub = &host.hub;
                let id = |registry, key| hub.registry_id(registry, key).unwrap();
                let mut responses = Vec::new();
                for out in rx.try_iter() {
                    let bodies = match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(_) => panic!("statistics request disconnected the guest"),
                    };
                    for body in bodies {
                        let mut r = Reader::new(&body);
                        let id = r.varint().unwrap();
                        if id == AWARD_STATS {
                            let Clientbound::AwardStats(response) =
                                Clientbound::decode(id, r.rest()).unwrap()
                            else {
                                panic!("not an AwardStats response");
                            };
                            responses.push(response.stats);
                        }
                    }
                }
                assert_eq!(
                    responses,
                    [vec![
                        (
                            id("minecraft:stat_type", "minecraft:custom"),
                            id("minecraft:custom_stat", "minecraft:jump"),
                            -7
                        ),
                        (
                            id("minecraft:stat_type", "minecraft:mined"),
                            id("minecraft:block", "minecraft:stone"),
                            0
                        ),
                    ]]
                );
                game.save_connected_guest_statistics(host);
                let path = sunrise_gameplay::game::server_side::stats::player_json(
                    world.path(),
                    "stats",
                    uuid_ints(12),
                );
                let saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                assert_eq!(saved, restored);

                // In a transaction the parked guest's restore waits for the commit: an abort
                // leaves the counter and its score as they were, a commit moves both.
                let jump9 = json!({
                    "DataVersion": version,
                    "stats": {"minecraft:custom": {"minecraft:jump": 9}}
                })
                .to_string();
                use mod_api::HostTransaction as _;
                for commit in [false, true] {
                    let server: &mut dyn ServerHost = game;
                    let result = server.transaction(|server, _| {
                        assert!(player.set_statistics(server, &jump9));
                        assert_eq!(player.stat(server, StatType::Custom, "jump"), Some(-7));
                        if commit { Ok(()) } else { Err(()) }
                    });
                    assert_eq!(result.is_ok(), commit);
                    let expected = if commit { 9 } else { -7 };
                    assert_eq!(player.stat(game, StatType::Custom, "jump"), Some(expected));
                    assert_eq!(
                        game.score.board.score("guest12", "jump").unwrap().value,
                        expected
                    );
                }

                assert!(player.set_statistics(game, &json!({"DataVersion": version}).to_string()));
                let cleared = player.statistics(game).unwrap();
                assert_eq!(serde_json::from_str::<Value>(&cleared).unwrap(), empty);
                assert_eq!(awards.load(Ordering::Relaxed), 0);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_resting_guest_reads_from_their_own_record() {
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

        with_game(|game| {
            let settings = sunrise_gameplay::game::server_side::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                announce: false,
                ..NetConfig::lan(0, "StatsTest".into(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 10,
                simulation_distance: 10,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, _rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 1,
                    uuid: 123,
                    name: "guest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                20,
                [0.0; 3],
                8,
            );
            guest.placed = true;
            guest.recipe_book.recipes.insert("minecraft:stick".into());
            let host = game.server.lan.as_mut().unwrap();
            host.hub.guests.push(guest);
            let mut play =
                sunrise_gameplay::game::server_side::lan_actions_server::GuestPlay::default();
            play.stats.add("custom", "minecraft:jump", 7);
            host.play.insert(1, play);
            game.stats.add("custom", "minecraft:jump", 3);

            let player = game.player(123).unwrap();
            let jump = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.player_stat(player, StatType::Custom, "jump")
            };
            assert_eq!(jump(game), Some(7));
            assert_eq!(
                game.player_has_discovered_recipe(player, "stick"),
                Some(true)
            );
            assert_eq!(
                game.player_has_discovered_recipe(player, "minecraft:diamond_pickaxe"),
                Some(false)
            );
            let mut recipes = Vec::new();
            assert_eq!(
                player.visit_discovered_recipes(game, &mut |id| {
                    recipes.push(id.to_owned());
                    ControlFlow::Continue(())
                }),
                Ok(ControlFlow::Continue(()))
            );
            assert_eq!(recipes, ["minecraft:stick"]);
            // In the guest's turn their state is in the game, not merged with the resting book.
            game.level.recipe_book.recipes.clear();
            game.server.lan_acting_uuid = Some(123);
            game.server.lan_acting_entity = Some(20);
            assert_eq!(jump(game), Some(3));
            assert_eq!(
                player.visit_discovered_recipes(game, &mut |_| panic!("empty acting book")),
                Ok(ControlFlow::Continue(()))
            );
            let book = &mut game.level.recipe_book.recipes;
            book.insert("minecraft:torch".into());
            book.insert("minecraft:oak_planks".into());
            let mut first = Vec::new();
            assert_eq!(
                player.visit_discovered_recipes(game, &mut |id| {
                    first.push(id.to_owned());
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, ["minecraft:oak_planks"]);
            game.server.lan_acting_uuid = None;
            game.server.lan_acting_entity = None;
            // A guest without play state yet has counted nothing.
            game.server.lan.as_mut().unwrap().play.remove(&1);
            assert_eq!(jump(game), Some(0));
            game.server.lan.as_mut().unwrap().hub.guests.remove(0);
            assert_eq!(
                player.visit_discovered_recipes(game, &mut |_| panic!("stale player")),
                Err(EntityReadError::Stale)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_advancement_progress_reads_live_state_and_refuses_unavailable() {
        use mod_api::{
            CriterionContext, CriterionTest, CriterionTestError, EntityRef,
            PlayerAdvancementDoneEvent, PlayerAdvancementProgressEvent, ProgressType, WorldEpoch,
        };
        use std::sync::Mutex;
        use sunrise_sim::advancements::{Advancement, Criterion, Tree};

        struct Accept;
        impl CriterionTest for Accept {
            fn test(&mut self, _: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
                Ok(true)
            }
        }

        struct ObserveProgress {
            id: Identifier,
            on_progress: Arc<Mutex<Vec<(String, ProgressType, Option<AdvancementProgress>)>>>,
            on_done: Arc<Mutex<Vec<Option<AdvancementProgress>>>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for ObserveProgress {
            fn player_advancement_progress(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerAdvancementProgressEvent<'_>,
            ) {
                if event.advancement != self.id.as_str() {
                    return;
                }
                let live = host
                    .player(event.player)
                    .and_then(|player| player.advancement_progress(host, &self.id));
                assert_eq!(live.as_ref(), Some(event.progress));
                self.on_progress.lock().unwrap().push((
                    event.criterion.to_owned(),
                    event.progress_type,
                    live,
                ));
            }

            fn player_advancement_done(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerAdvancementDoneEvent<'_>,
            ) {
                if event.advancement != self.id.as_str() {
                    return;
                }
                let live = host
                    .player(event.player)
                    .and_then(|player| player.advancement_progress(host, &self.id));
                self.on_done.lock().unwrap().push(live);
            }
        }

        let adv_id = Identifier::parse("testmod:two_step").unwrap();
        let unknown_id = Identifier::parse("testmod:missing").unwrap();
        let step_one = Identifier::parse("testmod:step_one").unwrap();
        let step_two = Identifier::parse("testmod:step_two").unwrap();
        let on_progress = Arc::new(Mutex::new(Vec::new()));
        let on_done = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "observe_advancement_progress",
            Box::new(ObserveProgress {
                id: adv_id.clone(),
                on_progress: on_progress.clone(),
                on_done: on_done.clone(),
            }),
        )]
        .into();

        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                game.build_registries();
                game.ui.registries.permission = 4;
                let mut tree = Tree::default();
                tree.list.push(Advancement {
                    id: adv_id.as_str().into(),
                    parent: None,
                    display: None,
                    criteria: vec![
                        Criterion {
                            name: "first".into(),
                            trigger: step_one.as_str().into(),
                            conditions: serde_json::Value::Null.into(),
                            player: None,
                        },
                        Criterion {
                            name: "second".into(),
                            trigger: step_two.as_str().into(),
                            conditions: serde_json::Value::Null.into(),
                            player: None,
                        },
                    ],
                    requirements: vec![vec!["first".into()], vec!["second".into()]],
                    rewards: Default::default(),
                });
                tree.by_id.insert(adv_id.as_str().into(), 0);
                tree.by_trigger
                    .insert(step_one.as_str().into(), vec![(0, 0)]);
                tree.by_trigger
                    .insert(step_two.as_str().into(), vec![(0, 1)]);
                game.adv.tree = Arc::new(tree);
                game.adv.loaded = true;
                game.ui.registries.advancements = vec![adv_id.as_str().into()];
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|guest| {
                        guest.advancements_loaded = true;
                    });

                let player = game.player(12).unwrap();
                let other = game.player(13).unwrap();

                // Untouched loaded advancement: all criteria remaining, not done.
                let untouched = player.advancement_progress(game, &adv_id).unwrap();
                assert!(!untouched.done);
                assert_eq!(untouched.completed_requirements, 0);
                assert_eq!(untouched.requirements, vec![vec!["first"], vec!["second"]]);
                assert_eq!(
                    untouched
                        .criteria
                        .iter()
                        .map(|c| (c.name.as_str(), c.obtained_at.is_some()))
                        .collect::<Vec<_>>(),
                    vec![("first", false), ("second", false)]
                );
                assert_eq!(player.advancement_progress(game, &unknown_id), None);

                // First criterion granted: 1 completed, 1 remaining, not done.
                assert_eq!(
                    player.trigger_criterion(game, &step_one, &mut Accept),
                    Ok(())
                );
                let after_one = player.advancement_progress(game, &adv_id).unwrap();
                assert!(!after_one.done);
                assert_eq!(after_one.completed_requirements, 1);
                assert_eq!(
                    after_one
                        .criteria
                        .iter()
                        .map(|c| (c.name.as_str(), c.obtained_at.is_some()))
                        .collect::<Vec<_>>(),
                    vec![("first", true), ("second", false)]
                );
                assert_eq!(
                    other.advancement_progress(game, &adv_id).as_ref(),
                    Some(&untouched)
                );
                assert!(on_done.lock().unwrap().is_empty());

                // Second criterion granted: done, and both hooks read the post-grant live state.
                assert_eq!(
                    player.trigger_criterion(game, &step_two, &mut Accept),
                    Ok(())
                );
                let after_two = player.advancement_progress(game, &adv_id).unwrap();
                assert!(after_two.done);
                assert_eq!(after_two.completed_requirements, 2);
                assert_eq!(
                    after_two
                        .criteria
                        .iter()
                        .map(|c| (c.name.as_str(), c.obtained_at.is_some()))
                        .collect::<Vec<_>>(),
                    vec![("first", true), ("second", true)]
                );
                assert_eq!(*on_done.lock().unwrap(), vec![Some(after_two.clone())]);

                // Displaced player during a nested turn still reads its own live progress.
                game.as_guest(12, |game| {
                    assert_eq!(
                        player.advancement_progress(game, &adv_id).as_ref(),
                        Some(&after_two)
                    );
                    game.as_guest(13, |game| {
                        assert_eq!(
                            player.advancement_progress(game, &adv_id).as_ref(),
                            Some(&after_two)
                        );
                        assert_eq!(
                            other.advancement_progress(game, &adv_id).as_ref(),
                            Some(&untouched)
                        );
                    })
                    .unwrap();
                })
                .unwrap();

                // `/advancement revoke`: done is false again and the criterion returns to remaining.
                assert!(
                    game.native_run_command(
                        "advancement revoke guest12 only testmod:two_step second"
                    )
                );
                let after_revoke = player.advancement_progress(game, &adv_id).unwrap();
                assert!(!after_revoke.done);
                assert_eq!(after_revoke.completed_requirements, 1);
                assert_eq!(
                    after_revoke
                        .criteria
                        .iter()
                        .map(|c| (c.name.as_str(), c.obtained_at.is_some()))
                        .collect::<Vec<_>>(),
                    vec![("first", true), ("second", false)]
                );
                assert_eq!(
                    after_revoke.criteria[0].obtained_at,
                    after_one.criteria[0].obtained_at
                );
                assert_eq!(
                    *on_progress.lock().unwrap(),
                    vec![
                        ("first".to_owned(), ProgressType::Grant, Some(after_one)),
                        ("second".to_owned(), ProgressType::Grant, Some(after_two)),
                        (
                            "second".to_owned(),
                            ProgressType::Revoke,
                            Some(after_revoke)
                        ),
                    ]
                );

                // A guest whose advancements the engine has not loaded yet answers None,
                // and the other guest is unaffected.
                let hub = &mut game.server.lan.as_mut().unwrap().hub;
                hub.guests
                    .update_connection(12, |g| g.advancements_loaded = false)
                    .unwrap();
                assert_eq!(player.advancement_progress(game, &adv_id), None);
                assert!(other.advancement_progress(game, &adv_id).is_some());
                let hub = &mut game.server.lan.as_mut().unwrap().hub;
                hub.guests
                    .update_connection(12, |g| g.advancements_loaded = true)
                    .unwrap();
                assert!(player.advancement_progress(game, &adv_id).is_some());

                // Stale handles, client role, and offline players answer None.
                let stale_epoch = mod_api::__private::game::player_ref(
                    EntityRef::new(
                        WorldEpoch::new(player.entity().epoch().get() + 1),
                        player.entity().id(),
                    ),
                    player.uuid(),
                );
                let wrong_uuid = mod_api::__private::game::player_ref(player.entity(), 999);
                let wrong_entity =
                    mod_api::__private::game::player_ref(other.entity(), player.uuid());
                for stale in [stale_epoch, wrong_uuid, wrong_entity] {
                    assert_eq!(stale.advancement_progress(game, &adv_id), None);
                }

                let saved_role = game.options.execution_role;
                game.options.execution_role =
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
                assert_eq!(player.advancement_progress(game, &adv_id), None);
                game.options.execution_role = saved_role;

                game.server.lan.as_mut().unwrap().hub.guests.clear();
                assert_eq!(player.advancement_progress(game, &adv_id), None);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_award_and_revoke_advancement_route_through_engine_path() {
        use mod_api::{PlayerAdvancementDoneEvent, PlayerAdvancementProgressEvent, ProgressType};
        use std::sync::Mutex;
        use sunrise_sim::advancements::{Advancement, Criterion, Rewards, Tree};

        struct ObserveChanges {
            id: Identifier,
            on_progress: Arc<Mutex<Vec<(String, ProgressType)>>>,
            on_done: Arc<Mutex<Vec<String>>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for ObserveChanges {
            fn player_advancement_progress(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerAdvancementProgressEvent<'_>,
            ) {
                if event.advancement == self.id.as_str() {
                    self.on_progress
                        .lock()
                        .unwrap()
                        .push((event.criterion.to_owned(), event.progress_type));
                }
            }

            fn player_advancement_done(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerAdvancementDoneEvent<'_>,
            ) {
                if event.advancement == self.id.as_str() {
                    self.on_done
                        .lock()
                        .unwrap()
                        .push(event.advancement.to_owned());
                }
            }
        }

        let adv_id = Identifier::parse("testmod:two_step").unwrap();
        let unknown_id = Identifier::parse("testmod:missing").unwrap();
        let on_progress = Arc::new(Mutex::new(Vec::new()));
        let on_done = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "observe_advancement_changes",
            Box::new(ObserveChanges {
                id: adv_id.clone(),
                on_progress: on_progress.clone(),
                on_done: on_done.clone(),
            }),
        )]
        .into();

        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                game.build_registries();
                let mut tree = Tree::default();
                tree.list.push(Advancement {
                    id: adv_id.as_str().into(),
                    parent: None,
                    display: None,
                    criteria: vec![
                        Criterion {
                            name: "first".into(),
                            trigger: "minecraft:impossible".into(),
                            conditions: serde_json::Value::Null.into(),
                            player: None,
                        },
                        Criterion {
                            name: "second".into(),
                            trigger: "minecraft:impossible".into(),
                            conditions: serde_json::Value::Null.into(),
                            player: None,
                        },
                    ],
                    requirements: vec![vec!["first".into()], vec!["second".into()]],
                    rewards: Rewards {
                        experience: 25,
                        ..Default::default()
                    },
                });
                tree.by_id.insert(adv_id.as_str().into(), 0);
                game.adv.tree = Arc::new(tree);
                game.adv.loaded = true;
                game.ui.registries.advancements = vec![adv_id.as_str().into()];
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_all(|guest| {
                        guest.advancements_loaded = true;
                    });

                let player = game.player(12).unwrap();
                let other = game.player(13).unwrap();

                // Unknown advancement and unknown criterion report which was missing.
                assert_eq!(
                    player.award_advancement(game, &unknown_id, None),
                    Err(AdvancementError::UnknownAdvancement)
                );
                assert_eq!(
                    player.revoke_advancement(game, &unknown_id, None),
                    Err(AdvancementError::UnknownAdvancement)
                );
                assert_eq!(
                    player.award_advancement(game, &adv_id, Some("missing")),
                    Err(AdvancementError::UnknownCriterion)
                );
                assert_eq!(
                    player.revoke_advancement(game, &adv_id, Some("missing")),
                    Err(AdvancementError::UnknownCriterion)
                );

                // Award one criterion: true, progress shows it, not done yet.
                assert_eq!(
                    player.award_advancement(game, &adv_id, Some("first")),
                    Ok(true)
                );
                let after_first = player.advancement_progress(game, &adv_id).unwrap();
                assert!(!after_first.done);
                assert_eq!(after_first.completed_requirements, 1);
                assert_eq!(
                    after_first
                        .criteria
                        .iter()
                        .map(|c| (c.name.as_str(), c.obtained_at.is_some()))
                        .collect::<Vec<_>>(),
                    vec![("first", true), ("second", false)]
                );
                assert!(on_done.lock().unwrap().is_empty());
                assert_eq!(player.experience(game).unwrap().total, 0);

                // Award the same criterion again: false, no change.
                assert_eq!(
                    player.award_advancement(game, &adv_id, Some("first")),
                    Ok(false)
                );

                // Award all remaining criteria: done, reward experience given, done hook fires once.
                assert_eq!(player.award_advancement(game, &adv_id, None), Ok(true));
                let after_all = player.advancement_progress(game, &adv_id).unwrap();
                assert!(after_all.done);
                assert_eq!(after_all.completed_requirements, 2);
                assert_eq!(player.experience(game).unwrap().total, 25);
                assert_eq!(*on_done.lock().unwrap(), vec![adv_id.as_str().to_owned()]);
                assert!(!other.advancement_progress(game, &adv_id).unwrap().done);

                // Awarding an already-done advancement with None returns false and does not re-reward.
                assert_eq!(player.award_advancement(game, &adv_id, None), Ok(false));
                assert_eq!(player.experience(game).unwrap().total, 25);
                assert_eq!(on_done.lock().unwrap().len(), 1);

                // Revoke one criterion: true, no longer done.
                assert_eq!(
                    player.revoke_advancement(game, &adv_id, Some("second")),
                    Ok(true)
                );
                let after_revoke_one = player.advancement_progress(game, &adv_id).unwrap();
                assert!(!after_revoke_one.done);
                assert_eq!(after_revoke_one.completed_requirements, 1);
                assert_eq!(
                    after_revoke_one
                        .criteria
                        .iter()
                        .map(|c| (c.name.as_str(), c.obtained_at.is_some()))
                        .collect::<Vec<_>>(),
                    vec![("first", true), ("second", false)]
                );

                // Revoking an unobtained criterion returns false.
                assert_eq!(
                    player.revoke_advancement(game, &adv_id, Some("second")),
                    Ok(false)
                );

                // Revoke all remaining obtained criteria, then revoking again returns false.
                assert_eq!(player.revoke_advancement(game, &adv_id, None), Ok(true));
                let after_revoke_all = player.advancement_progress(game, &adv_id).unwrap();
                assert!(!after_revoke_all.done);
                assert_eq!(after_revoke_all.completed_requirements, 0);
                assert_eq!(player.revoke_advancement(game, &adv_id, None), Ok(false));

                assert_eq!(
                    *on_progress.lock().unwrap(),
                    vec![
                        ("first".to_owned(), ProgressType::Grant),
                        ("second".to_owned(), ProgressType::Grant),
                        ("second".to_owned(), ProgressType::Revoke),
                        ("first".to_owned(), ProgressType::Revoke),
                    ]
                );

                // Inside a transaction, award and revoke validate immediately and defer
                // their progress change, rewards and hooks until the outermost commit.
                use mod_api::HostTransaction as _;
                on_progress.lock().unwrap().clear();
                on_done.lock().unwrap().clear();
                let xp_before = player.experience(game).unwrap().total;

                let server: &mut dyn ServerHost = game;
                let aborted = server.transaction(|server, _| {
                    assert_eq!(
                        player.award_advancement(server, &unknown_id, None),
                        Err(AdvancementError::UnknownAdvancement)
                    );
                    assert_eq!(
                        player.award_advancement(server, &adv_id, Some("missing")),
                        Err(AdvancementError::UnknownCriterion)
                    );
                    assert_eq!(player.award_advancement(server, &adv_id, None), Ok(true));
                    assert_eq!(player.revoke_advancement(server, &adv_id, None), Ok(false));
                    let pending = player.advancement_progress(server, &adv_id).unwrap();
                    assert!(!pending.done);
                    assert_eq!(pending.completed_requirements, 0);
                    Err::<(), ()>(())
                });
                assert!(aborted.is_err());
                let after_abort = player.advancement_progress(game, &adv_id).unwrap();
                assert!(!after_abort.done);
                assert_eq!(after_abort.completed_requirements, 0);
                assert_eq!(player.experience(game).unwrap().total, xp_before);
                assert!(on_progress.lock().unwrap().is_empty());
                assert!(on_done.lock().unwrap().is_empty());

                let server: &mut dyn ServerHost = game;
                let committed = server.transaction(|server, _| {
                    assert_eq!(player.award_advancement(server, &adv_id, None), Ok(true));
                    assert!(!player.advancement_progress(server, &adv_id).unwrap().done);
                    Ok::<(), ()>(())
                });
                assert!(committed.is_ok());
                let after_commit = player.advancement_progress(game, &adv_id).unwrap();
                assert!(after_commit.done);
                assert_eq!(after_commit.completed_requirements, 2);
                assert_eq!(player.experience(game).unwrap().total, xp_before + 25);
                assert_eq!(
                    *on_progress.lock().unwrap(),
                    vec![
                        ("first".to_owned(), ProgressType::Grant),
                        ("second".to_owned(), ProgressType::Grant),
                    ]
                );
                assert_eq!(*on_done.lock().unwrap(), vec![adv_id.as_str().to_owned()]);

                on_progress.lock().unwrap().clear();
                let server: &mut dyn ServerHost = game;
                let aborted_revoke = server.transaction(|server, _| {
                    assert_eq!(player.revoke_advancement(server, &adv_id, None), Ok(true));
                    assert!(player.advancement_progress(server, &adv_id).unwrap().done);
                    Err::<(), ()>(())
                });
                assert!(aborted_revoke.is_err());
                assert!(player.advancement_progress(game, &adv_id).unwrap().done);
                assert!(on_progress.lock().unwrap().is_empty());

                let server: &mut dyn ServerHost = game;
                let committed_revoke = server.transaction(|server, _| {
                    assert_eq!(
                        player.revoke_advancement(server, &adv_id, Some("second")),
                        Ok(true)
                    );
                    assert!(player.advancement_progress(server, &adv_id).unwrap().done);
                    Ok::<(), ()>(())
                });
                assert!(committed_revoke.is_ok());
                let after_commit_revoke = player.advancement_progress(game, &adv_id).unwrap();
                assert!(!after_commit_revoke.done);
                assert_eq!(after_commit_revoke.completed_requirements, 1);
                assert_eq!(
                    *on_progress.lock().unwrap(),
                    vec![("second".to_owned(), ProgressType::Revoke)]
                );

                // Displaced player during a nested turn is unavailable for writes.
                game.as_guest(12, |game| {
                    game.as_guest(13, |game| {
                        assert_eq!(
                            player.award_advancement(game, &adv_id, None),
                            Err(AdvancementError::PlayerUnavailable)
                        );
                        assert_eq!(
                            player.revoke_advancement(game, &adv_id, None),
                            Err(AdvancementError::PlayerUnavailable)
                        );
                    })
                    .unwrap();
                })
                .unwrap();

                // Stale handles, client role, and offline players are refused.
                let wrong_uuid = mod_api::__private::game::player_ref(player.entity(), 999);
                assert_eq!(
                    wrong_uuid.award_advancement(game, &adv_id, None),
                    Err(AdvancementError::PlayerUnavailable)
                );
                assert_eq!(
                    wrong_uuid.revoke_advancement(game, &adv_id, None),
                    Err(AdvancementError::PlayerUnavailable)
                );

                let saved_role = game.options.execution_role;
                game.options.execution_role =
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
                assert_eq!(
                    player.award_advancement(game, &adv_id, None),
                    Err(AdvancementError::NotServer)
                );
                assert_eq!(
                    player.revoke_advancement(game, &adv_id, None),
                    Err(AdvancementError::NotServer)
                );
                game.options.execution_role = saved_role;

                game.server.lan.as_mut().unwrap().hub.guests.clear();
                assert_eq!(
                    player.award_advancement(game, &adv_id, None),
                    Err(AdvancementError::PlayerUnavailable)
                );
                assert_eq!(
                    player.revoke_advancement(game, &adv_id, None),
                    Err(AdvancementError::PlayerUnavailable)
                );
            });
        });
    }
}
