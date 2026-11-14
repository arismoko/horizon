//! The pending-child collection adapter. The existing unspawned editor resolves the same
//! arena objects. No resident entity ID, attachment holder or join is published here.

use super::Game;
use horizon_client::mobs::{Mob, Species, finalize};
use horizon_core::geometry::Vec3;
use horizon_mod_runtime::{
    attachments::Attachments,
    pending_children::{Action, Borrow, Error, Owned, PendingChildren},
};
use mod_api::{
    PendingChildrenEdits, PendingChildrenRef, PendingMobSpec, UnattachedMob, UnspawnedMobError,
    UnspawnedMobRef, WorldEpoch,
};
use std::{cell::RefCell, rc::Rc};

/// All unpublished per-object data travels with its actual Mob. A route must move its
/// existing UUID and attachments here, then install them only at residency commit.
pub(super) struct PendingMob {
    pub(super) mob: Mob,
    /// Set for a child a mod constructed; an engine-made child already owns its published UUID.
    pub(super) uuid: Option<[i32; 4]>,
    pub(super) attachments: Attachments,
    pub(super) dimension: super::Dim,
}

pub(super) fn pending_error(error: Error) -> UnspawnedMobError {
    match error {
        Error::Expired => UnspawnedMobError::Expired,
        Error::Ownership => UnspawnedMobError::Ownership,
        Error::Capacity => UnspawnedMobError::Capacity,
        Error::Budget => UnspawnedMobError::Budget,
        Error::Invalid | Error::Stale | Error::Failed => UnspawnedMobError::Invalid,
    }
}

/// Ends the previous listener's leases when a new listener of the dispatching hook has begun.
pub(super) fn sync_listener(
    arena: &RefCell<PendingChildren<PendingMob>>,
    hook: &std::cell::Cell<Option<mod_api::HookId>>,
) {
    if let Some(hook) = hook.get() {
        let _ = arena
            .borrow_mut()
            .sync_listener(mod_api::__private::listener_serial(hook));
    }
}

pub(super) fn pending_key(
    game: &Game<'_>,
    mob: UnspawnedMobRef,
) -> Result<Option<Borrow>, UnspawnedMobError> {
    sync_listener(
        &game.lent_mobs.borrow().pending,
        &game.lent_mobs.borrow().hook,
    );
    if game.native_lifecycle.epoch() != Some(mob.epoch()) {
        return Err(UnspawnedMobError::Expired);
    }
    Ok(mob
        .pending_parts()
        .map(|(action, lease, slot)| Borrow::from_parts(mob.epoch().get(), action, lease, slot)))
}

fn handle(key: Borrow) -> UnspawnedMobRef {
    let (world, action, lease, slot) = key.parts();
    UnspawnedMobRef::pending(WorldEpoch::new(world), action, lease, slot)
}

/// One callback's collection editor. It takes short arena borrows per operation; the
/// dispatcher must never keep a RefMut across guest code. Renew before each listener.
pub(super) struct Children {
    resources: std::sync::Arc<super::Resources>,
    arena: Rc<RefCell<PendingChildren<PendingMob>>>,
    action: Action,
    epoch: WorldEpoch,
    dimension: super::Dim,
    breeding: bool,
    hook: Rc<std::cell::Cell<Option<mod_api::HookId>>>,
}

impl Children {
    /// Capture only shared resources and action ownership, leaving Game available as a
    /// mutable ServerHost during dispatch. The caller must still recheck the live world,
    /// parent and read-set versions before accepting the frozen output.
    pub(super) fn new(game: &Game<'_>, action: Action, breeding: bool) -> Self {
        game.expire_standalone_lends();
        Self {
            resources: game.res.clone(),
            arena: game.lent_mobs.borrow().pending.clone(),
            action,
            epoch: game.lend_epoch(),
            dimension: game.dim,
            breeding,
            hook: game.lent_mobs.borrow().hook.clone(),
        }
    }

    pub(super) fn receiver(&self) -> Result<PendingChildrenRef, UnspawnedMobError> {
        sync_listener(&self.arena, &self.hook);
        let (world, lease) = self
            .arena
            .borrow()
            .lease(self.action)
            .map_err(pending_error)?;
        Ok(PendingChildrenRef::new(
            WorldEpoch::new(world),
            self.action.id(),
            lease,
        ))
    }

    fn check(&self, receiver: PendingChildrenRef) -> Result<(), UnspawnedMobError> {
        sync_listener(&self.arena, &self.hook);
        let (world, action, lease) = receiver.parts();
        if world != self.epoch || action != self.action.id() {
            return Err(UnspawnedMobError::Expired);
        }
        self.arena
            .borrow()
            .check_lease(world.get(), self.action, lease)
            .map_err(pending_error)
    }

    fn key(&self, mob: UnspawnedMobRef) -> Result<Borrow, UnspawnedMobError> {
        let (action, lease, slot) = mob.pending_parts().ok_or(UnspawnedMobError::Ownership)?;
        self.check(PendingChildrenRef::new(mob.epoch(), action, lease))?;
        let key = Borrow::from_parts(mob.epoch().get(), action, lease, slot);
        if self
            .arena
            .borrow()
            .get(key)
            .map_err(pending_error)?
            .dimension
            != self.dimension
        {
            return Err(UnspawnedMobError::Expired);
        }
        Ok(key)
    }

    /// Object invariants in addition to the caller's authoritative parent/read-set check.
    /// In particular this validates the complete list before any insertion consumer runs.
    #[cfg(test)]
    pub(super) fn valid_child(&self, child: &PendingMob) -> bool {
        valid_child(&self.resources, self.dimension, self.breeding, child)
    }

    fn edit(
        &self,
        mob: UnspawnedMobRef,
        edit: impl FnOnce(&mut Mob) -> Result<(), UnspawnedMobError>,
    ) -> Result<(), UnspawnedMobError> {
        let key = self.key(mob)?;
        edit(
            &mut self
                .arena
                .borrow_mut()
                .get_mut(key)
                .map_err(pending_error)?
                .mob,
        )
    }
}

impl PendingChildrenEdits for Children {
    fn children(&self) -> Result<PendingChildrenRef, UnspawnedMobError> {
        self.receiver()
    }

    fn len(&self, children: PendingChildrenRef) -> Result<u32, UnspawnedMobError> {
        self.check(children)?;
        self.arena
            .borrow()
            .len(self.action)
            .map(|n| n as u32)
            .map_err(pending_error)
    }

    fn is_empty(&self, children: PendingChildrenRef) -> Result<bool, UnspawnedMobError> {
        self.len(children).map(|n| n == 0)
    }

    fn child(
        &self,
        children: PendingChildrenRef,
        index: u32,
    ) -> Result<UnspawnedMobRef, UnspawnedMobError> {
        self.check(children)?;
        self.arena
            .borrow()
            .borrow(self.action, index as usize)
            .map(handle)
            .map_err(pending_error)
    }

    fn construct(
        &self,
        children: PendingChildrenRef,
        spec: PendingMobSpec,
    ) -> Result<UnattachedMob, UnspawnedMobError> {
        self.check(children)?;
        self.arena
            .borrow()
            .can_construct(self.action)
            .map_err(pending_error)?;
        if spec.dimension.to_string() != self.dimension.id()
            || !super::native_mobs::spawnable(spec.position)
            || !spec.yaw.is_finite()
            || (self.breeding && !spec.baby)
        {
            return Err(UnspawnedMobError::Invalid);
        }
        let kind = self
            .resources
            .mob_types()
            .get(&spec.kind.to_string())
            .ok_or(UnspawnedMobError::Invalid)?
            .clone();
        if (spec.baby && !finalize::supports_baby(kind.species))
            || !size_ok(kind.species, spec.size)
            || (kind.species == Species::SulfurCube && !spec.baby && spec.size == Some(1))
        {
            return Err(UnspawnedMobError::Invalid);
        }
        // Pending identity is action/lease/slot, never this placeholder resident id.
        // The UUID and complete constructor RNG are retained without publishing persist data.
        let (random, uuid) = horizon_client::mobs::living_random();
        let mut mob = Mob::with_random(
            0,
            kind,
            Vec3::new(spec.position[0], spec.position[1], spec.position[2]),
            spec.yaw,
            random,
        );
        finalize::set_baby(&mut mob, spec.baby);
        if let Some(size) = spec.size {
            apply_size(&mut mob, size);
        }
        let child = PendingMob {
            mob,
            uuid: Some(uuid),
            attachments: Attachments::default(),
            dimension: self.dimension,
        };
        let mut arena = self.arena.borrow_mut();
        let owned = arena.construct(self.action, child).map_err(pending_error)?;
        Ok(UnattachedMob::new(handle(
            arena.borrow_owned(owned).map_err(pending_error)?,
        )))
    }

    fn append(
        &self,
        children: PendingChildrenRef,
        child: UnattachedMob,
    ) -> Result<(), UnspawnedMobError> {
        self.check(children)?;
        let key = self.key(child.mob())?;
        self.arena
            .borrow_mut()
            .append(self.action, Owned::from_borrow(key))
            .map_err(pending_error)
    }

    fn replace(
        &self,
        children: PendingChildrenRef,
        index: u32,
        child: UnattachedMob,
    ) -> Result<(), UnspawnedMobError> {
        self.check(children)?;
        let key = self.key(child.mob())?;
        self.arena
            .borrow_mut()
            .replace(self.action, index as usize, Owned::from_borrow(key))
            .map_err(pending_error)
    }

    fn remove(&self, children: PendingChildrenRef, index: u32) -> Result<(), UnspawnedMobError> {
        self.check(children)?;
        self.arena
            .borrow_mut()
            .remove(self.action, index as usize)
            .map_err(pending_error)
    }

    fn set_position(
        &self,
        mob: UnspawnedMobRef,
        position: [f64; 3],
    ) -> Result<(), UnspawnedMobError> {
        if !super::native_mobs::spawnable(position) {
            return Err(UnspawnedMobError::Invalid);
        }
        self.edit(mob, |m| {
            m.position = Vec3::new(position[0], position[1], position[2]);
            m.previous = m.position;
            Ok(())
        })
    }

    fn set_yaw(&self, mob: UnspawnedMobRef, yaw: f32) -> Result<(), UnspawnedMobError> {
        if !yaw.is_finite() {
            return Err(UnspawnedMobError::Invalid);
        }
        self.edit(mob, |m| {
            m.yaw = yaw;
            m.previous_yaw = yaw;
            m.body_yaw = yaw;
            m.previous_body_yaw = yaw;
            m.head_yaw = yaw;
            m.previous_head_yaw = yaw;
            Ok(())
        })
    }

    fn set_baby(&self, mob: UnspawnedMobRef, baby: bool) -> Result<(), UnspawnedMobError> {
        if self.breeding && !baby {
            return Err(UnspawnedMobError::Invalid);
        }
        self.edit(mob, |m| {
            if baby && !finalize::supports_baby(m.species()) {
                return Err(UnspawnedMobError::Invalid);
            }
            finalize::set_baby(m, baby);
            Ok(())
        })
    }

    fn set_size(&self, mob: UnspawnedMobRef, size: u32) -> Result<(), UnspawnedMobError> {
        self.edit(mob, |m| {
            if !size_ok(m.species(), Some(size)) {
                return Err(UnspawnedMobError::Invalid);
            }
            apply_size(m, size);
            Ok(())
        })
    }
}

/// Whether a final child satisfies every object invariant: a registered kind, the action's
/// dimension, a finite spawnable position and yaw, baby state the species supports (a
/// breeding child must be one) and a size the species allows.
pub(super) fn valid_child(
    resources: &super::Resources,
    dimension: super::Dim,
    breeding: bool,
    child: &PendingMob,
) -> bool {
    let mob = &child.mob;
    let species = mob.species();
    child.dimension == dimension
        && resources.mob_types().contains_key(&mob.kind.id)
        && super::native_mobs::spawnable([mob.position.x, mob.position.y, mob.position.z])
        && [
            mob.yaw,
            mob.previous_yaw,
            mob.body_yaw,
            mob.previous_body_yaw,
            mob.head_yaw,
            mob.previous_head_yaw,
        ]
        .into_iter()
        .all(f32::is_finite)
        && (!breeding || mob.is_baby())
        && (!mob.is_baby() || finalize::supports_baby(species))
        && match species {
            Species::Slime | Species::MagmaCube | Species::SulfurCube | Species::Phantom => {
                u32::try_from(mob.extra.size).is_ok_and(|size| size_ok(species, Some(size)))
            }
            _ => true,
        }
}

fn size_ok(species: Species, size: Option<u32>) -> bool {
    match (species, size) {
        (Species::Slime | Species::MagmaCube | Species::SulfurCube, Some(size)) => {
            i32::try_from(size).is_ok_and(|size| horizon_client::mobs::cube::SIZES.contains(&size))
        }
        (Species::Phantom, Some(size)) => i32::try_from(size)
            .is_ok_and(|size| horizon_client::mobs::phantom::SIZES.contains(&size)),
        (Species::Slime | Species::MagmaCube | Species::SulfurCube | Species::Phantom, None) => {
            false
        }
        (_, None) => true,
        (_, Some(_)) => false,
    }
}

fn apply_size(mob: &mut Mob, size: u32) {
    match mob.species() {
        Species::Slime | Species::MagmaCube => {
            horizon_client::mobs::cube::set_size(mob, size as i32)
        }
        Species::SulfurCube => horizon_client::mobs::sulfur_cube::set_size(mob, size as i32, true),
        Species::Phantom => horizon_client::mobs::phantom::set_size(mob, size as i32),
        _ => unreachable!("species size checked before mutation"),
    }
}

#[cfg(test)]
#[path = "native_pending_children/tests.rs"]
mod tests;
