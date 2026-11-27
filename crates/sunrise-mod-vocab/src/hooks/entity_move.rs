//! The `entity_move` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Movement phase 3 (Move, `Entity.move`): one physical `Entity.move`
    /// invocation (#2414, design #2308, the maintainer ruling of the same issue).
    /// The entity `entity` of `kind` (`minecraft:zombie`) is about to move by
    /// `delta` (blocks this tick) for `cause`'s sake - itself, the player behind
    /// a position packet, a piston, a shulker box lid, a shulker mob's opening
    /// lid, or a moving collider carrying or shoving the body. It fires before
    /// the move's own handling, no-physics and the stuck multiplier included, in
    /// the one gate every kind's physics resolves through, so a forgotten
    /// family cannot move around it. Change `delta` to replace the movement
    /// (still through every ordinary restriction) or set `event.cancelled` or
    /// return `Flow::Cancel` to skip it: no position, velocity, collision-flag or
    /// fall change, and no displacement notice. Replacement components must be
    /// finite; there is no additional per-axis limit. A non-finite replacement
    /// rejects that listener's delta and veto together and logs the mod and
    /// field. An unchanged game delta is not revalidated. An earlier accepted
    /// veto cannot be cleared. If a listener panics or incurs a phase refusal,
    /// its edits and cancel flag are discarded; earlier results stand and later
    /// eligible listeners still run. Only a panic disables this mod's callback
    /// for the session. It takes no Host - the maintainer ruling, like the
    /// enderman's teleport: the listener only decides the delta, and its
    /// `context` is the loop `context` of the side that ticks the body.
    ///
    /// Fires on every side that simulates the body (vanilla
    /// `Entity.canSimulateMovement`), and also for piston, shulker and other
    /// non-`SelfMovement` pushes (`MoverType.isServerAndClientSimulated`) on
    /// bodies the client only observes. A listener is a pure function of its
    /// inputs and what both sides know that tick; `context.side()` tells the
    /// sides apart. The server keeps vanilla's movement checks and its position
    /// wins. A `side = "server"` mod's listener is not called for a body a
    /// client controls (a player, or a vehicle a player is driving) and runs
    /// normally for every other body. Beyond NeoForge: there is no counterpart.
    ///
    /// Since version 2: `MoveCause::Carried`, a moving collider carrying a body that stands on it
    /// once a tick (`EntityRef::set_collider`), is a new cause under the same signature, so a
    /// listener written for version 1 hears moves it never heard. `MoveCause` is
    /// `#[non_exhaustive]`: match it with a wildcard arm.
    ///
    /// Version 3 isolates each listener's proposals and veto; an accepted veto is sticky.
    ///
    /// Version 4 allows every finite replacement delta without a per-axis limit.
    ///
    /// Version 5: `MoveCause::Carried` also covers a translating or turning collider shoving a
    /// body in its path, not just carrying one standing on it.
    ///
    /// Version 6: fires on both sides (`side = "both"`) wherever a side simulates
    /// the body or applies a non-`SelfMovement` push; a `side = "server"` mod's
    /// listener is skipped for a body a client controls (a player or a vehicle a
    /// player is driving).
    #[hook(
        version = 6,
        side = "both",
        contract = "cancel",
        out = "canceled",
        context = "loop",
        cancellable = true,
        output_validator = "crate::entity::types::refused_move_output",
        neoforge = "none (fork addition, #2308)",
        target = "sunrise_sim::move_gate::native_entity_move"
    )]
    fn entity_move(
        entity: u32,
        kind: &str,
        cause: MoveCause,
        delta: &mut [f64; 3],
        canceled: &mut bool,
    );
}
