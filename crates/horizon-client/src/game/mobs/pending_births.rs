//! The engine side of pending children (docs/modding/src/concepts/pending-children.md):
//! it constructs the children of a split or a breeding, lends them to `mob_split` and
//! `baby_entity_spawn` listeners as an engine-owned, ordered list, then freezes, validates and
//! hands back what is left, once. Native and wasm listeners reach the same arena through
//! the same editor (`native_pending_children`), so a call has the same effect from either.
//!
//! Nothing here replays construction or draws randomness: a veto, a stale parent or an invalid
//! list keeps every draw construction already made and spawns no child.

use super::Game;
use super::native_pending_children::{Children, PendingMob, valid_child};
use horizon_client::mobs::Mob;
use horizon_mod_runtime::{
    attachments::Attachments,
    pending_children::{Decision, PendingChildren},
};
use mod_api::PendingChildrenEdits;

/// A split's most children, and a breeding's only one (the design's limits).
const SPLIT_CAPACITY: usize = 64;

/// What a breeding's baby was derived from, per parent: compared again before the baby is
/// accepted, so a listener that changed a parent through the host cannot leave a baby built
/// from the old state. Anything the baby's kind, variant, colour, ownership or age came from.
#[derive(PartialEq)]
struct ParentState {
    kind: String,
    position: [f64; 3],
    age: i32,
    in_love: i32,
    variant: u8,
    variant2: u8,
    color: u8,
    tame: bool,
}

fn parent_state(mob: &Mob) -> ParentState {
    ParentState {
        kind: mob.kind.id.clone(),
        position: [mob.position.x, mob.position.y, mob.position.z],
        age: mob.extra.age,
        in_love: mob.extra.in_love,
        variant: mob.extra.variant,
        variant2: mob.extra.variant2,
        color: mob.extra.color,
        tame: mob.extra.tame,
    }
}

/// Restores the hook the pending arena was serving when a dispatch ends, even by unwinding.
struct HookScope {
    cell: std::rc::Rc<std::cell::Cell<Option<mod_api::HookId>>>,
    previous: Option<mod_api::HookId>,
}
impl Drop for HookScope {
    fn drop(&mut self) {
        self.cell.set(self.previous);
    }
}

impl Game<'_> {
    /// Lends `initial` to the listeners `fire` calls and returns the children that survive them,
    /// in order, each with its entity number issued but not yet in any level. Empty means no
    /// child: a cancel, a list a listener emptied, a trapped wasm listener, a stale parent (one of
    /// `parents` is gone or no longer in the state the children were built from, or the world or
    /// dimension changed) or a final child that breaks an invariant. Nothing is recomputed.
    ///
    /// `hook` is the hook `fire` dispatches: its listener count ends each listener's leases.
    /// `breeding` selects the one-baby limit and requires every child to be a baby. A child that
    /// already has an entity number keeps its UUID and constructor attachments here, outside the
    /// game's tables, until it joins; children a listener removed or replaced are forgotten, also
    /// when a listener panics, so none leaves state behind.
    pub(super) fn dispatch_pending_children(
        &mut self,
        hook: mod_api::HookId,
        breeding: bool,
        initial: Vec<Mob>,
        parents: &[u32],
        fire: impl FnOnce(&mut Self, &dyn PendingChildrenEdits, &mut bool),
    ) -> Vec<Mob> {
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        let capacity = if breeding { 1 } else { SPLIT_CAPACITY };
        let (dim, epoch, resources) = (self.dim, self.lend_epoch(), self.res.clone());
        let before: Vec<(u32, Option<ParentState>)> = parents
            .iter()
            .map(|&id| (id, self.mobs.iter().find(|m| m.id == id).map(parent_state)))
            .collect();
        // A child with a number already published state: take its UUID and, where nothing is
        // bound to it, its constructor attachments back out of the game's tables, so none of
        // it is visible before the child joins.
        let reserved: Vec<u32> = initial.iter().map(|m| m.id).filter(|&id| id != 0).collect();
        let initial = initial
            .into_iter()
            .map(|mob| {
                let id = mob.id;
                let uuid = (id != 0).then(|| self.persist.uuids.remove(&id)).flatten();
                let attachments = match self.entity_attachments.get(&id) {
                    Some(held)
                        if id != 0 && !held.has_index_entries() && !held.has_index_binding() =>
                    {
                        self.entity_attachments.remove(&id).unwrap_or_default()
                    }
                    _ => Attachments::default(),
                };
                PendingMob {
                    mob,
                    uuid,
                    attachments,
                    dimension: dim,
                }
            })
            .collect();
        let (arena, hook_cell) = {
            let lent = self.lent_mobs.borrow();
            (lent.pending.clone(), lent.hook.clone())
        };
        let _scope = HookScope {
            previous: hook_cell.replace(Some(hook)),
            cell: hook_cell.clone(),
        };
        let failures = self.wasm_dispatch_failures.clone();
        let run = catch_unwind(AssertUnwindSafe(|| {
            PendingChildren::run_scoped(
                &arena,
                epoch.get(),
                capacity,
                initial,
                |action| {
                    let editor = Children::new(&*self, action, breeding);
                    // The baseline: listeners entered from here on end the previous lease.
                    super::native_pending_children::sync_listener(&arena, &hook_cell);
                    let failed_before = failures.get();
                    let mut cancel = false;
                    fire(self, &editor, &mut cancel);
                    // A trapped wasm listener leaves its edits behind: none of them may commit.
                    if failures.get() != failed_before {
                        return Decision::Failed;
                    }
                    // A listener's Host writes ran against the live world: the frozen list is
                    // only valid if what it was made from still stands.
                    if self.lend_epoch() != epoch
                        || self.dim != dim
                        || before.iter().any(|(id, state)| {
                            let now = self.mobs.iter().find(|m| m.id == *id).map(parent_state);
                            now.is_none() || now.as_ref() != state.as_ref()
                        })
                    {
                        return Decision::Stale;
                    }
                    if cancel {
                        // Monotonic: a later reader of the frame sees it cancelled too.
                        let _ = arena.borrow_mut().cancel(action);
                        return Decision::Cancel;
                    }
                    Decision::Accept
                },
                |child| valid_child(&resources, dim, breeding, child),
            )
        }));
        let mut kept = Vec::new();
        let panic = match run {
            Ok(Ok(mut frozen)) => {
                while frozen
                    .consume_next::<()>(|child| {
                        kept.push(child);
                        Ok(())
                    })
                    .unwrap_or(false)
                {}
                None
            }
            Ok(Err(_)) => None,
            Err(panic) => Some(panic),
        };
        // The numbers reserved for children that did not survive; also before a panic resumes.
        for id in reserved {
            if !kept.iter().any(|child| child.mob.id == id) {
                self.forget_entity(id);
            }
        }
        if let Some(panic) = panic {
            resume_unwind(panic);
        }
        kept.into_iter()
            .map(|child| self.issue_pending_child(child))
            .collect()
    }

    /// The child as an ordinary unjoined mob. A child a mod constructed gets its entity number
    /// now, so breeding's finalization and its advancement see the real one. Its UUID and
    /// attachments, and those an engine-made child kept out of the tables while it waited, are
    /// published here: the first moment it can join.
    fn issue_pending_child(&mut self, child: PendingMob) -> Mob {
        let PendingMob {
            mut mob,
            uuid,
            attachments,
            ..
        } = child;
        let fresh = mob.id == 0;
        if fresh {
            self.next_entity += 1;
            mob.id = self.next_entity;
        }
        if let Some(uuid) = uuid {
            self.persist.uuids.insert(mob.id, uuid);
        }
        if !attachments.to_nbt().is_empty() {
            self.entity_attachments
                .entry(mob.id)
                .or_default()
                .replace_contents(attachments);
        }
        if fresh {
            let kind = super::native_hooks::mob_kind(&mob);
            self.initialize_entity(mod_api::Holder::Entity(mob.id), &kind);
        }
        mob
    }

    /// `AbstractCubeMob.remove`'s children once the parent `dying` is dead: the count is drawn,
    /// then each half-size child is constructed in order with its yaw draw from the parent. With a
    /// `mob_split` listener the whole list is constructed first and lent; otherwise each
    /// child joins as it is made, exactly as vanilla does.
    pub(super) fn split_cube_children(&mut self, dying: &mut Mob) {
        let drawn = horizon_client::mobs::cube::split_count(dying);
        if drawn == 0 {
            return;
        }
        if !mod_api::__private::listened(mod_api::HookId::MobSplit) {
            for i in 0..drawn {
                self.next_entity += 1;
                let mut child = Mob::new(self.next_entity, dying.kind.clone(), dying.position, 0.0);
                horizon_client::mobs::cube::split_child(dying, &mut child, i);
                self.mobs.push(child);
                self.native_mobs_joined(self.mobs.len() - 1, false);
            }
            return;
        }
        // The numbers are issued when the children join, so a listener's own spawns cannot
        // collide; the constructor still seeds from the number the child would have.
        let first = self.next_entity + 1;
        let children = (0..drawn)
            .map(|i| {
                let mut child = Mob::new(first + i, dying.kind.clone(), dying.position, 0.0);
                horizon_client::mobs::cube::split_child(dying, &mut child, i);
                child.id = 0;
                child
            })
            .collect();
        let dimension = self.dim.id();
        let (parent, kind, size) = (
            dying.id,
            super::native_hooks::mob_kind(dying),
            dying.extra.size,
        );
        let kept = self.dispatch_pending_children(
            mod_api::HookId::MobSplit,
            false,
            children,
            &[],
            |game, edits, cancel| {
                game.native_mob_split(mod_api::__private::MobSplitCall {
                    dimension,
                    parent,
                    kind: &kind,
                    size,
                    child_size: size / 2,
                    edits,
                    cancel,
                });
            },
        );
        for child in kept {
            self.mobs.push(child);
            self.native_mobs_joined(self.mobs.len() - 1, false);
        }
    }

    /// `Animal.spawnChildFromBreeding` once `baby` is constructed: lends it to
    /// `baby_entity_spawn` listeners and returns the baby that remains (the listener's
    /// replacement when it swapped one in), or `None` when the baby is vetoed.
    pub(super) fn breed_with_listeners(
        &mut self,
        parent: u32,
        partner: u32,
        player: (Option<u128>, Option<i32>),
        baby: Mob,
    ) -> Option<Mob> {
        let dimension = self.dim.id();
        self.dispatch_pending_children(
            mod_api::HookId::BabyEntitySpawn,
            true,
            vec![baby],
            &[parent, partner],
            |game, edits, cancel| {
                game.native_baby_entity_spawn(mod_api::__private::BabyEntitySpawnCall {
                    dimension,
                    parent,
                    partner,
                    player: player.0,
                    player_entity_id: player.1,
                    edits,
                    cancel,
                });
            },
        )
        .pop()
    }
}
