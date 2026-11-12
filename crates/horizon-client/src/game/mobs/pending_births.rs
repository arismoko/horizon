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
use mod_api::{PendingChildrenEdits, PendingChildrenRef};

/// A split's most children, and a breeding's only one (the design's limits).
const SPLIT_CAPACITY: usize = 64;

impl Game<'_> {
    /// Lends `initial` to the listeners `fire` calls and returns the children that survive them,
    /// in order, each with its entity number issued but not yet in any level. Empty means no
    /// child: a cancel, a list a listener emptied, a stale parent (one of `live` is gone, or
    /// the world or dimension changed) or a final child that breaks an invariant.
    ///
    /// `breeding` selects the one-baby limit and requires every child to be a baby. Children a
    /// listener removed or replaced are forgotten here, so none leaves state behind.
    pub(super) fn dispatch_pending_children(
        &mut self,
        breeding: bool,
        initial: Vec<Mob>,
        live: &[u32],
        fire: impl FnOnce(&mut Self, PendingChildrenRef, &dyn PendingChildrenEdits, &mut bool),
    ) -> Vec<Mob> {
        let capacity = if breeding { 1 } else { SPLIT_CAPACITY };
        let (dim, epoch, resources) = (self.dim, self.lend_epoch(), self.res.clone());
        // Only a child with a number already published state (UUID, constructor
        // attachments) that must go if the child does.
        let reserved: Vec<u32> = initial.iter().map(|m| m.id).filter(|&id| id != 0).collect();
        let initial = initial
            .into_iter()
            .map(|mob| PendingMob {
                mob,
                uuid: None,
                attachments: Attachments::default(),
                dimension: dim,
            })
            .collect();
        let arena = self.lent_mobs.borrow().pending.clone();
        let frozen = PendingChildren::run_scoped(
            &arena,
            epoch.get(),
            capacity,
            initial,
            |action| {
                let editor = Children::new(&*self, action, breeding);
                let Ok(receiver) = editor.receiver() else {
                    return Decision::Failed;
                };
                let mut cancel = false;
                fire(self, receiver, &editor, &mut cancel);
                // A listener's Host writes ran against the live world: the frozen list is
                // only valid if what it was made from still stands.
                if self.lend_epoch() != epoch
                    || self.dim != dim
                    || !live.iter().all(|id| self.mobs.iter().any(|m| m.id == *id))
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
        );
        let mut kept = Vec::new();
        if let Ok(mut frozen) = frozen {
            while frozen
                .consume_next::<()>(|child| {
                    kept.push(child);
                    Ok(())
                })
                .unwrap_or(false)
            {}
        }
        for id in reserved {
            if !kept.iter().any(|child| child.mob.id == id) {
                self.forget_entity(id);
            }
        }
        kept.into_iter()
            .map(|child| self.issue_pending_child(child))
            .collect()
    }

    /// The child as an ordinary unjoined mob: a child a mod constructed gets its entity number
    /// and UUID now, so breeding's finalization and its advancement see the real one; an
    /// engine-made child already has both.
    fn issue_pending_child(&mut self, child: PendingMob) -> Mob {
        let PendingMob {
            mut mob,
            uuid,
            attachments,
            ..
        } = child;
        if mob.id == 0 {
            self.next_entity += 1;
            mob.id = self.next_entity;
            if let Some(uuid) = uuid {
                self.persist.uuids.insert(mob.id, uuid);
            }
            if !attachments.to_nbt().is_empty() {
                self.entity_attachments
                    .entry(mob.id)
                    .or_default()
                    .replace_contents(attachments);
            }
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
            false,
            children,
            &[],
            |game, children, edits, cancel| {
                game.native_mob_split(mod_api::__private::MobSplitCall {
                    dimension,
                    parent,
                    kind: &kind,
                    size,
                    child_size: size / 2,
                    children,
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
            true,
            vec![baby],
            &[parent, partner],
            |game, children, edits, cancel| {
                game.native_baby_entity_spawn(mod_api::__private::BabyEntitySpawnCall {
                    dimension,
                    parent,
                    partner,
                    player: player.0,
                    player_entity_id: player.1,
                    children,
                    edits,
                    cancel,
                });
            },
        )
        .pop()
    }
}
