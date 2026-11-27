//! Vanilla's `GoalSelector` (priorities, MOVE/LOOK/JUMP/TARGET control flags,
//! interruption) and the shared goals of `net.minecraft.world.entity.ai.goal`.
use super::{Mob, MobEvent, MobWorld, Seen, Species, Who, world::TargetInfo};
use std::ops::ControlFlow;
use std::sync::Arc;
use sunrise_core::{
    geometry::{Aabb, Vec3},
    jmath,
    random::RandomSource,
};

include!("goals/core.rs");

impl Mob {
    /// PathfinderMob.getWalkTargetValue.
    pub fn walk_target_value(&self, w: &MobWorld, p: [i32; 3]) -> f32 {
        super::ai::walk_target_value(self, w, p)
    }
    fn random_direction(&mut self, h: i32, v: i32) -> [i32; 3] {
        let x = self.random.next_int_bound(2 * h + 1).unwrap_or(0) - h;
        let y = self.random.next_int_bound(2 * v + 1).unwrap_or(0) - v;
        let z = self.random.next_int_bound(2 * h + 1).unwrap_or(0) - h;
        [x, y, z]
    }
    #[allow(clippy::too_many_arguments)]
    fn random_direction_within(
        &mut self,
        min: f64,
        max: f64,
        v: i32,
        fly_height: i32,
        xd: f64,
        zd: f64,
        max_rad: f64,
    ) -> Option<[i32; 3]> {
        let center =
            sunrise_core::geometry::trig::atan2(zd, xd) - f64::from(std::f32::consts::FRAC_PI_2);
        let yrad = center + f64::from(2.0 * self.random.next_float() - 1.0) * max_rad;
        let dist = (min + self.random.next_double().sqrt() * (max - min))
            * f64::from(std::f32::consts::SQRT_2);
        let xt = -dist * jmath::sin(yrad);
        let zt = dist * jmath::cos(yrad);
        if xt.abs() > max || zt.abs() > max {
            return None;
        }
        let yt = self.random.next_int_bound(2 * v + 1).unwrap_or(0) - v + fly_height;
        Some([xt.floor() as i32, yt, zt.floor() as i32])
    }
    /// RandomPos.generateRandomPosTowardDirection: a mob with a home leans
    /// half the distance back toward it.
    fn toward(&mut self, h: f64, d: [i32; 3]) -> [i32; 3] {
        let (mut xt, mut zt) = (f64::from(d[0]), f64::from(d[2]));
        if let Some((c, _)) = self.ai.home
            && h > 1.0
        {
            if self.position.x > f64::from(c[0]) {
                xt -= self.random.next_double() * h / 2.0;
            } else {
                xt += self.random.next_double() * h / 2.0;
            }
            if self.position.z > f64::from(c[2]) {
                zt -= self.random.next_double() * h / 2.0;
            } else {
                zt += self.random.next_double() * h / 2.0;
            }
        }
        [
            (xt + self.position.x).floor() as i32,
            (f64::from(d[1]) + self.position.y).floor() as i32,
            (zt + self.position.z).floor() as i32,
        ]
    }
    /// `GoalUtils.isRestricted(mobRestricted(mob, h), mob, pos)`: a mob
    /// near its home keeps the positions it picks within it.
    pub fn restricted(&self, h: f64, p: [i32; 3]) -> bool {
        let Some((c, r)) = self.ai.home else {
            return false;
        };
        let near = {
            let d = |i: usize, v: f64| f64::from(c[i]) + 0.5 - v;
            let (x, y, z) = (
                d(0, self.position.x),
                d(1, self.position.y),
                d(2, self.position.z),
            );
            let reach = f64::from(r) + h + 1.0;
            x * x + y * y + z * z < reach * reach
        };
        near && !within_home(c, r, p)
    }
    fn default_check(&self, w: &MobWorld, h: f64, p: [i32; 3]) -> bool {
        p[1] >= w.world.bottom_y()
            && p[1] < w.world.top_y()
            && !self.restricted(h, p)
            && self.stable_destination(w, p)
            && self.malus(w.path_type_static(p[0], p[1], p[2])) == 0.0
    }
    fn pick_best(
        &mut self,
        w: &MobWorld,
        mut supplier: impl FnMut(&mut Self) -> Option<[i32; 3]>,
    ) -> Option<[f64; 3]> {
        let mut best: Option<[i32; 3]> = None;
        let mut best_weight = f64::NEG_INFINITY;
        for _ in 0..10 {
            if let Some(p) = supplier(self) {
                let v = f64::from(self.walk_target_value(w, p));
                if v > best_weight {
                    best_weight = v;
                    best = Some(p);
                }
            }
        }
        best.map(|p| {
            [
                f64::from(p[0]) + 0.5,
                f64::from(p[1]),
                f64::from(p[2]) + 0.5,
            ]
        })
    }
    /// DefaultRandomPos.getPos
    pub fn default_random_pos(&mut self, w: &MobWorld, h: i32, v: i32) -> Option<[f64; 3]> {
        self.pick_best(w, |m| {
            let d = m.random_direction(h, v);
            let p = m.toward(f64::from(h), d);
            m.default_check(w, f64::from(h), p).then_some(p)
        })
    }
    /// DefaultRandomPos.getPosAway
    pub fn default_random_pos_away(
        &mut self,
        w: &MobWorld,
        h: i32,
        v: i32,
        away: [f64; 3],
    ) -> Option<[f64; 3]> {
        let (xd, zd) = (self.position.x - away[0], self.position.z - away[2]);
        self.pick_best(w, |m| {
            let d = m.random_direction_within(
                0.0,
                f64::from(h),
                v,
                0,
                xd,
                zd,
                std::f64::consts::FRAC_PI_2,
            )?;
            let p = m.toward(f64::from(h), d);
            m.default_check(w, f64::from(h), p).then_some(p)
        })
    }
    /// DefaultRandomPos.getPosTowards
    pub fn default_random_pos_towards(
        &mut self,
        w: &MobWorld,
        h: i32,
        v: i32,
        to: [f64; 3],
        max_rad: f64,
    ) -> Option<[f64; 3]> {
        let (xd, zd) = (to[0] - self.position.x, to[2] - self.position.z);
        self.pick_best(w, |m| {
            let d = m.random_direction_within(0.0, f64::from(h), v, 0, xd, zd, max_rad)?;
            let p = m.toward(f64::from(h), d);
            m.default_check(w, f64::from(h), p).then_some(p)
        })
    }
    fn land_up_out_of_solid(&self, w: &MobWorld, mut p: [i32; 3]) -> Option<[i32; 3]> {
        if w.solid(p[0], p[1], p[2]) {
            p[1] += 1;
            while p[1] < w.world.top_y() && w.solid(p[0], p[1], p[2]) {
                p[1] += 1;
            }
        }
        let water = w.fluid(p[0], p[1], p[2]) == crate::blocks::Fluid::Water;
        (!water && self.malus(w.path_type_static(p[0], p[1], p[2])) == 0.0).then_some(p)
    }
    /// Where the mob stands and looks (`getViewVector(1)`: its head's pitch and yaw), as a
    /// hit's [`crate::health::Victim`].
    pub fn victim(&self) -> crate::health::Victim {
        let view = sunrise_world::view_direction::view_vector(self.head_pitch, self.head_yaw);
        crate::health::Victim {
            position: [self.position.x, self.position.y, self.position.z],
            view: [view.x, view.y, view.z],
        }
    }
    /// `Entity.getViewVector(0)`: `calculateViewVector(xRotO, yHeadRotO)`.
    pub fn view_vector(&self) -> [f64; 3] {
        use sunrise_core::geometry::trig::Mth;
        let to_rad = std::f32::consts::PI / 180.0;
        let x = self.previous_head_pitch * to_rad;
        // LivingEntity.getViewYRot(0): the head's yaw of the last tick.
        let y = -self.previous_head_yaw * to_rad;
        let (y_cos, y_sin, x_cos, x_sin) = (y.mcos(), y.msin(), x.mcos(), x.msin());
        [
            f64::from(y_sin * x_cos),
            f64::from(-x_sin),
            f64::from(y_cos * x_cos),
        ]
    }
    /// `Entity.getLookAngle`: `calculateViewVector(xRot, yRot)`, the facing
    /// of the body (`getHeadLookAngle` is the one with the head's yaw).
    pub fn look_angle(&self) -> [f64; 3] {
        use sunrise_core::geometry::trig::Mth;
        let to_rad = std::f32::consts::PI / 180.0;
        let x = self.head_pitch * to_rad;
        let y = -self.yaw * to_rad;
        let (y_cos, y_sin, x_cos, x_sin) = (y.mcos(), y.msin(), x.mcos(), x.msin());
        [
            f64::from(y_sin * x_cos),
            f64::from(-x_sin),
            f64::from(y_cos * x_cos),
        ]
    }
    /// AirAndWaterRandomPos.getPos: within the radians of a direction, kept
    /// near home, moved up out of solid blocks and free of path maluses.
    pub fn air_and_water_random_pos(
        &mut self,
        w: &MobWorld,
        h: i32,
        v: i32,
        fly_height: i32,
        xd: f64,
        zd: f64,
    ) -> Option<[f64; 3]> {
        // Every caller passes `(float) (Math.PI / 2)`.
        let max_rad = f64::from(std::f32::consts::FRAC_PI_2);
        self.pick_best(w, |m| {
            m.air_and_water_pos(w, h, v, fly_height, xd, zd, max_rad)
        })
    }
    /// AirAndWaterRandomPos.generateRandomPos.
    #[allow(clippy::too_many_arguments)]
    fn air_and_water_pos(
        &mut self,
        w: &MobWorld,
        h: i32,
        v: i32,
        fly_height: i32,
        xd: f64,
        zd: f64,
        max_rad: f64,
    ) -> Option<[i32; 3]> {
        let d = self.random_direction_within(0.0, f64::from(h), v, fly_height, xd, zd, max_rad)?;
        let mut p = self.toward(f64::from(h), d);
        if p[1] < w.world.bottom_y() || p[1] >= w.world.top_y() || self.restricted(f64::from(h), p)
        {
            return None;
        }
        // RandomPos.moveUpOutOfSolid
        if w.solid(p[0], p[1], p[2]) {
            p[1] += 1;
            while p[1] < w.world.top_y() && w.solid(p[0], p[1], p[2]) {
                p[1] += 1;
            }
        }
        (self.malus(w.path_type_static(p[0], p[1], p[2])) == 0.0).then_some(p)
    }
    /// AirRandomPos.getPosTowards: an AirAndWaterRandomPos towards a point,
    /// out of water.
    pub fn air_random_pos_towards(
        &mut self,
        w: &MobWorld,
        h: i32,
        v: i32,
        fly_height: i32,
        to: [f64; 3],
        max_rad: f64,
    ) -> Option<[f64; 3]> {
        let (xd, zd) = (to[0] - self.position.x, to[2] - self.position.z);
        self.pick_best(w, |m| {
            let p = m.air_and_water_pos(w, h, v, fly_height, xd, zd, max_rad)?;
            (w.fluid(p[0], p[1], p[2]) != crate::blocks::Fluid::Water).then_some(p)
        })
    }
    /// HoverRandomPos.getPos: a stable spot within the radians of a
    /// direction, raised a random `hover_min..=hover_max` blocks above the
    /// solid ground it is in, out of water and free of path maluses.
    #[allow(clippy::too_many_arguments)]
    pub fn hover_random_pos(
        &mut self,
        w: &MobWorld,
        h: i32,
        v: i32,
        xd: f64,
        zd: f64,
        max_rad: f64,
        hover_max: i32,
        hover_min: i32,
    ) -> Option<[f64; 3]> {
        self.pick_best(w, |m| {
            let d = m.random_direction_within(0.0, f64::from(h), v, 0, xd, zd, max_rad)?;
            // LandRandomPos.generateRandomPosTowardDirection
            let p = m.toward(f64::from(h), d);
            if p[1] < w.world.bottom_y()
                || p[1] >= w.world.top_y()
                || m.restricted(f64::from(h), p)
                || !m.stable_destination(w, p)
            {
                return None;
            }
            let above = m
                .random
                .next_int_bound(hover_max - hover_min + 1)
                .unwrap_or(0)
                + hover_min;
            let p = move_up_to_above_solid(w, p, above);
            let water = w.fluid(p[0], p[1], p[2]) == crate::blocks::Fluid::Water;
            (!water && m.malus(w.path_type_static(p[0], p[1], p[2])) == 0.0).then_some(p)
        })
    }
    /// `WaterAvoidingRandomFlyingGoal.getPosition`: a hover spot ahead, else
    /// any air or water spot ahead.
    pub fn flying_stroll_pos(&mut self, w: &MobWorld) -> Option<[f64; 3]> {
        let [xd, _, zd] = self.view_vector();
        let max_rad = f64::from(std::f32::consts::FRAC_PI_2);
        self.hover_random_pos(w, 8, 7, xd, zd, max_rad, 3, 1)
            .or_else(|| self.air_and_water_random_pos(w, 8, 4, -2, xd, zd))
    }
    /// LandRandomPos.getPos
    pub fn land_random_pos(&mut self, w: &MobWorld, h: i32, v: i32) -> Option<[f64; 3]> {
        self.pick_best(w, |m| {
            let d = m.random_direction(h, v);
            let p = m.toward(f64::from(h), d);
            if p[1] < w.world.bottom_y()
                || p[1] >= w.world.top_y()
                || m.restricted(f64::from(h), p)
                || !m.stable_destination(w, p)
            {
                return None;
            }
            m.land_up_out_of_solid(w, p)
        })
    }
    /// LandRandomPos.getPos(mob, h, v, scorer): the best of ten by `score`.
    pub fn land_random_pos_scored(
        &mut self,
        w: &MobWorld,
        h: i32,
        v: i32,
        score: impl Fn([i32; 3]) -> f64,
    ) -> Option<[f64; 3]> {
        let mut best: Option<[i32; 3]> = None;
        let mut best_weight = f64::NEG_INFINITY;
        for _ in 0..10 {
            let d = self.random_direction(h, v);
            let p = self.toward(f64::from(h), d);
            if p[1] < w.world.bottom_y()
                || p[1] >= w.world.top_y()
                || self.restricted(f64::from(h), p)
                || !self.stable_destination(w, p)
            {
                continue;
            }
            if let Some(p) = self.land_up_out_of_solid(w, p) {
                let v = score(p);
                if v > best_weight {
                    best_weight = v;
                    best = Some(p);
                }
            }
        }
        best.map(|p| {
            [
                f64::from(p[0]) + 0.5,
                f64::from(p[1]),
                f64::from(p[2]) + 0.5,
            ]
        })
    }
    /// `LandRandomPos.getPosAway`: a spot away from `away`, the best of ten tries.
    pub fn land_random_pos_away(
        &mut self,
        w: &MobWorld,
        h: i32,
        v: i32,
        away: [f64; 3],
    ) -> Option<[f64; 3]> {
        self.land_random_pos_away_between(w, 0.0, f64::from(h), v, away)
    }
    /// `LandRandomPos.getPosAway(mob, minHorizontalDist, maxHorizontalDist, verticalDist, avoidPos)`.
    pub fn land_random_pos_away_between(
        &mut self,
        w: &MobWorld,
        min_h: f64,
        max_h: f64,
        v: i32,
        away: [f64; 3],
    ) -> Option<[f64; 3]> {
        let (mut xd, mut zd) = (self.position.x - away[0], self.position.z - away[2]);
        if xd == 0.0 && zd == 0.0 {
            xd = self.random.next_double() - 0.5;
            zd = self.random.next_double() - 0.5;
        }
        self.land_pos_in_direction(w, min_h, max_h, v, xd, zd)
    }
    /// `LandRandomPos.getPosTowards`: a spot within a quarter turn of the
    /// way to `to`, the best of ten tries.
    pub fn land_random_pos_towards(
        &mut self,
        w: &MobWorld,
        h: i32,
        v: i32,
        to: [f64; 3],
    ) -> Option<[f64; 3]> {
        let (xd, zd) = (to[0] - self.position.x, to[2] - self.position.z);
        self.land_pos_in_direction(w, 0.0, f64::from(h), v, xd, zd)
    }
    /// `LandRandomPos.getPosInDirection`: ten tries within a quarter turn
    /// (`(float) (Math.PI / 2)`) of the direction `(xd, zd)`.
    fn land_pos_in_direction(
        &mut self,
        w: &MobWorld,
        min_h: f64,
        max_h: f64,
        v: i32,
        xd: f64,
        zd: f64,
    ) -> Option<[f64; 3]> {
        self.pick_best(w, |m| {
            let d = m.random_direction_within(
                min_h,
                max_h,
                v,
                0,
                xd,
                zd,
                f64::from(std::f32::consts::FRAC_PI_2),
            )?;
            let p = m.toward(max_h, d);
            if p[1] < w.world.bottom_y()
                || p[1] >= w.world.top_y()
                || m.restricted(max_h, p)
                || !m.stable_destination(w, p)
            {
                return None;
            }
            m.land_up_out_of_solid(w, p)
        })
    }
    pub fn target_info(&self, w: &MobWorld) -> Option<TargetInfo> {
        self.ai.target.and_then(|t| w.target_info(t))
    }
    /// Sensing.hasLineOfSight (cached per tick in vanilla).
    pub fn sees(&mut self, w: &MobWorld, who: Who) -> bool {
        if let Some(v) = self.ai.seen_cache.iter().find(|(k, _)| *k == who) {
            return v.1;
        }
        let Some(t) = w.target_info(who).filter(|_| !super::ravager::blind(self)) else {
            return false;
        };
        let eye = Vec3::new(self.position.x, self.eye_y(), self.position.z);
        let v = w.line_of_sight_for(
            Some(self.position.y),
            eye,
            Vec3::new(t.position.x, t.eye_y, t.position.z),
        );
        self.ai.seen_cache.push((who, v));
        v
    }
    /// Mob.isWithinMeleeAttackRange: the box inflated by sqrt(2.04) - 0.6, or
    /// for an active item with `attack_range` (the spears) by its
    /// `effectiveMaxRange` for a mob (`max_reach * mob_factor`), and not
    /// within its `effectiveMinRange` (`min_reach * mob_factor`).
    pub fn within_melee_range(&self, w: &MobWorld, t: &TargetInfo) -> bool {
        self.melee_reaches(w, t.hitbox)
    }
    /// [`Self::within_melee_range`] for a target's box (`getHitbox`).
    pub fn melee_reaches(&self, w: &MobWorld, target: Aabb) -> bool {
        // getVehicle().getBoundingBox() now: a mob vehicle's box as the
        // snapshot has it after its own tick, else the box positionRider
        // left for this same vehicle (a boat, a minecart, primed TNT).
        let vehicle = self.extra.mount.and_then(|v| {
            w.seen(v)
                .map(|s| s.bounds)
                .or_else(|| self.extra.seat.filter(|s| s.0 == v).map(|s| s.2))
        });
        self.melee_reaches_beside(vehicle, target)
    }
    /// [`Self::melee_reaches`] with the vehicle's box given.
    pub fn melee_reaches_beside(&self, vehicle: Option<Aabb>, target: Aabb) -> bool {
        let (max, min) = match self.active_attack_range() {
            Some((max, min)) => (f64::from(max), f64::from(min)),
            None => (2.04f32.sqrt() as f64 - 0.6, 0.0),
        };
        let reaches = |range: f64| self.attack_box(range, vehicle).intersects(target);
        reaches(max) && (min <= 0.0 || !reaches(min))
    }
    /// The active item's `attack_range` for a mob: `effectiveMaxRange` and
    /// `effectiveMinRange` (`max_reach` and `min_reach` times `mob_factor`).
    /// A patch on the held stack wins (`!minecraft:attack_range` removes it;
    /// the codec's defaults are 0, 3 and a factor of 1); else the spears'.
    fn active_attack_range(&self) -> Option<(f32, f32)> {
        let item = self.active_item()?;
        let patch = (self.extra.main_hand_id() == Some(item))
            .then(|| self.extra.main_hand_stack().and_then(|s| s.data()))
            .flatten();
        if patch.is_some_and(|d| d.other("!minecraft:attack_range").is_some()) {
            return None;
        }
        if let Some(range) = patch.and_then(|d| d.other("minecraft:attack_range")) {
            let float = |key: &str, default: f32| match range.get(key) {
                Some(sunrise_core::nbt::Tag::Float(bits)) => f32::from_bits(*bits),
                Some(sunrise_core::nbt::Tag::Double(bits)) => f64::from_bits(*bits) as f32,
                Some(sunrise_core::nbt::Tag::Int(v)) => *v as f32,
                _ => default,
            };
            let factor = float("mob_factor", 1.0);
            return Some((
                float("max_reach", 3.0) * factor,
                float("min_reach", 0.0) * factor,
            ));
        }
        crate::spear_sim::Kinetic::of(item).map(|_| {
            let r = &crate::spear_sim::REACH;
            (r.max * r.mob_factor, r.min * r.mob_factor)
        })
    }
    /// `LivingEntity.getActiveItem`: the item being used, else the main hand's.
    pub fn active_item(&self) -> Option<&str> {
        if super::spear_use::is_using_item(self) && self.extra.using_off_hand {
            self.extra.off_hand_id()
        } else {
            self.extra.main_hand_id()
        }
    }
    /// Each vanilla mob's own sitting test, as its renderer and goals read it: `Fox.isSitting`,
    /// `Panda.isSitting`, `Camel.isCamelSitting` and `TamableAnimal.isInSittingPose`. None for a
    /// species with no such test.
    pub fn sitting(&self) -> Option<bool> {
        Some(match self.species() {
            Species::Fox => super::fox::flag(self, super::fox::SITTING),
            Species::Panda => super::panda::is_sitting(self),
            Species::Camel | Species::CamelHusk => super::camel::sitting(self),
            _ if self.is_tamable_animal() => self.extra.sitting_pose,
            _ => return None,
        })
    }
    /// Whether the species is a `TamableAnimal` (not the horses and llamas,
    /// which are tamed another way).
    pub fn is_tamable_animal(&self) -> bool {
        matches!(
            self.species(),
            Species::Wolf
                | Species::Cat
                | Species::Parrot
                | Species::Nautilus
                | Species::ZombieNautilus
        )
    }
    /// A `TamableAnimal` or an `AbstractHorse` (camels and llamas too): the
    /// mobs `isTame` (`isTamed`) and an owner belong to. An allay's liked
    /// player is kept as an owner but is not one.
    pub fn is_tamable(&self) -> bool {
        self.is_tamable_animal() || super::equine::is_equine(self.species())
    }
    /// `IronGolem.isPlayerCreated`: a golem a player built, which
    /// `IronGolem.canAttack` never lets attack a player.
    pub fn built_by_player(&self) -> bool {
        self.species() == Species::IronGolem && self.extra.player_created
    }
    /// `canAttack` for the player: a golem a player built never attacks
    /// players (`IronGolem.canAttack`), and a pet never attacks its owner
    /// (`TamableAnimal.canAttack`: `isOwnedBy`). In Peaceful nothing attacks a
    /// player (`LivingEntity.canAttack`, `TargetingConditions.test`: combat
    /// conditions fail when the difficulty is PEACEFUL).
    pub fn spares_players(&self, w: &MobWorld) -> bool {
        (w.difficulty.base == 0 && w.selected_player().is_some())
            || self.built_by_player()
            || (self.is_tamable_animal()
                && self.extra.tame
                && w.selected_player()
                    .is_some_and(|p| super::tame::owned_by(self.extra.owner, p.uuid)))
    }
    /// `canAttack` of any target: the player rules above, and
    /// `IronGolem.canAttack`'s refusal of creepers. The callers check that
    /// the target is alive and can be seen as an enemy.
    pub fn spares_target(&self, w: &MobWorld, who: Who) -> bool {
        // Dolphin.canAttack: a baby dolphin attacks nobody, so whatever hurt it
        // is never taken as its target.
        if self.species() == Species::Dolphin && self.is_baby() {
            return true;
        }
        match who {
            Who::Player => self.spares_players(w),
            Who::Mob(id) => {
                // `Mob.canAttack` refuses a Ghast for every mob.
                w.seen(id).is_some_and(|s| s.species == Species::Ghast)
                    || (self.species() == Species::IronGolem
                        && w.seen(id).is_some_and(|s| s.species == Species::Creeper))
            }
        }
    }
    /// The setter's own refusal (#1175), one rule both the write handle and
    /// a goal tick's `set_target` ask: a brain-backed species' target is its
    /// brain's to write (`getTarget` overrides it, #934). The read path does
    /// not ask this — it reads a brain's own target back.
    pub fn refuses_set_target(&self) -> bool {
        super::ai::brain_target(self).is_some()
    }
    /// Apply an AI target change after listeners have had a chance to alter it.
    pub fn set_ai_target(&mut self, w: &MobWorld, target: Option<Who>) -> super::TargetChange {
        let is_brain = super::ai::brain_target(self).is_some();
        let change = w.change_target(self, is_brain, target);
        if let super::TargetChange::Set(target) = change {
            self.set_target(target);
        }
        change
    }
    /// `Mob.asValidTarget` and the modeled `canAttack` overrides over the
    /// target's facts (#1175): one rule set both the write handle and a goal
    /// tick's view ask — the facts of the target, not where they were read.
    /// The warden's own override (`Warden.canTargetEntity`) reads the
    /// scoreboard and the world border, so the write handle answers it around
    /// this; a goal tick refuses the brain-backed warden before it.
    pub fn valid_target(&self, w: &MobWorld, target: &TargetFacts) -> bool {
        match target {
            TargetFacts::Mob {
                id,
                species,
                alive,
                invulnerable,
                baby,
            } => {
                // `asValidTarget`: dead, invulnerable and ghast targets never
                // store.
                if !*alive || *invulnerable || *species == Species::Ghast {
                    return false;
                }
                // Breeze.canAttack names only an iron golem.
                if self.species() == Species::Breeze && *species != Species::IronGolem {
                    return false;
                }
                // Illager.canAttack spares a baby villager or trader.
                if matches!(
                    self.species(),
                    Species::Evoker | Species::Vindicator | Species::Illusioner | Species::Pillager
                ) && matches!(*species, Species::Villager | Species::WanderingTrader)
                    && *baby
                {
                    return false;
                }
                // Creeper.canAttack never names a goat.
                if self.species() == Species::Creeper && *species == Species::Goat {
                    return false;
                }
                !self.spares_target(w, Who::Mob(*id))
            }
            TargetFacts::Player {
                alive,
                invulnerable,
                creative,
                spectator,
                uuid,
            } => {
                // The player rules the write handle always asked: a target
                // that cannot fight, Peaceful, a player-built golem, and a
                // pet never attacking its owner.
                *alive
                    && !*invulnerable
                    && !*creative
                    && !*spectator
                    && w.difficulty.base != 0
                    && !self.built_by_player()
                    && !(self.is_tamable_animal()
                        && self.extra.tame
                        && super::tame::owned_by(self.extra.owner, *uuid))
                    && !(self.species() == Species::Dolphin && self.is_baby())
            }
        }
    }
    /// TargetingConditions.forCombat().test for the player: alive, not
    /// creative, within range scaled by visibility, optionally visible.
    pub fn can_target_player(&mut self, w: &MobWorld, range: f64, check_sight: bool) -> bool {
        let Some(p) = w.selected_player() else {
            return false;
        };
        if self.spares_players(w) {
            return false;
        }
        // TargetingConditions.isCombat: never an ally.
        if p.creative || p.allies.contains(&self.id) {
            return false;
        }
        self.player_in_range(w, range, check_sight, true)
    }
    /// forCombat().ignoreInvisibilityTesting() for the player: the check for
    /// a target the mob already has, which ignores the visibility scaling.
    pub fn can_keep_target_player(&mut self, w: &MobWorld, range: f64) -> bool {
        let Some(p) = w.selected_player() else {
            return false;
        };
        if self.spares_players(w) {
            return false;
        }
        if p.creative || p.allies.contains(&self.id) {
            return false;
        }
        self.player_in_range(w, range, true, false)
    }
    /// TargetingConditions.forNonCombat().range(range).test for the player
    /// (seen by anyone, within range scaled by visibility, in sight).
    pub fn can_see_player(&mut self, w: &MobWorld, range: f64) -> bool {
        self.player_in_range(w, range, true, true)
    }
    /// `TargetingConditions.forNonCombat().range(range).ignoreLineOfSight()`
    /// for the player (still scaled by visibility).
    pub fn can_see_player_through_walls(&mut self, w: &MobWorld, range: f64) -> bool {
        self.player_in_range(w, range, false, true)
    }
    /// `scale` is `TargetingConditions.testInvisible`: scale the range by
    /// the player's visibility.
    fn player_in_range(
        &mut self,
        w: &MobWorld,
        range: f64,
        check_sight: bool,
        scale: bool,
    ) -> bool {
        let Some(p) = w.selected_player() else {
            return false;
        };
        if !p.alive || p.spectator {
            return false;
        }
        // `TargetingConditions.test` measures only `if (this.range > 0.0)`: the unranged
        // `TargetingConditions.DEFAULT` reaches any distance.
        if range > 0.0 {
            // LivingEntity.getVisibilityPercent
            let visibility = if scale {
                w.player_visibility(p, self.id, p.visibility_percent(self.species()))
            } else {
                1.0
            };
            let dist = (range * visibility).max(2.0);
            if self.distance_sqr(p.position) > dist * dist {
                return false;
            }
        }
        !check_sight || self.sees(w, Who::Player)
    }
}

// --- goals ---------------------------------------------------------------

include!("goals/wander.rs");

include!("goals/melee.rs");

include!("goals/target.rs");

/// TemptGoal: follow a player holding one of the items.
#[derive(Clone, Debug)]
pub struct Tempt {
    pub speed: f64,
    pub items: Arc<[String]>,
    pub can_scare: bool,
    pub stop_distance: f64,
    p: [f64; 3],
    rot: [f64; 2],
    calm_down: i32,
}
impl Tempt {
    pub fn new(speed: f64, items: Arc<[String]>, can_scare: bool) -> Self {
        Self {
            speed,
            items,
            can_scare,
            stop_distance: 2.5,
            p: [0.0; 3],
            rot: [0.0; 2],
            calm_down: 0,
        }
    }
    fn wants(&self, item: &Option<Arc<str>>) -> bool {
        item.as_ref()
            .is_some_and(|i| self.items.iter().any(|t| t.as_str() == &**i))
    }
}
impl Goal for Tempt {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.calm_down > 0 {
            self.calm_down -= 1;
            return false;
        }
        let Some(p) = cx.w.selected_player() else {
            return false;
        };
        let range =
            cx.w.player_visibility(p, m.id, p.visibility_percent(m.species()));
        p.alive
            && !p.spectator
            && (self.wants(&p.main_hand) || self.wants(&p.off_hand))
            && m.distance_sqr(p.position) <= (10.0 * range).max(2.0).powi(2)
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.can_scare
            && let Some(p) = cx.w.selected_player()
        {
            if m.distance_sqr(p.position) < 36.0 {
                let moved = (p.position.x - self.p[0]).powi(2)
                    + (p.position.y - self.p[1]).powi(2)
                    + (p.position.z - self.p[2]).powi(2);
                if moved > 0.010_000_000_000_000_002 {
                    return false;
                }
                let (yaw, pitch) = look_angles(p.look);
                if (pitch - self.rot[0]).abs() > 5.0 || (yaw - self.rot[1]).abs() > 5.0 {
                    return false;
                }
            } else {
                self.p = [p.position.x, p.position.y, p.position.z];
            }
            let (yaw, pitch) = look_angles(p.look);
            self.rot = [pitch, yaw];
        }
        self.can_use(m, cx)
    }
    #[inline]
    fn start(&mut self, _: &mut Mob, cx: &mut Cx) {
        if let Some(p) = cx.w.selected_player() {
            self.p = [p.position.x, p.position.y, p.position.z];
        }
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
        self.calm_down = reduced(100);
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = cx.w.selected_player().cloned() else {
            return;
        };
        let (y, x) = (m.kind.max_head_y_rot + 20.0, m.max_head_x_rot());
        m.look_at_speed(p.eye, y, x);
        if m.distance_sqr(p.position) < self.stop_distance * self.stop_distance {
            m.nav_stop();
        } else {
            m.nav_move_to_entity(
                cx.w,
                [p.position.x, p.position.y, p.position.z],
                1,
                self.speed,
            );
        }
    }
}
fn look_angles(look: [f64; 3]) -> (f64, f64) {
    let yaw = (-look[0]).atan2(look[2]).to_degrees();
    let pitch = (-look[1]).asin().to_degrees();
    (yaw, pitch)
}
/// BreedGoal: walk to a partner in love and breed after 60 ticks.
#[derive(Clone, Debug)]
pub struct Breed {
    pub speed: f64,
    partner: Option<u32>,
    love_time: i32,
}
impl Breed {
    pub fn new(speed: f64) -> Self {
        Self {
            speed,
            partner: None,
            love_time: 0,
        }
    }
}
impl Goal for Breed {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.extra.in_love <= 0 {
            return false;
        }
        let species = m.species();
        // Wolf.canMate and Cat.canMate: both tame. The sitting pose sits on the
        // partner side only, and only for a wolf (`wolf.isInSittingPose()`), so
        // a tamed mob ordered to sit still walks off to breed.
        let tamable = matches!(species, Species::Wolf | Species::Cat);
        if tamable && !m.extra.tame {
            return false;
        }
        // Mule keeps AbstractHorse.canMate; CamelHusk.canMate and canFallInLove are false.
        if matches!(species, Species::Mule | Species::CamelHusk) {
            return false;
        }
        // TurtleBreedGoal.canUse: not while carrying an egg.
        if species == Species::Turtle && super::turtle::has_egg(m) {
            return false;
        }
        let id = m.id;
        // Horses and donkeys breed together (AbstractHorse.canParent: tame).
        // Sniffer.canMate: calm states only.
        let sniffer = species == Species::Sniffer;
        if sniffer && !super::sniffer::can_mate_state(m.extra.variant) {
            return false;
        }
        let llama = super::llama::is_llama(species);
        let equine = matches!(species, Species::Horse | Species::Donkey) || llama;
        // Camel.canMate asks AbstractHorse.canParent of both sides: not a
        // vehicle or a passenger and at full health (a camel is always tame).
        let camel = species == Species::Camel;
        // AbstractHorse.canParent: tame, an adult, not a vehicle or passenger
        // and at full health.
        if (equine || camel)
            && (!m.extra.tame
                || m.is_baby()
                || m.rider.is_some()
                || m.extra.rider.is_some()
                || m.extra.mount.is_some()
                || m.health < m.attrs.max_health)
        {
            return false;
        }
        self.partner =
            cx.w.nearby(m.position, 8.0, move |s| {
                let same = s.species == species
                    || llama && super::llama::is_llama(s.species)
                    || !llama && equine && matches!(s.species, Species::Horse | Species::Donkey);
                s.id != id
                    && same
                    && s.in_love
                    && !s.baby
                    && (!tamable || s.tame)
                    // Wolf.canMate: a sitting wolf is no partner.
                    && !(species == Species::Wolf && s.sitting)
                    // AbstractHorse.canParent of the partner.
                    && (!(equine || camel) || s.tame && !s.ridden && s.health >= s.max_health)
                    && (!sniffer || super::sniffer::can_mate_state(s.variant))
                    // BreedGoal.getFreePartner: `!potentialPartner.isPanicking()`.
                    && !s.panicking
            })
            .find(|s| m.distance_sqr(s.position) <= 64.0)
            .map(|s| s.id);
        self.partner.is_some()
    }
    #[inline]
    fn can_continue(&mut self, _: &mut Mob, cx: &mut Cx) -> bool {
        self.partner
            .and_then(|p| cx.w.seen(p))
            // BreedGoal.canContinueToUse: `!this.partner.isPanicking()`.
            .is_some_and(|s| s.alive && s.in_love && !s.panicking)
            && self.love_time < 60
    }
    #[inline]
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.partner = None;
        self.love_time = 0;
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = self.partner.and_then(|p| cx.w.seen(p)).cloned() else {
            return;
        };
        let x = m.max_head_x_rot();
        m.look_at_speed([p.position.x, p.eye_y, p.position.z], 10.0, x);
        m.nav_move_to_entity(
            cx.w,
            [p.position.x, p.position.y, p.position.z],
            1,
            self.speed,
        );
        self.love_time += 1;
        if self.love_time >= reduced(60) && m.distance_sqr(p.position) < 9.0 {
            let equine = super::equine::breed_offspring(m, &p).map(Box::new);
            let parent_variant =
                super::draws_parent_variant(m.species()).then(|| m.random.next_boolean());
            let (wolf_voice, experience) = super::breeding::voice_and_experience(
                m,
                cx.w,
                equine.is_some() || parent_variant.is_some(),
            );
            // NeoForge Animal.spawnChildFromBreeding / Fox.FoxBreedGoal.breed:
            // when a baby_entity_spawn listener is present on a live-baby route, defer the
            // initiating parent's cooldown until Game::breed so an uncanceled empty slot does
            // not leave the parent cooled down.
            if !cx.w.asks_live_birth(m.species()) {
                m.extra.in_love = 0;
                super::finalize::set_age(m, 6000, Some(cx.w));
            }
            // TurtleBreedGoal.breed: the mother carries an egg, not a baby.
            if m.species() == Species::Turtle {
                m.extra.flag = true;
            }
            // Frog.spawnChildFromBreeding: IS_PREGNANT instead of a baby.
            if m.species() == Species::Frog {
                super::frog::set_pregnant(m, true);
            }
            cx.events.push(MobEvent::Breed {
                parent: m.id,
                partner: p.id,
                equine,
                parent_variant,
                wolf_voice,
                experience,
            });
        }
    }
}
/// FollowParentGoal.
#[derive(Clone, Debug)]
pub struct FollowParent {
    pub speed: f64,
    parent: Option<u32>,
    recalc: i32,
}
impl FollowParent {
    pub fn new(speed: f64) -> Self {
        Self {
            speed,
            parent: None,
            recalc: 0,
        }
    }
}
impl Goal for FollowParent {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        0
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.extra.age >= 0 {
            return false;
        }
        let species = m.species();
        let pos = m.position;
        let mut best: Option<(u32, f64)> = None;
        // Every position in range puts the box's bottom face in this area.
        let area = Aabb::new(
            Vec3::new(pos.x - 9.0, pos.y - 5.0, pos.z - 9.0),
            Vec3::new(pos.x + 9.0, pos.y + 5.0, pos.z + 9.0),
        );
        for s in cx.w.others_near(area).filter(|s| {
            s.alive
                && s.species == species
                && !s.baby
                && (s.position.x - pos.x).abs() <= 8.0
                && (s.position.y - pos.y).abs() <= 4.0
                && (s.position.z - pos.z).abs() <= 8.0
        }) {
            let d = m.distance_sqr(s.position);
            if best.is_none_or(|b| d <= b.1) {
                best = Some((s.id, d));
            }
        }
        match best {
            Some((id, d)) if d >= 9.0 => {
                self.parent = Some(id);
                true
            }
            _ => false,
        }
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.extra.age >= 0 {
            return false;
        }
        let Some(p) = self.parent.and_then(|p| cx.w.seen(p)) else {
            return false;
        };
        let d = m.distance_sqr(p.position);
        p.alive && (9.0..=256.0).contains(&d)
    }
    #[inline]
    fn start(&mut self, _: &mut Mob, _: &mut Cx) {
        self.recalc = 0;
    }
    #[inline]
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.parent = None;
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.recalc -= 1;
        if self.recalc <= 0 {
            self.recalc = reduced(10);
            if let Some(p) = self.parent.and_then(|p| cx.w.seen(p)) {
                let at = [p.position.x, p.position.y, p.position.z];
                m.nav_move_to_entity(cx.w, at, 1, self.speed);
            }
        }
    }
}
/// `FollowMobGoal` `(mob, speed, stopDistance, areaSize)`: follows the first
/// visible mob of another class within `area` blocks and keeps `stop` away
/// from it (a parrot hovers by the other animals).
///
/// Left out: vanilla also backs away when the followed mob's look control
/// wants this mob's position; only another `FollowMobGoal` mob of a different
/// class would look at it, and parrots are the only ones that have the goal.
#[derive(Clone, Debug)]
pub struct FollowMob {
    speed: f64,
    stop: f64,
    area: f64,
    following: Option<u32>,
    recalc: i32,
    old_water: f32,
}
impl FollowMob {
    pub fn new(speed: f64, stop: f32, area: f32) -> Self {
        Self {
            speed,
            stop: f64::from(stop),
            area: f64::from(area),
            following: None,
            recalc: 0,
            old_water: 0.0,
        }
    }
}
impl Goal for FollowMob {
    #[inline]
    fn name(&self) -> &'static str {
        "FollowMobGoal"
    }
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    /// `FollowMobGoal.canUse`.
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let species = m.species();
        // `getEntitiesOfClass(Mob, getBoundingBox().inflate(areaSize))`.
        let area = m
            .bounds()
            .inflate(Vec3::new(self.area, self.area, self.area));
        let found =
            cx.w.others_near(area)
                .find(|s| s.species != species && s.bounds.intersects(area) && !s.invisible)
                .map(|s| s.id);
        self.following = found;
        found.is_some()
    }
    /// `FollowMobGoal.canContinueToUse`.
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(at) = self
            .following
            .and_then(|id| cx.w.seen(id))
            .map(|s| s.position)
        else {
            return false;
        };
        !m.ai.nav.is_done() && m.distance_sqr(at) > self.stop * self.stop
    }
    /// `FollowMobGoal.start`.
    #[inline]
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        self.recalc = 0;
        self.old_water = m.malus(super::path::PathType::Water);
        m.set_malus(super::path::PathType::Water, 0.0);
    }
    /// `FollowMobGoal.stop`.
    #[inline]
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        self.following = None;
        m.nav_stop();
        m.set_malus(super::path::PathType::Water, self.old_water);
    }
    /// `FollowMobGoal.tick`.
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some((at, eye)) = self
            .following
            .and_then(|id| cx.w.seen(id))
            .map(|s| (s.position, s.eye_y))
        else {
            return;
        };
        if m.extra.leash.is_some() {
            return;
        }
        let max = m.max_head_x_rot();
        m.look_at_speed([at.x, eye, at.z], 10.0, max);
        self.recalc -= 1;
        if self.recalc <= 0 {
            self.recalc = reduced(10);
            let d = m.distance_sqr(at);
            if d > self.stop * self.stop {
                m.nav_move_to_entity(cx.w, [at.x, at.y, at.z], 1, self.speed);
            } else {
                m.nav_stop();
                // `distSqr <= stopDistance` (not squared, as vanilla writes it):
                // too close, so it moves away from the mob.
                if d <= self.stop {
                    let (dx, dz) = (at.x - m.position.x, at.z - m.position.z);
                    m.nav_move_to(
                        cx.w,
                        [m.position.x - dx, m.position.y, m.position.z - dz],
                        self.speed,
                    );
                }
            }
        }
    }
}
/// RestrictSunGoal: avoid sunlit nodes while it is bright outside.
#[derive(Clone, Debug)]
pub struct RestrictSun;
impl Goal for RestrictSun {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        0
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        cx.w.daylight && m.extra.head_id().is_none()
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.nav.avoid_sun = true;
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.nav.avoid_sun = false;
    }
}
/// `FleeSunGoal.getHidePos`: a spot within 10 blocks that the sky does not
/// see and the mob likes.
pub fn hide_pos(m: &mut Mob, w: &MobWorld) -> Option<[f64; 3]> {
    let (bx, by, bz) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    for _ in 0..10 {
        let p = [
            bx + m.random.next_int_bound(20).unwrap_or(0) - 10,
            by + m.random.next_int_bound(6).unwrap_or(0) - 3,
            bz + m.random.next_int_bound(20).unwrap_or(0) - 10,
        ];
        if !w.can_see_sky(p[0], p[1], p[2]) && m.walk_target_value(w, p) < 0.0 {
            return Some([
                f64::from(p[0]) + 0.5,
                f64::from(p[1]),
                f64::from(p[2]) + 0.5,
            ]);
        }
    }
    None
}
/// FleeSunGoal: find shade while burning.
#[derive(Clone, Debug)]
pub struct FleeSun {
    pub speed: f64,
    wanted: [f64; 3],
}
impl FleeSun {
    pub fn new(speed: f64) -> Self {
        Self {
            speed,
            wanted: [0.0; 3],
        }
    }
}
impl Goal for FleeSun {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.ai.target.is_some()
            || !cx.w.daylight
            || m.fire_ticks <= 0
            || m.extra.head_id().is_some()
        {
            return false;
        }
        let (bx, by, bz) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        if !cx.w.can_see_sky(bx, by, bz) {
            return false;
        }
        match hide_pos(m, cx.w) {
            Some(at) => {
                self.wanted = at;
                true
            }
            None => false,
        }
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.nav_move_to(cx.w, self.wanted, self.speed);
    }
}
/// EatBlockGoal (sheep): eat short grass or turn grass blocks to dirt.
#[derive(Clone, Debug, Default)]
pub struct EatBlock {
    tick: i32,
}
impl Goal for EatBlock {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK | JUMP
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let chance = reduced(if m.is_baby() { 50 } else { 1000 });
        if m.random.next_int_bound(chance).unwrap_or(1) != 0 {
            return false;
        }
        let (x, y, z) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        let here = cx.w.block_name(x, y, z).to_owned();
        cx.w.has_tag("minecraft:edible_for_sheep", &here)
            || cx.w.block_name(x, y - 1, z) == "minecraft:grass_block"
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.tick = reduced(40);
        m.extra.anim_ticks = 40;
        // EatBlockGoal.start: broadcastEntityEvent 10, so the head goes down
        // on every client.
        let at = [m.position.x, m.position.y, m.position.z];
        cx.events.push(MobEvent::EntityEvent {
            mob: m.id,
            event: 10,
            at,
        });
        m.nav_stop();
    }
    #[inline]
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.tick = 0;
    }
    #[inline]
    fn can_continue(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        self.tick > 0
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.tick = (self.tick - 1).max(0);
        if self.tick == reduced(4) {
            let (x, y, z) = (
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            );
            let here = cx.w.block_name(x, y, z).to_owned();
            // EatBlockGoal.tick: both halves of the world change sit behind
            // MOB_GRIEFING (`ifeq 89`, `ifeq 182` in the bytecode); `Mob.ate`
            // runs either way, so a sheep still sheds its wool and ages up.
            if cx.w.has_tag("minecraft:edible_for_sheep", &here) {
                if cx.w.can_grief(m.id) {
                    cx.events.push(MobEvent::BreakBlock {
                        pos: [x, y, z],
                        by: None,
                        drop: false,
                        breaker: None,
                    });
                }
                super::ai::ate(m, cx.events);
            } else if cx.w.block_name(x, y - 1, z) == "minecraft:grass_block" {
                // The particles and the dirt `setBlock` are the same gate.
                if cx.w.can_grief(m.id) {
                    cx.events.push(MobEvent::SetBlockName {
                        mob: Some(m.id),
                        pos: [x, y - 1, z],
                        name: "minecraft:dirt",
                        effect: true,
                        flags: sunrise_world::level::flags::UPDATE_CLIENTS,
                    });
                }
                super::ai::ate(m, cx.events);
            }
        }
    }
}

/// BreathAirGoal: swim up to air when the air supply runs low.
#[derive(Clone, Debug, Default)]
pub struct BreathAir {
    movement_paused: bool,
}
impl BreathAir {
    /// `BreathAirGoal.findAirPosition`: `BlockPos.neighborColumn` walks the
    /// center, north, east, south and west columns, each bottom to top.
    fn air_position(
        [x, y, z]: [i32; 3],
        mut gives_air: impl FnMut([i32; 3]) -> bool,
    ) -> Option<[i32; 3]> {
        for (dx, dz) in [(0, 0), (0, -1), (1, 0), (0, 1), (-1, 0)] {
            for yy in y..=y + 8 {
                let pos = [x + dx, yy, z + dz];
                if gives_air(pos) {
                    return Some(pos);
                }
            }
        }
        None
    }
    fn find_air(m: &mut Mob, cx: &mut Cx) {
        let (x, y, z) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        let d = Self::air_position([x, y, z], |[x, y, z]| {
            let s = cx.w.world.block(x, y, z);
            let f = cx.w.facts.get(s);
            (f.fluid == crate::blocks::Fluid::None || &*f.name == "minecraft:bubble_column")
                && cx
                    .w
                    .data
                    .pathfindable(s, super::data::PathComputationType::Land)
        })
        .unwrap_or([x, (m.position.y + 8.0).floor() as i32, z]);
        m.nav_move_to(
            cx.w,
            [f64::from(d[0]), f64::from(d[1] + 1), f64::from(d[2])],
            1.0,
        );
    }

    fn move_to_air(m: &mut Mob, cx: &mut Cx, pause: bool) -> Option<super::living::TravelPause> {
        Self::find_air(m, cx);
        let input = Vec3::new(
            f64::from(m.ai.xxa),
            f64::from(m.ai.yya),
            f64::from(m.ai.zza),
        );
        m.move_relative(0.02, input);
        m.travel_move(
            cx.w,
            cx.level,
            cx.events,
            pause,
            sunrise_world::travel::TravelMode::Water,
            |_, _, _, _| {},
        )
        .break_value()
    }
}
impl Goal for BreathAir {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.air < 140
    }
    #[inline]
    fn interruptable(&self) -> bool {
        false
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.nav_stop();
        Self::find_air(m, cx);
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let _ = Self::move_to_air(m, cx, false);
    }
    #[inline]
    fn tick_until_movement_pause(
        &mut self,
        m: &mut Mob,
        cx: &mut Cx,
    ) -> Option<super::living::TravelPause> {
        let movement = Self::move_to_air(m, cx, true);
        self.movement_paused = movement.is_some();
        movement
    }
    #[inline]
    fn after_movement(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        std::mem::take(&mut self.movement_paused)
    }
    #[inline]
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.movement_paused = false;
    }
}
/// TryFindLiquidGoal (water): head for nearby water when stranded on land.
#[derive(Clone, Debug, Default)]
pub struct TryFindWater {
    wanted: [f64; 3],
}
impl Goal for TryFindWater {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.ground.on_ground || m.fluids.in_water {
            return false;
        }
        let (bx, by, bz) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        for _ in 0..10 {
            let p = [
                bx + m.random.next_int_bound(20).unwrap_or(0) - 10,
                by + 2 - m.random.next_int_bound(8).unwrap_or(0),
                bz + m.random.next_int_bound(20).unwrap_or(0) - 10,
            ];
            if cx.w.fluid(p[0], p[1], p[2]) == crate::blocks::Fluid::Water {
                self.wanted = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
                return true;
            }
        }
        false
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.nav_move_to(cx.w, self.wanted, 1.0);
    }
}
/// The block a mob stands in (`blockPosition`).
fn block_of(m: &Mob) -> [i32; 3] {
    [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ]
}
/// `MoveToBlockGoal`'s shared state: search, walk to and stay at a block.
#[derive(Clone, Debug, Default)]
pub struct MoveToBlock {
    pub speed: f64,
    pub range: i32,
    pub vertical_range: i32,
    pub vertical_start: i32,
    pub next_start: i32,
    pub try_ticks: i32,
    pub max_stay: i32,
    pub reached: bool,
    pub block: [i32; 3],
}
impl MoveToBlock {
    pub fn new(speed: f64, range: i32, vertical_range: i32) -> Self {
        Self {
            speed,
            range,
            vertical_range,
            ..Default::default()
        }
    }
    /// `canUse` with a custom `nextStartTick` delay.
    pub fn can_use(
        &mut self,
        m: &mut Mob,
        w: &MobWorld,
        next: i32,
        valid: impl Fn(&MobWorld, [i32; 3]) -> bool,
    ) -> bool {
        if self.next_start > 0 {
            self.next_start -= 1;
            return false;
        }
        self.next_start = next;
        self.find_nearest(m, w, valid)
    }
    /// `MoveToBlockGoal.findNearestBlock`: rings outward, alternating heights.
    pub fn find_nearest(
        &mut self,
        m: &Mob,
        w: &MobWorld,
        valid: impl Fn(&MobWorld, [i32; 3]) -> bool,
    ) -> bool {
        match self.walk(block_of(m), |p| valid(w, p)) {
            Some(p) => {
                self.block = p;
                true
            }
            None => false,
        }
    }
    /// `findNearestBlock`'s walk out from `base`: the first cell `valid` takes.
    fn walk(&self, base: [i32; 3], mut valid: impl FnMut([i32; 3]) -> bool) -> Option<[i32; 3]> {
        let next = |v: i32| if v > 0 { -v } else { 1 - v };
        let mut y = self.vertical_start;
        while y <= self.vertical_range {
            for r in 0..self.range {
                let mut x = 0;
                while x <= r {
                    let mut z = if x < r && x > -r { r } else { 0 };
                    while z <= r {
                        let p = [base[0] + x, base[1] + y - 1, base[2] + z];
                        if valid(p) {
                            return Some(p);
                        }
                        z = next(z);
                    }
                    x = next(x);
                }
            }
            y = next(y);
        }
        None
    }
    /// The cells [`Self::walk`] can reach from `base`, as a box (inclusive), or
    /// `None` when it walks none.
    fn walk_bounds(&self, base: [i32; 3]) -> Option<([i32; 3], [i32; 3])> {
        if self.range <= 0 || self.vertical_start > self.vertical_range {
            return None;
        }
        // The heights go out from `vertical_start` both ways, past neither it
        // nor `vertical_range` by more than one, a block below.
        let reach = self.range - 1;
        let height = self
            .vertical_start
            .saturating_abs()
            .max(self.vertical_range.saturating_abs())
            .saturating_add(1);
        Some((
            [
                base[0].saturating_sub(reach),
                base[1].saturating_sub(height).saturating_sub(1),
                base[2].saturating_sub(reach),
            ],
            [
                base[0].saturating_add(reach),
                base[1].saturating_add(height),
                base[2].saturating_add(reach),
            ],
        ))
    }
    /// Whether [`Self::find_nearest`] may find a block whose state `matches`
    /// (its `valid` needs one): false when no section of the cells it walks
    /// holds such a state in its palette, so the walk would find nothing.
    pub fn may_find(
        &self,
        m: &Mob,
        w: &MobWorld,
        matches: impl FnMut(sunrise_world::registry::BlockStateId) -> bool,
    ) -> bool {
        self.walk_bounds(block_of(m))
            .is_some_and(|(lo, hi)| w.world.may_hold(lo, hi, matches))
    }
    pub fn can_continue(&self, w: &MobWorld, valid: impl Fn(&MobWorld, [i32; 3]) -> bool) -> bool {
        self.try_ticks >= -self.max_stay && self.try_ticks <= 1200 && valid(w, self.block)
    }
    pub fn start(&mut self, m: &mut Mob, w: &MobWorld) {
        let b = self.block;
        m.nav_move_to(
            w,
            [
                f64::from(b[0]) + 0.5,
                f64::from(b[1] + 1),
                f64::from(b[2]) + 0.5,
            ],
            self.speed,
        );
        self.try_ticks = 0;
        let r = m.random.next_int_bound(1200).unwrap_or(0) + 1200;
        self.max_stay = m.random.next_int_bound(r).unwrap_or(0) + 1200;
    }
    /// `MoveToBlockGoal.tick` (accepted distance 1 from the block above).
    pub fn tick(&mut self, m: &mut Mob, w: &MobWorld) {
        self.tick_within(m, w, 1.0);
    }
    /// `MoveToBlockGoal.tick` with the goal's `acceptedDistance`.
    pub fn tick_within(&mut self, m: &mut Mob, w: &MobWorld, accepted: f64) {
        let t = [
            f64::from(self.block[0]) + 0.5,
            f64::from(self.block[1] + 1) + 0.5,
            f64::from(self.block[2]) + 0.5,
        ];
        let d2 = (m.position.x - t[0]).powi(2)
            + (m.position.y - t[1]).powi(2)
            + (m.position.z - t[2]).powi(2);
        if d2 >= accepted * accepted {
            self.reached = false;
            self.try_ticks += 1;
            if self.try_ticks % 40 == 0 {
                m.nav_move_to(w, [t[0], t[1] - 0.5, t[2]], self.speed);
            }
        } else {
            self.reached = true;
            self.try_ticks -= 1;
        }
    }
}

#[cfg(test)]
mod breeding_variant_tests {
    use super::*;
    use crate::mobs::{BreedExperience, world::with_test_world};
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, random::RandomSource};

    #[test]
    #[ignore = "daily"]
    fn chicken_breeding_coin_precedes_same_tick_egg_with_or_without_listener() {
        #[derive(Debug, Clone)]
        struct SeedAtBreed(Breed);
        impl Goal for SeedAtBreed {
            goal_clone!();
            #[inline]
            fn flags(&self) -> u8 {
                MOVE | LOOK
            }
            #[inline]
            fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
                true
            }
            #[inline]
            fn can_continue(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
                true
            }
            #[inline]
            fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
                m.random = sunrise_core::random::LegacyRandom::new(1);
                self.0.tick(m, cx);
            }
        }
        assert!(with_test_world(&[], |world, types| {
            let kind = Arc::new(types.types["minecraft:chicken"].clone());
            let mut mate = Mob::new(2, kind.clone(), Vec3::new(1.0, 64.0, 0.0), 0.0);
            mate.extra.in_love = 600;
            let partner = [mate.seen(None, &crate::items::Items::empty())];
            for asks_baby_spawn in [false, true] {
                let w = MobWorld {
                    side: crate::motion::dry_side(),
                    goal_host: None,
                    asks_baby_spawn,
                    mob_drops: false,
                    others: &partner,
                    ..world.clone()
                };
                let mut parent = Mob::new(1, kind.clone(), Vec3::new(0.0, 64.0, 0.0), 0.0);
                parent.persistent = true;
                parent.extra.in_love = 600;
                parent.extra.egg_time = 1;
                parent.ai.targets.clear();
                parent.ai.goals = GoalSelector {
                    generation: 0,
                    goals: vec![Wrapped {
                        priority: 2,
                        running: true,
                        goal: Box::new(SeedAtBreed(Breed {
                            speed: 1.0,
                            partner: Some(2),
                            love_time: reduced(60) - 1,
                        })),
                    }],
                    locked: [Some(0), Some(0), None, None],
                    disabled: 0,
                    brain_starters: 0,
                };
                let mut events = Vec::new();
                parent.tick(
                    &w,
                    &mut crate::health::TestLevel::new(1).level(),
                    &mut events,
                );
                assert!(events.iter().any(|event| matches!(
                    event,
                    MobEvent::Breed {
                        parent_variant: Some(true),
                        ..
                    }
                )));
                assert_eq!(parent.extra.egg_time, 8313);
                assert_eq!(parent.extra.age, if asks_baby_spawn { 0 } else { 5999 });
            }
        }));
    }

    #[test]
    #[ignore = "daily"]
    fn variant_and_experience_are_drawn_in_order_before_later_goal_work() {
        assert!(with_test_world(&[], |world, types| {
            for id in [
                "minecraft:pig",
                "minecraft:cow",
                "minecraft:chicken",
                "minecraft:fox",
                "minecraft:cat",
                "minecraft:wolf",
            ] {
                let kind = Arc::new(types.types[id].clone());
                let partner = [Mob::new(2, kind.clone(), Vec3::new(1.0, 64.0, 0.0), 0.0)
                    .seen(None, &crate::items::Items::empty())];
                for (ask_mods, mob_drops) in
                    [(false, true), (false, false), (true, true), (true, false)]
                {
                    let world = MobWorld {
                        side: crate::motion::dry_side(),
                        goal_host: None,
                        asks_baby_spawn: ask_mods,
                        mob_drops,
                        others: &partner,
                        ..world.clone()
                    };
                    let mut parent = Mob::new(1, kind.clone(), Vec3::new(0.0, 64.0, 0.0), 0.0);
                    parent.random = sunrise_core::random::LegacyRandom::new(0);
                    let mut goal = Breed {
                        speed: 1.0,
                        partner: Some(2),
                        love_time: reduced(60) - 1,
                    };
                    let mut events = Vec::new();
                    let mut test_level = crate::health::TestLevel::new(1);
                    let level = &mut test_level.level();
                    let mut cx = Cx {
                        w: &world,
                        level,
                        events: &mut events,
                    };
                    goal.tick(&mut parent, &mut cx);
                    let (choice, experience) = events
                        .iter()
                        .find_map(|event| match event {
                            MobEvent::Breed {
                                parent_variant,
                                experience,
                                ..
                            } => Some((*parent_variant, *experience)),
                            _ => None,
                        })
                        .expect("the animals breed");
                    assert_eq!(choice, Some(true), "{id}");
                    assert_eq!(
                        experience,
                        if ask_mods || id == "minecraft:fox" {
                            BreedExperience::Deferred
                        } else {
                            // Computed with an independent implementation of the documented
                            // java.util.Random algorithm, not from the game's output.
                            // Seed 0: coat is draw 1, wolf voice draw 2,
                            // XP draw 3 (bits31 = 516548029, remainder 4).
                            BreedExperience::Drawn(mob_drops.then_some(if id == "minecraft:wolf" {
                                5
                            } else {
                                3
                            }))
                        },
                        "{id}"
                    );
                    assert_eq!(
                        parent.random.state(),
                        if !ask_mods && id == "minecraft:wolf" && mob_drops {
                            67_704_983_304_270 // Java LCG seed 0, draw 3.
                        } else if !ask_mods
                            && (id == "minecraft:wolf" || (mob_drops && id != "minecraft:fox"))
                        {
                            234_029_835_660_143
                        } else {
                            205_749_139_540_596
                        },
                        "{id}"
                    );
                    if !ask_mods && mob_drops && id != "minecraft:fox" {
                        assert_eq!(
                            parent.random.next_int(),
                            if id == "minecraft:wolf" {
                                -1_690_734_402 // Java LCG seed 0, signed upper 32 bits of draw 4.
                            } else {
                                1_033_096_058
                            },
                            "{id}: same-tick later draw"
                        );
                    }
                }
            }
        }));
    }
}

#[cfg(test)]
mod eat_block_tests {
    use super::*;
    use crate::mobs::world::with_built_test_world_rules;

    /// A sheep on a wide grass shelf, ticked long enough for its `EatBlockGoal`
    /// (1 in 1000 a tick) to fire several times: every event it pushed, with
    /// the `mob_griefing` game rule at `griefing` and `mob_drops` on. `tufts`
    /// puts short grass at the sheep's own feet, so the goal takes the
    /// `destroyBlock` half; without it the sheep stands on the grass and takes
    /// the grass-to-dirt half.
    fn sheep_events(griefing: bool, tufts: bool) -> Option<Vec<MobEvent>> {
        sheep_events_hooked(griefing, tufts, None)
    }

    fn sheep_events_hooked(
        griefing: bool,
        tufts: bool,
        hooks: Option<&dyn crate::mobs::MobHooks>,
    ) -> Option<Vec<MobEvent>> {
        let mut out = None;
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            let grass = registry.default_state("minecraft:grass_block").unwrap();
            let tuft = registry.default_state("minecraft:short_grass").unwrap();
            for cx in -3..3 {
                for cz in -3..3 {
                    world
                        .chunks
                        .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                }
            }
            for x in -8..=8 {
                for z in -8..=8 {
                    world.set(x, 63, z, grass).unwrap();
                    if tufts {
                        world.set(x, 64, z, tuft).unwrap();
                    }
                }
            }
        };
        with_built_test_world_rules(&[], griefing, true, build, |w, types| {
            let mut hooked = w.clone();
            hooked.mob_hooks = hooks;
            let w = &hooked;
            let kind = Arc::new(types.types["minecraft:sheep"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            let mut test_level = crate::health::TestLevel::new(0);
            let level = &mut test_level.level();
            let mut events = Vec::new();
            for _ in 0..6000 {
                crate::mobs::ai::tick(&mut m, w, level, &mut events);
            }
            out = Some(events);
        });
        out
    }

    /// `EatBlockGoal.tick` puts the `levelEvent` + `setBlock` behind
    /// `MOB_GRIEFING` and leaves `Mob.ate` outside it, so with the rule off the
    /// goal still runs (`EntityEvent` 10) and still shears the coat, and the
    /// grass stays.
    #[test]
    #[ignore = "daily"]
    fn the_eat_goal_turns_grass_to_dirt_only_with_mob_griefing() {
        let Some(on) = sheep_events(true, false) else {
            return;
        };
        let Some(off) = sheep_events(false, false) else {
            return;
        };
        let dirt = |events: &[MobEvent]| {
            events
                .iter()
                .filter(|e| matches!(e, MobEvent::SetBlockName { name, .. } if *name == "minecraft:dirt"))
                .count()
        };
        let ate = |events: &[MobEvent]| {
            events
                .iter()
                .filter(|e| matches!(e, MobEvent::EntityEvent { event: 10, .. }))
                .count()
        };
        assert!(dirt(&on) > 0, "mob_griefing on: {}", dirt(&on));
        assert_eq!(dirt(&off), 0, "mob_griefing off must leave the grass alone");
        assert!(ate(&off) > 0, "the goal still runs with the rule off");
    }

    /// Grants griefing to mob 1 only: what a mod's `entity_mob_griefing` does.
    struct GriefFor1;
    impl crate::mobs::MobHooks for GriefFor1 {
        fn enderman_anger_suppressed(&self, _: u32, _: &crate::mobs::PlayerView) -> bool {
            false
        }
        fn can_grief(&self, mob: u32, rule: bool) -> bool {
            rule || mob == 1
        }
        fn grief_hooked(&self) -> bool {
            true
        }
        fn living_visibility(&self, _: &crate::mobs::PlayerView, _: u32, visibility: f64) -> f64 {
            visibility
        }
    }

    /// NeoForge's `canEntityGrief`: with the rule off, a mod's yes for this
    /// sheep lets it eat the grass (and only through `MobWorld::can_grief`).
    #[test]
    #[ignore = "daily"]
    fn a_mods_grief_answer_overrides_the_rule_for_its_mob() {
        let Some(off) = sheep_events_hooked(false, false, Some(&GriefFor1)) else {
            return;
        };
        let dirt = off
            .iter()
            .filter(
                |e| matches!(e, MobEvent::SetBlockName { name, .. } if *name == "minecraft:dirt"),
            )
            .count();
        assert!(dirt > 0, "the hook's yes eats the grass with the rule off");
    }

    /// The edible half of the same gate (`ifeq 89` skips
    /// `Level.destroyBlock(pos, false)`): short grass at the sheep's feet
    /// disappears with the rule on and stays with it off, while the goal eats
    /// either way.
    #[test]
    #[ignore = "daily"]
    fn the_eat_goal_breaks_short_grass_only_with_mob_griefing() {
        let Some(on) = sheep_events(true, true) else {
            return;
        };
        let Some(off) = sheep_events(false, true) else {
            return;
        };
        let bitten = |events: &[MobEvent]| {
            events
                .iter()
                .filter(|e| matches!(e, MobEvent::BreakBlock { drop: false, .. }))
                .count()
        };
        let ate = |events: &[MobEvent]| {
            events
                .iter()
                .filter(|e| matches!(e, MobEvent::EntityEvent { event: 10, .. }))
                .count()
        };
        assert!(bitten(&on) > 0, "mob_griefing on: {}", bitten(&on));
        assert_eq!(
            bitten(&off),
            0,
            "mob_griefing off must leave the tufts alone"
        );
        assert!(ate(&off) > 0, "the goal still runs with the rule off");
    }
}

#[cfg(test)]
mod built_by_player_tests {
    use super::*;

    /// A mob of `species`, from a made-up type.
    fn mob(species: Species) -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        Arc::make_mut(&mut kind).species = species;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// `IronGolem.canAttack`: only a golem a player built spares players, so
    /// the save restore drops that target (`Mob.asValidTarget`) while another
    /// mob's `PlayerCreated` byte means nothing.
    #[test]
    fn only_a_golem_spares_the_player_that_built_it() {
        let mut built = mob(Species::IronGolem);
        built.extra.player_created = true;
        assert!(built.built_by_player());
        assert!(!mob(Species::IronGolem).built_by_player());
        let mut zombie = mob(Species::Zombie);
        zombie.extra.player_created = true;
        assert!(!zombie.built_by_player());
    }

    #[test]
    fn pets_and_horses_are_tamable_but_an_allay_is_not() {
        for s in [
            Species::Wolf,
            Species::Parrot,
            Species::Horse,
            Species::Camel,
        ] {
            assert!(mob(s).is_tamable(), "{s:?}");
        }
        for s in [Species::Allay, Species::Fox, Species::Zombie] {
            assert!(!mob(s).is_tamable(), "{s:?}");
        }
    }
}

#[cfg(test)]
mod remove_all_tests {
    use super::*;
    use crate::mobs::world::with_built_test_world_rules;
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    /// A goal that always wants to walk and leaves a mark while it runs, so
    /// `removeAllGoals` has one running to stop.
    #[derive(Clone, Debug)]
    struct Walk;
    impl Goal for Walk {
        goal_clone!();
        #[inline]
        fn flags(&self) -> u8 {
            MOVE
        }
        #[inline]
        fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
            true
        }
        #[inline]
        fn start(&mut self, m: &mut Mob, _: &mut Cx) {
            m.ai.panicking = true;
        }
        #[inline]
        fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
            m.ai.panicking = false;
        }
        #[inline]
        fn name(&self) -> &'static str {
            "Walk"
        }
    }

    /// `removeAllGoals` stops a running goal before it goes, frees the flag
    /// lock it held, and moves the locks behind the removed one, so the next
    /// tick still finds the survivor that kept its place.
    #[test]
    #[ignore = "daily"]
    fn remove_all_stops_the_running_goal_and_moves_the_locks() {
        let ran = with_built_test_world_rules(
            &[],
            true,
            true,
            |_, _| {},
            |w, types| {
                let kind = Arc::new(types.types["minecraft:zombie"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                // `Mob::new` registered the species' own goals; only the ones
                // this test adds matter here.
                let mut goals = {
                    let mut goals = std::mem::take(&mut m.ai.goals);
                    goals.clear();
                    goals
                };
                goals.add(0, Float);
                goals.add(1, Walk);
                let mut test_level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                {
                    let mut cx = Cx {
                        w,
                        level: &mut test_level.level(),
                        events: &mut events,
                    };
                    goals.tick(&mut m, &mut cx);
                    // The Walk runs; Float needs water the empty world has none of.
                    assert_eq!(
                        goals.entries().collect::<Vec<_>>(),
                        vec![(0, "Float", false), (1, "Walk", true)]
                    );
                    assert!(m.ai.panicking);
                    // The Walk before it goes: its lock index moves with the list.
                    let removed = goals.remove_all(|g| g.name() == "Float", &mut m, &mut cx);
                    assert_eq!(removed, 1);
                    // A stale lock would point past the list's end here.
                    goals.tick(&mut m, &mut cx);
                    assert_eq!(goals.entries().collect::<Vec<_>>(), vec![(1, "Walk", true)]);
                    // The running Walk's own removal stops it.
                    let removed = goals.remove_all(|g| g.name() == "Walk", &mut m, &mut cx);
                    assert_eq!(removed, 1);
                }
                assert!(!m.ai.panicking);
                assert_eq!(goals.entries().count(), 0);
                m.ai.goals = goals;
            },
        );
        assert!(ran, "the test world's resources are missing");
    }
}
#[cfg(test)]
mod avoid_entity_targeting_tests {
    use super::*;
    use crate::mobs::world::test_player;
    use crate::mobs::world::{with_built_test_world, with_test_world};
    use std::sync::Arc;

    /// `(the player is inside the plain distance range, the goal picks them)`.
    /// The wall stands at z 5..6, between the rabbit at z 0.5 and the player at
    /// z 10.5, so sight is the only thing that changes between the two.
    fn target(difficulty: u8, wall: bool) -> Option<(bool, bool)> {
        let build = move |world: &mut crate::world::World,
                          registry: &sunrise_world::registry::BlockRegistry| {
            for cx in -1..1 {
                for cz in -1..1 {
                    world.chunks.insert(
                        (cx, cz),
                        std::sync::Arc::new(crate::chunk::Chunk::empty(cx, cz)),
                    );
                }
            }
            let stone = registry.default_state("minecraft:stone").unwrap();
            for x in -4..5 {
                for z in -4..15 {
                    world.set(x, 64, z, stone).unwrap();
                }
            }
            if wall {
                for x in -4..5 {
                    for y in 65..68 {
                        for z in 5..7 {
                            world.set(x, y, z, stone).unwrap();
                        }
                    }
                }
            }
        };
        let mut out = None;
        let ran = with_built_test_world(&[], build, |w, types| {
            let mut world = w.clone();
            world.difficulty = super::super::spawning::LocalDifficulty::new(difficulty, 0, 0, 0.0);
            let player = test_player(Vec3::new(0.5, 65.0, 10.5));
            world.player = Some(player.clone());
            let mut m = Mob::new(
                1,
                Arc::new(types.types["minecraft:rabbit"].clone()),
                Vec3::new(0.5, 65.0, 0.5),
                0.0,
            );
            let mut goal = AvoidEntity::new(Avoid::Player, 20.0, 1.0, 1.0);
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            let mut cx = Cx {
                w: &world,
                level: &mut test_level.level(),
                events: &mut events,
            };
            let in_range = m.distance_sqr(player.position) <= 20.0 * 20.0;
            out = Some((in_range, goal.avoid_target(&mut m, &mut cx).is_some()));
        });
        ran.then_some(out).flatten()
    }

    /// `Sensing.hasLineOfSight`: the player is ten blocks away, well inside the
    /// 20 block range, but the wall is between them, so the goal must not pick
    /// them (it did before: only the distance was checked).
    #[test]
    #[ignore = "daily"]
    fn a_rabbit_does_not_flee_a_player_behind_a_wall() {
        if let Some((in_range, picked)) = target(2, true) {
            assert!(in_range, "the player is inside the plain distance range");
            assert!(!picked, "the rabbit picked a player it cannot see");
        }
    }

    /// `LivingEntity.canAttack`: in Peaceful no player is a target at all, so
    /// the goal does not pick them even in plain sight.
    #[test]
    #[ignore = "daily"]
    fn a_rabbit_does_not_flee_in_peaceful() {
        if let Some((in_range, picked)) = target(0, false) {
            assert!(in_range, "the player is inside the plain distance range");
            assert!(!picked, "the rabbit picked a player in Peaceful");
        }
    }

    /// The control: the same world on normal difficulty with nothing in the way
    /// still picks the player.
    #[test]
    #[ignore = "daily"]
    fn a_rabbit_flees_a_player_it_can_see() {
        if let Some((in_range, picked)) = target(2, false) {
            assert!(in_range, "the player is inside the plain distance range");
            assert!(picked, "the rabbit ignored a player in the open");
        }
    }

    /// Upstream's visibility regression, also through the selected-guest path.
    #[test]
    #[ignore = "daily"]
    fn an_invisible_player_tempts_only_within_two_blocks() {
        let mut outcomes = Vec::new();
        if !with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:cow"].clone());
            for guest in [false, true] {
                for (invisible, x) in [(false, 5.5), (true, 5.5), (true, 1.5)] {
                    let mut world = w.clone();
                    let mut p = test_player(Vec3::new(x, 64.0, 0.5));
                    p.main_hand = Some("minecraft:wheat".into());
                    p.invisible = invisible;
                    let selection = super::super::player_selection::PlayerSelection::new(
                        vec![super::super::player_selection::TickPlayer {
                            entity_id: 42,
                            connection: Some(7),
                            view: p.clone(),
                        }],
                        Some(42),
                    )
                    .unwrap();
                    world.player = Some(if guest {
                        test_player(Vec3::new(100.0, 64.0, 0.5))
                    } else {
                        p
                    });
                    world.player_selection = guest.then_some(&selection);
                    let mut m = Mob::new(1, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
                    m.ai.goals.add(
                        3,
                        Tempt::new(1.25, Arc::from(vec!["minecraft:wheat".to_string()]), false),
                    );
                    let mut level = crate::health::TestLevel::new(0);
                    let mut events = Vec::new();
                    let mut cx = Cx {
                        w: &world,
                        level: &mut level.level(),
                        events: &mut events,
                    };
                    let mut goals = std::mem::take(&mut m.ai.goals);
                    goals.tick(&mut m, &mut cx);
                    outcomes.push(goals.is_running("Tempt"));
                }
            }
        }) {
            return;
        }
        assert_eq!(outcomes, vec![true, false, true, true, false, true]);
    }

    /// `TemptGoal` finds its player with `TargetingConditions.forNonCombat()`,
    /// which never picks a spectator: a spectator holding wheat does not tempt
    /// an animal.
    #[test]
    #[ignore = "daily"]
    fn a_spectator_does_not_tempt_an_animal() {
        let mut outcomes = Vec::new();
        if !with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:cow"].clone());
            for spectator in [false, true] {
                let mut world = w.clone();
                let mut p = test_player(Vec3::new(2.5, 64.0, 0.5));
                p.main_hand = Some("minecraft:wheat".into());
                p.spectator = spectator;
                world.player = Some(p);
                let mut m = Mob::new(1, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
                m.ai.goals.add(
                    3,
                    Tempt::new(1.25, Arc::from(vec!["minecraft:wheat".to_string()]), false),
                );
                let mut lr = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut cx = Cx {
                    w: &world,
                    level: &mut lr.level(),
                    events: &mut events,
                };
                let mut goals = std::mem::take(&mut m.ai.goals);
                goals.tick(&mut m, &mut cx);
                outcomes.push(goals.is_running("Tempt"));
                m.ai.goals = goals;
            }
        }) {
            return;
        }
        assert_eq!(
            outcomes,
            vec![true, false],
            "a survival player tempts; a spectator does not"
        );
    }
}

#[cfg(test)]
mod peaceful_targets_tests {
    use super::*;
    use crate::mobs::world::{test_player, with_test_world};

    /// `LivingEntity.canAttack` / `TargetingConditions.test`: in Peaceful a
    /// hurt wolf neither takes its attacker (a player) as target nor picks one
    /// by sight, while at Normal it does.
    #[test]
    #[ignore = "daily"]
    fn peaceful_mobs_never_target_the_player() {
        assert!(
            with_test_world(&[], |w, types| {
                let mut w = w.clone();
                w.player = Some(test_player(Vec3::new(2.5, 64.0, 0.5)));
                let kind = Arc::new(types.types["minecraft:wolf"].clone());
                let mut wolf = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                wolf.ai.last_hurt_by = Some(Who::Player);
                wolf.ai.last_hurt_by_time = 5;
                let mut test_level = crate::health::TestLevel::new(0);
                let level = &mut test_level.level();
                let mut events = Vec::new();
                let mut cx = Cx {
                    w: &w,
                    level,
                    events: &mut events,
                };
                assert!(
                    HurtByTarget::default().can_use(&mut wolf, &mut cx),
                    "normal"
                );
                assert!(wolf.can_target_player(&w, 16.0, false), "normal");

                let mut peaceful = w.clone();
                peaceful.difficulty = crate::mobs::spawning::LocalDifficulty::new(0, 18000, 0, 0.0);
                let mut cx = Cx {
                    w: &peaceful,
                    level,
                    events: &mut events,
                };
                assert!(
                    !HurtByTarget::default().can_use(&mut wolf, &mut cx),
                    "peaceful"
                );
                assert!(!wolf.can_target_player(&peaceful, 16.0, false), "peaceful");
                assert!(!wolf.can_keep_target_player(&peaceful, 16.0), "peaceful");
            }),
            "vanilla data is missing"
        );
    }

    /// `Mob.canAttack` refuses a Ghast, and `HurtByTargetGoal.canUse` ends
    /// with `canAttack(lastHurtByMob)`: a ghast's fireball is not retaliated.
    #[test]
    #[ignore = "daily"]
    fn a_hurt_mob_does_not_retaliate_against_a_ghast() {
        let mut seen = None;
        assert!(
            with_test_world(&[], |_, types| {
                let mob = |id: u32, name: &str, at: Vec3| {
                    Mob::new(id, Arc::new(types.types[name].clone()), at, 0.0)
                };
                let ghast = mob(2, "minecraft:ghast", Vec3::new(2.5, 64.0, 0.5));
                let zombie = mob(3, "minecraft:zombie", Vec3::new(3.5, 64.0, 0.5));
                seen = Some([
                    ghast.seen(None, &crate::items::Items::empty()),
                    zombie.seen(None, &crate::items::Items::empty()),
                ]);
            }),
            "vanilla data is missing"
        );
        let Some(seen) = seen else { return };
        assert!(
            with_test_world(&seen, |w, types| {
                let kind = Arc::new(types.types["minecraft:cow"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                let mut lr = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut cx = Cx {
                    w,
                    level: &mut lr.level(),
                    events: &mut events,
                };
                m.ai.last_hurt_by = Some(Who::Mob(2));
                m.ai.last_hurt_by_time = 5;
                assert!(
                    !HurtByTarget::default().can_use(&mut m, &mut cx),
                    "a ghast is not a valid attacker to retaliate against"
                );
                m.ai.last_hurt_by = Some(Who::Mob(3));
                m.ai.last_hurt_by_time = 6;
                assert!(
                    HurtByTarget::default().can_use(&mut m, &mut cx),
                    "a zombie attacker is retaliated against"
                );
            }),
            "vanilla data is missing"
        );
    }
}
#[cfg(test)]
mod move_to_block_tests {
    use super::MoveToBlock;

    /// Every cell the walk tries lies in the box `may_find` asks the
    /// palettes about, for the ranges the goals use and odd ones; and the
    /// walk stops at the first cell it is given.
    #[test]
    fn the_walk_stays_in_its_bounds() {
        for range in [-1, 0, 1, 2, 8, 24] {
            for vertical_range in [-2, 0, 1, 2, 3, 6] {
                for vertical_start in [-3, -1, 0, 1, 2] {
                    let goal = MoveToBlock {
                        range,
                        vertical_range,
                        vertical_start,
                        ..Default::default()
                    };
                    for base in [[0, 64, 0], [-17, -60, 31], [100, 319, -100]] {
                        let bounds = goal.walk_bounds(base);
                        let mut walked = 0;
                        let found = goal.walk(base, |p| {
                            walked += 1;
                            let (lo, hi) = bounds.expect("a walk with no bounds");
                            assert!((0..3).all(|a| lo[a] <= p[a] && p[a] <= hi[a]), "{p:?}");
                            false
                        });
                        assert_eq!(found, None);
                        assert_eq!(
                            walked > 0,
                            bounds.is_some(),
                            "{range} {vertical_range} {vertical_start}"
                        );
                    }
                }
            }
        }
        let goal = MoveToBlock::new(1.0, 24, 2);
        assert_eq!(goal.walk([5, 64, 5], |_| true), Some([5, 63, 5]));
    }
}

#[cfg(test)]
mod visibility_hook_tests {
    use crate::mobs::world::{test_player, with_test_world};
    use crate::mobs::{Mob, MobHooks, PlayerView};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    /// A mod's `living_visibility`: a quarter as visible to mob 1, a
    /// hundred times as visible to anyone else.
    struct Scale;
    impl MobHooks for Scale {
        fn enderman_anger_suppressed(&self, _: u32, _: &PlayerView) -> bool {
            false
        }
        fn can_grief(&self, _: u32, rule: bool) -> bool {
            rule
        }
        fn grief_hooked(&self) -> bool {
            false
        }
        fn living_visibility(&self, _: &PlayerView, looking: u32, visibility: f64) -> f64 {
            if looking == 1 {
                visibility * 0.25
            } else {
                visibility * 100.0
            }
        }
    }

    /// NeoForge's `LivingVisibilityEvent` scales the range a targeting check
    /// sees the player from, and `getVisibilityPercent` clamps it to 10.
    #[test]
    #[ignore = "daily"]
    fn a_mods_visibility_scales_the_targeting_range() {
        assert!(
            with_test_world(&[], |w, types| {
                let mut w = w.clone();
                let kind = Arc::new(types.types["minecraft:zombie"].clone());
                let mut near = Mob::new(1, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
                let mut far = Mob::new(2, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                w.player = Some(test_player(Vec3::new(10.5, 64.0, 0.5)));
                assert!(near.can_see_player_through_walls(&w, 16.0));
                w.mob_hooks = Some(&Scale);
                assert!(
                    !near.can_see_player_through_walls(&w, 16.0),
                    "16 x 0.25 = 4 blocks"
                );
                // x100 clamps to 10: 160 blocks.
                w.player = Some(test_player(Vec3::new(150.5, 64.0, 0.5)));
                assert!(far.can_see_player_through_walls(&w, 16.0));
                w.player = Some(test_player(Vec3::new(170.5, 64.0, 0.5)));
                assert!(!far.can_see_player_through_walls(&w, 16.0));
            }),
            "vanilla data is missing"
        );
    }

    /// A worn head's `mob_visibility` halves the range only for the types
    /// vanilla names: a zombie head fools a zombie, not a husk, and a
    /// skeleton skull a skeleton, not a stray.
    #[test]
    #[ignore = "daily"]
    fn a_worn_head_hides_the_player_only_from_its_own_type() {
        assert!(
            with_test_world(&[], |w, types| {
                let mut w = w.clone();
                let mob = |id, name: &str| {
                    let kind = Arc::new(types.types[name].clone());
                    Mob::new(id, kind, Vec3::new(0.5, 64.0, 0.5), 0.0)
                };
                let wearing = |head: &str| {
                    let mut p = test_player(Vec3::new(10.5, 64.0, 0.5));
                    p.head = Some(head.into());
                    p
                };
                // 16 blocks, halved to 8: the player 10 blocks away is unseen.
                for (looking, head, hidden) in [
                    ("minecraft:zombie", "minecraft:zombie_head", true),
                    ("minecraft:husk", "minecraft:zombie_head", false),
                    ("minecraft:skeleton", "minecraft:skeleton_skull", true),
                    ("minecraft:stray", "minecraft:skeleton_skull", false),
                    ("minecraft:creeper", "minecraft:creeper_head", true),
                    ("minecraft:piglin_brute", "minecraft:piglin_head", true),
                    ("minecraft:zombie", "minecraft:skeleton_skull", false),
                ] {
                    w.player = Some(wearing(head));
                    let mut m = mob(1, looking);
                    assert_eq!(
                        m.can_see_player_through_walls(&w, 16.0),
                        !hidden,
                        "{looking} looking at {head}"
                    );
                }
            }),
            "vanilla data is missing"
        );
    }
}

#[cfg(test)]
mod target_change_hook_tests {
    use super::*;
    use crate::mobs::world::{test_player, with_test_world};
    use crate::mobs::{MobHooks, PlayerView, TargetChange};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Cancel(AtomicUsize);
    impl MobHooks for Cancel {
        fn enderman_anger_suppressed(&self, _: u32, _: &PlayerView) -> bool {
            false
        }
        fn can_grief(&self, _: u32, rule: bool) -> bool {
            rule
        }
        fn grief_hooked(&self) -> bool {
            false
        }
        fn living_visibility(&self, _: &PlayerView, _: u32, v: f64) -> f64 {
            v
        }
        fn living_change_target(
            &self,
            _: &mut Mob,
            _: &MobWorld<'_>,
            brain: bool,
            target: Option<Who>,
        ) -> TargetChange {
            assert!(!brain);
            assert_eq!(target, Some(Who::Player));
            self.0.fetch_add(1, Ordering::Relaxed);
            TargetChange::Keep
        }
    }

    #[test]
    #[ignore = "daily"]
    fn nearest_target_goal_respects_cancellation() {
        assert!(
            with_test_world(&[], |world, types| {
                let hook = Cancel(AtomicUsize::new(0));
                let mut world = world.clone();
                world.player = Some(test_player(Vec3::new(1.5, 64.0, 0.5)));
                world.mob_hooks = Some(&hook);
                let kind = Arc::new(types.types["minecraft:zombie"].clone());
                let mut mob = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                let mut goal = NearestTarget::player(false);
                goal.found = Some(Who::Player);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut cx = Cx {
                    w: &world,
                    level: &mut level.level(),
                    events: &mut events,
                };
                goal.start(&mut mob, &mut cx);
                assert_eq!(mob.ai.target, None);
                assert_eq!(hook.0.load(Ordering::Relaxed), 1);
                mob.ai.target = Some(Who::Player);
                assert!(target_continue(
                    &mut goal.target_goal,
                    false,
                    &mut mob,
                    &mut cx
                ));
                assert_eq!(hook.0.load(Ordering::Relaxed), 2);
                assert_eq!(mob.ai.target, Some(Who::Player));
            }),
            "prepared entity resources are required"
        );
    }
}

#[cfg(test)]
mod breed_xp_tests {
    use super::*;

    /// Two llamas in love, two blocks apart on a flat grass shelf, ticked by
    /// the AI until the first breeds: its own events, with the `mob_drops`
    /// game rule at `drops`. `None` without the vanilla data.
    fn llama_events(drops: bool) -> Option<Vec<MobEvent>> {
        // The partner's `Seen` needs its kind, so two loads.
        let mut kind = None;
        crate::mobs::world::with_built_test_world_rules(&[], true, drops, shelf, |_, types| {
            kind = Some(Arc::new(types.types["minecraft:llama"].clone()));
        });
        let kind = kind?;
        let mut partner = Mob::new(2, kind.clone(), Vec3::new(2.5, 64.0, 0.5), 0.0);
        // AbstractHorse.canParent: only tame llamas breed.
        partner.extra.tame = true;
        partner.extra.in_love = 600;
        let seen = [partner.seen(None, &crate::items::Items::empty())];
        let mut out = None;
        crate::mobs::world::with_built_test_world_rules(&seen, true, drops, shelf, |w, _| {
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.extra.in_love = 600;
            m.extra.tame = true;
            let mut test_level = crate::health::TestLevel::new(0);
            let level = &mut test_level.level();
            let mut events = Vec::new();
            for _ in 0..600 {
                crate::mobs::ai::tick(&mut m, w, level, &mut events);
                if events
                    .iter()
                    .any(|e| matches!(e, MobEvent::Breed { parent: 1, .. }))
                {
                    break;
                }
            }
            out = Some(events);
        });
        out
    }

    /// A flat grass shelf two llamas can stand on.
    fn shelf(world: &mut crate::world::World, registry: &sunrise_world::registry::BlockRegistry) {
        let grass = registry.default_state("minecraft:grass_block").unwrap();
        for cx in -3..3 {
            for cz in -3..3 {
                world
                    .chunks
                    .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
            }
        }
        for x in -8..=8 {
            for z in -8..=8 {
                world.set(x, 63, z, grass).unwrap();
            }
        }
    }

    /// `finalizeSpawnChildFromBreeding` makes the experience draw only inside
    /// the `mob_drops` check, from the breeding parent's own random: the
    /// breed goal carries one 1-7 orb with the rule on, no orb with it off,
    /// and the pair breeds either way.
    #[test]
    #[ignore = "daily"]
    fn the_breed_xp_follows_the_mob_drops_rule() {
        let Some(on) = llama_events(true) else {
            return;
        };
        let Some(off) = llama_events(false) else {
            return;
        };
        let orb = |events: &[MobEvent]| {
            events.iter().find_map(|e| match e {
                MobEvent::Breed {
                    experience: crate::mobs::BreedExperience::Drawn(amount),
                    ..
                } => *amount,
                _ => None,
            })
        };
        assert!(
            orb(&on).is_some_and(|a| (1..=7).contains(&a)),
            "mob_drops on: one orb of 1-7, got {:?}",
            orb(&on)
        );
        assert!(orb(&off).is_none(), "mob_drops off: no experience draw");
        assert!(
            off.iter().any(|e| matches!(
                e,
                MobEvent::Breed {
                    experience: crate::mobs::BreedExperience::Drawn(None),
                    ..
                }
            )),
            "mob_drops off: the already-decided reward is empty, not deferred"
        );
        assert!(
            on.iter()
                .any(|e| matches!(e, MobEvent::Breed { parent: 1, .. })),
            "mob_drops on: the pair breeds"
        );
        assert!(
            off.iter()
                .any(|e| matches!(e, MobEvent::Breed { parent: 1, .. })),
            "mob_drops off: the pair still breeds"
        );
    }
}

#[cfg(test)]
#[path = "breed_partner_range_tests.rs"]
mod breed_partner_range_tests;

#[cfg(test)]
mod within_home_tests {
    use super::within_home;

    /// `Mob.isWithinHome`: `Vec3i.distSqr` in doubles against the radius
    /// squared as a Java `int`.
    #[test]
    fn within_home_matches_vanilla_at_the_edges() {
        assert!(within_home([10, 64, -5], 6, [12, 64, -2]));
        assert!(
            !within_home([10, 64, -5], 6, [16, 64, -5]),
            "the radius itself is out"
        );
        assert!(!within_home([i32::MIN, 0, 0], 6, [i32::MAX, 0, 0]));
        assert!(!within_home([i32::MIN; 3], i32::MAX, [i32::MAX; 3]));
        // 65 536² wraps to 0; 46 341² wraps negative.
        assert!(!within_home([0, 0, 0], 65_536, [0, 0, 0]));
        assert!(!within_home([0, 0, 0], 46_341, [0, 0, 0]));
        assert!(within_home([0, 0, 0], 46_340, [46_339, 0, 0]));
    }
}

#[cfg(test)]
mod melee_range_tests {
    use super::*;

    fn mob_holding(item: &str, components: Option<&str>) -> Mob {
        let kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        let mut m = Mob::new(1, Arc::new(kind), Vec3::new(0.5, 64.0, 0.5), 0.0);
        let mut stack = crate::items::ItemStack::new(item, 1);
        if let Some(text) = components {
            let tag = sunrise_core::nbt::parse_snbt(text).unwrap().0;
            (stack.damage, stack.extra) = crate::item_data::decode_components(Some(&tag));
        }
        m.extra.set_main_hand_stack(Some(stack));
        m
    }
    /// A one-block target whose near face is `gap` blocks east of the mob's box.
    fn target(m: &Mob, gap: f64) -> Aabb {
        let x = m.bounds().max_position().x + gap;
        Aabb::new(Vec3::new(x, 64.0, 0.0), Vec3::new(x + 1.0, 65.0, 1.0))
    }

    /// `Mob.isWithinMeleeAttackRange` with `getActiveItem().get(ATTACK_RANGE)`:
    /// no component reaches sqrt(2.04) - 0.6 (about 0.83); a spear 4.5 and 2.0
    /// times its mob factor 0.5; a patched component with the codec's
    /// defaults (`mob_factor` 1, `min_reach` 0); a removed one the default.
    #[test]
    fn the_active_items_attack_range_sets_the_reach() {
        let sword = mob_holding("minecraft:iron_sword", None);
        assert!(sword.melee_reaches_beside(None, target(&sword, 0.8)));
        assert!(!sword.melee_reaches_beside(None, target(&sword, 0.9)));

        let spear = mob_holding("minecraft:iron_spear", None);
        assert!(spear.melee_reaches_beside(None, target(&spear, 2.2)));
        assert!(!spear.melee_reaches_beside(None, target(&spear, 2.3)));
        assert!(
            !spear.melee_reaches_beside(None, target(&spear, 0.5)),
            "inside the minimum range"
        );
        assert!(spear.melee_reaches_beside(None, target(&spear, 1.1)));

        let long = mob_holding(
            "minecraft:iron_sword",
            Some(r#"{"minecraft:attack_range":{max_reach:6.0f}}"#),
        );
        assert!(long.melee_reaches_beside(None, target(&long, 5.9)));
        assert!(
            long.melee_reaches_beside(None, target(&long, 0.0)),
            "no minimum by default"
        );
        assert!(!long.melee_reaches_beside(None, target(&long, 6.1)));

        let blunt = mob_holding(
            "minecraft:iron_spear",
            Some(r#"{"!minecraft:attack_range":{}}"#),
        );
        assert!(blunt.melee_reaches_beside(None, target(&blunt, 0.5)));
        assert!(!blunt.melee_reaches_beside(None, target(&blunt, 2.0)));
    }

    /// `Mob.getAttackBoundingBox`: a rider's box widened on x and z to its
    /// vehicle's (a 1.4 wide horse under a 0.6 wide zombie) before the reach
    /// is added; `LivingEntity.getHitbox`: a rider is hit from its seat up.
    #[test]
    fn a_riders_attack_box_takes_its_vehicles_width() {
        let mut rider = mob_holding("minecraft:iron_sword", None);
        let at = rider.position;
        let horse = Aabb::new(
            Vec3::new(at.x - 0.7, at.y - 1.0, at.z - 0.7),
            Vec3::new(at.x + 0.7, at.y + 0.6, at.z + 0.7),
        );
        // Its near face 0.9 past the zombie's box: past the default reach of
        // the zombie alone (about 0.83), within it from the horse's side.
        let target = target(&rider, 0.9);
        assert!(!rider.melee_reaches_beside(None, target));
        rider.extra.mount = Some(9);
        rider.seated(9, 0.7, horse);
        assert!(rider.melee_reaches_beside(Some(horse), target));
        // Its own box keeps its height: the horse's lower box adds nothing.
        let below = Aabb::new(
            Vec3::new(at.x + 0.9, at.y - 1.0, at.z),
            Vec3::new(at.x + 1.1, at.y - 0.5, at.z + 1.0),
        );
        assert!(!rider.melee_reaches_beside(Some(horse), below));

        assert_eq!(rider.hitbox().min_position().y, at.y + 0.7);
        // A seat left from another vehicle is not this one's.
        rider.extra.mount = Some(10);
        assert_eq!(rider.hitbox().min_position().y, at.y);
        rider.extra.mount = None;
        assert_eq!(rider.hitbox().min_position().y, at.y);
    }
}

#[cfg(test)]
mod melee_continue_tests {
    use super::*;
    use crate::mobs::world::{test_player, with_test_world};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    /// Tick one `<MeleeAttack>` goal through the real selector.
    fn tick(goals: &mut GoalSelector, m: &mut Mob, world: &MobWorld) {
        let mut fixture = crate::health::TestLevel::new(0);
        let mut events = Vec::new();
        let mut cx = Cx {
            w: world,
            level: &mut fixture.level(),
            events: &mut events,
        };
        goals.tick(m, &mut cx);
    }

    /// `MeleeAttackGoal.canContinueToUse`'s follow branch refuses a target
    /// outside the mob's home (`isWithinHome(target.blockPosition())`).
    #[test]
    #[ignore = "daily"]
    fn a_chase_stops_outside_the_mobs_home() {
        let mut outcomes = None;
        let ran = with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:iron_golem"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.ai.home = Some(([0, 64, 0], 8));
            m.ai.target = Some(Who::Player);
            let mut world = w.clone();
            world.player = Some(test_player(Vec3::new(0.5, 64.0, 0.5)));
            let mut goals = GoalSelector::default();
            goals.add(1, MeleeAttack::new(1.0, true));
            tick(&mut goals, &mut m, &world);
            let started = goals.is_running("MeleeAttack");
            world.player.as_mut().unwrap().position = Vec3::new(100.5, 64.0, 0.5);
            tick(&mut goals, &mut m, &world);
            outcomes = Some((started, goals.is_running("MeleeAttack")));
        });
        if !ran {
            return;
        }
        assert_eq!(
            outcomes,
            Some((true, false)),
            "the chase starts at home and stops when the target leaves its radius"
        );
    }

    /// The same branch refuses a spectator (`!(player.isSpectator() || player.isCreative())`).
    #[test]
    #[ignore = "daily"]
    fn a_chase_stops_when_the_target_is_a_spectator() {
        let mut outcomes = None;
        let ran = with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:iron_golem"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.ai.target = Some(Who::Player);
            let mut world = w.clone();
            world.player = Some(test_player(Vec3::new(0.5, 64.0, 0.5)));
            let mut goals = GoalSelector::default();
            goals.add(1, MeleeAttack::new(1.0, true));
            tick(&mut goals, &mut m, &world);
            let started = goals.is_running("MeleeAttack");
            world.player.as_mut().unwrap().spectator = true;
            tick(&mut goals, &mut m, &world);
            outcomes = Some((started, goals.is_running("MeleeAttack")));
        });
        if !ran {
            return;
        }
        assert_eq!(
            outcomes,
            Some((true, false)),
            "a mob chases a survival player and stops when they become a spectator"
        );
    }
}

#[cfg(test)]
mod check_and_perform_attack_tests {
    use super::*;
    use crate::mobs::world::{test_player, with_test_world};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    /// What `call` leaves: the goal's cooldown, whether the mob swings, and its hits on the
    /// player.
    fn strike(
        goal: &mut MeleeAttack,
        m: &mut Mob,
        world: &MobWorld,
        call: impl FnOnce(&mut MeleeAttack, &mut Mob, &mut Cx),
    ) -> (i32, bool, usize) {
        let mut fixture = crate::health::TestLevel::new(0);
        let mut events = Vec::new();
        let mut cx = Cx {
            w: world,
            level: &mut fixture.level(),
            events: &mut events,
        };
        call(goal, m, &mut cx);
        let hits = events
            .iter()
            .filter(|e| matches!(e, MobEvent::AttackPlayer(..)))
            .count();
        (goal.until_attack, m.ai.swinging, hits)
    }

    /// `MeleeAttackGoal.checkAndPerformAttack`: while the cooldown runs, nothing; once it is
    /// over, with the target in reach and in sight, `resetAttackCooldown` (20 ticks), the swing
    /// and one hit. `tick` counts the cooldown down before it calls the method, as vanilla's
    /// does, so a zombie one tick from its next attack strikes in that tick.
    #[test]
    #[ignore = "daily"]
    fn the_strike_comes_after_the_cooldown_countdown_in_tick() {
        let mut outcomes = Vec::new();
        let ran = with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:zombie"].clone());
            let mut world = w.clone();
            world.player = Some(test_player(Vec3::new(1.5, 64.0, 0.5)));
            let t = world.target_info(Who::Player).unwrap();
            for (until_attack, through_tick) in [(1, false), (0, false), (1, true)] {
                let mut m = Mob::new(1, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
                m.ai.target = Some(Who::Player);
                let mut goal = MeleeAttack::new(1.0, false);
                goal.until_attack = until_attack;
                outcomes.push(strike(&mut goal, &mut m, &world, |goal, m, cx| {
                    if through_tick {
                        goal.tick(m, cx);
                    } else {
                        goal.check_and_perform_attack(m, Who::Player, &t, cx);
                    }
                }));
            }
        });
        assert!(ran, "prepared mob resources required");
        assert_eq!(outcomes, [(1, false, 0), (20, true, 1), (20, true, 1)]);
    }

    /// `FoxMeleeAttackGoal.checkAndPerformAttack` replaces the base method without calling
    /// it: a fox's `tick` strikes and bites through its own, and the base method, which a
    /// mod's row on `MeleeAttackGoal.checkAndPerformAttack` takes, has no bite.
    #[test]
    #[ignore = "daily"]
    fn a_foxs_bite_is_not_in_the_base_method() {
        let mut bites = Vec::new();
        let ran = with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:fox"].clone());
            let mut world = w.clone();
            world.player = Some(test_player(Vec3::new(1.5, 64.0, 0.5)));
            let t = world.target_info(Who::Player).unwrap();
            for through_tick in [true, false] {
                let mut m = Mob::new(1, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
                m.ai.target = Some(Who::Player);
                let mut goal = MeleeAttack::new(1.2, true).bite("entity.fox.bite");
                let mut fixture = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut cx = Cx {
                    w: &world,
                    level: &mut fixture.level(),
                    events: &mut events,
                };
                if through_tick {
                    goal.tick(&mut m, &mut cx);
                } else {
                    goal.check_and_perform_attack(&mut m, Who::Player, &t, &mut cx);
                }
                let bitten = events
                    .iter()
                    .filter(|e| matches!(e, MobEvent::Sound("entity.fox.bite", ..)))
                    .count();
                bites.push(bitten);
            }
        });
        assert!(ran, "vanilla data is missing");
        assert_eq!(bites, [1, 0]);
    }

    /// Whether `test` of this module runs here: in the test process, it reruns `test` alone in
    /// a fresh child process, asserts the child passed and returns false; in that child, true.
    /// The prepared-resource world writes process-wide tables (the block palette, the item
    /// tables), which no other test of the shared process may see.
    fn in_fresh_process(test: &str) -> bool {
        const CHILD: &str = "SUNRISE_TEST_MELEE_ATTACK_CHILD";
        let module = module_path!().split_once("::").unwrap().1;
        let name = format!("{module}::{test}");
        if std::env::var_os(CHILD).is_some_and(|value| value == name.as_str()) {
            return true;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([&name, "--exact", "--include-ignored", "--test-threads=1"])
            .env(CHILD, &name)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("test result: ok. 1 passed;"),
            "{name} failed in its own process:\n{stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        false
    }

    /// `FoxMeleeAttackGoal.checkAndPerformAttack` has no `swingForAttack`: a fox's strike
    /// resets the cooldown, hurts its target and bites, but doesn't swing (#4733). The world
    /// is built in a fresh child process.
    #[test]
    #[ignore = "daily"]
    fn a_foxs_strike_hurts_and_bites_without_a_swing() {
        if !in_fresh_process("a_foxs_strike_hurts_and_bites_without_a_swing") {
            return;
        }
        let mut outcome = None;
        let ran = with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:fox"].clone());
            let mut world = w.clone();
            world.player = Some(test_player(Vec3::new(1.5, 64.0, 0.5)));
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.ai.target = Some(Who::Player);
            let mut goal = MeleeAttack::new(1.2, true).bite("entity.fox.bite");
            let mut fixture = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            let mut cx = Cx {
                w: &world,
                level: &mut fixture.level(),
                events: &mut events,
            };
            goal.tick(&mut m, &mut cx);
            let hits = events
                .iter()
                .filter(|e| matches!(e, MobEvent::AttackPlayer(..)))
                .count();
            let bites = events
                .iter()
                .filter(|e| matches!(e, MobEvent::Sound("entity.fox.bite", ..)))
                .count();
            outcome = Some((goal.until_attack, hits, bites, m.ai.swinging));
        });
        assert!(ran, "prepared mob resources required");
        assert_eq!(outcome, Some((20, 1, 1, false)));
    }
}

#[cfg(test)]
mod breath_air_tests {
    use super::BreathAir;

    #[test]
    fn searches_neighbor_columns_in_vanilla_order() {
        let origin = [10, 20, 30];
        let air = [[10, 27, 29], [11, 21, 30], [10, 22, 31], [9, 23, 30]];
        assert_eq!(
            BreathAir::air_position(origin, |p| air.contains(&p)),
            Some([10, 27, 29])
        );
        assert_eq!(
            BreathAir::air_position(origin, |p| p == [10, 28, 30] || air.contains(&p)),
            Some([10, 28, 30])
        );
        for p in air {
            assert_eq!(BreathAir::air_position(origin, |q| q == p), Some(p));
        }
        // One block above the 9-block columns: not searched.
        assert_eq!(BreathAir::air_position(origin, |p| p == [10, 29, 30]), None);
    }
}

#[cfg(test)]
mod hurt_by_anger_tests {
    use super::*;
    use crate::mobs::world::{test_player, with_test_world};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    /// `HurtByTargetGoal.canUse` refuses a player when the `universal_anger`
    /// game rule is on: the anger target goals handle that hit instead.
    #[test]
    #[ignore = "daily"]
    fn universal_anger_leaves_the_players_hit_to_the_anger_goals() {
        let mut outcomes = None;
        if !with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:cow"].clone());
            let mut truth = Vec::new();
            for rule in [false, true] {
                let mut world = w.clone();
                world.player = Some(test_player(Vec3::new(2.5, 64.0, 0.5)));
                world.universal_anger = rule;
                let mut m = Mob::new(1, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
                m.ai.last_hurt_by = Some(Who::Player);
                m.ai.last_hurt_by_time = 1;
                m.ai.goals.add(1, HurtByTarget::new());
                let mut lr = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut cx = Cx {
                    w: &world,
                    level: &mut lr.level(),
                    events: &mut events,
                };
                let mut goals = std::mem::take(&mut m.ai.goals);
                goals.tick(&mut m, &mut cx);
                truth.push(goals.is_running("HurtByTarget"));
                m.ai.goals = goals;
            }
            outcomes = Some(truth);
        }) {
            return;
        }
        assert_eq!(
            outcomes,
            Some(vec![true, false]),
            "a hit is retaliated with the rule off and left to the anger goals with it on"
        );
    }
}

impl Mob {
    /// `Mob.isWithinHome(BlockPos.containing(at))`; true with no home.
    pub(super) fn within_home_at(&self, at: sunrise_core::geometry::Vec3) -> bool {
        let block = [
            at.x.floor() as i32,
            at.y.floor() as i32,
            at.z.floor() as i32,
        ];
        self.ai
            .home
            .is_none_or(|(center, radius)| within_home(center, radius, block))
    }
}

#[cfg(test)]
mod breed_continue_tests {
    use super::*;
    use crate::mobs::world::with_test_world;
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    /// `BreedGoal.canContinueToUse` refuses a partner that is panicking. The
    /// goal starts with a calm partner and must stop when the partner panics.
    #[test]
    #[ignore = "daily"]
    fn a_breed_goal_stops_when_the_partner_panics() {
        let mut kind = None;
        if !with_test_world(&[], |_, types| {
            kind = Some(Arc::new(types.types["minecraft:cow"].clone()));
        }) {
            return;
        }
        let Some(kind) = kind else { return };
        let mut partner = Mob::new(2, kind.clone(), Vec3::new(2.5, 64.0, 0.5), 0.0);
        partner.extra.in_love = 600;
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        m.extra.in_love = 600;
        m.ai.goals.add(1, Breed::new(1.0));
        let mut started = None;
        let calm = [partner.seen(None, &crate::items::Items::empty())];
        if !with_test_world(&calm, |w, _| {
            let mut damage = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            let mut cx = Cx {
                w,
                level: &mut damage.level(),
                events: &mut events,
            };
            let mut goals = std::mem::take(&mut m.ai.goals);
            goals.tick(&mut m, &mut cx);
            started = Some(goals.is_running("Breed"));
            m.ai.goals = goals;
        }) {
            return;
        }
        partner.ai.panicking = true;
        let panicking = [partner.seen(None, &crate::items::Items::empty())];
        let mut stopped = None;
        if !with_test_world(&panicking, |w, _| {
            let mut damage = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            let mut cx = Cx {
                w,
                level: &mut damage.level(),
                events: &mut events,
            };
            let mut goals = std::mem::take(&mut m.ai.goals);
            goals.tick(&mut m, &mut cx);
            stopped = Some(!goals.is_running("Breed"));
            m.ai.goals = goals;
        }) {
            return;
        }
        assert_eq!(
            (started, stopped),
            (Some(true), Some(true)),
            "the breed goal starts for a calm partner and stops when it panics"
        );
    }
}
