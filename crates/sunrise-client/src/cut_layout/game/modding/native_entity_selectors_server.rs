#![cfg(test)]
use sunrise_command::args::Selector;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::SelectorSort;
    use std::sync::{Arc, Mutex};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::commands::Target;

    struct Teammates(Arc<Mutex<Vec<(mod_api::CommandSource, u32)>>>);
    impl mod_api::EntitySelectorType for Teammates {
        fn base(&self) -> mod_api::SelectorBase {
            mod_api::SelectorBase::new(false, Some(1), SelectorSort::Nearest, "test.allies")
        }
        fn matches(
            &self,
            _: &mut dyn mod_api::ServerHost,
            source: &mod_api::CommandSource,
            entity: mod_api::EntityRef,
        ) -> bool {
            self.0.lock().unwrap().push((source.clone(), entity.id()));
            matches!(entity.id(), 112 | 115)
        }
    }
    struct PanickingMatches(Arc<Mutex<Vec<u32>>>);
    impl mod_api::EntitySelectorType for PanickingMatches {
        fn base(&self) -> mod_api::SelectorBase {
            mod_api::SelectorBase::new(false, Some(1), SelectorSort::Nearest, "test.broken")
        }
        fn matches(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mod_api::CommandSource,
            entity: mod_api::EntityRef,
        ) -> bool {
            self.0.lock().unwrap().push(entity.id());
            match entity.id() {
                112 | 115 => true,
                113 => panic!("fixture predicate failure"),
                _ => false,
            }
        }
    }

    struct TeamMod {
        allies_calls: Arc<Mutex<Vec<(mod_api::CommandSource, u32)>>>,
        broken_calls: Arc<Mutex<Vec<u32>>>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for TeamMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_entity_selector("allies", Teammates(self.allies_calls.clone()))
                .unwrap();
            r.register_entity_selector("broken", PanickingMatches(self.broken_calls.clone()))
                .unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn nearest_teammate_filters_before_limit_and_drops_false_and_panicking_candidates() {
        let allies_calls = Arc::new(Mutex::new(Vec::new()));
        let broken_calls = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "selector_fixture",
            Box::new(TeamMod {
                allies_calls: allies_calls.clone(),
                broken_calls: broken_calls.clone(),
            }),
        )]
        .into();
        let _ = mod_api::testing::rebuild_parts(|| -> Result<(), ()> {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, || {
                sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
                    game.build_registries();
                    // A body and level UUID alone do not identify a server command
                    // source. Enter a real guest turn so its UUID/name/permission
                    // and entity identity are installed by the production path.
                    game.as_guest(12, |game| {
                        sunrise_gameplay::game::native_entity_refs::tests::fixture_resident_column(
                            &mut game.world,
                            (0, 0),
                        );
                        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                        for (id, x) in [(111, 1.0), (112, 2.0), (113, 1.5), (114, 0.5), (115, 3.0)]
                        {
                            game.mobs.push(sunrise_sim::mobs::Mob::new(
                                id,
                                kind.clone(),
                                Vec3::new(x, 0.0, 0.0),
                                0.0,
                            ));
                        }
                        let selector = |token: &str| Selector {
                            kind: 'e',
                            include_entities: true,
                            token: Some(token.into()),
                            limit: 1,
                            sort: sunrise_command::args::Sort::Nearest,
                            pos: [Some(0.0); 3],
                            ..Default::default()
                        };

                        assert_eq!(game.select(&selector("allies")), [Target::Mob(112)]);
                        {
                            let calls = allies_calls.lock().unwrap();
                            for id in [111, 112, 113, 114, 115] {
                                let (source, _) = calls
                                    .iter()
                                    .find(|(_, candidate)| *candidate == id)
                                    .expect("every mob must be filtered before limit");
                                let mod_api::CommandSource::Player { player, .. } = source else {
                                    panic!("expected the acting player source");
                                };
                                assert_eq!(player.uuid(), 12);
                                assert_eq!(player.entity().id(), 12);
                            }
                        }

                        // A panicking selector disables only itself for the rest of the session,
                        // breaks out before remaining candidates, and emits a panic notice while
                        // its sibling selector keeps working.
                        let _ = mod_api::__private::take_hook_panics();
                        assert_eq!(game.select(&selector("broken")), [Target::Mob(112)]);
                        let calls_after_panic = broken_calls.lock().unwrap().len();
                        assert!(
                            !broken_calls.lock().unwrap().contains(&114)
                                && !broken_calls.lock().unwrap().contains(&115),
                            "poisoned selector must stop without evaluating remaining candidates"
                        );
                        let notices: Vec<_> = mod_api::__private::take_hook_panics()
                            .into_iter()
                            .filter(|notice| notice.mod_id == "selector_fixture")
                            .collect();
                        assert_eq!(notices.len(), 1);
                        assert_eq!(notices[0].hook, "entity_selector.matches");

                        assert!(game.select(&selector("broken")).is_empty());
                        assert_eq!(broken_calls.lock().unwrap().len(), calls_after_panic);
                        assert_eq!(
                            sunrise_gameplay_common::game::native_entity_selectors::selector_tokens()
                                .iter()
                                .map(|token| token.name.as_str())
                                .collect::<Vec<_>>(),
                            ["allies"]
                        );
                        assert_eq!(game.select(&selector("allies")), [Target::Mob(112)]);
                    })
                    .unwrap();
                });
            });
            Err(())
        });
    }
}
