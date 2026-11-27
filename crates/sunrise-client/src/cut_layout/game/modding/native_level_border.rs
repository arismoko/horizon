#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::{
        DimensionKey, Hooks, LevelRef, ModEntry, ServerHost, WorldBorderBoundsChangeFinishEvent,
        WorldBorderBoundsChangedEvent, WorldBorderCenterChangedEvent, WorldEpoch,
    };
    use std::sync::{Arc, Mutex, mpsc};
    use sunrise_gameplay::game::{
        native_entity_refs::tests::run_fresh_cases, native_guest_turn_tests::with_guests,
    };
    use sunrise_gameplay_common::game::Dim;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound as C};
    use sunrise_sim::lan::{Out, PlayerSender};

    #[derive(Debug, PartialEq)]
    enum Notice {
        Center(LevelRef),
        Start(LevelRef, Option<i64>),
        Finish(LevelRef, Option<[f64; 4]>),
    }

    struct Observe(Arc<Mutex<Vec<Notice>>>);

    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn world_border_center_changed(
            &self,
            _: &mut dyn ServerHost,
            event: &mut WorldBorderCenterChangedEvent,
        ) {
            self.0.lock().unwrap().push(Notice::Center(event.level));
        }

        fn world_border_bounds_changed(
            &self,
            _: &mut dyn ServerHost,
            event: &mut WorldBorderBoundsChangedEvent,
        ) {
            self.0
                .lock()
                .unwrap()
                .push(Notice::Start(event.level, event.lerp_ticks));
        }

        fn world_border_bounds_change_finish(
            &self,
            host: &mut dyn ServerHost,
            event: &mut WorldBorderBoundsChangeFinishEvent,
        ) {
            self.0
                .lock()
                .unwrap()
                .push(Notice::Finish(event.level, event.level.world_border(host)));
        }
    }

    #[derive(Debug, PartialEq)]
    enum Packet {
        Center(f64, f64),
        Size(f64),
        Lerp(f64, f64, i64),
        WarningBlocks(i32),
        WarningTime(i32),
    }

    fn border_packets(receiver: &mpsc::Receiver<Out>) -> Vec<Packet> {
        let mut packets = Vec::new();
        for out in receiver.try_iter() {
            let bodies = match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            };
            for body in bodies {
                let mut reader = Reader::new(&body);
                let id = reader.varint().unwrap();
                packets.extend(match C::decode(id, reader.rest()).unwrap() {
                    C::SetBorderCenter(p) => Some(Packet::Center(p.x, p.z)),
                    C::SetBorderSize(p) => Some(Packet::Size(p.size)),
                    C::SetBorderLerpSize(p) => {
                        Some(Packet::Lerp(p.old_size, p.new_size, p.lerp_time))
                    }
                    C::SetBorderWarningDistance(p) => Some(Packet::WarningBlocks(p.blocks)),
                    C::SetBorderWarningDelay(p) => Some(Packet::WarningTime(p.delay)),
                    _ => None,
                });
            }
        }
        packets
    }

    #[test]
    #[ignore = "daily"]
    fn border_setters_notify_the_named_level() {
        if run_fresh_cases(
            "cut_layout::game::server_side::native_level_border::tests::border_setters_notify_the_named_level",
            "SUNRISE_BORDER_SETTERS_CHILD",
            &["level-routes"],
        ) {
            return;
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "border_observer",
            Box::new(Observe(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                game.server.world_root = game.options.game_dir.clone().unwrap();
                game.enter_level(Dim::Nether).unwrap();
                game.enter_level(Dim::Overworld).unwrap();
                let overworld = game.level(Dim::Overworld.id()).unwrap();
                let nether = game.level(Dim::Nether.id()).unwrap();
                let stale = LevelRef::new(
                    WorldEpoch::new(nether.epoch().get() + 1),
                    nether.dimension(),
                );
                let old_generation = LevelRef::with_generation(
                    nether.epoch(),
                    nether.dimension(),
                    nether.generation() + 1,
                );
                let unopened = LevelRef::new(nether.epoch(), DimensionKey::END);
                let hub = &mut game.server.lan.as_mut().unwrap().hub;
                let [overworld_guest, nether_guest] = [(12, Dim::Overworld), (13, Dim::Nether)]
                    .map(|(conn, dim)| {
                        let (tx, rx) = mpsc::channel();
                        hub.update_guest(conn, |guest| {
                            guest.dimension = dim.id().to_owned();
                            guest.sender = PlayerSender::new(tx, Arc::default());
                        })
                        .unwrap();
                        rx
                    });

                game.as_console(|game| {
                    assert!(game.run_command("worldborder set 20"));
                    assert!(!game.run_command("worldborder set 20"));
                    assert!(!game.run_command("worldborder set 0.5"));
                    assert!(game.run_command("worldborder center 2.5 -4.5"));
                    assert!(!game.run_command("worldborder center 2.5 -4.5"));
                    assert!(game.run_command("worldborder warning distance 8"));
                    assert!(game.run_command("worldborder warning time 40t"));
                    assert!(game.run_command("worldborder damage amount 2"));
                    assert!(game.run_command("worldborder damage buffer 3"));
                });
                assert!(
                    overworld.set_world_border_center(game, [2.5, -4.5]),
                    "equal center still notifies"
                );
                assert!(nether.set_world_border_center(game, [10.0, 20.0]));
                assert!(nether.set_world_border_size(game, 0.5, -1));
                assert_eq!(nether.world_border(game), Some([9.75, 19.75, 10.25, 20.25]));
                assert!(!nether.set_world_border_center(game, [f64::NAN, 0.0]));
                assert!(!stale.set_world_border_center(game, [0.0, 0.0]));
                assert!(!old_generation.set_world_border_warning(game, 10, 10));
                assert!(!nether.set_world_border_size(game, -4.0, 0));
                assert!(!nether.set_world_border_size(game, f64::INFINITY, 2));
                assert!(!unopened.set_world_border_size(game, 100.0, 0));
                assert_eq!(stale.world_border(game), None);
                assert_eq!(nether.world_border(game), Some([9.75, 19.75, 10.25, 20.25]));
                assert!(
                    nether.set_world_border_size(game, 0.5, 9),
                    "timed equal size is a start"
                );
                assert!(nether.set_world_border_size(game, 4.5, 2));
                assert!(nether.set_world_border_damage(game, -0.5, -3.0));
                assert!(!nether.set_world_border_damage(game, 7.0, f64::NAN));
                assert!(nether.set_world_border_warning(game, -1, -2));
                assert_eq!(game.dim, Dim::Overworld);
                assert_eq!(overworld.world_border(game), Some([-7.5, -14.5, 12.5, 5.5]));
                assert_eq!(
                    (game.border.damage_per_block, game.border.safe_zone),
                    (2.0, 3.0)
                );
                let parked = game.server.levels.0.parked_border(Dim::Nether).unwrap();
                assert_eq!(
                    (
                        parked.size(),
                        parked.target_size(),
                        parked.remaining_ticks()
                    ),
                    (0.5, 4.5, 2)
                );
                assert_eq!((parked.damage_per_block, parked.safe_zone), (-0.5, -3.0));
                assert_eq!((parked.warning_distance, parked.warning_time), (-1, -2));
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        Notice::Start(overworld, None),
                        Notice::Center(overworld),
                        Notice::Center(overworld),
                        Notice::Center(nether),
                        Notice::Start(nether, None),
                        Notice::Start(nether, Some(9)),
                        Notice::Start(nether, Some(2))
                    ],
                );
                assert_eq!(
                    border_packets(&overworld_guest),
                    [
                        Packet::Size(20.0),
                        Packet::Center(2.5, -4.5),
                        Packet::WarningBlocks(8),
                        Packet::WarningTime(40),
                        Packet::Center(2.5, -4.5)
                    ],
                );
                assert_eq!(
                    border_packets(&nether_guest),
                    [
                        Packet::Center(10.0, 20.0),
                        Packet::Size(0.5),
                        Packet::Lerp(0.5, 0.5, 0),
                        Packet::Lerp(0.5, 4.5, 2),
                        Packet::WarningBlocks(-1),
                        Packet::WarningTime(-2)
                    ],
                );

                heard.lock().unwrap().clear();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(13, |guest| {
                        guest.dimension = Dim::Overworld.id().to_owned();
                    })
                    .unwrap();
                assert_eq!(game.levels_to_tick(), [Dim::Overworld]);
                game.tick_server(false).unwrap();
                assert!(heard.lock().unwrap().is_empty());
                assert_eq!(
                    game.server
                        .levels
                        .0
                        .parked_border(Dim::Nether)
                        .unwrap()
                        .remaining_ticks(),
                    1,
                );
                game.tick_server(false).unwrap();
                game.tick_server(false).unwrap();
                assert_eq!(game.dim, Dim::Overworld);
                assert_eq!(game.levels_to_tick(), [Dim::Overworld]);
                assert_eq!(
                    *heard.lock().unwrap(),
                    [Notice::Finish(nether, Some([7.75, 17.75, 12.25, 22.25]))]
                );
                assert!(border_packets(&overworld_guest).is_empty());
                assert!(border_packets(&nether_guest).is_empty());
            })
        });
    }
}
