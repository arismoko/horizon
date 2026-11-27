use mod_api::{CrosshairTarget, EntityRef, LevelRef, PlayerAbilities, PlayerRef};
use sunrise_command::args::GameMode;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    join_seq::residency::{Kind, Owner, Source},
    lan::playerdata::uuid_ints,
};
use sunrise_world::stepped_interpolation::PosRot;
use {
    crate::game::client_side::interactions, crate::game::pick_entity::CrosshairDecision,
    crate::game::pick_entity::aabb_extents, crate::game::text_displays::Shows,
    crate::game::tutorial, sunrise_gameplay_common::game::Hit,
    sunrise_gameplay_common::game::commands_common::Target,
    sunrise_gameplay_common::game::dragon_common::SPECIAL,
    sunrise_gameplay_common::game::dragon_common::dragon_part_index,
    sunrise_gameplay_common::game::game_mode_common::mode_from_id,
    sunrise_gameplay_common::game::pick_ray::PickAccumulator,
    sunrise_gameplay_common::game::pick_ray::pick_ray_segment,
    sunrise_gameplay_common::game::remote::movement::MovePacket,
    sunrise_gameplay_common::game::view_vector,
};

impl crate::game::ClientGame<'_> {
    // Hands, inventory & equipment.
    pub(in crate::game) fn client_wear_glider(&mut self, _points: u32) {}

    pub(in crate::game) fn client_on_inventory_item_added(&mut self, stack: &ItemStack) {
        if matches!(
            self.client.tutorial.step,
            tutorial::Step::FindTree | tutorial::Step::PunchTree | tutorial::Step::CraftPlanks
        ) {
            self.tutorial_get_item(Some(stack));
        }
    }

    pub(in crate::game) fn client_reset_attack_strength(&mut self) {
        self.client.hand_strength.reset();
    }

    pub(in crate::game) fn client_main_hand_attack_speed(&self) -> f64 {
        if let Some(player) = &self.common.player
            && let Ok(speed) = player
                .attributes
                .computed_value_of("minecraft:attack_speed")
            && (self.client.remote.is_some()
                || !self.client.predicted_equipment_modifiers.is_empty()
                || self.common.potions.effects.get("haste").is_some()
                || self.common.potions.effects.get("mining_fatigue").is_some())
        {
            return speed;
        }
        let base = self
            .common
            .player
            .as_ref()
            .and_then(|p| {
                p.attributes
                    .computed_value_of("minecraft:attack_speed")
                    .ok()
            })
            .unwrap_or(sunrise_sim::attack_strength::BASE_ATTACK_SPEED);
        let held_speed = self
            .common
            .held()
            .and_then(|s| self.items().get(&s.item))
            .map_or(0.0, |i| f64::from(i.attack_speed));
        let value = self
            .common
            .potions
            .effects
            .modifiers()
            .into_iter()
            .filter(|m| m.0 == "minecraft:attack_speed")
            .fold(base + held_speed, |v, m| v * (1.0 + m.2));
        self.common.sanitize_attack_speed(value)
    }

    pub(in crate::game) fn client_known_player_movement(&self) -> Vec3 {
        if let Some(i) = self
            .common
            .riding
            .and_then(|v| self.common.mobs.iter().position(|m| m.id == v))
        {
            return self.common.mob_speed(i).scale(1.0 / 20.0);
        }
        self.common.player.as_ref().map_or(Vec3::ZERO, |p| {
            p.position.subtract(self.common.previous_position)
        })
    }

    pub(in crate::game) fn client_apply_equipment(&mut self) {
        let mut applied = std::mem::take(&mut self.client.predicted_equipment_modifiers);
        let ran = self.apply_equipment_modifiers_with(&mut applied);
        self.client.predicted_equipment_modifiers = applied;
        if ran && let Some(player) = self.common.player.as_mut() {
            let _ = player.clamp_health_to_max();
        }
    }

    // Game mode & abilities.
    pub(in crate::game) fn client_may_build(&self) -> bool {
        self.remote_abilities().is_some_and(|a| a.may_build)
    }

    pub(in crate::game) fn client_instabuild(&self) -> bool {
        self.remote_abilities().is_some_and(|a| a.instabuild)
    }

    pub(in crate::game) fn client_player_info_game_mode(&self) -> Option<GameMode> {
        self.client
            .remote
            .as_ref()
            .and_then(|r| r.own_player_info_game_mode().map(mode_from_id))
    }

    pub(in crate::game) fn client_player_abilities(
        &self,
        player: PlayerRef,
    ) -> Option<PlayerAbilities> {
        if self.common.native_lifecycle.epoch() != Some(player.entity().epoch()) {
            return None;
        }
        let remote = self.client.remote.as_ref().filter(|r| r.logged_in)?;
        if remote.profile() != player.uuid()
            || u32::try_from(remote.entity_id).ok()? != player.entity().id()
        {
            return None;
        }
        let mut a = *self.remote_abilities()?;
        if let Some(body) = self.common.player.as_ref() {
            a.flying = body.flying;
            a.flying_speed = body.flying_speed;
        }
        Some(PlayerAbilities::from(
            mod_api::__private::built::PlayerAbilities {
                invulnerable: a.invulnerable,
                flying: a.flying,
                may_fly: a.may_fly,
                instabuild: a.instabuild,
                may_build: a.may_build,
                fly_speed: a.flying_speed,
                walk_speed: a.walking_speed,
            },
        ))
    }

    // Player identity & residency.
    pub(in crate::game) fn client_own_entity_id(&self) -> i32 {
        self.client.remote.as_ref().map_or_else(
            || self.common.player.as_ref().map_or(0, |p| p.entity_id),
            |r| r.entity_id,
        )
    }

    pub(in crate::game) fn client_own_profile_uuid(&self) -> Option<u128> {
        self.client.remote.as_ref().map(|r| r.profile())
    }

    pub(in crate::game) fn client_own_profile_name(&self) -> String {
        self.client
            .remote
            .as_ref()
            .filter(|r| !r.profile_name.is_empty())
            .map_or_else(
                || self.common.options.player_name(),
                |r| r.profile_name.clone(),
            )
    }

    /// Preserves checked `u32::try_from(remote.entity_id)` from `native_server.rs::player`.
    pub(in crate::game) fn client_local_player(&self) -> Option<PlayerRef> {
        let epoch = self.common.native_lifecycle.epoch()?;
        let remote = self.client.remote.as_ref().filter(|r| r.logged_in)?;
        let id = u32::try_from(remote.entity_id).ok()?;
        Some(mod_api::__private::game::player_ref(
            EntityRef::new(epoch, id),
            remote.profile(),
        ))
    }

    pub(in crate::game) fn client_remove_local_player_body(&mut self) {
        self.reset_client_item_use();
        self.common.hook.discard();
        let id = self
            .common
            .player
            .as_ref()
            .map(|player| player.entity_id as u32);
        self.common.resident_entities.local_player.retire();
        self.common.player_state.attachments.retire();
        self.common.player = None;
        if self.client.remote.is_some()
            && let Some(id) = id
        {
            self.pick_section_remove(id);
        }
    }

    pub(in crate::game) fn client_bind_local_player_residency(&mut self) {
        let journal = self.common.resident_entities.table.borrow().producer();
        let Some(player) = &self.common.player else {
            self.common.resident_entities.local_player.retire();
            return;
        };
        let id = player.entity_id as u32;
        let old = self.common.resident_entities.local_player.identity();
        let admitted = self.common.resident_entities.local_player.bind(
            &journal,
            Owner {
                dimension: self.common.dim.key(),
                kind: Kind::Player,
            },
            id,
            Source::LocalPlayer,
            self.client
                .remote
                .as_ref()
                .filter(|remote| remote.logged_in)
                .map(|remote| uuid_ints(remote.profile())),
        );
        if self.client.remote.is_some() && admitted {
            if let Some(old) = old.filter(|old| old.id.get() != id) {
                self.pick_section_remove(old.id.get());
            }
            self.pick_section_sync_id(id);
        }
        if let Some(remote) = self
            .client
            .remote
            .as_ref()
            .filter(|remote| remote.logged_in)
            && let Some(epoch) = self.common.native_lifecycle.epoch()
            && let Ok(wire_id) = u32::try_from(remote.entity_id)
        {
            remote
                .session
                .payload_transport()
                .bind_player(mod_api::__private::game::player_ref(
                    EntityRef::new(epoch, wire_id),
                    remote.profile(),
                ));
        }
    }

    // Aim, raycast & targeting.
    /// Verbatim body of live `game_client.rs::aim`.
    pub(in crate::game) fn client_aim(&self) -> Option<([f64; 3], [f64; 3])> {
        if let Some(ray) = self.client.client_pick.ray {
            return Some(ray);
        }
        if let Some(e) = self.spectated_camera_snapshot() {
            let eye = [
                e.position[0],
                e.position[1] + f64::from(e.eye_height),
                e.position[2],
            ];
            return Some((eye, view_vector(e.y_rot, e.x_rot)));
        }
        let eye = self.common.eye_position()?;
        Some((eye, view_vector(self.common.yaw, self.common.pitch)))
    }

    pub(in crate::game) fn client_pick_unfiltered(&self) -> Option<Hit> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hit) = self.client.client_pick.test_forced_hit {
            return Some(hit);
        }
        let (origin, dir) = self.client_aim()?;
        self.clip_ray(
            origin,
            dir,
            self.common.block_reach.max(self.common.entity_reach),
        )
    }

    pub(in crate::game) fn client_pick(&self) -> Option<Hit> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hit) = self.client.client_pick.test_forced_hit {
            return Some(hit);
        }
        if self.uses_stored_pick() {
            return match self.crosshair_target() {
                Some(CrosshairTarget::Block { pos, hit, .. }) => Some(Hit {
                    pos: [pos.x, pos.y, pos.z],
                    face: hit.face,
                    point: hit.point,
                    inside: hit.inside,
                }),
                _ => None,
            };
        }
        if let Some(hit) = self.client.tick_hit.get() {
            return hit;
        }
        match self.compute_crosshair_decision() {
            CrosshairDecision::Block(hit) => Some(hit),
            _ => None,
        }
    }

    pub(in crate::game) fn client_mob_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, point) = hit?;
            let index = self.common.mobs.iter().position(|m| m.id == entity.id())?;
            let feet = self.common.mobs[index].position;
            return Some((
                index,
                distance,
                [point[0] - feet.x, point[1] - feet.y, point[2] - feet.z],
            ));
        }
        if let Some(hit) = self.client.tick_mob {
            return hit;
        }
        let CrosshairDecision::Entity(hit) = self.compute_crosshair_decision() else {
            return None;
        };
        let i = self.common.mob_index.find_ref(&self.common.mobs, hit.id)?;
        let feet = self.common.mobs[i].position;
        Some((
            i,
            hit.distance_sq.sqrt(),
            [
                hit.point[0] - feet.x,
                hit.point[1] - feet.y,
                hit.point[2] - feet.z,
            ],
        ))
    }

    pub(in crate::game) fn client_block_distance(&self) -> Option<f64> {
        if self.uses_stored_pick() {
            return match self.crosshair_target() {
                Some(CrosshairTarget::Block { hit, .. }) => Some(self.pick_distance(hit.point)),
                _ => None,
            };
        }
        self.client_pick_unfiltered().map(|h| {
            let eye = self.client_aim().map(|(eye, _)| eye).unwrap_or_default();
            (0..3)
                .map(|a| (h.point[a] - eye[a]).powi(2))
                .sum::<f64>()
                .sqrt()
        })
    }

    pub(in crate::game) fn client_frame_hit(&self) -> Option<(usize, f64)> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, _) = hit?;
            let index = self
                .common
                .item_frames
                .iter()
                .position(|f| f.id == entity.id())?;
            return Some((index, distance));
        }
        let (eye, dir) = self.client_aim()?;
        let reach = self.common.entity_reach;
        let (from, to) = pick_ray_segment(eye, dir, reach);
        let mut pick = PickAccumulator::new(reach * reach);
        let mut best = None;
        for (i, f) in self.common.item_frames.iter().enumerate() {
            if pick.consider_box(self, f.id, from, to, aabb_extents(f.bounds())) {
                best = Some((i, pick.hovered.unwrap().distance_sq.sqrt()));
            }
        }
        best
    }

    pub(in crate::game) fn client_boat_hit(&self) -> Option<(usize, f64)> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, _) = hit?;
            let index = self.common.boats.iter().position(|f| f.id == entity.id())?;
            return Some((index, distance));
        }
        let (eye, dir) = self.client_aim()?;
        let reach = self.common.entity_reach;
        let (from, to) = pick_ray_segment(eye, dir, reach);
        let mut pick = PickAccumulator::new(reach * reach);
        let mut best = None;
        for (i, b) in self.common.boats.iter().enumerate() {
            if pick.consider_box(
                self,
                b.id,
                from,
                to,
                self.client_copy_box_inner(b.id, b.position, b.bounds()),
            ) {
                best = Some((i, pick.hovered.unwrap().distance_sq.sqrt()));
            }
        }
        best
    }

    pub(in crate::game) fn client_cart_hit(&self) -> Option<(usize, f64)> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, _) = hit?;
            let index = self.common.carts.iter().position(|f| f.id == entity.id())?;
            return Some((index, distance));
        }
        let (eye, dir) = self.client_aim()?;
        let reach = self.common.entity_reach;
        let (from, to) = pick_ray_segment(eye, dir, reach);
        let mut pick = PickAccumulator::new(reach * reach);
        let mut best = None;
        for (i, c) in self.common.carts.iter().enumerate() {
            if pick.consider_box(
                self,
                c.id,
                from,
                to,
                self.client_copy_box_inner(c.id, c.position, c.bounds()),
            ) {
                best = Some((i, pick.hovered.unwrap().distance_sq.sqrt()));
            }
        }
        best
    }

    pub(in crate::game) fn client_armor_stand_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, point) = hit?;
            let index = self
                .common
                .armor_stands
                .iter()
                .position(|f| f.id == entity.id())?;
            return Some((index, distance, point));
        }
        let (eye, dir) = self.client_aim()?;
        let reach = self.common.entity_reach;
        let (from, to) = pick_ray_segment(eye, dir, reach);
        let mut pick = PickAccumulator::new(reach * reach);
        let mut best = None;
        for (i, a) in self.common.armor_stands.iter().enumerate() {
            if a.marker {
                continue;
            }
            if pick.consider_box(
                self,
                a.id,
                from,
                to,
                self.client_copy_box_inner(a.id, a.position, a.bounds()),
            ) {
                let hit = pick.hovered.unwrap();
                best = Some((i, hit.distance_sq.sqrt(), hit.point));
            }
        }
        best
    }

    pub(in crate::game) fn client_painting_hit(&self) -> Option<(usize, f64)> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, _) = hit?;
            let index = self
                .common
                .paintings
                .iter()
                .position(|f| f.id == entity.id())?;
            return Some((index, distance));
        }
        let (eye, dir) = self.client_aim()?;
        let reach = self.common.entity_reach;
        let (from, to) = pick_ray_segment(eye, dir, reach);
        let mut pick = PickAccumulator::new(reach * reach);
        let mut best = None;
        for (i, p) in self.common.paintings.iter().enumerate() {
            if pick.consider_box(self, p.id, from, to, aabb_extents(p.bounds())) {
                best = Some((i, pick.hovered.unwrap().distance_sq.sqrt()));
            }
        }
        best
    }

    pub(in crate::game) fn client_dragon_hit(&self) -> Option<(u32, f64)> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, _) = hit?;
            let id = entity.id();
            if let Some(d) = self
                .common
                .dragon
                .as_ref()
                .filter(|d| d.id != 0 || self.client.remote.is_some())
                && let Some(part) = dragon_part_index(d.id, id)
            {
                return Some((SPECIAL + part as u32, distance));
            }
            return self
                .common
                .special_targets()
                .into_iter()
                .any(|(special, _)| special == id)
                .then_some((id, distance));
        }
        let (eye, dir) = self.client_aim()?;
        let reach = self.common.entity_reach;
        let (from, to) = pick_ray_segment(eye, dir, reach);
        let mut pick = PickAccumulator::new(reach * reach);
        let mut best = None;
        for (id, b) in self.common.special_targets() {
            if pick.consider_box(self, id, from, to, b) {
                best = Some((id, pick.hovered.unwrap().distance_sq.sqrt()));
            }
        }
        best
    }

    pub(in crate::game) fn client_uses_stored_pick(&self) -> bool {
        self.uses_stored_pick()
    }

    pub(in crate::game) fn client_pick_shares_vehicle(&self, id: u32) -> bool {
        self.pick_shares_vehicle(id)
    }

    pub(in crate::game) fn client_refresh_client_pick(&mut self, partial: f32) {
        self.refresh_client_pick(partial);
    }

    #[allow(dead_code)]
    pub(in crate::game) fn client_release_tick_targets(&mut self) {
        self.release_tick_targets();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::game) fn client_set_test_forced_hit(&mut self, hit: Option<Hit>) {
        self.client.client_pick.test_forced_hit = hit;
    }

    // Client item-use hooks.
    pub(in crate::game) fn client_reset_client_item_use(&mut self) {
        self.reset_client_item_use();
    }
    pub(in crate::game) fn client_tick_client_item_use(&mut self) {
        self.tick_client_item_use();
    }

    // Dual-role entity & level lookups.
    /// Replaces `native_client_display`/`live_client_display` borrow bridge with owned `[f64; 3]`.
    pub(in crate::game) fn client_client_display_position(
        &self,
        entity: EntityRef,
    ) -> Option<[f64; 3]> {
        let (remote, id) = self.native_client_entity_context(entity).ok()?;
        if !remote.has_level() {
            return None;
        }
        Shows::from_kind(remote.tracked(id)?.0)?;
        self.client
            .text_displays
            .0
            .get(&id)
            .map(|d| d.position().to_array())
    }

    pub(in crate::game) fn client_entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let Some(target) = self.native_client_entity_target(entity) else {
            return self.native_client_tracked(entity).map(|(_, pos)| pos);
        };
        match target {
            Target::Player => self
                .common
                .player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z]),
            Target::Guest(_) => None,
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| [m.position.x, m.position.y, m.position.z]),
            Target::Item(id) => self
                .common
                .dropped
                .iter()
                .find(|i| i.id == id)
                .map(|i| [i.position.x, i.position.y, i.position.z]),
            Target::Orb(id) => self
                .common
                .orbs
                .iter()
                .find(|o| o.id == id)
                .map(|o| [o.position.x, o.position.y, o.position.z]),
            Target::Projectile(id) => self
                .common
                .creatures
                .projectiles
                .iter()
                .find(|a| a.id == id)
                .map(|a| [a.position.x, a.position.y, a.position.z]),
            Target::Stand(id) => self
                .common
                .armor_stands
                .iter()
                .find(|a| a.id == id)
                .map(|a| [a.position.x, a.position.y, a.position.z]),
            Target::Frame(id) => self
                .common
                .item_frames
                .iter()
                .find(|f| f.id == id)
                .map(|f| {
                    let (lo, hi) = f.bounds();
                    [
                        (lo[0] + hi[0]) / 2.0,
                        (lo[1] + hi[1]) / 2.0,
                        (lo[2] + hi[2]) / 2.0,
                    ]
                }),
            Target::Painting(id) => self.common.paintings.iter().find(|p| p.id == id).map(|p| {
                let (lo, hi) = p.bounds();
                [
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                ]
            }),
            Target::Boat(id) => self
                .common
                .boats
                .iter()
                .find(|b| b.id == id)
                .map(|b| [b.position.x, b.position.y, b.position.z]),
            Target::Cart(id) => self
                .common
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
            Target::Other(_) => self.native_client_tracked(entity).map(|(_, pos)| pos),
            Target::Crystal(id) => self
                .common
                .crystals
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
            Target::Dragon(id) => self
                .common
                .dragon
                .as_ref()
                .filter(|d| d.id == id)
                .map(|d| [d.position.x, d.position.y, d.position.z]),
            Target::Cushion(id) => self
                .common
                .cushions
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
            Target::DragonPart(id) => self.common.dragon.as_ref().and_then(|d| {
                dragon_part_index(d.id, id).map(|i| {
                    let p = d.parts[i].0;
                    [p.x, p.y, p.z]
                })
            }),
            Target::Tnt(id) => self
                .common
                .tnts
                .iter()
                .find(|t| t.id == id)
                .map(|t| [t.position.x, t.position.y, t.position.z]),
            Target::Falling(id) => self
                .common
                .falling
                .iter()
                .find(|f| f.id == id)
                .map(|f| [f.position.x, f.position.y, f.position.z]),
            Target::Rocket(id) => self
                .common
                .rockets
                .iter()
                .find(|r| r.id == id)
                .map(|r| [r.position.x, r.position.y, r.position.z]),
            Target::Cloud(id) => self
                .common
                .breath_clouds
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.at),
            Target::Bolt(id) => self.common.bolts.iter().find(|b| b.id == id).map(|b| b.at),
            Target::Hook(id) => self
                .common
                .hook
                .as_ref()
                .filter(|h| h.id() == id)
                .map(|h| [h.position.x, h.position.y, h.position.z])
                .or_else(|| {
                    self.client
                        .remote_hooks
                        .iter()
                        .find(|h| h.id() == id)
                        .map(|h| [h.position.x, h.position.y, h.position.z])
                }),
            Target::Trident(id) => self
                .common
                .tridents
                .iter()
                .find(|t| t.id == id)
                .map(|t| [t.position.x, t.position.y, t.position.z]),
            Target::Fangs(id) => self
                .common
                .fangs
                .list
                .iter()
                .find(|f| f.id == id)
                .map(|f| f.pos),
            Target::Spawner(id) => self
                .common
                .ominous_spawners
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.position),
            Target::Knot(id) => self
                .client
                .remote_knots
                .get(&id)
                .copied()
                .or_else(|| {
                    self.common
                        .resident_entities
                        .knots
                        .iter()
                        .find(|k| k.id == id)
                        .map(|k| k.fence)
                })
                .map(|fence| {
                    let p = sunrise_sim::mobs::leash::knot_state(fence).position;
                    [p.x, p.y, p.z]
                }),
            Target::General(id) => self
                .common
                .resident_entities
                .general
                .row(id)
                .map(|row| [row.position.x, row.position.y, row.position.z])
                .or_else(|| {
                    self.client
                        .remote
                        .as_ref()
                        .and_then(|r| r.general.rows.get(&(id as i32)))
                        .map(|row| row.position)
                }),
        }
    }

    pub(in crate::game) fn client_client_copy_box(
        &self,
        id: u32,
        position: Vec3,
        vanilla: Aabb,
    ) -> Aabb {
        self.client_copy_box_inner(id, position, vanilla)
    }

    pub(in crate::game) fn client_native_entity_read_target(
        &self,
        entity: EntityRef,
    ) -> Option<Target> {
        self.native_client_entity_target(entity)
    }

    pub(in crate::game) fn client_level(&self, dimension: &str) -> Option<LevelRef> {
        self.client_level_by_name(dimension)
    }

    pub(in crate::game) fn client_native_current_level(&self) -> Option<LevelRef> {
        self.native_client_level()
    }

    // Remote motion client helpers.
    pub(in crate::game) fn client_move_remote_interaction(
        &mut self,
        id: i32,
        position: [f64; 3],
        has_path: bool,
        rotation: Option<(f32, f32)>,
        packet: MovePacket,
        snap_old: Option<PosRot>,
    ) -> bool {
        let Some(i) = self.client.interactions.get_mut(&id) else {
            return false;
        };
        let target_pos = Vec3::new(position[0], position[1], position[2]);
        let d = target_pos.subtract(i.position);
        let snap = matches!(packet, MovePacket::PositionSyncSnap)
            || (matches!(packet, MovePacket::Teleport | MovePacket::PositionSync)
                && d.x * d.x + d.y * d.y + d.z * d.z > 4096.0);
        if snap {
            let (yaw, pitch) = rotation.unwrap_or((i.yaw, i.pitch));
            interactions::snap_to(i, target_pos, yaw, pitch);
            if let Some(old) = snap_old {
                i.previous = Vec3::new(old.position[0], old.position[1], old.position[2]);
                i.previous_yaw = if old.yaw.is_finite() { old.yaw } else { i.yaw };
                i.previous_pitch = if old.pitch.is_finite() {
                    (old.pitch % 360.0).clamp(-90.0, 90.0)
                } else {
                    i.pitch
                };
            }
        } else {
            if has_path {
                i.position = target_pos;
            }
            if let Some((yaw, pitch)) = rotation {
                interactions::set_rot(i, yaw, pitch);
            }
        }
        true
    }

    pub(in crate::game) fn client_move_remote_hook(
        &mut self,
        id: u32,
        position: [f64; 3],
        on_ground: bool,
    ) -> bool {
        let Some(h) = self.client.remote_hooks.iter_mut().find(|h| h.id() == id) else {
            return false;
        };
        h.move_to(position, on_ground);
        true
    }

    pub(in crate::game) fn client_has_remote_text_display(&self, id: i32) -> bool {
        self.client.text_displays.0.contains_key(&id)
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
