//! A recipe with several displays reaches a connected player's recipe book:
//! `ServerRecipeBook.addRecipes` sends a `ClientboundRecipeBookAddPacket.Entry`
//! for each display (`RecipeManager.listDisplaysForRecipe`), so
//! `minecraft:map_cloning`, a transmute recipe with a material count of 1 to 8,
//! sends eight (tools/test_recipe_book_variants_parity.py measures the same on
//! vanilla 26.3 with a real client).
use crate::game::load_resources;
use crate::settings::Settings;
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::Options;
use sunrise_gameplay_common::game::Progress;
#[cfg(test)]
use sunrise_gameplay_common::game::execution_role;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_sim::{
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};

const GUEST: &str = "RecipeGuest";

/// A host on a stone floor with one connected, placed guest (conn 2).
fn game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> (
    sunrise_gameplay::game::ServerGame<'a>,
    mpsc::Receiver<sunrise_sim::lan::Out>,
) {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res = resources.insert(loaded.resources);
    let mut game = sunrise_gameplay::game::ServerGame::new(
        res,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    for x in 0..2 {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
    }
    assert!(game.run_command("fill 0 99 0 31 99 15 stone"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(28.5, 100.0, 10.5)).unwrap());
    let net = NetConfig {
        announce: false,
        ..NetConfig::lan(0, "recipe-variants-test".into(), 2)
    };
    game.lan_host(
        sunrise_gameplay::game::server_side::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        net,
        HubConfig {
            default_game_mode: 1,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 2,
            uuid: 2,
            name: GUEST.into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        2,
        [12.5, 100.0, 10.5],
        1,
    );
    guest.placed = true;
    guest.loaded = true;
    // Its view, as the client sent it: straight down.
    guest.pitch = 90.0;
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    (game, rx)
}

/// Runs `command`: whether it succeeded and what the issuer was told.
fn run(game: &mut sunrise_gameplay::game::ServerGame<'_>, command: &str) -> (bool, Vec<String>) {
    game.server.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.server.chat_capture.take().unwrap_or_default();
    (
        ok,
        said.iter()
            .map(sunrise_game_data::text::Text::plain)
            .collect(),
    )
}

/// The display ids of every recipe book Add the guest was sent.
fn added(rx: &mpsc::Receiver<sunrise_sim::lan::Out>) -> Vec<Vec<i32>> {
    use sunrise_protocol::packets::play::Clientbound as C;
    let mut result = Vec::new();
    for out in rx.try_iter() {
        let bodies = match out {
            sunrise_sim::lan::Out::Packet(body) => vec![body],
            sunrise_sim::lan::Out::Packets(bodies) => bodies,
            sunrise_sim::lan::Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            if let Ok(C::RecipeBookAdd(add)) = C::decode(id, reader.rest()) {
                result.push(add.entries.iter().map(|e| e.id).collect());
            }
        }
    }
    result
}

fn step(game: &mut sunrise_gameplay::game::ServerGame<'_>, ticks: usize) {
    for _ in 0..ticks {
        assert!(game.run_command("tick step 1"));
        game.tick_server(false).unwrap();
    }
}

fn grant(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    rx: &mpsc::Receiver<sunrise_sim::lan::Out>,
    recipe: &str,
) -> Vec<Vec<i32>> {
    let _ = added(rx);
    let (ok, said) = run(game, &format!("recipe give {GUEST} {recipe}"));
    assert!(ok, "{said:?}");
    // The book's packets go out with a later tick's sync (measured: the next
    // one but one); two ticks take them all.
    step(game, 2);
    added(rx)
}

#[test]
#[ignore = "daily"]
fn a_recipe_with_eight_displays_sends_eight_entries() {
    let mut resources = None;
    let (mut game, rx) = game(&mut resources);
    assert!(game.run_command("tick freeze"));
    // The join's own book (a new player knows the crafting table) settles on
    // these ticks; then the book is emptied.
    step(&mut game, 3);
    let _ = run(&mut game, &format!("recipe take {GUEST} *"));
    step(&mut game, 2);
    let _ = added(&rx);
    let ordinary = grant(&mut game, &rx, "minecraft:crafting_table");
    assert_eq!(
        ordinary.iter().map(Vec::len).sum::<usize>(),
        1,
        "the ordinary recipe: {ordinary:?}"
    );
    let cloning = grant(&mut game, &rx, "minecraft:map_cloning");
    let ids: Vec<i32> = cloning.concat();
    assert_eq!(ids.len(), 8, "one entry per display: {cloning:?}");
    let mut distinct = ids.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 8);
}

fn ghosts(
    rx: &mpsc::Receiver<sunrise_sim::lan::Out>,
) -> Vec<sunrise_protocol::packets::recipes::PlaceGhostRecipe> {
    use sunrise_protocol::packets::play::Clientbound as C;
    let mut out_ghosts = Vec::new();
    for out in rx.try_iter() {
        let bodies = match out {
            sunrise_sim::lan::Out::Packet(body) => vec![body],
            sunrise_sim::lan::Out::Packets(bodies) => bodies,
            sunrise_sim::lan::Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            if let Ok(C::PlaceGhostRecipe(ghost)) = C::decode(id, reader.rest()) {
                out_ghosts.push(ghost);
            }
        }
    }
    out_ghosts
}

#[test]
#[ignore = "daily"]
fn clicked_multi_display_recipe_sends_exact_ghost_and_ordinary_placement_sends_none() {
    use sunrise_protocol::packets::{
        play::Serverbound as S,
        recipes::{PlaceRecipe, RecipeDisplay},
    };
    use sunrise_sim::recipe_book::{BookType, PacketNames, ghost_from_display};
    let mut resources = None;
    let (mut game, rx) = game(&mut resources);
    assert!(game.run_command("setblock 12 100 11 minecraft:crafting_table"));
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.game_mode = 0;
        g.recipe_book.recipes.insert("minecraft:map_cloning".into());
        g.recipe_book
            .recipes
            .insert("minecraft:crafting_table".into());
    });
    game.lan_open_crafting(2, [12, 100, 11]);
    let window = game
        .server
        .lan
        .as_ref()
        .unwrap()
        .menus
        .get(&2)
        .and_then(|m| m.crafting.table.as_ref().map(|(w, ..)| *w))
        .expect("crafting table window open");
    let _ = ghosts(&rx);

    let cloning_ids = game
        .stations()
        .recipe_index
        .display_ids("minecraft:map_cloning");
    assert_eq!(cloning_ids.len(), 8);
    // Display #3 is 1 filled map + 4 empty maps (5 ingredients), distinct from display #0 (2).
    let chosen_display = cloning_ids.start + 3;
    game.lan_guest_packet(
        2,
        S::PlaceRecipe(PlaceRecipe {
            container: window,
            recipe: chosen_display,
            use_max: false,
        }),
    );
    let sent = ghosts(&rx);
    assert_eq!(sent.len(), 1, "missing ingredients sends one ghost packet");
    assert_eq!(sent[0].container, window);
    let RecipeDisplay::Shapeless { ingredients, .. } = &sent[0].display else {
        panic!(
            "expected shapeless map_cloning display, got {:?}",
            sent[0].display
        );
    };
    assert_eq!(
        ingredients.len(),
        5,
        "ghost carries the clicked non-first display variant"
    );
    // Client ghost projection reads p.display directly (1 result + 5 input grid cells).
    let sync = game.server.lan.as_ref().unwrap().hub.sync_data();
    let item = |id: i32| {
        sync.builtin
            .get("minecraft:item")
            .and_then(|ids| ids.name(id))
            .map(str::to_owned)
    };
    let category = |id: i32| {
        sync.builtin
            .get("minecraft:recipe_book_category")
            .and_then(|ids| ids.name(id))
            .map(str::to_owned)
    };
    let tag = |name: &str| {
        game.items()
            .item_tag_order
            .get(name)
            .cloned()
            .unwrap_or_default()
    };
    let projected = ghost_from_display(
        &sent[0].display,
        &PacketNames {
            item: &item,
            category: &category,
            component: &|_| None,
            tag: &tag,
        },
        BookType::Crafting,
        3,
        true,
        game.items(),
        |stack| item(stack.item).map(|id| ItemStack::new(&id, stack.count as u32)),
    );
    assert_eq!(projected.len(), 6);

    // Wrong container window does not place or send a ghost.
    game.lan_guest_packet(
        2,
        S::PlaceRecipe(PlaceRecipe {
            container: 0,
            recipe: chosen_display,
            use_max: false,
        }),
    );
    assert!(ghosts(&rx).is_empty());

    // Ordinary placement moves real items into the crafting grid and emits no ghost packet.
    let crafting_display = game
        .stations()
        .recipe_index
        .display_ids("minecraft:crafting_table")
        .start;
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.inventory[0] = Some(ItemStack::new("minecraft:oak_planks", 4));
    });
    game.lan_guest_packet(
        2,
        S::PlaceRecipe(PlaceRecipe {
            container: window,
            recipe: crafting_display,
            use_max: false,
        }),
    );
    assert!(
        ghosts(&rx).is_empty(),
        "ordinary placement emits no ghost packet"
    );
    let placed_count: usize = game
        .server
        .lan
        .as_ref()
        .unwrap()
        .menus
        .get(&2)
        .and_then(|m| m.crafting.table.as_ref())
        .map(|(_, _, menu, _)| {
            (1..=9)
                .filter(|&slot| {
                    menu.slot(slot)
                        .is_some_and(|s| s.item.as_ref() == "minecraft:oak_planks")
                })
                .count()
        })
        .unwrap_or(0);
    assert_eq!(placed_count, 4);
}

/// A writable recipe-click listener that replaces the crafting window with a chest: the click's
/// placement must not move the ingredients into the crafting grid it named, nor send a ghost.
#[test]
#[ignore = "daily"]
fn a_recipe_click_listener_replacing_the_menu_stops_the_placement() {
    use sunrise_protocol::packets::{play::Serverbound as S, recipes::PlaceRecipe};
    struct Replacer;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Replacer {
        fn player_recipe_book_click(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::PlayerRecipeBookClickEvent<'_>,
        ) {
            let player = host.player(event.player).unwrap();
            player
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
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("replacer", Box::new(Replacer))].into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_gameplay::game::server_side::native_guest_turn_tests::with_guests(|game| {
            let (tx, rx) = mpsc::channel();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let table = game
                .pack
                .assets
                .registry
                .default_state("minecraft:crafting_table")
                .unwrap();
            game.world.set(3, 64, 6, table).unwrap();
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.sender = PlayerSender::new(tx, Arc::default());
                g.game_mode = 0;
                g.recipe_book
                    .recipes
                    .insert("minecraft:crafting_table".into());
                g.inventory[0] = Some(ItemStack::new("minecraft:oak_planks", 4));
            });
            game.lan_open_crafting(12, [3, 64, 6]);
            let window = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .menus
                .get(&12)
                .and_then(|m| m.crafting.table.as_ref().map(|(w, ..)| *w))
                .expect("crafting table window open");
            let _ = ghosts(&rx);
            let display = game
                .stations()
                .recipe_index
                .display_ids("minecraft:crafting_table")
                .start;
            game.lan_guest_packet(
                12,
                S::PlaceRecipe(PlaceRecipe {
                    container: window,
                    recipe: display,
                    use_max: false,
                }),
            );
            let planks = game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0]
                .as_ref()
                .map(|s| s.count);
            assert_eq!(planks, Some(4), "no ingredient left the inventory");
            assert!(ghosts(&rx).is_empty(), "no ghost for the replaced menu");
        });
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
use sunrise_gameplay_common::game::tables::GameItems as _;
