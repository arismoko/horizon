//! Adapter fixtures run the real Game transaction open/close, root journal,
//! deferred drops, and save gate. They need resources; the acceptance matrix
//! coordinator fixtures in decision_tests.rs do not.
use crate::game::native_entity_refs::tests::{fixture_resident_column, with_game};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::transactions::decision::storage::{ActionHost, GameHost};
use sunrise_gameplay_common::game::transactions::decision::{
    Effect, Identity, ItemStack, Menu, Mutation, Outcome, Reply, Side, Step, run, tests,
};
use sunrise_sim::entities::ArmorStand;

fn identity(game: &mut crate::game::ServerGame<'_>) -> Identity {
    fixture_resident_column(&mut game.common.world, (0, 0));
    game.common
        .armor_stands
        .push(ArmorStand::new(13, Vec3::ZERO, 0.0));
    let holder = game.entity(13).expect("fixture holder");
    Identity {
        side: Side::Server,
        world: holder.epoch(),
        actor: holder,
        target: holder,
    }
}

#[test]
#[ignore = "daily"]
fn cancel_action_undoes_real_game_listener_block_and_deferred_drop() {
    with_game(|game| {
        let identity = identity(game);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let before = game.world.block(0, 64, 0);
        let drops = game.dropped.len();
        let menu = Menu::new(tests::state());
        game.storage_scope(|game, _| {
            let mut host = GameHost::new(game, identity);
            assert_eq!(
                run(
                    &mut host,
                    &menu,
                    |_, s| tests::placement(s),
                    |host| {
                        host.game.journal_set_block([0, 64, 0], stone);
                        host.game
                            .drop_after_commit(1, ItemStack::new("minecraft:dirt", 2));
                        menu.edit(host, |s| {
                            s.slots[2] = Some(ItemStack::new("minecraft:dirt", 2))
                        });
                        Reply::Cancel
                    }
                ),
                Ok(Outcome::Canceled)
            );
            assert_eq!(menu.read(Clone::clone), tests::state());
            assert_eq!(host.game.world.block(0, 64, 0), before);
            assert_eq!(host.game.dropped.len(), drops);
            ((), true)
        });
        assert_eq!(game.dropped.len(), drops);
        assert!(game.transactions.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn nested_real_game_commit_then_trap_rolls_back_next_placement_succeeds() {
    with_game(|game| {
        let identity = identity(game);
        let dirt = game
            .pack
            .assets
            .registry
            .default_state("minecraft:dirt")
            .unwrap();
        let before = game.world.block(0, 64, 0);
        let menu = Menu::new(tests::state());
        game.storage_scope(|game, _| {
            let mut host = GameHost::new(game, identity);
            assert_eq!(
                run(
                    &mut host,
                    &menu,
                    |_, s| tests::placement(s),
                    |host| {
                        let identity = host.identity;
                        host.game.storage_scope(|game, _| {
                            let mut inner = GameHost::new(game, identity);
                            inner.game.journal_set_block([0, 64, 0], dirt);
                            menu.edit(&mut inner, |s| {
                                s.slots[2] = Some(ItemStack::new("minecraft:dirt", 2))
                            });
                            ((), true)
                        });
                        panic!("listener trap after nested commit");
                    }
                ),
                Ok(Outcome::ListenerFailed)
            );
            assert_eq!(host.game.world.block(0, 64, 0), before);
            assert_eq!(menu.read(Clone::clone), tests::state());
            assert_eq!(host.game.transactions.len(), 1);
            assert!(!host.menu_busy(menu.id));
            assert_eq!(
                run(
                    &mut host,
                    &menu,
                    |_, s| tests::placement(s),
                    |_| Reply::Allow
                ),
                Ok(Outcome::Applied)
            );
            assert_eq!(menu.read(|s| s.cursor.clone()), None);
            ((), false)
        });
        assert_eq!(menu.read(Clone::clone), tests::state());
        assert!(game.transactions.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn outer_real_game_abort_undoes_applied_menu_block_cost_rng_and_drops() {
    with_game(|game| {
        let identity = identity(game);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let before = game.world.block(0, 64, 0);
        let drops = game.dropped.len();
        let menu = Menu::new(tests::state());
        game.storage_scope(|game, _| {
            let mut host = GameHost::new(game, identity);
            assert_eq!(
                run(
                    &mut host,
                    &menu,
                    |_, s| {
                        let mut plan = tests::placement(s);
                        plan.steps.extend([
                            Step::Write(Mutation::Cost(2)),
                            Step::Write(Mutation::Rng(102)),
                        ]);
                        plan
                    },
                    |host| {
                        host.game.journal_set_block([0, 64, 0], stone);
                        host.game
                            .drop_after_commit(1, ItemStack::new("minecraft:dirt", 2));
                        Reply::Allow
                    }
                ),
                Ok(Outcome::Applied)
            );
            assert_eq!(menu.read(|s| (s.cost, s.rng)), (2, 102));
            assert_eq!(host.game.dropped.len(), drops);
            ((), false)
        });
        assert_eq!(menu.read(Clone::clone), tests::state());
        assert_eq!(game.world.block(0, 64, 0), before);
        assert_eq!(game.dropped.len(), drops);
        assert!(game.transactions.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn real_save_gate_and_save_all_refuse_action_savepoint() {
    with_game(|game| {
        let identity = identity(game);
        let menu = Menu::new(tests::state());
        game.storage_scope(|game, _| {
            let mut host = GameHost::new(game, identity);
            assert_eq!(
                run(
                    &mut host,
                    &menu,
                    |_, s| tests::placement(s),
                    |host| {
                        assert!(
                            sunrise_gameplay_common::game::CommonGame::admit_entity_save().is_err()
                        );
                        assert_eq!(
                            host.game.save_all_reply().translation_key(),
                            Some("commands.save.failed")
                        );
                        Reply::Cancel
                    }
                ),
                Ok(Outcome::Canceled)
            );
            assert_eq!(menu.read(Clone::clone), tests::state());
            ((), false)
        });
        assert!(game.transactions.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn only_root_commit_publishes_real_listener_and_plan_drops_in_order() {
    with_game(|game| {
        let identity = identity(game);
        let menu = Menu::new(tests::state());
        let initial = game.dropped.len();
        game.storage_scope(|game, _| {
            let mut host = GameHost::new(game, identity);
            assert_eq!(
                run(
                    &mut host,
                    &menu,
                    |_, s| {
                        let mut plan = tests::placement(s);
                        plan.steps.push(Step::Effect(Effect::Drop(
                            [0, 64, 0],
                            ItemStack::new("minecraft:sand", 3),
                        )));
                        plan
                    },
                    |host| {
                        host.defer(Effect::Drop(
                            [0, 64, 0],
                            ItemStack::new("minecraft:stone", 1),
                        ));
                        let identity = host.identity;
                        host.game.storage_scope(|game, _| {
                            let mut inner = GameHost::new(game, identity);
                            inner.defer(Effect::Drop(
                                [0, 64, 0],
                                ItemStack::new("minecraft:dirt", 2),
                            ));
                            ((), true)
                        });
                        assert_eq!(host.game.dropped.len(), initial);
                        Reply::Allow
                    }
                ),
                Ok(Outcome::Applied)
            );
            assert_eq!(host.game.dropped.len(), initial);
            ((), true)
        });
        let stacks: Vec<_> = game
            .dropped
            .iter()
            .skip(initial)
            .map(|item| item.stack.clone())
            .collect();
        assert_eq!(
            stacks,
            [
                ItemStack::new("minecraft:stone", 1),
                ItemStack::new("minecraft:dirt", 2),
                ItemStack::new("minecraft:sand", 3)
            ]
        );
        assert!(game.transactions.is_empty());
    });
}

// A decision abandoned by a listener unwind: the action savepoint closes, its
// save-gate lease is released (saving is admitted again), and the deferred
// drop is never published, even when the root commits. Like the others here,
// run serially (--test-threads=1): the save gate is process-wide.
#[test]
#[ignore = "daily"]
fn abandoned_token_releases_real_save_gate_and_publishes_nothing() {
    with_game(|game| {
        let identity = identity(game);
        let menu = Menu::new(tests::state());
        let drops = game.dropped.len();
        assert!(sunrise_gameplay_common::game::CommonGame::admit_entity_save().is_ok());
        game.storage_scope(|game, _| {
            let mut host = GameHost::new(game, identity);
            assert_eq!(
                run(
                    &mut host,
                    &menu,
                    |_, view| tests::placement(view),
                    |host| {
                        assert!(
                            sunrise_gameplay_common::game::CommonGame::admit_entity_save().is_err()
                        );
                        host.game
                            .drop_after_commit(1, ItemStack::new("minecraft:dirt", 2));
                        panic!("listener trap with a live decision");
                    }
                ),
                Ok(Outcome::ListenerFailed)
            );
            assert!(sunrise_gameplay_common::game::CommonGame::admit_entity_save().is_ok());
            assert!(!host.menu_busy(menu.id));
            assert_eq!(host.game.transactions.len(), 1);
            assert_eq!(menu.read(Clone::clone), tests::state());
            ((), true)
        });
        assert_eq!(game.dropped.len(), drops);
        assert!(game.transactions.is_empty());
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
