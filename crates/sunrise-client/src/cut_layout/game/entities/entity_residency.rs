#![cfg(test)]
use mod_api::DimensionKey;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;

#[cfg(test)]
#[path = "entity_residency/all_family_binding_tests.rs"]
mod all_family_binding_tests;

#[cfg(test)]
#[path = "entity_residency/lifetime_tests.rs"]
mod lifetime_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_client_lib::join_seq::Joined;
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    use sunrise_gameplay::game::commands::Target;
    use sunrise_gameplay::game::entity_residency::SlotAccess;
    use sunrise_gameplay::game::native_entity_refs::tests::{fixture_resident_column, with_game};
    use sunrise_sim::{
        join_seq::{RestoreOrder, residency::table::EntityLookup},
        mobs::Mob,
    };

    #[test]
    #[ignore = "daily"]
    fn resident_slot_access_routes_live_parked_taken_and_disjoint() {
        use std::num::NonZeroU32;
        use sunrise_gameplay_common::game::Dim;
        let id = |value| NonZeroU32::new(value).unwrap();
        with_game(|game| {
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind.clone(), Vec3::ZERO, 0.0));
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| 7),
                SlotAccess::Done(7)
            );
            assert_eq!(
                game.with_resident_slot(&DimensionKey::NETHER, id(410), |_| panic!(
                    "wrong dimension"
                )),
                SlotAccess::<()>::Missing
            );
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(999), |_| panic!("missing")),
                SlotAccess::<()>::Missing
            );
            let join = game.mobs[0].join_seq();
            let extracted = game.mobs.extract(join).unwrap();
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| panic!("extracted")),
                SlotAccess::<()>::Taken
            );
            game.mobs.push(Mob::new(411, kind.clone(), Vec3::ZERO, 0.0));
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(411), |_| 8),
                SlotAccess::Done(8)
            );
            game.mobs.put_back(0, extracted);
            let lifted = game.mobs.lift();
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| panic!("lifted")),
                SlotAccess::<()>::Taken
            );
            game.mobs.push(Mob::new(412, kind, Vec3::ZERO, 0.0));
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(412), |_| 9),
                SlotAccess::Done(9)
            );
            game.mobs.restore(lifted, RestoreOrder::SurvivorsFirst);
            game.mobs.update(0, |mob| mob.removed = true);
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| 10),
                SlotAccess::Done(10)
            );
            game.summon_with_nbt("minecraft:tnt", [0.5, 70.0, 0.5], "{}")
                .unwrap();
            game.lan_assign_ids();
            let tnt = id(game.tnts[0].id);
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, tnt, |slot| slot.is_bound()),
                SlotAccess::Done(false)
            );
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, tnt, |slot| slot.is_bound()),
                SlotAccess::Done(false)
            );
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| 11),
                SlotAccess::Done(11)
            );
            assert_eq!(game.dim, Dim::Nether);
            game.enter_level(Dim::Overworld).unwrap();
            game.mobs.remove(0);
            assert_eq!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(410), |_| panic!("removed")),
                SlotAccess::<()>::Missing
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn resident_slot_access_cushion_reentry_and_panic_preserve_actual_lifetime() {
        use std::{
            num::NonZeroU32,
            sync::{Arc, Mutex},
        };
        use sunrise_mod_runtime::attachments::{
            Attachments, HasAttachmentLifetime, index::AttachmentIndex,
        };
        with_game(|game| {
            sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
                game,
                sunrise_core::position::ChunkPos::new(0, 0),
            );
            game.reset_entity_residency();
            game.summon_with_nbt("minecraft:cushion", [0.5, 70.0, 0.5], "{}")
                .unwrap();
            game.lan_assign_ids();
            let id = NonZeroU32::new(game.cushions[0].id).unwrap();
            let holder = game.entity(id.get()).unwrap();
            let index = Arc::new(Mutex::new(AttachmentIndex::default()));
            let mut values = Attachments::default();
            let journal = game.resident_entities.table.borrow().producer();
            let result = game.with_resident_slot(&DimensionKey::OVERWORLD, id, |slot| {
                // Reenter the same structural journal: a retained drain borrow panics.
                journal.drain_into(&mut Vec::new());
                slot.bind(&mut values, &index, holder);
                slot.registration().unwrap()
            });
            let SlotAccess::Done(registration) = result else {
                panic!("resident slot unavailable")
            };
            game.cushions.update(0, |mob| {
                assert!(
                    registration
                        .same_registration(&mob.attachment_lifetime().registration().unwrap())
                )
            });
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.with_resident_slot(&DimensionKey::OVERWORLD, id, |slot| {
                    assert!(registration.same_registration(&slot.registration().unwrap()));
                    panic!("visitor panic");
                });
            }));
            assert!(panic.is_err());
            game.cushions.update(0, |mob| {
                assert!(
                    registration
                        .same_registration(&mob.attachment_lifetime().registration().unwrap())
                )
            });
            assert!(game.resident_entities.table.try_borrow_mut().is_ok());
            game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            assert!(matches!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id, |slot| slot.is_bound()),
                SlotAccess::Done(true)
            ));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn game_residency_tracks_lift_spawn_restore_and_epoch_rebind() {
        with_game(|game| {
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind.clone(), Vec3::ZERO, 0.0));
            let join = game.mobs[0].join_seq();
            let taken = game.mobs.lift();
            assert_eq!(game.server_resident_entity(410), EntityLookup::Taken);
            game.mobs.push(Mob::new(411, kind, Vec3::ZERO, 0.0));
            assert!(matches!(
                game.server_resident_entity(411),
                EntityLookup::Found {
                    value: Target::Mob(411),
                    ..
                }
            ));
            game.mobs.restore(taken, RestoreOrder::SurvivorsFirst);
            assert!(
                matches!(game.server_resident_entity(410), EntityLookup::Found { location, value: Target::Mob(410) } if location.join == join)
            );
            game.reset_entity_residency();
            assert!(matches!(
                game.server_resident_entity(410),
                EntityLookup::Found { .. }
            ));
            game.mobs.remove(0);
            assert_eq!(game.server_resident_entity(410), EntityLookup::Missing);
            assert!(matches!(
                game.server_resident_entity(411),
                EntityLookup::Found { .. }
            ));
            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn initial_non_overworld_owner_is_bound_before_level_notification() {
        use sunrise_gameplay_common::game::Dim;
        with_game(|game| {
            game.dim = Dim::Nether;
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind, Vec3::ZERO, 0.0));
            game.native_level_opened(Dim::Nether);
            assert!(
                matches!(game.server_resident_entity(410), EntityLookup::Found { location, value: Target::Mob(410) } if location.owner.dimension == DimensionKey::NETHER)
            );
            game.native_lifecycle_world_closed();
            assert_eq!(game.server_resident_entity(410), EntityLookup::Missing);
            game.native_lifecycle_open_started();
            assert!(
                matches!(game.server_resident_entity(410), EntityLookup::Found { location, .. } if location.owner.dimension == DimensionKey::NETHER)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn parked_owners_rebind_and_route_without_trial_dimension_swaps() {
        use mod_api::EntityReadError;
        use sunrise_gameplay_common::game::Dim;
        with_game(|game| {
            fixture_resident_column(&mut game.common.world, (0, 0));
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind.clone(), Vec3::ZERO, 0.0));
            let entity = game.entity(410).unwrap();
            let living = game.living(entity).unwrap();
            let assert_refused_visits = |game: &sunrise_gameplay::game::ServerGame<'_>, error| {
                assert_eq!(
                    entity.visit_parts(game, &mut |_| panic!("visited parts")),
                    Err(error)
                );
                assert_eq!(
                    entity.visit_passengers(game, &mut |_| panic!("visited passengers")),
                    Err(error)
                );
                assert_eq!(
                    entity.visit_indirect_passengers(game, &mut |_| panic!(
                        "visited indirect passengers"
                    )),
                    Err(error)
                );
                assert_eq!(
                    entity.visit_fluid_heights(game, &mut |_, _| panic!("visited fluids")),
                    Err(error)
                );
                assert_eq!(
                    living.visit_effects(game, &mut |_| panic!("visited effects")),
                    Err(error)
                );
            };
            game.enter_level(Dim::Nether).unwrap();
            assert_refused_visits(game, EntityReadError::Unavailable);
            game.mobs.push(Mob::new(411, kind, Vec3::ZERO, 0.0));
            assert!(
                matches!(game.server_resident_entity(410), EntityLookup::Found { location, .. } if location.owner.dimension == DimensionKey::OVERWORLD)
            );
            game.reset_entity_residency();
            assert!(matches!(
                game.server_resident_entity(410),
                EntityLookup::Found { .. }
            ));
            assert!(
                matches!(game.server_resident_entity(411), EntityLookup::Found { location, .. } if location.owner.dimension == DimensionKey::NETHER)
            );
            assert_eq!(game.level_entity(410), None);
            assert_eq!(game.level_entity(411), Some(Target::Mob(411)));
            assert_eq!(
                game.in_target_level(Target::Mob(410), |game| game.dim),
                Some(Dim::Overworld)
            );
            assert_eq!(game.dim, Dim::Nether);
            game.server.levels.0.clear();
            assert_refused_visits(game, EntityReadError::Stale);
            assert_eq!(game.server_resident_entity(410), EntityLookup::Missing);
            assert!(matches!(
                game.server_resident_entity(411),
                EntityLookup::Found { .. }
            ));
            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn logical_removal_is_not_lifted_absence_or_physical_retirement() {
        with_game(|game| {
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind.clone(), Vec3::ZERO, 0.0));
            let taken = game.mobs.lift();
            game.mobs.push(Mob::new(411, kind, Vec3::ZERO, 0.0));
            game.mobs[0].removed = true;
            assert_eq!(game.server_resident_entity(411), EntityLookup::Missing);
            // Conversion can restore a resident after a pre hook. A read while
            // removed must not destroy its physical residency row.
            game.mobs[0].removed = false;
            assert!(matches!(
                game.server_resident_entity(411),
                EntityLookup::Found { .. }
            ));
            game.persist.removals.push(Target::Mob(411));
            assert_eq!(game.server_resident_entity(411), EntityLookup::Missing);
            game.persist.removals.clear();
            assert!(matches!(
                game.server_resident_entity(411),
                EntityLookup::Found { .. }
            ));
            game.mobs.restore(taken, RestoreOrder::SurvivorsFirst);
            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn late_tracking_admission_publishes_the_previously_zero_id() {
        with_game(|game| {
            game.reset_entity_residency();
            game.add_primed_tnt(BlockPos::new(0, 70, 0), 0.0, 80, None, None);
            let mut tnt = game.tnts.remove(0);
            let retired = tnt.id;
            tnt.id = 0;
            game.tnts.push(tnt);
            assert_eq!(game.server_resident_entity(retired), EntityLookup::Missing);
            assert_eq!(game.server_resident_entity(0), EntityLookup::Missing);
            game.lan_assign_ids();
            let id = game.tnts[0].id;
            assert_ne!(id, 0);
            assert!(
                matches!(game.server_resident_entity(id), EntityLookup::Found { value: Target::Tnt(found), .. } if found == id)
            );
            game.lan_assign_ids();
            assert_eq!(game.tnts[0].id, id);
            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
        });
    }
}
