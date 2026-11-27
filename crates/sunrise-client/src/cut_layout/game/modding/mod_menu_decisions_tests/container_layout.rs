//! A block-backed menu lays out all its declared slots before the first write.
use super::{container_click, register_test_menu};
use mod_api::{
    BlockEntityPart, BlockUpdateFlags, Flow, Hooks, MenuQuickMoveEvent, ModContainer, ModEntry,
    Mods, Registrar,
};
use std::{cell::Cell, sync::Arc};
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_items::items::ItemStack;
use sunrise_level::{chunk::Chunk, containers};
use sunrise_sim::save;
use {
    sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_guest_turn_tests::with_guests_at,
    sunrise_gameplay_common::game::api_values,
    sunrise_gameplay_common::game::lan_actions::MenuOpen,
};

const BLOCK: &str = "layout_test:machine";
const MENU: &str = "layout_test:menu";
const POS: (i32, i32, i32) = (1, 64, 1);

thread_local! {
    static CLICK: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

struct LayoutMod;
#[mod_api::__subscriptions]
impl Hooks for LayoutMod {
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new(BLOCK, "minecraft:barrel").with_item(false))
            .unwrap();
    }

    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key(BLOCK);
        let kind = r.block_entity_type("machine").blocks(&[BLOCK]).build();
        r.block_entity_part(&kind, StoreShiftClick).unwrap();
        r.block_container(
            &block,
            ModContainer {
                slots: 6,
                menu: Some(MENU.into()),
                ..Default::default()
            },
        );
    }
}

struct StoreShiftClick;
#[mod_api::sunrise_part]
impl mod_api::BlockPart for StoreShiftClick {}
#[mod_api::sunrise_part]
impl BlockEntityPart for StoreShiftClick {
    fn menu_quick_move(&self, event: &mut MenuQuickMoveEvent<'_>) -> Flow {
        CLICK.set(Some((event.container_slots, event.slot)));
        let stack = event.stack.take().unwrap();
        assert_eq!(
            event
                .subject
                .with_view(|mut entity| entity.set_item_slot(0, Some(stack))),
            Some(Ok(true)),
        );
        Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn six_slot_menu_shift_click_uses_hotbar_zero_before_any_container_write() {
    const TEST: &str = "cut_layout::game::client_side::mod_menu_decisions_tests::container_layout::six_slot_menu_shift_click_uses_hotbar_zero_before_any_container_write";
    const CASE: &str = "SUNRISE_TEST_MOD_CONTAINER_LAYOUT_CASE";
    if run_fresh_cases(TEST, CASE, &["fresh", "sparse-save"]) {
        return;
    }
    let from_save = std::env::var(CASE).unwrap() == "sparse-save";
    let mods: Mods = vec![ModEntry::new("layout_test", Box::new(LayoutMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = fixture_root();
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        containers::with_mod_menu_types(
            [(
                MENU.into(),
                "layout_test".into(),
                6,
                0,
                Vec::new(),
                Vec::new(),
                None,
                None,
                true,
                None,
            )],
            || {
                with_guests_at(root.prepared().to_owned(), |game| {
                    register_test_menu(game, MENU);
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    let state = game.pack.assets.registry.default_state(BLOCK).unwrap();
                    if from_save {
                        game.world.set(POS.0, POS.1, POS.2, state).unwrap();
                        let tag = Tag::compound([
                            ("id", Tag::String(BLOCK.into())),
                            ("x", Tag::Int(POS.0)),
                            ("y", Tag::Int(POS.1)),
                            ("z", Tag::Int(POS.2)),
                            (
                                "Items",
                                Tag::List(vec![Tag::compound([
                                    ("Slot", Tag::Byte(1)),
                                    ("id", Tag::String("minecraft:stone".into())),
                                    ("count", Tag::Int(2)),
                                ])]),
                            ),
                        ]);
                        let entity = save::decode_block_entity_for_block(&tag, BLOCK).unwrap();
                        game.world.load_chunk_block_entities((0, 0), vec![entity]);
                    } else {
                        assert!(game.set_block_live(
                            [POS.0, POS.1, POS.2],
                            state,
                            BlockUpdateFlags::NONE.bits(),
                            true,
                        ));
                    }
                    let tables = game.native_containers();
                    game.world.block_entities.set_mod_containers(tables);

                    let stack = ItemStack::new("minecraft:stone", 10);
                    let untouched = ItemStack::new("minecraft:oak_log", 3);
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.brand = "sunrise".into();
                            guest.sender.menu_extra_rev =
                                sunrise_protocol::menu_extra::LIVE_DATA_REVISION;
                            guest.inventory[0] = Some(stack.clone());
                            guest.inventory[6] = Some(untouched.clone());
                        });
                    assert_eq!(game.open_block_menu(12, POS), MenuOpen::Opened);
                    let window = game.server.lan.as_ref().unwrap().hub.guests[0]
                        .menu
                        .as_ref()
                        .unwrap()
                        .window;
                    game.lan_guest_packet(12, container_click(window, 33, 1));

                    assert_eq!(CLICK.get(), Some((6, 33)));
                    let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                    let inventory = &guest.inventory;
                    assert_eq!(inventory[0], None, "the clicked hotbar slot must empty");
                    assert_eq!(inventory[6].as_ref(), Some(&untouched));
                    let slots = sunrise_sim::lan::menu::Slots::Block(POS)
                        .read(&game.world, guest)
                        .unwrap();
                    assert_eq!(
                        slots,
                        [
                            Some(stack),
                            from_save.then(|| ItemStack::new("minecraft:stone", 2)),
                            None,
                            None,
                            None,
                            None
                        ]
                    );
                    let total: u32 = slots
                        .iter()
                        .chain(inventory)
                        .flatten()
                        .map(|stack| stack.count)
                        .sum();
                    assert_eq!(total, if from_save { 15 } else { 13 });
                })
            },
        );
    });
}
