use std::sync::{
    Arc,
    atomic::{AtomicI32, AtomicUsize, Ordering},
    mpsc,
};
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_level::containers::ModMenuType;
use sunrise_level::containers::{
    Furnace, FurnaceKind, ModBlockEntity, ModBlockEntityFields, ModCrafting, ModLinks,
};
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_sim::lan::menu::Slots;
use sunrise_sim::lan::{
    Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig, menu::Menu as HubMenu,
};
use sunrise_sim::menu::place::PostPlace;

const STATION: (i32, i32, i32) = (4, 200, 4);
const CHEST: (i32, i32, i32) = (32, 200, 4);
const WINDOW: i32 = 7;

struct Lender {
    sources: Vec<[i32; 3]>,
    calls: Arc<AtomicUsize>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Lender {
    fn recipe_sources(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RecipeSourcesEvent<'_>,
    ) {
        assert_eq!(event.player, 3003);
        assert_eq!(event.menu, "test:lending");
        assert_eq!(event.pos, [4, 200, 4]);
        self.calls.fetch_add(1, Ordering::Relaxed);
        event.sources.extend_from_slice(&self.sources);
    }
    fn menu_links(&self, _: &mut dyn mod_api::ServerHost, event: &mut mod_api::MenuLinksEvent<'_>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        event.links.extend_from_slice(&self.sources);
    }
}

fn lending(sources: Vec<[i32; 3]>, calls: Arc<AtomicUsize>, run: impl FnOnce()) {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "lender",
            Box::new(Lender { sources, calls }),
        )]
        .into(),
        run,
    );
}

fn setup(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> &'static ModMenuType {
    game.build_registries();
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    game.world
        .chunks
        .insert((2, 0), Arc::new(sunrise_level::chunk::Chunk::empty(2, 0)));
    game.lan_host(
        sunrise_gameplay::game::lan::LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "lending-test".into(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    static MENU_TYPE: std::sync::LazyLock<ModMenuType> = std::sync::LazyLock::new(|| ModMenuType {
        id: "test:lending",
        owner: "test",
        slots: 37,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: vec![],
        inputs: Vec::new(),
        links: Some(ModLinks {
            from: 10,
            count: 27,
        }),
        crafting: Some(ModCrafting {
            grid: 1,
            size: 3,
            result: 0,
        }),
    });
    let t = &*MENU_TYPE;
    game.world.block_entities.insert(
        STATION,
        BlockEntity::Mod(Box::new(ModBlockEntity::new(
            "test:lending".into(),
            ModBlockEntityFields {
                items: vec![None; 37],
                ..Default::default()
            },
        ))),
    );
    game.world.block_entities.insert(
        CHEST,
        BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:oak_planks", 3))]),
    );
    let (tx, _rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 2,
            uuid: 3003,
            name: "LendingGuest".into(),
            properties: vec![],
            info: ClientInformation::new(2),
            brand: "sunrise".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        2,
        [4.5, 201.0, 4.5],
        2,
    );
    guest.placed = true;
    guest.loaded = true;
    guest.menu = Some(HubMenu {
        window: WINDOW,
        kind: 0,
        registry_id: t.id.into(),
        pos: Some(STATION),
        slots: Slots::Block(STATION),
        size: 37,
        state: 0,
        sent: vec![None; 73],
        drag: None,
        sent_data: vec![],
        reported_unsent_data_len: None,
        blockless_data: Vec::new(),

        mod_type: Some(t),
        craft: None,
        links: vec![],
        asks_still_valid: false,
        opening_item: None,
        opener_block: None,
        removed: false,
        slot_changed_seen: None,
    });
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    t
}

#[test]
#[ignore = "daily"]
fn lent_chest_supplies_transfer_and_placement_and_is_written_back() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        setup(game);
        assert!(
            game.lan_check_recipe(0, WINDOW, "minecraft:stick")
                .unwrap()
                .result
                .is_err()
        );
        let calls = Arc::new(AtomicUsize::new(0));
        lending(vec![[32, 200, 4]], calls.clone(), || {
            assert!(
                game.lan_check_recipe(0, WINDOW, "minecraft:stick")
                    .unwrap()
                    .result
                    .is_ok()
            );
            game.world.modified.clear();
            assert_eq!(
                game.lan_apply_recipe(0, WINDOW, |menu, env| menu.place_recipe(
                    env,
                    "minecraft:stick",
                    false,
                    false,
                    None
                )),
                Some(PostPlace::Nothing)
            );
            let BlockEntity::Chest(items) = game.world.block_entities.get(&CHEST).unwrap() else {
                panic!("chest removed")
            };
            assert_eq!(items[0], Some(ItemStack::new("minecraft:oak_planks", 1)));
            assert!(
                game.world.modified.contains(&(2, 0)),
                "lent chest never received container_changed"
            );
            let BlockEntity::Mod(station) = game.world.block_entities.get(&STATION).unwrap() else {
                panic!("station removed")
            };
            let grid: Vec<_> = station.items[1..10].iter().flatten().collect();
            assert_eq!(grid.len(), 2);
            assert!(
                grid.iter()
                    .all(|s| **s == ItemStack::new("minecraft:oak_planks", 1))
            );
        });
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    });
}

#[test]
#[ignore = "daily"]
fn recipe_checks_and_placement_use_late_sources_and_skip_invalid_answers() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        let t = setup(game);
        // Keep self lendable so removing only the self-position exclusion fails.
        let station = game.world.block_entities.get(&STATION).unwrap().clone();
        game.world
            .block_entities
            .insert(STATION, BlockEntity::Chest(vec![None; 27]));
        let invalid = (6, 200, 4);
        game.world.block_entities.insert(
            invalid,
            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                String::new(),
                ModBlockEntityFields::default(),
            ))),
        );
        let positions: Vec<_> = (0..70).map(|i| [100 + i, 200, 4]).collect();
        for &[x, y, z] in &positions {
            let stack = (x == 169).then(|| ItemStack::new("minecraft:oak_planks", 3));
            game.world
                .block_entities
                .insert((x, y, z), BlockEntity::Chest(vec![stack]));
        }
        let mut answers = vec![
            [4, 200, 4],
            [5, 200, 4],
            [6, 200, 4],
            positions[0],
            positions[0],
        ];
        answers.extend_from_slice(&positions[1..]);
        lending(answers, Arc::default(), || {
            let sources = game.lan_recipe_sources(0, t, STATION);
            assert_eq!(
                sources
                    .iter()
                    .map(|&((x, y, z), _)| [x, y, z])
                    .collect::<Vec<_>>(),
                positions
            );
            game.world.block_entities.insert(STATION, station);
            assert_eq!(
                game.lan_check_recipe(0, WINDOW, "minecraft:stick")
                    .unwrap()
                    .result,
                Ok(())
            );
            assert_eq!(
                game.lan_apply_recipe(0, WINDOW, |menu, env| menu.place_recipe(
                    env,
                    "minecraft:stick",
                    false,
                    false,
                    None
                )),
                Some(PostPlace::Nothing)
            );
            let container = game.world.block_entities.get(&(169, 200, 4)).unwrap();
            assert_eq!(
                container.slots(),
                &[Some(ItemStack::new("minecraft:oak_planks", 1))]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn inventory_menu_does_not_ask_recipe_sources() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        setup(game);
        let calls = Arc::new(AtomicUsize::new(0));
        lending(vec![[32, 200, 4]], calls.clone(), || {
            game.lan_check_recipe(0, 0, "minecraft:stick").unwrap();
            game.lan_apply_recipe(0, 0, |menu, env| {
                menu.place_recipe(env, "minecraft:stick", false, false, None)
            });
        });
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    });
}

#[test]
#[ignore = "daily"]
fn menu_links_expose_chest_contents_in_the_answering_tick() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        setup(game);
        lending(vec![[32, 200, 4]], Arc::default(), || {
            // Use the production tick entry point, so this also fails if links
            // are refreshed after the hub broadcasts the container slots.
            game.tick_lan(false);
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            let menu = guest.menu.as_ref().unwrap();
            assert_eq!(menu.links, vec![CHEST]);
            // `sent` is the hub's client-visible snapshot, updated only when
            // sync_containers emits ContainerSetSlot for this window.
            assert_eq!(
                menu.sent[10],
                Some(ItemStack::new("minecraft:oak_planks", 3))
            );
        });
        // A subscriber-free tick must preserve links and perform no transient
        // allocations on its thread, even if an allocation would be freed immediately.
        let before = game.server.lan.as_ref().unwrap().hub.guests[0]
            .menu
            .as_ref()
            .unwrap()
            .links
            .as_ptr();
        mod_platform::testing::with_mods(vec![].into(), || {
            #[cfg(not(any(feature = "system-allocator", feature = "alloc-census")))]
            {
                let (_, control) = sunrise_threads::allocation_count::measure(|| {
                    std::hint::black_box(vec![0u8; 128]);
                });
                assert!(
                    control > 0,
                    "allocation counter did not observe its control"
                );
                let (_, allocations) =
                    sunrise_threads::allocation_count::measure(|| game.lan_menu_links_tick());
                assert_eq!(allocations, 0, "listener-free link tick allocated");
            }
            #[cfg(any(feature = "system-allocator", feature = "alloc-census"))]
            game.lan_menu_links_tick();
        });
        let menu = game.server.lan.as_ref().unwrap().hub.guests[0]
            .menu
            .as_ref()
            .unwrap();
        assert_eq!(menu.links.as_ptr(), before);
        assert_eq!(menu.links, vec![CHEST]);
    });
}

#[test]
#[ignore = "daily"]
fn menu_links_skip_a_furnace_the_mod_names() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        setup(game);
        // A furnace is a block entity, but not one a menu can lend slots from:
        // `linkable_container` answers None for it, not only for a mod block
        // entity with an empty id.
        let furnace = (33, 200, 4);
        game.world.block_entities.insert(
            furnace,
            BlockEntity::Furnace(Furnace {
                kind: FurnaceKind::Furnace,
                items: [
                    Some(ItemStack::new("minecraft:iron_ore", 1)),
                    Some(ItemStack::new("minecraft:coal", 1)),
                    None,
                ],
                lit_remaining: 0,
                lit_total: 0,
                cooking_timer: 0,
                cooking_total: 200,
                speed: 1.0,
                recipes_used: Default::default(),
            }),
        );
        // The mod names the furnace first, then the chest: only the chest links.
        lending(
            vec![[furnace.0, furnace.1, furnace.2], [32, 200, 4]],
            Arc::default(),
            || {
                game.tick_lan(false);
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                let menu = guest.menu.as_ref().unwrap();
                assert_eq!(menu.links, vec![CHEST]);
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn menu_links_keep_past_sixty_four_and_preserve_order_without_duplicates_or_self() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        setup(game);
        static LONG_MENU_TYPE: std::sync::LazyLock<ModMenuType> =
            std::sync::LazyLock::new(|| ModMenuType {
                id: "test:lending",
                owner: "test",
                slots: 80,
                player_inventory: true,
                data_slots: 0,
                owned_slots: None,
                results: vec![],
                inputs: Vec::new(),
                links: Some(ModLinks {
                    from: 10,
                    count: 70,
                }),
                crafting: Some(ModCrafting {
                    grid: 1,
                    size: 3,
                    result: 0,
                }),
            });
        // Keep the opening container itself lendable so failing to skip `pos` shows up in `links`.
        game.world.block_entities.insert(
            STATION,
            BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:bedrock", 1)); 80]),
        );
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| {
                let menu = guest.menu.as_mut().unwrap();
                menu.size = 80;
                menu.sent = vec![None; 116];
                menu.mod_type = Some(&*LONG_MENU_TYPE);
            });
        let furnace = (99, 200, 4);
        game.world.block_entities.insert(
            furnace,
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
        let positions: Vec<_> = (0..66).map(|i| [100 + i, 200, 4]).collect();
        for (i, at) in positions.iter().enumerate() {
            let item = match i {
                0 => "minecraft:oak_planks",
                64 => "minecraft:diamond",
                65 => "minecraft:emerald",
                _ => "minecraft:stone",
            };
            game.world.block_entities.insert(
                (at[0], at[1], at[2]),
                BlockEntity::Chest(vec![Some(ItemStack::new(item, 1))]),
            );
        }
        let mut answers = vec![
            [STATION.0, STATION.1, STATION.2],
            [98, 200, 4],
            [furnace.0, furnace.1, furnace.2],
            positions[0],
            positions[0],
        ];
        answers.extend_from_slice(&positions[1..64]);
        answers.push(positions[3]);
        answers.push([STATION.0, STATION.1, STATION.2]);
        answers.push(positions[64]);
        answers.push(positions[65]);
        let expected_links: Vec<_> = positions.iter().map(|p| (p[0], p[1], p[2])).collect();
        lending(answers, Arc::default(), || {
            game.tick_lan(false);
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            let menu = guest.menu.as_ref().unwrap();
            assert_eq!(menu.links, expected_links);
            assert_eq!(
                menu.sent[10],
                Some(ItemStack::new("minecraft:oak_planks", 1))
            );
            assert_eq!(menu.sent[11], Some(ItemStack::new("minecraft:stone", 1)));
            assert_eq!(
                menu.sent[10 + 64],
                Some(ItemStack::new("minecraft:diamond", 1))
            );
            assert_eq!(
                menu.sent[10 + 65],
                Some(ItemStack::new("minecraft:emerald", 1))
            );
        });
    });
}

/// A writable `menu_links` listener that replaces the linked menu while it answers: the old
/// block's containers are not installed as links on the replacement.
#[test]
#[ignore = "daily"]
fn menu_links_answers_do_not_reach_a_menu_the_listener_replaced() {
    struct ReplacingLender(std::sync::atomic::AtomicBool);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ReplacingLender {
        fn menu_links(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::MenuLinksEvent<'_>,
        ) {
            event.links.push([32, 200, 4]);
            if !self.0.swap(true, Ordering::Relaxed) {
                host.player(event.player)
                    .unwrap()
                    .open_chest_menu(
                        host,
                        1,
                        mod_api::Text::literal("Replacement"),
                        Vec::new(),
                        mod_api::MenuReplace::InPlace,
                    )
                    .unwrap();
            }
        }
    }
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        setup(game);
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "replacing-lender",
            Box::new(ReplacingLender(Default::default())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            game.lan_menu_links_tick();
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_ne!(
                guest.menu.as_ref().map(|menu| menu.window),
                Some(WINDOW),
                "the listener replaced the menu"
            );
            assert!(
                guest.menu.as_ref().is_none_or(|menu| menu.links.is_empty()),
                "the old block's links did not reach the replacement"
            );
        });
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
