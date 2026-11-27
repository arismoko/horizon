//! `item_stacked_on_other` through a real listener on the local menu route and the LAN hub's
//! container click: decided, fired with `HostSide`, validated once and applied once. The
//! fixtures need prepared resources.
//!
//! This hook's cancellation is an override result, not an abort: a cancelled, handled click
//! sets the stacks the listener names and is Applied. The abort outcomes are a stale read, an
//! invalid answer and a failing listener; each undoes every write the listener made.
#[cfg(test)]
use crate::game::native_guest_turn_tests::with_guests;
use mod_api::{
    Flow, Hooks, HostSide, HostTransaction, ItemPlace, ItemResource, ItemStackedOnOtherEvent,
    ModEntry, Mods,
};
use std::cell::RefCell;
use sunrise_items::items::ItemStack;
#[cfg(test)]
use sunrise_sim::menu::Input;

/// What the listener does.
#[derive(Clone, Default)]
pub struct Script {
    /// Put dirt in this inventory slot through the `Host` (its own nested transaction).
    pub write_dirt: Option<i32>,
    /// ...and take it out again at once.
    pub take_dirt_back: bool,
    pub panic: bool,
    pub cancel: bool,
    pub handled: bool,
    /// Set the cursor to this stack (item, count); `Some(None)` empties it.
    pub set_carried: Option<Option<(&'static str, u32)>>,
    pub set_clicked: Option<Option<(&'static str, u32)>>,
    pub take_clicked: Option<u32>,
    pub dirt_rule: Option<(bool, u32)>,
    pub write_scoreboard: bool,
}

thread_local! {
    pub(in crate::game) static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
    /// What the listener saw each time: the cursor's and the clicked slot's counts.
    pub static SEEN: RefCell<Vec<(u32, u32)>> = const { RefCell::new(Vec::new()) };
    pub static PLAYER: std::cell::Cell<u128> = const { std::cell::Cell::new(0) };
    /// How often the menu type's `menu_slot_rules` part was asked.
    pub static ASKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(in crate::game) struct Rules;
#[mod_api::sunrise_part]
impl mod_api::MenuPart for Rules {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut mod_api::MenuSlotRulesEvent<'_>) {
        ASKS.set(ASKS.get() + 1);
        if mod_api::__private::item_key_id(event.stack.item()) == Some("minecraft:dirt")
            && let Some((may_place, max_stack)) = SCRIPT.with(|s| s.borrow().dirt_rule)
        {
            event.may_place = may_place;
            event.max_stack = max_stack;
        }
    }
}

pub fn script(script: Script) {
    SEEN.with(|seen| seen.borrow_mut().clear());
    SCRIPT.with(|s| *s.borrow_mut() = script);
}
pub fn dispatches() -> usize {
    SEEN.with(|seen| seen.borrow().len())
}

pub(in crate::game) fn api_stack(item: &str, count: u32) -> mod_api::ItemStack {
    mod_api::ItemStack::new(mod_api::__private::item_key(item).unwrap(), count).unwrap()
}

/// A script's edits on the lent click: the clicked slot's take or write, then the cursor.
fn edit_click(
    click: &mut mod_api::item::menu::StackedOnOther<'_>,
    take: Option<u32>,
    clicked: Option<Option<(&str, u32)>>,
    carried: Option<Option<(&str, u32)>>,
) {
    let stack = |r: Option<(&str, u32)>| r.map(|(item, count)| api_stack(item, count));
    match (take, clicked) {
        (Some(count), _) => {
            click.take_clicked(count);
        }
        (None, Some(write)) => click.set_clicked(stack(write)),
        (None, None) => {}
    }
    if let Some(write) = carried {
        click.set_carried(stack(write));
    }
}

pub(in crate::game) struct Listener;
#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x3", Rules);
        r.menu_part("minecraft:crafting", Rules);
    }
    fn item_stacked_on_other(
        &self,
        mut host: HostSide,
        event: &mut ItemStackedOnOtherEvent<'_, '_>,
    ) -> Flow {
        let counts = (
            event.click.carried().map_or(0, |s| s.count()),
            event.click.clicked().map_or(0, |s| s.count()),
        );
        SEEN.with(|seen| seen.borrow_mut().push(counts));
        let s = SCRIPT.with(|s| s.borrow().clone());
        if let (Some(slot), HostSide::Server(host)) = (s.write_dirt, &mut host) {
            let place = ItemPlace::PlayerSlot {
                player: PLAYER.get(),
                slot,
            };
            let dirt = api_stack("minecraft:dirt", 1);
            assert!(!host.transaction_matches(None), "inside the action");
            host.transaction(|host, tx| {
                assert_eq!(place.insert(host, tx, &ItemResource::of(&dirt), 2), Ok(2));
                if s.take_dirt_back {
                    assert_eq!(place.extract(host, tx, &ItemResource::of(&dirt), 2), Ok(2));
                }
                Ok::<_, ()>(())
            })
            .unwrap();
        }
        if s.write_scoreboard
            && let HostSide::Server(host) = &mut host
        {
            mod_api::prelude::ScoreboardServerHost::add_objective(
                &mut **host,
                "pickup",
                "dummy",
                mod_api::Text::literal("pickup"),
                mod_api::ObjectiveRenderType::Integer,
            )
            .unwrap();
        }
        if s.panic {
            panic!("a listener failed");
        }
        event.click.handled = s.handled;
        edit_click(event.click, s.take_clicked, s.set_clicked, s.set_carried);
        if s.cancel {
            event.cancelled = true;
            return Flow::Cancel;
        }
        Flow::Continue
    }
}

pub fn with_mod(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("stacked", Box::new(Listener))].into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_api::testing::engine_platform::with_mods(mods, test);
}

/// A local game with a player, stone 3 on the cursor and nothing in slot 9 (the first slot of
/// the main inventory, menu slot 9).
#[cfg(test)]
pub(in crate::game) fn with_local(test: impl FnOnce(&mut crate::game::ServerGame<'_>)) {
    with_guests(|game| {
        PLAYER.set(12);
        reset_local(game);
        test(game);
    });
}
#[cfg(test)]
pub(in crate::game) fn reset_local(game: &mut crate::game::ServerGame<'_>) {
    game.common.inventory = vec![None; game.inventory.len()];
    game.common.carried = Some(ItemStack::new("minecraft:stone", 3));
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |guest| {
            guest.inventory = vec![None; guest.inventory.len()];
            guest.carried = Some(ItemStack::new("minecraft:stone", 3));
            guest.menu = None;
        });
}
#[cfg(test)]
type SelectLocalFurnace = Box<dyn FnOnce(&mut crate::game::ServerGame<'_>)>;

#[cfg(test)]
fn with_local_furnace(
    pos: (i32, i32, i32),
    test: impl FnOnce(&mut crate::game::ServerGame<'_>, SelectLocalFurnace),
) {
    use sunrise_sim::lan::menu::Slots;

    with_local(|game| {
        let game_mode = if game.common.creative { 1 } else { 0 };
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        let inventory_menu = hub.guests[0].menu_before_replacement();
        let carried = hub.guests.update(0, |guest| {
            guest.game_mode = game_mode;
            // The local inventory and furnace share one cursor, so leave nothing to park.
            guest.carried.take()
        });
        hub.open_menu_window(
            12,
            "minecraft:furnace",
            sunrise_protocol::text::translatable("container.furnace", &[]),
            Some(pos),
            Slots::Block(pos),
            vec![None; 3],
            None,
        )
        .expect("local furnace window");
        let furnace_menu = hub.guests[0].menu_before_replacement();
        hub.guests.update(0, |guest| {
            inventory_menu(guest);
            guest.carried = carried;
        });
        test(
            game,
            Box::new(move |game| {
                game.common.container = Some(pos);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, furnace_menu);
            }),
        );
    });
}
#[cfg(test)]
fn clear_local_furnace(game: &mut crate::game::ServerGame<'_>) {
    game.common.container = None;
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |guest| {
            let menu = guest.menu.take().expect("selected local furnace");
            guest.carried = guest.close_menu_cursor(menu.window);
        });
}
pub fn count(stack: &Option<ItemStack>) -> u32 {
    stack.as_ref().map_or(0, |s| s.count)
}
/// The click: pickup on the first main-inventory slot. What is left: (slot 9, cursor, slot 20).
#[cfg(test)]
pub(in crate::game) fn click(game: &mut crate::game::ServerGame<'_>) -> (u32, u32, u32) {
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
    game.lan_guest_packet(
        12,
        Serverbound::ContainerClick(ContainerClick {
            container: 0,
            state: 0,
            slot: 9,
            button: 0,
            input: sunrise_sim::lan::menu::input::PICKUP,
            changed: Vec::new(),
            carried: None,
        }),
    );
    let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
    (
        count(&guest.inventory[9]),
        count(&guest.carried),
        count(&guest.inventory[20]),
    )
}
#[cfg(test)]
pub(in crate::game) fn local_menu_ops(
    game: &mut crate::game::ServerGame<'_>,
    ops: &[(Option<usize>, i32, Input)],
) -> bool {
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
    let inv = game.common.inventory.clone();
    let carried = game.common.carried.clone();
    let game_mode = if game.common.creative { 1 } else { 0 };
    let container_pos = game.common.container;
    let container = game
        .server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |guest| {
            let len = guest.inventory.len().min(inv.len());
            guest.inventory[..len].clone_from_slice(&inv[..len]);
            guest.game_mode = game_mode;
            guest.carried = carried;
            assert_eq!(guest.menu.as_ref().and_then(|menu| menu.pos), container_pos);
            guest.menu.as_ref().map_or(0, |menu| menu.window)
        });
    for &(slot, button, input) in ops {
        let input = match input {
            Input::Pickup => sunrise_sim::lan::menu::input::PICKUP,
            Input::QuickMove => sunrise_sim::lan::menu::input::QUICK_MOVE,
            Input::Swap => sunrise_sim::lan::menu::input::SWAP,
            Input::Clone => sunrise_sim::lan::menu::input::CLONE,
            Input::Throw => sunrise_sim::lan::menu::input::THROW,
            Input::QuickCraft => sunrise_sim::lan::menu::input::QUICK_CRAFT,
            Input::PickupAll => sunrise_sim::lan::menu::input::PICKUP_ALL,
        };
        let slot = match slot {
            None => -999,
            Some(s) if container != 0 && s >= 300 => (s - 300) as i32,
            Some(s) => s as i32,
        };
        game.lan_guest_packet(
            12,
            Serverbound::ContainerClick(ContainerClick {
                container,
                state: 0,
                slot: slot as i16,
                button: button as i8,
                input,
                changed: Vec::new(),
                carried: None,
            }),
        );
    }
    let (new_inv, new_carried) = {
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        (guest.inventory.clone(), guest.carried.clone())
    };
    let len = game.common.inventory.len().min(new_inv.len());
    game.common.inventory[..len].clone_from_slice(&new_inv[..len]);
    game.common.carried = new_carried;
    true
}

#[test]
#[ignore = "daily"]
fn an_accepted_pickup_is_one_dispatch_and_one_apply() {
    with_mod(|| {
        with_local(|game| {
            script(Script::default());
            // Row 72: stone 3 from the cursor into the empty slot; the listener saw the frozen
            // stacks (cursor 3, slot empty) once.
            assert_eq!(click(game), (3, 0, 0));
            assert_eq!(SEEN.with(|s| s.borrow().clone()), [(3, 0)]);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_failed_click_undoes_what_the_listener_wrote_and_applies_nothing() {
    with_mod(|| {
        with_local(|game| {
            // Row 73: the listener writes dirt 2 to an unrelated slot and then answers with a
            // stack on a click that is not cancelled and handled: invalid, so the whole action
            // is undone: slot 20 empty, the cursor still stone 3, the destination empty.
            script(Script {
                write_dirt: Some(20),
                set_carried: Some(Some(("minecraft:stone", 4))),
                ..Script::default()
            });
            assert_eq!(click(game), (0, 3, 0));
            assert_eq!(dispatches(), 1);
            // The next built-in click works.
            script(Script::default());
            assert_eq!(click(game), (3, 0, 0));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_write_to_what_the_click_read_makes_it_stale() {
    with_mod(|| {
        with_local(|game| {
            // Row 74: the listener changes the clicked slot itself (dirt 2 into slot 9) and
            // allows the click. Stale: slot 9 restored empty, the cursor stone 3, one dispatch.
            script(Script {
                write_dirt: Some(9),
                ..Script::default()
            });
            assert_eq!(click(game), (0, 3, 0));
            assert_eq!(dispatches(), 1);
            // Row 75: dirt in and out again within the listener restores what was there, and is
            // stale all the same, by the journaled-write epoch.
            reset_local(game);
            script(Script {
                write_dirt: Some(9),
                take_dirt_back: true,
                ..Script::default()
            });
            assert_eq!(click(game), (0, 3, 0));
            assert_eq!(dispatches(), 1);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_listener_that_panics_after_a_committed_write_is_undone_and_the_next_click_works() {
    with_mod(|| {
        with_local(|game| {
            // Row 79: the write committed in a nested transaction, then the listener traps:
            // the dirt is rolled back with the action.
            script(Script {
                write_dirt: Some(20),
                panic: true,
                ..Script::default()
            });
            assert_eq!(click(game), (0, 3, 0));
            assert!(game.transactions.is_empty());
            script(Script::default());
            assert_eq!(click(game), (3, 0, 0));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_cancelled_handled_click_sets_the_stacks_and_a_bad_stack_refuses_it() {
    with_mod(|| {
        with_local(|game| {
            // A valid override: cancelled and handled, the cursor emptied and the slot set.
            script(Script {
                cancel: true,
                handled: true,
                set_carried: Some(None),
                set_clicked: Some(Some(("minecraft:stone", 3))),
                ..Script::default()
            });
            assert_eq!(click(game), (3, 0, 0));
            // Row 89: a count past the item's maximum is invalid output: complete rollback.
            reset_local(game);
            script(Script {
                write_dirt: Some(20),
                cancel: true,
                handled: true,
                set_clicked: Some(Some(("minecraft:stone", 65))),
                ..Script::default()
            });
            assert_eq!(click(game), (0, 3, 0));
            // A cancelled click that is not handled runs the plain pickup.
            reset_local(game);
            script(Script {
                cancel: true,
                ..Script::default()
            });
            assert_eq!(click(game), (3, 0, 0));
            // The next built-in step works.
            reset_local(game);
            script(Script::default());
            assert_eq!(click(game), (3, 0, 0));
        });
    });
}

#[test]
#[ignore = "daily"]
fn an_outer_abort_after_an_applied_click_restores_the_inventory() {
    with_mod(|| {
        with_local(|game| {
            // Row 90: the click is Applied inside a transaction that then aborts.
            script(Script::default());
            game.storage_scope(|game, _| {
                assert_eq!(click(game), (3, 0, 0));
                ((), false)
            });
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!((count(&guest.inventory[9]), count(&guest.carried)), (0, 3));
        });
    });
}

#[test]
#[ignore = "daily"]
fn without_a_listener_the_click_is_plain_and_nothing_is_decided() {
    // Row 92: no mod at all.
    with_local(|game| {
        script(Script::default());
        let writes = game.transactions.writes();
        assert_eq!(click(game), (3, 0, 0));
        assert_eq!(dispatches(), 0);
        assert_eq!(game.transactions.writes(), writes);
        assert!(game.transactions.is_empty());
    });
    // A mod that does not listen to this hook behaves the same.
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}
    let mods: Mods = vec![ModEntry::new("quiet", Box::new(Quiet))].into();
    mod_platform::testing::with_mods(mods, || {
        with_local(|game| {
            assert_eq!(click(game), (3, 0, 0));
            assert_eq!(dispatches(), 0);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_lan_inventory_click_is_decided_the_same_way() {
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
    with_mod(|| {
        with_guests(|game| {
            PLAYER.set(12);
            let click = |game: &mut crate::game::ServerGame<'_>| {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.inventory = vec![None; guest.inventory.len()];
                        guest.carried = Some(ItemStack::new("minecraft:stone", 3));
                    });
                game.lan_guest_packet(
                    12,
                    Serverbound::ContainerClick(ContainerClick {
                        container: 0,
                        state: 0,
                        slot: 9,
                        button: 0,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    }),
                );
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                (
                    count(&guest.inventory[9]),
                    count(&guest.carried),
                    count(&guest.inventory[20]),
                )
            };
            script(Script::default());
            assert_eq!(click(game), (3, 0, 0));
            assert_eq!(SEEN.with(|s| s.borrow().clone()), [(3, 0)]);
            // A write to what the click read is stale.
            script(Script {
                write_dirt: Some(9),
                ..Script::default()
            });
            assert_eq!(click(game), (0, 3, 0));
            // A write to the slot the click read, then a panic: undone, nothing changes.
            // Run last because a panicking callback is retired for the rest of the session.
            script(Script {
                write_dirt: Some(20),
                panic: true,
                ..Script::default()
            });
            assert_eq!(click(game), (0, 3, 0));
            assert!(game.server.lan.is_some());
            script(Script::default());
            assert_eq!(click(game), (3, 0, 0));
        });
    });
}

/// The LAN menus the game runs for a guest: the hook fires on each, with the frozen stacks, and a
/// cancelled, handled answer that sets the cursor applies. Plain, the stone on the cursor would
/// have gone into the slot; set, the cursor ends as dirt 2.
#[cfg(test)]
pub(in crate::game) mod families {
    use super::*;
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};

    fn dirt_on_the_cursor() -> Script {
        Script {
            cancel: true,
            handled: true,
            set_carried: Some(Some(("minecraft:dirt", 2))),
            ..Script::default()
        }
    }

    fn cursor(game: &crate::game::ServerGame<'_>) -> Option<(String, u32)> {
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        guest
            .carried
            .as_ref()
            .map(|s| (s.item.to_string(), s.count))
    }

    fn packet(window: i32, slot: i16) -> Serverbound {
        Serverbound::ContainerClick(ContainerClick {
            container: window,
            state: 0,
            slot,
            button: 0,
            input: 0,
            changed: Vec::new(),
            carried: None,
        })
    }

    /// Opens the menu, then clicks a slot with stone 3 on the cursor: the hook fires once with the
    /// frozen cursor and the set cursor applies; then with a listener that only allows, the hook
    /// fires again and the plain click follows (`plain_cursor` is what it leaves on the cursor).
    fn family(
        open: impl FnOnce(&mut crate::game::ServerGame<'_>),
        click: impl Fn(&mut crate::game::ServerGame<'_>),
        plain_cursor: Option<(&str, u32)>,
    ) {
        with_mod(|| {
            with_guests(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                PLAYER.set(12);
                open(game);
                let give = |game: &mut crate::game::ServerGame<'_>| {
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.carried = Some(ItemStack::new("minecraft:stone", 3));
                        });
                };
                give(game);
                script(dirt_on_the_cursor());
                click(game);
                assert_eq!(dispatches(), 1, "the hook fires on this menu");
                assert_eq!(SEEN.with(|s| s.borrow()[0].0), 3, "with the frozen cursor");
                assert_eq!(cursor(game), Some(("minecraft:dirt".to_owned(), 2)));
                // A listener that only allows: asked again, and the plain click follows.
                give(game);
                script(Script::default());
                click(game);
                assert_eq!(dispatches(), 1);
                assert_eq!(
                    cursor(game),
                    plain_cursor.map(|(item, count)| (item.to_owned(), count))
                );
            });
        });
    }

    fn block(game: &mut crate::game::ServerGame<'_>, name: &str, pos: (i32, i32, i32)) {
        let state = game.pack.assets.registry.default_state(name).unwrap();
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn the_crafting_table_menu_decides_the_pickup() {
        let window = std::cell::Cell::new(0);
        family(
            |game| {
                block(game, "minecraft:crafting_table", (1, 64, 1));
                game.lan_open_crafting(12, [1, 64, 1]);
                let host = game.server.lan.as_ref().unwrap();
                window.set(host.menus[&12].crafting.table.as_ref().unwrap().0);
            },
            |game| game.lan_guest_packet(12, packet(window.get(), 1)),
            None,
        );
    }

    #[test]
    #[ignore = "daily"]
    fn the_menu_path_asks_rules_for_the_handled_replacement() {
        with_mod(|| {
            with_guests(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                block(game, "minecraft:crafting_table", (1, 64, 1));
                game.lan_open_crafting(12, [1, 64, 1]);
                let host = game.server.lan.as_mut().unwrap();
                let window = host.menus[&12].crafting.table.as_ref().unwrap().0;
                host.hub.guests.update(0, |guest| {
                    guest.carried = Some(ItemStack::new("minecraft:stone", 3))
                });
                for limit in [1, 2] {
                    script(Script {
                        cancel: true,
                        handled: true,
                        set_carried: Some(None),
                        set_clicked: Some(Some(("minecraft:dirt", 2))),
                        dirt_rule: Some((true, limit)),
                        ..Script::default()
                    });
                    ASKS.set(0);
                    game.lan_guest_packet(12, packet(window, 1));
                    let host = game.server.lan.as_ref().unwrap();
                    let menu = &host.menus[&12].crafting.table.as_ref().unwrap().2;
                    if limit == 1 {
                        assert_eq!(menu.slot(1), None);
                        assert_eq!(cursor(game), Some(("minecraft:stone".to_owned(), 3)));
                    } else {
                        assert_eq!(menu.slot(1), Some(&ItemStack::new("minecraft:dirt", 2)));
                        assert_eq!(cursor(game), None);
                    }
                    assert_eq!(ASKS.get(), 2);
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_furnace_menu_decides_the_pickup() {
        use sunrise_level::containers::{BlockEntity, Furnace, FurnaceKind};
        let window = std::cell::Cell::new(0);
        family(
            |game| {
                block(game, "minecraft:furnace", (1, 64, 1));
                game.world.block_entities.insert(
                    (1, 64, 1),
                    BlockEntity::Furnace(Furnace {
                        kind: FurnaceKind::Furnace,
                        items: [None, None, None],
                        lit_remaining: 0,
                        lit_total: 0,
                        cooking_timer: 0,
                        cooking_total: 200,
                        speed: 1.0,
                        recipes_used: Default::default(),
                    }),
                );
                game.open_block_menu(12, (1, 64, 1));
                let host = game.server.lan.as_ref().unwrap();
                window.set(host.hub.guests[0].menu.as_ref().unwrap().window);
            },
            |game| game.lan_guest_packet(12, packet(window.get(), 0)),
            None,
        );
    }

    /// A furnace at (1, 64, 1) holding `items` (input, fuel, output), open as guest 12's menu.
    fn open_furnace(game: &mut crate::game::ServerGame<'_>, items: [Option<ItemStack>; 3]) -> i32 {
        use sunrise_level::containers::{BlockEntity, Furnace, FurnaceKind};
        block(game, "minecraft:furnace", (1, 64, 1));
        game.world.block_entities.insert(
            (1, 64, 1),
            BlockEntity::Furnace(Furnace {
                kind: FurnaceKind::Furnace,
                items,
                lit_remaining: 0,
                lit_total: 0,
                cooking_timer: 0,
                cooking_total: 200,
                speed: 1.0,
                recipes_used: Default::default(),
            }),
        );
        game.open_block_menu(12, (1, 64, 1));
        game.server.lan.as_ref().unwrap().hub.guests[0]
            .menu
            .as_ref()
            .unwrap()
            .window
    }

    fn furnace_items(game: &crate::game::ServerGame<'_>) -> [Option<(String, u32)>; 3] {
        use sunrise_level::containers::BlockEntity;
        let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&(1, 64, 1)) else {
            panic!("the furnace is gone");
        };
        f.items
            .clone()
            .map(|s| s.map(|s| (s.item.to_string(), s.count)))
    }

    /// The furnace's write-back runs inside the click's action: an outer transaction that aborts
    /// puts back the furnace with the cursor, for a stack placed in it and for one taken out.
    #[test]
    #[ignore = "daily"]
    fn an_outer_abort_restores_the_furnace_with_the_cursor_for_place_and_take() {
        with_mod(|| {
            with_guests(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                PLAYER.set(12);
                let stone = |count| Some(ItemStack::new("minecraft:stone", count));
                let held = |game: &crate::game::ServerGame<'_>| cursor(game);
                // Place: stone 3 from the cursor into the input slot.
                let window = open_furnace(game, [None, None, None]);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = stone(3);
                    });
                script(Script::default());
                game.storage_scope(|game, _| {
                    game.lan_guest_packet(12, packet(window, 0));
                    assert_eq!(dispatches(), 1, "the click is decided");
                    assert_eq!(
                        furnace_items(game)[0],
                        Some(("minecraft:stone".to_owned(), 3))
                    );
                    assert_eq!(held(game), None);
                    ((), false)
                });
                assert_eq!(furnace_items(game), [None, None, None]);
                assert_eq!(held(game), Some(("minecraft:stone".to_owned(), 3)));
                // Take: the output slot's iron ingots onto an empty cursor.
                let window = open_furnace(
                    game,
                    [None, None, Some(ItemStack::new("minecraft:iron_ingot", 5))],
                );
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = None;
                    });
                script(Script::default());
                game.storage_scope(|game, _| {
                    game.lan_guest_packet(12, packet(window, 2));
                    assert_eq!(dispatches(), 1, "the click is decided");
                    assert_eq!(furnace_items(game)[2], None);
                    assert_eq!(held(game), Some(("minecraft:iron_ingot".to_owned(), 5)));
                    ((), false)
                });
                assert_eq!(
                    furnace_items(game),
                    [None, None, Some(("minecraft:iron_ingot".to_owned(), 5))]
                );
                assert_eq!(held(game), None);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_workstation_menu_decides_the_pickup() {
        use sunrise_gameplay_common::game::stations_common::{Kind, Station};
        let window = std::cell::Cell::new(0);
        family(
            |game| {
                block(game, "minecraft:stonecutter", (1, 64, 1));
                assert!(game.lan_open_station(12, &Station::new(Kind::Stonecutter, [1, 64, 1])));
                let host = game.server.lan.as_ref().unwrap();
                window.set(host.menus[&12].station.as_ref().unwrap().window);
            },
            |game| game.lan_guest_packet(12, packet(window.get(), 0)),
            None,
        );
    }

    #[test]
    #[ignore = "daily"]
    fn the_mount_menu_decides_the_pickup() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        let window = std::cell::Cell::new(0);
        family(
            |game| {
                let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
                let mut horse = Mob::new(60, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                horse.extra.tame = true;
                game.mobs.push(horse);
                game.lan_open_mount(12, 60);
                let host = game.server.lan.as_ref().unwrap();
                window.set(host.play[&12].mount_window.as_ref().unwrap().window);
            },
            |game| game.lan_guest_packet(12, packet(window.get(), 1)),
            // The stone is no horse armor: the slot refuses it and the cursor keeps it.
            Some(("minecraft:stone", 3)),
        );
    }

    #[test]
    #[ignore = "daily"]
    fn an_outer_abort_restores_the_mount_and_guest_without_publishing_the_click() {
        use crate::game::native_entity_refs::tests::run_fresh_cases;
        use std::sync::{Arc, mpsc};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::{buf::Reader, item::Stack, packets::play::Clientbound};
        use sunrise_sim::{
            lan::{Out, PlayerSender},
            mobs::Mob,
        };

        if run_fresh_cases(
            "game::stacked_on_other_tests::families::an_outer_abort_restores_the_mount_and_guest_without_publishing_the_click",
            "SUNRISE_TEST_MOUNT_MENU_TRANSACTION",
            &["mount-click"],
        ) {
            return;
        }
        with_mod(|| {
            with_guests(|game| {
                let (tx, rx) = mpsc::channel();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |g| {
                        g.sender = PlayerSender::new(tx, Arc::default());
                    })
                    .unwrap();
                let menu_packets = || -> Vec<_> {
                    rx.try_iter()
                        .flat_map(|out| match out {
                            Out::Packet(body) => vec![body],
                            Out::Packets(bodies) => bodies,
                            _ => Vec::new(),
                        })
                        .map(|body| {
                            let mut read = Reader::new(&body);
                            Clientbound::decode(read.varint().unwrap(), read.rest()).unwrap()
                        })
                        .filter(|p| {
                            matches!(
                                p,
                                Clientbound::ContainerSetContent(_)
                                    | Clientbound::ContainerSetSlot(_)
                                    | Clientbound::SetCursorItem(_)
                                    | Clientbound::SetPlayerInventory(_)
                            )
                        })
                        .collect()
                };
                let assert_snapshot = |window, slots: &[(usize, Option<(i32, i32)>)], carried| {
                    let packets = menu_packets();
                    let [Clientbound::ContainerSetContent(content)] = packets.as_slice() else {
                        panic!(
                            "expected one settled mount snapshot, got {} menu packets",
                            packets.len()
                        );
                    };
                    assert_eq!(content.container, window);
                    let stack = |s: &Option<Stack>| s.as_ref().map(|s| (s.item, s.count));
                    for &(slot, expected) in slots {
                        assert_eq!(stack(&content.items[slot]), expected, "menu slot {slot}");
                    }
                    assert_eq!(stack(&content.carried), carried);
                };
                let item_id = |name: &str| {
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .sync_data()
                        .builtin
                        .get("minecraft:item")
                        .unwrap()
                        .id(name)
                        .unwrap()
                };
                let (dirt_id, stone_id, emerald_id) = (
                    item_id("minecraft:dirt"),
                    item_id("minecraft:stone"),
                    item_id("minecraft:emerald"),
                );
                let mount = |game: &crate::game::ServerGame<'_>, id, name: &str| {
                    let kind = game.res.mob_types().get(name).unwrap().clone();
                    let mut mob = Mob::new(id, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                    mob.extra.tame = true;
                    mob
                };
                let open = |game: &mut crate::game::ServerGame<'_>, id| {
                    game.lan_open_mount(12, id);
                    let _ = menu_packets();
                    game.server.lan.as_ref().unwrap().play[&12]
                        .mount_window
                        .as_ref()
                        .unwrap()
                        .window
                };
                let guest_items = |game: &crate::game::ServerGame<'_>| {
                    let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
                    (g.inventory[9].clone(), g.carried.clone())
                };
                let dirt = Some(ItemStack::new("minecraft:dirt", 2));
                let stone = Some(ItemStack::new("minecraft:stone", 3));
                let mut donkey = mount(game, 60, "minecraft:donkey");
                donkey.extra.chested = true;
                donkey.extra.items = vec![dirt.clone()];
                game.mobs.push(donkey);
                let window = open(game, 60);
                game.world.modified.clear();
                game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.inventory[9] = None;
                    g.carried = stone.clone();
                });
                script(Script::default());
                game.storage_scope(|game, _| {
                    game.lan_guest_packet(12, packet(window, 2));
                    assert_eq!(game.mobs[0].extra.items[0], stone);
                    assert_eq!(guest_items(game), (None, dirt.clone()));
                    game.lan_guest_packet(12, packet(window, 17));
                    assert_eq!(guest_items(game), (dirt.clone(), None));
                    assert_eq!(SEEN.with(|seen| seen.borrow().clone()), [(3, 2), (2, 0)]);
                    assert!(!game.world.modified.contains(&(0, 0)));
                    assert!(menu_packets().is_empty(), "no tentative click reply");
                    ((), false)
                });
                assert_eq!(
                    game.mobs[0].extra.items.as_slice(),
                    std::slice::from_ref(&dirt)
                );
                assert_eq!(guest_items(game), (None, stone.clone()));
                assert!(!game.world.modified.contains(&(0, 0)));
                assert_snapshot(
                    window,
                    &[(2, Some((dirt_id, 2))), (17, None)],
                    Some((stone_id, 3)),
                );

                // A plain drag start joins the outer scope without dispatching a pickup.
                // After abort the next pickup must dispatch, not just reset a leaked drag.
                script(Script::default());
                game.storage_scope(|game, _| {
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state: 0,
                            slot: -999,
                            button: 0,
                            input: sunrise_sim::lan::menu::input::QUICK_CRAFT,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    assert_eq!(dispatches(), 0);
                    assert!(menu_packets().is_empty());
                    ((), false)
                });
                assert_snapshot(window, &[(2, Some((dirt_id, 2)))], Some((stone_id, 3)));

                script(Script {
                    take_clicked: Some(1),
                    ..Script::default()
                });
                game.storage_scope(|game, _| {
                    game.lan_guest_packet(12, packet(window, 2));
                    assert_eq!(dispatches(), 1, "an invalid answer was refused");
                    assert_eq!(
                        game.mobs[0].extra.items.as_slice(),
                        std::slice::from_ref(&dirt)
                    );
                    assert_eq!(guest_items(game), (None, stone.clone()));
                    assert!(
                        menu_packets().is_empty(),
                        "even a correction waits for settlement"
                    );
                    ((), true)
                });
                assert_snapshot(
                    window,
                    &[(2, Some((dirt_id, 2))), (17, None)],
                    Some((stone_id, 3)),
                );

                let tail = Some(ItemStack::new("minecraft:stick", 4));
                game.mobs[0].extra.items.resize(15, None);
                game.mobs[0].extra.items.push(tail.clone());
                script(Script {
                    cancel: true,
                    handled: true,
                    set_clicked: Some(Some(("minecraft:emerald", 1))),
                    set_carried: Some(None),
                    ..Script::default()
                });
                let emerald = Some(ItemStack::new("minecraft:emerald", 1));
                game.storage_scope(|game, _| {
                    game.lan_guest_packet(12, packet(window, 2));
                    assert_eq!(dispatches(), 1);
                    assert_eq!(game.mobs[0].extra.items[0], emerald);
                    assert_eq!(guest_items(game), (None, None));
                    assert!(menu_packets().is_empty());
                    ((), true)
                });
                assert_eq!(game.mobs[0].extra.items[0], emerald);
                assert_eq!(
                    game.mobs[0].extra.items[15], tail,
                    "the menu owns only its exposed slots"
                );
                assert!(game.world.modified.contains(&(0, 0)));
                assert_snapshot(window, &[(2, Some((emerald_id, 1))), (17, None)], None);

                let mut horse = mount(game, 61, "minecraft:horse");
                let mut saddle = ItemStack::new("minecraft:saddle", 1);
                saddle
                    .data_mut()
                    .enchantments
                    .push(("minecraft:unbreaking".into(), 2));
                let mut armor = ItemStack::new("minecraft:iron_horse_armor", 1);
                armor.damage = 7;
                armor
                    .data_mut()
                    .enchantments
                    .push(("minecraft:unbreaking".into(), 3));
                horse.set_saddle(Some(saddle.clone()));
                horse.extra.set_body_stack(Some(armor.clone()));
                game.mobs.push(horse);
                let window = open(game, 61);
                let equipment = |game: &crate::game::ServerGame<'_>| {
                    let horse = &game.mobs[1];
                    (horse.saddle_item(), horse.extra.body_stack().cloned())
                };
                for (slot, expected) in [(0, &saddle), (1, &armor)] {
                    script(Script::default());
                    game.storage_scope(|game, _| {
                        game.lan_guest_packet(12, packet(window, slot));
                        assert_eq!(dispatches(), 1);
                        assert_eq!(guest_items(game), (None, Some(expected.clone())));
                        let (worn_saddle, worn_armor) = equipment(game);
                        assert_eq!(worn_saddle.as_ref(), (slot != 0).then_some(&saddle));
                        assert_eq!(worn_armor.as_ref(), (slot != 1).then_some(&armor));
                        assert!(game.mobs[1].extra.items.is_empty(), "no chest slots");
                        assert!(menu_packets().is_empty());
                        ((), false)
                    });
                    assert_eq!(equipment(game), (Some(saddle.clone()), Some(armor.clone())));
                    assert_eq!(guest_items(game), (None, None));
                    assert_snapshot(window, &[], None);
                }

                // An answer that passes hook validation can still be refused by the slot.
                // Reopen the same mount with the same wire ID before settling the refusal:
                // the replacement generation must not receive a second window's reply.
                script(Script {
                    cancel: true,
                    handled: true,
                    set_clicked: Some(Some(("minecraft:stone", 1))),
                    ..Script::default()
                });
                game.world.modified.clear();
                game.storage_scope(|game, _| {
                    game.lan_guest_packet(12, packet(window, 1));
                    assert_eq!(dispatches(), 1);
                    assert_eq!(equipment(game), (Some(saddle.clone()), Some(armor.clone())));
                    assert_eq!(guest_items(game), (None, None));
                    assert!(menu_packets().is_empty());
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.next_window = window - 1;
                    });
                    assert_eq!(open(game, 61), window);
                    ((), true)
                });
                assert!(
                    menu_packets().is_empty(),
                    "no stale reply to the replacement window"
                );
                assert!(
                    !game.world.modified.contains(&(0, 0)),
                    "refusal does not mark the mount dirty"
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_trading_menu_decides_the_pickup() {
        use sunrise_gameplay_common::game::trading_common::MerchantState;
        use sunrise_sim::merchant::{Session, Trader};
        use {
            crate::game::server_side::native_merchant_refs::tests::spawn,
            sunrise_gameplay_common::game::server_side::native_merchant_refs::tests::offer,
        };
        let window = std::cell::Cell::new(0);
        family(
            |game| {
                spawn(game, 61, "minecraft:villager");
                game.mobs[0].extra.trade = Some(Box::new(Trader {
                    offers: vec![offer("minecraft:coal", "minecraft:diamond")],
                    ..Trader::default()
                }));
                game.mobs[0].extra.flag2 = true;
                let session =
                    Session::new(vec![offer("minecraft:coal", "minecraft:diamond")], true);
                game.lan_open_merchant(
                    12,
                    MerchantState {
                        session: Box::new(session),
                        scroll: 0,
                        dragging: false,
                        focused: None,
                        mob: Some(61),
                        name: None,
                    },
                );
                window.set(
                    game.server.lan.as_ref().unwrap().menus[&12]
                        .merchant
                        .as_ref()
                        .unwrap()
                        .window,
                );
            },
            |game| {
                let Serverbound::ContainerClick(click) = packet(window.get(), 0) else {
                    unreachable!()
                };
                assert!(game.lan_merchant_click(12, &click));
            },
            None,
        );
    }
}

#[test]
#[ignore = "daily"]
fn an_outer_abort_after_an_applied_lan_click_restores_the_guest_and_the_container() {
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
    use sunrise_sim::lan::menu::Slots;
    with_mod(|| {
        with_guests(|game| {
            PLAYER.set(12);
            script(Script::default());
            let pos = (1, 64, 1);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(pos.0, pos.1, pos.2, stone).unwrap();
            game.world.block_entities.insert(
                pos,
                sunrise_level::containers::BlockEntity::Chest(vec![None; 27]),
            );
            let host = game.server.lan.as_mut().unwrap();
            host.hub.open_container(
                12,
                "minecraft:generic_9x3",
                sunrise_core::nbt::Tag::String("test".into()),
                pos,
                Slots::Block(pos),
                vec![None; 27],
                None,
            );
            let window = host.hub.guests[0].menu.as_ref().unwrap().window;
            let packet = |container, slot| {
                Serverbound::ContainerClick(ContainerClick {
                    container,
                    state: 0,
                    slot,
                    button: 0,
                    input: 0,
                    changed: Vec::new(),
                    carried: None,
                })
            };
            let give = |game: &mut crate::game::ServerGame<'_>| {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.inventory = vec![None; guest.inventory.len()];
                        guest.carried = Some(ItemStack::new("minecraft:stone", 3));
                    });
            };
            let state = |game: &crate::game::ServerGame<'_>| {
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                (
                    game.world.block_entities.get(&pos).unwrap().slots()[0]
                        .as_ref()
                        .map_or(0, |s| s.count),
                    count(&guest.carried),
                    count(&guest.inventory[9]),
                )
            };
            // The hub's container click: the chest's slot and the cursor.
            give(game);
            game.storage_scope(|game, _| {
                game.lan_guest_packet(12, packet(window, 0));
                assert_eq!(state(game), (3, 0, 0));
                ((), false)
            });
            assert_eq!(state(game), (0, 3, 0));
            // The guest's inventory menu: close the chest before the window-0 click.
            game.guest_close_all(12);
            give(game);
            game.storage_scope(|game, _| {
                game.lan_guest_packet(12, packet(0, 9));
                assert_eq!(state(game), (0, 0, 3));
                ((), false)
            });
            assert_eq!(state(game), (0, 3, 0));
        });
    });
}

pub mod item_part_tests {
    use super::*;
    use mod_api::item::menu::StackedOnOtherDirection;
    use mod_api::{ItemPart, Registrar, RegistrationContext};

    #[derive(Clone, Default)]
    pub struct PartScript {
        pub cancel_carried: bool,
        pub cancel_clicked: bool,
        pub set_carried: Option<Option<(&'static str, u32)>>,
        pub set_clicked: Option<Option<(&'static str, u32)>>,
        pub take_clicked: Option<u32>,
        pub carried_phase_set_carried: Option<Option<(&'static str, u32)>>,
        pub carried_phase_set_clicked: Option<Option<(&'static str, u32)>>,
        pub carried_phase_take_clicked: Option<u32>,
        pub clicked_phase_set_carried: Option<Option<(&'static str, u32)>>,
        pub clicked_phase_set_clicked: Option<Option<(&'static str, u32)>>,
        pub clicked_phase_take_clicked: Option<u32>,
        pub panic_on_carried: bool,
        pub write_scoreboard: bool,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct PartObservation {
        pub tag: &'static str,
        pub direction: StackedOnOtherDirection,
        pub is_server: bool,
        pub primary: bool,
        pub carried: Option<(String, u32)>,
        pub clicked: Option<(String, u32)>,
    }

    thread_local! {
        pub(in crate::game) static PART_SCRIPT: RefCell<PartScript> = RefCell::new(PartScript::default());
        pub(in crate::game) static TRACE: RefCell<Vec<(&'static str, StackedOnOtherDirection)>> =
            const { RefCell::new(Vec::new()) };
        pub(in crate::game) static OBSERVATIONS: RefCell<Vec<PartObservation>> = const { RefCell::new(Vec::new()) };
        pub(in crate::game) static SMELTED: RefCell<Vec<(String, u32, u32)>> =
            const { RefCell::new(Vec::new()) };
    }

    pub fn set_part_script(script: PartScript) {
        TRACE.with(|t| t.borrow_mut().clear());
        OBSERVATIONS.with(|o| o.borrow_mut().clear());
        SMELTED.with(|s| s.borrow_mut().clear());
        PART_SCRIPT.with(|s| *s.borrow_mut() = script);
    }

    #[cfg(test)]
    pub(in crate::game) fn smelted_events() -> Vec<(String, u32, u32)> {
        SMELTED.with(|s| s.borrow().clone())
    }

    pub fn trace() -> Vec<(&'static str, StackedOnOtherDirection)> {
        TRACE.with(|t| t.borrow().clone())
    }

    pub fn observations() -> Vec<PartObservation> {
        OBSERVATIONS.with(|o| o.borrow().clone())
    }

    pub(in crate::game) struct TaggedPart(pub(in crate::game) &'static str);

    #[mod_api::sunrise_part]
    impl ItemPart for TaggedPart {
        fn item_stacked_on_other(
            &self,
            mut host: HostSide<'_>,
            event: &mut ItemStackedOnOtherEvent<'_, '_>,
        ) -> Flow {
            TRACE.with(|t| t.borrow_mut().push((self.0, event.click.direction)));
            let is_server = matches!(host, HostSide::Server(_));
            OBSERVATIONS.with(|o| {
                o.borrow_mut().push(PartObservation {
                    tag: self.0,
                    direction: event.click.direction,
                    is_server,
                    primary: event.click.action == mod_api::item::menu::ClickAction::Primary,
                    carried: event.click.carried().map(|s| {
                        (
                            mod_api::__private::item_key_id(s.item())
                                .unwrap_or_default()
                                .to_owned(),
                            s.count(),
                        )
                    }),
                    clicked: event.click.clicked().map(|s| {
                        (
                            mod_api::__private::item_key_id(s.item())
                                .unwrap_or_default()
                                .to_owned(),
                            s.count(),
                        )
                    }),
                });
            });
            let s = PART_SCRIPT.with(|s| s.borrow().clone());
            if s.write_scoreboard
                && let HostSide::Server(host) = &mut host
            {
                mod_api::prelude::ScoreboardServerHost::add_objective(
                    &mut **host,
                    "part_pickup",
                    "dummy",
                    mod_api::Text::literal("part_pickup"),
                    mod_api::ObjectiveRenderType::Integer,
                )
                .unwrap();
            }
            if s.panic_on_carried
                && event.click.direction == StackedOnOtherDirection::CarriedOnOther
            {
                panic!("item part panicked");
            }
            let (should_cancel, phase_carried, phase_clicked, phase_take) =
                match event.click.direction {
                    StackedOnOtherDirection::CarriedOnOther => (
                        s.cancel_carried,
                        s.carried_phase_set_carried.or(if s.cancel_carried {
                            s.set_carried
                        } else {
                            None
                        }),
                        s.carried_phase_set_clicked.or(if s.cancel_carried {
                            s.set_clicked
                        } else {
                            None
                        }),
                        s.carried_phase_take_clicked.or(if s.cancel_carried {
                            s.take_clicked
                        } else {
                            None
                        }),
                    ),
                    StackedOnOtherDirection::OtherStackedOnMe => (
                        s.cancel_clicked,
                        s.clicked_phase_set_carried.or(if s.cancel_clicked {
                            s.set_carried
                        } else {
                            None
                        }),
                        s.clicked_phase_set_clicked.or(if s.cancel_clicked {
                            s.set_clicked
                        } else {
                            None
                        }),
                        s.clicked_phase_take_clicked.or(if s.cancel_clicked {
                            s.take_clicked
                        } else {
                            None
                        }),
                    ),
                    StackedOnOtherDirection::Event => (false, None, None, None),
                };
            edit_click(event.click, phase_take, phase_clicked, phase_carried);
            if should_cancel {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    #[cfg(test)]
    pub(in crate::game) struct PartOnlyMod;
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl Hooks for PartOnlyMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let stick = r.item_key("minecraft:stick");
            let flint = r.item_key("minecraft:flint");
            r.item_part(stick, TaggedPart("stick"));
            r.item_part(flint, TaggedPart("flint"));
        }
    }

    #[cfg(test)]
    pub(in crate::game) struct PartWithFurnaceHooksMod;
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl Hooks for PartWithFurnaceHooksMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let stick = r.item_key("minecraft:stick");
            let flint = r.item_key("minecraft:flint");
            let ingot = r.item_key("minecraft:iron_ingot");
            r.item_part(stick, TaggedPart("stick"));
            r.item_part(flint, TaggedPart("flint"));
            r.item_part(ingot, TaggedPart("iron_ingot"));
        }
        fn item_smelted(&self, host: HostSide<'_>, event: &mut mod_api::ItemSmeltedEvent<'_>) {
            let HostSide::Server(host) = host else {
                panic!("hosted furnace clicks must dispatch on the server");
            };
            SMELTED.with(|s| {
                s.borrow_mut().push((
                    event.stack.item().id(host).unwrap().to_owned(),
                    event.stack.count(),
                    event.amount_removed,
                ));
            });
        }
    }

    #[cfg(test)]
    pub(in crate::game) fn with_part_and_furnace_hooks_mod(test: impl FnOnce()) {
        let mods: Mods = vec![ModEntry::new(
            "part_furnace",
            Box::new(PartWithFurnaceHooksMod),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, test);
    }

    pub struct GlobalAndPartMod(pub bool);
    #[mod_api::__subscriptions]
    impl Hooks for GlobalAndPartMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let stick = r.item_key("minecraft:stick");
            let flint = r.item_key("minecraft:flint");
            r.item_part(stick, TaggedPart("stick"));
            r.item_part(flint, TaggedPart("flint"));
        }
        fn item_stacked_on_other(
            &self,
            _: HostSide<'_>,
            event: &mut ItemStackedOnOtherEvent<'_, '_>,
        ) -> Flow {
            TRACE.with(|t| t.borrow_mut().push(("global", event.click.direction)));
            if self.0 {
                event.cancelled = true;
                return Flow::Cancel;
            }
            Flow::Continue
        }
    }

    #[cfg(test)]
    pub(in crate::game) fn with_part_only_mod(test: impl FnOnce()) {
        let mods: Mods = vec![ModEntry::new("part_stacked", Box::new(PartOnlyMod))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, test);
    }

    #[test]
    #[ignore = "daily"]
    fn item_part_overrides_run_in_vanilla_order_and_apply_transactionally() {
        with_part_only_mod(|| {
            with_local(|game| {
                // 1. Unrelated items pay no part dispatch and run plain pickup.
                set_part_script(PartScript::default());
                reset_local(game);
                assert_eq!(click(game), (3, 0, 0));
                assert!(trace().is_empty());

                // 2. CarriedOnOther runs first; cancelling skips OtherStackedOnMe and replaces stacks.
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:stick", 1))),
                    set_clicked: Some(Some(("minecraft:diamond", 2))),
                    ..PartScript::default()
                });
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    trace(),
                    [("stick", StackedOnOtherDirection::CarriedOnOther)]
                );
                assert_eq!(&*game.carried.as_ref().unwrap().item, "minecraft:stick");
                assert_eq!(
                    &*game.inventory[9].as_ref().unwrap().item,
                    "minecraft:diamond"
                );
                assert_eq!(game.inventory[9].as_ref().unwrap().count, 2);

                // 3. CarriedOnOther returning Continue defers to OtherStackedOnMe.
                set_part_script(PartScript {
                    cancel_clicked: true,
                    set_carried: Some(None),
                    set_clicked: Some(Some(("minecraft:flint", 4))),
                    ..PartScript::default()
                });
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    trace(),
                    [
                        ("stick", StackedOnOtherDirection::CarriedOnOther),
                        ("flint", StackedOnOtherDirection::OtherStackedOnMe),
                    ]
                );
                assert!(game.carried.is_none());
                assert_eq!(game.inventory[9].as_ref().unwrap().count, 4);

                // 4. Vanilla carried bundle takes a primary click on a non-empty slot BEFORE
                //    the clicked item's OtherStackedOnMe override.
                set_part_script(PartScript {
                    cancel_clicked: true,
                    set_clicked: Some(Some(("minecraft:diamond", 1))),
                    ..PartScript::default()
                });
                game.carried = Some(ItemStack::new("minecraft:bundle", 1));
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert!(
                    trace().is_empty(),
                    "vanilla carried bundle must win before clicked ItemPart"
                );
                assert!(game.inventory[9].is_none());

                // 5. Secondary click with carried vanilla bundle on a non-empty slot is NOT
                //    taken by vanilla BundleItem.overrideStackedOnOther, so clicked ItemPart runs.
                set_part_script(PartScript {
                    cancel_clicked: true,
                    set_clicked: Some(Some(("minecraft:diamond", 1))),
                    ..PartScript::default()
                });
                game.carried = Some(ItemStack::new("minecraft:bundle", 1));
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                assert!(local_menu_ops(game, &[(Some(9), 1, Input::Pickup)]));
                assert_eq!(
                    trace(),
                    [("flint", StackedOnOtherDirection::OtherStackedOnMe)]
                );
                assert_eq!(
                    &*game.inventory[9].as_ref().unwrap().item,
                    "minecraft:diamond"
                );

                // 6. Panic in an ItemPart rolls back host writes and leaves stacks unchanged.
                set_part_script(PartScript {
                    write_scoreboard: true,
                    panic_on_carried: true,
                    ..PartScript::default()
                });
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(&*game.carried.as_ref().unwrap().item, "minecraft:stick");
                assert_eq!(
                    &*game.inventory[9].as_ref().unwrap().item,
                    "minecraft:flint"
                );
                assert!(game.score.board.objective("part_pickup").is_none());
            });
        });

        // 7. Global cancel skips both ItemParts.
        let mods: Mods = vec![ModEntry::new(
            "global_cancel",
            Box::new(GlobalAndPartMod(true)),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_local(|game| {
                set_part_script(PartScript {
                    cancel_carried: true,
                    ..PartScript::default()
                });
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(trace(), [("global", StackedOnOtherDirection::Event)]);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_part_stacked_on_other_runs_on_lan_hub_and_inventory_clicks() {
        use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
        use sunrise_sim::lan::menu::Slots;
        with_part_only_mod(|| {
            with_guests(|game| {
                PLAYER.set(12);
                let pos = (1, 64, 1);
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, stone).unwrap();
                let mut chest = vec![None; 27];
                chest[0] = Some(ItemStack::new("minecraft:flint", 1));
                game.world.block_entities.insert(
                    pos,
                    sunrise_level::containers::BlockEntity::Chest(chest.clone()),
                );
                let host = game.server.lan.as_mut().unwrap();
                host.hub.open_container(
                    12,
                    "minecraft:generic_9x3",
                    sunrise_core::nbt::Tag::String("test".into()),
                    pos,
                    Slots::Block(pos),
                    chest,
                    None,
                );
                let window = host.hub.guests[0].menu.as_ref().unwrap().window;
                host.hub.guests.update(0, |guest| {
                    guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                });
                set_part_script(PartScript {
                    cancel_clicked: true,
                    set_carried: Some(None),
                    set_clicked: Some(Some(("minecraft:emerald", 3))),
                    ..PartScript::default()
                });
                let click_pkt = |container, slot| {
                    Serverbound::ContainerClick(ContainerClick {
                        container,
                        state: 0,
                        slot,
                        button: 0,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    })
                };
                // Hub container click fires CarriedOnOther then OtherStackedOnMe inside transaction.
                // Outer transaction abort restores both guest cursor and chest slot.
                game.storage_scope(|game, _| {
                    game.lan_guest_packet(12, click_pkt(window, 0));
                    assert_eq!(
                        trace(),
                        [
                            ("stick", StackedOnOtherDirection::CarriedOnOther),
                            ("flint", StackedOnOtherDirection::OtherStackedOnMe),
                        ]
                    );
                    assert!(
                        game.server.lan.as_ref().unwrap().hub.guests[0]
                            .carried
                            .is_none()
                    );
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[0]
                            .as_ref()
                            .map(|s| (&*s.item, s.count)),
                        Some(("minecraft:emerald", 3))
                    );
                    ((), false)
                });
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0]
                        .carried
                        .as_ref()
                        .map(|s| &*s.item),
                    Some("minecraft:stick")
                );
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[0]
                        .as_ref()
                        .map(|s| &*s.item),
                    Some("minecraft:flint")
                );

                // Guest inventory menu click also fires ItemPart::item_stacked_on_other.
                game.guest_close_all(12);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.inventory = vec![None; guest.inventory.len()];
                        guest.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                        guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                    });
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:diamond", 2))),
                    set_clicked: Some(None),
                    ..PartScript::default()
                });
                game.lan_guest_packet(12, click_pkt(0, 9));
                assert_eq!(
                    trace(),
                    [("stick", StackedOnOtherDirection::CarriedOnOther)]
                );
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(
                    guest.carried.as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:diamond", 2))
                );
                assert!(guest.inventory[9].is_none());
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_part_dispatches_on_resolved_pickup_and_ignores_other_drags() {
        use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
        use sunrise_sim::lan::menu::Slots;
        with_part_only_mod(|| {
            // 1. Local menu: 1-slot left and right drags dispatch Parts; multi-slot and
            //    middle-button drags do not.
            with_local(|game| {
                // 1-slot left drag (ty = 0) onto empty slot 9.
                game.creative = false;
                game.inventory[9] = None;
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(None),
                    set_clicked: Some(Some(("minecraft:diamond", 2))),
                    ..PartScript::default()
                });
                assert!(local_menu_ops(
                    game,
                    &[
                        (None, 0, Input::QuickCraft),
                        (Some(9), 1, Input::QuickCraft),
                        (None, 2, Input::QuickCraft),
                    ]
                ));
                assert_eq!(
                    trace(),
                    [("stick", StackedOnOtherDirection::CarriedOnOther)]
                );
                assert!(observations()[0].primary);
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:diamond", 2))
                );

                // 1-slot right drag (ty = 1) onto empty slot 10.
                game.inventory[10] = None;
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(None),
                    set_clicked: Some(Some(("minecraft:emerald", 1))),
                    ..PartScript::default()
                });
                assert!(local_menu_ops(
                    game,
                    &[
                        (None, 4, Input::QuickCraft),
                        (Some(10), 5, Input::QuickCraft),
                        (None, 6, Input::QuickCraft),
                    ]
                ));
                assert_eq!(
                    trace(),
                    [("stick", StackedOnOtherDirection::CarriedOnOther)]
                );
                assert!(!observations()[0].primary);
                assert_eq!(
                    game.inventory[10].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:emerald", 1))
                );

                // 2-slot left drag must NOT dispatch Parts.
                game.inventory[11] = None;
                game.inventory[12] = None;
                game.carried = Some(ItemStack::new("minecraft:stick", 2));
                set_part_script(PartScript {
                    cancel_carried: true,
                    ..PartScript::default()
                });
                assert!(local_menu_ops(
                    game,
                    &[
                        (None, 0, Input::QuickCraft),
                        (Some(11), 1, Input::QuickCraft),
                        (Some(12), 1, Input::QuickCraft),
                        (None, 2, Input::QuickCraft),
                    ]
                ));
                assert!(
                    trace().is_empty(),
                    "multi-slot drag must not dispatch Parts"
                );
                assert_eq!(game.inventory[11].as_ref().map(|s| s.count), Some(1));
                assert_eq!(game.inventory[12].as_ref().map(|s| s.count), Some(1));

                // 1-slot middle-button creative drag (ty = 2) must NOT dispatch Parts.
                game.creative = true;
                game.inventory[13] = None;
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    ..PartScript::default()
                });
                assert!(local_menu_ops(
                    game,
                    &[
                        (None, 8, Input::QuickCraft),
                        (Some(13), 9, Input::QuickCraft),
                        (None, 10, Input::QuickCraft),
                    ]
                ));
                assert!(
                    trace().is_empty(),
                    "middle-button drag must not dispatch Parts"
                );
            });

            // 2. LAN hub container and LAN inventory: 1-slot drag dispatches transactionally,
            //    direct Pickup during an active drag ONLY resets the drag without firing Parts,
            //    and multi-slot / middle-button drags do not dispatch.
            with_guests(|game| {
                PLAYER.set(12);
                let pos = (1, 64, 1);
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, stone).unwrap();
                game.world.block_entities.insert(
                    pos,
                    sunrise_level::containers::BlockEntity::Chest(vec![None; 27]),
                );
                let host = game.server.lan.as_mut().unwrap();
                host.hub.open_container(
                    12,
                    "minecraft:generic_9x3",
                    sunrise_core::nbt::Tag::String("test".into()),
                    pos,
                    Slots::Block(pos),
                    vec![None; 27],
                    None,
                );
                let window = host.hub.guests[0].menu.as_ref().unwrap().window;
                let pkt = |container, slot, button, input| {
                    Serverbound::ContainerClick(ContainerClick {
                        container,
                        state: 0,
                        slot,
                        button,
                        input,
                        changed: Vec::new(),
                        carried: None,
                    })
                };

                // LAN hub 1-slot left drag dispatches CarriedOnOther and rolls back on abort.
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                    });
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(None),
                    set_clicked: Some(Some(("minecraft:diamond", 3))),
                    ..PartScript::default()
                });
                game.storage_scope(|game, _| {
                    game.lan_guest_packet(12, pkt(window, -999, 0, 5));
                    game.lan_guest_packet(12, pkt(window, 0, 1, 5));
                    assert!(trace().is_empty());
                    game.lan_guest_packet(12, pkt(window, -999, 2, 5));
                    assert_eq!(
                        trace(),
                        [("stick", StackedOnOtherDirection::CarriedOnOther)]
                    );
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[0]
                            .as_ref()
                            .map(|s| (&*s.item, s.count)),
                        Some(("minecraft:diamond", 3))
                    );
                    ((), false)
                });
                assert!(game.world.block_entities.get(&pos).unwrap().slots()[0].is_none());
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0]
                        .carried
                        .as_ref()
                        .map(|s| &*s.item),
                    Some("minecraft:stick")
                );

                // LAN hub: direct PICKUP packet DURING an active drag ONLY resets the drag and
                // MUST NOT dispatch any Part or mutate slots.
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(None),
                    set_clicked: Some(Some(("minecraft:diamond", 1))),
                    ..PartScript::default()
                });
                game.lan_guest_packet(12, pkt(window, -999, 0, 5));
                game.lan_guest_packet(12, pkt(window, 0, 1, 5));
                game.lan_guest_packet(12, pkt(window, 0, 0, 0));
                assert!(
                    trace().is_empty(),
                    "direct pickup during active hub drag must only reset drag"
                );
                assert!(game.world.block_entities.get(&pos).unwrap().slots()[0].is_none());
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0]
                        .carried
                        .as_ref()
                        .map(|s| &*s.item),
                    Some("minecraft:stick")
                );
                // Drag was reset: a stray drag-end packet now does nothing either.
                game.lan_guest_packet(12, pkt(window, -999, 2, 5));
                assert!(trace().is_empty());

                // LAN hub: 2-slot drag and creative middle-button 1-slot drag do NOT dispatch.
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:stick", 2));
                        guest.game_mode = 1;
                    });
                set_part_script(PartScript {
                    cancel_carried: true,
                    ..PartScript::default()
                });
                game.lan_guest_packet(12, pkt(window, -999, 0, 5));
                game.lan_guest_packet(12, pkt(window, 1, 1, 5));
                game.lan_guest_packet(12, pkt(window, 2, 1, 5));
                game.lan_guest_packet(12, pkt(window, -999, 2, 5));
                assert!(
                    trace().is_empty(),
                    "hub 2-slot drag must not dispatch Parts"
                );
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                    });
                game.lan_guest_packet(12, pkt(window, -999, 8, 5));
                game.lan_guest_packet(12, pkt(window, 3, 9, 5));
                game.lan_guest_packet(12, pkt(window, -999, 10, 5));
                assert!(
                    trace().is_empty(),
                    "hub middle-button drag must not dispatch Parts"
                );

                // LAN inventory (window 0): 1-slot right drag dispatches; direct PICKUP during
                // active drag resets drag without dispatching.
                game.guest_close_all(12);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.game_mode = 0;
                        guest.inventory = vec![None; guest.inventory.len()];
                        guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                    });
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(None),
                    set_clicked: Some(Some(("minecraft:emerald", 2))),
                    ..PartScript::default()
                });
                game.lan_guest_packet(12, pkt(0, -999, 4, 5));
                game.lan_guest_packet(12, pkt(0, 9, 5, 5));
                game.lan_guest_packet(12, pkt(0, -999, 6, 5));
                assert_eq!(
                    trace(),
                    [("stick", StackedOnOtherDirection::CarriedOnOther)]
                );
                assert!(!observations()[0].primary);
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0].inventory[9]
                        .as_ref()
                        .map(|s| (&*s.item, s.count)),
                    Some(("minecraft:emerald", 2))
                );

                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.inventory[10] = None;
                        guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                    });
                set_part_script(PartScript {
                    cancel_carried: true,
                    ..PartScript::default()
                });
                game.lan_guest_packet(12, pkt(0, -999, 0, 5));
                game.lan_guest_packet(12, pkt(0, 10, 1, 5));
                game.lan_guest_packet(12, pkt(0, 10, 0, 0));
                assert!(
                    trace().is_empty(),
                    "direct pickup during active inventory drag must only reset drag"
                );
                assert!(game.server.lan.as_ref().unwrap().hub.guests[0].inventory[10].is_none());

                // 3. LAN merchant (villager trading) menu preserves drag state across packets:
                //    1-slot left/right drags dispatch ItemPart (and roll back on outer abort),
                //    a direct PICKUP during an active drag ONLY resets the drag without firing
                //    Parts or mutating slots, and a 2-slot drag does not dispatch Parts.
                {
                    use sunrise_gameplay_common::game::trading_common::MerchantState;
                    use sunrise_sim::merchant::{Session, Trader};
                    use {
                        crate::game::server_side::native_merchant_refs::tests::spawn,
                        sunrise_gameplay_common::game::server_side::native_merchant_refs::tests::offer,
                    };

                    spawn(game, 61, "minecraft:villager");
                    game.mobs[0].position = sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5);
                    game.mobs[0].extra.trade = Some(Box::new(Trader {
                        offers: vec![offer("minecraft:coal", "minecraft:diamond")],
                        ..Trader::default()
                    }));
                    game.mobs[0].extra.flag2 = true;
                    let session =
                        Session::new(vec![offer("minecraft:coal", "minecraft:diamond")], true);
                    game.lan_open_merchant(
                        12,
                        MerchantState {
                            session: Box::new(session),
                            scroll: 0,
                            dragging: false,
                            focused: None,
                            mob: Some(61),
                            name: None,
                        },
                    );
                    let mwin = game.server.lan.as_ref().unwrap().menus[&12]
                        .merchant
                        .as_ref()
                        .unwrap()
                        .window;

                    // 1-slot left drag on merchant slot 0 dispatches CarriedOnOther and rolls
                    // back slots, cursor, and drag state on outer abort.
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                        });
                    set_part_script(PartScript {
                        cancel_carried: true,
                        set_carried: Some(None),
                        set_clicked: Some(Some(("minecraft:diamond", 2))),
                        ..PartScript::default()
                    });
                    game.storage_scope(|game, _| {
                        game.lan_guest_packet(12, pkt(mwin, -999, 0, 5));
                        game.lan_guest_packet(12, pkt(mwin, 0, 1, 5));
                        assert!(trace().is_empty());
                        game.lan_guest_packet(12, pkt(mwin, -999, 2, 5));
                        assert_eq!(
                            trace(),
                            [("stick", StackedOnOtherDirection::CarriedOnOther)],
                            "LAN merchant 1-slot left drag must dispatch ItemPart"
                        );
                        assert!(observations()[0].primary);
                        assert_eq!(
                            game.server.lan.as_ref().unwrap().menus[&12]
                                .merchant
                                .as_ref()
                                .unwrap()
                                .slots[0]
                                .as_ref()
                                .map(|s| (&*s.item, s.count)),
                            Some(("minecraft:diamond", 2))
                        );
                        ((), false)
                    });
                    assert!(
                        game.server.lan.as_ref().unwrap().menus[&12]
                            .merchant
                            .as_ref()
                            .unwrap()
                            .slots[0]
                            .is_none()
                    );
                    assert_eq!(
                        game.server.lan.as_ref().unwrap().hub.guests[0]
                            .carried
                            .as_ref()
                            .map(|s| &*s.item),
                        Some("minecraft:stick")
                    );

                    // 1-slot right drag on merchant slot 1 dispatches with primary == false.
                    set_part_script(PartScript {
                        cancel_carried: true,
                        set_carried: Some(None),
                        set_clicked: Some(Some(("minecraft:emerald", 3))),
                        ..PartScript::default()
                    });
                    game.lan_guest_packet(12, pkt(mwin, -999, 4, 5));
                    game.lan_guest_packet(12, pkt(mwin, 1, 5, 5));
                    game.lan_guest_packet(12, pkt(mwin, -999, 6, 5));
                    assert_eq!(
                        trace(),
                        [("stick", StackedOnOtherDirection::CarriedOnOther)],
                        "LAN merchant 1-slot right drag must dispatch ItemPart"
                    );
                    assert!(!observations()[0].primary);
                    assert_eq!(
                        game.server.lan.as_ref().unwrap().menus[&12]
                            .merchant
                            .as_ref()
                            .unwrap()
                            .slots[1]
                            .as_ref()
                            .map(|s| (&*s.item, s.count)),
                        Some(("minecraft:emerald", 3))
                    );

                    // Direct PICKUP interrupting an active drag on the merchant window ONLY resets
                    // the drag: no Part fires, no slots change, and a stray drag-end does nothing.
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                        });
                    set_part_script(PartScript {
                        cancel_carried: true,
                        set_carried: Some(None),
                        set_clicked: Some(Some(("minecraft:diamond", 1))),
                        ..PartScript::default()
                    });
                    game.lan_guest_packet(12, pkt(mwin, -999, 0, 5));
                    game.lan_guest_packet(12, pkt(mwin, 0, 1, 5));
                    game.lan_guest_packet(12, pkt(mwin, 0, 0, 0));
                    assert!(
                        trace().is_empty(),
                        "direct pickup during active merchant drag must only reset drag"
                    );
                    assert!(
                        game.server.lan.as_ref().unwrap().menus[&12]
                            .merchant
                            .as_ref()
                            .unwrap()
                            .slots[0]
                            .is_none()
                    );
                    assert_eq!(
                        game.server.lan.as_ref().unwrap().hub.guests[0]
                            .carried
                            .as_ref()
                            .map(|s| &*s.item),
                        Some("minecraft:stick")
                    );
                    game.lan_guest_packet(12, pkt(mwin, -999, 2, 5));
                    assert!(
                        trace().is_empty(),
                        "stray drag-end after merchant drag reset must do nothing"
                    );

                    // 2-slot left drag on merchant payment slots 0 & 1 does NOT dispatch Parts.
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .menus
                        .get_mut(&12)
                        .unwrap()
                        .merchant
                        .as_mut()
                        .unwrap()
                        .slots = [None, None, None];
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.carried = Some(ItemStack::new("minecraft:stick", 2));
                        });
                    set_part_script(PartScript {
                        cancel_carried: true,
                        ..PartScript::default()
                    });
                    game.lan_guest_packet(12, pkt(mwin, -999, 0, 5));
                    game.lan_guest_packet(12, pkt(mwin, 0, 1, 5));
                    game.lan_guest_packet(12, pkt(mwin, 1, 1, 5));
                    game.lan_guest_packet(12, pkt(mwin, -999, 2, 5));
                    assert!(
                        trace().is_empty(),
                        "LAN merchant 2-slot drag must not dispatch Parts"
                    );
                    let mslots = &game.server.lan.as_ref().unwrap().menus[&12]
                        .merchant
                        .as_ref()
                        .unwrap()
                        .slots;
                    assert_eq!(mslots[0].as_ref().map(|s| s.count), Some(1));
                    assert_eq!(mslots[1].as_ref().map(|s| s.count), Some(1));
                }

                // 4. LAN mount menu (also rebuilt per packet) preserves drag state across packets.
                {
                    use sunrise_core::geometry::Vec3;
                    use sunrise_sim::mobs::Mob;

                    let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
                    let mut horse = Mob::new(62, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                    horse.extra.tame = true;
                    game.mobs.push(horse);
                    game.lan_open_mount(12, 62);
                    let mount_win = game.server.lan.as_ref().unwrap().play[&12]
                        .mount_window
                        .as_ref()
                        .unwrap()
                        .window;

                    // 1-slot left drag onto player inventory slot 2 of the mount menu dispatches.
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.inventory = vec![None; guest.inventory.len()];
                            guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                        });
                    set_part_script(PartScript {
                        cancel_carried: true,
                        set_carried: Some(None),
                        set_clicked: Some(Some(("minecraft:emerald", 2))),
                        ..PartScript::default()
                    });
                    game.lan_guest_packet(12, pkt(mount_win, -999, 0, 5));
                    game.lan_guest_packet(12, pkt(mount_win, 2, 1, 5));
                    game.lan_guest_packet(12, pkt(mount_win, -999, 2, 5));
                    assert_eq!(
                        trace(),
                        [("stick", StackedOnOtherDirection::CarriedOnOther)],
                        "LAN mount 1-slot left drag must dispatch ItemPart"
                    );
                    assert_eq!(
                        game.server.lan.as_ref().unwrap().hub.guests[0].inventory[9]
                            .as_ref()
                            .map(|s| (&*s.item, s.count)),
                        Some(("minecraft:emerald", 2))
                    );

                    // Direct PICKUP during active mount drag ONLY resets drag.
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.inventory[10] = None;
                            guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                        });
                    set_part_script(PartScript {
                        cancel_carried: true,
                        set_carried: Some(None),
                        set_clicked: Some(Some(("minecraft:diamond", 1))),
                        ..PartScript::default()
                    });
                    game.lan_guest_packet(12, pkt(mount_win, -999, 0, 5));
                    game.lan_guest_packet(12, pkt(mount_win, 3, 1, 5));
                    game.lan_guest_packet(12, pkt(mount_win, 3, 0, 0));
                    assert!(
                        trace().is_empty(),
                        "direct pickup during active mount drag must only reset drag"
                    );
                    assert!(
                        game.server.lan.as_ref().unwrap().hub.guests[0].inventory[10].is_none()
                    );
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn hosted_furnace_counts_transfers_separately_from_smelted_callbacks() {
        sunrise_gameplay_common::game::in_own_process(
            "game::stacked_on_other_tests::item_part_tests::hosted_furnace_counts_transfers_separately_from_smelted_callbacks",
            || {
                use sunrise_level::containers::{BlockEntity, Furnace, FurnaceKind};
                use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
                use sunrise_sim::lan::menu::input;
                with_part_and_furnace_hooks_mod(|| {
                    with_guests(|game| {
                        let pos = (1, 64, 1);
                        let ingot = "minecraft:iron_ingot";
                        game.world.chunks.insert(
                            (0, 0),
                            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                        );
                        let furnace = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:furnace")
                            .unwrap();
                        game.world.set(pos.0, pos.1, pos.2, furnace).unwrap();
                        game.world.block_entities.insert(
                            pos,
                            BlockEntity::Furnace(Furnace {
                                kind: FurnaceKind::Furnace,
                                items: [None, None, Some(ItemStack::new(ingot, 4))],
                                lit_remaining: 0,
                                lit_total: 0,
                                cooking_timer: 0,
                                cooking_total: 0,
                                speed: 1.0,
                                recipes_used: Default::default(),
                            }),
                        );
                        assert!(matches!(
                            game.open_block_menu(12, pos),
                            sunrise_gameplay_common::game::lan_actions::MenuOpen::Opened
                        ));
                        let window = game.server.lan.as_ref().unwrap().hub.guests[0]
                            .menu
                            .as_ref()
                            .unwrap()
                            .window;
                        let click = |input| {
                            Serverbound::ContainerClick(ContainerClick {
                                container: window,
                                state: 0,
                                slot: 2,
                                button: 0,
                                input,
                                changed: Vec::new(),
                                carried: None,
                            })
                        };
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |guest| {
                                guest
                                    .inventory
                                    .fill(Some(ItemStack::new("minecraft:dirt", 64)));
                                guest.inventory[0] = Some(ItemStack::new(ingot, 62));
                            });
                        set_part_script(PartScript::default());
                        game.lan_guest_packet(12, click(input::QUICK_MOVE));
                        assert_eq!(
                            game.world.block_entities.get(&pos).unwrap().slots()[2],
                            Some(ItemStack::new(ingot, 2))
                        );
                        assert_eq!(
                            game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0],
                            Some(ItemStack::new(ingot, 64))
                        );
                        assert_eq!(smelted_events(), [(ingot.to_owned(), 4, 2)]);

                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |guest| {
                                guest.inventory[0] = None;
                            });
                        set_part_script(PartScript::default());
                        game.lan_guest_packet(12, click(input::SWAP));
                        assert_eq!(
                            game.world.block_entities.get(&pos).unwrap().slots()[2],
                            None
                        );
                        assert_eq!(
                            game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0],
                            Some(ItemStack::new(ingot, 2))
                        );
                        assert!(smelted_events().is_empty());

                        let BlockEntity::Furnace(furnace) =
                            game.world.block_entities.get_mut(&pos).unwrap()
                        else {
                            unreachable!()
                        };
                        furnace.items[2] = Some(ItemStack::new(ingot, 4));
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |guest| {
                                guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                            });
                        set_part_script(PartScript {
                            carried_phase_take_clicked: Some(1),
                            cancel_clicked: true,
                            clicked_phase_take_clicked: Some(1),
                            ..PartScript::default()
                        });
                        game.lan_guest_packet(12, click(input::PICKUP));
                        assert_eq!(
                            trace(),
                            [
                                ("stick", StackedOnOtherDirection::CarriedOnOther),
                                ("iron_ingot", StackedOnOtherDirection::OtherStackedOnMe),
                            ]
                        );
                        assert_eq!(
                            game.world.block_entities.get(&pos).unwrap().slots()[2],
                            Some(ItemStack::new(ingot, 2))
                        );
                        assert_eq!(
                            smelted_events(),
                            [(ingot.to_owned(), 1, 1), (ingot.to_owned(), 1, 1),]
                        );
                    });
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn item_part_override_contract_supports_decline_edits_and_output_slots() {
        use sunrise_level::containers::{BlockEntity, Furnace, FurnaceKind};
        with_part_only_mod(|| {
            with_local_furnace((1, 64, 1), |game, select_furnace| {
                // A declining part passes its edits to the next part and then to plain pickup.
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    carried_phase_set_carried: Some(Some(("minecraft:stick", 2))),
                    carried_phase_set_clicked: Some(Some(("minecraft:flint", 3))),
                    clicked_phase_set_clicked: Some(Some(("minecraft:flint", 5))),
                    ..PartScript::default()
                });
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    trace(),
                    [
                        ("stick", StackedOnOtherDirection::CarriedOnOther),
                        ("flint", StackedOnOtherDirection::OtherStackedOnMe),
                    ]
                );
                // OtherStackedOnMe saw the stacks edited by the declining CarriedOnOther override.
                assert_eq!(
                    observations()[1].carried,
                    Some(("minecraft:stick".to_owned(), 2))
                );
                assert_eq!(
                    observations()[1].clicked,
                    Some(("minecraft:flint".to_owned(), 3))
                );
                // Plain pickup then swapped the final edited stacks (stick x2 <-> flint x5).
                assert_eq!(
                    game.carried.as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:flint", 5))
                );
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:stick", 2))
                );

                // 2. A Part answers with its `Flow` alone: cancelling with no edits takes the click (no plain pickup), and what it wrote through the host stays.
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    write_scoreboard: true,
                    ..PartScript::default()
                });
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    game.carried.as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:stick", 1))
                );
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:flint", 1))
                );
                assert!(game.score.board.objective("part_pickup").is_some());
                game.score.board.remove_objective("part_pickup");

                // 3. Handled direct edit and full take on a Furnace output slot (`Role::Output`)
                //    succeed inside the transaction and roll back on outer abort.
                let pos = (1, 64, 1);
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let furnace = Furnace {
                    kind: FurnaceKind::Furnace,
                    items: [None, None, Some(ItemStack::new("minecraft:flint", 4))],
                    lit_remaining: 0,
                    lit_total: 0,
                    cooking_timer: 0,
                    cooking_total: 0,
                    speed: 1.0,
                    recipes_used: std::collections::HashMap::new(),
                };
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Furnace(furnace));
                select_furnace(game);
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:stick", 2))),
                    set_clicked: Some(Some(("minecraft:diamond", 2))),
                    ..PartScript::default()
                });
                game.storage_scope(|game, _| {
                    assert!(local_menu_ops(game, &[(Some(302), 0, Input::Pickup)]));
                    assert_eq!(
                        game.carried.as_ref().map(|s| (&*s.item, s.count)),
                        Some(("minecraft:stick", 2))
                    );
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[2]
                            .as_ref()
                            .map(|s| (&*s.item, s.count)),
                        Some(("minecraft:diamond", 2))
                    );
                    ((), false)
                });
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[2]
                        .as_ref()
                        .map(|s| (&*s.item, s.count)),
                    Some(("minecraft:flint", 4))
                );

                // On a local furnace output slot, set_clicked clears or reduces the stack;
                // (`Set(None)`) or count reduction (`Set(Some(...))`) writes the slot directly
                // with NO XP orbs and preserves `recipes_used`, while `Take(4)` and `Take(2)` run
                // `safe_take` / `on_take`, spawning XP orbs and clearing `recipes_used`.
                let recipe = "minecraft:iron_ingot_from_smelting_raw_iron";
                assert!(
                    game.res.recipes().cooking_experience(recipe) > 0.0,
                    "expected positive smelting experience for {recipe}"
                );
                let seed_furnace = |game: &mut crate::game::ServerGame<'_>,
                                    pos: (i32, i32, i32)| {
                    let mut recipes_used = std::collections::HashMap::new();
                    recipes_used.insert(recipe.to_owned(), 10);
                    game.world.block_entities.insert(
                        pos,
                        BlockEntity::Furnace(Furnace {
                            kind: FurnaceKind::Furnace,
                            items: [None, None, Some(ItemStack::new("minecraft:iron_ingot", 4))],
                            lit_remaining: 0,
                            lit_total: 0,
                            cooking_timer: 0,
                            cooking_total: 0,
                            speed: 1.0,
                            recipes_used,
                        }),
                    );
                };

                // Local direct clear (`Set(None)`): empties output with NO XP and preserves recipes_used.
                seed_furnace(game, pos);
                game.orbs.clear();
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:stick", 2))),
                    set_clicked: Some(None),
                    ..PartScript::default()
                });
                assert!(local_menu_ops(game, &[(Some(302), 0, Input::Pickup)]));
                assert_eq!(
                    game.carried.as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:stick", 2))
                );
                assert!(game.world.block_entities.get(&pos).unwrap().slots()[2].is_none());
                assert!(
                    game.orbs.is_empty(),
                    "local furnace direct clear Set(None) must spawn no XP orbs"
                );
                let BlockEntity::Furnace(f) = game.world.block_entities.get(&pos).unwrap() else {
                    panic!("expected furnace");
                };
                assert_eq!(
                    f.recipes_used.get(recipe),
                    Some(&10),
                    "local furnace direct clear Set(None) must preserve recipes_used"
                );

                // Local direct count reduction (`Set(Some(iron_ingot x2))`): NO XP and preserves recipes_used.
                seed_furnace(game, pos);
                game.orbs.clear();
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:stick", 3))),
                    set_clicked: Some(Some(("minecraft:iron_ingot", 2))),
                    ..PartScript::default()
                });
                assert!(local_menu_ops(game, &[(Some(302), 0, Input::Pickup)]));
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[2]
                        .as_ref()
                        .map(|s| (&*s.item, s.count)),
                    Some(("minecraft:iron_ingot", 2))
                );
                assert!(
                    game.orbs.is_empty(),
                    "local furnace direct Set(Some) must spawn no XP orbs"
                );
                let BlockEntity::Furnace(f) = game.world.block_entities.get(&pos).unwrap() else {
                    panic!("expected furnace");
                };
                assert_eq!(f.recipes_used.get(recipe), Some(&10));

                // Local explicit full take (`Take(4)`): runs `safe_take` + `on_take`, spawns XP & clears recipes_used.
                seed_furnace(game, pos);
                game.orbs.clear();
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:stick", 5))),
                    take_clicked: Some(4),
                    ..PartScript::default()
                });
                assert!(local_menu_ops(game, &[(Some(302), 0, Input::Pickup)]));
                assert_eq!(
                    game.carried.as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:stick", 5))
                );
                assert!(game.world.block_entities.get(&pos).unwrap().slots()[2].is_none());
                assert_eq!(
                    game.orbs.iter().map(|o| o.points()).sum::<u32>(),
                    7,
                    "local furnace explicit Take(4) must pop smelting XP orbs (10 * 0.7 = 7)"
                );
                let BlockEntity::Furnace(f) = game.world.block_entities.get(&pos).unwrap() else {
                    panic!("expected furnace");
                };
                assert!(f.recipes_used.is_empty());

                // Local explicit partial take (`Take(2)`): runs `remove` + `on_take`, spawns XP & clears recipes_used.
                seed_furnace(game, pos);
                game.orbs.clear();
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:stick", 3))),
                    take_clicked: Some(2),
                    ..PartScript::default()
                });
                assert!(local_menu_ops(game, &[(Some(302), 0, Input::Pickup)]));
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[2]
                        .as_ref()
                        .map(|s| (&*s.item, s.count)),
                    Some(("minecraft:iron_ingot", 2))
                );
                assert_eq!(
                    game.orbs.iter().map(|o| o.points()).sum::<u32>(),
                    7,
                    "local furnace explicit Take(2) must pop smelting XP orbs"
                );
                let BlockEntity::Furnace(f) = game.world.block_entities.get(&pos).unwrap() else {
                    panic!("expected furnace");
                };
                assert!(f.recipes_used.is_empty());

                // A take of 0 records nothing: the cancelled click takes nothing from the slot, and the Part's host write stays.
                seed_furnace(game, pos);
                game.orbs.clear();
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    take_clicked: Some(0),
                    write_scoreboard: true,
                    ..PartScript::default()
                });
                assert!(local_menu_ops(game, &[(Some(302), 0, Input::Pickup)]));
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[2]
                        .as_ref()
                        .map(|s| (&*s.item, s.count)),
                    Some(("minecraft:iron_ingot", 4))
                );
                assert!(game.orbs.is_empty());
                assert!(game.score.board.objective("part_pickup").is_some());
                game.score.board.remove_objective("part_pickup");

                // A take of 5 from 4 takes the 4 the slot holds (`Slot.safeTake` takes at most what is there), with the take's smelting experience.
                seed_furnace(game, pos);
                game.orbs.clear();
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    take_clicked: Some(5),
                    ..PartScript::default()
                });
                assert!(local_menu_ops(game, &[(Some(302), 0, Input::Pickup)]));
                assert!(game.world.block_entities.get(&pos).unwrap().slots()[2].is_none());
                assert_eq!(game.orbs.iter().map(|o| o.points()).sum::<u32>(), 7);
                clear_local_furnace(game);
            });
        });

        // LAN guest output edits skip experience and item_smelted; explicit takes publish on commit.
        with_part_and_furnace_hooks_mod(|| {
            use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
            with_guests(|game| {
                PLAYER.set(12);
                let pos = (1, 64, 1);
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let furnace_state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:furnace")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, furnace_state).unwrap();
                let recipe = "minecraft:iron_ingot_from_smelting_raw_iron";
                let seed_furnace = |game: &mut crate::game::ServerGame<'_>| {
                    let mut recipes_used = std::collections::HashMap::new();
                    recipes_used.insert(recipe.to_owned(), 10);
                    game.world.block_entities.insert(
                        pos,
                        BlockEntity::Furnace(Furnace {
                            kind: FurnaceKind::Furnace,
                            items: [None, None, Some(ItemStack::new("minecraft:iron_ingot", 4))],
                            lit_remaining: 0,
                            lit_total: 0,
                            cooking_timer: 0,
                            cooking_total: 0,
                            speed: 1.0,
                            recipes_used,
                        }),
                    );
                };
                seed_furnace(game);
                assert!(matches!(
                    game.open_block_menu(12, pos),
                    sunrise_gameplay_common::game::lan_actions::MenuOpen::Opened
                ));
                let window = game.server.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .unwrap()
                    .window;
                let click_output = Serverbound::ContainerClick(ContainerClick {
                    container: window,
                    state: 0,
                    slot: 2,
                    button: 0,
                    input: 0,
                    changed: Vec::new(),
                    carried: None,
                });

                // LAN guest direct clear (`Set(None)`): clears output with NO XP, preserves
                // `recipes_used`, and fires no `item_smelted`.
                game.orbs.clear();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                    });
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:stick", 2))),
                    set_clicked: Some(None),
                    ..PartScript::default()
                });
                game.lan_guest_packet(12, click_output.clone());
                assert_eq!(
                    trace(),
                    [("stick", StackedOnOtherDirection::CarriedOnOther)]
                );
                assert!(game.world.block_entities.get(&pos).unwrap().slots()[2].is_none());
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0]
                        .carried
                        .as_ref()
                        .map(|s| (&*s.item, s.count)),
                    Some(("minecraft:stick", 2))
                );
                assert!(
                    game.orbs.is_empty(),
                    "LAN guest direct clear Set(None) must spawn no XP orbs"
                );
                let BlockEntity::Furnace(f) = game.world.block_entities.get(&pos).unwrap() else {
                    panic!("expected furnace");
                };
                assert_eq!(
                    f.recipes_used.get(recipe),
                    Some(&10),
                    "LAN guest direct clear Set(None) must preserve recipes_used"
                );
                assert!(
                    smelted_events().is_empty(),
                    "LAN guest direct clear Set(None) must not fire item_smelted"
                );

                // LAN guest direct count reduction (`Set(Some(iron_ingot x2))`): NO XP, NO hooks.
                seed_furnace(game);
                game.orbs.clear();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                    });
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:stick", 3))),
                    set_clicked: Some(Some(("minecraft:iron_ingot", 2))),
                    ..PartScript::default()
                });
                game.lan_guest_packet(12, click_output.clone());
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[2]
                        .as_ref()
                        .map(|s| (&*s.item, s.count)),
                    Some(("minecraft:iron_ingot", 2))
                );
                assert!(game.orbs.is_empty());
                let BlockEntity::Furnace(f) = game.world.block_entities.get(&pos).unwrap() else {
                    panic!("expected furnace");
                };
                assert_eq!(f.recipes_used.get(recipe), Some(&10));
                assert!(smelted_events().is_empty());

                // LAN guest explicit full take (`Take(4)`).
                seed_furnace(game);
                game.orbs.clear();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                    });
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:stick", 5))),
                    take_clicked: Some(4),
                    ..PartScript::default()
                });
                game.lan_guest_packet(12, click_output.clone());
                assert_eq!(
                    trace(),
                    [("stick", StackedOnOtherDirection::CarriedOnOther)]
                );
                assert!(game.world.block_entities.get(&pos).unwrap().slots()[2].is_none());
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0]
                        .carried
                        .as_ref()
                        .map(|s| (&*s.item, s.count)),
                    Some(("minecraft:stick", 5))
                );
                assert_eq!(
                    game.orbs.iter().map(|o| o.points()).sum::<u32>(),
                    7,
                    "LAN guest furnace explicit Take(4) must spawn smelting XP orbs"
                );
                let BlockEntity::Furnace(f) = game.world.block_entities.get(&pos).unwrap() else {
                    panic!("expected furnace");
                };
                assert!(f.recipes_used.is_empty());
                assert_eq!(
                    smelted_events(),
                    [("minecraft:iron_ingot".to_owned(), 4, 4)],
                    "LAN guest explicit Take(4) must run FurnaceResultSlot.onTake -> item_smelted"
                );

                // LAN guest explicit partial take (`Take(2)`).
                seed_furnace(game);
                game.orbs.clear();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                    });
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:stick", 3))),
                    take_clicked: Some(2),
                    ..PartScript::default()
                });
                game.lan_guest_packet(12, click_output.clone());
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[2]
                        .as_ref()
                        .map(|s| (&*s.item, s.count)),
                    Some(("minecraft:iron_ingot", 2))
                );
                assert_eq!(
                    game.orbs.iter().map(|o| o.points()).sum::<u32>(),
                    7,
                    "LAN guest furnace explicit Take(2) must spawn smelting XP orbs"
                );
                let BlockEntity::Furnace(f) = game.world.block_entities.get(&pos).unwrap() else {
                    panic!("expected furnace");
                };
                assert!(f.recipes_used.is_empty());
                assert_eq!(
                    smelted_events(),
                    [("minecraft:iron_ingot".to_owned(), 2, 2)],
                    "LAN guest explicit Take(2) must run remove + on_take -> item_smelted"
                );

                // Outer transaction abort on LAN guest furnace `Take(4)` restores furnace output
                // and recipes_used without popping XP orbs or firing post-commit furnace hooks.
                seed_furnace(game);
                game.orbs.clear();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:stick", 1));
                    });
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(Some(("minecraft:stick", 5))),
                    take_clicked: Some(4),
                    ..PartScript::default()
                });
                game.storage_scope(|game, _| {
                    game.lan_guest_packet(12, click_output);
                    assert!(game.world.block_entities.get(&pos).unwrap().slots()[2].is_none());
                    assert!(game.orbs.is_empty(), "XP orb award waits for root commit");
                    ((), false)
                });
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[2]
                        .as_ref()
                        .map(|s| (&*s.item, s.count)),
                    Some(("minecraft:iron_ingot", 4))
                );
                let BlockEntity::Furnace(f) = game.world.block_entities.get(&pos).unwrap() else {
                    panic!("expected furnace");
                };
                assert_eq!(f.recipes_used.get(recipe), Some(&10));
                assert!(game.orbs.is_empty());
                assert!(smelted_events().is_empty());
            });
        });

        // 4. Documented global listener contract is preserved: setting a stack without canceling,
        //    directly editing a Furnace output slot, or taking from the slot are
        //    ALL refused for Hooks::item_stacked_on_other.
        with_mod(|| {
            with_local_furnace((2, 64, 2), |game, select_furnace| {
                let pos = (2, 64, 2);
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                game.world.block_entities.insert(
                    pos,
                    BlockEntity::Furnace(Furnace {
                        kind: FurnaceKind::Furnace,
                        items: [None, None, Some(ItemStack::new("minecraft:flint", 4))],
                        lit_remaining: 0,
                        lit_total: 0,
                        cooking_timer: 0,
                        cooking_total: 0,
                        speed: 1.0,
                        recipes_used: std::collections::HashMap::new(),
                    }),
                );
                select_furnace(game);
                game.carried = Some(ItemStack::new("minecraft:stone", 1));
                for bad in [
                    Script {
                        cancel: true,
                        handled: true,
                        set_clicked: Some(Some(("minecraft:diamond", 2))),
                        ..Script::default()
                    },
                    Script {
                        cancel: true,
                        handled: true,
                        set_clicked: Some(None),
                        ..Script::default()
                    },
                    Script {
                        cancel: true,
                        handled: true,
                        take_clicked: Some(4),
                        ..Script::default()
                    },
                ] {
                    script(bad);
                    assert!(local_menu_ops(game, &[(Some(302), 0, Input::Pickup)]));
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[2]
                            .as_ref()
                            .map(|s| (&*s.item, s.count)),
                        Some(("minecraft:flint", 4)),
                        "global listener output-slot write/take must remain refused"
                    );
                }
                clear_local_furnace(game);
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 4));
                game.carried = Some(ItemStack::new("minecraft:stone", 1));
                script(Script {
                    cancel: true,
                    handled: true,
                    take_clicked: Some(2),
                    ..Script::default()
                });
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:flint", 4)),
                    "global listener must refuse a take even on ordinary slots"
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_part_uses_real_carried_override_outcome_for_non_bundle_items() {
        use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
        use sunrise_sim::lan::menu::Slots;
        let paper_with_bundle_contents = || {
            let mut paper = ItemStack::new("minecraft:paper", 1);
            paper.data_mut().bundle_contents = vec![ItemStack::new("minecraft:stone", 1)];
            paper
        };
        let bundle_without_contents = || {
            let mut bundle = ItemStack::new("minecraft:bundle", 1);
            bundle.data_mut().set_other(
                "!minecraft:bundle_contents",
                Some(sunrise_core::nbt::Tag::Compound(Default::default())),
            );
            bundle
        };
        let install_default_bundle_contents_non_bundle_items =
            |game: &mut crate::game::ServerGame<'_>| {
                let mut items = game.items().clone();
                // Simulate mod/custom non-BundleItem items whose default component prototype
                // includes `minecraft:bundle_contents` (`info.bundle == true`) without a
                // `BundleItem` class or template.
                for id in ["minecraft:stick", "minecraft:flint", "minecraft:paper"] {
                    let info = items.items.get_mut(id).unwrap();
                    info.bundle = true;
                    info.template = None;
                    assert!(!info.is_bundle_item());
                }
                let mut pouch = items.items["minecraft:stick"].clone();
                pouch.id = "test:pouch".into();
                pouch.bundle = true;
                pouch.template = None;
                assert!(!pouch.is_bundle_item());
                items.items.insert("test:pouch".into(), pouch);
                game.server_items = Some(std::sync::Arc::new(items));
            };
        let stack_with_bundle_contents = |id: &str| {
            let mut stack = ItemStack::new(id, 1);
            stack.data_mut().bundle_contents = vec![ItemStack::new("minecraft:stone", 1)];
            stack
        };
        with_part_only_mod(|| {
            // 1. Local menu: non-bundle items carrying `bundle_contents` (either per-stack OR as
            //    a default component with `info.bundle == true`) NEVER run vanilla `BundleItem`
            //    overrides; they run their own `ItemPart` and plain pickup.
            with_local(|game| {
                set_part_script(PartScript {
                    cancel_clicked: true,
                    set_clicked: Some(Some(("minecraft:diamond", 1))),
                    ..PartScript::default()
                });
                game.carried = Some(paper_with_bundle_contents());
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    trace(),
                    [("flint", StackedOnOtherDirection::OtherStackedOnMe)],
                    "non-bundle with bundle_contents must not suppress clicked ItemPart"
                );
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:diamond", 1))
                );

                // A real bundle with `bundle_contents` removed also returns false from
                // `BundleItem.overrideStackedOnOther`, letting clicked ItemPart run.
                set_part_script(PartScript {
                    cancel_clicked: true,
                    set_clicked: Some(Some(("minecraft:emerald", 1))),
                    ..PartScript::default()
                });
                game.carried = Some(bundle_without_contents());
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    trace(),
                    [("flint", StackedOnOtherDirection::OtherStackedOnMe)]
                );
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:emerald", 1))
                );

                // Now mark stick, flint, and `test:pouch` with `info.bundle == true` (default
                // `minecraft:bundle_contents`) and no `BundleItem` class.
                install_default_bundle_contents_non_bundle_items(game);

                // Carried non-bundle item with default `bundle_contents`: declining CarriedOnOther
                // MUST NOT trigger vanilla `BundleItem` insert; clicked ItemPart runs, and when
                // both decline plain pickup swaps the stacks instead of swallowing flint.
                set_part_script(PartScript::default());
                game.carried = Some(stack_with_bundle_contents("minecraft:stick"));
                game.inventory[9] = Some(stack_with_bundle_contents("minecraft:flint"));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    trace(),
                    [
                        ("stick", StackedOnOtherDirection::CarriedOnOther),
                        ("flint", StackedOnOtherDirection::OtherStackedOnMe),
                    ],
                    "non-BundleItem with default bundle_contents must run both ItemParts"
                );
                assert_eq!(
                    game.carried.as_ref().map(|s| &*s.item),
                    Some("minecraft:flint"),
                    "declining non-BundleItem with default bundle_contents must fall through to plain pickup swap"
                );
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| &*s.item),
                    Some("minecraft:stick")
                );

                // Clicked non-bundle item with default `bundle_contents`: primary click with stone
                // and secondary click with empty cursor must NOT trigger vanilla
                // `BundleItem.overrideOtherStackedOnMe`; its own OtherStackedOnMe and plain pickup
                // run instead.
                set_part_script(PartScript::default());
                game.carried = Some(ItemStack::new("minecraft:stone", 1));
                game.inventory[9] = Some(stack_with_bundle_contents("minecraft:flint"));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    trace(),
                    [("flint", StackedOnOtherDirection::OtherStackedOnMe)]
                );
                assert_eq!(
                    game.carried.as_ref().map(|s| &*s.item),
                    Some("minecraft:flint")
                );
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| &*s.item),
                    Some("minecraft:stone")
                );

                // A non-BundleItem with `info.bundle == true` and no `ItemPart` of its own
                // (`paper`) does NOT suppress clicked `flint`'s `OtherStackedOnMe` Part.
                set_part_script(PartScript {
                    cancel_clicked: true,
                    set_clicked: Some(Some(("minecraft:diamond", 1))),
                    ..PartScript::default()
                });
                game.carried = Some(stack_with_bundle_contents("minecraft:paper"));
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    trace(),
                    [("flint", StackedOnOtherDirection::OtherStackedOnMe)],
                    "non-BundleItem with default bundle_contents must not suppress clicked ItemPart"
                );
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:diamond", 1))
                );

                // Synthetic mod item `test:pouch` with `info.bundle == true`: neither carried
                // nor clicked triggers vanilla bundle insert/extract; plain pickup runs instead.
                set_part_script(PartScript::default());
                game.carried = Some(stack_with_bundle_contents("test:pouch"));
                game.inventory[9] = Some(ItemStack::new("minecraft:stone", 1));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| &*s.item),
                    Some("test:pouch")
                );
                assert_eq!(
                    game.carried.as_ref().map(|s| &*s.item),
                    Some("minecraft:stone")
                );
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert_eq!(game.carried.as_ref().map(|s| &*s.item), Some("test:pouch"));
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| &*s.item),
                    Some("minecraft:stone")
                );
                // Secondary click with empty cursor picks up `test:pouch` instead of popping
                // from `bundle_contents`.
                game.inventory[9] = Some(stack_with_bundle_contents("test:pouch"));
                game.carried = None;
                assert!(local_menu_ops(game, &[(Some(9), 1, Input::Pickup)]));
                assert_eq!(game.carried.as_ref().map(|s| &*s.item), Some("test:pouch"));
                assert!(game.inventory[9].is_none());

                // Dyed vanilla `BundleItem` (`minecraft:red_bundle`) STILL wins before clicked
                // `OtherStackedOnMe`.
                set_part_script(PartScript {
                    cancel_clicked: true,
                    set_clicked: Some(Some(("minecraft:diamond", 1))),
                    ..PartScript::default()
                });
                game.carried = Some(ItemStack::new("minecraft:red_bundle", 1));
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                assert!(local_menu_ops(game, &[(Some(9), 0, Input::Pickup)]));
                assert!(
                    trace().is_empty(),
                    "dyed vanilla BundleItem must still win before clicked ItemPart"
                );
                assert!(game.inventory[9].is_none());
            });

            // 2. LAN hub container: same shared carried override behavior.
            with_guests(|game| {
                PLAYER.set(12);
                let pos = (1, 64, 1);
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, stone).unwrap();
                let mut chest = vec![None; 27];
                chest[0] = Some(ItemStack::new("minecraft:flint", 1));
                game.world.block_entities.insert(
                    pos,
                    sunrise_level::containers::BlockEntity::Chest(chest.clone()),
                );
                let host = game.server.lan.as_mut().unwrap();
                host.hub.open_container(
                    12,
                    "minecraft:generic_9x3",
                    sunrise_core::nbt::Tag::String("test".into()),
                    pos,
                    Slots::Block(pos),
                    chest,
                    None,
                );
                let window = host.hub.guests[0].menu.as_ref().unwrap().window;
                let click_pkt = Serverbound::ContainerClick(ContainerClick {
                    container: window,
                    state: 0,
                    slot: 0,
                    button: 0,
                    input: 0,
                    changed: Vec::new(),
                    carried: None,
                });

                // Non-bundle carrying `bundle_contents` does NOT suppress clicked Part on LAN hub.
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(paper_with_bundle_contents());
                    });
                set_part_script(PartScript {
                    cancel_clicked: true,
                    set_clicked: Some(Some(("minecraft:diamond", 2))),
                    ..PartScript::default()
                });
                game.lan_guest_packet(12, click_pkt.clone());
                assert_eq!(
                    trace(),
                    [("flint", StackedOnOtherDirection::OtherStackedOnMe)],
                    "LAN hub must share real carried override outcome"
                );
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[0]
                        .as_ref()
                        .map(|s| (&*s.item, s.count)),
                    Some(("minecraft:diamond", 2))
                );

                // Non-BundleItem with default `bundle_contents` (`info.bundle == true`) on LAN
                // hub also runs its own ItemParts and plain pickup, NOT vanilla bundle override.
                install_default_bundle_contents_non_bundle_items(game);
                game.world.block_entities.get_mut(&pos).unwrap().slots_mut()[0] =
                    Some(stack_with_bundle_contents("minecraft:flint"));
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(stack_with_bundle_contents("minecraft:stick"));
                    });
                set_part_script(PartScript::default());
                game.lan_guest_packet(12, click_pkt.clone());
                assert_eq!(
                    trace(),
                    [
                        ("stick", StackedOnOtherDirection::CarriedOnOther),
                        ("flint", StackedOnOtherDirection::OtherStackedOnMe),
                    ],
                    "LAN hub must gate vanilla bundle override on BundleItem class, not info.bundle"
                );
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[0]
                        .as_ref()
                        .map(|s| &*s.item),
                    Some("minecraft:stick")
                );
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0]
                        .carried
                        .as_ref()
                        .map(|s| &*s.item),
                    Some("minecraft:flint")
                );

                // Real dyed `BundleItem` (`minecraft:red_bundle`) DOES suppress clicked Part on
                // LAN hub primary click.
                game.world.block_entities.get_mut(&pos).unwrap().slots_mut()[0] =
                    Some(ItemStack::new("minecraft:flint", 1));
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:red_bundle", 1));
                    });
                set_part_script(PartScript {
                    cancel_clicked: true,
                    set_clicked: Some(Some(("minecraft:diamond", 2))),
                    ..PartScript::default()
                });
                game.lan_guest_packet(12, click_pkt);
                assert!(
                    trace().is_empty(),
                    "real carried BundleItem must win before clicked ItemPart on LAN hub"
                );
                assert!(game.world.block_entities.get(&pos).unwrap().slots()[0].is_none());
            });
        });
    }
}

#[cfg(test)]
use sunrise_gameplay_common::game::tables::GameItems as _;

#[cfg(test)]
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
