#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_core::position::{BlockPos, ChunkPos};
use sunrise_gameplay_common::game::Dim;
#[cfg(test)]
#[path = "tests/portal_spawn_tests.rs"]
pub(crate) mod portal_spawn_tests;

#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay::game::server_side::nether_portal::tests::portal_cooldown_keeps_the_process;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

    use sunrise_gameplay::game::server_side::server_travel::TravelCause;

    use super::*;
    use sunrise_gameplay::game::native_entity_refs::tests::{
        prepare_entity_ticking_neighborhood, with_resident_game,
    };
    use sunrise_gameplay::game::server_side::nether_portal::{
        Plan, TransferredArrivalState, arrival_from_tag,
    };
    use sunrise_sim::nether_portal::ENTITY_DELAY;

    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    };

    #[test]
    #[ignore = "daily"]
    fn portal_capture_preserves_rejected_objects_and_reverses_admission() {
        with_gate(|game, _, _| {
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
            game.options.game_dir = Some(
                std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join("portal-tree"),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            assert!(game.run_command(r#"summon minecraft:pig 0 64 0 {UUID:[I;0,0,0,175],Tags:["portal_root"],Passengers:[{id:"minecraft:pig",UUID:[I;0,0,0,175],Passengers:[{id:"minecraft:pig",UUID:[I;0,0,0,176]}]}]}"#));
            let root = game
                .mobs
                .iter()
                .find(|m| {
                    game.persist_raw(m.id)
                        .is_some_and(|t| sunrise_sim::entity_nbt::uuid(t) == Some([0, 0, 0, 175]))
                })
                .unwrap()
                .id;
            let child = game.mob_passenger_ids(root)[0];
            let leaf = game.mob_passenger_ids(child)[0];
            assert!(!game.mobs.iter().any(|m| m.id == child));
            game.server.tickets.0.add_portal((0, 0));
            game.update_chunk_tickets();
            assert!(game.entity_ticking_area()(0, 0));
            game.set_portal_cooldown(leaf);
            let cooldown = game.server.portals.entities[&leaf].cooldown;
            game.handle_entity_portals();
            assert_eq!(game.server.portals.entities[&leaf].cooldown, cooldown);
            game.send_through_portal(root, BlockPos::new(0, 64, 0));
            assert!(game.mob_object(root).is_none());
            assert!(game.mob_object(child).is_none());
            assert!(game.mob_object(leaf).is_none());
            // Land the queued payload through the real arrival transform/adoption path.
            game.server.portals.arrivals[0].plan = Plan::end_portal(
                Dim::Overworld,
                Dim::Overworld,
                BlockPos::new(0, 64, 0),
                0.9,
                0.9,
            );
            assert_eq!(game.dim, Dim::Overworld);
            game.enter_level(Dim::End).unwrap();
            assert_eq!(game.dim, Dim::End);
            let target = game.server.portals.arrivals[0].plan.approximate;
            for z in (target.z >> 4) - 1..=(target.z >> 4) + 1 {
                for x in (target.x >> 4) - 1..=(target.x >> 4) + 1 {
                    game.server.streaming.insert_remote(
                        &mut game.common.world,
                        sunrise_level::chunk::Chunk::empty(x, z),
                    );
                }
            }
            game.resolve_arrivals();
            assert!(game.server.portals.arrivals.is_empty());
            let new_root = game
                .mob_objects()
                .find(|m| m.extra.mount.is_none())
                .unwrap()
                .id;
            let new_child = game.mob_passenger_ids(new_root)[0];
            let new_leaf = game.mob_passenger_ids(new_child)[0];
            assert!(!game.mobs.iter().any(|m| m.id == new_root));
            assert!(game.mobs.iter().any(|m| m.id == new_child));
            assert!(game.mobs_below_off_level().contains(&new_child));
            assert!(game.mobs_below_off_level().contains(&new_leaf));
            assert_eq!(
                game.server.portals.entities[&new_root].cooldown,
                ENTITY_DELAY
            );
            assert!(
                game.persist_raw(new_root)
                    .unwrap()
                    .get("sunrise:fresh_add")
                    .is_none()
            );
        });
    }
    #[cfg(test)]
    /// Cancels the travel of the entity whose id `spare` holds, and logs each
    /// (entity, dimension, cause) it is asked.
    struct Gate {
        spare: Arc<AtomicU32>,
        heard: Arc<Mutex<Vec<(u32, String, String)>>>,
        discard: bool,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Gate {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            e: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if self.spare.load(Ordering::Relaxed) == 1004
                && !e.mounting
                && e.entity == 102
                && let Some(sibling) = host.entity(103)
            {
                assert!(sibling.discard(host));
            }
            mod_api::Flow::Continue
        }

        fn entity_travel_to_dimension(
            &self,
            host: &mut dyn mod_api::ServerHost,
            e: &mut mod_api::EntityTravelToDimensionEvent,
        ) -> mod_api::Flow {
            self.heard.lock().unwrap().push((
                e.entity.id(),
                e.dimension.to_string(),
                e.cause.to_string(),
            ));
            let attack = self.spare.load(Ordering::Relaxed);
            if attack == 1001
                && e.entity.id() == 102
                && self
                    .heard
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|e| e.0 == 102)
                    .count()
                    == 1
            {
                let root = mod_api::EntityRef::new(e.entity.epoch(), 101);
                assert!(root.start_riding(host, e.entity, true).is_ok());
            }
            if attack == 1002 && e.entity.id() == 102 {
                assert!(mod_api::EntityRef::new(e.entity.epoch(), 103).discard(host));
            }
            if self.discard {
                assert!(e.entity.discard(host));
                return mod_api::Flow::Continue;
            }
            if e.entity.id() == self.spare.load(Ordering::Relaxed) {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    /// Runs `test` with a `Gate` installed, on a server with guests 12 and 13.
    fn with_gate(
        test: impl FnOnce(
            &mut sunrise_gameplay::game::ServerGame<'_>,
            &AtomicU32,
            &Mutex<Vec<(u32, String, String)>>,
        ),
    ) {
        with_gate_mode(false, test);
    }
    fn with_gate_mode(
        discard: bool,
        test: impl FnOnce(
            &mut sunrise_gameplay::game::ServerGame<'_>,
            &AtomicU32,
            &Mutex<Vec<(u32, String, String)>>,
        ),
    ) {
        let spare = Arc::new(AtomicU32::new(0));
        let heard = Arc::default();
        let mods = vec![mod_api::ModEntry::new(
            "gate",
            Box::new(Gate {
                spare: Arc::clone(&spare),
                heard: Arc::clone(&heard),
                discard,
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                test(game, &spare, &heard);
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_follows_the_class_then_the_first_passenger() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            game.boats
                .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                    101,
                    Arc::from("oak"),
                    Vec3::ZERO,
                    0.0,
                ));
            game.carts
                .push(sunrise_gameplay_common::game::minecarts::Minecart::new(
                    103,
                    sunrise_gameplay_common::game::minecarts::CartKind::Rideable,
                    Vec3::ZERO,
                ));
            let mob = |name: &str, id| {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                sunrise_sim::mobs::Mob::new(id, kind, Vec3::ZERO, 0.0)
            };
            let (cow, pig) = (mob("minecraft:cow", 102), mob("minecraft:pig", 104));
            game.mobs.push(cow);
            game.mobs.push(pig);
            assert_eq!(game.dimension_changing_delay(12), 10);
            let mut projectile = sunrise_sim::projectiles::Projectile::new(
                sunrise_sim::projectiles::ProjectileKind::Snowball,
                Vec3::ZERO,
                None,
            );
            projectile.id = 106;
            game.creatures.projectiles.push(projectile);
            assert_eq!(game.dimension_changing_delay(106), 2);
            let entry = BlockPos::new(0, 64, 0);
            // Boats and minecarts (`VehicleEntity`) are 10 whoever rides them;
            // a mob vehicle follows its first passenger. Riders: none, mob
            // only, player first, player then mob, then mob first.
            for (vehicle, by_class) in [(101, true), (103, true), (104, false)] {
                for (player, mob_order, by_rider) in [
                    (false, None, 300),
                    (false, Some(20), 300),
                    (true, None, 10),
                    (true, Some(20), 10),
                    (true, Some(-10), 300),
                ] {
                    let expected = if by_class { 10 } else { by_rider };
                    game.server.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                        g.vehicle = player.then_some(vehicle);
                        g.vehicle_order = 0;
                    });
                    game.mobs[0].extra.mount = mob_order.map(|_| vehicle);
                    game.mobs[0].extra.vehicle_order = mob_order.unwrap_or_default();
                    assert_eq!(
                        game.dimension_changing_delay(vehicle),
                        expected,
                        "vehicle {vehicle}, player {player}, mob {mob_order:?}"
                    );
                    portal_cooldown_keeps_the_process(game, vehicle, entry, expected);
                }
            }
        });
    }
    /// `EntityTravelToDimensionEvent` for server players: both are asked with
    /// their own id, the target level and the cause; the spared one stays, the
    /// other may go.
    #[test]
    #[ignore = "daily"]
    fn a_mod_cancels_one_players_travel_and_allows_the_other() {
        with_gate(|game, spare, heard| {
            // Guests 12 and 13 are entities 12 and 13.
            spare.store(12, Ordering::Relaxed);
            assert!(!game.server_player_travel_allowed(12, Dim::Nether, TravelCause::NetherPortal));
            assert!(game.server_player_travel_allowed(13, Dim::End, TravelCause::EndPortal));
            assert_eq!(
                *heard.lock().unwrap(),
                [
                    (
                        12,
                        "minecraft:the_nether".to_owned(),
                        "nether_portal".to_owned()
                    ),
                    (13, "minecraft:the_end".to_owned(), "end_portal".to_owned()),
                ]
            );
        });
    }
    /// The same for two dropped items at a Nether portal: the spared one is
    /// still in the level and waits for no arrival, the other has left it for
    /// one.
    #[test]
    #[ignore = "daily"]
    fn a_mod_cancels_one_items_portal_trip_and_allows_the_other() {
        with_gate(|game, spare, heard| {
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            let stone = || sunrise_items::items::ItemStack::new("minecraft:stone", 1);
            game.spawn_item(stone(), [0.5, 64.0, 0.5], [0.0; 3], 0);
            game.spawn_item(stone(), [2.5, 64.0, 0.5], [0.0; 3], 0);
            let ids: Vec<u32> = game.dropped.iter().map(|d| d.id).collect();
            assert_eq!(ids.len(), 2);
            spare.store(ids[0], Ordering::Relaxed);
            for id in &ids {
                game.send_through_portal(*id, BlockPos::new(0, 64, 0));
            }
            let left: Vec<u32> = game.dropped.iter().map(|d| d.id).collect();
            assert_eq!(left, [ids[0]]);
            assert_eq!(game.server.portals.arrivals.len(), 1);
            assert_eq!(game.server.portals.arrivals[0].plan.dim, Dim::Nether);
            assert_eq!(
                game.server.portals.arrivals[0].tag.get("PortalCooldown"),
                Some(&Tag::Int(300))
            );
            let nether = || "minecraft:the_nether".to_owned();
            let cause = || "nether_portal".to_owned();
            assert_eq!(
                *heard.lock().unwrap(),
                [(ids[0], nether(), cause()), (ids[1], nether(), cause())]
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn discarded_item_does_not_travel() {
        with_gate_mode(true, |game, _, heard| {
            game.spawn_item(
                sunrise_items::items::ItemStack::new("minecraft:stone", 1),
                [0.5, 64.0, 0.5],
                [0.0; 3],
                0,
            );
            let id = game.dropped[0].id;
            game.send_through_portal(id, BlockPos::new(0, 64, 0));
            assert!(game.server.portals.arrivals.is_empty());
            assert_eq!(heard.lock().unwrap().len(), 1);
            assert!(game.level_entity(id as i32).is_none());
            assert_eq!(game.persist.removals.len(), 1);
            game.native_removals_flush();
            assert!(game.dropped.is_empty());
            assert!(game.persist.removals.is_empty());
            game.native_removals_flush();
            assert!(game.dropped.is_empty());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn discarded_mob_and_tnt_do_not_travel() {
        with_gate_mode(true, |game, _, heard| {
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(sunrise_sim::mobs::Mob::new(
                101,
                kind,
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            ));
            game.spawn_primed_tnt(BlockPos::new(2, 64, 0), false, None, None);
            let tnt = game.tnts.last().unwrap().id;
            for id in [101, tnt] {
                game.send_through_portal(id, BlockPos::new(0, 64, 0));
                assert!(game.level_entity(id as i32).is_none());
            }
            assert!(game.server.portals.arrivals.is_empty());
            assert_eq!(heard.lock().unwrap().len(), 2);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn refused_mob_passenger_stays_behind() {
        with_gate(|game, spare, heard| {
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            for id in [101, 102, 103] {
                let mut mob =
                    sunrise_sim::mobs::Mob::new(id, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
                if id != 101 {
                    mob.extra.mount = Some(101);
                }
                game.mobs.push(mob);
            }
            spare.store(102, Ordering::Relaxed);
            game.send_through_portal(101, BlockPos::new(0, 64, 0));
            assert_eq!(game.server.portals.arrivals.len(), 1);
            assert_eq!(game.mobs.iter().map(|m| m.id).collect::<Vec<_>>(), [102]);
            assert_eq!(game.mobs[0].extra.mount, None);
            assert_eq!(
                heard
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|e| e.0)
                    .collect::<Vec<_>>(),
                [101, 102, 103]
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn passenger_callbacks_cannot_revisit_ancestors_or_discarded_siblings() {
        for attack in [1004, 1001, 1002] {
            with_gate(|game, spare, heard| {
                prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
                let kind = game.res.mob_types().get("minecraft:camel").unwrap().clone();
                for id in [101, 102, 103] {
                    let mut mob = sunrise_sim::mobs::Mob::new(
                        id,
                        kind.clone(),
                        Vec3::new(0.5, 64.0, 0.5),
                        0.0,
                    );
                    if id != 101 {
                        mob.extra.mount = Some(101);
                    }
                    game.mobs.push(mob);
                }
                spare.store(attack, Ordering::Relaxed);
                assert!(game.entity_may_travel(101, Dim::Nether, TravelCause::Teleport));
                let ids: Vec<_> = heard.lock().unwrap().iter().map(|e| e.0).collect();
                assert_eq!(
                    ids,
                    if attack == 1001 {
                        vec![101, 102, 103]
                    } else {
                        vec![101, 102]
                    }
                );
                assert!(
                    !(game.mobs[0].extra.mount == Some(102)
                        && game.mobs[1].extra.mount == Some(101))
                );
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn mixed_passengers_follow_boarding_not_connection_order() {
        with_gate(|game, _, heard| {
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            let kind = game
                .res
                .mob_types()
                .get("minecraft:happy_ghast")
                .unwrap()
                .clone();
            for id in [101, 102] {
                let mut mob =
                    sunrise_sim::mobs::Mob::new(id, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
                if id == 102 {
                    mob.extra.mount = Some(101);
                    mob.extra.vehicle_order = 20;
                }
                game.mobs.push(mob);
            }
            for (conn, order) in [(12, 30), (13, 10)] {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |g| {
                        g.vehicle = Some(101);
                        g.vehicle_order = order;
                    });
            }
            assert!(game.entity_may_travel(101, Dim::Nether, TravelCause::Teleport));
            assert_eq!(
                heard
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|e| e.0)
                    .collect::<Vec<_>>(),
                [101, 13, 102, 12]
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn portal_arrivals_keep_the_live_opt_out_without_writing_a_waiting_entity() {
        mod_platform::testing::with_mods(Default::default(), || {
            with_resident_game(|game| {
                let chunk = (0, 0);
                let at = BlockPos::new(4, 64, 4);
                let spawn = |game: &mut sunrise_gameplay::game::ServerGame<'_>, nbt| {
                    let target = game
                        .summon_with_nbt("minecraft:text_display", [4.0, 64.0, 4.0], nbt)
                        .unwrap()
                        .unwrap();
                    game.entity(game.resident_target_id(target).unwrap())
                        .unwrap()
                };
                let transient = spawn(game, "{UUID:[I;0,0,0,583]}");
                let persistent = spawn(game, "{UUID:[I;0,0,0,584]}");
                assert_eq!(transient.set_persistent(game, false), Ok(true));
                game.send_through_portal(transient.id(), at);
                game.send_through_portal(persistent.id(), at);
                let world = tempfile::tempdir().unwrap();
                game.server.world_root = world.path().to_path_buf();
                let path = game.arrivals_path();
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                game.save_portal_arrivals();
                let written = sunrise_sim::save::read_dat(&path).unwrap();
                let Some(Tag::List(saved)) = written.get("arrivals") else {
                    panic!("missing arrivals list");
                };
                assert_eq!(saved.len(), 1);
                let saved = arrival_from_tag(&saved[0]).unwrap();
                assert_eq!(
                    sunrise_sim::entity_nbt::uuid(&saved.tag),
                    Some([0, 0, 0, 584])
                );

                let waiting = std::mem::take(&mut game.server.portals.arrivals);
                assert_eq!(waiting.len(), 2);
                for pending in waiting {
                    let Some(TransferredArrivalState::Entity(state)) = pending.transferred else {
                        panic!("display must carry its live transfer state");
                    };
                    game.spawn_teleported_entity(chunk, pending.tag.into(), Some(state));
                }
                let level = game.native_current_level().unwrap();
                assert!(
                    game.level_entity_by_uuid(level, mod_api::EntityUuid(583))
                        .is_some()
                );
                let records = game.entity_chunks(&std::collections::HashSet::from([chunk]), false);
                assert_eq!(records[0].1.len(), 1);
                assert_eq!(
                    sunrise_sim::entity_nbt::uuid(&records[0].1[0]),
                    Some([0, 0, 0, 584])
                );
            });
        });
    }
}

#[cfg(test)]
#[path = "tests/portal_cooldown_tests.rs"]
pub(crate) mod cooldown_tests;

#[cfg(test)]
#[path = "nether_portal/arrival_effects_tests.rs"]
pub(crate) mod arrival_effects_tests;

#[cfg(test)]
#[path = "tests/nether_portal_stand_tests.rs"]
pub(crate) mod stand_tests;
