//! Scoped editing of engine-owned, constructed children before residency.

use crate::{Identifier, UnspawnedMobError, UnspawnedMobRef, WorldEpoch};

/// A listener-scoped lease of an ordered child collection, acquired with
/// [`PendingChildrenEdits::children`]. Retaining this value does not extend its lifetime: the
/// next listener, a nested dispatch or the callback's return expires it. Acquire it again in each
/// callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[non_exhaustive]
pub struct PendingChildrenRef {
    pub(crate) epoch: WorldEpoch,
    pub(crate) action: u64,
    pub(crate) lease: u64,
}

impl PendingChildrenRef {
    /// Engine transport constructor. Every operation validates the complete lease.
    // hidden: engine transport only; arena validation grants authority
    #[doc(hidden)]
    pub fn new(epoch: WorldEpoch, action: u64, lease: u64) -> Self {
        Self {
            epoch,
            action,
            lease,
        }
    }

    /// Engine transport components.
    // hidden: engine transport only; arena validation grants authority
    #[doc(hidden)]
    pub fn parts(self) -> (WorldEpoch, u64, u64) {
        (self.epoch, self.action, self.lease)
    }
}

/// A detached, action-owned child. Append or replacement consumes its ownership exactly
/// once, even if a mod retained a copy of this token. It cannot name a resident entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[non_exhaustive]
pub struct UnattachedMob {
    pub(crate) mob: UnspawnedMobRef,
}

impl UnattachedMob {
    /// Engine transport constructor; the arena checks that the child is detached.
    // hidden: engine transport only; arena validation grants authority
    #[doc(hidden)]
    pub fn new(mob: UnspawnedMobRef) -> Self {
        Self { mob }
    }

    /// Borrow the child for reads or edits. This never transfers its ownership.
    pub fn mob(self) -> UnspawnedMobRef {
        self.mob
    }
}

/// Initial values for a new child. Construction runs no spawn finalization or insertion.
/// The engine checks every field before constructing anything or consuming an arena slot.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct PendingMobSpec {
    /// A registered mob type.
    pub kind: Identifier,
    /// Must equal the action's dimension.
    pub dimension: Identifier,
    /// Finite coordinates within the world's entity bounds.
    pub position: [f64; 3],
    /// Finite yaw, in degrees.
    pub yaw: f32,
    /// Whether to initialize the species' baby state. Breeding requires a baby.
    pub baby: bool,
    /// Explicit size for a sized species; absent for other species.
    pub size: Option<u32>,
}

impl PendingMobSpec {
    /// Initial values; set `size` for a species with an explicit size.
    pub fn new(
        kind: Identifier,
        dimension: Identifier,
        position: [f64; 3],
        yaw: f32,
        baby: bool,
    ) -> Self {
        Self {
            kind,
            dimension,
            position,
            yaw,
            baby,
            size: None,
        }
    }
}

/// An editor lent beside a collection. Native and wasm use the same scoped operations and
/// ownership checks. Reads and rejected operations leave the collection unchanged. Accepted
/// edits are visible to the next listener; unattached children die when this lease ends.
/// This edits only nonresident objects; the action's transaction controls eventual insertion.
pub trait PendingChildrenEdits {
    /// The collection for this listener. Call it in every callback: a lease from an earlier
    /// listener, or from before a nested dispatch, answers `Expired`.
    fn children(&self) -> Result<PendingChildrenRef, UnspawnedMobError>;
    /// Number of attached children, in insertion order.
    fn len(&self, children: PendingChildrenRef) -> Result<u32, UnspawnedMobError>;
    /// Whether the collection is empty.
    fn is_empty(&self, children: PendingChildrenRef) -> Result<bool, UnspawnedMobError>;
    /// Reacquire a callback-scoped editor borrow of the child at `index`.
    fn child(
        &self,
        children: PendingChildrenRef,
        index: u32,
    ) -> Result<UnspawnedMobRef, UnspawnedMobError>;
    /// Construct a detached child; this does not add it to the ordered list.
    fn construct(
        &self,
        children: PendingChildrenRef,
        spec: PendingMobSpec,
    ) -> Result<UnattachedMob, UnspawnedMobError>;
    /// Append a detached child, consuming its ownership. Split lists hold at most 64
    /// children and breeding lists at most one; exceeding that limit is an error.
    fn append(
        &self,
        children: PendingChildrenRef,
        child: UnattachedMob,
    ) -> Result<(), UnspawnedMobError>;
    /// Replace one child in place, consuming the replacement and destroying the former child.
    fn replace(
        &self,
        children: PendingChildrenRef,
        index: u32,
        child: UnattachedMob,
    ) -> Result<(), UnspawnedMobError>;
    /// Destroy one child and expire all its borrows; later children retain their order.
    fn remove(&self, children: PendingChildrenRef, index: u32) -> Result<(), UnspawnedMobError>;
    /// Move an attached or detached child within the action's dimension.
    fn set_position(
        &self,
        mob: UnspawnedMobRef,
        position: [f64; 3],
    ) -> Result<(), UnspawnedMobError>;
    /// Set all initial yaw fields in degrees.
    fn set_yaw(&self, mob: UnspawnedMobRef, yaw: f32) -> Result<(), UnspawnedMobError>;
    /// Set a supported species' baby state. A breeding child must remain a baby.
    fn set_baby(&self, mob: UnspawnedMobRef, baby: bool) -> Result<(), UnspawnedMobError>;
    /// Set a sized species' size within its own bounds, without silently clamping it.
    fn set_size(&self, mob: UnspawnedMobRef, size: u32) -> Result<(), UnspawnedMobError>;
}
