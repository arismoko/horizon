#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_sim::mobs::Species;
use sunrise_sim::mobs::{finalize, spawning};
#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::random::{LegacyRandom, RandomSource};
    use sunrise_sim::mobs::spawn_rules::Reason;

    /// A zombie for the spawn tests, at a spot with room around it.
    fn zombie(game: &sunrise_gameplay::game::ServerGame<'_>) -> sunrise_sim::mobs::Mob {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        sunrise_sim::mobs::Mob::new(1, kind, Vec3::new(8.0, 65.0, 8.0), 0.0)
    }

    /// Any other mob for the spawn tests, at the same spot.
    fn mob_of(
        game: &sunrise_gameplay::game::ServerGame<'_>,
        id: u32,
        kind: &str,
    ) -> sunrise_sim::mobs::Mob {
        let kind = game.res.mob_types().get(kind).unwrap().clone();
        sunrise_sim::mobs::Mob::new(id, kind, Vec3::new(8.0, 65.0, 8.0), 0.0)
    }

    /// `Mob.finalizeSpawn`'s own two draws (`Mob.java:1078-1085`), then
    /// `Zombie.finalizeSpawn`'s, replayed on a fresh random of the same seed:
    /// the follow-range triangle, the left-handed roll, `pick_up_loot`, the
    /// baby roll and the break-doors roll, in vanilla's order.
    struct VanillaZombieDraws {
        follow_range: f64,
        left_handed: bool,
        pick_up_loot: bool,
        baby: bool,
        can_break_doors: bool,
    }
    fn vanilla_draws(random: &mut LegacyRandom, special: f32) -> VanillaZombieDraws {
        VanillaZombieDraws {
            follow_range: random.triangle(0.0, 0.114_850_000_000_000_01),
            left_handed: random.next_float() < 0.05,
            pick_up_loot: random.next_float() < 0.55 * special,
            baby: random.next_float() < 0.05,
            can_break_doors: random.next_float() < special * 0.1,
        }
    }

    /// The follow-range spawn bonus `finalizeSpawn` added, if any.
    fn random_spawn_bonus(mob: &sunrise_sim::mobs::Mob) -> Option<f64> {
        mob.extra
            .attributes
            .as_ref()?
            .modifiers
            .iter()
            .find(|m| m.id == "minecraft:random_spawn_bonus")
            .map(|m| m.amount)
    }

    /// finalizeSpawn draws from the level's random (`Ctx.random` is
    /// `world.random`), in vanilla's order: a shadow random of the same seed
    /// replaying `Mob.java`'s and `Zombie.finalizeSpawn`'s draws predicts
    /// every observable outcome, and the level random advances by exactly
    /// the draws the finalization made.
    #[test]
    #[ignore = "daily"]
    fn finalize_plain_draws_the_level_random_in_vanilla_order() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let mut mob = zombie(game);
            let seed = 0xC0FFEE_i64;
            game.world.random =
                sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(seed));
            let difficulty = game.local_difficulty([8.0, 65.0, 8.0]);
            game.finalize_plain(&mut mob, &difficulty, Reason::Natural);

            let mut shadow = LegacyRandom::new(seed);
            let draws = vanilla_draws(&mut shadow, difficulty.special());
            assert_eq!(mob.extra.left_handed, draws.left_handed);
            assert_eq!(mob.extra.pick_up_loot, draws.pick_up_loot);
            assert_eq!(mob.extra.can_break_doors, draws.can_break_doors);
            let bonus = random_spawn_bonus(&mob).expect("random_spawn_bonus added");
            assert!((bonus - draws.follow_range).abs() < 1e-12);
            // The baby roll picked its group state; a baby on this seed would
            // have ridden the same stream, so its flag matches the shadow too.
            assert_eq!(mob.extra.age < 0, draws.baby);
            // finalizeSpawn's later rolls (populate, enchant, the Halloween
            // head) advance the same stream past the shadow's replay.
            let before = shadow.next_int_bound(1000);
            assert_ne!(game.world.random.next_int_bound(1000), before);
        });
    }

    /// With no mod listening, the event half of the folded finalizeSpawn
    /// changes nothing: `finalize_with` finalizes exactly as `finalize_plain`
    /// does, on the same level stream.
    #[test]
    #[ignore = "daily"]
    fn finalize_with_without_listeners_finalizes_like_finalize_plain() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let mut with = zombie(game);
            let mut plain = zombie(game);
            let difficulty = game.local_difficulty([8.0, 65.0, 8.0]);
            game.world.random =
                sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(42_i64));
            let outcome = game.finalize_with(&mut with, Reason::Natural, None);
            assert!(outcome);
            // The same stream for the plain path: same seed, same draws.
            game.world.random =
                sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(42_i64));
            game.finalize_plain(&mut plain, &difficulty, Reason::Natural);

            assert_eq!(with.extra.left_handed, plain.extra.left_handed);
            assert_eq!(with.extra.pick_up_loot, plain.extra.pick_up_loot);
            assert_eq!(with.extra.can_break_doors, plain.extra.can_break_doors);
            assert_eq!(with.extra.age < 0, plain.extra.age < 0);
            let a = random_spawn_bonus(&with);
            let b = random_spawn_bonus(&plain);
            assert_eq!(a.is_some(), b.is_some());
            if let (Some(a), Some(b)) = (a, b) {
                assert!((a - b).abs() < 1e-12);
            }
        });
    }

    /// #2783: `/summon`, eggs, spawners and natural spawns run
    /// `WitherSkeleton.finalizeSpawn`: attack damage base 4.0, not the
    /// default 2.0.
    #[test]
    #[ignore = "daily"]
    fn runtime_wither_skeletons_hit_for_four() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let mut w = mob_of(game, 1, "minecraft:wither_skeleton");
            let difficulty = game.local_difficulty([8.0, 65.0, 8.0]);
            game.finalize_plain(&mut w, &difficulty, Reason::MobSummoned);
            assert_eq!(finalize::value(&w, finalize::Attr::AttackDamage), 4.0);
            assert_eq!(w.attrs.attack_damage, 4.0);
        });
    }

    /// #2783: runtime spawns run `Mob.finalizeSpawn`: the follow range
    /// bonus and 5 percent left-handed mobs, drawn from the level random.
    #[test]
    #[ignore = "daily"]
    fn runtime_spawns_run_the_mob_finalize_spawn() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let difficulty = game.local_difficulty([8.0, 65.0, 8.0]);
            let mut left = 0;
            for id in 1..=400u32 {
                let mut m = mob_of(game, id, "minecraft:skeleton");
                game.finalize_plain(&mut m, &difficulty, Reason::MobSummoned);
                assert!(finalize::has_modifier(
                    &m,
                    finalize::Attr::FollowRange,
                    "minecraft:random_spawn_bonus"
                ));
                left += usize::from(m.extra.left_handed);
            }
            // 5 percent of 400 is 20.
            assert!((5..=40).contains(&left), "{left} of 400 left-handed");
            // A bred baby never ran it.
            let mut cow = mob_of(game, 1, "minecraft:cow");
            game.finalize_plain(&mut cow, &difficulty, Reason::MobSummoned);
            cow.extra.left_handed = true;
            spawning::bred_offspring(&mut cow);
            assert!(!finalize::has_modifier(
                &cow,
                finalize::Attr::FollowRange,
                "minecraft:random_spawn_bonus"
            ));
            assert!(!cow.extra.left_handed);
            assert_eq!(
                cow.attrs.follow_range,
                finalize::value(&cow, finalize::Attr::FollowRange)
            );
            // CaveSpider.finalizeSpawn does not call super.
            let mut c = mob_of(game, 1, "minecraft:cave_spider");
            game.finalize_plain(&mut c, &difficulty, Reason::MobSummoned);
            assert!(!finalize::has_modifier(
                &c,
                finalize::Attr::FollowRange,
                "minecraft:random_spawn_bonus"
            ));
        });
    }

    /// `Zombie` / `AbstractSkeleton` halloween garb: 25% of empty heads, never
    /// on other mobs, and not when it is not Halloween.
    #[test]
    #[ignore = "daily"]
    fn halloween_pumpkins_only_undead_on_halloween() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let mut pumpkins = 0;
            let mut lanterns = 0;
            for id in 1..=400u32 {
                let mut m = mob_of(game, id, "minecraft:skeleton");
                m.extra.set_slot_id(sunrise_items::equipment::HEAD, None);
                let mut random = m.random.clone();
                finalize::halloween_head(&mut m, &mut random, true);
                match m.extra.head_id() {
                    Some("minecraft:jack_o_lantern") => {
                        pumpkins += 1;
                        lanterns += 1;
                        assert_eq!(finalize::stored_drop_chance(&m, 5), 0.0);
                    }
                    Some("minecraft:carved_pumpkin") => {
                        pumpkins += 1;
                        assert_eq!(finalize::stored_drop_chance(&m, 5), 0.0);
                    }
                    Some(other) => panic!("unexpected head {other}"),
                    None => {}
                }
            }
            assert!(
                (70..=130).contains(&pumpkins),
                "{pumpkins} of 400 skeletons"
            );
            assert!(lanterns < pumpkins / 2, "{lanterns} lanterns of {pumpkins}");
            let mut m = mob_of(game, 1, "minecraft:skeleton");
            m.extra.set_slot_id(sunrise_items::equipment::HEAD, None);
            let mut random = m.random.clone();
            finalize::halloween_head(&mut m, &mut random, false);
            assert!(m.extra.head_id().is_none());
            let mut cow = mob_of(game, 1, "minecraft:cow");
            cow.extra.set_slot_id(sunrise_items::equipment::HEAD, None);
            let mut random = cow.random.clone();
            finalize::halloween_head(&mut cow, &mut random, true);
            assert!(cow.extra.head_id().is_none());
        });
    }

    /// The strider's zombified-piglin rider needs `isSpawningMonsters`
    /// (`ServerLevel.isSpawningMonsters`: the difficulty and the `spawn_mobs`
    /// and `spawn_monsters` gamerules), not the difficulty alone.
    #[test]
    #[ignore = "daily"]
    fn the_striders_piglin_rider_needs_spawning_monsters() {
        use sunrise_game_data::gamerules::Value as RuleValue;
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            // The gate reads the difficulty setting (`ui.difficulty`), which a
            // fresh test game leaves peaceful; the rules default on.
            game.ui.difficulty = 2;
            let difficulty = game.local_difficulty([8.0, 65.0, 8.0]);
            let piglins = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.mobs
                    .iter()
                    .filter(|m| m.species() == Species::ZombifiedPiglin)
                    .count()
            };
            for id in 1..=400u32 {
                let mut s = mob_of(game, id, "minecraft:strider");
                game.finalize_plain(&mut s, &difficulty, Reason::Natural);
            }
            assert!(piglins(game) > 0, "riders with the rules on");
            // `spawn_monsters` off: no rider, though the difficulty allows it.
            assert!(game.ui.rules.set("spawn_monsters", RuleValue::Bool(false)));
            let before = piglins(game);
            for id in 1000..=1400u32 {
                let mut s = mob_of(game, id, "minecraft:strider");
                game.finalize_plain(&mut s, &difficulty, Reason::Natural);
            }
            assert_eq!(piglins(game), before);
        });
    }
}
