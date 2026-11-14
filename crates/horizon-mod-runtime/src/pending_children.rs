//! Ownership bookkeeping for constructed, nonresident spawn outputs.
//!
//! The engine moves its actual objects here; this module never clones them or publishes
//! residency. Native and wasm adapters must use the same instance and validate species,
//! coordinates and world membership before calling mutation methods. The caller supplies
//! the parent/read-set verdict after dispatch; refusal never reruns construction.
//!
//! This is not a transaction coordinator. The caller journals parent/RNG/Host writes and
//! buffers residency notifications under its existing action savepoint. Only open a frame
//! when there are subscribers: an unused `PendingChildren` allocates nothing.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_DEPTH: usize = 16;
const MAX_SLOTS: usize = 256;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn fresh_id() -> Result<u64, Error> {
    NEXT_ID
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| Error::Budget)
}

/// Identity of one engine-owned action, never reused, including across worlds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action(u64);

impl Action {
    /// Transport identity only; constructing a token grants no access to an arena.
    pub fn from_id(id: u64) -> Self {
        Self(id)
    }

    pub fn id(self) -> u64 {
        self.0
    }
}

/// An opaque callback lease. Slots are never reused within an action.
///
/// Adapters must not substitute an entity id for this identity. Each listener and each
/// reentrant dispatch ends the previous lease; reacquire editors for remaining entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Borrow {
    world: u64,
    action: Action,
    lease: u64,
    slot: usize,
}

impl Borrow {
    /// Engine transport adapter. Every access still checks all four components.
    pub fn from_parts(world: u64, action: u64, lease: u64, slot: u32) -> Self {
        Self {
            world,
            action: Action(action),
            lease,
            slot: slot as usize,
        }
    }

    pub fn parts(self) -> (u64, u64, u64, u32) {
        (self.world, self.action.0, self.lease, self.slot as u32)
    }
}

/// Ownership of a detached object, consumable by exactly one append or replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owned(Borrow);

impl Owned {
    /// Engine transport adapter; append/replace still check detached ownership.
    pub fn from_borrow(key: Borrow) -> Self {
        Self(key)
    }
}

/// A refused bookkeeping operation. No refused edit changes the ordered list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Expired,
    Ownership,
    Capacity,
    Budget,
    Invalid,
    Stale,
    Failed,
}

/// Trusted engine verdict after guest dispatch and parent/read-set validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Accept,
    Cancel,
    Stale,
    Failed,
}

struct Slot<T> {
    object: Option<T>,
    attached: bool,
}

struct Frame<T> {
    action: Action,
    world: u64,
    lease: u64,
    capacity: usize,
    slots: Vec<Slot<T>>,
    order: Vec<usize>,
    cancelled: bool,
    /// The listener count of the hook this frame serves when its lease was last issued.
    listener: u64,
}

impl<T> Frame<T> {
    fn renew(&mut self) -> Result<(), Error> {
        self.lease = fresh_id()?;
        // Detached ownership is callback-scoped too. Never leave leaked objects for a
        // later listener to acquire under a fresh lease.
        for slot in &mut self.slots {
            if !slot.attached {
                slot.object = None;
            }
        }
        Ok(())
    }

    fn key(&self, slot: usize) -> Borrow {
        Borrow {
            world: self.world,
            action: self.action,
            lease: self.lease,
            slot,
        }
    }

    fn resolve(&self, key: Borrow) -> Result<usize, Error> {
        if key.world != self.world || key.action != self.action || key.lease != self.lease {
            return Err(Error::Expired);
        }
        self.slots
            .get(key.slot)
            .filter(|slot| slot.object.is_some())
            .map(|_| key.slot)
            .ok_or(Error::Expired)
    }

    fn owned(&self, owned: Owned) -> Result<usize, Error> {
        let slot = self.resolve(owned.0)?;
        if self.slots[slot].attached {
            return Err(Error::Ownership);
        }
        Ok(slot)
    }
}

/// A bounded stack of action arenas. Only its innermost frame is addressable.
///
/// At most 16 frames of 256 lifetime slots are retained. Removal does not reuse a slot,
/// so hostile construct/remove loops cannot grow storage or retarget a retained token.
pub struct PendingChildren<T> {
    frames: Vec<Frame<T>>,
}

impl<T> Default for PendingChildren<T> {
    fn default() -> Self {
        Self { frames: Vec::new() }
    }
}

impl<T> PendingChildren<T> {
    fn frame(&self, action: Action) -> Result<&Frame<T>, Error> {
        self.frames
            .last()
            .filter(|frame| frame.action == action)
            .ok_or(Error::Expired)
    }

    fn frame_mut(&mut self, action: Action) -> Result<&mut Frame<T>, Error> {
        self.frames
            .last_mut()
            .filter(|frame| frame.action == action)
            .ok_or(Error::Expired)
    }

    /// Own already constructed children for the duration of dispatch. `capacity` is 64
    /// for splitting and 1 for breeding. Empty/oversized capacity is refused.
    ///
    /// Every exit drops unattached objects and expires all tokens. Unwinding closes the
    /// frame before resuming the panic; the runtime's dispatch fence still reports the
    /// listener failure and rolls back its Host writes. No consumer runs here.
    /// `validate` checks every final child before any can be consumed.
    pub fn run(
        &mut self,
        world: u64,
        capacity: usize,
        initial: Vec<T>,
        dispatch: impl FnOnce(&mut Self, Action) -> Decision,
        mut validate: impl FnMut(&T) -> bool,
    ) -> Result<FrozenChildren<T>, Error> {
        let action = self.begin(world, capacity, initial)?;
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(self, action)));
        match outcome {
            Ok(decision) => self.finish(action, decision, &mut validate),
            Err(panic) => {
                self.discard(action);
                std::panic::resume_unwind(panic)
            }
        }
    }

    /// Dispatch through a shared engine-owned arena without holding a `RefCell` borrow
    /// across guest code. Host reads and edits take short borrows of the same arena;
    /// nested actions use this entry point again, invalidating outer callback leases.
    ///
    /// The scope guard closes the action on native unwind (including a nested callback
    /// failure). A caught wasm trap must return `Decision::Failed`. This only owns child
    /// cleanup: the caller still coordinates Host-write rollback and parent/RNG journals.
    pub fn run_scoped(
        arena: &RefCell<Self>,
        world: u64,
        capacity: usize,
        initial: Vec<T>,
        dispatch: impl FnOnce(Action) -> Decision,
        mut validate: impl FnMut(&T) -> bool,
    ) -> Result<FrozenChildren<T>, Error> {
        let action = arena.borrow_mut().begin(world, capacity, initial)?;
        let mut scope = ActionScope {
            arena,
            action,
            open: true,
        };
        let decision = dispatch(action);
        // finish withdraws the frame even when validation panics. Disarm before
        // entering it so unwinding never attempts a second withdrawal.
        let mut borrowed = arena.borrow_mut();
        scope.open = false;
        borrowed.finish(action, decision, &mut validate)
    }

    fn begin(&mut self, world: u64, capacity: usize, initial: Vec<T>) -> Result<Action, Error> {
        if capacity == 0 || capacity > 64 || initial.len() > capacity {
            return Err(Error::Capacity);
        }
        if self.frames.len() >= MAX_DEPTH {
            return Err(Error::Budget);
        }
        let action = Action(fresh_id()?);
        let lease = fresh_id()?;
        if let Some(parent) = self.frames.last_mut() {
            parent.renew()?;
        }
        let order = (0..initial.len()).collect();
        let slots = initial
            .into_iter()
            .map(|object| Slot {
                object: Some(object),
                attached: true,
            })
            .collect();
        self.frames.push(Frame {
            action,
            world,
            lease,
            capacity,
            slots,
            order,
            cancelled: false,
            listener: 0,
        });
        Ok(action)
    }

    fn discard(&mut self, action: Action) {
        assert_eq!(self.frames.last().map(|frame| frame.action), Some(action));
        self.frames.pop();
    }

    fn finish(
        &mut self,
        action: Action,
        decision: Decision,
        mut validate: impl FnMut(&T) -> bool,
    ) -> Result<FrozenChildren<T>, Error> {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let frame = self.frame(action).expect("dispatch must retain its frame");
            if frame.cancelled || decision == Decision::Cancel {
                return Ok(false);
            }
            match decision {
                Decision::Stale => return Err(Error::Stale),
                Decision::Failed => return Err(Error::Failed),
                Decision::Accept | Decision::Cancel => {}
            }
            if !frame
                .order
                .iter()
                .all(|&slot| frame.slots[slot].object.as_ref().is_some_and(&mut validate))
            {
                return Err(Error::Invalid);
            }
            Ok(true)
        }));
        let mut frame = self.frames.pop().expect("action frame exists");
        match outcome {
            Err(panic) => std::panic::resume_unwind(panic),
            Ok(Err(error)) => Err(error),
            Ok(Ok(accepted)) => {
                let objects = if accepted {
                    frame
                        .order
                        .into_iter()
                        .map(|slot| frame.slots[slot].object.take().expect("validated child"))
                        .collect()
                } else {
                    std::collections::VecDeque::new()
                };
                Ok(FrozenChildren {
                    objects,
                    completed: 0,
                    recovery_required: false,
                })
            }
        }
    }

    /// Tell the innermost frame how many listeners of its hook have been entered. When the
    /// count moved since the lease was issued, a new listener (or a nested dispatch of the same
    /// hook) has begun: end the previous listener's leases, as [`next_listener`](Self::next_listener)
    /// does. The same count leaves the lease alone.
    pub fn sync_listener(&mut self, serial: u64) -> Result<(), Error> {
        let Some(frame) = self.frames.last_mut() else {
            return Ok(());
        };
        if frame.listener == serial {
            return Ok(());
        }
        frame.renew()?;
        frame.listener = serial;
        Ok(())
    }

    /// End the previous listener's leases while preserving its attached edits.
    pub fn next_listener(&mut self, action: Action) -> Result<(), Error> {
        self.frame_mut(action)?.renew()
    }

    /// The current collection lease, including for an empty collection.
    pub fn lease(&self, action: Action) -> Result<(u64, u64), Error> {
        let frame = self.frame(action)?;
        Ok((frame.world, frame.lease))
    }

    /// Check a collection receiver before any operation, even reads of its length.
    pub fn check_lease(&self, world: u64, action: Action, lease: u64) -> Result<(), Error> {
        let frame = self.frame(action)?;
        if frame.world != world || frame.lease != lease {
            return Err(Error::Expired);
        }
        Ok(())
    }

    /// A different scoped editor is entering guest code. No outer child borrow survives it.
    pub fn suspend(&mut self) -> Result<(), Error> {
        if let Some(frame) = self.frames.last_mut() {
            frame.renew()?;
        }
        Ok(())
    }

    /// Refuse construction before the adapter performs constructor work or draws randomness.
    pub fn can_construct(&self, action: Action) -> Result<(), Error> {
        if self.frame(action)?.slots.len() == MAX_SLOTS {
            return Err(Error::Budget);
        }
        Ok(())
    }

    /// Monotonic cancellation; subsequent listeners may inspect but cannot undo it.
    pub fn cancel(&mut self, action: Action) -> Result<(), Error> {
        self.frame_mut(action)?.cancelled = true;
        Ok(())
    }

    pub fn len(&self, action: Action) -> Result<usize, Error> {
        Ok(self.frame(action)?.order.len())
    }

    pub fn is_empty(&self, action: Action) -> Result<bool, Error> {
        Ok(self.frame(action)?.order.is_empty())
    }

    pub fn borrow(&self, action: Action, index: usize) -> Result<Borrow, Error> {
        let frame = self.frame(action)?;
        Ok(frame.key(*frame.order.get(index).ok_or(Error::Invalid)?))
    }

    /// Lend the detached object to its constructor for initialization.
    pub fn borrow_owned(&self, owned: Owned) -> Result<Borrow, Error> {
        self.frame(owned.0.action)?.owned(owned)?;
        Ok(owned.0)
    }

    pub fn get(&self, key: Borrow) -> Result<&T, Error> {
        let frame = self.frame(key.action)?;
        let slot = frame.resolve(key)?;
        frame.slots[slot].object.as_ref().ok_or(Error::Expired)
    }

    /// Engine-only access. Adapters validate proposed values before changing an object;
    /// `run` additionally validates the entire final list before handing off ownership.
    pub fn get_mut(&mut self, key: Borrow) -> Result<&mut T, Error> {
        let frame = self.frame_mut(key.action)?;
        let slot = frame.resolve(key)?;
        frame.slots[slot].object.as_mut().ok_or(Error::Expired)
    }

    /// Move a fully constructed, validated object into detached action ownership.
    pub fn construct(&mut self, action: Action, object: T) -> Result<Owned, Error> {
        let frame = self.frame_mut(action)?;
        if frame.slots.len() == MAX_SLOTS {
            return Err(Error::Budget);
        }
        let key = frame.key(frame.slots.len());
        frame.slots.push(Slot {
            object: Some(object),
            attached: false,
        });
        Ok(Owned(key))
    }

    pub fn append(&mut self, action: Action, owned: Owned) -> Result<(), Error> {
        let frame = self.frame_mut(action)?;
        let slot = frame.owned(owned)?;
        if frame.order.len() == frame.capacity {
            return Err(Error::Capacity);
        }
        frame.order.push(slot);
        frame.slots[slot].attached = true;
        Ok(())
    }

    pub fn remove(&mut self, action: Action, index: usize) -> Result<(), Error> {
        let frame = self.frame_mut(action)?;
        let slot = *frame.order.get(index).ok_or(Error::Invalid)?;
        frame.order.remove(index);
        frame.slots[slot].object = None;
        Ok(())
    }

    /// Replace in place, destroying the former object and invalidating its borrows.
    pub fn replace(&mut self, action: Action, index: usize, owned: Owned) -> Result<(), Error> {
        let frame = self.frame_mut(action)?;
        let replacement = frame.owned(owned)?;
        let previous = *frame.order.get(index).ok_or(Error::Invalid)?;
        frame.order[index] = replacement;
        frame.slots[replacement].attached = true;
        frame.slots[previous].object = None;
        Ok(())
    }
}

/// No mutable arena borrow survives a callback. Nested scopes unwind before their
/// parent guard, so each guard removes only its own innermost frame.
struct ActionScope<'a, T> {
    arena: &'a RefCell<PendingChildren<T>>,
    action: Action,
    open: bool,
}

impl<T> Drop for ActionScope<'_, T> {
    fn drop(&mut self) {
        if self.open {
            self.arena.borrow_mut().discard(self.action);
        }
    }
}

/// Final objects, with explicit progress across irreversible insertion consumers.
///
/// A consumer is entered at most once per child. A refusal, error or panic after entry
/// requires recovery; it never permits replay of that child or the rest of this list.
pub struct FrozenChildren<T> {
    objects: std::collections::VecDeque<T>,
    completed: usize,
    recovery_required: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConsumeError<E> {
    RecoveryRequired,
    Consumer(E),
}

impl<T> FrozenChildren<T> {
    /// Number of consumers that returned successfully (including a handled join veto).
    pub fn completed(&self) -> usize {
        self.completed
    }

    pub fn recovery_required(&self) -> bool {
        self.recovery_required
    }

    /// `Ok(false)` means the list is exhausted. The engine must handle a normal
    /// per-child join refusal inside `consume` and return success for that handled result.
    pub fn consume_next<E>(
        &mut self,
        consume: impl FnOnce(T) -> Result<(), E>,
    ) -> Result<bool, ConsumeError<E>> {
        if self.recovery_required {
            return Err(ConsumeError::RecoveryRequired);
        }
        let Some(object) = self.objects.pop_front() else {
            return Ok(false);
        };
        self.recovery_required = true;
        consume(object).map_err(ConsumeError::Consumer)?;
        self.completed += 1;
        self.recovery_required = false;
        Ok(true)
    }
}

#[cfg(test)]
#[path = "pending_children_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pending_children_scoped_tests.rs"]
mod scoped_tests;
