//! A storage-only collection editor for transport tests. Game validation is exercised by
//! native_pending_children's resource fixtures; this backend tests ownership and Wire only.
use horizon_mod_runtime::pending_children::{Action, Borrow, Error, Owned, PendingChildren};
use mod_api::{
    PendingChildrenEdits, PendingChildrenRef, PendingMobSpec, UnattachedMob, UnspawnedMobError,
    UnspawnedMobRef, WorldEpoch,
};
use std::cell::RefCell;

fn error(error: Error) -> UnspawnedMobError {
    match error {
        Error::Expired => UnspawnedMobError::Expired,
        Error::Ownership => UnspawnedMobError::Ownership,
        Error::Capacity => UnspawnedMobError::Capacity,
        Error::Budget => UnspawnedMobError::Budget,
        _ => UnspawnedMobError::Invalid,
    }
}

pub(super) struct Editor<'a> {
    pub(super) arena: &'a RefCell<PendingChildren<PendingMobSpec>>,
    pub(super) action: Action,
}

impl Editor<'_> {
    pub(super) fn receiver(&self) -> PendingChildrenRef {
        let (world, lease) = self.arena.borrow().lease(self.action).unwrap();
        PendingChildrenRef::new(WorldEpoch::new(world), self.action.id(), lease)
    }
    fn check(&self, children: PendingChildrenRef) -> Result<(), UnspawnedMobError> {
        let (world, action, lease) = children.parts();
        if action != self.action.id() {
            return Err(UnspawnedMobError::Expired);
        }
        self.arena
            .borrow()
            .check_lease(world.get(), self.action, lease)
            .map_err(error)
    }
    fn borrow(&self, mob: UnspawnedMobRef) -> Result<Borrow, UnspawnedMobError> {
        let (action, lease, slot) = mob.pending_parts().ok_or(UnspawnedMobError::Ownership)?;
        self.check(PendingChildrenRef::new(mob.epoch(), action, lease))?;
        Ok(Borrow::from_parts(mob.epoch().get(), action, lease, slot))
    }
    fn edit(
        &self,
        mob: UnspawnedMobRef,
        edit: impl FnOnce(&mut PendingMobSpec),
    ) -> Result<(), UnspawnedMobError> {
        let key = self.borrow(mob)?;
        edit(self.arena.borrow_mut().get_mut(key).map_err(error)?);
        Ok(())
    }
}

fn handle(key: Borrow) -> UnspawnedMobRef {
    let (world, action, lease, slot) = key.parts();
    UnspawnedMobRef::pending(WorldEpoch::new(world), action, lease, slot)
}

impl PendingChildrenEdits for Editor<'_> {
    fn children(&self) -> Result<PendingChildrenRef, UnspawnedMobError> {
        Ok(self.receiver())
    }
    fn len(&self, children: PendingChildrenRef) -> Result<u32, UnspawnedMobError> {
        self.check(children)?;
        self.arena
            .borrow()
            .len(self.action)
            .map(|n| n as u32)
            .map_err(error)
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
            .map_err(error)
    }
    fn construct(
        &self,
        children: PendingChildrenRef,
        spec: PendingMobSpec,
    ) -> Result<UnattachedMob, UnspawnedMobError> {
        self.check(children)?;
        let mut arena = self.arena.borrow_mut();
        let owned = arena.construct(self.action, spec).map_err(error)?;
        Ok(UnattachedMob::new(handle(
            arena.borrow_owned(owned).map_err(error)?,
        )))
    }
    fn append(
        &self,
        children: PendingChildrenRef,
        child: UnattachedMob,
    ) -> Result<(), UnspawnedMobError> {
        self.check(children)?;
        let key = self.borrow(child.mob())?;
        self.arena
            .borrow_mut()
            .append(self.action, Owned::from_borrow(key))
            .map_err(error)
    }
    fn replace(
        &self,
        children: PendingChildrenRef,
        index: u32,
        child: UnattachedMob,
    ) -> Result<(), UnspawnedMobError> {
        self.check(children)?;
        let key = self.borrow(child.mob())?;
        self.arena
            .borrow_mut()
            .replace(self.action, index as usize, Owned::from_borrow(key))
            .map_err(error)
    }
    fn remove(&self, children: PendingChildrenRef, index: u32) -> Result<(), UnspawnedMobError> {
        self.check(children)?;
        self.arena
            .borrow_mut()
            .remove(self.action, index as usize)
            .map_err(error)
    }
    fn set_position(
        &self,
        mob: UnspawnedMobRef,
        position: [f64; 3],
    ) -> Result<(), UnspawnedMobError> {
        self.edit(mob, |m| m.position = position)
    }
    fn set_yaw(&self, mob: UnspawnedMobRef, yaw: f32) -> Result<(), UnspawnedMobError> {
        self.edit(mob, |m| m.yaw = yaw)
    }
    fn set_baby(&self, mob: UnspawnedMobRef, baby: bool) -> Result<(), UnspawnedMobError> {
        self.edit(mob, |m| m.baby = baby)
    }
    fn set_size(&self, mob: UnspawnedMobRef, size: u32) -> Result<(), UnspawnedMobError> {
        self.edit(mob, |m| m.size = Some(size))
    }
}
