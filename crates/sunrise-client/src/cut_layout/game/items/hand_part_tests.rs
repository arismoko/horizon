#![cfg(test)]
//! A stick's `ItemPart` answers `should_cause_reequip_animation` and
//! `should_cause_block_break_reset` (`IItemExtension.shouldCauseReequipAnimation`,
//! `shouldCauseBlockBreakReset`) on the local player's client.
use mod_api::{
    ClientHost, Hooks, ItemPart, Registrar, RegistrationContext, ShouldCauseBlockBreakResetEvent,
    ShouldCauseReequipAnimationEvent,
};
use std::sync::Mutex;
use sunrise_items::items::ItemStack;

/// What the part answers: `None` leaves vanilla's answer.
pub(crate) static REEQUIP: Mutex<Option<bool>> = Mutex::new(None);
pub(crate) static RESET: Mutex<Option<bool>> = Mutex::new(None);
/// What the part was asked: `(old count, stack count, hand, slot changed)`
/// and, for a break, whether the break's stack had components.
pub(crate) static REEQUIP_ASKED: Mutex<Vec<(Option<u32>, u32, mod_api::Hand, bool)>> =
    Mutex::new(Vec::new());
pub(crate) static RESET_ASKED: Mutex<Vec<bool>> = Mutex::new(Vec::new());

#[derive(Clone)]
pub(in crate::game) struct Stick;
#[mod_api::sunrise_part]
impl ItemPart for Stick {
    fn should_cause_reequip_animation(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldCauseReequipAnimationEvent<'_>,
    ) {
        REEQUIP_ASKED.lock().unwrap().push((
            event.old.map(|s| s.count()),
            event.stack.count(),
            event.hand,
            event.slot_changed,
        ));
        if let Some(answer) = *REEQUIP.lock().unwrap() {
            event.reequip = answer;
        }
    }
    fn should_cause_block_break_reset(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldCauseBlockBreakResetEvent<'_>,
    ) {
        RESET_ASKED
            .lock()
            .unwrap()
            .push(event.stack.components().is_some());
        if let Some(answer) = *RESET.lock().unwrap() {
            event.reset = answer;
        }
    }
}
pub(crate) struct Parts;
#[mod_api::__subscriptions]
impl Hooks for Parts {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:stick");
        r.item_part(key, Stick);
    }
}

/// Runs `body` on a game whose stick has the part.
pub(crate) fn with_stick_part(
    body: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("hands", Box::new(Parts))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        crate::game::server_side::native_entity_refs::tests::with_client_game(body);
    });
}

/// Puts `main` in slot 0, selects it, and ticks until the attack strength is
/// full and the hand is up.
pub(crate) fn settle(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, main: ItemStack) {
    game.inventory[0] = Some(main);
    game.selected = 0;
    for _ in 0..20 {
        game.tick_hand();
    }
    assert_eq!(game.client.heights.main, 1.0);
    REEQUIP_ASKED.lock().unwrap().clear();
}

/// The main hand's height over the next six ticks, and the stick count it
/// shows after each.
pub(crate) fn dip(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) -> Vec<(f32, u32)> {
    (0..6)
        .map(|_| {
            game.tick_hand();
            let shown = game.client.visible_hands[0]
                .1
                .as_ref()
                .map_or(0, |s| s.count);
            (game.client.heights.main, shown)
        })
        .collect()
}

pub(crate) fn close(got: &[(f32, u32)], want: &[(f32, u32)]) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g.0 - w.0).abs() < 1e-6 && g.1 == w.1,
            "{got:?} != {want:?}"
        );
    }
}

#[test]
#[ignore = "daily"]
fn a_part_decides_whether_the_hand_lowers_for_a_new_stack() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::hand_part_tests::a_part_decides_whether_the_hand_lowers_for_a_new_stack",
        || {
            with_stick_part(|game| {
                let stick = |count| ItemStack::new("minecraft:stick", count);
                // From one stick in slot 0 to one in slot 1, as vanilla: shown at once.
                settle(game, stick(1));
                game.inventory[1] = Some(stick(1));
                game.selected = 1;
                close(&dip(game)[..1], &[(1.0, 1)]);
                assert_eq!(
                    REEQUIP_ASKED.lock().unwrap().drain(..).collect::<Vec<_>>(),
                    [(Some(1), 1, mod_api::Hand::Main, true)]
                );

                // The same move with the part answering `true`: the hand drops,
                // takes the new stick at the bottom and rises again.
                *REEQUIP.lock().unwrap() = Some(true);
                settle(game, stick(1));
                game.selected = 1;
                close(
                    &dip(game),
                    &[(0.6, 1), (0.2, 1), (0.0, 1), (0.4, 1), (0.8, 1), (1.0, 1)],
                );

                // From one stick to two in the same slot: vanilla dips, the part's
                // `false` shows them at once.
                *REEQUIP.lock().unwrap() = None;
                settle(game, stick(1));
                game.inventory[0] = Some(stick(2));
                game.hand_stack_replaced(0);
                close(&dip(game)[..2], &[(0.6, 1), (0.2, 1)]);
                assert_eq!(
                    REEQUIP_ASKED.lock().unwrap()[0],
                    (Some(1), 2, mod_api::Hand::Main, false)
                );
                *REEQUIP.lock().unwrap() = Some(false);
                settle(game, stick(1));
                game.inventory[0] = Some(stick(2));
                game.hand_stack_replaced(0);
                close(&dip(game)[..1], &[(1.0, 2)]);

                // A hand that shows what it holds, and an empty hand, ask nothing.
                REEQUIP_ASKED.lock().unwrap().clear();
                for _ in 0..20 {
                    game.tick_hand();
                }
                game.inventory[0] = None;
                game.tick_hand();
                assert!(REEQUIP_ASKED.lock().unwrap().is_empty());
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_part_decides_whether_a_changed_stick_restarts_the_break() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::hand_part_tests::a_part_decides_whether_a_changed_stick_restarts_the_break",
        || {
            with_stick_part(|game| {
                game.inventory[0] = Some(ItemStack::new("minecraft:stick", 1));
                game.selected = 0;
                let began = game.main_hand_key();
                // The same stick, a few more of it or a little damage: the same break,
                // and the part sees it asked.
                assert!(game.same_destroying_item(&began));
                game.inventory[0].as_mut().unwrap().count = 3;
                assert!(game.same_destroying_item(&began));
                assert_eq!(*RESET_ASKED.lock().unwrap(), [false, false]);

                // The stick gains a component (a name): vanilla restarts the break.
                game.inventory[0].as_mut().unwrap().data_mut().custom_name =
                    Some(sunrise_core::nbt::Tag::String("Wand".into()));
                assert!(!game.same_destroying_item(&began));
                // The part's `false` keeps it going; it saw the stick the break began with.
                *RESET.lock().unwrap() = Some(false);
                assert!(game.same_destroying_item(&began));
                assert_eq!(*RESET_ASKED.lock().unwrap(), [false, false, false, false]);
                // A break begun with an empty hand asks nothing.
                RESET_ASKED.lock().unwrap().clear();
                assert!(!game.same_destroying_item(&None));
                assert!(RESET_ASKED.lock().unwrap().is_empty());
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn without_a_part_the_break_compares_item_and_components() {
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        game.inventory[0] = Some(ItemStack::new("minecraft:stick", 1));
        game.selected = 0;
        let began = game.main_hand_key();
        // A use shrinks the stack in place: the same break.
        game.inventory[0].as_mut().unwrap().count = 5;
        assert!(game.same_destroying_item(&began));
        // The server's new stack with more wear: a damage component apart, so it restarts.
        game.inventory[0].as_mut().unwrap().damage = 2;
        assert!(!game.same_destroying_item(&began));
        game.inventory[0] = Some(ItemStack::new("minecraft:diamond_pickaxe", 1));
        assert!(!game.same_destroying_item(&began));
        game.inventory[0] = None;
        assert!(!game.same_destroying_item(&began));
        assert!(game.same_destroying_item(&None));
    });
}

#[test]
#[ignore = "daily"]
fn remote_mining_component_update_restarts_unless_the_part_refuses() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::hand_part_tests::remote_mining_component_update_restarts_unless_the_part_refuses",
        || {
            for keep in [false, true] {
                with_stick_part(|game| {
                    use std::sync::Arc;
                    use sunrise_core::geometry::Vec3;
                    use sunrise_level::chunk::Chunk;
                    use sunrise_protocol::packets::play;
                    use sunrise_sim::motion::PlayerMotion;
                    *RESET.lock().unwrap() = keep.then_some(false);
                    // Play packets are suppressed until login/configuration finishes.
                    let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
                    peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
                        .unwrap();
                    game.set_local_mode(sunrise_command::args::GameMode::Survival);
                    let mut player =
                        PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap();
                    player.ground.on_ground = true;
                    game.player = Some(player);
                    game.inventory[0] = Some(ItemStack::new("minecraft:stick", 1));
                    game.selected = 0;
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(0, 64, 0, stone).unwrap();
                    game.set_test_forced_hit(Some(sunrise_gameplay_common::game::Hit {
                        pos: [0, 64, 0],
                        face: [0, 1, 0],
                        point: [0.5, 65.0, 0.5],
                        inside: false,
                    }));
                    game.refresh_client_pick(1.0);
                    game.set_test_forced_hit(None);
                    for _ in 0..4 {
                        game.remote_continue_attack(true);
                    }
                    let before = game.mining.as_ref().unwrap().progress;
                    while peer.recv().is_ok() {}
                    game.inventory[0].as_mut().unwrap().data_mut().custom_name =
                        Some(sunrise_core::nbt::Tag::String("Wand".into()));
                    game.remote_continue_attack(true);
                    let mut actions = Vec::new();
                    while let Ok((id, bytes)) = peer.recv() {
                        if let Ok(play::Serverbound::PlayerAction(action)) =
                            play::Serverbound::decode(id, &bytes)
                        {
                            actions.push(action.action);
                        }
                    }
                    let after = game.mining.as_ref().unwrap().progress;
                    if keep {
                        assert!(actions.is_empty());
                        assert!((after - before - 1.0 / 150.0).abs() < 1e-6);
                    } else {
                        assert_eq!(actions, [2, 0]);
                        assert_eq!(after, 0.0);
                    }
                });
            }
        },
    );
}

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
