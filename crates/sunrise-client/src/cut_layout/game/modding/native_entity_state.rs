#![cfg(test)]
use mod_api::EntityRef;
#[cfg(test)]
pub(crate) mod tests {

    #[cfg(feature = "client")]
    use sunrise_gameplay::game::server_side::native_entity_state::tests::with_player;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_entity_state_common::NativeEntityStateCommonGameExt1 as _;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1;

    use super::*;

    #[cfg(feature = "client")]
    use sunrise_sim::motion::PlayerMotion;
    use sunrise_sim::{mobs::Mob, thrown_trident::ThrownTrident};

    #[cfg(feature = "client")]
    use super::super::native_entity_refs::tests::{run_fresh_cases, with_client_game};

    #[cfg(feature = "client")]
    use super::super::native_guest_turn_tests::with_resident_guests;

    #[cfg(feature = "client")]
    use mod_api::MainHand;

    #[cfg(feature = "client")]
    use sunrise_protocol::entity_data::{Value, serializer};

    #[cfg(feature = "client")]
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    #[test]
    #[ignore = "daily"]
    fn tick_count_reads_supported_entities_and_refuses_others() {
        use super::super::native_entity_refs::tests::bind_local_player;
        use super::super::native_guest_turn_tests::with_resident_guests;
        use mod_api::WorldEpoch;
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
        use sunrise_items::items::ItemStack;
        use sunrise_sim::entities::ExperienceOrb;
        use sunrise_sim::motion::PlayerMotion;

        with_resident_guests(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            game.player.as_mut().unwrap().health.tick_count = i32::MAX;
            game.player.as_mut().unwrap().health.advance_entity_clock();
            assert_eq!(player.tick_count(game), Some(i32::MIN));

            let acting_guest = game.entity(12).unwrap();
            let parked_guest = game.entity(13).unwrap();
            assert_eq!(parked_guest.tick_count(game), Some(0));
            game.as_guest(13, |game| {
                game.player.as_mut().unwrap().health.tick_count = 314;
            })
            .unwrap();
            assert_eq!(parked_guest.tick_count(game), Some(314));

            game.as_guest(12, |game| {
                game.player.as_mut().unwrap().health.tick_count = 2718;
                assert_eq!(acting_guest.tick_count(game), Some(2718));
            })
            .unwrap();

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(42, kind, Vec3::ZERO, 0.0));
            game.mobs[0].age = 2401;
            game.mobs[0].extra.age = -24000;
            let mob = game.entity(42).unwrap();
            assert_eq!(mob.tick_count(game), Some(2401));

            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            game.dropped[0].ticks = 77;
            game.dropped[0].age = 5000;
            let item = game.entity(game.dropped[0].id).unwrap();
            assert_eq!(item.tick_count(game), Some(77));

            game.orbs.push(ExperienceOrb {
                size_answer: None,
                attachment_lifetime: Default::default(),
                fall_distance: 0.0,
                rotation: [0.0, 0.0],
                join: Default::default(),
                id: 43,
                position: Vec3::ZERO,
                previous: Vec3::ZERO,
                velocity: Vec3::ZERO,
                value: 7,
                age: 600,
                ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
                count: 1,
                fire_ticks: 0,
                ticks: 21,
                health: 5,
                air: 300,
                no_gravity: true,
                following: None,
            });
            let orb = game.entity(43).unwrap();
            assert_eq!(orb.tick_count(game), Some(21));

            game.carts
                .push(Minecart::new(44, CartKind::Rideable, Vec3::ZERO));
            game.carts.update(0, |c| c.tick_count = 5);
            let cart = game.entity(44).unwrap();
            assert_eq!(cart.tick_count(game), Some(5));

            game.boats
                .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                    45,
                    "oak".into(),
                    Vec3::ZERO,
                    0.0,
                ));
            let boat = game.entity(45).unwrap();
            assert_eq!(boat.tick_count(game), None);
            let stale = EntityRef::new(WorldEpoch::new(mob.epoch().get() + 1), mob.id());
            assert_eq!(stale.tick_count(game), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_reads_simulated_kinds_and_refuses_others() {
        use super::super::native_guest_turn_tests::with_resident_guests;
        use mod_api::WorldEpoch;
        use sunrise_core::{geometry::Vec3, position::BlockPos};
        use sunrise_gameplay_common::game::{
            Dim, FallingBlock, boats_common::Boat, fireworks::Rocket, minecarts::CartKind,
            minecarts::Minecart,
        };
        use sunrise_items::items::ItemStack;
        use sunrise_sim::entities::{ArmorStand, ExperienceOrb};
        use sunrise_sim::projectiles::{Projectile, ProjectileKind};

        with_resident_guests(|game| {
            // Players keep their own state: in place during their turn, beside the hub otherwise.
            let host = game.server.lan.as_mut().unwrap();
            host.play.entry(12).or_default().portal.cooldown = 7;
            host.play.entry(13).or_default().portal.cooldown = 4;
            game.portal_player.cooldown = 99;
            let (acting, other) = (game.entity(12).unwrap(), game.entity(13).unwrap());
            assert_eq!(acting.portal_cooldown(game), Some(7));
            game.as_guest(12, |game| {
                assert_eq!(acting.portal_cooldown(game), Some(7));
                assert_eq!(other.portal_cooldown(game), Some(4));
            })
            .unwrap();
            assert_eq!(acting.portal_cooldown(game), Some(7));
            assert_eq!(other.portal_cooldown(game), Some(4));

            // A mob counts down from `setPortalCooldown`'s 300 as the level ticks.
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(Mob::new(40, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(40).unwrap();
            assert_eq!(mob.portal_cooldown(game), Some(0));
            game.set_portal_cooldown(40);
            assert_eq!(mob.portal_cooldown(game), Some(300));
            game.handle_entity_portals();
            assert_eq!(mob.portal_cooldown(game), Some(299));

            // The other kinds the level carries through portals.
            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            let item = game.dropped[0].id;
            game.orbs.push(ExperienceOrb {
                size_answer: None,
                attachment_lifetime: Default::default(),
                fall_distance: 0.0,
                rotation: [0.0, 0.0],
                join: Default::default(),
                no_gravity: false,
                id: 41,
                position: Vec3::ZERO,
                previous: Vec3::ZERO,
                velocity: Vec3::ZERO,
                value: 1,
                age: 0,
                ground: Default::default(),
                count: 1,
                fire_ticks: 0,
                ticks: 0,
                health: 5,
                air: 300,
                following: None,
            });
            game.boats
                .push(Boat::new(42, "oak".into(), Vec3::ZERO, 0.0));
            game.carts
                .push(Minecart::new(43, CartKind::Rideable, Vec3::ZERO));
            game.add_primed_tnt(BlockPos::new(0, 64, 0), 0.0, 80, None, None);
            let tnt = game.tnts[0].id;
            let arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::ZERO,
                None,
            );
            game.add_projectile(arrow);
            let arrow = game.creatures.projectiles[0].id;
            game.add_projectile(Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None));
            let snowball = game.creatures.projectiles[1].id;
            game.rockets
                .push(Rocket::remote(44, Vec3::ZERO, Vec3::ZERO));
            game.tridents
                .push(ThrownTrident::remote(45, Vec3::ZERO, 0.0, 0.0));
            // Entity defaults to 300; VehicleEntity overrides it to 10 and
            // Projectile to 2 (`getDimensionChangingDelay` in vanilla 26.3).
            for (id, delay) in [
                (item, 300),
                (41, 300),
                (42, 10),
                (43, 10),
                (tnt, 300),
                (arrow, 2),
                (snowball, 2),
                (44, 2),
                (45, 2),
            ] {
                let entity = game.entity(id).unwrap();
                assert_eq!(entity.portal_cooldown(game), Some(0), "entity {id}");
                game.set_portal_cooldown(id);
                assert_eq!(entity.portal_cooldown(game), Some(delay), "entity {id}");
            }

            // A falling block keeps its own, as its save does.
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:sand")
                .unwrap();
            game.server.next_entity += 1;
            let falling = game.server.next_entity;
            game.falling.push(FallingBlock {
                size_answer: None,
                ground: Default::default(),
                fire_ticks: 0,
                sync_velocity: false,
                attachment_lifetime: Default::default(),
                rotation: [0.0, 0.0],
                join: Default::default(),
                id: falling,
                state,
                blocks_building: true,
                start_pos: BlockPos::default(),
                position: Vec3::ZERO,
                previous: Vec3::ZERO,
                velocity: Vec3::ZERO,
                age: 0,
                no_gravity: false,
                portal: sunrise_sim::nether_portal::PortalState {
                    cooldown: 25,
                    process: None,
                },
                drop_item: false,
                fall_distance: 0.0,
                stuck: Vec3::ZERO,
                damage: sunrise_sim::falling_block::Damage::falling("minecraft:sand"),
                saved: None,
            });
            let falling = game.entity(falling).unwrap();
            assert_eq!(falling.portal_cooldown(game), Some(25));

            // No portal cooldown: an armor stand, a wind charge, a stale handle, a parked level.
            game.armor_stands.push(ArmorStand::new(46, Vec3::ZERO, 0.0));
            let stand = game.entity(46).unwrap();
            assert_eq!(stand.portal_cooldown(game), None);
            game.add_projectile(Projectile::new(
                ProjectileKind::WindCharge,
                Vec3::ZERO,
                None,
            ));
            let charge = game.entity(game.creatures.projectiles[2].id).unwrap();
            assert_eq!(charge.portal_cooldown(game), None);
            let stale = EntityRef::new(WorldEpoch::new(mob.epoch().get() + 1), mob.id());
            assert_eq!(stale.portal_cooldown(game), None);
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(mob.portal_cooldown(game), None);
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(mob.portal_cooldown(game), Some(299));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_is_refused_for_an_acting_guest() {
        use super::super::native_guest_turn_tests::with_resident_guests;

        with_resident_guests(|game| {
            let host = game.server.lan.as_mut().unwrap();
            host.play.entry(12).or_default().portal.cooldown = 7;
            let guest = game.entity(12).unwrap();
            assert!(!guest.set_portal_cooldown(game, 40));
            assert_eq!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .play
                    .get(&12)
                    .unwrap()
                    .portal
                    .cooldown,
                7
            );
            assert_eq!(guest.portal_cooldown(game), Some(7));
        });
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn on_fire_reads_timers_immunity_and_client_shared_flag() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_player(|g, player| {
                for ticks in [-1, 0, 1] {
                    g.player.as_mut().unwrap().fire_ticks = ticks;
                    assert_eq!(player.is_on_fire(g), Some(ticks > 0));
                }
                for (id, kind, expected) in [
                    (40, "minecraft:zombie", true),
                    (41, "minecraft:blaze", false),
                ] {
                    let kind = g.res.mob_types().get(kind).unwrap().clone();
                    g.mobs
                        .push(Mob::new(id, kind, sunrise_core::geometry::Vec3::ZERO, 0.0));
                    g.mobs.last_mut().unwrap().fire_ticks = 20;
                    assert_eq!(g.entity(id).unwrap().is_on_fire(g), Some(expected));
                }
                assert_eq!(
                    EntityRef::new(mod_api::WorldEpoch::new(0), 40).is_on_fire(g),
                    None
                );
            });
            super::super::native_entity_refs::tests::with_client_game(|g| {
                let _server = sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(g);
                g.player =
                    Some(PlayerMotion::new(g.motion, sunrise_core::geometry::Vec3::ZERO).unwrap());
                let local = g.entity(7).unwrap();
                // Spawn actual renderable client copies through the normal packet path.
                for (id, name) in [(15, "minecraft:armor_stand"), (16, "minecraft:zombie")] {
                    let kind = g
                        .client
                        .remote
                        .as_ref()
                        .unwrap()
                        .builtin
                        .get("minecraft:entity_type")
                        .unwrap()
                        .id(name)
                        .unwrap();
                    g.handle_packet(sunrise_protocol::packets::play::Clientbound::AddEntity(
                        sunrise_protocol::packets::play::AddEntity {
                            id,
                            uuid: id as u128,
                            kind,
                            position: [0.0; 3],
                            velocity: [0.0; 3],
                            pitch: 0,
                            yaw: 0,
                            head_yaw: 0,
                            data: 0,
                        },
                    ))
                    .unwrap();
                }
                let stand = g.entity(15).unwrap();
                let zombie = g.entity(16).unwrap();
                // Tracked copies answer the same type-level fire immunity as the server.
                for (id, name, immune) in [
                    (17, "minecraft:tnt", true),
                    (18, "minecraft:end_crystal", true),
                    (19, "minecraft:blaze", true),
                    (20, "minecraft:pig", false),
                ] {
                    let kind = g
                        .client
                        .remote
                        .as_ref()
                        .unwrap()
                        .builtin
                        .get("minecraft:entity_type")
                        .unwrap()
                        .id(name)
                        .unwrap();
                    g.handle_packet(sunrise_protocol::packets::play::Clientbound::AddEntity(
                        sunrise_protocol::packets::play::AddEntity {
                            id,
                            uuid: id as u128,
                            kind,
                            position: [0.0; 3],
                            velocity: [0.0; 3],
                            pitch: 0,
                            yaw: 0,
                            head_yaw: 0,
                            data: 0,
                        },
                    ))
                    .unwrap();
                    assert_eq!(
                        g.entity(id as u32)
                            .unwrap()
                            .fire_immune(&*g as &dyn mod_api::ClientHost),
                        Some(immune),
                        "{name}"
                    );
                }
                for flag in [1, 0] {
                    for id in [7, 15, 16] {
                        g.handle_packet(
                            sunrise_protocol::packets::play::Clientbound::SetEntityData(
                                sunrise_protocol::packets::play::SetEntityData {
                                    id,
                                    values: vec![(
                                        0,
                                        0,
                                        sunrise_protocol::entity_data::Value::Byte(flag),
                                    )],
                                },
                            ),
                        )
                        .unwrap();
                    }
                    assert_eq!(
                        local.is_on_fire(&*g as &dyn mod_api::ClientHost),
                        Some(flag != 0)
                    );
                    assert_eq!(
                        stand.is_on_fire(&*g as &dyn mod_api::ClientHost),
                        Some(flag != 0)
                    );
                    assert!(local.set_client_fire_immunity(g, true));
                    assert_eq!(local.fire_immune(g), Some(true));
                    assert_eq!(local.is_on_fire(g), Some(false));
                    assert!(local.set_client_fire_immunity(g, false));
                    assert_eq!(local.is_on_fire(g), Some(flag != 0));
                    // Revoking while the server flag is unchanged must reveal the
                    // existing flame again, both in the handle and in the draw state.
                    for (grant, burning) in [(true, false), (false, flag != 0)] {
                        assert!(zombie.set_client_fire_immunity(g, grant));
                        assert!(stand.set_client_fire_immunity(g, grant));
                        assert_eq!(zombie.is_on_fire(g), Some(burning));
                        assert_eq!(stand.is_on_fire(g), Some(burning));
                        assert_eq!(
                            g.mobs.iter().find(|m| m.id == 16).unwrap().is_on_fire(),
                            burning
                        );
                        assert_eq!(
                            g.armor_stands
                                .iter()
                                .find(|s| s.id == 15)
                                .unwrap()
                                .is_on_fire(),
                            burning
                        );
                    }
                }
                g.player.as_mut().unwrap().fire_ticks = 2;
                assert_eq!(
                    local.is_on_fire(&*g as &dyn mod_api::ClientHost),
                    Some(true)
                );
            });
        });
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn under_water_reads_previous_eye_sample_and_current_contact_on_both_sides() {
        fn check_under_water<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
            g: &mut G,
            client: bool,
            player: EntityRef,
            mob: EntityRef,
        ) {
            for old_eye in [false, true] {
                for contact in [false, true] {
                    let p = g.player.as_mut().unwrap();
                    p.fluids.was_eye_in_water = old_eye;
                    p.fluids.was_touching_water = contact;
                    assert_eq!(
                        g.entity_is_under_water(player),
                        Some(if client { old_eye } else { old_eye && contact })
                    );
                    let m = g.mobs.iter_mut().find(|m| m.id == 15).unwrap();
                    m.was_eye_in_water = old_eye;
                    m.fluids.eye_in_water = !old_eye;
                    m.fluids.in_water = contact;
                    assert_eq!(g.entity_is_under_water(mob), Some(old_eye && contact));
                    assert_eq!(
                        g.entity_is_under_water(EntityRef::new(mod_api::WorldEpoch::new(0), 15)),
                        None
                    );
                }
            }
        }
        mod_platform::testing::with_mods(Vec::new().into(), || {
            super::super::native_entity_refs::tests::with_resident_game(|g| {
                g.player =
                    Some(PlayerMotion::new(g.motion, sunrise_core::geometry::Vec3::ZERO).unwrap());
                g.options.dedicated = Some(Vec::new());
                super::super::native_entity_refs::tests::bind_local_player(g, 7, 123);
                let player = g.entity(7).unwrap();
                let kind = g.res.mob_types().get("minecraft:zombie").unwrap().clone();
                g.mobs
                    .push(Mob::new(15, kind, sunrise_core::geometry::Vec3::ZERO, 0.0));
                let mob = g.entity(15).unwrap();
                check_under_water(g, false, player, mob);
            });
            super::super::native_entity_refs::tests::with_client_game(|g| {
                let _server = sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(g);
                g.player =
                    Some(PlayerMotion::new(g.motion, sunrise_core::geometry::Vec3::ZERO).unwrap());
                let player = g.entity(7).unwrap();
                g.spawn_entity(15, "minecraft:zombie", [0.0; 3], [0.0; 3]);
                let mob = g.entity(15).unwrap();
                check_under_water(g, true, player, mob);
            });
        });
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn secondary_use_reads_reachable_client_handles_and_keeps_server_flags() {
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::connection::Pipe;
        use winit::keyboard::KeyCode;

        super::super::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "SneakReadTest").unwrap();
            // The joined client's state; the local-player and entity constructors must both
            // resolve it, not just a hand-built entity id accepted by an unreachable stub.
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let local = game.local_player().unwrap();
            let entity = game.entity(7).unwrap();
            assert_eq!(local.entity(), entity);
            game.insert_device_key(KeyCode::ShiftLeft);
            assert!(!game.player.as_ref().unwrap().shift_key_down);
            assert_eq!(entity.is_shift_key_down(game), Some(true));
            game.remove_device_key(&KeyCode::ShiftLeft);
            game.player.as_mut().unwrap().shift_key_down = true;
            assert_eq!(
                entity.is_shift_key_down(game),
                Some(false),
                "live secondary use, not last tick's body flag"
            );
            let stale = EntityRef::new(mod_api::WorldEpoch::new(0), 7);
            assert_eq!(stale.is_shift_key_down(game), None);
            game.client.remote.as_mut().unwrap().logged_in = false;
            assert!(game.local_player().is_none());
            assert_eq!(entity.is_shift_key_down(game), None);
            game.clear_device_keys();
        });
        with_player(|game, player| {
            game.player.as_mut().unwrap().shift_key_down = true;
            assert_eq!(
                player.is_shift_key_down(game),
                Some(true),
                "server keeps its semantic entity flag"
            );
        });
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn client_fluid_reads_use_cached_local_and_tracked_bodies() {
        use super::super::native_entity_refs::tests::with_client_game;
        use mod_api::EntityReadError;
        use std::ops::ControlFlow;
        use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::play;

        with_client_game(|game| {
            let _server = logged_client(game);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let local = game.entity(7).unwrap();
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(
                local.visit_fluid_heights(host, &mut |_, _| panic!("dry player visited")),
                Ok(ControlFlow::Continue(()))
            );
            assert_eq!(local.eye_fluid(host), Some(None));

            game.spawn_entity(15, "minecraft:zombie", [2.0, 0.0, 0.0], [0.0; 3]);
            let zombie = game.entity(15).unwrap();
            // No water or lava in the world: these answers must come from the last
            // client fluid update, without a fresh world query or a server body.
            for (water, lava, eye_water, eye_lava, expected, eye) in [
                (
                    0.75,
                    0.0,
                    true,
                    false,
                    vec![("minecraft:water".to_owned(), 0.75)],
                    Some("minecraft:water"),
                ),
                (
                    0.0,
                    0.5,
                    false,
                    false,
                    vec![("minecraft:lava".to_owned(), 0.5)],
                    None,
                ),
                (
                    0.25,
                    1.5,
                    false,
                    true,
                    vec![
                        ("minecraft:lava".to_owned(), 1.5),
                        ("minecraft:water".to_owned(), 0.25),
                    ],
                    Some("minecraft:lava"),
                ),
                (0.0, 0.0, false, false, Vec::new(), None),
            ] {
                let fluids = &mut game.mobs.iter_mut().find(|m| m.id == 15).unwrap().fluids;
                fluids.water = water;
                fluids.lava = lava;
                fluids.in_water = water > 0.0;
                fluids.in_lava = lava > 0.0;
                fluids.eye_in_water = eye_water;
                fluids.eye_in_lava = eye_lava;
                let host = &*game as &dyn mod_api::ClientHost;
                let mut fluids = Vec::new();
                assert_eq!(
                    zombie.visit_fluid_heights(host, &mut |id, depth| {
                        fluids.push((id.to_owned(), depth));
                        ControlFlow::Continue(())
                    }),
                    Ok(ControlFlow::Continue(()))
                );
                assert_eq!(fluids, expected);
                assert_eq!(zombie.eye_fluid(host), Some(eye));
            }
            game.spawn_entity(16, "minecraft:armor_stand", [2.0, 0.0, 0.0], [0.0; 3]);
            let stand = game.entity(16).unwrap();
            let host = &*game as &dyn mod_api::ClientHost;
            assert_eq!(
                stand.visit_fluid_heights(host, &mut |_, _| panic!("absent fluid visit")),
                Err(EntityReadError::Unavailable)
            );
            assert_eq!(stand.eye_fluid(host), None);
            let stale = EntityRef::new(mod_api::WorldEpoch::new(0), 15);
            assert_eq!(
                stale.visit_fluid_heights(host, &mut |_, _| panic!("absent fluid visit")),
                Err(EntityReadError::Stale)
            );
            assert_eq!(stale.eye_fluid(host), None);
            game.handle_packet(play::Clientbound::RemoveEntities(play::RemoveEntities {
                ids: vec![15],
            }))
            .unwrap();
            assert_eq!(
                zombie.visit_fluid_heights(&*game as &dyn mod_api::ClientHost, &mut |_, _| panic!(
                    "absent fluid visit"
                )),
                Err(EntityReadError::Stale)
            );
            assert_eq!(zombie.eye_fluid(&*game as &dyn mod_api::ClientHost), None);
            game.client.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(
                local.visit_fluid_heights(&*game as &dyn mod_api::ClientHost, &mut |_, _| panic!(
                    "absent fluid visit"
                )),
                Err(EntityReadError::Unavailable)
            );
            assert_eq!(local.eye_fluid(&*game as &dyn mod_api::ClientHost), None);
        });
    }
    /// A client reads every entity it tracks as the server sent it: a mob's left-handed flag
    /// (`Mob.DATA_MOB_FLAGS_ID`) and another player's main hand (`Avatar.DATA_PLAYER_MAIN_HAND`);
    /// its own player follows its Main Hand option. An entity that isn't living, and one the
    /// client doesn't track, has none.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_client_reads_the_main_arm_the_server_sent() {
        if run_fresh_cases(
            "cut_layout::game::server_side::native_entity_state::tests::a_client_reads_the_main_arm_the_server_sent",
            "SUNRISE_TEST_CLIENT_MAIN_ARM_CASE",
            &["client"],
        ) {
            return;
        }
        use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;
        mod_platform::testing::with_mods(Default::default(), || {
            with_client_game(|game| {
                let _server = logged_client(game);
                game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                let types = game.client.remote.as_ref().unwrap().builtin.clone();
                let kind = |name: &str| {
                    types
                        .get("minecraft:entity_type")
                        .unwrap()
                        .id(name)
                        .unwrap()
                };
                game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                    actions: 1,
                    entries: vec![play::PlayerInfo {
                        uuid: 132,
                        name: Some("Remote".into()),
                        ..Default::default()
                    }],
                }))
                .unwrap();
                for (id, name) in [
                    (31, "minecraft:zombie"),
                    (32, "minecraft:player"),
                    (33, "minecraft:experience_orb"),
                ] {
                    game.handle_packet(C::AddEntity(play::AddEntity {
                        id,
                        uuid: 100 + id as u128,
                        kind: kind(name),
                        position: [2.0, 64.0, 3.0],
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 0,
                    }))
                    .unwrap();
                }
                let (zombie, player, orb) = (
                    game.entity(31).unwrap(),
                    game.entity(32).unwrap(),
                    game.entity(33).unwrap(),
                );
                let local = game.local_player().unwrap().entity();
                let read = |game: &sunrise_client_gameplay::game::ClientGame<'_>,
                            entity: EntityRef| {
                    let host: &dyn mod_api::ClientHost = game;
                    entity.main_arm(host)
                };
                assert_eq!(read(game, zombie), Some(MainHand::Right));
                assert_eq!(read(game, player), Some(MainHand::Right));
                for left in [true, false] {
                    // No AI 1, left-handed 2, aggressive 4; HumanoidArm 0 is the left arm.
                    let flags = if left { 2 | 4 } else { 4 };
                    game.handle_packet(C::SetEntityData(play::SetEntityData {
                        id: 31,
                        values: vec![(15, serializer::BYTE, Value::Byte(flags))],
                    }))
                    .unwrap();
                    game.handle_packet(C::SetEntityData(play::SetEntityData {
                        id: 32,
                        values: vec![(15, serializer::HUMANOID_ARM, Value::Id(i32::from(!left)))],
                    }))
                    .unwrap();
                    game.client.settings.main_hand_left = left;
                    let arm = if left {
                        MainHand::Left
                    } else {
                        MainHand::Right
                    };
                    assert_eq!(read(game, zombie), Some(arm), "the mob");
                    assert_eq!(read(game, player), Some(arm), "the other player");
                    assert_eq!(read(game, local), Some(arm), "the local player");
                }
                assert_eq!(read(game, orb), None, "not a living entity");
                assert_eq!(read(game, EntityRef::new(local.epoch(), 99)), None);
            });
        });
    }
    /// A server reads a player's main arm from the Main Hand its client reported, and a mob's
    /// from its left-handed flag.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_server_reads_the_reported_main_hand_and_the_left_handed_flag() {
        if run_fresh_cases(
            "cut_layout::game::server_side::native_entity_state::tests::a_server_reads_the_reported_main_hand_and_the_left_handed_flag",
            "SUNRISE_TEST_SERVER_MAIN_ARM_CASE",
            &["server"],
        ) {
            return;
        }
        with_resident_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            let host = game.server.lan.as_mut().unwrap();
            host.hub.update_guest(12, |g| g.info.main_hand = 0).unwrap();
            let read = |game: &sunrise_gameplay::game::ServerGame<'_>, entity: EntityRef| {
                let host: &dyn mod_api::ServerHost = game;
                entity.main_arm(host)
            };
            assert_eq!(read(game, a.entity()), Some(MainHand::Left));
            assert_eq!(read(game, b.entity()), Some(MainHand::Right));
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let at = sunrise_core::geometry::Vec3::ZERO;
            game.mobs
                .push(sunrise_sim::mobs::Mob::new(500, zombie, at, 0.0));
            let mob = game.entity(500).unwrap();
            assert_eq!(read(game, mob), Some(MainHand::Right));
            game.mobs.last_mut().unwrap().extra.left_handed = true;
            assert_eq!(read(game, mob), Some(MainHand::Left));
        });
    }
}
