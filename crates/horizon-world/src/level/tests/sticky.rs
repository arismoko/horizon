use super::*;

/// A level whose block parts answer the piston structure resolver's questions.
struct StickyLevel {
    inner: TestLevel,
    /// The states a part makes slime blocks (`adjust_is_slime_block`).
    slime: HashSet<BlockStateId>,
    /// The states a part makes sticky (`adjust_is_sticky_block`).
    sticky: HashSet<BlockStateId>,
    /// The (state, other) pairs a part of `state` refuses to stick to (`adjust_can_stick_to`).
    refused: HashSet<(BlockStateId, BlockStateId)>,
}
impl crate::level::BlockEntityNeighbors for StickyLevel {}
impl LevelAccess for StickyLevel {
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
    fn adjust_is_slime_block(&self, state: BlockStateId, vanilla: bool) -> bool {
        vanilla || self.slime.contains(&state)
    }
    fn adjust_is_sticky_block(&self, state: BlockStateId, vanilla: bool) -> bool {
        vanilla || self.sticky.contains(&state)
    }
    fn adjust_can_stick_to(&self, state: BlockStateId, other: BlockStateId, vanilla: bool) -> bool {
        vanilla && !self.refused.contains(&(state, other))
    }
}

fn state(f: &Fixture, name: &str) -> BlockStateId {
    f.registry
        .default_state(&format!("minecraft:{name}"))
        .unwrap_or_else(|| panic!("unknown block {name}"))
}

/// A piston at the origin facing east is powered from the west, with `pushed` east of it and
/// `above` on top of that, in a level whose parts `parts` sets up: the states at `(1, 2, 0)` and
/// `(2, 2, 0)` once it extended, where `above` was and where it goes if it moved along.
fn push_pair(
    f: &Fixture,
    pushed: BlockStateId,
    above: BlockStateId,
    parts: impl FnOnce(&mut StickyLevel),
) -> (BlockStateId, BlockStateId) {
    let mut level = StickyLevel {
        inner: TestLevel::new(&f.behaviours),
        slime: HashSet::new(),
        sticky: HashSet::new(),
        refused: HashSet::new(),
    };
    parts(&mut level);
    let mut ticks = Ticks::new(0);
    for x in -4..4 {
        for z in -4..4 {
            ticks.add_empty_chunk(ChunkPos::new(x, z)).unwrap();
        }
    }
    let stone = state(f, "stone");
    for x in -3..=3 {
        for z in -3..=3 {
            level.inner.blocks.insert(BlockPos::new(x, 0, z), stone);
        }
    }
    let piston = f
        .registry
        .resolve(&BlockState {
            name: "minecraft:piston".into(),
            properties: [("facing".to_owned(), "east".to_owned())].into(),
        })
        .unwrap();
    let redstone = state(f, "redstone_block");
    for (pos, block) in [
        (BlockPos::new(0, 1, 0), piston),
        (BlockPos::new(1, 1, 0), pushed),
        (BlockPos::new(1, 2, 0), above),
        (BlockPos::new(-1, 1, 0), redstone),
    ] {
        f.behaviours
            .set_block(&mut level, &mut ticks, pos, block, flags::UPDATE_ALL);
    }
    for _ in 0..4 {
        level.inner.time += 1;
        f.behaviours.tick_scheduled(&mut level, &mut ticks);
        f.behaviours.run_block_events(&mut level, &mut ticks);
        f.behaviours.tick_block_entities(&mut level, &mut ticks);
    }
    let extended = f.registry.state(level.block_state(BlockPos::new(0, 1, 0)));
    assert_eq!(
        extended
            .and_then(|s| s.properties.get("extended"))
            .map(String::as_str),
        Some("true"),
        "the piston extended"
    );
    assert_eq!(level.block_state(BlockPos::new(2, 1, 0)), pushed);
    (
        level.block_state(BlockPos::new(1, 2, 0)),
        level.block_state(BlockPos::new(2, 2, 0)),
    )
}

#[test]
fn vanilla_slime_pulls_the_block_on_top_and_stone_does_not() {
    let Some(f) = fixture() else {
        return;
    };
    let (slime, stone, air) = (
        state(f, "slime_block"),
        state(f, "stone"),
        f.behaviours.air(),
    );
    assert_eq!(push_pair(f, slime, stone, |_| {}), (air, stone));
    let gold = state(f, "gold_block");
    assert_eq!(push_pair(f, gold, stone, |_| {}), (stone, air));
}

#[test]
fn a_sticky_part_block_pulls_its_neighbours_along_with_a_piston() {
    let Some(f) = fixture() else {
        return;
    };
    // Gold is an ordinary block until a part makes it sticky: then the stone on top moves too.
    let (gold, stone, air) = (
        state(f, "gold_block"),
        state(f, "stone"),
        f.behaviours.air(),
    );
    let sticky = |level: &mut StickyLevel| {
        level.sticky.insert(gold);
    };
    assert_eq!(push_pair(f, gold, stone, sticky), (air, stone));
}

#[test]
fn a_sticky_part_block_lets_go_of_what_can_stick_to_says_no_to() {
    let Some(f) = fixture() else {
        return;
    };
    // Honey on a sticky gold block moves with it, as two sticky blocks hold on to each other,
    // unless either side refuses: the resolver asks both.
    let (gold, honey, air) = (
        state(f, "gold_block"),
        state(f, "honey_block"),
        f.behaviours.air(),
    );
    for (refused, expected) in [
        (None, (air, honey)),
        (Some((gold, honey)), (honey, air)),
        (Some((honey, gold)), (honey, air)),
    ] {
        let parts = |level: &mut StickyLevel| {
            level.sticky.insert(gold);
            level.refused.extend(refused);
        };
        assert_eq!(
            push_pair(f, gold, honey, parts),
            expected,
            "refused {refused:?}"
        );
    }
}

#[test]
fn a_slime_part_block_is_held_apart_from_honey_as_slime_is() {
    let Some(f) = fixture() else {
        return;
    };
    // Honey on a sticky gold block moves with it, unless a part also says gold is slime.
    let (gold, honey, air) = (
        state(f, "gold_block"),
        state(f, "honey_block"),
        f.behaviours.air(),
    );
    for (slime, expected) in [(false, (air, honey)), (true, (honey, air))] {
        let parts = |level: &mut StickyLevel| {
            level.sticky.insert(gold);
            if slime {
                level.slime.insert(gold);
            }
        };
        assert_eq!(
            push_pair(f, gold, honey, parts),
            expected,
            "gold is slime: {slime}"
        );
    }
}
