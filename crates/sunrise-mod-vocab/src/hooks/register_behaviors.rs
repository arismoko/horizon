//! The `register_behaviors` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The mob's mod brain behaviors, joined into the species' brain beside
    /// vanilla's (#824): built wherever the brain is built — its first tick and
    /// every rebuild — started and stopped in `Brain.tick`'s order, fresh state
    /// each build, nothing saved. Branch on the mob with `event.behaviors.mob()`.
    ///
    /// Staged where the brain is constructed or rebuilt (or at construction for
    /// species with no brain, where adds refuse with [`BehaviorRefusal::NoBrain`])
    /// and dispatched with a scoped server host at the owning server boundary; the
    /// listener's transactional writes and the staged behaviors apply to the
    /// original brain together, refusing without recomputing if the mob or its
    /// brain changed while listeners ran. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::register_mob_parts::ServerGame<'_>::native_register_behaviors",
        receiver = "write",
        dispatch = "direct",
        side = "server",
        contract = "listen",
        scope = "entity",
        key = "behaviors",
        neoforge = "none (vanilla LivingEntity.makeBrain)"
    )]
    fn register_behaviors(behaviors: &mut BehaviorRegistrar<'_>);
}
