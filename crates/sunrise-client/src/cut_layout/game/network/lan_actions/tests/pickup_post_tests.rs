//! `item_picked_up_post` on whole and partial pickups through the game's own pickup
//! paths, the host's player (`tick_dropped_items`) and a LAN guest (`guest_pickups`),
//! with literal counts.
use mod_api::{
    Hooks, HostTransaction, ItemPickedUpEvent, ItemPickedUpPostEvent, ModEntry, ServerHost,
    TriState,
};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};
use sunrise_items::items::ItemStack;

#[derive(Clone, Default)]
pub(in crate::game) enum PreAction {
    #[default]
    None,
    SetStack(mod_api::ItemStack),
    AbortStackWrite(mod_api::ItemStack),
    ClearStack,
    SetDelay(i32),
    Discard,
    KillCommand,
}

#[derive(Default)]
pub(in crate::game) struct State {
    /// `item_picked_up`'s answer.
    pub(in crate::game) answer: TriState,
    /// Mutation performed inside `item_picked_up`.
    pub(in crate::game) pre_action: PreAction,
    /// A count the notice sets as the remainder.
    pub(in crate::game) raise: Option<u32>,
    /// Whether the notice empties the remainder.
    pub(in crate::game) empty: bool,
    /// Whether the notice removes every entity it can.
    pub(in crate::game) kill: bool,
    /// The notices heard, in order.
    pub(in crate::game) notices: Vec<Notice>,
    pub(in crate::game) touched: Vec<(u32, Option<mod_api::ItemStack>)>,
    pub(in crate::game) posted: Vec<u32>,
}

/// One notice heard: player, original count, remainder count.
pub(crate) type Notice = (u128, u32, Option<u32>);

#[derive(Clone, Default)]
pub(crate) struct Probe(pub(in crate::game) Arc<Mutex<State>>);
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn item_picked_up(&self, host: &mut dyn ServerHost, event: &mut ItemPickedUpEvent<'_>) {
        let read = event
            .item
            .with_stack(host, |stack| stack.project())
            .flatten();
        let (answer, action) = {
            let mut state = self.0.lock().unwrap();
            state.touched.push((event.item.entity().id(), read));
            (state.answer, state.pre_action.clone())
        };
        match action {
            PreAction::None => {}
            PreAction::SetStack(stack) => {
                assert_eq!(event.item.set_stack(host, Some(stack)), Ok(true));
            }
            PreAction::AbortStackWrite(stack) => {
                let result = host.transaction(|host, _| {
                    assert_eq!(event.item.set_stack(host, Some(stack)), Ok(true));
                    Err::<(), ()>(())
                });
                assert_eq!(result, Err(mod_api::TransactionError::Aborted(())));
            }
            PreAction::ClearStack => {
                assert_eq!(event.item.set_stack(host, None), Ok(true));
            }
            PreAction::SetDelay(ticks) => {
                assert!(event.item.set_pickup_delay(host, ticks));
            }
            PreAction::Discard => {
                assert!(event.item.entity().discard(host));
            }
            PreAction::KillCommand => {
                assert!(host.commands().run_command("kill @e[type=minecraft:item]"));
            }
        }
        *event.can_pickup = answer;
    }
    fn item_picked_up_post(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ItemPickedUpPostEvent<'_>,
    ) {
        self.0.lock().unwrap().posted.push(event.item.entity().id());
        self.heard(
            host,
            event.item,
            event.player,
            event.original,
            event.remaining,
        );
    }
}

impl Probe {
    /// Records the notice, then makes the case's changes.
    fn heard(
        &self,
        host: &mut dyn ServerHost,
        item: mod_api::ItemEntityRef,
        player: u128,
        original: &mod_api::ItemStack,
        remaining: &mut Option<mod_api::ItemStack>,
    ) {
        let (raise, empty, kill) = {
            let mut state = self.0.lock().unwrap();
            state.notices.push((
                player,
                original.count(),
                remaining.as_ref().map(|rest| rest.count()),
            ));
            (state.raise, state.empty, state.kill)
        };
        if let Some(count) = raise {
            let mut rest = original.clone();
            rest.set_count(count).unwrap();
            *remaining = Some(rest);
        }
        if empty {
            *remaining = None;
        }
        if kill {
            assert!(item.entity().discard(host));
        }
    }
}

/// One pickup case: the listener's setup, the inventory's room for apples,
/// and the ground stack's count and pickup delay.
pub(crate) struct Case {
    pub answer: TriState,
    pub raise: Option<u32>,
    pub empty: bool,
    pub kill: bool,
    pub room: u32,
    pub(crate) count: u32,
    pub(crate) delay: i32,
}

pub(crate) const CASE: Case = Case {
    answer: TriState::Default,
    raise: None,
    empty: false,
    kill: false,
    room: 8,
    count: 8,
    delay: 0,
};

/// What a case left: the notices heard, the apples on the ground (`None`
/// when the entity is gone) and the apples in the inventory's first slot.
pub(crate) type Outcome = (Vec<Notice>, Option<u32>, u32);

pub(crate) fn apples(room: u32) -> Option<ItemStack> {
    Some(match room {
        0 => ItemStack::new("minecraft:stone", 64),
        room => ItemStack::new("minecraft:apple", 64 - room),
    })
    .filter(|stack| stack.count > 0)
}

pub(crate) fn clear_ground(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    while !game.dropped.is_empty() {
        game.remove_dropped(0);
    }
}

pub(crate) fn arm(probe: &Probe, case: &Case) {
    let mut state = probe.0.lock().unwrap();
    state.answer = case.answer;
    state.pre_action = PreAction::None;
    state.raise = case.raise;
    state.empty = case.empty;
    state.kill = case.kill;
    state.notices.clear();
    state.touched.clear();
    state.posted.clear();
}

pub(crate) fn spawn(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    at: [f64; 3],
    case: &Case,
) -> u32 {
    let level = game.level(game.dim.id()).unwrap();
    game.level_spawn_item(
        level,
        at[0],
        at[1],
        at[2],
        ItemStack::new("minecraft:apple", case.count),
        case.delay,
    )
    .unwrap()
    .id()
}

pub(crate) fn ground(game: &sunrise_gameplay::game::ServerGame<'_>, id: u32) -> Option<u32> {
    game.dropped
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.stack.count)
}

pub(crate) fn with_probe(probe: &Probe, test: impl FnOnce()) {
    let mods: mod_api::Mods = vec![ModEntry::new("pickup_post", Box::new(probe.clone()))].into();
    with_mods(mods, test);
}

/// The host's player picks up through the lifted pass.
pub(crate) fn host_case(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    probe: &Probe,
    case: &Case,
) -> Outcome {
    clear_ground(game);
    game.inventory
        .fill(Some(ItemStack::new("minecraft:stone", 64)));
    game.offhand = Some(ItemStack::new("minecraft:stone", 64));
    game.inventory[0] = apples(case.room);
    arm(probe, case);
    let id = spawn(game, [1.5, 64.0, 1.5], case);
    let mut round = game.native_tick_round();
    game.tick_dropped_items(&mut round);
    game.native_removals_flush();
    let first = game.inventory[0].as_ref().map_or(0, |stack| {
        if &*stack.item == "minecraft:apple" {
            stack.count
        } else {
            0
        }
    });
    let notices = std::mem::take(&mut probe.0.lock().unwrap().notices);
    (notices, ground(game, id), first)
}

#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(crate) use sunrise_client_gameplay::game::client_side::lan_actions_client::pickups::pickup_post_tests::host_animations;

pub(crate) fn host_stat(game: &sunrise_gameplay::game::ServerGame<'_>) -> i64 {
    game.stats
        .values
        .get("minecraft:picked_up")
        .and_then(|items| items.get("minecraft:apple"))
        .map_or(0, |&n| i64::from(n))
}

/// Guest 12 picks up next to itself.
pub(crate) fn guest_case(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    probe: &Probe,
    case: &Case,
) -> Outcome {
    clear_ground(game);
    let room = case.room;
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(12, |guest| {
            guest
                .inventory
                .fill(Some(ItemStack::new("minecraft:stone", 64)));
            guest.offhand = Some(ItemStack::new("minecraft:stone", 64));
            guest.inventory[0] = apples(room);
        })
        .unwrap();
    arm(probe, case);
    let id = spawn(game, [3.0, 64.0, 5.0], case);
    let gi = game.guest_index(12).unwrap();
    game.guest_pickups(gi);
    if case.kill {
        assert!(
            game.entity(id).is_none(),
            "Post discard takes effect immediately"
        );
    }
    game.native_removals_flush();
    let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
    let first = guest.inventory[0].as_ref().map_or(0, |stack| {
        if &*stack.item == "minecraft:apple" {
            stack.count
        } else {
            0
        }
    });
    let notices = std::mem::take(&mut probe.0.lock().unwrap().notices);
    (notices, ground(game, id), first)
}

#[test]
#[ignore = "daily"]
fn guest_pickup_tells_whole_and_partial_pickups_and_keeps_listener_changes() {
    let probe = Probe::default();
    with_probe(&probe, || {
        sunrise_gameplay::game::native_guest_turn_tests::with_resident_guests(|game| {
            // The whole stack goes in: one post notice naming guest 12, original 8.
            assert_eq!(
                guest_case(game, &probe, &CASE),
                (vec![(12, 8, None)], None, 64)
            );

            // 8 offered, 5 fit: one post notice, original 8 (untouched by the insertion),
            // remainder 3.
            let partial = Case { room: 5, ..CASE };
            assert_eq!(
                guest_case(game, &probe, &partial),
                (vec![(12, 8, Some(3))], Some(3), 64)
            );

            // The listener on a partial pickup raises the remainder to 6: the entity keeps 6.
            let raised = Case {
                raise: Some(6),
                ..partial
            };
            assert_eq!(
                guest_case(game, &probe, &raised),
                (vec![(12, 8, Some(3))], Some(6), 64)
            );

            // The listener on a partial pickup empties the remainder: the entity goes.
            let emptied = Case {
                empty: true,
                ..partial
            };
            assert_eq!(
                guest_case(game, &probe, &emptied),
                (vec![(12, 8, Some(3))], None, 64)
            );

            // The listener on a partial pickup kills the entity: it stays gone, with no panic.
            let killed = Case {
                kill: true,
                ..partial
            };
            assert_eq!(
                guest_case(game, &probe, &killed),
                (vec![(12, 8, Some(3))], None, 64)
            );

            // Refused by `item_picked_up`: nothing is told.
            let refused = Case {
                answer: TriState::False,
                ..CASE
            };
            assert_eq!(guest_case(game, &probe, &refused), (vec![], Some(8), 56));

            // A full inventory takes nothing, even when `True` forces the pickup.
            let full = Case { room: 0, ..CASE };
            assert_eq!(guest_case(game, &probe, &full), (vec![], Some(8), 0));
            let forced = Case {
                answer: TriState::True,
                room: 0,
                delay: 40,
                ..CASE
            };
            assert_eq!(guest_case(game, &probe, &forced), (vec![], Some(8), 0));

            // The listener puts 3 back: the entity stays with 3.
            let raised = Case {
                raise: Some(3),
                ..CASE
            };
            assert_eq!(
                guest_case(game, &probe, &raised),
                (vec![(12, 8, None)], Some(3), 64)
            );

            // The listener puts 3 back, then kills the entity: it stays gone, with no panic.
            let killed = Case {
                raise: Some(3),
                kill: true,
                ..CASE
            };
            assert_eq!(
                guest_case(game, &probe, &killed),
                (vec![(12, 8, None)], None, 64)
            );
        });
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

fn ground_state(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    id: u32,
) -> Option<(String, u32, i32)> {
    game.dropped.iter().find(|item| item.id == id).map(|item| {
        (
            item.stack.item.to_string(),
            item.stack.count,
            item.pickup_delay,
        )
    })
}

/// Both the host-player and guest pickup paths share one NeoForge
/// `ItemEntity.playerTouch` sequence: `Pre` reads and mutates the live ground
/// item entity, captures the pre-event stack before `Pre`, respects live
/// post-hook `pickup_delay`, preserves explicit `set_stack` edits (including
/// empty `None`), and never resurrects an item removed by `discard` or `/kill`.
#[test]
#[ignore = "daily"]
fn pre_pickup_edits_and_removals_match_on_host_and_guest() {
    let probe = Probe::default();
    with_probe(&probe, || {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            game.build_registries();
            game.ui.dispatcher = sunrise_gameplay_common::game::chat_common::SessionDispatcher(
                Arc::new(sunrise_command::vanilla::build_for(false, true)),
            );
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(1.5, 64.0, 1.5),
                )
                .unwrap(),
            );
            game.level.player_uuid = Some(sunrise_sim::lan::playerdata::uuid_ints(7));
            game.server.lan_acting_uuid = Some(7);
            game.server.lan_acting_name = Some("pickup_post".into());
            // The guest fixture reserves network IDs below the pickup entities.
            game.server.next_entity = game.server.next_entity.max(100);
            sunrise_gameplay::game::native_entity_refs::tests::fixture_resident_column(
                &mut game.world,
                (0, 0),
            );

            let replacement = sunrise_gameplay_common::game::api_values::test_snapshot(
                &ItemStack::new("minecraft:emerald", 5),
            );

            for is_host in [true, false] {
                let expected_player = if is_host { 7_u128 } else { 12_u128 };
                let at = if is_host {
                    [1.5, 64.0, 1.5]
                } else {
                    [3.0, 64.0, 5.0]
                };

                let run = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                           case: &Case,
                           pre_action: PreAction,
                           flush: bool|
                 -> (u32, Vec<Notice>, Option<(String, u32, i32)>, u32) {
                    clear_ground(game);
                    if is_host {
                        game.inventory
                            .fill(Some(ItemStack::new("minecraft:stone", 64)));
                        game.offhand = Some(ItemStack::new("minecraft:stone", 64));
                        game.inventory[0] = apples(case.room);
                    } else {
                        let room = case.room;
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .update_guest(12, |guest| {
                                guest
                                    .inventory
                                    .fill(Some(ItemStack::new("minecraft:stone", 64)));
                                guest.offhand = Some(ItemStack::new("minecraft:stone", 64));
                                guest.inventory[0] = apples(room);
                            })
                            .unwrap();
                    }
                    arm(&probe, case);
                    probe.0.lock().unwrap().pre_action = pre_action;
                    let id = spawn(game, at, case);
                    if is_host {
                        let mut round = game.native_tick_round();
                        game.tick_dropped_items(&mut round);
                    } else {
                        let gi = game.guest_index(12).unwrap();
                        game.guest_pickups(gi);
                    }
                    if flush {
                        game.native_removals_flush();
                    }
                    let first = if is_host {
                        game.inventory[0].as_ref().map_or(0, |stack| {
                            if &*stack.item == "minecraft:apple" {
                                stack.count
                            } else {
                                0
                            }
                        })
                    } else {
                        game.server
                            .lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .inventory[0]
                            .as_ref()
                            .map_or(0, |stack| {
                                if &*stack.item == "minecraft:apple" {
                                    stack.count
                                } else {
                                    0
                                }
                            })
                    };
                    let notices = std::mem::take(&mut probe.0.lock().unwrap().notices);
                    (id, notices, ground_state(game, id), first)
                };

                // 1. Pre stack read & replacement on False veto: entity keeps replacement, no pickup.
                let (id, notices, ground_after, first) = run(
                    game,
                    &Case {
                        answer: TriState::False,
                        ..CASE
                    },
                    PreAction::SetStack(replacement.clone()),
                    true,
                );
                assert_eq!(notices, vec![]);
                assert_eq!(first, 56);
                assert_eq!(
                    ground_after,
                    Some(("minecraft:emerald".into(), 5, 0)),
                    "False veto preserves Pre set_stack replacement"
                );
                assert_eq!(
                    probe.0.lock().unwrap().touched,
                    vec![(
                        id,
                        Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                            &ItemStack::new("minecraft:apple", 8),
                        ))
                    )],
                    "Pre sees the live pre-event ground stack on both paths"
                );

                // 2. Pre stack replacement on partial pickup: pre-event apples are inserted,
                // Post sees original 8 / remainder 3, and entity keeps replacement emerald(5).
                let (_, notices, ground_after, first) = run(
                    game,
                    &Case { room: 5, ..CASE },
                    PreAction::SetStack(replacement.clone()),
                    true,
                );
                assert_eq!(notices, vec![(expected_player, 8, Some(3))]);
                assert_eq!(first, 64);
                assert_eq!(
                    ground_after,
                    Some(("minecraft:emerald".into(), 5, 0)),
                    "partial pickup preserves Pre set_stack replacement"
                );

                // 3. All pre-event apples fit, and the live replacement is left independently.
                for (kind, count) in [("minecraft:emerald", 5), ("minecraft:apple", 8)] {
                    let replacement = sunrise_gameplay_common::game::api_values::test_snapshot(
                        &ItemStack::new(kind, count),
                    );
                    let (_, notices, ground_after, first) =
                        run(game, &CASE, PreAction::SetStack(replacement), true);
                    assert_eq!(notices, vec![(expected_player, 8, None)]);
                    assert_eq!(first, 64, "the inventory took the eight captured apples");
                    assert_eq!(ground_after, Some((kind.into(), count, 0)));
                }
                // An aborted setter doesn't replace the captured stack.
                let (_, notices, ground_after, first) = run(
                    game,
                    &CASE,
                    PreAction::AbortStackWrite(replacement.clone()),
                    true,
                );
                assert_eq!(notices, vec![(expected_player, 8, None)]);
                assert_eq!(first, 64);
                assert_eq!(ground_after, None);

                // 4. Pre empty set_stack(None) on False veto: entity stays resident with empty air stack.
                let (_, notices, ground_after, first) = run(
                    game,
                    &Case {
                        answer: TriState::False,
                        ..CASE
                    },
                    PreAction::ClearStack,
                    true,
                );
                assert_eq!(notices, vec![]);
                assert_eq!(first, 56);
                assert_eq!(ground_after, Some(("minecraft:air".into(), 0, 0)));

                // 5. Pre empty set_stack(None) on partial pickup: pre-event apples inserted,
                // entity keeps empty air stack instead of being overwritten with remainder apples.
                let (_, notices, ground_after, first) =
                    run(game, &Case { room: 5, ..CASE }, PreAction::ClearStack, true);
                assert_eq!(notices, vec![(expected_player, 8, Some(3))]);
                assert_eq!(first, 64);
                assert_eq!(ground_after, Some(("minecraft:air".into(), 0, 0)));

                // 6. Pre explicit cooldown with False veto: delay is preserved, no pickup.
                let (_, notices, ground_after, first) = run(
                    game,
                    &Case {
                        answer: TriState::False,
                        ..CASE
                    },
                    PreAction::SetDelay(25),
                    true,
                );
                assert_eq!(notices, vec![]);
                assert_eq!(first, 56);
                assert_eq!(ground_after, Some(("minecraft:apple".into(), 8, 25)));

                // 7. Pre explicit cooldown with Default: live post-hook delay blocks pickup.
                let (_, notices, ground_after, first) =
                    run(game, &CASE, PreAction::SetDelay(25), true);
                assert_eq!(notices, vec![]);
                assert_eq!(first, 56);
                assert_eq!(ground_after, Some(("minecraft:apple".into(), 8, 25)));

                // 8. Default needs exactly zero, while True overrides a negative delay.
                let (_, notices, ground_after, first) = run(
                    game,
                    &Case { delay: 40, ..CASE },
                    PreAction::SetDelay(-5),
                    true,
                );
                assert!(notices.is_empty());
                assert_eq!(first, 56);
                assert_eq!(ground_after, Some(("minecraft:apple".into(), 8, -5)));
                for (delay, answer) in [(0, TriState::Default), (-5, TriState::True)] {
                    let (_, notices, ground_after, first) = run(
                        game,
                        &Case {
                            delay: 40,
                            answer,
                            ..CASE
                        },
                        PreAction::SetDelay(delay),
                        true,
                    );
                    assert_eq!(notices, vec![(expected_player, 8, None)]);
                    assert_eq!(first, 64);
                    assert_eq!(ground_after, None);
                }

                // 9. Pre removal via EntityRef::discard: no pickup, no Post, entity flushed away.
                let (id, notices, _, first) = run(game, &CASE, PreAction::Discard, false);
                assert_eq!(notices, vec![]);
                assert_eq!(first, 56);
                assert!(
                    game.entity(id).is_none(),
                    "discarded item is logically removed immediately"
                );
                game.native_removals_flush();
                assert_eq!(ground_state(game, id), None);

                // 10. Pre removal via /kill: no pickup, no Post, and never resurrected.
                let (_, notices, ground_after, first) =
                    run(game, &CASE, PreAction::KillCommand, false);
                assert_eq!(notices, vec![]);
                assert_eq!(first, 56);
                assert_eq!(
                    ground_after, None,
                    "/kill in Pre must not be resurrected on return"
                );
            }
        });
    });
}
