//! Entities for native server mods (docs/modding/src/how-to/native-mods.md, "Entities"): spawn
//! any entity from its compound as `/summon` does through [`LevelRef::summon`], and move, show or remove
//! the entities Sunrise keeps without simulating them (displays, markers,
//! interactions); `native_display_refs.rs` poses displays.
use super::commands::Target;
#[cfg(test)]
use super::commands::{data::summon_position_ok, place_summoned};
use mod_api::{EntityRef, LevelRef, LevelWriteError};
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Compound;
#[cfg(test)]
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::commands_common::target_id;

/// The id of a summoned stack's root: its command target's, else `first`,
/// the first id handed out, when an entity no command target stands for
/// (`live`) holds it.
fn root_id(target: Option<Target>, first: u32, live: impl Fn(u32) -> bool) -> Option<u32> {
    match target {
        Some(t) => target_id(t),
        None => live(first).then_some(first),
    }
}

impl crate::game::ServerGame<'_> {
    /// Spawns an entity of `kind` (`minecraft:item_display`) at `pos` in the
    /// server level `level` names from `nbt` (its compound, as `/summon
    /// <kind> <pos> <nbt>` takes it; passengers included): its [`EntityRef`].
    /// Routes parked dimensions by handle and refuses with
    /// [`LevelWriteError::InTransaction`] when summoning into a parked level
    /// while a Host transaction is open. Returns `Ok(None)` for a position
    /// outside `/summon`'s bounds, a chunk that isn't loaded, a kind Sunrise
    /// can't load, one a mod kept out (`native_entity_join_level`), or with
    /// entity ids exhausted. Inside a transaction the handle comes back at
    /// once: a join a mod cancels at the root commit leaves its handle stale.
    pub fn level_summon(
        &mut self,
        level: LevelRef,
        kind: &str,
        pos: [f64; 3],
        nbt: Compound,
    ) -> Result<Option<EntityRef>, LevelWriteError> {
        self.with_server_level_mut(
            level,
            LevelWriteError::Unavailable,
            LevelWriteError::Stale,
            Some(LevelWriteError::InTransaction),
            |game| {
                let epoch = game
                    .common
                    .native_lifecycle
                    .epoch()
                    .ok_or(LevelWriteError::Unavailable)?;
                if game.entity_ids_exhausted() {
                    return Ok(None);
                }
                let chunk = ((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4);
                if !game.common.world.chunks.contains_key(&chunk) {
                    return Ok(None);
                }
                let first = game.server.next_entity + 1;
                let deferred = (
                    game.common.tnts.len(),
                    game.common.falling.len(),
                    game.common.rockets.len(),
                );
                let Ok(target) = game.summon_from_compound(kind, pos, nbt) else {
                    return Ok(None);
                };
                // These roots take their id as they load, and a rocket has no command
                // Target variant. Select the newly appended root, never an older waiting entity.
                let root = match kind {
                    "minecraft:tnt" if deferred.0 < game.common.tnts.len() => {
                        Some(super::entity_residency::assign_resident_id!(
                            game.common.tnts,
                            deferred.0,
                            e,
                            &mut game.server.next_entity,
                            None
                        ))
                    }
                    "minecraft:falling_block" if deferred.1 < game.common.falling.len() => {
                        Some(super::entity_residency::assign_resident_id!(
                            game.common.falling,
                            deferred.1,
                            f,
                            &mut game.server.next_entity,
                            Some(&f.attachment_lifetime)
                        ))
                    }
                    "minecraft:firework_rocket" if deferred.2 < game.common.rockets.len() => {
                        Some(super::entity_residency::assign_resident_id!(
                            game.common.rockets,
                            deferred.2,
                            e,
                            &mut game.server.next_entity,
                            None
                        ))
                    }
                    _ => None,
                };
                // Otherwise the root is the first entity spawned, also for kinds no
                // command target stands for (experience orbs, end crystals, cushions). An id handed out to an
                // entity a mod then kept out is not given back, so the root must be
                // there.
                let id = root.or_else(|| {
                    root_id(target, first, |id| {
                        game.common.orbs.iter().any(|o| o.id == id)
                            || game.common.crystals.iter().any(|c| c.id == id)
                            || game.common.cushions.iter().any(|c| c.id == id)
                    })
                });
                Ok(id.map(|id| EntityRef::new(epoch, id)))
            },
        )
    }
}

/// The entities the walk does not hold: Sunrise keeps these in lists of their own.
#[derive(Clone, Copy)]
enum Apart {
    Crystal(usize),
    Cushion(usize),
    Painting(usize),
    Frame(usize),
    Other(usize),
    Dragon,
}

/// One entity a mod is told of: in the walk, or apart from it.
#[derive(Clone, Copy)]
enum Slot {
    Walk(super::entity::EntityHandle),
    Apart(Apart),
}

/// Entities joining and leaving the level (`native_entity_join_level`,
/// `native_entity_leave_level`), for every kind in the entity walk and the
/// families kept apart from it. Kept out of `native_hooks.rs`, whose lines may not hold macros.
impl crate::game::ServerGame<'_> {
    /// Whether an entity may have joined since the last announcement. A join
    /// in another level may cause one harmless walk of this level.
    fn joins_pending(&self) -> bool {
        sunrise_sim::join_seq::JoinSeq::latest() > self.common.persist.joined_seq
    }
    /// Tells `native_entity_join_level` once for every entity added since the
    /// last call, of any kind, in insertion order. A mod that cancels takes
    /// the entity back out here,
    /// before it ticks again, and `/summon` of it fails (`joins_canceled`).
    /// `loaded_from_disk` is for the entities that came with a chunk. Returns
    /// whether `watch` (an id, or 0) is still in the level. Walks the lists
    /// with static dispatch and allocates nothing but the kind name of an
    /// entity it announces; nothing runs when no mod is loaded, on a client
    /// of a server (which tells nothing), or when nothing is new.
    /// Inside a transaction open in this level, construction is still told now, and the join
    /// at its root commit (`transactions/entities.rs`), with this call's `loaded_from_disk`, in
    /// insertion order and before the transaction's removals take effect; the entity is kept
    /// until then. The same holds for a call a listener makes while that commit is told: its
    /// joins are told after the batch being told. A `through` is the commit's own flush of one
    /// batch, never held.
    pub fn native_joins_flush(&mut self, loaded_from_disk: bool, watch: u32) -> bool {
        self.flush_joins(loaded_from_disk, watch, &[], None)
    }

    /// `addFreshEntity` at a call site that has a specific new row. It does not wait
    /// for an unrelated item still being decided by `item_toss`. The level's journal
    /// can hold the join, but construction still happens at the call.
    pub(in crate::game) fn announce_fresh_join(
        &mut self,
        handle: super::entity::EntityHandle,
        id: u32,
    ) -> bool {
        if !self.common.is_logical_server() || mod_api::__private::mods().is_empty() {
            return true;
        }
        if self.common.hold_announcement(
            sunrise_gameplay_common::game::transactions::entities::Announcement::FreshJoin {
                handle,
                id,
            },
        ) {
            let kind = self
                .slot_kind_and_position(Slot::Walk(handle))
                .0
                .into_owned();
            self.construct_joining(id, &kind);
            return true;
        }
        self.admit_fresh_join(handle, id)
    }

    /// Admit one fresh row now, either synchronously or when its journal is told.
    pub(in crate::game) fn admit_fresh_join(
        &mut self,
        handle: super::entity::EntityHandle,
        id: u32,
    ) -> bool {
        if self.join_was_announced(id) {
            return true;
        }
        let Some(handle) = self.refresh_entity_handle(id, handle) else {
            return false;
        };
        use sunrise_sim::join_seq::Joined as _;
        let seq = super::entity::with_entity!(self, handle, e => e.join_seq());
        if seq > self.common.persist.joined_seq {
            self.common.persist.admitted_joins.push((seq, id));
        }
        self.common.native_lifecycle.admitting_joins.push(id);
        let kept = self.admit_join(Slot::Walk(handle), id, false);
        self.common.native_lifecycle.admitting_joins.pop();
        kept
    }
    /// [`Self::native_joins_flush`], never announcing the entities `removed` names and, with a
    /// `through`, none that joined after it: a root commit's, for the entities its transaction
    /// added and removed, each held flush keeping its own `loaded_from_disk`.
    pub(in crate::game::server_side) fn flush_joins(
        &mut self,
        loaded_from_disk: bool,
        watch: u32,
        removed: &[u32],
        through: Option<sunrise_sim::join_seq::JoinSeq>,
    ) -> bool {
        if !self.common.is_logical_server()
            || mod_api::__private::mods().is_empty()
            || !self.joins_pending()
        {
            return true;
        }
        self.assign_pending_join_ids();
        // Read before this call's own announcement is held: it reaches the entities to construct.
        let held_before = self.common.held_joins_through();
        if through.is_none()
            && self.common.hold_announcement(
                sunrise_gameplay_common::game::transactions::entities::Announcement::Joins {
                    loaded_from_disk,
                    through: sunrise_sim::join_seq::JoinSeq::latest(),
                },
            )
        {
            // What an earlier held flush of this level told is constructed already; with a toss
            // still asked it may have stopped short, so then everything since the mark is looked at.
            let mut mark = self.common.persist.joined_seq;
            if self.common.native_lifecycle.held_joins.is_empty() {
                mark = mark.max(held_before);
            }
            while let Some((slot, id, seq)) = self.next_unannounced(mark, &[], None) {
                mark = seq;
                let kind = self.slot_kind_and_position(slot).0.into_owned();
                self.construct_joining(id, &kind);
            }
            return true;
        }
        let latest = sunrise_sim::join_seq::JoinSeq::latest();
        let observed = through.map_or(latest, |through| through.min(latest));
        let mut kept = true;
        while let Some((slot, id, seq)) =
            self.next_unannounced(self.common.persist.joined_seq, removed, through)
        {
            self.common.persist.joined_seq = seq;
            if !self.admit_join(slot, id, loaded_from_disk) && id == watch {
                kept = false;
            }
        }
        // A held toss is not past yet: the mark stays before it.
        if self.common.native_lifecycle.held_joins.is_empty() {
            self.common.persist.joined_seq = self.common.persist.joined_seq.max(observed);
        }
        self.common
            .persist
            .admitted_joins
            .retain(|&(seq, _)| seq > self.common.persist.joined_seq);
        kept
    }

    /// The shared `PersistentEntitySectionManager.addEntity` boundary, for a fresh
    /// row or for a row waiting in a batch. NeoForge asks `EntityJoinLevelEvent`
    /// before filing the entity in its section.
    fn admit_join(&mut self, slot: Slot, id: u32, loaded_from_disk: bool) -> bool {
        let (kind, at) = self.slot_kind_and_position(slot);
        let kind = kind.into_owned();
        let mut canceled = false;
        let dimension = self.common.dim.id();
        let admission = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.construct_joining(id, &kind);
            // No lifecycle epoch means no mod is loaded to hear it.
            if let Some(epoch) = self.common.native_lifecycle.epoch() {
                self.native_entity_join_level(
                    dimension,
                    &kind,
                    [at.x, at.y, at.z],
                    loaded_from_disk,
                    mod_api::EntityRef::new(epoch, id),
                    &mut canceled,
                );
            }
            #[cfg(any(test, feature = "test-support"))]
            {
                use super::pending_births::escaping_fault::{self, At};
                escaping_fault::reach(At::Admission, id);
            }
        }));
        let spawn = self
            .common
            .native_lifecycle
            .pending_item_spawns
            .iter()
            .position(|entry| entry.0 == id)
            .map(|index| {
                self.common
                    .native_lifecycle
                    .pending_item_spawns
                    .remove(index)
            });
        if canceled || admission.is_err() {
            // A listener may have written other rows before unwinding. Retire this
            // admission only, and do not use an index that may now name another row.
            if let Some(slot) = self.refresh_join_slot(id, slot) {
                let retained = admission.is_ok()
                    && matches!(slot, Slot::Walk(super::entity::EntityHandle::Mob(index))
                        if self.detach_natural_candidate(id, index));
                if !retained {
                    self.discard_slot(slot, id);
                }
            }
            self.forget_entity(id);
            if let Err(payload) = admission {
                std::panic::resume_unwind(payload);
            }
            self.common.persist.joins_canceled += 1;
            return false;
        }
        if self.refresh_join_slot(id, slot).is_some() {
            // A teleported candidate already went through admission in its destination.
            self.remember_join(id, kind);
            // `addEntity` files it under its section, untold.
            self.entity_section_joined(id, [at.x, at.y, at.z]);
            if let Some((id, item, count, position)) = spawn {
                self.native_item_spawn(id, &item, count, position);
            }
        }
        true
    }

    /// `Entity.<init>` comes before `addFreshEntity`: an entity that started on a path that
    /// doesn't tell it (`entity_initialization.rs`) is told here.
    fn construct_joining(&mut self, id: u32, kind: &str) {
        self.initialize_entity(mod_api::Holder::Entity(id), kind);
        // Constructor writes were canonical but unbound before insertion.
        // The real lifetime is now resident, before join listeners query it.
        if self
            .common
            .entity_attachments
            .get(&id)
            .is_some_and(|map| map.has_index_entries())
            && let Err(error) = self.bind_resident_entity_attachments(id)
        {
            log::warn!("constructor attachment binding failed for entity {id}: {error:?}");
        }
    }

    pub fn assign_pending_join_ids(&mut self) {
        self.assign_dragon_and_crystal_ids();
        use super::entity_residency::assign_resident_ids;
        assign_resident_ids!(self.common.tnts, t, &mut self.server.next_entity, None);
        assign_resident_ids!(
            self.common.falling,
            f,
            &mut self.server.next_entity,
            Some(&f.attachment_lifetime)
        );
        assign_resident_ids!(
            self.common.creatures.projectiles,
            p,
            &mut self.server.next_entity,
            Some(&p.attachment_lifetime)
        );
        assign_resident_ids!(self.common.tridents, t, &mut self.server.next_entity, None);
        assign_resident_ids!(self.common.rockets, r, &mut self.server.next_entity, None);
    }
    /// The entity inserted earliest after `mark` (and, with a `through`, no later than it), but
    /// the `skipped`.
    fn next_unannounced(
        &self,
        mark: sunrise_sim::join_seq::JoinSeq,
        skipped: &[u32],
        through: Option<sunrise_sim::join_seq::JoinSeq>,
    ) -> Option<(Slot, u32, sunrise_sim::join_seq::JoinSeq)> {
        use super::entity::Entity;
        use sunrise_sim::join_seq::{JoinSeq, Joined};
        let mut best: Option<(Slot, u32, JoinSeq)> = None;
        let mut offer = |slot: Slot, id: u32, seq: JoinSeq| {
            if skipped.contains(&id)
                || self
                    .common
                    .persist
                    .removals
                    .iter()
                    .chain(self.common.persist.destroying_vehicles.iter())
                    .any(|target| target_id(*target) == Some(id))
            {
                return;
            }
            if seq > mark
                && through.is_none_or(|through| seq <= through)
                && best.is_none_or(|(_, _, previous)| seq < previous)
                && !self.common.native_lifecycle.admitting_joins.contains(&id)
                && !self.join_was_announced(id)
            {
                best = Some((slot, id, seq));
            }
        };
        for handle in self.entity_handles() {
            let (id, seq) = super::entity::with_entity!(self, handle, e => (e.id(), e.join_seq()));
            offer(Slot::Walk(handle), id, seq);
        }
        for (i, c) in self.common.crystals.iter().enumerate() {
            offer(Slot::Apart(Apart::Crystal(i)), c.id, c.join);
        }
        for (i, c) in self.common.cushions.iter().enumerate() {
            offer(Slot::Apart(Apart::Cushion(i)), c.id, c.join);
        }
        for (i, p) in self.common.paintings.iter().enumerate() {
            offer(Slot::Apart(Apart::Painting(i)), p.id, p.join);
        }
        for (i, f) in self.common.item_frames.iter().enumerate() {
            offer(Slot::Apart(Apart::Frame(i)), f.id, f.join);
        }
        if let Some(d) = self.common.dragon.as_ref() {
            offer(Slot::Apart(Apart::Dragon), d.id, d.join);
        }
        for (i, other) in self
            .server
            .others
            .0
            .iter()
            .enumerate()
            .filter(|(_, e)| !super::general::quarantined_kind(&e.kind))
        {
            offer(Slot::Apart(Apart::Other(i)), other.id, other.join);
        }
        // A toss still being asked (`item_toss`) ends the flush: it and what joined after wait.
        best.filter(|&(_, id, _)| !self.common.native_lifecycle.held_joins.contains(&id))
    }

    fn join_was_announced(&self, id: u32) -> bool {
        self.common
            .persist
            .live
            .binary_search_by_key(&id, |entry| entry.0)
            .is_ok()
            || self
                .common
                .persist
                .admitted_joins
                .iter()
                .any(|&(_, admitted)| admitted == id)
    }

    fn remember_join(&mut self, id: u32, kind: String) {
        let index = self
            .common
            .persist
            .live
            .binary_search_by_key(&id, |entry| entry.0)
            .unwrap_or_else(|i| i);
        self.common.persist.live.insert(index, (id, kind, true));
    }
    /// The type id and position a mod is told of.
    fn slot_kind_and_position(&self, slot: Slot) -> (std::borrow::Cow<'_, str>, Vec3) {
        use super::entity::Entity;
        let block = |p: [i32; 3]| {
            Vec3::new(
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            )
        };
        match slot {
            Slot::Walk(h) => {
                super::entity::with_entity!(self, h, e => (e.native_type(), e.position()))
            }
            Slot::Apart(Apart::Crystal(i)) => (
                "minecraft:end_crystal".into(),
                self.common.crystals[i].position,
            ),
            Slot::Apart(Apart::Cushion(i)) => {
                ("minecraft:cushion".into(), self.common.cushions[i].position)
            }
            Slot::Apart(Apart::Painting(i)) => (
                "minecraft:painting".into(),
                block(self.common.paintings[i].pos),
            ),
            Slot::Apart(Apart::Frame(i)) => {
                let f = &self.common.item_frames[i];
                let kind = if f.glow {
                    "minecraft:glow_item_frame"
                } else {
                    "minecraft:item_frame"
                };
                (kind.into(), block(f.pos))
            }
            Slot::Apart(Apart::Other(i)) => (
                self.server.others.0[i].kind.as_str().into(),
                self.server.others.0[i].position,
            ),
            Slot::Apart(Apart::Dragon) => (
                "minecraft:ender_dragon".into(),
                self.common
                    .dragon
                    .as_ref()
                    .map_or(Vec3::ZERO, |d| d.position),
            ),
        }
    }

    fn refresh_join_slot(&self, id: u32, slot: Slot) -> Option<Slot> {
        let apart = match slot {
            Slot::Walk(handle) => return self.refresh_entity_handle(id, handle).map(Slot::Walk),
            Slot::Apart(Apart::Crystal(_)) => self
                .common
                .crystals
                .iter()
                .position(|e| e.id == id)
                .map(Apart::Crystal),
            Slot::Apart(Apart::Cushion(_)) => self
                .common
                .cushions
                .iter()
                .position(|e| e.id == id)
                .map(Apart::Cushion),
            Slot::Apart(Apart::Painting(_)) => self
                .common
                .paintings
                .iter()
                .position(|e| e.id == id)
                .map(Apart::Painting),
            Slot::Apart(Apart::Frame(_)) => self
                .common
                .item_frames
                .iter()
                .position(|e| e.id == id)
                .map(Apart::Frame),
            Slot::Apart(Apart::Other(_)) => self
                .server
                .others
                .0
                .iter()
                .position(|e| e.id == id)
                .map(Apart::Other),
            Slot::Apart(Apart::Dragon) => self
                .common
                .dragon
                .as_ref()
                .filter(|e| e.id == id)
                .map(|_| Apart::Dragon),
        };
        apart.map(Slot::Apart)
    }
    /// Takes the entity out of the level, before it ever ticked.
    fn discard_slot(&mut self, slot: Slot, id: u32) {
        match slot {
            // A listener can cause another join or a removal; resolve by id
            // rather than discarding whatever now occupies the old index.
            Slot::Walk(hint) => {
                if let Some(handle) = self.refresh_entity_handle(id, hint) {
                    self.discard_entity(handle);
                }
            }
            Slot::Apart(Apart::Crystal(i)) => drop(self.common.crystals.remove(i)),
            Slot::Apart(Apart::Cushion(i)) => drop(self.common.cushions.remove(i)),
            Slot::Apart(Apart::Painting(i)) => drop(self.common.paintings.remove(i)),
            Slot::Apart(Apart::Frame(i)) => drop(self.common.item_frames.remove(i)),
            Slot::Apart(Apart::Dragon) => self.discard_resident_dragon(),
            Slot::Apart(Apart::Other(i)) => drop(self.server.others.0.remove(i)),
        }
    }
    /// Entities that were moved, not added (a portal's arrival keeps being
    /// silent): everything now in the level counts as announced, and will
    /// be told when it leaves.
    pub fn native_joins_skip(&mut self) {
        let observed = sunrise_sim::join_seq::JoinSeq::latest();
        if self.common.is_logical_server() && !mod_api::__private::mods().is_empty() {
            self.assign_pending_join_ids();
            while let Some((slot, id, seq)) =
                self.next_unannounced(self.common.persist.joined_seq, &[], None)
            {
                self.common.persist.joined_seq = seq;
                let kind = self.slot_kind_and_position(slot).0.into_owned();
                self.remember_join(id, kind);
            }
        }
        self.common.persist.joined_seq = self.common.persist.joined_seq.max(observed);
        self.common
            .persist
            .admitted_joins
            .retain(|&(seq, _)| seq > self.common.persist.joined_seq);
    }
    /// An entity taken out of the level without leaving it (an item through
    /// an End gateway): forgotten, not told of.
    pub(in crate::game::server_side) fn native_leave_skip(&mut self, id: u32) {
        self.entity_section_forget(id);
        if let Ok(i) = self.common.persist.live.binary_search_by_key(&id, |l| l.0) {
            self.common.persist.live.remove(i);
        }
    }
    /// Entity `id` has left the level: told once, if mods were told it joined.
    pub fn native_entity_left(&mut self, id: u32) {
        if self.common.hold_announcement(
            sunrise_gameplay_common::game::transactions::entities::Announcement::Left(id),
        ) {
            return;
        }
        self.entity_left_now(id);
    }
    /// [`Self::native_entity_left`] told now, though a transaction or a commit holds the rest:
    /// the root commit's own telling.
    pub(in crate::game::server_side) fn entity_left_now(&mut self, id: u32) {
        self.entity_section_forget(id);
        if let Ok(i) = self.common.persist.live.binary_search_by_key(&id, |l| l.0) {
            let (id, kind, _) = self.common.persist.live.remove(i);
            let dimension = self.common.dim.id();
            self.native_entity_leave_level(dimension, id, &kind);
        }
    }
    /// Tells `native_entity_leave_level` for every announced entity that is
    /// no longer in any list (died, despawned, picked up, burnt, blown up, gone
    /// through a portal, unloaded), lowest id first. Marks the entities still
    /// there in the registry and tells the rest: no allocation, and nothing
    /// runs when no mod is loaded or none was announced.
    pub fn native_leaves_flush(&mut self) {
        self.native_removals_flush();
        if self.common.persist.live.is_empty()
            || self.common.hold_announcement(
                sunrise_gameplay_common::game::transactions::entities::Announcement::Leaves,
            )
        {
            return;
        }
        self.leaves_now();
    }
    /// [`Self::native_leaves_flush`] past its guards and told now, though a transaction or a
    /// commit holds the rest: the root commit's own telling.
    pub(in crate::game::server_side) fn leaves_now(&mut self) {
        use super::entity::Entity;
        for l in &mut self.common.persist.live {
            l.2 = false;
        }
        let live = &mut self.common.persist.live;
        let mut seen = |id: u32| {
            if let Ok(i) = live.binary_search_by_key(&id, |l| l.0) {
                live[i].2 = true;
            }
        };
        super::entity::for_each_entity!(super::entity::entity_lists_of!(self.common), e => {
            seen(e.id());
        });
        self.common.crystals.iter().for_each(|c| seen(c.id));
        self.common.cushions.iter().for_each(|c| seen(c.id));
        self.common.paintings.iter().for_each(|p| seen(p.id));
        self.common.item_frames.iter().for_each(|f| seen(f.id));
        self.server.others.0.iter().for_each(|e| seen(e.id));
        if let Some(d) = self.common.dragon.as_ref() {
            seen(d.id);
        }
        while let Some(i) = self.common.persist.live.iter().position(|l| !l.2) {
            let (id, kind, _) = self.common.persist.live.remove(i);
            self.entity_section_forget(id);
            let dimension = self.common.dim.id();
            self.native_entity_leave_level(dimension, id, &kind);
        }
    }
}

use sunrise_gameplay_common::game::entity_initialization_common::EntityInitializationCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

    // NBT loads assign ids; reinstall an unbound payload as a pending owner.

    #[test]
    fn a_passenger_without_a_position_stands_where_the_stack_is_summoned() {
        let mut rider = Compound::new();
        rider.insert("id".into(), Tag::String("minecraft:pig".into()));
        let mut root = Compound::new();
        root.insert("id".into(), Tag::String("minecraft:pig".into()));
        root.insert("Passengers".into(), Tag::List(vec![Tag::Compound(rider)]));
        let mut tag = Tag::Compound(root);
        place_summoned(&mut tag, [1.5, 70.0, -2.5]);
        let pos = Tag::List(
            [1.5, 70.0, -2.5]
                .map(|v: f64| Tag::Double(v.to_bits()))
                .to_vec(),
        );
        let Tag::Compound(root) = &tag else { panic!() };
        assert_eq!(root.get(&"Pos".into()), Some(&pos));
        let Some(Tag::List(riders)) = root.get(&"Passengers".into()) else {
            panic!("no passengers");
        };
        let Tag::Compound(rider) = &riders[0] else {
            panic!()
        };
        assert_eq!(rider.get(&"Pos".into()), Some(&pos));
    }

    #[test]
    fn an_orb_root_has_an_id_only_while_it_is_in_the_level() {
        // Spawned: the orb holds the first id.
        assert_eq!(root_id(None, 36, |id| id == 36), Some(36));
        // A mod kept it out: id 36 was handed out, but no entity holds it.
        assert_eq!(root_id(None, 36, |_| false), None);
        // A command target answers for itself.
        assert_eq!(root_id(Some(Target::Mob(40)), 36, |_| false), Some(40));
        assert_eq!(root_id(Some(Target::Player), 36, |_| true), None);
    }

    #[test]
    fn summon_positions_are_summons() {
        assert!(summon_position_ok([0.5, 64.0, 0.5]));
        assert!(summon_position_ok([-3.0e7, 64.0, 0.5]));
        assert!(!summon_position_ok([3.0e7, 64.0, 0.5]));
        assert!(!summon_position_ok([0.5, f64::NAN, 0.5]));
    }

    #[test]
    #[ignore = "daily"]
    fn receiver_summon_refuses_a_tree_with_a_resident_passenger_uuid() {
        use sunrise_core::nbt::parse_snbt;
        use sunrise_level::chunk::Chunk;

        crate::game::native_entity_refs::tests::with_game(|game| {
            game.common
                .world
                .chunks
                .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.common.dim.id()).unwrap();
            let compound = |s| match parse_snbt(s).unwrap().0 {
                Tag::Compound(c) => c,
                _ => panic!("expected a compound"),
            };
            let existing = level
                .summon(
                    game,
                    "minecraft:pig",
                    [1.5, 70.0, 1.5],
                    compound("{UUID:[I;22,33,44,55]}"),
                )
                .unwrap()
                .unwrap();
            let next = game.server.next_entity;
            let tree = compound(
                r#"{UUID:[I;11,22,33,44],Passengers:[{id:"minecraft:pig",UUID:[I;22,33,44,55]}]}"#,
            );
            assert_eq!(
                level.summon(game, "minecraft:pig", [2.5, 70.0, 1.5], tree),
                Ok(None)
            );
            assert_eq!(
                game.common
                    .mobs
                    .iter()
                    .map(|mob| mob.id)
                    .collect::<Vec<_>>(),
                [existing.id()]
            );
            assert_eq!(
                game.server.next_entity, next,
                "not even the root entered construction"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn summon_into_parked_level_uses_level_handle_and_refuses_in_transaction() {
        use sunrise_gameplay_common::game::Dim;
        use sunrise_level::chunk::Chunk;

        crate::game::native_entity_refs::tests::with_game(|game| {
            game.common
                .world
                .chunks
                .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
            let overworld = game.level("minecraft:overworld").unwrap();
            let nether = Dim::Nether;
            game.enter_command_level(nether).unwrap();
            game.common
                .world
                .chunks
                .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));

            game.storage_scope(|game, _| {
                assert_eq!(
                    overworld.summon(game, "minecraft:pig", [1.5, 70.0, 1.5], Compound::new()),
                    Err(LevelWriteError::InTransaction)
                );
                ((), false)
            });

            let pig = overworld
                .summon(game, "minecraft:pig", [1.5, 70.0, 1.5], Compound::new())
                .unwrap()
                .expect("summoned in parked overworld");
            assert_eq!(game.common.dim, nether);
            assert!(game.common.mobs.is_empty());
            assert!(
                game.server
                    .levels
                    .0
                    .parked_mobs(Dim::Overworld)
                    .unwrap()
                    .iter()
                    .any(|mob| mob.id == pig.id())
            );

            let stale_epoch = mod_api::WorldEpoch::new(
                game.common
                    .native_lifecycle
                    .epoch()
                    .unwrap()
                    .get()
                    .wrapping_add(1),
            );
            let stale = LevelRef::new(stale_epoch, mod_api::DimensionKey::OVERWORLD);
            assert_eq!(
                stale.summon(game, "minecraft:pig", [1.5, 70.0, 1.5], Compound::new()),
                Err(LevelWriteError::Stale)
            );

            game.enter_command_level(Dim::Overworld).unwrap();
            assert_eq!(game.entity(pig.id()), Some(pig));
        });
    }
}
