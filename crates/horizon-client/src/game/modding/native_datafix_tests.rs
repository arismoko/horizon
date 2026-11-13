//! `upgrade_block_entity_nbt` and `upgrade_entity_nbt` through a server host, as a mod calls them: an old villager
//! takes its profession, and a future version is refused by name.
use super::native_entity_refs::tests::with_game;
use horizon_core::nbt::{Compound, Tag, parse_snbt};

fn compound(text: &str) -> Compound {
    match parse_snbt(text).unwrap().0 {
        Tag::Compound(c) => c,
        other => panic!("expected a compound, got {other:?}"),
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mods_host_upgrades_old_data_and_refuses_a_future_version() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let host: &dyn mod_api::ServerHost = game;
            // DataVersion 1900: Profession 1 with Career 2 is a cartographer.
            let villager =
                compound("{id: \"minecraft:villager\", Profession: 1, Career: 2, CareerLevel: 1}");
            let upgraded = host.upgrade_entity_nbt(1900, villager).unwrap();
            let Some(Tag::Compound(data)) = upgraded.get(&"VillagerData".into()) else {
                panic!("VillagerData: {upgraded:?}");
            };
            assert_eq!(
                data.get(&"profession".into()),
                Some(&Tag::String("minecraft:cartographer".into()))
            );
            let sign = compound("{id: \"minecraft:sign\"}");
            let current = horizon_world::datafix::CURRENT;
            assert_eq!(
                host.upgrade_block_entity_nbt(current + 1, sign),
                Err(format!(
                    "data version {} is newer than the game's ({current})",
                    current + 1
                ))
            );
        });
    });
}
