//! A client of a server lists and numbers its stonecutter buttons by the list the server sent
//! (`ClientboundUpdateRecipesPacket`), whatever recipes the client has itself.

use super::*;
use crate::recipe_book::PacketNames;
use sunrise_protocol::packets::recipes::{HolderSet, SlotDisplay};

const REGISTRY: [&str; 4] = [
    "minecraft:stone",
    "minecraft:stone_bricks",
    "minecraft:stone_slab",
    "minecraft:stone_stairs",
];

fn names<T>(test: impl FnOnce(&PacketNames) -> T) -> T {
    let item = |id: i32| {
        REGISTRY
            .get(usize::try_from(id).ok()?)
            .map(|n| (*n).to_owned())
    };
    let category = |_: i32| None;
    let tag = |_: &str| Vec::new();
    test(&PacketNames {
        item: &item,
        category: &category,
        component: &|_| None,
        tag: &tag,
    })
}

/// The server's order, its inputs and its results come through as the client's recipes, an
/// unknown registry id keeping its place.
#[test]
fn the_servers_list_becomes_the_clients_recipes_in_its_order() {
    let sent = [
        (HolderSet::Items(vec![0]), SlotDisplay::Item(2)),
        (HolderSet::Items(vec![0, 1]), SlotDisplay::Item(3)),
        (HolderSet::Items(vec![9]), SlotDisplay::Item(77)),
        (HolderSet::Items(vec![0]), SlotDisplay::Item(1)),
    ];
    let recipes = names(|names| names.stonecutting(&sent, |_| None));
    assert_eq!(
        recipes.iter().map(|r| &*r.result.item).collect::<Vec<_>>(),
        [
            "minecraft:stone_slab",
            "minecraft:stone_stairs",
            "minecraft:air",
            "minecraft:stone_bricks"
        ],
    );
    assert_eq!(
        recipes.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["server:0", "server:1", "server:2", "server:3"],
    );
}

/// The client's own table lists bricks before slab; the server's list has the slab first, and
/// the client's menu follows the server's.
#[test]
fn a_client_menu_lists_the_servers_stonecutter_recipes_not_its_own() {
    with_menu(Kind::Stonecutter, |env, menu| {
        let sent = [
            (HolderSet::Items(vec![0]), SlotDisplay::Item(2)),
            (HolderSet::Items(vec![0]), SlotDisplay::Item(1)),
        ];
        let server = names(|names| {
            env.station_recipes
                .with_stonecutting(names.stonecutting(&sent, |_| None))
        });
        let env = Env {
            station_recipes: &server,
            ..*env
        };
        menu.on_client = true;
        menu.set_slot(&env, 0, stack("stone", 1));
        let listed: Vec<_> = menu
            .stone
            .recipes
            .iter()
            .map(|&r| &*server.stonecutting[r].result.item)
            .collect();
        assert_eq!(listed, ["minecraft:stone_slab", "minecraft:stone_bricks"]);
        // The button the server will read as 1 is the client's second entry.
        assert!(menu.button(&env, 1));
        assert_eq!(
            menu.slot(1),
            None,
            "a client's result is the server's slot update"
        );
    });
}
