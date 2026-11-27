#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use mod_api::{ActiveEffect, EntityReadError, EntityRef, Host, LivingRef, WorldEpoch};
    use std::ops::ControlFlow;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_living_body::NativeLivingBodyGameExt2;
    use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
    use sunrise_sim::mobs::Mob;

    pub(crate) fn collect_effects(
        host: &dyn Host,
        living: LivingRef,
    ) -> Result<Vec<ActiveEffect>, EntityReadError> {
        let mut effects = Vec::new();
        assert_eq!(
            living.visit_effects(host, &mut |effect| {
                effects.push(effect);
                ControlFlow::Continue(())
            })?,
            ControlFlow::Continue(())
        );
        Ok(effects)
    }

    pub(crate) fn effect(id: &'static str, duration: i32, amplifier: i32) -> ActiveEffect {
        ActiveEffect::from(mod_api::__private::built::ActiveEffect {
            id: mod_platform::__private::part_tests::effect_key(id),
            duration,
            amplifier,
            ambient: false,
            visible: true,
            show_icon: true,
        })
    }

    #[test]
    #[ignore = "daily"]
    fn active_effect_ids_are_load_owned_slots() {
        use sunrise_client::entities::ArmorStand;

        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(21, kind, Vec3::ZERO, 0.0));
            let zombie = game.living(game.entity(21).unwrap()).unwrap();
            // The same load-owned slot is used for application, enumeration and diagnostics.
            let speed = ActiveEffect::new(
                mod_platform::__private::part_tests::effect_key("minecraft:speed"),
                200,
                0,
                false,
                true,
                true,
            );
            let slowness = ActiveEffect::new(
                mod_platform::__private::part_tests::effect_key("minecraft:slowness"),
                90,
                2,
                true,
                false,
                true,
            );
            assert!(game.living_add_effect(zombie, &speed));
            assert!(game.living_add_effect(zombie, &slowness));
            let listed = collect_effects(game, zombie).unwrap();
            assert_eq!(listed, [speed.clone(), slowness]);
            assert_eq!(
                listed[0].id,
                mod_platform::__private::part_tests::effect_key("speed")
            );
            assert_eq!(listed[0].id.id(game), Some("minecraft:speed"));

            let mut first = Vec::new();
            assert_eq!(
                zombie.visit_effects(game, &mut |effect| {
                    first.push(effect);
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, [speed]);

            game.armor_stands.push(ArmorStand::new(22, Vec3::ZERO, 0.0));
            let stand = game.living(game.entity(22).unwrap()).unwrap();
            assert_eq!(
                stand.visit_effects(game, &mut |_| panic!("unavailable effects visited")),
                Err(EntityReadError::Unavailable)
            );
        });
    }

    /// `MobEffectInstance.showIcon` crosses `add_effect` and the reads both ways, for a player and
    /// a mob, apart from the particle flag; adding it again with the other value changes it.
    #[test]
    #[ignore = "daily"]
    fn an_effects_icon_flag_is_written_and_read_for_players_and_mobs() {
        use sunrise_sim::motion::PlayerMotion;
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(Mob::new(23, kind, Vec3::ZERO, 0.0));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 20;
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let key = mod_platform::__private::part_tests::effect_key("minecraft:speed");
            for id in [23, 20] {
                let living = game.living(game.entity(id).unwrap()).unwrap();
                for show_icon in [false, true] {
                    let given = ActiveEffect::new(key, 200, 0, false, true, show_icon);
                    assert!(game.living_add_effect(living, &given), "{id} {show_icon}");
                    let read = game.living_effect(living, key).unwrap();
                    assert_eq!((read.show_icon, read.visible), (show_icon, true), "{id}");
                    assert_eq!(collect_effects(game, living).unwrap(), vec![read]);
                }
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn resolved_keys_keep_the_mobs_hidden_effect_identity() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(Mob::new(22, kind, Vec3::ZERO, 0.0));
            let cow = game.living(game.entity(22).unwrap()).unwrap();
            let key = mod_platform::__private::part_tests::effect_key("speed");
            assert!(game.living_add_effect(cow, &effect("speed", 2, 1)));
            assert!(!game.living_add_effect(cow, &effect("speed", 5, 0)));
            let mob = &game.mobs[0];
            assert_eq!(mob.extra.effect_chains[0].id, "minecraft:speed");
            assert_eq!(mob.extra.effect_chains[0].registry_index, Some(0));
            assert_eq!(game.living_effect(cow, key).unwrap().id, key);
            assert_eq!(game.living_effect(cow, key).unwrap().amplifier, 1);
        });
    }

    /// A mob marked removed but still listed counts as absent, as the effect reads treat it: the
    /// three writes return false and change nothing.
    #[test]
    #[ignore = "daily"]
    fn effect_writes_on_a_removed_mob_change_nothing() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(Mob::new(24, kind, Vec3::ZERO, 0.0));
            let cow = game.living(game.entity(24).unwrap()).unwrap();
            let speed = mod_platform::__private::part_tests::effect_key("speed");
            assert!(game.living_add_effect(cow, &effect("speed", 200, 0)));
            game.mobs[0].removed = true;
            assert!(!game.living_add_effect(cow, &effect("slowness", 200, 0)));
            assert!(!game.living_remove_effect(cow, speed));
            assert!(!game.living_remove_all_effects(cow));
            let held: Vec<_> = game.mobs[0].extra.effects.iter().map(|e| e.id).collect();
            assert_eq!(held, ["minecraft:speed"]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn indexed_consumable_effects_apply_to_a_mob() {
        use sunrise_core::random::RandomSource;
        use sunrise_items::items::ItemStack;
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let cow = game.living(game.entity(12).unwrap()).unwrap();
            let item = |id| ItemStack::new(id, 1);
            let mut expected = game.mobs[0].random.clone();
            let mut level_random = game.world.random.clone();
            expected.next_float();
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:pufferfish"), 0),
                Some(true)
            );
            assert_eq!(game.mobs[0].random.next_float(), expected.next_float());
            assert_eq!(game.world.random.next_float(), level_random.next_float());
            assert!(game.living_potion_affected(
                cow,
                mod_platform::__private::part_tests::effect_key("minecraft:poison")
            ));
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:honey_bottle"), 0),
                Some(true)
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:honey_bottle"), 0),
                Some(false)
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:milk_bucket"), 0),
                Some(true)
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:milk_bucket"), 0),
                Some(false)
            );
            // This fixture has no server sound hub; a sound cannot be queued.
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:ominous_bottle"), 0),
                None
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:chorus_fruit"), 0),
                None
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:pufferfish"), 1),
                None
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:stone"), 0),
                None
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("missing:unknown"), 0),
                None
            );
            let stale = LivingRef::new(EntityRef::new(
                WorldEpoch::new(cow.entity().epoch().get() + 1),
                12,
            ));
            assert_eq!(
                game.living_apply_consume_effect(stale, &item("minecraft:pufferfish"), 0),
                None
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_mob_takes_effects_by_the_games_own_paths() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            for (id, kind) in [
                (12, "minecraft:zombie"),
                (13, "minecraft:spider"),
                (14, "minecraft:cow"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let spider = game.living(game.entity(13).unwrap()).unwrap();
            let cow = game.living(game.entity(14).unwrap()).unwrap();

            // Reusing the registered key reads the applied instance.
            assert!(game.living_add_effect(zombie, &effect("minecraft:speed", 200, 0)));
            assert!(game.living_potion_affected(
                zombie,
                mod_platform::__private::part_tests::effect_key("speed")
            ));
            assert_eq!(
                game.living_effect(
                    zombie,
                    mod_platform::__private::part_tests::effect_key("speed")
                ),
                Some(effect("minecraft:speed", 200, 0))
            );
            assert_eq!(
                collect_effects(game, zombie),
                Ok(vec![effect("minecraft:speed", 200, 0)])
            );

            // A re-add is `MobEffectInstance.update`: a weaker or shorter
            // one changes nothing, a stronger replaces with its own
            // duration, an equal one only extends.
            assert!(!game.living_add_effect(zombie, &effect("minecraft:speed", 100, 0)));
            assert_eq!(
                game.living_effect(
                    zombie,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed")
                ),
                Some(effect("minecraft:speed", 200, 0))
            );
            assert!(game.living_add_effect(zombie, &effect("minecraft:speed", 90, 1)));
            assert_eq!(
                game.living_effect(
                    zombie,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed")
                ),
                Some(effect("minecraft:speed", 90, 1))
            );

            // canBeAffected: a spider shrugs off poison and an undead one
            // with it, a cow takes it.
            assert!(!game.living_add_effect(spider, &effect("minecraft:poison", 100, 0)));
            assert_eq!(collect_effects(game, spider), Ok(Vec::new()));
            assert!(!game.living_add_effect(zombie, &effect("minecraft:poison", 100, 0)));
            assert!(game.living_add_effect(cow, &effect("minecraft:poison", 100, 0)));

            // A duration below -1 changes nothing.

            assert!(!game.living_add_effect(zombie, &effect("minecraft:speed", -2, 0)));

            // remove_effect takes its absorption hearts with it; a second
            // remove has nothing to do.
            assert!(game.living_add_effect(zombie, &effect("minecraft:absorption", 100, 1)));
            game.mobs[0].absorption = 8.0;
            assert!(game.living_remove_effect(
                zombie,
                mod_platform::__private::part_tests::effect_key("minecraft:absorption")
            ));
            assert_eq!(game.mobs[0].absorption, 0.0);
            assert!(!game.living_remove_effect(
                zombie,
                mod_platform::__private::part_tests::effect_key("minecraft:absorption")
            ));
            assert!(game.living_potion_affected(
                zombie,
                mod_platform::__private::part_tests::effect_key("minecraft:speed")
            ));

            // remove_all_effects takes the rest, and says when there was none.
            assert!(game.living_remove_all_effects(zombie));
            assert_eq!(collect_effects(game, zombie), Ok(Vec::new()));
            assert!(!game.living_potion_affected(
                zombie,
                mod_platform::__private::part_tests::effect_key("minecraft:speed")
            ));
            assert!(!game.living_remove_all_effects(zombie));

            // A stale handle reads nothing and writes nothing.
            let stale = EntityRef::new(WorldEpoch::new(zombie.entity().epoch().get() + 1), 12);
            let stale = LivingRef::new(stale);
            assert!(!game.living_add_effect(stale, &effect("minecraft:speed", 100, 0)));
            assert_eq!(
                game.living_effect(
                    stale,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed")
                ),
                None
            );
            assert_eq!(
                stale.visit_effects(game, &mut |_| panic!("stale effects visited")),
                Err(EntityReadError::Stale)
            );
            assert!(!game.living_potion_affected(
                stale,
                mod_platform::__private::part_tests::effect_key("minecraft:speed")
            ));
            assert!(!game.living_remove_effect(
                stale,
                mod_platform::__private::part_tests::effect_key("minecraft:speed")
            ));
            assert!(!game.living_remove_all_effects(stale));
        });
    }
}
