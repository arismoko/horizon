//! `menu_slot_rules`' `may_pickup` through a real `MenuPart` on a mod menu type: each path that
//! takes from a slot asks it, and a refusal leaves the slot and the cursor as they were, on the
//! server (a LAN guest's click) and in the client's prediction alike, with the slots a client
//! predicted otherwise sent back; a drag and a creative clone never ask it. The fixtures need
//! prepared resources.
use mod_api::{
    BlockEntityPart, BlockPart, Hooks, MenuContainer, MenuPart, MenuPickAllEvent,
    MenuQuickMoveEvent, MenuSlotRulesEvent, ModEntry, Mods, Registrar, prelude::PlayersHost as _,
};
use std::{
    cell::{Cell, RefCell},
    sync::{Arc, mpsc},
};
use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
use sunrise_items::items::ItemStack;
use sunrise_level::{
    chunk::Chunk,
    containers::{
        BlockEntity, ModBlockEntity, ModBlockEntityFields, ModContainer, ModCrafting, ModLinks,
        ModMenuType,
    },
};
use sunrise_protocol::{
    buf::Reader,
    item::Hashed,
    packets::play::{Clientbound as C, ContainerClick, Serverbound},
};
use sunrise_sim::{
    lan::{
        Out, PlayerSender,
        menu::{ModCraft, Slots},
    },
    menu::{Input, Kind, Menu},
};

/// The menu type the pot opens: a prepared registry slot (`minecraft:generic_9x3`) for a
/// synthetic mod menu type, so the guest's click names it.
const MENU: &str = "minecraft:generic_9x3";
const POT_TYPE: &str = "pot:pot";
/// The pot's slot that only shows its stack: it refuses pickup and placement. (The rule's slot
/// is the backing index, so the tests leave the player's inventory index 1 alone.)
const SHOWN: usize = 1;
/// The pot's result slot: as a mod menu type's result, it refuses placement.
const RESULT: usize = 2;
/// The pot's slot that takes a stack but gives nothing up.
const KEEPS: usize = 4;
const POS: (i32, i32, i32) = (1, 64, 1);
const LINK: (i32, i32, i32) = (2, 64, 1);
const LINKED_SLOT: usize = 3;
/// The pot's container slots: the menu's hotbar 0 is slot `SLOTS + 27`.
const SLOTS: usize = 27;

thread_local! {
    /// Every question the part was asked: slot, item and count.
    static ASKED: RefCell<Vec<(usize, String, u32)>> = const { RefCell::new(Vec::new()) };
    /// The slots `menu_quick_move` was heard for.
    static MOVED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    /// Optional per-call `may_pickup` overrides for slot 0 (popped from the front).
    static SLOT0_PICKUP: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    static NEEDS_LINKED_NEIGHBOR: Cell<bool> = const { Cell::new(false) };
    /// Ask the override to leave this many items instead of passing the click.
    static QUICK_MOVE_LEFT: Cell<Option<u32>> = const { Cell::new(None) };
    /// The clicked stack the override was given.
    static QUICK_MOVE_SOURCE: Cell<Option<u32>> = const { Cell::new(None) };
    static QUICK_MOVE_CONSUME: Cell<Option<usize>> = const { Cell::new(None) };
    static LINK_CHANGE: Cell<Option<LinkChange>> = const { Cell::new(None) };
    static LINK_CHANGED: Cell<bool> = const { Cell::new(false) };
    /// The pot slot `menu_pick_all` keeps out of a gather, if any.
    static PICK_ALL_KEEPS: Cell<Option<usize>> = const { Cell::new(None) };
    /// Every `menu_pick_all` question: slot, its count and the cursor's count.
    static PICK_ALL_ASKED: RefCell<Vec<(usize, u32, u32)>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy, Debug)]
enum LinkChange {
    Insert,
    Remove,
}

struct Pot;
#[mod_api::sunrise_part]
impl MenuPart for Pot {
    fn menu_pick_all(&self, _: &dyn mod_api::Host, event: &mut MenuPickAllEvent<'_>) {
        if event.container == MenuContainer::Main {
            PICK_ALL_ASKED.with_borrow_mut(|asked| {
                asked.push((event.slot, event.stack.count(), event.carried.count()))
            });
            if PICK_ALL_KEEPS.get() == Some(event.slot) {
                event.allowed = false;
            }
        }
    }
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        let item = mod_api::__private::item_key_id(event.stack.item())
            .unwrap()
            .to_owned();
        ASKED.with(|asked| {
            asked
                .borrow_mut()
                .push((event.slot, item, event.stack.count()))
        });
        if event.slot == 0
            && let Some(allowed) = SLOT0_PICKUP.with(|seq| {
                let mut seq = seq.borrow_mut();
                (!seq.is_empty()).then(|| seq.remove(0))
            })
        {
            event.may_pickup = allowed;
        }
        if event.container == MenuContainer::Main && event.slot == 0 && NEEDS_LINKED_NEIGHBOR.get()
        {
            event.may_pickup = event
                .context
                .slot(MenuContainer::Main, LINKED_SLOT)
                .is_some();
        }
        if event.slot == SHOWN {
            event.may_pickup = false;
            event.may_place = false;
        }
        event.may_pickup &= event.slot != KEEPS;
        if event.container == mod_api::MenuContainer::Main
            && event.slot == 0
            && event
                .context
                .slot(mod_api::MenuContainer::Grid, 0)
                .is_some()
        {
            event.may_pickup &= event
                .context
                .slot(mod_api::MenuContainer::Result, 0)
                .is_some_and(|stack| {
                    mod_api::__private::item_key_id(stack.item()) == Some("minecraft:oak_planks")
                        && stack.count() == 4
                });
        }
    }
}
/// The pot block's shift-click: usually left to the game, unless the fixture supplies a remainder.
struct PotBlock;
#[mod_api::sunrise_part]
impl BlockPart for PotBlock {}
#[mod_api::sunrise_part]
impl BlockEntityPart for PotBlock {
    fn menu_quick_move(&self, event: &mut MenuQuickMoveEvent<'_>) -> mod_api::Flow {
        MOVED.with(|moved| moved.borrow_mut().push(event.slot));
        if let Some(left) = QUICK_MOVE_LEFT.take() {
            let source = event.stack.as_mut().unwrap();
            QUICK_MOVE_SOURCE.set(Some(source.count()));
            if let Some(slot) = QUICK_MOVE_CONSUME.take() {
                assert_eq!(
                    event
                        .subject
                        .with_view(|mut view| view.set_item_slot(slot as u32, None)),
                    Some(Ok(true))
                );
            }
            if let Some(change) = LINK_CHANGE.take() {
                use mod_api::{
                    BlockState, BlockUpdateFlags, HostBlockStates, ITEMS, ItemResource,
                    ResourceHandler,
                };
                let mut moved = source.clone();
                moved.set_count(source.count() - left).unwrap();
                assert_eq!(
                    event
                        .subject
                        .with_view(|mut view| view.set_item_slot(0, Some(moved))),
                    Some(Ok(true))
                );
                let pos = [LINK.0, LINK.1, LINK.2];
                match change {
                    LinkChange::Insert => {
                        let result: Result<(), mod_api::TransactionError<()>> =
                            event.subject.transaction(|scope, tx| {
                                let host = scope.host();
                                let level = host.level("minecraft:overworld").unwrap();
                                let items = level.capability(host, &ITEMS, pos, None).unwrap();
                                assert_eq!(
                                    items.insert_at(host, tx, 0, &ItemResource::of(source), 1),
                                    Ok(1)
                                );
                                Ok(())
                            });
                        result.unwrap();
                    }
                    LinkChange::Remove => {
                        let host = event.subject.host();
                        let level = host.level("minecraft:overworld").unwrap();
                        let air = BlockState::of(
                            &host.block_states(),
                            mod_api::__private::block_key("minecraft:air").unwrap(),
                            &[],
                        )
                        .unwrap();
                        assert_eq!(
                            level.set_block(host, pos, air, BlockUpdateFlags::KNOWN_SHAPE),
                            Ok(true)
                        );
                    }
                }
                LINK_CHANGED.set(true);
            }
            if left == 0 {
                *event.stack = None;
            } else {
                source.set_count(left).unwrap();
            }
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }
}
/// The pot's mod, with the block's `menu_quick_move` listener or without it.
struct PotMod {
    pub quick_move: bool,
}
#[mod_api::__subscriptions]
impl Hooks for PotMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part(MENU, Pot);
        let kind = r.block_entity_type("pot").blocks(&[]).build();
        if self.quick_move {
            r.block_entity_part(&kind, PotBlock).unwrap();
        }
    }
}

fn pot_type() -> &'static ModMenuType {
    Box::leak(Box::new(ModMenuType {
        id: MENU,
        owner: "test",
        slots: SLOTS,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: vec![RESULT],
        inputs: Vec::new(),
        crafting: None,
        links: None,
    }))
}
fn stone(count: u32) -> Option<ItemStack> {
    Some(ItemStack::new("minecraft:stone", count))
}
/// What the rule was asked, in order: slot, item and count.
type Asks = Vec<(usize, String, u32)>;
fn asked() -> Asks {
    ASKED.with(|asked| std::mem::take(&mut *asked.borrow_mut()))
}
/// How many times `asks` name `count` of `item` in `slot`.
fn times(asks: &Asks, (slot, item, count): (usize, &str, u32)) -> usize {
    let question = (slot, item.to_owned(), count);
    asks.iter().filter(|asked| **asked == question).count()
}

/// Guest 12 with the pot at `POS` open, and what their client is sent.
fn with_pot(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &mpsc::Receiver<Out>)) {
    with_pot_mod(true, test);
}

/// `with_pot`, with the block's `menu_quick_move` listener when `quick_move`.
fn with_pot_mod(
    quick_move: bool,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &mpsc::Receiver<Out>),
) {
    let mods: Mods = vec![ModEntry::new("pot", Box::new(PotMod { quick_move }))].into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    crate::entry::install_client_hooks();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            // The fixture bypasses production content loading. Resolve the pot's type in this
            // scoped Parts table so quick moves reach its registered block-entity listener.
            mod_api::__private::resolve_block_entity_types([POT_TYPE], |id| {
                (id == POT_TYPE).then_some(())
            })
            .unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(POS.0, POS.1, POS.2, stone).unwrap();
            // The block's container shows its slots to the menu.
            let container = Arc::new(ModContainer {
                slots: SLOTS,
                inputs: vec![0],
                ..Default::default()
            });
            let state = game.world.block(POS.0, POS.1, POS.2).0;
            let containers = [(state, container)].into_iter().collect();
            game.world.block_entities.set_mod_containers(containers);
            let fields = ModBlockEntityFields {
                items: vec![None; SLOTS],
                ..Default::default()
            };
            let entity = ModBlockEntity::new(POT_TYPE.into(), fields);
            game.world
                .block_entities
                .insert(POS, BlockEntity::Mod(Box::new(entity)));
            let (tx, rx) = mpsc::channel();
            let host = game.server.lan.as_mut().unwrap();
            let title = sunrise_core::nbt::Tag::String("pot".into());
            let items = vec![None; SLOTS];
            host.hub
                .open_container(12, MENU, title, POS, Slots::Block(POS), items, None);
            let menu_type = pot_type();
            host.hub.guests.update(0, |guest| {
                guest.menu.as_mut().unwrap().mod_type = Some(menu_type);
                guest.sender = PlayerSender::new(tx, Arc::default());
                // Within reach, so the menu stays valid for `menu_quick_move`.
                guest.position = [1.5, 64.0, 2.5];
            });
            test(game, &rx);
        });
    });
}

/// A client's copy of the pot's menu: `slots` by menu index, `carried` on the cursor.
fn prediction(slots: &[(usize, Option<ItemStack>)], carried: Option<ItemStack>) -> Menu {
    let mut menu = Menu::new(Kind::Mod(pot_type()));
    menu.set_registry_id(Some(MENU));
    for (slot, stack) in slots {
        menu.load_slot(*slot, stack.clone());
    }
    menu.carried = carried;
    menu
}

/// What a menu shows: the pot's slots, the inventory by index, then the cursor.
type Shown = Vec<Option<(String, u32)>>;
fn shown(stack: Option<&ItemStack>) -> Option<(String, u32)> {
    stack.map(|s| (s.item.to_string(), s.count))
}
fn predicted(menu: &Menu) -> Shown {
    (0..SLOTS)
        .map(|s| shown(menu.slot(s)))
        .chain((0..36).map(|i| shown(menu.player_item(i))))
        .chain([shown(menu.carried.as_ref())])
        .collect()
}
fn served(game: &sunrise_gameplay::game::ServerGame<'_>) -> Shown {
    let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
    let pot = game.world.block_entities.get(&POS).unwrap().slots();
    pot.iter()
        .map(Option::as_ref)
        .chain(guest.inventory[..36].iter().map(Option::as_ref))
        .chain([guest.carried.as_ref()])
        .map(shown)
        .collect()
}
/// Starts both sides with the same pot, inventory and cursor state.
fn serve(game: &mut sunrise_gameplay::game::ServerGame<'_>, menu: &Menu) {
    let pot = game.world.block_entities.get_mut(&POS).unwrap().slots_mut();
    for (s, slot) in pot.iter_mut().enumerate() {
        *slot = menu.slot(s).cloned();
    }
    let hub = &mut game.server.lan.as_mut().unwrap().hub;
    let window = hub.guests[0].menu.as_ref().unwrap().window;
    hub.guests.update(0, |guest| {
        for i in 0..36 {
            guest.inventory[i] = menu.player_item(i).cloned();
        }
        guest.carried = menu.carried.clone();
    });
    hub.resend_menu(12, window, &game.common.world);
}

/// A client's claim that a slot or the cursor holds `count` stone.
fn claim(game: &mut sunrise_gameplay::game::ServerGame<'_>, count: u32) -> Option<Hashed> {
    let item = game
        .server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .item_id(&"minecraft:stone".into());
    let count = i32::try_from(count).unwrap();
    Some(Hashed {
        item,
        count,
        added: Vec::new(),
        removed: Vec::new(),
    })
}

/// The guest's click, sent as a client that predicted the take sends it (`changed`, `carried`),
/// and the slots and cursor the guest is sent back: (slot, count), with `-1` for the cursor.
fn click(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    rx: &mpsc::Receiver<Out>,
    (slot, button, input): (i32, i32, Input),
    changed: Vec<(i16, Option<Hashed>)>,
    carried: Option<Hashed>,
) -> Vec<(i16, u32)> {
    let id = match input {
        Input::Pickup => 0,
        Input::QuickMove => 1,
        Input::Swap => 2,
        Input::Clone => 3,
        Input::Throw => 4,
        Input::QuickCraft => 5,
        Input::PickupAll => 6,
    };
    let menu = game.server.lan.as_ref().unwrap().hub.guests[0]
        .menu
        .as_ref();
    let (container, state) = menu.map(|m| (m.window, m.state)).unwrap();
    rx.try_iter().for_each(drop);
    game.lan_guest_packet(
        12,
        Serverbound::ContainerClick(ContainerClick {
            container,
            state,
            slot: i16::try_from(slot).unwrap(),
            button: i8::try_from(button).unwrap(),
            input: id,
            changed,
            carried,
        }),
    );
    let count = |item: &Option<sunrise_protocol::item::Stack>| {
        item.as_ref().map_or(0, |s| u32::try_from(s.count).unwrap())
    };
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => Vec::new(),
        })
        .filter_map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            match C::decode(id, reader.rest()).unwrap() {
                C::ContainerSetSlot(set) => Some((set.slot, count(&set.item))),
                C::SetCursorItem(cursor) => Some((-1, count(&cursor.item))),
                _ => None,
            }
        })
        .collect()
}

/// `action` from `slots` and `carried` in the client's prediction, then on the server as that
/// client sends it: the prediction, and what each side asked the rule.
fn asked_by_both(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    rx: &mpsc::Receiver<Out>,
    (slots, carried): (&[(usize, Option<ItemStack>)], Option<ItemStack>),
    action: (i32, i32, Input),
) -> (Menu, Asks, Asks) {
    let mut menu = prediction(slots, carried);
    serve(game, &menu);
    asked();
    menu.click(&game.menu_env(), action.0, action.1, action.2)
        .unwrap();
    let predicted = asked();
    click(game, rx, action, Vec::new(), None);
    (menu, predicted, asked())
}

/// `click` from `slots` and `carried`, on the server and in the client's prediction: what each
/// leaves, and what the guest is sent back when its client claims the shown slot emptied.
fn refused(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    rx: &mpsc::Receiver<Out>,
    (slots, carried): (&[(usize, Option<ItemStack>)], Option<ItemStack>),
    action: (i32, i32, Input),
) -> (Shown, Shown, Vec<(i16, u32)>) {
    let mut menu = prediction(slots, carried);
    serve(game, &menu);
    let before = predicted(&menu);
    menu.click(&game.menu_env(), action.0, action.1, action.2)
        .unwrap();
    assert_eq!(predicted(&menu), before, "the prediction refuses the take");
    let carried = menu.carried.as_ref().map(|c| c.count);
    let carried = carried.and_then(|count| claim(game, count));
    let sent = click(game, rx, action, vec![(SHOWN as i16, None)], carried);
    (before, served(game), sent)
}

#[test]
#[ignore = "daily"]
fn a_refused_pickup_click_takes_and_merges_nothing_and_the_slot_is_sent_back() {
    with_pot(|game, rx| {
        let held = [(SHOWN, stone(5))];
        // Either button with an empty cursor, and the left button onto a matching cursor:
        // nothing leaves the slot. A client that predicted the take is sent the slot back.
        for (carried, button) in [(None, 0), (None, 1), (stone(3), 0)] {
            asked();
            let (before, after, sent) = refused(
                game,
                rx,
                (&held, carried.clone()),
                (SHOWN as i32, button, Input::Pickup),
            );
            assert_eq!(after, before, "{carried:?} {button}");
            assert!(sent.contains(&(SHOWN as i16, 5)), "{sent:?}");
            // The rule was asked for the stack the slot holds.
            assert!(asked().contains(&(SHOWN, "minecraft:stone".into(), 5)));
        }
        // A client that predicted the five stone on its cursor is sent the empty cursor.
        let mut menu = prediction(&held, None);
        serve(game, &menu);
        menu.click(&game.menu_env(), SHOWN as i32, 0, Input::Pickup)
            .unwrap();
        let cursor = claim(game, 5);
        let sent = click(
            game,
            rx,
            (SHOWN as i32, 0, Input::Pickup),
            vec![(SHOWN as i16, None)],
            cursor,
        );
        assert_eq!(sent, [(SHOWN as i16, 5), (-1, 0)]);
    });
}

#[test]
#[ignore = "daily"]
fn an_allowed_pickup_takes_the_stack_on_both_sides() {
    with_pot(|game, rx| {
        let mut menu = prediction(&[(0, stone(5))], None);
        serve(game, &menu);
        asked();
        menu.click(&game.menu_env(), 0, 0, Input::Pickup).unwrap();
        let predicted_asks = asked();
        assert_eq!(menu.slot(0), None);
        assert_eq!(menu.carried.as_ref().map(|c| c.count), Some(5));
        let cursor = claim(game, 5);
        let sent = click(game, rx, (0, 0, Input::Pickup), vec![(0, None)], cursor);
        // The server agrees with the prediction: nothing is sent back, and it asked the same.
        assert!(sent.is_empty(), "{sent:?}");
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(asked(), predicted_asks);
        assert!(predicted_asks.contains(&(0, "minecraft:stone".into(), 5)));
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_quick_move_moves_nothing_and_asks_before_menu_quick_move() {
    with_pot(|game, rx| {
        MOVED.with(|moved| moved.borrow_mut().clear());
        let held = [(SHOWN, stone(5))];
        let (before, after, sent) =
            refused(game, rx, (&held, None), (SHOWN as i32, 0, Input::QuickMove));
        assert_eq!(after, before);
        assert!(sent.contains(&(SHOWN as i16, 5)), "{sent:?}");
        // The pickup rule refused before the block's shift-click was asked.
        assert!(MOVED.with(|moved| moved.borrow().is_empty()));
        // An allowed slot is asked the block's shift-click, which leaves it to the game: the
        // stack moves to the inventory, as the prediction moved it.
        let mut menu = prediction(&[(0, stone(5))], None);
        serve(game, &menu);
        menu.click(&game.menu_env(), 0, 0, Input::QuickMove)
            .unwrap();
        click(game, rx, (0, 0, Input::QuickMove), Vec::new(), None);
        assert_eq!(MOVED.with(|moved| moved.borrow().clone()), [0]);
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(menu.slot(0), None);
        // The server's prepared menu checks the pickup rule once before `menu_quick_move`;
        // when the override passes, the same menu continues without repeating that check.
        let (menu, client_asks, server_asks) =
            asked_by_both(game, rx, (&[(0, stone(5))], None), (0, 0, Input::QuickMove));
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(server_asks, client_asks);
        // Reverse inventory traversal inserts into hotbar slot 8 (inventory index 8).
        assert_eq!(menu.player_item(8).map(|s| s.count), Some(5));
        assert_eq!(
            times(&server_asks, (8, "minecraft:stone", 5)),
            2,
            "{server_asks:?}"
        );
        let from = SLOTS + 21;
        MOVED.with(|moved| moved.borrow_mut().clear());
        let (menu, client_asks, server_asks) = asked_by_both(
            game,
            rx,
            (&[(from, stone(5))], None),
            (from as i32, 0, Input::QuickMove),
        );
        assert_eq!(MOVED.with(|moved| moved.borrow().clone()), [from]);
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(server_asks, client_asks);
        assert_eq!(
            times(&server_asks, (30, "minecraft:stone", 5)),
            1,
            "{server_asks:?}"
        );
        // A refused shift-click refuses before the hook and consumes the click without re-asking.
        MOVED.with(|moved| moved.borrow_mut().clear());
        let action = (SHOWN as i32, 0, Input::QuickMove);
        let (_, client_asks, server_asks) =
            asked_by_both(game, rx, (&[(SHOWN, stone(5))], None), action);
        assert_eq!(client_asks, [(SHOWN, "minecraft:stone".to_owned(), 5)]);
        assert_eq!(server_asks, client_asks);
        assert!(MOVED.with(|moved| moved.borrow().is_empty()));

        // A stateful pickup rule whose answer flips between calls is still only asked once:
        // false -> true stays refused; true -> false still completes the fallback move.
        MOVED.with(|moved| moved.borrow_mut().clear());
        let menu = prediction(&[(0, stone(5))], None);
        serve(game, &menu);
        asked();
        SLOT0_PICKUP.with(|seq| *seq.borrow_mut() = vec![false, true]);
        click(game, rx, (0, 0, Input::QuickMove), Vec::new(), None);
        assert_eq!(served(game), predicted(&menu));
        assert!(MOVED.with(|moved| moved.borrow().is_empty()));
        assert_eq!(asked(), [(0, "minecraft:stone".to_owned(), 5)]);
        assert_eq!(SLOT0_PICKUP.with(|seq| seq.borrow().len()), 1);

        MOVED.with(|moved| moved.borrow_mut().clear());
        let mut expected = prediction(&[(0, stone(5))], None);
        serve(game, &expected);
        SLOT0_PICKUP.with(|seq| *seq.borrow_mut() = vec![true]);
        expected
            .click(&game.menu_env(), 0, 0, Input::QuickMove)
            .unwrap();
        asked();
        SLOT0_PICKUP.with(|seq| *seq.borrow_mut() = vec![true, false]);
        click(game, rx, (0, 0, Input::QuickMove), Vec::new(), None);
        assert_eq!(MOVED.with(|moved| moved.borrow().clone()), [0]);
        assert_eq!(served(game), predicted(&expected));
        assert_eq!(SLOT0_PICKUP.with(|seq| seq.borrow().len()), 1);
        SLOT0_PICKUP.with(|seq| seq.borrow_mut().clear());
    });
}

fn link_pot(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    let state = game.world.block(POS.0, POS.1, POS.2);
    game.world.set(LINK.0, LINK.1, LINK.2, state).unwrap();
    game.world.block_entities.insert(
        LINK,
        BlockEntity::Mod(Box::new(ModBlockEntity::new(
            POT_TYPE.into(),
            ModBlockEntityFields {
                items: vec![stone(7)],
                ..Default::default()
            },
        ))),
    );
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |guest| {
            let menu = guest.menu.as_mut().unwrap();
            menu.mod_type = Some(Box::leak(Box::new(ModMenuType {
                links: Some(ModLinks {
                    from: LINKED_SLOT,
                    count: 1,
                }),
                ..pot_type().clone()
            })));
            menu.links = vec![LINK];
        });
}

#[test]
#[ignore = "daily"]
fn an_unhandled_quick_move_uses_the_live_linked_neighbor_for_its_only_pickup_check() {
    with_pot(|game, rx| {
        serve(game, &prediction(&[(0, stone(5))], None));
        link_pot(game);
        asked();
        MOVED.with(|moved| moved.borrow_mut().clear());
        NEEDS_LINKED_NEIGHBOR.set(true);
        click(game, rx, (0, 0, Input::QuickMove), Vec::new(), None);
        NEEDS_LINKED_NEIGHBOR.set(false);
        assert_eq!(MOVED.with(|moved| moved.borrow().clone()), [0]);
        assert_eq!(times(&asked(), (0, "minecraft:stone", 5)), 1);
        assert_eq!(
            game.world.block_entities.get(&POS).unwrap().slots()[0],
            None
        );
        assert_eq!(
            game.world.block_entities.get(&LINK).unwrap().slots()[0],
            stone(7)
        );
        assert_eq!(
            game.server.lan.as_ref().unwrap().hub.guests[0].inventory[8],
            stone(5)
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_handled_quick_move_writes_its_remainder_to_the_link_not_the_owner() {
    with_pot(|game, rx| {
        serve(game, &prediction(&[(LINKED_SLOT, stone(2))], None));
        link_pot(game);
        QUICK_MOVE_LEFT.set(Some(3));
        click(
            game,
            rx,
            (LINKED_SLOT as i32, 0, Input::QuickMove),
            Vec::new(),
            None,
        );
        assert_eq!(QUICK_MOVE_SOURCE.take(), Some(7));
        assert_eq!(
            game.world.block_entities.get(&LINK).unwrap().slots()[0],
            stone(3)
        );
        assert_eq!(
            game.world.block_entities.get(&POS).unwrap().slots()[LINKED_SLOT],
            stone(2)
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_quick_move_rolls_back_both_ends_if_its_linked_source_changes() {
    with_pot(|game, rx| {
        link_pot(game);
        let generation = game.world.block_entities.generation(&LINK).unwrap();
        let block = game.world.block(LINK.0, LINK.1, LINK.2);
        for change in [LinkChange::Insert, LinkChange::Remove] {
            QUICK_MOVE_LEFT.set(Some(3));
            LINK_CHANGE.set(Some(change));
            click(
                game,
                rx,
                (LINKED_SLOT as i32, 0, Input::QuickMove),
                Vec::new(),
                None,
            );
            assert!(
                LINK_CHANGED.take(),
                "the callback reached the source: {change:?}"
            );
            assert_eq!(QUICK_MOVE_SOURCE.take(), Some(7));
            assert_eq!(game.world.block(LINK.0, LINK.1, LINK.2), block);
            assert_eq!(
                game.world.block_entities.generation(&LINK),
                Some(generation)
            );
            assert_eq!(
                game.world.block_entities.get(&LINK).unwrap().slots()[0],
                stone(7)
            );
            assert_eq!(
                game.world.block_entities.get(&POS).unwrap().slots()[0],
                None
            );
            assert!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .inventory
                    .iter()
                    .all(Option::is_none)
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_cached_machine_menu_reopens_a_linked_slot_when_its_backing_appears() {
    with_pot(|game, rx| {
        serve(game, &prediction(&[(0, stone(5))], stone(2)));
        link_pot(game);
        let backing = game.world.block_entities.remove(&LINK).unwrap();
        MOVED.with(|moved| moved.borrow_mut().clear());
        // Cache the machine menu during a drag while the link has no backing.
        click(game, rx, (-999, 0, Input::QuickCraft), Vec::new(), None);
        game.world.block_entities.insert(LINK, backing);
        // The first shift-click ends the drag. The link becoming available must not reset it.
        click(
            game,
            rx,
            (LINKED_SLOT as i32, 0, Input::QuickMove),
            Vec::new(),
            None,
        );
        assert!(MOVED.with(|moved| moved.borrow().is_empty()));
        assert_eq!(
            game.world.block_entities.get(&LINK).unwrap().slots()[0],
            stone(7)
        );
        click(
            game,
            rx,
            (LINKED_SLOT as i32, 0, Input::QuickMove),
            Vec::new(),
            None,
        );
        assert_eq!(MOVED.with(|moved| moved.borrow().clone()), [LINKED_SLOT]);
        assert_eq!(
            game.world.block_entities.get(&LINK).unwrap().slots()[0],
            None
        );
        assert_eq!(
            game.server.lan.as_ref().unwrap().hub.guests[0].inventory[8],
            stone(7)
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_handled_quick_move_reads_and_leaves_the_shown_crafting_result() {
    with_pot(|game, rx| {
        let craft = ModCrafting {
            grid: 5,
            size: 2,
            result: RESULT,
        };
        let log = Some(ItemStack::new("minecraft:oak_log", 1));
        serve(game, &prediction(&[(craft.grid, log.clone())], None));
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| {
                let menu = guest.menu.as_mut().unwrap();
                menu.mod_type = Some(Box::leak(Box::new(ModMenuType {
                    crafting: Some(craft),
                    ..pot_type().clone()
                })));
                menu.craft = Some(ModCraft {
                    grid: vec![log, None, None, None],
                    result: stone(5),
                });
            });
        QUICK_MOVE_LEFT.set(Some(3));
        QUICK_MOVE_CONSUME.set(Some(craft.grid));
        click(
            game,
            rx,
            (RESULT as i32, 0, Input::QuickMove),
            Vec::new(),
            None,
        );
        assert_eq!(QUICK_MOVE_SOURCE.take(), Some(5));
        let block = game.world.block_entities.get(&POS).unwrap();
        assert_eq!(block.slots()[craft.grid], None);
        assert_eq!(block.slots()[RESULT], None);

        game.lan_mod_crafting_tick();
        let host = game.server.lan.as_ref().unwrap();
        assert_eq!(host.hub.view_slot(0, &game.world, RESULT), stone(3));
        // The next shift-click is given what the player still sees, not a recipe lookup on an
        // empty grid. The handler can empty the result without re-consuming an input.
        QUICK_MOVE_LEFT.set(Some(0));
        click(
            game,
            rx,
            (RESULT as i32, 0, Input::QuickMove),
            Vec::new(),
            None,
        );
        assert_eq!(QUICK_MOVE_SOURCE.take(), Some(3));
        let host = game.server.lan.as_ref().unwrap();
        assert_eq!(host.hub.view_slot(0, &game.world, RESULT), None);

        // A subsequent input change invalidates the published result. Even before the next
        // tick, a click reads the newly prepared result (one oak log makes four planks).
        game.world.block_entities.get_mut(&POS).unwrap().slots_mut()[craft.grid] =
            Some(ItemStack::new("minecraft:oak_log", 1));
        QUICK_MOVE_LEFT.set(Some(2));
        click(
            game,
            rx,
            (RESULT as i32, 0, Input::QuickMove),
            Vec::new(),
            None,
        );
        assert_eq!(QUICK_MOVE_SOURCE.take(), Some(4));
        let host = game.server.lan.as_ref().unwrap();
        assert_eq!(
            host.hub.view_slot(0, &game.world, RESULT),
            Some(ItemStack::new("minecraft:oak_planks", 2))
        );
    });
}

#[test]
#[ignore = "daily"]
fn quick_move_projection_failures_report_every_time_and_refuse_the_move() {
    // Use a separate process because the test logger is process-global.
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::slot_pickup_tests::quick_move_projection_failures_report_every_time_and_refuse_the_move",
        "SUNRISE_TEST_QUICK_MOVE_PROJECTION_CHILD",
        &["projection"],
    ) {
        return;
    }
    struct Capture(std::sync::Mutex<Vec<String>>);
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            let message = record.args().to_string();
            if message.starts_with("menu_quick_move: cannot project ") {
                self.0.lock().unwrap().push(message);
            }
        }
        fn flush(&self) {}
    }
    static LOG: Capture = Capture(std::sync::Mutex::new(Vec::new()));
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Warn);

    with_pot(|game, rx| {
        let unknown = Some(ItemStack::new("pot:unregistered", 7));
        link_pot(game);
        MOVED.with(|moved| moved.borrow_mut().clear());
        // The clicked stack comes from the link, not the pot. If it cannot cross the
        // API boundary, neither the listener nor ordinary quick-move routing runs.
        game.world
            .block_entities
            .get_mut(&LINK)
            .unwrap()
            .slots_mut()[0] = unknown.clone();
        for _ in 0..2 {
            click(
                game,
                rx,
                (LINKED_SLOT as i32, 0, Input::QuickMove),
                Vec::new(),
                None,
            );
            assert_eq!(
                game.world.block_entities.get(&LINK).unwrap().slots()[0],
                unknown
            );
        }
        assert!(MOVED.with(|moved| moved.borrow().is_empty()));
        assert!(
            game.server.lan.as_ref().unwrap().hub.guests[0]
                .inventory
                .iter()
                .all(Option::is_none)
        );

        // With a valid clicked stack, an unread row no longer vetoes a live subject loan.
        game.world
            .block_entities
            .get_mut(&LINK)
            .unwrap()
            .slots_mut()[0] = stone(7);
        serve(game, &prediction(&[(0, stone(5)), (KEEPS, stone(1))], None));
        game.world.block_entities.get_mut(&POS).unwrap().slots_mut()[SHOWN] = unknown.clone();
        QUICK_MOVE_LEFT.set(Some(2));
        QUICK_MOVE_CONSUME.set(Some(KEEPS));
        click(game, rx, (0, 0, Input::QuickMove), Vec::new(), None);
        assert_eq!(QUICK_MOVE_SOURCE.take(), Some(5));
        let slots = game.world.block_entities.get(&POS).unwrap().slots();
        assert_eq!(slots[0], stone(2));
        assert_eq!(slots[SHOWN], unknown);
        assert_eq!(slots[KEEPS], None);
        assert_eq!(MOVED.with(|moved| moved.borrow().clone()), [0]);

        let records = LOG.0.lock().unwrap();
        assert_eq!(records.len(), 2, "{records:?}");
        assert!(
            records
                .iter()
                .all(|s| s.contains("clicked stack") && s.contains("UnknownItem")),
            "{records:?}"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_quick_move_without_menu_quick_move_asks_once() {
    with_pot_mod(false, |game, rx| {
        // No listener: the game's routing asks alone, once for the clicked stack, as the
        // prediction does, refused or not.
        let (menu, client_asks, server_asks) =
            asked_by_both(game, rx, (&[(0, stone(5))], None), (0, 0, Input::QuickMove));
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(server_asks, client_asks);
        // Reverse inventory traversal inserts into hotbar slot 8 (inventory index 8).
        assert_eq!(menu.player_item(8).map(|s| s.count), Some(5));
        assert_eq!(
            times(&server_asks, (8, "minecraft:stone", 5)),
            2,
            "{server_asks:?}"
        );
        let from = SLOTS + 21;
        let (menu, client_asks, server_asks) = asked_by_both(
            game,
            rx,
            (&[(from, stone(5))], None),
            (from as i32, 0, Input::QuickMove),
        );
        assert_eq!(menu.player_item(30), None);
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(server_asks, client_asks);
        assert_eq!(
            times(&server_asks, (30, "minecraft:stone", 5)),
            1,
            "{server_asks:?}"
        );
        let action = (SHOWN as i32, 0, Input::QuickMove);
        let (_, client_asks, server_asks) =
            asked_by_both(game, rx, (&[(SHOWN, stone(5))], None), action);
        assert_eq!(server_asks, client_asks);
        assert_eq!(server_asks, [(SHOWN, "minecraft:stone".to_owned(), 5)]);
    });
}

#[test]
#[ignore = "daily"]
fn each_path_asks_the_rule_once_per_slot_and_stack_on_both_sides() {
    with_pot(|game, rx| {
        let hotbar = SLOTS + 27;
        let dirt = || Some(ItemStack::new("minecraft:dirt", 2));
        let (stone5, dirt2) = (("minecraft:stone", 5), ("minecraft:dirt", 2));
        // Pot slot 3 answers true, the shown slot refuses. Each row: the slots and cursor, the
        // click, and how often vanilla's live `mayPickup` / `tryRemove` / `allowModification` /
        // `mayPlace` / `getMaxStackSize` checks ask each (slot, item, count) on each side.
        type Row = (
            Vec<(usize, Option<ItemStack>)>,
            Option<ItemStack>,
            (i32, i32, Input),
            Vec<(usize, (&'static str, u32), usize)>,
        );
        let rows: Vec<Row> = vec![
            // Pickup with either button: `mayPickup` + `tryRemove` (`mayPickup`, `allowModification`).
            (
                vec![(3, stone(5))],
                None,
                (3, 0, Input::Pickup),
                vec![(3, stone5, 4)],
            ),
            (
                vec![(3, stone(5))],
                None,
                (3, 1, Input::Pickup),
                vec![(3, stone5, 4)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 0, Input::Pickup),
                vec![(SHOWN, stone5, 1)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 1, Input::Pickup),
                vec![(SHOWN, stone5, 1)],
            ),
            // A matching cursor: `mayPickup` on held, then `mayPlace` and `safeInsert` on carried.
            (
                vec![(3, stone(5))],
                stone(3),
                (3, 0, Input::Pickup),
                vec![(3, stone5, 1), (3, ("minecraft:stone", 3), 3)],
            ),
            (
                vec![(SHOWN, stone(5))],
                stone(3),
                (1, 0, Input::Pickup),
                vec![(SHOWN, stone5, 1), (SHOWN, ("minecraft:stone", 3), 0)],
            ),
            // Hotbar and offhand swaps; with both stacks present the hotbar's is asked for
            // `mayPlace` and `getMaxStackSize`, unless the pickup is refused first.
            (
                vec![(3, stone(5))],
                None,
                (3, 0, Input::Swap),
                vec![(3, stone5, 1)],
            ),
            (
                vec![(3, stone(5)), (hotbar, dirt())],
                None,
                (3, 0, Input::Swap),
                vec![(3, stone5, 1), (3, dirt2, 2)],
            ),
            (
                vec![(3, stone(5))],
                None,
                (3, 40, Input::Swap),
                vec![(3, stone5, 1)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 0, Input::Swap),
                vec![(SHOWN, stone5, 1)],
            ),
            (
                vec![(SHOWN, stone(5)), (hotbar, dirt())],
                None,
                (1, 0, Input::Swap),
                vec![(SHOWN, stone5, 1), (SHOWN, dirt2, 0)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 40, Input::Swap),
                vec![(SHOWN, stone5, 1)],
            ),
            // Throws of one and of all: `safeTake` -> `tryRemove` (`mayPickup`, `allowModification`).
            (
                vec![(3, stone(5))],
                None,
                (3, 0, Input::Throw),
                vec![(3, stone5, 3)],
            ),
            (
                vec![(3, stone(5))],
                None,
                (3, 1, Input::Throw),
                vec![(3, stone5, 3)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 0, Input::Throw),
                vec![(SHOWN, stone5, 1)],
            ),
            (
                vec![(SHOWN, stone(5))],
                None,
                (1, 1, Input::Throw),
                vec![(SHOWN, stone5, 1)],
            ),
            // Pick-all on the refusing slot and on an empty one: each pass checks live `mayPickup`
            // and `safeTake` calls `tryRemove`.
            (
                vec![(0, stone(3)), (SHOWN, stone(5))],
                stone(1),
                (1, 0, Input::PickupAll),
                vec![(0, ("minecraft:stone", 3), 4), (SHOWN, stone5, 3)],
            ),
            (
                vec![(0, stone(3)), (SHOWN, stone(5))],
                stone(1),
                (5, 0, Input::PickupAll),
                vec![(0, ("minecraft:stone", 3), 4), (SHOWN, stone5, 2)],
            ),
            // The result slot onto a matching cursor, refused whole and taken whole.
            (
                vec![(RESULT, stone(10))],
                stone(60),
                (2, 0, Input::Pickup),
                vec![
                    (RESULT, ("minecraft:stone", 10), 4),
                    (RESULT, ("minecraft:stone", 60), 1),
                ],
            ),
            (
                vec![(RESULT, stone(10))],
                stone(50),
                (2, 0, Input::Pickup),
                vec![
                    (RESULT, ("minecraft:stone", 10), 4),
                    (RESULT, ("minecraft:stone", 50), 1),
                ],
            ),
        ];
        for (slots, carried, action, expected) in rows {
            let (menu, client_asks, server_asks) =
                asked_by_both(game, rx, (&slots, carried.clone()), action);
            let row = format!("{action:?} {carried:?}");
            assert_eq!(served(game), predicted(&menu), "{row}");
            assert_eq!(server_asks, client_asks, "{row}");
            for (slot, (item, count), n) in expected {
                assert_eq!(
                    times(&server_asks, (slot, item, count)),
                    n,
                    "{row} {server_asks:?}"
                );
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_swap_keeps_both_stacks() {
    with_pot(|game, rx| {
        let hotbar = SLOTS + 27;
        let dirt = Some(ItemStack::new("minecraft:dirt", 2));
        // An empty hotbar slot, one that holds dirt (both stacks present) and the empty off
        // hand (button 40): no exchange.
        for (other, button) in [(None, 0), (dirt, 0), (None, 40)] {
            let held = [(SHOWN, stone(5)), (hotbar, other.clone())];
            let (before, after, sent) =
                refused(game, rx, (&held, None), (SHOWN as i32, button, Input::Swap));
            assert_eq!(after, before, "{other:?} {button}");
            assert!(sent.contains(&(SHOWN as i16, 5)), "{sent:?}");
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_throw_drops_nothing() {
    with_pot(|game, rx| {
        let held = [(SHOWN, stone(5))];
        // One item, and the whole stack (the control variant repeats while it takes).
        for button in [0, 1] {
            let dropped = game.dropped.len();
            let (before, after, sent) = refused(
                game,
                rx,
                (&held, None),
                (SHOWN as i32, button, Input::Throw),
            );
            assert_eq!(after, before, "{button}");
            assert!(sent.contains(&(SHOWN as i16, 5)), "{sent:?}");
            assert_eq!(game.dropped.len(), dropped, "nothing was thrown");
        }
    });
}

#[test]
#[ignore = "daily"]
fn pick_all_on_a_refusing_slot_gathers_from_the_others_and_skips_it() {
    with_pot(|game, rx| {
        // The double click lands on the shown slot, which refuses, then on an empty slot: either
        // way the cursor gathers the stone of slot 0 and the shown slot keeps its own.
        for clicked in [SHOWN, 5] {
            let mut menu = prediction(&[(0, stone(3)), (SHOWN, stone(5))], stone(1));
            serve(game, &menu);
            menu.click(&game.menu_env(), clicked as i32, 0, Input::PickupAll)
                .unwrap();
            assert_eq!(menu.slot(0), None);
            assert_eq!(menu.slot(SHOWN).map(|s| s.count), Some(5));
            assert_eq!(menu.carried.as_ref().map(|c| c.count), Some(4));
            let cursor = claim(game, 4);
            let action = (clicked as i32, 0, Input::PickupAll);
            let sent = click(game, rx, action, vec![(0, None)], cursor);
            assert!(sent.is_empty(), "{sent:?}");
            assert_eq!(served(game), predicted(&menu));
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_menu_part_keeps_a_slot_out_of_pick_all() {
    with_pot(|game, rx| {
        // The pot keeps slot 0 out of a gather: a double click on an empty slot gathers the stone
        // of slot 5 and leaves slot 0's, in the prediction and on the server alike.
        PICK_ALL_KEEPS.set(Some(0));
        PICK_ALL_ASKED.take();
        let mut menu = prediction(&[(0, stone(3)), (5, stone(2))], stone(1));
        serve(game, &menu);
        menu.click(&game.menu_env(), 6, 0, Input::PickupAll)
            .unwrap();
        assert_eq!(menu.slot(0).map(|s| s.count), Some(3));
        assert_eq!(menu.slot(5), None);
        assert_eq!(menu.carried.as_ref().map(|c| c.count), Some(3));
        // The part heard the slots the gather would take, with the cursor gathering into.
        let heard = PICK_ALL_ASKED.take();
        assert!(heard.contains(&(0, 3, 1)), "{heard:?}");
        assert!(heard.contains(&(5, 2, 1)), "{heard:?}");
        let cursor = claim(game, 3);
        let sent = click(game, rx, (6, 0, Input::PickupAll), vec![(5, None)], cursor);
        assert!(sent.is_empty(), "{sent:?}");
        assert_eq!(served(game), predicted(&menu));
        // Without the part's refusal both slots are gathered.
        PICK_ALL_KEEPS.set(None);
        let mut menu = prediction(&[(0, stone(3)), (5, stone(2))], stone(1));
        menu.click(&game.menu_env(), 6, 0, Input::PickupAll)
            .unwrap();
        assert_eq!((menu.slot(0), menu.slot(5)), (None, None));
        assert_eq!(menu.carried.as_ref().map(|c| c.count), Some(6));
    });
}

#[test]
#[ignore = "daily"]
fn a_drag_that_ends_on_one_slot_is_a_pickup_click_and_asks() {
    with_pot(|game, rx| {
        // Eight stone dragged onto a slot that takes stone but gives nothing up, holding five:
        // the drag becomes a pickup click on it, which asks for its five and merges nothing.
        let mut menu = prediction(&[(KEEPS, stone(5))], stone(8));
        serve(game, &menu);
        let before = predicted(&menu);
        asked();
        for (slot, button) in [(-999, 0), (KEEPS as i32, 1), (-999, 2)] {
            menu.click(&game.menu_env(), slot, button, Input::QuickCraft)
                .unwrap();
            click(
                game,
                rx,
                (slot, button, Input::QuickCraft),
                Vec::new(),
                None,
            );
        }
        assert_eq!(predicted(&menu), before);
        assert_eq!(served(game), before);
        assert!(asked().contains(&(KEEPS, "minecraft:stone".into(), 5)));
    });
}

#[test]
#[ignore = "daily"]
fn a_result_slot_gives_all_or_nothing_to_a_matching_cursor() {
    with_pot(|game, rx| {
        // Ten stone in the result, sixty on the cursor: only four fit, so nothing is taken.
        let held = [(RESULT, stone(10))];
        let (before, after, _) = refused(
            game,
            rx,
            (&held, stone(60)),
            (RESULT as i32, 0, Input::Pickup),
        );
        assert_eq!(after, before);
        // With room for all ten, the cursor takes them.
        let mut menu = prediction(&held, stone(50));
        serve(game, &menu);
        menu.click(&game.menu_env(), RESULT as i32, 0, Input::Pickup)
            .unwrap();
        assert_eq!(menu.carried.as_ref().map(|c| c.count), Some(60));
        click(
            game,
            rx,
            (RESULT as i32, 0, Input::Pickup),
            Vec::new(),
            None,
        );
        assert_eq!(served(game), predicted(&menu));
    });
}

#[test]
#[ignore = "daily"]
fn an_ordinary_slot_gives_on_every_path_as_predicted() {
    with_pot(|game, rx| {
        // Quick move, a hotbar and an off-hand swap, a throw of one and of all, on slot 0 that
        // answers true: the server does what the prediction did.
        for (button, input) in [
            (0, Input::QuickMove),
            (0, Input::Swap),
            (40, Input::Swap),
            (0, Input::Throw),
            (1, Input::Throw),
        ] {
            let mut menu = prediction(&[(0, stone(5))], None);
            serve(game, &menu);
            menu.click(&game.menu_env(), 0, button, input).unwrap();
            assert_ne!(menu.slot(0).map(|s| s.count), Some(5), "{button}");
            click(game, rx, (0, button, input), Vec::new(), None);
            assert_eq!(served(game), predicted(&menu), "{button}");
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_drag_and_a_creative_clone_never_ask_the_pickup_rule() {
    with_pot(|game, rx| {
        // A drag of eight stone over slot 0, the shown slot (holding five) and slot 2: only
        // placement and capacity are asked, each for the carried eight, never for the shown
        // slot's five; the shown slot refuses, the other two take four each.
        let mut menu = prediction(&[(SHOWN, stone(5))], stone(8));
        serve(game, &menu);
        asked();
        for (slot, button) in [(-999, 0), (0, 1), (SHOWN as i32, 1), (3, 1), (-999, 2)] {
            menu.click(&game.menu_env(), slot, button, Input::QuickCraft)
                .unwrap();
            click(
                game,
                rx,
                (slot, button, Input::QuickCraft),
                Vec::new(),
                None,
            );
        }
        let asks = asked();
        assert!(!asks.is_empty());
        assert!(asks.iter().all(|(_, _, count)| *count == 8), "{asks:?}");
        assert_eq!(served(game), predicted(&menu));
        assert_eq!(menu.slot(0).map(|s| s.count), Some(4));
        assert_eq!(menu.slot(SHOWN).map(|s| s.count), Some(5));
        assert_eq!(menu.slot(3).map(|s| s.count), Some(4));
        // A creative clone of the shown slot fills the cursor and asks nothing.
        let mut menu = prediction(&[(SHOWN, stone(5))], None);
        menu.creative = true;
        serve(game, &menu);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| {
                guest.game_mode = sunrise_sim::lan::guest::CREATIVE;
            });
        menu.click(&game.menu_env(), SHOWN as i32, 2, Input::Clone)
            .unwrap();
        click(game, rx, (SHOWN as i32, 2, Input::Clone), Vec::new(), None);
        assert_eq!(menu.carried.as_ref().map(|c| c.count), Some(64));
        assert_eq!(served(game), predicted(&menu));
        assert!(asked().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn quick_move_and_live_rules_see_the_same_prepared_result_until_the_grid_changes() {
    with_pot(|game, rx| {
        let craft = ModCrafting {
            grid: 5,
            size: 2,
            result: RESULT,
        };
        let input = stone(1);
        let entity = game.world.block_entities.get_mut(&POS).unwrap();
        entity.slots_mut()[0] = stone(8);
        entity.slots_mut()[craft.grid] = input.clone();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| {
                let menu = guest.menu.as_mut().unwrap();
                menu.mod_type = Some(Box::leak(Box::new(ModMenuType {
                    crafting: Some(craft),
                    ..pot_type().clone()
                })));
                menu.craft = Some(ModCraft {
                    grid: vec![input, None, None, None],
                    result: Some(ItemStack::new("minecraft:diamond", 1)),
                });
            });
        let player = game.player(12).unwrap();
        let menu = player.container_menu(game).unwrap();
        let result = menu.slot(game, RESULT).unwrap().unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(result.item()),
            Some("minecraft:diamond")
        );
        assert_eq!(menu.slot_gives(game, 0), Some(false));
        MOVED.with(|moved| moved.borrow_mut().clear());
        click(game, rx, (0, 0, Input::QuickMove), vec![], None);
        assert!(MOVED.with(|moved| moved.borrow().is_empty()));
        assert_eq!(served(game)[0], Some(("minecraft:stone".to_owned(), 8)));

        // Before the next sync, the live grid crafts oak planks, not the old preview.
        game.world.block_entities.get_mut(&POS).unwrap().slots_mut()[5] =
            Some(ItemStack::new("minecraft:oak_log", 1));
        assert_eq!(menu.slot_gives(game, 0), Some(true));
        click(game, rx, (0, 0, Input::QuickMove), vec![], None);
        assert!(MOVED.with(|moved| moved.borrow().as_slice() == [0]));
        assert_eq!(served(game)[0], None);
    });
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
