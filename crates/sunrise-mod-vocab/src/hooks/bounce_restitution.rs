//! The `bounce_restitution` hook and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A simulating entity's downward block collision asks for the contacted state's restitution
    /// (`IBlockExtension.getBounceRestitution`, through `IEntityExtension.getBlockBounciness`).
    /// `event.state` and `event.pos` are the pair selected by `Entity.move` at `getOnPosLegacy`.
    /// That position uses the supporting block's column but need not be the supporting cell.
    /// `event.entity` is the entity's id and `event.kind` is its type.
    ///
    /// Set `event.factor` to change the block's coefficient, seeded with the block's own value.
    /// For a nonliving entity the block's factor is multiplied by `0.8` in `f32` before comparing
    /// it with the entity's bounciness. Non-finite edits are ignored; a zero factor does not veto
    /// the entity's own bounciness. The query is skipped for a fall no faster than effective
    /// gravity, a body suppressing bounce, or a state in `#minecraft:suppresses_bounce`.
    /// This is distinct from `bounce_on`, which runs after the bounce is decided.
    ///
    /// The read-only `LevelSide` is the side simulating the movement. Register on both sides and
    /// answer as a pure function of the event and level: prediction and replay may ask again.
    /// No random draws, persistent writes or counters should change an answer. Only Parts on the
    /// selected state are asked; a client interpolating packets does not query a block.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        version = 2,
        contract = "collect",
        out = "factor",
        scope = "block",
        key = "state",
        host = "level",
        target = "sunrise_gameplay_common::game::native_hooks::bounce_restitution::FrictionView<'_>::native_bounce_restitution",
        neoforge = "IBlockExtension.getBounceRestitution"
    )]
    fn bounce_restitution(
        entity: i32,
        kind: &str,
        state: BlockState,
        pos: BlockPos,
        factor: &mut f32,
    );
}
