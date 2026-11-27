//! The `register_goals` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The mob's mod goals, added to its two selectors as `GoalSelector.addGoal`
    /// does in vanilla's `registerGoals`: fresh at every construction, so a goal's
    /// state is never saved. Branch on the mob with `event.goals.mob()`.
    ///
    /// Staged at `Mob` construction and dispatched with a scoped server host at
    /// the owning server boundary; the listener's transactional writes and the
    /// staged goals apply together, and apply refuses without recomputing if the
    /// mob or its selectors changed while listeners ran. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::register_mob_parts::ServerGame<'_>::native_register_goals",
        receiver = "write",
        dispatch = "direct",
        side = "server",
        contract = "listen",
        scope = "entity",
        key = "goals",
        neoforge = "none (vanilla Mob.registerGoals)"
    )]
    fn register_goals(goals: &mut GoalRegistrar<'_>);
}
