//! The engine side of pending children for `mob_split` and `baby_entity_spawn`:
//! constructs the children on the stack, lends them to listeners via scoped editors
//! (`SplitChildList` / `BabyChildSlot`), and inserts whatever survives in order if not cancelled.
//!
//! Nothing here replays construction or draws randomness: a cancel or empty output keeps every
//! draw construction already made and spawns no child, and unwinding drops pending state via RAII.

use super::native_pending_children::{BabyChildSlot, ChildConstraint, PendingMob, SplitChildList};
use crate::game::ServerGame;
use mod_api::{BabyEntitySpawnChild, MobSplitChildren, UnspawnedMob, UnspawnedMobError};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use sunrise_sim::mobs::Mob;

static NEXT_CHILD_ACTION: AtomicU64 = AtomicU64::new(1);

/// Result of dispatching `baby_entity_spawn`: distinguishes cancellation (which resets both
/// parents per NeoForge's `Animal.spawnChildFromBreeding` / `Fox.FoxBreedGoal.breed`) from
/// an uncanceled empty slot (which skips breeding finalization without resetting parents).
pub(in crate::game::server_side) enum BreedOutcome {
    Accepted(Option<(PendingMob, u64)>),
    Cancelled,
}

impl ServerGame<'_> {
    /// Run construction with the holder reachable, but retire only this unpublished mob's
    /// metadata if dispatch unwinds. Unrelated Host writes stay in the level.
    pub(in crate::game::server_side) fn initialize_unpublished_mob(&mut self, id: u32, kind: &str) {
        let run = catch_unwind(AssertUnwindSafe(|| {
            self.initialize_entity(mod_api::Holder::Entity(id), kind);
        }));
        if let Err(panic) = run {
            self.forget_entity(id);
            resume_unwind(panic);
        }
    }

    /// Move constructor state out of the level before lending a proposed child. Dropping
    /// the owned value now discards just this child's unpublished data.
    fn stash_pending_child(&mut self, mut mob: Mob, action: u64) -> PendingMob {
        let id = std::mem::replace(&mut mob.id, 0);
        let (uuid, attachments, initialized) = self.take_pending_state(id);
        PendingMob {
            mob,
            uuid,
            attachments,
            initialized,
            origin_reservation: Some((action, id)),
            dimension: self.common.dim,
            resources: self.common.res.clone(),
            attributes: self.common.motion.attributes.clone(),
        }
    }

    fn take_pending_state(
        &mut self,
        id: u32,
    ) -> (
        Option<[i32; 4]>,
        sunrise_mod_runtime::attachments::Attachments,
        bool,
    ) {
        let uuid = self.common.persist.uuids.remove(&id);
        let mut attachments = self
            .common
            .entity_attachments
            .remove(&id)
            .unwrap_or_default();
        attachments.unbind_index();
        let initialized = self
            .common
            .entity_initialization
            .initialized
            .remove(&mod_api::Holder::Entity(id));
        self.forget_entity(id);
        (uuid, attachments, initialized)
    }

    /// Reborrow the active server Host through the scoped editor to run the constructor now,
    /// not if and when the caller puts this detached mob in a slot or list.
    pub(in crate::game) fn initialize_pending_child(
        &mut self,
        child: UnspawnedMob,
        action: u64,
    ) -> Result<UnspawnedMob, UnspawnedMobError> {
        let mut child = child
            .into_any()
            .downcast::<PendingMob>()
            .map_err(|_| UnspawnedMobError::Invalid)?;
        if child.dimension != self.common.dim || child.origin_reservation.is_some() {
            return Err(UnspawnedMobError::Invalid);
        }
        let id = self
            .server
            .next_entity
            .checked_add(1)
            .ok_or(UnspawnedMobError::Unsupported)?;
        self.server.next_entity = id;
        if let Some(uuid) = child.uuid {
            self.common.persist.uuids.insert(id, uuid);
        }
        let kind = sunrise_gameplay_common::game::native_hooks::mob_kind(&child.mob);
        self.initialize_unpublished_mob(id, &kind);
        (child.uuid, child.attachments, child.initialized) = self.take_pending_state(id);
        child.origin_reservation = Some((action, id));
        Ok(UnspawnedMob::new(child))
    }

    /// Publish one pending child's constructor state immediately before it is offered to the level.
    pub(in crate::game::server_side) fn issue_pending_child(
        &mut self,
        child: PendingMob,
        action: u64,
    ) -> Mob {
        let PendingMob {
            mut mob,
            uuid,
            attachments,
            initialized,
            origin_reservation,
            ..
        } = child;
        // Reuse the ID only in its originating action. Detached children reattached to a
        // different action or world get an ID belonging to that destination.
        let reused = origin_reservation
            .filter(|&(origin, id)| {
                origin == action
                    && !self.common.persist.uuids.contains_key(&id)
                    && !self.common.entity_attachments.contains_key(&id)
                    && !self.common.mobs.iter().any(|m| m.id == id)
            })
            .map(|(_, id)| id);
        mob.id = match reused {
            Some(id) => id,
            None => {
                self.server.next_entity += 1;
                self.server.next_entity
            }
        };
        let id = mob.id;
        if let Some(uuid) = uuid {
            self.common.persist.uuids.insert(id, uuid);
        }
        if !attachments.is_empty() {
            self.common
                .entity_attachments
                .entry(id)
                .or_default()
                .replace_contents(attachments);
        }
        if initialized {
            self.common
                .entity_initialization
                .initialized
                .insert(mod_api::Holder::Entity(id));
        }
        mob
    }

    /// Construct each original split proposal, including its own UUID and the parent's yaw draw.
    fn split_proposal(
        &mut self,
        dying: &mut Mob,
        index: u32,
        action: u64,
        kind: &str,
    ) -> PendingMob {
        self.server.next_entity += 1;
        let (random, uuid) = sunrise_sim::mobs::living_random();
        let mut child = Mob::with_random(
            self.server.next_entity,
            dying.kind.clone(),
            dying.position,
            0.0,
            random,
        );
        self.common.persist.uuids.insert(child.id, uuid);
        self.initialize_unpublished_mob(child.id, kind);
        sunrise_sim::mobs::cube::split_child(dying, &mut child, index);
        self.stash_pending_child(child, action)
    }

    /// NeoForge's `AbstractCubeMob.remove` collects the whole proposal list before admission.
    /// The unedited list fits inline; only a split listener needs a growable list.
    pub fn split_cube_children(&mut self, dying: &mut Mob) {
        let drawn = sunrise_sim::mobs::cube::split_count(dying);
        let action = NEXT_CHILD_ACTION.fetch_add(1, Ordering::Relaxed);
        let mob_type = dying.kind.clone();
        let kind = &mob_type.id;
        if !mod_api::__private::listened(mod_api::HookId::MobSplit) {
            // AbstractCubeMob.getSplitCount yields 2–4; SulfurCube.getSplitCount yields 0 or 2.
            let mut proposals: [Option<PendingMob>; 4] = std::array::from_fn(|_| None);
            for i in 0..drawn {
                proposals[i as usize] = Some(self.split_proposal(dying, i, action, kind));
            }
            self.admit_split_children(proposals.into_iter().flatten(), action);
            return;
        }
        let dimension = self.common.dim;
        let resources = self.common.res.clone();
        let attributes = self.common.motion.attributes.clone();
        let (parent, size) = (dying.id, dying.extra.size);
        let mut pending = (0..drawn)
            .map(|i| self.split_proposal(dying, i, action, kind))
            .collect();
        let mut cancel = false;
        {
            let mut editor = SplitChildList {
                action,
                children: &mut pending,
                resources,
                attributes,
                dimension,
            };
            self.native_mob_split(mod_api::__private::MobSplitCall {
                dimension: dimension.id(),
                parent,
                kind,
                size,
                child_size: size / 2,
                children: MobSplitChildren::new(&mut editor),
                cancel: &mut cancel,
            });
        }
        #[cfg(any(test, feature = "test-support"))]
        escaping_fault::reach(escaping_fault::At::SplitDispatched, parent);
        if !cancel {
            self.admit_split_children(pending, action);
        }
    }

    fn admit_split_children(
        &mut self,
        children: impl IntoIterator<Item = PendingMob>,
        action: u64,
    ) {
        for child in children {
            // A failure admitting one child leaves earlier joins alone and drops the rest
            // with their unpublished state, with no state on the level to clean up.
            let mob = self.issue_pending_child(child, action);
            self.common.mobs.push(mob);
            self.admit_new_mobs(self.common.mobs.len() - 1);
        }
    }

    /// Hold the child off-table through breeding credit and any preceding admissions.
    pub(in crate::game::server_side) fn stash_breeding_child(
        &mut self,
        child: Mob,
    ) -> (PendingMob, u64) {
        let action = NEXT_CHILD_ACTION.fetch_add(1, Ordering::Relaxed);
        (self.stash_pending_child(child, action), action)
    }

    /// `Animal.spawnChildFromBreeding` / `Fox.FoxBreedGoal.breed` once the proposed `baby`
    /// (if any) is constructed: lends it in a stack-local optional slot to `baby_entity_spawn`
    /// listeners and distinguishes cancellation from an uncanceled empty slot.
    pub(in crate::game::server_side) fn breed_with_listeners(
        &mut self,
        parent: u32,
        partner: u32,
        player: (Option<u128>, Option<i32>),
        baby: Option<Mob>,
        constraint: ChildConstraint,
    ) -> BreedOutcome {
        let dimension = self.common.dim;
        let resources = self.common.res.clone();
        let attributes = self.common.motion.attributes.clone();
        let action = NEXT_CHILD_ACTION.fetch_add(1, Ordering::Relaxed);
        let mut slot = baby.map(|baby| self.stash_pending_child(baby, action));
        let mut cancel = false;
        {
            let mut editor = BabyChildSlot {
                action,
                slot: &mut slot,
                resources,
                attributes,
                dimension,
                constraint,
            };
            self.native_baby_entity_spawn(mod_api::__private::BabyEntitySpawnCall {
                dimension: dimension.id(),
                parent,
                partner,
                player: player.0,
                player_entity_id: player.1,
                child: BabyEntitySpawnChild::new(&mut editor),
                cancel: &mut cancel,
            });
        }
        #[cfg(any(test, feature = "test-support"))]
        escaping_fault::reach(escaping_fault::At::BabyDispatched, parent);
        if cancel {
            return BreedOutcome::Cancelled;
        }
        BreedOutcome::Accepted(slot.map(|kept| (kept, action)))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod escaping_fault {
    use std::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum At {
        Constructed,
        Admission,
        SplitDispatched,
        BabyDispatched,
    }

    thread_local! {
        static ARMED: Cell<Option<(At, u32)>> = const { Cell::new(None) };
    }

    /// Fail at an engine checkpoint, not from inside a mod callback. The unique payload
    /// lets the fixture distinguish an exercised cleanup path from an unrelated assertion.
    pub(in crate::game) fn reach(at: At, id: u32) {
        if ARMED.get() == Some((at, id)) {
            ARMED.set(None);
            resume_unwind(Box::new((at, id)));
        }
    }

    pub fn expect(at: At, id: u32, run: impl FnOnce()) {
        let previous = ARMED.replace(Some((at, id)));
        let result = catch_unwind(AssertUnwindSafe(run));
        ARMED.set(previous);
        let payload = result.expect_err("engine checkpoint did not unwind");
        assert_eq!(payload.downcast_ref::<(At, u32)>(), Some(&(at, id)));
    }
}

use sunrise_gameplay_common::game::entity_initialization_common::EntityInitializationCommonGameExt1 as _;
