//! `level.relight_chunks` (queued, not lit synchronously) and `chunk.resend` through the public
//! handles, covering active and parked server levels as well as each named refusal.

use mod_api::{ChunkRefusal, LevelRef, LightLayer, WorldEpoch};
use std::sync::Arc;
use sunrise_gameplay::game::native_entity_refs::tests::{run_fresh_cases, with_game};
use sunrise_gameplay::game::server_side::native_guest_turn_tests::with_guests;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use sunrise_level::chunk::Chunk;

/// A stone roof over the chunk (100, 100), all eight neighbours loaded around it.
fn roofed(game: &mut sunrise_gameplay_common::game::CommonGame<'_>) {
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for dz in -1..=1 {
        for dx in -1..=1 {
            let mut chunk = Chunk::empty(100 + dx, 100 + dz);
            if dx == 0 && dz == 0 {
                for (x, z) in (0..16).flat_map(|x| (0..16).map(move |z| (x, z))) {
                    chunk.set(x, 100, z, stone.0);
                }
            }
            game.world
                .chunks
                .insert((100 + dx, 100 + dz), Arc::new(chunk));
        }
    }
}

fn stale(level: LevelRef) -> LevelRef {
    LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension())
}

/// A relight is queued at once, nothing is lit during the call, and the count is the columns
/// queued (the loaded ring included); a chunk not itself loaded queues none (not even loaded
/// neighbours), and large batches are accepted without artificial per-call or queue caps.
#[test]
#[ignore = "daily"]
fn a_mod_queues_the_chunks_it_wrote() {
    with_game(|game| {
        roofed(game);
        let level = game.level(game.dim.id()).unwrap();
        game.world.dirty_light.clear();
        // Unloaded chunk adjacent to a loaded chunk queues nothing.
        assert_eq!(level.relight_chunks(game, &[[102, 100]]), Ok(0));
        assert!(game.world.dirty_light.is_empty());
        assert_eq!(level.relight_chunks(game, &[[100, 100]]), Ok(9));
        assert_eq!(game.world.dirty_light.len(), 9);
        assert!(game.world.dirty_light.contains(&(100, 100)));
        // Asked again before the light update: nothing new is queued.
        assert_eq!(level.relight_chunks(game, &[[100, 100]]), Ok(0));
        assert_eq!(level.relight_chunks(game, &[[400, 400]]), Ok(0));
        assert_eq!(game.world.dirty_light.len(), 9);
    });
}

/// Calls past the former 256-chunk and 4096-queued limits are accepted and scheduled on the
/// owning level's queue without refusal.
#[test]
#[ignore = "daily"]
fn relight_chunks_accepts_large_batches_without_synthetic_quotas() {
    with_game(|game| {
        roofed(game);
        let level = game.level(game.dim.id()).unwrap();
        game.world.dirty_light.clear();
        game.world
            .dirty_light
            .extend((0..4200).map(|i| (1000 + i, 1000)));
        let mut chunks: Vec<[i32; 2]> = (0..300).map(|i| [400 + i, 400]).collect();
        chunks.push([100, 100]);
        assert_eq!(level.relight_chunks(game, &chunks), Ok(9));
        assert_eq!(game.world.dirty_light.len(), 4209);
        assert!(game.world.dirty_light.contains(&(100, 100)));
    });
}

/// A handle of an earlier open of the world is `Stale`, for the level and for a chunk of it.
#[test]
#[ignore = "daily"]
fn a_stale_handle_is_refused_by_name() {
    with_game(|game| {
        roofed(game);
        let level = game.level(game.dim.id()).unwrap();
        assert_eq!(
            stale(level).relight_chunks(game, &[[100, 100]]),
            Err(ChunkRefusal::Stale)
        );
        let chunk = mod_api::__private::game::chunk_ref(stale(level), [100, 100]);
        assert_eq!(chunk.resend(game), Err(ChunkRefusal::Stale));
    });
}

/// A parked open level accepts relights on its own world and advances them without a level tick.
#[test]
#[ignore = "daily"]
fn parked_level_relights_on_the_owning_dimension() {
    with_game(|game| {
        roofed(game);
        let overworld = game.level("minecraft:overworld").unwrap();
        game.world.dirty_light.clear();
        game.enter_level(Dim::Nether).unwrap();
        game.world.dirty_light.clear();

        assert_eq!(overworld.relight_chunks(game, &[[100, 100]]), Ok(9));
        assert!(game.world.dirty_light.is_empty());
        assert_eq!(
            game.server
                .levels
                .0
                .parked_world_of("minecraft:overworld")
                .unwrap()
                .dirty_light
                .len(),
            9
        );

        // Drive parked light updates until the queued roofed column finishes lighting.
        // `with_game` creates `Game` on the title screen with `streaming.paused = true`;
        // unpause the parked Overworld streaming pool as an open world does.
        game.server
            .levels
            .0
            .parked_mut(Dim::Overworld)
            .unwrap()
            .streaming
            .paused = false;
        for _ in 0..40 {
            game.update_parked_light(&[Dim::Nether]);
            if !game
                .server
                .levels
                .0
                .parked
                .get(&Dim::Overworld)
                .unwrap()
                .streaming
                .has_pending_light(
                    &game
                        .server
                        .levels
                        .0
                        .parked
                        .get(&Dim::Overworld)
                        .unwrap()
                        .world,
                )
            {
                break;
            }
            game.server
                .levels
                .0
                .parked_mut(Dim::Overworld)
                .unwrap()
                .streaming
                .wait(std::time::Duration::from_millis(50));
        }
        // Under the stone roof at y = 100, sky light at (1608, 99, 1608) is no longer 15.
        let sky_under_roof = game.level_brightness(overworld, LightLayer::Sky, [1608, 99, 1608]);
        assert!(
            sky_under_roof.is_some_and(|v| v < 15),
            "expected parked relight to darken sky light beneath the roof, got {sky_under_roof:?}"
        );
    });
}

/// A live block log and an in-memory guest are not proof the guest was sent the chunk.
#[test]
#[ignore = "daily"]
fn parked_chunk_resend_checks_sent_watchers_on_owning_dimension() {
    with_guests(|game| {
        roofed(game);
        let overworld = game.level("minecraft:overworld").unwrap();
        game.world.block_log = Some(Vec::new());
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.dimension = "minecraft:overworld".into();
            g.center = Some((100, 100));
            g.set_address(sunrise_sim::lan::conn::MEMORY_ADDRESS);
        });
        game.enter_level(Dim::Nether).unwrap();

        let chunk = mod_api::__private::game::chunk_ref(overworld, [100, 100]);
        assert_eq!(chunk.resend(game), Ok(()));
        let world = game
            .server
            .levels
            .0
            .parked_world_of("minecraft:overworld")
            .unwrap();
        assert!(world.block_log.is_some());
        assert!(world.chunk_resends.is_empty());
        assert!(world.bulk_log.is_empty(), "no far-terrain handoff queued");

        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.sent_chunks.insert((100, 100));
        });
        assert_eq!(chunk.resend(game), Ok(()));
        let world = game
            .server
            .levels
            .0
            .parked_world_of("minecraft:overworld")
            .unwrap();
        assert!(world.chunk_resends.contains_key(&(100, 100)));
        assert!(game.world.chunk_resends.is_empty());
    });
}

/// An interior holder receives the corrected light in the chunk packet, not a separate light
/// update. The block values stay the same; only the stored light was stale.
#[test]
#[ignore = "daily"]
fn resend_waits_for_the_relight_before_sending_to_an_interior_holder() {
    use std::time::{Duration, Instant};
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
    use sunrise_sim::lan::{Out, PlayerSender};

    if run_fresh_cases(
        "cut_layout::game::native_chunk_relight_tests::resend_waits_for_the_relight_before_sending_to_an_interior_holder",
        "SUNRISE_CHUNK_RESEND_CHILD",
        &["game"],
    ) {
        return;
    }
    with_guests(|game| {
        roofed(game);
        let level = game.level("minecraft:overworld").unwrap();
        let pos = [1608, 110, 1608];
        let key = (100, 100);
        let torch = game
            .pack
            .assets
            .registry
            .default_state("minecraft:torch")
            .unwrap();
        let column = Arc::make_mut(game.world.chunks.get_mut(&key).unwrap());
        column.set(pos[0], pos[1], pos[2], torch.0);
        column.lit = true;
        assert_eq!(
            game.level_brightness(level, LightLayer::Block, pos),
            Some(0)
        );

        let (tx, rx) = std::sync::mpsc::channel();
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.sender = PlayerSender::new(tx, Arc::default());
            g.dimension = "minecraft:overworld".into();
            g.center = Some(key);
            g.sent_chunks
                .extend((99..=101).flat_map(|x| (99..=101).map(move |z| (x, z))));
        });
        game.world.light_log = Some(Default::default());
        game.server.streaming.paused = false;
        assert_eq!(level.relight_chunks(game, &[[key.0, key.1]]), Ok(9));
        assert_eq!(
            game.chunk(level, [key.0, key.1]).unwrap().resend(game),
            Ok(())
        );
        game.lan_broadcast_changes();
        assert!(rx.try_recv().is_err(), "light was still queued");
        game.server.streaming.update_light(&mut game.common.world);
        assert!(game.server.streaming.has_column_light_job(key));
        game.lan_broadcast_changes();
        assert!(rx.try_recv().is_err(), "worker light was not yet applied");

        let deadline = Instant::now() + Duration::from_secs(10);
        while game.server.streaming.has_column_light_job(key) {
            assert!(Instant::now() < deadline, "light worker did not finish");
            game.server.streaming.wait(Duration::from_millis(50));
            game.server.streaming.update_light(&mut game.common.world);
        }
        game.lan_broadcast_changes();
        let Out::Packet(body) = rx.try_recv().expect("resend reaches the interior holder") else {
            panic!("expected a chunk packet");
        };
        let mut reader = Reader::new(&body);
        let id = reader.varint().unwrap();
        let Clientbound::LevelChunkWithLight(packet) =
            Clientbound::decode(id, reader.rest()).unwrap()
        else {
            panic!("expected LevelChunkWithLight");
        };
        assert_eq!((packet.x, packet.z), key);
        let mut received = Chunk::empty(packet.x, packet.z);
        let layers = packet.light.layers(game.world.layout().sections() + 2);
        sunrise_level::light::light_from_layers(&mut received, &layers, true);
        assert_eq!(received.light(pos[0], pos[1], pos[2]) & 15, 14);
        assert!(
            rx.try_recv().is_err(),
            "the interior holder needed only the resend"
        );
    });
}

/// A later `check_light` on an ordinary air cell after `update_light` does not keep an earlier
/// resend queued once the light work that preceded the resend has completed.
#[test]
#[ignore = "daily"]
fn later_light_check_does_not_prevent_chunk_resend() {
    use std::time::{Duration, Instant};
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
    use sunrise_sim::lan::{Out, PlayerSender};

    if run_fresh_cases(
        "cut_layout::game::native_chunk_relight_tests::later_light_check_does_not_prevent_chunk_resend",
        "SUNRISE_CHUNK_RESEND_MARKER_CHILD",
        &["game"],
    ) {
        return;
    }
    with_guests(|game| {
        roofed(game);
        let level = game.level("minecraft:overworld").unwrap();
        let torch_pos = [1608, 110, 1608];
        let air_pos = [1608, 105, 1608];
        let key = (100, 100);
        let torch = game
            .pack
            .assets
            .registry
            .default_state("minecraft:torch")
            .unwrap();
        let column = Arc::make_mut(game.world.chunks.get_mut(&key).unwrap());
        column.set(torch_pos[0], torch_pos[1], torch_pos[2], torch.0);
        column.lit = true;

        let (tx, rx) = std::sync::mpsc::channel();
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.sender = PlayerSender::new(tx, Arc::default());
            g.dimension = "minecraft:overworld".into();
            g.center = Some(key);
            g.sent_chunks
                .extend((99..=101).flat_map(|x| (99..=101).map(move |z| (x, z))));
        });
        game.world.light_log = Some(Default::default());
        game.server.streaming.paused = false;

        // 1. Queue a relight and resend, wait for the whole-column job to apply, then queue a
        // fresh no-change air light check before broadcasting: the resend must still go out.
        assert_eq!(level.relight_chunks(game, &[[key.0, key.1]]), Ok(9));
        assert_eq!(
            game.chunk(level, [key.0, key.1]).unwrap().resend(game),
            Ok(())
        );
        game.server.streaming.update_light(&mut game.common.world);
        let deadline = Instant::now() + Duration::from_secs(10);
        while game.server.streaming.has_column_light_job(key)
            || game.world.dirty_light.contains(&key)
        {
            assert!(Instant::now() < deadline, "light worker did not finish");
            game.server.streaming.wait(Duration::from_millis(50));
            game.server.streaming.update_light(&mut game.common.world);
        }
        assert!(game.level_check_light(level, air_pos));
        assert!(!game.world.light_edits.is_empty());
        game.lan_broadcast_changes();
        let Out::Packet(body) = rx
            .try_recv()
            .expect("later pending check must not block completed resend")
        else {
            panic!("expected a chunk packet");
        };
        let mut reader = Reader::new(&body);
        let id = reader.varint().unwrap();
        let Clientbound::LevelChunkWithLight(packet) =
            Clientbound::decode(id, reader.rest()).unwrap()
        else {
            panic!("expected LevelChunkWithLight");
        };
        assert_eq!((packet.x, packet.z), key);
        let mut received = Chunk::empty(packet.x, packet.z);
        let layers = packet.light.layers(game.world.layout().sections() + 2);
        sunrise_level::light::light_from_layers(&mut received, &layers, true);
        assert_eq!(
            received.light(torch_pos[0], torch_pos[1], torch_pos[2]) & 15,
            14
        );
        assert!(rx.try_recv().is_err());

        // 2. A resend queued while a pending `check_light` exists waits for that check's
        // `update_light`, and then still sends even if another check is queued before broadcast.
        assert!(!game.world.light_edits.is_empty());
        assert_eq!(
            game.chunk(level, [key.0, key.1]).unwrap().resend(game),
            Ok(())
        );
        game.lan_broadcast_changes();
        assert!(
            rx.try_recv().is_err(),
            "earlier pending check not drained yet"
        );
        game.server.streaming.update_light(&mut game.common.world);
        assert!(game.level_check_light(level, air_pos));
        assert!(!game.world.light_edits.is_empty());
        game.lan_broadcast_changes();
        let Out::Packet(body) = rx
            .try_recv()
            .expect("later check after update_light must not block resend")
        else {
            panic!("expected a chunk packet");
        };
        let mut reader = Reader::new(&body);
        let id = reader.varint().unwrap();
        let Clientbound::LevelChunkWithLight(packet) =
            Clientbound::decode(id, reader.rest()).unwrap()
        else {
            panic!("expected LevelChunkWithLight");
        };
        assert_eq!((packet.x, packet.z), key);
        assert!(rx.try_recv().is_err());
    });
}

#[test]
#[ignore = "daily"]
fn resend_after_air_check_does_not_wait_for_later_column_relight() {
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
    use sunrise_sim::lan::{Out, PlayerSender};

    if run_fresh_cases(
        "cut_layout::game::native_chunk_relight_tests::resend_after_air_check_does_not_wait_for_later_column_relight",
        "SUNRISE_CHUNK_RESEND_LATER_RELIGHT_CHILD",
        &["game"],
    ) {
        return;
    }
    with_guests(|game| {
        roofed(game);
        let level = game.level("minecraft:overworld").unwrap();
        let key = (100, 100);
        let (tx, rx) = std::sync::mpsc::channel();
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.sender = PlayerSender::new(tx, Arc::default());
            g.dimension = "minecraft:overworld".into();
            g.center = Some(key);
            g.sent_chunks.insert(key);
        });
        game.server.streaming.paused = false;

        assert!(level.check_light(game, [1608, 105, 1608]));
        assert_eq!(game.chunk(level, [100, 100]).unwrap().resend(game), Ok(()));
        game.lan_broadcast_changes();
        assert!(rx.try_recv().is_err(), "the earlier check has not drained");

        assert_eq!(level.relight_chunks(game, &[[100, 100]]), Ok(9));
        game.server.streaming.update_light(&mut game.common.world);
        assert!(game.world.light_edits.is_empty());
        assert!(game.server.streaming.has_column_light_job(key));
        game.lan_broadcast_changes();

        let Out::Packet(body) = rx
            .try_recv()
            .expect("resend before the later light job applies")
        else {
            panic!("expected a chunk packet");
        };
        let mut reader = Reader::new(&body);
        let id = reader.varint().unwrap();
        let Clientbound::LevelChunkWithLight(packet) =
            Clientbound::decode(id, reader.rest()).unwrap()
        else {
            panic!("expected LevelChunkWithLight");
        };
        assert_eq!((packet.x, packet.z), key);
        assert!(rx.try_recv().is_err());
    });
}

/// Loading is required even with no recipients; a loaded chunk with none is a no-op.
#[test]
#[ignore = "daily"]
fn a_chunk_is_resent_only_when_loaded() {
    with_game(|game| {
        roofed(game);
        let level = game.level(game.dim.id()).unwrap();
        let loaded = game.chunk(level, [100, 100]).unwrap();
        assert_eq!(loaded.resend(game), Ok(()));
        let unloaded = mod_api::__private::game::chunk_ref(level, [400, 400]);
        assert_eq!(unloaded.resend(game), Err(ChunkRefusal::NotLoaded));
        assert!(game.world.chunk_resends.is_empty());
    });
}

/// On a client both are `Client`.
#[test]
#[ignore = "daily"]
fn a_client_cannot_relight_or_resend() {
    with_game(|game| {
        roofed(game);
        let role = game.common.options.execution_role;
        game.common.options.execution_role = ExecutionRole::Client;
        let level = LevelRef::new(WorldEpoch::new(0), game.dim.key());
        assert_eq!(
            level.relight_chunks(game, &[[100, 100]]),
            Err(ChunkRefusal::Client)
        );
        let chunk = mod_api::__private::game::chunk_ref(level, [100, 100]);
        assert_eq!(chunk.resend(game), Err(ChunkRefusal::Client));
        game.common.options.execution_role = role;
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
