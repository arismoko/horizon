#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_sim::join_seq::JoinList;
use sunrise_sim::mobs::Who;
use sunrise_sim::projectiles::Projectile;
#[cfg(test)]
#[path = "native_projectiles/impact_probe.rs"]
pub(crate) mod impact_probe;

#[cfg(test)]
#[path = "native_projectiles/impact_panic_tests.rs"]
mod impact_panic_tests;

#[cfg(test)]
mod nested_spawn_tests {
    use super::*;
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;

    const CASE: &str = "SUNRISE_PROJECTILE_NESTED_SPAWN_CASE";

    fn impact_handoff_balances_the_pass(park: bool) {
        use sunrise_sim::join_seq::residency::{Change, Journal, Kind, Owner};
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            if park {
                game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                    .unwrap();
            }
            let journal = Journal::default();
            let owner = Owner {
                dimension: game.dim.key(),
                kind: Kind::Projectile,
            };
            game.creatures
                .projectiles
                .bind_residency(&journal, owner, |p| (p.id, p.join));
            let mut shot = Projectile::new(
                sunrise_sim::projectiles::ProjectileKind::Egg,
                Vec3::new(3.5, 80.0, 3.5),
                None,
            );
            shot.id = 71;
            game.creatures.projectiles.push(shot);
            let join = game.creatures.projectiles[0].join;
            let mut pass = game.creatures.projectiles.lift();
            let mut child = pass[0].clone();
            child.id = 72;
            child.join = sunrise_sim::join_seq::JoinSeq::UNJOINED;
            game.creatures.projectiles.push(child);
            let child_join = game.creatures.projectiles[0].join;
            game.native_settle_impacts(
                &mut pass,
                Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
            );
            assert_eq!(pass.iter().map(|p| p.id).collect::<Vec<_>>(), [71, 72]);
            game.creatures
                .projectiles
                .restore(pass, sunrise_sim::join_seq::RestoreOrder::SpawnsFirst);
            if park {
                game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                    .unwrap();
            }
            let mut events = Vec::new();
            journal.drain_into(&mut events);
            assert!(events.iter().all(|event| event.owner == owner));
            let balance: i32 = events
                .iter()
                .map(|event| match event.change {
                    Change::Lifted => 1,
                    Change::Restored => -1,
                    _ => 0,
                })
                .sum();
            assert_eq!(balance, 0, "impact exposure left an unmatched lifted pass");
            if park {
                game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                    .unwrap();
            }
            assert_eq!(
                game.creatures
                    .projectiles
                    .iter()
                    .map(|p| (p.id, p.join))
                    .collect::<Vec<_>>(),
                [(71, join), (72, child_join)]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn impact_handoff_balances_the_active_owner() {
        if !run_fresh_cases(
            "cut_layout::game::server_side::native_projectiles::nested_spawn_tests::impact_handoff_balances_the_active_owner",
            CASE,
            &["active-owner"],
        ) {
            impact_handoff_balances_the_pass(false);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn impact_handoff_balances_the_parked_owner() {
        if !run_fresh_cases(
            "cut_layout::game::server_side::native_projectiles::nested_spawn_tests::impact_handoff_balances_the_parked_owner",
            CASE,
            &["parked-owner"],
        ) {
            impact_handoff_balances_the_pass(true);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn impact_exposure_retains_the_existing_spawn_buffer_for_the_next_pass() {
        if run_fresh_cases(
            "cut_layout::game::server_side::native_projectiles::nested_spawn_tests::impact_exposure_retains_the_existing_spawn_buffer_for_the_next_pass",
            CASE,
            &["spawn-buffer"],
        ) {
            return;
        }
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            fn shot(id: u32) -> Projectile {
                let mut p = Projectile::new(
                    sunrise_sim::projectiles::ProjectileKind::Egg,
                    sunrise_core::geometry::Vec3::new(3.5, 80.0, 3.5),
                    None,
                );
                p.id = id;
                p
            }
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let mut taken = JoinList::default();
            for id in [71, 73, 74] {
                taken.push(shot(id));
            }
            let _ = taken.remove(1);
            let mut expected: Vec<_> = taken.iter().map(|p| (p.id, p.join)).collect();
            game.creatures.projectiles.push(shot(72));
            let join = game.creatures.projectiles[0].join;
            expected.push((72, join));
            game.native_settle_impacts(
                &mut taken,
                Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
            );
            assert_eq!(
                taken.iter().map(|p| (p.id, p.join)).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(taken[2].age, 0);
            assert_eq!(taken[2].join, join);
            game.creatures
                .projectiles
                .restore(taken, sunrise_sim::join_seq::RestoreOrder::SpawnsFirst);
            game.tick_projectiles().unwrap();
            let spawned = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == 72)
                .unwrap();
            assert_eq!(spawned.age, 1);
            assert_eq!(spawned.join, join);
        });
    }
}

#[cfg(test)]
mod pearl_hit_tests {
    use super::*;
    use sunrise_gameplay_common::game::dragon_common::SPECIAL;
    use sunrise_sim::projectiles::Hit;

    /// `ender_pearl_teleport`'s hit fields: a block and its face normal, or an
    /// entity with the cell of its hit point and no face.
    #[test]
    #[ignore = "daily"]
    fn a_pearl_tells_the_block_face_or_the_entity_it_hit() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let wall = Hit::Block {
                pos: [4, 70, -2],
                at: Vec3::new(4.0, 70.5, -1.5),
                face: [-1, 0, 0],
            };
            assert_eq!(
                game.pearl_hit(wall),
                (None, [4, 70, -2], [-1, 0, 0], [4.0, 70.5, -1.5])
            );
            let cow = Hit::Entity {
                who: Who::Mob(42),
                at: Vec3::new(1.25, 64.5, -0.5),
            };
            assert_eq!(
                game.pearl_hit(cow),
                (Some(42), [1, 64, -1], [0; 3], [1.25, 64.5, -0.5])
            );
        });
    }

    /// A dragon part is the dragon and the local player is its own entity.
    #[test]
    #[ignore = "daily"]
    fn a_pearl_names_the_dragon_for_a_part_and_the_local_player_by_its_own_id() {
        use sunrise_gameplay_common::game::dragon_common::Dragon;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 225009;
            game.dragon.replace(dragon);
            let at = Vec3::new(1.5, 64.5, -0.5);
            let part = Hit::Entity {
                who: Who::Mob(SPECIAL + 2),
                at,
            };
            assert_eq!(
                game.pearl_hit(part),
                (Some(225009), [1, 64, -1], [0; 3], [1.5, 64.5, -0.5])
            );
            let player = Hit::Entity {
                who: Who::Player,
                at,
            };
            // The local player is the entity the game gives it, here as the game reports it.
            let own = game.own_entity_id() as u32;
            assert_eq!(game.pearl_hit(player).0, Some(own));
        });
    }

    /// A listener hears the hit through `native_pearl_teleport` and its write comes back.
    #[test]
    #[ignore = "daily"]
    fn a_listening_mod_hears_the_pearls_hit_and_its_target_comes_back() {
        use mod_api::{EnderPearlTeleportEvent, Flow, Hooks, ModEntry, ServerHost};
        use std::sync::{Arc, Mutex};
        use sunrise_gameplay_common::game::dragon_common::Dragon;
        type Heard = Arc<Mutex<Vec<(Option<u32>, [i32; 3])>>>;
        struct Hear(Heard);
        #[mod_api::__subscriptions]
        impl Hooks for Hear {
            fn ender_pearl_teleport(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EnderPearlTeleportEvent<'_>,
            ) -> Flow {
                self.0
                    .lock()
                    .unwrap()
                    .push((event.hit_entity, event.hit_block));
                *event.target = [1.0, 2.0, 3.0];
                Flow::Continue
            }
        }
        let heard = Heard::default();
        let mods: mod_api::Mods = vec![ModEntry::new("hear", Box::new(Hear(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
                dragon.id = 225009;
                game.dragon.replace(dragon);
                let owner = mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 7);
                let hit = Hit::Entity {
                    who: Who::Mob(SPECIAL + 2),
                    at: Vec3::new(1.5, 64.5, -0.5),
                };
                let (target, damage, cancel) = game.native_pearl_teleport(
                    owner,
                    9,
                    Vec3::ZERO,
                    Vec3::new(8.0, 70.0, 8.0),
                    0.0,
                    hit,
                );
                assert_eq!((target, damage, cancel), ([1.0, 2.0, 3.0], 0.0, false));
                assert_eq!(*heard.lock().unwrap(), [(Some(225009), [1, 64, -1])]);
            });
        });
    }
}
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
