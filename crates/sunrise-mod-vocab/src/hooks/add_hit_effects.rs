//! `ClientLevel.addBreakingBlockEffects`: the block's client hit particles.
pub trait Hooks {
    /// Asked on the client for the non-air, visible block being hit, after its optional hit
    /// sound and before any outline sampling. Return `Flow::Cancel` to replace or omit the
    /// crumb, or `Flow::Continue` for the normal crumb (even if the state normally suppresses
    /// terrain particles). The supplied face is the one being hit. `random` is the level's
    /// random, also used by a call to `LevelRef::add_breaking_particles` during this callback.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        scope = "block",
        key = "state",
        target = "sunrise_client_gameplay::game::native_hooks::block_particle_effects::ClientGame<'_>::native_add_hit_effects",
        neoforge = "IClientBlockExtensions.addHitEffects"
    )]
    fn add_hit_effects(
        level: LevelRef,
        state: BlockState,
        pos: BlockPos,
        face: Direction,
        random: &mut dyn RandomSource,
        cancel: &mut bool,
    );
}
