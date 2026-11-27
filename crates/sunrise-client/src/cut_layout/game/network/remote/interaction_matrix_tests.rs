//! Independently specified outcomes for the interaction hooks.
#[cfg(feature = "client")]
use super::*;
#[path = "interaction_matrix_tests/fixture.rs"]
pub(crate) mod fixture;

#[cfg(feature = "client")]
pub(crate) struct Native;
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn right_click_block(&self, host: HostSide<'_>, event: &mut RightClickBlockEvent<'_>) -> Flow {
        fixture::right_click_block(host, event)
    }
    fn use_item_on_block(&self, host: HostSide<'_>, event: &mut UseItemOnBlockEvent<'_>) -> Flow {
        fixture::use_item_on_block(host, event)
    }
}

#[cfg(feature = "client")]
#[derive(Debug)]
pub(crate) struct Case {
    pub(in crate::game) controls: [u8; 6],
    pub(in crate::game) held: bool,
    pub(in crate::game) other_held: bool,
    pub(in crate::game) sneak: bool,
    pub(in crate::game) phases: Vec<u8>,
    pub(in crate::game) used: bool,
    // Server and client swing counts, written without result helper methods.
    pub(in crate::game) swings: [u32; 2],
    pub(in crate::game) item_award: bool,
    pub(in crate::game) default_award: bool,
    pub(in crate::game) lever: bool,
}

#[cfg(feature = "client")]
pub(crate) fn cases(hand: Hand) -> Vec<Case> {
    let mut cases = Vec::new();
    // RightClickBlock: TRUE overrides secondary use; Default skips the block
    // only when sneaking with either hand full; FALSE always skips it.
    // The item phase requires a stack unless useItem is explicitly TRUE.
    for held in [false, true] {
        for other_held in [false, true] {
            for sneak in [false, true] {
                for (block_switch, block) in
                    [(0, !(sneak && (held || other_held))), (1, true), (2, false)]
                {
                    for (item_switch, before, after) in
                        [(0, held, held), (1, held, true), (2, false, false)]
                    {
                        let mut phases = vec![0];
                        if before {
                            phases.push(1);
                        }
                        if block {
                            phases.push(2);
                        }
                        if !block && after {
                            phases.push(3);
                        }
                        cases.push(Case {
                            controls: [block_switch, item_switch, 0, 5, 5, 0],
                            held,
                            other_held,
                            sneak,
                            phases,
                            used: block || after,
                            swings: [0, 0],
                            // ITEM_USED_ON_BLOCK sees the stack copied before the
                            // block phase, whichever phase consumes: air when the
                            // hand is empty, including a forced empty item phase.
                            item_award: block || after,
                            default_award: false,
                            lever: false,
                        });
                    }
                }
            }
        }
    }
    // Canceling RightClickBlock ends its turn even for PASS/TRY; no
    // block or item phase and no advancement is produced.
    for (answer, used, swings) in [
        (1, false, [0, 0]),
        (2, true, [0, 0]),
        (3, true, [1, 1]),
        (4, true, [1, 0]),
        (5, true, [0, 0]),
        (6, false, [0, 0]),
    ] {
        cases.push(Case {
            controls: [0, 0, 0, 0, 0, answer],
            held: true,
            other_held: false,
            sneak: false,
            phases: vec![0],
            used,
            swings,
            item_award: false,
            default_award: false,
            lever: false,
        });
    }
    // Only PASS before the block continues. Every other answer ends the hand:
    // FAIL and the three consuming answers use the click, TRY does not. A
    // before-block answer awards no criterion.
    for (answer, continues, used, swings) in [
        (1, true, true, [0, 0]),
        (2, false, true, [0, 0]),
        (3, false, true, [1, 1]),
        (4, false, true, [1, 0]),
        (5, false, true, [0, 0]),
        (6, false, false, [0, 0]),
    ] {
        cases.push(Case {
            controls: [1, 0, answer, 1, 5, 0],
            held: true,
            other_held: false,
            sneak: false,
            phases: if continues {
                vec![0, 1, 2, 3]
            } else {
                vec![0, 1]
            },
            used,
            swings,
            item_award: continues,
            default_award: false,
            lever: false,
        });
    }
    // A canceled block PASS/FAIL delegates to the item phase; TRY also asks
    // the block's empty method, which stone passes. Consuming answers stop.
    for (answer, continues, swings) in [
        (1, true, [0, 0]),
        (2, true, [0, 0]),
        (3, false, [1, 1]),
        (4, false, [1, 0]),
        (5, false, [0, 0]),
        (6, true, [0, 0]),
    ] {
        cases.push(Case {
            controls: [1, 0, 0, answer, 5, 0],
            held: true,
            other_held: false,
            sneak: false,
            phases: if continues {
                vec![0, 1, 2, 3]
            } else {
                vec![0, 1, 2]
            },
            used: true,
            swings,
            item_award: true,
            default_award: false,
            lever: false,
        });
    }
    // After-block PASS/TRY stop this item method without consuming the click;
    // FAIL vetoes it. Only consuming results award ITEM_USED_ON_BLOCK.
    for (answer, used, swings, award) in [
        (1, false, [0, 0], false),
        (2, true, [0, 0], false),
        (3, true, [1, 1], true),
        (4, true, [1, 0], true),
        (5, true, [0, 0], true),
        (6, false, [0, 0], false),
    ] {
        cases.push(Case {
            controls: [1, 0, 0, 1, answer, 0],
            held: true,
            other_held: false,
            sneak: false,
            phases: vec![0, 1, 2, 3],
            used,
            swings,
            item_award: award,
            default_award: false,
            lever: false,
        });
    }
    // Natural lever use and canceled TRY reach useWithoutItem only on MAIN.
    // Explicit PASS/FAIL in BLOCK skip that method, so the item answers.
    for (answer, empty_method) in [(0, true), (6, true), (1, false), (2, false)] {
        let pulls = empty_method && hand == Hand::Main;
        cases.push(Case {
            controls: [1, 0, 1, answer, 5, 0],
            held: true,
            other_held: false,
            sneak: true,
            phases: if pulls {
                vec![0, 1, 2]
            } else {
                vec![0, 1, 2, 3]
            },
            used: true,
            swings: if pulls { [1, 1] } else { [0, 0] },
            item_award: !pulls,
            default_award: pulls,
            lever: true,
        });
    }
    cases
}

#[cfg(feature = "client")]
pub(crate) fn advancement_tree(game: &mut sunrise_gameplay_common::game::CommonGame<'_>) {
    use sunrise_sim::advancements as a;
    let mut tree = a::Tree::default();
    for (index, name) in ["item_used_on_block", "default_block_use"]
        .into_iter()
        .enumerate()
    {
        let id = format!("test:{name}");
        let trigger = format!("minecraft:{name}");
        tree.by_id.insert(id.clone(), index);
        tree.by_trigger.insert(trigger.clone(), vec![(index, 0)]);
        tree.list.push(a::Advancement {
            id,
            parent: None,
            display: None,
            criteria: vec![a::Criterion {
                name: "seen".into(),
                trigger,
                conditions: serde_json::json!({}).into(),
                player: None,
            }],
            requirements: vec![vec!["seen".into()]],
            rewards: Default::default(),
        });
    }
    game.adv.tree = Arc::new(tree);
    game.adv.loaded = true;
    game.adv.progress = Default::default();
}

#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(crate) use crate::game::server_side::remote_server::interaction_event_tests::interaction_matrix_tests::matrix;

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn native_interaction_shared_matrix() {
    // Loading the fixture's block-entity type freezes the process-wide block entity type table.
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::interaction_event_tests::interaction_matrix_tests::native_interaction_shared_matrix",
        || {
            matrix(
                vec![mod_api::ModEntry::new(
                    "interaction_fixture",
                    Box::new(Native),
                )]
                .into(),
            );
        },
    );
}
