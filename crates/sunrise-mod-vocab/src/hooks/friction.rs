//! The `friction` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The friction of `state` at `pos` (`IBlockExtension.getFriction`), for
    /// an optional entity handle. The state need not occupy that position.
    /// The read-only level host reads the side that made the query: blocks, states, light,
    /// fluids, weather, enabled features, tags and resident block entities, as the other
    /// level hosts do, without loading chunks. It has no container, shape or signal view and
    /// answers `None` for those. Block-entity handles are the side's own: a client's is not
    /// the server's, so answer from state both sides hold.
    /// Set `event.friction` to answer; it starts at the state's own coefficient. Any finite answer is used as
    /// given, as vanilla uses what `getFriction` returns; a non-finite one keeps that coefficient.
    /// Runs on both sides, including during client replay. Keep the answer a pure
    /// function of the event and the level reads.
    #[notes = r#"## Block friction

`BlockPart::friction` answers NeoForge's `IBlockExtension.getFriction` for a state at a position,
with an optional `EntityRef`; `level.entity_kind(entity)` reads its type key. It runs for ground travel, ground drag, boat ground friction and lead
angular friction, and for explicit `LevelRef::friction` queries. The read-only `LevelSide` host
can read the level or ask for another state's answer at the same position, for example a block
taking its material's friction. The queried state need not be what the level holds at that position.
The borrowed view reads block states, their properties, fluids and light, without loading chunks.
`block_state_id` parses a block name with properties as elsewhere.
It also reads weather, enabled features, tags and the side's resident block entities, but has no
container, shape or signal view (those answer `None`). A client's block entity is its own copy, not the
server's: answer from state both sides hold. Each listener gets the original `pos`, `state` and `entity`;
only `friction` carries from one listener to the next.

An active query on the same level, position, state and entity id (or no entity) cannot be repeated.
A recursive or over-deep query returns `None` (`Ok(None)` through `LevelRef`). Without a part,
or when the part leaves the answer alone or writes a non-finite value, the state's own
coefficient is used; any finite value is used as given. Physics still applies vanilla's friction modifier and drag at the sites that
used them before. Register the part on both sides and keep its answer pure so prediction and
replay agree."#]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        cancellable = false,
        scope = "block",
        key = "state",
        host = "level",
        version = 3,
        event_per_listener = true,
        target = "sunrise_gameplay_common::game::native_hooks::friction::FrictionView<'_>::native_friction",
        neoforge = "IBlockExtension.getFriction"
    )]
    fn friction(pos: BlockPos, state: BlockState, entity: Option<EntityRef>, friction: &mut f32);
}
