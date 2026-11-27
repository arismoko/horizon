#![cfg(test)]
use mod_api::{DragonPhase, DragonRef, EntityRef};
use sunrise_gameplay_common::game::dragon_common::Dragon;
use sunrise_gameplay_common::game::native_entity_motion_values::NativeEntityMotionValuesGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6 as _;

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::{with_game, with_resident_game};
    use super::*;
    use mod_api::{EntityReadError, MobRef, WorldEpoch};
    use std::ops::ControlFlow;
    use sunrise_core::geometry::Vec3;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::Dim;
    use sunrise_gameplay_common::game::test_fixtures::{
        collect_entity_visit, fixture_resident_column,
    };
    use sunrise_protocol::entity_data::{Value, serializer};
    use sunrise_sim::{entities::ArmorStand, mobs::Mob};

    fn dragon_in(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> (EntityRef, DragonRef) {
        let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 15;
        game.dragon.replace(dragon);
        let entity = game.entity(15).unwrap();
        (entity, game.dragon(entity).unwrap())
    }

    #[test]
    #[ignore = "daily"]
    fn dragon_target_change_can_be_cancelled() {
        struct Cancel;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Cancel {
            fn living_change_target(
                &self,
                host: &dyn mod_api::Host,
                event: &mut mod_api::LivingChangeTargetEvent<'_>,
            ) -> mod_api::Flow {
                assert!(event.mob.living().health(host).is_some());
                mod_api::Flow::Cancel
            }
        }
        let mods = vec![mod_api::ModEntry::new("cancel-target", Box::new(Cancel))].into();
        mod_platform::testing::with_mods(mods, || {
            with_resident_game(|game| {
                let (_, dragon) = dragon_in(game);
                assert!(!dragon.mob().set_target(game, None));
                assert_eq!(game.dragon.as_ref().unwrap().target, None);
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_dragon_is_a_mob_with_mob_state() {
        with_resident_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind.clone(), Vec3::ZERO, 0.0));
            let mut shielded = Mob::new(13, kind, Vec3::ZERO, 0.0);
            shielded.invulnerable_flag = true;
            game.mobs.push(shielded);
            game.armor_stands.push(ArmorStand::new(14, Vec3::ZERO, 0.0));
            let (entity, dragon) = dragon_in(game);
            assert!(game.dragon(game.entity(12).unwrap()).is_none());
            let mob = game.mob(entity).unwrap();
            assert_eq!(mob, dragon.mob());
            assert_eq!(mob.living().entity(), entity);

            // Aggression is a synced mob flag; persistence is saved.
            assert_eq!(mob.is_aggressive(game), Some(false));
            assert!(mob.set_aggressive(game, true));
            assert_eq!(mob.is_aggressive(game), Some(true));
            let state = game.lan_dragon_state().unwrap();
            assert!(state.meta.contains(&(15, serializer::BYTE, Value::Byte(4))));
            assert!(mob.set_aggressive(game, false));
            let state = game.lan_dragon_state().unwrap();
            assert!(state.meta.iter().all(|m| m.0 != 15));
            assert_eq!(mob.persistence_required(game), Some(false));
            assert!(mob.set_persistence_required(game));
            assert_eq!(mob.persistence_required(game), Some(true));
            let saved = game
                .dragon
                .as_ref()
                .unwrap()
                .to_nbt([0; 4], None, &Default::default());
            assert_eq!(saved.get("PersistenceRequired"), Some(&Tag::Byte(1)));
            assert!(Dragon::from_nbt(&saved, 0).unwrap().persistent);
            // `getBooleanOr` takes any nonzero number as true.
            for (value, want) in [
                (Tag::Byte(2), true),
                (Tag::Int(1), true),
                (Tag::Byte(0), false),
            ] {
                let mut tag = saved.clone();
                if let Tag::Compound(map) = &mut tag {
                    map.insert("PersistenceRequired".into(), value);
                }
                assert_eq!(Dragon::from_nbt(&tag, 0).unwrap().persistent, want);
            }

            // A target a mod set: valid mobs only, and it goes stale.
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let shield = game.living(game.entity(13).unwrap()).unwrap();
            let stand = game.living(game.entity(14).unwrap()).unwrap();
            assert_eq!(mob.target(game), None);
            assert!(mob.set_target(game, Some(zombie)));
            assert_eq!(mob.target(game), Some(zombie));
            assert!(!mob.set_target(game, Some(shield)));
            assert!(!mob.set_target(game, Some(stand)));
            assert!(!mob.set_target(game, Some(dragon.mob().living())));
            assert_eq!(mob.target(game), Some(zombie));
            game.mobs[0].invulnerable_flag = true;
            assert_eq!(mob.target(game), None);
            game.mobs[0].invulnerable_flag = false;
            assert_eq!(mob.target(game), Some(zombie));
            assert!(mob.set_target(game, None));
            assert_eq!(mob.target(game), None);

            // The dragon flies by phase, so MobRef navigation calls are unavailable.
            assert!(!mob.move_to(game, [1.0, 1.0, 1.0], 1.0));
            assert!(!mob.stop(game));
            assert_eq!(mob.is_done(game), None);

            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                15,
            ));
            assert!(!stale.set_aggressive(game, true));
            assert_eq!(stale.persistence_required(game), None);
            assert!(!stale.set_target(game, None));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_phase_is_read_set_synced_and_saved() {
        with_resident_game(|game| {
            let (entity, dragon) = dragon_in(game);
            // A constructed dragon (as with /summon) hovers; only the fight's
            // create_new_dragon switches it to the holding pattern.
            assert_eq!(dragon.phase(game), Some(DragonPhase::Hover));
            for (i, phase) in [
                DragonPhase::HoldingPattern,
                DragonPhase::StrafePlayer,
                DragonPhase::LandingApproach,
                DragonPhase::Landing,
                DragonPhase::Takeoff,
                DragonPhase::SittingFlaming,
                DragonPhase::SittingScanning,
                DragonPhase::SittingAttacking,
                DragonPhase::ChargingPlayer,
                DragonPhase::Dying,
                DragonPhase::Hover,
            ]
            .into_iter()
            .enumerate()
            {
                assert_eq!(phase.id(), i as i32);
                assert_eq!(DragonPhase::from_id(i as i32), Some(phase));
                assert!(dragon.set_phase(game, phase));
                assert_eq!(dragon.phase(game), Some(phase));
                let state = game.lan_dragon_state().unwrap();
                let synced = state.meta.iter().find(|m| m.0 == 16);
                if phase == DragonPhase::Hover {
                    assert_eq!(synced, None);
                } else {
                    assert_eq!(synced, Some(&(16, serializer::INT, Value::Int(i as i32))));
                }
                let saved = game
                    .dragon
                    .as_ref()
                    .unwrap()
                    .to_nbt([0; 4], None, &Default::default());
                assert_eq!(saved.get("DragonPhase"), Some(&Tag::Int(i as i32)));
                let loaded = Dragon::from_nbt(&saved, 0).unwrap();
                assert_eq!(loaded.native_phase(), phase);
            }
            assert_eq!(DragonPhase::from_id(11), None);
            assert_eq!(DragonPhase::from_id(-1), None);

            // Setting the phase it holds changes nothing, a strafe included.
            assert!(dragon.set_phase(game, DragonPhase::Hover));
            assert_eq!(dragon.phase(game), Some(DragonPhase::Hover));

            // The fight: unloaded until the End loads it, and only the End's.
            assert_eq!(dragon.alive_crystals(game), None);
            assert_eq!(dragon.previously_killed(game), None);
            game.server.fight.loaded = true;
            game.server.fight.alive_crystals = 3;
            game.server.fight.previously_killed = true;
            assert_eq!(dragon.alive_crystals(game), None, "not in the End");
            assert_eq!(dragon.previously_killed(game), None);
            game.dim = Dim::End;
            // This fixture relocates the level in place rather than loading
            // another one. Rebind its owners to the new dimension as well.
            game.reset_entity_residency();
            assert_eq!(dragon.alive_crystals(game), Some(3));
            assert_eq!(dragon.previously_killed(game), Some(true));

            let stale = DragonRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                15,
            ));
            assert!(!stale.set_phase(game, DragonPhase::Landing));
            assert_eq!(stale.phase(game), None);
            assert_eq!(stale.alive_crystals(game), None);
            game.dragon.discard();
            assert_eq!(dragon.phase(game), None);
            assert!(!dragon.set_phase(game, DragonPhase::Landing));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn leaving_flaming_discards_the_cloud_at_once() {
        use sunrise_gameplay_common::game::dragon_common::Breath;
        with_resident_game(|game| {
            let (_, dragon) = dragon_in(game);
            let cloud = |from_dragon| Breath {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 0,
                at: [0.0; 3],
                motion: [0.0; 3],
                fall_distance: 0.0,
                fire_ticks: 0,
                first_tick: true,
                radius: 5.0,
                age: 0,
                duration: 200,
                from_dragon,
                victims: Vec::new(),
                potion: None,
            };
            assert!(dragon.set_phase(game, DragonPhase::SittingFlaming));
            game.breath_clouds.push(cloud(true));
            game.breath_clouds.push(cloud(false));
            assert!(dragon.set_phase(game, DragonPhase::Hover));
            assert_eq!(game.breath_clouds.len(), 1, "the flame cloud is gone");
            assert!(!game.breath_clouds[0].from_dragon);
            // Back in and out within one hook leaves nothing of the old cloud.
            assert!(dragon.set_phase(game, DragonPhase::SittingFlaming));
            game.breath_clouds.push(cloud(true));
            assert!(dragon.set_phase(game, DragonPhase::Hover));
            assert!(dragon.set_phase(game, DragonPhase::SittingFlaming));
            assert_eq!(game.breath_clouds.len(), 1);
            // The gameplay path leaves the flag for the tick.
            game.breath_clouds.push(cloud(true));
            game.dragon
                .as_mut()
                .unwrap()
                .set_native_phase(DragonPhase::Hover);
            game.dragon
                .as_mut()
                .unwrap()
                .set_native_phase(DragonPhase::SittingFlaming);
            game.tick_breath();
            assert!(game.breath_clouds.iter().all(|c| !c.from_dragon));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_invulnerable_player_is_not_a_target() {
        with_resident_game(|game| {
            let (entity, _) = dragon_in(game);
            let mob = game.mob(entity).unwrap();
            game.player =
                Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 30, 7);
            let target = game.entity(30).unwrap();
            let living = game.living(target).unwrap();
            assert!(mob.set_target(game, Some(living)));
            game.player.as_mut().unwrap().invulnerable = true;
            assert_eq!(mob.target(game), None);
            assert!(!mob.set_target(game, Some(living)));
        });
    }

    /// Part `i` of the test dragon: a position nothing else shares and, from
    /// vanilla's `EnderDragon` constructor, the part's width and height.
    fn part_fixture(i: usize) -> ([f64; 3], [f64; 2]) {
        const SIZES: [[f64; 2]; 8] = [
            [1.0, 1.0],
            [3.0, 3.0],
            [5.0, 3.0],
            [2.0, 2.0],
            [2.0, 2.0],
            [2.0, 2.0],
            [4.0, 2.0],
            [4.0, 2.0],
        ];
        let i_f = i as f64;
        (
            [100.0 + 10.0 * i_f, 70.0 + i_f, -20.0 + 0.5 * i_f],
            SIZES[i],
        )
    }

    fn place_parts(game: &mut sunrise_gameplay_common::game::CommonGame<'_>) {
        for i in 0..8 {
            let [x, y, z] = part_fixture(i).0;
            game.dragon.as_mut().unwrap().parts[i].0 = Vec3::new(x, y, z);
        }
    }

    fn assert_parts_are_the_dragons_own<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
        game: &G,
        dragon: EntityRef,
    ) {
        let parts =
            collect_entity_visit(|visit| dragon.visit_parts(game.as_host(), visit)).unwrap();
        let ids: Vec<u32> = parts.iter().map(|part| part.id()).collect();
        assert_eq!(ids, (dragon.id() + 1..=dragon.id() + 8).collect::<Vec<_>>());
        assert_eq!(game.entity_is_multipart(dragon), Some(true));
        for (i, part) in parts.iter().enumerate() {
            let ([x, y, z], [width, height]) = part_fixture(i);
            // The handle `entity` gives is the one `parts` listed.
            assert_eq!(game.entity(part.id()), Some(*part));
            assert_ne!(*part, dragon);
            assert_eq!(game.entity_position(*part), Some([x, y, z]));
            assert_eq!(
                game.entity_bounding_box(*part),
                Some([
                    [x - width / 2.0, y, z - width / 2.0],
                    [x + width / 2.0, y + height, z + width / 2.0]
                ])
            );
            assert_eq!(
                game.entity_kind(*part)
                    .and_then(|kind| kind.id(game.as_host()))
                    .as_deref(),
                Some("minecraft:ender_dragon")
            );
            assert_eq!(game.entity_rotation(*part), Some([0.0, 0.0]));
            assert_eq!(game.entity_velocity(*part), Some([0.0; 3]));
            assert_eq!(game.entity_level(*part), game.entity_level(dragon));
            assert_eq!(game.entity_is_multipart(*part), Some(false));
            assert_eq!(
                collect_entity_visit(|visit| part.visit_parts(game.as_host(), visit)),
                Ok(Vec::new())
            );
            // Its rotation is none, so what is up for it is up.
            assert!(game.entity_up_vector(*part).unwrap()[1] > 0.999_999);
            // No living entity, mob or dragon behind it.
            assert!(game.living(*part).is_none());
            assert!(game.mob(*part).is_none());
        }
        // The parent keeps its own place and has no box.
        assert_eq!(game.entity_position(dragon), Some([0.0; 3]));
        assert_eq!(game.entity_bounding_box(dragon), None);
    }

    #[test]
    #[ignore = "daily"]
    fn the_dragons_parts_resolve_as_entities_of_their_own() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let (entity, _) = dragon_in(game);
            place_parts(game);
            assert_parts_are_the_dragons_own(game, entity);
            let parts = collect_entity_visit(|visit| entity.visit_parts(game, visit)).unwrap();
            let mut visited = None;
            assert_eq!(
                entity.visit_parts(game, &mut |part| {
                    assert!(visited.replace(part).is_none());
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(visited, Some(parts[0]));
            // If one part is temporarily out of the readable view, do not visit an earlier part.
            game.server.held_out.push(parts[7].id());
            assert_eq!(
                entity.visit_parts(game, &mut |_| panic!("partial dragon was visited")),
                Err(EntityReadError::Unavailable)
            );
            assert_eq!(game.server.held_out.pop(), Some(parts[7].id()));
            assert!(parts.iter().all(|part| game.dragon(*part).is_none()));
            // Only the eight ids after the dragon's are its parts.
            assert!(game.entity(entity.id() - 1).is_none());
            assert!(game.entity(entity.id() + 9).is_none());
            // A part cannot leave alone.
            assert!(!game.entity_discard(parts[0]));
            assert!(
                !game.entity_remove(parts[7], mod_api::RemovalReason::Killed)
                    && game.entity(parts[7].id()).is_some()
            );
            // A handle of an earlier open names nothing.
            let stale = EntityRef::new(WorldEpoch::new(entity.epoch().get() + 1), parts[0].id());
            assert_eq!(game.entity_position(stale), None);
            assert_eq!(game.entity_bounding_box(stale), None);
            assert_eq!(game.entity_is_multipart(stale), None);
            let stale_dragon = EntityRef::new(stale.epoch(), entity.id());
            assert_eq!(
                collect_entity_visit(|visit| stale_dragon.visit_parts(game, visit)),
                Err(EntityReadError::Stale)
            );
            // Parts cannot move independently of their parent, even without rotation.
            let before = game.entity_position(parts[2]);
            assert!(!game.entity_teleport_to(parts[2], [10.0, 80.0, 10.0], None));
            assert_eq!(game.entity_position(parts[2]), before);
            // A dragon that is dying still has its parts.
            game.dragon.as_mut().unwrap().health = 0.0;
            assert_eq!(game.entity_position(parts[2]), Some(part_fixture(2).0));
            // A dragon a mod removed takes its parts with it, before the sweep.
            assert!(game.entity_discard(entity));
            for part in &parts {
                assert_eq!(game.entity_position(*part), None);
                assert!(game.entity(part.id()).is_none());
            }
            assert_eq!(
                collect_entity_visit(|visit| entity.visit_parts(game, visit)),
                Err(EntityReadError::Stale)
            );
            game.native_removals_flush();
            assert!(game.dragon.is_none());
            for part in &parts {
                assert_eq!(game.entity_bounding_box(*part), None);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_dragon_that_has_no_id_yet_has_no_parts() {
        with_game(|game| {
            game.dragon.replace(Dragon::new(Vec3::ZERO, 0.0, 0));
            // The ids after 0 are the player's and its neighbours', not parts.
            for id in 1..=8 {
                assert!(game.level_dragon_part(id).is_none());
            }
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_clients_dragon_parts_resolve_through_its_own_copy() {
        use super::super::native_entity_refs::tests::with_client_game;
        use sunrise_protocol::connection::Pipe;
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "PartReadTest").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player =
                Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.spawn_entity(15, "minecraft:ender_dragon", [0.0; 3], [0.0; 3]);
            place_parts(game);
            // The server tells this client of the dragon alone; the parts are its copies of it.
            let dragon = game.entity(15).unwrap();
            assert_parts_are_the_dragons_own(game, dragon);
            let parts = collect_entity_visit(|visit| dragon.visit_parts(game, visit)).unwrap();
            for part in &parts {
                assert_eq!(part.vehicle(game), None);
                assert_eq!(part.root_vehicle(game), Some(*part));
                assert_eq!(part.is_passenger(game), Some(false));
                assert_eq!(part.is_vehicle(game), Some(false));
                assert_eq!(
                    collect_entity_visit(|visit| part.visit_passengers(game, visit)),
                    Ok(Vec::new())
                );
            }
            // The window's own player is not one, and a stale handle is none.
            assert_eq!(game.entity(7).map(|e| e.id()), Some(7));
            let stale = EntityRef::new(WorldEpoch::new(dragon.epoch().get() + 1), parts[3].id());
            assert_eq!(game.entity_position(stale), None);
            game.client.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(game.entity_position(parts[3]), None);
        });
    }
}
