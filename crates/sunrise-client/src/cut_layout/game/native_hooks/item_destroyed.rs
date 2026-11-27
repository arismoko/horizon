#![cfg(test)]
use sunrise_gameplay_common::game::tables::GameItems as _;
use sunrise_items::items::ItemStack;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

#[cfg(test)]
mod item_destroyed_tests {
    use crate::game::native_hooks::item_destroyed::*;
    use mod_api::{
        Hooks, ItemDestroyedEvent, ItemPart, Registrar, RegistrationContext, ServerHost,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::GameGameExt1;

    #[derive(Clone)]
    struct DestroyPart {
        pub item: &'static str,
        drop_item: Option<(&'static str, u32)>,
        write_scoreboard: bool,
        panic: bool,
        seen: Arc<Mutex<Vec<(String, Option<[f64; 3]>)>>>,
    }

    #[mod_api::sunrise_part]
    impl ItemPart for DestroyPart {
        fn item_destroyed(
            &self,
            host: &mut dyn ServerHost,
            event: &mut ItemDestroyedEvent<'_, '_>,
        ) {
            let pos = event.item.entity().position(host);
            self.seen
                .lock()
                .unwrap()
                .push((event.damage_type.to_owned(), pos));
            if self.write_scoreboard {
                mod_api::prelude::ScoreboardServerHost::add_objective(
                    host,
                    "destroyed_write",
                    "dummy",
                    mod_api::Text::literal("destroyed_write"),
                    mod_api::ObjectiveRenderType::Integer,
                )
                .unwrap();
            }
            if let Some((item, count)) = self.drop_item {
                let key = mod_api::__private::item_key(item).unwrap();
                event
                    .drops
                    .push(mod_api::ItemStack::new(key, count).unwrap())
                    .unwrap();
            }
            if self.panic {
                panic!("item_destroyed part failed");
            }
        }
    }

    struct DestroyMod(Vec<DestroyPart>);

    #[mod_api::__subscriptions]
    impl Hooks for DestroyMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            for part in &self.0 {
                let key = r.item_key(part.item);
                r.item_part(key, part.clone());
            }
        }
    }

    fn setup_server_world(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        game.options.dedicated = Some(Vec::new());
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        game.build_registries();
        game.set_allow_commands(true);
        game.world = sunrise_level::world::World::new(game.res.facts().air, 1);
        sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            game,
            ChunkPos::new(0, 0),
        );
        game.server.streaming.forced.insert((0, 0));
        game.update_chunk_tickets();
        assert!(game.entity_ticking_area()(4, 4));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        game.dropped.clear();
    }

    fn tick_until_seen(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        seen: &Arc<Mutex<Vec<(String, Option<[f64; 3]>)>>>,
        max_ticks: usize,
    ) {
        for _ in 0..max_ticks {
            game.tick_world(true).unwrap();
            if !seen.lock().unwrap().is_empty() {
                break;
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn item_destroyed_fires_on_damage_command_and_explosion() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "destroy_drops",
            Box::new(DestroyMod(vec![DestroyPart {
                item: "minecraft:stick",
                drop_item: Some(("minecraft:emerald", 3)),
                write_scoreboard: false,
                panic: false,
                seen: seen.clone(),
            }])),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                setup_server_world(game);

                // 1. Real `/damage` command on a dropped item entity.
                game.spawn_item(
                    ItemStack::new("minecraft:stick", 1),
                    [4.5, 64.0, 4.5],
                    [0.0; 3],
                    0,
                );
                let id = game.dropped.last().unwrap().id;
                assert!(game.as_console(|g| {
                    g.run_command("damage @e[type=item,limit=1] 10 minecraft:generic")
                }));
                assert!(game.dropped.iter().all(|e| e.id != id));
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(&*game.dropped[0].stack.item, "minecraft:emerald");
                assert_eq!(game.dropped[0].stack.count, 3);
                {
                    let log = seen.lock().unwrap();
                    assert_eq!(log.len(), 1);
                    assert_eq!(log[0].0, "minecraft:generic");
                    assert_eq!(log[0].1, Some([4.5, 64.0, 4.5]));
                }

                // 2. Real explosion entry point (`server_explode`).
                seen.lock().unwrap().clear();
                game.dropped.clear();
                game.spawn_item(
                    ItemStack::new("minecraft:stick", 1),
                    [5.5, 64.0, 5.5],
                    [0.0; 3],
                    0,
                );
                let id = game.dropped.last().unwrap().id;
                game.server_explode(
                    [5.5, 64.0, 5.5],
                    2.0,
                    sunrise_gameplay::game::server_side::explosion::BlockInteraction::Keep,
                    false,
                )
                .unwrap();
                assert!(game.dropped.iter().all(|e| e.id != id));
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(&*game.dropped[0].stack.item, "minecraft:emerald");
                assert_eq!(game.dropped[0].stack.count, 3);
                {
                    let log = seen.lock().unwrap();
                    assert_eq!(log.len(), 1);
                    assert_eq!(log[0].0, "minecraft:explosion");
                    assert_eq!(log[0].1, Some([5.5, 64.0, 5.5]));
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_destroyed_fires_on_cactus_fire_and_lava_through_world_tick() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "destroy_env",
            Box::new(DestroyMod(vec![DestroyPart {
                item: "minecraft:stick",
                drop_item: Some(("minecraft:emerald", 3)),
                write_scoreboard: false,
                panic: false,
                seen: seen.clone(),
            }])),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                setup_server_world(game);
                let block_state = |game: &sunrise_gameplay::game::ServerGame<'_>, id: &str| {
                    game.pack.assets.registry.default_state(id).unwrap()
                };

                // 1. Cactus contact through normal `tick_world(true)`.
                let sand = block_state(game, "minecraft:sand");
                let cactus = block_state(game, "minecraft:cactus");
                game.world.set(6, 63, 6, sand).unwrap();
                game.world.set(6, 64, 6, cactus).unwrap();
                game.spawn_item(
                    ItemStack::new("minecraft:stick", 1),
                    [5.93, 64.0, 6.5],
                    [0.0; 3],
                    100,
                );
                tick_until_seen(game, &seen, 10);
                assert_eq!(seen.lock().unwrap().len(), 1);
                assert_eq!(seen.lock().unwrap()[0].0, "minecraft:cactus");
                assert!(
                    game.dropped
                        .iter()
                        .any(|e| &*e.stack.item == "minecraft:emerald" && e.stack.count == 3)
                );

                // 2. Fire block (`minecraft:in_fire`) through normal `tick_world(true)`.
                seen.lock().unwrap().clear();
                game.dropped.clear();
                let netherrack = block_state(game, "minecraft:netherrack");
                let fire = block_state(game, "minecraft:fire");
                game.world.set(8, 63, 6, netherrack).unwrap();
                game.world.set(8, 64, 6, fire).unwrap();
                game.spawn_item(
                    ItemStack::new("minecraft:stick", 1),
                    [8.5, 64.1, 6.5],
                    [0.0; 3],
                    100,
                );
                tick_until_seen(game, &seen, 10);
                assert_eq!(seen.lock().unwrap().len(), 1);
                assert_eq!(seen.lock().unwrap()[0].0, "minecraft:in_fire");
                assert!(
                    game.dropped
                        .iter()
                        .any(|e| &*e.stack.item == "minecraft:emerald" && e.stack.count == 3)
                );

                // 3. Burning item (`minecraft:on_fire`) through normal `tick_world(true)`.
                seen.lock().unwrap().clear();
                game.dropped.clear();
                game.spawn_item(
                    ItemStack::new("minecraft:stick", 1),
                    [3.5, 64.0, 3.5],
                    [0.0; 3],
                    100,
                );
                let burning = game.dropped.last_mut().unwrap();
                burning.health = 1;
                burning.fire_ticks = 20;
                tick_until_seen(game, &seen, 5);
                assert_eq!(seen.lock().unwrap().len(), 1);
                assert_eq!(seen.lock().unwrap()[0].0, "minecraft:on_fire");
                assert!(
                    game.dropped
                        .iter()
                        .any(|e| &*e.stack.item == "minecraft:emerald" && e.stack.count == 3)
                );

                // 4. Lava (`minecraft:lava`) through normal `tick_world(true)`.
                seen.lock().unwrap().clear();
                game.dropped.clear();
                let stone = block_state(game, "minecraft:stone");
                let lava = block_state(game, "minecraft:lava");
                for y in 64..=66 {
                    for (x, z) in [(10, 6), (12, 6), (11, 5), (11, 7)] {
                        game.world.set(x, y, z, stone).unwrap();
                    }
                    game.world.set(11, y, 6, lava).unwrap();
                }
                game.spawn_item(
                    ItemStack::new("minecraft:stick", 1),
                    [11.5, 64.2, 6.5],
                    [0.0; 3],
                    100,
                );
                tick_until_seen(game, &seen, 5);
                assert_eq!(seen.lock().unwrap().len(), 1);
                assert_eq!(seen.lock().unwrap()[0].0, "minecraft:lava");
                assert!(
                    game.dropped
                        .iter()
                        .any(|e| &*e.stack.item == "minecraft:emerald" && e.stack.count == 3)
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_destroyed_fires_from_native_mod_entity_hurt() {
        struct HurtOnTickMod {
            part: DestroyPart,
            armed: Arc<AtomicBool>,
            hurt_results: Arc<Mutex<Vec<Option<bool>>>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for HurtOnTickMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let key = r.item_key(self.part.item);
                r.item_part(key, self.part.clone());
            }

            fn entity_tick_post(
                &self,
                mut host: mod_api::HostSide<'_>,
                event: &mut mod_api::EntityTickPostEvent<'_>,
            ) {
                let mod_api::HostSide::Server(server) = &mut host else {
                    return;
                };
                if !self.armed.load(Ordering::Relaxed) {
                    return;
                }
                if event.kind == "minecraft:item" && self.armed.swap(false, Ordering::Relaxed) {
                    let entity = mod_api::prelude::EntitiesHost::entity(&**server, event.entity)
                        .expect("ticked item entity ref");
                    let hit = entity.hurt(*server, "minecraft:magic", 10.0, None, None);
                    self.hurt_results.lock().unwrap().push(hit);
                }
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let armed = Arc::new(AtomicBool::new(true));
        let hurt_results = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "destroy_entity_hurt",
            Box::new(HurtOnTickMod {
                part: DestroyPart {
                    item: "minecraft:stick",
                    drop_item: Some(("minecraft:emerald", 3)),
                    write_scoreboard: false,
                    panic: false,
                    seen: seen.clone(),
                },
                armed,
                hurt_results: hurt_results.clone(),
            }),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                setup_server_world(game);
                game.spawn_item(
                    ItemStack::new("minecraft:stick", 1),
                    [4.5, 64.0, 4.5],
                    [0.0; 3],
                    100,
                );
                let id = game.dropped.last().unwrap().id;
                game.tick_world(true).unwrap();

                assert_eq!(*hurt_results.lock().unwrap(), vec![Some(true)]);
                assert!(game.dropped.iter().all(|e| e.id != id));
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(&*game.dropped[0].stack.item, "minecraft:emerald");
                assert_eq!(game.dropped[0].stack.count, 3);
                let log = seen.lock().unwrap();
                assert_eq!(log.len(), 1);
                assert_eq!(log[0].0, "minecraft:magic");
                assert!(log[0].1.is_some());
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_destroyed_does_not_fire_on_kill_void_or_despawn() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "destroy_negative",
            Box::new(DestroyMod(vec![DestroyPart {
                item: "minecraft:stick",
                drop_item: Some(("minecraft:emerald", 3)),
                write_scoreboard: false,
                panic: false,
                seen: seen.clone(),
            }])),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                setup_server_world(game);

                // 1. `/kill` discards the item without firing `ItemStack.onDestroyed`.
                game.spawn_item(
                    ItemStack::new("minecraft:stick", 1),
                    [4.5, 64.0, 4.5],
                    [0.0; 3],
                    0,
                );
                assert!(game.as_console(|g| g.run_command("kill @e[type=item]")));
                assert!(game.dropped.is_empty(), "/kill must remove without drops");
                assert!(seen.lock().unwrap().is_empty(), "/kill must not fire hook");

                // 2. Void removal (`y < min_y - 64`) discards without firing `onDestroyed`.
                let below_void = f64::from(game.level_min_y() - 64) - 2.0;
                game.spawn_item(
                    ItemStack::new("minecraft:stick", 1),
                    [4.5, below_void, 4.5],
                    [0.0; 3],
                    0,
                );
                game.tick_world(true).unwrap();
                assert!(game.dropped.is_empty(), "void must remove without drops");
                assert!(seen.lock().unwrap().is_empty(), "void must not fire hook");

                // 3. Lifetime despawn (`age >= 6000`) discards without firing `onDestroyed`.
                game.spawn_item(
                    ItemStack::new("minecraft:stick", 1),
                    [4.5, 64.0, 4.5],
                    [0.0; 3],
                    100,
                );
                game.dropped.last_mut().unwrap().age = 5999;
                game.tick_world(true).unwrap();
                assert!(game.dropped.is_empty(), "despawn must remove without drops");
                assert!(
                    seen.lock().unwrap().is_empty(),
                    "despawn must not fire hook"
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_destroyed_bundle_listener_sees_original_contents_before_spill() {
        #[derive(Clone)]
        struct BundleInspector {
            observed: Arc<Mutex<Vec<(usize, usize, usize)>>>,
        }

        fn bundle_entry_count(stack: &mod_api::ItemStack) -> usize {
            match stack.components().and_then(|c| {
                c.as_nbt()
                    .get(&mod_api::JavaString::from("minecraft:bundle_contents"))
            }) {
                Some(mod_api::Tag::List(entries)) => entries.len(),
                _ => 0,
            }
        }

        #[mod_api::sunrise_part]
        impl ItemPart for BundleInspector {
            fn item_destroyed(
                &self,
                host: &mut dyn ServerHost,
                event: &mut ItemDestroyedEvent<'_, '_>,
            ) {
                let on_event = bundle_entry_count(event.stack);
                let on_entity = event
                    .item
                    .with_stack(host, |s| s.project())
                    .flatten()
                    .map_or(0, |s| bundle_entry_count(&s));
                self.observed
                    .lock()
                    .unwrap()
                    .push((on_event, on_entity, event.drops.len()));
            }
        }

        struct InspectorMod(BundleInspector);

        #[mod_api::__subscriptions]
        impl Hooks for InspectorMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let key = r.item_key("minecraft:bundle");
                r.item_part(key, self.0.clone());
            }
        }

        let observed = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "bundle_inspector",
            Box::new(InspectorMod(BundleInspector {
                observed: observed.clone(),
            })),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                setup_server_world(game);
                let make_bundle = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                    let mut bundle = ItemStack::new("minecraft:bundle", 1);
                    let mut contents =
                        sunrise_items::bundle::Contents::of(game.items(), &bundle).unwrap();
                    let mut iron = ItemStack::new("minecraft:iron_ingot", 4);
                    assert_eq!(contents.try_insert(game.items(), &mut iron, true), 4);
                    contents.store(game.items(), &mut bundle);
                    bundle
                };

                // 1. `/damage` path: both event.stack and event.item.with_stack(host, ...) still carry
                // the original bundle contents, and event.drops starts with them.
                let bundle = make_bundle(game);
                game.spawn_item(bundle, [3.5, 64.0, 3.5], [0.0; 3], 0);
                assert!(game.as_console(|g| {
                    g.run_command("damage @e[type=item,limit=1] 10 minecraft:on_fire")
                }));
                assert_eq!(*observed.lock().unwrap(), vec![(1, 1, 1)]);
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(&*game.dropped[0].stack.item, "minecraft:iron_ingot");
                assert_eq!(game.dropped[0].stack.count, 4);

                // 2. Explosion path (`server_explode`): same original-stack visibility.
                observed.lock().unwrap().clear();
                game.dropped.clear();
                let bundle = make_bundle(game);
                game.spawn_item(bundle, [4.5, 64.0, 4.5], [0.0; 3], 0);
                game.server_explode(
                    [4.5, 64.0, 4.5],
                    2.0,
                    sunrise_gameplay::game::server_side::explosion::BlockInteraction::Keep,
                    false,
                )
                .unwrap();
                assert_eq!(*observed.lock().unwrap(), vec![(1, 1, 1)]);
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(&*game.dropped[0].stack.item, "minecraft:iron_ingot");
                assert_eq!(game.dropped[0].stack.count, 4);

                // 3. Normal `tick_world(true)` lava destruction path: same original-stack visibility.
                observed.lock().unwrap().clear();
                game.dropped.clear();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let lava = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:lava")
                    .unwrap();
                for y in 64..=66 {
                    for (x, z) in [(5, 6), (7, 6), (6, 5), (6, 7)] {
                        game.world.set(x, y, z, stone).unwrap();
                    }
                    game.world.set(6, y, 6, lava).unwrap();
                }
                let bundle = make_bundle(game);
                game.spawn_item(bundle, [6.5, 64.2, 6.5], [0.0; 3], 100);
                for _ in 0..5 {
                    game.tick_world(true).unwrap();
                    if !observed.lock().unwrap().is_empty() {
                        break;
                    }
                }
                assert_eq!(*observed.lock().unwrap(), vec![(1, 1, 1)]);
                assert!(
                    game.dropped
                        .iter()
                        .any(|e| &*e.stack.item == "minecraft:iron_ingot" && e.stack.count == 4)
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_destroyed_panic_rolls_back_writes_and_keeps_vanilla_bundle_drops() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "destroy_panic",
            Box::new(DestroyMod(vec![DestroyPart {
                item: "minecraft:bundle",
                drop_item: Some(("minecraft:diamond", 5)),
                write_scoreboard: true,
                panic: true,
                seen: seen.clone(),
            }])),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                setup_server_world(game);
                let mut bundle = ItemStack::new("minecraft:bundle", 1);
                let mut contents =
                    sunrise_items::bundle::Contents::of(game.items(), &bundle).unwrap();
                let mut iron = ItemStack::new("minecraft:iron_ingot", 4);
                assert_eq!(contents.try_insert(game.items(), &mut iron, true), 4);
                contents.store(game.items(), &mut bundle);

                game.spawn_item(bundle, [3.5, 64.0, 3.5], [0.0; 3], 0);
                let id = game.dropped.last().unwrap().id;
                assert!(game.as_console(|g| {
                    g.run_command("damage @e[type=item,limit=1] 10 minecraft:on_fire")
                }));

                // Destroyed item is still removed; scoreboard write and panicking drops rolled back;
                // vanilla bundle contents still spilled cleanly.
                assert!(game.dropped.iter().all(|e| e.id != id));
                assert!(game.score.board.objective("destroyed_write").is_none());
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(&*game.dropped[0].stack.item, "minecraft:iron_ingot");
                assert_eq!(game.dropped[0].stack.count, 4);
                assert_eq!(seen.lock().unwrap().len(), 1);
            });
        });
    }
}
