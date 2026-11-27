use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_gameplay_common::game::ItemStack;
use {
    crate::game::server_side::item_use::interact_vibration_tests::with_game,
    sunrise_gameplay::game::server_side::sculk_server::item_use_event_fixture::EVENTS,
};

#[test]
#[ignore = "daily"]
fn bundle_completes_on_tick_200() {
    struct Listener(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn living_entity_use_item_finish(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::LivingEntityUseItemFinishEvent<'_>,
        ) {
            assert_eq!(
                mod_api::__private::item_key_id(event.result.as_ref().unwrap().item()),
                Some("minecraft:bundle")
            );
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "bundle-finish",
        Box::new(Listener(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game("minecraft:bundle", |game| {
            game.held_mut().as_mut().unwrap().data_mut().bundle_contents = vec![
                ItemStack::new("minecraft:stone", 1),
                ItemStack::new("minecraft:dirt", 1),
                ItemStack::new("minecraft:stick", 1),
            ];
            game.server_set_acting_use(true);
            game.server.lan_acting = Some(1);
            EVENTS.with(|events| events.borrow_mut().clear());
            assert!(game.start_using_item());
            for _ in 0..199 {
                game.tick_using_server();
            }
            assert_eq!(game.using, Some((game.selected, 199)));
            assert!(game.server.guest_entity_events.is_empty());
            assert_eq!(calls.load(Ordering::Relaxed), 0);
            EVENTS.with(|events| {
                assert!(
                    events
                        .borrow()
                        .iter()
                        .all(|e| e.0 != "minecraft:item_interact_finish")
                )
            });
            game.tick_using_server();
            assert_eq!(game.using, None);
            assert_eq!(game.server.guest_entity_events, vec![9]);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            EVENTS.with(|events| {
                assert_eq!(
                    events
                        .borrow()
                        .iter()
                        .filter(|e| e.0 == "minecraft:item_interact_finish")
                        .cloned()
                        .collect::<Vec<_>>(),
                    vec![(
                        "minecraft:item_interact_finish",
                        [10.5, 64.0, -3.25],
                        Some(sunrise_sim::mobs::Who::Player)
                    )]
                )
            });
            game.tick_using_server();
            assert_eq!(game.server.guest_entity_events, vec![9]);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        })
        .expect("prepared resources required");
    });
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
