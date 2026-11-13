//! `item_stacked_on_other` through a real listener on the local menu route and the LAN hub's
//! container click: decided, fired with `HostSide`, validated once and applied once. The
//! fixtures need prepared resources.
//!
//! This hook's cancellation is an override result, not an abort: a cancelled, handled click
//! sets the stacks the listener names and is Applied. The abort outcomes are a stale read, an
//! invalid answer and a failing listener; each undoes every write the listener made.
use crate::game::native_entity_refs::tests::with_game;
use crate::game::native_guest_turn_tests::with_guests;
use horizon_client::{items::ItemStack, menu::Input};
use mod_api::item::menu::StackResult;
use mod_api::{
    Flow, Hooks, HostSide, ItemPlace, ItemResource, ItemStackedOnOtherEvent, ModEntry, Mods,
};
use std::cell::RefCell;

/// What the listener does.
#[derive(Clone, Default)]
struct Script {
    /// Put dirt in this inventory slot through the `Host` (its own nested transaction).
    write_dirt: Option<i32>,
    /// ...and take it out again at once.
    take_dirt_back: bool,
    panic: bool,
    cancel: bool,
    handled: bool,
    /// Set the cursor to this stack (item, count); `Some(None)` empties it.
    set_carried: Option<Option<(&'static str, u32)>>,
    set_clicked: Option<Option<(&'static str, u32)>>,
}

thread_local! {
    static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
    /// What the listener saw each time: the cursor's and the clicked slot's counts.
    static SEEN: RefCell<Vec<(u32, u32)>> = const { RefCell::new(Vec::new()) };
    static PLAYER: std::cell::Cell<u128> = const { std::cell::Cell::new(0) };
    /// How often the menu type's `menu_slot_rules` part was asked.
    static ASKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Rules;
#[mod_api::horizon_part]
impl mod_api::MenuPart for Rules {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, _: &mut mod_api::MenuSlotRulesEvent<'_>) {
        ASKS.set(ASKS.get() + 1);
    }
}

fn script(script: Script) {
    SEEN.with(|seen| seen.borrow_mut().clear());
    SCRIPT.with(|s| *s.borrow_mut() = script);
}
fn dispatches() -> usize {
    SEEN.with(|seen| seen.borrow().len())
}

fn api_stack(item: &str, count: u32) -> mod_api::ItemStack {
    mod_api::ItemStack::new(mod_api::__private::item_key(item).unwrap(), count).unwrap()
}

struct Listener;
#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x3", Rules);
    }
    fn item_stacked_on_other(
        &self,
        mut host: HostSide,
        event: &mut ItemStackedOnOtherEvent<'_>,
    ) -> Flow {
        let counts = (
            event.event.carried.as_ref().map_or(0, |s| s.count()),
            event.event.clicked.as_ref().map_or(0, |s| s.count()),
        );
        SEEN.with(|seen| seen.borrow_mut().push(counts));
        let s = SCRIPT.with(|s| s.borrow().clone());
        if let (Some(slot), HostSide::Server(host)) = (s.write_dirt, &mut host) {
            let place = ItemPlace::PlayerSlot {
                player: PLAYER.get(),
                slot,
            };
            let dirt = api_stack("minecraft:dirt", 1);
            let tx = host.transaction_current().expect("inside the action");
            assert_eq!(
                place.insert(&mut **host, &tx, &ItemResource::of(&dirt), 2),
                2
            );
            if s.take_dirt_back {
                assert_eq!(
                    place.extract(&mut **host, &tx, &ItemResource::of(&dirt), 2),
                    2
                );
            }
        }
        if s.panic {
            panic!("a listener failed");
        }
        let set = |r: Option<Option<(&str, u32)>>| match r {
            None => StackResult::Keep,
            Some(None) => StackResult::Set(None),
            Some(Some((item, count))) => StackResult::Set(Some(api_stack(item, count))),
        };
        event.result.handled = s.handled;
        event.result.carried = set(s.set_carried);
        event.result.clicked = set(s.set_clicked);
        if s.cancel {
            event.cancelled = true;
            return Flow::Cancel;
        }
        Flow::Continue
    }
}

fn with_mod(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("stacked", Box::new(Listener))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, test);
}

/// A local game with a player, the inventory screen open, stone 3 on the cursor and nothing in
/// slot 9 (the first slot of the main inventory, menu slot 9).
fn with_local(test: impl FnOnce(&mut crate::game::Game<'_>)) {
    with_game(|game| {
        let _serving = crate::game::integrated::tests::install_serving(game, false);
        game.level.player_uuid = Some([0, 0, 0, 99]);
        PLAYER.set(99);
        let mut body = horizon_client::motion::PlayerMotion::new(
            game.motion,
            horizon_core::geometry::Vec3::new(0.0, 64.0, 0.0),
        )
        .unwrap();
        body.entity_id = 99;
        game.player = Some(body);
        game.next_entity = 100;
        game.screen = crate::game::Screen::Inventory;
        reset_local(game);
        test(game);
    });
}
fn reset_local(game: &mut crate::game::Game<'_>) {
    game.inventory = vec![None; game.inventory.len()];
    game.carried = Some(ItemStack::new("minecraft:stone", 3));
}
fn count(stack: &Option<ItemStack>) -> u32 {
    stack.as_ref().map_or(0, |s| s.count)
}
/// The click: pickup on the first main-inventory slot. What is left: (slot 9, cursor, slot 20).
fn click(game: &mut crate::game::Game<'_>) -> (u32, u32, u32) {
    assert!(game.menu_ops(&[(Some(9), 0, Input::Pickup)]));
    (
        count(&game.inventory[9]),
        count(&game.carried),
        count(&game.inventory[20]),
    )
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
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
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
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
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
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
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
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
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
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
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_outer_abort_after_an_applied_click_restores_the_inventory() {
    with_mod(|| {
        with_local(|game| {
            // Row 90: the click is Applied inside a transaction that then aborts.
            script(Script::default());
            let seal = game.transaction_open();
            assert_eq!(click(game), (3, 0, 0));
            game.transaction_close(&seal, false);
            assert_eq!((count(&game.inventory[9]), count(&game.carried)), (0, 3));
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
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
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_lan_hub_click_is_decided_the_same_way() {
    use horizon_client::lan::menu::Slots;
    use horizon_protocol::packets::play::{ContainerClick, Serverbound};
    with_mod(|| {
        with_guests(|game| {
            let pos = (1, 64, 1);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(pos.0, pos.1, pos.2, stone).unwrap();
            PLAYER.set(12);
            let open = |game: &mut crate::game::Game<'_>| {
                game.world.block_entities.insert(
                    pos,
                    horizon_client::containers::BlockEntity::Chest(vec![None; 27]),
                );
                let host = game.lan.as_mut().unwrap();
                host.hub.open_container(
                    12,
                    "minecraft:generic_9x3",
                    horizon_core::nbt::Tag::String("test".into()),
                    pos,
                    Slots::Block(pos),
                    vec![None; 27],
                    None,
                );
                let window = host.hub.guests[0].menu.as_ref().unwrap().window;
                host.hub.guests.update(0, |guest| {
                    guest.inventory = vec![None; guest.inventory.len()];
                    guest.carried = Some(ItemStack::new("minecraft:stone", 3));
                });
                window
            };
            let click = |game: &mut crate::game::Game<'_>, window: i32| {
                game.lan_guest_packet(
                    12,
                    Serverbound::ContainerClick(ContainerClick {
                        container: window,
                        state: 0,
                        slot: 0,
                        button: 0,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    }),
                );
                let slot = game.world.block_entities.get(&pos).unwrap().slots()[0]
                    .as_ref()
                    .map_or(0, |s| s.count);
                let host = game.lan.as_ref().unwrap();
                let guest = &host.hub.guests[0];
                (slot, count(&guest.carried), count(&guest.inventory[20]))
            };
            let window = open(game);
            script(Script::default());
            ASKS.set(0);
            // Row 72 on the LAN route: stone 3 into the empty slot, one dispatch with the
            // frozen stacks, and the LAN host is back.
            assert_eq!(click(game, window), (3, 0, 0));
            // The slot rules were asked while the click was planned and nowhere else: once for
            // each distinct question the click consults (a plain click asks the same one twice,
            // may place and capacity), though the plan tried both ways the hook can leave it and
            // the click ran after it with the stored answer.
            assert_eq!(ASKS.get(), 1);
            assert_eq!(SEEN.with(|s| s.borrow().clone()), [(3, 0)]);
            assert!(game.lan.is_some());
            // Rows 73 and 84: a write and then a panic: undone, the click does nothing, the
            // guest's swap and the host are restored, and the next click works.
            let window = open(game);
            script(Script {
                write_dirt: Some(20),
                panic: true,
                ..Script::default()
            });
            assert_eq!(click(game, window), (0, 3, 0));
            assert!(game.lan.is_some());
            assert!(game.lan_guest_swaps.is_empty());
            let window = open(game);
            script(Script::default());
            assert_eq!(click(game, window), (3, 0, 0));
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_lan_inventory_click_is_decided_the_same_way() {
    use horizon_protocol::packets::play::{ContainerClick, Serverbound};
    with_mod(|| {
        with_guests(|game| {
            PLAYER.set(12);
            let click = |game: &mut crate::game::Game<'_>| {
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
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
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                (
                    count(&guest.inventory[9]),
                    count(&guest.carried),
                    count(&guest.inventory[20]),
                )
            };
            script(Script::default());
            assert_eq!(click(game), (3, 0, 0));
            assert_eq!(SEEN.with(|s| s.borrow().clone()), [(3, 0)]);
            // A write to the slot the click read, then a panic: undone, nothing changes.
            script(Script {
                write_dirt: Some(20),
                panic: true,
                ..Script::default()
            });
            assert_eq!(click(game), (0, 3, 0));
            assert!(game.lan.is_some());
            // A write to what the click read is stale.
            script(Script {
                write_dirt: Some(9),
                ..Script::default()
            });
            assert_eq!(click(game), (0, 3, 0));
            script(Script::default());
            assert_eq!(click(game), (3, 0, 0));
        });
    });
}

/// The LAN menus the game runs for a guest: the hook fires on each, with the frozen stacks, and a
/// cancelled, handled answer that sets the cursor applies. Plain, the stone on the cursor would
/// have gone into the slot; set, the cursor ends as dirt 2.
mod families {
    use super::*;
    use crate::game::Game;
    use horizon_protocol::packets::play::{ContainerClick, Serverbound};

    fn dirt_on_the_cursor() -> Script {
        Script {
            cancel: true,
            handled: true,
            set_carried: Some(Some(("minecraft:dirt", 2))),
            ..Script::default()
        }
    }

    fn cursor(game: &Game<'_>) -> Option<(String, u32)> {
        let guest = &game.lan.as_ref().unwrap().hub.guests[0];
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
        open: impl FnOnce(&mut Game<'_>),
        click: impl Fn(&mut Game<'_>),
        plain_cursor: Option<(&str, u32)>,
    ) {
        with_mod(|| {
            with_guests(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                );
                PLAYER.set(12);
                open(game);
                let give = |game: &mut Game<'_>| {
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
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

    fn block(game: &mut Game<'_>, name: &str, pos: (i32, i32, i32)) {
        let state = game.pack.assets.registry.default_state(name).unwrap();
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_crafting_table_menu_decides_the_pickup() {
        let window = std::cell::Cell::new(0);
        family(
            |game| {
                block(game, "minecraft:crafting_table", (1, 64, 1));
                game.lan_open_crafting(12, [1, 64, 1]);
                let host = game.lan.as_ref().unwrap();
                window.set(host.menus[&12].crafting.table.as_ref().unwrap().0);
            },
            |game| game.lan_guest_packet(12, packet(window.get(), 1)),
            None,
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_furnace_menu_decides_the_pickup() {
        use horizon_client::containers::{BlockEntity, Furnace, FurnaceKind};
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
                let host = game.lan.as_ref().unwrap();
                window.set(host.hub.guests[0].menu.as_ref().unwrap().window);
            },
            |game| game.lan_guest_packet(12, packet(window.get(), 0)),
            None,
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_workstation_menu_decides_the_pickup() {
        use crate::game::stations::{Kind, Station};
        let window = std::cell::Cell::new(0);
        family(
            |game| {
                block(game, "minecraft:stonecutter", (1, 64, 1));
                assert!(game.lan_open_station(12, &Station::new(Kind::Stonecutter, [1, 64, 1])));
                let host = game.lan.as_ref().unwrap();
                window.set(host.menus[&12].station.as_ref().unwrap().window);
            },
            |game| game.lan_guest_packet(12, packet(window.get(), 0)),
            None,
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_mount_menu_decides_the_pickup() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        let window = std::cell::Cell::new(0);
        family(
            |game| {
                let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
                let mut horse = Mob::new(60, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                horse.extra.tame = true;
                game.mobs.push(horse);
                game.lan_open_mount(12, 60);
                let host = game.lan.as_ref().unwrap();
                window.set(host.play[&12].mount_window.as_ref().unwrap().window);
            },
            |game| game.lan_guest_packet(12, packet(window.get(), 1)),
            // The stone is no horse armor: the slot refuses it and the cursor keeps it.
            Some(("minecraft:stone", 3)),
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_trading_menu_decides_the_pickup() {
        use crate::game::native_merchant_refs::tests::{offer, spawn};
        use crate::game::trading::MerchantState;
        use horizon_client::merchant::{Session, Trader};
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
                    game.lan.as_ref().unwrap().play[&12]
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
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_outer_abort_after_an_applied_lan_click_restores_the_guest_and_the_container() {
    use horizon_client::lan::menu::Slots;
    use horizon_protocol::packets::play::{ContainerClick, Serverbound};
    with_mod(|| {
        with_guests(|game| {
            PLAYER.set(12);
            script(Script::default());
            let pos = (1, 64, 1);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
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
                horizon_client::containers::BlockEntity::Chest(vec![None; 27]),
            );
            let host = game.lan.as_mut().unwrap();
            host.hub.open_container(
                12,
                "minecraft:generic_9x3",
                horizon_core::nbt::Tag::String("test".into()),
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
            let give = |game: &mut crate::game::Game<'_>| {
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.inventory = vec![None; guest.inventory.len()];
                    guest.carried = Some(ItemStack::new("minecraft:stone", 3));
                });
            };
            let state = |game: &crate::game::Game<'_>| {
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
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
            let seal = game.transaction_open();
            game.lan_guest_packet(12, packet(window, 0));
            assert_eq!(state(game), (3, 0, 0));
            game.transaction_close(&seal, false);
            assert_eq!(state(game), (0, 3, 0));
            // The guest's inventory menu: slot 9 and the cursor.
            give(game);
            let seal = game.transaction_open();
            game.lan_guest_packet(12, packet(0, 9));
            assert_eq!(state(game), (0, 0, 3));
            game.transaction_close(&seal, false);
            assert_eq!(state(game), (0, 3, 0));
        });
    });
}
