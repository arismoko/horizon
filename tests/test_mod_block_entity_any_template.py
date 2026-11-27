"""Run the real mod block entity checks (removal, piston, render list) with fakes."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from tests.test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]
LEVEL = ROOT / "crates/sunrise-world/src/level"
COMMON = ROOT / "crates/sunrise-gameplay-common/src"
CLIENT_GAMEPLAY = ROOT / "crates/sunrise-client-gameplay/src"


def between(source, start, end):
    i = source.index(start) + len(start)
    return source[i:source.index(end, i)]


FIXTURE = r"""#![allow(unused)]
mod state_flags { pub const BLOCK_ENTITY: u32 = 1; }
mod flags {
    pub const UPDATE_NEIGHBORS: i32 = 1;
    pub const UPDATE_MOVE_BY_PISTON: i32 = 64;
    pub const UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS: i32 = 256;
    pub const UPDATE_SKIP_ON_PLACE: i32 = 512;
}
#[derive(Clone, Copy)] enum Class { Comparator, BaseRail }
type BlockId = u32;
#[derive(Clone, Copy, PartialEq, Debug)] pub struct BlockStateId(u32);
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] struct BlockPos { x: i32, y: i32, z: i32 }
enum BlockEntity { Mod(u8), Chest(u8) }
struct Facts { block_entity: bool }
struct Res;
impl Res { fn facts(&self) -> &Self { self } fn get(&self, s: BlockStateId) -> Facts { Facts { block_entity: s.0 == 1 } } }
struct Table;
impl Table {
    fn block(&self, state: BlockStateId) -> BlockId {
        if state == BlockStateId(10) { 0 } else { state.0 }
    }
    fn int(&self, _: BlockStateId, _: ()) -> i32 { unreachable!("no redstone wire") }
}
struct Named { redstone_wire: BlockId, moving_piston: BlockId }
struct Props { power: () }
struct BlockData;
impl BlockData { fn is(&self, _: Class) -> bool { false } }
struct Blocks { templated: Vec<BlockStateId>, table: Table, named: Named, props: Props }
impl Blocks {
    fn new(templated: Vec<BlockStateId>) -> Self {
        Self { templated, table: Table, named: Named { redstone_wire: 20, moving_piston: 21 }, props: Props { power: () } }
    }
    fn has_flag(&self, s: BlockStateId, _f: u32) -> bool { self.templated.contains(&s) }
    fn is_block(&self, s: BlockStateId, block: BlockId) -> bool { self.table.block(s) == block }
    fn instance_of(&self, _: BlockStateId, _: Class) -> bool { false }
    fn block_data(&self, _: BlockId) -> BlockData { BlockData }
    fn keeps_block_entity(&self, old: BlockStateId, new: BlockStateId) -> bool {
        self.table.block(old) == self.table.block(new)
    }
}
#[derive(Default)] struct Entities(std::collections::HashMap<(i32, i32, i32), BlockEntity>);
impl std::ops::Deref for Entities { type Target = std::collections::HashMap<(i32, i32, i32), BlockEntity>; fn deref(&self) -> &Self::Target { &self.0 } }
// BlockEntities::mods_near has its own test in sunrise-client-render; here every mod entity is near.
impl Entities {
    fn stateful_entity(&self, state: u32) -> bool { state == 2 }
    fn has_mods(&self) -> bool { self.0.values().any(|e| matches!(e, BlockEntity::Mod(_))) }
    fn mods_near(&self, _x: f64, _z: f64, _r: f64) -> impl Iterator<Item = &(i32, i32, i32)> {
        self.0.iter().filter(|(_, e)| matches!(e, BlockEntity::Mod(_))).map(|(p, _)| p)
    }
}
struct DVec3 { x: f64, z: f64 }
const VIEW_DISTANCE: f64 = 64.0;
struct World { block_entities: Entities }
impl World { fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId { BlockStateId((x != 0) as u32) } }
struct GameLevel {
    world: World, carries: Option<bool>, state: BlockStateId, removed: Option<BlockStateId>,
}
// The extracted chunk writer's LevelAccess parameter is this fixture's level.
type L = GameLevel;
impl GameLevel {
    HAS_MOD
    HAS_NATIVE
    fn adjust_piston_carries_block_entity(&self, _pos: BlockPos, _state: BlockStateId) -> Option<bool> { self.carries }
    fn is_loaded(&self, _: BlockPos) -> bool { true }
    fn is_client_side(&self) -> bool { false }
    fn ensure_block_entity(&mut self, _: BlockPos, _: BlockStateId) {}
    fn set_raw(&mut self, _: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        Some(std::mem::replace(&mut self.state, state))
    }
    fn observe_block_changed(&self, _: BlockPos, _: BlockStateId, _: BlockStateId, _: i32) {}
    fn remove_block_entity(&mut self, pos: BlockPos, old: BlockStateId, side_effects: bool) -> u8 {
        assert!(side_effects);
        assert_eq!(self.removed.replace(old), None);
        self.world.block_entities.0.remove(&(pos.x, pos.y, pos.z));
        0
    }
}
struct ViewDistances { furthest: f64 }
struct CommonGame { world: World, res: Res }
struct ClientGame { common: CommonGame }
impl ClientGame {
    // The parts' view distances are their own test (`view_distances_are_asked_once_per_mod_block`).
    fn view_distances(&self) -> ViewDistances { ViewDistances { furthest: VIEW_DISTANCE } }
    UNTEMPLATED
}
BLOCK_WRITE
#[derive(Default)] struct WireGraph;
impl WireGraph {
    fn invalidate(&mut self) {}
    fn set_power(&mut self, _: BlockPos, _: u8) { unreachable!("no redstone wire") }
}
#[derive(Default)] struct Ticks { wire_graph: WireGraph, comparator_outputs: std::collections::HashMap<BlockPos, u8> }
struct MovingPiston { moved_state: BlockStateId }
struct Sim<'a> { b: Blocks, level: &'a mut L, ticks: Ticks }
impl<'a> Sim<'a> {
    fn new(b: Blocks, level: &'a mut L) -> Self { Self { b, level, ticks: Ticks::default() } }
    fn is_in_valid_bounds(&self, _: BlockPos) -> bool { true }
    fn state(&self, _: BlockPos) -> BlockStateId { self.level.state }
    fn block_of(&self, state: BlockStateId) -> BlockId { self.b.table.block(state) }
    fn remove_moving_piston(&mut self, _: BlockPos) -> Option<MovingPiston> { unreachable!("no moving piston") }
    fn land_carried(&mut self, _: BlockPos, _: BlockStateId, _: bool, _: bool) { unreachable!("no moving piston") }
    fn update_neighbors_at(&mut self, _: BlockPos, _: BlockId) { unreachable!("no jukebox") }
    fn affect_neighbors_after_removal(&mut self, _: BlockStateId, _: BlockPos, _: bool) {}
    fn on_place(&mut self, _: BlockStateId, _: BlockPos, _: BlockStateId, _: bool) {}
    CHUNK_WRITE_METHODS
    fn removes(&mut self, old: BlockStateId, state: BlockStateId, pos: BlockPos) -> bool {
        self.level.state = old;
        self.level.removed = None;
        let write = self.begin_block_write(pos, state, 0).unwrap();
        let journal_removal = write.effectful_removal();
        assert!(self.finish_chunk_write(write, None, GameLevel::ensure_block_entity));
        assert_eq!(self.level.removed, journal_removal);
        self.level.removed.is_some()
    }
    fn pushable_tail(&self, state: BlockStateId, pos: BlockPos) -> bool {
        let b = &self.b;
        PUSHABLE_TAIL
    }
}
fn at(x: i32) -> BlockPos { BlockPos { x, y: 64, z: 0 } }
fn level(entities: &[(i32, BlockEntity)]) -> GameLevel {
    GameLevel { world: World { block_entities: Entities(entities.iter()
        .map(|(x, e)| ((*x, 64, 0), match e { BlockEntity::Mod(_) => BlockEntity::Mod(0), BlockEntity::Chest(_) => BlockEntity::Chest(0) }))
        .collect()) }, carries: None, state: BlockStateId(0), removed: None }
}
#[test]
fn a_mod_block_entity_on_a_template_without_one_is_removed_and_not_pushed() {
    let stone = BlockStateId(0);
    let mut world = level(&[(5, BlockEntity::Mod(0))]);
    let mut sim = Sim::new(Blocks::new(vec![BlockStateId(1)]), &mut world);
    // the same block replaced by itself (state change) keeps it
    assert!(!sim.removes(stone, BlockStateId(10), at(5)));
    // stone template, a mod entity here: removed with its block, never pushed
    assert!(!sim.pushable_tail(stone, at(5)));
    assert!(sim.removes(stone, BlockStateId(9), at(5)));
    assert!(!sim.level.world.block_entities.contains_key(&(5, 64, 0)));
    // stone with no entity: nothing to remove, pushable as before
    assert!(!sim.removes(stone, BlockStateId(9), at(6)));
    assert!(sim.pushable_tail(stone, at(6)));
    // a template that has one still removes it, as before
    assert!(sim.removes(BlockStateId(1), BlockStateId(9), at(6)));
    assert!(!sim.pushable_tail(BlockStateId(1), at(6)));
    // A state with a native factory is also protected before its entity has loaded.
    assert!(sim.removes(BlockStateId(2), BlockStateId(9), at(6)));
    assert!(!sim.pushable_tail(BlockStateId(2), at(6)));
    // a vanilla entity on a template without the flag is not a mod's
    let mut world = level(&[(5, BlockEntity::Chest(0)), (6, BlockEntity::Mod(0))]);
    let mut sim = Sim::new(Blocks::new(vec![]), &mut world);
    assert!(!sim.removes(stone, BlockStateId(9), at(5)));
}
#[test]
fn a_carry_answer_moves_mod_and_vanilla_block_entities_alike() {
    let stone = BlockStateId(0);
    let mut world = level(&[(5, BlockEntity::Mod(0)), (6, BlockEntity::Chest(0))]);
    let mut sim = Sim::new(Blocks::new(vec![BlockStateId(1)]), &mut world);
    sim.level.carries = Some(true);
    // a mod entity moves, whatever its template
    assert!(sim.pushable_tail(stone, at(5)));
    assert!(sim.pushable_tail(BlockStateId(1), at(5)));
    // #3453: one block-entity rule, so a vanilla one moves too, and so does a template
    // with one before it loaded; the carry machinery moves their position-owned state
    assert!(sim.pushable_tail(BlockStateId(1), at(6)));
    assert!(sim.pushable_tail(BlockStateId(1), at(7)));
    // a native factory's state whose entity has not loaded is the mod's own
    assert!(sim.pushable_tail(BlockStateId(2), at(7)));
    // a refusal holds even a block without one
    sim.level.carries = Some(false);
    assert!(!sim.pushable_tail(stone, at(7)));
    assert!(!sim.pushable_tail(stone, at(5)));
}
#[test]
fn the_render_list_holds_mod_entities_of_templates_without_one() {
    let game = ClientGame {
        common: CommonGame {
            world: World { block_entities: Entities([((0, 64, 0), BlockEntity::Mod(0)), ((1, 64, 0), BlockEntity::Mod(0)),
                                            ((2, 64, 0), BlockEntity::Chest(0))].into_iter().collect()) },
            res: Res,
        },
    };
    // x == 0 gives state 0 (template without an entity); x != 0 state 1 (has one: the mesher lists it)
    let mut listed = game.untemplated_mod_block_entities(DVec3 { x: 0.0, z: 0.0 });
    listed.sort();
    assert_eq!(listed, vec![([0, 64, 0], 0)]);
}
"""


class ModBlockEntityAnyTemplateTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_removal_piston_and_render_list(self):
        game = (COMMON / "game/blocks/level_access.rs").read_text(encoding="utf-8")
        draw = (CLIENT_GAMEPLAY / "game/modding/native_block_entity_draw.rs").read_text(encoding="utf-8")
        update = (LEVEL / "update.rs").read_text(encoding="utf-8")
        pistons = (LEVEL / "pistons.rs").read_text(encoding="utf-8")
        journal_start = update.index("pub struct BlockWrite {")
        block_write = update[journal_start:update.index("\nimpl<L:", journal_start)]
        chunk_write = "\n".join(method(update, name) for name in ("begin_block_write", "finish_chunk_write"))
        tail = between(pistons, "        } else if b.table.bool(state, b.props.extended) {\n            return false;\n        }\n", "    }\n    /// `IBlockExtension.isSlimeBlock`")
        fixture = (FIXTURE.replace("HAS_MOD", method(game, "has_mod_block_entity"))
                   .replace("HAS_NATIVE", method(game, "has_native_block_entity"))
                   .replace("UNTEMPLATED", method(draw, "untemplated_mod_block_entities"))
                   .replace("BLOCK_WRITE", block_write)
                   .replace("CHUNK_WRITE_METHODS", chunk_write.replace("super::classes::", ""))
                   .replace("PUSHABLE_TAIL", tail)
                   .replace("super::data::state_flags::", "state_flags::"))
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "mbe.rs", Path(folder) / "mbe.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
