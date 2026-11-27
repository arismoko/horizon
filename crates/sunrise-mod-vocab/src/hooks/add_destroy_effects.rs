//! `ClientLevel.addDestroyBlockEffect`: the destroyed block's client particles.
pub trait Hooks {
    /// Asked on the client before a non-air state's destruction debris. `state` is the
    /// destroyed state, not what the client now sees at `pos`; an invisible state can still
    /// be asked. Return `Flow::Cancel` to replace or omit the burst, or `Flow::Continue` for
    /// the normal burst (even if the state normally suppresses terrain particles). The break
    /// sound is independent. `random` is the client level's random.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        scope = "block",
        key = "state",
        target = "sunrise_client_gameplay::game::native_hooks::block_particle_effects::ClientGame<'_>::native_add_destroy_effects",
        neoforge = "IClientBlockExtensions.addDestroyEffects"
    )]
    fn add_destroy_effects(
        level: LevelRef,
        state: BlockState,
        pos: BlockPos,
        random: &mut dyn RandomSource,
        cancel: &mut bool,
    );
}
