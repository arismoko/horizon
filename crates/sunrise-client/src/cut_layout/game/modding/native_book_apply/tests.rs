use mod_api::{Hand, HostTransaction, ItemPlace, ItemResource, MenuSlotPlace, ServerHost};
use sunrise_gameplay_common::game::native_book_displays::NativeBookDisplaysGameExt1 as _;
use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
use sunrise_gameplay_common::game::tables::GameItems as _;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
use sunrise_level::containers::{BlockEntity, ModBlockEntity, ModMenuType};
use sunrise_sim::menu::place::{PostPlace, custom_plan::MenuPlacement};
#[cfg(test)]
use sunrise_sim::recipe_book::placement_sync as sync;
use sunrise_sim::{
    crafting::{Ingredient, Placement},
    lan::menu::{Menu as Open, Slots},
    recipe_book::{settings_connection::Connection, settings_sync::Authority},
};

/// A guest with a three-slot mod menu open on a block, and the catalogue & display its client would send.
fn open_mod_menu(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
) -> ((i32, i32, i32), sync::CatalogIdentity, sync::DisplayId) {
    open_mod_menu_named(game, "review:menu")
}

fn open_mod_menu_named(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    menu_id: &'static str,
) -> ((i32, i32, i32), sync::CatalogIdentity, sync::DisplayId) {
    let pos = (3, 64, 5);
    let kind = Box::leak(Box::new(ModMenuType {
        id: menu_id,
        owner: "review",
        slots: 3,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: vec![2],
        inputs: Vec::new(),
        crafting: None,
        links: None,
    }));
    let mut block = ModBlockEntity::new("review:block".into(), Default::default());
    block.items = vec![
        Some(sunrise_items::items::ItemStack::new("minecraft:dirt", 1)),
        None,
        None,
    ];
    game.world
        .block_entities
        .insert(pos, BlockEntity::Mod(Box::new(block)));
    let state = game.world.block(pos.0, pos.1, pos.2).0;
    game.world.block_entities.set_mod_containers(
        [(
            state,
            std::sync::Arc::new(sunrise_level::containers::ModContainer {
                slots: 3,
                ..Default::default()
            }),
        )]
        .into_iter()
        .collect(),
    );
    game.world.modified.clear();
    let (window, instance) = game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        if let Some(open) = &g.menu {
            assert!(g.close_menu_cursor(open.window).is_none());
        }
        // Use the real window lifecycle so a resend can find this window's cursor.
        let window = g.next_menu_window();
        g.menu = Some(Open {
            window,
            kind: 0,
            registry_id: menu_id.into(),
            pos: Some(pos),
            slots: Slots::Block(pos),
            size: 3,
            state: 0,
            sent: vec![],
            drag: None,
            sent_data: vec![],
            reported_unsent_data_len: None,
            blockless_data: Vec::new(),

            mod_type: Some(kind),
            craft: None,
            links: vec![],
            asks_still_valid: false,
            opening_item: None,
            opener_block: None,
            removed: false,
            slot_changed_seen: None,
        });
        (window, g.menu_generation)
    });
    let catalog = sync::CatalogIdentity {
        epoch: 1,
        number: 1,
        window,
        nonce: 1,
        instance,
        generation: 1,
        menu: menu_id.to_owned().try_into().unwrap(),
        book: "review:book".to_owned().try_into().unwrap(),
    };
    let display = sync::DisplayId {
        recipe: "review:recipe".to_owned().try_into().unwrap(),
        source: 0,
    };
    let mut server = sunrise_gameplay_common::game::native_book_placement::Server::new(1);
    server.peer.hello(1, sync::schema()).unwrap();
    server.peer.publish(catalog.clone()).unwrap();
    game.native_net.placement.servers.insert(12, server);
    let mut connection = Connection::default();
    let book = catalog.book.clone();
    connection.authority = Some(Authority::negotiate(1, vec![book.clone()], &[book].into()));
    game.native_net.book_connections.insert(12, connection);
    (pos, catalog, display)
}

#[test]
#[ignore = "daily"]
fn a_mod_layout_without_a_listening_part_refuses_and_changes_nothing() {
    super::super::native_guest_turn_tests::with_guests(|game| {
        let (pos, catalog, display) = open_mod_menu(game);
        let plan = MenuPlacement::for_mod(
            "review:menu".into(),
            Placement::of(
                vec![Some(Ingredient::of(vec!["minecraft:iron_ore".into()]))],
                None,
            ),
        )
        .unwrap();
        assert_eq!(
            game.apply_mod_book_plan(12, &catalog, &display, &plan, false),
            Err(sync::Refusal::Unsupported)
        );
        assert_eq!(
            game.world.block_entities.get(&pos).unwrap().slots()[0]
                .as_ref()
                .unwrap()
                .count,
            1
        );
        assert!(game.world.modified.is_empty());
    });
}

struct CountingPlacer(std::sync::Arc<std::sync::atomic::AtomicUsize>);
#[mod_api::sunrise_part]
impl mod_api::MenuPart for CountingPlacer {
    fn recipe_book_place(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RecipeBookPlaceEvent<'_>,
    ) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *event.placement = mod_api::RecipeBookPlacement::Grid {
            width: 1,
            height: 1,
            slots: vec![0],
            clear: vec![0],
        };
    }
}

struct PanickingPlacer {
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    book: mod_api::RecipeBookTypeKey,
}
#[mod_api::sunrise_part]
impl mod_api::MenuPart for PanickingPlacer {
    fn recipe_book_place(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RecipeBookPlaceEvent<'_>,
    ) {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let player = host.players().acting_player().unwrap();
        assert!(player.set_recipe_book_settings(
            host,
            &self.book,
            mod_api::RecipeBookSettings {
                open: true,
                filtering: true,
            }
        ));
        *event.placement = mod_api::RecipeBookPlacement::Grid {
            width: 1,
            height: 1,
            slots: vec![0],
            clear: vec![0],
        };
        panic!("menu placer exploded");
    }
}

#[test]
#[ignore = "daily"]
fn observed_mod_click_supplies_grid_once_and_second_click_acts_on_changed_grid() {
    struct CountingMod(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CountingMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.menu_part("review:menu", CountingPlacer(self.0.clone()));
        }
    }
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "review",
        Box::new(CountingMod(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let (pos, catalog, display) = open_mod_menu(game);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[9] = Some(sunrise_items::items::ItemStack::new(
                    "minecraft:iron_ore",
                    3,
                ));
            });
            let plan = MenuPlacement::for_mod(
                "review:menu".into(),
                Placement::of(
                    vec![Some(Ingredient::of(vec!["minecraft:iron_ore".into()]))],
                    None,
                ),
            )
            .unwrap();
            assert_eq!(
                game.apply_mod_book_plan(12, &catalog, &display, &plan, false),
                Ok(PostPlace::Nothing)
            );
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            let first = game.world.block_entities.get(&pos).unwrap().slots()[0]
                .as_ref()
                .unwrap();
            assert_eq!(first.item.as_ref(), "minecraft:iron_ore");
            assert_eq!(first.count, 1);

            assert_eq!(
                game.apply_mod_book_plan(12, &catalog, &display, &plan, false),
                Ok(PostPlace::Nothing)
            );
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
            let second = game.world.block_entities.get(&pos).unwrap().slots()[0]
                .as_ref()
                .unwrap();
            assert_eq!(second.item.as_ref(), "minecraft:iron_ore");
            assert_eq!(second.count, 2);
            let left = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(12)
                .unwrap()
                .inventory[9]
                .as_ref()
                .unwrap()
                .count;
            assert_eq!(left, 1);
        });
    });
}

/// A `recipe_sources` listener that breaks the block backing the open menu, without closing
/// the player's window.
struct BreaksTheBacking(std::sync::Arc<std::sync::atomic::AtomicUsize>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for BreaksTheBacking {
    fn recipe_sources(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::RecipeSourcesEvent<'_>,
    ) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let level = host.level("minecraft:overworld").unwrap();
        let stone = host.block_state_id("minecraft:stone").unwrap();
        assert_eq!(
            level.set_block(host, event.pos, stone, mod_api::BlockUpdateFlags::NONE),
            Ok(true)
        );
    }
}

#[test]
#[ignore = "daily"]
fn a_recipe_sources_listener_that_breaks_the_backing_refuses_the_placement() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("breaker", Box::new(BreaksTheBacking(calls.clone()))),
        mod_api::ModEntry::new(
            "review",
            Box::new({
                struct Menu;
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Menu {
                    fn register(
                        &self,
                        r: &mut dyn mod_api::Registrar,
                        _: &mod_api::RegistrationContext,
                    ) {
                        r.menu_part("review:menu", CountingPlacer(Default::default()));
                    }
                }
                Menu
            }),
        ),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let (pos, catalog, display) = open_mod_menu(game);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[9] = Some(sunrise_items::items::ItemStack::new(
                    "minecraft:iron_ore",
                    3,
                ));
            });
            let plan = MenuPlacement::for_mod(
                "review:menu".into(),
                Placement::of(
                    vec![Some(Ingredient::of(vec!["minecraft:iron_ore".into()]))],
                    None,
                ),
            )
            .unwrap();
            assert_eq!(
                game.apply_mod_book_plan(12, &catalog, &display, &plan, false),
                Err(sync::Refusal::Changed)
            );
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert!(!matches!(
                game.world.block_entities.get(&pos),
                Some(BlockEntity::Mod(_))
            ));
            let left = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(12)
                .unwrap()
                .inventory[9]
                .as_ref()
                .unwrap()
                .count;
            assert_eq!(left, 3, "the refused placement took nothing");
        });
    });
}

#[test]
#[ignore = "daily"]
fn panicking_recipe_book_place_discards_answer_disables_only_that_callback_and_keeps_other_menus_running()
 {
    use sunrise_protocol::packets::play::Clientbound as C;
    use sunrise_sim::recipe_book::{
        BookFlags, settings_codec,
        settings_sync::{CHANNEL, Message},
    };

    struct TwoMenuMod {
        panic_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        other_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        changes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for TwoMenuMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let book = r.recipe_book_type("review:book").unwrap();
            r.menu_part(
                "review:menu",
                PanickingPlacer {
                    calls: self.panic_calls.clone(),
                    book,
                },
            );
            r.menu_part("review:other", CountingPlacer(self.other_calls.clone()));
        }
        fn player_recipe_book_settings_change(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::PlayerRecipeBookSettingsChangeEvent<'_>,
        ) {
            self.changes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let changes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let panic_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let other_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "review",
        Box::new(TwoMenuMod {
            panic_calls: panic_calls.clone(),
            other_calls: other_calls.clone(),
            changes: changes.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let _ = mod_api::__private::take_hook_panics();
            let (pos, catalog, display) = open_mod_menu_named(game, "review:menu");
            let (tx, rx) = std::sync::mpsc::channel();
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.sender = sunrise_sim::lan::PlayerSender::new(tx, Default::default());
                g.inventory[9] = Some(sunrise_items::items::ItemStack::new(
                    "minecraft:iron_ore",
                    3,
                ));
            });
            let plan = MenuPlacement::for_mod(
                "review:menu".into(),
                Placement::of(
                    vec![Some(Ingredient::of(vec!["minecraft:iron_ore".into()]))],
                    None,
                ),
            )
            .unwrap();

            assert_eq!(
                game.apply_mod_book_plan(12, &catalog, &display, &plan, false),
                Ok(PostPlace::Nothing)
            );
            assert_eq!(panic_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            let untouched = game.world.block_entities.get(&pos).unwrap().slots()[0]
                .as_ref()
                .unwrap();
            assert_eq!(untouched.item.as_ref(), "minecraft:dirt");
            assert_eq!(untouched.count, 1);
            let saved = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(12)
                .unwrap()
                .recipe_book
                .write();
            assert!(
                saved
                    .get(mod_api::item::recipe_book::saved_fields::EXTENSION)
                    .is_none()
            );
            assert_eq!(changes.load(std::sync::atomic::Ordering::SeqCst), 0);
            assert!(!drain_clientbound(&rx).iter().any(|packet| matches!(packet,
                C::CustomPayload(payload) if payload.channel == CHANNEL
            )));
            let notices = mod_api::__private::take_hook_panics();
            assert!(
                notices
                    .iter()
                    .any(|n| n.mod_id == "review" && n.hook == "recipe_book_place"),
                "expected panic notice naming mod review and recipe_book_place, got {notices:?}"
            );

            assert_eq!(
                game.apply_mod_book_plan(12, &catalog, &display, &plan, false),
                Ok(PostPlace::Nothing)
            );
            assert_eq!(
                panic_calls.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "disabled callback must not run again in the session"
            );
            assert!(
                !mod_api::__private::take_hook_panics()
                    .iter()
                    .any(|n| n.mod_id == "review" && n.hook == "recipe_book_place")
            );

            let (other_pos, other_catalog, other_display) =
                open_mod_menu_named(game, "review:other");
            let other_plan = MenuPlacement::for_mod(
                "review:other".into(),
                Placement::of(
                    vec![Some(Ingredient::of(vec!["minecraft:iron_ore".into()]))],
                    None,
                ),
            )
            .unwrap();
            assert_eq!(
                game.apply_mod_book_plan(12, &other_catalog, &other_display, &other_plan, false),
                Ok(PostPlace::Nothing)
            );
            assert_eq!(other_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            let placed = game.world.block_entities.get(&other_pos).unwrap().slots()[0]
                .as_ref()
                .unwrap();
            assert_eq!(placed.item.as_ref(), "minecraft:iron_ore");
            assert_eq!(placed.count, 1);

            // A committed change still publishes once to the negotiated peer and the hook.
            let player = game.player(12).unwrap();
            let book = mod_api::RecipeBookTypeKey::registered()
                .into_iter()
                .next()
                .unwrap();
            game.storage_transaction(|game, _| {
                assert!(player.set_recipe_book_settings(
                    game,
                    &book,
                    mod_api::RecipeBookSettings {
                        open: true,
                        filtering: true,
                    }
                ));
                assert_eq!(changes.load(std::sync::atomic::Ordering::SeqCst), 0);
                Ok::<(), ()>(())
            })
            .unwrap();
            assert_eq!(changes.load(std::sync::atomic::Ordering::SeqCst), 1);
            let messages: Vec<_> = drain_clientbound(&rx)
                .into_iter()
                .filter_map(|packet| match packet {
                    C::CustomPayload(payload) if payload.channel == CHANNEL => {
                        settings_codec::decode(&payload.data)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                messages,
                [Message::Update {
                    epoch: 1,
                    revision: 1,
                    key: other_catalog.book,
                    flags: BookFlags {
                        open: true,
                        filtering: true
                    },
                }]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn committed_book_settings_are_coalesced_before_a_listener_rewrites_them() {
    use mod_api::{RecipeBookIdentity, RecipeBookSettings};
    use std::sync::{Arc, Mutex};
    use sunrise_protocol::packets::play::Clientbound as C;
    use sunrise_sim::recipe_book::{
        BookFlags, CustomBooks, settings_codec,
        settings_sync::{CHANNEL, Message},
    };

    const COMMITTED: BookFlags = BookFlags {
        open: true,
        filtering: true,
    };
    const REWRITTEN: BookFlags = BookFlags {
        open: false,
        filtering: true,
    };

    struct RewritingMod(Arc<Mutex<Vec<BookFlags>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RewritingMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.recipe_book_type("review:book").unwrap();
        }
        fn player_recipe_book_settings_change(
            &self,
            host: &mut dyn ServerHost,
            event: &mut mod_api::PlayerRecipeBookSettingsChangeEvent<'_>,
        ) {
            self.0.lock().unwrap().push(BookFlags {
                open: event.open,
                filtering: event.filtering,
            });
            if event.open
                && let RecipeBookIdentity::Custom(book) = &event.book
            {
                let player = host.player(event.player).unwrap();
                assert!(player.set_recipe_book_settings(
                    host,
                    book,
                    RecipeBookSettings {
                        open: REWRITTEN.open,
                        filtering: REWRITTEN.filtering,
                    }
                ));
            }
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "review",
        Box::new(RewritingMod(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let (_, catalog, _) = open_mod_menu(game);
            let (tx, rx) = std::sync::mpsc::channel();
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.sender = sunrise_sim::lan::PlayerSender::new(tx, Default::default());
            });
            let player = game.player(12).unwrap();
            let book = mod_api::RecipeBookTypeKey::registered()
                .into_iter()
                .next()
                .unwrap();
            game.storage_transaction(|host, _| {
                for filtering in [false, true] {
                    assert!(player.set_recipe_book_settings(
                        host,
                        &book,
                        RecipeBookSettings {
                            open: true,
                            filtering,
                        }
                    ));
                }
                assert!(seen.lock().unwrap().is_empty());
                assert!(drain_clientbound(&rx).is_empty());
                Ok::<(), ()>(())
            })
            .unwrap();

            assert_eq!(*seen.lock().unwrap(), [COMMITTED, REWRITTEN]);
            let saved = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(12)
                .unwrap()
                .recipe_book
                .write();
            let flags = CustomBooks::read(Some(&saved)).get(book.as_str());
            assert_eq!(flags, REWRITTEN);
            let messages: Vec<_> = drain_clientbound(&rx)
                .into_iter()
                .filter_map(|packet| match packet {
                    C::CustomPayload(payload) if payload.channel == CHANNEL => {
                        settings_codec::decode(&payload.data)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                messages,
                [
                    Message::Update {
                        epoch: 1,
                        revision: 1,
                        key: catalog.book.clone(),
                        flags: COMMITTED
                    },
                    Message::Update {
                        epoch: 1,
                        revision: 2,
                        key: catalog.book,
                        flags
                    },
                ]
            );
        });
    });
}

struct BackedPack;
#[mod_api::__subscriptions]
impl mod_api::Hooks for BackedPack {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let item = r.item_key("minecraft:shulker_box");
        r.item_container(&item, |_: &dyn mod_api::Host| Some(27));
    }
}

/// The menu's part: it places the clicked recipe in a one-slot grid, as a mod would.
struct Placer(std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>);
#[mod_api::sunrise_part]
impl mod_api::MenuPart for Placer {
    fn recipe_book_place(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RecipeBookPlaceEvent<'_>,
    ) {
        self.0.lock().unwrap().push("menu decision");
        *event.placement = mod_api::RecipeBookPlacement::Grid {
            width: 1,
            height: 1,
            slots: vec![0],
            clear: vec![0],
        };
    }
}

struct BackedRouterMod {
    book: std::sync::Mutex<Option<mod_api::RecipeBookTypeKey>>,
    click_order: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for BackedRouterMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_recipe_type("review:furnace");
        r.register_raw_recipe_serializer("review:furnace", "review:furnace");
        let key = r.recipe_book_type("review:book").unwrap();
        *self.book.lock().unwrap() = Some(key.clone());
        r.menu_recipe_book(
            "review:menu",
            &key,
            "review:furnace",
            mod_api::RecipeBookLayout::Mod,
        )
        .unwrap();
        r.menu_part("review:menu", Placer(self.click_order.clone()));
    }
    fn player_recipe_book_click(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::PlayerRecipeBookClickEvent<'_>,
    ) {
        assert_eq!(
            (event.player, event.name, event.recipe, event.all),
            (12, "guest12", "review:recipe", false)
        );
        self.click_order.lock().unwrap().push("general click");
    }
    fn recipe_book_categories(
        &self,
        _: mod_api::HostSide,
        event: &mut mod_api::RecipeBookCategoriesEvent<'_>,
    ) {
        event.categories.push(mod_api::RecipeBookCategory {
            id: "review:cat".into(),
            recipe_type: "review:furnace".into(),
            icon: "minecraft:furnace".into(),
            secondary_icon: None,
            target_menu: "review:menu".into(),
            book: self.book.lock().unwrap().clone().unwrap(),
            search_group: "review:search".into(),
            displays: vec![mod_api::RecipeBookDisplayEntry {
                recipe: "review:recipe".into(),
                displays: vec![router_display(
                    "minecraft:iron_ore",
                    "minecraft:iron_ingot",
                    0.7,
                )],
                placement: Some(mod_api::PlacementInfo {
                    cells: vec![Some(mod_api::Ingredient::of(vec![
                        "minecraft:iron_ore".into(),
                    ]))],
                    shape: None,
                }),
            }],
        });
    }
}

fn drain_clientbound(
    rx: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
) -> Vec<sunrise_protocol::packets::play::Clientbound> {
    use sunrise_protocol::packets::play::Clientbound as C;
    let mut packets = Vec::new();
    for out in rx.try_iter() {
        let bodies = match out {
            sunrise_sim::lan::Out::Packet(body) => vec![body],
            sunrise_sim::lan::Out::Packets(bodies) => bodies,
            sunrise_sim::lan::Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            if let Ok(packet) = C::decode(id, reader.rest()) {
                packets.push(packet);
            }
        }
    }
    packets
}

fn placement_frames_in(
    packets: &[sunrise_protocol::packets::play::Clientbound],
) -> Vec<sync::Frame> {
    use sunrise_protocol::packets::play::Clientbound as C;
    use sunrise_sim::recipe_book::{settings_codec, settings_sync::Message};
    packets
        .iter()
        .filter_map(|packet| match packet {
            C::CustomPayload(payload) if payload.channel == "sunrise:recipe_book_settings" => {
                match settings_codec::decode(&payload.data) {
                    Some(Message::Placement(frame)) => Some(frame),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn drain_placement_frames(
    rx: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
) -> Vec<sync::Frame> {
    placement_frames_in(&drain_clientbound(rx))
}

#[test]
#[ignore = "daily"]
fn a_listening_part_places_the_recipe_through_the_real_stage() {
    use sunrise_protocol::packets::play::Clientbound as C;
    let click_order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("backed_pack_fixture", Box::new(BackedPack)),
        mod_api::ModEntry::new(
            "review",
            Box::new(BackedRouterMod {
                book: Default::default(),
                click_order: click_order.clone(),
            }),
        ),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_level::containers::with_mod_menu_types(
            [(
                "review:menu".into(),
                "review".into(),
                3,
                0,
                vec![2],
                vec![],
                None,
                None,
                true,
                None,
            )],
            || {
                super::super::native_guest_turn_tests::with_guests(|game| {
                    mod_api::__private::resolve_menus(["review:menu"]).unwrap();
                    let recipes = sunrise_sim::crafting::Recipes::from_sources(
                        &game.options.resources,
                        game.items(),
                        &sunrise_world::loot::PackOverlay::NONE,
                        vec![(
                            "review:recipe".into(),
                            serde_json::json!({"type": "review:furnace"}),
                        )],
                        game.res.core.item_data.context(),
                    )
                    .unwrap();
                    game.recipe_tables.install_test_recipes(recipes);
                    game.recipe_tables.books = None;

                    let (_, mut catalog, display) = open_mod_menu(game);
                    game.ensure_mod_book_displays();
                    let generation = game
                        .recipe_tables
                        .books
                        .as_ref()
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .generation();
                    catalog.generation = generation;
                    {
                        let mut server =
                            sunrise_gameplay_common::game::native_book_placement::Server::new(1);
                        server.peer.hello(1, sync::schema()).unwrap();
                        server.peer.publish(catalog.clone()).unwrap();
                        server.displays.insert("review:recipe".into(), 1);
                        game.native_net.placement.servers.insert(12, server);
                    }
                    let (tx, rx) = std::sync::mpsc::channel();
                    let uuid = game
                        .server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(12)
                        .unwrap()
                        .uuid;
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.sender = sunrise_sim::lan::PlayerSender::new(tx, Default::default());
                        g.recipe_book.recipes.insert("review:recipe".into());
                        let open = g.menu.as_mut().unwrap();
                        open.slots = Slots::Native;
                        open.pos = None;
                        g.native_menu = vec![None; open.size];
                        g.inventory[0] = Some(sunrise_items::items::ItemStack::new(
                            "minecraft:shulker_box",
                            1,
                        ));
                        g.inventory[9] = Some(sunrise_items::items::ItemStack::new(
                            "minecraft:iron_ore",
                            3,
                        ));
                    });
                    game.player_menu_opening(12);
                    let menu = game.native_menu_ref(uuid, 1).unwrap();
                    let places = (0..3)
                        .map(|slot| MenuSlotPlace::HeldItem {
                            hand: Hand::Main,
                            slot,
                        })
                        .collect();
                    assert_eq!(menu.back_slots(game, places), Ok(()));
                    // Put an unrelated stack in slot 1 so we prove placement leaves it untouched.
                    let coal = mod_api::ItemStack::new(
                        mod_api::__private::item_key("minecraft:coal").unwrap(),
                        2,
                    )
                    .unwrap();
                    let coal_res = ItemResource::of(&coal);
                    {
                        let host: &mut dyn ServerHost = game;
                        assert_eq!(
                            host.transaction(|host, tx| {
                                if (ItemPlace::MenuSlot { menu, slot: 1 })
                                    .insert(host, tx, &coal_res, 2)
                                    == Ok(2)
                                {
                                    Ok(())
                                } else {
                                    Err(())
                                }
                            }),
                            Ok(())
                        );
                    }
                    let _ = drain_clientbound(&rx);

                    // Live-router placement writes the real handler, updates player inventory,
                    // leaves unrelated slot 1 alone, and syncs via normal container resend.
                    game.guest_mod_book_placement(
                        12,
                        sync::Frame::Place {
                            catalog: catalog.clone(),
                            display: display.clone(),
                            use_max: false,
                        },
                    );
                    assert_eq!(
                        click_order.lock().unwrap().as_slice(),
                        ["general click", "menu decision"]
                    );
                    let packets = drain_clientbound(&rx);
                    assert!(placement_frames_in(&packets).is_empty());
                    assert!(
                        packets
                            .iter()
                            .any(|p| matches!(p, C::ContainerSetContent(c) if c.container == 1))
                    );
                    let placed = {
                        let host: &mut dyn ServerHost = game;
                        ItemPlace::MenuSlot { menu, slot: 0 }
                            .with_stack(host, |stack| stack.project())
                            .flatten()
                            .unwrap()
                    };
                    assert_eq!(
                        placed.item(),
                        mod_api::__private::item_key("minecraft:iron_ore").unwrap()
                    );
                    assert_eq!(placed.count(), 1);
                    let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
                    assert_eq!(
                        guest.native_menu[0]
                            .as_ref()
                            .map(|s| (s.item.as_ref(), s.count)),
                        Some(("minecraft:iron_ore", 1))
                    );
                    assert_eq!(guest.inventory[9].as_ref().unwrap().count, 2);
                    let untouched = {
                        let host: &mut dyn ServerHost = game;
                        ItemPlace::MenuSlot { menu, slot: 1 }
                            .with_stack(host, |stack| stack.project())
                            .flatten()
                            .unwrap()
                    };
                    assert_eq!(
                        untouched.item(),
                        mod_api::__private::item_key("minecraft:coal").unwrap()
                    );
                    assert_eq!(untouched.count(), 2);

                    // Missing inputs clear slot 0 back to player inventory, leave slot 1 alone,
                    // sync the cleared menu, and send one Ghost frame.
                    let dirt = mod_api::ItemStack::new(
                        mod_api::__private::item_key("minecraft:dirt").unwrap(),
                        1,
                    )
                    .unwrap();
                    let dirt_res = ItemResource::of(&dirt);
                    {
                        let host: &mut dyn ServerHost = game;
                        assert_eq!(
                            host.transaction(|host, tx| {
                                let p = ItemPlace::MenuSlot { menu, slot: 0 };
                                let old = p.resource(host).ok_or(())?;
                                let count = p.amount(host);
                                if p.extract(host, tx, &old, count) != Ok(count)
                                    || p.insert(host, tx, &dirt_res, 1) != Ok(1)
                                {
                                    return Err(());
                                }
                                Ok(())
                            }),
                            Ok(())
                        );
                    }
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.inventory[9] = None;
                    });
                    let _ = drain_clientbound(&rx);
                    game.guest_mod_book_placement(
                        12,
                        sync::Frame::Place {
                            catalog: catalog.clone(),
                            display: display.clone(),
                            use_max: false,
                        },
                    );
                    let packets = drain_clientbound(&rx);
                    assert_eq!(
                        placement_frames_in(&packets),
                        vec![sync::Frame::Ghost {
                            catalog: catalog.clone(),
                            recipe: display.recipe.clone(),
                            display: router_display(
                                "minecraft:iron_ore",
                                "minecraft:iron_ingot",
                                0.7,
                            ),
                        }]
                    );
                    let cleared = {
                        let host: &mut dyn ServerHost = game;
                        ItemPlace::MenuSlot { menu, slot: 0 }
                            .with_stack(host, |stack| stack.project())
                            .flatten()
                    };
                    assert!(cleared.is_none());
                    let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
                    assert!(guest.native_menu[0].is_none());
                    assert!(
                        guest
                            .inventory
                            .iter()
                            .flatten()
                            .any(|s| { s.item.as_ref() == "minecraft:dirt" && s.count == 1 })
                    );

                    // A replaced window (new menu generation) refuses and leaves storage intact.
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.menu_generation += 1;
                        g.inventory[9] = Some(sunrise_items::items::ItemStack::new(
                            "minecraft:iron_ore",
                            1,
                        ));
                    });
                    let _ = drain_clientbound(&rx);
                    game.guest_mod_book_placement(
                        12,
                        sync::Frame::Place {
                            catalog,
                            display,
                            use_max: false,
                        },
                    );
                    assert!(drain_placement_frames(&rx).is_empty());
                    assert_eq!(
                        game.server
                            .lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .inventory[9]
                            .as_ref()
                            .unwrap()
                            .count,
                        1
                    );
                });
            },
        );
    });
}

fn router_display(ingredient: &str, result: &str, experience: f32) -> mod_api::RecipeBookDisplay {
    // The server resolves display examples before sending a Ghost. Use concrete stacks
    // so this fixture can compare the exact frame without depending on the fuel registry.
    let stack = |id| {
        mod_api::RecipeSlotDisplay::ItemStack(
            mod_api::ItemStack::new(mod_api::__private::item_key(id).unwrap(), 1).unwrap(),
        )
    };
    mod_api::RecipeBookDisplay::Furnace {
        ingredient: stack(ingredient),
        fuel: stack("minecraft:coal"),
        result: stack(result),
        station: stack("minecraft:furnace"),
        duration: 200,
        experience,
    }
}

struct RouterMod(std::sync::Mutex<Option<mod_api::RecipeBookTypeKey>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for RouterMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_recipe_type("review:furnace");
        r.register_raw_recipe_serializer("review:furnace", "review:furnace");
        let key = r.recipe_book_type("review:book").unwrap();
        *self.0.lock().unwrap() = Some(key.clone());
        r.menu_recipe_book(
            "review:menu",
            &key,
            "review:furnace",
            mod_api::RecipeBookLayout::Furnace {
                input: 0,
                fuel: 1,
                result: 2,
            },
        )
        .unwrap();
    }
    fn recipe_book_categories(
        &self,
        _: mod_api::HostSide,
        event: &mut mod_api::RecipeBookCategoriesEvent<'_>,
    ) {
        event.categories.push(mod_api::RecipeBookCategory {
            id: "review:cat".into(),
            recipe_type: "review:furnace".into(),
            icon: "minecraft:furnace".into(),
            secondary_icon: None,
            target_menu: "review:menu".into(),
            book: self.0.lock().unwrap().clone().unwrap(),
            search_group: "review:search".into(),
            displays: vec![mod_api::RecipeBookDisplayEntry {
                recipe: "review:recipe".into(),
                displays: vec![
                    router_display("minecraft:iron_ore", "minecraft:iron_ingot", 0.7),
                    router_display("minecraft:gold_ore", "minecraft:gold_ingot", 1.0),
                ],
                placement: Some(mod_api::PlacementInfo {
                    cells: vec![Some(mod_api::Ingredient::of(vec![
                        "minecraft:iron_ore".into(),
                    ]))],
                    shape: None,
                }),
            }],
        });
    }
}

#[test]
#[ignore = "daily"]
fn mod_book_router_places_without_ack_and_ghosts_selected_variant() {
    use sunrise_protocol::packets::play::Clientbound as C;
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "review",
        Box::new(RouterMod(Default::default())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_level::containers::with_mod_menu_types(
            [(
                "review:menu".into(),
                "review".into(),
                3,
                0,
                vec![2],
                vec![],
                None,
                None,
                true,
                None,
            )],
            || {
                super::super::native_guest_turn_tests::with_guests(|game| {
                    mod_api::__private::resolve_menus(["review:menu"]).unwrap();
                    let recipes = sunrise_sim::crafting::Recipes::from_sources(
                        &game.options.resources,
                        game.items(),
                        &sunrise_world::loot::PackOverlay::NONE,
                        vec![(
                            "review:recipe".into(),
                            serde_json::json!({"type": "review:furnace"}),
                        )],
                        game.res.core.item_data.context(),
                    )
                    .unwrap();
                    game.recipe_tables.install_test_recipes(recipes);
                    game.recipe_tables.books = None;

                    let (pos, mut catalog, display) = open_mod_menu(game);
                    if let Some(BlockEntity::Mod(block)) = game.world.block_entities.get_mut(&pos) {
                        block.items[1] =
                            Some(sunrise_items::items::ItemStack::new("minecraft:coal", 2));
                    }
                    game.ensure_mod_book_displays();
                    let generation = game
                        .recipe_tables
                        .books
                        .as_ref()
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .generation();
                    catalog.generation = generation;
                    {
                        let mut server =
                            sunrise_gameplay_common::game::native_book_placement::Server::new(1);
                        server.peer.hello(1, sync::schema()).unwrap();
                        server.peer.publish(catalog.clone()).unwrap();
                        server.displays.insert("review:recipe".into(), 2);
                        game.native_net.placement.servers.insert(12, server);
                    }
                    let (tx, rx) = std::sync::mpsc::channel();
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.sender = sunrise_sim::lan::PlayerSender::new(tx, Default::default());
                        g.recipe_book.recipes.insert("review:recipe".into());
                        g.inventory[9] = Some(sunrise_items::items::ItemStack::new(
                            "minecraft:iron_ore",
                            2,
                        ));
                    });
                    // Prime initial container snapshot so sync_containers reports real changes.
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .sync_containers(&game.common.world);
                    let _ = drain_clientbound(&rx);

                    // Ordinary placement updates real slots, leaves unrelated fuel slot 1 intact,
                    // emits no ACK or Ghost frame, and reaches the client via normal container sync.
                    game.guest_mod_book_placement(
                        12,
                        sync::Frame::Place {
                            catalog: catalog.clone(),
                            display: display.clone(),
                            use_max: false,
                        },
                    );
                    assert!(drain_placement_frames(&rx).is_empty());
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[0]
                            .as_ref()
                            .unwrap()
                            .item
                            .as_ref(),
                        "minecraft:iron_ore"
                    );
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[1]
                            .as_ref()
                            .map(|s| (s.item.as_ref(), s.count)),
                        Some(("minecraft:coal", 2))
                    );
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .sync_containers(&game.common.world);
                    let sync_packets = drain_clientbound(&rx);
                    assert!(sync_packets.iter().any(|p| matches!(
                        p,
                        C::ContainerSetSlot(s) if s.container == 1
                    ) || matches!(
                        p,
                        C::ContainerSetContent(c) if c.container == 1
                    )));

                    // Missing ingredients clears slot 0, keeps fuel slot 1, and sends one Ghost frame
                    // for the clicked non-first variant.
                    if let Some(BlockEntity::Mod(block)) = game.world.block_entities.get_mut(&pos) {
                        block.items[0] =
                            Some(sunrise_items::items::ItemStack::new("minecraft:dirt", 1));
                    }
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.inventory[9] = None;
                    });
                    let second_variant = sync::DisplayId {
                        recipe: display.recipe.clone(),
                        source: 1,
                    };
                    game.guest_mod_book_placement(
                        12,
                        sync::Frame::Place {
                            catalog: catalog.clone(),
                            display: second_variant,
                            use_max: false,
                        },
                    );
                    assert_eq!(
                        drain_placement_frames(&rx),
                        vec![sync::Frame::Ghost {
                            catalog: catalog.clone(),
                            recipe: display.recipe.clone(),
                            display: router_display(
                                "minecraft:gold_ore",
                                "minecraft:gold_ingot",
                                1.0,
                            ),
                        }]
                    );
                    assert!(game.world.block_entities.get(&pos).unwrap().slots()[0].is_none());
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[1]
                            .as_ref()
                            .map(|s| (s.item.as_ref(), s.count)),
                        Some(("minecraft:coal", 2))
                    );

                    // A refused placement emits no Ghost/ACK and does not block later placement.
                    game.guest_mod_book_placement(
                        12,
                        sync::Frame::Place {
                            catalog: catalog.clone(),
                            display: sync::DisplayId {
                                recipe: display.recipe.clone(),
                                source: 99,
                            },
                            use_max: false,
                        },
                    );
                    assert!(drain_placement_frames(&rx).is_empty());
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.inventory[9] = Some(sunrise_items::items::ItemStack::new(
                            "minecraft:iron_ore",
                            1,
                        ));
                    });
                    game.guest_mod_book_placement(
                        12,
                        sync::Frame::Place {
                            catalog: catalog.clone(),
                            display: display.clone(),
                            use_max: false,
                        },
                    );
                    assert!(drain_placement_frames(&rx).is_empty());
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[0]
                            .as_ref()
                            .unwrap()
                            .item
                            .as_ref(),
                        "minecraft:iron_ore"
                    );

                    // A replaced window refuses without mutating the block or player inventory.
                    if let Some(BlockEntity::Mod(block)) = game.world.block_entities.get_mut(&pos) {
                        block.items[0] = None;
                    }
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.menu_generation += 1;
                        g.inventory[9] = Some(sunrise_items::items::ItemStack::new(
                            "minecraft:iron_ore",
                            1,
                        ));
                    });
                    game.guest_mod_book_placement(
                        12,
                        sync::Frame::Place {
                            catalog,
                            display,
                            use_max: false,
                        },
                    );
                    assert!(drain_placement_frames(&rx).is_empty());
                    assert!(game.world.block_entities.get(&pos).unwrap().slots()[0].is_none());
                    assert_eq!(
                        game.server
                            .lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .inventory[9]
                            .as_ref()
                            .unwrap()
                            .count,
                        1
                    );
                });
            },
        );
    });
}
