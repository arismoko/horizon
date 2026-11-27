#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::growth_features::install_live_features;

    use std::sync::Arc;

    /// The live features follow the world being opened: a world with a data
    /// pack's feature sees it, the next world without one does not (it used to
    /// keep the first world's registry for the whole process), and re-opening
    /// the same packs reuses the registry built for them.
    #[test]
    #[ignore = "daily"]
    fn the_live_features_follow_the_world_being_opened() {
        // Other feature fixtures can evict the process-wide last-built cache.
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::client_side::growth::tests::the_live_features_follow_the_world_being_opened",
            "SUNRISE_LIVE_FEATURE_CACHE_CHILD",
            &["reopen"],
        ) {
            return;
        }
        use sunrise_world::worldgen::overrides::WorldgenOverrides;
        crate::game::native_entity_refs::tests::with_game(|game| {
            let mut with_tree = WorldgenOverrides::default();
            with_tree.add_file("mymod", "data/mymod/worldgen/feature/tree.json", || {
                Some(r#"{"type":"minecraft:no_op","config":{}}"#.to_owned())
            });
            let has_tree = |game: &crate::game::ServerGame<'_>| {
                let features = game.res.behaviours().live_features().unwrap();
                assert!(features.configured("minecraft:oak").is_some());
                features.configured("mymod:tree").is_some()
            };
            let open = |game: &crate::game::ServerGame<'_>, overrides: &WorldgenOverrides| {
                install_live_features(&game.res, &game.pack, &game.options, overrides)
            };
            open(game, &WorldgenOverrides::default());
            assert!(!has_tree(game));
            open(game, &with_tree);
            assert!(has_tree(game));
            let first = game.res.behaviours().live_features().unwrap();
            open(game, &with_tree);
            assert!(Arc::ptr_eq(
                &first,
                &game.res.behaviours().live_features().unwrap()
            ));
            open(game, &WorldgenOverrides::default());
            assert!(!has_tree(game));
        });
    }
}
