#![cfg(test)]

#[cfg(test)]
mod tests {
    use core::ops::ControlFlow;
    use mod_api::{DatapackReadError as E, *};
    use sunrise_gameplay_common::game::native_datapack_refs::NativeDatapackRefsGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

    #[test]
    #[ignore = "daily"]
    fn level_registry_reads_check_world_session_and_generation() {
        struct RegistryMod;
        #[mod_api::__subscriptions]
        impl Hooks for RegistryMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_datapack_registry(
                    "spell",
                    DatapackRegistrySync::Required,
                    vec![DatapackField::required(
                        "power",
                        DatapackFieldKind::Int { min: 0, max: 100 },
                    )],
                )
                .unwrap();
            }
        }
        mod_platform::testing::with_mods(
            vec![ModEntry::new("example", Box::new(RegistryMod))].into(),
            || {
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    game.install_live_features();
                    let level = game.level(game.dim.id()).unwrap();
                    let candidate = |power: i64| {
                        DatapackCandidate::build(
                            mod_api::__private::datapack_registry_declarations(),
                            &[(
                                "data/addon/example/spell/fire.json".into(),
                                vec![(
                                    "pack".to_owned(),
                                    format!(r#"{{"power":{power}}}"#).into_bytes(),
                                )],
                            )],
                            &mut mod_api::__private::NoVanillaValueDecoder,
                        )
                        .unwrap()
                    };
                    game.mods.datapack_registries = Some(std::sync::Arc::new(
                        DatapackRegistrySet::from_candidate(candidate(7)),
                    ));
                    assert_eq!(
                        game.level_visit_datapack_entries(level, "example:spell", &mut |id| {
                            assert_eq!(id, "addon:fire");
                            ControlFlow::Break(())
                        }),
                        Ok(ControlFlow::Break(()))
                    );
                    let mut visited_refusal = false;
                    let mut unexpected_visit = |_: &DatapackEntry| visited_refusal = true;
                    for invalid in [
                        LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension()),
                        LevelRef::new(level.epoch(), DimensionKey::NETHER),
                    ] {
                        assert_eq!(
                            game.level_visit_datapack_entries(
                                invalid,
                                "example:spell",
                                &mut |_| { panic!("unavailable level visited") }
                            ),
                            Err(E::UnknownRegistry)
                        );
                        assert_eq!(
                            game.level_datapack_read(
                                invalid,
                                "example:spell",
                                "addon:fire",
                                &mut unexpected_visit,
                            ),
                            Err(E::UnknownRegistry)
                        );
                    }
                    let mut visits = 0;
                    game.level_datapack_read(level, "example:spell", "addon:fire", &mut |entry| {
                        assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(7))]);
                        visits += 1;
                    })
                    .unwrap();
                    assert_eq!(visits, 1);
                    // Open and park the Nether; parked loaded server dimensions share registryAccess.
                    game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                        .unwrap();
                    game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                        .unwrap();
                    let parked = LevelRef::new(level.epoch(), DimensionKey::NETHER);
                    assert_eq!(
                        game.level_visit_datapack_entries(parked, "example:spell", &mut |id| {
                            assert_eq!(id, "addon:fire");
                            ControlFlow::Break(())
                        }),
                        Ok(ControlFlow::Break(()))
                    );
                    let mut parked_visits = 0;
                    game.level_datapack_read(parked, "example:spell", "addon:fire", &mut |entry| {
                        assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(7))]);
                        parked_visits += 1;
                    })
                    .unwrap();
                    assert_eq!(parked_visits, 1);
                    assert_eq!(
                        game.level_datapack_read(
                            level,
                            "example:missing",
                            "addon:fire",
                            &mut unexpected_visit,
                        ),
                        Err(E::UnknownRegistry)
                    );
                    assert_eq!(
                        game.level_datapack_read(
                            level,
                            "example:spell",
                            "addon:missing",
                            &mut unexpected_visit,
                        ),
                        Err(E::MissingEntry)
                    );
                    game.mods.datapack_registries = Some(std::sync::Arc::new(
                        DatapackRegistrySet::from_candidate(candidate(9)),
                    ));
                    let mut reloaded_visits = 0;
                    game.level_datapack_read(parked, "example:spell", "addon:fire", &mut |entry| {
                        assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(9))]);
                        reloaded_visits += 1;
                    })
                    .unwrap();
                    assert_eq!(reloaded_visits, 1);
                    assert!(!visited_refusal);
                });
            },
        );
    }
}
