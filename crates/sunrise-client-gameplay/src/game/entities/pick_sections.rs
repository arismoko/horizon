//! The client section index shared by spatial queries and crosshair picking.
use std::collections::BTreeMap;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::entity::Entity as _;
use sunrise_gameplay_common::game::general::general_type_catalog;
use sunrise_gameplay_common::game::pick_entity_common::{aabb_extents, knot_bounds};
use sunrise_level::fast_hash::FxHashMap;
use sunrise_sim::join_seq::{
    JoinSeq, Joined as _,
    residency::table::{SectionKey, SectionSearch},
};

pub(in crate::game) use sunrise_gameplay_common::game::pick_entity_common::PickKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::game) struct PickMember {
    pub kind: PickKind,
    pub id: u32,
}

fn hanging_center((lo, hi): ([f64; 3], [f64; 3])) -> [f64; 3] {
    [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        (lo[2] + hi[2]) * 0.5,
    ]
}

fn knot_position(block: [i32; 3]) -> [f64; 3] {
    let p = sunrise_sim::mobs::leash::knot_state(block).position;
    [p.x, p.y, p.z]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PickFingerprint {
    epoch: Option<mod_api::WorldEpoch>,
    dim: Option<Dim>,
    is_remote: bool,
    mobs: (usize, Option<JoinSeq>),
    boats: (usize, Option<JoinSeq>),
    carts: (usize, Option<JoinSeq>),
    armor_stands: (usize, Option<JoinSeq>),
    tnts: (usize, Option<JoinSeq>),
    falling: (usize, Option<JoinSeq>),
    projectiles: (usize, Option<JoinSeq>),
    tridents: (usize, Option<JoinSeq>),
    rockets: (usize, Option<JoinSeq>),
    general: (usize, Option<JoinSeq>),
    paintings: (usize, Option<JoinSeq>),
    item_frames: (usize, Option<JoinSeq>),
    cushions: (usize, Option<JoinSeq>),
    interactions: usize,
    remote_general: (usize, Option<JoinSeq>),
    remote_knots: usize,
    local_knots: (usize, Option<JoinSeq>),
    crystals: (usize, Option<JoinSeq>),
    displays: (usize, Option<JoinSeq>),
    items: (usize, Option<JoinSeq>),
    pending_items: usize,
    orbs: (usize, Option<JoinSeq>),
    player: Option<(u32, JoinSeq)>,
    dragon: Option<(u32, JoinSeq)>,
}

impl PickFingerprint {
    fn same_level(self, other: Self) -> bool {
        self.epoch == other.epoch && self.dim == other.dim && self.is_remote == other.is_remote
    }

    #[cfg(feature = "client")]
    fn of(game: &crate::game::ClientGame<'_>) -> Self {
        Self {
            epoch: game.native_lifecycle.epoch(),
            dim: Some(game.dim),
            is_remote: game.client.remote.is_some(),
            mobs: (game.mobs.len(), game.mobs.last().map(|e| e.join_seq())),
            boats: (game.boats.len(), game.boats.last().map(|e| e.join_seq())),
            carts: (game.carts.len(), game.carts.last().map(|e| e.join_seq())),
            armor_stands: (
                game.armor_stands.len(),
                game.armor_stands.last().map(|e| e.join_seq()),
            ),
            tnts: (game.tnts.len(), game.tnts.last().map(|e| e.join_seq())),
            falling: (
                game.falling.len(),
                game.falling.last().map(|e| e.join_seq()),
            ),
            projectiles: (
                game.creatures.projectiles.len(),
                game.creatures.projectiles.last().map(|e| e.join_seq()),
            ),
            tridents: (
                game.tridents.len(),
                game.tridents.last().map(|e| e.join_seq()),
            ),
            rockets: (
                game.rockets.len(),
                game.rockets.last().map(|e| e.join_seq()),
            ),
            general: (
                game.resident_entities.general.rows.len(),
                game.resident_entities
                    .general
                    .rows
                    .last()
                    .map(|e| e.join_seq()),
            ),
            paintings: (game.paintings.len(), game.paintings.last().map(|e| e.join)),
            item_frames: (
                game.item_frames.len(),
                game.item_frames.last().map(|e| e.join),
            ),
            cushions: (game.cushions.len(), game.cushions.last().map(|e| e.join)),
            interactions: game.client.interactions.len(),
            remote_general: game.client.remote.as_ref().map_or((0, None), |r| {
                (
                    r.general.rows.len(),
                    r.general.rows.values().next_back().map(|e| e.join),
                )
            }),
            remote_knots: game.client.remote_knots.len(),
            local_knots: (
                game.resident_entities.knots.len(),
                game.resident_entities.knots.last().map(|e| e.join),
            ),
            crystals: (game.crystals.len(), game.crystals.last().map(|e| e.join)),
            displays: (
                game.client.text_displays.0.len(),
                game.client.text_displays.0.last().map(|(_, d)| d.join),
            ),
            items: (
                game.dropped.len(),
                game.dropped.last().map(|e| e.join_seq()),
            ),
            pending_items: game
                .client
                .remote
                .as_ref()
                .map_or(0, |r| r.pending_items.len()),
            orbs: (game.orbs.len(), game.orbs.last().map(|e| e.join_seq())),
            player: game.client_section_player().map(|p| {
                (
                    p.entity_id as u32,
                    game.resident_entities.local_player.join(),
                )
            }),
            dragon: game.dragon.as_ref().map(|d| (d.id, d.join)),
        }
    }
}

/// Where a crossing falls in the client's tick: vanilla's `ClientLevel.tickEntities` moves each
/// entity at its own turn, in the order the level admitted them, so two entities entering one
/// section in the same tick arrive in that order whichever Sunrise loop moved them first.
/// Outside the entity ticks (packets, spawns) arrivals keep the order they happen in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival {
    tick: u64,
    /// 1 inside the entity ticks, 2 after them (packets and spawns until the next tick).
    phase: u8,
    /// The admission order inside the entity ticks, otherwise the running count.
    order: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    member: PickMember,
    arrival: Arrival,
}

/// Sparse section map matching `EntitySectionStorage`: keys are packed `SectionPos` values and
/// each section keeps its entities in the order they arrived, across spawns and crossings.
#[derive(Default)]
pub(in crate::game) struct PickSectionStorage {
    sections: BTreeMap<SectionKey, Vec<Slot>>,
    /// Each member's kind, section and admission order.
    by_id: FxHashMap<u32, (PickKind, SectionKey, u64)>,
    initialized: bool,
    fingerprint: PickFingerprint,
    scratch_removed: Vec<u32>,
    scratch_added: Vec<((u64, u32), PickMember, [f64; 3])>,
    tick: u64,
    in_entity_ticks: bool,
    arrivals: u64,
}

impl PickSectionStorage {
    pub(in crate::game::client_side) fn clear(&mut self) {
        self.sections.clear();
        self.by_id.clear();
        self.initialized = false;
    }

    /// `ClientLevel.tickEntities` starts: crossings from here take the movers' admission order.
    pub(in crate::game::client_side) fn begin_entity_ticks(&mut self) {
        self.tick += 1;
        self.in_entity_ticks = true;
    }

    /// The entity ticks are over: later arrivals follow every crossing of this tick.
    pub(in crate::game::client_side) fn end_entity_ticks(&mut self) {
        self.in_entity_ticks = false;
    }

    fn arrival(&mut self, seq: u64) -> Arrival {
        if self.in_entity_ticks {
            Arrival {
                tick: self.tick,
                phase: 1,
                order: seq,
            }
        } else {
            self.arrivals += 1;
            Arrival {
                tick: self.tick,
                phase: 2,
                order: self.arrivals,
            }
        }
    }

    fn insert(&mut self, key: SectionKey, member: PickMember, seq: u64) {
        let arrival = self.arrival(seq);
        let bucket = self.sections.entry(key).or_default();
        // Almost always the end: only a same-tick crossing of an earlier-admitted entity
        // steps back over the later-admitted ones that crossed before it here.
        let at = bucket
            .iter()
            .rposition(|slot| slot.arrival <= arrival)
            .map_or(0, |i| i + 1);
        bucket.insert(at, Slot { member, arrival });
        self.by_id.insert(member.id, (member.kind, key, seq));
    }

    fn take_out(&mut self, member: PickMember, key: SectionKey) {
        if let Some(bucket) = self.sections.get_mut(&key) {
            if let Some(pos) = bucket.iter().position(|slot| slot.member == member) {
                bucket.remove(pos);
            }
            if bucket.is_empty() {
                self.sections.remove(&key);
            }
        }
    }

    pub(in crate::game::client_side) fn place(
        &mut self,
        member: PickMember,
        at: [f64; 3],
        seq: u64,
    ) {
        self.remove(member.id);
        self.insert(SectionKey::at(at), member, seq);
    }

    pub(in crate::game::client_side) fn move_id(&mut self, id: u32, to: [f64; 3]) {
        let Some(&(kind, old_key, seq)) = self.by_id.get(&id) else {
            return;
        };
        let new_key = SectionKey::at(to);
        if old_key == new_key {
            return;
        }
        let member = PickMember { kind, id };
        self.take_out(member, old_key);
        self.insert(new_key, member, seq);
    }

    pub(in crate::game::client_side) fn remove(&mut self, id: u32) {
        let Some((kind, old_key, _)) = self.by_id.remove(&id) else {
            return;
        };
        self.take_out(PickMember { kind, id }, old_key);
    }

    pub(in crate::game::client_side) fn kind_of(&self, id: u32) -> Option<PickKind> {
        self.by_id.get(&id).map(|&(kind, _, _)| kind)
    }

    #[cfg(feature = "client")]
    fn note_fingerprint(&mut self, game: &crate::game::ClientGame<'_>) {
        let fp = PickFingerprint::of(game);
        if !self.fingerprint.same_level(fp) {
            self.clear();
        }
        self.fingerprint = fp;
    }

    /// Reconciles only when the world context or container counts changed outside incremental hooks.
    #[cfg(feature = "client")]
    fn sync_with(&mut self, game: &crate::game::ClientGame<'_>) {
        let fp = PickFingerprint::of(game);
        if self.initialized && self.fingerprint == fp {
            return;
        }
        // TransientEntitySectionManager.Callback.onMove already moved entities into
        // section-entry order, even if the client has not queried the level yet.
        if !self.fingerprint.same_level(fp) {
            self.clear();
        } else {
            self.scratch_removed.clear();
            for (&id, &(kind, _, _)) in &self.by_id {
                if game.pick_member_position_of_kind(kind, id).is_none()
                    && !game.side_entity_is_taken(id)
                {
                    self.scratch_removed.push(id);
                }
            }
            for i in 0..self.scratch_removed.len() {
                let id = self.scratch_removed[i];
                self.remove(id);
            }
        }

        self.scratch_added.clear();
        let by_id = &self.by_id;
        let added = &mut self.scratch_added;
        let mut consider = |seq: u64, kind: PickKind, id: u32, pos: [f64; 3]| {
            if !by_id.contains_key(&id) {
                added.push(((seq, id), PickMember { kind, id }, pos));
            }
        };
        visit_section_members(game, &mut consider);

        if self.scratch_added.len() > 1 {
            self.scratch_added.sort_by_key(|(key, _, _)| *key);
        }
        for i in 0..self.scratch_added.len() {
            let ((seq, _), member, pos) = self.scratch_added[i];
            self.place(member, pos, seq);
        }
        self.scratch_added.clear();
        self.fingerprint = fp;
        self.initialized = true;
    }

    /// `EntitySectionStorage.forEachAccessibleNonEmptySection(bb, consumer)`.
    fn for_each_in_bounds(&self, search: Aabb, mut visit: impl FnMut(PickMember)) {
        if !search
            .min_position()
            .components()
            .into_iter()
            .chain(search.max_position().components())
            .all(f64::is_finite)
        {
            return;
        }
        let search = SectionSearch::new(search);
        for (_, slots) in search.sections(&self.sections, None) {
            for slot in slots {
                visit(slot.member);
            }
        }
    }

    fn candidate_after(
        &self,
        search: SectionSearch,
        after: &mut Option<(SectionKey, usize)>,
    ) -> Option<PickMember> {
        if let Some((key, offset)) = *after
            && let Some(slot) = self
                .sections
                .get(&key)
                .and_then(|slots| slots.get(offset + 1))
        {
            *after = Some((key, offset + 1));
            return Some(slot.member);
        }
        let (&key, slots) = search
            .sections(&self.sections, after.map(|(key, _)| key))
            .next()?;
        *after = Some((key, 0));
        slots.first().map(|slot| slot.member)
    }
}

/// Every entity the section index files, with its admission order, kind, id and position, family
/// by family.
#[cfg(feature = "client")]
pub(in crate::game) fn visit_section_members(
    game: &crate::game::ClientGame<'_>,
    consider: &mut impl FnMut(u64, PickKind, u32, [f64; 3]),
) {
    for e in game
        .mobs
        .iter()
        .filter(|e| game.is_logical_server() || game.dragon.as_ref().is_none_or(|d| d.id != e.id))
    {
        let p = e.position();
        consider(e.join_seq().get(), PickKind::Mob, e.id(), [p.x, p.y, p.z]);
    }
    for e in game.boats.iter() {
        let p = e.position();
        consider(e.join_seq().get(), PickKind::Boat, e.id(), [p.x, p.y, p.z]);
    }
    for e in game.carts.iter() {
        let p = e.position();
        consider(e.join_seq().get(), PickKind::Cart, e.id(), [p.x, p.y, p.z]);
    }
    for e in game.armor_stands.iter() {
        let p = e.position();
        consider(
            e.join_seq().get(),
            PickKind::ArmorStand,
            e.id(),
            [p.x, p.y, p.z],
        );
    }
    for e in game.tnts.iter() {
        let p = e.position();
        consider(e.join_seq().get(), PickKind::Tnt, e.id(), [p.x, p.y, p.z]);
    }
    for e in game.falling.iter() {
        let p = e.position();
        consider(
            e.join_seq().get(),
            PickKind::Falling,
            e.id(),
            [p.x, p.y, p.z],
        );
    }
    for e in game.creatures.projectiles.iter() {
        let p = e.position();
        consider(
            e.join_seq().get(),
            PickKind::Projectile,
            e.id(),
            [p.x, p.y, p.z],
        );
    }
    for e in game.tridents.iter() {
        let p = e.position();
        consider(
            e.join_seq().get(),
            PickKind::Trident,
            e.id(),
            [p.x, p.y, p.z],
        );
    }
    for e in game.rockets.iter() {
        let p = e.position();
        consider(
            e.join_seq().get(),
            PickKind::Rocket,
            e.id(),
            [p.x, p.y, p.z],
        );
    }
    for e in game.resident_entities.general.rows.iter() {
        let p = e.position();
        consider(
            e.join_seq().get(),
            PickKind::General,
            e.id(),
            [p.x, p.y, p.z],
        );
    }
    for p in game.paintings.iter() {
        consider(
            p.join.get(),
            PickKind::Painting,
            p.id,
            hanging_center(p.bounds()),
        );
    }
    for f in game.item_frames.iter() {
        consider(
            f.join.get(),
            PickKind::ItemFrame,
            f.id,
            hanging_center(f.bounds()),
        );
    }
    for c in game.cushions.iter() {
        consider(
            c.join.get(),
            PickKind::Cushion,
            c.id,
            [c.position.x, c.position.y, c.position.z],
        );
    }
    for (&id, i) in &game.client.interactions {
        consider(
            i.join.get(),
            PickKind::Interaction,
            id as u32,
            [i.position.x, i.position.y, i.position.z],
        );
    }
    if let Some(remote) = &game.client.remote {
        for (&id, row) in &remote.general.rows {
            consider(
                row.join.get(),
                PickKind::RemoteGeneral,
                id as u32,
                row.position,
            );
        }
    }
    for (&id, &block) in &game.client.remote_knots {
        consider(u64::MAX, PickKind::RemoteKnot, id, knot_position(block));
    }
    for k in game.resident_entities.knots.iter() {
        consider(
            k.join.get(),
            PickKind::LocalKnot,
            k.id,
            knot_position(k.fence),
        );
    }
    for c in game.crystals.iter().filter(|c| c.id != 0) {
        consider(
            c.join.get(),
            PickKind::Crystal,
            c.id,
            [c.position.x, c.position.y, c.position.z],
        );
    }

    if let Some(remote) = &game.client.remote {
        for (&id, d) in &game.client.text_displays.0 {
            consider(
                d.join.get(),
                PickKind::RemoteDisplay,
                id as u32,
                d.position().to_array(),
            );
        }
        for e in game.dropped.iter() {
            consider(
                e.join_seq().get(),
                PickKind::Item,
                e.id(),
                e.position().components(),
            );
        }
        for (&id, pending) in &remote.pending_items {
            consider(
                pending.join.get(),
                PickKind::Item,
                id as u32,
                pending.position,
            );
        }
        for e in game.orbs.iter() {
            consider(
                e.join_seq().get(),
                PickKind::Orb,
                e.id(),
                e.position().components(),
            );
        }
        if let Some(p) = game.client_section_player() {
            consider(
                game.resident_entities.local_player.join().get(),
                PickKind::LocalPlayer,
                p.entity_id as u32,
                p.position.components(),
            );
        }
        if let Some(d) = game.dragon.as_ref().filter(|d| d.id != 0) {
            consider(
                d.join.get(),
                PickKind::Dragon,
                d.id,
                d.position.components(),
            );
        }
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Take one candidate per borrow so match checks and nested native reads can use the host.
    pub(in crate::game) fn client_section_candidates(
        &self,
        search: SectionSearch,
    ) -> impl Iterator<Item = PickMember> + '_ {
        self.client
            .client_pick
            .sections
            .borrow_mut()
            .sync_with(self);
        let mut after = None;
        std::iter::from_fn(move || {
            self.client
                .client_pick
                .sections
                .borrow()
                .candidate_after(search, &mut after)
        })
    }

    fn client_section_player(&self) -> Option<&sunrise_sim::motion::PlayerMotion> {
        let remote = self.client.remote.as_ref()?;
        self.common
            .player
            .as_ref()
            .filter(|p| p.entity_id == remote.entity_id)
    }

    /// Files or replaces entity `id` at its current section in entry order.
    pub fn pick_section_sync_id(&mut self, id: u32) {
        if let Some((kind, pos)) = self.locate_pick_member(id) {
            let seq = self.pick_member_seq(kind, id);
            let mut storage = self.client.client_pick.sections.borrow_mut();
            storage.note_fingerprint(self);
            storage.place(PickMember { kind, id }, pos, seq);
        }
    }

    /// Updates entity `id`'s section after a position write; no-op within the same section.
    pub(in crate::game) fn pick_section_move(&self, id: u32, to: [f64; 3]) {
        self.client
            .client_pick
            .sections
            .borrow_mut()
            .move_id(id, to);
    }

    /// Updates entity `id`'s section from its live position without scanning unrelated families.
    pub(in crate::game) fn pick_section_sync_moved_id(&mut self, id: u32) {
        let kind = self.client.client_pick.sections.borrow().kind_of(id);
        let pos = match kind {
            Some(kind) => self.pick_member_position_of_kind(kind, id),
            None => self.locate_pick_member(id).map(|(_, pos)| pos),
        };
        if let Some(pos) = pos {
            self.client
                .client_pick
                .sections
                .borrow_mut()
                .move_id(id, pos);
        }
    }

    /// Removes entity `id` from section storage immediately on discard/removal.
    pub(in crate::game) fn pick_section_remove(&mut self, id: u32) {
        let mut storage = self.client.client_pick.sections.borrow_mut();
        storage.note_fingerprint(self);
        storage.remove(id);
    }

    /// Clears section storage when changing or resetting a level.
    pub(in crate::game::client_side) fn pick_section_clear(&mut self) {
        self.client.client_pick.sections.borrow_mut().clear();
    }

    /// Updates sections for seated passengers after `position_riders` moves them.
    pub(in crate::game::client_side) fn sync_seated_pick_sections(&mut self) {
        let mut storage = self.client.client_pick.sections.borrow_mut();
        if !storage.initialized && storage.by_id.is_empty() {
            return;
        }
        for m in self.common.mobs.iter() {
            if m.extra.mount.is_some() {
                let p = m.position();
                storage.move_id(m.id(), [p.x, p.y, p.z]);
            }
        }
        for a in self.common.armor_stands.iter() {
            if a.vehicle.is_some() {
                let p = a.position();
                storage.move_id(a.id(), [p.x, p.y, p.z]);
            }
        }
        for (&id, i) in &self.client.interactions {
            if i.vehicle.is_some() {
                storage.move_id(id as u32, [i.position.x, i.position.y, i.position.z]);
            }
        }
        if let Some(remote) = &self.client.remote {
            for (&id, row) in &remote.general.rows {
                if remote.passenger_vehicle(id as u32).is_some() {
                    storage.move_id(id as u32, row.position);
                }
            }
        }
    }

    fn locate_pick_member(&self, id: u32) -> Option<(PickKind, [f64; 3])> {
        if let Some(remote) = &self.client.remote {
            let kind = match remote.tracked(id as i32).map(|(kind, _)| kind) {
                _ if remote.entity_id as u32 == id => Some(PickKind::LocalPlayer),
                Some("minecraft:ender_dragon") => Some(PickKind::Dragon),
                Some("minecraft:item") => Some(PickKind::Item),
                Some("minecraft:experience_orb") => Some(PickKind::Orb),
                Some(kind) if crate::game::text_displays::Shows::from_kind(kind).is_some() => {
                    Some(PickKind::RemoteDisplay)
                }
                _ => None,
            };
            if let Some(kind) = kind {
                return Some((kind, self.pick_member_position_of_kind(kind, id)?));
            }
        }
        if let Some(i) = self.common.mob_index.find_ref(&self.common.mobs, id) {
            let p = self.common.mobs[i].position;
            return Some((PickKind::Mob, [p.x, p.y, p.z]));
        }
        if let Some(b) = self.common.boats.iter().find(|e| e.id() == id) {
            let p = b.position();
            return Some((PickKind::Boat, [p.x, p.y, p.z]));
        }
        if let Some(c) = self.common.carts.iter().find(|e| e.id() == id) {
            let p = c.position();
            return Some((PickKind::Cart, [p.x, p.y, p.z]));
        }
        if let Some(a) = self.common.armor_stands.iter().find(|e| e.id() == id) {
            let p = a.position();
            return Some((PickKind::ArmorStand, [p.x, p.y, p.z]));
        }
        if let Some(t) = self.common.tnts.iter().find(|e| e.id() == id) {
            let p = t.position();
            return Some((PickKind::Tnt, [p.x, p.y, p.z]));
        }
        if let Some(f) = self.common.falling.iter().find(|e| e.id() == id) {
            let p = f.position();
            return Some((PickKind::Falling, [p.x, p.y, p.z]));
        }
        if let Some(p) = self
            .common
            .creatures
            .projectiles
            .iter()
            .find(|e| e.id() == id)
        {
            let pos = p.position();
            return Some((PickKind::Projectile, [pos.x, pos.y, pos.z]));
        }
        if let Some(t) = self.common.tridents.iter().find(|e| e.id() == id) {
            let p = t.position();
            return Some((PickKind::Trident, [p.x, p.y, p.z]));
        }
        if let Some(r) = self.common.rockets.iter().find(|e| e.id() == id) {
            let p = r.position();
            return Some((PickKind::Rocket, [p.x, p.y, p.z]));
        }
        if let Some(g) = self.common.resident_entities.general.row(id) {
            let p = g.position();
            return Some((PickKind::General, [p.x, p.y, p.z]));
        }
        if let Some(p) = self.common.paintings.iter().find(|e| e.id == id) {
            return Some((PickKind::Painting, hanging_center(p.bounds())));
        }
        if let Some(f) = self.common.item_frames.iter().find(|e| e.id == id) {
            return Some((PickKind::ItemFrame, hanging_center(f.bounds())));
        }
        if let Some(c) = self.common.cushions.iter().find(|e| e.id == id) {
            return Some((
                PickKind::Cushion,
                [c.position.x, c.position.y, c.position.z],
            ));
        }
        if let Some(i) = self.client.interactions.get(&(id as i32)) {
            return Some((
                PickKind::Interaction,
                [i.position.x, i.position.y, i.position.z],
            ));
        }
        if let Some(row) = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.general.rows.get(&(id as i32)))
        {
            return Some((PickKind::RemoteGeneral, row.position));
        }
        if let Some(&block) = self.client.remote_knots.get(&id) {
            return Some((PickKind::RemoteKnot, knot_position(block)));
        }
        if let Some(k) = self
            .common
            .resident_entities
            .knots
            .iter()
            .find(|k| k.id == id)
        {
            return Some((PickKind::LocalKnot, knot_position(k.fence)));
        }
        if let Some(c) = self.common.crystals.iter().find(|c| c.id == id) {
            return Some((
                PickKind::Crystal,
                [c.position.x, c.position.y, c.position.z],
            ));
        }
        None
    }

    /// The member's admission order, its turn in `ClientLevel.tickEntities` (as `sync_with`
    /// reads it); members without one come last.
    fn pick_member_seq(&self, kind: PickKind, id: u32) -> u64 {
        let id32 = id as i32;
        match kind {
            PickKind::RemoteDisplay => self.client.text_displays.0.get(&id32).map(|d| d.join),
            PickKind::Item => self
                .common
                .dropped
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.join_seq())
                .or_else(|| {
                    self.client
                        .remote
                        .as_ref()?
                        .pending_items
                        .get(&id32)
                        .map(|p| p.join)
                }),
            PickKind::Orb => self
                .common
                .orbs
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.join_seq()),
            PickKind::LocalPlayer => Some(self.common.resident_entities.local_player.join()),
            PickKind::Dragon => self.common.dragon.as_ref().map(|d| d.join),
            PickKind::Mob => self
                .common
                .mob_index
                .find_ref(&self.common.mobs, id)
                .map(|i| self.common.mobs[i].join_seq()),
            PickKind::Boat => self
                .common
                .boats
                .iter()
                .find(|e| e.id() == id)
                .map(|e| e.join_seq()),
            PickKind::Cart => self
                .common
                .carts
                .iter()
                .find(|e| e.id() == id)
                .map(|e| e.join_seq()),
            PickKind::ArmorStand => self
                .common
                .armor_stands
                .iter()
                .find(|e| e.id() == id)
                .map(|e| e.join_seq()),
            PickKind::Tnt => self
                .common
                .tnts
                .iter()
                .find(|e| e.id() == id)
                .map(|e| e.join_seq()),
            PickKind::Falling => self
                .common
                .falling
                .iter()
                .find(|e| e.id() == id)
                .map(|e| e.join_seq()),
            PickKind::Projectile => self
                .common
                .creatures
                .projectiles
                .iter()
                .find(|e| e.id() == id)
                .map(|e| e.join_seq()),
            PickKind::Trident => self
                .common
                .tridents
                .iter()
                .find(|e| e.id() == id)
                .map(|e| e.join_seq()),
            PickKind::Rocket => self
                .common
                .rockets
                .iter()
                .find(|e| e.id() == id)
                .map(|e| e.join_seq()),
            PickKind::General => self
                .common
                .resident_entities
                .general
                .row(id)
                .map(|e| e.join_seq()),
            PickKind::Painting => self
                .common
                .paintings
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.join),
            PickKind::ItemFrame => self
                .common
                .item_frames
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.join),
            PickKind::Cushion => self
                .common
                .cushions
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.join),
            PickKind::Interaction => self.client.interactions.get(&id32).map(|e| e.join),
            PickKind::RemoteGeneral => self
                .client
                .remote
                .as_ref()
                .and_then(|r| r.general.rows.get(&id32).map(|e| e.join)),
            PickKind::RemoteKnot => None,
            PickKind::LocalKnot => self
                .common
                .resident_entities
                .knots
                .iter()
                .find(|k| k.id == id)
                .map(|k| k.join),
            PickKind::Crystal => self
                .common
                .crystals
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.join),
        }
        .map_or(u64::MAX, |seq| seq.get())
    }

    fn pick_member_position_of_kind(&self, kind: PickKind, id: u32) -> Option<[f64; 3]> {
        match kind {
            PickKind::RemoteDisplay => self
                .client
                .text_displays
                .0
                .get(&(id as i32))
                .map(|d| d.position().to_array()),
            PickKind::Item => self
                .common
                .dropped
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.position.components())
                .or_else(|| {
                    self.client
                        .remote
                        .as_ref()?
                        .pending_items
                        .get(&(id as i32))
                        .map(|item| item.position)
                }),
            PickKind::Orb => self
                .common
                .orbs
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.position.components()),
            PickKind::LocalPlayer => self
                .client_section_player()
                .filter(|p| p.entity_id as u32 == id)
                .map(|p| p.position.components()),
            PickKind::Dragon => self
                .common
                .dragon
                .as_ref()
                .filter(|d| d.id == id)
                .map(|d| d.position.components()),
            PickKind::Mob => self
                .common
                .mob_index
                .find_ref(&self.common.mobs, id)
                .map(|i| {
                    let p = self.common.mobs[i].position;
                    [p.x, p.y, p.z]
                }),
            PickKind::Boat => self.common.boats.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::Cart => self.common.carts.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::ArmorStand => {
                self.common
                    .armor_stands
                    .iter()
                    .find(|e| e.id() == id)
                    .map(|e| {
                        let p = e.position();
                        [p.x, p.y, p.z]
                    })
            }
            PickKind::Tnt => self.common.tnts.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::Falling => self.common.falling.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::Projectile => self
                .common
                .creatures
                .projectiles
                .iter()
                .find(|e| e.id() == id)
                .map(|e| {
                    let p = e.position();
                    [p.x, p.y, p.z]
                }),
            PickKind::Trident => self.common.tridents.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::Rocket => self.common.rockets.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::General => self.common.resident_entities.general.row(id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::Painting => self
                .common
                .paintings
                .iter()
                .find(|e| e.id == id)
                .map(|e| hanging_center(e.bounds())),
            PickKind::ItemFrame => self
                .common
                .item_frames
                .iter()
                .find(|e| e.id == id)
                .map(|e| hanging_center(e.bounds())),
            PickKind::Cushion => self
                .common
                .cushions
                .iter()
                .find(|e| e.id == id)
                .map(|e| [e.position.x, e.position.y, e.position.z]),
            PickKind::Interaction => self
                .client
                .interactions
                .get(&(id as i32))
                .map(|e| [e.position.x, e.position.y, e.position.z]),
            PickKind::RemoteGeneral => self
                .client
                .remote
                .as_ref()?
                .general
                .rows
                .get(&(id as i32))
                .map(|e| e.position),
            PickKind::RemoteKnot => self
                .client
                .remote_knots
                .get(&id)
                .copied()
                .map(knot_position),
            PickKind::LocalKnot => self
                .common
                .resident_entities
                .knots
                .iter()
                .find(|k| k.id == id)
                .map(|k| knot_position(k.fence)),
            PickKind::Crystal => self
                .common
                .crystals
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
        }
    }

    /// A member's id, box and pick radius; a client's copy of a non-mob entity takes the box of
    /// its `entity_size` answer.
    fn resolve_pick_member(&self, member: PickMember) -> Option<(u32, Aabb, f64)> {
        let id = member.id;
        match member.kind {
            PickKind::RemoteDisplay
            | PickKind::Item
            | PickKind::Orb
            | PickKind::LocalPlayer
            | PickKind::Dragon => None,
            PickKind::Mob => {
                let m = &self.common.mobs[self.common.mob_index.find_ref(&self.common.mobs, id)?];
                m.is_pickable().then(|| (m.id, m.bounds(), 0.0))
            }
            PickKind::Boat => self.common.boats.iter().find(|e| e.id == id).map(|e| {
                (
                    e.id,
                    self.client_copy_box(e.id, e.position, e.bounds()),
                    0.0,
                )
            }),
            PickKind::Cart => self.common.carts.iter().find(|e| e.id == id).map(|e| {
                (
                    e.id,
                    self.client_copy_box(e.id, e.position, e.bounds()),
                    0.0,
                )
            }),
            PickKind::ArmorStand => self
                .common
                .armor_stands
                .iter()
                .find(|e| e.id == id)
                .filter(|e| e.is_pickable())
                .map(|e| {
                    (
                        e.id,
                        self.client_copy_box(e.id, e.position, e.bounds()),
                        0.0,
                    )
                }),
            PickKind::Tnt => self.common.tnts.iter().find(|e| e.id == id).map(|e| {
                (
                    e.id,
                    self.client_copy_box(e.id, e.position, e.bounds()),
                    0.0,
                )
            }),
            PickKind::Falling => self.common.falling.iter().find(|e| e.id == id).map(|e| {
                (
                    e.id,
                    self.client_copy_box(e.id, e.position, e.bounds()),
                    0.0,
                )
            }),
            PickKind::Projectile => self
                .common
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .filter(|p| self.projectile_is_pickable(p))
                .map(|p| {
                    let bounds = self.client_copy_box(p.id, p.position, p.bounds());
                    (p.id, bounds, self.projectile_pick_radius(p))
                }),
            PickKind::Trident => self
                .common
                .tridents
                .iter()
                .find(|t| t.id == id)
                .filter(|t| self.trident_is_pickable(t))
                .map(|t| {
                    (
                        t.id,
                        self.client_copy_box(t.id, t.position, t.bounds()),
                        1.0,
                    )
                }),
            PickKind::Rocket => self
                .common
                .rockets
                .iter()
                .find(|r| r.id == id)
                .filter(|_| self.rocket_is_pickable())
                .map(|r| {
                    (
                        r.id,
                        self.client_copy_box(r.id, r.position, r.bounds()),
                        1.0,
                    )
                }),
            PickKind::General => self
                .common
                .resident_entities
                .general
                .row(id)
                .filter(|r| r.is_pickable())
                .map(|r| (r.id, r.bounds(), 0.0)),
            PickKind::Painting => self
                .common
                .paintings
                .iter()
                .find(|p| p.id == id)
                .map(|p| (p.id, aabb_extents(p.bounds()), 0.0)),
            PickKind::ItemFrame => self
                .common
                .item_frames
                .iter()
                .find(|f| f.id == id)
                .map(|f| (f.id, aabb_extents(f.bounds()), 0.0)),
            PickKind::Cushion => self
                .common
                .cushions
                .iter()
                .find(|c| c.id == id)
                .map(|c| (c.id, aabb_extents(c.bounds()), 0.0)),
            PickKind::Interaction => self
                .client
                .interactions
                .get(&(id as i32))
                .map(|i| (id, self.client_copy_box(id, i.position, i.bounds()), 0.0)),
            PickKind::RemoteGeneral => self
                .client
                .remote
                .as_ref()?
                .general
                .rows
                .get(&(id as i32))
                .filter(|row| general_type_catalog().pickable(row.type_slot))
                .map(|row| (id, row.bounds(), 0.0)),
            PickKind::RemoteKnot => self
                .client
                .remote_knots
                .get(&id)
                .copied()
                .map(|block| (id, knot_bounds(block), 0.0)),
            PickKind::LocalKnot => self
                .common
                .resident_entities
                .knots
                .iter()
                .find(|k| k.id == id)
                .map(|k| (k.id, knot_bounds(k.fence), 0.0)),
            PickKind::Crystal => self.common.crystals.iter().find(|c| c.id == id).map(|c| {
                (
                    c.id,
                    self.client_copy_box(c.id, c.position, c.bounds()),
                    0.0,
                )
            }),
        }
    }

    fn live_entity_bounds_for_pick(&self, id: u32) -> Option<Aabb> {
        if let Some(i) = self.common.mob_index.find_ref(&self.common.mobs, id) {
            return Some(self.common.mobs[i].bounds());
        }
        if let Some(a) = self.common.armor_stands.iter().find(|e| e.id == id) {
            return Some(a.bounds());
        }
        if let Some(b) = self.common.boats.iter().find(|e| e.id == id) {
            return Some(b.bounds());
        }
        if let Some(c) = self.common.carts.iter().find(|e| e.id == id) {
            return Some(c.bounds());
        }
        if let Some(t) = self.common.tnts.iter().find(|e| e.id == id) {
            return Some(t.bounds());
        }
        if let Some(f) = self.common.falling.iter().find(|e| e.id == id) {
            return Some(f.bounds());
        }
        if let Some(p) = self.common.paintings.iter().find(|e| e.id == id) {
            return Some(aabb_extents(p.bounds()));
        }
        if let Some(f) = self.common.item_frames.iter().find(|e| e.id == id) {
            return Some(aabb_extents(f.bounds()));
        }
        if let Some(c) = self.common.cushions.iter().find(|e| e.id == id) {
            return Some(aabb_extents(c.bounds()));
        }
        if let Some(i) = self.client.interactions.get(&(id as i32)) {
            return Some(i.bounds());
        }
        if let Some(p) = self
            .common
            .creatures
            .projectiles
            .iter()
            .find(|e| e.id == id)
        {
            return Some(p.bounds());
        }
        if let Some(t) = self.common.tridents.iter().find(|e| e.id == id) {
            return Some(t.bounds());
        }
        if let Some(r) = self.common.rockets.iter().find(|e| e.id == id) {
            return Some(r.bounds());
        }
        if let Some(c) = self.common.crystals.iter().find(|e| e.id == id) {
            return Some(c.bounds());
        }
        if let Some(d) = self
            .common
            .dragon
            .as_ref()
            .filter(|d| (d.id != 0 || self.client.remote.is_some()) && d.id == id)
        {
            let p = d.position;
            return Some(Aabb::new(
                Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
                Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
            ));
        }
        if let Some(row) = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.general.rows.get(&(id as i32)))
        {
            return Some(row.bounds());
        }
        if let Some(&block) = self.client.remote_knots.get(&id) {
            return Some(knot_bounds(block));
        }
        if let Some(k) = self
            .common
            .resident_entities
            .knots
            .iter()
            .find(|k| k.id == id)
        {
            return Some(knot_bounds(k.fence));
        }
        None
    }

    /// `cameraEntity.getBoundingBox()` at the camera entity's current position.
    pub(in crate::game::client_side) fn pick_camera_bounds(&self) -> Option<Aabb> {
        if let Some(r) = self.client.remote.as_ref()
            && let Some(cam_id) = r.camera
        {
            if let Some(detached) = r
                .detached_camera
                .as_deref()
                .filter(|d| d.entity_id == cam_id as u32)
            {
                let p = detached.camera.position;
                let v = Vec3::new(p[0], p[1], p[2]);
                return Some(Aabb::new(v, v));
            }
            if cam_id == r.entity_id {
                return self.common.player.as_ref().map(|p| p.bounds());
            }
            return self.live_entity_bounds_for_pick(cam_id as u32);
        }
        self.common.player.as_ref().map(|p| p.bounds())
    }

    /// `Level.getEntities(cameraEntity, search_box, EntitySelector.CAN_BE_PICKED)`:
    /// visits ordinary entities in `EntitySectionStorage` order (filtered by raw box),
    /// then appends live dragon parts (unless spectating that dragon).
    pub fn for_each_pickable_in_bounds(
        &self,
        search_box: Aabb,
        mut visit: impl FnMut(u32, Aabb, f64),
    ) {
        {
            self.client
                .client_pick
                .sections
                .borrow_mut()
                .sync_with(self);
        }
        {
            let storage = self.client.client_pick.sections.borrow();
            storage.for_each_in_bounds(search_box, |member| {
                if let Some((id, raw_bb, radius)) = self.resolve_pick_member(member)
                    && !self.is_pick_camera(id)
                    && raw_bb.intersects(search_box)
                {
                    visit(id, raw_bb, radius);
                }
            });
        }
        if let Some(d) = self
            .common
            .dragon
            .as_ref()
            .filter(|d| !self.is_pick_camera(d.id))
        {
            let has_wire_id = d.id != 0 || self.client.remote.is_some();
            for i in 0..8 {
                let part_id = if has_wire_id {
                    sunrise_gameplay_common::game::dragon_common::dragon_part_id(d.id, i)
                } else {
                    sunrise_gameplay_common::game::dragon_common::SPECIAL + i as u32
                };
                let part_bb = d.part_geometry(i).1;
                if !self.is_pick_camera(part_id) && part_bb.intersects(search_box) {
                    visit(part_id, part_bb, 0.0);
                }
            }
        }
    }
}

use sunrise_gameplay_common::game::side_ops::mobs_rest::MobsRestSideGameExt as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::ControlFlow;

    #[test]
    fn section_cursor_uses_entry_order_and_refiles_crossings() {
        let mut storage = PickSectionStorage::default();
        for (id, kind, x) in [
            (31, PickKind::RemoteDisplay, 17.0),
            (92, PickKind::RemoteDisplay, 15.5),
            (55, PickKind::Mob, 14.5),
        ] {
            storage.place(PickMember { kind, id }, [x, 65.5, 3.0], u64::from(id));
        }
        let bounds = Aabb::new(
            Vec3::new(-f64::MAX, 64.0, 0.0),
            Vec3::new(f64::MAX, 68.0, 5.0),
        );
        let search = SectionSearch::new(bounds);
        let visit = |storage: &PickSectionStorage, f: &mut dyn FnMut(u32) -> ControlFlow<()>| {
            let mut after = None;
            while let Some(member) = storage.candidate_after(search, &mut after) {
                f(member.id)?;
            }
            ControlFlow::Continue(())
        };
        let collect = |storage: &PickSectionStorage| {
            let mut ids = Vec::new();
            assert_eq!(
                visit(storage, &mut |id| {
                    ids.push(id);
                    ControlFlow::Continue(())
                }),
                ControlFlow::Continue(())
            );
            ids
        };
        assert_eq!(collect(&storage), [92, 55, 31]);
        storage.move_id(92, [15.25, 65.5, 3.0]);
        assert_eq!(collect(&storage), [92, 55, 31]);
        storage.move_id(92, [16.5, 65.5, 3.0]);
        assert_eq!(collect(&storage), [55, 31, 92]);
        let mut seen = Vec::new();
        assert_eq!(
            visit(&storage, &mut |id| {
                seen.push(id);
                ControlFlow::Break(())
            }),
            ControlFlow::Break(())
        );
        assert_eq!(seen, [55]);
    }
}
