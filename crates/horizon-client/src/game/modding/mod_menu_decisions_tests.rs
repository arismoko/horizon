//! `menu_clicked` and `menu_quick_move` through real listeners: both are server hooks on the menus
//! the game runs for a guest, and each now runs as one decided action (`decided_action`): the
//! hook is fired inside an action savepoint, its answer is validated once and applied once, and a
//! refusal (stale, invalid, a failing listener) changes nothing and undoes every write the
//! listener made. The fixtures need prepared resources.
use crate::game::native_guest_turn_tests::with_guests;
use horizon_client::{
    containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields},
    items::ItemStack,
    lan::menu::Slots,
};
use horizon_protocol::packets::play::{ContainerClick, Serverbound};
use mod_api::{
    BlockEntityPart, Flow, Hooks, ItemPlace, ItemResource, MenuClickedEvent, MenuQuickMoveEvent,
    ModEntry, Mods, Registrar, ServerHost,
};
use std::cell::{Cell, RefCell};

#[derive(Clone, Default)]
struct Script {
    /// Put dirt in guest slot 20 through the `Host`, in the listener's own nested transaction.
    write_dirt: bool,
    panic: bool,
    /// `menu_quick_move`: take 9 of the stack into slots 1 and 2 of the block (4, then 5).
    take: bool,
    /// ...and answer with a stack past the item's maximum in slot 1.
    invalid: bool,
}

thread_local! {
    static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
    static DISPATCHES: Cell<usize> = const { Cell::new(0) };
    /// Whether a transaction was open at each dispatch: the action's savepoint.
    static IN_ACTION: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

fn script(script: Script) {
    DISPATCHES.set(0);
    super::decided_action::DECIDED.set(0);
    IN_ACTION.with(|a| a.borrow_mut().clear());
    SCRIPT.with(|s| *s.borrow_mut() = script);
}

fn write_dirt(host: &mut dyn ServerHost) {
    let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
    let dirt = mod_api::ItemStack::new(key, 1).unwrap();
    let place = ItemPlace::PlayerSlot {
        player: 12,
        slot: 20,
    };
    let tx = host.transaction_current().expect("inside the action");
    assert_eq!(place.insert(host, &tx, &ItemResource::of(&dirt), 2), 2);
}

struct Part;
#[mod_api::horizon_part]
impl mod_api::BlockPart for Part {}
#[mod_api::horizon_part]
impl BlockEntityPart for Part {
    fn menu_quick_move(
        &self,
        host: &mut dyn ServerHost,
        event: &mut MenuQuickMoveEvent<'_>,
    ) -> Flow {
        DISPATCHES.set(DISPATCHES.get() + 1);
        IN_ACTION.with(|a| a.borrow_mut().push(host.transaction_current().is_some()));
        let s = SCRIPT.with(|s| s.borrow().clone());
        let stone = |count| {
            mod_api::ItemStack::new(
                mod_api::__private::item_key("minecraft:stone").unwrap(),
                count,
            )
            .unwrap()
        };
        if s.write_dirt {
            write_dirt(host);
        }
        if s.take {
            event.entity.items.resize(9, None);
            event.entity.items[1] = Some(stone(if s.invalid { 65 } else { 4 }));
            event.entity.items[2] = Some(stone(5));
            event.stack.set_count(1).unwrap();
        }
        if s.panic {
            panic!("a listener failed");
        }
        if s.take {
            return Flow::Cancel;
        }
        Flow::Continue
    }
}

struct Mod;
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("depot").blocks(&[]).build();
        r.block_entity_part(&kind, Part).unwrap();
    }
    fn menu_clicked(&self, host: &mut dyn ServerHost, event: &mut MenuClickedEvent) {
        DISPATCHES.set(DISPATCHES.get() + 1);
        IN_ACTION.with(|a| a.borrow_mut().push(host.transaction_current().is_some()));
        // Guest 12's menu from `open_native_menu`.
        assert_eq!((event.uuid, event.window), (12, 5));
        let s = SCRIPT.with(|s| s.borrow().clone());
        if s.write_dirt {
            write_dirt(host);
        }
        if s.panic {
            panic!("a listener failed");
        }
    }
}

fn with_mod(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("movemod", Box::new(Mod))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, test);
}

/// How many decisions were entered since the last `script`.
fn decided() -> usize {
    super::decided_action::DECIDED.get()
}

fn count(stack: &Option<ItemStack>) -> u32 {
    stack.as_ref().map_or(0, |s| s.count)
}

fn inventory(game: &crate::game::Game<'_>, slot: usize) -> u32 {
    count(&game.lan.as_ref().unwrap().hub.guests[0].inventory[slot])
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_click_on_a_mod_menu_tells_the_mods_inside_an_action_and_a_failure_undoes_it() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_native_menu(game);
            let click = |game: &mut crate::game::Game<'_>| {
                game.lan_guest_packet(12, container_click(window, 3, 0));
                game.run_native_menu_events();
            };
            // Row 72: the mods hear the click once, inside the action, and what the listener
            // wrote through the `Host` stays when it finishes.
            script(Script {
                write_dirt: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(decided(), 1);
            assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
            assert_eq!(inventory(game, 20), 2);
            assert!(game.transactions.is_empty());
            // Rows 79 and 84: a write and then a panic: the write is undone, the click is not
            // heard again, and the next click works.
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[20] = None;
            });
            script(Script {
                write_dirt: true,
                panic: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(inventory(game, 20), 0);
            assert!(game.transactions.is_empty());
            script(Script::default());
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            // Row 90: heard inside a transaction that then aborts: the write is undone.
            script(Script {
                write_dirt: true,
                ..Script::default()
            });
            let seal = game.transaction_open();
            click(game);
            assert_eq!(inventory(game, 20), 2);
            game.transaction_close(&seal, false);
            assert_eq!(inventory(game, 20), 0);
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn without_a_listener_a_click_on_a_mod_menu_is_heard_plainly() {
    // Row 92: no mod at all: the plain call, no transaction and no journaled write.
    with_guests(|game| {
        let window = open_native_menu(game);
        script(Script::default());
        let writes = game.transactions.writes();
        game.lan_guest_packet(12, container_click(window, 3, 0));
        game.run_native_menu_events();
        assert_eq!(DISPATCHES.get(), 0);
        // The ordinary path: the decision is never entered.
        assert_eq!(decided(), 0);
        assert_eq!(game.transactions.writes(), writes);
        assert!(game.transactions.is_empty());
    });
}

fn container_click(window: i32, slot: i16, input: i32) -> Serverbound {
    Serverbound::ContainerClick(ContainerClick {
        container: window,
        state: 0,
        slot,
        button: 0,
        input,
        changed: Vec::new(),
        carried: None,
    })
}

/// A menu a mod opened (`Slots::Native`) for guest 12.
fn open_native_menu(game: &mut crate::game::Game<'_>) -> i32 {
    use horizon_client::lan::menu::Menu;
    let menu = Menu {
        window: 5,
        kind: 2,
        pos: None,
        slots: Slots::Native,
        size: 9,
        state: 0,
        sent: Vec::new(),
        drag: None,
        sent_data: Vec::new(),
        hidden: Vec::new(),
        mod_type: None,
        registry_id: "test:native".into(),
        craft: None,
        links: Vec::new(),
        asks_still_valid: false,
    };
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |guest| guest.menu = Some(menu));
    5
}

const POS: (i32, i32, i32) = (1, 64, 1);

/// A mod container (a block entity of the mod's registered type) with stone 10 in slot 0, open
/// as a 9-slot menu for guest 12.
fn open_depot(game: &mut crate::game::Game<'_>) -> i32 {
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
    );
    let mut fields = ModBlockEntityFields {
        items: vec![None; 9],
        ..Default::default()
    };
    fields.items[0] = Some(ItemStack::new("minecraft:stone", 10));
    game.world.block_entities.insert(
        POS,
        BlockEntity::Mod(Box::new(ModBlockEntity::new(
            "movemod:depot".into(),
            fields,
        ))),
    );
    let host = game.lan.as_mut().unwrap();
    host.hub.open_container(
        12,
        "minecraft:generic_9x1",
        horizon_core::nbt::Tag::String("test".into()),
        POS,
        Slots::Block(POS),
        vec![None; 9],
        None,
    );
    host.hub.guests[0].menu.as_ref().unwrap().window
}

/// The block's slots 0, 1 and 2, and guest 12's slot 20.
fn depot(game: &crate::game::Game<'_>) -> (u32, u32, u32, u32) {
    let Some(BlockEntity::Mod(m)) = game.world.block_entities.get(&POS) else {
        panic!("the block entity is gone");
    };
    let slot = |k: usize| count(m.items.get(k).unwrap_or(&None));
    (slot(0), slot(1), slot(2), inventory(game, 20))
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mod_quick_move_is_one_action_and_a_refusal_leaves_no_prefix() {
    with_mod(|| {
        with_guests(|game| {
            open_depot(game);
            let click = |game: &mut crate::game::Game<'_>| {
                let window = game.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .unwrap()
                    .window;
                game.lan_guest_packet(12, container_click(window, 0, 1));
            };
            // Row 72: the mod takes 9 of the 10 into two slots: the move applies once, inside the
            // action, and the plain routing is skipped.
            script(Script {
                take: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
            assert_eq!(depot(game), (1, 4, 5, 0));
            assert!(game.transactions.is_empty());
            // Row 77: the same move, and the listener also writes the guest's inventory: stale,
            // so the whole move is refused: the source keeps its 10 and neither target slot is
            // left holding a prefix of the move.
            open_depot(game);
            script(Script {
                take: true,
                write_dirt: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(depot(game), (10, 0, 0, 0));
            // Row 89: an invalid answer (a stack past the maximum) refuses the whole move too.
            open_depot(game);
            script(Script {
                take: true,
                invalid: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(depot(game), (10, 0, 0, 0));
            // Rows 79 and 84: a panic after writing: the block entity is put back as it was
            // (it is out of the world while the mods are asked), the write is undone, and the next
            // click works.
            open_depot(game);
            script(Script {
                take: true,
                write_dirt: true,
                panic: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(depot(game), (10, 0, 0, 0));
            assert!(game.transactions.is_empty());
            // The panic alone, with no `Host` write (which would make the move stale by itself):
            // the move the listener answered is still refused and the block entity is put back.
            open_depot(game);
            script(Script {
                take: true,
                panic: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(depot(game), (10, 0, 0, 0));
            assert!(game.transactions.is_empty());
            open_depot(game);
            script(Script {
                take: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(depot(game), (1, 4, 5, 0));
            // Row 90: applied inside a transaction that then aborts: the move is undone.
            open_depot(game);
            let seal = game.transaction_open();
            click(game);
            assert_eq!(depot(game), (1, 4, 5, 0));
            game.transaction_close(&seal, false);
            assert_eq!(depot(game), (10, 0, 0, 0));
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mod_quick_move_no_mod_took_is_left_to_the_games_routing() {
    let run = || {
        let mut result = None;
        with_guests(|game| {
            let window = open_depot(game);
            script(Script::default());
            game.lan_guest_packet(12, container_click(window, 0, 1));
            assert!(game.transactions.is_empty());
            result = Some(depot(game));
        });
        result.unwrap()
    };
    // Row 92: with no part for the hook, the plain click: no dispatch, and the result is the
    // game's own routing.
    let plain = run();
    assert_eq!(DISPATCHES.get(), 0);
    // The ordinary path: the decision is never entered.
    assert_eq!(decided(), 0);
    // A listener that only looks is asked once, in the action, and leaves the click to the same
    // routing: the same result.
    with_mod(|| {
        assert_eq!(run(), plain);
        assert_eq!(DISPATCHES.get(), 1);
        assert_eq!(decided(), 1);
        assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
    });
}
