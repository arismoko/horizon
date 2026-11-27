//! The `entity_inside` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity's block sweep crossed a block state this part is attached to (`Block.entityInside`), and the part is told mid-sweep, where vanilla runs the block's own `entityInside`. `event.precise` is `Entity.checkInsideBlocks`' flag: the entity moved nearly a block, or its box at the target touches the block. It runs inside the entity's move on the level random host (`&mut dyn LevelRandomHost`): it reads blocks and draws the level's random in vanilla's order, interleaved with the vanilla blocks the same walk visits. `event.apply` queues one of the five `InsideBlockEffectType`s, and `event.run_before`/`event.run_after` queue owned callbacks around an effect slot. When the sweep flushes (`InsideBlockEffectApplier.StepBasedCollector.applyAndClear`), each callback is lent an `&mut InsideEntityView<'_>` over the swept entity and its live level, stopping as soon as the entity dies. Level writes (`set_block`, `destroy_block`) apply directly to the live level with no touched-block state-unchanged skip or sibling-write rollback, and `hurt_with_source` resolves optional source handles directly without exposing a `Taken` refusal. A panic in the part call drops its staged work; a panic in a queued callback leaves completed live writes in place unless an existing transaction journals them. Either panic disables that mod's `entity_inside` callback for the session, logs the mod's ID, and queues a client error toast. Other mods' callbacks and built-in effects continue while the entity is alive. Fires for players, mobs, items and projectiles alike, each on the shared walk inside its own tick or batch. An item runs one sweep, after `step_on`; the same ordered slots hold built-in effects and Part callbacks.
    #[hook(
        side = "server",
        version = 4,
        contract = "collect",
        out = "effects",
        scope = "block",
        key = "state",
        host = "level",
        target = "sunrise_gameplay_common::game::native_hooks::entity_inside::HitLevel<'_>::native_entity_inside",
        neoforge = "none (vanilla Block.entityInside; NeoForge adds no event or extension for it)"
    )]
    fn entity_inside(
        dimension: &str,
        entity: i32,
        kind: &str,
        state: BlockState,
        pos: BlockPos,
        precise: bool,
        effects: &mut InsideEffects,
    );
}
