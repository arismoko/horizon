#![cfg(test)]
use sunrise_world::carrier::codec;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_sim::save::carrier::Store;

    use sunrise_gameplay::game::server_side::carrier_transfers::TransferError;

    use sunrise_gameplay_common::game::server_side::carrier_transfers::tests::copy_dir;

    use sunrise_gameplay_common::game::server_side::carrier_transfers::tests::region_files;

    use sunrise_gameplay_common::game::server_side::carrier_transfers::tests::transfer_root;

    use sunrise_gameplay_common::game::server_side::carrier_transfers::tests::write_level_dat;

    use super::*;

    use crate::game::native_entity_refs::tests::with_game;

    use crate::game::spill_common;

    use std::sync::Arc;

    use sunrise_core::geometry::Direction as CoreDirection;

    use sunrise_core::position::BlockPos;

    use sunrise_gameplay_common::world_biomes;

    /// One transfer's whole life: the chest moves to the carrier, its
    /// cell becomes air, the operation commits, and the block has
    /// exactly one owner — the carrier file, decoded back.
    #[test]
    #[ignore = "daily"]
    fn assembling_moves_a_chest_to_one_owner_and_commits() {
        let root =
            std::env::temp_dir().join(format!("sunrise-carrier-transfer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        sunrise_sim::save::write_dat(
            &root.join("level.dat"),
            sunrise_core::nbt::Tag::compound([(
                "Data",
                sunrise_core::nbt::Tag::Compound(Default::default()),
            )]),
        )
        .unwrap();
        with_game(|game| {
            // The harness's world root is empty: give the transfer a
            // throwaway world of its own, as the save tests do.
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();

            game.carrier_assemble(7, &[BlockPos::new(4, 64, 4)])
                .unwrap();

            // The cell is air and the fence is lifted.
            assert_eq!(game.world.block(4, 64, 4), air);
            assert!(!game.world.carrier_fences.locks_cell(&[4, 64, 4]));
            assert!(!game.world.carrier_fences.locks_carrier(7));

            // One owner: the chest lives in the carrier file, and only
            // there — the op file is gone and the journal committed.
            let carrier_bytes = std::fs::read(
                root.join("data/sunrise_carrier_00000000000000000000000000000007-1.nbt"),
            )
            .unwrap();
            let carrier = codec::decode(&carrier_bytes).unwrap();
            assert_eq!(carrier.uuid(), 7);
            assert_eq!(carrier.cells().len(), 1);
            assert_eq!(carrier.cells()[0].name(), "minecraft:chest");
            let mut leftovers = std::fs::read_dir(root.join("data")).unwrap();
            assert!(leftovers.all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_str()
                    .unwrap()
                    .starts_with("sunrise_carrier_op_")
            }));
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// While a fence holds a cell, each refusal route refuses exactly at
    /// its one chokepoint, and the moment the fence clears they pass.
    #[test]
    #[ignore = "daily"]
    fn fenced_cells_refuse_every_route_at_its_chokepoint() {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let fenced = [4, 64, 4];
            game.world.carrier_fences.lock_cell(fenced);
            game.world.carrier_fences.lock_carrier(7);
            // The carrier is fenced too: its inventory and motion have
            // no route of their own until slice 4, so the fence read is
            // the seam that will refuse them.
            assert!(game.world.carrier_fences.locks_carrier(7));

            // Breaking.
            assert!(game.break_block(fenced).is_err());
            // Pistons: a move pushing the fenced cell refuses whole.
            game.with_level(|_b, level, _ticks| {
                assert!(!sunrise_world::level::LevelAccess::allow_piston_move(
                    level,
                    BlockPos::new(4, 64, 3),
                    CoreDirection::North,
                    true,
                    &[BlockPos::new(4, 64, 4)],
                    &[]
                ));
            });
            // The block entity's container does not open.
            assert!(game.refuse_locked_container((4, 64, 4), "minecraft:chest"));
            // Every mod write through the level gate refuses.
            let level = game.level(game.dim.id()).unwrap();
            assert!(!game.native_level_write_allowed(level, fenced));

            // No operation, no refusals: the checks cost one set load.
            game.world.carrier_fences.unlock_cell(fenced);
            assert!(game.native_level_write_allowed(level, fenced));
            assert!(!game.refuse_locked_container((4, 64, 4), "minecraft:chest"));
        });
    }
    /// (item 7) An empty selection refuses before any indexing.
    #[test]
    #[ignore = "daily"]
    fn an_empty_assembly_refuses() {
        with_game(|game| match game.carrier_assemble(51, &[]) {
            Err(TransferError::EmptySelection) => {}
            other => panic!("an empty selection must refuse, got {other:?}"),
        });
    }
    /// (item 7) A destination arithmetic that overflows refuses instead
    /// of wrapping the world.
    #[test]
    #[ignore = "daily"]
    fn an_overflowing_destination_refuses() {
        let root = transfer_root("overflow");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.carrier_assemble(52, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            // A translation that overflows i32 on add refuses.
            match game.carrier_disassemble(52, 1, [i32::MAX, 64, 4], [1.0, 0.0, 0.0, 0.0]) {
                // Overflow past the loaded area reports the unreachable
                // chunk as often as the range: either refusal is honest.
                Err(TransferError::LocalRange) | Err(TransferError::CellNotLoaded(_)) => {}
                other => panic!("an overflowing destination must refuse, got {other:?}"),
            }
            // The carrier stays whole.
            assert!(
                root.join(format!("data/sunrise_carrier_{:032x}-1.nbt", 52_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// (item 7) An unsupported block entity in the selection refuses in
    /// preflight — the world keeps the block and its entity.
    #[test]
    #[ignore = "daily"]
    fn a_ticking_block_entity_refuses_before_the_lift() {
        let root = transfer_root("passive");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let furnace = registry.default_state("minecraft:furnace").unwrap();
            game.world.set(4, 64, 4, furnace).unwrap();
            game.world.block_entities.insert(
                (4, 64, 4),
                sunrise_level::chunk::block_entity::BlockEntity::Furnace(
                    sunrise_level::chunk::block_entity::Furnace {
                        kind: sunrise_level::chunk::block_entity::FurnaceKind::Furnace,
                        items: [None, None, None],
                        lit_remaining: 0,
                        lit_total: 0,
                        cooking_timer: 0,
                        cooking_total: 0,
                        speed: 1.0,
                        recipes_used: Default::default(),
                    },
                ),
            );
            match game.carrier_assemble(53, &[BlockPos::new(4, 64, 4)]) {
                Err(TransferError::UnsupportedBlock(pos)) => assert_eq!(pos, [4, 64, 4]),
                other => panic!("a furnace must refuse, got {other:?}"),
            }
            // The world is untouched: the block and its entity remain,
            // and no store was created for this transfer.
            assert_eq!(game.world.block(4, 64, 4), furnace);
            assert!(game.world.block_entities.contains_key(&(4, 64, 4)));
            assert!(
                !root
                    .join(format!("data/sunrise_carrier_{:032x}-1.nbt", 53_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// The live index capacity is separate from the retained tombstone
    /// capacity: ten sequential transfers fit an eight-carrier index,
    /// but all ten deletion witnesses survive this world session.
    #[test]
    #[ignore = "daily"]
    fn repeated_transfers_beyond_the_index_capacity_stay_bounded() {
        let root = transfer_root("capacity");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            // Eight live index entries are allowed; tombstones have their
            // own independent section bound.
            game.world.carrier_fences.capacity_override = Some(8);
            for i in 0..10u32 {
                game.world.set(4, 64, 4, chest).unwrap();
                game.carrier_assemble(1000 + i as u128, &[BlockPos::new(4, 64, 4)])
                    .unwrap_or_else(|e| panic!("cycle {i}: {e}"));
                game.carrier_disassemble(
                    1000 + i as u128,
                    2 * i + 1,
                    [4, 64, 4],
                    [1.0, 0.0, 0.0, 0.0],
                )
                .unwrap_or_else(|e| panic!("cycle {i}: {e}"));
            }
            let sunrise_sim::save::carrier::Opened::Live(store) = Store::open(&root).unwrap()
            else {
                panic!("must open");
            };
            assert_eq!(store.journal().tombstones.len(), 10);
            // Bounded: no operation files, no carrier files beyond the
            // journal's two slots.
            let mut files = 0;
            for entry in std::fs::read_dir(root.join("data")).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("sunrise_carrier") {
                    files += 1;
                }
            }
            assert!(
                files <= 2,
                "the store must stay bounded at the two slots, found {files} files"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    #[ignore = "daily"]
    fn capacity_refusal_leaves_world_untouched() {
        let root = transfer_root("admission");
        with_game(|game| {
            game.server.world_root = root.clone();
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.carrier_fences.capacity_override = Some(0);
            game.world.set(4, 64, 4, chest).unwrap();
            let error = game
                .carrier_assemble(7, &[BlockPos::new(4, 64, 4)])
                .unwrap_err();
            assert!(error.to_string().contains("CarrierCapacityReopenNeeded"));
            assert_eq!(game.world.block(4, 64, 4), chest);
            assert!(!game.world.carrier_fences.locks_cell(&[4, 64, 4]));
            let sunrise_sim::save::carrier::Opened::Live(store) =
                Store::open_with_capacity(&root, 0).unwrap()
            else {
                panic!("must open")
            };
            assert!(store.journal().in_progress.is_none());
            assert!(store.journal().carriers.is_empty());
            assert!(!root.join("dimensions/minecraft/overworld/region").exists());
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    #[ignore = "daily"]
    fn saved_tag_refuses_assembly_and_disassembly_without_mutation() {
        let root = transfer_root("saved-tag");
        with_game(|game| {
            game.server.world_root = root.clone();
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            let entity = sunrise_level::chunk::block_entity::BlockEntity::Chest(vec![
                None,
                Some(sunrise_items::items::ItemStack {
                    item: "minecraft:stone".into(),
                    count: 3,
                    damage: 0,
                    extra: None,
                }),
            ]);
            let tag = sunrise_sim::save::block_entity_encode::encode(
                (4, 64, 4),
                &entity,
                "minecraft:chest",
                &Default::default(),
            );
            spill_common::put_saved_entity(&mut game.world, [4, 64, 4], tag.clone());
            assert!(matches!(
                game.carrier_assemble(7, &[BlockPos::new(4, 64, 4)]),
                Err(TransferError::SavedBlockEntity([4, 64, 4]))
            ));
            assert_eq!(game.world.block(4, 64, 4), chest);
            assert!(
                std::fs::read_dir(root.join("data"))
                    .unwrap()
                    .next()
                    .is_none()
            );
            Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).saved = None;
            game.carrier_assemble(7, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            spill_common::put_saved_entity(&mut game.world, [4, 64, 4], tag.clone());
            assert!(matches!(
                game.carrier_disassemble(7, 1, [4, 64, 4], [1.0, 0.0, 0.0, 0.0]),
                Err(TransferError::SavedBlockEntity([4, 64, 4]))
            ));
            let images = [sunrise_sim::save::carrier::OperationCell {
                world: [4, 64, 4],
                local: [0, 0, 0],
                before: sunrise_sim::save::carrier::CellImage {
                    name: "minecraft:chest".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
                after: sunrise_sim::save::carrier::CellImage {
                    name: "minecraft:air".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
            }];
            assert!(matches!(
                game.revalidate_destinations(&images),
                Err(TransferError::SavedBlockEntity([4, 64, 4]))
            ));
            assert_eq!(game.world.block(4, 64, 4), air);
            assert_eq!(
                spill_common::saved_entity(&game.world, [4, 64, 4]),
                Some(tag)
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    #[ignore = "daily"]
    fn assembly_revalidation_refuses_saved_tag_without_mutation() {
        let root = transfer_root("assembly-revalidation-saved-tag");
        with_game(|game| {
            game.server.world_root = root.clone();
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            let entity = sunrise_level::chunk::block_entity::BlockEntity::Chest(vec![
                None,
                Some(sunrise_items::items::ItemStack {
                    item: "minecraft:stone".into(),
                    count: 3,
                    damage: 0,
                    extra: None,
                }),
            ]);
            let tag = sunrise_sim::save::block_entity_encode::encode(
                (4, 64, 4),
                &entity,
                "minecraft:chest",
                &Default::default(),
            );
            spill_common::put_saved_entity(&mut game.world, [4, 64, 4], tag.clone());
            let images = [sunrise_sim::save::carrier::OperationCell {
                world: [4, 64, 4],
                local: [0, 0, 0],
                before: sunrise_sim::save::carrier::CellImage {
                    name: "minecraft:chest".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
                after: sunrise_sim::save::carrier::CellImage {
                    name: "minecraft:air".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
            }];
            assert!(matches!(
                game.revalidate_cells(&images, registry),
                Err(TransferError::SavedBlockEntity([4, 64, 4]))
            ));
            assert_eq!(game.world.block(4, 64, 4), chest);
            assert_eq!(
                spill_common::saved_entity(&game.world, [4, 64, 4]),
                Some(tag)
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// (item 6) A crash after a finished disassembly: the tombstone
    /// outlives the session, and the next open's recovery-plus-cleanup
    /// resolves it cleanly — the carrier file is gone, the journal
    /// holds no operation, and the world reopens.
    #[test]
    #[ignore = "daily"]
    fn a_crash_after_a_finished_disassembly_recovers_clean() {
        let root = transfer_root("post-disassembly");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.carrier_assemble(61, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            game.carrier_disassemble(61, 1, [4, 64, 4], [1.0, 0.0, 0.0, 0.0])
                .unwrap();

            // The next open: recovery (nothing pending) plus the
            // production cleanup, which the open route runs.
            game.carrier_recover_all_levels().unwrap();

            // Clean: no tombstone, no carrier file, no operation.
            let store = match sunrise_sim::save::carrier::Store::open(&root).unwrap() {
                sunrise_sim::save::carrier::Opened::Live(store) => store,
                _ => panic!("the store must open"),
            };
            assert!(store.journal().in_progress.is_none());
            assert!(!store.journal().tombstoned(61, 1));
            assert!(
                !root
                    .join(format!("data/sunrise_carrier_{:032x}-1.nbt", 61_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// (item 2) Sparse, non-collinear selections keep their geometry:
    /// the local grid is offset from the selection's minimum corner, so
    /// gaps stay gaps and the input order is irrelevant — the same
    /// selection shuffled produces the byte-identical carrier.
    #[test]
    #[ignore = "daily"]
    fn sparse_selection_geometry_survives_and_order_is_irrelevant() {
        let root = transfer_root("geometry");
        let sparse = [
            BlockPos::new(4, 64, 4),
            BlockPos::new(9, 64, 4),
            BlockPos::new(4, 68, 7),
            BlockPos::new(7, 71, 9),
        ];
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            for pos in sparse {
                game.world.set(pos.x, pos.y, pos.z, stone).unwrap();
            }
            // Forward order.
            game.carrier_assemble(31, &sparse).unwrap();
            let forward =
                std::fs::read(root.join(format!("data/sunrise_carrier_{:032x}-1.nbt", 31_u128)))
                    .unwrap();
            let carrier = codec::decode(&forward).unwrap();
            // Local positions are offsets from the minimum corner (4,64,4):
            // the gaps survive as gaps.
            let mut locals: Vec<[i8; 3]> = carrier
                .cells()
                .iter()
                .map(|c| [c.pos().x(), c.pos().y(), c.pos().z()])
                .collect();
            locals.sort();
            assert_eq!(locals, [[0, 0, 0], [0, 4, 3], [3, 7, 5], [5, 0, 0]]);

            // The same selection in reverse order (a second UUID):
            // the cells decode to the identical geometry.
            for pos in sparse {
                game.world.set(pos.x, pos.y, pos.z, stone).unwrap();
            }
            let mut reversed = sparse.to_vec();
            reversed.reverse();
            game.carrier_assemble(32, &reversed).unwrap();
            let backward =
                std::fs::read(root.join(format!("data/sunrise_carrier_{:032x}-2.nbt", 32_u128)))
                    .unwrap();
            let forward_carrier = codec::decode(&forward).unwrap();
            let backward_carrier = codec::decode(&backward).unwrap();
            assert_eq!(forward_carrier.transform(), backward_carrier.transform());
            assert_eq!(
                forward_carrier.cells(),
                backward_carrier.cells(),
                "the geometry is identical regardless of input order"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// (item 2) A non-default state survives assembly and disassembly
    /// exactly: oak stairs keep facing=north and half=bottom, a chest
    /// keeps facing — resolved from the saved properties at placement,
    /// never from the block's default state.
    #[test]
    #[ignore = "daily"]
    fn non_default_states_survive_the_round_trip() {
        let root = transfer_root("states");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            // Oak stairs, facing north, half bottom — not the default
            // (the default stairs face north only by luck; resolve the
            // exact state and check it differs from the default).
            // Oak stairs, facing south and top-half — the opposite of
            // the default — so the test genuinely pins a non-default
            // state through the round trip.
            let descriptor = sunrise_world::chunk::BlockState {
                name: "minecraft:oak_stairs".to_string(),
                properties: [
                    ("facing".to_string(), "south".to_string()),
                    ("half".to_string(), "top".to_string()),
                    ("shape".to_string(), "straight".to_string()),
                    ("waterlogged".to_string(), "false".to_string()),
                ]
                .into_iter()
                .collect(),
            };
            let stairs = registry.resolve(&descriptor).unwrap();
            let default_stairs = registry.default_state("minecraft:oak_stairs").unwrap();
            let stairs_props = &registry.state(stairs).unwrap().properties;
            let default_props = &registry.state(default_stairs).unwrap().properties;
            assert!(
                stairs_props
                    .iter()
                    .any(|(k, v)| { default_props.get(k).is_some_and(|dv| dv != v) }),
                "south/top must differ from the default stairs state"
            );
            game.world.set(4, 64, 4, stairs).unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            game.carrier_assemble(41, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            assert_eq!(game.world.block(4, 64, 4), air);
            game.carrier_disassemble(41, 1, [4, 64, 4], [1.0, 0.0, 0.0, 0.0])
                .unwrap();
            // The exact state came back, not the default.
            assert_eq!(
                game.world.block(4, 64, 4),
                stairs,
                "the non-default state survives the round trip"
            );
            let restored = registry.state(game.world.block(4, 64, 4)).unwrap();
            assert_eq!(restored.name, "minecraft:oak_stairs");
            assert_eq!(
                restored.properties.get("facing").map(String::as_str),
                stairs_props.get("facing").map(String::as_str)
            );
            assert_eq!(
                restored.properties.get("half").map(String::as_str),
                stairs_props.get("half").map(String::as_str)
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// (a1) The live order, not the test helper: a real carrier_assemble
    /// whose flush fails after prepared, then recovery — the cargo (cells,
    /// states, block entities, items) must be in the published carrier
    /// file, and the source cell empty. If the live code ever publishes
    /// the carrier after prepared again, this test fails.
    #[test]
    #[ignore = "daily"]
    fn the_live_assembly_publishes_the_cargo_before_prepared() {
        let root = transfer_root("live-cargo");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.world.set(6, 64, 4, chest).unwrap();
            game.world.block_entities.insert(
                (4, 64, 4),
                sunrise_level::chunk::block_entity::BlockEntity::Chest(Vec::new()),
            );
            game.world.block_entities.insert(
                (6, 64, 4),
                sunrise_level::chunk::block_entity::BlockEntity::Chest(vec![
                    None,
                    Some(sunrise_items::items::ItemStack {
                        item: "minecraft:stone".into(),
                        count: 3,
                        damage: 0,
                        extra: None,
                    }),
                ]),
            );

            // Crash after prepared.
            game.world.carrier_fences.fail_flush = true;
            assert!(
                game.carrier_assemble(21, &[BlockPos::new(4, 64, 4), BlockPos::new(6, 64, 4)])
                    .is_err()
            );

            // Recovery, as the next open runs it.
            let mut store = match sunrise_sim::save::carrier::Store::open(&root).unwrap() {
                sunrise_sim::save::carrier::Opened::Live(store) => store,
                _ => panic!("the store must reopen"),
            };
            let region = root.join("dimensions/minecraft/overworld/region");
            let states = sunrise_sim::save::carrier::RegistryStates::new(registry);
            let report = sunrise_sim::save::carrier::recover(
                &mut store,
                &region,
                &states,
                "minecraft:overworld",
            )
            .unwrap();
            assert_eq!(report.applied + report.verified, 1);
            assert!(store.journal().in_progress.is_none());

            // The source cells are empty.
            for x in [4, 6] {
                assert_eq!(game.world.block(x, 64, 4), air);
            }

            // The cargo lives in the published carrier: both cells, the
            // chest states, and the entities' items.
            let bytes =
                std::fs::read(root.join(format!("data/sunrise_carrier_{:032x}-1.nbt", 21_u128)))
                    .unwrap();
            let carrier = codec::decode(&bytes).unwrap();
            assert_eq!(carrier.cells().len(), 2, "the whole cargo is carried");
            let names: Vec<&str> = carrier.cells().iter().map(|c| c.name()).collect();
            assert!(names.iter().all(|n| *n == "minecraft:chest"));
            // The entity payload rides each cell; one of them carries
            // an item stack.
            let carried: Vec<usize> = carrier
                .cells()
                .iter()
                .map(|c| c.data().map(|d| d.len()).unwrap_or(0))
                .collect();
            assert!(
                carried.iter().all(|len| *len > 0),
                "both entities are carried: {carried:?}"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// A flush that fails after `prepared`: the fences hold, the next
    /// transfer refuses, and recovery at a reopen finishes the
    /// operation with the chest's items in exactly one place.
    #[test]
    #[ignore = "daily"]
    fn a_flush_failure_after_prepared_recovers_on_reopen() {
        let root =
            std::env::temp_dir().join(format!("sunrise-carrier-flush-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        write_level_dat(&root);
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();

            // The writer's barrier fails after prepared.
            game.world.carrier_fences.fail_flush = true;
            assert!(
                game.carrier_assemble(7, &[BlockPos::new(4, 64, 4)])
                    .is_err()
            );
            // The fences hold and the world refuses further transfers.
            assert!(game.world.carrier_fences.locks_cell(&[4, 64, 4]));
            assert!(game.world.carrier_fences.is_broken());
            assert!(
                game.carrier_assemble(8, &[BlockPos::new(6, 64, 6)])
                    .is_err()
            );

            // Recovery, as the next open runs it: the operation replays,
            // the journal commits and the chest is air in the world.
            let mut store = match sunrise_sim::save::carrier::Store::open(&root).unwrap() {
                sunrise_sim::save::carrier::Opened::Live(store) => store,
                _ => panic!("the store must reopen"),
            };
            let region = root.join("dimensions/minecraft/overworld/region");
            let states = sunrise_sim::save::carrier::RegistryStates::new(registry);
            let report = sunrise_sim::save::carrier::recover(
                &mut store,
                &region,
                &states,
                "minecraft:overworld",
            )
            .unwrap();
            // The interrupted save left the post-state on disk, so the
            // reopen verifies it rather than replaying — either way the
            // operation commits and exactly one side owns the chest.
            assert_eq!(report.applied + report.verified, 1);
            assert!(
                store.journal().in_progress.is_none(),
                "the operation committed"
            );
            // One owner: the items live in the carrier file, and the
            // operation file is gone.
            assert!(
                root.join("data/sunrise_carrier_00000000000000000000000000000007-1.nbt")
                    .is_file()
            );
            assert!(!root.join("data/sunrise_carrier_op_1.nbt").is_file());
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// Review item 5's fail-closed: a commit that fails after
    /// `prepared` stops the whole session, not just further transfers.
    /// The game's fatal error is set (what the event loop's error path
    /// turns into its exit), a further edit of the fenced cell refuses,
    /// every save refuses so nothing is written over the region before
    /// the exit, and the next open's recovery still finishes the
    /// operation with exactly one owner.
    #[test]
    #[ignore = "daily"]
    fn a_failed_commit_stops_the_session_until_recovery() {
        let root = transfer_root("fail-closed");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();

            // The writer's barrier fails after prepared.
            game.world.carrier_fences.fail_flush = true;
            assert!(
                game.carrier_assemble(9, &[BlockPos::new(4, 64, 4)])
                    .is_err()
            );

            // The session stopped like a fatal save error: `Game::error`
            // is set, which tells the event handler to exit the loop.
            assert!(game.error.is_some(), "the failed commit stops the session");

            // A further edit of the fenced cell refuses.
            assert!(game.world.carrier_fences.locks_cell(&[4, 64, 4]));
            assert!(game.break_block([4, 64, 4]).is_err());

            // Every save refuses — the autosave's and the exit path's
            // chokepoint — so nothing is written over the region
            // between the failure and the exit.
            let region = root.join("dimensions/minecraft/overworld/region");
            let before = region_files(&region);
            assert!(
                game.save_game().is_err(),
                "a save refuses after the failure"
            );
            assert_eq!(before, region_files(&region), "the region is untouched");

            // Recovery, as the next open runs it: the operation replays
            // or verifies, commits, and the chest has exactly one owner.
            let mut store = match sunrise_sim::save::carrier::Store::open(&root).unwrap() {
                sunrise_sim::save::carrier::Opened::Live(store) => store,
                _ => panic!("the store must reopen"),
            };
            let states = sunrise_sim::save::carrier::RegistryStates::new(registry);
            let report = sunrise_sim::save::carrier::recover(
                &mut store,
                &region,
                &states,
                "minecraft:overworld",
            )
            .unwrap();
            assert_eq!(report.applied + report.verified, 1);
            assert!(store.journal().in_progress.is_none());
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// Assemble then disassemble: at every step the chest has exactly
    /// one owner — the world, the carrier's images or the carrier file.
    #[test]
    #[ignore = "daily"]
    fn assemble_then_disassemble_keeps_one_owner_at_every_step() {
        let root = transfer_root("cycle");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();

            game.carrier_assemble(9, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            assert_eq!(game.world.block(4, 64, 4), air);
            let carrier_bytes =
                std::fs::read(root.join(format!("data/sunrise_carrier_{:032x}-1.nbt", 9_u128)))
                    .unwrap();
            assert_eq!(codec::decode(&carrier_bytes).unwrap().cells().len(), 1);

            game.carrier_disassemble(9, 1, [4, 64, 4], [1.0, 0.0, 0.0, 0.0])
                .unwrap();
            assert_eq!(game.world.block(4, 64, 4), chest);
            // The carrier file is gone: one owner, the world.
            assert!(
                !root
                    .join(format!("data/sunrise_carrier_{:032x}-1.nbt", 9_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// A destination that already holds a block entity refuses, with
    /// the carrier intact.
    #[test]
    #[ignore = "daily"]
    fn a_destination_with_a_block_entity_refuses() {
        let root = transfer_root("dest");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.carrier_assemble(5, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            // A second chest occupies the destination cell.
            game.world.set(10, 64, 10, chest).unwrap();
            game.world.block_entities.insert(
                (10, 64, 10),
                sunrise_level::chunk::block_entity::BlockEntity::Chest(Vec::new()),
            );
            match game.carrier_disassemble(5, 1, [10, 64, 10], [1.0, 0.0, 0.0, 0.0]) {
                Err(TransferError::DestinationOccupied(pos)) => assert_eq!(pos, [10, 64, 10]),
                other => panic!("expected an occupied destination, got {other:?}"),
            }
            assert!(
                root.join(format!("data/sunrise_carrier_{:032x}-1.nbt", 5_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// A rotation that is not a cube symmetry refuses; a cube rotation
    /// without an adapter refuses too; the carrier survives both.
    #[test]
    #[ignore = "daily"]
    fn rotations_refuse_without_symmetry_or_an_adapter() {
        let root = transfer_root("rot");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.carrier_assemble(6, &[BlockPos::new(4, 64, 4)])
                .unwrap();

            // A sixth of a turn about Y: not a cube symmetry.
            let half = 0.5_f64;
            match game.carrier_disassemble(6, 1, [8, 64, 8], [half, 0.0, half, 0.0]) {
                Err(TransferError::NotCubeSymmetry) => {}
                other => panic!("expected a symmetry refusal, got {other:?}"),
            }
            // A quarter turn is a symmetry, but no adapter exists yet.
            let quarter = std::f64::consts::FRAC_1_SQRT_2;
            match game.carrier_disassemble(6, 1, [8, 64, 8], [quarter, 0.0, quarter, 0.0]) {
                Err(TransferError::RotationNeedsAdapter) => {}
                other => panic!("expected an adapter refusal, got {other:?}"),
            }
            assert!(
                root.join(format!("data/sunrise_carrier_{:032x}-1.nbt", 6_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// Revalidation itself: images built from the world, the world then
    /// changed underneath — the mismatch refuses and nothing moved.
    #[test]
    #[ignore = "daily"]
    fn revalidation_refuses_a_changed_cell_without_touching_the_world() {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            let descriptor = registry.state(chest).unwrap();
            let properties = descriptor
                .properties
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let images = [sunrise_sim::save::carrier::OperationCell {
                world: [4, 64, 4],
                local: [0, 0, 0],
                before: sunrise_sim::save::carrier::CellImage {
                    name: "minecraft:chest".to_string(),
                    properties,
                    entity: None,
                },
                after: sunrise_sim::save::carrier::CellImage {
                    name: "minecraft:air".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
            }];
            assert!(game.revalidate_cells(&images, registry).is_ok());
            game.world.set(4, 64, 4, stone).unwrap();
            match game.revalidate_cells(&images, registry) {
                Err(TransferError::Revalidation(pos)) => assert_eq!(pos, [4, 64, 4]),
                other => panic!("expected a revalidation refusal, got {other:?}"),
            }
            // The world keeps the changed cell; nothing was lifted.
            assert_eq!(game.world.block(4, 64, 4), stone);
        });
    }
    /// A Nether transfer that crashed after `prepared`, recovered by a
    /// world opening in the Overworld with the Nether not parked: the
    /// operation replays in the Nether, the Overworld is untouched and
    /// the player stays where the world opened.
    #[test]
    #[ignore = "daily"]
    fn a_nether_transfer_recovers_while_the_world_opens_in_the_overworld() {
        let root = transfer_root("nether");
        // Opening a second level on demand reads the data-fix tables of
        // the prepared resources (SUNRISE_RESOURCES is their root for
        // that lookup; the test harness points its sibling variable at
        // the same folder).
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();

            // The transfer happens in the Nether, opened on demand.
            game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            assert_eq!(game.dim, crate::game::Dim::Nether);
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/the_nether"),
                sunrise_sim::save::LevelShape::NETHER,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            game.world.set(4, 64, 4, chest).unwrap();
            game.world.carrier_fences.fail_flush = true;
            let failed = game.carrier_assemble(11, &[BlockPos::new(4, 64, 4)]);
            assert!(failed.is_err(), "the injected flush must fail: {failed:?}");
            let why = failed.unwrap_err().to_string();
            assert!(
                why.contains("injected"),
                "the failure must be the injected flush, not an earlier refusal: {why}"
            );

            // The world reopens in the Overworld: recovery enters the
            // Nether itself, replays there and returns.
            game.world.carrier_fences.fail_flush = false;
            game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap();
            assert_eq!(game.dim, crate::game::Dim::Overworld);
            // A fresh open holds no parked level: drop the Nether so
            // recovery has to open it from disk, as at world load.
            game.server
                .levels
                .0
                .parked
                .remove(&sunrise_gameplay_common::game::Dim::Nether);
            assert!(
                !game
                    .server
                    .levels
                    .0
                    .parked
                    .contains_key(&crate::game::Dim::Nether)
            );
            game.carrier_recover_all_levels().unwrap();
            assert_eq!(
                game.dim,
                crate::game::Dim::Overworld,
                "the player stays in the level the world opened in"
            );

            // One owner: the items are in the carrier file, the Nether
            // cell is air, and the Overworld never wrote a thing.
            assert!(
                root.join(format!("data/sunrise_carrier_{:032x}-1.nbt", 11_u128))
                    .is_file()
            );
            assert!(!root.join("data/sunrise_carrier_op_1.nbt").is_file());
            let nether = root.join("dimensions/minecraft/the_nether");
            let chunk = sunrise_world::carrier::region_strict::read_chunk(
                &nether.join("region"),
                sunrise_core::position::ChunkPos::new(0, 0),
            )
            .unwrap()
            .expect("the nether chunk is on disk");
            let data = sunrise_world::chunk::ChunkData::new(chunk).unwrap();
            assert_eq!(
                data.block(sunrise_core::position::BlockPos::new(4, 64, 4))
                    .unwrap()
                    .unwrap()
                    .name,
                "minecraft:air"
            );
            let overworld = root.join("dimensions/minecraft/overworld/region");
            assert!(
                !overworld.join("r.0.0.mca").exists(),
                "the overworld's region is untouched"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// A journal naming a dimension the world cannot hold refuses the
    /// load with that name, and no level is touched.
    #[test]
    #[ignore = "daily"]
    fn an_unknown_dimension_refuses_the_load_without_touching_levels() {
        let root = transfer_root("nope");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            // A prepared operation whose journal names minecraft:nope.
            let mut store = sunrise_sim::save::carrier::Store::first_use(&root, true).unwrap();
            store
                .begin(
                    sunrise_sim::save::carrier::OpKind::Assemble,
                    12,
                    1,
                    "minecraft:nope",
                )
                .unwrap();
            drop(store);

            let back = game.dim;
            let refused = game.carrier_world_opened();
            match refused {
                Err(error) => {
                    let text = error.to_string();
                    assert!(
                        text.contains("minecraft:nope")
                            && text.contains("which this world does not hold"),
                        "{text}"
                    );
                }
                Ok(()) => panic!("an unknown dimension must refuse the load"),
            }
            assert!(game.admit_level_save().is_err());
            // No level was touched: the game still holds the level it
            // opened in, with nothing parked.
            assert_eq!(game.dim, back);
            assert!(!game.world.carrier_fences.locks_carrier(12));
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// The region writer's per-chunk ordering: an older snapshot,
    /// submitted first and completing after the post-state's write,
    /// leaves the post-state on disk. The writer is one thread over an
    /// mpsc channel (batches run in submission order) and
    /// `IOWorker.pendingWrites` replaces a chunk's pending record with
    /// the newer submission's sequence, so its `newest` filter
    /// (save.rs: "a chunk queued again since is written once, by its
    /// newest batch") drops the older snapshot. No record number of
    /// our own is needed; this test pins the guarantee.
    #[test]
    #[ignore = "daily"]
    fn an_older_snapshot_completing_late_cannot_overwrite_the_post_state() {
        let root = transfer_root("writer");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            let save = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let dirt = registry.default_state("minecraft:dirt").unwrap();
            let mut pre_chunk = sunrise_level::chunk::Chunk::empty(0, 0);
            pre_chunk.set(4, 64, 4, stone.0);
            let pre = Arc::new(pre_chunk);
            let mut post_chunk = sunrise_level::chunk::Chunk::empty(0, 0);
            post_chunk.set(4, 64, 4, dirt.0);
            let post = Arc::new(post_chunk);

            // Both snapshots submitted back to back, one barrier: the
            // pre-image's write lands behind the post-state's.
            save.save_later(&[pre], registry, world_biomes!(game))
                .unwrap();
            save.save_later(&[post], registry, world_biomes!(game))
                .unwrap();
            save.flush().unwrap();

            let chunk = sunrise_world::carrier::region_strict::read_chunk(
                &root.join("dimensions/minecraft/overworld/region"),
                sunrise_core::position::ChunkPos::new(0, 0),
            )
            .unwrap()
            .expect("the chunk is on disk");
            let data = sunrise_world::chunk::ChunkData::new(chunk).unwrap();
            assert_eq!(
                data.block(sunrise_core::position::BlockPos::new(4, 64, 4))
                    .unwrap()
                    .unwrap()
                    .name,
                "minecraft:dirt",
                "the post-state must survive the older snapshot"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// Every save route refuses while the #2488 obligation holds —
    /// autosave, save-all, requested parked saves and the shutdown
    /// save all enter through `admit_entity_save` — and admits again
    /// the moment the transfer commits. The transfer runs inside one
    /// synchronous call, so the steps cannot interleave with a save;
    /// this pins the gate each route passes.
    #[test]
    #[ignore = "daily"]
    fn every_save_route_refuses_under_the_obligation_and_admits_after_commit() {
        let root = transfer_root("saves");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();

            // Under the obligation — the transfer's own state between
            // acquire and release — every route refuses.
            {
                let _lease = sunrise_sim::join_seq::visibility::PassLease::enter();
                assert!(game.save_game_with(true).is_err(), "autosave");
                assert!(game.save_level(true).is_err(), "save-all");
                game.server.levels.0.request_save(game.dim);
                game.run_requested_saves();
                // The requested save was refused while lifted and stays
                // queued for the next tick, as #2488 defines.
                assert!(game.server.levels.0.save_pending(game.dim));
            }
            // Committed: the routes admit again (the full save writes
            // the harness's throwaway world, which is allowed here).
            game.carrier_assemble(13, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            assert!(game.save_game_with(true).is_ok(), "after the commit");
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// The supported backup procedure: `save-all` flush, no pending
    /// operation, copy the world folder, load the copy — the carriers
    /// and chunks come out identical.
    #[test]
    #[ignore = "daily"]
    fn the_supported_backup_restores_identical_carriers_and_chunks() {
        let root = transfer_root("backup");
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.server.save.0 = std::sync::Arc::new(sunrise_sim::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.carrier_assemble(14, &[BlockPos::new(4, 64, 4)])
                .unwrap();

            // save-all with its flush, and no pending operation.
            assert!(game.save_game_with(true).is_ok());
            let store = match sunrise_sim::save::carrier::Store::open(&root).unwrap() {
                sunrise_sim::save::carrier::Opened::Live(store) => store,
                _ => panic!("the store must open"),
            };
            assert!(
                store.journal().in_progress.is_none(),
                "no operation pending: the world is quiescent"
            );
            drop(store);

            // Copy the whole world folder.
            let copy = root.parent().unwrap().join(format!(
                "sunrise-carrier-backup-copy-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&copy);
            copy_dir(&root, &copy).unwrap();

            // Load the copy: the carrier file and every region byte
            // are identical to the original's.
            let carrier = format!("data/sunrise_carrier_{:032x}-1.nbt", 14_u128);
            assert_eq!(
                std::fs::read(root.join(&carrier)).unwrap(),
                std::fs::read(copy.join(&carrier)).unwrap(),
                "the carrier file copies identically"
            );
            let original =
                std::fs::read(root.join("dimensions/minecraft/overworld/region/r.0.0.mca"))
                    .unwrap();
            let restored =
                std::fs::read(copy.join("dimensions/minecraft/overworld/region/r.0.0.mca"))
                    .unwrap();
            assert_eq!(original, restored, "the chunk bytes copy identically");
            // And the copy's store opens with the carrier listed.
            match sunrise_sim::save::carrier::Store::open(&copy).unwrap() {
                sunrise_sim::save::carrier::Opened::Live(store) => {
                    assert!(store.journal().lists(14, 1));
                }
                _ => panic!("the copy's store must open"),
            }
            let _ = std::fs::remove_dir_all(&copy);
        });
        let _ = std::fs::remove_dir_all(&root);
    }
    /// A world without transfers creates nothing and fences nothing.
    #[test]
    #[ignore = "daily"]
    fn a_world_without_transfers_writes_no_store_files() {
        let root =
            std::env::temp_dir().join(format!("sunrise-carrier-quiet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        with_game(|game| {
            game.server.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(4, 64, 4, stone).unwrap();
            assert!(!game.world.carrier_fences.locks_cell(&[4, 64, 4]));
            let names: Vec<String> = std::fs::read_dir(root.join("data"))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_str().unwrap().to_string())
                .collect();
            assert!(
                names.is_empty(),
                "a world without transfers wrote {names:?}"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
}
