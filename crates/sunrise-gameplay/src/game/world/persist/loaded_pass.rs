//! Save visibility while authoritative rows are lent out.
use super::*;

/// A load tree after feature-disabled branches have been removed.
pub struct StagedEntityTree(Tag);

impl StagedEntityTree {
    pub(in crate::game) fn tag(&self) -> &Tag {
        &self.0
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn stage_entity_tree(&self, mut tag: Tag) -> Option<StagedEntityTree> {
        self.retain_feature_eligible_entity_tree(&mut tag)
            .then_some(StagedEntityTree(tag))
    }

    /// Publish just this command or structure placement, not any completed chunk loads
    /// still waiting for `PersistentEntitySectionManager.processPendingLoads`.
    pub(in crate::game) fn spawn_staged_entity_tree(
        &mut self,
        key: ChunkKey,
        mut tree: StagedEntityTree,
    ) {
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        let mut waiting = std::mem::take(&mut self.common.world.loaded_entities);
        mark_fresh(&mut tree.0);
        self.publish_entity_tree(
            key,
            tree,
            None,
            sunrise_level::world::EntityLoadReason::Ordinary,
        );
        self.native_entities_load(self.common.dim.id(), [key.0, key.1], 1);
        waiting.append(&mut self.common.world.loaded_entities);
        self.common.world.loaded_entities = waiting;
    }

    /// Spawn the entities of chunks that finished loading.
    pub fn spawn_loaded_entities(&mut self) {
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        let loaded = std::mem::take(&mut self.common.world.loaded_entities);
        for (key, tags) in loaded {
            let owns_disk_section = self.persist.hidden_entity_sections.contains(&key);
            let count = tags.len();
            let mut skipped = 0;
            for pending in tags {
                let (tag, reason) = pending.into_parts();
                // The worker's snapshot of this record can predate a move or
                // removal. A save that took ownership already admitted it.
                if reason == sunrise_level::world::EntityLoadReason::DiskSection
                    && owns_disk_section
                {
                    skipped += 1;
                    continue;
                }
                self.spawn_loaded_entity_reason(key, tag, None, reason);
            }
            if skipped == 0 || skipped < count {
                self.native_entities_load(self.common.dim.id(), [key.0, key.1], count - skipped);
            }
        }
    }

    /// Install one fixture record or explicitly owned live transfer through the same
    /// metadata and join boundary. The transfer token belongs only to this call.
    #[cfg(any(test, feature = "test-support"))]
    pub fn spawn_loaded_entity(
        &mut self,
        key: ChunkKey,
        tag: Tag,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        self.spawn_loaded_entity_reason(
            key,
            tag,
            transferred,
            sunrise_level::world::EntityLoadReason::Ordinary,
        );
    }

    pub fn spawn_loaded_entity_reason(
        &mut self,
        key: ChunkKey,
        tag: Tag,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
        reason: sunrise_level::world::EntityLoadReason,
    ) {
        if let Some(tree) = self.stage_entity_tree(tag) {
            self.publish_entity_tree(key, tree, transferred, reason);
        }
    }

    fn publish_entity_tree(
        &mut self,
        key: ChunkKey,
        StagedEntityTree(mut tag): StagedEntityTree,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
        reason: sunrise_level::world::EntityLoadReason,
    ) {
        let player_uuid = self.common.level.player_uuid;
        // A world older builds named by the offline profile names the player that
        // way in its entities too: migrate them as they load, so the next save of
        // the chunk keeps the account's UUID (`playerdata::migrate_owner`).
        if let Some((old, new)) = self.server.owner_migration {
            entity_nbt::migrate_player_uuid(
                &mut tag,
                sunrise_sim::lan::playerdata::uuid_ints(old),
                sunrise_sim::lan::playerdata::uuid_ints(new),
            );
        }
        // PersistentEntitySectionManager.addEntity's `loaded`.
        let from_disk = !take_fresh(&mut tag);
        // An End gateway's traveller never left the level: nothing joins.
        let moved = take_moved(&mut tag);
        // Whatever was added since the last tick joins before this one.
        self.native_joins_flush(false, 0);
        let kind = entity_nbt::kind(&tag).unwrap_or_default();
        let is_mob = self.common.res.mob_types().contains_key(kind.as_str());
        if transferred.is_none()
            && from_disk
            && is_mob
            && entity_nbt::position(&tag).is_none()
            && !self.common.world.chunks.contains_key(&(0, 0))
        {
            // The decoder defaults Pos to zero, but that destination has
            // not been read. Keep the source record rather than admitting
            // an entity no resident chunk can save (or overwriting origin).
            // Admission is irreversible: a duplicate is rejected now,
            // not retried after its current owner has left the level.
            let duplicate = entity_nbt::uuid(&tag).is_some_and(|uuid| {
                self.restored_vehicle(uuid).is_some() || self.target_with_uuid(uuid).is_some()
            });
            if !duplicate {
                self.common
                    .persist
                    .unknown
                    .entry(key)
                    .or_default()
                    .push(tag);
            }
            return;
        }
        if !sunrise_gameplay_common::game::general::quarantined_kind(&kind) {
            drop_unleashable_leash(&mut tag, &kind, is_mob);
        }
        let uuid = entity_nbt::uuid(&tag);
        // This shortcut deduplicates disk copies, never explicit runtime ownership.
        if transferred.is_none() && uuid.is_some_and(|u| self.restored_vehicle(u).is_some()) {
            return;
        }
        if matches!(
            kind.as_str(),
            "minecraft:experience_orb"
                | "minecraft:arrow"
                | "minecraft:spectral_arrow"
                | "minecraft:llama_spit"
                | "minecraft:shulker_bullet"
                | "minecraft:fireball"
                | "minecraft:small_fireball"
                | "minecraft:dragon_fireball"
                | "minecraft:wind_charge"
                | "minecraft:breeze_wind_charge"
                | "minecraft:wither_skull"
                | "minecraft:falling_block"
                | "minecraft:firework_rocket"
                | "minecraft:trident"
                | "minecraft:end_crystal"
                | "minecraft:ender_dragon"
        ) && let Some(at) = entity_nbt::position(&tag)
        {
            let key = anon_key(&kind, at);
            let was = self
                .common
                .persist
                .anon
                .insert(key.clone(), (at, tag.clone()));
            self.common.journal_anon((key, was));
        }
        // Kinds outside `entity_lists!` first, then the listed kinds
        // (`Entity::load_nbt`), then whatever else is a compound.
        let transferring = transferred.is_some();
        let unloaded = if kind == "minecraft:lightning_bolt" && !is_mob && !transferring {
            let _ = self.load_bolt(tag, None, !from_disk);
            Ok(())
        } else if kind == "minecraft:ender_dragon" && !is_mob && !transferring {
            self.load_dragon_transferred(tag, uuid, None, from_disk);
            Ok(())
        } else if kind == "minecraft:tnt"
            && !is_mob
            && reason == sunrise_level::world::EntityLoadReason::TeleportedTnt
        {
            self.load_tnt_with_portal(tag, uuid, transferred, true);
            Ok(())
        } else if let Some(state) = transferred {
            self.load_listed_transferred(&kind, uuid, player_uuid, tag, state)
        } else {
            self.load_unlisted(key, &kind, uuid, tag)
                .or_else(|tag| self.load_listed(&kind, uuid, player_uuid, tag))
        };
        if let Err(tag) = unloaded {
            if transferring {
                log::warn!("refusing unsupported transferred entity kind {kind}");
                return;
            }
            if !kind.is_empty() && entity_nbt::position(&tag).is_some() {
                // A record no listed family claimed: the missing-mod gate
                // decides whether the unknown-kind path may keep it.
                let at = entity_nbt::position(&tag).unwrap();
                if self.general_quarantine_admits(&tag, &kind, at) {
                    self.load_other(tag, &kind);
                }
            } else {
                self.common
                    .persist
                    .unknown
                    .entry(key)
                    .or_default()
                    .push(tag);
            }
        }
        // PersistentEntitySectionManager.addEntity: every kind that
        // just joined tells the mods; a moved one does not join.
        if moved {
            self.native_joins_skip();
        } else {
            self.native_joins_flush(from_disk, 0);
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
