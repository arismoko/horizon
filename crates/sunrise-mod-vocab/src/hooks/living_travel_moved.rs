//! The `living_travel_moved` hook: declaration the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Movement phase 4 (after the branch's move, before drag and gravity): a
    /// `LivingEntity.travel` branch has just made its own `Entity.move(SELF)`,
    /// and its drag, gravity, the fluid-height jump-out rule and fall-flying's
    /// wall damage have not run yet. Vanilla's `travelInAir`, `travelInFluid`,
    /// `travelFlying` and `travelFallFlying` all read the velocity back after
    /// the move, so a mod that changes how a body moves through a medium edits
    /// it here, where the collision has happened and the drag has not.
    ///
    /// Fires once per travel branch that moves, for every `TravelMode`
    /// (`entity`, `kind` and `mode` identify the body and the branch, as for
    /// `living_travel`). `velocity` is the post-collision velocity; `moved` is
    /// the displacement the move made, `[0.0; 3]` when an `entity_move`
    /// listener vetoed it, and the hook still fires then. `on_ground_before` is
    /// the ground flag the body entered the move with, `on_ground` the flag
    /// after it, and `jumping` the body's jump input this tick; a rule that
    /// needs to remember the last tick's ground state reads `on_ground_before`
    /// instead of writing mod state during movement. A travel that
    /// `living_travel` cancelled makes no move and fires nothing.
    ///
    /// Replace `velocity` with a finite vector and the branch's drag and
    /// gravity then act on it as they would on the game's own. A non-finite
    /// vector drops that listener's edit; later listeners see the last accepted
    /// velocity. If a listener panics or incurs a phase refusal, its edit is
    /// discarded and later listeners still run. A panic disables the failed
    /// callback for this hook for the session. The hook takes the loop context
    /// of the side that runs the travel, not a `Host`: read equipment, fluids
    /// and other state from what the listener already knows and `context`, as
    /// `living_travel` does.
    ///
    /// Fires on every side that simulates the body: the server for each living
    /// body it simulates, and a client for its own player and the living
    /// vehicle it drives, replayed (simulated) ticks included. A listener is a
    /// pure function of its inputs and what both sides know that tick; the
    /// server keeps vanilla's movement checks and its position wins. A
    /// `side = "server"` mod's listener is not called for a body a client
    /// controls (a player, or a vehicle a player is driving) and runs normally
    /// for every other body. With no listener no event is built.
    /// Beyond NeoForge: there is no counterpart.
    #[hook(
        side = "both",
        contract = "collect",
        out = "velocity",
        context = "loop",
        cancellable = false,
        version = 1,
        output_validator = "crate::entity::types::refused_velocity_output",
        neoforge = "none (fork addition, #5074)",
        target = "sunrise_sim::travel_gate::native_living_travel_moved"
    )]
    fn living_travel_moved(
        entity: u32,
        kind: &str,
        mode: TravelMode,
        moved: [f64; 3],
        on_ground_before: bool,
        on_ground: bool,
        jumping: bool,
        velocity: &mut [f64; 3],
    );
}
