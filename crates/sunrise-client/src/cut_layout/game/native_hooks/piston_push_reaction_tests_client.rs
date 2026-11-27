use crate::game::native_hooks::piston_push_reaction_tests::state;
use std::sync::Arc;
use sunrise_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use sunrise_world::level::LevelAccess;

pub(crate) fn run_client_piston_case(root: std::path::PathBuf) {
    sunrise_client_gameplay::game::native_entity_refs::tests::with_game_role_client(
        root,
        ExecutionRole::Client,
        |game| {
            use sunrise_core::nbt::Tag;
            use sunrise_items::items::ItemStack;
            use sunrise_level::{chunk::Chunk, containers::BlockEntity};
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "CarryReplay").unwrap();
            let from = (3, 64, 2);
            let to = (4, 64, 2);
            let registry = game.pack.assets.registry;
            let name = "minecraft:chest";
            let block = registry.default_state(name).unwrap();
            let piston = state(registry, "minecraft:piston", &[("facing", "east")]);
            let power = registry.default_state("minecraft:redstone_block").unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            game.with_level(|_, level, _| {
                level.set_raw(BlockPos::new(2, 64, 2), piston);
                level.set_raw(BlockPos::new(1, 64, 2), power);
                level.set_raw(BlockPos::new(3, 64, 2), block);
            });
            let mut slots = vec![None; 27];
            slots[4] = Some(ItemStack::new("minecraft:diamond", 7));
            game.world
                .block_entities
                .insert(from, BlockEntity::Chest(slots.clone()));
            let custom = Tag::String("Moving chest".into());
            game.world.block_names.insert(from, custom.clone());
            let lock = Tag::compound([("items", Tag::String("minecraft:diamond".into()))]);
            game.world.set_block_lock(
                from,
                &Tag::compound([
                    ("id", Tag::String(name.into())),
                    ("x", Tag::Int(3)),
                    ("y", Tag::Int(64)),
                    ("z", Tag::Int(2)),
                    ("lock", lock.clone()),
                    (
                        "components",
                        Tag::compound([(
                            "minecraft:custom_data",
                            Tag::compound([("carry_test", Tag::Int(11))]),
                        )]),
                    ),
                ]),
            );
            game.world.set_container_stack_limit(from, 13);
            game.world.chest_openers.insert(from, (2, 4.0));
            game.client.be_anims.chests.insert(
                [3, 64, 2],
                sunrise_client::block_entity_anim::ChestLid {
                    should_be_open: true,
                    openness: 0.7,
                    o_openness: 0.6,
                },
            );
            game.container = Some(from);
            game.with_level(|b, level, ticks| {
                b.client_block_event(
                    level,
                    ticks,
                    BlockPos::new(2, 64, 2),
                    0,
                    Direction::East as i32,
                );
            });
            assert!(
                game.container.is_none(),
                "source menu closes rather than following another holder"
            );
            assert!(game.world.block_entities.get(&from).is_none());
            assert!(!game.world.block_names.contains_key(&from));
            assert!(game.world.block_lock(from).is_none());
            assert_eq!(game.world.container_stack_limit(from), None);
            assert!(!game.world.chest_openers.contains_key(&from));
            assert!(!game.client.be_anims.chests.contains_key(&[3, 64, 2]));
            let at = BlockPos::new(4, 64, 2);
            let moving = game
                .block_ticks
                .moving_pistons
                .iter()
                .find(|(p, _)| *p == at)
                .unwrap()
                .1;
            let saved = game.saved_moving_piston(at, &moving).unwrap();
            game.world.block_entities.discard_carried(&to);
            let mut extras = sunrise_sim::save::ChunkExtras::default();
            extras.block_entities.push(saved);
            game.restore_moving_piston(at, moving, Some(&extras));
            for _ in 0..10 {
                game.world.game_time += 1;
                game.with_level(|b, level, ticks| b.client_tick_block_entities(level, ticks));
            }
            assert_eq!(game.world.block_entities.iter_mut().count(), 1);
            assert!(game.world.block_entities.get(&from).is_none());
            assert_eq!(game.world.block_names.get(&to), Some(&custom));
            assert_eq!(game.world.block_lock(to), Some(&lock));
            assert_eq!(game.world.container_stack_limit(to), Some(13));
            assert_eq!(
                game.world
                    .block_entity_extras(to)
                    .source
                    .unwrap()
                    .get("components")
                    .and_then(|t| t.get("minecraft:custom_data"))
                    .and_then(|t| t.get("carry_test")),
                Some(&Tag::Int(11))
            );
            assert!(
                matches!(game.world.block_entities.get(&to), Some(BlockEntity::Chest(after)) if after == &slots)
            );
            game.client.be_anims.classify(registry);
            game.block_entity_event([4, 64, 2], 1, 1);
            game.tick_chest_lid([4, 64, 2]);
            assert_eq!(game.client.be_anims.chests[&[4, 64, 2]].openness, 0.1);
            assert!(!game.client.be_anims.chests.contains_key(&[3, 64, 2]));
        },
    );
}
