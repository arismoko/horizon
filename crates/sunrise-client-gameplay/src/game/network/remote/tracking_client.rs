use super::*;
use sunrise_client::particles::TrackingBody;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `recreateFromPacket`: yRot and xRot from the packet, the body and head
    /// both at its head yaw (LivingEntity).
    #[allow(dead_code)]
    pub(in crate::game) fn client_attachment_entity_exists(&self, holder: Holder) -> bool {
        self.client
            .remote
            .as_ref()
            .is_some_and(|remote| match holder {
                Holder::Entity(id) => remote.entities.contains_key(&(id as i32)),
                Holder::Player(uuid) => {
                    remote.profile == uuid || remote.level_players.values().any(|p| *p == uuid)
                }
                _ => false,
            })
    }

    pub(in crate::game::client_side) fn detach_remote_tracking_entity(&mut self, id: i32) {
        let target = TrackingTarget::Entity(id);
        if self.client.particles.0.has_tracking_target(target) {
            let last_body = self.remote_entity_tracking_body(id);
            self.client
                .particles
                .0
                .detach_tracking_target(target, || last_body);
        }
    }
    /// The current entity position and dimensions used by `TrackingEmitter.tick`.
    /// Read body copies rather than the network position of an interpolated entity.
    pub(in crate::game::client_side) fn remote_entity_tracking_body(
        &self,
        id: i32,
    ) -> Option<TrackingBody> {
        let remote = self.client.remote.as_ref()?;
        if let Some(row) = remote.general.rows.get(&id) {
            let (width, height) = sunrise_gameplay_common::game::general::general_type_catalog()
                .dimensions(row.type_slot);
            return Some(TrackingBody {
                feet: row.position,
                width,
                height,
            });
        }
        let (kind, last_position) = remote.tracked(id)?;
        let eid = id as u32;
        // The dragon's render and motion body is separate from its mob row.
        if kind != "minecraft:ender_dragon"
            && (kind == "minecraft:player" || self.common.res.mob_types().contains_key(kind))
            && let Some(i) = self.common.mob_index.find_ref(&self.common.mobs, eid)
        {
            return Some(mob_tracking_body(&self.common.mobs[i]));
        }
        if let Some(entity) = self.client.interactions.get(&id) {
            let Vec3 { x, y, z } = entity.position;
            return Some(TrackingBody {
                feet: [x, y, z],
                width: f64::from(entity.width),
                height: f64::from(entity.height),
            });
        }
        let (mut width, mut height) = self
            .common
            .res
            .rest()
            .spawner_types
            .get(kind)?
            .entity_dimensions();
        let feet = match kind {
            "minecraft:ender_dragon" => {
                let dragon = self
                    .common
                    .dragon
                    .as_ref()
                    .filter(|dragon| dragon.id == eid)?;
                let Vec3 { x, y, z } = dragon.position;
                [x, y, z]
            }
            "minecraft:armor_stand" => {
                let stand = self
                    .common
                    .armor_stands
                    .iter()
                    .find(|stand| stand.id == eid)?;
                (width, height) = sunrise_gameplay_common::game::entity::armor_stand_size(stand);
                let Vec3 { x, y, z } = stand.position;
                [x, y, z]
            }
            "minecraft:area_effect_cloud" => {
                let cloud = self
                    .client
                    .remote_clouds
                    .iter()
                    .find(|cloud| cloud.id == eid)?;
                width = f64::from(cloud.radius * 2.0);
                cloud.at
            }
            "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:experience_bottle" => {
                let (_, _, _, position, _) = self
                    .common
                    .potions
                    .thrown_tracked()
                    .find(|(id, ..)| *id == eid)?;
                [position.x, position.y, position.z]
            }
            "minecraft:fishing_bobber" => {
                let hook = self
                    .client
                    .remote_hooks
                    .iter()
                    .find(|hook| hook.id == eid)?;
                let Vec3 { x, y, z } = hook.position;
                [x, y, z]
            }
            _ => self
                .client
                .text_displays
                .0
                .get(&id)
                .map(|display| display.position().to_array())
                .or_else(|| self.remote_simulated_entity_position(id))
                .unwrap_or(last_position),
        };
        Some(TrackingBody {
            feet,
            width,
            height,
        })
    }

    /// Current simulated position of a client-mirrored entity (excluding server-only `others`).
    pub(in crate::game::client_side) fn remote_simulated_entity_position(
        &self,
        id: i32,
    ) -> Option<[f64; 3]> {
        if let Some(row) = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.general.rows.get(&id))
        {
            return Some(row.position);
        }
        if let Some(entity) = self.client.interactions.get(&id) {
            return Some([entity.position.x, entity.position.y, entity.position.z]);
        }
        if let Some(display) = self.client.text_displays.0.get(&id) {
            return Some(display.position().to_array());
        }
        let eid = u32::try_from(id).ok()?;
        if let Some(dragon) = self.common.dragon.as_ref() {
            if dragon.id == eid {
                return Some([dragon.position.x, dragon.position.y, dragon.position.z]);
            }
            if let Some(part) =
                sunrise_gameplay_common::game::dragon_common::dragon_part_index(dragon.id, eid)
            {
                let p = dragon.parts[part].0;
                return Some([p.x, p.y, p.z]);
            }
        }
        if let Some(m) = self.common.mob_index.get_ref(&self.common.mobs, eid) {
            return Some([m.position.x, m.position.y, m.position.z]);
        }
        if let Some(b) = self.common.boats.iter().find(|b| b.id == eid) {
            return Some([b.position.x, b.position.y, b.position.z]);
        }
        if let Some(c) = self.common.carts.iter().find(|c| c.id == eid) {
            return Some([c.position.x, c.position.y, c.position.z]);
        }
        if let Some(d) = self.common.dropped.iter().find(|d| d.id == eid) {
            return Some([d.position.x, d.position.y, d.position.z]);
        }
        if let Some(o) = self.common.orbs.iter().find(|o| o.id == eid) {
            return Some([o.position.x, o.position.y, o.position.z]);
        }
        if let Some(p) = self
            .common
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == eid)
        {
            return Some([p.position.x, p.position.y, p.position.z]);
        }
        if let Some(t) = self.common.tridents.iter().find(|t| t.id == eid) {
            return Some([t.position.x, t.position.y, t.position.z]);
        }
        if let Some(r) = self.common.rockets.iter().find(|r| r.id == eid) {
            return Some([r.position.x, r.position.y, r.position.z]);
        }
        if let Some(a) = self.common.armor_stands.iter().find(|a| a.id == eid) {
            return Some([a.position.x, a.position.y, a.position.z]);
        }
        if let Some(c) = self.common.cushions.iter().find(|c| c.id == eid) {
            return Some([c.position.x, c.position.y, c.position.z]);
        }
        if let Some(f) = self.common.item_frames.iter().find(|f| f.id == eid) {
            let (lo, hi) = f.bounds();
            return Some([
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            ]);
        }
        if let Some(p) = self.common.paintings.iter().find(|p| p.id == eid) {
            let (lo, hi) = p.bounds();
            return Some([
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            ]);
        }
        if let Some(c) = self.common.crystals.iter().find(|c| c.id == eid) {
            return Some([c.position.x, c.position.y, c.position.z]);
        }
        if let Some(t) = self.common.tnts.iter().find(|t| t.id == eid) {
            return Some([t.position.x, t.position.y, t.position.z]);
        }
        if let Some(f) = self.common.falling.iter().find(|f| f.id == eid) {
            return Some([f.position.x, f.position.y, f.position.z]);
        }
        if let Some(c) = self.common.breath_clouds.iter().find(|c| c.id == eid) {
            return Some(c.at);
        }
        if let Some(b) = self.common.bolts.iter().find(|b| b.id == eid) {
            return Some(b.at);
        }
        if let Some(f) = self.common.fangs.list.iter().find(|f| f.id == eid) {
            return Some(f.pos);
        }
        if let Some(s) = self.common.ominous_spawners.iter().find(|s| s.id == eid) {
            return Some(s.position);
        }
        if let Some((_, _, _, p, _)) = self
            .common
            .potions
            .thrown_tracked()
            .find(|(tid, ..)| *tid == eid)
        {
            return Some([p.x, p.y, p.z]);
        }
        if let Some(c) = self.client.remote_clouds.iter().find(|c| c.id == eid) {
            return Some(c.at);
        }
        if let Some((_, p)) = self
            .common
            .end
            .eye_positions()
            .find(|(eye_id, _)| *eye_id == eid)
        {
            return Some(p);
        }
        if let Some(h) = self.client.remote_hooks.iter().find(|h| h.id == eid) {
            return Some([h.position.x, h.position.y, h.position.z]);
        }
        if let Some(&fence) = self.client.remote_knots.get(&eid) {
            let p = sunrise_sim::mobs::leash::knot_state(fence).position;
            return Some([p.x, p.y, p.z]);
        }
        None
    }
    pub fn spawn_entity(&mut self, id: i32, kind: &str, position: [f64; 3], rotation: [f32; 3]) {
        self.detach_remote_tracking_entity(id);
        let [yaw, pitch, head] = rotation;
        let at = Vec3::new(position[0], position[1], position[2]);
        let mob_type = if kind == "minecraft:player" {
            self.client
                .remote
                .as_ref()
                .and_then(|r| r.player_type.clone())
        } else {
            self.common.res.mob_types().get(kind).cloned()
        };
        if kind == "minecraft:ender_dragon" {
            self.spawn_remote_dragon(id, position, yaw);
        }
        if kind == "minecraft:experience_orb" {
            // Its value (ExperienceOrb.DATA_VALUE) follows in SetEntityData.
            self.common.orbs.retain(|o| o.id != id as u32);
            self.common
                .orbs
                .push(ExperienceOrb::new(id as u32, at, Vec3::ZERO, 1));
        }
        if let Some(t) = mob_type {
            // The window's copy of the server's mob: no mod part goals —
            // their `register_goals` ran on the authoritative side (#1077).
            let mut mob = Mob::new_remote(id as u32, t, at, head);
            super::super::remote_mob_data::client_defaults(&mut mob);
            if kind == "minecraft:player"
                && let Some(r) = self.client.remote.as_ref()
            {
                mob.extra.spectator = r.remote_player_info_game_mode(id) == Some(SPECTATOR);
            }
            (mob.yaw, mob.previous_yaw) = (yaw, yaw);
            (mob.head_pitch, mob.previous_head_pitch) = (pitch, pitch);
            mob.persistent = true;
            self.common.mobs.retain(|m| m.id != id as u32);
            self.common.mobs.push(mob);
        }
        if let Some(r) = self.client.remote.as_mut() {
            // A new entity has a new interpolation handler (a mob's or an
            // armor stand's), even under an id the server used before.
            r.stepped.remove(&id);
            r.synced_attributes.remove(&id);
            if r.entities
                .get(&id)
                .is_some_and(|old| old.kind == "minecraft:item")
            {
                self.common.dropped.retain(|item| item.id != id as u32);
                r.pending_items.remove(&id);
            }
            if kind == "minecraft:item" {
                r.pending_items.insert(
                    id,
                    PendingItem {
                        join: JoinSeq::next(),
                        position,
                        velocity: Vec3::ZERO,
                        no_gravity: false,
                    },
                );
            }
            let tracked_yaw = if yaw.is_finite() { yaw } else { 0.0 };
            let tracked_pitch = if pitch.is_finite() {
                (pitch % 360.0).clamp(-90.0, 90.0)
            } else {
                0.0
            };
            r.entities.insert(
                id,
                Tracked {
                    kind: kind.to_owned(),
                    position,
                    previous: position,
                    yaw: tracked_yaw,
                    pitch: tracked_pitch,
                    previous_yaw: tracked_yaw,
                    previous_pitch: tracked_pitch,
                    codec_base: position,
                    on_fire: false,
                    mod_fire_immune: false,
                    silent: false,
                    vehicle: None,
                    tick_count: 0,
                    size_answer: None,
                },
            );
        }
    }
}
