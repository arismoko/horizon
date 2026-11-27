//! Command block rules: the block's state reads, conditional chaining, power and edit records.
use sunrise_core::position::BlockPos;
use sunrise_level::{
    block_entities::BlockEntityLease,
    command_block::{self, CommandBlock, Mode},
    containers::BlockEntity,
    world::World,
};
use sunrise_world::{
    chunk::BlockState,
    registry::{BlockRegistry, BlockStateId},
};

type Pos = (i32, i32, i32);

/// What `ServerboundSetCommandBlockPacket` carries.
pub struct CommandBlockEdit {
    pub pos: Pos,
    pub command: String,
    pub track_output: bool,
    pub mode: Mode,
    pub conditional: bool,
    pub auto: bool,
}

pub fn state_of(registry: &BlockRegistry, state: BlockStateId) -> Option<&BlockState> {
    registry.state(state)
}
pub fn facing_step(state: &BlockState) -> [i32; 3] {
    match state.properties.get("facing").map(String::as_str) {
        Some("down") => [0, -1, 0],
        Some("up") => [0, 1, 0],
        Some("south") => [0, 0, 1],
        Some("west") => [-1, 0, 0],
        Some("east") => [1, 0, 0],
        _ => [0, 0, -1],
    }
}
/// `Direction.toYRot`: south 0, west 90, north 180, east (and up/down) 270.
pub fn facing_yaw(state: &BlockState) -> f32 {
    match state.properties.get("facing").map(String::as_str) {
        Some("south") => 0.0,
        Some("west") => 90.0,
        Some("north") => 180.0,
        _ => 270.0,
    }
}
pub fn conditional(state: &BlockState) -> bool {
    state
        .properties
        .get("conditional")
        .is_some_and(|c| c == "true")
}
pub fn offset(pos: Pos, step: [i32; 3]) -> Pos {
    (pos.0 + step[0], pos.1 + step[1], pos.2 + step[2])
}
pub fn command_block_mut(world: &mut World, pos: Pos) -> Option<&mut CommandBlock> {
    match world.block_entities.get_mut(&pos)? {
        BlockEntity::CommandBlock(c) => Some(c),
        _ => None,
    }
}

/// `CommandBlockEntity.markConditionMet`: a conditional block needs the
/// command block behind it to have succeeded.
pub fn mark_condition_met(world: &mut World, registry: &BlockRegistry, pos: Pos) -> bool {
    let Some(state) = state_of(registry, world.block(pos.0, pos.1, pos.2)) else {
        return false;
    };
    let mut met = true;
    if conditional(state) {
        let back = facing_step(state).map(|v| -v);
        let behind = offset(pos, back);
        let behind_block = state_of(registry, world.block(behind.0, behind.1, behind.2))
            .is_some_and(|s| Mode::of_block(&s.name).is_some());
        met = behind_block
            && matches!(world.block_entities.get(&behind),
                Some(BlockEntity::CommandBlock(c)) if c.success_count > 0);
    }
    if let Some(c) = command_block_mut(world, pos) {
        c.condition_met = met;
    }
    met
}

/// `CommandBlockEntity.setPowered` from `CommandBlock.setPoweredAndUpdate`
/// (the level's neighbour hook): the previous flag and `isAutomatic`; a
/// rising signal that will tick the block checks its condition.
pub fn set_powered(
    world: &mut World,
    registry: &BlockRegistry,
    pos: Pos,
    powered: bool,
) -> Option<(bool, bool)> {
    let mode = state_of(registry, world.block(pos.0, pos.1, pos.2))
        .and_then(|s| Mode::of_block(&s.name))?;
    let c = command_block_mut(world, pos)?;
    let (was, auto) = (c.powered, c.auto);
    c.powered = powered;
    if command_block::power_schedules(mode, was, powered, auto) {
        mark_condition_met(world, registry, pos);
    }
    Some((was, auto))
}

/// `getAnalogOutputSignal`: the success count.
pub fn analog_signal(world: &World, pos: Pos) -> Option<i32> {
    match world.block_entities.get(&pos)? {
        BlockEntity::CommandBlock(c) => Some(c.success_count),
        _ => None,
    }
}

/// One state-preserving command edit, scoped to its exact removal boundary.
pub struct CommandBlockTransition {
    pub lease: BlockEntityLease,
    pub old: BlockStateId,
    pub new: BlockStateId,
    pub available: bool,
}

impl CommandBlockTransition {
    pub fn preserve(&mut self, world: &mut World, pos: BlockPos, old: BlockStateId) -> bool {
        if !self.available || self.old != old || self.new != world.block(pos.x, pos.y, pos.z) {
            return false;
        }
        if !world
            .block_entities
            .rekey(&(pos.x, pos.y, pos.z), &mut self.lease)
        {
            return false;
        }
        self.available = false;
        true
    }
}
