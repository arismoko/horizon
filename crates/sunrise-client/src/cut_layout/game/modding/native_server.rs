#![cfg(test)]
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use sunrise_gameplay_common::game::native_server::tests::Watcher;
#[cfg(test)]
#[path = "native_server/command_failure_tests.rs"]
pub(crate) mod command_failure_tests;
#[cfg(test)]
#[path = "native_server/command_shape_tests.rs"]
pub(crate) mod command_shape_tests;
#[cfg(test)]
#[path = "native_server/selector_tests.rs"]
pub(crate) mod selector_tests;
#[cfg(test)]
#[path = "native_server/sidebar_tests.rs"]
pub(crate) mod sidebar_tests;

#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;

    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;

    use sunrise_gameplay_common::game::native_server::tests::player_guest;

    use super::*;

    /// Box bounds come from mod code: one that is not finite reads no orb and never
    /// panics, and an inverted box reads what the ordered one does.
    #[test]
    #[ignore = "daily"]
    fn experience_orbs_refuse_bounds_that_are_not_finite_and_swap_an_inverted_box() {
        use mod_api::{ExperienceOrbWriteError, LevelReadError, LevelRef};
        use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.options.dedicated = Some(Vec::new());
        let at = sunrise_core::geometry::Vec3::new(1.0, 70.0, 1.0);
        let mk_orb = |id| sunrise_sim::entities::ExperienceOrb {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            id,
            position: at,
            previous: at,
            velocity: sunrise_core::geometry::Vec3::ZERO,
            value: 3,
            age: 0,
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
            count: 1,
            fire_ticks: 0,
            ticks: 0,
            health: 5,
            air: 300,
            no_gravity: true,
            following: None,
        };
        game.orbs.push(mk_orb(40));
        game.orbs.push(mk_orb(41));
        let overworld = game.level(game.dim.id()).expect("active overworld handle");
        let visit_ids =
            |game: &sunrise_gameplay::game::ServerGame<'_>, level: LevelRef, min, max| {
                let mut ids = Vec::new();
                let flow = level.visit_experience_orbs(game, min, max, &mut |orb| {
                    ids.push(orb.entity().id());
                    core::ops::ControlFlow::Continue(())
                })?;
                Ok::<_, LevelReadError>((flow, ids))
            };
        assert_eq!(
            visit_ids(&game, overworld, [0.0, 69.0, 0.0], [2.0, 71.0, 2.0]),
            Ok((core::ops::ControlFlow::Continue(()), vec![40, 41]))
        );
        // Early stop on ControlFlow::Break(()).
        let mut stopped = Vec::new();
        assert_eq!(
            overworld.visit_experience_orbs(
                &game,
                [0.0, 69.0, 0.0],
                [2.0, 71.0, 2.0],
                &mut |orb| {
                    stopped.push(orb.entity().id());
                    core::ops::ControlFlow::Break(())
                }
            ),
            Ok(core::ops::ControlFlow::Break(()))
        );
        assert_eq!(stopped, [40]);

        for (min, max) in [
            ([f64::NAN, 69.0, 0.0], [2.0, 71.0, 2.0]),
            ([0.0, 69.0, 0.0], [2.0, f64::NAN, 2.0]),
            ([f64::NEG_INFINITY, 69.0, 0.0], [2.0, 71.0, 2.0]),
            ([0.0, 69.0, 0.0], [2.0, 71.0, f64::INFINITY]),
            ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]),
        ] {
            let mut called = false;
            assert_eq!(
                overworld.visit_experience_orbs(&game, min, max, &mut |_| {
                    called = true;
                    core::ops::ControlFlow::Continue(())
                }),
                Err(LevelReadError::Unavailable),
                "{min:?} {max:?}"
            );
            assert!(!called, "refused visit must not invoke callback");
        }
        assert_eq!(
            visit_ids(&game, overworld, [2.0, 69.0, 0.0], [0.0, 71.0, 2.0]),
            Ok((core::ops::ControlFlow::Continue(()), vec![40, 41]))
        );
        assert_eq!(
            visit_ids(&game, overworld, [2.0, 71.0, 2.0], [0.0, 69.0, 0.0]),
            Ok((core::ops::ControlFlow::Continue(()), vec![40, 41]))
        );

        // Park the overworld by switching to the Nether: the overworld handle still visits,
        // narrows, reads and mutates its parked orbs.
        game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
            .unwrap();
        let mut parked_orbs = Vec::new();
        assert_eq!(
            overworld.visit_experience_orbs(
                &game,
                [0.0, 69.0, 0.0],
                [2.0, 71.0, 2.0],
                &mut |orb| {
                    parked_orbs.push(orb);
                    core::ops::ControlFlow::Continue(())
                }
            ),
            Ok(core::ops::ControlFlow::Continue(()))
        );
        assert_eq!(
            parked_orbs
                .iter()
                .map(|orb| orb.entity().id())
                .collect::<Vec<_>>(),
            vec![40, 41]
        );
        let orb = parked_orbs[0];
        assert_eq!(game.experience_orb(orb.entity()), Ok(Some(orb)));
        assert_eq!(orb.entity().position(&game), Some([1.0, 70.0, 1.0]));
        assert_eq!(orb.value(&game), Ok(3));
        assert_eq!(orb.count(&game), Ok(1));
        assert_eq!(orb.age(&game), Ok(0));
        assert_eq!(orb.set_value(&mut game, 50_000), Ok(()));
        assert_eq!(orb.set_count(&mut game, 4), Ok(()));
        assert_eq!(orb.set_age(&mut game, -12), Ok(()));
        assert_eq!(orb.value(&game), Ok(50_000));
        assert_eq!(orb.count(&game), Ok(4));
        assert_eq!(orb.age(&game), Ok(-12));

        // Client side refuses with Unavailable.
        let role = game.options.execution_role;
        game.options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        assert_eq!(
            overworld.visit_experience_orbs(&game, [0.0, 69.0, 0.0], [2.0, 71.0, 2.0], &mut |_| {
                core::ops::ControlFlow::Continue(())
            }),
            Err(LevelReadError::Unavailable)
        );
        assert_eq!(
            game.experience_orb(orb.entity()),
            Err(LevelReadError::Unavailable)
        );
        assert_eq!(orb.value(&game), Err(LevelReadError::Unavailable));
        assert_eq!(
            orb.set_value(&mut game, 1),
            Err(ExperienceOrbWriteError::Unavailable)
        );
        game.options.execution_role = role;
    }
    /// A mod's `announce` reaches the dedicated server's event stream and nothing else's.
    /// The integrated server thread keeps the launch configuration (`options.dedicated`) too,
    /// so it cannot be what decides.
    #[test]
    #[ignore = "daily"]
    fn server_events_are_posted_only_on_the_dedicated_role() {
        use sunrise_gameplay_common::game::execution_role::ExecutionRole;
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.options.dedicated = Some(Vec::new());
        for (role, posted) in [
            (ExecutionRole::DedicatedServer, true),
            (ExecutionRole::IntegratedServer, false),
            (ExecutionRole::Client, false),
        ] {
            game.options.execution_role = role;
            assert_eq!(
                game.native_server_event("Alex", 7, "hello"),
                posted,
                "{role:?}"
            );
        }
    }
    pub(crate) fn with_command_sources(
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        use sunrise_core::geometry::Vec3;
        use sunrise_level::{
            chunk::Chunk, chunk::block_entity::BlockEntity, command_block::CommandBlock,
        };
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            game.build_registries();
            game.register_native_commands();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:command_block")
                .unwrap();
            game.world.set(1, 64, 1, state).unwrap();
            game.world.block_entities.insert(
                (1, 64, 1),
                BlockEntity::CommandBlock(Box::new(CommandBlock {
                    command: "execute in minecraft:the_nether run origin".into(),
                    condition_met: true,
                    ..Default::default()
                })),
            );
            let mut cart = sunrise_gameplay_common::game::minecarts::Minecart::new(
                99,
                sunrise_gameplay_common::game::minecarts::CartKind::CommandBlock,
                Vec3::new(1.0, 64.0, 1.0),
            );
            cart.command.command = "origin".into();
            game.carts.push(cart);
            test(game);
        });
    }
    /// `ChunkWatchEvent`: the hub's records reach the mods in order, each
    /// with the player (entity id and UUID, even after they left) and the
    /// chunk in its level; a level the server doesn't hold is skipped.
    #[test]
    #[ignore = "daily"]
    fn the_hubs_chunk_watches_reach_the_mods_in_order() {
        use sunrise_sim::lan::chunk_watch::{
            ChunkWatch, ChunkWatchKind, ChunkWatchLog, LevelWatches,
        };
        let heard = std::sync::Arc::default();
        let mods = vec![mod_api::ModEntry::new(
            "watch",
            Box::new(Watcher(std::sync::Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan(game);
                let here = game.dim.id();
                let watch = |kind, chunk| ChunkWatch {
                    kind,
                    uuid: 127,
                    entity_id: 42,
                    chunk,
                };
                let run = |dimension: &str, watches| LevelWatches {
                    dimension: dimension.into(),
                    watches,
                };
                game.server.lan.as_mut().unwrap().hub.chunk_watches = ChunkWatchLog::new(vec![
                    run(
                        here,
                        vec![
                            watch(ChunkWatchKind::Watch, [1, 2]),
                            watch(ChunkWatchKind::Sent, [1, 2]),
                        ],
                    ),
                    run(
                        "minecraft:nowhere",
                        vec![watch(ChunkWatchKind::Unwatch, [0, 0])],
                    ),
                    run(here, vec![watch(ChunkWatchKind::Unwatch, [1, 2])]),
                ]);
                game.run_chunk_watches();
                assert!(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .chunk_watches
                        .is_empty()
                );
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        ("watch", 42, 127, here, [1, 2]),
                        ("sent", 42, 127, here, [1, 2]),
                        ("unwatch", 42, 127, here, [1, 2]),
                    ]
                );
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn panicking_command_executors_keep_other_usages_and_only_undo_journaled_writes() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_server::tests::panicking_command_executors_keep_other_usages_and_only_undo_journaled_writes",
            || {
                use std::sync::atomic::AtomicUsize;
                const JOURNALED: [i32; 3] = [2, 70, 2];
                const UNJOURNALED: [i32; 3] = [3, 70, 2];
                const ABANDONED: [i32; 3] = [4, 70, 2];
                const REFUSED: [i32; 3] = [5, 70, 2];
                struct Executors(Arc<AtomicUsize>);
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Executors {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        for (usage, pos, opens_root) in [
                            ("journalboom", JOURNALED, false),
                            ("plainboom", UNJOURNALED, false),
                            ("rootboom", ABANDONED, true),
                            ("ordinaryfailure", REFUSED, true),
                        ] {
                            let refuses = self.0.clone();
                            event.commands.push(mod_api::Command::new(usage).executes(
                                move |host, _, _| {
                                    let mutate = |host: &mut dyn mod_api::ServerHost| {
                                        let level = host.level("minecraft:overworld").unwrap();
                                        let state = host.block_state_id("minecraft:stone").unwrap();
                                        if level.set_block(
                                            host,
                                            pos,
                                            state,
                                            mod_api::BlockUpdateFlags::NONE,
                                        ) != Ok(true)
                                        {
                                            return Err(
                                                mod_api::Text::literal("block not placed").into()
                                            );
                                        }
                                        if usage == "ordinaryfailure" {
                                            refuses.fetch_add(1, Ordering::Relaxed);
                                            return Err(
                                                mod_api::Text::literal("ordinary failure").into()
                                            );
                                        }
                                        panic!("executor regression panic");
                                    };
                                    if opens_root {
                                        let res = mod_api::HostTransaction::transaction(
                                            host,
                                            |host, _| {
                                                mod_api::HostTransaction::transaction(
                                                    host,
                                                    |host, _| mutate(host),
                                                )
                                                .map_err(|e| match e {
                                                    mod_api::TransactionError::Aborted(msg) => msg,
                                                    _ => mod_api::Text::literal("tx failed").into(),
                                                })
                                            },
                                        );
                                        match res {
                                            Ok(()) => Ok(0),
                                            Err(mod_api::TransactionError::Aborted(msg)) => {
                                                Err(msg)
                                            }
                                            Err(_) => {
                                                Err(mod_api::Text::literal("tx failed").into())
                                            }
                                        }
                                    } else {
                                        mutate(host).map(|()| 0)
                                    }
                                },
                            ));
                        }
                        event
                            .commands
                            .push(mod_api::Command::new("worksagain").executes(|_, _, _| Ok(7)));
                    }
                }
                let refuses = Arc::new(AtomicUsize::new(0));
                mod_platform::testing::with_mods(
                    vec![mod_api::ModEntry::new(
                        "executor_owner",
                        Box::new(Executors(refuses.clone())),
                    )]
                    .into(),
                    || {
                        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                            game.options.dedicated = Some(Vec::new());
                            game.world
                                .chunks
                                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                            let air = game.world.block(JOURNALED[0], JOURNALED[1], JOURNALED[2]);
                            let stone = game
                                .pack
                                .assets
                                .registry
                                .default_state("minecraft:stone")
                                .unwrap();
                            game.build_registries();
                            game.build_native_commands(&[]);

                            game.storage_transaction(|game, outer| {
                                let (failure, outcome) = game.recording_command_outcome(|game| {
                                    game.execute_line("journalboom", game.server_source())
                                });
                                assert_eq!(failure.unwrap_err().key, "command.failed");
                                assert_eq!(outcome, None);
                                assert_eq!(
                                    game.world.block(JOURNALED[0], JOURNALED[1], JOURNALED[2]),
                                    air
                                );
                                assert!(mod_api::ServerHost::transaction_matches(
                                    game,
                                    Some(outer)
                                ));
                                Ok::<_, ()>(())
                            })
                            .unwrap();
                            let failure = game
                                .execute_line("plainboom", game.server_source())
                                .unwrap_err();
                            assert!(failure.unexpected);
                            assert_eq!(
                                game.world
                                    .block(UNJOURNALED[0], UNJOURNALED[1], UNJOURNALED[2]),
                                stone
                            );

                            let failure = game
                                .execute_line("rootboom", game.server_source())
                                .unwrap_err();
                            assert!(failure.unexpected);
                            assert!(!game.has_open_transaction());
                            assert_eq!(
                                game.world.block(ABANDONED[0], ABANDONED[1], ABANDONED[2]),
                                air
                            );

                            game.build_native_commands(&[]);
                            for usage in ["journalboom", "rootboom"] {
                                assert_eq!(
                                    game.execute_line(usage, game.server_source())
                                        .unwrap_err()
                                        .key,
                                    "command.unknown.command"
                                );
                            }
                            assert_eq!(
                                game.execute_line("worksagain", game.server_source()),
                                Ok(7)
                            );
                            for _ in 0..2 {
                                assert_eq!(
                                    game.execute_line("ordinaryfailure", game.server_source())
                                        .unwrap_err()
                                        .message
                                        .plain(),
                                    "ordinary failure"
                                );
                                assert!(!game.has_open_transaction());
                                assert_eq!(
                                    game.world.block(REFUSED[0], REFUSED[1], REFUSED[2]),
                                    air
                                );
                            }
                            assert_eq!(refuses.load(Ordering::Relaxed), 2);
                            assert_eq!(
                                mod_api::__private::take_hook_panics(),
                                [mod_api::__private::HookPanicReport {
                                    mod_id: "executor_owner",
                                    hook: "command_executor",
                                }; 3]
                            );
                        })
                    },
                );
            },
        );
    }
    #[test]
    #[ignore = "daily"]
    fn native_unauthorized_commands_never_reach_the_listener() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_server::tests::native_unauthorized_commands_never_reach_the_listener",
            || {
                struct Commands(Arc<Mutex<Vec<mod_api::CommandInvocation>>>);
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Commands {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        event.commands.extend(
                            [
                                mod_api::Command::op("foo [value:integer]", 4),
                                mod_api::Command::new("foo bar"),
                                mod_api::Command::op("choice <a:integer>", 4),
                                mod_api::Command::new("choice <a:integer> bar"),
                                mod_api::Command::new("choice <b:word>"),
                            ]
                            .map(|command| {
                                let heard = self.0.clone();
                                command.executes(move |_, _, invocation| {
                                    heard.lock().unwrap().push(invocation.clone());
                                    Ok(1)
                                })
                            }),
                        );
                    }
                }
                let heard = Arc::new(Mutex::new(Vec::new()));
                let mods = vec![mod_api::ModEntry::new(
                    "commands",
                    Box::new(Commands(heard.clone())),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan(
                            game,
                        );
                        game.register_native_commands();
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .push(player_guest());
                        for line in ["foo", "foo 7", "foo nope"] {
                            assert_eq!(game.perform_native_command(1, 0, line, false), Some(false));
                        }
                        assert!(
                            heard.lock().unwrap().is_empty(),
                            "denied usages must not call a mod"
                        );
                        assert_eq!(
                            game.perform_native_command(1, 0, "foo bar", false),
                            Some(true)
                        );
                        assert_eq!(game.perform_native_command(1, 4, "foo", false), Some(true));
                        assert_eq!(
                            game.perform_native_command(1, 4, "foo 7", false),
                            Some(true)
                        );
                        for line in ["choice 7", "choice 7 bar", "choice text"] {
                            assert_eq!(game.perform_native_command(1, 0, line, false), Some(true));
                        }
                        let heard = heard.lock().unwrap();
                        assert_eq!(heard.len(), 6);
                        assert_eq!(heard[0].literals, ["foo", "bar"]);
                        assert_eq!(heard[1].literals, ["foo"]);
                        assert!(heard[1].arguments.is_empty());
                        assert_eq!(heard[2].literals, ["foo"]);
                        assert_eq!(
                            heard[2].argument("value"),
                            Some(&mod_api::CommandArgumentValue::Integer(7))
                        );
                        assert_eq!(heard[3].literals, ["choice"]);
                        assert!(heard[3].argument("a").is_none());
                        assert_eq!(
                            heard[3].argument("b"),
                            Some(&mod_api::CommandArgumentValue::String("7".into()))
                        );
                        assert_eq!(heard[4].literals, ["choice", "bar"]);
                        assert_eq!(
                            heard[4].argument("a"),
                            Some(&mod_api::CommandArgumentValue::Integer(7))
                        );
                        assert_eq!(heard[5].literals, ["choice"]);
                        assert_eq!(
                            heard[5].argument("b"),
                            Some(&mod_api::CommandArgumentValue::String("text".into()))
                        );
                    });
                });
            },
        );
    }
}

#[cfg(test)]
pub(crate) mod fake_action_tests {

    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6;

    use sunrise_gameplay_common::game::native_server::fake_action_tests::on_top;

    use sunrise_gameplay::game::native_server::fake_action_tests::fake;

    use sunrise_gameplay::game::native_server::fake_action_tests::held;

    use sunrise_gameplay::game::native_server::fake_action_tests::place;

    use sunrise_gameplay::game::native_server::fake_action_tests::trader;

    use sunrise_gameplay::game::native_server::fake_action_tests::zombie;

    use mod_api::PlayerAction;
    use mod_api::{ActionOutcome, ActionRefused, FakeEffect};

    use mod_api::Hand;

    use sunrise_items::items::ItemStack;

    use sunrise_gameplay_common::game::{
        boats_common::Boat, minecarts::CART_KEY_Y, minecarts::CartKind, minecarts::Minecart,
    };

    /// A fake player's use on a trader frees it again: the session the use
    /// opened closes through the merchant's own close path, so the trader is
    /// not left trading and the next use opens it as the first did.
    #[test]
    #[ignore = "daily"]
    fn a_trader_a_fake_player_used_trades_with_the_next_one() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:stick"), 0);
            let id = 6000;
            let target = trader(game, id, [2.5, 64.0, 0.5]);
            let interact = PlayerAction::InteractOn {
                hand: Hand::Main,
                target,
                location: [0.0, 0.5, 0.0],
            };
            assert_eq!(
                game.player_perform_action(player, interact),
                Ok(ActionOutcome::Consumed)
            );
            let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
            // With no offers the menu never opens, and this test would say nothing.
            let offers = mob.extra.trade.as_ref().map_or(0, |t| t.offers.len());
            assert!(offers > 0, "a wandering trader has offers");
            assert!(!mob.extra.flag2, "the trader is not left trading");
            assert!(game.station.is_none(), "the session is not left open");
            assert_eq!(
                game.player_perform_action(player, interact),
                Ok(ActionOutcome::Consumed),
                "the next use opens it again",
            );
        });
    }
    /// A chest minecart, a hopper minecart and a chest boat: the items stay
    /// on the vehicle, and the menu's stand-in is out of the level, where a
    /// transfer would edit it instead of the vehicle.
    #[test]
    #[ignore = "daily"]
    fn a_vehicles_chest_a_fake_player_used_keeps_its_items_and_no_stand_in() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let (player, fake_conn, _) = fake(game, None, 0);
            let at = sunrise_core::geometry::Vec3::new(2.5, 64.0, 0.5);
            let diamonds = Some(ItemStack::new("minecraft:diamond", 5));
            let mut vehicles = Vec::new();
            for (id, kind) in [(6100, CartKind::Chest), (6101, CartKind::Hopper)] {
                let mut cart = Minecart::new(id, kind, at);
                cart.items[3] = diamonds.clone();
                vehicles.push((id, false));
                game.carts.push(cart);
            }
            let mut boat = Boat::new(6102, "oak".into(), at, 0.0).with_chest();
            boat.items[3] = diamonds.clone();
            // ChestBoat.interact opens the chest for a sneaking player.
            vehicles.push((6102, true));
            game.boats.push(boat);

            for (id, sneaking) in vehicles {
                let target = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id);
                let interact = PlayerAction::InteractOn {
                    hand: Hand::Main,
                    target,
                    location: [0.0, 0.5, 0.0],
                };
                // Twice: the second use is the next one finding it as it was.
                for _ in 0..2 {
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(fake_conn, |guest| guest.sneaking = sneaking)
                        .unwrap();
                    assert_eq!(
                        game.player_perform_action(player, interact),
                        Ok(ActionOutcome::Consumed)
                    );
                    let key = (i32::MAX - 15, CART_KEY_Y, id as i32);
                    assert!(
                        game.world.block_entities.get(&key).is_none(),
                        "vehicle {id} left a stand-in",
                    );
                    assert!(game.container.is_none());
                    let items = game
                        .carts
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| &c.items)
                        .or_else(|| game.boats.iter().find(|b| b.id == id).map(|b| &b.items))
                        .unwrap();
                    assert_eq!(items[3], diamonds, "vehicle {id} lost its items");
                }
            }
        });
    }
    /// A real guest keeps its vehicle menu over a cart a fake player then
    /// uses: the refused open closes only a stand-in nobody is looking at,
    /// and the staging does not reset what the viewer's menu is editing.
    #[test]
    #[ignore = "daily"]
    fn a_viewer_keeps_the_vehicle_menu_a_fake_player_used() {
        use sunrise_level::containers::BlockEntity;
        use sunrise_sim::lan::menu::Slots;
        use {
            sunrise_gameplay_common::game::minecarts::CART_KEY_Y,
            sunrise_gameplay_common::game::minecarts::CartKind,
            sunrise_gameplay_common::game::minecarts::Minecart,
        };
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let (player, fake_conn, real_conn) = fake(game, None, 0);
            let at = sunrise_core::geometry::Vec3::new(2.5, 64.0, 0.5);
            let mut cart = Minecart::new(6110, CartKind::Chest, at);
            cart.items[3] = Some(ItemStack::new("minecraft:diamond", 5));
            game.carts.push(cart);
            let gi = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .position(|g| g.conn == real_conn)
                .unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(real_conn, |guest| guest.position = [0.5, 64.0, 0.5])
                .unwrap();
            assert_eq!(
                game.guest_interact(real_conn, gi, 6110, Hand::Main, [2.5, 64.5, 0.5]),
                Ok(true),
                "the guest's use opens the vehicle's menu",
            );
            let key = (i32::MAX - 15, CART_KEY_Y, 6110);
            let viewer = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(real_conn)
                    .unwrap()
                    .menu
                    .as_ref()
                    .is_some_and(|m| m.slots == Slots::Block(key))
            };
            assert!(viewer(game), "the guest's use opened the vehicle's menu");
            assert!(game.world.block_entities.contains_key(&key));
            // The viewer's live edit: its menu's clicks write through to the
            // stand-in (`source.write` after every click).
            let emerald = Some(ItemStack::new("minecraft:emerald", 2));
            if let Some(sunrise_level::containers::BlockEntity::Chest(items)) =
                game.world.block_entities.get_mut(&key)
            {
                items[4] = emerald.clone();
            }
            let target = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 6110);
            assert_eq!(
                game.player_perform_action(
                    player,
                    PlayerAction::InteractOn {
                        hand: Hand::Main,
                        target,
                        location: [0.0, 0.5, 0.0],
                    },
                ),
                Ok(ActionOutcome::Consumed)
            );
            assert!(viewer(game), "the fake's use kept the viewer's menu");
            assert!(
                game.world.block_entities.contains_key(&key),
                "the fake's use kept the stand-in the menu backs",
            );
            let BlockEntity::Chest(items) = &game.world.block_entities[&key] else {
                unreachable!()
            };
            assert_eq!(items[3], Some(ItemStack::new("minecraft:diamond", 5)));
            assert_eq!(items[4], emerald, "the staging kept the viewer's edit");
            game.guest_close_all(real_conn);
            assert!(!game.world.block_entities.contains_key(&key));
            let items = &game.carts.iter().find(|c| c.id == 6110).unwrap().items;
            assert_eq!(
                items[3],
                Some(ItemStack::new("minecraft:diamond", 5)),
                "the viewer's close put the items back",
            );
            assert_eq!(items[4], emerald, "the viewer's edit reached the vehicle");
            assert!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(fake_conn)
                    .unwrap()
                    .menu
                    .is_none()
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn scoped_fake_use_returns_items_and_restores_persistent_state() {
        use mod_api::PlayerActionSetup;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, Some("minecraft:diamond"), 1);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |guest| {
                    guest.yaw = 31.0;
                    guest.pitch = 12.0;
                    guest.selected = 3;
                    guest.inventory[3] = Some(ItemStack::new("minecraft:emerald", 4));
                    guest.offhand = Some(ItemStack::new("minecraft:apple", 2));
                    guest.sneaking = true;
                    guest.previous_game_mode = Some(3);
                    guest.motion.as_mut().unwrap().shift_key_down = true;
                })
                .unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .play
                .get_mut(&conn)
                .unwrap()
                .cooldowns
                .add("minecraft:bucket".into(), 100);
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(sunrise_sim::mobs::Mob::new(
                5000,
                kind,
                sunrise_core::geometry::Vec3::new(2.0, 64.0, 0.5),
                0.0,
            ));
            let target = game.entity(5000).unwrap();
            let setup = PlayerActionSetup::new(
                Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                    &ItemStack::new("minecraft:bucket", 2),
                )),
                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                170.0,
                -15.0,
                false,
            );
            let action = PlayerAction::InteractOn {
                hand: Hand::Main,
                target,
                location: [0.0, 0.5, 0.0],
            };
            let result = player.perform_action_scoped(game, action, setup).unwrap();
            assert_eq!(result.outcome, ActionOutcome::Consumed);
            assert_eq!(
                result.held,
                Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                    &ItemStack::new("minecraft:bucket", 1)
                ))
            );
            assert_eq!(
                result.other,
                vec![sunrise_gameplay_common::game::api_values::test_snapshot(
                    &ItemStack::new("minecraft:milk_bucket", 1)
                )]
            );
            let guest = game.server.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert_eq!(guest.position, [0.5, 64.0, 0.5]);
            assert_eq!(
                (guest.yaw, guest.pitch, guest.selected, guest.game_mode),
                (31.0, 12.0, 3, 1)
            );
            assert_eq!(guest.previous_game_mode, Some(3));
            assert_eq!(guest.offhand, Some(ItemStack::new("minecraft:apple", 2)));
            assert_eq!(
                guest.inventory[3],
                Some(ItemStack::new("minecraft:emerald", 4))
            );
            assert!(guest.sneaking && guest.motion.as_ref().unwrap().shift_key_down);
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(game.fake_player(level, 2, "fake"), Some(player));
            // The original cooldown is still in force, not the temporary action's timer.
            assert!(
                game.server.lan.as_ref().unwrap().play[&conn]
                    .cooldowns
                    .is_on_cooldown("minecraft:bucket")
            );
            assert_eq!(game.server.lan.as_ref().unwrap().hub.guests.len(), 2);
        });
    }
    /// An armor stand holding a saved item this build doesn't have hands it to a scoped action:
    /// the stack can't cross, so it goes back into the world where the fake stood, unchanged,
    /// with the default pickup delay, and the result counts it instead of panicking.
    #[test]
    #[ignore = "daily"]
    fn scoped_fake_use_returns_an_unrepresentable_stack_to_the_world() {
        use mod_api::PlayerActionSetup;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let (player, _, _) = fake(game, None, 0);
            let mut stand = sunrise_sim::entities::ArmorStand::new(
                5001,
                sunrise_core::geometry::Vec3::new(2.0, 64.0, 0.5),
                0.0,
            );
            stand.equipment[0] = Some(ItemStack::new("test:missing", 1));
            game.armor_stands.push(stand);
            let target = game.entity(5001).unwrap();
            let setup = PlayerActionSetup::new(
                None,
                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                90.0,
                0.0,
                false,
            );
            let action = PlayerAction::InteractOn {
                hand: Hand::Main,
                target,
                location: [0.0, 1.5, 0.0],
            };
            let dropped = game.dropped.len();
            let result = player.perform_action_scoped(game, action, setup).unwrap();
            assert_eq!(result.outcome, ActionOutcome::Consumed);
            assert_eq!((result.held, result.other.len()), (None, 0));
            assert_eq!(result.unrepresentable, 1);
            assert_eq!(game.armor_stands.last().unwrap().equipment[0], None);
            assert_eq!(game.dropped.len(), dropped + 1);
            let item = game.dropped.last().unwrap();
            assert_eq!(item.stack, ItemStack::new("test:missing", 1));
            assert_eq!(item.pickup_delay, 10);
            assert_eq!((item.position.x, item.position.z), (0.5, 0.5));
        });
    }
    /// A join listener that keeps item entities out discards the returned stack as it would any
    /// drop: no item entity is left, and the result does not count it.
    #[test]
    #[ignore = "daily"]
    fn a_vetoed_return_is_not_counted() {
        use mod_api::{EntityJoinLevelEvent, Flow, Hooks, PlayerActionSetup, ServerHost};
        struct KeepItemsOut;
        #[mod_api::__subscriptions]
        impl Hooks for KeepItemsOut {
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                if event.kind == "minecraft:item" {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "keep-items-out",
            Box::new(KeepItemsOut),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let (player, _, _) = fake(game, None, 0);
                let mut stand = sunrise_sim::entities::ArmorStand::new(
                    5001,
                    sunrise_core::geometry::Vec3::new(2.0, 64.0, 0.5),
                    0.0,
                );
                stand.equipment[0] = Some(ItemStack::new("test:missing", 1));
                game.armor_stands.push(stand);
                let target = game.entity(5001).unwrap();
                let setup = PlayerActionSetup::new(
                    None,
                    [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                    90.0,
                    0.0,
                    false,
                );
                let action = PlayerAction::InteractOn {
                    hand: Hand::Main,
                    target,
                    location: [0.0, 1.5, 0.0],
                };
                let dropped = game.dropped.len();
                let result = player.perform_action_scoped(game, action, setup).unwrap();
                assert_eq!(result.outcome, ActionOutcome::Consumed);
                assert_eq!((result.held, result.other.len()), (None, 0));
                assert_eq!(result.unrepresentable, 0, "the vetoed stack is not counted");
                assert_eq!(game.armor_stands.last().unwrap().equipment[0], None);
                assert_eq!(game.dropped.len(), dropped, "no item entity is left");
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn scoped_fake_use_restores_the_persistent_menu_owner() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, Some("minecraft:diamond"), 1);
            let mut menu = Box::new(sunrise_sim::menu::Menu::new(
                sunrise_sim::menu::Kind::Inventory,
            ));
            menu.carried = Some(ItemStack::new("minecraft:stick", 3));
            let owner = (&*menu) as *const sunrise_sim::menu::Menu;
            *game
                .server
                .lan
                .as_mut()
                .unwrap()
                .menus
                .entry(conn)
                .or_default()
                .crafting
                .inventory = Some(menu);
            place(game, [1, 64, 0], "minecraft:chest");
            let setup = mod_api::PlayerActionSetup::new(
                Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                    &ItemStack::new("minecraft:oak_log", 1),
                )),
                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                0.0,
                90.0,
                false,
            );
            let result = game
                .player_perform_action_scoped(player, on_top([1, 64, 0]), setup)
                .unwrap();
            assert_eq!(
                result.outcome,
                ActionOutcome::EffectBlocked(FakeEffect::Menu)
            );
            let restored = game.server.lan.as_ref().unwrap().menus[&conn]
                .crafting
                .inventory
                .as_ref()
                .unwrap();
            assert_eq!((&**restored) as *const sunrise_sim::menu::Menu, owner);
            assert_eq!(restored.carried, Some(ItemStack::new("minecraft:stick", 3)));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn fake_entity_interaction_checks_targets_and_does_not_use_items_in_air() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:ender_pearl"), 0);
            let near = zombie(game, 5000, [2.0, 64.0, 0.5]);
            let far = zombie(game, 5001, [20.0, 64.0, 0.5]);
            let interact = |target| PlayerAction::InteractOn {
                hand: Hand::Main,
                target,
                location: [0.0; 3],
            };
            assert_eq!(
                game.player_perform_action(player, interact(near)),
                Ok(ActionOutcome::Passed)
            );
            assert_eq!(
                held(game, 2),
                Some(ItemStack::new("minecraft:ender_pearl", 1))
            );
            assert!(game.creatures.projectiles.is_empty());
            assert_eq!(
                game.player_perform_action(player, interact(far)),
                Err(ActionRefused::OutOfReach)
            );
            assert_eq!(
                game.player_perform_action(player, interact(player.entity())),
                Err(ActionRefused::InvalidTarget)
            );
            let stale = mod_api::EntityRef::new(mod_api::WorldEpoch::new(0), near.id());
            assert_eq!(
                game.player_perform_action(player, interact(stale)),
                Err(ActionRefused::InvalidTarget)
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn mob_target_setters_refuse_persistent_and_acting_fakes() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, None, 0);
            let entity = zombie(game, 5000, [2.0, 64.0, 0.5]);
            let mob = game.mob(entity).unwrap();
            let target = game.living(player.entity()).unwrap();
            assert!(!mob.set_target(game, Some(target)));
            assert_eq!(mob.target(game), None);
            assert_eq!(
                game.as_guest(conn, |g| mob.set_target(g, Some(target))),
                Some(false)
            );
            assert_eq!(mob.target(game), None);
            assert!(
                !game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .mob_players
                    .contains_key(&5000)
            );
        });
    }
    /// `as_guest_at` starts a turn only for the guest it names: another `conn` at that place,
    /// a place past the list's end, and a guest already in a turn are refused, not a panic.
    #[test]
    #[ignore = "daily"]
    fn a_turn_at_a_place_is_refused_for_another_guest() {
        use sunrise_gameplay_common::game::lan_actions::TurnRefused;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let (_, fake_conn, real_conn) = fake(game, None, 0);
            let gi = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .position(|g| g.conn == fake_conn)
                .unwrap();
            assert_eq!(
                game.as_guest_at(gi, real_conn, |_| ()),
                Err(TurnRefused::Gone)
            );
            assert_eq!(
                game.as_guest_at(999, fake_conn, |_| ()),
                Err(TurnRefused::Gone)
            );
            let nested = game.as_guest_at(gi, fake_conn, |g| g.as_guest_at(gi, fake_conn, |_| ()));
            assert_eq!(nested, Ok(Err(TurnRefused::Nested)));
            assert_eq!(game.as_guest_at(gi, fake_conn, |_| 7), Ok(7));
        });
    }
    /// 1, 64 and 256 fakes each clicking the stone floor with an empty hand every tick, the full
    /// admission and use path (vanilla passes). Run with `-- --ignored --nocapture`, in release.
    #[test]
    #[ignore = "timing"]
    fn fakes_acting_each_tick_cost() {
        #[derive(Clone, Copy, Debug)]
        enum Kind {
            UseOn,
            Attack,
            BreakBlock,
        }
        for kind in [Kind::UseOn, Kind::Attack, Kind::BreakBlock] {
            for count in [1u128, 64, 256] {
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    let (first, ..) = fake(game, None, 0);
                    let level = game.level(game.dim.id()).unwrap();
                    let mut players = vec![first];
                    for i in 1..count {
                        players.push(
                            game.fake_player(level, 100 + i, &format!("fake{i}"))
                                .unwrap(),
                        );
                    }
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update_all(|g| {
                            g.position = [0.5, 64.0, 0.5];
                            g.yaw = -90.0;
                        });
                    let target = zombie(game, 5000, [2.0, 64.0, 0.5]);
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    let (action, expected) = match kind {
                        Kind::UseOn => (on_top([1, 63, 0]), ActionOutcome::Passed),
                        Kind::Attack => (PlayerAction::Attack { target }, ActionOutcome::Consumed),
                        Kind::BreakBlock => (
                            PlayerAction::BreakBlock { pos: [1, 63, 0] },
                            ActionOutcome::Consumed,
                        ),
                    };
                    let ticks = 200;
                    let start = std::time::Instant::now();
                    for _ in 0..ticks {
                        // The zombie never dies, and each break has a stone to break.
                        game.mobs[0].health = 20.0;
                        for &player in &players {
                            if let Kind::BreakBlock = kind {
                                game.world.set(1, 63, 0, stone).unwrap();
                            }
                            let outcome = game.player_perform_action(player, action);
                            assert_eq!(outcome, Ok(expected));
                        }
                    }
                    let elapsed = start.elapsed();
                    let per_tick = elapsed / ticks;
                    let per_action = elapsed / (ticks * players.len() as u32);
                    eprintln!(
                        "fake_action kind={kind:?} players={count} ns_per_action={}",
                        per_action.as_nanos()
                    );
                    println!(
                        "{kind:?}, {count} fakes: {per_tick:?} a tick, {per_action:?} an action"
                    );
                });
            }
        }
    }
    #[test]
    #[ignore = "daily"]
    fn a_real_player_is_unaffected_at_every_effect_site() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let (_, _, real_conn) = fake(game, None, 0);
            for effect in [
                FakeEffect::Menu,
                FakeEffect::Sleep,
                FakeEffect::Ride,
                FakeEffect::ChangeDimension,
            ] {
                assert_eq!(
                    game.as_guest(real_conn, |g| g.fake_blocks(effect)),
                    Some(false)
                );
            }
            // A real guest's click on a chest is the vanilla click, nothing stopped.
            place(game, [1, 64, 0], "minecraft:chest");
            let gi = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .position(|g| g.conn == real_conn)
                .unwrap();
            let hit = sunrise_gameplay_common::game::Hit {
                pos: [1, 64, 0],
                face: [0, 1, 0],
                point: [1.5, 65.0, 0.5],
                inside: false,
            };
            assert_eq!(
                game.guest_use_item_on(real_conn, gi, hit, Hand::Main, None),
                Ok(true)
            );
            assert_eq!(
                game.as_guest(real_conn, |g| g.native_may_board(99)),
                Some(true)
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod player_rule_tests {
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
    use {
        crate::game::native_server::*,
        sunrise_gameplay_common::game::native_server::{
            GlobalPos, Identifier, PlayerRespawnConfig,
        },
    };

    /// The guests of `with_guests` are rows 0 and 1; both stand in the level in place.
    fn in_this_level(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        let level = game.dim.id().to_owned();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|g| {
                g.dimension = level.clone();
            });
    }

    fn at(dimension: &str, [x, y, z]: [i32; 3]) -> GlobalPos {
        GlobalPos::new(
            Identifier::parse(dimension).unwrap(),
            BlockPos::new(x, y, z),
        )
    }

    fn respawn(dimension: &str, pos: [i32; 3], yaw: f32, forced: bool) -> PlayerRespawnConfig {
        PlayerRespawnConfig {
            position: at(dimension, pos),
            yaw,
            forced,
        }
    }

    /// `/spawnpoint`'s write and `getRespawnConfig`'s read, on each player's own record.
    #[test]
    #[ignore = "daily"]
    fn a_respawn_point_is_read_and_written_in_the_players_own_record() {
        with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            assert_eq!(a.respawn_config(&*game), Some(None), "none until set");

            // The yaw wraps as `RespawnData.of` does: 270 degrees is -90.
            let nether = "minecraft:the_nether";
            assert!(a.set_respawn_config(game, Some(respawn(nether, [1, 2, 3], 270.0, true))));
            assert_eq!(
                a.respawn_config(&*game),
                Some(Some(respawn(nether, [1, 2, 3], -90.0, true)))
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].respawn_elsewhere,
                Some((nether.into(), [1, 2, 3], -90.0, 0.0, true)),
                "the record a respawn reads"
            );
            assert_eq!(
                b.respawn_config(&*game),
                Some(None),
                "the other is untouched"
            );

            // The acting player reads the point swapped into place.
            let seen = game.as_guest(12, |game| a.respawn_config(&*game)).unwrap();
            assert_eq!(seen, Some(Some(respawn(nether, [1, 2, 3], -90.0, true))));

            // Refused writes change nothing: a dimension the game does not have, a yaw that is not a number.
            let nowhere = respawn("test:nowhere", [9, 9, 9], 0.0, false);
            assert!(!a.set_respawn_config(game, Some(nowhere)));
            let not_a_number = respawn(nether, [9, 9, 9], f32::NAN, false);
            assert!(!a.set_respawn_config(game, Some(not_a_number)));
            assert_eq!(
                a.respawn_config(&*game),
                Some(Some(respawn(nether, [1, 2, 3], -90.0, true)))
            );

            // None forgets it.
            assert!(a.set_respawn_config(game, None));
            assert_eq!(a.respawn_config(&*game), Some(None));
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].respawn_elsewhere,
                None
            );
        });
    }

    /// The write goes through `spawn_point_change`, so a mod's `player_set_spawn` veto refuses it, as it does `/spawnpoint`.
    #[test]
    #[ignore = "daily"]
    fn a_mods_veto_refuses_the_respawn_write() {
        struct Veto;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Veto {
            fn player_set_spawn(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mut mod_api::PlayerSetSpawnEvent,
            ) -> mod_api::Flow {
                mod_api::Flow::Cancel
            }
        }
        let mods = vec![mod_api::ModEntry::new("veto", Box::new(Veto))].into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let a = game.player(12).unwrap();
                let overworld = "minecraft:overworld";
                let config = respawn(overworld, [1, 2, 3], 0.0, false);
                assert!(!a.set_respawn_config(game, Some(config)));
                assert_eq!(a.respawn_config(&*game), Some(None));
            });
        });
    }

    /// The client's text-filtering option, as its `ClientInformation` said.
    #[test]
    #[ignore = "daily"]
    fn text_filtering_is_what_the_clients_information_said() {
        with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.info.text_filtering = true;
            });
            assert_eq!(a.text_filtering(&*game), Some(true));
            assert_eq!(b.text_filtering(&*game), Some(false));
            // A later `ClientInformation` replaces it.
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.info.text_filtering = false;
            });
            assert_eq!(a.text_filtering(&*game), Some(false));
        });
    }

    /// The sleep reads and the counter write, with the night skip's own counts as the oracle.
    #[test]
    #[ignore = "daily"]
    fn sleep_eligibility_and_the_timer_follow_the_players_state() {
        with_guests(|game| {
            in_this_level(game);
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            let (active, sleeping, deep) = game.sleep_counts();
            assert_eq!(a.sleep_timer(&*game), Some(0));
            assert_eq!(a.counts_for_sleep(&*game), Some(true));
            assert_eq!(a.sleep_ignored(&*game), Some(false));

            // A player in bed at 100 ticks is one of the deep sleepers the night waits for.
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.bed = Some([5, 64, 5]);
            });
            assert!(a.set_sleep_timer(game, 100));
            assert_eq!(a.sleep_timer(&*game), Some(100));
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].sleep_timer,
                100
            );
            assert_eq!(game.sleep_counts(), (active, sleeping + 1, deep + 1));
            assert!(a.set_sleep_timer(game, -1));
            assert_eq!(a.sleep_timer(&*game), Some(-1));
            assert_eq!(game.sleep_counts(), (active, sleeping + 1, deep));
            assert!(a.set_sleep_timer(game, 110));
            assert_eq!(a.sleep_timer(&*game), Some(110));

            // Ignoring leaves an out-of-bed player out of the count, and an in-bed one in.
            assert!(b.set_sleep_ignored(game, true));
            assert_eq!(b.sleep_ignored(&*game), Some(true));
            assert_eq!(b.counts_for_sleep(&*game), Some(false));
            assert_eq!(game.sleep_counts().0, active - 1);
            assert!(a.set_sleep_ignored(game, true));
            assert_eq!(a.counts_for_sleep(&*game), Some(true), "in bed");
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.bed = None);
            assert_eq!(a.counts_for_sleep(&*game), Some(false), "out of bed");
            assert!(a.set_sleep_ignored(game, false));
            assert!(b.set_sleep_ignored(game, false));
            assert_eq!(a.counts_for_sleep(&*game), Some(true));

            // A spectator is never counted.
            game.server.lan.as_mut().unwrap().hub.guests.update(1, |g| {
                g.game_mode = 3;
            });
            assert_eq!(b.counts_for_sleep(&*game), Some(false));

            // The acting player's state is the one in place.
            let seen = game
                .as_guest(12, |game| {
                    (a.sleep_timer(&*game), a.counts_for_sleep(&*game))
                })
                .unwrap();
            assert_eq!(seen, (Some(110), Some(true)));
        });
    }
}
use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
