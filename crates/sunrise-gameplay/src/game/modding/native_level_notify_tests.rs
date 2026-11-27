//! #968/#3720: crack effects wait for commit; explicit block-update requests survive abort.
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{HostTransaction, TransactionError};
use std::sync::Arc;
use sunrise_level::chunk::Chunk;

#[test]
#[ignore = "daily"]
fn crack_packets_wait_for_root_commit_and_preserve_fifo() {
    use crate::game::native_guest_turn_tests::with_guests;
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
    use sunrise_sim::lan::{Out, PlayerSender};
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let (tx, rx) = std::sync::mpsc::channel();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |guest| {
                guest.sender = PlayerSender::new(tx, Arc::default());
            })
            .unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let host: &mut dyn mod_api::ServerHost = game;
        let aborted: Result<(), TransactionError<()>> = host.transaction(|host, _| {
            host.transaction(|host, _| {
                assert!(level.set_break_progress(host, -7, [1, 64, 1], 2));
                Ok::<_, ()>(())
            })
            .unwrap();
            assert!(rx.try_recv().is_err());
            Err(())
        });
        assert!(aborted.is_err());
        assert!(rx.try_recv().is_err());
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let host: &mut dyn mod_api::ServerHost = game;
            let _: Result<(), TransactionError<()>> = host.transaction(|host, _| {
                assert!(level.set_break_progress(host, -7, [1, 64, 1], 4));
                assert!(rx.try_recv().is_err());
                panic!("abort crack transaction");
            });
        }));
        assert!(panic.is_err());
        assert!(rx.try_recv().is_err());
        let host: &mut dyn mod_api::ServerHost = game;
        host.transaction(|host, _| {
            assert!(level.set_break_progress(host, -7, [1, 64, 1], 1));
            host.transaction(|host, _| {
                assert!(level.set_break_progress(host, -7, [1, 64, 1], 6));
                Ok::<_, ()>(())
            })
            .unwrap();
            assert!(level.set_break_progress(host, -7, [1, 64, 1], -1));
            assert!(rx.try_recv().is_err());
            Ok::<_, ()>(())
        })
        .unwrap();
        let stages: Vec<_> = rx
            .try_iter()
            .map(|out| {
                let Out::Packet(body) = out else {
                    panic!("expected packet")
                };
                let mut reader = Reader::new(&body);
                let id = reader.varint().unwrap();
                let Clientbound::BlockDestruction(packet) =
                    Clientbound::decode(id, reader.rest()).unwrap()
                else {
                    panic!("expected crack packet")
                };
                assert_eq!(packet.id, -7);
                assert_eq!(packet.pos, [1, 64, 1]);
                packet.progress
            })
            .collect();
        assert_eq!(stages, [1, 6, 255]);
    });
}

#[test]
#[ignore = "daily"]
fn break_progress_waits_for_commit_but_explicit_block_updates_survive_abort() {
    use crate::game::native_guest_turn_tests::with_guests;
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
    use sunrise_sim::lan::{Out, PlayerSender};
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.world.block_log = Some(Vec::new());
        let (tx, rx) = std::sync::mpsc::channel();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |guest| {
                guest.sender = PlayerSender::new(tx, Arc::default());
            })
            .unwrap();
        let drain_cracks = || -> Vec<([i32; 3], u8)> {
            rx.try_iter()
                .filter_map(|out| {
                    let Out::Packet(body) = out else {
                        return None;
                    };
                    let mut reader = Reader::new(&body);
                    let id = reader.varint().unwrap();
                    match Clientbound::decode(id, reader.rest()).unwrap() {
                        Clientbound::BlockDestruction(packet) if packet.id == -7 => {
                            Some((packet.pos, packet.progress))
                        }
                        _ => None,
                    }
                })
                .collect()
        };
        let level = game.level(game.dim.id()).unwrap();
        let host: &mut dyn mod_api::ServerHost = game;
        let aborted: Result<(), TransactionError<()>> = host.transaction(|host, _| {
            assert!(level.set_break_progress(host, -7, [1, 64, 1], 3));
            assert!(level.send_block_updated(host, [2, 64, 2]));
            Err(())
        });
        assert!(aborted.is_err());
        assert!(drain_cracks().is_empty(), "no crack after abort");
        assert!(game.world.block_log.as_ref().unwrap().contains(&(2, 64, 2)));

        let host: &mut dyn mod_api::ServerHost = game;
        host.transaction(|host, _| {
            assert!(level.set_break_progress(host, -7, [1, 64, 1], 3));
            assert!(level.send_block_updated(host, [2, 64, 2]));
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(drain_cracks(), [([1, 64, 1], 3)]);
        assert_eq!(game.world.block_log.as_ref().unwrap(), &vec![(2, 64, 2)]);

        // Outside a transaction the crack is cleared at once (any stage outside 0..10).
        assert!(game.level_set_break_progress(level, -7, [1, 64, 1], -1));
        assert_eq!(drain_cracks(), [([1, 64, 1], 255)]);
    });
}

#[test]
#[ignore = "daily"]
fn bone_meal_on_an_invalid_block_changes_nothing() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(5, 64, 5, stone).unwrap();
        game.world.modified.clear();
        let level = game.level(game.dim.id()).unwrap();
        assert!(!game.level_bone_meal(level, [5, 64, 5]));
        assert!(!game.level_bone_meal(level, [100_000, 64, 0]));
        assert_eq!(game.world.block(5, 64, 5), stone);
        assert!(game.world.modified.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn biome_writes_roll_back_and_only_the_root_commit_notifies_clients() {
    use crate::game::native_guest_turn_tests::with_guests;
    use sunrise_sim::lan::PlayerSender;

    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.world.modified.clear();
        game.world.dirty_sections.clear();
        let (tx, rx) = std::sync::mpsc::channel();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |guest| {
                guest.sender = PlayerSender::new(tx, Arc::default());
                guest.sent_chunks.insert((0, 0));
            })
            .unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let pos = [0, 64, 0];
        let old = 0_u16;
        assert_eq!(game.world.chunks[&(0, 0)].biome(0, 64, 0), 0);
        let target = 1_u16;
        let name = world_biomes!(game).list[usize::from(target)].name.clone();
        let old_name = world_biomes!(game).list[usize::from(old)].name.clone();
        let host: &mut dyn mod_api::ServerHost = game;
        let aborted: Result<(), TransactionError<()>> = host.transaction(|host, _| {
            assert!(level.set_biome(host, pos, &name));
            assert!(rx.try_recv().is_err());
            host.transaction(|host, _| {
                assert!(level.set_biome(host, [4, 64, 0], &name));
                Ok::<_, ()>(())
            })
            .unwrap();
            assert!(
                !level.set_biome(host, pos, &name),
                "unchanged cell is refused"
            );
            Err(())
        });
        assert!(aborted.is_err());
        assert_eq!(game.world.chunks[&(0, 0)].biome(0, 64, 0), 0);
        assert_eq!(game.world.chunks[&(0, 0)].biome(4, 64, 0), 0);
        assert!(game.world.modified.is_empty());
        assert!(game.world.dirty_sections.is_empty());
        assert!(rx.try_recv().is_err());

        // LevelReader.getBiome reads the cell its fuzzy zoom selects, which for a block on a
        // cell's edge such as `pos` can be a neighbour in an unloaded chunk: read back through a
        // block whose zoomed cell is `pos`'s own.
        let zoom = game.server.streaming.biome_zoom;
        let probe = (0..16)
            .flat_map(|x| (64..68).flat_map(move |y| (0..16).map(move |z| [x, y, z])))
            .find(|&[x, y, z]| {
                zoom.quart_at_block(sunrise_core::position::BlockPos::new(x, y, z)) == [0, 16, 0]
            })
            .expect("a block in the chunk whose zoomed cell is pos's");
        let host: &mut dyn mod_api::ServerHost = game;
        host.transaction(|host, _| {
            assert!(level.set_biome(host, pos, &name));
            let inner: Result<(), TransactionError<()>> = host.transaction(|host, _| {
                assert!(level.set_biome(host, pos, &old_name));
                Err(())
            });
            assert!(inner.is_err());
            assert_eq!(level.biome_at(host, probe).as_deref(), Some(name.as_str()));
            host.transaction(|host, _| {
                assert!(level.set_biome(host, pos, &old_name));
                assert!(level.set_biome(host, [3, 67, 3], &name));
                Ok::<_, ()>(())
            })
            .unwrap();
            assert!(rx.try_recv().is_err());
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(game.world.chunks[&(0, 0)].biome(3, 67, 3), target);
        assert!(game.world.modified.contains(&(0, 0)));
        assert!(!game.world.dirty_sections.is_empty());
        let sunrise_sim::lan::Out::Packet(body) = rx.try_recv().unwrap() else {
            panic!("expected a biome packet");
        };
        let mut reader = sunrise_protocol::buf::Reader::new(&body);
        let id = reader.varint().unwrap();
        let packet =
            sunrise_protocol::packets::play::Clientbound::decode(id, reader.rest()).unwrap();
        let sunrise_protocol::packets::play::Clientbound::ChunksBiomes(packet) = packet else {
            panic!("expected biome contents");
        };
        assert_eq!(packet.chunks.len(), 1);
        assert_eq!(packet.chunks[0].pos, [0, 0]);
        let sections = packet.chunks[0].biomes(24, 8).unwrap();
        // Y = 64 is section 8 above min-y -64; local quart (0, 0, 0).
        assert_eq!(sections[8].get(0), 1);
        assert_eq!(sections[8].get(1), 0);
        assert_eq!(sections[0].get(0), 0);
        assert!(rx.try_recv().is_err(), "one biome notification per column");
    });
}

#[test]
#[ignore = "daily"]
fn biome_negative_min_y_and_unwind_restore_without_publication() {
    use crate::game::native_guest_turn_tests::with_guests;
    use sunrise_sim::lan::PlayerSender;
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((-1, -1), Arc::new(Chunk::empty(-1, -1)));
        game.world.modified.clear();
        game.world.dirty_sections.clear();
        let (tx, rx) = std::sync::mpsc::channel();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |guest| {
                guest.sender = PlayerSender::new(tx, Arc::default());
                guest.sent_chunks.insert((-1, -1));
            })
            .unwrap();
        assert_eq!(game.world.bottom_y(), -64);
        assert_eq!(game.world.chunks[&(-1, -1)].biome(-1, -64, -1), 0);
        let name = world_biomes!(game).list[1].name.clone();
        let level = game.level(game.dim.id()).unwrap();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let host: &mut dyn mod_api::ServerHost = game;
            let _: Result<(), TransactionError<()>> = host.transaction(|host, _| {
                assert!(!level.set_biome(host, [-1, -65, -1], &name));
                assert!(!level.set_biome(host, [-1, 320, -1], &name));
                assert!(level.set_biome(host, [-1, -64, -1], &name));
                assert_eq!(
                    level.biome_at(host, [-4, -61, -4]).as_deref(),
                    Some(name.as_str())
                );
                assert!(rx.try_recv().is_err());
                panic!("abort biome transaction");
            });
        }));
        assert!(panic.is_err());
        assert_eq!(game.world.chunks[&(-1, -1)].biome(-1, -64, -1), 0);
        assert!(game.world.modified.is_empty());
        assert!(game.world.dirty_sections.is_empty());
        assert!(rx.try_recv().is_err());
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
