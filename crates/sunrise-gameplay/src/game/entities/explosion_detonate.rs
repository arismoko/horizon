//! `explosion_detonate`: the blocks and entities a server explosion is about to
//! reach (NeoForge's `ExplosionEvent.Detonate`), and the list of them the
//! listeners leave behind.
use crate::game::ServerGame;
use mod_api::EntityRef;
use sunrise_gameplay_common::game::entity::EntityHandle;

impl ServerGame<'_> {
    /// `ServerExplosion.explode` posting `onExplosionDetonate` before any block or
    /// entity is hit. The entities are `level.getEntities(source, box)`, the box
    /// the centre ± (2 × radius + 1) floored, without the exploding source; every
    /// kind is in it: the entity-list walk, then the item frames, paintings,
    /// cushions and end crystals the walk misses. The list they leave in
    /// `combat.blast_hit_list` is what the blast's walks then use: what a
    /// listener leaves in it is eligible for the normal hurt/push checks.
    /// Listener order and repeated entries are preserved across entity kinds.
    pub fn explosion_detonate(
        &mut self,
        center: [f64; 3],
        radius: f32,
        blocks: &mut Vec<[i32; 3]>,
    ) {
        // `ServerExplosion.hurtEntities` fires it only for a radius of at least 0.00001.
        if radius < 1.0e-5 || !mod_api::__private::listened(mod_api::HookId::ExplosionDetonate) {
            return;
        }
        let reach = f64::from(radius) * 2.0 + 1.0;
        let min = center.map(|c| (c - reach).floor());
        let max = center.map(|c| (c + reach).floor());
        let source = self
            .server
            .combat
            .0
            .explosion_direct
            .map(|(id, _)| id)
            .or(self.server.combat.0.explosion_source);
        let mut entities = self
            .level(self.common.dim.id())
            .and_then(|level| self.level_entities_in(level, min, max))
            .unwrap_or_default();
        entities.retain(|entity| Some(entity.id()) != source);
        self.append_blast_specials(&mut entities, source, min, max);
        let dimension = self.common.dim.id();
        self.native_explosion_detonate(dimension, center, blocks, &mut entities);
        entities.retain(|entity| self.native_entity_target(*entity).is_some());
        self.server.combat.0.blast_hit_ids = entities.iter().map(|entity| entity.id()).collect();
        self.server.combat.0.blast_hit_list =
            Some(entities.iter().map(|entity| entity.id()).collect());
    }

    /// The four entity kinds the entity-list walk misses: item frames,
    /// paintings, cushions and end crystals. Appended after it, in their
    /// storage order, those whose box meets the blast's box
    /// (`Level.getEntities`), the exploding source excluded with the rest.
    fn append_blast_specials(
        &self,
        entities: &mut Vec<EntityRef>,
        source: Option<u32>,
        min: [f64; 3],
        max: [f64; 3],
    ) {
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            return;
        };
        let in_box = |lo: [f64; 3], hi: [f64; 3]| (0..3).all(|k| lo[k] < max[k] && hi[k] > min[k]);
        let listed = |id: u32, lo: [f64; 3], hi: [f64; 3], entities: &mut Vec<EntityRef>| {
            if Some(id) != source && in_box(lo, hi) {
                entities.push(EntityRef::new(epoch, id));
            }
        };
        for frame in self.common.item_frames.iter() {
            let (lo, hi) = frame.bounds();
            listed(frame.id, lo, hi, entities);
        }
        for painting in self.common.paintings.iter() {
            let (lo, hi) = painting.bounds();
            listed(painting.id, lo, hi, entities);
        }
        for cushion in self.common.cushions.iter() {
            let (lo, hi) = cushion.bounds();
            listed(cushion.id, lo, hi, entities);
        }
        for crystal in self.common.crystals.iter() {
            let bounds = crystal.bounds();
            let lo = [
                bounds.min(sunrise_core::geometry::Axis::X),
                bounds.min(sunrise_core::geometry::Axis::Y),
                bounds.min(sunrise_core::geometry::Axis::Z),
            ];
            let hi = [
                bounds.max(sunrise_core::geometry::Axis::X),
                bounds.max(sunrise_core::geometry::Axis::Y),
                bounds.max(sunrise_core::geometry::Axis::Z),
            ];
            listed(crystal.id, lo, hi, entities);
        }
    }

    /// The handle of the storaged entity `id` names, for the listed walk.
    /// The kinds outside the storages (frames, paintings, cushions, crystals,
    /// the dragon and its parts, hooks, players) answer None: their own
    /// passes and the player walk read the list for them.
    pub(in crate::game::server_side) fn blast_handle(&self, id: u32) -> Option<EntityHandle> {
        if let Some(i) = self.common.mobs.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Mob(i));
        }
        if let Some(i) = self.common.dropped.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Item(i));
        }
        if let Some(i) = self.common.boats.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Boat(i));
        }
        if let Some(i) = self.common.carts.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Cart(i));
        }
        if let Some(i) = self.common.armor_stands.iter().position(|m| m.id == id) {
            return Some(EntityHandle::ArmorStand(i));
        }
        if let Some(i) = self.common.tnts.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Tnt(i));
        }
        if let Some(i) = self.common.falling.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Falling(i));
        }
        if let Some(i) = self.common.orbs.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Orb(i));
        }
        if let Some(i) = self
            .common
            .creatures
            .projectiles
            .iter()
            .position(|m| m.id == id)
        {
            return Some(EntityHandle::Projectile(i));
        }
        if let Some(i) = self.common.rockets.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Rocket(i));
        }
        if let Some(i) = self.common.tridents.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Trident(i));
        }
        if let Some(i) = self
            .common
            .resident_entities
            .general
            .rows
            .iter()
            .position(|m| m.id == id)
        {
            return Some(EntityHandle::General(i));
        }
        None
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
