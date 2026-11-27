#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use super::super::native_entity_refs::tests::with_game;
    use crate::game::server_side::damage_item_tests::{Rule, with_wear_part};
    use sunrise_command::args::GameMode;
    use sunrise_core::{
        geometry::Vec3,
        random::{LegacyRandom, RandomSource},
    };
    use sunrise_gameplay::game::native_entity_refs::tests::{
        bind_local_player, fixture_resident_column, run_fresh_cases, run_with_game_stack,
    };
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::{
        mobs::{Mob, Who, finalize},
        motion::PlayerMotion,
        projectiles::{Hit, Projectile, ProjectileKind},
    };

    #[test]
    #[ignore = "daily"]
    fn mob_projectile_hits_on_players_run_thorns() {
        if run_fresh_cases(
            "cut_layout::game::server_side::enchant_effects::tests::mob_projectile_hits_on_players_run_thorns",
            "SUNRISE_POST_ATTACK_PROJECTILE_CHILD",
            &["hits"],
        ) {
            return;
        }
        run_with_game_stack(|| {
            with_game(|game| {
                let victim = Vec3::new(3.5, 64.0, 5.5);
                let shooter = Vec3::new(40.5, 64.0, 5.5);
                for column in [(0, 0), (2, 0)] {
                    fixture_resident_column(&mut game.world, column);
                }
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                for kind in [
                    ProjectileKind::SmallFireball,
                    ProjectileKind::LargeFireball { power: 0 },
                    ProjectileKind::LlamaSpit,
                    ProjectileKind::ShulkerBullet {
                        target: None,
                        dir: None,
                        steps: 0,
                        delta: [0.0; 3],
                        seed: 1,
                    },
                    ProjectileKind::WindCharge,
                    ProjectileKind::BreezeWindCharge,
                    ProjectileKind::WitherSkull { dangerous: false },
                ] {
                    game.player = Some(PlayerMotion::new(game.motion, victim).unwrap());
                    bind_local_player(game, 77, 77);
                    game.set_mode(GameMode::Survival);
                    let mut chest = ItemStack::new("minecraft:diamond_chestplate", 1);
                    chest.set_crafting_enchantments(vec![("minecraft:thorns".into(), 255)]);
                    game.armor[1] = Some(chest);
                    let mob = Mob::new(41, cow.clone(), shooter, 0.0);
                    let before = mob.health;
                    game.mobs.push(mob);
                    let mut shot = Projectile::new(kind, victim, Some(Who::Mob(41)));
                    shot.id = 90;
                    shot.previous = victim.add_vector(Vec3::new(1.0, 1.0, 0.0));
                    game.projectile_hit(
                        &shot,
                        Hit::Entity {
                            who: Who::Player,
                            at: victim,
                        },
                    )
                    .unwrap();
                    // The shooter is out of blast range, so only the victim's enchantment
                    // can have hurt it. Thorns' level makes the trigger deterministic.
                    let mob = game.mobs.pop().unwrap();
                    assert!(
                        mob.health < before,
                        "{:?} did not trigger Thorns",
                        shot.kind
                    );
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn enchantment_sound_skips_silent_entities_and_uses_their_category() {
        if run_fresh_cases(
            "cut_layout::game::server_side::enchant_effects::tests::enchantment_sound_skips_silent_entities_and_uses_their_category",
            "SUNRISE_ENCHANTMENT_SOUND_CHILD",
            &["sound"],
        ) {
            return;
        }
        run_with_game_stack(|| {
            with_game(|game| {
                fixture_resident_column(&mut game.world, (0, 0));
                let at = Vec3::new(3.5, 64.0, 5.5);
                let zombie = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(41, zombie, at, 0.0);
                mob.random.set_seed(42);
                mob.silent = true;
                game.mobs.push(mob);
                game.start_audio_capture();
                let effect = serde_json::json!({
                    "sound": "minecraft:entity.zombie.ambient", "volume": 0.75,
                    "pitch": {"type": "minecraft:uniform", "min_inclusive": 0.5, "max_exclusive": 1.5}
                });
                game.play_enchantment_sound(&effect, 1, 41, at.components());
                assert!(game.server.pending_sounds.is_empty());
                let mut expected = LegacyRandom::new(42);
                assert_eq!(game.mobs[0].random.next_int(), expected.next_int());

                game.mobs[0].silent = false;
                game.play_enchantment_sound(&effect, 1, 41, at.components());
                let [heard] = game.server.pending_sounds.as_slice() else {
                    panic!("expected one sound from the audible actor");
                };
                assert_eq!(heard.source, mod_api::SoundSource::Hostile.index());
                assert_eq!(heard.pitch, expected.next_float() + 0.5);
                assert_eq!(game.mobs[0].random.next_int(), expected.next_int());
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn apply_mob_effect_rounds_negative_ties_and_has_no_source() {
        use std::sync::{Arc, Mutex};
        if run_fresh_cases(
            "cut_layout::game::server_side::enchant_effects::tests::apply_mob_effect_rounds_negative_ties_and_has_no_source",
            "SUNRISE_POST_ATTACK_MOB_EFFECT_CHILD",
            &["apply"],
        ) {
            return;
        }
        run_with_game_stack(|| {
            struct Observer(Arc<Mutex<Vec<Option<u32>>>>);
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for Observer {
                fn mob_effect_added(
                    &self,
                    _: &mut dyn mod_api::ServerHost,
                    event: &mut mod_api::MobEffectAddedEvent<'_>,
                ) {
                    self.0
                        .lock()
                        .unwrap()
                        .push(event.source.map(|source| source.id()));
                }
            }
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mods: mod_api::Mods = Arc::from([mod_api::ModEntry::new(
                "post_attack_effect_observer",
                Box::new(Observer(seen.clone())),
            )]);
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                    bind_local_player(game, 77, 77);
                    game.apply_post_attack(
                        &serde_json::json!({
                            "type": "minecraft:apply_mob_effect", "to_apply": "minecraft:speed",
                            "min_duration": -0.025, "max_duration": -0.025,
                            "min_amplifier": 1.5, "max_amplifier": 1.5
                        }),
                        1,
                        77,
                        Some(77),
                        Some(mod_api::EquipmentSlot::MainHand),
                    );
                    let effect = game.potions.effects.get("speed").unwrap();
                    // Java Math.round(-0.5F) is 0; -1 would give an infinite effect instead.
                    assert_eq!((effect.duration, effect.amplifier), (0, 2));
                    assert_eq!(*seen.lock().unwrap(), [None]);
                });
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_armour_obeys_negative_cancel_and_breaking_answers() {
        with_wear_part("minecraft:iron_helmet", Rule::Set(-1), |game, wear| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut mob = Mob::new(901, kind, Vec3::ZERO, 0.0);
            finalize::set_item_stack(&mut mob, 5, ItemStack::new("minecraft:iron_helmet", 1));
            game.mobs.push(mob);
            for rule in [Rule::Set(i32::MIN), Rule::Cancel] {
                wear.set(rule);
                game.wear_mob_piece(901, 5, 1);
                assert_eq!(
                    finalize::item_stack(game.mobs.last().unwrap(), 5)
                        .unwrap()
                        .damage,
                    0
                );
                assert_eq!(wear.take_asked(), [1]);
            }
            wear.set(Rule::Set(164));
            game.wear_mob_piece(901, 5, 1);
            assert_eq!(
                finalize::item_stack(game.mobs.last().unwrap(), 5)
                    .unwrap()
                    .damage,
                164
            );
            wear.set(Rule::Set(i32::MAX));
            game.wear_mob_piece(901, 5, 1);
            assert!(finalize::item_stack(game.mobs.last().unwrap(), 5).is_none());
            assert_eq!(wear.take_asked(), [1, 1]);
            game.wear_mob_piece(901, 5, 1);
            assert!(wear.take_asked().is_empty());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn thorns_and_boots_wear_break_the_equipped_piece() {
        use sunrise_core::geometry::Vec3;
        use sunrise_items::items::ItemStack;
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let mut chest = ItemStack::new("minecraft:diamond_chestplate", 1);
            chest.damage = game.items().get(&chest.item).unwrap().max_damage - 1;
            chest.data_mut().enchantments = vec![("minecraft:thorns".into(), 255)];
            game.armor[1] = Some(chest);
            game.player.as_mut().unwrap().entity_id = 1;
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(2, kind, Vec3::ZERO, 0.0));
            game.thorns(2);
            assert!(game.armor[1].is_none());
            let mut boots = ItemStack::new("minecraft:diamond_boots", 1);
            boots.damage = game.items().get(&boots.item).unwrap().max_damage - 1;
            game.armor[3] = Some(boots);
            game.wear_boots(1);
            assert!(game.armor[3].is_none());
        });
    }
}
