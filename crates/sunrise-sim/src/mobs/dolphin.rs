//! Dolphin goals: `DolphinSwimToTreasureGoal`, `DolphinSwimWithPlayerGoal`
//! and `DolphinJumpGoal`. `extra.flag` is `gotFish` and `extra.home` the
//! `treasurePos` the game found for it (`MobEvent::DolphinTreasure`). `extra.point` is where the
//! game saw the first item floating within 8 blocks (`Dolphin.ALLOWED_ITEMS`).
use super::{
    Mob, MobEvent, Who,
    goals::{Cx, Goal, JUMP, LOOK, MOVE, goal_clone, reduced},
};
use crate::blocks::Fluid;
use std::ops::ControlFlow;
use sunrise_core::geometry::trig::Mth;
use sunrise_core::random::RandomSource;

/// `Entity.getMotionDirection`: the horizontal step of the yaw
/// (`Direction.fromYRot`: south, west, north, east).
fn motion_step(yaw: f32) -> [i32; 2] {
    match (yaw / 90.0 + 0.5).floor() as i32 & 3 {
        0 => [0, 1],
        1 => [-1, 0],
        2 => [0, -1],
        _ => [1, 0],
    }
}

/// `DolphinSwimToTreasureGoal`: a dolphin that was fed a fish leads the way to
/// a shipwreck or an ocean ruin within 50 sectors.
#[derive(Clone, Debug, Default)]
pub struct SwimToTreasure {
    stuck: bool,
}
impl SwimToTreasure {
    /// `BlockPos.containing(x, dolphin.getY(), z).closerToCenterThan(position, 4)`.
    fn arrived(m: &Mob) -> bool {
        let Some(t) = m.extra.home else { return false };
        let d = [
            f64::from(t[0]) + 0.5 - m.position.x,
            m.position.y.floor() + 0.5 - m.position.y,
            f64::from(t[2]) + 0.5 - m.position.z,
        ];
        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 16.0
    }
}
impl Goal for SwimToTreasure {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    #[inline]
    fn interruptable(&self) -> bool {
        false
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.flag && m.air >= 100
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.home.is_some() && !Self::arrived(m) && !self.stuck && m.air >= 100
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.stuck = false;
        m.nav_stop();
        // The game looks for the structure and sets the position, or none.
        m.extra.home = None;
        cx.events.push(MobEvent::DolphinTreasure { mob: m.id });
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        if m.extra.home.is_none() || Self::arrived(m) || self.stuck {
            m.extra.flag = false;
        }
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(t) = m.extra.home else { return };
        let close = m.ai.nav.target_pos.is_some_and(|p| {
            let d = [
                f64::from(p[0]) + 0.5 - m.position.x,
                f64::from(p[1]) + 0.5 - m.position.y,
                f64::from(p[2]) + 0.5 - m.position.z,
            ];
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 144.0
        });
        if !close && !m.ai.nav.is_done() {
            return;
        }
        let treasure = [
            f64::from(t[0]) + 0.5,
            f64::from(t[1]) + 0.5,
            f64::from(t[2]) + 0.5,
        ];
        let eighth = f64::from(std::f32::consts::FRAC_PI_8);
        let half = f64::from(std::f32::consts::FRAC_PI_2);
        let mut next = m.default_random_pos_towards(cx.w, 16, 1, treasure, eighth);
        if next.is_none() {
            next = m.default_random_pos_towards(cx.w, 8, 4, treasure, half);
        }
        // isPathfindable(WATER) is a water fluid state.
        if let Some(n) = next
            && cx.w.fluid(
                n[0].floor() as i32,
                n[1].floor() as i32,
                n[2].floor() as i32,
            ) != Fluid::Water
        {
            next = m.default_random_pos_towards(cx.w, 8, 5, treasure, half);
        }
        let Some(next) = next else {
            self.stuck = true;
            return;
        };
        let (y, x) = (m.kind.max_head_y_rot + 20.0, m.kind.max_head_x_rot);
        m.look_at_speed(next, y, x);
        m.nav_move_to(cx.w, next, 1.3);
        // `adjustedTickDelay(80)`: this goal does not require every tick, so
        // 40, and the draw is the level's random, not the dolphin's.
        if cx.level.random().next_int_bound(reduced(80)).unwrap_or(1) == 0 {
            // Entity event 38: happy villager particles about the dolphin.
            cx.events.push(MobEvent::AroundSelf {
                mob: m.id,
                event: 38,
                kind: "happy_villager",
                feet: [m.position.x, m.position.y, m.position.z],
                width: f64::from(m.width()),
                height: f64::from(m.height()),
                burst: sunrise_level::particle_options::Burst::DOLPHIN,
            });
        }
    }
}

/// `DolphinSwimWithPlayerGoal`: swim up to a swimming player and give them
/// Dolphin's Grace.
#[derive(Clone, Debug)]
pub struct SwimWithPlayer {
    speed: f64,
}
impl SwimWithPlayer {
    pub fn new(speed: f64) -> Self {
        Self { speed }
    }
}
impl Goal for SwimWithPlayer {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // getNearestPlayer(forNonCombat().range(10).ignoreLineOfSight()).
        m.can_see_player_through_walls(cx.w, 10.0)
            && cx.w.selected_player().is_some_and(|p| p.swimming)
            && m.ai.target != Some(Who::Player)
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        cx.w.selected_player()
            .is_some_and(|p| p.swimming && m.distance_sqr(p.position) < 256.0)
    }
    #[inline]
    fn start(&mut self, _: &mut Mob, cx: &mut Cx) {
        cx.events.push(cx.w.route_player_event(grace()));
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = cx.w.selected_player().cloned() else {
            return;
        };
        let (y, x) = (m.kind.max_head_y_rot + 20.0, m.kind.max_head_x_rot);
        m.look_at_speed(p.eye, y, x);
        if m.distance_sqr(p.position) < 6.25 {
            m.nav_stop();
        } else {
            m.nav_move_to_entity(
                cx.w,
                [p.position.x, p.position.y, p.position.z],
                1,
                self.speed,
            );
        }
        if p.swimming && m.random.next_int_bound(6).unwrap_or(1) == 0 {
            cx.events.push(cx.w.route_player_event(grace()));
        }
    }
}
/// `MobEffectInstance(DOLPHINS_GRACE, 100)`.
fn grace() -> MobEvent {
    MobEvent::EffectPlayer {
        effect: "minecraft:dolphins_grace",
        ticks: 100,
        amplifier: 0,
        ambient: false,
    }
}

/// `DolphinJumpGoal`: leap out of clear water and arc back in.
#[derive(Clone, Debug)]
pub struct Jump {
    interval: i32,
    breached: bool,
}
impl Jump {
    pub fn new(interval: i32) -> Self {
        Self {
            interval: reduced(interval),
            breached: false,
        }
    }
}
impl Goal for Jump {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    #[inline]
    fn interruptable(&self) -> bool {
        false
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.random.next_int_bound(self.interval).unwrap_or(1) != 0 {
            return false;
        }
        let [sx, sz] = motion_step(m.yaw);
        let (bx, by, bz) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        let w = cx.w;
        [0, 1, 4, 5, 6, 7].into_iter().all(|i| {
            let (x, z) = (bx + sx * i, bz + sz * i);
            // waterIsClear, then surfaceIsClear.
            w.fluid(x, by, z) == Fluid::Water
                && !w.has_tag("minecraft:blocks_dolphin_jump", w.block_name(x, by, z))
                && w.facts.is_air(w.world.block(x, by + 1, z))
                && w.facts.is_air(w.world.block(x, by + 2, z))
        })
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        let yd = m.velocity.y;
        (yd * yd >= f64::from(0.03f32)
            || m.head_pitch == 0.0
            || m.head_pitch.abs() >= 10.0
            || !m.fluids.in_water)
            && !m.ground.on_ground
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        let [sx, sz] = motion_step(m.yaw);
        m.velocity.x += f64::from(sx) * 0.6;
        m.velocity.y += 0.7;
        m.velocity.z += f64::from(sz) * 0.6;
        m.nav_stop();
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.head_pitch = 0.0;
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let already = self.breached;
        if !already {
            self.breached = cx.w.fluid(
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ) == Fluid::Water;
        }
        if self.breached && !already {
            m.sound("entity.dolphin.jump", cx.events);
        }
        let v = m.velocity;
        if v.y * v.y < f64::from(0.03f32) && m.head_pitch != 0.0 {
            // Mth.rotLerp(0.2, xRot, 0).
            m.head_pitch += 0.2 * super::living::wrap(0.0 - m.head_pitch);
        } else if v.length() > f64::from(1.0e-5f32) {
            m.head_pitch = breach_pitch(v);
        }
    }
}

/// `DolphinJumpGoal.tick`'s pitch: `Math.atan2(-y, horizontalDistance)` times `Mth.RAD_TO_DEG`.
fn breach_pitch(v: sunrise_core::geometry::Vec3) -> f32 {
    let horizontal = sunrise_core::geometry::length(v.x, v.z);
    (sunrise_core::jmath::atan2(-v.y, horizontal)
        * f64::from(sunrise_core::geometry::trig::RAD_TO_DEG)) as f32
}

/// `FollowPlayerRiddenEntityGoal(mob, AbstractBoat / AbstractNautilus)`: a
/// dolphin swims to the back of a boat or nautilus a player steers within 5
/// blocks of it, then ahead of it in the direction it moves, and goes back
/// when it is more than 12 blocks off.
///
/// Only the player the mob sees (the nearest) is considered.
#[derive(Clone, Debug)]
pub struct FollowPlayerRidden {
    nautilus: bool,
    following: bool,
    recalc: i32,
    leading: bool,
    // One rider sample belongs to the whole move, even if its landing yields again.
    pending_rider: Option<(sunrise_core::geometry::Vec3, f32)>,
}
impl FollowPlayerRidden {
    pub fn new(nautilus: bool) -> Self {
        Self {
            nautilus,
            following: false,
            recalc: 0,
            leading: false,
            pending_rider: None,
        }
    }
    /// `Entity.hasMovedHorizontallyRecently` of the rider, and the player's
    /// own position (its yaw gives the directions).
    fn rider<'a>(&self, cx: &'a Cx) -> Option<&'a super::PlayerView> {
        cx.w.selected_player().filter(|p| {
            p.vehicle
                .as_ref()
                .is_some_and(|v| v.nautilus == self.nautilus)
        })
    }
    fn moved(p: &super::PlayerView) -> bool {
        p.vehicle.as_ref().is_some_and(|v| {
            (v.velocity.x * v.velocity.x + v.velocity.z * v.velocity.z).sqrt() > 1.0e-5
        })
    }
}
impl Goal for FollowPlayerRidden {
    goal_clone!();
    #[inline]
    fn name(&self) -> &'static str {
        "FollowPlayerRiddenEntityGoal"
    }
    #[inline]
    fn flags(&self) -> u8 {
        0
    }
    /// `FollowPlayerRiddenEntityGoal.canUse`.
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(p) = self.rider(cx) else {
            return false;
        };
        if self.following && Self::moved(p) {
            return true;
        }
        let area = m
            .bounds()
            .inflate(sunrise_core::geometry::Vec3::new(5.0, 5.0, 5.0));
        p.vehicle
            .as_ref()
            .is_some_and(|v| v.bounds.intersects(area))
            && Self::moved(p)
    }
    /// `FollowPlayerRiddenEntityGoal.canContinueToUse`.
    #[inline]
    fn can_continue(&mut self, _: &mut Mob, cx: &mut Cx) -> bool {
        self.following
            && self
                .rider(cx)
                .is_some_and(|p| p.passenger && Self::moved(p))
    }
    /// `FollowPlayerRiddenEntityGoal.start`.
    #[inline]
    fn start(&mut self, _: &mut Mob, cx: &mut Cx) {
        self.following = self.rider(cx).is_some();
        self.recalc = 0;
        self.leading = false;
    }
    /// `FollowPlayerRiddenEntityGoal.stop`.
    #[inline]
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.following = false;
        self.pending_rider = None;
    }
    /// `FollowPlayerRiddenEntityGoal.tick`.
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.move_to_rider(m, cx, false);
    }

    #[inline]
    fn tick_until_movement_pause(
        &mut self,
        m: &mut Mob,
        cx: &mut Cx,
    ) -> Option<super::living::TravelPause> {
        self.move_to_rider(m, cx, true)
    }

    #[inline]
    fn after_movement(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if let Some((position, yaw)) = self.pending_rider.take() {
            self.repath(m, cx, position, yaw);
            true
        } else {
            false
        }
    }
}

impl FollowPlayerRidden {
    fn move_to_rider(
        &mut self,
        m: &mut Mob,
        cx: &mut Cx,
        pause: bool,
    ) -> Option<super::living::TravelPause> {
        let p = self.rider(cx)?;
        let (position, yaw) = (p.position, p.yaw);
        let speed = if self.leading { 0.01 } else { 0.015 };
        let input = sunrise_core::geometry::Vec3::new(
            f64::from(m.ai.xxa),
            f64::from(m.ai.yya),
            f64::from(m.ai.zza),
        );
        m.move_relative(speed, input);
        if let ControlFlow::Break(movement) = m.travel_move(
            cx.w,
            cx.level,
            cx.events,
            pause,
            sunrise_world::travel::TravelMode::Water,
            |_, _, _, _| {},
        ) {
            self.pending_rider = Some((position, yaw));
            return Some(movement);
        }
        self.repath(m, cx, position, yaw);
        None
    }

    fn repath(
        &mut self,
        m: &mut Mob,
        cx: &mut Cx,
        position: sunrise_core::geometry::Vec3,
        yaw: f32,
    ) {
        self.recalc -= 1;
        if self.recalc > 0 {
            return;
        }
        self.recalc = reduced(10);
        let [dx, dz] = motion_step(yaw);
        let block = [
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ];
        if !self.leading {
            // The block behind the player, one lower.
            let behind = [block[0] - dx, block[1] - 1, block[2] - dz];
            m.nav_move_to(
                cx.w,
                [
                    f64::from(behind[0]),
                    f64::from(behind[1]),
                    f64::from(behind[2]),
                ],
                1.0,
            );
            if m.distance_sqr(position) < 16.0 {
                self.recalc = 0;
                self.leading = true;
            }
        } else {
            let ahead = [block[0] + dx * 10, block[1] - 1, block[2] + dz * 10];
            m.nav_move_to(
                cx.w,
                [
                    f64::from(ahead[0]),
                    f64::from(ahead[1]),
                    f64::from(ahead[2]),
                ],
                1.0,
            );
            if m.distance_sqr(position) > 144.0 {
                self.recalc = 0;
                self.leading = false;
            }
        }
    }
}

/// Where the first item floating in the water within 8 blocks lies (`Dolphin.ALLOWED_ITEMS`:
/// no pickup delay, alive, in water), found by the game each tick.
pub fn set_item_at(m: &mut Mob, at: Option<[f64; 3]>) {
    m.extra.point = at;
}
/// `Dolphin.ItemGoal.drop` of the item in the mouth (`dropItem`): thrown from the eye, in
/// the direction it faces, with a pickup delay of 40. False when the mouth is empty.
fn drop_item(m: &mut Mob, events: &mut Vec<MobEvent>) -> bool {
    let Some(stack) = super::finalize::item_stack(m, super::equip::Slot::MainHand.index()) else {
        return false;
    };
    let y_hand = m.eye_y() - f64::from(0.3_f32);
    let pow = 0.3_f32;
    let dir = m.random.next_float() * (std::f32::consts::PI * 2.0);
    let pow2 = 0.02_f32 * m.random.next_float();
    let to_rad = std::f32::consts::PI / 180.0;
    let (yaw, pitch) = (m.yaw * to_rad, m.head_pitch * to_rad);
    let motion = [
        f64::from(pow * -yaw.msin() * pitch.mcos() + dir.mcos() * pow2),
        f64::from(pow * pitch.msin() * 1.5),
        f64::from(pow * yaw.mcos() * pitch.mcos() + dir.msin() * pow2),
    ];
    events.push(MobEvent::ThrowItem {
        mob: m.id,
        stack,
        at: [m.position.x, y_hand, m.position.z],
        motion,
        pickup_delay: 40,
    });
    m.extra.set_main_hand_stack(None);
    true
}
/// `Dolphin.MoveToItemGoal`: swim to a floating item, playing a sound; drops what it holds
/// when it stops and rests for up to 100 ticks.
#[derive(Clone, Debug, Default)]
pub struct MoveToItem {
    cooldown: i64,
}
impl Goal for MoveToItem {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.cooldown > cx.w.game_time {
            return false;
        }
        m.extra.point.is_some()
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        if drop_item(m, cx.events) {
            self.cooldown = cx.w.game_time + i64::from(m.random.next_int_bound(100).unwrap_or(0));
        }
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(at) = m.extra.point {
            m.nav_move_to_entity(cx.w, at, 0, f64::from(1.2_f32));
            // `Entity.playSound(DOLPHIN_PLAY, 1.0, 1.0)` is silent-aware.
            m.sound("entity.dolphin.play", cx.events);
        }
        self.cooldown = 0;
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(at) = m.extra.point {
            m.nav_move_to_entity(cx.w, at, 0, f64::from(1.2_f32));
        }
    }
}
/// `Dolphin.PlayWithItemsGoal`: a dolphin that holds something throws it away.
#[derive(Clone, Debug, Default)]
pub struct PlayWithItems;
impl Goal for PlayWithItems {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        0
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.extra.main_hand_id().is_some()
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        drop_item(m, cx.events);
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        drop_item(m, cx.events);
    }
}

#[cfg(test)]
mod item_tests {
    use super::*;
    use crate::items::ItemStack;
    use crate::mobs::{Species, sensor_scan_tests::kind};
    use sunrise_core::geometry::Vec3;

    fn dolphin() -> Mob {
        let mut kind = kind();
        let k = std::sync::Arc::make_mut(&mut kind);
        k.id = "minecraft:dolphin".into();
        k.species = Species::Dolphin;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// `Dolphin.ItemGoal.dropItem`: a dolphin facing south (yaw 0, level) throws the item
    /// forward (+Z) at 0.3, off by at most 0.02 sideways, with a pickup delay of 40, and
    /// its mouth is empty after. An empty mouth throws nothing.
    #[test]
    fn a_dolphin_throws_its_item_ahead_and_empties_its_mouth() {
        let mut m = dolphin();
        let mut events = Vec::new();
        assert!(!drop_item(&mut m, &mut events), "nothing to throw");
        assert!(events.is_empty());
        let stack = ItemStack {
            item: "minecraft:cod".into(),
            count: 3,
            damage: 0,
            extra: None,
        };
        crate::mobs::finalize::set_item_stack(
            &mut m,
            crate::mobs::equip::Slot::MainHand.index(),
            stack,
        );
        assert!(drop_item(&mut m, &mut events));
        let [
            MobEvent::ThrowItem {
                stack,
                motion,
                pickup_delay,
                ..
            },
        ] = events.as_slice()
        else {
            panic!("one thrown item, got {events:?}");
        };
        assert_eq!(
            (&*stack.item, stack.count, *pickup_delay),
            ("minecraft:cod", 3, 40)
        );
        assert!(
            motion[0].abs() <= 0.02 + 1e-6,
            "sideways at most 0.02: {motion:?}"
        );
        assert!(
            (motion[2] - 0.3).abs() <= 0.02 + 1e-6,
            "ahead at 0.3: {motion:?}"
        );
        assert_eq!(motion[1], 0.0, "level, so no climb");
        assert!(m.extra.main_hand_id().is_none(), "the mouth is empty");
    }
}

#[cfg(test)]
mod target_tests {
    use super::*;
    use crate::mobs::world::with_test_world;
    use crate::mobs::{Species, sensor_scan_tests::kind};
    use sunrise_core::geometry::Vec3;

    fn dolphin(baby: bool) -> Mob {
        let mut kind = kind();
        let k = std::sync::Arc::make_mut(&mut kind);
        k.id = "minecraft:dolphin".into();
        k.species = Species::Dolphin;
        let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
        m.extra.age = if baby { -24000 } else { 0 };
        m
    }

    /// `Dolphin.canAttack`: a baby takes nobody as its target, so a hit on it
    /// is not answered; an adult does take the one that hurt it.
    #[test]
    #[ignore = "daily"]
    fn a_baby_dolphin_spares_whatever_hurt_it() {
        let ran = with_test_world(&[], |w, _| {
            assert!(
                dolphin(true).spares_target(w, Who::Mob(7)),
                "a baby dolphin fights nobody"
            );
            assert!(
                !dolphin(false).spares_target(w, Who::Mob(7)),
                "an adult dolphin takes its attacker"
            );
        });
        assert!(ran, "vanilla data is missing");
    }
}

#[cfg(test)]
mod treasure_play_tests {
    use super::*;
    use crate::mobs::world::with_test_world;
    use crate::mobs::{Species, sensor_scan_tests::kind};
    use sunrise_core::geometry::Vec3;

    fn dolphin() -> Mob {
        let mut kind = kind();
        let k = std::sync::Arc::make_mut(&mut kind);
        k.id = "minecraft:dolphin".into();
        k.species = Species::Dolphin;
        Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0)
    }

    /// `DolphinSwimToTreasureGoal.tick`: `level.getRandom().nextInt(this.adjustedTickDelay(80))`,
    /// the level's random and 40 ticks for a goal that does not tick every tick.
    #[test]
    #[ignore = "daily"]
    fn a_dolphin_hearts_its_treasure_on_the_levels_random_every_40_ticks() {
        let mut out = None;
        if !with_test_world(&[], |w, _| {
            let mut runs = Vec::new();
            // 119: `new Random(119).nextInt(40) == 0`, but `nextInt(80) != 0`;
            // 1: neither is 0. Vanilla draws 40 from the level's random.
            for seed in [119, 1] {
                let mut m = dolphin();
                // A treasure far enough that `canContinueToUse` holds.
                m.extra.home = Some([40, 64, 40]);
                let mut lr = crate::health::TestLevel::new(seed);
                let mut events = Vec::new();
                let mut cx = Cx {
                    w,
                    level: &mut lr.level(),
                    events: &mut events,
                };
                let mut goal = SwimToTreasure::default();
                goal.tick(&mut m, &mut cx);
                let heart = events
                    .iter()
                    .any(|e| matches!(e, MobEvent::AroundSelf { event: 38, .. }));
                runs.push((seed, goal.stuck, heart));
            }
            out = Some(runs);
        }) {
            return;
        }
        for (seed, stuck, _) in out.as_ref().unwrap() {
            assert!(!stuck, "seed {seed}: the dolphin found no spot to swim to");
        }
        let runs = out.unwrap();
        assert!(
            runs[0].2,
            "seed 119: the level's random says heart (nextInt(40) == 0)"
        );
        assert!(
            !runs[1].2,
            "seed 1: the level's random says no heart (nextInt(40) != 0)"
        );
    }

    /// `Dolphin.MoveToItemGoal.start`: `playSound(DOLPHIN_PLAY, 1.0, 1.0)`,
    /// which `Entity.playSound` drops for a silent dolphin.
    #[test]
    #[ignore = "daily"]
    fn a_silent_dolphin_makes_no_sound_swimming_to_an_item() {
        let ran = with_test_world(&[], |w, _| {
            for silent in [false, true] {
                let mut m = dolphin();
                m.extra.point = Some([8.5, 64.0, 0.5]);
                m.silent = silent;
                let mut lr = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut cx = Cx {
                    w,
                    level: &mut lr.level(),
                    events: &mut events,
                };
                MoveToItem::default().start(&mut m, &mut cx);
                let sounds: Vec<_> = events
                    .iter()
                    .filter_map(|e| match e {
                        MobEvent::Sound(name, at, volume, pitch) => {
                            Some((*name, *at, *volume, *pitch))
                        }
                        _ => None,
                    })
                    .collect();
                if silent {
                    assert!(sounds.is_empty(), "a silent dolphin squeaked: {sounds:?}");
                } else {
                    let feet = [m.position.x, m.position.y, m.position.z];
                    assert_eq!(sounds, vec![("entity.dolphin.play", feet, 1.0, 1.0)]);
                }
            }
        });
        assert!(ran, "vanilla data is missing");
    }
}

#[cfg(test)]
mod jump_turn_tests {
    use super::*;
    use crate::mobs::Mob;
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    /// The pinned vanilla 26.3 side is `VanillaTurnOracle`'s `dolphin.txt`
    /// (`tools/test_mob_turn_parity.py`, recorded into `.local/witness/26.3/mob_turn/dolphin.txt`):
    /// `DolphinJumpGoal.tick` on a dolphin out of water, from each motion and pitch, as the pitch
    /// bits it leaves (rising, falling, straight up and down, nearly still).
    #[test]
    #[ignore = "run by tools/test_mob_turn_parity.py"]
    fn the_jump_turn_matches_the_pinned_oracle() {
        let witness_dir = std::env::var_os("SUNRISE_MOB_TURN_WITNESS_DIR").expect(
            "SUNRISE_MOB_TURN_WITNESS_DIR must be set; run through tools/test_mob_turn_parity.py, which records the witness",
        );
        let witness_path = std::path::Path::new(&witness_dir).join("dolphin.txt");
        let witness = std::fs::read_to_string(&witness_path).unwrap_or_else(|err| {
            panic!(
                "failed to read {} ({err}); run through tools/test_mob_turn_parity.py, which records the witness",
                witness_path.display()
            )
        });
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            let mut rows = Vec::new();
            for v in MOTIONS {
                for pitch in [0.0f32, 30.0, -12.5] {
                    let kind = Arc::new(types.types["minecraft:dolphin"].clone());
                    let mut m = Mob::new(1, kind, Vec3::new(2.5, 64.0, 2.5), 0.0);
                    m.velocity = Vec3::new(v[0], v[1], v[2]);
                    m.head_pitch = pitch;
                    let mut test_level = crate::health::TestLevel::new(0);
                    let level = &mut test_level.level();
                    let mut events = Vec::new();
                    let mut cx = Cx {
                        w,
                        level,
                        events: &mut events,
                    };
                    Jump::new(10).tick(&mut m, &mut cx);
                    rows.push(format!(
                        "{} {} {} {:08x} {:08x}",
                        bits(v[0]),
                        bits(v[1]),
                        bits(v[2]),
                        pitch.to_bits(),
                        m.head_pitch.to_bits()
                    ));
                }
            }
            let actual = rows.join("\n") + "\n";
            if let Some(dir) = std::env::var_os("SUNRISE_TURN_OUTPUT_DIR") {
                std::fs::write(std::path::Path::new(&dir).join("dolphin.txt"), &actual).unwrap();
            }
            assert_eq!(rows.len(), MOTIONS.len() * 3);
            assert_eq!(actual, witness);
        });
        assert!(ran, "prepared mob data must run the jump fixture");
    }

    const MOTIONS: [[f64; 3]; 13] = [
        [1.0, 0.0, 0.0],
        [1.0, -0.0, 0.0],
        [0.3, 0.5, 0.0],
        [0.3, -0.5, 0.0],
        [0.5, 1.0, 0.0],
        [0.01, -1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.1, 0.1, 0.0],
        [0.1, -0.1, 0.0],
        [0.2, 0.05, 0.3],
        [1e-6, 0.0, 0.0],
        [0.0, 0.0, 0.0],
    ];
}
