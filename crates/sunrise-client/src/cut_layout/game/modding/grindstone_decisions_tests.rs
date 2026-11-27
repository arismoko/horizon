//! `grindstone_place` and `grindstone_take` through real listeners: inline dispatch matching
//! NeoForge `CommonHooks.onGrindstoneChange` and `CommonHooks.onGrindstoneTake`.
//! The fixtures need prepared resources.
use mod_api::{
    Flow, GrindstonePlaceEvent, GrindstoneTakeEvent, Hooks, HostTransaction, ItemPlace,
    ItemResource, ModEntry, Mods, Registrar, ServerHost,
};
use std::cell::{Cell, RefCell};
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
use {
    sunrise_gameplay::game::native_guest_turn_tests::with_guests,
    sunrise_gameplay_common::game::stations_common::{Kind as StationKind, Station},
};

#[derive(Clone, Default)]
struct Script {
    place_output: Option<u32>,
    place_xp: Option<i32>,
    place_cancel: bool,
    place_write: bool,
    place_panicking_write: bool,
    place_panic: bool,
    take_panic: bool,
    take_write: bool,
    take_panicking_write: bool,
    take_cancel: bool,
    take_xp: Option<i32>,
    take_new_top: Option<u32>,
    take_new_bottom: Option<u32>,
    restore_top_once: bool,
    place_requires_top: bool,
}

thread_local! {
    static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
    static DISPATCHES: Cell<usize> = const { Cell::new(0) };
    static PLACE_TOP_COUNT: Cell<Option<u32>> = const { Cell::new(None) };
    static PLACE_OBSERVED: RefCell<Vec<(Option<u32>, Option<u32>)>> = const { RefCell::new(Vec::new()) };
    static TAKE_TOP_COUNT: Cell<Option<u32>> = const { Cell::new(None) };
    static TAKE_XP: Cell<Option<i32>> = const { Cell::new(None) };
}

fn script(script: Script) {
    DISPATCHES.set(0);
    PLACE_TOP_COUNT.set(None);
    PLACE_OBSERVED.with(|v| v.borrow_mut().clear());
    TAKE_TOP_COUNT.set(None);
    TAKE_XP.set(None);
    SCRIPT.with(|s| *s.borrow_mut() = script);
}

fn dirt(count: u32) -> mod_api::ItemStack {
    mod_api::ItemStack::new(
        mod_api::__private::item_key("minecraft:dirt").unwrap(),
        count,
    )
    .unwrap()
}

fn write_dirt(host: &mut dyn ServerHost) {
    let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
    let dirt = mod_api::ItemStack::new(key, 1).unwrap();
    let place = ItemPlace::PlayerSlot {
        player: 12,
        slot: 20,
    };
    host.transaction(|host, tx| {
        assert_eq!(place.insert(host, tx, &ItemResource::of(&dirt), 2), Ok(2));
        Ok::<_, ()>(())
    })
    .unwrap();
}

fn panic_during_write(host: &mut dyn ServerHost) {
    let place = ItemPlace::PlayerSlot {
        player: 12,
        slot: 21,
    };
    let _: Result<(), mod_api::TransactionError<()>> = host.transaction(|host, tx| {
        assert_eq!(
            place.insert(host, tx, &ItemResource::of(&dirt(1)), 3),
            Ok(3)
        );
        panic!("a grindstone listener failed during a write");
    });
}

struct Mod;

#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, _: &mut dyn Registrar, _: &mod_api::RegistrationContext) {}

    fn grindstone_place(
        &self,
        host: &mut dyn ServerHost,
        event: &mut GrindstonePlaceEvent<'_>,
    ) -> mod_api::Flow {
        DISPATCHES.set(DISPATCHES.get() + 1);
        PLACE_TOP_COUNT.set(event.inputs.top.map(mod_api::ItemStack::count));
        PLACE_OBSERVED.with(|v| {
            v.borrow_mut().push((
                event.inputs.top.map(mod_api::ItemStack::count),
                event.inputs.bottom.map(mod_api::ItemStack::count),
            ));
        });
        let s = SCRIPT.with(|s| s.borrow().clone());
        if s.place_write {
            write_dirt(host);
        }
        if s.place_panicking_write {
            panic_during_write(host);
        }
        if let Some(count) = s.place_output
            && (!s.place_requires_top || event.inputs.top.is_some())
        {
            event.edits.output = Some(dirt(count));
        }
        if let Some(xp) = s.place_xp {
            event.edits.xp = xp;
        }
        if s.place_cancel {
            event.cancelled = true;
        }
        if s.place_panic {
            panic!("a place listener failed");
        }
        Flow::Continue
    }

    fn grindstone_take(
        &self,
        host: &mut dyn ServerHost,
        event: &mut GrindstoneTakeEvent<'_>,
    ) -> mod_api::Flow {
        DISPATCHES.set(DISPATCHES.get() + 1);
        TAKE_TOP_COUNT.set(event.inputs.top.map(mod_api::ItemStack::count));
        TAKE_XP.set(Some(event.edits.xp));
        let s = SCRIPT.with(|s| s.borrow().clone());
        if s.take_write {
            write_dirt(host);
        }
        if s.take_panicking_write {
            panic_during_write(host);
        }
        event.edits.new_top =
            if SCRIPT.with(|script| std::mem::take(&mut script.borrow_mut().restore_top_once)) {
                Some(dirt(1))
            } else {
                s.take_new_top.map(dirt)
            };
        event.edits.new_bottom = s.take_new_bottom.map(dirt);
        if let Some(xp) = s.take_xp {
            event.edits.xp = xp;
        }
        if s.take_cancel {
            event.cancelled = true;
        }
        if s.take_panic {
            panic!("a take listener failed");
        }
        Flow::Continue
    }
}

fn with_mod(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("grindmod", Box::new(Mod))].into();
    mod_platform::testing::with_mods(mods, || {
        assert!(mod_api::__private::listened(
            mod_api::HookId::GrindstonePlace
        ));
        assert!(mod_api::__private::listened(
            mod_api::HookId::GrindstoneTake
        ));
        test();
    });
}

fn without_mod(test: impl FnOnce()) {
    let mods: Mods = Vec::new().into();
    mod_platform::testing::with_mods(mods, test);
}

fn engine_item(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    id: &str,
    count: u32,
) -> ItemStack {
    let key = mod_api::__private::item_key(id).unwrap();
    {
        use sunrise_gameplay_common::game::api_values::*;
        mod_api::ItemStack::new(key, count)
            .unwrap()
            .try_into_engine(game.items())
    }
    .ok()
    .unwrap()
}

fn open_grindstone(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> i32 {
    // ContainerClick checks stillValid before routing to the workstation. Give the menu a
    // real block, as the other workstation fixtures do, so that check keeps it open.
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
    );
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:grindstone")
        .unwrap();
    game.world.set(1, 64, 1, state).unwrap();
    assert!(game.lan_open_station(12, &Station::new(StationKind::Grindstone, [1, 64, 1])));
    let host = game.server.lan.as_ref().unwrap();
    assert!(game.lan_station_valid(
        &host.hub.guests[0],
        host.menus[&12].station.as_ref().unwrap(),
    ));
    game.server.lan.as_ref().unwrap().menus[&12]
        .station
        .as_ref()
        .unwrap()
        .window
}

fn enchanted_sword(game: &mut sunrise_gameplay::game::ServerGame<'_>, damage: u32) -> ItemStack {
    let mut sword = engine_item(game, "minecraft:iron_sword", 1);
    sword.damage = damage;
    sword.set_crafting_enchantments(vec![("minecraft:sharpness".into(), 3)]);
    sword
}

fn disenchanted_sword(damage: u32) -> ItemStack {
    let mut sword = ItemStack::new("minecraft:iron_sword", 1);
    sword.damage = damage;
    sword
}

fn slot(game: &sunrise_gameplay::game::ServerGame<'_>, index: usize) -> Option<ItemStack> {
    game.server.lan.as_ref().unwrap().menus[&12]
        .station
        .as_ref()
        .unwrap()
        .menu
        .slot(index)
        .cloned()
}

fn count(stack: &Option<ItemStack>) -> u32 {
    stack.as_ref().map_or(0, |s| s.count)
}

fn inventory(game: &sunrise_gameplay::game::ServerGame<'_>, index: usize) -> u32 {
    count(&game.server.lan.as_ref().unwrap().hub.guests[0].inventory[index])
}

fn total_orb_xp(game: &sunrise_gameplay::game::ServerGame<'_>) -> u32 {
    game.orbs
        .iter()
        .map(|orb| orb.value as u32 * orb.count)
        .sum()
}

fn container_click(window: i32, slot: i16, button: i8, input: i32) -> Serverbound {
    Serverbound::ContainerClick(ContainerClick {
        state: 0,
        changed: Vec::new(),
        carried: None,
        container: window,
        slot,
        button,
        input,
    })
}

fn click(game: &mut sunrise_gameplay::game::ServerGame<'_>, window: i32, slot: i16, button: i8) {
    game.lan_guest_packet(12, container_click(window, slot, button, 0));
}

/// A sword in the grindstone's top input with a place listener's result in the output, ready for
/// the take click.
fn placed(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> (i32, ItemStack) {
    let window = open_grindstone(game);
    let sword = enchanted_sword(game, 200);
    let original = sword.clone();
    script(Script {
        place_output: Some(1),
        place_xp: Some(5),
        ..Script::default()
    });
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.carried = Some(sword);
    });
    click(game, window, 0, 0);
    (window, original)
}

#[test]
#[ignore = "daily"]
fn a_place_listener_sets_the_grindstone_result() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            script(Script {
                place_output: Some(1),
                place_xp: Some(5),
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 3);
            assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));
            click(game, window, 2, 0);
            assert_eq!(TAKE_XP.get(), Some(5));
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].carried,
                Some(ItemStack::new("minecraft:dirt", 1))
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_place_answer_does_not_validate_borrowed_inputs() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let mut top = enchanted_sword(game, 200);
            // Load a count above the sword's stack limit without the click's insertion rules.
            top.count = 2;
            let bottom = enchanted_sword(game, 150);
            let host = game.server.lan.as_mut().unwrap();
            host.menus
                .get_mut(&12)
                .unwrap()
                .station
                .as_mut()
                .unwrap()
                .menu
                .load_slot(0, Some(top.clone()));
            host.hub.guests.update(0, |g| {
                g.carried = Some(bottom.clone());
            });
            script(Script {
                place_output: Some(1),
                place_xp: Some(5),
                ..Script::default()
            });

            click(game, window, 1, 0);

            assert_eq!(DISPATCHES.get(), 3);
            assert_eq!(PLACE_TOP_COUNT.get(), Some(2));
            assert_eq!(slot(game, 0), Some(top));
            assert_eq!(slot(game, 1), Some(bottom));
            assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_cancelled_place_leaves_no_grindstone_result() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            // Cancel wins before validating edits.output or edits.xp, and keeps listener writes.
            script(Script {
                place_output: Some(65),
                place_xp: Some(-2),
                place_write: true,
                place_cancel: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 3);
            assert_eq!(count(&slot(game, 2)), 0);
            assert_eq!(inventory(game, 20), 6);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn negative_place_xp_falls_through_to_vanilla_award() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            script(Script {
                place_output: Some(1),
                place_xp: Some(-2),
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 3);
            assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));

            let events = game.server.lan_events.len();
            script(Script::default());
            click(game, window, 2, 0);
            let resolved_xp = TAKE_XP.get().expect("take listener received resolved XP");
            assert!(resolved_xp > 0, "vanilla Sharpness III award is positive");
            assert_eq!(total_orb_xp(game), resolved_xp as u32);
            assert_eq!(game.server.lan_events.len(), events + 1);
            assert_eq!(slot(game, 0), None);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn invalid_place_output_keeps_vanilla_result() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            // Dirt stacks to 64, so count 65 is ignored and vanilla's result stays intact.
            script(Script {
                place_output: Some(65),
                place_xp: Some(5),
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 3);
            assert_eq!(slot(game, 2), Some(disenchanted_sword(200)));
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_grindstone_without_a_listener_keeps_the_vanilla_result() {
    without_mod(|| {
        with_guests(|game| {
            assert!(!mod_api::__private::listened(
                mod_api::HookId::GrindstonePlace
            ));
            assert!(!mod_api::__private::listened(
                mod_api::HookId::GrindstoneTake
            ));
            let window = open_grindstone(game);
            let a = enchanted_sword(game, 200);
            let b = enchanted_sword(game, 150);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(a);
            });
            click(game, window, 0, 0);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(b);
            });
            click(game, window, 1, 0);
            assert!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .menus
                    .get(&12)
                    .is_some_and(|p| p.station.is_some()),
                "the grindstone station is gone after the click"
            );
            // Iron swords have 250 durability: 50 + 100 remaining + floor(250 * .05)
            // leaves 162 durability, i.e. damage 88. Sharpness is removed.
            assert_eq!(slot(game, 2), Some(disenchanted_sword(88)));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_take_answer_validates_only_the_remainders() {
    with_mod(|| {
        with_guests(|game| {
            let (window, mut top) = placed(game);
            let bottom = enchanted_sword(game, 150);
            // Model a resident stack that can be read but could not be written by a mod.
            top.count = 2;
            let menu = &mut game
                .server
                .lan
                .as_mut()
                .unwrap()
                .menus
                .get_mut(&12)
                .unwrap()
                .station
                .as_mut()
                .unwrap()
                .menu;
            menu.load_slot(0, Some(top));
            menu.load_slot(1, Some(bottom));
            let result = Some(ItemStack::new("minecraft:dirt", 1));
            assert_eq!(slot(game, 2), result);

            // An invalid bottom remainder (65 dirt > max 64) is ignored and clears that slot
            // without rolling back the click, valid top remainder, or XP.
            script(Script {
                take_new_top: Some(2),
                take_new_bottom: Some(65),
                ..Script::default()
            });
            click(game, window, 2, 0);
            assert_eq!(TAKE_TOP_COUNT.get(), Some(2));
            assert_eq!(slot(game, 0), Some(ItemStack::new("minecraft:dirt", 2)));
            assert_eq!(slot(game, 1), None);
            assert_eq!(slot(game, 2), None);
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].carried,
                result
            );
            assert_eq!(total_orb_xp(game), 5);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_cancelled_take_keeps_the_click_and_listener_writes() {
    with_mod(|| {
        with_guests(|game| {
            let (window, original) = placed(game);
            let events = game.server.lan_events.len();
            script(Script {
                take_write: true,
                take_cancel: true,
                take_xp: Some(-2),
                take_new_top: Some(1),
                take_new_bottom: Some(65),
                ..Script::default()
            });
            click(game, window, 2, 0);
            // The result stays taken and listener writes stay; XP, sound and input replacement
            // are skipped, and cancel runs before validating remainders.
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].carried,
                Some(ItemStack::new("minecraft:dirt", 1))
            );
            assert_eq!(slot(game, 0), Some(original));
            assert_eq!(inventory(game, 20), 2);
            assert!(game.orbs.is_empty());
            assert_eq!(game.server.lan_events.len(), events);
            assert!(PLACE_OBSERVED.with(|seen| seen.borrow().is_empty()));
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn non_positive_take_xp_awards_no_orbs_and_fires_sound() {
    for xp in [0, -2] {
        with_mod(|| {
            with_guests(|game| {
                let (window, _) = placed(game);
                let events = game.server.lan_events.len();
                script(Script {
                    take_xp: Some(xp),
                    take_new_top: Some(1),
                    ..Script::default()
                });
                click(game, window, 2, 0);
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0].carried,
                    Some(ItemStack::new("minecraft:dirt", 1))
                );
                assert!(game.orbs.is_empty());
                assert_eq!(game.server.lan_events.len(), events + 1);
                assert_eq!(slot(game, 0), Some(ItemStack::new("minecraft:dirt", 1)));
                assert_eq!(slot(game, 1), None);
                assert!(game.transactions.is_empty());
            });
        });
    }
}

#[test]
#[ignore = "daily"]
fn an_outer_abort_undoes_an_accepted_grindstone_take() {
    use std::sync::{Arc, mpsc};
    use sunrise_core::{
        geometry::Vec3,
        random::{LegacyRandom, RandomSource},
    };
    use sunrise_sim::lan::PlayerSender;

    with_mod(|| {
        with_guests(|game| {
            let (window, original) = placed(game);
            // Five XP splits into 3, 1, 1. The 3-XP piece merges into this resident orb.
            let mut expected_random = LegacyRandom::new(23);
            let matching_id = 40 + expected_random.next_int_bound(40).unwrap() as u32;
            game.server.next_entity = matching_id - 1;
            game.add_orb([1.5, 64.5, 1.5], 3);
            let orb = &mut game.orbs[0];
            orb.position = Vec3::new(1.5, 64.5, 1.5);
            orb.age = 42;
            let join = orb.join;
            let (tx, rx) = mpsc::channel();
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.sender = PlayerSender::new(tx, Arc::default());
            });
            let events = game.server.lan_events.len();
            script(Script::default());

            for commit in [false, true] {
                *game.world.random = LegacyRandom::new(23);
                game.storage_scope(|game, _| {
                    click(game, window, 2, 0);
                    assert_eq!(slot(game, 0), None);
                    assert_eq!(
                        game.server.lan.as_ref().unwrap().hub.guests[0].carried,
                        Some(ItemStack::new("minecraft:dirt", 1))
                    );
                    assert_eq!(game.orbs[game.orbs.locate(join).unwrap()].count, 2);
                    assert!(
                        rx.try_recv().is_err(),
                        "published station content before commit"
                    );
                    assert_eq!(
                        game.server.lan_events.len(),
                        events,
                        "published level event before commit"
                    );
                    ((), commit)
                });
                if !commit {
                    assert_eq!(slot(game, 0), Some(original.clone()));
                    assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));
                    assert_eq!(
                        game.server.lan.as_ref().unwrap().hub.guests[0].carried,
                        None
                    );
                    assert_eq!(game.orbs.len(), 1);
                    let orb = &game.orbs[game.orbs.locate(join).unwrap()];
                    assert_eq!((orb.count, orb.age), (1, 42));
                    assert_ne!(
                        (*game.world.random).clone().next_int(),
                        LegacyRandom::new(23).next_int()
                    );
                    assert!(rx.try_recv().is_err(), "published station content on abort");
                    assert_eq!(
                        game.server.lan_events.len(),
                        events,
                        "published level event on abort"
                    );
                } else {
                    assert_eq!(total_orb_xp(game), 8);
                    assert!(
                        rx.try_recv().is_ok(),
                        "did not publish station content on commit"
                    );
                    assert_eq!(game.server.lan_events.len(), events + 1);
                }
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn panicking_grindstone_listeners_keep_vanilla_and_initial_behavior() {
    struct HealthyPlaceListener;

    #[mod_api::__subscriptions]
    impl Hooks for HealthyPlaceListener {
        fn grindstone_place(
            &self,
            _: &mut dyn ServerHost,
            event: &mut GrindstonePlaceEvent<'_>,
        ) -> Flow {
            SCRIPT.with_borrow(|s| {
                if !s.place_panic
                    && let Some((count, xp)) = s.place_output.zip(s.place_xp)
                {
                    event.edits.output = Some(dirt(count));
                    event.edits.xp = xp;
                }
            });
            Flow::Continue
        }
    }

    let mods: Mods = vec![
        ModEntry::new("grindmod", Box::new(Mod)),
        ModEntry::new("healthy-place", Box::new(HealthyPlaceListener)),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword.clone());
            });
            script(Script {
                place_output: Some(1),
                place_xp: Some(5),
                place_panic: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(slot(game, 0), Some(sword));
            assert_eq!(slot(game, 2), Some(disenchanted_sword(200)));
            assert!(game.transactions.is_empty());

            // The panicking Place listener is skipped for the rest of this scope, but
            // the healthy subscriber can still supply an override for the Take test.
            script(Script::default());
            click(game, window, 0, 0);
            script(Script {
                place_output: Some(1),
                place_xp: Some(5),
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 0);
            let output = Some(ItemStack::new("minecraft:dirt", 1));
            assert_eq!(slot(game, 2), output);

            let events = game.server.lan_events.len();
            script(Script {
                take_xp: Some(99),
                take_new_top: Some(1),
                take_panic: true,
                ..Script::default()
            });
            click(game, window, 2, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(TAKE_XP.get(), Some(5));
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].carried,
                output
            );
            assert_eq!(slot(game, 0), None);
            assert_eq!(slot(game, 1), None);
            assert_eq!(total_orb_xp(game), 5);
            assert_eq!(game.server.lan_events.len(), events + 1);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn panicking_listener_aborts_scoped_transaction_and_keeps_committed_writes() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            script(Script {
                place_write: true,
                place_panicking_write: true,
                place_panic: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(inventory(game, 20), 2, "committed write survives");
            assert_eq!(inventory(game, 21), 0, "panicking scope rolled back");
            assert!(
                game.transactions.is_empty(),
                "no abandoned transaction remains open"
            );

            // Take also aborts any panicking scope left behind by a panicking listener.
            script(Script {
                take_write: true,
                take_panicking_write: true,
                take_panic: true,
                ..Script::default()
            });
            click(game, window, 2, 0);
            assert_eq!(inventory(game, 20), 4, "committed take write survives");
            assert_eq!(inventory(game, 21), 0, "panicking take scope rolled back");
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn equal_stack_swap_and_multi_slot_drag_fire_grindstone_place_live() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let mut sticks = engine_item(game, "minecraft:stick", 2);
            sticks.set_crafting_enchantments(vec![("minecraft:sharpness".into(), 1)]);

            // Each input in the multi-slot drag triggers a pair of live result notifications.
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sticks);
            });
            script(Script::default());
            game.lan_guest_packet(12, container_click(window, -999, 0, 5));
            game.lan_guest_packet(12, container_click(window, 0, 1, 5));
            game.lan_guest_packet(12, container_click(window, 1, 1, 5));
            game.lan_guest_packet(12, container_click(window, -999, 2, 5));
            assert_eq!(
                PLACE_OBSERVED.with(|v| v.borrow().clone()),
                vec![
                    (Some(1), None),
                    (Some(1), None),
                    (Some(1), Some(1)),
                    (Some(1), Some(1))
                ]
            );

            // A hotbar swap with an identical sword leaves the input equal, but still notifies
            // twice through Slot.setByPlayer and once through Slot.onTake.
            let sword = enchanted_sword(game, 200);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .menus
                .get_mut(&12)
                .unwrap()
                .station
                .as_mut()
                .unwrap()
                .menu
                .load_slot(0, Some(sword.clone()));
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[0] = Some(sword.clone());
            });
            script(Script::default());
            game.lan_guest_packet(12, container_click(window, 0, 0, 2));
            assert_eq!(slot(game, 0).as_ref(), Some(&sword));
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(guest.inventory[0].as_ref(), Some(&sword));
            assert!(guest.carried.is_none());
            assert_eq!(
                PLACE_OBSERVED.with(|v| v.borrow().clone()),
                vec![(Some(1), Some(1)); 3]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn place_only_take_awards_xp_and_event_before_replacing_inputs() {
    use std::sync::{Arc, Mutex};

    struct PlaceOnlyMod {
        seen: Arc<Mutex<Vec<(Option<u32>, Option<u32>, i64)>>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for PlaceOnlyMod {
        fn register(&self, _: &mut dyn Registrar, _: &mod_api::RegistrationContext) {}

        fn grindstone_place(
            &self,
            host: &mut dyn ServerHost,
            event: &mut GrindstonePlaceEvent<'_>,
        ) -> mod_api::Flow {
            let at = event.pos.map(f64::from);
            let mut orb_xp = 0;
            let level = host.level("minecraft:overworld").unwrap();
            let _ = level
                .visit_experience_orbs(host, at.map(|v| v - 3.0), at.map(|v| v + 4.0), &mut |orb| {
                    orb_xp +=
                        i64::from(orb.value(host).unwrap()) * i64::from(orb.count(host).unwrap());
                    core::ops::ControlFlow::Continue(())
                })
                .unwrap();
            self.seen.lock().unwrap().push((
                event.inputs.top.map(mod_api::ItemStack::count),
                event.inputs.bottom.map(mod_api::ItemStack::count),
                orb_xp,
            ));
            if event.inputs.top.is_some() && event.inputs.bottom.is_some() {
                event.edits.output = Some(dirt(1));
                event.edits.xp = 7;
            }
            Flow::Continue
        }
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "place-only",
        Box::new(PlaceOnlyMod { seen: seen.clone() }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        assert!(mod_api::__private::listened(
            mod_api::HookId::GrindstonePlace
        ));
        assert!(!mod_api::__private::listened(
            mod_api::HookId::GrindstoneTake
        ));
        with_guests(|game| {
            // The real integrated server sets this marker too; the host's orb query
            // needs it when the fixture has no Serving loop.
            game.options.dedicated = Some(Vec::new());
            let window = open_grindstone(game);
            let top = enchanted_sword(game, 200);
            let bottom = enchanted_sword(game, 150);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(top);
            });
            click(game, window, 0, 0);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(bottom);
            });
            click(game, window, 1, 0);
            seen.lock().unwrap().clear();

            // Taking the result without a GrindstoneTake listener must award XP before
            // clearing slot 0 and slot 1 and sending the final container notification.
            let events = game.server.lan_events.len();
            click(game, window, 2, 0);
            assert_eq!(
                *seen.lock().unwrap(),
                vec![(None, Some(1), 7), (None, None, 7), (None, None, 7)]
            );
            assert_eq!(game.server.lan_events.len(), events + 1);
            assert_eq!(total_orb_xp(game), 7);
        });
    });
}

#[test]
#[ignore = "daily"]
fn grindstone_on_take_rebuilds_the_result_before_shift_click_or_throw_all_repeats() {
    for (button, input) in [(0, 1), (1, 4)] {
        with_mod(|| {
            with_guests(|game| {
                let (window, _) = placed(game);
                let events = game.server.lan_events.len();
                script(Script {
                    place_output: Some(1),
                    place_xp: Some(7),
                    place_requires_top: true,
                    restore_top_once: true,
                    ..Script::default()
                });
                game.lan_guest_packet(12, container_click(window, 2, button, input));

                assert_eq!(
                    DISPATCHES.get(),
                    8,
                    "two takes, three Place notifications per take"
                );
                assert_eq!(
                    PLACE_OBSERVED.with(|seen| seen.borrow().clone()),
                    [
                        (Some(1), None),
                        (Some(1), None),
                        (Some(1), None),
                        (None, None),
                        (None, None),
                        (None, None),
                    ]
                );
                assert_eq!(
                    game.server.lan_events[events..]
                        .iter()
                        .map(|e| e.kind)
                        .collect::<Vec<_>>(),
                    [1042, 1042]
                );
                assert_eq!(
                    total_orb_xp(game),
                    12,
                    "the awards are 5 then 7 from the rebuilt result"
                );
                assert!((0..3).all(|i| slot(game, i).is_none()));
                if input == 1 {
                    assert_eq!(inventory(game, 8), 2);
                } else {
                    assert_eq!(
                        game.dropped
                            .iter()
                            .map(|drop| drop.stack.count)
                            .sum::<u32>(),
                        2
                    );
                }
            });
        });
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
