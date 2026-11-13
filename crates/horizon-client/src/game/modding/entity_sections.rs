//! `entity_entering_section` (`EntityEvent.EnteringSection`): vanilla tells it from
//! `Entity.setPosRaw`, whose `EntityInLevelCallback.onMove` (`PersistentEntitySectionManager`'s
//! callback on a server, `TransientEntitySectionManager`'s on a client) compares the section the
//! entity is filed under with the one its new position is in, and moves it at once when they
//! differ. Horizon's kinds keep their positions in their own fields, so the hook is told from
//! the places that write them: [`EntitySections`] files each entity under its section, and each
//! position-update boundary hands it the entity's new position.
//!
//! The boundaries: an entity joining (the server's `native_joins_flush`, a client's
//! `AddEntity`) files it untold. Each entity's own tick ends at one (its `EntityTickEvent.Post`
//! for the mobs, items, minecarts and boats, [`Family`] for the kinds that tick without one, a
//! client's entities after its tick). A teleport, a piston push, a mod's position write, each
//! move a server player sent and each packet that moves an entity on a client are one each.
//! Every boundary that sees another section queues a crossing from the section the entity was
//! filed under, so two moves told apart are two crossings, and a move out and back is two.
//!
//! The queue is told as soon as the game is free of the entity lists ([`Game::tell_entity_sections`]):
//! after the entity's tick, the packet or the teleport. What a listener moves is filed then and
//! told at the next boundary, never from inside its own call.
//!
//! With no listener each boundary is one subscription check and nothing is filed. With one, a
//! boundary is a hash lookup and nothing is allocated once the table and the queue have grown.

use super::Game;
use super::entity::Entity as _;
use horizon_core::geometry::Vec3;
use mod_api::{DimensionKey, EntityRef, WorldEpoch};
use std::collections::HashMap;
use std::collections::hash_map::Entry;

#[cfg(test)]
#[path = "entity_sections/tests.rs"]
mod tests;

/// `SectionPos.blockToSectionCoord(Mth.floor(v))`: `Mth.floor` truncates (saturating, NaN to 0,
/// as Java's cast does) and steps down for a negative fraction with Java's wrapping `int`.
fn section_coord(v: f64) -> i32 {
    let truncated = v as i32;
    let block = if v < f64::from(truncated) {
        truncated.wrapping_sub(1)
    } else {
        truncated
    };
    block >> 4
}

fn section_of([x, y, z]: [f64; 3]) -> [i32; 3] {
    [section_coord(x), section_coord(y), section_coord(z)]
}

/// A section change a boundary found: the entity, its old and its new section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Crossing {
    pub(super) id: u32,
    pub(super) old: [i32; 3],
    pub(super) new: [i32; 3],
}

/// Where a boundary is: the world open and the level the entity is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Scope {
    epoch: WorldEpoch,
    level: DimensionKey,
}

/// The section each entity is filed under, and the crossings not told yet.
#[derive(Default)]
pub(super) struct EntitySections {
    /// The world open the table was filled in: another one starts it over.
    epoch: Option<WorldEpoch>,
    /// Each entity's level and section.
    filed: HashMap<u32, (DimensionKey, [i32; 3])>,
    /// The crossings found since the last telling, in the order they happened.
    queue: Vec<Crossing>,
    /// The batch being told; kept for its capacity.
    telling: Vec<Crossing>,
    /// A telling is under way: a listener's moves wait for the next one.
    busy: bool,
    /// The mobs' position updates during their family's tick; kept for its capacity.
    pub(super) ticked: Vec<(u32, Vec3)>,
}

impl EntitySections {
    /// The table for `scope`'s world: another world's entries are dropped.
    fn table(&mut self, scope: Scope) -> &mut HashMap<u32, (DimensionKey, [i32; 3])> {
        if self.epoch != Some(scope.epoch) {
            self.epoch = Some(scope.epoch);
            self.filed.clear();
            self.queue.clear();
        }
        &mut self.filed
    }

    /// `addEntity`: entity `id` is filed at `at`, untold. A new entity under an id seen before
    /// (a client's `AddEntity` replacing one) starts over too.
    pub(super) fn placed(&mut self, scope: Scope, id: u32, at: [f64; 3]) {
        self.table(scope).insert(id, (scope.level, section_of(at)));
    }

    /// `setPosRaw`: entity `id` is at `to` now. Filed under another section of the same level,
    /// that is a crossing. One not filed in this level yet (it changed level, or its join is
    /// still to be told) is filed where it was, `from` when the boundary knows it, so a move
    /// before its join is told is a crossing too; otherwise it is filed untold.
    pub(super) fn moved(&mut self, scope: Scope, id: u32, from: Option<[f64; 3]>, to: [f64; 3]) {
        let new = section_of(to);
        let old = match self.table(scope).entry(id) {
            Entry::Occupied(mut entry) => {
                let (level, section) = entry.get_mut();
                if *level != scope.level {
                    *level = scope.level;
                    *section = new;
                    return;
                }
                std::mem::replace(section, new)
            }
            Entry::Vacant(entry) => {
                entry.insert((scope.level, new));
                match from {
                    Some(from) => section_of(from),
                    None => return,
                }
            }
        };
        if old != new {
            self.queue.push(Crossing { id, old, new });
        }
    }

    /// `id` is gone (removed, or respawned as a new entity under the same id): it is filed
    /// again where it next appears, untold.
    pub(super) fn forget(&mut self, id: u32) {
        self.filed.remove(&id);
    }

    /// The crossings to tell now, or none while a telling is under way.
    fn begin_telling(&mut self) -> Option<Vec<Crossing>> {
        if self.busy || self.queue.is_empty() {
            return None;
        }
        self.busy = true;
        let mut batch = std::mem::take(&mut self.telling);
        std::mem::swap(&mut batch, &mut self.queue);
        Some(batch)
    }

    /// The telling is over; `batch` goes back for its capacity.
    fn end_telling(&mut self, mut batch: Vec<Crossing>) {
        batch.clear();
        self.telling = batch;
        self.busy = false;
    }
}

/// The kinds that tick without an `EntityTickEvent` round: each ends its family's tick with
/// [`Game::entity_sections_stepped`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Family {
    ArmorStands,
    Tridents,
    Rockets,
    Projectiles,
    /// Experience orbs, primed TNT and falling blocks, which tick in join order.
    Ordered,
    FishingHooks,
    Dragon,
}

impl Game<'_> {
    /// Where boundaries are now: None with no listener (nothing is filed) or no world open.
    pub(super) fn entity_section_scope(&self) -> Option<Scope> {
        if !mod_api::__private::listened(mod_api::HookId::EntityEnteringSection) {
            return None;
        }
        Some(Scope {
            epoch: self.native_lifecycle.epoch()?,
            level: self.native_level_key(),
        })
    }

    /// An entity joined at `at`: filed untold.
    pub(super) fn entity_section_placed(&mut self, id: u32, at: [f64; 3]) {
        if let Some(scope) = self.entity_section_scope() {
            self.native_lifecycle.entity_sections.placed(scope, id, at);
        }
    }

    /// Entity `id` was put from `from` (when known) at `to`; told at once when the game is free.
    pub(super) fn entity_section_moved(&mut self, id: u32, from: Option<[f64; 3]>, to: [f64; 3]) {
        if let Some(scope) = self.entity_section_scope() {
            self.native_lifecycle
                .entity_sections
                .moved(scope, id, from, to);
            self.tell_entity_sections();
        }
    }

    /// A server player's moves this tick (`moves`: each one's delta, in order), which left it at
    /// `end`: each is one `setPos`, so a move out of a section and one back are two crossings.
    pub(super) fn entity_section_walked(
        &mut self,
        id: u32,
        end: [f64; 3],
        moves: impl Iterator<Item = [f64; 3]> + Clone,
    ) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        let mut from = moves
            .clone()
            .fold(end, |p, d| [p[0] - d[0], p[1] - d[1], p[2] - d[2]]);
        let sections = &mut self.native_lifecycle.entity_sections;
        for d in moves {
            let to = [from[0] + d[0], from[1] + d[1], from[2] + d[2]];
            sections.moved(scope, id, Some(from), to);
            from = to;
        }
        self.tell_entity_sections();
    }

    /// Entity `id`'s position was written: it is wherever it is now (any kind, either side).
    pub(super) fn entity_section_moved_id(&mut self, id: u32) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        if let Some(at) = self.entity_position(EntityRef::new(scope.epoch, id)) {
            self.native_lifecycle
                .entity_sections
                .moved(scope, id, None, at);
        }
    }

    /// Entity `id` is gone, or replaced under the same id: it starts over untold.
    pub(super) fn entity_section_forget(&mut self, id: u32) {
        self.native_lifecycle.entity_sections.forget(id);
    }

    /// The end of a family's tick for the kinds without an `EntityTickEvent` round: each one's
    /// own move, then the crossings are told.
    pub(super) fn entity_sections_stepped(&mut self, family: Family) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        // Out of the game while its lists are walked; nothing is told meanwhile.
        let mut sections = std::mem::take(&mut self.native_lifecycle.entity_sections);
        let mut step = |id: u32, from: Vec3, to: Vec3| {
            if id != 0 {
                sections.moved(
                    scope,
                    id,
                    Some([from.x, from.y, from.z]),
                    [to.x, to.y, to.z],
                );
            }
        };
        macro_rules! each {
            ($($list:ident).+) => {
                for e in self.$($list).+.iter() {
                    if !e.is_removed() {
                        step(e.id(), e.previous_position(), e.position());
                    }
                }
            };
        }
        match family {
            Family::ArmorStands => each!(armor_stands),
            Family::Tridents => each!(tridents),
            Family::Rockets => each!(rockets),
            Family::Projectiles => each!(creatures.projectiles),
            Family::Ordered => {
                each!(orbs);
                each!(tnts);
                each!(falling);
            }
            Family::FishingHooks => {
                for (hook, _) in self.fishing_hooks_here() {
                    step(hook.id(), hook.previous, hook.position);
                }
            }
            Family::Dragon => {
                if let Some(d) = self.dragon.as_ref() {
                    step(d.id, d.previous, d.position);
                }
            }
        }
        self.native_lifecycle.entity_sections = sections;
        self.tell_entity_sections();
    }

    /// What a family's ticks moved, in order (`moves`: each mob's teleports in its tick, then
    /// where its tick left it): each filed as one position update, and `moves` emptied. Told
    /// with the family's round.
    pub(super) fn entity_sections_ticked(&mut self, moves: &mut Vec<(u32, Vec3)>) {
        if let Some(scope) = self.entity_section_scope() {
            let sections = &mut self.native_lifecycle.entity_sections;
            for &(id, at) in moves.iter() {
                sections.moved(scope, id, None, [at.x, at.y, at.z]);
            }
        }
        moves.clear();
    }

    /// Entity `id`'s position was written from `from` to `to` (`setPosRaw`) where nothing is told
    /// yet: inside its family's tick (so one about to be removed is filed first), or a rider
    /// carried along by its vehicle's teleport. Filed now, told at the boundary that follows.
    pub(super) fn entity_section_step(&mut self, id: u32, from: [f64; 3], to: [f64; 3]) {
        if id != 0
            && let Some(scope) = self.entity_section_scope()
        {
            self.native_lifecycle
                .entity_sections
                .moved(scope, id, Some(from), to);
        }
    }

    /// `Entity.teleport` placed as `/tp` places it ([`Game::place_teleport_in_level`]), then filed
    /// and told as one position update (`/tp`, a mob's ender pearl). Its old position, as the
    /// placement returns it.
    pub(super) fn place_teleport_filed(
        &mut self,
        t: super::commands::Target,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        relative: [bool; 3],
        turn_riders: bool,
    ) -> Option<[f64; 3]> {
        let from = self.place_teleport_in_level(t, pos, rotation, relative, turn_riders);
        if let (Some(entity), Some(to)) = (self.network_entity_id(t), self.target_position(t)) {
            self.entity_section_moved(entity as u32, from, to);
        }
        from
    }

    /// An entity joined (`addEntity`): filed untold where it is now, which a join listener may
    /// have changed from `at`, where it joined.
    pub(super) fn entity_section_joined(&mut self, id: u32, at: [f64; 3]) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        let now = self.entity_position(EntityRef::new(scope.epoch, id));
        self.native_lifecycle
            .entity_sections
            .placed(scope, id, now.unwrap_or(at));
    }

    /// The projectiles tick ([`Game::tick_projectiles_now`]), and each one's move is filed.
    pub(super) fn tick_projectiles(&mut self) -> super::Result<()> {
        let result = self.tick_projectiles_now();
        self.entity_sections_stepped(Family::Projectiles);
        result
    }

    /// A client's entities after its tick: each one where its interpolation or its own move
    /// put it (its live position, not the packet's), and this window's player.
    pub(super) fn client_entity_sections_stepped(&mut self) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        let own = self.own_entity_id();
        let mut sections = std::mem::take(&mut self.native_lifecycle.entity_sections);
        if let Some(r) = self.remote.as_ref() {
            for &id in r.entities.keys().filter(|&&id| id != own && id > 0) {
                if let Some(at) = self.entity_position(EntityRef::new(scope.epoch, id as u32)) {
                    sections.moved(scope, id as u32, None, at);
                }
            }
        }
        if let Some(player) = self.player.as_ref()
            && own > 0
        {
            let at = player.position;
            sections.moved(scope, own as u32, None, [at.x, at.y, at.z]);
        }
        self.native_lifecycle.entity_sections = sections;
        self.tell_entity_sections();
    }

    /// A client handles one packet from its server ([`Game::apply_packet`]). A packet that adds,
    /// moves or removes an entity is a position-update boundary: `handleAddEntity` files the
    /// entity untold, a move (`handleMoveEntity`, `handleEntityPositionSync`,
    /// `handleTeleportEntity`, `handleMovePlayer`'s `PlayerPosition`, a vehicle's) is told when
    /// it put the entity in another section, and `handleRespawn`'s new player starts over.
    /// An entity a move only starts interpolating moves at the client's entity tick.
    pub(in crate::game) fn handle_packet(
        &mut self,
        packet: horizon_protocol::packets::play::Clientbound,
    ) -> super::Result<()> {
        use horizon_protocol::packets::play::{self, Clientbound as C};
        if self.entity_section_scope().is_none() {
            return self.apply_packet(packet);
        }
        let moved = match &packet {
            C::MoveEntityPos(play::MoveEntityPos(m))
            | C::MoveEntityPosRot(play::MoveEntityPosRot(m)) => Some(m.id),
            C::EntityPositionSync(play::EntityPositionSync { id, .. })
            | C::TeleportEntity(play::TeleportEntity { id, .. })
            | C::MoveMinecartAlongTrack(play::MoveMinecartAlongTrack { id, .. }) => Some(*id),
            C::PlayerPosition(_) => Some(self.own_entity_id()),
            C::MoveVehicle(_) => self.driven_mount().map(|m| m.id as i32),
            _ => None,
        };
        let added = match &packet {
            C::AddEntity(p) => Some((p.id, p.position)),
            _ => None,
        };
        if let C::RemoveEntities(p) = &packet {
            for &id in &p.ids {
                self.entity_section_forget(id as u32);
            }
        }
        let respawned = matches!(packet, C::Respawn(_));
        let result = self.apply_packet(packet);
        if let Some((id, at)) = added.filter(|&(id, _)| id > 0) {
            self.entity_section_placed(id as u32, at);
        }
        if respawned {
            self.entity_section_forget(self.own_entity_id() as u32);
        }
        if let Some(id) = moved.filter(|&id| id > 0) {
            self.entity_section_moved_id(id as u32);
            self.tell_entity_sections();
        }
        result
    }

    /// Tells `entity_entering_section` each queued crossing, in order. Called from inside a
    /// listener it does nothing: what the listener moved is told at the next boundary.
    pub(super) fn tell_entity_sections(&mut self) {
        let Some(batch) = self.native_lifecycle.entity_sections.begin_telling() else {
            return;
        };
        if let Some(epoch) = self.native_lifecycle.epoch() {
            for c in &batch {
                let chunk_changed = c.old[0] != c.new[0] || c.old[2] != c.new[2];
                self.native_entity_entering_section(
                    EntityRef::new(epoch, c.id),
                    c.old,
                    c.new,
                    chunk_changed,
                );
            }
        }
        self.native_lifecycle.entity_sections.end_telling(batch);
    }
}
