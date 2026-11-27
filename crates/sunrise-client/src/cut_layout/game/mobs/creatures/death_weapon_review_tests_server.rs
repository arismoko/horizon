use super::tests::WeaponProbe;
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::TryIntoApi;
use sunrise_items::items::ItemStack;

pub(crate) fn fixture_with_source(
    expected: Option<ItemStack>,
    expected_source: Option<(&'static str, i32)>,
    run: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        game.server.world_root = scratch.path().to_path_buf();
        game.server.save.0 = Arc::new(sunrise_sim::save::SaveDir::new(
            scratch.path().join("save"),
            sunrise_sim::save::LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        game.build_registries();
        game.server.next_entity = 1000;
        let loot = sunrise_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server.server_loot.0.set(Some(Arc::new(loot)));
        let seen = Arc::new(Mutex::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "review_weapon",
            Box::new(WeaponProbe {
                expected: expected.map(|stack| stack.try_into_api().unwrap()),
                expected_source,
                swap: None,
                seen: seen.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || run(game));
        assert_eq!(*seen.lock().unwrap(), 1);
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
