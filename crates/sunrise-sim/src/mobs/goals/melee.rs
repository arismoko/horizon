// Included in goals.rs to retain its private scope.
/// MeleeAttackGoal (and ZombieAttackGoal's raised arms when `zombie`).
#[derive(Clone, Debug)]
pub struct MeleeAttack {
    pub speed: f64,
    pub follow_unseen: bool,
    pub zombie: bool,
    /// `PolarBear.PolarBearMeleeAttackGoal`: rears up before each strike.
    pub polar_bear: bool,
    path_target: [f64; 3],
    recalc: i32,
    pub until_attack: i32,
    last_check: i64,
    raise_arm: i32,
    /// Ticks between attacks (20 unless set).
    pub interval: i32,
    /// Ticks between attacks for a baby, when they differ.
    pub baby_interval: Option<i32>,
    /// The sound played after each strike (a fox's bite).
    pub bite: Option<&'static str>,
}
impl MeleeAttack {
    pub fn new(speed: f64, follow_unseen: bool) -> Self {
        Self {
            speed,
            follow_unseen,
            zombie: false,
            polar_bear: false,
            path_target: [0.0; 3],
            recalc: 0,
            until_attack: 0,
            last_check: 0,
            raise_arm: 0,
            interval: 20,
            baby_interval: None,
            bite: None,
        }
    }
    pub fn zombie(mut self) -> Self {
        self.zombie = true;
        self
    }
    pub fn polar_bear(mut self) -> Self {
        self.polar_bear = true;
        self
    }
    pub fn interval(mut self, ticks: i32) -> Self {
        self.interval = ticks;
        self
    }
    /// `MeleeAttack.create(adult)` for adults, `create(baby)` for babies.
    pub fn intervals(mut self, adult: i32, baby: i32) -> Self {
        self.interval = adult;
        self.baby_interval = Some(baby);
        self
    }
    /// `FoxMeleeAttackGoal.checkAndPerformAttack`: the bite after each strike.
    pub fn bite(mut self, sound: &'static str) -> Self {
        self.bite = Some(sound);
        self
    }
    /// `MeleeAttackGoal.checkAndPerformAttack`, which `tick` calls after the path
    /// recalculation and the cooldown's countdown: when `canPerformAttack` holds,
    /// `resetAttackCooldown`, then `swingForAttack` and `doHurtTarget`. The cooldowns of
    /// the piglins', brutes', hoglins' and zoglins' `MeleeAttack.create` behaviour come
    /// through here too. The polar bear's and the fox's goals override the method without
    /// calling it, as in vanilla, and don't come through here. A native mod takes the strike
    /// here through `[[native.mixin]]`, where a Java mod's mixin on `checkAndPerformAttack`
    /// cancels or delays it (#4678).
    #[sunrise_mod_api_macros::replaceable]
    pub fn check_and_perform_attack(&mut self, m: &mut Mob, who: Who, t: &TargetInfo, cx: &mut Cx) {
        if self.can_perform_attack(m, who, t, cx.w) {
            self.reset_attack_cooldown(m);
            m.swing();
            super::ai::do_hurt_target(m, who, cx);
        }
    }
    /// `FoxMeleeAttackGoal.checkAndPerformAttack`, which replaces the base method without
    /// calling it: when `canPerformAttack` holds, `resetAttackCooldown`, the strike, then
    /// the bite `sound`. It has no `swingForAttack`: a fox's strike doesn't swing (#4733).
    fn fox_check_and_perform_attack(
        &mut self,
        sound: &'static str,
        m: &mut Mob,
        who: Who,
        t: &TargetInfo,
        cx: &mut Cx,
    ) {
        if self.can_perform_attack(m, who, t, cx.w) {
            self.reset_attack_cooldown(m);
            super::ai::do_hurt_target(m, who, cx);
            let at = [m.position.x, m.position.y, m.position.z];
            cx.events.push(MobEvent::Sound(sound, at, 1.0, 1.0));
        }
    }
    /// `MeleeAttackGoal.canPerformAttack`: the cooldown is over, the target is within melee
    /// range and in sight.
    fn can_perform_attack(&self, m: &mut Mob, who: Who, t: &TargetInfo, w: &MobWorld) -> bool {
        self.until_attack <= 0 && m.within_melee_range(w, t) && m.sees(w, who)
    }
    /// `MeleeAttackGoal.resetAttackCooldown`: the adult's interval, or the baby's when it
    /// has one.
    fn reset_attack_cooldown(&mut self, m: &Mob) {
        let interval = match self.baby_interval {
            Some(baby) if m.is_baby() => baby,
            _ => self.interval,
        };
        self.until_attack = adjusted(true, interval);
    }
}
impl Goal for MeleeAttack {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if cx.w.game_time - self.last_check < 20 {
            return false;
        }
        self.last_check = cx.w.game_time;
        let Some(t) = m.target_info(cx.w) else {
            return false;
        };
        if !t.alive {
            return false;
        }
        let path = m.create_path_to_entity(cx.w, [t.position.x, t.position.y, t.position.z], 0);
        let ok = path.is_some() || m.within_melee_range(cx.w, &t);
        m.ai.pending_path = path;
        ok
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(t) = m.target_info(cx.w) else {
            return false;
        };
        if !t.alive {
            return false;
        }
        if !self.follow_unseen {
            return !m.ai.nav.is_done();
        }
        // `MeleeAttackGoal.canContinueToUse`'s follow branch re-checks
        // `isWithinHome(target.blockPosition())` and refuses a spectator or
        // creative player.
        let target_block = [
            t.position.x.floor() as i32,
            t.position.y.floor() as i32,
            t.position.z.floor() as i32,
        ];
        m.ai.home
            .is_none_or(|(center, radius)| within_home(center, radius, target_block))
            && !(t.is_player && (t.creative || cx.w.selected_player().is_some_and(|p| p.spectator)))
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let path = m.ai.pending_path.take();
        m.nav_move_to_path(cx.w, path, self.speed);
        m.ai.aggressive = true;
        self.recalc = 0;
        self.until_attack = 0;
        self.raise_arm = 0;
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(t) = m.target_info(cx.w)
            && t.is_player
            && t.creative
        {
            m.set_ai_target(cx.w, None);
        }
        m.ai.aggressive = false;
        m.nav_stop();
        if self.polar_bear {
            super::polar_bear::set_standing(m, false);
        }
    }
    #[inline]
    fn every_tick(&self) -> bool {
        true
    }
    /// `MeleeAttackGoal.tick`: look at the target, recalculate the path, count the cooldown
    /// down, then `checkAndPerformAttack`, or the polar bear's or the fox's own
    /// (`ZombieAttackGoal.tick` raises the arms after it).
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else { return };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        m.look_at_speed([t.position.x, t.eye_y, t.position.z], 30.0, 30.0);
        self.recalc = (self.recalc - 1).max(0);
        let moved = (t.position.x - self.path_target[0]).powi(2)
            + (t.position.y - self.path_target[1]).powi(2)
            + (t.position.z - self.path_target[2]).powi(2)
            >= 1.0;
        if (self.follow_unseen || m.sees(cx.w, who))
            && self.recalc <= 0
            && (self.path_target == [0.0; 3] || moved || m.random.next_float() < 0.05)
        {
            self.path_target = [t.position.x, t.position.y, t.position.z];
            self.recalc = 4 + m.random.next_int_bound(7).unwrap_or(0);
            let d = m.distance_sqr(t.position);
            if d > 1024.0 {
                self.recalc += 10;
            } else if d > 256.0 {
                self.recalc += 5;
            }
            if !m.nav_move_to_entity(cx.w, self.path_target, 0, self.speed) {
                self.recalc += 15;
            }
            self.recalc = adjusted(true, self.recalc);
        }
        self.until_attack = (self.until_attack - 1).max(0);
        if self.polar_bear {
            let cooldown = adjusted(true, self.interval);
            super::polar_bear::check_and_perform_attack(
                &mut self.until_attack,
                cooldown,
                m,
                who,
                &t,
                cx,
            );
        } else if let Some(sound) = self.bite {
            self.fox_check_and_perform_attack(sound, m, who, &t, cx);
        } else {
            self.check_and_perform_attack(m, who, &t, cx);
        }
        if self.zombie {
            self.raise_arm += 1;
            m.ai.aggressive = self.raise_arm >= 5 && self.until_attack < adjusted(true, 20) / 2;
        }
    }
}
/// LeapAtTargetGoal.
#[derive(Clone, Debug)]
pub struct LeapAtTarget {
    pub yd: f32,
    target: [f64; 3],
}
impl LeapAtTarget {
    pub fn new(yd: f32) -> Self {
        Self {
            yd,
            target: [0.0; 3],
        }
    }
}
impl Goal for LeapAtTarget {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(t) = m.target_info(cx.w) else {
            return false;
        };
        let d = m.distance_sqr(t.position);
        if !(4.0..=16.0).contains(&d) || !m.ground.on_ground {
            return false;
        }
        self.target = [t.position.x, t.position.y, t.position.z];
        m.random.next_int_bound(reduced(5)).unwrap_or(1) == 0
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ground.on_ground
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        let mut d = [
            self.target[0] - m.position.x,
            0.0,
            self.target[2] - m.position.z,
        ];
        let len_sq = d[0] * d[0] + d[2] * d[2];
        if len_sq > 1.0e-7 {
            let l = len_sq.sqrt();
            d = [
                d[0] / l * 0.4 + m.velocity.x * 0.2,
                0.0,
                d[2] / l * 0.4 + m.velocity.z * 0.2,
            ];
        }
        m.velocity = Vec3::new(d[0], f64::from(self.yd), d[2]);
    }
}
