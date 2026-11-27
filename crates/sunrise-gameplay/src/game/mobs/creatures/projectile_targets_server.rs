use super::super::{commands::Target, entity::Entity as _, lan_mobs::guest_bounds};
use super::ProjWorld;
use crate::game::server_side::minecarts_server::CartHit;
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use sunrise_gameplay_common::game::creatures::projectile_targets::NonMobHit;
use sunrise_gameplay_common::game::creatures::projectile_targets::ProjectileHitInfo;
use sunrise_gameplay_common::game::creatures::projectile_targets::TargetQuery;
use sunrise_gameplay_common::game::creatures::projectile_targets::extents;
use sunrise_gameplay_common::game::creatures::projectile_targets::is_pickable_projectile;
use sunrise_gameplay_common::game::creatures::projectile_targets::knot_bounds;
use sunrise_gameplay_common::game::deflect_common::redirectable;
use sunrise_sim::entities::arrows::ArrowTargets;
use sunrise_sim::mobs::{MobWorld, Who};
use sunrise_sim::projectiles::{
    Projectile, ProjectileKind, ProjectileWorld, TargetCandidate, TargetOrder,
};

/// Zero-allocation live world view for `ProjectileWorld` and `ArrowTargets`.
pub(in crate::game) struct LiveProjWorld<'a, 'r> {
    pub game: &'a crate::game::ServerGame<'r>,
    pub w: MobWorld<'a>,
    pub border: Option<[f64; 4]>,
    pub query: TargetQuery,
    pub extra_projectiles: &'a [TargetCandidate],
}

impl<'a, 'r> LiveProjWorld<'a, 'r> {
    pub(in crate::game) fn base(&self) -> ProjWorld<'_, 'a> {
        ProjWorld {
            w: &self.w,
            targets: &[],
            border: self.border,
        }
    }
}

impl ProjectileWorld for LiveProjWorld<'_, '_> {
    fn clip(&self, from: Vec3, to: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
        self.base().clip(from, to)
    }
    fn in_water(&self, bounds: Aabb) -> bool {
        self.base().in_water(bounds)
    }
    fn in_lava_box(&self, bounds: Aabb) -> bool {
        self.base().in_lava_box(bounds)
    }
    fn pushed_by_currents(&self, bounds: Aabb, velocity: Vec3) -> Vec3 {
        self.base().pushed_by_currents(bounds, velocity)
    }
    fn targets(&self) -> &[(Who, Aabb)] {
        &[]
    }
    fn visit_targets(&self, visit: impl FnMut(TargetCandidate)) {
        self.game
            .visit_projectile_targets(self.query, self.extra_projectiles, visit);
    }
    fn owner_tree_contains(&self, owner: Who, candidate: Who) -> bool {
        self.game.in_same_vehicle_tree(owner, candidate)
    }
    fn outside_owner_collision_range(&self, owner: Who, swept: Aabb) -> bool {
        self.game.outside_owner_collision_range(owner, swept)
    }
    fn target_bounds(&self, target: Who) -> Option<Aabb> {
        self.game.live_target_bounds(target)
    }
    fn below_world_y(&self) -> f64 {
        self.base().below_world_y()
    }
    fn empty(&self, p: [i32; 3]) -> bool {
        self.base().empty(p)
    }
    fn can_stand_on(&self, p: [i32; 3], entity_y: f64) -> bool {
        self.base().can_stand_on(p, entity_y)
    }
    fn bubble_column(&self, p: [i32; 3]) -> Option<(bool, bool)> {
        self.base().bubble_column(p)
    }
    fn stuck_speed(&self, p: [i32; 3]) -> Option<Vec3> {
        self.base().stuck_speed(p)
    }
}

impl ArrowTargets for LiveProjWorld<'_, '_> {
    fn visit(&self, visit: impl FnMut(TargetCandidate)) {
        self.game
            .visit_projectile_targets(self.query, self.extra_projectiles, visit);
    }
    fn owner_tree_contains(&self, owner: Who, candidate: Who) -> bool {
        self.game.in_same_vehicle_tree(owner, candidate)
    }
    fn outside_owner_collision_range(&self, owner: Who, swept: Aabb) -> bool {
        self.game.outside_owner_collision_range(owner, swept)
    }
}

impl<'r> crate::game::ServerGame<'r> {
    /// Construct a live zero-allocation world view for projectile sweeps.
    pub(in crate::game) fn live_proj_world<'a>(
        &'a self,
        query: TargetQuery,
        extra_projectiles: &'a [TargetCandidate],
    ) -> LiveProjWorld<'a, 'r> {
        LiveProjWorld {
            game: self,
            w: self.mob_world_base(&[]),
            border: Some(self.common.border.bounds()),
            query,
            extra_projectiles,
        }
    }

    /// Resolve a LAN guest connection id from their entity id.
    pub(in crate::game) fn lan_guest_conn_by_id(&self, id: u32) -> Option<u32> {
        self.server.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .iter()
                .find(|g| g.entity_id as u32 == id)
                .map(|g| g.conn)
        })
    }

    /// Exact vanilla damage type id for `shot`, using the live resolved owner.
    pub(in crate::game) fn projectile_damage_type(&self, shot: &Projectile) -> &'static str {
        let has_owner = self
            .resolved_projectile_owner(shot.owner, shot.guest)
            .is_some();
        match shot.kind {
            ProjectileKind::Arrow { .. } => "minecraft:arrow",
            ProjectileKind::Trident => "minecraft:trident",
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                if has_owner {
                    "minecraft:fireball"
                } else {
                    "minecraft:unattributed_fireball"
                }
            }
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge => {
                "minecraft:wind_charge"
            }
            ProjectileKind::LlamaSpit => "minecraft:spit",
            ProjectileKind::ShulkerBullet { .. } => "minecraft:mob_projectile",
            ProjectileKind::WitherSkull { .. } => {
                if has_owner {
                    "minecraft:wither_skull"
                } else {
                    "minecraft:magic"
                }
            }
            _ => "minecraft:thrown",
        }
    }

    /// Immediate vehicle (`Entity.getVehicle`) of `who`, if any.
    pub(in crate::game) fn mount_of(&self, who: Who) -> Option<Who> {
        match who {
            Who::Player => self.common.riding.map(Who::Mob),
            Who::Mob(id) => self
                .server
                .lan
                .as_ref()
                .and_then(|h| h.hub.guests.iter().find(|g| g.entity_id as u32 == id))
                .and_then(|g| g.vehicle)
                .or_else(|| {
                    self.common
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .and_then(|m| m.extra.mount)
                })
                .or_else(|| {
                    self.common
                        .armor_stands
                        .iter()
                        .find(|a| a.id == id)
                        .and_then(|a| a.vehicle)
                })
                .or_else(|| {
                    self.common
                        .resident_entities
                        .general
                        .row(id)
                        .and_then(|r| r.mount)
                })
                .map(Who::Mob),
        }
    }

    /// `Entity.getRootVehicle`: climb vehicle pointers to the root entity.
    pub(in crate::game) fn root_vehicle_of(&self, mut who: Who) -> Who {
        for _ in 0..16 {
            let Some(next) = self.mount_of(who) else {
                break;
            };
            if next == who {
                break;
            }
            who = next;
        }
        who
    }

    /// `Entity.isPassengerOfSameVehicle`: whether `a` and `b` share the same root vehicle tree.
    pub(in crate::game) fn in_same_vehicle_tree(&self, a: Who, b: Who) -> bool {
        a == b || self.root_vehicle_of(a) == self.root_vehicle_of(b)
    }

    /// `Projectile.isOutsideOwnerCollisionRange`: true when `swept` touches no pickable entity
    /// in `owner.getRootVehicle().getSelfAndPassengers()`.
    pub(in crate::game) fn outside_owner_collision_range(&self, owner: Who, swept: Aabb) -> bool {
        let root = self.root_vehicle_of(owner);
        let mut outside = true;
        self.visit_projectile_targets(
            TargetQuery {
                include_guests: true,
                ..TargetQuery::default()
            },
            &[],
            |c| {
                if outside && c.bounds.intersects(swept) && self.root_vehicle_of(c.who) == root {
                    outside = false;
                }
            },
        );
        outside
    }

    /// Current bounding box of a live target (for `ShulkerBullet` homing).
    pub(in crate::game) fn live_target_bounds(&self, target: Who) -> Option<Aabb> {
        match target {
            Who::Player => self
                .player_view()
                .filter(|p| p.alive && !self.common.spectator())
                .map(|p| p.bounds),
            Who::Mob(id) => {
                if let Some(m) = self
                    .common
                    .mobs
                    .iter()
                    .find(|m| m.id == id && m.alive() && !m.removed)
                {
                    return Some(m.bounds());
                }
                if let Some(host) = self.server.lan.as_ref()
                    && let Some(g) = host.hub.guests.iter().find(|g| {
                        g.entity_id as u32 == id
                            && host.hub.here(g)
                            && g.loaded
                            && !g.dead
                            && !g.spectator()
                    })
                {
                    return Some(guest_bounds(g));
                }
                if let Some(c) = self.common.crystals.iter().find(|c| c.id == id) {
                    return Some(c.bounds());
                }
                if id >= sunrise_gameplay_common::game::dragon_common::SPECIAL {
                    let part =
                        (id - sunrise_gameplay_common::game::dragon_common::SPECIAL) as usize;
                    return self
                        .common
                        .dragon_part_boxes()
                        .and_then(|boxes| boxes.get(part).copied());
                }
                if let Some(a) = self.common.armor_stands.iter().find(|a| a.id == id) {
                    return Some(a.bounds());
                }
                if let Some(b) = self.common.boats.iter().find(|b| b.id == id) {
                    return Some(b.bounds());
                }
                if let Some(c) = self.common.carts.iter().find(|c| c.id == id) {
                    return Some(c.bounds());
                }
                None
            }
        }
    }

    /// `Projectile.onRedirectProjectile` (`AIM_DEFLECT`, or `MOMENTUM_DEFLECT` for `ShulkerBullet`).
    pub(in crate::game) fn redirect_projectile(
        &mut self,
        target_id: u32,
        hitter_owner: Option<Who>,
        hitter_owner_uuid: Option<[i32; 4]>,
        hitter_guest: Option<u32>,
        momentum: Option<Vec3>,
    ) {
        let Some(j) = self
            .common
            .creatures
            .projectiles
            .iter()
            .position(|q| q.id == target_id && redirectable(q))
        else {
            return;
        };
        let dir = match momentum {
            Some(v) => (v.length_squared() > 0.0).then(|| v.normalize()),
            None => self.owner_look(hitter_owner, hitter_guest),
        };
        self.common.creatures.projectiles.update(j, |target| {
            target.deflect(
                sunrise_sim::projectiles::Turn::Direction(dir),
                [1.0; 3],
                (hitter_owner, hitter_owner_uuid, hitter_guest),
                true,
            );
        });
    }

    /// `AbstractArrow.canHitEntity`: player-on-player check (`owner.canHarmPlayer(target)`).
    pub fn can_arrow_harm_player(&self, owner: Who, owner_guest: Option<u32>, target: Who) -> bool {
        // A player by entity id with their team; none when `who` is no player.
        let guest = |conn: u32| {
            self.server
                .lan
                .as_ref()
                .and_then(|h| h.hub.guest(conn))
                .map(|g| (g.entity_id, self.common.score.board.players_team(&g.name)))
        };
        let guest_by_id = |id: u32| {
            self.lan_guest_conn_by_id(id)
                .map(|conn| (id as i32, guest(conn).and_then(|(_, team)| team)))
        };
        let attacker = match owner {
            Who::Player => match owner_guest {
                Some(conn) if self.server.lan_acting != Some(conn) => guest(conn),
                _ => Some((self.own_entity_id(), self.player_team())),
            },
            Who::Mob(oid) => guest_by_id(oid),
        };
        let Some((attacker, attacker_team)) = attacker else {
            return true;
        };
        let victim = match target {
            Who::Player => Some((self.own_entity_id(), self.player_team())),
            Who::Mob(tid) => guest_by_id(tid),
        };
        let Some((victim, victim_team)) = victim else {
            return true;
        };
        self.player_may_harm(attacker, victim, attacker_team, victim_team)
    }
    /// Vanilla entity type id for any projectile-hittable `Who`.
    pub(in crate::game::server_side) fn entity_kind_for_who(&self, who: Who) -> Option<String> {
        match who {
            Who::Player => Some("minecraft:player".to_owned()),
            Who::Mob(id) => {
                if id >= sunrise_gameplay_common::game::dragon_common::SPECIAL {
                    return Some("minecraft:ender_dragon".to_owned());
                }
                if self.common.is_crystal_id(id) {
                    return Some("minecraft:end_crystal".to_owned());
                }
                if let Some(m) = self.common.mobs.iter().find(|m| m.id == id) {
                    return Some(m.kind.id.clone());
                }
                if self.lan_guest_conn_by_id(id).is_some() {
                    return Some("minecraft:player".to_owned());
                }
                if self.common.armor_stands.iter().any(|a| a.id == id) {
                    return Some("minecraft:armor_stand".to_owned());
                }
                if let Some(b) = self.common.boats.iter().find(|b| b.id == id) {
                    return Some(b.entity_type());
                }
                if let Some(c) = self.common.carts.iter().find(|c| c.id == id) {
                    return Some(c.kind.id().to_owned());
                }
                if let Some(f) = self.common.item_frames.iter().find(|f| f.id == id) {
                    return Some(f.kind().to_owned());
                }
                if self.common.paintings.iter().any(|p| p.id == id) {
                    return Some("minecraft:painting".to_owned());
                }
                if self.common.cushions.iter().any(|c| c.id == id) {
                    return Some("minecraft:cushion".to_owned());
                }
                if self.natural_knot(id).is_some()
                    || self
                        .server
                        .others
                        .0
                        .iter()
                        .any(|o| o.id == id && o.kind == "minecraft:leash_knot")
                {
                    return Some("minecraft:leash_knot".to_owned());
                }
                if self.common.falling.iter().any(|f| f.id == id) {
                    return Some("minecraft:falling_block".to_owned());
                }
                if self.common.tnts.iter().any(|t| t.id == id) {
                    return Some("minecraft:tnt".to_owned());
                }
                if let Some(p) = self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)
                {
                    return Some(p.entity_type().to_owned());
                }
                if self.common.dropped.iter().any(|i| i.id == id) {
                    return Some("minecraft:item".to_owned());
                }
                None
            }
        }
    }
    /// Whether `Entity.isInvulnerableToBase` / native hook refuses a projectile hit on `id`.
    pub(in crate::game::server_side) fn object_refuses_projectile(
        &mut self,
        info: &ProjectileHitInfo,
        id: u32,
        kind: &str,
        causing: i32,
        creative: bool,
    ) -> bool {
        let source = sunrise_sim::health::HurtSource {
            damage_type: info.damage_type,
            causing,
            direct: info.direct_id as i32,
            position: Some([info.position.x, info.position.y, info.position.z]),
            direct_type: None,
        };
        let vanilla = self.saved_refuses_damage(id, info.damage_type, creative);
        crate::game::server_side::entity_server::damage_level_of!(
            self,
            self.server.combat.0.attack(),
            no_push,
            None
        )
        .invulnerability_check(
            sunrise_sim::health::Hurting::Entity(id as i32, kind, source),
            vanilla,
        )
    }
    /// All non-mob pickable entities (`Entity::can_be_hit_by_projectile` plus `BlockAttachedEntity`).
    #[cfg(any(test, feature = "test-support"))]
    pub fn pickable_object_targets(&self) -> Vec<(Who, Aabb)> {
        let mut out = Vec::new();
        for b in self.common.boats.iter() {
            if b.can_be_hit_by_projectile() {
                out.push((Who::Mob(b.id), b.bounds()));
            }
        }
        for c in self.common.carts.iter() {
            if c.can_be_hit_by_projectile() {
                out.push((Who::Mob(c.id), c.bounds()));
            }
        }
        for a in self.common.armor_stands.iter() {
            if a.can_be_hit_by_projectile() {
                out.push((Who::Mob(a.id), a.bounds()));
            }
        }
        for t in self.common.tnts.iter() {
            if t.can_be_hit_by_projectile() {
                out.push((Who::Mob(t.id), t.bounds()));
            }
        }
        for f in self.common.falling.iter() {
            if f.can_be_hit_by_projectile() {
                out.push((Who::Mob(f.id), f.bounds()));
            }
        }
        for f in self.common.item_frames.iter() {
            out.push((Who::Mob(f.id), extents(f.bounds())));
        }
        for p in self.common.paintings.iter() {
            out.push((Who::Mob(p.id), extents(p.bounds())));
        }
        for c in self.common.cushions.iter() {
            out.push((Who::Mob(c.id), extents(c.bounds())));
        }
        for k in self.common.resident_entities.knots.iter() {
            let pos = sunrise_sim::mobs::leash::knot_state(k.fence).position;
            out.push((Who::Mob(k.id), knot_bounds(pos)));
        }
        for o in self.server.others.0.iter() {
            if o.kind == "minecraft:leash_knot" {
                out.push((Who::Mob(o.id), knot_bounds(o.position)));
            }
        }
        out
    }
    /// Unified `hurtServer` dispatch for every non-mob pickable target.
    pub(in crate::game::server_side) fn projectile_hurts_non_mob(
        &mut self,
        info: &ProjectileHitInfo,
        id: u32,
    ) -> NonMobHit {
        let (causing_target, by_player, creative, causing_id) = self.projectile_source_info(info);
        let mob_blocked = !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING)
            && matches!(causing_target, Some(Target::Mob(_)));

        // ArmorStand.hurtServer (igniteForSeconds clears freeze; refusal restores fire_ticks only).
        if let Some(i) = self.common.armor_stands.iter().position(|a| a.id == id) {
            let prev_fire = self.common.armor_stands[i].fire_ticks;
            if let Some(secs) = info.ignite_seconds
                && !self.common.armor_stands[i].mod_fire_immune
            {
                let ticks = (secs * 20.0) as i32;
                let a = &mut self.common.armor_stands[i];
                a.fire_ticks = a.fire_ticks.max(ticks);
                if let Some(sunrise_core::nbt::Tag::Compound(map)) =
                    self.common.persist.raw.get_mut(&id)
                {
                    map.remove(&"TicksFrozen".into());
                }
            }
            let hurt = self.hurt_armor_stand(
                id,
                info.damage_type,
                causing_target,
                info.direct_id,
                Some([info.position.x, info.position.y, info.position.z]),
            );
            if !hurt
                && info.ignite_seconds.is_some()
                && let Some(a) = self.common.armor_stands.iter_mut().find(|a| a.id == id)
            {
                a.fire_ticks = prev_fire;
            }
            return if hurt {
                NonMobHit::Hurt
            } else {
                NonMobHit::Refused
            };
        }

        // ItemFrame / GlowItemFrame (`ItemFrame.hurtServer`).
        if let Some(i) = self.common.item_frames.iter().position(|f| f.id == id) {
            let kind = self.common.item_frames[i].kind();
            if self.object_refuses_projectile(info, id, kind, causing_id, creative) {
                return NonMobHit::Refused;
            }
            let hurt = self.projectile_hurts_frame_by(
                id,
                by_player,
                creative,
                causing_target,
                info.damage_type,
            );
            return if hurt {
                NonMobHit::Hurt
            } else {
                NonMobHit::Refused
            };
        }

        // Painting (`BlockAttachedEntity.hurtServer`).
        if self.common.paintings.iter().any(|p| p.id == id) {
            if mob_blocked
                || self.object_refuses_projectile(
                    info,
                    id,
                    "minecraft:painting",
                    causing_id,
                    creative,
                )
            {
                return NonMobHit::Refused;
            }
            let Some(i) = self.common.paintings.iter().position(|p| p.id == id) else {
                return NonMobHit::Refused;
            };
            let broken = self.break_painting_by(i, by_player, causing_target);
            return if broken {
                NonMobHit::Hurt
            } else {
                NonMobHit::Refused
            };
        }

        // Cushion (`Cushion.hurtServer`: checks `mayBuild` and `mayInteract` spawn protection).
        if let Some(pos) = self
            .common
            .cushions
            .iter()
            .find(|c| c.id == id)
            .map(|c| BlockPos::new(c.pos[0], c.pos[1], c.pos[2]))
        {
            let (may_build, may_interact) = match causing_target {
                Some(Target::Player) => (self.may_build(), self.may_interact_at(pos)),
                Some(Target::Guest(conn)) if self.server.lan_acting == Some(conn) => {
                    (self.may_build(), self.may_interact_at(pos))
                }
                Some(Target::Guest(conn)) => (
                    self.command_guest(conn)
                        .is_none_or(|g| g.abilities.may_build),
                    self.as_guest(conn, |g| g.may_interact_at(pos))
                        .unwrap_or(true),
                ),
                _ => (true, true),
            };
            if !may_build
                || !may_interact
                || mob_blocked
                || self.object_refuses_projectile(
                    info,
                    id,
                    "minecraft:cushion",
                    causing_id,
                    creative,
                )
            {
                return NonMobHit::Refused;
            }
            let Some(i) = self.common.cushions.iter().position(|c| c.id == id) else {
                return NonMobHit::Refused;
            };
            let broken = self.break_cushion_by(i, causing_target);
            return if broken {
                NonMobHit::Hurt
            } else {
                NonMobHit::Refused
            };
        }

        // LeashFenceKnotEntity (`BlockAttachedEntity.hurtServer`), natural or explicit.
        let is_knot = self.natural_knot(id).is_some()
            || self
                .server
                .others
                .0
                .iter()
                .any(|o| o.id == id && o.kind == "minecraft:leash_knot");
        if is_knot {
            if mob_blocked
                || self.object_refuses_projectile(
                    info,
                    id,
                    "minecraft:leash_knot",
                    causing_id,
                    creative,
                )
            {
                return NonMobHit::Refused;
            }
            let broken = self.break_leash_knot(id, causing_target);
            return if broken {
                NonMobHit::Hurt
            } else {
                NonMobHit::Refused
            };
        }

        // Pickable projectiles (`ShulkerBullet.hurtServer` destroys; `#redirectable_projectile` refuses).
        if let Some(i) = self
            .common
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == id)
        {
            if matches!(
                self.creatures.projectiles[i].kind,
                ProjectileKind::ShulkerBullet { .. }
            ) {
                if self.object_refuses_projectile(
                    info,
                    id,
                    "minecraft:shulker_bullet",
                    causing_id,
                    creative,
                ) {
                    return NonMobHit::Refused;
                }
                let Some(i) = self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .position(|p| p.id == id)
                else {
                    return NonMobHit::Refused;
                };
                let _ = self.destroy_shulker_bullet(i);
                return NonMobHit::Hurt;
            }
            if redirectable(&self.common.creatures.projectiles[i]) {
                return NonMobHit::Refused;
            }
        }

        // FallingBlockEntity (`Entity.hurtServer`: always returns false).
        if self.common.falling.iter().any(|f| f.id == id) {
            return NonMobHit::Refused;
        }

        // PrimedTnt (`Entity.hurtServer`: always returns false).
        if self.common.tnts.iter().any(|t| t.id == id) {
            return NonMobHit::Refused;
        }

        // Boat (`VehicleEntity.hurtServer`).
        if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
            let kind = self.common.boats[i].entity_type();
            if self.object_refuses_projectile(info, id, &kind, causing_id, creative) {
                return NonMobHit::Refused;
            }
            let Some(i) = self.common.boats.iter().position(|b| b.id == id) else {
                return NonMobHit::Refused;
            };
            self.hurt_boat(i, info.amount, creative);
            return NonMobHit::Hurt;
        }

        // Minecart (`VehicleEntity.hurtServer`).
        if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
            let kind = self.common.carts[i].kind.id();
            if self.object_refuses_projectile(info, id, kind, causing_id, creative) {
                return NonMobHit::Refused;
            }
            let Some(i) = self.common.carts.iter().position(|c| c.id == id) else {
                return NonMobHit::Refused;
            };
            if info.is_arrow && info.damage_type == "minecraft:arrow" {
                self.arrow_hits_cart(
                    i,
                    info.amount,
                    info.velocity,
                    info.burning_arrow,
                    creative,
                    info.owner,
                    info.guest,
                );
            } else {
                let hit = if info.damage_type == "minecraft:trident" {
                    let by = match info.owner {
                        Some(Who::Player) => Some(info.guest.map_or(Target::Player, Target::Guest)),
                        Some(Who::Mob(m)) => Some(Target::Mob(m)),
                        None => None,
                    };
                    CartHit::attributed(by)
                } else {
                    CartHit::projectile(info.owner, info.guest)
                };
                self.hurt_cart(i, info.amount, creative, false, hit);
            }
            return NonMobHit::Hurt;
        }

        NonMobHit::NotObject
    }
    /// Resolve a projectile's causing target, creative flag, and entity id.
    pub(in crate::game::server_side) fn projectile_source_info(
        &self,
        info: &ProjectileHitInfo,
    ) -> (Option<Target>, bool, bool, i32) {
        let resolved = self.resolved_projectile_owner(info.owner, info.guest);
        let causing_target = match resolved {
            Some(Who::Player) => Some(info.guest.map_or(Target::Player, Target::Guest)),
            Some(Who::Mob(id)) => Some(Target::Mob(id)),
            None => None,
        };
        let by_player = resolved == Some(Who::Player);
        let creative = match resolved {
            Some(Who::Player) => match info.guest {
                Some(conn) if self.server.lan_acting == Some(conn) => self.common.creative,
                Some(conn) => self.command_guest(conn).is_some_and(|g| g.creative()),
                None => self.common.creative,
            },
            _ => false,
        };
        let causing_id = match resolved {
            Some(Who::Player) => match info.guest {
                Some(conn) => self.command_guest(conn).map_or(0, |g| g.entity_id),
                None => self.own_entity_id(),
            },
            Some(Who::Mob(id)) => id as i32,
            None => 0,
        };
        (causing_target, by_player, creative, causing_id)
    }
    /// `Projectile.getOwner()`: returns `None` when the recorded owner entity is no longer present.
    pub(in crate::game) fn resolved_projectile_owner(
        &self,
        owner: Option<Who>,
        guest: Option<u32>,
    ) -> Option<Who> {
        match owner? {
            Who::Player => match guest {
                Some(conn) if self.server.lan_acting != Some(conn) => {
                    self.command_guest(conn).map(|_| Who::Player)
                }
                _ => self.common.player.as_ref().map(|_| Who::Player),
            },
            Who::Mob(id) => {
                if self.common.mobs.iter().any(|m| m.id == id && !m.removed)
                    || self.lan_guest_conn_by_id(id).is_some()
                {
                    Some(Who::Mob(id))
                } else {
                    None
                }
            }
        }
    }
    /// Traverse every live pickable target matching `query`, carrying exact entity position and
    /// `EntitySectionStorage` encounter order (`TargetOrder`).
    pub(in crate::game) fn visit_projectile_targets(
        &self,
        query: TargetQuery,
        extra_projectiles: &[TargetCandidate],
        mut visit: impl FnMut(TargetCandidate),
    ) {
        let allows = |id: u32, default_no_physics: bool| {
            !query.skip_no_physics
                || !self
                    .common
                    .persist
                    .no_physics
                    .get(&id)
                    .copied()
                    .unwrap_or(default_no_physics)
        };
        for m in self.common.mobs.iter() {
            if m.can_be_hit_by_projectile() && (!query.skip_no_physics || !m.no_physics(None)) {
                visit(TargetCandidate {
                    who: Who::Mob(m.id),
                    bounds: m.bounds(),
                    position: m.position,
                    order: TargetOrder::entity(m.position, m.join.get()),
                });
            }
        }
        if let Some(p) = self.player_view()
            && p.alive
            && !self.common.spectator()
            && query
                .arrow_friendly_fire
                .is_none_or(|(owner, og)| self.can_arrow_harm_player(owner, og, Who::Player))
        {
            let pos = self.common.player.as_ref().map_or_else(
                || {
                    let c = p.bounds.center();
                    Vec3::new(c.x, p.bounds.min_position().y, c.z)
                },
                |pl| pl.position,
            );
            visit(TargetCandidate {
                who: Who::Player,
                bounds: p.bounds,
                position: pos,
                order: TargetOrder::entity(pos, 0),
            });
        }
        if query.include_guests
            && let Some(host) = self.server.lan.as_ref()
        {
            for g in &host.hub.guests {
                if host.hub.here(g) && g.loaded && !g.dead && !g.spectator() {
                    let who = Who::Mob(g.entity_id as u32);
                    if query
                        .arrow_friendly_fire
                        .is_some_and(|(owner, og)| !self.can_arrow_harm_player(owner, og, who))
                    {
                        continue;
                    }
                    let pos = Vec3::new(g.position[0], g.position[1], g.position[2]);
                    visit(TargetCandidate {
                        who,
                        bounds: guest_bounds(g),
                        position: pos,
                        order: TargetOrder::entity(pos, g.entity_id as u64),
                    });
                }
            }
        }
        if !query.skip_crystals {
            for c in self.common.crystals.iter() {
                if allows(c.id, false) {
                    visit(TargetCandidate {
                        who: Who::Mob(c.id),
                        bounds: c.bounds(),
                        position: c.position,
                        order: TargetOrder::entity(c.position, c.join.get()),
                    });
                }
            }
        }
        for b in self.common.boats.iter() {
            if b.can_be_hit_by_projectile()
                && (!query.skip_no_physics
                    || !b.no_physics(self.common.persist.no_physics.get(&b.id).copied()))
            {
                visit(TargetCandidate {
                    who: Who::Mob(b.id),
                    bounds: b.bounds(),
                    position: b.position,
                    order: TargetOrder::entity(b.position, b.join.get()),
                });
            }
        }
        for c in self.common.carts.iter() {
            if c.can_be_hit_by_projectile()
                && (!query.skip_no_physics
                    || !c.no_physics(self.common.persist.no_physics.get(&c.id).copied()))
            {
                visit(TargetCandidate {
                    who: Who::Mob(c.id),
                    bounds: c.bounds(),
                    position: c.position,
                    order: TargetOrder::entity(c.position, c.join.get()),
                });
            }
        }
        for a in self.common.armor_stands.iter() {
            if a.can_be_hit_by_projectile()
                && (!query.skip_no_physics
                    || !a.no_physics(self.common.persist.no_physics.get(&a.id).copied()))
            {
                visit(TargetCandidate {
                    who: Who::Mob(a.id),
                    bounds: a.bounds(),
                    position: a.position,
                    order: TargetOrder::entity(a.position, a.join.get()),
                });
            }
        }
        for t in self.common.tnts.iter() {
            if t.can_be_hit_by_projectile()
                && (!query.skip_no_physics
                    || !t.no_physics(self.common.persist.no_physics.get(&t.id).copied()))
            {
                visit(TargetCandidate {
                    who: Who::Mob(t.id),
                    bounds: t.bounds(),
                    position: t.position,
                    order: TargetOrder::entity(t.position, t.join.get()),
                });
            }
        }
        for f in self.common.falling.iter() {
            if f.can_be_hit_by_projectile()
                && (!query.skip_no_physics
                    || !f.no_physics(self.common.persist.no_physics.get(&f.id).copied()))
            {
                visit(TargetCandidate {
                    who: Who::Mob(f.id),
                    bounds: f.bounds(),
                    position: f.position,
                    order: TargetOrder::entity(f.position, f.join.get()),
                });
            }
        }
        // HangingEntity.recalculateBoundingBox sets Entity.position() to the box center.
        for f in self.common.item_frames.iter() {
            if allows(f.id, false) {
                let bounds = extents(f.bounds());
                let pos = bounds.center();
                visit(TargetCandidate {
                    who: Who::Mob(f.id),
                    bounds,
                    position: pos,
                    order: TargetOrder::entity(pos, f.join.get()),
                });
            }
        }
        for p in self.common.paintings.iter() {
            if allows(p.id, false) {
                let bounds = extents(p.bounds());
                let pos = bounds.center();
                visit(TargetCandidate {
                    who: Who::Mob(p.id),
                    bounds,
                    position: pos,
                    order: TargetOrder::entity(pos, p.join.get()),
                });
            }
        }
        for c in self.common.cushions.iter() {
            if allows(c.id, false) {
                visit(TargetCandidate {
                    who: Who::Mob(c.id),
                    bounds: extents(c.bounds()),
                    position: c.position,
                    order: TargetOrder::entity(c.position, c.join.get()),
                });
            }
        }
        for k in self.common.resident_entities.knots.iter() {
            if allows(k.id, false) {
                let pos = sunrise_sim::mobs::leash::knot_state(k.fence).position;
                visit(TargetCandidate {
                    who: Who::Mob(k.id),
                    bounds: knot_bounds(pos),
                    position: pos,
                    order: TargetOrder::entity(pos, k.join.get()),
                });
            }
        }
        for o in self.server.others.0.iter() {
            if o.kind == "minecraft:leash_knot" && allows(o.id, false) {
                visit(TargetCandidate {
                    who: Who::Mob(o.id),
                    bounds: knot_bounds(o.position),
                    position: o.position,
                    order: TargetOrder::entity(o.position, o.join.get()),
                });
            }
        }
        // Pickable projectiles (`#redirectable_projectile` + `ShulkerBullet`).
        for q in self.common.creatures.projectiles.iter() {
            if query.skip_self_id == Some(q.id) || !is_pickable_projectile(q) {
                continue;
            }
            if query.skip_wind_charges
                && matches!(
                    q.kind,
                    ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
                )
            {
                continue;
            }
            if !allows(q.id, false) {
                continue;
            }
            visit(TargetCandidate {
                who: Who::Mob(q.id),
                bounds: q.bounds(),
                position: q.position,
                order: TargetOrder::entity(q.position, q.join.get()),
            });
        }
        for &c in extra_projectiles {
            if let Who::Mob(id) = c.who
                && query.skip_self_id != Some(id)
                && allows(id, false)
            {
                visit(c);
            }
        }
        if query.include_items {
            for item in self.common.dropped.iter() {
                if item.health > 0 && allows(item.id, false) {
                    visit(TargetCandidate {
                        who: Who::Mob(item.id),
                        bounds: item.bounds(),
                        position: item.position,
                        order: TargetOrder::entity(item.position, item.join.get()),
                    });
                }
            }
        }
        // Level.getEntities visits dragon parts after all regular entities.
        if let Some(boxes) = self.common.dragon_part_boxes() {
            for (i, bounds) in boxes.into_iter().enumerate() {
                let c = bounds.center();
                let pos = Vec3::new(c.x, bounds.min_position().y, c.z);
                visit(TargetCandidate {
                    who: Who::Mob(sunrise_gameplay_common::game::dragon_common::SPECIAL + i as u32),
                    bounds,
                    position: pos,
                    order: TargetOrder::dragon_part(i as u32),
                });
            }
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
