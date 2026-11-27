#![cfg(test)]
use sunrise_core::position::BlockPos;

#[cfg(test)]
mod landing_crossing_tests {
    use crate::game::native_hooks::block_landing::*;
    use mod_api::{BlockPart, FallOnEvent, Hooks, LevelRandomHost, ModEntry, Registrar};
    use std::sync::{Arc, Mutex};
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_gameplay_common::game::native_hooks::HitLevel;

    #[derive(Clone, Default)]
    struct LandingPart(Arc<Mutex<Option<(mod_api::BlockState, bool)>>>);
    #[mod_api::sunrise_part]
    impl BlockPart for LandingPart {
        fn fall_on(&self, _: &mut dyn LevelRandomHost, event: &mut FallOnEvent<'_>) {
            let (state, invalid) = self.0.lock().unwrap().unwrap();
            event.distance = 99.0;
            event.damage_multiplier = 2.0;
            event.request(mod_api::LandingWrite::SetBlock(state));
            if invalid {
                event.request(mod_api::LandingWrite::SetBlock(
                    mod_api::__private::block_state_from_ordinal(state.block(), u16::MAX),
                ));
            }
        }
    }
    struct LandingMod(LandingPart);
    #[mod_api::__subscriptions]
    impl Hooks for LandingMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, self.0.clone());
        }
    }

    #[test]
    #[ignore = "daily"]
    fn landing_crossing_refuses_invalid_writes_and_sibling_damage() {
        let part = LandingPart::default();
        let mods: mod_api::Mods = vec![ModEntry::new(
            "landing_atomic",
            Box::new(LandingMod(part.clone())),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let state = stone.try_into_api().unwrap();
                let original =
                    vec![sunrise_world::level::LandingWrite::DestroyBlock { drop: false }];
                for invalid in [false, true] {
                    *part.0.lock().unwrap() = Some((state, invalid));
                    let mut distance = 5.0;
                    let mut multiplier = 1.0;
                    let mut writes = original.clone();
                    let mut level = sunrise_sim::health::TestLevel::new(0);
                    HitLevel(level.level()).native_fall_on(
                        sunrise_gameplay_common::game::native_hooks::fire::FallOnCall {
                            dimension: "minecraft:overworld",
                            entity: 1,
                            kind: "minecraft:player",
                            state: stone,
                            pos: BlockPos::new(1, 64, 1),
                            distance: &mut distance,
                            damage_multiplier: &mut multiplier,
                            writes: &mut writes,
                        },
                    );
                    if invalid {
                        assert_eq!(
                            (distance, multiplier),
                            (5.0, 1.0),
                            "invalid write must not commit sibling inputs"
                        );
                        assert_eq!(writes, original, "valid prefix must not reach engine queue");
                    } else {
                        assert_eq!(
                            (distance, multiplier),
                            (99.0, 2.0),
                            "the real registered callback fired"
                        );
                        assert_eq!(
                            writes,
                            vec![
                                original[0],
                                sunrise_world::level::LandingWrite::SetBlock(stone)
                            ]
                        );
                    }
                }
            });
        });
    }
}
