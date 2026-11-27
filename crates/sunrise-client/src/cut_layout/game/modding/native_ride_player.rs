#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::RidePlayerRefusal as Refusal;
    use mod_api::{EntityMountEvent, Flow, Hooks, ServerHost};
    use std::sync::{Arc, Mutex};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::test_fixtures::{
        collect_entity_visit, fixture_resident_column,
    };
    use sunrise_sim::lan::entity::Riding;
    use sunrise_sim::mobs::{Mob, riders};

    /// The `entity_mount` calls a test heard: rider, vehicle, mounting.
    type Heard = Arc<Mutex<Vec<(u32, u32, bool)>>>;

    /// Hears `entity_mount`, and cancels every dismount when `keep` is set.
    struct Mounts(Heard, bool);
    #[mod_api::__subscriptions]
    impl Hooks for Mounts {
        fn entity_mount(&self, _: &mut dyn ServerHost, event: &mut EntityMountEvent<'_>) -> Flow {
            self.0
                .lock()
                .unwrap()
                .push((event.entity, event.vehicle, event.mounting));
            if self.1 && !event.mounting {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    /// A pig seated on a guest, then `case` run on the game: afterwards the pig rides nothing
    /// and `entity_mount` heard it get off the guest, a listener that cancels it or not.
    fn ejected(case: impl Fn(&mut sunrise_gameplay::game::ServerGame<'_>, u32)) {
        for keep in [false, true] {
            let heard = Heard::default();
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "mounts",
                Box::new(Mounts(heard.clone(), keep)),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    // `entity_mount` is heard on a server thread.
                    game.options.dedicated = Some(Vec::new());
                    fixture_resident_column(&mut game.world, (0, 0));
                    let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    game.mobs.push(Mob::new(80, pig, Vec3::ZERO, 0.0));
                    let player =
                        sunrise_gameplay::game::native_player_death::tests::add_guest(game)
                            .entity();
                    let pig = game.entity(80).unwrap();
                    assert_eq!(game.entity_ride_player(pig, player, false), Ok(()));
                    heard.lock().unwrap().clear();
                    let conn = game.server.lan.as_ref().unwrap().hub.guests[0].conn;
                    case(game, conn);
                    // The unload save takes the pig away with its chunk; every other case keeps it.
                    let mount = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                        game.mobs
                            .iter()
                            .find(|m| m.id == 80 && !m.removed)
                            .map(|m| m.extra.mount)
                    };
                    assert!(
                        matches!(mount(game), None | Some(None)),
                        "ejected, a cancelling listener or not"
                    );
                    assert_eq!(game.seated_passengers(player.id()), Vec::new());
                    // It stays off: a later save and a tick of the world do not seat it again.
                    let _ = game.save_entities(false);
                    let _ = game.tick_creature_entities();
                    assert!(matches!(mount(game), None | Some(None)), "stays dismounted");
                    assert_eq!(game.seated_passengers(player.id()), Vec::new());
                    let heard = heard.lock().unwrap();
                    assert!(heard.contains(&(80, player.id(), false)), "{heard:?}");
                    assert!(!heard.iter().any(|&(_, _, mounting)| mounting), "{heard:?}");
                });
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_save_ejects_a_mob_riding_a_player() {
        ejected(|game, _| {
            let _ = game.save_entities(false);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_level_save_ejects_a_mob_riding_a_player() {
        ejected(|game, _| {
            let _ = game.save_level(true);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_autosave_ejects_a_mob_riding_a_player() {
        ejected(|game, _| {
            let _ = game.save_game_with(false);
        });
    }

    /// `save-all` from the control channel: the save staged across event-loop turns,
    /// whose entity stage snapshots one chunk at a time.
    #[test]
    #[ignore = "daily"]
    fn a_staged_save_ejects_a_mob_riding_a_player() {
        ejected(|game, _| {
            // The entity stage runs once per loaded chunk, as the unload save's fixture has one.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let (reply, _answer) = std::sync::mpsc::channel();
            let mut save = game.test_begin_server_control_save(true, reply).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            while !game.test_poll_server_closing_save(&mut save).unwrap() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the save did not finish"
                );
                std::thread::yield_now();
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_unload_save_ejects_a_mob_riding_a_player() {
        ejected(|game, _| {
            let keys = [(0, 0)].into_iter().collect();
            let (records, _) = game.unloading_entity_snapshots(&keys);
            game.server.save.0.save_entities_later(records).unwrap();
            game.remove_unloading_entities(&keys);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_logout_ejects_a_mob_riding_the_player() {
        ejected(|game, _| {
            let host = game.server.lan.as_mut().unwrap();
            host.hub.guests.update(0, |g| g.placed = true);
            // The hub takes a guest out of its list before it reports the leave.
            let guest = host.hub.guests.remove(0);
            let event = sunrise_sim::lan::hub::HostEvent::Left(Box::new(guest));
            let host = game.server.lan.take().unwrap();
            game.server.lan = game.lan_process_events(host, [event], &[], [0.5, 64.0, 0.5]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_death_ejects_a_mob_riding_the_player() {
        ejected(|game, conn| {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.health = 0.0);
            game.guest_die(0, conn);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_dimension_change_ejects_a_mob_riding_the_player() {
        ejected(|game, conn| {
            game.start_server_trip(conn, sunrise_gameplay_common::game::Dim::Nether, None);
        });
    }

    /// The piggyback pack's cases on a guest: the refusals, the seat on top of
    /// the player as it moves, the guest's own `SetPassengers`, getting off,
    /// and the player leaving the level.
    #[test]
    #[ignore = "daily"]
    fn a_mob_rides_a_guest_sits_on_it_and_the_guest_is_told() {
        super::super::native_entity_refs::tests::with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            for id in [80, 81] {
                game.mobs.push(Mob::new(id, pig.clone(), Vec3::ZERO, 0.0));
            }
            let player =
                sunrise_gameplay::game::native_player_death::tests::add_guest(game).entity();
            let (first, second) = (game.entity(80).unwrap(), game.entity(81).unwrap());
            let mut riding = Riding::default();
            // Nothing rides a player: no one is told anything.
            game.lan_riding(&[], &mut riding);
            assert!(riding.told_self.is_empty());

            assert_eq!(
                game.entity_ride_player(player, first, false),
                Err(Refusal::NotAPlayer)
            );
            assert_eq!(
                game.entity_ride_player(player, player, false),
                Err(Refusal::SameEntity)
            );
            assert_eq!(game.entity_ride_player(first, player, false), Ok(()));
            assert_eq!(game.mobs[0].extra.mount, Some(player.id()));
            assert_eq!(game.entity_vehicle(first), Some(player));
            assert_eq!(
                collect_entity_visit(|visit| player.visit_passengers(game, visit)),
                Ok(vec![first])
            );
            assert_eq!(
                game.entity_ride_player(first, player, false),
                Err(Refusal::AlreadyRiding)
            );
            assert_eq!(
                game.entity_ride_player(second, player, false),
                Err(Refusal::Full)
            );

            // The seat: the top of the player's box, less the pig's attachment.
            game.sync_riders();
            let [x, _, z] = game.entity_position(player).unwrap();
            let [low, high] = game.entity_bounding_box(player).unwrap();
            let m = &game.mobs[0];
            let offset = riders::vehicle_attachment(m, (high[0] - low[0]) as f32);
            assert_eq!(
                m.extra.mount,
                Some(player.id()),
                "a player vehicle stays valid"
            );
            assert_eq!(
                [m.position.x, m.position.y, m.position.z],
                [x, high[1] - offset, z]
            );
            assert!(m.extra.passenger);

            // The guest's own client is told, once.
            game.lan_riding(&[], &mut riding);
            let id = player.id() as i32;
            assert_eq!(riding.passengers(id), [80]);
            assert_eq!(riding.told_self.get(&id), Some(&vec![80]));

            // Forced past the one-passenger limit, as vanilla's force is.
            assert_eq!(game.entity_ride_player(second, player, true), Ok(()));
            assert!(game.entity_stop_riding(first));
            assert_eq!(game.mobs[0].extra.mount, None);
            assert!(game.entity_eject_passengers(player));
            game.lan_riding(&[], &mut riding);
            assert!(riding.told_self.is_empty(), "told it carries nothing now");

            // Off a vehicle, the boarding cooldown runs; ride_player forces it on. The plain
            // start_riding keeps refusing a player vehicle, forced or not.
            assert_eq!(
                game.entity_ride_player(first, player, false),
                Err(Refusal::CannotRide)
            );
            assert_eq!(
                game.entity_start_riding(first, player, true),
                Err(mod_api::StartRidingRefusal::PlayerVehicle)
            );
            assert_eq!(
                game.entity_start_riding(first, player, false),
                Err(mod_api::StartRidingRefusal::PlayerVehicle)
            );
            assert_eq!(game.entity_vehicle(first), None);
            assert_eq!(game.entity_ride_player(first, player, true), Ok(()));
            assert_eq!(game.entity_vehicle(first), Some(player));

            // The player riding the second pig: that pig can't ride the player.
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.vehicle = Some(81));
            assert_eq!(
                game.entity_ride_player(second, player, true),
                Err(Refusal::Cycle)
            );
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.vehicle = None;
                    guest.dimension = "minecraft:the_nether".into();
                });
            // The player left the level: its passenger gets off.
            game.sync_riders();
            assert_eq!(game.mobs[0].extra.mount, None);
        });
    }
}
