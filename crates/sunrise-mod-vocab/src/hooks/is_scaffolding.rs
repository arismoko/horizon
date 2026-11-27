//! The `is_scaffolding` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether the block state this part is attached to stops a sneaking player's slide down a climbable as scaffolding does (`IBlockExtension.isScaffolding`, which `LivingEntity.handleOnClimbable` asks of the block the player stands in). `entity` is the player's id and `pos` the block the player's position is in; `state` is that block's state. `scaffolding` starts with vanilla's answer, whether the state is `minecraft:scaffolding`, and a part may set either value: `true` lets a sneaking player keep sliding down a climbable in the block, as in scaffolding, and `false` stops the slide there though it is scaffolding. It is asked whenever the player is on a climbable and moving down, sneaking or not; the answer only matters while sneaking, and vanilla asks it of a `Player` only, so mobs never ask it. It fires on both sides and on the client's replayed ticks, so register the mod on both and keep the listener a pure function of its inputs. It is given the loop `context` of the side that ticks the player, and takes no `Host`. With no part attached to the state, the owning game checks the hook subscription and prepared API block key before any event conversion, so the cost is one table lookup. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        scope = "block",
        key = "state",
        cancellable = false,
        target = "sunrise_sim::motion::surface::native_is_scaffolding",
        neoforge = "IBlockExtension.isScaffolding"
    )]
    fn is_scaffolding(entity: u32, pos: BlockPos, state: BlockState, scaffolding: &mut bool);
}
