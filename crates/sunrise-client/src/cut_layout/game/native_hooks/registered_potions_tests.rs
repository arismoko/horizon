//! `ItemsClientHost::potions` lists vanilla's potions in registry order with their ids.
use sunrise_gameplay_common::game::native_hooks::registered_potions::registered_potions;

/// The list is `Potions`' registration order, namespaced, and built once.
#[test]
fn vanilla_potions_in_registry_order_with_ids() {
    let ids: Vec<&str> = registered_potions().iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids.len(), 46);
    assert_eq!(
        ids[..6],
        [
            "minecraft:water",
            "minecraft:mundane",
            "minecraft:thick",
            "minecraft:awkward",
            "minecraft:night_vision",
            "minecraft:long_night_vision",
        ]
    );
    assert_eq!(
        ids[ids.len() - 4..],
        [
            "minecraft:wind_charged",
            "minecraft:weaving",
            "minecraft:oozing",
            "minecraft:infested",
        ]
    );
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len());
    assert!(std::ptr::eq(registered_potions(), registered_potions()));
}

/// The game's read is the prepared `registries.json` report's `minecraft:potion` registry, id for
/// id in protocol-id order.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn host_read_matches_the_prepared_potion_registry() {
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let report = sunrise_protocol::registry::Builtin::find(&root).unwrap();
        let builtin = sunrise_protocol::registry::Builtin::load(&report).unwrap();
        let expected = &builtin.get("minecraft:potion").unwrap().names;
        let listed: Vec<String> = game.potions().into_iter().map(|p| p.id).collect();
        assert_eq!(&listed, expected);
    });
}
