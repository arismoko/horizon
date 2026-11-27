//! The `step_on` hook: declaration read by the mod API generator (see `hooks/mod.rs`).

pub trait Hooks {
    /// `Block.stepOn` in `Entity.applyEffectsFromBlocks`, before inside-block effects.
    /// It runs on the block at `getOnPosLegacy` for a grounded body affected by blocks,
    /// including an item whose idle tick did not move. `state` and `pos` name that same
    /// contact; the position need not be the actual supporting cell. Each applicable
    /// effects pass gets its own callback.
    ///
    /// `stepping_carefully` is the body's `Entity.isSteppingCarefully` at this contact.
    /// `movement` is its current velocity. Set it to change the velocity before the
    /// inside effects and later drag. There is no veto.
    /// `bounds` is the body's current world-space box, `[min, max]` in XYZ order.
    /// Only Parts on the contacted state run, after that block's vanilla step behavior.
    /// The server lends the level random host as `fall_on` does, and accepts
    /// `event.write_entity(EntityWrite::…)` for the stepping entity before the inside scan.
    /// Hurt writes apply to players, mobs, dropped items and experience orbs.
    /// Use `LevelHost::entity_type_is_living(event.kind)` when the block only affects living
    /// bodies, as magma does.
    /// Boats and minecarts take `Hurt` from this contact; their `entity_inside`
    /// contact does not deal it yet (#4177). Armor stands and projectiles ignore contact hurt.
    /// `event.request(LandingWrite::…)` queues a change at `pos` through the `fall_on`
    /// drain: requests apply at the batch end if `pos` still holds `state`.
    /// A Part's invalid block state or non-finite movement or damage discards only its
    /// own outputs; earlier edits remain, and the next Part sees the last accepted movement.
    /// The client lends a read-only level and accepts only movement edits.
    /// Keep movement answers consistent between the two sides and on replay.
    #[hook(
        side = "both",
        version = 3,
        contract = "collect",
        out = "movement",
        scope = "block",
        key = "state",
        host = "level_random_side",
        dispatch = "direct",
        target = "sunrise_gameplay_common::game::native_hooks::bounce_restitution::StepOnView<'_>::native_step_on",
        neoforge = "none (vanilla Block.stepOn)"
    )]
    fn step_on(
        entity: i32,
        kind: &str,
        state: BlockState,
        pos: BlockPos,
        stepping_carefully: bool,
        bounds: [[f64; 3]; 2],
        movement: &mut [f64; 3],
        writes: &mut StepOnWrites,
    );
}
