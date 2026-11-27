//! Parts attached to the player's own `InventoryMenu` (`minecraft:inventory`): `menu_slot_rules`
//! on the server (a LAN guest's window 0) and in the client's prediction, every mod's Part with
//! any refusal standing, `menu_clicked`, and `recipe_book_place` taking the 2×2 grid's placement
//! over. The fixtures need prepared resources.
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{
    Flow, Hooks, MenuClickedEvent, MenuContainer, MenuPart, MenuSlotRulesEvent, ModEntry, Mods,
    RecipeBookPlaceEvent, Registrar, ServerHost,
};
use std::cell::{Cell, RefCell};
use sunrise_client_gameplay::game::ClientGame;
use sunrise_gameplay::game::{ServerGame, native_guest_turn_tests::with_guests};
use sunrise_gameplay_common::game::tables::GameItems as _;
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::{
    play::{ContainerClick, Serverbound},
    recipes::PlaceRecipe,
};
use sunrise_sim::menu::{INVENTORY_MENU, Input, Kind, Menu};

/// The hotbar slots the lock refuses, as `Inventory` indexes them: hotbar slots 7 to 9.
const LOCKED: std::ops::Range<usize> = 6..9;
/// `InventoryMenu`'s first hotbar slot: the menu shows hotbar slot `i` at `HOTBAR + i`.
const HOTBAR: usize = 36;

thread_local! {
    /// The menu slots `menu_clicked` was heard for.
    static CLICKED: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    /// Whether `menu_clicked` handles the click itself.
    static HANDLES: Cell<bool> = const { Cell::new(false) };
}

/// Refuses placement into and pickup from the locked hotbar slots.
struct Lock;
#[mod_api::sunrise_part]
impl MenuPart for Lock {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        if event.container == MenuContainer::Player && LOCKED.contains(&event.slot) {
            event.may_place = false;
            event.may_pickup = false;
        }
    }
}
/// Allows every slot, whatever it was told.
struct Allow;
#[mod_api::sunrise_part]
impl MenuPart for Allow {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        event.may_place = true;
        event.may_pickup = true;
    }
}
/// Hears clicks, and handles them itself when `HANDLES` says so.
struct Clicks;
#[mod_api::sunrise_part]
impl MenuPart for Clicks {
    fn menu_clicked(&self, _: &mut dyn ServerHost, event: &mut MenuClickedEvent<'_>) -> Flow {
        assert_eq!((event.menu, event.window), (INVENTORY_MENU, 0));
        CLICKED.with(|clicked| clicked.borrow_mut().push(event.slot));
        if HANDLES.get() {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}
/// Takes the recipe book's placement over and places nothing: the 2×2 grid is locked.
struct NoPlacement;
#[mod_api::sunrise_part]
impl MenuPart for NoPlacement {
    fn recipe_book_place(&self, _: &mut dyn ServerHost, event: &mut RecipeBookPlaceEvent<'_>) {
        assert_eq!(event.menu, INVENTORY_MENU);
        *event.placement = mod_api::RecipeBookPlacement::Nothing;
    }
}

/// A mod attaching one Part to the inventory menu.
struct InventoryMod(fn(&mut dyn Registrar));
#[mod_api::__subscriptions]
impl Hooks for InventoryMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        (self.0)(r);
    }
}

/// Install `mods` in order with shared menu parts and client hook wiring.
fn with_inventory_mods_scope(
    mods: Vec<(&'static str, fn(&mut dyn Registrar))>,
    test: impl FnOnce(),
) {
    let mods: Mods = mods
        .into_iter()
        .map(|(id, register)| ModEntry::new(id, Box::new(InventoryMod(register))))
        .collect::<Vec<_>>()
        .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    crate::entry::install_client_hooks();
    mod_platform::testing::with_mods(mods, test);
}

/// Guest 12, with nothing but the inventory open, and `mods` installed in that order.
fn with_inventory_mods(
    mods: Vec<(&'static str, fn(&mut dyn Registrar))>,
    test: impl FnOnce(&mut ServerGame<'_>),
) {
    with_inventory_mods_scope(mods, || {
        with_guests(|game| {
            assert!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .is_none()
            );
            test(game);
        })
    });
}

fn lock(r: &mut dyn Registrar) {
    r.menu_part(INVENTORY_MENU, Lock);
}

fn stack(item: &str, count: u32) -> Option<ItemStack> {
    Some(ItemStack::new(item, count))
}

/// What the player holds, by `Inventory` index (the 36 main slots), then the cursor.
type Held = (Vec<Option<ItemStack>>, Option<ItemStack>);

fn held(inventory: &[(usize, Option<ItemStack>)], carried: Option<ItemStack>) -> Held {
    let mut slots = vec![None; 36];
    for (i, stack) in inventory {
        slots[*i] = stack.clone();
    }
    (slots, carried)
}

/// The client's prediction of `click` on its `InventoryMenu`, from `before`: what it leaves.
fn predicted(game: &ClientGame<'_>, before: &Held, click: (usize, i32, Input)) -> Held {
    let mut menu = Menu::new(Kind::Inventory);
    for (i, stack) in before.0.iter().enumerate() {
        menu.set_player_item(i, stack.clone());
    }
    menu.carried = before.1.clone();
    menu.click(&game.menu_env(), click.0 as i32, click.1, click.2)
        .unwrap();
    let inventory = (0..36).map(|i| menu.player_item(i).cloned()).collect();
    (inventory, menu.carried.clone())
}

/// Guest 12 clicks window 0 from `before` on the server: what it is left with.
fn served(game: &mut ServerGame<'_>, before: &Held, click: (usize, i32, Input)) -> Held {
    let hub = &mut game.server.lan.as_mut().unwrap().hub;
    hub.guests.update(0, |guest| {
        guest.inventory[..36].clone_from_slice(&before.0);
        guest.carried = before.1.clone();
    });
    let state = hub.guests[0].menu_state;
    let input = match click.2 {
        Input::Pickup => 0,
        Input::QuickMove => 1,
        Input::Swap => 2,
        other => panic!("{other:?} is not clicked here"),
    };
    game.lan_guest_packet(
        12,
        Serverbound::ContainerClick(ContainerClick {
            container: 0,
            state,
            slot: i16::try_from(click.0).unwrap(),
            button: i8::try_from(click.1).unwrap(),
            input,
            changed: Vec::new(),
            carried: None,
        }),
    );
    let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
    (guest.inventory[..36].to_vec(), guest.carried.clone())
}

/// `click` from `before`, predicted on a real client and served on a real server: both must
/// leave the player as they were.
fn refused_on_both_sides(cases: &[(Held, (usize, i32, Input))]) {
    with_client_game(|client| {
        for (before, click) in cases {
            assert_eq!(
                predicted(client, before, *click),
                *before,
                "predicted {click:?}"
            );
        }
    });
    with_guests(|server| {
        assert!(
            server.server.lan.as_ref().unwrap().hub.guests[0]
                .menu
                .is_none()
        );
        for (before, click) in cases {
            assert_eq!(served(server, before, *click), *before, "served {click:?}");
        }
    });
}

#[test]
#[ignore = "daily"]
fn locked_hotbar_slots_refuse_a_click_a_shift_click_a_pickup_and_a_swap_on_both_sides() {
    with_inventory_mods_scope(vec![("lock", lock)], || {
        // A pickup from a locked slot that holds stone.
        let holding = held(&[(7, stack("minecraft:stone", 5))], None);
        // A click that would put the cursor's stone into an empty locked slot.
        let carrying = held(&[], stack("minecraft:stone", 3));
        // A shift-click from the main inventory, whose only room is in the locked hotbar slots
        // (`InventoryMenu.quickMoveStack` moves a main slot's stack to slots 36 to 44).
        let mut full: Vec<_> = (0..6).map(|i| (i, stack("minecraft:dirt", 64))).collect();
        full.push((9, stack("minecraft:stone", 5)));
        refused_on_both_sides(&[
            (holding.clone(), (HOTBAR + 7, 0, Input::Pickup)),
            (carrying, (HOTBAR + 6, 0, Input::Pickup)),
            (held(&full, None), (9, 0, Input::QuickMove)),
            (holding, (HOTBAR + 7, 0, Input::Swap)),
        ]);
        // An unlocked hotbar slot still takes the cursor's stone, on both sides.
        let carrying = held(&[], stack("minecraft:stone", 3));
        let placed = held(&[(5, stack("minecraft:stone", 3))], None);
        with_client_game(|client| {
            assert_eq!(
                predicted(client, &carrying, (HOTBAR + 5, 0, Input::Pickup)),
                placed
            );
        });
        with_guests(|server| {
            assert_eq!(
                served(server, &carrying, (HOTBAR + 5, 0, Input::Pickup)),
                placed
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_refusal_from_one_mods_inventory_part_stands_against_another_mods_allowing_part() {
    fn allow(r: &mut dyn Registrar) {
        r.menu_part(INVENTORY_MENU, Allow);
    }
    with_inventory_mods_scope(vec![("lock", lock), ("allow", allow)], || {
        let carrying = held(&[], stack("minecraft:stone", 3));
        refused_on_both_sides(&[(carrying, (HOTBAR + 6, 0, Input::Pickup))]);
    });
}

#[test]
#[ignore = "daily"]
fn a_handled_inventory_click_skips_the_default_click() {
    fn clicks(r: &mut dyn Registrar) {
        r.menu_part(INVENTORY_MENU, Clicks);
    }
    with_inventory_mods(vec![("clicks", clicks)], |game| {
        CLICKED.with(|clicked| clicked.borrow_mut().clear());
        let holding = held(&[(9, stack("minecraft:stone", 5))], None);
        HANDLES.set(true);
        assert_eq!(served(game, &holding, (9, 0, Input::Pickup)), holding);
        // Left to the game, the same click takes the stone.
        HANDLES.set(false);
        let taken = held(&[], stack("minecraft:stone", 5));
        assert_eq!(served(game, &holding, (9, 0, Input::Pickup)), taken);
        assert_eq!(CLICKED.with(|clicked| clicked.borrow().clone()), [9, 9]);
    });
}

#[test]
#[ignore = "daily"]
fn an_inventory_part_that_places_nothing_refuses_the_2x2_grid_a_recipe() {
    fn no_placement(r: &mut dyn Registrar) {
        r.menu_part(INVENTORY_MENU, NoPlacement);
    }
    with_inventory_mods(vec![("grid", no_placement)], |game| {
        let index = game.stations().recipe_index.clone();
        let planks = index.first_display("minecraft:oak_planks").unwrap();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| {
                guest.dimension = "minecraft:overworld".into();
                guest.loaded = true;
                guest
                    .recipe_book
                    .add_recipes(&index, &["minecraft:oak_planks".to_owned()]);
                guest.inventory[0] = stack("minecraft:oak_log", 3);
            });
        game.lan_guest_packet(
            12,
            Serverbound::PlaceRecipe(PlaceRecipe {
                container: 0,
                recipe: planks,
                use_max: false,
            }),
        );
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(guest.craft_grid, [None, None, None, None]);
        assert_eq!(guest.craft_result, None);
        assert_eq!(guest.inventory[0], stack("minecraft:oak_log", 3));
    });
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
