//! First End exit is a PlayerList respawn; later exits are teleports of the same player.
use mod_api::{
    BlockPart, BlockPos, EntityTravelToDimensionEvent, Flow, GlobalPos, Hooks, Host, Identifier,
    ModEntry, PlayerChangedDimensionEvent, PlayerCloneEvent, PlayerRespawnConfig,
    PlayerRespawnPositionEvent, PlayerRespawnedEvent, PlayerSetSpawnEvent, PlayerTeleportedEvent,
    Registrar, RegistrationContext, RespawnPositionEvent, ServerHost,
};
use std::sync::{Arc, Mutex, mpsc};
use sunrise_core::{dimension::Dimension, random::LegacyRandom};
use sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::world_biomes;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
use sunrise_sim::lan::{Out, guest::InboxItem, hub::game_event};
use {
    crate::game::server_side::server_travel::credits_respawn_tests::server_with_a_nether_anchor,
    sunrise_gameplay::game::server_side::server_travel::credits_respawn_tests::charge_anchor,
    sunrise_gameplay::game::server_side::server_travel::credits_respawn_tests::light_spawn_chunks,
    sunrise_gameplay_common::game::Dim,
    sunrise_gameplay_common::game::server_side::server_travel::credits_respawn_tests::ANCHOR,
    sunrise_gameplay_common::game::server_side::server_travel::credits_respawn_tests::placed_guest,
    sunrise_gameplay_common::game::server_side::server_travel::credits_respawn_tests::received_packets,
};

fn put_guest_in_end_exit(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.dimension = Dim::End.id().into();
        g.seen_credits = true;
    });
    game.enter_level(Dim::End).unwrap();
    game.server
        .streaming
        .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
    assert!(
        game.set_state_named([0, 100, 0], "minecraft:end_portal", &[])
            .unwrap()
    );
}

#[derive(Debug, PartialEq, Eq)]
enum EndHook {
    Travel(String, String),
    Changed(String, String),
    Position(bool),
    CommandTeleport,
    Clone(bool),
    Completed(bool),
}

type Heard = Arc<Mutex<Vec<EndHook>>>;

struct EndTravel {
    veto: Option<Dim>,
    heard: Heard,
}

struct TiltedLanding;

#[mod_api::sunrise_part]
impl BlockPart for TiltedLanding {
    fn respawn_position(&self, _: &dyn Host, event: &mut RespawnPositionEvent<'_>) {
        *event.position = Some(([9.5, 102.0, 9.5], 30.0, 400.0));
    }
}

#[mod_api::__subscriptions]
impl Hooks for EndTravel {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let glass = r.block_key("minecraft:glass");
        r.block_part(glass, TiltedLanding);
    }

    fn player_teleported(&self, _: &mut dyn ServerHost, _: &mut PlayerTeleportedEvent<'_>) {
        self.heard.lock().unwrap().push(EndHook::CommandTeleport);
    }

    fn entity_travel_to_dimension(
        &self,
        _: &mut dyn ServerHost,
        event: &mut EntityTravelToDimensionEvent,
    ) -> Flow {
        self.heard.lock().unwrap().push(EndHook::Travel(
            event.cause.to_owned(),
            event.dimension.to_owned(),
        ));
        if self.veto.is_some_and(|dim| event.dimension == dim.id()) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn player_changed_dimension(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerChangedDimensionEvent<'_>,
    ) {
        self.heard
            .lock()
            .unwrap()
            .push(EndHook::Changed(event.from.to_owned(), event.to.to_owned()));
    }

    fn player_respawn_position(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerRespawnPositionEvent<'_>,
    ) {
        self.heard
            .lock()
            .unwrap()
            .push(EndHook::Position(event.from_end_fight));
        *event.position = [4.5, 98.0, 4.5];
        event.copy_original_spawn_position = false;
    }

    fn player_clone(&self, _: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
        self.heard
            .lock()
            .unwrap()
            .push(EndHook::Clone(event.was_death));
    }

    fn player_respawned(&self, _: &mut dyn ServerHost, event: &mut PlayerRespawnedEvent<'_>) {
        self.heard
            .lock()
            .unwrap()
            .push(EndHook::Completed(event.end_conquered));
    }
}

fn with_end_travel(
    veto: Option<Dim>,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &mpsc::Receiver<Out>, &Heard)
    + Send
    + 'static,
) {
    run_with_game_stack(move || {
        let heard = Heard::default();
        let mods: mod_api::Mods = vec![ModEntry::new(
            "end-travel",
            Box::new(EndTravel {
                veto,
                heard: heard.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            let mut resources = None;
            let (_scratch, mut game, rx) = server_with_a_nether_anchor(&mut resources);
            test(&mut game, &rx, &heard);
        });
    });
}

#[test]
#[ignore = "daily"]
fn credits_respawn_is_not_subject_to_the_end_travel_veto() {
    with_end_travel(Some(Dim::Nether), |game, rx, heard| {
        game.enter_level(Dim::Nether).unwrap();
        light_spawn_chunks(game);
        charge_anchor(game);
        put_guest_in_end_exit(game);

        // As a control, the same listener can veto an ordinary teleport through this portal.
        game.handle_server_portal(2);
        assert_eq!(
            std::mem::take(&mut *heard.lock().unwrap()),
            [EndHook::Travel(
                "end_portal".into(),
                Dim::Nether.id().into()
            )]
        );
        assert!(!game.on_server_trip(2));

        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.seen_credits = false);
        game.handle_server_portal(2);
        let host = game.server.lan.as_mut().unwrap();
        assert!(host.hub.guests[0].won_game);
        assert!(!host.hub.guests[0].placed);
        assert!(
            received_packets(rx)
                .any(|packet| matches!(packet, C::GameEvent(event) if event.event == 4))
        );
        host.hub.guests.update(0, |g| {
            g.inbox.push_back(InboxItem::Packet(
                S::ClientCommand(play::ClientCommand { action: 0 }),
                None,
            ));
        });
        game.tick_lan_packets();
        let host = game.server.lan.as_ref().unwrap();
        let player = &host.hub.guests[0];
        assert!(!player.won_game);
        assert!(player.placed);
        assert_eq!(player.dimension, Dim::Nether.id());
        assert_eq!(player.position, [4.5, 98.0, 4.5]);
        assert_eq!(
            *heard.lock().unwrap(),
            [
                EndHook::Position(true),
                EndHook::Clone(false),
                EndHook::Completed(true)
            ]
        );
    });
}

fn ordinary_end_exit(available: bool, veto: Option<Dim>, spectator: bool) {
    with_end_travel(veto, move |game, rx, heard| {
        light_spawn_chunks(game);
        assert!(game.ui.rules.set_string("respawn_radius", "0"));
        assert!(
            game.set_state_named([0, 99, 0], "minecraft:stone", &[])
                .unwrap()
        );
        put_guest_in_end_exit(game);
        if spectator {
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.game_mode = sunrise_sim::lan::guest::SPECTATOR;
            });
            assert!(game.spectator_use_portal(2, 0, [0, 100, 0]));
        } else {
            game.handle_server_portal(2);
        }
        // The saved-point chunks aren't ready yet. The level being searched is not a move.
        assert_eq!(game.command_guest(2).unwrap().dimension, Dim::End.id());
        assert!(heard.lock().unwrap().is_empty());
        assert!(!received_packets(rx).any(|packet| matches!(packet, C::Respawn(_))));

        game.enter_level(Dim::Nether).unwrap();
        light_spawn_chunks(game);
        let charged = available.then(|| charge_anchor(game));
        game.tick_server_travel();
        if let Some(charged) = charged {
            let [x, y, z] = ANCHOR;
            assert_eq!(
                game.world.block(x, y, z),
                charged,
                "teleporting does not consume a charge"
            );
        }
        assert!(!game.on_server_trip(2));
        let player = game.command_guest(2).unwrap();
        let (dim, at) = if available {
            (Dim::Nether, [10.5, 101.0, 10.5])
        } else {
            (Dim::Overworld, [0.5, 100.0, 0.5])
        };
        assert_eq!(
            player
                .respawn_elsewhere
                .as_ref()
                .map(|point| (point.0.as_str(), point.1)),
            Some((Dim::Nether.id(), ANCHOR)),
            "even a missing point still belongs to the same player after a teleport",
        );
        let mut transitions = Vec::new();
        let (mut unavailable, mut positions, mut portal_sounds) = (0, 0, 0);
        for packet in received_packets(rx) {
            match packet {
                C::Respawn(packet) => transitions.push((packet.spawn.dimension, packet.keep)),
                C::PlayerPosition(_) => positions += 1,
                C::GameEvent(event) if event.event == game_event::NO_RESPAWN_BLOCK_AVAILABLE => {
                    unavailable += 1;
                }
                C::LevelEvent(event) if event.kind == 1032 => portal_sounds += 1,
                _ => {}
            }
        }
        assert_eq!(
            portal_sounds, 0,
            "outbound player transitions use DO_NOTHING"
        );
        let mut expected = vec![EndHook::Travel("end_portal".into(), dim.id().into())];
        if veto == Some(dim) {
            assert_eq!(player.dimension, Dim::End.id());
            assert_eq!(player.position, [0.5, 100.0, 0.5]);
            assert!(transitions.is_empty());
            assert_eq!((positions, unavailable), (0, 0));
        } else {
            assert_eq!(player.dimension, dim.id());
            assert_eq!(player.position, at);
            assert_eq!(
                transitions,
                [(dim.id().to_owned(), 3)],
                "one move, keeping player data"
            );
            assert_eq!((positions, unavailable), (1, usize::from(!available)));
            expected.push(EndHook::Changed(Dim::End.id().into(), dim.id().into()));
        }
        assert_eq!(*heard.lock().unwrap(), expected);
    });
}

#[test]
#[ignore = "daily"]
fn ordinary_end_exit_to_a_valid_point_only_teleports() {
    ordinary_end_exit(true, None, false);
}

#[test]
#[ignore = "daily"]
fn ordinary_end_exit_with_a_missing_point_only_teleports() {
    ordinary_end_exit(false, None, false);
}

#[test]
#[ignore = "daily"]
fn a_travel_veto_can_refuse_a_spectators_resolved_end_exit() {
    ordinary_end_exit(false, Some(Dim::Overworld), true);
}

#[test]
#[ignore = "daily"]
fn a_cold_end_exit_keeps_ticking_the_source_player_before_the_fallback_veto() {
    with_end_travel(Some(Dim::Overworld), |game, rx, heard| {
        // The saved Nether point is missing, and the fallback Overworld column is cold.
        assert!(!game.server.streaming.is_lit(0, 0));
        game.enter_level(Dim::Nether).unwrap();
        light_spawn_chunks(game);
        put_guest_in_end_exit(game);
        let slot = game
            .as_guest(2, |g| {
                let slot = g.held_slot();
                *g.stack_in_mut(slot) =
                    Some(sunrise_items::items::ItemStack::new("minecraft:bow", 1));
                assert!(g.start_using_item());
                g.add_cooldown(Arc::from("minecraft:ender_pearl"), 1);
                g.server.destroy_delay = 2;
                g.player.as_mut().unwrap().health.tick_count = 10;
                slot
            })
            .unwrap();

        game.handle_server_portal(2);
        assert!(heard.lock().unwrap().is_empty());
        game.tick_guests();
        assert_eq!(
            game.as_guest(2, |g| {
                (
                    g.player.as_ref().unwrap().health.tick_count,
                    g.server.destroy_delay,
                    g.using,
                    g.cooldowns.is_on_cooldown("minecraft:ender_pearl"),
                )
            }),
            Some((11, 1, Some((slot, 1)), false))
        );

        game.enter_level(Dim::Overworld).unwrap();
        light_spawn_chunks(game);
        assert!(game.ui.rules.set_string("respawn_radius", "0"));
        assert!(
            game.set_state_named([0, 99, 0], "minecraft:stone", &[])
                .unwrap()
        );
        game.tick_server_travel();
        assert_eq!(
            *heard.lock().unwrap(),
            [EndHook::Travel(
                "end_portal".into(),
                Dim::Overworld.id().into()
            )]
        );
        let player = game.command_guest(2).unwrap();
        assert_eq!(player.dimension, Dim::End.id());
        assert_eq!(player.position, [0.5, 100.0, 0.5]);
        assert!(!received_packets(rx).any(|packet| matches!(
            packet,
            C::Respawn(_)
                | C::PlayerPosition(_)
                | C::GameEvent(play::GameEvent {
                    event: game_event::NO_RESPAWN_BLOCK_AVAILABLE,
                    ..
                })
        )));
    });
}

fn cold_legacy_end_exit(point: [f64; 3], floor: [i32; 3], landing: [f64; 3]) {
    with_end_travel(None, move |game, rx, heard| {
        game.level.world_spawn_angle = [20.0, 5.0];
        assert!(!game.server.streaming.is_lit(0, 0));
        put_guest_in_end_exit(game);
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.respawn_elsewhere = None;
            g.spawn = Some(point);
            (g.yaw, g.pitch) = (-67.0, 27.0);
        });

        game.handle_server_portal(2);
        assert_eq!(game.command_guest(2).unwrap().dimension, Dim::End.id());
        assert!(heard.lock().unwrap().is_empty());
        assert!(
            !received_packets(rx)
                .any(|packet| matches!(packet, C::Respawn(_) | C::PlayerPosition(_)))
        );

        game.enter_level(Dim::Overworld).unwrap();
        light_spawn_chunks(game);
        assert!(game.ui.rules.set_string("respawn_radius", "1"));
        assert!(game.set_state_named(floor, "minecraft:stone", &[]).unwrap());
        game.tick_server_travel();
        game.tick_server_travel();

        let player = game.command_guest(2).unwrap();
        assert_eq!(player.dimension, Dim::Overworld.id());
        assert_eq!(player.position, landing);
        assert_eq!([player.yaw, player.pitch], [-67.0, 27.0]);
        let packets: Vec<_> = received_packets(rx).collect();
        assert_eq!(
            packets
                .iter()
                .filter(|p| matches!(p, C::Respawn(_)))
                .count(),
            1
        );
        let positions: Vec<_> = packets
            .iter()
            .filter_map(|p| match p {
                C::PlayerPosition(p) => Some((p.position, p.yaw, p.pitch)),
                _ => None,
            })
            .collect();
        assert_eq!(positions, [(landing, -67.0, 27.0)]);
        assert_eq!(
            *heard.lock().unwrap(),
            [
                EndHook::Travel("end_portal".into(), Dim::Overworld.id().into()),
                EndHook::Changed(Dim::End.id().into(), Dim::Overworld.id().into()),
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn an_end_exit_resolves_a_cold_legacy_personal_column_and_keeps_facing() {
    cold_legacy_end_exit([9.5, 64.0, 9.5], [9, 112, 9], [9.5, 113.0, 9.5]);
}

#[test]
#[ignore = "daily"]
fn an_end_exit_searches_around_a_cold_legacy_point_on_the_shared_spawn_column() {
    // The shared spawn column is empty; only the column east of it has a floor.
    cold_legacy_end_exit([0.5, 64.0, 0.5], [1, 112, 0], [1.5, 113.0, 0.5]);
}

#[test]
#[ignore = "daily"]
fn an_end_exit_to_a_point_in_the_end_sends_no_respawn_packet() {
    with_end_travel(None, |game, rx, heard| {
        game.resolve_parts().unwrap();
        game.enter_level(Dim::End).unwrap();
        light_spawn_chunks(game);
        put_guest_in_end_exit(game);
        assert!(
            game.set_state_named([9, 100, 9], "minecraft:glass", &[])
                .unwrap()
        );
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.respawn_elsewhere = Some((Dim::End.id().to_owned(), [9, 100, 9], 0.0, 0.0, false));
        });
        game.handle_server_portal(2);
        assert!(!game.on_server_trip(2));
        let player = game.command_guest(2).unwrap();
        assert_eq!(player.position, [9.5, 102.0, 9.5]);
        assert_eq!(
            *heard.lock().unwrap(),
            [EndHook::Travel("end_portal".into(), Dim::End.id().into())]
        );
        let mut positions = Vec::new();
        for packet in received_packets(rx) {
            match packet {
                C::PlayerPosition(p) => positions.push((p.position, p.yaw, p.pitch)),
                C::Respawn(_) | C::LevelEvent(play::LevelEvent { kind: 1032, .. }) => {
                    panic!("same-level End exit sends only a position transition");
                }
                _ => {}
            }
        }
        assert_eq!(positions, [([9.5, 102.0, 9.5], 30.0, 90.0)]);
        assert_eq!(player.pitch, 90.0);
    });
}

const ANCHOR_DEPLETE: &str = "minecraft:block.respawn_anchor.deplete";
const REDIRECTED: [f64; 3] = [200.5, 72.0, 200.5];
const CLONE_POINT: [i32; 3] = [44, 75, 21];
const LATE_ANCHOR: [i32; 3] = [170, 90, 170];

#[derive(Clone, Copy)]
enum AnchorRules {
    Redirect,
    VetoCopy,
    SetInCompletion,
    EditOriginal,
}

fn forced_point(dim: Dim, [x, y, z]: [i32; 3]) -> PlayerRespawnConfig {
    PlayerRespawnConfig {
        position: GlobalPos::new(Identifier::parse(dim.id()).unwrap(), BlockPos::new(x, y, z)),
        yaw: 0.0,
        forced: true,
    }
}

#[mod_api::__subscriptions]
impl Hooks for AnchorRules {
    fn player_respawn_position(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerRespawnPositionEvent<'_>,
    ) {
        match self {
            Self::Redirect => {
                *event.level = Dimension::Overworld;
                *event.position = REDIRECTED;
            }
            Self::EditOriginal => {
                assert!(
                    event
                        .player
                        .set_respawn_config(host, Some(forced_point(Dim::Overworld, CLONE_POINT)))
                );
                event.copy_original_spawn_position = true;
            }
            _ => {}
        }
    }

    fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
        if matches!(self, Self::VetoCopy) {
            assert!(
                host.player(event.player)
                    .unwrap()
                    .set_respawn_config(host, Some(forced_point(Dim::Overworld, CLONE_POINT)))
            );
        }
    }

    fn player_respawned(&self, host: &mut dyn ServerHost, event: &mut PlayerRespawnedEvent<'_>) {
        if matches!(self, Self::SetInCompletion) {
            assert!(
                host.player(event.player)
                    .unwrap()
                    .set_respawn_config(host, Some(forced_point(Dim::End, LATE_ANCHOR)))
            );
        }
    }

    fn player_set_spawn(&self, _: &mut dyn ServerHost, event: &mut PlayerSetSpawnEvent) -> Flow {
        if matches!(self, Self::VetoCopy) && event.pos == Some(ANCHOR) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn with_anchor_death(
    rules: AnchorRules,
    test: impl FnOnce(
        &sunrise_gameplay::game::ServerGame<'_>,
        &mpsc::Receiver<Out>,
        &mpsc::Receiver<Out>,
    ) + Send
    + 'static,
) {
    run_with_game_stack(move || {
        let mods = vec![ModEntry::new("anchor-rules", Box::new(rules))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let (_scratch, mut game, rx) = server_with_a_nether_anchor(&mut resources);
            light_spawn_chunks(&mut game);
            *game.world.random = LegacyRandom::new(12345);
            if matches!(rules, AnchorRules::SetInCompletion) {
                game.enter_level(Dim::End).unwrap();
                let [x, y, z] = LATE_ANCHOR;
                let registry = game.pack.assets.registry;
                let anchor = registry.default_state("minecraft:respawn_anchor").unwrap();
                let mut chunk = Chunk::with_layout(x >> 4, z >> 4, game.world.layout());
                chunk.set(x & 15, y, z & 15, anchor.0);
                game.server
                    .save
                    .0
                    .save(&[Arc::new(chunk)], registry, world_biomes!(game))
                    .unwrap();
                assert!(
                    !game.world.loaded(x, z),
                    "writing the fixture does not load its chunk"
                );
            }
            game.enter_level(Dim::Nether).unwrap();
            light_spawn_chunks(&mut game);
            charge_anchor(&mut game);
            let (bystander, other) = placed_guest(3, Dim::Nether, [11.5, 101.0, 10.5]);
            game.server.lan.as_mut().unwrap().hub.guests.push(bystander);
            game.enter_level(Dim::Overworld).unwrap();
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.dead = true;
                g.health = 0.0;
            });
            if matches!(rules, AnchorRules::EditOriginal) {
                let player = game.player(game.command_guest(2).unwrap().uuid).unwrap();
                assert!(!game.player_set_respawn_config(
                    player,
                    Some(forced_point(Dim::Overworld, CLONE_POINT))
                ));
            }
            game.guest_respawn(2);
            assert!(!game.on_server_trip(2));
            assert!(game.server.lan.as_ref().unwrap().hub.guests[0].placed);
            test(&game, &rx, &other);
        });
    });
}

fn anchor_sounds(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    rx: &mpsc::Receiver<Out>,
) -> Vec<play::Sound> {
    let registry_id = game
        .server
        .lan
        .as_ref()
        .unwrap()
        .hub
        .registry_id("minecraft:sound_event", ANCHOR_DEPLETE);
    received_packets(rx)
        .filter_map(|packet| {
            let C::Sound(sound) = packet else { return None };
            let is_deplete = match &sound.sound.1 {
                Some((name, _)) => name.as_str() == ANCHOR_DEPLETE,
                None => registry_id == Some(sound.sound.0),
            };
            is_deplete.then_some(sound)
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn a_forced_spawnpoint_on_a_respawn_anchor_still_depletes_with_its_sound() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        // PlayerList plays the sound even for a forced point. The anchor itself keeps its charge.
        let mut resources = None;
        let (_scratch, mut game, rx) = server_with_a_nether_anchor(&mut resources);
        game.enter_level(Dim::Nether).unwrap();
        light_spawn_chunks(&mut game);
        let charged = charge_anchor(&mut game);
        game.build_registries();
        assert!(game.as_console(|g| g.run_command("spawnpoint Guest2 10 100 10")));
        let expected = (Dim::Nether.id().to_owned(), ANCHOR, 0.0, 0.0, true);
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        assert_eq!(hub.guests[0].respawn_elsewhere.as_ref(), Some(&expected));
        hub.update_guest(2, |g| {
            g.dead = true;
            g.health = 0.0;
        });
        game.guest_respawn(2);
        let guest = game.command_guest(2).unwrap();
        assert!(!guest.dead);
        assert_eq!(guest.dimension, Dim::Nether.id());
        assert_eq!(guest.respawn_elsewhere.as_ref(), Some(&expected));
        let sounds = anchor_sounds(&game, &rx);
        assert_eq!(sounds.len(), 1);
        let sound = &sounds[0];
        assert_eq!([sound.x, sound.y, sound.z], ANCHOR.map(|c| c * 8));
        assert_eq!((sound.source, sound.volume, sound.pitch), (4, 1.0, 1.0));
        assert_eq!(game.world.block(ANCHOR[0], ANCHOR[1], ANCHOR[2]), charged);
    });
}

#[test]
#[ignore = "daily"]
fn redirected_respawn_sends_the_anchor_sound_only_to_its_player() {
    with_anchor_death(AnchorRules::Redirect, |game, rx, other| {
        let player = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(player.dimension, Dim::Overworld.id());
        assert_eq!(player.position, REDIRECTED);
        let sounds = anchor_sounds(game, rx);
        assert_eq!(sounds.len(), 1);
        let sound = &sounds[0];
        assert_eq!([sound.x, sound.y, sound.z], [80, 800, 80]);
        assert_eq!((sound.source, sound.volume, sound.pitch), (4, 1.0, 1.0));
        // Java Random(12345).nextLong(), from the placement level, not the anchor's level.
        assert_eq!(sound.seed, 6_674_089_274_190_705_457);
        assert!(
            anchor_sounds(game, other).is_empty(),
            "nearby players do not hear this packet"
        );
    });
}

#[test]
#[ignore = "daily"]
fn vetoing_the_spawn_copy_keeps_the_clone_point_and_suppresses_anchor_sound() {
    with_anchor_death(AnchorRules::VetoCopy, |game, rx, other| {
        let player = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(player.dimension, Dim::Nether.id());
        assert_eq!(player.position, [10.5, 101.0, 10.5]);
        assert_eq!(
            player
                .respawn_elsewhere
                .as_ref()
                .map(|point| (point.0.as_str(), point.1)),
            Some((Dim::Overworld.id(), CLONE_POINT)),
        );
        assert!(anchor_sounds(game, rx).is_empty());
        assert!(anchor_sounds(game, other).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn completion_can_select_an_anchor_in_an_unloaded_chunk() {
    with_anchor_death(AnchorRules::SetInCompletion, |game, rx, other| {
        let player = game.command_guest(2).unwrap();
        assert_eq!(player.dimension, Dim::Nether.id());
        assert_eq!(
            player
                .respawn_elsewhere
                .as_ref()
                .map(|point| (point.0.as_str(), point.1)),
            Some((Dim::End.id(), LATE_ANCHOR))
        );
        let sounds = anchor_sounds(game, rx);
        let [sound] = sounds.as_slice() else {
            panic!("expected one sound from the saved End anchor")
        };
        assert_eq!([sound.x, sound.y, sound.z], [1360, 720, 1360]);
        assert!(anchor_sounds(game, other).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn the_dead_original_can_change_the_point_copied_to_the_replacement() {
    with_anchor_death(AnchorRules::EditOriginal, |game, _, _| {
        let player = game.command_guest(2).unwrap();
        assert_eq!(player.dimension, Dim::Nether.id());
        assert_eq!(
            player.position,
            [10.5, 101.0, 10.5],
            "this respawn's destination was already found"
        );
        assert_eq!(
            player
                .respawn_elsewhere
                .as_ref()
                .map(|point| (point.0.as_str(), point.1)),
            Some((Dim::Overworld.id(), CLONE_POINT)),
            "the replacement copies the original's edited point"
        );
    });
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
