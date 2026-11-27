#![cfg(test)]
#[cfg(test)]
#[path = "tests/infested_roll_tests.rs"]
pub(crate) mod infested_roll_tests;

#[cfg(test)]
pub(crate) mod pet_credit_tests {
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::lan::playerdata::uuid_ints;
    use sunrise_sim::mobs::Mob;

    /// A tame wolf's hit on the player in place is its owner's
    /// (`resolvePlayerResponsibleForDamage` asks `instanceof Wolf`), so a
    /// content mod's type copying the wolf (#4031) names its owner too, and
    /// an untamed one names no one.
    #[test]
    #[ignore = "daily"]
    fn a_tame_mod_wolf_types_hit_is_its_owners() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let wolf = game.res.mob_types().get("minecraft:wolf").cloned().unwrap();
            let mut hound = (*wolf).clone();
            hound.id = "ruby:hound".into();
            for (kind, tame, credit) in [
                (wolf.clone(), true, Some(12)),
                (Arc::new(hound.clone()), true, Some(12)),
                (Arc::new(hound), false, None),
            ] {
                game.server.combat.0.last_hurt_by_player = None;
                game.mobs.clear();
                let mut pet = Mob::new(7, kind, Vec3::ZERO, 0.0);
                pet.extra.tame = tame;
                pet.extra.owner = Some(uuid_ints(12));
                game.mobs.push(pet);
                game.resolve_player_responsible_for_player(7, None);
                assert_eq!(
                    game.server.combat.0.last_hurt_by_player.map(|(c, _)| c.0),
                    credit
                );
            }
        });
    }
}

#[cfg(test)]
pub(crate) mod guest_hit_tests {
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::mobs::{Damage, DamageKind, Mob, MobEvent, Species, Who};
    use sunrise_sim::motion::PlayerMotion;

    /// A bystander stands exactly under the mob that bites, listed first: the
    /// search by position ignores height and would have named the bystander.
    #[test]
    #[ignore = "daily"]
    fn an_effect_hit_on_a_guest_credits_the_mob_that_swung() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap());
            for (id, kind, y) in [
                (4, "minecraft:zombie", 90.0),
                (5, "minecraft:cave_spider", 100.0),
            ] {
                let kind = game.res.mob_types().get(kind).cloned().unwrap();
                game.mobs
                    .push(Mob::new(id, kind, Vec3::new(1.5, y, 0.5), 0.0));
            }
            let from = [1.5, 100.0, 0.5];
            let hits = [
                MobEvent::AttackPlayerWithEffect {
                    attacker: 5,
                    amount: 2.0,
                    from,
                    effect: "minecraft:poison",
                    ticks: 140,
                    amplifier: 0,
                },
                MobEvent::Sting {
                    target: Who::Player,
                    amount: 2.0,
                    poison_ticks: 200,
                    source: Damage {
                        kind: DamageKind::Sting,
                        attacker: Some(Who::Mob(5)),
                        from: Some(from),
                        direct: None,
                        attacker_species: Some(Species::Bee),
                    },
                },
            ];
            for hit in hits {
                game.server.combat.0.pending.clear();
                game.guest_mob_event(5, hit);
                let noted: Vec<u32> = (game.server.combat.0.pending.iter())
                    .map(|p| p.attacker.0)
                    .collect();
                assert_eq!(noted, [5]);
                assert_eq!(game.server.combat.0.melee_mob, None, "the hit used it up");
            }
        });
    }
}

#[cfg(test)]
#[path = "combat/zombie_ignite_tests.rs"]
pub(crate) mod zombie_ignite_tests;
