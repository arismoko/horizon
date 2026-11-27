#![cfg(test)]
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{
        BlockEdit, BlockRun, BlockUpdateFlags, Host, HostTransaction, LevelReadError, LevelRef,
        LevelWriteError, ServerHost, WorldEpoch,
    };
    use std::sync::Arc;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_level::chunk::{Chunk, MIN_Y};
    use sunrise_protocol::{
        connection::Pipe,
        connection_groups::{self, Cell, Entry, Section},
        packets::{
            custom_payload::CustomPayload,
            play::{self, Clientbound as C},
        },
        payload_negotiation::Phase,
    };
    use sunrise_sim::ticket_levels::{Sources, ViewedTicket};
    use {
        crate::game::client_side::viewed_worlds::tests::client_with_pipe,
        crate::game::client_side::viewed_worlds::tests::close as viewed_close,
        crate::game::client_side::viewed_worlds::tests::column as viewed_column,
        crate::game::client_side::viewed_worlds::tests::open as viewed_open,
        crate::game::client_side::viewed_worlds::tests::run as viewed_run,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
        sunrise_gameplay_common::game::Dim,
    };

    fn state(game: &sunrise_gameplay::game::ServerGame<'_>, name: &str) -> mod_api::BlockState {
        let host: &dyn Host = game;
        host.block_state_id(name).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn connection_group_api_errors_state_preservation_and_block_replacement() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.native_lifecycle.world_open_ready().unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.connection_group_log = Some(Default::default());

            let level = game.level("minecraft:overworld").unwrap();
            let log_y = state(game, "minecraft:oak_log[axis=y]");
            let log_x = state(game, "minecraft:oak_log[axis=x]");
            let stone = state(game, "minecraft:stone");
            let glass = state(game, "minecraft:glass");
            let copper_chest = state(game, "minecraft:copper_chest");
            let exposed_copper_chest = state(game, "minecraft:exposed_copper_chest");

            assert_eq!(
                level.set_block(game, [2, 64, 3], log_y, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(level.connection_group(game, [2, 64, 3]), Ok(None));

            // Group 0 is a valid value; repeating it reports Ok(false) without extra log entries.
            assert_eq!(
                level.set_connection_group(game, [2, 64, 3], Some(0)),
                Ok(true)
            );
            assert_eq!(level.connection_group(game, [2, 64, 3]), Ok(Some(0)));
            assert_eq!(
                level.set_connection_group(game, [2, 64, 3], Some(0)),
                Ok(false)
            );
            assert_eq!(
                game.world.connection_group_log.as_ref().unwrap(),
                &[(2, 64, 3)].into_iter().collect()
            );

            // Unloaded chunk, out-of-height, and carrier-fenced positions return Ok(false).
            assert_eq!(
                level.set_connection_group(game, [32, 64, 0], Some(1)),
                Ok(false)
            );
            assert_eq!(
                level.set_connection_group(game, [2, 9999, 3], Some(1)),
                Ok(false)
            );
            game.world.carrier_fences.lock_cell([2, 64, 3]);
            assert_eq!(
                level.set_connection_group(game, [2, 64, 3], Some(9)),
                Ok(false)
            );
            assert_eq!(level.connection_group(game, [2, 64, 3]), Ok(Some(0)));
            game.world.carrier_fences.unlock_cell([2, 64, 3]);
            assert_eq!(level.connection_group(game, [32, 64, 0]), Ok(None));
            assert_eq!(level.connection_group(game, [2, 9999, 3]), Ok(None));

            // Stale handle refuses on both read and write.
            let stale = LevelRef::new(WorldEpoch::new(0), level.dimension());
            assert_eq!(
                stale.connection_group(game, [2, 64, 3]),
                Err(LevelReadError::Stale)
            );
            assert_eq!(
                stale.set_connection_group(game, [2, 64, 3], Some(1)),
                Err(LevelWriteError::Stale)
            );

            // Same-block state change preserves the connection group.
            game.world.connection_group_log.as_mut().unwrap().clear();
            assert_eq!(
                level.set_block(game, [2, 64, 3], log_x, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(level.connection_group(game, [2, 64, 3]), Ok(Some(0)));
            assert!(game.world.connection_group_log.as_ref().unwrap().is_empty());

            // Replacing with a different block also publishes the automatic clear.
            assert_eq!(
                level.set_block(game, [2, 64, 3], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(level.connection_group(game, [2, 64, 3]), Ok(None));
            assert_eq!(
                game.world.connection_group_log.as_ref().unwrap(),
                &[(2, 64, 3)].into_iter().collect()
            );

            // Weathering a copper chest preserves the connection group (same rule as block entities).
            assert_eq!(
                level.set_block(game, [2, 64, 3], copper_chest, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [2, 64, 3], Some(7)),
                Ok(true)
            );
            game.world.connection_group_log.as_mut().unwrap().clear();
            assert_eq!(
                level.set_block(
                    game,
                    [2, 64, 3],
                    exposed_copper_chest,
                    BlockUpdateFlags::NONE
                ),
                Ok(true)
            );
            assert_eq!(level.connection_group(game, [2, 64, 3]), Ok(Some(7)));
            assert!(game.world.connection_group_log.as_ref().unwrap().is_empty());

            // Bulk block edits follow the same block-entity lifetime rule: same-block state
            // changes and copper-chest weathering preserve groups; replacing with another block
            // clears them.
            assert_eq!(
                level.set_block(game, [5, 64, 5], log_y, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [5, 64, 5], Some(1)),
                Ok(true)
            );
            assert_eq!(
                level.set_block(game, [6, 64, 5], copper_chest, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [6, 64, 5], Some(2)),
                Ok(true)
            );
            assert_eq!(
                level.set_block(game, [7, 64, 5], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [7, 64, 5], Some(3)),
                Ok(true)
            );
            let edit = BlockEdit {
                palette: vec![log_x, exposed_copper_chest, glass],
                filters: Vec::new(),
                runs: vec![
                    BlockRun {
                        x: 5,
                        z: 5,
                        y: 64,
                        height: 1,
                        surface: None,
                        block: 0,
                        only: None,
                    },
                    BlockRun {
                        x: 6,
                        z: 5,
                        y: 64,
                        height: 1,
                        surface: None,
                        block: 1,
                        only: None,
                    },
                    BlockRun {
                        x: 7,
                        z: 5,
                        y: 64,
                        height: 1,
                        surface: None,
                        block: 2,
                        only: None,
                    },
                ],
            };
            assert_eq!(game.apply_block_edit(&edit), Ok(3));
            assert_eq!(level.connection_group(game, [5, 64, 5]), Ok(Some(1)));
            assert_eq!(level.connection_group(game, [6, 64, 5]), Ok(Some(2)));
            assert_eq!(level.connection_group(game, [7, 64, 5]), Ok(None));

            // AnvilMenu's last wear step writes directly to World rather than calling
            // the simulation's block setter.
            let anvil = [8, 64, 5];
            let damaged = state(game, "minecraft:damaged_anvil");
            assert_eq!(
                level.set_block(game, anvil, damaged, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(level.set_connection_group(game, anvil, Some(30)), Ok(true));
            use sunrise_core::random::{LegacyRandom, LevelRandom};
            game.world.random = LevelRandom::for_level(LegacyRandom::new(4096));
            assert!(game.anvil_used(anvil, false));
            assert_eq!(game.world.block(8, 64, 5), game.world.air);
            assert_eq!(level.connection_group(game, anvil), Ok(None));

            // Live features' section writer uses set_raw on the installed world.
            let raw = [9, 64, 5];
            assert_eq!(
                level.set_block(game, raw, log_y, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(level.set_connection_group(game, raw, Some(31)), Ok(true));
            let at = sunrise_core::position::BlockPos::new(raw[0], raw[1], raw[2]);
            let air = game.world.air;
            game.with_level(|_, level, _| {
                assert!(sunrise_world::level::LevelAccess::set_raw(level, at, air).is_some());
            });
            assert_eq!(level.connection_group(game, raw), Ok(None));

            // Clone's destination contains the same block state, but not the same entity.
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("setblock 10 64 5 chest"));
            assert!(game.run_command("setblock 11 64 5 chest"));
            let destination = [11, 64, 5];
            let old_entity = game.block_entity(level, destination).unwrap();
            let old_state = level.block_state(game, destination);
            assert_eq!(
                level.set_connection_group(game, destination, Some(32)),
                Ok(true)
            );
            assert!(game.run_command("clone 10 64 5 10 64 5 11 64 5"));
            assert_eq!(level.block_state(game, destination), old_state);
            assert_ne!(game.block_entity(level, destination), Some(old_entity));
            assert_eq!(level.connection_group(game, destination), Ok(None));

            // Parked level refuses writes with Unavailable while reads still succeed.
            assert_eq!(
                level.set_connection_group(game, [2, 64, 3], Some(19)),
                Ok(true)
            );
            assert!(game.enter_command_level(Dim::Nether).is_some());
            assert_eq!(level.connection_group(game, [2, 64, 3]), Ok(Some(19)));
            assert_eq!(
                level.set_connection_group(game, [2, 64, 3], Some(20)),
                Err(LevelWriteError::Unavailable)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn connection_group_transactions_abort_silently_and_commit_with_block_writes() {
        // The fixture's resource load replaces the process's block namespace table.
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::native_connection_groups::tests::connection_group_transactions_abort_silently_and_commit_with_block_writes",
            "SUNRISE_GROUP_TRANSACTIONS_CHILD",
            &["run"],
        ) {
            return;
        }
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.native_lifecycle.world_open_ready().unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.block_log = Some(Vec::new());
            game.world.connection_group_log = Some(Default::default());
            let non_ticking = Sources {
                viewed: vec![ViewedTicket {
                    center: (0, 0),
                    radius: 0,
                    ticking: false,
                }],
                ..Sources::default()
            };
            game.test_update_tickets(Some(non_ticking.clone()));
            assert!(!game.test_block_ticking(0, 0));

            let level = game.level("minecraft:overworld").unwrap();
            let stone = state(game, "minecraft:stone");
            let glass = state(game, "minecraft:glass");
            let log_y = state(game, "minecraft:oak_log[axis=y]");
            let log_x = state(game, "minecraft:oak_log[axis=x]");
            let torch = state(game, "minecraft:torch");
            let air = state(game, "minecraft:air");
            assert_eq!(
                level.set_block(game, [4, 64, 4], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [4, 64, 4], Some(11)),
                Ok(true)
            );
            game.world.dirty_sections.clear();
            game.world.block_log.as_mut().unwrap().clear();
            game.world.connection_group_log.as_mut().unwrap().clear();

            // Aborted transaction sees tentative write inside, restores silently on abort.
            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_connection_group(game, [4, 64, 4], Some(22)),
                    Ok(true)
                );
                assert_eq!(level.connection_group(game, [4, 64, 4]), Ok(Some(22)));
                ((), false)
            });
            assert_eq!(level.connection_group(game, [4, 64, 4]), Ok(Some(11)));
            assert!(game.world.dirty_sections.is_empty());
            assert!(game.world.connection_group_log.as_ref().unwrap().is_empty());

            // Aborted transaction replacing a block restores the cleared group silently.
            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_block(game, [4, 64, 4], glass, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                assert_eq!(level.connection_group(game, [4, 64, 4]), Ok(None));
                ((), false)
            });
            assert_eq!(level.connection_group(game, [4, 64, 4]), Ok(Some(11)));
            assert!(game.world.dirty_sections.is_empty());
            assert!(game.world.connection_group_log.as_ref().unwrap().is_empty());

            // A committed replacement with no vanilla notification still publishes its clear.
            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_block(game, [4, 64, 4], glass, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                assert_eq!(level.connection_group(game, [4, 64, 4]), Ok(None));
                ((), true)
            });
            assert_eq!(level.connection_group(game, [4, 64, 4]), Ok(None));
            assert!(game.world.block_log.as_ref().unwrap().is_empty());
            assert_eq!(
                game.world.connection_group_log.as_ref().unwrap(),
                &[(4, 64, 4)].into_iter().collect()
            );

            // Committed transaction replacing a block and setting a new group keeps the new group
            // and logs only the explicit write.
            assert_eq!(
                level.set_connection_group(game, [4, 64, 4], Some(11)),
                Ok(true)
            );
            game.world.dirty_sections.clear();
            game.world.block_log.as_mut().unwrap().clear();
            game.world.connection_group_log.as_mut().unwrap().clear();
            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_block(game, [4, 64, 4], stone, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                assert_eq!(level.connection_group(game, [4, 64, 4]), Ok(None));
                assert_eq!(
                    level.set_connection_group(game, [4, 64, 4], Some(33)),
                    Ok(true)
                );
                assert_eq!(level.connection_group(game, [4, 64, 4]), Ok(Some(33)));
                ((), true)
            });
            let sy = (64 - MIN_Y) >> 4;
            assert_eq!(level.connection_group(game, [4, 64, 4]), Ok(Some(33)));
            assert!(game.world.dirty_sections.contains(&(0, sy, 0)));
            assert_eq!(
                game.world.connection_group_log.as_ref().unwrap(),
                &[(4, 64, 4)].into_iter().collect()
            );

            // Held non-ticking column round-trip (`oak_log[y] -> stone -> oak_log[x]`):
            // there is no vanilla send, but the group lifetime ended.
            assert_eq!(
                level.set_block(game, [5, 64, 5], log_y, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [5, 64, 5], Some(7)),
                Ok(true)
            );
            game.world.block_log.as_mut().unwrap().clear();
            game.world.connection_group_log.as_mut().unwrap().clear();
            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_block(game, [5, 64, 5], stone, BlockUpdateFlags::CLIENTS),
                    Ok(true)
                );
                assert_eq!(level.connection_group(game, [5, 64, 5]), Ok(None));
                assert_eq!(
                    level.set_block(game, [5, 64, 5], log_x, BlockUpdateFlags::CLIENTS),
                    Ok(true)
                );
                ((), true)
            });
            assert_eq!(level.connection_group(game, [5, 64, 5]), Ok(None));
            assert!(game.world.block_log.as_ref().unwrap().is_empty());
            assert_eq!(
                game.world.connection_group_log.as_ref().unwrap(),
                &[(5, 64, 5)].into_iter().collect()
            );

            // Held non-ticking column with explicit write sandwiched between two replacing writes
            // (`oak_log[y] -> stone -> set_connection_group(99) -> oak_log[x]`): second replace
            // clears the group and only its final value is published.
            assert_eq!(
                level.set_block(game, [7, 64, 7], log_y, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [7, 64, 7], Some(7)),
                Ok(true)
            );
            game.world.block_log.as_mut().unwrap().clear();
            game.world.connection_group_log.as_mut().unwrap().clear();
            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_block(game, [7, 64, 7], stone, BlockUpdateFlags::CLIENTS),
                    Ok(true)
                );
                assert_eq!(
                    level.set_connection_group(game, [7, 64, 7], Some(99)),
                    Ok(true)
                );
                assert_eq!(
                    level.set_block(game, [7, 64, 7], log_x, BlockUpdateFlags::CLIENTS),
                    Ok(true)
                );
                assert_eq!(level.connection_group(game, [7, 64, 7]), Ok(None));
                ((), true)
            });
            assert_eq!(level.connection_group(game, [7, 64, 7]), Ok(None));
            assert!(game.world.block_log.as_ref().unwrap().is_empty());
            assert_eq!(
                game.world.connection_group_log.as_ref().unwrap(),
                &[(7, 64, 7)].into_iter().collect()
            );

            // Returning to the same block state does not extend a cleared group's lifetime.
            assert_eq!(
                level.set_block(game, [7, 65, 7], log_y, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [7, 65, 7], Some(13)),
                Ok(true)
            );
            game.world.block_log.as_mut().unwrap().clear();
            game.world.connection_group_log.as_mut().unwrap().clear();
            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_block(game, [7, 65, 7], stone, BlockUpdateFlags::CLIENTS),
                    Ok(true)
                );
                assert_eq!(
                    level.set_block(game, [7, 65, 7], log_y, BlockUpdateFlags::CLIENTS),
                    Ok(true)
                );
                ((), true)
            });
            assert_eq!(level.connection_group(game, [7, 65, 7]), Ok(None));
            assert!(game.world.block_log.as_ref().unwrap().is_empty());
            assert_eq!(
                game.world.connection_group_log.as_ref().unwrap(),
                &[(7, 65, 7)].into_iter().collect()
            );

            // A same-state transaction can also publish a new group without adding vanilla traffic.
            game.test_update_tickets(Some(Sources {
                forced: vec![(0, 0)],
                ..Sources::default()
            }));
            assert!(game.test_block_ticking(0, 0));
            assert_eq!(level.connection_group(game, [7, 65, 7]), Ok(None));
            game.world.connection_group_log.as_mut().unwrap().clear();
            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_block(game, [7, 65, 7], stone, BlockUpdateFlags::CLIENTS),
                    Ok(true)
                );
                assert_eq!(
                    level.set_block(game, [7, 65, 7], log_y, BlockUpdateFlags::CLIENTS),
                    Ok(true)
                );
                assert_eq!(
                    level.set_connection_group(game, [7, 65, 7], Some(15)),
                    Ok(true)
                );
                ((), true)
            });
            assert_eq!(level.connection_group(game, [7, 65, 7]), Ok(Some(15)));
            assert!(game.world.block_log.as_ref().unwrap().is_empty());
            assert_eq!(
                game.world.connection_group_log.as_ref().unwrap(),
                &[(7, 65, 7)].into_iter().collect()
            );
            game.test_update_tickets(Some(non_ticking));

            // Cross-position cascade on held non-ticking column: `set_connection_group(torch)`
            // followed by breaking its support pops the torch, clears its group, and drops the
            // superseded explicit log entry.
            assert_eq!(
                level.set_block(game, [6, 64, 6], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_block(game, [6, 65, 6], torch, BlockUpdateFlags::NONE),
                Ok(true)
            );
            game.world.block_log.as_mut().unwrap().clear();
            game.world.connection_group_log.as_mut().unwrap().clear();
            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_connection_group(game, [6, 65, 6], Some(42)),
                    Ok(true)
                );
                assert_eq!(
                    level.set_block(
                        game,
                        [6, 64, 6],
                        air,
                        BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
                    ),
                    Ok(true)
                );
                ((), true)
            });
            assert_eq!(game.world.block(6, 65, 6), game.world.air);
            assert_eq!(level.connection_group(game, [6, 65, 6]), Ok(None));
            assert!(game.world.block_log.as_ref().unwrap().is_empty());
            assert!(game.world.connection_group_log.as_ref().unwrap().is_empty());

            // Strict replacements clear the tentative group at the call. On commit they replay
            // in order with explicit group writes; on abort, both block and group come back.
            let single_replace = |pos: [i32; 3], to: mod_api::BlockState| BlockEdit {
                palette: vec![to],
                filters: Vec::new(),
                runs: vec![BlockRun {
                    x: pos[0],
                    z: pos[2],
                    y: pos[1],
                    height: 1,
                    surface: None,
                    block: 0,
                    only: None,
                }],
            };

            let reset_group_publication = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.world.dirty_sections.clear();
                game.world.connection_group_log.as_mut().unwrap().clear();
            };
            let assert_quiet = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                assert!(game.world.dirty_sections.is_empty());
                assert!(game.world.connection_group_log.as_ref().unwrap().is_empty());
            };

            // Commit after set_connection_group + bulk replace.
            assert_eq!(
                level.set_block(game, [8, 64, 8], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [8, 64, 8], Some(1)),
                Ok(true)
            );
            reset_group_publication(game);
            let edit_commit = single_replace([8, 64, 8], glass);
            {
                let host: &mut dyn ServerHost = game;
                let res: Result<(), mod_api::TransactionError<()>> =
                    host.transaction(|host, _tx| {
                        assert_eq!(
                            level.set_connection_group(host, [8, 64, 8], Some(2)),
                            Ok(true)
                        );
                        assert_eq!(level.edit_blocks(host, &edit_commit), Ok(1));
                        assert_eq!(level.connection_group(host, [8, 64, 8]), Ok(None));
                        Ok(())
                    });
                assert_eq!(res, Ok(()));
            }
            assert_eq!(level.block_state(game, [8, 64, 8]), Some(glass));
            assert_eq!(level.connection_group(game, [8, 64, 8]), Ok(None));
            assert_eq!(
                game.world.connection_group_log.as_ref().unwrap(),
                &[(8, 64, 8)].into_iter().collect()
            );

            // Abort after set_connection_group(Some) + bulk replace.
            assert_eq!(
                level.set_block(game, [9, 64, 9], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [9, 64, 9], Some(1)),
                Ok(true)
            );
            reset_group_publication(game);
            let edit_abort = single_replace([9, 64, 9], glass);
            {
                let host: &mut dyn ServerHost = game;
                let res: Result<(), mod_api::TransactionError<()>> =
                    host.transaction(|host, _tx| {
                        assert_eq!(
                            level.set_connection_group(host, [9, 64, 9], Some(2)),
                            Ok(true)
                        );
                        assert_eq!(level.edit_blocks(host, &edit_abort), Ok(1));
                        Err(())
                    });
                assert_eq!(res, Err(mod_api::TransactionError::Aborted(())));
            }
            assert_eq!(level.block_state(game, [9, 64, 9]), Some(stone));
            assert_eq!(level.connection_group(game, [9, 64, 9]), Ok(Some(1)));
            assert_quiet(game);

            // Abort after tentative set_connection_group(None) + bulk replace.
            assert_eq!(
                level.set_block(game, [10, 64, 10], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [10, 64, 10], Some(1)),
                Ok(true)
            );
            reset_group_publication(game);
            let edit_tentative_none = single_replace([10, 64, 10], glass);
            {
                let host: &mut dyn ServerHost = game;
                let res: Result<(), mod_api::TransactionError<()>> =
                    host.transaction(|host, _tx| {
                        assert_eq!(
                            level.set_connection_group(host, [10, 64, 10], None),
                            Ok(true)
                        );
                        assert_eq!(level.edit_blocks(host, &edit_tentative_none), Ok(1));
                        Err(())
                    });
                assert_eq!(res, Err(mod_api::TransactionError::Aborted(())));
            }
            assert_eq!(level.block_state(game, [10, 64, 10]), Some(stone));
            assert_eq!(level.connection_group(game, [10, 64, 10]), Ok(Some(1)));
            assert_quiet(game);

            // A fresh group written after the bulk replacement commits with it, or aborts with
            // it back to the group the transaction started from.
            assert_eq!(
                level.set_block(game, [11, 64, 11], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [11, 64, 11], Some(1)),
                Ok(true)
            );
            reset_group_publication(game);
            let edit_after_commit = single_replace([11, 64, 11], glass);
            {
                let host: &mut dyn ServerHost = game;
                let res: Result<(), mod_api::TransactionError<()>> =
                    host.transaction(|host, _tx| {
                        assert_eq!(
                            level.set_connection_group(host, [11, 64, 11], Some(2)),
                            Ok(true)
                        );
                        assert_eq!(level.edit_blocks(host, &edit_after_commit), Ok(1));
                        assert_eq!(
                            level.set_connection_group(host, [11, 64, 11], Some(3)),
                            Ok(true)
                        );
                        Ok(())
                    });
                assert_eq!(res, Ok(()));
            }
            assert_eq!(level.block_state(game, [11, 64, 11]), Some(glass));
            assert_eq!(level.connection_group(game, [11, 64, 11]), Ok(Some(3)));
            assert_eq!(
                game.world.connection_group_log.as_ref().unwrap(),
                &[(11, 64, 11)].into_iter().collect()
            );

            assert_eq!(
                level.set_block(game, [12, 64, 12], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                level.set_connection_group(game, [12, 64, 12], Some(1)),
                Ok(true)
            );
            reset_group_publication(game);
            let edit_after_abort = single_replace([12, 64, 12], glass);
            {
                let host: &mut dyn ServerHost = game;
                let res: Result<(), mod_api::TransactionError<()>> =
                    host.transaction(|host, _tx| {
                        assert_eq!(
                            level.set_connection_group(host, [12, 64, 12], Some(2)),
                            Ok(true)
                        );
                        assert_eq!(level.edit_blocks(host, &edit_after_abort), Ok(1));
                        assert_eq!(
                            level.set_connection_group(host, [12, 64, 12], Some(3)),
                            Ok(true)
                        );
                        Err(())
                    });
                assert_eq!(res, Err(mod_api::TransactionError::Aborted(())));
            }
            assert_eq!(level.block_state(game, [12, 64, 12]), Some(stone));
            assert_eq!(level.connection_group(game, [12, 64, 12]), Ok(Some(1)));
            assert_quiet(game);

            // A nested bulk round-trip replaces the block twice. Each abort restores the group
            // its own transaction started from.
            let pos = [13, 64, 13];
            assert_eq!(
                level.set_block(game, pos, stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(level.set_connection_group(game, pos, Some(1)), Ok(true));
            let mut round_trip = single_replace(pos, glass);
            round_trip.palette.push(stone);
            round_trip.runs.push(BlockRun {
                block: 1,
                ..round_trip.runs[0]
            });
            reset_group_publication(game);
            game.storage_scope(|game, _| {
                assert_eq!(level.set_connection_group(game, pos, Some(2)), Ok(true));
                game.storage_scope(|game, _| {
                    let host: &mut dyn ServerHost = game;
                    assert_eq!(level.edit_blocks(host, &round_trip), Ok(2));
                    assert_eq!(level.connection_group(game, pos), Ok(None));
                    ((), false)
                });
                assert_eq!(level.connection_group(game, pos), Ok(Some(2)));
                assert_quiet(game);
                ((), false)
            });
            assert_eq!(level.block_state(game, pos), Some(stone));
            assert_eq!(level.connection_group(game, pos), Ok(Some(1)));
            assert_quiet(game);

            // Aborting the first ever tentative write must not opt the column into group sync.
            let untouched = [17, 64, 1];
            game.world
                .chunks
                .insert((1, 0), Arc::new(Chunk::empty(1, 0)));
            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_connection_group(game, untouched, Some(3)),
                    Ok(true)
                );
                ((), false)
            });
            assert_eq!(level.connection_group(game, untouched), Ok(None));
            assert!(game.world.chunks[&(1, 0)].connection_groups.is_empty());

            // A command can temporarily put another dimension in place while a transaction
            // stays open. The same coordinates there must not rewrite the parked baseline.
            let pos = [14, 64, 14];
            assert_eq!(
                level.set_block(game, pos, stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(level.set_connection_group(game, pos, Some(10)), Ok(true));
            let nether = game.storage_scope(|game, _| {
                assert_eq!(level.set_connection_group(game, pos, Some(11)), Ok(true));
                assert_eq!(game.enter_command_level(Dim::Nether), Some(Dim::Overworld));
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let nether = game.level("minecraft:the_nether").unwrap();
                assert_eq!(nether.set_connection_group(game, pos, Some(20)), Ok(true));
                let host: &mut dyn ServerHost = game;
                assert_eq!(nether.edit_blocks(host, &single_replace(pos, glass)), Ok(1));
                assert_eq!(nether.connection_group(game, pos), Ok(None));
                assert_eq!(nether.set_connection_group(game, pos, Some(21)), Ok(true));
                assert_eq!(game.enter_command_level(Dim::Overworld), Some(Dim::Nether));
                (nether, false)
            });
            assert_eq!(level.connection_group(game, pos), Ok(Some(10)));
            assert_eq!(nether.connection_group(game, pos), Ok(Some(21)));

            // A transaction opened in the other level owns its clear too. Its commit becomes
            // final there, even though the enclosing Overworld transaction later aborts.
            game.storage_scope(|game, _| {
                assert_eq!(level.set_connection_group(game, pos, Some(11)), Ok(true));
                assert_eq!(game.enter_command_level(Dim::Nether), Some(Dim::Overworld));
                game.storage_scope(|game, _| {
                    let host: &mut dyn ServerHost = game;
                    assert_eq!(nether.edit_blocks(host, &single_replace(pos, stone)), Ok(1));
                    ((), true)
                });
                assert_eq!(nether.block_state(game, pos), Some(stone));
                assert_eq!(nether.connection_group(game, pos), Ok(None));
                assert_eq!(game.enter_command_level(Dim::Overworld), Some(Dim::Nether));
                ((), false)
            });
            assert_eq!(level.connection_group(game, pos), Ok(Some(10)));
            assert_eq!(nether.connection_group(game, pos), Ok(None));
        });
    }

    /// Records, at each `block_changed`, the block at [`BULK_CELL`].
    struct SeesBulkCell(Arc<std::sync::Mutex<Vec<([i32; 3], Option<mod_api::BlockState>)>>>);

    const BULK_CELL: [i32; 3] = [12, 64, 12];

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for SeesBulkCell {
        fn block_changed(
            &self,
            level: &dyn mod_api::LevelHost,
            event: &mut mod_api::BlockChangedEvent,
        ) {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            self.0
                .lock()
                .unwrap()
                .push((pos, level.block_state(BULK_CELL)));
        }
    }

    /// A callback replayed at the root commit before a later bulk edit sees the bulk cell's
    /// block as it stood before the edit, alongside the group restored with it.
    #[test]
    #[ignore = "daily"]
    fn a_replayed_callback_sees_a_later_bulk_cell_before_its_edit() {
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::native_connection_groups::tests::a_replayed_callback_sees_a_later_bulk_cell_before_its_edit",
            "SUNRISE_GROUP_REPLAY_CHILD",
            &["run"],
        ) {
            return;
        }
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "sees_bulk_cell",
            Box::new(SeesBulkCell(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.native_lifecycle.world_open_ready().unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.world.connection_group_log = Some(Default::default());
                let level = game.level("minecraft:overworld").unwrap();
                let stone = state(game, "minecraft:stone");
                let glass = state(game, "minecraft:glass");
                let ordinary = [2, 64, 2];
                assert_eq!(
                    level.set_block(game, BULK_CELL, stone, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                assert_eq!(
                    level.set_connection_group(game, BULK_CELL, Some(5)),
                    Ok(true)
                );
                let edit = BlockEdit {
                    palette: vec![glass],
                    filters: Vec::new(),
                    runs: vec![BlockRun {
                        x: BULK_CELL[0],
                        z: BULK_CELL[2],
                        y: BULK_CELL[1],
                        height: 1,
                        surface: None,
                        block: 0,
                        only: None,
                    }],
                };
                seen.lock().unwrap().clear();
                {
                    let host: &mut dyn ServerHost = game;
                    let res: Result<(), mod_api::TransactionError<()>> =
                        host.transaction(|host, _tx| {
                            assert_eq!(
                                level.set_block(
                                    host,
                                    ordinary,
                                    stone,
                                    BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
                                ),
                                Ok(true)
                            );
                            assert_eq!(level.edit_blocks(host, &edit), Ok(1));
                            Ok(())
                        });
                    assert_eq!(res, Ok(()));
                }
                let heard: Vec<_> = seen
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(pos, _)| *pos == ordinary)
                    .map(|&(_, bulk)| bulk)
                    .collect();
                assert_eq!(heard, [Some(stone)]);
                assert_eq!(level.block_state(game, BULK_CELL), Some(glass));
                assert_eq!(level.connection_group(game, BULK_CELL), Ok(None));
            });
        });
    }

    fn group_packet(pos: [i32; 3], group: Option<u64>) -> C {
        C::CustomPayload(CustomPayload {
            channel: connection_groups::CHANNEL.into(),
            data: Section {
                pos: pos.map(|coord| coord >> 4),
                entries: vec![Entry {
                    cell: Cell::of(pos),
                    group,
                }],
            }
            .encode()
            .unwrap(),
        })
    }

    fn deliver_group_bundle(
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
        packets: Vec<C>,
    ) {
        let epoch = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .session
            .payload_transport()
            .epoch();
        let negotiated = epoch.is_some_and(|epoch| {
            epoch.phase == Phase::Play
                && epoch
                    .peer
                    .engine_receives(Phase::Play, connection_groups::CHANNEL)
        });
        assert!(
            game.route_connection_group_bundle(&packets, negotiated)
                .unwrap()
        );
    }

    fn negotiate(game: &sunrise_client_gameplay::game::ClientGame<'_>, with_groups: bool) {
        let remote = game.client.remote.as_ref().unwrap();
        let transport = remote.session.payload_transport();
        let entries: Vec<_> = transport
            .local
            .manifest
            .entries()
            .iter()
            .filter(|entry| with_groups || entry.id() != connection_groups::CHANNEL)
            .cloned()
            .collect();
        transport.accept(&entries).unwrap();
        transport.finish_configuration().unwrap();
        assert_eq!(
            transport
                .epoch()
                .unwrap()
                .peer
                .engine_receives(Phase::Play, connection_groups::CHANNEL),
            with_groups
        );
    }

    #[test]
    #[ignore = "daily"]
    fn client_applies_connection_group_payloads_and_refuses_client_writes() {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "ConnGroupsClient").unwrap();
            game.client.remote.as_mut().unwrap().logged_in = true;
            game.native_lifecycle.world_open_ready().unwrap();
            negotiate(game, true);
            let level = game.level("minecraft:overworld").unwrap();
            // Client writes are now refused at compile time (`ClientGame` is not a `ServerHost`).
            let sy = (64 - MIN_Y) >> 4;
            game.handle_packet(viewed_column(1, 0)).unwrap();
            game.world.dirty_sections.clear();
            let border = group_packet([15, 64, 2], Some(55));
            let interior = group_packet([1, 80, 2], Some(66));
            deliver_group_bundle(
                game,
                vec![viewed_column(0, 0), border.clone(), interior.clone()],
            );
            assert_eq!(level.connection_group(game, [15, 64, 2]), Ok(Some(55)));
            assert_eq!(level.connection_group(game, [1, 80, 2]), Ok(Some(66)));

            // A resend compares one complete map. Reordering its sections does
            // not invalidate the mesh of either side of an unchanged border.
            game.world.dirty_sections.clear();
            deliver_group_bundle(game, vec![viewed_column(0, 0), interior, border]);
            assert!(!game.world.dirty_sections.contains(&(0, sy, 0)));
            assert!(!game.world.dirty_sections.contains(&(1, sy, 0)));

            let pos = [15, 64, 2];
            let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
            let stone = game.world.block(pos[0], pos[1], pos[2]);
            let air = game.world.air;
            for (sequence, server, shown, id) in [(1, stone, air, 88), (2, air, air, 99)] {
                game.predict_blocks(&[(at, shown)], sequence);
                let group_before = level.connection_group(game, pos);
                deliver_group_bundle(
                    game,
                    vec![
                        C::BlockUpdate(play::BlockUpdate {
                            pos,
                            state: server.0 as i32,
                        }),
                        group_packet(pos, Some(id)),
                    ],
                );
                assert_eq!(game.world.block(pos[0], pos[1], pos[2]), shown);
                assert_eq!(level.connection_group(game, pos), group_before);
                game.handle_packet(C::BlockChangedAck(play::BlockChangedAck { sequence }))
                    .unwrap();
                assert_eq!(game.world.block(pos[0], pos[1], pos[2]), server);
                assert_eq!(level.connection_group(game, pos), Ok(Some(id)));
                assert!(game.world.dirty_sections.contains(&(1, sy, 0)));
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_column_snapshot_supersedes_only_older_deferred_group_corrections() {
        let mut resources = None;
        let (mut game, _server) = client_with_pipe(&mut resources);
        negotiate(&game, true);
        let pos = [3, 64, 5];
        let [x, y, z] = pos;
        let at = sunrise_core::position::BlockPos::new(x, y, z);
        let column = viewed_column(0, 0);
        game.handle_packet(column.clone()).unwrap();
        let state = game.world.block(x, y, z);
        let old_state = game.world.air;
        let later_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:dirt")
            .unwrap();
        assert_ne!(old_state, state);
        assert_ne!(later_state, state);
        let level = game.level("minecraft:overworld").unwrap();
        for (sequence, snapshot, later_group) in [
            (1, Some(27), None),
            (2, None, None),
            (3, Some(27), Some(41)),
        ] {
            game.predict_blocks(&[(at, state)], sequence);
            let before = level.connection_group(&game, pos);
            deliver_group_bundle(
                &mut game,
                vec![
                    C::BlockUpdate(play::BlockUpdate {
                        pos,
                        state: old_state.0 as i32,
                    }),
                    group_packet(pos, Some(19)),
                ],
            );
            assert_eq!(level.connection_group(&game, pos), before);
            if let Some(group) = snapshot {
                deliver_group_bundle(
                    &mut game,
                    vec![column.clone(), group_packet(pos, Some(group))],
                );
            } else {
                game.handle_packet(column.clone()).unwrap();
            }
            assert_eq!(game.world.block(x, y, z), state);
            assert_eq!(level.connection_group(&game, pos), Ok(snapshot));
            if let Some(group) = later_group {
                deliver_group_bundle(
                    &mut game,
                    vec![
                        C::BlockUpdate(play::BlockUpdate {
                            pos,
                            state: later_state.0 as i32,
                        }),
                        group_packet(pos, Some(group)),
                    ],
                );
                assert_eq!(game.world.block(x, y, z), state);
                assert_eq!(level.connection_group(&game, pos), Ok(snapshot));
            }
            game.handle_packet(C::BlockChangedAck(play::BlockChangedAck { sequence }))
                .unwrap();
            let expected_state = if later_group.is_some() {
                later_state
            } else {
                state
            };
            assert_eq!(game.world.block(x, y, z), expected_state);
            assert_eq!(
                level.connection_group(&game, pos),
                Ok(later_group.or(snapshot))
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn received_groups_join_and_unjoin_a_real_mesh_across_the_column_border() {
        use sunrise_assets::resource_source::{Resource, ResourceRoot};
        use sunrise_client::{
            graphics::LeafCullingMode,
            mesher::{MeshScratch, MeshTables, Mesher, SectionMesh},
        };
        use sunrise_core::biome::BiomeZoom;

        let mut resources = None;
        let (mut game, _server) = client_with_pipe(&mut resources);
        negotiate(&game, true);
        let root = ResourceRoot::directory(game.options.resources.clone());
        let mut sheet = Vec::new();
        {
            let mut png = png::Encoder::new(&mut sheet, 2, 1);
            png.set_color(png::ColorType::Rgb);
            png.write_header()
                .unwrap()
                .write_image_data(&[200, 0, 0, 0, 0, 200])
                .unwrap();
        }
        let rule = br#"{
            "method": "table", "blocks": ["minecraft:stone"], "faces": ["up"],
            "connect_group": true, "sheet": "test:block/group",
            "table": {"tests": ["left", "right"], "grid": [2, 1], "tiles": [0, 1, 1, 1]}
        }"#;
        let root = root.with_runtime_textures(
            [
                (
                    "assets/test/textures/block/group.png".into(),
                    Resource::bytes(sheet),
                ),
                (
                    "assets/test/sunrise/connected/group.json".into(),
                    Resource::bytes(rule.to_vec()),
                ),
            ]
            .into_iter()
            .collect(),
        );
        let assets = game.pack.assets.clone();
        let render = sunrise_client::assets::RenderAssets::load(&root, &assets).unwrap();
        assert!(render.connected.has_group_rules());
        let visibility =
            sunrise_client::models::load_visibility(root.prepared(), assets.registry).unwrap();
        let tables =
            MeshTables::new(&assets, &render, game.res.facts(), visibility.clone(), None).unwrap();
        let biomes = game.res.biomes().clone();
        let mut mesher = Mesher {
            nether_shading: false,
            shader_pack_lighting: false,
            separate_ao: false,
            shader_pack: false,
            assets: &assets,
            render: &render,
            tables: &tables,
            biomes: &biomes,
            visibility,
            zoom: BiomeZoom::new(0),
            blend_radius: 0,
            smooth_lighting: false,
            cutout_leaves: false,
            opaque_ice: false,
            translucent_layers: &[],
            layer_overrides: &[],
            leaf_culling: LeafCullingMode::Off,
            hidden_states: &[],
            part_tables: Default::default(),
        };
        let mut scratch = MeshScratch::default();
        let sy = (79 - MIN_Y) >> 4;
        let top_layer = |game: &sunrise_client_gameplay::game::ClientGame<'_>,
                         mesher: &mut Mesher<'_>,
                         scratch: &mut MeshScratch| {
            let mut region = [[None; 3]; 3];
            region[1][1] = game.world.chunks.get(&(0, 0)).map(Arc::as_ref);
            region[1][2] = game.world.chunks.get(&(1, 0)).map(Arc::as_ref);
            let mesh = mesher.mesh_reusing(&region, sy, SectionMesh::default(), scratch);
            mesh.solid
                .as_chunks::<4>()
                .0
                .iter()
                .find(|quad| {
                    quad.iter().all(|v| v.pos[1] == 16.0)
                        && quad.iter().any(|v| v.pos[0] == 15.0 && v.pos[2] == 2.0)
                        && quad.iter().any(|v| v.pos[0] == 16.0 && v.pos[2] == 3.0)
                })
                .expect("stone top face at the column edge")[0]
                .layer
        };
        let send_column =
            |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, cx, local_x| {
                deliver_group_bundle(
                    game,
                    vec![
                        crate::game::client_side::viewed_worlds::tests::layered(cx, 0, 9),
                        group_packet([(cx << 4) + local_x, 79, 2], Some(7)),
                    ],
                );
            };
        send_column(&mut game, 0, 15);
        let alone = top_layer(&game, &mut mesher, &mut scratch);
        send_column(&mut game, 1, 0);
        let joined = top_layer(&game, &mut mesher, &mut scratch);
        assert_ne!(alone, joined);
        game.world.dirty_sections.clear();
        game.handle_packet(C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [1, 0] }))
            .unwrap();
        assert!(game.world.dirty_sections.contains(&(0, sy, 0)));
        assert_eq!(top_layer(&game, &mut mesher, &mut scratch), alone);
    }

    #[test]
    #[ignore = "daily"]
    fn viewed_group_bundles_are_scoped_and_refuse_invalid_targets() {
        let mut resources = None;
        let pos = [15, 64, 2];
        let packet = |pos| C::BlockUpdate(play::BlockUpdate { pos, state: 1 });
        let send = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, pos, group| {
            deliver_group_bundle(
                game,
                vec![
                    viewed_run(1, &[packet(pos)]),
                    viewed_run(1, &[group_packet(pos, group)]),
                ],
            );
        };
        for reject in [None, Some(false), Some(true)] {
            let (mut game, _server) = client_with_pipe(&mut resources);
            // Vanilla server adoption, before any mod negotiation, is unchanged.
            game.handle_packet(viewed_column(0, 0)).unwrap();
            let own = game.level("minecraft:overworld").unwrap();
            assert!(own.block_state(&game, pos).is_some());
            assert_eq!(own.connection_group(&game, pos), Ok(None));
            negotiate(&game, reject.unwrap_or(true));
            game.handle_packet(viewed_open(1, "minecraft:the_nether"))
                .unwrap();
            if let Some(with_groups) = reject {
                game.handle_packet(viewed_run(1, &[viewed_column(0, 0)]))
                    .unwrap();
                let target = if with_groups { [32, 64, 0] } else { pos };
                send(&mut game, target, Some(70));
                assert!(
                    game.client.remote.is_none(),
                    "unnegotiated or unloaded group target must disconnect"
                );
                continue;
            }
            deliver_group_bundle(
                &mut game,
                vec![
                    viewed_run(1, &[viewed_column(0, 0)]),
                    viewed_run(1, &[group_packet(pos, Some(50))]),
                ],
            );
            let viewed = game.level("minecraft:the_nether").unwrap();
            let count = game.client.remote.as_ref().unwrap().group_count.clone();
            assert!(!count.allows_replacement(0, 1, 1));
            assert_eq!(viewed.connection_group(&game, pos), Ok(Some(50)));
            assert_eq!(own.connection_group(&game, pos), Ok(None));
            send(&mut game, pos, Some(60));
            assert_eq!(viewed.connection_group(&game, pos), Ok(Some(60)));
            send(&mut game, pos, None);
            assert_eq!(viewed.connection_group(&game, pos), Ok(None));
            assert!(count.allows_replacement(0, 1, 1));
            game.handle_packet(viewed_close(1)).unwrap();
            assert_eq!(own.connection_group(&game, pos), Ok(None));
        }
    }
}
