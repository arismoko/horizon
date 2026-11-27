use super::*;
use mod_api::{EntityRef, LevelRef, PlayerAbilities, PlayerRef};
use sunrise_command::args::GameMode;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::{
    Hit,
    commands_common::Target,
    entity::Entity as _,
    held_items::attack_reach,
    pick_entity_common::{AttackRangePickResult, CrosshairDecision, aabb_extents},
    pick_ray::{PickAccumulator, PickCandidate, pick_ray_segment},
    remote::movement::MovePacket,
    view_vector,
};
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    join_seq::residency::{Kind, Owner, Source},
    lan::playerdata::ints_uuid,
};
use sunrise_world::stepped_interpolation::PosRot;

fn v3(a: [f64; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

fn server_knot_bounds(block: [i32; 3]) -> Aabb {
    let knot = sunrise_sim::mobs::leash::knot_state(block);
    let p = knot.position;
    let half = knot.width / 2.0;
    Aabb::new(
        Vec3::new(p.x - half, p.y, p.z - half),
        Vec3::new(p.x + half, p.y + knot.height, p.z + half),
    )
}

impl crate::game::ServerGame<'_> {
    // Hands, inventory & equipment.
    pub(in crate::game) fn server_wear_glider(&mut self, points: u32) {
        let mut pieces: Vec<usize> = sunrise_gameplay_common::game::glide_wear::gliding_slots(
            self.common.res.items(),
            &self.common.armor,
            self.common.inventory[self.common.selected].as_ref(),
            self.common.offhand.as_ref(),
        )
        .collect();
        if pieces.is_empty() {
            return;
        }
        pieces.sort_unstable_by_key(|&slot| [5, 4, 3, 2, 0, 1][slot]);
        let pick = sunrise_level::random_source::entity().next_int_bound(pieces.len() as i32);
        let slot = pieces[pick.unwrap_or(0) as usize];
        let stack = match slot {
            4 => self.common.inventory[self.common.selected].clone(),
            5 => self.common.offhand.clone(),
            i => self.common.armor[i].clone(),
        };
        if let Some(stack) = stack
            && sunrise_sim::stations::is_damageable(self.common.res.items(), &stack)
        {
            let loss = self.wear(&stack, points);
            let target = match slot {
                4 => &mut self.common.inventory[self.common.selected],
                5 => &mut self.common.offhand,
                i => &mut self.common.armor[i],
            };
            if let Some(piece) = target.as_mut() {
                piece.damage += loss;
            }
        }
    }

    pub(in crate::game) fn server_on_inventory_item_added(&mut self, _stack: &ItemStack) {}

    pub(in crate::game) fn server_reset_attack_strength(&mut self) {
        self.server.attack_ticks = 0;
    }

    pub(in crate::game) fn server_main_hand_attack_speed(&self) -> f64 {
        if let Some(player) = &self.common.player
            && let Ok(speed) = player
                .attributes
                .computed_value_of("minecraft:attack_speed")
            && (!self.server.applied_modifiers.is_empty()
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
        let value = base
            + self
                .attack_item_info()
                .map_or(0.0, |i| f64::from(i.attack_speed));
        let value = self
            .common
            .potions
            .effects
            .modifiers()
            .into_iter()
            .filter(|m| m.0 == "minecraft:attack_speed")
            .fold(value, |v, m| v * (1.0 + m.2));
        self.common.sanitize_attack_speed(value)
    }

    pub(in crate::game) fn server_known_player_movement(&self) -> Vec3 {
        if let Some(i) = self
            .common
            .riding
            .and_then(|v| self.common.mobs.iter().position(|m| m.id == v))
        {
            return self.common.mob_speed(i).scale(1.0 / 20.0);
        }
        if self.server.lan_acting.is_some() {
            return self.server.known_movement;
        }
        self.common.player.as_ref().map_or(Vec3::ZERO, |p| {
            p.position.subtract(self.common.previous_position)
        })
    }

    pub(in crate::game) fn server_apply_equipment(&mut self) {
        if !self.server.lan_guest_swaps.is_empty() {
            return;
        }
        self.put_equipment_modifiers();
    }

    // Game mode & abilities.
    pub(in crate::game) fn server_may_build(&self) -> bool {
        self.server.acting_abilities.map_or(
            matches!(self.mode, GameMode::Survival | GameMode::Creative),
            |a| a.may_build,
        )
    }

    pub(in crate::game) fn server_instabuild(&self) -> bool {
        self.server
            .acting_abilities
            .map_or(self.common.creative, |a| a.instabuild)
    }

    pub(in crate::game) fn server_player_info_game_mode(&self) -> Option<GameMode> {
        Some(self.common.mode)
    }

    pub(in crate::game) fn server_player_abilities(
        &self,
        player: PlayerRef,
    ) -> Option<PlayerAbilities> {
        self.player_read_both(player)?.abilities()
    }

    /// Server handler for `Serverbound::ChangeGameMode` (called by `lan_actions/packets_server.rs`).
    pub(in crate::game::server_side) fn server_change_game_mode(&mut self, mode: GameMode) {
        if self.set_game_mode(mode) {
            let name = Text::translate(&format!("gameMode.{}", mode.name()), &[]);
            self.feedback(Text::translate("commands.gamemode.success.self", &[name]));
        }
    }

    // Player identity & residency.
    pub(in crate::game) fn server_own_entity_id(&self) -> i32 {
        self.common.player.as_ref().map_or(0, |p| p.entity_id)
    }

    /// Acting guest's UUID during a turn; legacy local body's saved UUID only when that body exists.
    pub(in crate::game) fn server_own_profile_uuid(&self) -> Option<u128> {
        self.server.lan_acting_uuid.or_else(|| {
            self.common
                .player
                .as_ref()
                .and(self.common.level.player_uuid)
                .map(ints_uuid)
        })
    }

    pub(in crate::game) fn server_own_profile_name(&self) -> String {
        self.acting_player_name().to_owned()
    }

    pub(in crate::game) fn server_local_player(&self) -> Option<PlayerRef> {
        None
    }

    pub(in crate::game) fn server_remove_local_player_body(&mut self) {
        self.common.hook.discard();
        self.common.resident_entities.local_player.retire();
        self.common.player_state.attachments.retire();
        self.common.player = None;
    }

    pub(in crate::game) fn server_bind_local_player_residency(&mut self) {
        if self.server.lan_acting.is_some() {
            return;
        }
        let journal = self.common.resident_entities.table.borrow().producer();
        let Some(player) = &self.common.player else {
            self.common.resident_entities.local_player.retire();
            return;
        };
        let id = player.entity_id as u32;
        let _ = self.common.resident_entities.local_player.bind(
            &journal,
            Owner {
                dimension: self.common.dim.key(),
                kind: Kind::Player,
            },
            id,
            Source::LocalPlayer,
            self.common.level.player_uuid,
        );
    }

    // Aim, raycast & targeting.
    pub(in crate::game) fn server_aim(&self) -> Option<([f64; 3], [f64; 3])> {
        let eye = self.common.eye_position()?;
        Some((eye, view_vector(self.common.yaw, self.common.pitch)))
    }

    pub(in crate::game) fn server_pick_unfiltered(&self) -> Option<Hit> {
        if let Some(hit) = self.server.forced_hit {
            return Some(hit);
        }
        let (origin, dir) = self.server_aim()?;
        self.clip_ray(
            origin,
            dir,
            self.common.block_reach.max(self.common.entity_reach),
        )
    }

    /// Visits every authoritative server pickable entity intersecting `search_box`.
    fn server_for_each_pickable_in_bounds(
        &self,
        search_box: Aabb,
        mut visit: impl FnMut(u32, Aabb, f64),
    ) {
        let mut emit = |id: u32, raw_bb: Aabb, radius: f64| {
            if !self.server_is_pick_camera(id) && raw_bb.intersects(search_box) {
                visit(id, raw_bb, radius);
            }
        };
        for m in self.common.mobs.iter() {
            if m.is_pickable() {
                emit(m.id, m.bounds(), 0.0);
            }
        }
        for b in self.common.boats.iter() {
            emit(b.id, b.bounds(), 0.0);
        }
        for c in self.common.carts.iter() {
            emit(c.id, c.bounds(), 0.0);
        }
        for a in self.common.armor_stands.iter() {
            if a.is_pickable() {
                emit(a.id, a.bounds(), 0.0);
            }
        }
        for t in self.common.tnts.iter() {
            emit(t.id, t.bounds(), 0.0);
        }
        for f in self.common.falling.iter() {
            emit(f.id, f.bounds(), 0.0);
        }
        for p in self.common.creatures.projectiles.iter() {
            if self.projectile_is_pickable(p) {
                emit(p.id, p.bounds(), self.projectile_pick_radius(p));
            }
        }
        for t in self.common.tridents.iter() {
            if self.trident_is_pickable(t) {
                emit(t.id, t.bounds(), 1.0);
            }
        }
        if self.rocket_is_pickable() {
            for r in self.common.rockets.iter() {
                emit(r.id, r.bounds(), 1.0);
            }
        }
        for row in self.common.resident_entities.general.rows.iter() {
            if row.is_pickable() {
                emit(row.id, row.bounds(), 0.0);
            }
        }
        for p in self.common.paintings.iter() {
            emit(p.id, aabb_extents(p.bounds()), 0.0);
        }
        for f in self.common.item_frames.iter() {
            emit(f.id, aabb_extents(f.bounds()), 0.0);
        }
        for c in self.common.cushions.iter() {
            emit(c.id, aabb_extents(c.bounds()), 0.0);
        }
        for k in self.common.resident_entities.knots.iter() {
            emit(k.id, server_knot_bounds(k.fence), 0.0);
        }
        for c in self.common.crystals.iter() {
            emit(c.id, c.bounds(), 0.0);
        }
        if let Some(d) = self
            .common
            .dragon
            .as_ref()
            .filter(|d| !self.server_is_pick_camera(d.id))
        {
            let has_wire_id = d.id != 0;
            for i in 0..8 {
                let part_id = if has_wire_id {
                    sunrise_gameplay_common::game::dragon_common::dragon_part_id(d.id, i)
                } else {
                    sunrise_gameplay_common::game::dragon_common::SPECIAL + i as u32
                };
                emit(part_id, d.part_geometry(i).1, 0.0);
            }
        }
    }

    /// Authoritative `AttackRange.getClosestHit` + block reach filter.
    fn server_attack_range_pick_result(&self, range: &serde_json::Value) -> AttackRangePickResult {
        let Some((eye_arr, look_arr)) = self.server_aim() else {
            return AttackRangePickResult::Miss;
        };
        let eye = v3(eye_arr);
        let look = v3(look_arr);
        let (min, max, margin) = attack_reach(range, self.own_player_is_creative());
        let movement = self.server_known_player_movement();
        let from = eye.add_vector(look.scale(f64::from(min)));
        let max_len = f64::from(max) + movement.dot(look).max(0.0);
        let mut to = eye.add_vector(look.scale(max_len));

        let filter_block = |hit: Hit| {
            let d2 = eye.distance_squared(v3(hit.point));
            if d2 < self.common.block_reach * self.common.block_reach {
                AttackRangePickResult::Block(hit)
            } else {
                AttackRangePickResult::Miss
            }
        };

        let block_hit = (max_len > 0.0)
            .then(|| self.clip_ray(eye_arr, look_arr, max_len))
            .flatten();
        if let Some(hit) = block_hit {
            to = v3(hit.point);
            if eye.distance_squared(to) < eye.distance_squared(from) {
                return filter_block(hit);
            }
        }

        let clip = |a: Vec3, b: Vec3| {
            let length = a.distance_squared(b).sqrt();
            if length <= 0.0 {
                return None;
            }
            let d = b.subtract(a).scale(1.0 / length);
            self.clip_ray([a.x, a.y, a.z], [d.x, d.y, d.z], length)
                .map(|hit| v3(hit.point))
        };

        let half = f64::from(margin) * 0.5;
        let search_area = Aabb::new(
            Vec3::new(from.x - half, from.y - half, from.z - half),
            Vec3::new(from.x + half, from.y + half, from.z + half),
        )
        .expand_towards(to.subtract(from))
        .inflate(Vec3::new(1.0, 1.0, 1.0));
        let grow = Vec3::new(f64::from(margin), f64::from(margin), f64::from(margin));

        let mut nearest: Option<PickCandidate> = None;
        self.server_for_each_pickable_in_bounds(search_area, |id, raw_bb, _radius| {
            let inside = raw_bb.contains(from);
            let hit_point = if inside {
                Some(from)
            } else if let Some(at) = raw_bb.clip(from, to) {
                Some(at)
            } else if margin > 0.0 {
                raw_bb.inflate(grow).clip(from, to).and_then(|outside| {
                    let centre = raw_bb.center();
                    let toward = clip(outside, centre).unwrap_or(centre);
                    raw_bb.clip(outside, toward)
                })
            } else {
                None
            };
            if let Some(at) = hit_point {
                let distance_sq = eye.distance_squared(at);
                if nearest.is_none_or(|best| distance_sq < best.distance_sq) {
                    nearest = Some(PickCandidate {
                        id,
                        point: [at.x, at.y, at.z],
                        distance_sq,
                        inside,
                    });
                }
            }
        });

        if let Some(hit) = nearest {
            AttackRangePickResult::Entity(hit)
        } else if let Some(hit) = block_hit {
            filter_block(hit)
        } else {
            AttackRangePickResult::Miss
        }
    }

    /// Authoritative server `compute_crosshair_decision` equivalent.
    pub(in crate::game::server_side) fn server_pick_ray_decision(&self) -> CrosshairDecision {
        if let Some(range) = self.common.active_attack_range() {
            match self.server_attack_range_pick_result(&range) {
                AttackRangePickResult::Entity(hit) => return CrosshairDecision::Entity(hit),
                AttackRangePickResult::Block(hit) => return CrosshairDecision::Block(hit),
                AttackRangePickResult::Miss => {}
            }
        }
        let Some((eye, dir)) = self.server_aim() else {
            return CrosshairDecision::Miss;
        };
        let from = v3(eye);
        let max_distance = self.common.block_reach.max(self.common.entity_reach);
        let block_hit = self.server_pick_unfiltered();
        let (entity_max_distance, search_max_distance_sq, block_distance_sq) = match block_hit {
            Some(block) => {
                let d2 = from.distance_squared(v3(block.point));
                (d2.sqrt(), d2, d2)
            }
            None => (
                max_distance,
                max_distance * max_distance,
                pick_ray_segment(eye, dir, max_distance)
                    .1
                    .distance_squared(from),
            ),
        };
        let (seg_from, seg_to) = pick_ray_segment(eye, dir, entity_max_distance);
        let search_box = self
            .common
            .player
            .as_ref()
            .map_or_else(|| Aabb::new(seg_from, seg_from), |p| p.bounds())
            .expand_towards(Vec3::new(
                dir[0] * entity_max_distance,
                dir[1] * entity_max_distance,
                dir[2] * entity_max_distance,
            ))
            .inflate(Vec3::new(1.0, 1.0, 1.0));
        let mut pick = PickAccumulator::new(search_max_distance_sq);
        self.server_for_each_pickable_in_bounds(search_box, |id, raw_bb, radius| {
            let bb = if radius == 0.0 {
                raw_bb
            } else {
                raw_bb.inflate(Vec3::new(radius, radius, radius))
            };
            pick.consider_box(self, id, seg_from, seg_to, bb);
        });
        if let Some(hit) = pick.hovered
            && hit.distance_sq < block_distance_sq
        {
            return if hit.distance_sq < self.common.entity_reach * self.common.entity_reach {
                CrosshairDecision::Entity(hit)
            } else {
                CrosshairDecision::Miss
            };
        }
        if let Some(block) = block_hit
            && from.distance_squared(v3(block.point))
                < self.common.block_reach * self.common.block_reach
        {
            CrosshairDecision::Block(block)
        } else {
            CrosshairDecision::Miss
        }
    }

    pub(in crate::game) fn server_pick(&self) -> Option<Hit> {
        if let Some(hit) = self.server.forced_hit {
            return Some(hit);
        }
        match self.server_pick_ray_decision() {
            CrosshairDecision::Block(hit) => Some(hit),
            _ => None,
        }
    }

    pub(in crate::game) fn server_mob_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        let CrosshairDecision::Entity(hit) = self.server_pick_ray_decision() else {
            return None;
        };
        let i = self.common.mob_index.find_ref(&self.common.mobs, hit.id)?;
        let point = v3(hit.point);
        let feet = self.common.mobs[i].position;
        Some((
            i,
            hit.distance_sq.sqrt(),
            [point.x - feet.x, point.y - feet.y, point.z - feet.z],
        ))
    }

    pub(in crate::game) fn server_block_distance(&self) -> Option<f64> {
        self.server_pick_unfiltered().map(|h| {
            let eye = self.server_aim().map(|(eye, _)| eye).unwrap_or_default();
            (0..3)
                .map(|a| (h.point[a] - eye[a]).powi(2))
                .sum::<f64>()
                .sqrt()
        })
    }

    pub(in crate::game) fn server_frame_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.server_aim()?;
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

    pub(in crate::game) fn server_boat_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.server_aim()?;
        let reach = self.common.entity_reach;
        let (from, to) = pick_ray_segment(eye, dir, reach);
        let mut pick = PickAccumulator::new(reach * reach);
        let mut best = None;
        for (i, b) in self.common.boats.iter().enumerate() {
            if pick.consider_box(self, b.id, from, to, b.bounds()) {
                best = Some((i, pick.hovered.unwrap().distance_sq.sqrt()));
            }
        }
        best
    }

    pub(in crate::game) fn server_cart_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.server_aim()?;
        let reach = self.common.entity_reach;
        let (from, to) = pick_ray_segment(eye, dir, reach);
        let mut pick = PickAccumulator::new(reach * reach);
        let mut best = None;
        for (i, c) in self.common.carts.iter().enumerate() {
            if pick.consider_box(self, c.id, from, to, c.bounds()) {
                best = Some((i, pick.hovered.unwrap().distance_sq.sqrt()));
            }
        }
        best
    }

    pub(in crate::game) fn server_armor_stand_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        let (eye, dir) = self.server_aim()?;
        let reach = self.common.entity_reach;
        let (from, to) = pick_ray_segment(eye, dir, reach);
        let mut pick = PickAccumulator::new(reach * reach);
        let mut best = None;
        for (i, a) in self.common.armor_stands.iter().enumerate() {
            if a.marker {
                continue;
            }
            if pick.consider_box(self, a.id, from, to, a.bounds()) {
                let hit = pick.hovered.unwrap();
                best = Some((i, hit.distance_sq.sqrt(), hit.point));
            }
        }
        best
    }

    pub(in crate::game) fn server_painting_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.server_aim()?;
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

    pub(in crate::game) fn server_dragon_hit(&self) -> Option<(u32, f64)> {
        let (eye, dir) = self.server_aim()?;
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

    pub(in crate::game) fn server_uses_stored_pick(&self) -> bool {
        false
    }

    fn server_pick_vehicle_step(&self, entity: u32) -> Option<u32> {
        if self.server_pick_camera_id() == Some(entity) {
            return self.common.riding;
        }
        self.vehicle_of_any(entity).or_else(|| {
            self.common
                .armor_stands
                .iter()
                .find(|a| a.id == entity)
                .and_then(|a| a.vehicle)
        })
    }

    fn server_pick_vehicle_root(&self, id: u32) -> Option<u32> {
        let mut slow = id;
        let mut fast = id;
        loop {
            let Some(next) = self.server_pick_vehicle_step(fast) else {
                return Some(fast);
            };
            let Some(next2) = self.server_pick_vehicle_step(next) else {
                return Some(next);
            };
            fast = next2;
            slow = self.server_pick_vehicle_step(slow)?;
            if slow == fast {
                return None;
            }
        }
    }

    pub(in crate::game) fn server_pick_shares_vehicle(&self, id: u32) -> bool {
        let actor_root = self
            .server_pick_camera_id()
            .and_then(|actor| self.server_pick_vehicle_root(actor))
            .or_else(|| {
                self.common
                    .riding
                    .and_then(|v| self.server_pick_vehicle_root(v))
            });
        actor_root.is_some() && self.server_pick_vehicle_root(id) == actor_root
    }

    pub(in crate::game) fn server_refresh_client_pick(&mut self, _partial: f32) {}
    #[allow(dead_code)]
    pub(in crate::game) fn server_release_tick_targets(&mut self) {}

    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::game) fn server_set_test_forced_hit(&mut self, hit: Option<Hit>) {
        self.server.forced_hit = hit;
    }

    // Client item-use hooks (no-ops on server).
    pub(in crate::game) fn server_reset_client_item_use(&mut self) {}
    pub(in crate::game) fn server_tick_client_item_use(&mut self) {}

    // Dual-role entity & level lookups.
    pub(in crate::game) fn server_client_display_position(
        &self,
        _entity: EntityRef,
    ) -> Option<[f64; 3]> {
        None
    }

    pub(in crate::game) fn server_entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        match self.native_entity_target(entity) {
            Some(target) => self.target_position(target),
            None => self
                .native_orb_ref(entity)
                .ok()
                .map(|orb| orb.position.components()),
        }
    }

    pub(in crate::game) fn server_client_copy_box(
        &self,
        _id: u32,
        _position: Vec3,
        vanilla: Aabb,
    ) -> Aabb {
        vanilla
    }

    pub(in crate::game) fn server_native_entity_read_target(
        &self,
        entity: EntityRef,
    ) -> Option<Target> {
        self.native_entity_target(entity)
    }

    pub(in crate::game) fn server_level(&self, dimension: &str) -> Option<LevelRef> {
        let epoch = self.common.native_lifecycle.epoch()?;
        let dim = (!self.server.levels.0.active_closed)
            .then_some(self.common.dim)
            .into_iter()
            .chain(self.server.levels.0.parked_dims())
            .find(|d| d.id() == dimension)?;
        Some(self.level_ref(epoch, dim.key()))
    }

    pub(in crate::game) fn server_native_current_level(&self) -> Option<LevelRef> {
        Some(self.level_ref(self.common.native_lifecycle.epoch()?, self.common.dim.key()))
    }

    // Remote motion client helpers (no-ops on server).
    pub(in crate::game) fn server_move_remote_interaction(
        &mut self,
        _id: i32,
        _position: [f64; 3],
        _has_path: bool,
        _rotation: Option<(f32, f32)>,
        _packet: MovePacket,
        _snap_old: Option<PosRot>,
    ) -> bool {
        false
    }
    pub(in crate::game) fn server_move_remote_hook(
        &mut self,
        _id: u32,
        _position: [f64; 3],
        _on_ground: bool,
    ) -> bool {
        false
    }
    pub(in crate::game) fn server_has_remote_text_display(&self, _id: i32) -> bool {
        false
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
