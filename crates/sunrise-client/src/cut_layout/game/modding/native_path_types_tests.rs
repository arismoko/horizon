//! Positional block Parts run against the owning readonly level, not during state resolution.
use super::native_entity_refs::tests::with_game;
use mod_api::{
    AdjacentBlockPathTypeEvent, BlockPart, BlockPathTypeEvent, Hooks, Host, IsPathfindableEvent,
    LevelHost, PathComputationType, PathType, Registrar,
};
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_sim::mobs::{
    Mob,
    path::{Evaluator, Finder},
};

struct Thorny;
#[mod_api::sunrise_part]
impl BlockPart for Thorny {
    fn block_path_type(
        &self,
        level: &dyn LevelHost,
        event: &mut BlockPathTypeEvent,
    ) -> Option<PathType> {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let answer = match pos {
            [4, 64, _] if level.block_state_id("minecraft:smooth_stone") == Some(event.state) => {
                Some(PathType::Damaging)
            }
            [15, 65, 1] if level.block_state_id("minecraft:air") == Some(event.state) => {
                Some(PathType::Blocked)
            }
            _ => None,
        };
        if answer.is_some() {
            // These override positions are in the loaded fixture chunk. Finder
            // also asks void-air positions outside it, where the level read is None.
            assert_eq!(level.block_state(pos), Some(event.state));
        }
        answer
    }
}
struct ThornyMod;
#[mod_api::__subscriptions]
impl Hooks for ThornyMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for name in [
            "minecraft:smooth_stone",
            "minecraft:air",
            "minecraft:oak_fence",
            "minecraft:oak_door",
        ] {
            let key = r.block_key(name);
            r.block_part(key, Thorny);
        }
    }
}

type AdjacentCalls = Arc<Mutex<Vec<([i32; 3], PathType)>>>;

struct Beside(AdjacentCalls);
#[mod_api::sunrise_part]
impl BlockPart for Beside {
    fn is_pathfindable(&self, host: &dyn Host, event: &mut IsPathfindableEvent) {
        if event.kind == PathComputationType::Land
            && host.block_state_id("minecraft:glass") == Some(event.state)
        {
            event.pathfindable = true;
        }
    }
    fn block_path_type(
        &self,
        level: &dyn LevelHost,
        event: &mut BlockPathTypeEvent,
    ) -> Option<PathType> {
        (level.block_state_id("minecraft:seagrass") == Some(event.state))
            .then_some(PathType::Blocked)
    }
    fn adjacent_block_path_type(
        &self,
        host: &dyn Host,
        event: &mut AdjacentBlockPathTypeEvent,
    ) -> Option<PathType> {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let level = host.level(host.dimension()).unwrap();
        assert_eq!(level.block_state(host, pos), Some(event.state));
        self.0.lock().unwrap().push((pos, event.path_type));
        if pos == [0, 64, 0] {
            Some(PathType::Walkable)
        } else if host.block_state_id("minecraft:glass") == Some(event.state) {
            Some(PathType::FireInNeighbor)
        } else {
            None
        }
    }
}
struct BesideMod(AdjacentCalls);
#[mod_api::__subscriptions]
impl Hooks for BesideMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for name in [
            "minecraft:stone",
            "minecraft:magma_block",
            "minecraft:glass",
            "minecraft:seagrass",
        ] {
            let key = r.block_key(name);
            r.block_part(key, Beside(Arc::clone(&self.0)));
        }
    }
}

struct ChestReader;
#[mod_api::sunrise_part]
impl BlockPart for ChestReader {
    fn block_path_type(
        &self,
        level: &dyn LevelHost,
        event: &mut BlockPathTypeEvent,
    ) -> Option<PathType> {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        if pos != [3, 64, 1] {
            return None;
        }
        let chest = level
            .block_entity(pos)
            .expect("the chest at the queried position");
        let data = level
            .block_entity_data(chest)
            .expect("the chest's saved data");
        let items = match data.get(&"Items".into()) {
            Some(mod_api::Tag::List(items)) => items.len(),
            _ => 0,
        };
        // The same read-only level answers the neighbouring positions: a block
        // without a block entity, an unloaded chunk and outside the build height.
        assert!(level.block_entity([4, 64, 1]).is_none());
        assert!(level.block_entity([40, 64, 40]).is_none());
        let outside_y = level.min_y() + level.height();
        assert!(level.block_entity([3, outside_y, 1]).is_none());
        assert!(level.block_entity([3, 400, 1]).is_none());
        // Java's unchecked subtraction: a listener's `i32::MIN` darkening wraps
        // instead of panicking under overflow checks, so block light still wins.
        assert_eq!(
            level.raw_brightness(pos, i32::MIN),
            level.brightness(mod_api::LightLayer::Block, pos)
        );
        match items {
            1 => Some(PathType::Walkable),
            2 => Some(PathType::Damaging),
            _ => None,
        }
    }
}
struct ChestReaderMod;
#[mod_api::__subscriptions]
impl Hooks for ChestReaderMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let key = r.block_key("minecraft:chest");
        r.block_part(key, ChestReader);
    }
}

/// A stone floor with a chest at `[3, 64, 1]` in the level in place, holding `items` diamonds.
fn chest_level(game: &mut sunrise_gameplay::game::ServerGame<'_>, items: usize) {
    let registry = game.pack.assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    let chest = registry.default_state("minecraft:chest").unwrap();
    for x in 0..6 {
        for z in 0..6 {
            game.world.set(x, 64, z, stone).unwrap();
        }
    }
    game.world.set(3, 64, 1, chest).unwrap();
    let mut contents = vec![None; 27];
    for slot in contents.iter_mut().take(items) {
        *slot = Some(sunrise_items::items::ItemStack::new("minecraft:diamond", 3));
    }
    game.world.block_entities.insert(
        (3, 64, 1),
        sunrise_level::containers::BlockEntity::Chest(contents),
    );
}

#[test]
#[ignore = "daily"]
fn a_block_part_reads_a_block_entity_through_its_owning_level() {
    with_parts("chest-reader", ChestReaderMod, |game| {
        chest_level(game, 1);
        // The part answers from the chest's saved data, so the override reaches
        // past the chest's own vanilla Blocked type.
        assert_eq!(
            game.mob_world_base(&[]).path_type_from_state(3, 64, 1),
            PathType::Walkable
        );
        // A second loaded dimension has a chest of its own at the same position:
        // the level host answers from that level's block entity, not a fixed one.
        let scratch = tempfile::tempdir().unwrap();
        game.server.world_root = scratch.path().to_path_buf();
        game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
            .unwrap();
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        chest_level(game, 2);
        assert_eq!(
            game.mob_world_base(&[]).path_type_from_state(3, 64, 1),
            PathType::Damaging
        );
    });
}

#[test]
#[ignore = "daily"]
fn cache_follows_update_flags_and_direct_block_entity_notifications() {
    with_game(|game| {
        use std::cell::Cell;
        use sunrise_core::position::BlockPos;
        let pos = [5, 64, 5];
        let packed_pos = BlockPos::new(pos[0], pos[1], pos[2]);
        let air = game.res.facts().air;
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        game.world.block_log = None;
        game.world.enable_path_type_cache();
        let calls = Cell::new(0);
        let read = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            game.world.cached_path_type(packed_pos, || {
                calls.set(calls.get() + 1);
                if game.world.block(pos[0], pos[1], pos[2]) == air {
                    PathType::Open
                } else {
                    PathType::Blocked
                }
            })
        };
        assert_eq!(read(game), PathType::Open);
        game.level_set_with_flags(pos, stone, 0).unwrap();
        assert_eq!(read(game), PathType::Open);
        assert_eq!(
            calls.get(),
            1,
            "a non-notifying write must leave the cached type stale"
        );
        game.level_set_with_flags(pos, stone, 2).unwrap();
        read(game);
        assert_eq!(
            calls.get(),
            1,
            "an unchanged state must not reach sendBlockUpdated"
        );
        game.level_set_with_flags(pos, air, 2).unwrap();
        read(game);
        assert_eq!(
            calls.get(),
            2,
            "UPDATE_CLIENTS clears despite the absent network log"
        );
        game.mark_block_entity_updated((pos[0], pos[1], pos[2]));
        read(game);
        assert_eq!(
            calls.get(),
            3,
            "direct block-entity notification clears unchanged state"
        );
        game.notify_block_clients((pos[0], pos[1], pos[2]));
        read(game);
        assert_eq!(calls.get(), 4);
        game.world.block_log = Some(vec![(pos[0], pos[1], pos[2])]);
        game.notify_block_clients((pos[0], pos[1], pos[2]));
        read(game);
        assert_eq!(
            calls.get(),
            5,
            "log deduplication must not suppress invalidation"
        );
    });
}

fn with_parts(
    id: &'static str,
    hooks: impl Hooks + 'static,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(id, Box::new(hooks))].into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            game.resolve_block_paths();
            test(game);
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_block_part_gives_its_block_a_path_type() {
    with_parts("thorny", ThornyMod, |game| {
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let smooth = registry.default_state("minecraft:smooth_stone").unwrap();
        for x in 0..9 {
            for z in 0..3 {
                game.world
                    .set(x, 64, z, if x == 4 { smooth } else { stone })
                    .unwrap();
            }
        }
        game.world.set(2, 64, 1, smooth).unwrap();
        game.world
            .set(
                14,
                65,
                1,
                registry.default_state("minecraft:oak_fence").unwrap(),
            )
            .unwrap();
        game.world
            .set(
                13,
                65,
                1,
                registry.default_state("minecraft:oak_door").unwrap(),
            )
            .unwrap();
        let world = game.mob_world_base(&[]);
        // The same state answers differently by position; None keeps vanilla's special cases.
        assert_eq!(world.path_type_from_state(4, 64, 1), PathType::Damaging);
        assert_eq!(world.path_type_from_state(2, 64, 1), PathType::Blocked);
        assert_eq!(world.path_type_from_state(14, 65, 1), PathType::Fence);
        assert_eq!(
            world.path_type_from_state(13, 65, 1),
            PathType::DoorWoodClosed
        );
        // The override runs even before the air early return.
        assert_eq!(world.path_type_from_state(15, 65, 1), PathType::Blocked);
        assert_eq!(world.path_type_static(4, 65, 1), PathType::Damaging);
        assert_eq!(world.path_type_static(1, 65, 1), PathType::Walkable);
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut zombie = Mob::new(50, kind, Vec3::new(0.5, 65.0, 1.5), 0.0).path_mob();
        zombie.on_ground = true;
        let path = Finder::new(&world, &zombie, Evaluator::Walk).find(&[[8, 65, 1]], 16.0, 0, 256);
        assert!(path.is_none_or(
            |path| !path.reached && path.nodes.iter().map(|node| node.x).max() < Some(4)
        ));
    });
}

#[test]
#[ignore = "daily"]
fn a_block_part_gives_the_nodes_beside_its_block_a_path_type() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    with_parts("beside", BesideMod(Arc::clone(&calls)), |game| {
        let registry = game.pack.assets.registry;
        let state = |name: &str| registry.default_state(name).unwrap();
        for x in 0..6 {
            for z in 0..6 {
                game.world.set(x, 64, z, state("minecraft:stone")).unwrap();
            }
        }
        game.world
            .set(1, 64, 0, state("minecraft:magma_block"))
            .unwrap();
        game.world.set(1, 65, 4, state("minecraft:glass")).unwrap();
        game.world.set(8, 64, 8, state("minecraft:stone")).unwrap();
        game.world
            .set(7, 64, 7, state("minecraft:seagrass"))
            .unwrap();
        let world = game.mob_world_base(&[]);
        // First neighbour is (0,64,0), whose own type is Blocked, not the node's Walkable.
        // Some(Walkable) ends the scan even though it equals the node's incoming type.
        assert_eq!(
            world.check_neighbours(1, 65, 1, PathType::Walkable),
            PathType::Walkable
        );
        assert_eq!(*calls.lock().unwrap(), [([0, 64, 0], PathType::Blocked)]);
        // None from magma does not suppress its vanilla fire contribution.
        assert_eq!(world.path_type_static(0, 65, 0), PathType::FireInNeighbor);
        assert_eq!(world.path_type_static(0, 65, 4), PathType::FireInNeighbor);
        assert_eq!(world.path_type_static(4, 65, 4), PathType::Walkable);
        // Seagrass is overridden to Blocked. A declined adjacent answer keeps
        // that classified type; contained vanilla water adds no separate border.
        assert_eq!(world.path_type_from_state(7, 64, 7), PathType::Blocked);
        assert_eq!(world.path_type_static(8, 65, 8), PathType::Walkable);
        let observed = calls.lock().unwrap();
        // Land made glass Open, the positional path query declined, and adjacency
        // saw that final baseline. Seagrass instead reached adjacency as the
        // positional override's Blocked, not its exported Water or the node type.
        assert!(observed.contains(&([1, 65, 4], PathType::Open)));
        assert!(observed.contains(&([7, 64, 7], PathType::Blocked)));
    });
}
