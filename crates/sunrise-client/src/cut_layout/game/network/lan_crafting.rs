#![cfg(test)]
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_level::containers::ModMenuType;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_sim::lan::menu::Slots;
#[cfg(test)]
mod machine_recipe_placement_tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_core::position::ChunkPos;
    use sunrise_protocol::packets::{
        configuration::ClientInformation,
        play::{Serverbound, UseItemOn},
        recipes::PlaceRecipe,
    };
    use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

    #[test]
    #[ignore = "daily"]
    fn h234_registered_place_recipe_reaches_actual_furnace_and_preserves_controls() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            assert!(game.world.loaded(8, 8));
            game.lan_host(
                sunrise_gameplay::game::lan::LanSettings {
                    port: 0,
                    guest_commands: true,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::loopback(0, "machine-place-test".into(), 2)
                },
                HubConfig {
                    default_game_mode: 0,
                    guest_commands: true,
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
            let (tx, _rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 2,
                    uuid: 234,
                    name: "RecipeGuest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                2,
                [8.5, 200.0, 8.5],
                2,
            );
            guest.dimension = "minecraft:overworld".into();
            guest.placed = true;
            guest.loaded = true;
            guest.selected = 1;
            guest.permission = 2;
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            let clock = game.world.game_time;
            let planks = game
                .stations()
                .recipe_index
                .first_display("minecraft:oak_planks")
                .unwrap();
            let charcoal = game
                .stations()
                .recipe_index
                .first_display("minecraft:charcoal")
                .unwrap();
            let command = |game: &mut sunrise_gameplay::game::ServerGame<'_>, text: &str| {
                assert!(game.as_guest(2, |g| g.run_command(text)).unwrap(), "{text}");
            };
            let place = |game: &mut sunrise_gameplay::game::ServerGame<'_>, container, recipe| {
                game.lan_guest_packet(
                    2,
                    Serverbound::PlaceRecipe(PlaceRecipe {
                        container,
                        recipe,
                        use_max: false,
                    }),
                );
            };
            let open = |game: &mut sunrise_gameplay::game::ServerGame<'_>, pos| {
                game.lan_guest_packet(
                    2,
                    Serverbound::UseItemOn(UseItemOn {
                        hand: 0,
                        pos,
                        face: 1,
                        cursor: [0.5, 1.0, 0.5],
                        inside: false,
                        world_border: false,
                        sequence: 234,
                    }),
                );
                assert!(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(2)
                        .unwrap()
                        .ack_up_to
                        .get()
                        >= 234
                );
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .menus
                    .get(&2)
                    .and_then(|p| p.crafting.table.as_ref())
                    .map(|(window, ..)| *window)
                    .or_else(|| {
                        game.server
                            .lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(2)
                            .unwrap()
                            .menu
                            .as_ref()
                            .map(|m| m.window)
                    })
                    .expect("actual table or machine window opened")
            };
            command(game, "recipe give @s minecraft:oak_planks");
            command(game, "recipe give @s minecraft:charcoal");
            assert!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(2)
                    .unwrap()
                    .recipe_book
                    .knows("minecraft:charcoal")
            );
            command(game, "item replace entity @s hotbar.0 with oak_log 3");
            place(game, 0, planks);
            assert_eq!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(2)
                    .unwrap()
                    .craft_grid[0],
                Some(ItemStack::new("minecraft:oak_log", 1))
            );
            assert_eq!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(2)
                    .unwrap()
                    .craft_result,
                Some(ItemStack::new("minecraft:oak_planks", 4))
            );
            assert_eq!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(2)
                    .unwrap()
                    .inventory[0]
                    .as_ref()
                    .unwrap()
                    .count,
                2
            );
            game.lan_guest_packet(
                2,
                Serverbound::ContainerClose(sunrise_protocol::packets::play::ContainerClose {
                    container: 0,
                }),
            );
            assert!(game.run_command("setblock 10 200 8 crafting_table"));
            let window = open(game, [10, 200, 8]);
            place(game, window, planks);
            let (_, _, table, _) = game.server.lan.as_ref().unwrap().menus[&2]
                .crafting
                .table
                .as_ref()
                .unwrap();
            assert_eq!(
                table.slot(0),
                Some(&ItemStack::new("minecraft:oak_planks", 4))
            );
            game.guest_close_all(2);
            let mut failures = Vec::new();
            for (label, count, known) in [
                ("place", 3, true),
                ("ghost", 0, true),
                ("forgotten", 3, false),
            ] {
                game.as_guest(2, |g| {
                    g.run_command("clear @s");
                })
                .unwrap();
                if count > 0 {
                    command(game, "item replace entity @s hotbar.0 with oak_log 3");
                }
                command(
                    game,
                    "item replace entity @s hotbar.8 with stick[minecraft:custom_data={h234:1}] 7",
                );
                assert!(game.run_command("setblock 10 200 8 air"));
                assert!(game.run_command("setblock 10 200 8 furnace"));
                assert!(game.run_command("item replace block 10 200 8 container.1 with coal[minecraft:custom_data={h234:2}] 2"));
                let window = open(game, [10, 200, 8]);
                if !known {
                    command(game, "recipe take @s minecraft:charcoal");
                }
                assert_eq!(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(2)
                        .unwrap()
                        .recipe_book
                        .knows("minecraft:charcoal"),
                    known
                );
                let before = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(2)
                    .unwrap()
                    .inventory
                    .clone();
                let pos = (10, 200, 8);
                let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
                    panic!("actual furnace missing")
                };
                assert!(f.items[0].is_none() && f.items[2].is_none());
                let fuel = f.items[1].clone();
                assert_eq!(fuel.as_ref().unwrap().count, 2);
                place(game, window, charcoal);
                let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
                    panic!("furnace removed")
                };
                assert_eq!(f.items[1], fuel);
                assert!(f.items[2].is_none());
                let mut expected = before.clone();
                let input = if label == "place" {
                    expected[0].as_mut().unwrap().count = 2;
                    Some(ItemStack::new("minecraft:oak_log", 1))
                } else {
                    None
                };
                if f.items[0] != input
                    || game
                        .server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(2)
                        .unwrap()
                        .inventory
                        != expected
                {
                    failures.push((
                        label,
                        f.items.clone(),
                        game.server
                            .lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(2)
                            .unwrap()
                            .inventory
                            .clone(),
                    ));
                }
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guest(2).unwrap().uuid,
                    234
                );
                assert_eq!(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(2)
                        .unwrap()
                        .health,
                    20.0
                );
                assert_eq!(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(2)
                        .unwrap()
                        .position,
                    [8.5, 200.0, 8.5]
                );
                assert_eq!(game.world.game_time, clock);
                game.guest_close_all(2);
            }
            assert!(
                failures.is_empty(),
                "actual registered PlaceRecipe furnace failures: {failures:?}"
            );
        });
    }
}

#[cfg(test)]
mod recipe_check_tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_level::containers::{ModBlockEntity, ModBlockEntityFields, ModCrafting};
    use sunrise_protocol::packets::configuration::ClientInformation;
    use sunrise_sim::lan::recipes;
    use sunrise_sim::lan::{
        Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig, menu::Menu as HubMenu,
    };

    const STATION: (i32, i32, i32) = (4, 200, 4);
    const WINDOW: i32 = 7;

    /// What a check must leave alone: the station's block entity, the guest's
    /// inventory, grid and cursor, its menus' slots, and the station's chunk
    /// unmarked as changed (`container_changed` marks it; this takes the mark).
    fn state(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> String {
        let changed = game
            .world
            .modified
            .remove(&(STATION.0 >> 4, STATION.2 >> 4));
        let host = game.server.lan.as_ref().unwrap();
        let g = &host.hub.guests[0];
        format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {:?}",
            game.world.block_entities.get(&STATION),
            g.inventory,
            g.craft_grid,
            g.carried,
            g.menu.as_ref().map(|m| (&m.sent, m.state, &m.craft)),
            host.menus
                .get(&2)
                .is_some_and(|p| p.crafting.machine.is_some()),
            changed,
        )
    }

    /// The packets sent to the guest since the last call.
    fn sent(rx: &mpsc::Receiver<Out>) -> Vec<C> {
        let mut bodies = Vec::new();
        for out in rx.try_iter() {
            match out {
                Out::Packet(body) => bodies.push(body),
                Out::Packets(more) => bodies.extend(more),
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            }
        }
        bodies
            .iter()
            .map(|body| {
                let mut r = sunrise_protocol::buf::Reader::new(body);
                let id = r.varint().unwrap();
                C::decode(id, r.rest()).unwrap()
            })
            .collect()
    }

    /// A check's reply: the window, the tag, the result byte and the missing cells.
    fn reply(packets: &[C]) -> (i32, i32, u8, Vec<u8>) {
        let [C::CustomPayload(p)] = packets else {
            panic!("expected only the reply, got {packets:?}");
        };
        assert_eq!(p.channel, recipes::CHECK_CHANNEL);
        let mut r = sunrise_protocol::buf::Reader::new(&p.data);
        let (window, tag, result) = (r.varint().unwrap(), r.varint().unwrap(), r.u8().unwrap());
        let mut cells = Vec::new();
        if result == 1 {
            for _ in 0..r.varint().unwrap() {
                cells.push(r.u8().unwrap());
            }
        }
        assert_eq!(r.remaining(), 0);
        (window, tag, result, cells)
    }

    fn request(window: i32, tag: i32, id: &str) -> Vec<u8> {
        let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
        w.varint(window);
        w.varint(tag);
        w.string(id);
        w.data
    }

    /// A check on a guest's mod crafting station answers as the transfer
    /// would and changes nothing: no slot, no block entity, no chunk marked
    /// changed, no packet but its reply. The transfer after it does move.
    #[test]
    #[ignore = "daily"]
    fn a_check_answers_and_leaves_the_world_and_the_guest_alone() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.lan_host(
                sunrise_gameplay::game::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::loopback(0, "recipe-check-test".into(), 2)
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
            let station: &'static ModMenuType = Box::leak(Box::new(ModMenuType {
                id: "test:station",
                owner: "test",
                slots: 10,
                player_inventory: true,
                data_slots: 0,
                owned_slots: None,
                results: vec![],
                inputs: Vec::new(),
                links: None,
                crafting: Some(ModCrafting {
                    grid: 1,
                    size: 3,
                    result: 0,
                }),
            }));
            let mut items = vec![None; 10];
            items[5] = Some(ItemStack::new("minecraft:cobblestone", 1));
            game.world.block_entities.insert(
                STATION,
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "test:station".into(),
                    ModBlockEntityFields {
                        items,
                        ..Default::default()
                    },
                ))),
            );
            let (tx, rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 2,
                    uuid: 2640,
                    name: "CheckGuest".into(),
                    properties: Vec::new(),
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
            guest.inventory[0] = Some(ItemStack::new("minecraft:oak_planks", 2));
            guest.menu = Some(HubMenu {
                window: WINDOW,
                kind: 0,
                registry_id: "test:station".into(),
                pos: Some(STATION),
                slots: Slots::Block(STATION),
                size: 10,
                state: 0,
                sent: vec![None; 46],
                drag: None,
                sent_data: Vec::new(),
                reported_unsent_data_len: None,
                blockless_data: Vec::new(),

                mod_type: Some(station),
                craft: None,
                links: Vec::new(),
                asks_still_valid: false,
                opening_item: None,
                opener_block: None,
                removed: false,
                slot_changed_seen: None,
            });
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            sent(&rx);
            game.world.modified.clear();
            let before = state(game);
            let check = |game: &mut sunrise_gameplay::game::ServerGame<'_>, window, tag, id| {
                game.guest_recipe_check(2, &request(window, tag, id));
                assert_eq!(state(game), before, "a check of {id} changed something");
                reply(&sent(&rx))
            };
            // Two planks make a stick; the grid's cobblestone has room to go.
            assert_eq!(
                check(game, WINDOW, 1, "minecraft:stick"),
                (WINDOW, 1, 0, vec![])
            );
            // A crafting table's last two planks lack.
            assert_eq!(
                check(game, WINDOW, 2, "minecraft:crafting_table"),
                (WINDOW, 2, 1, vec![2, 3])
            );
            // An unknown recipe, and a window the guest doesn't have open:
            // not into this menu.
            assert_eq!(
                check(game, WINDOW, 3, "minecraft:no_such_recipe"),
                (WINDOW, 3, 3, vec![])
            );
            assert_eq!(
                check(game, WINDOW + 1, 4, "minecraft:stick"),
                (WINDOW + 1, 4, 3, vec![])
            );
            // The transfer the first check answered for does move them.
            game.guest_recipe_transfer(2, &{
                let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
                w.varint(WINDOW);
                w.string("minecraft:stick");
                w.data
            });
            assert_ne!(state(game), before, "the transfer moved nothing");
        });
    }

    /// Ten checks at once are answered, the next is dropped, and one more
    /// goes through after one tick.
    #[test]
    #[ignore = "daily"]
    fn checks_past_the_limit_go_unanswered() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let (tx, rx) = mpsc::channel();
            let host = game.server.lan.as_mut().unwrap();
            host.hub.guests.update(0, |g| {
                g.brand = "sunrise".into();
                g.sender = PlayerSender::new(tx, Arc::default());
            });
            let conn = host.hub.guests[0].conn;
            for tag in 0..11 {
                game.guest_recipe_check(conn, &request(0, tag, "minecraft:stick"));
            }
            let replies = sent(&rx);
            assert_eq!(replies.len(), 10);
            game.tick_guest_connections(false).unwrap();
            drop(sent(&rx));
            game.guest_recipe_check(conn, &request(0, 11, "minecraft:stick"));
            assert_eq!(reply(&sent(&rx)).1, 11);
        });
    }
}

#[cfg(test)]
#[path = "lan_crafting/lending_tests.rs"]
mod lending_tests;

#[cfg(test)]
#[path = "lan_crafting/stonecutter_stat_tests.rs"]
mod stonecutter_stat_tests;

#[cfg(test)]
mod empty_ingredient_placement_tests {
    struct Predicate;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Predicate {
        fn ingredient(
            &self,
            _: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::IngredientEvent<'_>,
        ) -> Option<mod_api::IngredientDefinition> {
            let kind = event.kind;
            (kind == "empty_fixture:predicate")
                .then(|| mod_api::IngredientDefinition::new(vec![], |_, _| true))
        }
    }

    use super::*;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_gameplay_common::world_recipes;
    use sunrise_gameplay_common::world_stations;
    use sunrise_protocol::packets::{configuration::ClientInformation, play, recipes::PlaceRecipe};
    use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

    #[test]
    #[ignore = "daily"]
    fn server_packet_refuses_retained_empty_ingredient_recipes() {
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("empty_fixture", Box::new(Predicate))].into(),
            || {
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    let dir = std::env::temp_dir()
                        .join(format!("sunrise-empty-ingredients-{}", std::process::id()));
                    let pack = dir.join("datapacks/empty");
                    std::fs::create_dir_all(pack.join("data/empty_fixture/recipe")).unwrap();
                    std::fs::create_dir_all(pack.join("data/empty_fixture/tags/item")).unwrap();
                    std::fs::write(
                        pack.join("pack.mcmeta"),
                        r#"{"pack":{"description":"fixture"}}"#,
                    )
                    .unwrap();
                    std::fs::write(
                        pack.join("data/empty_fixture/tags/item/empty.json"),
                        r#"{"values":[]}"#,
                    )
                    .unwrap();
                    std::fs::write(
                        dir.join("datapacks.json"),
                        r#"{"enabled":["file/empty"],"disabled":[]}"#,
                    )
                    .unwrap();
                    for (id, ingredient) in [
                        ("direct", serde_json::json!([])),
                        ("missing", serde_json::json!("#empty_fixture:missing")),
                        ("tag", serde_json::json!("#empty_fixture:empty")),
                        (
                            "custom",
                            serde_json::json!({"type":"empty_fixture:predicate"}),
                        ),
                    ] {
                        std::fs::write(
                            pack.join(format!("data/empty_fixture/recipe/{id}.json")),
                            serde_json::json!({"type":"minecraft:crafting_shaped","pattern":["A"],
                        "key":{"A":ingredient},"result":{"id":"minecraft:stick"}})
                            .to_string(),
                        )
                        .unwrap();
                    }
                    std::fs::write(
                        pack.join("data/empty_fixture/recipe/a_invalid.json"),
                        serde_json::json!({"type":"minecraft:crafting_transmute",
                    "input":"minecraft:stone","material":"#empty_fixture:missing",
                    "result":{"id":"minecraft:stick"}})
                        .to_string(),
                    )
                    .unwrap();
                    std::fs::write(
                        pack.join("data/empty_fixture/recipe/b_neighbor.json"),
                        serde_json::json!({"type":"minecraft:crafting_shaped","pattern":["A"],
                    "key":{"A":"minecraft:stone"},"result":{"id":"minecraft:stick"}})
                        .to_string(),
                    )
                    .unwrap();
                    game.server.world_root = dir.clone();
                    let resources = game.options.resources.clone();
                    let stack = sunrise_gameplay::game::functions::pack_stack(
                        &dir,
                        &resources,
                        &sunrise_assets::resource_source::ResourceSource::directory(
                            resources.clone(),
                        ),
                        &Default::default(),
                        false,
                        &[],
                        &[],
                    );
                    // Malformed recipes abort registry loading; keep each refusal isolated
                    // from the valid recipes whose empty ingredients we test over the wire.
                    let invalid = ["direct", "missing", "a_invalid"].map(|id| {
                        let path = pack.join(format!("data/empty_fixture/recipe/{id}.json"));
                        let json = std::fs::read(&path).unwrap();
                        std::fs::remove_file(&path).unwrap();
                        (id, path, json)
                    });
                    for (id, path, json) in invalid {
                        std::fs::write(&path, json).unwrap();
                        let error = game
                            .stage_world_tables(&stack, false, &resources)
                            .err()
                            .expect("malformed recipe must refuse the load")
                            .to_string();
                        assert!(
                            error.contains(&format!("data/empty_fixture/recipe/{id}.json")),
                            "{error}"
                        );
                        std::fs::remove_file(path).unwrap();
                    }
                    game.server.world_tables.0 = game
                        .stage_world_tables(&stack, false, &resources)
                        .unwrap()
                        .tables;
                    game.build_registries();
                    {
                        // The generated Host entrypoint serves mod callers;
                        // check its real answer directly (Ingredient holds in-process
                        // predicates and has no Wire form).
                        let host: &dyn mod_api::Host = game;
                        let empty = host
                            .recipe_ingredient(r##""#empty_fixture:empty""##)
                            .unwrap()
                            .unwrap();
                        assert!(empty.items.is_empty());
                        assert!(empty.order.is_empty());
                        assert!(
                            host.recipe_ingredient(r##""#empty_fixture:missing""##)
                                .unwrap()
                                .is_none()
                        );
                        assert!(host.recipe_ingredient("[]").unwrap().is_none());
                        let stone = mod_api::ItemStack::of(
                            mod_api::__private::item_key("minecraft:stone").unwrap(),
                        );
                        assert!(!empty.test(host, &stone));
                    }
                    game.lan_host(
                        sunrise_gameplay::game::lan::LanSettings {
                            port: 0,
                            guest_commands: true,
                            force_game_mode: false,
                        },
                        NetConfig {
                            announce: false,
                            ..NetConfig::loopback(0, "empty-ingredient-test".into(), 2)
                        },
                        HubConfig {
                            default_game_mode: 0,
                            guest_commands: true,
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
                    let (tx, _rx) = mpsc::channel();
                    let mut guest = Guest::new(
                        Joined {
                            conn: 2,
                            uuid: 3739,
                            name: "EmptyIngredientGuest".into(),
                            properties: vec![],
                            info: ClientInformation::new(2),
                            brand: "test".into(),
                            sender: PlayerSender::new(tx, Arc::default()),
                            latency: Arc::new(AtomicI32::new(0)),
                            address: "127.0.0.1:25565".parse().unwrap(),
                            expected: false,
                        },
                        2,
                        [8.5, 200.0, 8.5],
                        2,
                    );
                    guest.dimension = "minecraft:overworld".into();
                    guest.placed = true;
                    guest.loaded = true;
                    guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 16));
                    guest.craft_grid[0] = Some(ItemStack::new("minecraft:dirt", 3));
                    let ids = ["empty_fixture:tag", "empty_fixture:custom"].map(str::to_owned);
                    let index = &world_stations!(game).recipe_index;
                    for id in [
                        "empty_fixture:direct",
                        "empty_fixture:missing",
                        "empty_fixture:a_invalid",
                        "empty_fixture:tag",
                        "empty_fixture:custom",
                    ] {
                        assert!(index.first_display(id).is_none(), "{id}");
                    }
                    assert_eq!(guest.recipe_book.add_recipes(index, &ids), (0, vec![]));
                    guest
                        .recipe_book
                        .add_recipes(index, &["empty_fixture:b_neighbor".to_owned()]);
                    let sync = game.server.lan.as_ref().unwrap().hub.sync_data();
                    assert!(sync.advancements.recipes_added(&ids, sync).is_none());
                    let display = index.first_display("empty_fixture:b_neighbor").unwrap();
                    assert_eq!(
                        sync.advancements.recipe_of_display(display),
                        Some("empty_fixture:b_neighbor")
                    );
                    {
                        use sunrise_protocol::{
                            buf::{Codec, Reader},
                            packets::recipes::RecipeBookAdd,
                        };
                        let requested = [
                            ids[0].clone(),
                            ids[1].clone(),
                            "empty_fixture:b_neighbor".to_owned(),
                        ];
                        let bytes = sync.advancements.recipes_added(&requested, sync).unwrap();
                        let packet = RecipeBookAdd::read(&mut Reader::new(&bytes)).unwrap();
                        assert_eq!(packet.entries.len(), 1);
                        assert_eq!(packet.entries[0].id, display);
                    }
                    for id in &ids {
                        assert!(!guest.recipe_book.knows(id), "{id}");
                        assert!(world_recipes!(game).placement(id).unwrap().impossible());
                        assert!(
                            world_recipes!(game).by_type["minecraft:crafting"]
                                .iter()
                                .any(|recipe| &recipe.id == id)
                        );
                    }
                    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
                    for display in [-1, i32::MAX] {
                        game.lan_guest_packet(
                            2,
                            play::Serverbound::PlaceRecipe(PlaceRecipe {
                                container: 0,
                                recipe: display,
                                use_max: true,
                            }),
                        );
                        let guest = game.server.lan.as_ref().unwrap().hub.guest(2).unwrap();
                        assert_eq!(
                            guest.inventory[0],
                            Some(ItemStack::new("minecraft:stone", 16))
                        );
                        assert_eq!(
                            guest.craft_grid[0],
                            Some(ItemStack::new("minecraft:dirt", 3))
                        );
                        assert!(guest.craft_grid[1..].iter().all(Option::is_none));
                        assert!(guest.craft_result.is_none());
                    }
                    let display = world_stations!(game)
                        .recipe_index
                        .first_display("empty_fixture:b_neighbor")
                        .unwrap();
                    game.lan_guest_packet(
                        2,
                        play::Serverbound::PlaceRecipe(PlaceRecipe {
                            container: 0,
                            recipe: display,
                            use_max: false,
                        }),
                    );
                    let guest = game.server.lan.as_ref().unwrap().hub.guest(2).unwrap();
                    assert_eq!(guest.inventory[0].as_ref().unwrap().count, 15);
                    assert_eq!(
                        guest.craft_grid[0],
                        Some(ItemStack::new("minecraft:stone", 1))
                    );
                    assert_eq!(
                        guest.craft_result,
                        Some(ItemStack::new("minecraft:stick", 1))
                    );
                    std::fs::remove_dir_all(dir).unwrap();
                });
            },
        );
    }
}
use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
