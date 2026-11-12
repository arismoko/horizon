//! Adapter fixtures run the real Game transaction open/close, root journal,
//! deferred drops, and save gate. They need resources; the acceptance matrix
//! coordinator fixtures in decision_tests.rs do not.
use super::storage::GameHost;
use super::*;
use crate::game::{Game, native_entity_refs::tests::with_game};
use horizon_client::{chunk::Chunk, entities::ArmorStand};
use horizon_core::geometry::Vec3;
use std::sync::Arc;

fn identity(game: &mut Game<'_>) -> Identity {
    game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
    let holder = game.entity(13).expect("fixture holder");
    Identity {
        side: Side::Server,
        world: holder.epoch(),
        actor: holder,
        target: holder,
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn cancel_action_undoes_real_game_listener_block_and_deferred_drop() {
    with_game(|game| {
        let identity = identity(game);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let before = game.world.block(0, 64, 0);
        let drops = game.dropped.len();
        let menu = Menu::new(tests::state());
        let root = game.transaction_open();
        let mut host = GameHost { game, identity };
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
        host.game.transaction_close(&root, true);
        assert_eq!(host.game.dropped.len(), drops);
        assert!(host.game.transactions.is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn nested_real_game_commit_then_trap_rolls_back_next_placement_succeeds() {
    with_game(|game| {
        let identity = identity(game);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let dirt = game
            .pack
            .assets
            .registry
            .default_state("minecraft:dirt")
            .unwrap();
        let before = game.world.block(0, 64, 0);
        let menu = Menu::new(tests::state());
        let root = game.transaction_open();
        let mut host = GameHost { game, identity };
        assert_eq!(
            run(
                &mut host,
                &menu,
                |_, s| tests::placement(s),
                |host| {
                    let child = host.game.transaction_open();
                    host.game.journal_set_block([0, 64, 0], dirt);
                    menu.edit(host, |s| {
                        s.slots[2] = Some(ItemStack::new("minecraft:dirt", 2))
                    });
                    host.game.transaction_close(&child, true);
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
        host.game.transaction_close(&root, false);
        assert_eq!(menu.read(Clone::clone), tests::state());
        assert!(host.game.transactions.is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn outer_real_game_abort_undoes_applied_menu_block_cost_rng_and_drops() {
    with_game(|game| {
        let identity = identity(game);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let before = game.world.block(0, 64, 0);
        let drops = game.dropped.len();
        let menu = Menu::new(tests::state());
        let root = game.transaction_open();
        let mut host = GameHost { game, identity };
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
        host.game.transaction_close(&root, false);
        assert_eq!(menu.read(Clone::clone), tests::state());
        assert_eq!(host.game.world.block(0, 64, 0), before);
        assert_eq!(host.game.dropped.len(), drops);
        assert!(host.game.transactions.is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn real_save_gate_and_save_all_refuse_action_savepoint() {
    with_game(|game| {
        let identity = identity(game);
        let menu = Menu::new(tests::state());
        let root = game.transaction_open();
        let mut host = GameHost { game, identity };
        assert_eq!(
            run(
                &mut host,
                &menu,
                |_, s| tests::placement(s),
                |host| {
                    assert!(Game::admit_entity_save().is_err());
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
        host.game.transaction_close(&root, false);
        assert!(host.game.transactions.is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn only_root_commit_publishes_real_listener_and_plan_drops_in_order() {
    with_game(|game| {
        let identity = identity(game);
        let menu = Menu::new(tests::state());
        let initial = game.dropped.len();
        let root = game.transaction_open();
        let mut host = GameHost { game, identity };
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
                    let child = host.game.transaction_open();
                    host.defer(Effect::Drop(
                        [0, 64, 0],
                        ItemStack::new("minecraft:dirt", 2),
                    ));
                    host.game.transaction_close(&child, true);
                    assert_eq!(host.game.dropped.len(), initial);
                    Reply::Allow
                }
            ),
            Ok(Outcome::Applied)
        );
        assert_eq!(host.game.dropped.len(), initial);
        host.game.transaction_close(&root, true);
        let stacks: Vec<_> = host
            .game
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
        assert!(host.game.transactions.is_empty());
    });
}

// A decision abandoned by a listener unwind: the action savepoint closes, its
// save-gate lease is released (saving is admitted again), and the deferred
// drop is never published, even when the root commits. Like the others here,
// run serially (--test-threads=1): the save gate is process-wide.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn abandoned_token_releases_real_save_gate_and_publishes_nothing() {
    with_game(|game| {
        let identity = identity(game);
        let menu = Menu::new(tests::state());
        let drops = game.dropped.len();
        assert!(Game::admit_entity_save().is_ok());
        let root = game.transaction_open();
        let mut host = GameHost { game, identity };
        assert_eq!(
            run(
                &mut host,
                &menu,
                |_, view| tests::placement(view),
                |host| {
                    assert!(Game::admit_entity_save().is_err());
                    host.game
                        .drop_after_commit(1, ItemStack::new("minecraft:dirt", 2));
                    panic!("listener trap with a live decision");
                }
            ),
            Ok(Outcome::ListenerFailed)
        );
        assert!(Game::admit_entity_save().is_ok());
        assert!(!host.menu_busy(menu.id));
        assert_eq!(host.game.transactions.len(), 1);
        assert_eq!(menu.read(Clone::clone), tests::state());
        host.game.transaction_close(&root, true);
        assert_eq!(host.game.dropped.len(), drops);
        assert!(host.game.transactions.is_empty());
    });
}
