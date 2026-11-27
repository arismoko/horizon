use std::sync::Arc;
use sunrise_items::items::ItemStack;
use sunrise_protocol::connection::Pipe;
use {
    sunrise_gameplay_common::game::stations_common::Kind,
    sunrise_gameplay_common::game::stations_common::Station,
};

/// A client on a server whose data pack adds `fixture:drill`, which the
/// game's own table doesn't have: the registry sync sent its definition.
fn with_client_on_a_server_with_a_custom_enchantment(
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "AnvilTest").unwrap();
        let definition = serde_json::json!({
            "description": "Drill", "supported_items": "minecraft:diamond_pickaxe",
            "weight": 1, "max_level": 3, "min_cost": {"base": 1, "per_level_above_first": 0},
            "max_cost": {"base": 1, "per_level_above_first": 0}, "anvil_cost": 1,
            "slots": ["mainhand"], "effects": {}
        });
        let synced = game
            .stations()
            .enchantments
            .with_entries([("fixture:drill", &definition)])
            .expect("fixture:drill is a valid enchantment")
            .expect("the game's own table lacks fixture:drill");
        game.client.remote.as_mut().unwrap().enchantments = Some(Arc::new(synced));
        test(game);
    });
}

/// `AnvilMenu.createResult` on the client reads the synced registry, so a
/// book of a data pack's enchantment on a pickaxe shows its result, as the
/// server computes it, instead of the red cross (#8688).
#[test]
#[ignore = "daily"]
fn a_servers_custom_enchantment_book_gives_an_anvil_result() {
    with_client_on_a_server_with_a_custom_enchantment(|game| {
        let mut book = ItemStack::new("minecraft:enchanted_book", 1);
        book.set_crafting_enchantments(vec![("fixture:drill".into(), 2)]);
        let mut station = Station::new(Kind::Anvil, [0, 64, 0]);
        station.inputs[0] = Some(ItemStack::new("minecraft:diamond_pickaxe", 1));
        station.inputs[1] = Some(book);
        game.open_station_client(station);

        game.station_changed();

        let station = game.station.as_ref().unwrap();
        let result = station.result.as_ref().expect("an anvil result");
        assert_eq!(&*result.item, "minecraft:diamond_pickaxe");
        assert_eq!(
            result.enchantments(),
            &[(Arc::<str>::from("fixture:drill"), 2)]
        );
        assert!(station.anvil.cost > 0);
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::stations_common::StationsCommonGameExt1 as _;
