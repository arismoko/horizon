use mod_api::{EntityRef, LevelRef};

/// What a resolved entity shows to a reader.
///
/// A view borrows the subsystem that made it, shared, so it cannot be held across a write, a level
/// swap or a tick: those take `&mut` of the subsystem, which the borrow checker refuses while a
/// view lives. Its getters do not revalidate: the handle was resolved once, in
/// [`EntityRead::resolve`].
///
/// ```compile_fail
/// use engine_access::{EntityRead, EntityView, LevelWrite};
/// use horizon_world::registry::BlockStateId;
/// use mod_api::{BlockUpdateFlags, EntityRef, LevelRef};
/// fn held_across_a_write<G: EntityRead + LevelWrite>(g: &mut G, e: EntityRef, at: LevelRef) {
///     let view = g.resolve(e).unwrap();
///     let _ = g.set_block(at, [0, 0, 0], BlockStateId(0), BlockUpdateFlags::NONE);
///     let _ = view.position();
/// }
/// ```
///
/// Reading what is needed first, then writing, compiles:
///
/// ```
/// use engine_access::{EntityRead, EntityView, LevelWrite};
/// use horizon_world::registry::BlockStateId;
/// use mod_api::{BlockUpdateFlags, EntityRef, LevelRef};
/// fn read_then_write<G: EntityRead + LevelWrite>(g: &mut G, e: EntityRef, at: LevelRef) {
///     let position = g.resolve(e).and_then(|view| view.position());
///     let _ = (position, g.set_block(at, [0, 0, 0], BlockStateId(0), BlockUpdateFlags::NONE));
/// }
/// ```
pub trait EntityView {
    /// The entity's position (`Entity.position`); None where the subsystem has none to give.
    fn position(&self) -> Option<[f64; 3]>;
    /// Yaw and pitch in degrees. None for an entity the subsystem only tracks (a client's copy of
    /// a server entity), which carries no rotation.
    fn rotation(&self) -> Option<[f32; 2]>;
    /// The entity's type key, such as `minecraft:zombie`.
    fn kind(&self) -> Option<String>;
    /// The level the entity is in.
    fn level(&self) -> Option<LevelRef>;
}

/// Reads of entities by handle.
pub trait EntityRead {
    /// A view of what `entity` names, or None for a stale or foreign handle. One resolution; the
    /// view's getters answer from it.
    type View<'a>: EntityView
    where
        Self: 'a;

    fn resolve(&self, entity: EntityRef) -> Option<Self::View<'_>>;
}

/// Where an entity's UUID stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UuidState {
    /// The entity has one.
    Known(u128),
    /// An entity whose UUID the subsystem cannot read without writing: asking for it assigns one ([`EntityIdentity::assign_uuid`]).
    Unassigned,
}

/// Entity identity, which a read may complete by writing: a server entity's UUID is assigned the
/// first time it is asked for.
pub trait EntityIdentity {
    /// The UUID state of the entity, or None for a stale handle or an entity with none to give
    /// (a client's tracked entity the server never named).
    fn uuid_state(&self, entity: EntityRef) -> Option<UuidState>;
    /// Gives an [`UuidState::Unassigned`] entity its UUID and returns it.
    fn assign_uuid(&mut self, entity: EntityRef) -> u128;
}

/// Leashing, the write that follows an entity read.
pub trait EntityLeash {
    /// Leashes `entity` to the fence knot at `pos` (`Leashable.setLeashedTo`), taking it off its
    /// vehicle. False when the entity takes no lead or the handle is stale.
    fn leash_to_knot(&mut self, entity: EntityRef, pos: [i32; 3]) -> bool;
}
