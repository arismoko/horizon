#![cfg(test)]
use mod_api::Placement;
use sunrise_core::position::BlockPos;
#[cfg(test)]
mod entity_place_tests {
    use crate::game::native_hooks::entity_place::*;
    use mod_api::{EntityPlaceEvent, Flow, Hooks, ModEntry, ServerHost};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    /// Records what `entity_place` is told and answers with `cancel`.
    struct Placer {
        pub seen: Arc<Mutex<Vec<(BlockPos, Placement)>>>,
        cancel: bool,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Placer {
        fn entity_place(&self, _host: &mut dyn ServerHost, event: &mut EntityPlaceEvent) -> Flow {
            self.seen
                .lock()
                .unwrap()
                .push((event.pos, event.placement.clone()));
            if self.cancel {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    /// A bed placement reaches the hook as one `Multi` of both halves, a
    /// cancel is reported, and with no listener nothing is asked.
    #[test]
    #[ignore = "daily"]
    fn a_multi_placement_reaches_the_hook_once_and_cancels() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "place_probe",
            Box::new(Placer {
                seen: seen.clone(),
                cancel: true,
            }),
        )];
        with_mods(mods.into(), || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let (foot, head) = (BlockPos::new(1, 64, 1), BlockPos::new(1, 64, 2));
                let state = game.world.block(0, 0, 0);
                assert!(game.place_canceled(1, foot, state, state, Some((head, state))));
            });
        });
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, BlockPos::new(1, 64, 1));
        assert!(matches!(&seen[0].1, Placement::Multi(blocks) if blocks.len() == 2));
        with_mods(Vec::new().into(), || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let at = BlockPos::new(1, 64, 1);
                let state = game.world.block(0, 0, 0);
                assert!(!game.place_canceled(1, at, state, state, None));
            });
        });
    }
}
