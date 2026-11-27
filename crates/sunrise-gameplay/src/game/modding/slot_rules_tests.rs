//! `menu_slot_rules` through a real `MenuPart` on the read-only `Host`: what a click asks, in
//! what order, how often, and what a click does without a listener, with a failing one and with
//! one that answers out of range. The fixtures need prepared resources.
#[cfg(test)]
use crate::game::native_entity_refs::tests::with_game;
#[cfg(test)]
use crate::game::native_guest_turn_tests::with_guests;
use mod_api::{Hooks, MenuPart, MenuSlotRulesEvent, ModEntry, Mods, Registrar};
use std::cell::RefCell;
use sunrise_items::items::ItemStack;
use sunrise_sim::menu::{Input, Kind, Menu};

thread_local! {
    /// Every question the part was asked: slot, item and count.
    static ASKED: RefCell<Vec<(usize, String, u32)>> = const { RefCell::new(Vec::new()) };
    /// What the part answers for `maximum`, and whether it panics on dirt.
    static ANSWER: RefCell<(Option<u32>, bool)> = const { RefCell::new((None, false)) };
    /// Whether the guest 12 resolved through the `Host`, at each question.
    static SAW_GUEST: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

struct Rules;
#[mod_api::sunrise_part]
impl MenuPart for Rules {
    fn menu_slot_rules(&self, host: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        SAW_GUEST.with(|saw| saw.borrow_mut().push(host.player(12).is_some()));
        let item = mod_api::__private::item_key_id(event.stack.item())
            .unwrap()
            .to_owned();
        ASKED.with(|asked| {
            asked
                .borrow_mut()
                .push((event.slot, item.clone(), event.stack.count()))
        });
        let (maximum, panic) = ANSWER.with(|answer| *answer.borrow());
        if panic && item == "minecraft:dirt" {
            event.max_stack = 0;
            panic!("a listener failed");
        }
        if let Some(maximum) = maximum {
            event.max_stack = maximum;
        }
    }
}
struct Listener;
#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x3", Rules);
    }
}
/// A mod with no part for any menu.
struct Quiet;
#[mod_api::__subscriptions]
impl Hooks for Quiet {}

fn reset(maximum: Option<u32>, panic: bool) {
    SAW_GUEST.with(|saw| saw.borrow_mut().clear());
    ASKED.with(|asked| asked.borrow_mut().clear());
    ANSWER.with(|answer| *answer.borrow_mut() = (maximum, panic));
}
fn asked() -> Vec<(usize, String, u32)> {
    ASKED.with(|asked| asked.borrow().clone())
}

/// A chest menu with its type registered, empty, the cursor holding `count` of `item`.
fn chest(carried: (&str, u32)) -> Menu {
    let mut menu = Menu::new(Kind::Chest(3));
    menu.set_registry_id(Some("minecraft:generic_9x3"));
    menu.carried = Some(ItemStack::new(carried.0, carried.1));
    menu
}
/// What a pickup click on slot 0 leaves: the slot and the cursor.
fn pickup(game: &crate::game::ServerGame<'_>, menu: &mut Menu) -> (u32, u32) {
    menu.click(&game.menu_env(), 0, 0, Input::Pickup).unwrap();
    (
        menu.slot(0).map_or(0, |s| s.count),
        menu.carried.as_ref().map_or(0, |s| s.count),
    )
}

pub fn with_mods(mods: Vec<ModEntry>, test: impl FnOnce()) {
    let mods: Mods = mods.into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, test);
}

struct StagedReader;
#[mod_api::sunrise_part]
impl MenuPart for StagedReader {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        use mod_api::MenuContainer::{Main, Player};
        assert!(event.context.slot(Main, 2).is_none()); // unknown item
        assert!(event.context.slot(Main, usize::MAX).is_none());
        // Live player inventory, armor (36..40 feet-to-head) and offhand (40) are readable.
        let player_stone = event.context.slot(Player, 0).unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(player_stone.item()),
            Some("minecraft:stone")
        );
        assert_eq!(player_stone.count(), 1);
        assert!(event.context.slot(Player, 1).is_none());
        let boots = event.context.slot(Player, 36).unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(boots.item()),
            Some("minecraft:iron_boots")
        );
        let offhand = event.context.slot(Player, 40).unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(offhand.item()),
            Some("minecraft:shield")
        );
        let ingredient = event.context.slot(Main, 1).unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(ingredient.item()),
            Some("minecraft:oak_planks")
        );
        event.max_stack = ingredient.count();
    }
}

struct StagedListener;
#[mod_api::__subscriptions]
impl Hooks for StagedListener {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x3", StagedReader);
    }
}

#[test]
#[ignore = "daily"]
fn staged_slot_rule_reads_hide_unrepresentable_and_player_stacks() {
    with_mods(
        vec![ModEntry::new("rules", Box::new(StagedListener))],
        || {
            with_game(|game| {
                let mut menu = chest(("minecraft:stone", 8));
                menu.load_slot(1, Some(ItemStack::new("minecraft:oak_planks", 3)));
                menu.load_slot(2, Some(ItemStack::new("test:unregistered", 1)));
                menu.set_player_item(0, Some(ItemStack::new("minecraft:stone", 1)));
                menu.set_player_item(36, Some(ItemStack::new("minecraft:iron_boots", 1)));
                menu.set_player_item(40, Some(ItemStack::new("minecraft:shield", 1)));
                assert_eq!(pickup(game, &mut menu), (3, 5));
            });
        },
    );
}

/// A stack the parts can't be asked about (#6030): an item the mods' registry never resolved
/// is not placed into a slot whose rules a part answers, and nothing is asked; one already in
/// the slot can still be taken out by the slot's own rule.
#[test]
#[ignore = "daily"]
fn a_stack_the_parts_cannot_be_asked_about_is_not_placed() {
    with_mods(vec![ModEntry::new("rules", Box::new(Listener))], || {
        with_game(|game| {
            reset(None, false);
            let mut menu = chest(("test:unregistered", 8));
            assert_eq!(pickup(game, &mut menu), (0, 8));
            assert_eq!(asked(), []);
            let mut menu = chest(("minecraft:stone", 1));
            menu.carried = None;
            menu.load_slot(0, Some(ItemStack::new("test:unregistered", 3)));
            assert_eq!(pickup(game, &mut menu), (0, 3));
            assert_eq!(asked(), []);
        });
    });
    // Without a part the slot's own rules decide, and the stack goes in as before.
    with_mods(vec![ModEntry::new("quiet", Box::new(Quiet))], || {
        with_game(|game| {
            let mut menu = chest(("test:unregistered", 8));
            assert_eq!(pickup(game, &mut menu), (8, 0));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_click_asks_the_part_for_each_slot_and_stack_it_consults_in_order() {
    with_mods(vec![ModEntry::new("rules", Box::new(Listener))], || {
        with_game(|game| {
            reset(None, false);
            let mut menu = chest(("minecraft:stone", 8));
            // Slot.safeInsert asks separately about placement and capacity.
            assert_eq!(pickup(game, &mut menu), (8, 0));
            let stone = |_| (0, "minecraft:stone".to_owned(), 8);
            assert_eq!(asked(), [stone(()), stone(())]);
            // Taking the stack back asks once: `Slot.allowModification` is `mayPickup` and
            // `mayPlace` of the slot's own stack, eight stone in slot 0.
            reset(None, false);
            menu.carried = None;
            assert_eq!(pickup(game, &mut menu), (0, 8));
            assert_eq!(asked(), [(0, "minecraft:stone".to_owned(), 8)]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn the_answers_are_bounded_to_one_to_ninety_nine_and_the_items_limit() {
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
    use sunrise_sim::lan::menu::Slots;
    with_mods(vec![ModEntry::new("rules", Box::new(Listener))], || {
        with_game(|game| {
            // A maximum of 0 holds none (leaving the slot empty), 3 holds three, and a huge one
            // holds what the item stacks to: 64 stone, not 99 and not the answer.
            for (answer, held) in [(0, 0), (3, 3), (u32::MAX, 64)] {
                reset(Some(answer), false);
                let mut menu = chest(("minecraft:stone", 64));
                assert_eq!(pickup(game, &mut menu), (held, 64 - held), "{answer}");
                if answer == 0 {
                    assert!(menu.slot(0).is_none());
                }
            }
        });
        // A LAN view click with max_stack == 0 likewise places nothing and leaves no zero-count
        // stack behind.
        with_guests(|game| {
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
            host.hub.guests.update(0, |guest| {
                guest.carried = Some(ItemStack::new("minecraft:stone", 64))
            });
            reset(Some(0), false);
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
            assert!(game.world.block_entities.get(&pos).unwrap().slots()[0].is_none());
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .carried
                    .as_ref()
                    .map(|s| s.count),
                Some(64)
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_failing_slot_rule_does_not_override_or_disable_the_other_parts() {
    struct Healthy;
    #[mod_api::__subscriptions]
    impl Hooks for Healthy {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.menu_part("minecraft:generic_9x3", Healthy);
        }
    }
    #[mod_api::sunrise_part]
    impl MenuPart for Healthy {
        fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
            assert!(
                event.max_stack > 0,
                "the failed part's answer must not be visible"
            );
            event.may_place = false;
        }
    }
    with_mods(
        vec![
            ModEntry::new("rules", Box::new(Listener)),
            ModEntry::new("zz_healthy_rules", Box::new(Healthy)),
        ],
        || {
            with_game(|game| {
                reset(None, true);
                let menu = chest(("minecraft:dirt", 8));
                let dirt = menu.carried.as_ref().unwrap();
                let env = game.menu_env();
                // One live check: only the healthy part's refusal takes effect.
                assert_eq!(menu.slot_holds(&env, 0, dirt), 0);
                assert_eq!(asked(), [(0, "minecraft:dirt".to_owned(), 8)]);
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "rules",
                        hook: "menu_slot_rules",
                    }]
                );
                reset(None, true);
                assert_eq!(menu.slot_holds(&env, 0, dirt), 0);
                assert!(asked().is_empty());
                assert!(mod_api::__private::take_hook_panics().is_empty());
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn without_a_part_nothing_is_asked_and_the_click_is_the_same() {
    // The same click with no mod installed, with a mod that has no part, and with a part on
    // another menu type: the final state is identical and the part is never asked.
    let run = |game: &crate::game::ServerGame<'_>, registry: &str| {
        let mut menu = chest(("minecraft:stone", 8));
        menu.set_registry_id(Some(registry));
        pickup(game, &mut menu)
    };
    let baseline = RefCell::new(None);
    with_game(|game| {
        reset(None, false);
        *baseline.borrow_mut() = Some(run(game, "minecraft:generic_9x3"));
        assert_eq!(asked(), []);
    });
    with_mods(vec![ModEntry::new("quiet", Box::new(Quiet))], || {
        with_game(|game| {
            reset(None, false);
            assert_eq!(Some(run(game, "minecraft:generic_9x3")), *baseline.borrow());
            assert_eq!(asked(), []);
        });
    });
    with_mods(vec![ModEntry::new("rules", Box::new(Listener))], || {
        with_game(|game| {
            reset(None, false);
            // A menu type the part is not attached to: not asked, same answer.
            assert_eq!(Some(run(game, "minecraft:generic_9x6")), *baseline.borrow());
            assert_eq!(asked(), []);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_lan_hub_click_asks_the_same_questions_in_the_same_order() {
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
    use sunrise_sim::lan::menu::Slots;
    with_mods(vec![ModEntry::new("rules", Box::new(Listener))], || {
        with_guests(|game| {
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
            host.hub.guests.update(0, |guest| {
                guest.carried = Some(ItemStack::new("minecraft:stone", 8))
            });
            reset(None, false);
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
            let lan = asked();
            // The hub's click ran with the LAN host lifted out of the game, and is back. The
            // `Host` read meanwhile answered as if no LAN were open: the guest is not found.
            assert!(game.server.lan.is_some());
            assert_eq!(SAW_GUEST.with(|saw| saw.borrow().clone()), [false, false]);
            assert!(game.player(12).is_some());
            // The local click of the same state asks the same, in the same order.
            reset(None, false);
            let mut menu = chest(("minecraft:stone", 8));
            assert_eq!(pickup(game, &mut menu), (8, 0));
            assert_eq!(lan, asked());
            assert_eq!(lan.len(), 2);
            // Both questions on the local route can read the guest through the Host.
            assert_eq!(SAW_GUEST.with(|saw| saw.borrow().clone()), [true, true]);
            let placed = game.world.block_entities.get(&pos).unwrap().slots()[0]
                .as_ref()
                .map_or(0, |s| s.count);
            assert_eq!(placed, 8);
        });
    });
}

#[test]
#[ignore = "daily"]
fn nested_menu_reads_use_base_slot_rules() {
    use sunrise_gameplay_common::game::api_values::test_snapshot;
    use sunrise_level::{chunk::Chunk, containers::BlockEntity};
    use sunrise_sim::lan::menu::Slots;

    thread_local! {
        static CALLS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    struct ReadsOwnMenu;
    #[mod_api::sunrise_part]
    impl MenuPart for ReadsOwnMenu {
        fn menu_slot_rules(&self, host: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
            let calls = CALLS.get() + 1;
            CALLS.set(calls);
            // Without the guard, make reentry fail the assertions rather than overflow.
            if calls > 2 {
                event.max_stack = 1;
                return;
            }
            let Some(menu) = event
                .context
                .player
                .and_then(|uuid| host.player(uuid))
                .and_then(|player| player.container_menu(host))
            else {
                return;
            };
            if matches!(menu.slot(host, 0), Some(None))
                && menu.carried(host).flatten().map(|stack| stack.count()) == Some(8)
                && menu.slot_holds(host, 0, event.stack) == Some(64)
                && menu.slot_holds(host, 1, event.stack) == Some(64)
            {
                event.max_stack = 3;
            }
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for ReadsOwnMenu {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.menu_part("minecraft:generic_9x3", ReadsOwnMenu);
        }
    }

    sunrise_gameplay_common::game::in_own_process(
        "game::slot_rules_tests::nested_menu_reads_use_base_slot_rules",
        || {
            with_mods(vec![ModEntry::new("rules", Box::new(ReadsOwnMenu))], || {
                with_guests(|game| {
                    let pos = (1, 64, 1);
                    game.world
                        .chunks
                        .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
                    let state = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:chest")
                        .unwrap();
                    game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                    game.world
                        .block_entities
                        .insert(pos, BlockEntity::Chest(vec![None; 27]));
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
                    host.hub.guests.update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:stone", 8));
                    });
                    let player = game.player(12).unwrap();
                    let menu = player.container_menu(game).unwrap();
                    let stone = test_snapshot(&ItemStack::new("minecraft:stone", 8));
                    CALLS.set(0);
                    assert_eq!(menu.slot_holds(game, 0, &stone), Some(3));
                    // Placement and capacity use the same answer from the live view.
                    assert_eq!(CALLS.get(), 1);
                });
            });
        },
    );
}

struct LiveSiblingPart;
#[mod_api::sunrise_part]
impl MenuPart for LiveSiblingPart {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        if event.container == mod_api::MenuContainer::Main && event.slot == 1 {
            event.may_pickup = event.stack.count() == 3;
        }
        if event.container == mod_api::MenuContainer::Main && event.slot == 0 {
            assert_eq!(event.context.player, Some(12));
            assert_eq!(event.context.pos, Some([1, 64, 1]));
            if let Some(sibling) = event.context.slot(mod_api::MenuContainer::Main, 1) {
                event.max_stack = sibling.count();
                event.may_pickup = sibling.count() >= 4;
            } else {
                event.may_place = false;
                event.may_pickup = false;
            }
        }
    }
}

struct LiveSiblingListener;
#[mod_api::__subscriptions]
impl Hooks for LiveSiblingListener {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x1", LiveSiblingPart);
        let helmet = r.item_key("minecraft:iron_helmet");
        r.item_part(helmet, GuestFeetOnly);
    }
}

struct GuestFeetOnly;
#[mod_api::sunrise_part]
impl mod_api::ItemPart for GuestFeetOnly {
    fn can_equip(&self, host: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
        assert_eq!(host.player(12).unwrap().entity(), event.entity.entity());
        event.can_equip = event.slot == "feet";
    }
}

/// An admitted block-backed menu, with registration and storage scoped to this game.
fn open_live_rules_fixture(
    game: &mut crate::game::ServerGame<'_>,
    menu_type: &'static sunrise_level::containers::ModMenuType,
    items: Vec<Option<ItemStack>>,
) {
    use std::sync::Arc;
    use sunrise_level::{
        chunk::Chunk,
        containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields, ModContainer},
    };
    use sunrise_sim::lan::menu::Slots;
    // The game fixture stops before content loading freezes the block-entity type registry.
    // Resolve this fixture's declarations in its scoped Parts table so block-entity parts fire.
    let types = mod_api::__private::block_entity_types();
    if !types.is_empty() {
        mod_api::__private::resolve_block_entity_types(
            types.iter().map(|(_, kind)| kind.id.as_str()),
            |id| types.iter().any(|(_, kind)| kind.id == id).then_some(()),
        )
        .unwrap();
    }
    let pos = (1, 64, 1);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let stone_state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(pos.0, pos.1, pos.2, stone_state).unwrap();
    let state = game.world.block(pos.0, pos.1, pos.2).0;
    game.world.block_entities.set_mod_containers(
        [(
            state,
            Arc::new(ModContainer {
                slots: menu_type.slots,
                ..Default::default()
            }),
        )]
        .into_iter()
        .collect(),
    );
    let entity = ModBlockEntity::new(
        "test:box".into(),
        ModBlockEntityFields {
            items,
            ..Default::default()
        },
    );
    game.world
        .block_entities
        .insert(pos, BlockEntity::Mod(Box::new(entity)));

    let host = game.server.lan.as_mut().unwrap();
    host.hub.open_container(
        12,
        menu_type.id,
        sunrise_core::nbt::Tag::String("box".into()),
        pos,
        Slots::Block(pos),
        vec![None; menu_type.slots],
        None,
    );
    host.hub
        .update_guest(12, |guest| {
            guest.menu.as_mut().unwrap().mod_type = Some(menu_type);
        })
        .unwrap();
}

#[test]
#[ignore = "daily"]
fn live_slot_queries_read_live_siblings_limits_acting_armor_and_stale_handles() {
    use sunrise_core::game_mode::GameMode;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_level::containers::{BlockEntity, ModLinks, ModMenuType};
    use sunrise_sim::lan::menu::Slots;

    let menu_type: &'static ModMenuType = Box::leak(Box::new(ModMenuType {
        id: "minecraft:generic_9x1",
        owner: "test",
        slots: 9,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: Vec::new(),
        inputs: Vec::new(),
        crafting: None,
        links: Some(ModLinks { from: 1, count: 2 }),
    }));

    with_mods(
        vec![ModEntry::new("live_sibling", Box::new(LiveSiblingListener))],
        || {
            with_guests(|game| {
                let pos = (1, 64, 1);
                let mut items = vec![None; 9];
                items[0] = Some(ItemStack::new("minecraft:stone", 2));
                items[1] = Some(ItemStack::new("minecraft:oak_planks", 55)); // raw, not the linked stack
                items[2] = Some(ItemStack::new("minecraft:stone", 7)); // unbound, not a click target
                open_live_rules_fixture(game, menu_type, items);
                let linked = (1, 64, 2);
                game.world.block_entities.insert(
                    linked,
                    BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:oak_planks", 3))]),
                );
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.menu.as_mut().unwrap().links.push(linked);
                    })
                    .unwrap();

                let player = game.player(12).unwrap();
                let menu = game.player_container_menu(player).unwrap();
                let stone_api = ItemStack::new("minecraft:stone", 1).try_into_api().unwrap();

                // Block-backed mod menu queries take read-only `&dyn Host` and observe live
                // sibling slots without reopening.
                let read: &dyn mod_api::Host = game;
                assert_eq!(menu.slot_holds(read, 0, &stone_api), Some(3));
                assert_eq!(menu.slot_gives(read, 0), Some(false));
                assert_eq!(menu.slot_gives(read, 1), Some(true));

                assert_eq!(menu.slot_holds(read, 2, &stone_api), Some(0));
                assert_eq!(menu.slot_gives(read, 2), Some(false));
                assert!(matches!(
                    menu.slot_backing(read, 1),
                    Some(mod_api::MenuSlotBacking::BlockEntity {
                        pos: [1, 64, 2],
                        slot: 0,
                        ..
                    })
                ));
                assert_eq!(menu.slot_backing(read, 2), None);

                // Direct `ItemPlace::MenuSlot` writes reach the bound linked container and
                // refuse unbound linked indices without touching the opener's shadow slots.
                {
                    use mod_api::{HostTransaction, ItemPlace, ItemResource, ServerHost};
                    let planks = ItemResource::of(
                        &ItemStack::new("minecraft:oak_planks", 1)
                            .try_into_api()
                            .unwrap(),
                    );
                    let stone_res = ItemResource::of(&stone_api);
                    let bound = ItemPlace::MenuSlot { menu, slot: 1 };
                    let unbound = ItemPlace::MenuSlot { menu, slot: 2 };
                    let host: &mut dyn ServerHost = game;
                    let moved = host
                        .transaction(|host, tx| {
                            Ok::<_, ()>((
                                bound.extract(host, tx, &planks, 1),
                                bound.insert(host, tx, &planks, 2),
                                unbound.extract(host, tx, &stone_res, 1),
                                unbound.insert(host, tx, &stone_res, 1),
                            ))
                        })
                        .unwrap();
                    assert_eq!(moved, (Ok(1), Ok(2), Ok(0), Ok(0)));
                }
                assert_eq!(
                    game.world.block_entities.get(&linked).unwrap().slots()[0]
                        .as_ref()
                        .map(|s| s.count),
                    Some(4)
                );
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[1]
                        .as_ref()
                        .map(|s| s.count),
                    Some(55)
                );
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[2]
                        .as_ref()
                        .map(|s| s.count),
                    Some(7)
                );

                game.world
                    .block_entities
                    .get_mut(&linked)
                    .unwrap()
                    .slots_mut()[0] = Some(ItemStack::new("minecraft:oak_planks", 5));
                assert_eq!(menu.slot_holds(game, 0, &stone_api), Some(5));
                assert_eq!(menu.slot_gives(game, 0), Some(true));

                // Block container_stack_limit caps container slots without capping player slots.
                game.world.set_container_stack_limit(pos, 2);
                assert_eq!(menu.slot_holds(game, 0, &stone_api), Some(2));
                assert_eq!(menu.slot_holds(game, 9, &stone_api), Some(64));

                // Double chest applies each half's container_stack_limit independently,
                // and replacing the menu invalidates the previous handle.
                let left = (2, 64, 1);
                let right = (3, 64, 1);
                game.world
                    .block_entities
                    .insert(left, BlockEntity::Chest(vec![None; 27]));
                game.world
                    .block_entities
                    .insert(right, BlockEntity::Chest(vec![None; 27]));
                game.world.set_container_stack_limit(left, 5);
                game.world.set_container_stack_limit(right, 11);
                game.server.lan.as_mut().unwrap().hub.open_container(
                    12,
                    "minecraft:generic_9x6",
                    sunrise_core::nbt::Tag::String("double".into()),
                    left,
                    Slots::Double(left, right),
                    vec![None; 54],
                    None,
                );
                assert_eq!(menu.slot_holds(game, 0, &stone_api), None);
                assert_eq!(menu.slot_gives(game, 0), None);

                let double_menu = game.player_container_menu(player).unwrap();
                assert_eq!(double_menu.slot_holds(game, 0, &stone_api), Some(5));
                assert_eq!(double_menu.slot_holds(game, 27, &stone_api), Some(11));
                assert_eq!(double_menu.slot_holds(game, 54, &stone_api), Some(64));

                game.server.lan.as_mut().unwrap().hub.close_container(12);
                assert_eq!(double_menu.slot_holds(game, 0, &stone_api), None);
                assert_eq!(double_menu.slot_gives(game, 0), None);

                // Kind::Inventory observes acting-turn bound armor and creative mode.
                let inv = game.player_container_menu(player).unwrap();
                let mut bound_helmet = ItemStack::new("minecraft:diamond_helmet", 1);
                bound_helmet.set_crafting_enchantments(vec![("minecraft:binding_curse".into(), 1)]);
                game.as_guest(12, |game| {
                    let helmet = ItemStack::new("minecraft:iron_helmet", 1)
                        .try_into_api()
                        .unwrap();
                    assert_eq!(inv.slot_holds(game, 5, &helmet), Some(0));
                    assert_eq!(inv.slot_holds(game, 8, &helmet), Some(1));
                    game.armor[0] = Some(bound_helmet);
                    assert_eq!(inv.slot_gives(game, 5), Some(false));
                    assert_eq!(inv.slot_gives(game, 6), Some(true));
                    game.set_mode(GameMode::Creative);
                    assert_eq!(inv.slot_gives(game, 5), Some(true));
                    game.set_mode(GameMode::Survival);
                })
                .unwrap();
                assert_eq!(inv.slot_gives(game, 5), Some(false));
            });
        },
    );
}

struct LiveReusePart;
#[mod_api::sunrise_part]
impl MenuPart for LiveReusePart {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        let item = mod_api::__private::item_key_id(event.stack.item())
            .unwrap()
            .to_owned();
        ASKED.with(|asked| {
            asked
                .borrow_mut()
                .push((event.slot, item, event.stack.count()))
        });
        if event.container == mod_api::MenuContainer::Main && event.slot == 0 {
            event.may_pickup = event
                .context
                .slot(mod_api::MenuContainer::Main, 1)
                .is_some_and(|s| s.count() >= 4);
        }
    }
}

struct LiveReuseListener;
#[mod_api::__subscriptions]
impl Hooks for LiveReuseListener {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x1", LiveReusePart);
    }
}

#[test]
#[ignore = "daily"]
fn mod_menu_slot_queries_and_clicks_use_the_same_live_pickup_rule() {
    use sunrise_level::containers::ModMenuType;
    use sunrise_protocol::packets::play::ContainerClick;

    let menu_type = Box::leak(Box::new(ModMenuType {
        id: "minecraft:generic_9x1",
        owner: "test",
        slots: 9,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: Vec::new(),
        inputs: Vec::new(),
        crafting: None,
        links: None,
    }));
    with_mods(
        vec![ModEntry::new("live_reuse", Box::new(LiveReuseListener))],
        || {
            with_guests(|game| {
                let pos = (1, 64, 1);
                let stone = ItemStack::new("minecraft:stone", 8);
                let mut items = vec![None; 9];
                items[0] = Some(stone.clone());
                items[1] = Some(ItemStack::new("minecraft:oak_planks", 2));
                open_live_rules_fixture(game, menu_type, items);
                let menu = game
                    .player_container_menu(game.player(12).unwrap())
                    .unwrap();
                let shift = ContainerClick {
                    container: menu.container_id(game).unwrap(),
                    state: 0,
                    slot: 0,
                    button: 0,
                    input: 1,
                    changed: Vec::new(),
                    carried: None,
                };
                for (sibling, gives) in [(2, false), (5, true)] {
                    game.world.block_entities.get_mut(&pos).unwrap().slots_mut()[1] =
                        Some(ItemStack::new("minecraft:oak_planks", sibling));
                    reset(None, false);
                    assert_eq!(menu.slot_gives(game, 0), Some(gives));
                    assert_eq!(asked(), [(0, "minecraft:stone".to_owned(), 8)]);
                    reset(None, false);
                    assert_eq!(game.lan_machine_click(12, &shift), Some(true));
                    assert_eq!(
                        asked()
                            .iter()
                            .filter(|ask| ask.0 == 0 && ask.1 == "minecraft:stone")
                            .count(),
                        1
                    );
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[0],
                        (!gives).then_some(stone.clone())
                    );
                }
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn cached_machine_menu_unhides_linked_slot_when_storage_binds() {
    use sunrise_level::containers::{BlockEntity, ModLinks, ModMenuType};
    use sunrise_protocol::packets::play::ContainerClick;

    let menu_type: &'static ModMenuType = Box::leak(Box::new(ModMenuType {
        id: "minecraft:generic_9x1",
        owner: "test",
        slots: 9,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: Vec::new(),
        inputs: Vec::new(),
        crafting: None,
        links: Some(ModLinks { from: 1, count: 2 }),
    }));

    with_guests(|game| {
        open_live_rules_fixture(game, menu_type, vec![None; 9]);
        let linked_a = (1, 64, 2);
        let linked_b = (1, 64, 3);
        game.world.block_entities.insert(
            linked_a,
            BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:oak_planks", 3))]),
        );
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |guest| {
                guest.menu.as_mut().unwrap().links.push(linked_a);
            })
            .unwrap();
        let window = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(12)
            .unwrap()
            .menu
            .as_ref()
            .unwrap()
            .window;
        let click_slot = |slot: i16| ContainerClick {
            container: window,
            state: 0,
            slot,
            button: 0,
            input: 0,
            changed: Vec::new(),
            carried: None,
        };

        // First click caches the machine `Menu` while slot 2 is unbound (hidden).
        assert_eq!(game.lan_machine_click(12, &click_slot(2)), Some(true));
        assert!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(12)
                .unwrap()
                .carried
                .is_none()
        );

        // Bind a second container to slot 2 in the same open window without reopening.
        game.world.block_entities.insert(
            linked_b,
            BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:dirt", 4))]),
        );
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |guest| {
                guest.menu.as_mut().unwrap().links.push(linked_b);
            })
            .unwrap();

        assert_eq!(game.lan_machine_click(12, &click_slot(2)), Some(true));
        let carried = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(12)
            .unwrap()
            .carried
            .as_ref()
            .expect("slot 2 should unhide on the cached machine menu");
        assert_eq!(carried.item.as_ref(), "minecraft:dirt");
        assert_eq!(carried.count, 4);
        assert!(game.world.block_entities.get(&linked_b).unwrap().slots()[0].is_none());
    });
}

struct QuickMoveRoutedPart;
#[mod_api::sunrise_part]
impl mod_api::BlockPart for QuickMoveRoutedPart {}
#[mod_api::sunrise_part]
impl mod_api::BlockEntityPart for QuickMoveRoutedPart {
    fn menu_quick_move(&self, event: &mut mod_api::MenuQuickMoveEvent<'_>) -> mod_api::Flow {
        if event.slot == 1
            && let Some(stack) = event.stack.as_mut()
            && stack.count() >= 2
        {
            let mut moved = stack.clone();
            moved.set_count(2).unwrap();
            event
                .subject
                .with_view(|mut entity| entity.set_item_slot(0, Some(moved)))
                .unwrap()
                .unwrap();
            stack.set_count(stack.count() - 2).unwrap();
            return mod_api::Flow::Cancel;
        }
        if event.slot == 7 {
            event
                .subject
                .with_view(|mut entity| {
                    if entity.item_slot(8).is_none() {
                        let mut ingredient = entity
                            .item_slot(3)
                            .and_then(|held| held.to_owned())
                            .unwrap();
                        ingredient.set_count(ingredient.count() - 1).unwrap();
                        entity.set_item_slot(3, Some(ingredient)).unwrap();
                        let mut moved = event.stack.as_ref().unwrap().clone();
                        moved.set_count(2).unwrap();
                        entity.set_item_slot(8, Some(moved)).unwrap();
                        event.stack.as_mut().unwrap().set_count(2).unwrap();
                    }
                })
                .unwrap();
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }
}

struct QuickMoveRoutedListener;
#[mod_api::__subscriptions]
impl Hooks for QuickMoveRoutedListener {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("box").blocks(&[]).build();
        r.block_entity_part(&kind, QuickMoveRoutedPart).unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn quick_move_routes_bound_linked_and_craft_result_slots() {
    use sunrise_level::containers::{BlockEntity, ModCrafting, ModLinks, ModMenuType};
    use sunrise_protocol::packets::play::ContainerClick;

    let menu_type: &'static ModMenuType = Box::leak(Box::new(ModMenuType {
        id: "minecraft:generic_9x1",
        owner: "test",
        slots: 9,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: Vec::new(),
        inputs: Vec::new(),
        crafting: Some(ModCrafting {
            grid: 3,
            size: 2,
            result: 7,
        }),
        links: Some(ModLinks { from: 1, count: 2 }),
    }));

    with_mods(
        vec![ModEntry::new("test", Box::new(QuickMoveRoutedListener))],
        || {
            with_guests(|game| {
                let pos = (1, 64, 1);
                let linked = (1, 64, 2);
                let mut items = vec![None; 9];
                items[1] = Some(ItemStack::new("minecraft:oak_planks", 55));
                items[2] = Some(ItemStack::new("minecraft:stone", 7));
                items[3] = Some(ItemStack::new("minecraft:oak_log", 2));
                open_live_rules_fixture(game, menu_type, items);
                game.world.block_entities.insert(
                    linked,
                    BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:dirt", 6))]),
                );
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.menu.as_mut().unwrap().links.push(linked);
                    })
                    .unwrap();
                game.lan_mod_crafting_tick();

                let window = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(12)
                    .unwrap()
                    .menu
                    .as_ref()
                    .unwrap()
                    .window;
                let shift_click = |slot: i16| ContainerClick {
                    container: window,
                    state: 0,
                    slot,
                    button: 0,
                    input: 1,
                    changed: Vec::new(),
                    carried: None,
                };

                // Bound linked slot: reads and settles to the linked chest, never opener slot 1.
                assert_eq!(game.lan_machine_click(12, &shift_click(1)), Some(true));
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[0]
                        .as_ref()
                        .map(|s| (s.item.as_ref(), s.count)),
                    Some(("minecraft:dirt", 2))
                );
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[1]
                        .as_ref()
                        .map(|s| s.count),
                    Some(55)
                );
                assert_eq!(
                    game.world.block_entities.get(&linked).unwrap().slots()[0]
                        .as_ref()
                        .map(|s| (s.item.as_ref(), s.count)),
                    Some(("minecraft:dirt", 4))
                );

                // Unbound linked slot does not fire `menu_quick_move` or touch opener slot 2.
                assert_eq!(game.lan_machine_click(12, &shift_click(2)), Some(true));
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[2]
                        .as_ref()
                        .map(|s| s.count),
                    Some(7)
                );

                // The v6 handler debits the ingredient itself and keeps a partial result
                // visible on that grid without writing into the block entity's result slot.
                assert_eq!(game.lan_machine_click(12, &shift_click(7)), Some(true));
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[8]
                        .as_ref()
                        .map(|s| (s.item.as_ref(), s.count)),
                    Some(("minecraft:oak_planks", 2))
                );
                assert!(game.world.block_entities.get(&pos).unwrap().slots()[7].is_none());
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[3]
                        .as_ref()
                        .map(|s| s.count),
                    Some(1)
                );
                let player = game.player(12).unwrap();
                let menu = game.player_container_menu(player).unwrap();
                let next_result = menu.slot(game, 7).unwrap().unwrap();
                assert_eq!(
                    mod_api::__private::item_key_id(next_result.item()),
                    Some("minecraft:oak_planks")
                );
                assert_eq!(next_result.count(), 2);

                // Handled without taking leaves the grid and `entity.items[7]` untouched.
                assert_eq!(game.lan_machine_click(12, &shift_click(7)), Some(true));
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[3]
                        .as_ref()
                        .map(|s| s.count),
                    Some(1)
                );
                assert!(game.world.block_entities.get(&pos).unwrap().slots()[7].is_none());
            });
        },
    );
}

use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
