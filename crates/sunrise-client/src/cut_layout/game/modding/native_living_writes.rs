#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use super::super::native_entity_refs::tests::{with_game, with_resident_game};
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_living_body::NativeLivingBodyGameExt3;

    use super::super::native_living_flags::tests::{guest, guest_server, packets, sync_guests};

    use super::super::native_living_position::tests::floor_and_zombies;

    use sunrise_sim::mobs::stuck::Stuck;

    use mod_api::client::first_person::Hand;

    use mod_api::{EntityRef, Identifier, WorldEpoch};

    use sunrise_core::geometry::Vec3;

    use sunrise_items::items::ItemStack;

    use sunrise_sim::lan::waypoints::Icon;

    use sunrise_sim::mobs::Mob;

    #[test]
    #[ignore = "daily"]
    fn a_mob_swings_while_its_hand_and_loot_flags_live_on_the_mob_handle() {
        with_resident_game(|game| {
            for (id, kind) in [(12, "minecraft:zombie"), (13, "minecraft:cow")] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let zombie = game.living(game.entity(12).unwrap()).unwrap();

            assert!(game.living_swing(zombie, Hand::Off, false));
            assert!(game.mobs[0].ai.swinging);
            assert_eq!(game.mobs[0].extra.swing_right_arm, Some(false));
            // In the first half of a swing it doesn't restart, past it does.
            game.mobs[0].ai.swing_time = 1;
            assert!(!game.living_swing(zombie, Hand::Main, false));
            game.mobs[0].ai.swing_time = 3;
            assert!(game.living_swing(zombie, Hand::Main, false));
            assert_eq!(game.mobs[0].ai.swing_time, -1);

            // A mob keeps what is stuck in it, but no spin and, using nothing,
            // no item use timer.
            assert_eq!(game.living_arrow_count(zombie), Some(0));
            assert!(game.living_set_arrow_count(zombie, 3));
            assert!(game.living_set_stinger_count(zombie, 2));
            assert!(game.living_set_arrow_removal_ticks(zombie, 4));
            assert!(game.living_set_stinger_removal_ticks(zombie, 5));
            assert_eq!(
                game.mobs[0].extra.stuck,
                Stuck {
                    arrows: 3,
                    stingers: 2,
                    remove_arrow_time: 4,
                    remove_stinger_time: 5,
                }
            );
            assert_eq!(game.living_arrow_count(zombie), Some(3));
            assert_eq!(game.living_stinger_count(zombie), Some(2));
            assert_eq!(game.living_item_use_remaining(zombie), Some(0));
            assert!(!game.living_set_item_use_remaining(zombie, 3));
            assert!(!game.living_set_riptiding(zombie, 20, 8.0));

            let stale = EntityRef::new(WorldEpoch::new(zombie.entity().epoch().get() + 1), 12);
            let stale = mod_api::LivingRef::new(stale);
            assert!(!game.living_swing(stale, Hand::Main, true));
            assert_eq!(game.living_arrow_count(stale), None);
            assert!(!game.living_set_stinger_count(stale, 1));
            assert_eq!(game.living_item_use_remaining(stale), None);
            assert!(!game.living_set_item_use_remaining(stale, 1));
            assert!(!game.living_set_riptiding(stale, 20, 8.0));
            let style = Identifier::parse("example:flag").unwrap();
            assert!(!game.living_set_waypoint_style(stale, &style));
            assert!(!game.living_set_waypoint_color(stale, Some(0xFF)));
        });
    }
    /// What is stuck in a mob falls out in the mob's own ticks, one in a count's
    /// worth of `20 * (30 - count)` ticks as for a player, the timer a write set
    /// running first.
    #[test]
    #[ignore = "daily"]
    fn a_mobs_stuck_arrows_and_stingers_fall_out_in_its_ticks() {
        with_game(|game| {
            floor_and_zombies(game);
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            assert!(game.living_set_arrow_count(zombie, 3));
            assert!(game.living_set_arrow_removal_ticks(zombie, 2));
            assert!(game.living_set_stinger_count(zombie, 1));
            assert!(game.living_set_stinger_removal_ticks(zombie, 1));
            game.tick_mobs().unwrap();
            // The stinger's last tick ran, the arrow's has one more.
            assert_eq!(game.living_stinger_count(zombie), Some(0));
            assert_eq!(game.living_arrow_count(zombie), Some(3));
            game.tick_mobs().unwrap();
            assert_eq!(game.living_arrow_count(zombie), Some(2));
            // The next one waits `20 * (30 - 2)` ticks from the new count.
            assert_eq!(game.mobs[0].extra.stuck.remove_arrow_time, 0);
            game.tick_mobs().unwrap();
            assert_eq!(
                game.mobs[0].extra.stuck.remove_arrow_time,
                20 * (30 - 2) - 1
            );
            // The other zombie has none of it.
            assert_eq!(game.mobs[1].extra.stuck, Stuck::default());
        });
    }
    /// A spear's use counts its own ticks, which a write to the ticks left
    /// moves: the next tick goes on from there.
    #[test]
    #[ignore = "daily"]
    fn a_mobs_spear_use_goes_on_from_the_ticks_a_write_left() {
        use sunrise_items::equipment::MAINHAND;
        with_game(|game| {
            floor_and_zombies(game);
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let spear = ItemStack::new("minecraft:iron_spear", 1);
            game.mobs[0].extra.set_slot_stack(MAINHAND, Some(spear));
            game.mobs[0].ai.targets.clear();
            game.mobs[0].ai.goals.clear();
            assert!(game.living_start_using_item(zombie, Hand::Main));
            let own = game.living_item_use_remaining(zombie).unwrap();
            assert!(own > 10);
            assert!(game.living_set_item_use_remaining(zombie, own - 10));
            assert_eq!(game.mobs[0].extra.using_item, 11);
            game.tick_mobs().unwrap();
            assert_eq!(game.mobs[0].extra.using_item, 12);
            assert_eq!(game.living_item_use_remaining(zombie), Some(own - 11));
        });
    }
    /// A waypoint write is `/waypoint modify`: what it stores is what the command
    /// stores, for the entities the command takes.
    #[test]
    #[ignore = "daily"]
    fn a_waypoint_write_on_a_mob_is_kept_with_the_waypoint() {
        use sunrise_gameplay_common::game::commands_common::Target;
        with_game(|game| {
            floor_and_zombies(game);
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let style = Identifier::parse("example:flag").unwrap();
            assert!(game.living_set_waypoint_style(zombie, &style));
            // The bits above the colour are dropped.
            assert!(game.living_set_waypoint_color(zombie, Some(0xAB12_34CD)));
            let icon = game.waypoint_icon(Target::Mob(12));
            assert_eq!(icon.style, "example:flag");
            assert_eq!(icon.color, Some(0x12_34CD));
            // The other zombie keeps the default icon, and a colour of none
            // gives the colour back.
            assert_eq!(game.waypoint_icon(Target::Mob(13)), Icon::default());
            assert!(game.living_set_waypoint_color(zombie, None));
            let icon = game.waypoint_icon(Target::Mob(12));
            assert_eq!((icon.style.as_str(), icon.color), ("example:flag", None));
        });
    }
    /// A guest's waypoint is told again to the players who see it, with the new
    /// style and colour, and a spin shows in the flags the players around get,
    /// counts down in the guest's ticks and ends when asked to.
    #[test]
    #[ignore = "daily"]
    fn a_guests_waypoint_and_riptide_writes_reach_the_players_around() {
        use sunrise_protocol::{
            entity_data::Value,
            packets::play::{Clientbound as C, waypoint_operation},
        };
        with_game(|game| {
            guest_server(game, false, true);
            let (spinner, _rx) = guest(game, 12);
            let (_, observer) = guest(game, 13);
            let host = game.server.lan.as_mut().unwrap();
            for conn in [12, 13] {
                host.hub.update_guest(conn, |g| g.ticks = 5).unwrap();
            }
            for conn in [12, 13] {
                game.lan_register_player_waypoint(conn);
            }
            let waypoints = |sent: &[C]| -> Vec<(i32, String, Option<i32>)> {
                sent.iter()
                    .filter_map(|p| match p {
                        C::TrackedWaypoint(w) => Some((w.operation, w.style.clone(), w.color)),
                        _ => None,
                    })
                    .collect()
            };

            game.lan_update_waypoints();
            assert_eq!(
                waypoints(&packets(&observer)),
                [(waypoint_operation::TRACK, "minecraft:default".into(), None)]
            );
            let style = Identifier::parse("example:flag").unwrap();
            assert!(game.living_set_waypoint_style(spinner, &style));
            assert!(game.living_set_waypoint_color(spinner, Some(0x12_34AB)));
            game.lan_update_waypoints();
            assert_eq!(
                waypoints(&packets(&observer)),
                [
                    (
                        waypoint_operation::UNTRACK,
                        "minecraft:default".into(),
                        None
                    ),
                    (
                        waypoint_operation::TRACK,
                        "example:flag".into(),
                        // The packet carries the colour as ARGB, opaque.
                        Some(0xFF12_34AB_u32 as i32)
                    ),
                ]
            );

            let spin_flag = |sent: &[C]| -> Option<bool> {
                sent.iter().rev().find_map(|p| match p {
                    C::SetEntityData(p) if p.id == 12 => p.values.iter().find_map(|v| match v {
                        (8, _, Value::Byte(flags)) => Some(flags & 4 != 0),
                        _ => None,
                    }),
                    _ => None,
                })
            };
            assert!(!game.living_set_riptiding(spinner, 20, f32::INFINITY));
            assert!(game.living_set_riptiding(spinner, 20, 8.0));
            game.lan_living_flags();
            sync_guests(game);
            assert_eq!(spin_flag(&packets(&observer)), Some(true));
            game.tick_guests();
            game.as_guest(12, |game| {
                let player = game.player.as_ref().unwrap();
                assert_eq!(player.auto_spin_attack_ticks, 19);
                assert_eq!(player.auto_spin_attack_dmg, 8.0);
            })
            .unwrap();
            // Ended by a write: the damage goes with it, and the flag.
            assert!(game.living_set_riptiding(spinner, 0, 8.0));
            game.lan_living_flags();
            sync_guests(game);
            assert_eq!(spin_flag(&packets(&observer)), Some(false));
            game.as_guest(12, |game| {
                let player = game.player.as_ref().unwrap();
                assert_eq!(player.auto_spin_attack_ticks, 0);
                assert_eq!(player.auto_spin_attack_dmg, 0.0);
            })
            .unwrap();
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_player_keeps_its_stuck_arrows_and_item_use_timer() {
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_gameplay::game::server_side::lan::LanSettings;
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        with_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.server.level_shapes.clone(),
                level_seeds: game.server.levels.0.runtime.seeds(),
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 12,
                    uuid: 12,
                    name: "guest12".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                12,
                [3.0, 64.0, 5.0],
                10,
            );
            guest.placed = true;
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            // A first turn starts a fresh combat record: take it first.
            game.as_guest(12, |_| {}).unwrap();
            let player = game.living(game.entity(12).unwrap()).unwrap();

            // Between its turns the guest's counts are in its combat record.
            assert_eq!(game.living_arrow_count(player), Some(0));
            assert!(game.living_set_arrow_count(player, 4));
            assert!(game.living_set_stinger_count(player, 2));
            assert!(game.living_set_arrow_removal_ticks(player, 7));
            assert!(game.living_set_stinger_removal_ticks(player, 9));
            let stuck = &game.server.guest_combat.0[&12].stuck;
            assert_eq!((stuck.arrows, stuck.stingers), (4, 2));
            assert_eq!((stuck.remove_arrow_time, stuck.remove_stinger_time), (7, 9));
            assert_eq!(game.living_arrow_count(player), Some(4));
            assert_eq!(game.living_stinger_count(player), Some(2));
            assert!(game.living_swing(player, Hand::Off, false));
            // No item in use: nothing to move.
            assert_eq!(game.living_item_use_remaining(player), Some(0));
            assert!(!game.living_set_item_use_remaining(player, 5));

            // Between its turns its use is in its own record too: a bow's is 72000.
            assert_eq!(game.living_used_item_hand(player), None);
            game.as_guest(12, |game| {
                game.offhand = Some(ItemStack::new("minecraft:bow", 1));
            })
            .unwrap();
            assert!(game.living_start_using_item(player, Hand::Off));
            assert_eq!(game.living_used_item_hand(player), Some(Hand::Off));
            assert_eq!(game.living_item_use_remaining(player), Some(72_000));
            assert!(game.living_release_using_item(player));
            assert_eq!(game.living_used_item_hand(player), None);
            assert!(!game.living_release_using_item(player));

            // The acting player's own state: an item in use for 3 of 32 ticks.
            game.as_guest(12, |game| {
                let player = game.living(game.entity(12).unwrap()).unwrap();
                assert_eq!(game.living_arrow_count(player), Some(4));
                game.using = Some((0, 3));
                game.use_duration = 32;
                assert_eq!(game.living_item_use_remaining(player), Some(29));
                assert!(game.living_set_item_use_remaining(player, 1));
                assert_eq!(game.use_duration, 4);
                assert_eq!(game.living_item_use_remaining(player), Some(1));
                game.using = None;
                // Its swing is queued for it when asked: not in the first half.
                game.swing.ticks = 0;
                game.server.guest_packets.clear();
                assert!(game.living_swing(player, Hand::Main, true));
                assert_eq!(game.server.guest_packets.len(), 1);
                game.swing.ticks = 1;
                assert!(!game.living_swing(player, Hand::Main, true));
                assert_eq!(game.server.guest_packets.len(), 1);
            })
            .unwrap();
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_mobs_swing_reaches_clients_with_the_hand_asked_for() {
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_gameplay::game::server_side::lan::LanSettings;
        use sunrise_protocol::packets::{configuration::ClientInformation, play::Clientbound};
        use sunrise_sim::lan::{
            Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig,
        };
        with_resident_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.server.level_shapes.clone(),
                level_seeds: game.server.levels.0.runtime.seeds(),
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 12,
                    uuid: 12,
                    name: "guest12".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                12,
                [3.0, 64.0, 5.0],
                10,
            );
            guest.placed = true;
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(40, kind, Vec3::new(3.0, 64.0, 5.0), 0.0));
            let zombie = game.living(game.entity(40).unwrap()).unwrap();
            // The hand in the swing packet the flush sends after each swing.
            let mut sent = Vec::new();
            for natural in [false, true] {
                for hand in [Hand::Off, Hand::Main] {
                    game.mobs[0].ai.swinging = false;
                    game.mobs[0].ai.swing_time = 0;
                    if natural {
                        // A swing no hand was named for (a melee attack).
                        game.mobs[0].swing();
                    } else {
                        assert!(game.living_swing(zombie, hand, false));
                    }
                    game.lan_flush_effects();
                    while let Ok(Out::Packet(body)) = rx.try_recv() {
                        let mut reader = sunrise_protocol::buf::Reader::new(&body);
                        let id = reader.varint().unwrap();
                        if let Ok(Clientbound::SwingAnimation(p)) =
                            Clientbound::decode(id, reader.rest())
                        {
                            sent.push(p.hand);
                        }
                    }
                    // The swing runs on, so the next one is a new start.
                    game.mobs[0].ai.swing_time = 3;
                    game.lan_flush_effects();
                    if natural {
                        break;
                    }
                }
            }
            assert_eq!(sent, [1, 0, 0]);
        });
    }
}
