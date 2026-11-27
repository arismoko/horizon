#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_core::random::LegacyRandom;
use sunrise_sim::mobs::Species;
use sunrise_sim::mobs::riders;
#[cfg(test)]
pub(crate) mod off_level_tests {
    use super::*;
    use sunrise_sim::mobs::Mob;
    use {
        sunrise_gameplay::game::server_side::minecarts_server::CartHit,
        sunrise_gameplay_common::game::boats_common::Boat,
        sunrise_gameplay_common::game::entity_memory::OffLevel,
    };

    #[test]
    #[ignore = "daily"]
    fn dying_riding_edges_do_not_depend_on_unrelated_retained_objects() {
        for retained in [false, true] {
            for dying_vehicle in [false, true] {
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                    let mut vehicle = Mob::new(161, kind.clone(), Vec3::ZERO, 0.0);
                    let mut rider = Mob::new(162, kind.clone(), Vec3::ZERO, 0.0);
                    rider.extra.mount = Some(161);
                    if dying_vehicle {
                        vehicle.health = 0.0;
                    } else {
                        rider.health = 0.0;
                    }
                    game.mobs.push(vehicle);
                    game.mobs.push(rider);
                    if retained {
                        game.persist
                            .off_level
                            .insert(169, OffLevel::Mob(Mob::new(169, kind, Vec3::ZERO, 0.0)));
                    }
                    game.sync_riders();
                    assert_eq!(
                        game.mob_object(162).unwrap().extra.mount,
                        Some(161),
                        "retained={retained}, dying_vehicle={dying_vehicle}"
                    );
                });
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn first_rider_order_does_not_depend_on_unrelated_retained_objects() {
        for retained in [false, true] {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                let mut vehicle = Mob::new(171, kind.clone(), Vec3::ZERO, 0.0);
                vehicle.extra.rider = Some(172);
                game.mobs.push(vehicle);
                for (id, order) in [(172, 1), (173, 0)] {
                    let mut rider = Mob::new(id, kind.clone(), Vec3::ZERO, 0.0);
                    rider.extra.mount = Some(171);
                    rider.extra.vehicle_order = order;
                    game.mobs.push(rider);
                }
                if retained {
                    game.persist
                        .off_level
                        .insert(179, OffLevel::Mob(Mob::new(179, kind, Vec3::ZERO, 0.0)));
                }
                game.sync_riders();
                assert_eq!(
                    game.mob_object(171).unwrap().extra.rider,
                    Some(173),
                    "retained={retained}"
                );
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn content_seat_positioning_counts_retained_passengers() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let mut kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            std::sync::Arc::make_mut(&mut kind).passenger_attachments =
                vec![[1.0, 2.0, 3.0], [-1.0, 4.0, 5.0]];
            game.mobs.push(Mob::new(
                101,
                kind.clone(),
                Vec3::new(10.0, 64.0, 20.0),
                0.0,
            ));
            let mut held = Mob::new(102, kind.clone(), Vec3::ZERO, 0.0);
            held.extra.mount = Some(101);
            held.extra.vehicle_order = 0;
            game.persist.off_level.insert(102, OffLevel::Mob(held));
            let mut resident = Mob::new(103, kind, Vec3::ZERO, 0.0);
            resident.extra.mount = Some(101);
            resident.extra.vehicle_order = 1;
            game.mobs.push(resident);
            game.position_riders();
            let position = game.mob_object(103).unwrap().position;
            assert_eq!(position.x, 9.0);
            assert_eq!(position.z, 25.0);
            assert_eq!(game.mob_object(102).unwrap().position, Vec3::ZERO);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn dismounted_retained_multiseat_vehicle_snaps_each_frozen_passenger() {
        for name in ["minecraft:camel", "minecraft:happy_ghast"] {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let kind = game.res.mob_types().get(name).unwrap().clone();
                let mut root = Mob::new(70, pig.clone(), Vec3::new(10.0, 200.0, 20.0), 0.0);
                root.removed = true;
                let mut vehicle = Mob::new(71, kind, root.position, 0.0);
                vehicle.extra.mount = Some(70);
                game.mobs.push(root);
                game.persist.off_level.insert(71, OffLevel::Mob(vehicle));
                // Camel.getPassengerAttachmentPoint: two Animals occupy z +0.7/-0.5.
                // EntityTypes.HAPPY_GHAST: front, left, back, right, all at y +4.
                let expected: &[(f64, f64)] = if name == "minecraft:camel" {
                    &[(0.0, 0.7), (0.0, -0.5)]
                } else {
                    &[(0.0, 1.7), (-1.7, 0.0), (0.0, -1.7), (1.7, 0.0)]
                };
                // Reverse storage order: seats follow vehicle_order, not resident order.
                for seat in (0..expected.len()).rev() {
                    let id = 72 + seat as u32;
                    let mut rider = Mob::new(id, pig.clone(), Vec3::ZERO, 0.0);
                    rider.age = 19;
                    rider.extra.mount = Some(71);
                    rider.extra.vehicle_order = seat as i64;
                    if name == "minecraft:happy_ghast" && seat == 2 {
                        game.persist.off_level.insert(id, OffLevel::Mob(rider));
                        let mut child = Mob::new(80, pig.clone(), Vec3::ZERO, 0.0);
                        child.extra.mount = Some(id);
                        child.age = 23;
                        game.mobs.push(child);
                    } else {
                        game.mobs.push(rider);
                    }
                }
                game.sync_riders();
                // The normal pass must not overwrite the one-time snap of frozen descendants.
                game.position_riders();
                let vehicle = game.mob_object(71).unwrap();
                assert_eq!(vehicle.extra.mount, None);
                for (seat, &(x, z)) in expected.iter().enumerate() {
                    let rider = game.mob_object(72 + seat as u32).unwrap();
                    assert!((rider.position.x - vehicle.position.x - x).abs() < 1e-6);
                    assert!((rider.position.z - vehicle.position.z - z).abs() < 1e-6);
                    assert_eq!(rider.previous, rider.position);
                    assert_eq!(rider.age, 19);
                    if name == "minecraft:happy_ghast" {
                        assert!((rider.position.y - vehicle.position.y - 4.0).abs() < 1e-6);
                    }
                }
                if name == "minecraft:happy_ghast" {
                    let parent = game.mob_object(74).unwrap();
                    let child = game.mob_object(80).unwrap();
                    assert_eq!(child.position.x, parent.position.x);
                    assert_eq!(child.position.z, parent.position.z);
                    assert!((child.position.y - parent.position.y - 0.86875).abs() < 1e-6);
                    assert_eq!(child.age, 23);
                }
            });
        }
    }

    struct CountCanceledDismount(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CountCanceledDismount {
        fn entity_mount(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            mod_api::Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn destroyed_resident_vehicles_keep_canceled_rejected_edges() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        for cart in [false, true] {
            let cancel = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "toggle_dismount",
                Box::new(CountCanceledDismount(cancel.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let at = Vec3::new(10.0, 200.0, 20.0);
                    if cart {
                        game.carts
                            .push(sunrise_gameplay_common::game::minecarts::Minecart::new(
                                71,
                                sunrise_gameplay_common::game::minecarts::CartKind::Rideable,
                                at,
                            ));
                    } else {
                        game.boats.push(Boat::new(71, "oak".into(), at, 0.0));
                    }
                    let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    let mut rider = Mob::new(72, pig.clone(), Vec3::new(5.0, 190.0, 6.0), 0.0);
                    rider.extra.mount = Some(71);
                    game.persist.off_level.insert(72, OffLevel::Mob(rider));
                    let mut child = Mob::new(73, pig, Vec3::ZERO, 0.0);
                    child.extra.mount = Some(72);
                    child.age = 19;
                    game.mobs.push(child);
                    if cart {
                        game.hurt_cart(0, 5.0, true, false, CartHit::default());
                    } else {
                        assert!(game.hurt_boat(0, 5.0, true));
                    }
                    assert_eq!(cancel.load(Ordering::Relaxed), 1);
                    game.sync_riders();
                    let rider = game.mob_object(72).unwrap();
                    assert_eq!(rider.extra.mount, Some(71));
                    assert_eq!(rider.position, Vec3::new(5.0, 190.0, 6.0));
                    let child = game.mob_object(73).unwrap();
                    assert_eq!(child.position, Vec3::ZERO);
                    assert_eq!(child.age, 19);
                    game.prune_entity_memory();
                    assert!(game.persist.removed_vehicles.contains_key(&71));
                    assert!(game.persist.off_level.contains_key(&72));
                    assert!(game.mobs.iter().any(|m| m.id == 73));
                    assert!(!game.mobs.iter().any(|m| m.id == 72));
                    assert_eq!(cancel.load(Ordering::Relaxed), 1);
                });
            });
        }
    }

    struct CancelDismount;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CancelDismount {
        fn entity_mount(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            assert!(!event.mounting);
            mod_api::Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn invalid_retained_mount_uses_cancellable_dismount() {
        for cancel in [false, true] {
            let mods: mod_api::Mods = if cancel {
                vec![mod_api::ModEntry::new(
                    "cancel_dismount",
                    Box::new(CancelDismount),
                )]
                .into()
            } else {
                Vec::new().into()
            };
            mod_platform::testing::with_mods(mods, || {
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    let mut root = Mob::new(71, pig.clone(), Vec3::new(1.0, 200.0, 0.5), 0.0);
                    root.removed = true;
                    root.extra.rider = Some(72);
                    let mut rider = Mob::new(72, pig.clone(), Vec3::new(1.0, 190.0, 0.5), 0.0);
                    rider.extra.mount = Some(71);
                    rider.extra.passenger = true;
                    let mut child = Mob::new(73, pig, rider.position, 0.0);
                    child.extra.mount = Some(72);
                    child.age = 19;
                    game.mobs.push(child);
                    game.mobs.push(root);
                    game.persist.off_level.insert(72, OffLevel::Mob(rider));
                    game.sync_riders();
                    let rider = game.mob_object(72).unwrap();
                    if cancel {
                        assert_eq!(rider.extra.mount, Some(71));
                        assert!(rider.extra.passenger);
                        assert_eq!(rider.position.y, 190.0);
                        assert_eq!(game.mob_object(71).unwrap().extra.rider, Some(72));
                        assert_eq!(game.mob_object(73).unwrap().position.y, 190.0);
                    } else {
                        assert_eq!(rider.extra.mount, None);
                        assert!(!rider.extra.passenger);
                        assert_eq!(
                            rider.extra.boarding_cooldown,
                            sunrise_gameplay::game::server_side::boat_passengers::BOARDING_COOLDOWN
                        );
                        assert_eq!(rider.position.y, 200.0);
                        assert_eq!(game.mob_object(71).unwrap().extra.rider, None);
                        assert!(game.mob_object(73).unwrap().position.y > 200.0);
                    }
                    let child = game.mob_object(73).unwrap();
                    assert_eq!(child.age, 19);
                    assert_eq!(child.extra.mount, Some(72));
                });
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn dying_objects_keep_passenger_links_until_removal() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut root = Mob::new(71, pig.clone(), Vec3::new(1.0, 200.0, 0.5), 0.0);
            root.health = 0.0;
            let mut parent = Mob::new(72, pig.clone(), root.position, 0.0);
            parent.extra.mount = Some(71);
            let mut child = Mob::new(73, pig, root.position, 0.0);
            child.extra.mount = Some(72);
            game.mobs.push(root);
            game.mobs.push(child);
            game.persist.off_level.insert(72, OffLevel::Mob(parent));
            for dying_passenger in [false, true] {
                if dying_passenger {
                    game.mob_object_mut(72).unwrap().health = 0.0;
                }
                game.sync_riders();
                assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                assert_eq!(game.mob_object(71).unwrap().extra.rider, Some(72));
                assert_eq!(game.mob_object(73).unwrap().extra.mount, Some(72));
                assert_eq!(game.mob_object(72).unwrap().extra.rider, Some(73));
                assert_eq!(game.mob_object(73).unwrap().position.y, 200.0);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn synchronization_preserves_retained_controller_and_frozen_descendant() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut root = Mob::new(71, pig.clone(), Vec3::new(1.0, 80.0, 0.5), 0.0);
            root.extra.rider = Some(72);
            let mut parent = Mob::new(72, pig.clone(), root.position, 0.0);
            parent.extra.mount = Some(71);
            parent.extra.rider = Some(73);
            let mut child = Mob::new(73, pig, Vec3::new(7.0, 90.0, 4.0), 0.0);
            child.extra.mount = Some(72);
            child.extra.passenger = true;
            child.extra.boarding_cooldown = 12;
            let before = child.position;
            game.mobs.push(root);
            game.mobs.push(child);
            game.persist.off_level.insert(72, OffLevel::Mob(parent));
            game.sync_riders();
            assert_eq!(game.mob_object(71).unwrap().extra.rider, Some(72));
            assert_eq!(game.mob_object(72).unwrap().extra.rider, Some(73));
            let child = game.mob_object(73).unwrap();
            assert_eq!(child.extra.mount, Some(72));
            assert!(child.extra.passenger);
            assert_eq!(child.extra.boarding_cooldown, 12);
            assert_eq!(child.position, before);
        });
    }
}

#[cfg(test)]
pub(crate) mod nested_vehicle_discard_tests {
    use crate::game::server_side::native_entity_refs::tests::{
        collect_all_entities, collect_entities_in,
    };
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5;

    type VehicleKeySlot = Arc<std::sync::Mutex<Option<mod_api::AttachmentKey<i32>>>>;

    struct VehicleValueMod(VehicleKeySlot);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for VehicleValueMod {
        fn register(
            &self,
            registrar: &mut dyn mod_api::Registrar,
            _: &mod_api::RegistrationContext,
        ) {
            let key = registrar
                .register_attachment_type(mod_api::AttachmentDescriptor::new(
                    "vehicle_boundary:value",
                    || 0i32,
                ))
                .unwrap();
            *self.0.lock().unwrap() = Some(key);
        }
    }

    fn set_vehicle_test_attachment(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        key: &mod_api::AttachmentKey<i32>,
    ) {
        // The real store path binds the resident owner's registration. Rebinding
        // it manually would replace that lifetime before removal is exercised.
        key.set(game, mod_api::Holder::Entity(71), 37).unwrap();
    }

    #[cfg(test)]
    struct CancelUntilAllowed {
        cancel: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
    }

    #[cfg(test)]
    struct CancelPlayerDismount {
        cancel: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
    }

    #[cfg(test)]
    struct RemovalBoundaryObserver {
        kind: &'static str,
        cancel: Arc<AtomicBool>,
        mounts: Arc<AtomicUsize>,
        leaves: Arc<AtomicUsize>,
        destroys: Arc<AtomicUsize>,
        key: VehicleKeySlot,
    }

    #[cfg(test)]
    #[derive(Default)]
    struct SpillJoins {
        vehicle: AtomicUsize,
        items: AtomicUsize,
    }

    #[cfg(test)]
    struct ObserveDestroyedVehicle {
        kind: &'static str,
        calls: Arc<AtomicUsize>,
        key: VehicleKeySlot,
    }

    #[cfg(test)]
    struct CancelLive {
        count: Arc<AtomicUsize>,
        cross_level: bool,
    }

    #[cfg(test)]
    struct DiscardVehicleOnDismount(Arc<AtomicUsize>);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for DiscardVehicleOnDismount {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            assert!(!event.mounting);
            assert_eq!(event.entity, 72);
            self.0.fetch_add(1, Ordering::Relaxed);
            // The nested notification sees the root already logically removed.
            // Do not recurse deliberately or hold a lock across a host call.
            if let Some(vehicle) = host.entity(event.vehicle) {
                assert!(vehicle.discard(host));
            }
            mod_api::Flow::Cancel
        }
    }
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CancelUntilAllowed {
        fn entity_mount(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if !event.mounting && event.entity == 72 {
                self.calls.fetch_add(1, Ordering::Relaxed);
                if self.cancel.load(Ordering::Relaxed) {
                    return mod_api::Flow::Cancel;
                }
            }
            mod_api::Flow::Continue
        }
    }
    #[cfg(test)]
    struct CancelPassengerTick(Arc<AtomicBool>);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CancelPassengerTick {
        fn entity_tick_pre(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::EntityTickPreEvent<'_>,
        ) -> mod_api::Flow {
            if event.entity == 72 && self.0.load(Ordering::Relaxed) {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CancelPlayerDismount {
        fn entity_mount(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if !event.mounting && event.vehicle == 71 {
                self.calls.fetch_add(1, Ordering::Relaxed);
                if self.cancel.load(Ordering::Relaxed) {
                    return mod_api::Flow::Cancel;
                }
            }
            mod_api::Flow::Continue
        }
    }
    #[cfg(test)]
    struct RepositionCanceledPassenger(Arc<AtomicUsize>);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RepositionCanceledPassenger {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if !event.mounting
                && event.entity == 72
                && event.vehicle == 71
                && self.0.fetch_add(1, Ordering::Relaxed) == 0
            {
                assert!(
                    host.commands()
                        .run_command("tp 000000a0-0000-0000-0000-000000000048 30.5 180 40.5")
                );
                return mod_api::Flow::Cancel;
            }
            mod_api::Flow::Continue
        }
    }
    pub fn assert_vehicle_queries_hidden(
        host: &mut dyn mod_api::ServerHost,
        kind: &str,
        key: &mod_api::AttachmentKey<i32>,
    ) {
        assert!(host.entity(71).is_none());
        assert!(
            host.entity(73).is_some(),
            "removal must not hide unrelated live handles"
        );
        let level = host.level("minecraft:overworld").unwrap();
        for entities in [
            collect_all_entities(host, level).expect("the level remains queryable"),
            collect_entities_in(host, level, [-1000.0; 3], [1000.0; 3])
                .expect("spatial queries remain available"),
        ] {
            assert!(!entities.iter().any(|entity| entity.id() == 71));
            assert!(entities.iter().any(|entity| entity.id() == 73));
        }
        assert!(
            !host
                .commands()
                .run_command(&format!("execute if entity @e[type=minecraft:{kind}]"))
        );
        assert!(
            !host
                .commands()
                .run_command("execute if entity 000000a0-0000-0000-0000-000000000047")
        );
        let batch = mod_api::AttachmentQueries::capture(host, key).unwrap();
        assert!(
            batch
                .query(key)
                .unwrap()
                .iter()
                .try_next()
                .unwrap()
                .is_none()
        );
    }
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RemovalBoundaryObserver {
        fn register(
            &self,
            registrar: &mut dyn mod_api::Registrar,
            _: &mod_api::RegistrationContext,
        ) {
            let key = registrar
                .register_attachment_type(mod_api::AttachmentDescriptor::new(
                    "vehicle_boundary:value",
                    || 0i32,
                ))
                .unwrap();
            *self.key.lock().unwrap() = Some(key);
        }

        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if !event.mounting && event.entity == 72 {
                self.mounts.fetch_add(1, Ordering::Relaxed);
                let key = self.key.lock().unwrap().clone().unwrap();
                assert_vehicle_queries_hidden(host, self.kind, &key);
                // A stale vehicle must not become a new parent while removal
                // is notifying the old canceled edge.
                let level = host.level("minecraft:overworld").unwrap();
                let removed = mod_api::EntityRef::new(level.epoch(), 71);
                assert!(removed.position(host).is_none());
                assert!(
                    host.entity(73)
                        .unwrap()
                        .start_riding(host, removed, true)
                        .is_err()
                );
                if self.cancel.load(Ordering::Relaxed) {
                    return mod_api::Flow::Cancel;
                }
            }
            mod_api::Flow::Continue
        }
        fn entity_leave_level(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityLeaveLevelEvent<'_>,
        ) {
            if event.entity == 71 {
                self.leaves.fetch_add(1, Ordering::Relaxed);
                let key = self.key.lock().unwrap().clone().unwrap();
                assert_vehicle_queries_hidden(host, self.kind, &key);
            }
        }
        fn vehicle_destroy(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::VehicleDestroyEvent<'_>,
        ) {
            self.destroys.fetch_add(1, Ordering::Relaxed);
        }
    }
    #[cfg(test)]
    struct ObserveSpillItem(Arc<SpillJoins>);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ObserveSpillItem {
        fn entity_join_level(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityJoinLevelEvent<'_>,
        ) -> mod_api::Flow {
            if event.subject.id() == 71 {
                assert_eq!(
                    self.0.vehicle.fetch_add(1, Ordering::Relaxed),
                    0,
                    "the vehicle must not join again during its sweep"
                );
            }
            if event.kind == "minecraft:item" {
                if self.0.items.fetch_add(1, Ordering::Relaxed) == 0 {
                    let vehicle = host.entity(71).expect("contents drop before removal");
                    assert!(!vehicle.discard(host), "the spill must not reenter removal");
                } else {
                    assert!(
                        host.entity(71).is_none(),
                        "the vehicle item drops after removal"
                    );
                }
            }
            mod_api::Flow::Continue
        }
    }
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ObserveDestroyedVehicle {
        fn register(
            &self,
            registrar: &mut dyn mod_api::Registrar,
            _: &mod_api::RegistrationContext,
        ) {
            let key = registrar
                .register_attachment_type(mod_api::AttachmentDescriptor::new(
                    "vehicle_boundary:value",
                    || 0i32,
                ))
                .unwrap();
            *self.key.lock().unwrap() = Some(key);
        }

        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if !event.mounting && event.entity == 72 {
                self.calls.fetch_add(1, Ordering::Relaxed);
                assert!(host.entity(event.vehicle).is_none());
                assert!(!host.commands().run_command(&format!(
                    "execute if entity @e[type=minecraft:{}]",
                    self.kind
                )));
                let key = self.key.lock().unwrap().clone().unwrap();
                let batch = mod_api::AttachmentQueries::capture(host, &key).unwrap();
                assert!(
                    batch
                        .query(&key)
                        .unwrap()
                        .iter()
                        .try_next()
                        .unwrap()
                        .is_none()
                );
                return mod_api::Flow::Cancel;
            }
            mod_api::Flow::Continue
        }
    }
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CancelLive {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            assert!(!event.mounting);
            assert_eq!(event.entity, 72);
            let n = self.count.fetch_add(1, Ordering::Relaxed);
            if self.cross_level {
                assert!(host.commands().run_command(&format!(
                    "execute in minecraft:the_nether run worldborder set {}",
                    37 + n
                )));
                assert!(
                    host.entity(72).is_some(),
                    "the source level is restored inside the callback"
                );
            }
            mod_api::Flow::Cancel
        }
    }
    #[cfg(test)]
    struct ReparentLater(Arc<AtomicUsize>);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ReparentLater {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if !event.mounting && event.entity == 73 {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
            if !event.mounting && event.entity == 72 {
                let later = host.entity(73).unwrap();
                let replacement = host.entity(74).unwrap();
                assert!(later.start_riding(host, replacement, true).is_ok());
                return mod_api::Flow::Cancel;
            }
            mod_api::Flow::Continue
        }
    }
    #[cfg(test)]
    struct ReparentSelf(Arc<AtomicUsize>);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ReparentSelf {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if !event.mounting && event.entity == 72 && event.vehicle == 71 {
                self.0.fetch_add(1, Ordering::Relaxed);
                host.entity(71).unwrap().discard(host);
                let rider = host.entity(72).unwrap();
                let replacement = host.entity(74).unwrap();
                assert!(rider.start_riding(host, replacement, true).is_ok());
            }
            mod_api::Flow::Continue
        }
    }
    #[cfg(test)]
    struct ReviewKillOnMount;
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ReviewKillOnMount {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if event.mounting {
                host.entity(event.vehicle).unwrap().discard(host);
            }
            mod_api::Flow::Continue
        }
    }

    use super::*;

    use crate::game::{boats_common::Boat, entity_memory::OffLevel};

    use sunrise_gameplay::game::server_side::minecarts_server::CartHit;
    use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;

    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use sunrise_sim::mobs::Mob;

    fn discard_during_dismount(cart: bool) {
        let count = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "discard_vehicle_on_dismount",
            Box::new(DiscardVehicleOnDismount(count.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                for column in [(0, 0), (0, 1)] {
                    fixture_resident_column(&mut game.world, column);
                }
                let at = Vec3::new(10.0, 200.0, 20.0);
                if cart {
                    game.carts
                        .push(sunrise_gameplay_common::game::minecarts::Minecart::new(
                            71,
                            sunrise_gameplay_common::game::minecarts::CartKind::Rideable,
                            at,
                        ));
                } else {
                    game.boats.push(Boat::new(71, "oak".into(), at, 0.0));
                }
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let mut rider = Mob::new(72, kind.clone(), Vec3::new(2.0, 190.0, 3.0), 0.0);
                rider.extra.mount = Some(71);
                game.mobs.push(rider);
                let mut child = Mob::new(73, kind, Vec3::ZERO, 0.0);
                child.extra.mount = Some(72);
                child.age = 19;
                game.persist.off_level.insert(73, OffLevel::Mob(child));
                let rider = game.entity(72).unwrap();
                game.entity_stop_riding(rider);
                assert_eq!(
                    count.load(Ordering::Relaxed),
                    1,
                    "nested removal must not dispatch the same in-flight dismount twice"
                );
                assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                assert_eq!(
                    game.mob_object(72).unwrap().position,
                    Vec3::new(2.0, 190.0, 3.0)
                );
                let child = game.mob_object(73).unwrap();
                assert_eq!(child.extra.mount, Some(72));
                assert_eq!(child.position, Vec3::ZERO);
                assert_eq!(child.age, 19);
                assert!(game.entity(73).is_none());
                game.native_removals_flush();
                assert_eq!(
                    count.load(Ordering::Relaxed),
                    1,
                    "deferred sweep must not notify again"
                );
                assert!(game.entity(71).is_none());
                assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                assert!(game.persist.removed_vehicles.contains_key(&71));
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn nested_vehicle_discard_notifies_a_boat_dismount_once() {
        discard_during_dismount(false);
    }
    #[test]
    #[ignore = "daily"]
    fn nested_vehicle_discard_notifies_a_cart_dismount_once() {
        discard_during_dismount(true);
    }
    fn with_vehicle_tree(
        cart: bool,
        rejected: bool,
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.options.game_dir = Some(
                std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join(format!("guard-{:?}", std::thread::current().id())),
            );
            game.server.world_root = game.options.game_dir.as_ref().unwrap().join("world");
            for column in [(0, 0), (0, 1)] {
                fixture_resident_column(&mut game.world, column);
            }
            let at = Vec3::new(10.0, 200.0, 20.0);
            if cart {
                game.carts
                    .push(sunrise_gameplay_common::game::minecarts::Minecart::new(
                        71,
                        sunrise_gameplay_common::game::minecarts::CartKind::Rideable,
                        at,
                    ));
            } else {
                game.boats.push(Boat::new(71, "oak".into(), at, 0.0));
            }
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut rider = Mob::new(72, kind.clone(), Vec3::new(2.0, 190.0, 3.0), 0.0);
            rider.extra.mount = Some(71);
            if rejected {
                game.persist.off_level.insert(72, OffLevel::Mob(rider));
            } else {
                game.mobs.push(rider);
            }
            let mut child = Mob::new(73, kind, Vec3::ZERO, 0.0);
            child.extra.mount = Some(72);
            child.age = 19;
            game.mobs.push(child);
            // These manually installed owners bypass the normal allocator.
            // Destruction drops must not reuse the removed vehicle's ID.
            game.server.next_entity = game.server.next_entity.max(73);
            test(game);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removed_vehicle_resident_retry_requires_an_eligible_uncanceled_entity_tick() {
        for cart in [false, true] {
            let cancel_mount = Arc::new(AtomicBool::new(true));
            let cancel_tick = Arc::new(AtomicBool::new(true));
            let calls = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![
                mod_api::ModEntry::new(
                    "cancel_mount",
                    Box::new(CancelUntilAllowed {
                        cancel: cancel_mount.clone(),
                        calls: calls.clone(),
                    }),
                ),
                mod_api::ModEntry::new(
                    "cancel_tick",
                    Box::new(CancelPassengerTick(cancel_tick.clone())),
                ),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_vehicle_tree(cart, false, |game| {
                    game.server.settings.simulation_distance = 4;
                    game.server.settings.render_distance = 12;
                    game.player = Some(
                        sunrise_sim::motion::PlayerMotion::new(
                            game.motion,
                            Vec3::new(8.5, 200.0, 8.5),
                        )
                        .unwrap(),
                    );
                    super::super::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
                        game,
                        sunrise_core::position::ChunkPos::new(0, 0),
                    );
                    game.world
                        .chunks
                        .insert((8, 0), Arc::new(sunrise_level::chunk::Chunk::empty(8, 0)));
                    game.update_chunk_tickets();
                    assert!(game.entity_ticking_area()(2, 3));
                    assert!(!game.entity_ticking_area()(136, 3));
                    if cart {
                        game.hurt_cart(0, 5.0, false, false, CartHit::default());
                    } else {
                        assert!(game.hurt_boat(0, 5.0, false));
                    }
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                    cancel_mount.store(false, Ordering::Relaxed);
                    game.mob_object_mut(72).unwrap().position = Vec3::new(136.0, 190.0, 3.0);
                    game.tick_creature_entities().unwrap();
                    assert_eq!(
                        calls.load(Ordering::Relaxed),
                        1,
                        "non-ticking resident must not retry"
                    );
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                    game.mob_object_mut(72).unwrap().position = Vec3::new(2.0, 190.0, 3.0);
                    game.tick_creature_entities().unwrap();
                    assert_eq!(
                        calls.load(Ordering::Relaxed),
                        1,
                        "canceled Pre must suppress retry"
                    );
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                    assert!(game.persist.removed_vehicles.contains_key(&71));
                    cancel_tick.store(false, Ordering::Relaxed);
                    game.tick_creature_entities().unwrap();
                    assert_eq!(calls.load(Ordering::Relaxed), 2);
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
                    assert!(!game.persist.removed_vehicles.contains_key(&71));
                    game.tick_creature_entities().unwrap();
                    assert_eq!(
                        calls.load(Ordering::Relaxed),
                        2,
                        "completed dismount must not replay"
                    );
                });
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn removed_vehicle_is_owned_by_canceled_player_until_player_leaves() {
        for cart in [false, true] {
            let cancel = Arc::new(AtomicBool::new(true));
            let calls = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "cancel_player_dismount",
                Box::new(CancelPlayerDismount {
                    cancel: cancel.clone(),
                    calls: calls.clone(),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_vehicle_tree(cart, false, |game| {
                    fixture_resident_column(&mut game.world, (0, 1));
                    game.mobs.clear();
                    game.player = Some(
                        sunrise_sim::motion::PlayerMotion::new(
                            game.motion,
                            Vec3::new(2.0, 190.0, 3.0),
                        )
                        .unwrap(),
                    );
                    super::super::native_entity_refs::tests::bind_local_player(game, 70, 70);
                    game.riding = Some(71);
                    let vehicle = game.entity(71).unwrap();
                    if cart {
                        game.hurt_cart(0, 5.0, false, false, CartHit::default());
                    } else {
                        assert!(game.hurt_boat(0, 5.0, false));
                    }
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                    assert_eq!(game.riding, Some(71));
                    assert!(game.persist.removed_vehicles.contains_key(&71));
                    assert!(game.native_entity_target(vehicle).is_none());
                    assert_eq!(
                        game.entity_visit_passengers(vehicle, &mut |_| panic!(
                            "stale vehicle was visited"
                        )),
                        Err(mod_api::EntityReadError::Stale)
                    );
                    game.stop_riding();
                    assert_eq!(calls.load(Ordering::Relaxed), 2);
                    assert_eq!(game.riding, Some(71));
                    assert!(game.persist.removed_vehicles.contains_key(&71));
                    cancel.store(false, Ordering::Relaxed);
                    game.stop_riding();
                    assert_eq!(calls.load(Ordering::Relaxed), 3);
                    assert_eq!(game.riding, None);
                    assert!(!game.persist.removed_vehicles.contains_key(&71));
                });
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn guest_vehicle_tick_retries_canceled_removed_vehicle_without_losing_edge() {
        for cart in [false, true] {
            let cancel = Arc::new(AtomicBool::new(true));
            let calls = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "cancel_guest_dismount",
                Box::new(CancelPlayerDismount {
                    cancel: cancel.clone(),
                    calls: calls.clone(),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_vehicle_tree(cart, false, |game| {
                    game.mobs.clear();
                    sunrise_gameplay::game::native_player_death::tests::add_guest(game);
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| guest.vehicle = Some(71));
                    if cart {
                        game.hurt_cart(0, 5.0, false, false, CartHit::default());
                    } else {
                        assert!(game.hurt_boat(0, 5.0, false));
                    }
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                    game.guest_vehicle_tick(1);
                    assert_eq!(calls.load(Ordering::Relaxed), 2);
                    assert_eq!(
                        game.server.lan.as_ref().unwrap().hub.guests[0].vehicle,
                        Some(71)
                    );
                    assert!(game.persist.removed_vehicles.contains_key(&71));
                    cancel.store(false, Ordering::Relaxed);
                    game.guest_vehicle_tick(1);
                    assert_eq!(calls.load(Ordering::Relaxed), 3);
                    assert_eq!(
                        game.server.lan.as_ref().unwrap().hub.guests[0].vehicle,
                        None
                    );
                    assert!(!game.persist.removed_vehicles.contains_key(&71));
                });
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn canceled_dismount_keeps_callback_reposition_for_later_removed_vehicle_dismount() {
        for cart in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "reposition_passenger",
                Box::new(RepositionCanceledPassenger(calls.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_vehicle_tree(cart, false, |game| {
                    sunrise_gameplay::game::native_player_death::tests::add_guest(game);
                    game.build_registries();
                    game.set_allow_commands(true);
                    game.persist.uuids.insert(72, [160, 0, 0, 72]);
                    if cart {
                        game.hurt_cart(0, 5.0, false, false, CartHit::default());
                    } else {
                        assert!(game.hurt_boat(0, 5.0, false));
                    }
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                    assert_eq!(
                        game.mob_object(72).unwrap().position,
                        Vec3::new(30.5, 180.0, 40.5)
                    );
                    game.dismount_mob(72);
                    assert_eq!(calls.load(Ordering::Relaxed), 2);
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
                    assert_eq!(
                        game.mob_object(72).unwrap().position,
                        Vec3::new(30.5, 200.0, 40.5)
                    );
                    assert!(!game.persist.removed_vehicles.contains_key(&71));
                });
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn every_vehicle_removal_boundary_hides_queries_and_retires_once() {
        // Damage, burning TNT, command kill, and deferred native removal
        // share the same visibility/retention contract for both vehicle lists.
        for mode in 0..9 {
            let cart = matches!(mode, 1 | 2 | 4 | 6 | 8);
            let kind = if mode == 2 {
                "tnt_minecart"
            } else if cart {
                "minecart"
            } else {
                "oak_boat"
            };
            let cancel = Arc::new(AtomicBool::new(true));
            let mounts = Arc::new(AtomicUsize::new(0));
            let leaves = Arc::new(AtomicUsize::new(0));
            let destroys = Arc::new(AtomicUsize::new(0));
            let key_slot = VehicleKeySlot::default();
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "vehicle_boundary",
                Box::new(RemovalBoundaryObserver {
                    kind,
                    cancel: cancel.clone(),
                    mounts: mounts.clone(),
                    leaves: leaves.clone(),
                    destroys: destroys.clone(),
                    key: key_slot.clone(),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_vehicle_tree(cart, true, |game| {
                    fixture_resident_column(&mut game.world, (0, 0));
                    fixture_resident_column(&mut game.world, (0, 1));
                    let key = key_slot
                        .lock()
                        .unwrap()
                        .clone()
                        .expect("RemovalBoundaryObserver registered");
                    sunrise_gameplay::game::native_player_death::tests::add_guest(game);
                    game.native_level_opened(sunrise_gameplay_common::game::Dim::Overworld);
                    game.build_registries();
                    game.set_allow_commands(true);
                    if mode == 2 {
                        game.carts.update(0, |cart| {
                            cart.kind = sunrise_gameplay_common::game::minecarts::CartKind::Tnt
                        });
                    }
                    set_vehicle_test_attachment(game, &key);
                    let vehicle = game.entity(71).unwrap();
                    let uuid = [160, 0, 0, 71];
                    let holder = sunrise_sim::save::uuid_text(uuid);
                    game.persist.uuids.insert(71, uuid);
                    game.score.board.set(&holder, "boundary", 7);
                    assert!(game.native_joins_flush(false, 0));
                    match mode {
                        0 => {
                            assert!(game.hurt_boat(0, 5.0, false));
                        }
                        1 => game.hurt_cart(0, 5.0, false, false, CartHit::default()),
                        2 => {
                            game.arrow_hits_cart(0, 1.0, Vec3::ZERO, true, false, None, None);
                        }
                        3 | 4 => {
                            assert!(game.run_command(&format!("kill @e[type=minecraft:{kind}]")));
                        }
                        5 | 6 => {
                            assert!(vehicle.discard(game));
                        }
                        _ => {
                            assert!(game.entity_remove(vehicle, mod_api::RemovalReason::Killed));
                        }
                    }
                    assert_vehicle_queries_hidden(game, kind, &key);
                    assert!(game.entity_position(vehicle).is_none());
                    assert_eq!(
                        game.entity_visit_passengers(vehicle, &mut |_| panic!(
                            "stale vehicle was visited"
                        )),
                        Err(mod_api::EntityReadError::Stale)
                    );
                    assert!(game.vehicle_by_uuid(uuid).is_none());
                    if !cart {
                        assert!(game.boat_by_uuid(uuid).is_none());
                    }
                    assert!(!vehicle.discard(game));
                    assert!(game.score.board.score(&holder, "boundary").is_none());
                    game.score.board.set(&holder, "boundary", 11);
                    for _ in 0..3 {
                        game.native_removals_flush();
                        game.native_leaves_flush();
                        assert_vehicle_queries_hidden(game, kind, &key);
                        assert_eq!(mounts.load(Ordering::Relaxed), 1);
                        assert_eq!(leaves.load(Ordering::Relaxed), 1);
                        assert_eq!(destroys.load(Ordering::Relaxed), usize::from(mode < 2));
                        assert!(game.persist.removed_vehicles.contains_key(&71));
                        assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                        assert!(game.persist.mount_notifications.is_empty());
                        game.prune_entity_memory();
                        assert_eq!(
                            game.score
                                .board
                                .score(&holder, "boundary")
                                .map(|score| score.value),
                            Some(11)
                        );
                    }
                    assert_eq!(
                        game.score
                            .board
                            .score(&holder, "boundary")
                            .map(|score| score.value),
                        Some(11)
                    );
                    game.score_destroyed_entities();
                    assert_eq!(
                        game.score
                            .board
                            .score(&holder, "boundary")
                            .map(|score| score.value),
                        Some(11)
                    );
                    cancel.store(false, Ordering::Relaxed);
                    game.dismount_mob(72);
                    assert_eq!(mounts.load(Ordering::Relaxed), 2);
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
                    assert!(!game.persist.removed_vehicles.contains_key(&71));
                    assert!(!game.persist.uuids.contains_key(&71));
                    assert!(!game.entity_attachments.contains_key(&71));
                    game.native_removals_flush();
                    game.dismount_mob(72);
                    assert_vehicle_queries_hidden(game, kind, &key);
                    assert_eq!(mounts.load(Ordering::Relaxed), 2);
                    assert_eq!(leaves.load(Ordering::Relaxed), 1);
                    assert_eq!(destroys.load(Ordering::Relaxed), usize::from(mode < 2));
                    assert_eq!(
                        game.score
                            .board
                            .score(&holder, "boundary")
                            .map(|score| score.value),
                        Some(11)
                    );
                });
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn retained_destroyed_vehicle_spills_contents_exactly_once() {
        for cart in [false, true] {
            for mode in 0..3 {
                let cancel = Arc::new(AtomicBool::new(true));
                let calls = Arc::new(AtomicUsize::new(0));
                let joins = Arc::new(SpillJoins::default());
                let mods: mod_api::Mods = vec![
                    mod_api::ModEntry::new(
                        "cancel_spilling_vehicle",
                        Box::new(CancelUntilAllowed {
                            cancel: cancel.clone(),
                            calls: calls.clone(),
                        }),
                    ),
                    mod_api::ModEntry::new(
                        "observe_spill",
                        Box::new(ObserveSpillItem(joins.clone())),
                    ),
                ]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    with_vehicle_tree(cart, true, |game| {
                        fixture_resident_column(&mut game.world, (0, 1));
                        game.server.next_entity = game.server.next_entity.max(100);
                        let items = vec![Some(sunrise_items::items::ItemStack::new(
                            "minecraft:diamond",
                            5,
                        ))];
                        if cart {
                            game.carts.update(0, |vehicle| {
                                vehicle.kind =
                                    sunrise_gameplay_common::game::minecarts::CartKind::Chest;
                                vehicle.items = items;
                            });
                        } else {
                            game.boats.update(0, |vehicle| {
                                vehicle.chest = true;
                                vehicle.items = items;
                            });
                        }
                        game.native_joins_flush(false, 0);
                        assert_eq!(joins.vehicle.load(Ordering::Relaxed), 1);
                        match mode {
                            0 if cart => game.hurt_cart(0, 5.0, false, false, CartHit::default()),
                            0 => {
                                assert!(game.hurt_boat(0, 5.0, false));
                            }
                            1 => {
                                assert!(game.kill_target_in_level(if cart {
                                    sunrise_gameplay_common::game::commands_common::Target::Cart(71)
                                } else {
                                    sunrise_gameplay_common::game::commands_common::Target::Boat(71)
                                }));
                            }
                            _ => {
                                assert!(game.entity(71).unwrap().discard(game));
                            }
                        }
                        game.native_removals_flush();
                        let diamonds = |game: &sunrise_gameplay::game::ServerGame<'_>| -> u32 {
                            game.dropped
                                .iter()
                                .filter(|item| item.stack.item.as_ref() == "minecraft:diamond")
                                .map(|item| item.stack.count)
                                .sum()
                        };
                        assert_eq!(diamonds(game), 5);
                        assert!(joins.items.load(Ordering::Relaxed) > 0);
                        assert_eq!(calls.load(Ordering::Relaxed), 1);
                        assert!(game.persist.removed_vehicles.contains_key(&71));
                        for _ in 0..2 {
                            game.native_removals_flush();
                            game.prune_entity_memory();
                            assert_eq!(diamonds(game), 5);
                        }
                        cancel.store(false, Ordering::Relaxed);
                        game.dismount_mob(72);
                        assert!(!game.persist.removed_vehicles.contains_key(&71));
                        game.native_removals_flush();
                        assert_eq!(diamonds(game), 5);
                        assert_eq!(calls.load(Ordering::Relaxed), 2);
                    });
                });
            }
        }
    }
    #[test]
    #[ignore = "daily"]
    fn destroyed_vehicle_is_already_invisible_inside_dismount_callback() {
        for cart in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let kind = if cart { "minecart" } else { "oak_boat" };
            let key_slot = VehicleKeySlot::default();
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "vehicle_boundary",
                Box::new(ObserveDestroyedVehicle {
                    kind,
                    calls: calls.clone(),
                    key: key_slot.clone(),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_vehicle_tree(cart, false, |game| {
                    let key = key_slot
                        .lock()
                        .unwrap()
                        .clone()
                        .expect("ObserveDestroyedVehicle registered");
                    sunrise_gameplay::game::native_player_death::tests::add_guest(game);
                    game.native_level_opened(sunrise_gameplay_common::game::Dim::Overworld);
                    game.build_registries();
                    game.set_allow_commands(true);
                    set_vehicle_test_attachment(game, &key);
                    let batch = mod_api::AttachmentQueries::capture(game, &key).unwrap();
                    let query = batch.query(&key).unwrap();
                    let mut rows = query.iter();
                    assert!(rows.try_next().unwrap().is_some());
                    assert!(rows.try_next().unwrap().is_none());
                    if cart {
                        game.hurt_cart(0, 5.0, false, false, CartHit::default());
                    } else {
                        assert!(game.hurt_boat(0, 5.0, false));
                    }
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                });
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn retained_vehicle_is_not_visible_to_selectors_or_attachment_queries() {
        for cart in [false, true] {
            let cancel = Arc::new(AtomicBool::new(true));
            let calls = Arc::new(AtomicUsize::new(0));
            let key_slot = VehicleKeySlot::default();
            let mods: mod_api::Mods = vec![
                mod_api::ModEntry::new(
                    "cancel_visibility",
                    Box::new(CancelUntilAllowed { cancel, calls }),
                ),
                mod_api::ModEntry::new(
                    "vehicle_boundary",
                    Box::new(VehicleValueMod(key_slot.clone())),
                ),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_vehicle_tree(cart, false, |game| {
                    let key = key_slot
                        .lock()
                        .unwrap()
                        .clone()
                        .expect("VehicleValueMod registered");
                    game.native_level_opened(sunrise_gameplay_common::game::Dim::Overworld);
                    game.build_registries();
                    game.set_allow_commands(true);
                    super::super::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
                        game,
                        sunrise_core::position::ChunkPos::new(0, 0),
                    );
                    let kind = if cart { "minecart" } else { "oak_boat" };
                    let command = format!("execute if entity @e[type=minecraft:{kind}]");
                    assert!(game.run_command(&command));
                    set_vehicle_test_attachment(game, &key);
                    let batch = mod_api::AttachmentQueries::capture(game, &key).unwrap();
                    assert!(
                        batch
                            .query(&key)
                            .unwrap()
                            .iter()
                            .try_next()
                            .unwrap()
                            .is_some()
                    );
                    let vehicle = game.entity(71).unwrap();
                    if cart {
                        game.hurt_cart(0, 5.0, false, false, CartHit::default());
                    } else {
                        assert!(game.hurt_boat(0, 5.0, false));
                    }
                    assert!(game.persist.removed_vehicles.contains_key(&71));
                    assert!(game.entity(71).is_none());
                    assert!(game.native_entity_target(vehicle).is_none());
                    assert_eq!(
                        game.entity_visit_passengers(vehicle, &mut |_| panic!(
                            "stale vehicle was visited"
                        )),
                        Err(mod_api::EntityReadError::Stale)
                    );
                    assert!(!game.run_command(&command));
                    let batch = mod_api::AttachmentQueries::capture(game, &key).unwrap();
                    assert!(
                        batch
                            .query(&key)
                            .unwrap()
                            .iter()
                            .try_next()
                            .unwrap()
                            .is_none()
                    );
                });
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn destroyed_vehicles_preserve_canceled_rejected_edges_until_explicit_dismount() {
        for mode in 0..3 {
            let cancel = Arc::new(AtomicBool::new(true));
            let calls = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "cancel_destroyed_vehicle",
                Box::new(CancelUntilAllowed {
                    cancel: cancel.clone(),
                    calls: calls.clone(),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_vehicle_tree(mode != 0, true, |game| {
                    let uuid = [160, 0, 0, 71];
                    let holder = sunrise_sim::save::uuid_text(uuid);
                    game.persist.uuids.insert(71, uuid);
                    game.score.board.set(&holder, "retained", 7);
                    if mode == 0 {
                        assert!(game.hurt_boat(0, 5.0, false));
                    } else if mode == 1 {
                        game.hurt_cart(0, 5.0, false, false, CartHit::default());
                    } else {
                        game.carts.update(0, |cart| {
                            cart.kind = sunrise_gameplay_common::game::minecarts::CartKind::Tnt
                        });
                        game.arrow_hits_cart(0, 1.0, Vec3::ZERO, true, false, None, None);
                    }
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                    assert!(game.entity(71).is_none());
                    assert!(game.entity(72).is_none());
                    assert!(game.entity(73).is_some());
                    assert!(game.persist.removed_vehicles.contains_key(&71));
                    assert!(game.score.board.score(&holder, "retained").is_none());
                    game.score.board.set(&holder, "retained", 11);
                    game.score_destroyed_entities();
                    assert_eq!(
                        game.score.board.score(&holder, "retained").map(|s| s.value),
                        Some(11)
                    );
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                    assert_eq!(game.mob_object(73).unwrap().position, Vec3::ZERO);
                    game.sync_riders();
                    assert_eq!(
                        calls.load(Ordering::Relaxed),
                        1,
                        "rejected riders do not tick"
                    );
                    assert_eq!(game.mob_object(73).unwrap().age, 19);
                    cancel.store(false, Ordering::Relaxed);
                    game.dismount_mob(72);
                    assert_eq!(calls.load(Ordering::Relaxed), 2);
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
                    assert_eq!(
                        game.mob_object(72).unwrap().position,
                        Vec3::new(2.0, 200.0, 3.0)
                    );
                    let child = game.mob_object(73).unwrap();
                    assert_eq!(child.extra.mount, Some(72));
                    assert_eq!(child.position.x, 2.0);
                    assert_eq!(child.position.z, 3.0);
                    assert!((child.position.y - 200.86875).abs() < 1e-6);
                    assert!(!game.persist.removed_vehicles.contains_key(&71));
                });
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn removed_vehicle_dismount_snaps_each_recursive_multiseat_passenger() {
        for cart in [false, true] {
            let cancel = Arc::new(AtomicBool::new(true));
            let calls = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "cancel_multiseat",
                Box::new(CancelUntilAllowed {
                    cancel: cancel.clone(),
                    calls,
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_vehicle_tree(cart, true, |game| {
                    let camel = game.res.mob_types().get("minecraft:camel").unwrap().clone();
                    let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    let mut parent = Mob::new(72, camel, Vec3::new(2.0, 190.0, 3.0), 0.0);
                    parent.extra.mount = Some(71);
                    game.persist.off_level.insert(72, OffLevel::Mob(parent));
                    game.mob_object_mut(73).unwrap().extra.vehicle_order = 1;
                    let mut front = Mob::new(74, pig.clone(), Vec3::ZERO, 0.0);
                    front.extra.mount = Some(72);
                    front.extra.vehicle_order = 0;
                    game.mobs.push(front);
                    let mut grandchild = Mob::new(75, pig, Vec3::ZERO, 0.0);
                    grandchild.extra.mount = Some(73);
                    game.mobs.push(grandchild);
                    if cart {
                        game.hurt_cart(0, 5.0, false, false, CartHit::default());
                    } else {
                        assert!(game.hurt_boat(0, 5.0, false));
                    }
                    cancel.store(false, Ordering::Relaxed);
                    game.dismount_mob(72);
                    let parent = game.mob_object(72).unwrap();
                    let front = game.mob_object(74).unwrap();
                    let back = game.mob_object(73).unwrap();
                    let grandchild = game.mob_object(75).unwrap();
                    assert!((front.position.z - parent.position.z - 0.7).abs() < 1e-6);
                    assert!((back.position.z - parent.position.z + 0.5).abs() < 1e-6);
                    assert_eq!(grandchild.position.x, back.position.x);
                    assert_eq!(grandchild.position.z, back.position.z);
                    assert!((grandchild.position.y - back.position.y - 0.86875).abs() < 1e-6);
                    assert!(!game.persist.removed_vehicles.contains_key(&71));
                });
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn resident_passenger_can_later_leave_removed_vehicle_via_native_handle() {
        for cart in [false, true] {
            let cancel = Arc::new(AtomicBool::new(true));
            let calls = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "cancel_resident",
                Box::new(CancelUntilAllowed {
                    cancel: cancel.clone(),
                    calls: calls.clone(),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_vehicle_tree(cart, false, |game| {
                    let vehicle = game.entity(71).unwrap();
                    let rider = game.entity(72).unwrap();
                    if cart {
                        game.hurt_cart(0, 5.0, false, false, CartHit::default());
                    } else {
                        assert!(game.hurt_boat(0, 5.0, false));
                    }
                    assert!(game.native_entity_target(vehicle).is_none());
                    assert_eq!(game.entity_is_passenger(rider), Some(true));
                    assert_eq!(game.entity_vehicle(rider), None);
                    cancel.store(false, Ordering::Relaxed);
                    assert!(game.entity_stop_riding(rider));
                    assert_eq!(calls.load(Ordering::Relaxed), 2);
                    assert_eq!(
                        game.mob_object(72).unwrap().position,
                        Vec3::new(2.0, 200.0, 3.0)
                    );
                    assert!(!game.persist.removed_vehicles.contains_key(&71));
                });
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn kill_and_discard_notify_rejected_edges_once_without_changing_membership() {
        for cart in [false, true] {
            for discard in [false, true] {
                let count = Arc::new(AtomicUsize::new(0));
                let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                    "cancel_cleanup",
                    Box::new(CancelLive {
                        count: count.clone(),
                        cross_level: false,
                    }),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    with_vehicle_tree(cart, true, |game| {
                        if discard {
                            assert!(game.entity(71).unwrap().discard(game));
                        } else {
                            game.kill_target_in_level(if cart {
                                sunrise_gameplay_common::game::commands_common::Target::Cart(71)
                            } else {
                                sunrise_gameplay_common::game::commands_common::Target::Boat(71)
                            });
                        }
                        game.native_removals_flush();
                        assert_eq!(count.load(Ordering::Relaxed), 1);
                        assert!(game.entity(71).is_none());
                        assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                        assert_eq!(
                            game.mob_object(72).unwrap().position,
                            Vec3::new(2.0, 190.0, 3.0)
                        );
                        assert!(game.persist.removed_vehicles.contains_key(&71));
                        assert!(game.entity(72).is_none());
                        assert!(game.persist.off_level.contains_key(&72));
                        let child = game.mob_object(73).unwrap();
                        assert_eq!(child.extra.mount, Some(72));
                        assert_eq!(child.position, Vec3::ZERO);
                        assert_eq!(child.age, 19);
                        assert!(game.entity(73).is_some());
                        assert!(game.persist.mount_notifications.is_empty());
                        game.native_removals_flush();
                        assert_eq!(count.load(Ordering::Relaxed), 1);
                    });
                });
            }
        }
    }
    #[test]
    #[ignore = "daily"]
    fn destruction_preserves_later_rider_reparenting_without_an_extra_notification() {
        for mode in 0..5 {
            let cart = mode >= 2;
            let count = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "reparent_cleanup",
                Box::new(ReparentLater(count.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_vehicle_tree(cart, false, |game| {
                    fixture_resident_column(&mut game.world, (0, 0));
                    fixture_resident_column(&mut game.world, (0, 1));
                    game.mob_object_mut(72).unwrap().extra.vehicle_order = 1;
                    let later = game.mob_object_mut(73).unwrap();
                    later.extra.mount = Some(71);
                    later.extra.vehicle_order = 2;
                    let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                    game.mobs.push(Mob::new(74, kind, Vec3::ZERO, 0.0));
                    match mode {
                        0 => {
                            assert!(game.kill_target_in_level(
                                sunrise_gameplay_common::game::commands_common::Target::Boat(71),
                            ));
                        }
                        1 => {
                            assert!(game.hurt_boat(0, 5.0, false));
                        }
                        2 => {
                            assert!(game.kill_target_in_level(
                                sunrise_gameplay_common::game::commands_common::Target::Cart(71),
                            ));
                        }
                        3 => game.hurt_cart(0, 5.0, false, false, CartHit::default()),
                        _ => {
                            game.carts.update(0, |cart| {
                                cart.kind = sunrise_gameplay_common::game::minecarts::CartKind::Tnt
                            });
                            game.arrow_hits_cart(0, 1.0, Vec3::ZERO, true, false, None, None);
                        }
                    }
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                    assert_eq!(game.mob_object(73).unwrap().extra.mount, Some(74));
                    assert_eq!(
                        count.load(Ordering::Relaxed),
                        1,
                        "only deliberate reparenting dismounts the later rider"
                    );
                    game.native_removals_flush();
                    assert_eq!(game.mob_object(73).unwrap().extra.mount, Some(74));
                    assert_eq!(count.load(Ordering::Relaxed), 1);
                    assert!(game.persist.mount_notifications.is_empty());
                });
            });
        }
    }
    fn live_cancels(cross_level: bool) {
        let count = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "live_cancel",
            Box::new(CancelLive {
                count: count.clone(),
                cross_level,
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_vehicle_tree(false, false, |game| {
                if cross_level {
                    // The console service requires an active server hub.
                    sunrise_gameplay::game::native_player_death::tests::add_guest(game);
                    game.build_registries();
                    game.set_allow_commands(true);
                    assert!(game.run_command("worldborder set 1000"));
                }
                let rider = game.entity(72).unwrap();
                for n in 1..=2 {
                    assert!(!game.entity_stop_riding(rider));
                    assert_eq!(count.load(Ordering::Relaxed), n);
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                    assert_eq!(
                        game.mob_object(72).unwrap().position,
                        Vec3::new(2.0, 190.0, 3.0)
                    );
                    assert!(game.persist.mount_notifications.is_empty());
                    assert_eq!(game.dim, sunrise_gameplay_common::game::Dim::Overworld);
                }
                if cross_level {
                    assert_eq!(game.border.size(), 1000.0);
                    assert_eq!(
                        game.server
                            .levels
                            .0
                            .parked_border(sunrise_gameplay_common::game::Dim::Nether)
                            .unwrap()
                            .size(),
                        38.0
                    );
                    game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                        .unwrap();
                    assert!(game.persist.mount_notifications.is_empty());
                    game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                        .unwrap();
                    assert!(game.persist.mount_notifications.is_empty());
                }
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn independent_live_cancellations_both_notify_and_retain_the_relationship() {
        live_cancels(false);
    }
    #[test]
    #[ignore = "daily"]
    fn scoped_cross_level_callbacks_restore_context_and_clear_notification_keys() {
        live_cancels(true);
    }
    #[test]
    #[ignore = "daily"]
    fn review_cycle_through_retained_object_is_refused() {
        with_vehicle_tree(false, true, |game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(74, kind, Vec3::ZERO, 0.0));
            game.mob_object_mut(72).unwrap().extra.mount = Some(74);
            assert!(
                !game.start_riding_target(
                    sunrise_gameplay_common::game::commands_common::Target::Mob(74),
                    sunrise_gameplay_common::game::commands_common::Target::Mob(73),
                    true
                ),
                "cycle through retained object must be rejected"
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn dismount_preserves_its_own_riders_callback_replacement() {
        let count = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "reparent_self",
            Box::new(ReparentSelf(count.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_vehicle_tree(false, false, |game| {
                let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                game.mobs.push(Mob::new(74, kind, Vec3::ZERO, 0.0));
                game.dismount_mob(72);
                assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(74));
                assert_eq!(game.mob_passenger_ids(74), vec![72]);
                assert!(
                    !game
                        .seated_passengers(71)
                        .iter()
                        .any(|(_, id, _)| *id == 72)
                );
                assert_eq!(count.load(Ordering::Relaxed), 1);
                assert!(game.persist.mount_notifications.is_empty());
            })
        });
    }
    #[test]
    #[ignore = "daily"]
    fn review_mount_callback_must_not_link_removed_vehicle() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "review",
            Box::new(ReviewKillOnMount),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_vehicle_tree(false, false, |game| {
                game.mob_object_mut(72).unwrap().extra.mount = None;
                let ok = game.start_riding_target(
                    sunrise_gameplay_common::game::commands_common::Target::Mob(72),
                    sunrise_gameplay_common::game::commands_common::Target::Boat(71),
                    true,
                );
                assert!(!ok, "mount must refuse when callback removes vehicle");
                assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
            })
        });
    }
}

#[cfg(test)]
pub(crate) mod content_seat_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn content_seats_keep_independent_offsets() {
        super::super::native_entity_refs::tests::with_game(|game| {
            use std::sync::Arc;
            let mut kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            Arc::make_mut(&mut kind).passenger_attachments =
                vec![[1.0, 2.0, 3.0], [-1.0, 4.0, 5.0]];
            let vehicle = sunrise_sim::mobs::Mob::new(1, kind, Vec3::ZERO, 0.0);
            assert_eq!(
                riders::passenger_attachment_at(&vehicle, 0),
                [1.0, 2.0, 3.0]
            );
            assert_eq!(
                riders::passenger_attachment_at(&vehicle, 1),
                [-1.0, 4.0, 5.0]
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod trap_scatter_tests {

    use super::*;
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay::game::server_side::passengers::trap_horse_scatter;

    /// `SkeletonTrapGoal.tick`: springing a skeleton trap draws the three
    /// additional horses' horizontal velocity scatter from the original trap
    /// horse's own `random` after each rider is created.
    #[test]
    #[ignore = "daily"]
    fn spring_skeleton_trap_draws_scatter_from_trap_horse_after_each_rider() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        let mut resources = None;
        let mut game = super::super::game_tests::tests::server_game(&mut resources);
        // Mounting resolves epoch-scoped resident handles, just as in an open world.
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let at = [8.5, 64.0, 8.5];
        let trap_id = game
            .spawn_mob_raw("minecraft:skeleton_horse", at, false)
            .unwrap();
        let mut expected_horse_rng = LegacyRandom::new(777);
        if let Some(horse) = game.mobs.iter_mut().find(|m| m.id == trap_id) {
            horse.extra.trap = true;
            horse.random = expected_horse_rng.clone();
        }
        game.spring_skeleton_trap(trap_id);

        let expected_pushes: Vec<(f64, f64)> = (0..3)
            .map(|_| trap_horse_scatter(&mut expected_horse_rng))
            .collect();
        let horse = game.mobs.iter_mut().find(|m| m.id == trap_id).unwrap();
        assert!(!horse.extra.trap);
        assert_eq!(horse.random.next_long(), expected_horse_rng.next_long());

        let extra_horses: Vec<&sunrise_sim::mobs::Mob> = game
            .mobs
            .iter()
            .filter(|m| m.species() == Species::SkeletonHorse && m.id != trap_id)
            .collect();
        assert_eq!(extra_horses.len(), 3);
        for (h, (dx, dz)) in extra_horses.iter().zip(expected_pushes) {
            assert_eq!((h.velocity.x, h.velocity.z), (dx, dz));
            assert!(h.extra.rider.is_some());
        }
    }
    /// `SkeletonTrapGoal.enchant` draws from `skeleton.getRandom()`, and an
    /// empty slot draws nothing.
    #[test]
    #[ignore = "daily"]
    fn enchant_item_on_an_empty_slot_draws_nothing() {
        let mut resources = None;
        let mut game = super::super::game_tests::tests::server_game(&mut resources);
        let id = game
            .spawn_mob_raw("minecraft:skeleton_horse", [8.5, 64.0, 8.5], false)
            .unwrap();
        let seeded = || LegacyRandom::new(5);
        game.mobs.iter_mut().find(|m| m.id == id).unwrap().random = seeded();
        game.enchant_item(id, sunrise_sim::mobs::finalize::MAINHAND, 0.5);
        let m = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
        assert_eq!(m.random.next_long(), seeded().next_long());
    }
}

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;
