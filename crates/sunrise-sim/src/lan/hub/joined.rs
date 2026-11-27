//! Entities that join a level between tracking passes (`ChunkMap.addEntity`): paired as they
//! joined, before their first tick.
use super::{Hub, entity_type, take_tracking, tracking_pass};
use crate::lan::{entity::EntityState, guest::Guest};

impl Hub {
    /// Fresh entities that joined during the current tick before the tracking pass:
    /// paired from their state as they joined (ChunkMap.addEntity), before their first tick.
    pub fn note_fresh_joined(&mut self, entities: &[EntityState]) {
        let joined: Vec<_> = entities
            .iter()
            .filter(|e| !self.tracks(e.id))
            .cloned()
            .collect();
        if joined.is_empty() {
            return;
        }
        self.fresh_joined
            .entry(self.config.dimension.clone())
            .or_default()
            .extend(joined);
    }

    /// `ChunkMap.addEntity` for late arrivals, before their sounds are sent.
    /// Pairing does not run another tracking pass or advance entity clocks.
    pub fn note_joined(
        &mut self,
        entities: &[EntityState],
        entity_attachments: Option<
            &crate::fast_hash::FxHashMap<u32, sunrise_mod_runtime::attachments::Attachments>,
        >,
    ) {
        if entities.is_empty() {
            return;
        }
        let dimension = &self.config.dimension;
        let watches = |g: &Guest| g.watches_level(dimension);
        let mut outs: Vec<Vec<crate::lan::Body>> = self
            .guests
            .iter()
            .filter(|g| watches(g))
            .map(|_| Vec::new())
            .collect();
        let (mut viewers, player_attachments) =
            self.guests
                .tracking_viewers(watches, outs.iter_mut(), false);
        let attachment_lookup = |e: &EntityState| {
            if e.kind == "minecraft:player" {
                player_attachments.get(&e.id).map(|&(uuid, map)| {
                    (sunrise_mod_runtime::attachments::Holder::Player(uuid), map)
                })
            } else {
                let id = u32::try_from(e.id).ok()?;
                entity_attachments
                    .and_then(|maps| maps.get(&id))
                    .map(|map| (sunrise_mod_runtime::attachments::Holder::Entity(id), map))
            }
        };
        let attachments: Option<&crate::lan::entity::InitialAttachmentLookup<'_>> =
            self.sync_attachments.then_some(&attachment_lookup);
        let level = self.entities.entry(dimension.clone()).or_default();
        level.range_percent = Some(self.entity_range_percent);
        let (types, sync) = (&mut self.entity_types, &self.sync);
        level.pair_after_tick(entities, &self.riding, &mut viewers, attachments, |kind| {
            entity_type(types, sync, kind)
        });
        drop(viewers);
        drop(player_attachments);
        // As a tracking pass's: held for the game while a mod listens (`tracking_pass`).
        let hold = tracking_pass::tracking_listened();
        let mut rows = outs.iter_mut();
        for i in 0..self.guests.len() {
            if !watches(&self.guests[i]) {
                continue;
            }
            let out = rows.next().expect("one output per visible guest");
            if hold && self.guests[i].tracker.cut() {
                self.guests.update(i, |g| {
                    take_tracking(
                        g,
                        &mut self.native_tracking,
                        &mut self.attachment_snapshots,
                        self.sync_attachments,
                        Some(std::mem::take(out)),
                    );
                });
            } else {
                self.guests[i].sender.send_tracking_bodies(out);
            }
        }
    }
}
