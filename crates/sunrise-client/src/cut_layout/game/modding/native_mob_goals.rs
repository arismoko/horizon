#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::{with_game, with_resident_game};
    use mod_api::{EntityRef, GoalSpec, MobFilter, MobRef, WorldEpoch};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6;
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::mobs::Mob;

    /// A mob of `kind` in the game's list, its species goals registered by
    /// `Mob::new`, as a handle.
    fn mob_in_game(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        kind: &str,
        id: u32,
    ) -> MobRef {
        let t = game.res.mob_types().get(kind).unwrap().clone();
        let mut m = Mob::new(id, t, Vec3::new(3.5, 65.0, 3.5), 0.0);
        m.ground.on_ground = true;
        game.mobs.push(m);
        let entity = game.entity(id).unwrap();
        game.mob(entity).unwrap()
    }

    /// Whether the mob's goal selector holds a goal of kind `id`.
    fn has(game: &sunrise_gameplay::game::ServerGame<'_>, mob: MobRef, id: &str) -> bool {
        mob.goals(game).unwrap().iter().any(|e| e.id == id)
    }

    #[test]
    #[ignore = "daily"]
    fn goal_constructors_follow_vanilla_classes() {
        use sunrise_sim::mobs::Species;
        with_game(|game| {
            let mut mismatches = Vec::new();
            for (species, accepts) in [
                (Species::Slime, true),
                (Species::MagmaCube, true),
                (Species::Shulker, true),
                (Species::Ghast, false),
                (Species::Phantom, false),
                (Species::Bat, false),
            ] {
                let actual = game
                    .boxed_goal(
                        species,
                        sunrise_sim::mobs::nav::NavKind::Ground,
                        &GoalSpec::Panic {
                            speed: 1.0,
                            water_range: None,
                        },
                    )
                    .is_some();
                if actual != accepts {
                    mismatches.push(format!(
                        "{species:?}: accepted {actual}, expected {accepts}"
                    ));
                }
            }
            assert!(mismatches.is_empty(), "{}", mismatches.join("; "));
            for species in [Species::Bat, Species::Ghast, Species::Phantom] {
                assert!(
                    game.boxed_goal(
                        species,
                        sunrise_sim::mobs::nav::NavKind::Ground,
                        &GoalSpec::EatBlock
                    )
                    .is_some()
                );
                assert!(
                    game.boxed_goal(
                        species,
                        sunrise_sim::mobs::nav::NavKind::Ground,
                        &GoalSpec::RestrictSun
                    )
                    .is_none()
                );
                assert!(
                    game.boxed_goal(
                        species,
                        sunrise_sim::mobs::nav::NavKind::Ground,
                        &GoalSpec::HurtByTarget { alert: false }
                    )
                    .is_none()
                );
                assert!(
                    game.boxed_goal(
                        species,
                        sunrise_sim::mobs::nav::NavKind::Ground,
                        &GoalSpec::FollowMob {
                            speed: 1.0,
                            stop: 2.0,
                            area: 8.0
                        }
                    )
                    .is_some()
                );
            }
            for species in [
                Species::Cod,
                Species::Dolphin,
                Species::Guardian,
                Species::Frog,
                Species::Drowned,
                Species::Turtle,
                Species::Axolotl,
            ] {
                assert!(
                    game.boxed_goal(
                        species,
                        sunrise_sim::mobs::nav::NavKind::Water { breach: false },
                        &GoalSpec::FollowMob {
                            speed: 1.0,
                            stop: 2.0,
                            area: 8.0
                        }
                    )
                    .is_none()
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn follow_mob_uses_the_mobs_actual_navigation() {
        with_resident_game(|game| {
            let mob = mob_in_game(game, "minecraft:cow", 501);
            let goal = GoalSpec::FollowMob {
                speed: 1.0,
                stop: 2.0,
                area: 8.0,
            };
            for nav in [
                sunrise_sim::mobs::nav::NavKind::Water { breach: false },
                sunrise_sim::mobs::nav::NavKind::Amphibious {
                    prefers_shallow: false,
                },
            ] {
                game.mobs[0].ai.nav.kind = nav;
                assert!(!game.mob_add_goal(mob, 0, &goal));
                assert!(!game.mob_add_target_goal(mob, 0, &goal));
            }
            for nav in [
                sunrise_sim::mobs::nav::NavKind::Ground,
                sunrise_sim::mobs::nav::NavKind::WallClimber,
                sunrise_sim::mobs::nav::NavKind::Flying,
            ] {
                game.mobs[0].ai.nav.kind = nav;
                assert!(game.mob_add_goal(mob, 0, &goal));
                assert!(game.mob_add_target_goal(mob, 0, &goal));
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn removing_running_goal_supplies_snapshot_and_persists_level_random() {
        use sunrise_core::random::RandomSource;
        use sunrise_sim::mobs::goals::{Cx, Goal};
        #[derive(Clone, Debug)]
        struct StopProbe;
        impl Goal for StopProbe {
            fn flags(&self) -> u8 {
                0
            }
            fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
                true
            }
            fn box_clone(&self) -> Box<dyn Goal> {
                Box::new(self.clone())
            }
            fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
                m.extra.followers = cx.w.others.len() as i32;
                m.health = cx.level.random().next_int_bound(1000).unwrap() as f32;
            }
        }
        with_resident_game(|game| {
            let handle = mob_in_game(game, "minecraft:zombie", 99);
            mob_in_game(game, "minecraft:zombie", 100);
            let mut mobs = game.mobs.lift();
            let snapshot: Vec<_> = mobs
                .iter()
                .map(|m| {
                    sunrise_gameplay_common::game::creatures::seen(
                        m,
                        game.level.player_uuid,
                        game.items(),
                    )
                })
                .collect();
            let mut test_level = sunrise_sim::health::TestLevel::new(0);
            let mut level = test_level.level();
            let mut events = Vec::new();
            let mut selector = std::mem::take(&mut mobs[0].ai.goals);
            selector.clear();
            selector.add(0, StopProbe);
            {
                let world = game.mob_world_base(&snapshot);
                let mut cx = Cx {
                    w: &world,
                    level: &mut level,
                    events: &mut events,
                };
                selector.tick(&mut mobs[0], &mut cx);
            }
            mobs[0].ai.goals = selector;
            game.mobs
                .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
            let mut expected = game.world.random.clone();
            let draw = expected.next_int_bound(1000).unwrap();
            let next = expected.next_int_bound(1000).unwrap();
            assert_eq!(handle.remove_goal(game, "StopProbe"), Some(1));
            assert_eq!(game.mobs[0].extra.followers, 2);
            assert_eq!(game.mobs[0].health, draw as f32);
            assert_eq!(game.world.random.next_int_bound(1000).unwrap(), next);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn removing_hold_ground_notifies_nearby_raiders() {
        use sunrise_sim::mobs::{
            Who,
            goals::Cx,
            raider::{self, HoldGroundAttack},
        };
        with_resident_game(|game| {
            let handle = mob_in_game(game, "minecraft:pillager", 99);
            mob_in_game(game, "minecraft:pillager", 100);
            let mut mobs = game.mobs.lift();
            raider::state_mut(&mut mobs[0]).patrolling = true;
            mobs[0].ai.target = Some(Who::Player);
            let snapshot: Vec<_> = mobs
                .iter()
                .map(|m| {
                    sunrise_gameplay_common::game::creatures::seen(
                        m,
                        game.level.player_uuid,
                        game.items(),
                    )
                })
                .collect();
            let mut test_level = sunrise_sim::health::TestLevel::new(0);
            let mut level = test_level.level();
            let mut events = Vec::new();
            let mut selector = std::mem::take(&mut mobs[0].ai.goals);
            selector.clear();
            selector.add(0, HoldGroundAttack::new(8.0));
            {
                let world = game.mob_world_base(&snapshot);
                let mut cx = Cx {
                    w: &world,
                    level: &mut level,
                    events: &mut events,
                };
                selector.tick(&mut mobs[0], &mut cx);
            }
            assert!(selector.is_running("HoldGroundAttack"));
            mobs[0].ai.goals = selector;
            game.mobs
                .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
            assert_eq!(handle.remove_goal(game, "HoldGroundAttack"), Some(1));
            assert!(game.mobs[1].ai.aggressive);
            assert_eq!(game.mobs[1].ai.target, Some(Who::Player));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn removing_school_goal_updates_the_leader() {
        use sunrise_sim::mobs::{aquatic::FollowFlockLeader, goals::Cx};
        with_resident_game(|game| {
            let follower = mob_in_game(game, "minecraft:cod", 99);
            let _leader = mob_in_game(game, "minecraft:cod", 100);
            let mut mobs = game.mobs.lift();
            mobs[0].extra.leader = Some(100);
            mobs[1].extra.followers = 1;
            let snapshot: Vec<_> = mobs
                .iter()
                .map(|m| m.seen(game.level.player_uuid, game.items()))
                .collect();
            let mut test_level = sunrise_sim::health::TestLevel::new(0);
            let mut level = test_level.level();
            let mut events = Vec::new();
            let mut selector = std::mem::take(&mut mobs[0].ai.goals);
            selector.clear();
            selector.add(0, FollowFlockLeader::new(0));
            {
                let world = game.mob_world_base(&snapshot);
                let mut context = Cx {
                    w: &world,
                    level: &mut level,
                    events: &mut events,
                };
                selector.tick(&mut mobs[0], &mut context);
            }
            assert!(selector.is_running("FollowFlockLeaderGoal"));
            assert!(
                events.is_empty(),
                "starting an established follower must not change the school"
            );
            mobs[0].ai.goals = selector;
            game.mobs
                .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
            assert_eq!(follower.remove_goal(game, "FollowFlockLeaderGoal"), Some(1));
            assert_eq!(
                game.mobs[0].extra.leader, None,
                "the actual stop method must run"
            );
            assert_eq!(
                game.mobs[1].extra.followers, 0,
                "stopping a follower must decrement its leader"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn goals_read_add_remove_and_refuse_what_the_mob_does_not_hold() {
        crate::game::server_side::entity_server::taken_pass_tests::in_both_dimensions(
            |game, balance| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in 0..16 {
                    for z in 0..16 {
                        game.world.set(x, 64, z, stone).unwrap();
                    }
                }
                let zombie = mob_in_game(game, "minecraft:zombie", 99);
                let slime = mob_in_game(game, "minecraft:slime", 100);
                let cow = mob_in_game(game, "minecraft:cow", 101);
                // The species set reads in the canonical ids.
                assert!(has(game, zombie, "melee_attack"));
                assert!(has(game, zombie, "look_at_player"));
                let targets = zombie.target_goals(game).unwrap();
                assert!(targets.iter().any(|e| e.id == "nearest_target"));
                assert!(targets.iter().any(|e| e.id == "hurt_by_target"));
                // Add: a Panic the zombie did not have.
                assert!(zombie.add_goal(
                    game,
                    4,
                    &GoalSpec::Panic {
                        speed: 1.4,
                        water_range: None,
                    }
                ));
                assert!(
                    zombie
                        .goals(game)
                        .unwrap()
                        .iter()
                        .any(|e| { e.priority == 4 && e.id == "panic" && !e.running })
                );
                // The target selector takes a species filter.
                assert!(zombie.add_target_goal(
                    game,
                    3,
                    &GoalSpec::NearestTarget {
                        prey: MobFilter::Species(vec!["minecraft:skeleton".into()]),
                        must_see: true,
                        must_reach: false,
                        interval: 10,
                    }
                ));
                // Refusals: non-finite speed, a mob with no PathfinderMob goals,
                // a goal only an animal holds, an unknown species id.
                assert!(!zombie.add_goal(
                    game,
                    4,
                    &GoalSpec::Panic {
                        speed: f64::NAN,
                        water_range: None,
                    }
                ));
                assert!(slime.add_goal(
                    game,
                    4,
                    &GoalSpec::Panic {
                        speed: 1.0,
                        water_range: None,
                    }
                ));
                assert!(slime.add_goal(
                    game,
                    4,
                    &GoalSpec::MeleeAttack {
                        speed: 1.0,
                        follow_unseen: false,
                    }
                ));
                assert!(slime.add_goal(game, 4, &GoalSpec::Float));
                assert!(!zombie.add_goal(game, 4, &GoalSpec::Breed { speed: 1.0 }));
                assert!(cow.add_goal(game, 4, &GoalSpec::Breed { speed: 1.0 }));
                assert!(!zombie.add_target_goal(
                    game,
                    3,
                    &GoalSpec::NearestTarget {
                        prey: MobFilter::Species(vec!["minecraft:unknown".into()]),
                        must_see: true,
                        must_reach: false,
                        interval: 10,
                    }
                ));
                // Remove: the species' melee attack goes, and only that one.
                crate::game::server_side::entity_server::taken_pass_tests::assert_mob_spawn_survives(
                game,
                balance,
                |game| {
                    assert_eq!(zombie.remove_goal(game, "melee_attack"), Some(1));
                },
            );
                assert!(!has(game, zombie, "melee_attack"));
                assert_eq!(zombie.remove_goal(game, "melee_attack"), Some(0));
                assert_eq!(zombie.remove_target_goal(game, "hurt_by_target"), Some(1));
                // A stale handle and a mob whose copy is held out.
                let stale = MobRef::new(EntityRef::new(WorldEpoch::new(999), 99));
                assert!(!stale.add_goal(game, 1, &GoalSpec::Float));
                assert!(stale.goals(game).is_none());
                let taken = game.mobs.lift();
                assert!(zombie.goals(game).is_none());
                assert_eq!(zombie.remove_goal(game, "panic"), None);
                game.mobs
                    .restore(taken, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
            },
        );
    }
}
