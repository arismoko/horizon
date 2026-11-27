//! A client start publishes its models only once the world-rest worker it started is joined, so
//! the renderer table never reads the rest tables before they exist (#1377).
#![cfg(feature = "client")]

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::initial_client_models_tests_client::with_loading_client;

#[test]
#[ignore = "daily"]
fn title_ticks_join_world_resources_without_the_desktop_model_publication() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Reader(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Reader {
        fn client_tick_post(
            &self,
            host: &mut dyn mod_api::ClientHost,
            _: &mut mod_api::ClientTickPostEvent,
        ) {
            // This Host method reads items in
            // WorldRest, even when there is no open level.
            let stone = host
                .item_key(&mod_api::Identifier::parse("minecraft:stone").unwrap())
                .unwrap();
            assert_eq!(host.max_stack_size(stone), Some(64));
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let reads = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "title_reader",
        Box::new(Reader(reads.clone())),
    )]
    .into();
    with_loading_client(|game| {
        mod_platform::testing::with_mods(mods, || {
            // The scoped mod fixture has its own key table, independent of startup's.
            mod_api::__private::resolve_items(["minecraft:stone"]).unwrap();
            // Alternate window frontends need not have run publish_initial_client_models.
            let saves = tempfile::tempdir().unwrap();
            game.test_open_title(saves.path().to_path_buf());
            assert!(game.rest_loader.is_some());
            game.client_tick_steps(0, None).unwrap();
            assert!(game.rest_loader.is_some(), "an empty frame need not wait");
            game.client_tick_steps(1, None).unwrap();
            assert!(game.rest_loader.is_none(), "the tick joined the loader");
            assert!(
                game.res.rest.get().is_some(),
                "menu hooks have their tables"
            );
            assert_eq!(
                reads.load(Ordering::Relaxed),
                1,
                "the menu listener ran successfully, not skipped or quarantined"
            );
        });
    });
}
