use super::tests::{Fixture, TestLevel, fixture};
use super::*;
use std::cell::RefCell;

/// A level that records, at each moment, the state change and the east side of the fence at
/// the origin's west.
struct StateChangeLevel {
    inner: TestLevel,
    fence: BlockPos,
    /// `observe_block_changed`: the change and the fence's east side then.
    changed: RefCell<Vec<(BlockStateId, BlockStateId, String)>>,
    /// `observe_block_state_change`: the change and the fence's east side then.
    state_changes: RefCell<Vec<(BlockStateId, BlockStateId, String)>>,
    registry: &'static BlockRegistry,
}
impl StateChangeLevel {
    fn east(&self) -> String {
        self.registry
            .state(self.block_state(self.fence))
            .and_then(|s| s.properties.get("east").cloned())
            .unwrap_or_default()
    }
}
impl crate::level::BlockEntityNeighbors for StateChangeLevel {}
impl LevelAccess for StateChangeLevel {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.inner.block_state(pos)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        self.inner.set_raw(pos, state)
    }
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.inner.is_loaded(pos)
    }
    fn random(&mut self) -> &mut LegacyRandom {
        self.inner.random()
    }
    fn game_time(&self) -> i64 {
        self.inner.game_time()
    }
    fn observe_block_changed(&self, _pos: BlockPos, old: BlockStateId, new: BlockStateId) {
        let east = self.east();
        self.changed.borrow_mut().push((old, new, east));
    }
    fn observe_block_state_change(&self, _pos: BlockPos, old: BlockStateId, new: BlockStateId) {
        let east = self.east();
        self.state_changes.borrow_mut().push((old, new, east));
    }
}

fn state(f: &Fixture, name: &str) -> BlockStateId {
    f.registry
        .default_state(&format!("minecraft:{name}"))
        .unwrap_or_else(|| panic!("unknown block {name}"))
}

#[test]
fn a_state_change_is_heard_after_the_neighbours_settled_and_only_when_the_write_held() {
    let Some(f) = fixture() else {
        return;
    };
    let b = &f.behaviours;
    let registry: &'static BlockRegistry = &f.registry;
    let mut level = StateChangeLevel {
        inner: TestLevel::new(b),
        fence: BlockPos::new(-1, 1, 0),
        changed: RefCell::new(Vec::new()),
        state_changes: RefCell::new(Vec::new()),
        registry,
    };
    let mut ticks = Ticks::new(0);
    let [fence, stone, sponge, wet_sponge, water] =
        ["oak_fence", "stone", "sponge", "wet_sponge", "water"].map(|name| state(f, name));
    let air = b.air();
    level.inner.set_raw(BlockPos::new(-1, 1, 0), fence);
    // Stone east of the fence, flags 3: `block_changed` hears it before the fence connects,
    // the state change after.
    let origin = BlockPos::new(0, 1, 0);
    assert!(b.set_block(&mut level, &mut ticks, origin, stone, flags::UPDATE_ALL));
    assert_eq!(
        level.changed.borrow().first(),
        Some(&(air, stone, "false".to_owned()))
    );
    // The fence's own shape update is a write inside the stone's, heard first.
    let connected = level.block_state(BlockPos::new(-1, 1, 0));
    assert_eq!(
        *level.state_changes.borrow(),
        [
            (fence, connected, "true".to_owned()),
            (air, stone, "true".to_owned())
        ]
    );
    // Writing the same state again does not hold, and is not heard.
    assert!(!b.set_block(&mut level, &mut ticks, origin, stone, flags::UPDATE_ALL));
    assert_eq!(level.state_changes.borrow().len(), 2);
    // A sponge beside water turns wet in its `on_place`: the sponge's own write is not heard,
    // the write that replaced it is.
    level.state_changes.borrow_mut().clear();
    level.inner.set_raw(BlockPos::new(6, 1, 0), water);
    let at = BlockPos::new(5, 1, 0);
    b.set_block(&mut level, &mut ticks, at, sponge, flags::UPDATE_ALL);
    assert_eq!(level.block_state(at), wet_sponge);
    let heard: Vec<_> = level
        .state_changes
        .borrow()
        .iter()
        .map(|(old, new, _)| (*old, *new))
        .filter(|(old, new)| [*old, *new].contains(&sponge))
        .collect();
    assert_eq!(heard, [(sponge, wet_sponge)]);
}
