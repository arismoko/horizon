//! `OminousItemSpawner`: the item an ominous trial spawner drops on its
//! fight. It floats, growing and turning (`OminousItemSpawnerRenderer`), for
//! 60 to 120 ticks, then falls as an item or is fired straight down as a
//! projectile (`spawnItem`). Saved with its chunk's entities (persist.rs).
use crate::game::ServerGame;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::ominous_item_spawner_common::OminousItemSpawner;
use sunrise_items::items::ItemStack;

/// `TICKS_BEFORE_ABOUT_TO_SPAWN_SOUND`.
pub(in crate::game) const ABOUT_TO_SPAWN: i64 = 36;
/// `ProjectileItem.DispenseConfig` of the items fired down: power,
/// uncertainty and `overrideDispenseEvent`; None for items that just fall.
pub(in crate::game::server_side) fn projectile(item: &str) -> Option<(f32, f32, Option<i32>)> {
    Some(match item {
        "minecraft:fire_charge" => (1.0, 6.666_666_5, Some(1018)),
        "minecraft:wind_charge" => (1.0, 6.666_666_5, Some(1051)),
        "minecraft:firework_rocket" => (0.5, 1.0, Some(1004)),
        "minecraft:experience_bottle"
        | "minecraft:splash_potion"
        | "minecraft:lingering_potion" => (1.375, 3.0, None),
        "minecraft:arrow"
        | "minecraft:spectral_arrow"
        | "minecraft:tipped_arrow"
        | "minecraft:snowball"
        | "minecraft:egg"
        | "minecraft:blue_egg"
        | "minecraft:brown_egg" => (1.1, 6.0, None),
        _ => return None,
    })
}

pub(in crate::game::server_side) fn block(p: [f64; 3]) -> [i32; 3] {
    p.map(|v| v.floor() as i32)
}

impl ServerGame<'_> {
    /// `OminousItemSpawner.create` added at `position`.
    pub fn add_ominous_item_spawner(&mut self, item: ItemStack, position: [f64; 3], delay: i32) {
        self.server.next_entity += 1;
        self.common.ominous_spawners.push(OminousItemSpawner {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: self.server.next_entity,
            item,
            position,
            spawn_item_after: i64::from(delay),
            tick_count: 0,
            fire_ticks: 0,
            synced_fire: false,
            has_visual_fire: false,
            uuid: sunrise_sim::save::random_uuid(),
            raw: None,
        });
    }
    /// A saved spawner, as `readAdditionalSaveData` leaves it: its tick
    /// count starts again from 0.
    pub fn load_ominous_item_spawner(&mut self, tag: sunrise_core::nbt::Tag) {
        self.load_ominous_item_spawner_transferred(tag, None);
    }
    pub(in crate::game::server_side) fn load_ominous_item_spawner_transferred(
        &mut self,
        mut tag: sunrise_core::nbt::Tag,
        transferred: Option<super::entity_initialization::TransferredEntityState>,
    ) {
        let Some((item, at, spawn_item_after)) =
            sunrise_sim::entity_nbt::ominous_item_spawner_from_nbt(&tag)
        else {
            return;
        };
        self.server.next_entity += 1;
        let id = self.server.next_entity;
        let saved = sunrise_sim::entity_nbt::uuid(&tag);
        // Its saved UUID and the attachments saved under it (#7398).
        if let Some(state) = transferred {
            self.adopt_transferred_entity_state(id, state);
        }
        self.remember_uuid(id, saved, &tag);
        // Entity.load's `setYRot`, `setXRot`, and `setRot`; the save writes the rotation back from this tag.
        let rotation = sunrise_sim::entity_nbt::loaded_rotation(&tag);
        if let sunrise_core::nbt::Tag::Compound(map) = &mut tag {
            map.insert("Rotation".into(), sunrise_sim::save::floats(&rotation));
        }
        let uuid = self.uuid_for(id);
        let fire_ticks = sunrise_sim::entity_nbt::fire_ticks_from_nbt(&tag);
        let has_visual_fire = sunrise_sim::entity_nbt::flag(&tag, "HasVisualFire");
        self.common.ominous_spawners.push(OminousItemSpawner {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            item,
            position: [at.x, at.y, at.z],
            spawn_item_after,
            tick_count: 0,
            fire_ticks,
            synced_fire: false,
            has_visual_fire,
            uuid,
            raw: Some(Box::new(tag)),
        });
    }
    /// Every item spawner's tick: `commonTick`'s count, `tickServer` (the
    /// warning sound, then the item) and `tickClient` (particles every 5
    /// game ticks).
    pub fn tick_ominous_item_spawners(&mut self) {
        // ServerLevel.tick: only entities in entity-ticking chunks that joined before the pass tick.
        let ticking = self.entity_ticking_area();
        let through = self.server.ticked_entities_through;
        let mut spawners = self.common.ominous_spawners.lift();
        #[cfg(any(test, feature = "test-support"))]
        crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
        spawners.retain_mut(|s| {
            if s.id > through
                || !ticking(s.position[0].floor() as i32, s.position[2].floor() as i32)
            {
                return true;
            }
            // Entity.baseTick: the spawner burns down; its hurt does nothing.
            if s.fire_ticks > 0 {
                s.fire_ticks -= 1;
            }
            s.synced_fire = s.fire_ticks > 0 || s.has_visual_fire;
            s.tick_count += 1;
            let at = block(s.position);
            let center = at.map(|v| f64::from(v) + 0.5);
            if s.tick_count == s.spawn_item_after - ABOUT_TO_SPAWN {
                self.server_play_sound_on(
                    6,
                    "minecraft:block.trial_spawner.about_to_spawn_item",
                    center,
                    1.0,
                    1.0,
                );
            }
            if s.tick_count >= s.spawn_item_after {
                self.spawn_ominous_item(s);
                return self.forget_finished_spawner(s.id);
            }
            true
        });
        self.common.ominous_spawners.restore(
            spawners,
            sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst,
        );
    }
    /// `spawnItem`: a projectile shot down, or an item entity; level event
    /// 3021 and the entity_place game event.
    pub(in crate::game::server_side) fn spawn_ominous_item(&mut self, s: &OminousItemSpawner) {
        // An empty item spawns nothing, and no event marks it.
        if &*s.item.item == "minecraft:air" {
            return;
        }
        let at = block(s.position);
        let p = s.position;
        match projectile(&s.item.item) {
            Some((power, uncertainty, event)) => {
                if let Some(event) = event {
                    self.server_level_event_at(event, at, 0);
                }
                let from = Vec3::new(p[0], p[1], p[2]);
                self.shoot_projectile_item(from, [0.0, -1.0, 0.0], &s.item, power, uncertainty);
            }
            None => {
                self.spawn_item_own(s.item.clone(), p, 0);
            }
        }
        self.server_level_event_at(3021, at, 1);
        self.post_block_game_event("minecraft:entity_place", at, None, None);
    }
}
