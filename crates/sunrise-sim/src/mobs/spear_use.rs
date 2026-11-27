//! Mobs using a spear (`KineticWeapon`): `startUsingItem` of the main hand,
//! the stab of every tick of the use (`KineticWeapon.damageEntities`) and
//! what makes a mob use it: `SpearUseGoal` of zombies and zombified piglins,
//! and the fight activity's `SpearApproach`, `SpearAttack` and `SpearRetreat`
//! of piglins.
//! A mob's `using_item` counts the ticks of the use up from 1, which the
//! living entity flags tell the clients (`mob_meta`), so the arm and the
//! spear are drawn charging for them as well.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, WeaponAttack, WeaponHit, Who,
    brain::Timed,
    goals::{Cx, Goal, LOOK, MOVE, goal_clone, reduced},
};
use crate::spear_sim::{Kinetic, REACH, hits_in_reach};
use sunrise_core::geometry::{Aabb, Vec3, trig::Mth};

/// The kinetic weapon in the main hand (`getMainHandItem().get(KINETIC_WEAPON)`).
pub fn main_hand_kinetic(m: &Mob) -> Option<Kinetic> {
    m.extra.main_hand_id().and_then(Kinetic::of)
}

/// Whether `using_item` is a timer that vanilla keeps apart from a use: a
/// piglin with something in its off hand admires it (a memory), and a witch
/// drinks (`Witch.usingTime`, its own `DATA_USING_ITEM`).
pub fn counts_own_timer(m: &Mob) -> bool {
    match m.species() {
        Species::Piglin => m.extra.off_hand_id().is_some(),
        Species::Witch => m.extra.flag,
        _ => false,
    }
}

/// `LivingEntity.isUsingItem`, which an admiring piglin and a drinking witch
/// are not.
pub fn is_using_item(m: &Mob) -> bool {
    m.extra.using_item > 0 && !counts_own_timer(m)
}

/// `LivingEntity.startUsingItem` of a hand, false when it is empty or a use
/// runs: the use of a kinetic weapon starts with a new `recentKineticEnemies`
/// (`update_using_item` stabs with the main hand's).
pub fn start_using_item(m: &mut Mob, off_hand: bool) -> bool {
    let held = if off_hand {
        m.extra.off_hand_id()
    } else {
        m.extra.main_hand_id()
    };
    if held.is_none() || is_using_item(m) {
        return false;
    }
    m.extra.using_item = 1;
    m.extra.using_off_hand = off_hand;
    if !off_hand && main_hand_kinetic(m).is_some() {
        m.extra.stabbed = Some(Vec::new());
    }
    true
}

/// `LivingEntity.stopUsingItem`: `recentKineticEnemies` is gone.
pub fn stop_using_item(m: &mut Mob) {
    if is_using_item(m) {
        m.extra.using_item = 0;
    }
    m.extra.stabbed = None;
}

/// `LivingEntity.updatingUsingItem` of a kinetic weapon, in `LivingEntity.tick`
/// before the AI: the stab of this tick, then one more tick used. A use whose
/// weapon left the main hand ends. `known_speed` is what the mob moved last
/// tick (`Entity.lastKnownSpeed`).
pub fn update_using_item(m: &mut Mob, w: &MobWorld, known_speed: Vec3, events: &mut Vec<MobEvent>) {
    if m.extra.stabbed.is_none() {
        return;
    }
    let Some(kinetic) = main_hand_kinetic(m).filter(|_| is_using_item(m)) else {
        stop_using_item(m);
        return;
    };
    // `ItemStack.onUseTick` gets the ticks left; the use began at 1.
    let ticks_used = m.extra.using_item - 1;
    damage_entities(m, &kinetic, ticks_used, w, known_speed, events);
    count_use_tick(m);
}

/// One more tick of a use. Vanilla counts `useItemRemaining` down as a Java
/// int, which a mod may set to anything; here the ticks used count up, and a
/// counter a mod pushed to `i32::MAX` stays there rather than wrapping below
/// 1, which would end the use that vanilla keeps running.
pub fn count_use_tick(m: &mut Mob) {
    if is_using_item(m) {
        m.extra.using_item = m.extra.using_item.saturating_add(1);
    }
}

/// What a stab can meet: who, where and how fast it moves per second.
struct Candidate {
    who: Who,
    bounds: Aabb,
    speed: Vec3,
}

/// The entities a stab may hit (`PiercingWeapon.canHitEntity`): the player
/// the mob sees and the other living mobs near it, but not the mount it sits
/// on or the rider it carries.
fn candidates(m: &Mob, w: &MobWorld) -> Vec<Candidate> {
    let mut out = Vec::new();
    if let Some(p) = w.selected_player().filter(|p| p.alive && !p.spectator) {
        out.push(Candidate {
            who: Who::Player,
            bounds: p.bounds,
            speed: p.velocity.scale(20.0),
        });
    }
    let r = 6.0;
    let area = m.bounds().inflate(Vec3::new(r, r, r));
    for s in w.others_near(area) {
        if s.id == m.id || !s.alive || m.extra.mount == Some(s.id) || m.extra.rider == Some(s.id) {
            continue;
        }
        out.push(Candidate {
            who: Who::Mob(s.id),
            bounds: s.bounds,
            speed: s.known_speed.scale(20.0),
        });
    }
    out
}

/// `Entity.getHeadLookAngle`: `calculateViewVector(xRot, yHeadRot)`.
fn head_look_angle(m: &Mob) -> Vec3 {
    let to_rad = std::f32::consts::PI / 180.0;
    let x = m.head_pitch * to_rad;
    let y = -m.head_yaw * to_rad;
    let (y_cos, y_sin, x_cos, x_sin) = (y.mcos(), y.msin(), x.mcos(), x.msin());
    Vec3::new(
        f64::from(y_sin * x_cos),
        f64::from(-x_sin),
        f64::from(y_cos * x_cos),
    )
}

/// `KineticWeapon.getMotion`: the known speed per second of the attacker, or
/// of the mount it rides.
fn motion(m: &Mob, w: &MobWorld, known_speed: Vec3) -> Vec3 {
    m.extra
        .mount
        .and_then(|id| w.seen(id))
        .map_or(known_speed, |s| s.known_speed)
        .scale(20.0)
}

/// `KineticWeapon.damageEntities` for a mob: from `delay_ticks` of the use
/// on, every entity along the view that the speed allows is stabbed, once in
/// `contact_cooldown_ticks` each. A mob's conditions are scaled by 0.2 and
/// its reach by the `mob_factor` of the spear's `AttackRange`.
fn damage_entities(
    m: &mut Mob,
    k: &Kinetic,
    ticks_used: i32,
    w: &MobWorld,
    known_speed: Vec3,
    events: &mut Vec<MobEvent>,
) {
    if ticks_used < k.delay_ticks {
        return;
    }
    let ticks_used = ticks_used - k.delay_ticks;
    let look = m.look_angle();
    let look = Vec3::new(look[0], look[1], look[2]);
    let attacker_speed = look.dot(motion(m, w, known_speed));
    // `AttackRange.effectiveMinRange` and `effectiveMaxRange` of a mob.
    let reach = (
        REACH.min * REACH.mob_factor,
        REACH.max * REACH.mob_factor,
        REACH.hitbox_margin,
    );
    let targets = candidates(m, w);
    let boxes: Vec<(usize, Aabb)> = targets
        .iter()
        .enumerate()
        .map(|(i, t)| (i, t.bounds))
        .collect();
    let eye = Vec3::new(m.position.x, m.eye_y(), m.position.z);
    let hits = hits_in_reach(
        &boxes,
        eye,
        head_look_angle(m),
        m.velocity,
        reach,
        |from, to| w.clip_block(from, to),
    );
    let now = w.game_time;
    let action_factor = f64::from(0.2_f32);
    // `getAttributeBaseValue(ATTACK_DAMAGE)`: the base alone, without the
    // modifiers (Strength, Weakness, the weapon's, a command's) that
    // `m.attrs.attack_damage` holds.
    let base = super::finalize::base_value(m, super::finalize::Attr::AttackDamage) as f32;
    let mut affected = false;
    for (contact, (i, _)) in hits.into_iter().enumerate() {
        let target = &targets[i];
        let recent = m.extra.stabbed.as_ref().is_some_and(|stabbed| {
            stabbed.iter().any(|&(who, at)| {
                who == target.who && now - at < i64::from(k.contact_cooldown_ticks)
            })
        });
        if recent {
            continue;
        }
        // `LivingEntity.rememberStabbedEntity`.
        if let Some(stabbed) = m.extra.stabbed.as_mut() {
            match stabbed.iter_mut().find(|(who, _)| *who == target.who) {
                Some(entry) => entry.1 = now,
                None => stabbed.push((target.who, now)),
            }
        }
        let relative = (attacker_speed - look.dot(target.speed)).max(0.0);
        let dismounts = k
            .dismount
            .test(ticks_used, attacker_speed, relative, action_factor);
        let pushes = k
            .knockback
            .test(ticks_used, attacker_speed, relative, action_factor);
        let damages = k
            .damage
            .test(ticks_used, attacker_speed, relative, action_factor);
        if dismounts || pushes || damages {
            let amount = base + (relative * f64::from(k.damage_multiplier)).floor() as i32 as f32;
            affected |= stab_attack(
                m,
                target.who,
                contact,
                amount,
                (damages, pushes, dismounts),
                w,
                events,
            );
        }
    }
    if affected {
        // `Level.broadcastEntityEvent(mob, 2)`: the hit is felt and heard.
        events.push(MobEvent::EntityEvent {
            mob: m.id,
            event: 2,
            at: [m.position.x, m.position.y, m.position.z],
        });
    }
}

/// `LivingEntity.stabAttack` of a mob on `target`, with what the conditions
/// allow (`deals_damage`, `deals_knockback`, `dismounts`): whether it was
/// affected. The damage and the pushes are the game's events; a hit that
/// the target's damage cooldown refuses still counts as landed here.
fn stab_attack(
    m: &mut Mob,
    target: Who,
    contact: usize,
    base_damage: f32,
    (deals_damage, deals_knockback, dismounts): (bool, bool, bool),
    w: &MobWorld,
    events: &mut Vec<MobEvent>,
) -> bool {
    let Some(weapon) = m.extra.main_hand_stack() else {
        return false;
    };
    let from = [m.position.x, m.position.y, m.position.z];
    let to_rad = std::f32::consts::PI / 180.0;
    // `LivingEntity.stabAttack`: a type in `cannot_be_dismounted_by_item_usage` stays seated.
    let flag = crate::items::EntityTypeTags::CANNOT_BE_DISMOUNTED_BY_ITEM_USAGE;
    let seated = match target {
        Who::Player => w
            .items
            .is_some_and(|items| items.entity_type_tags.has_flag("minecraft:player", flag)),
        Who::Mob(id) => w.seen(id).is_some_and(|s| w.entity_tag_bit(&s.kind, flag)),
    };
    let dismounts = dismounts && !seated && w.target_info(target).is_some_and(|t| t.passenger);
    events.push(w.route_player_event(MobEvent::WeaponAttack(WeaponHit {
        target,
        source: Damage {
            kind: DamageKind::Spear,
            attacker: Some(Who::Mob(m.id)),
            from: Some(from),
            direct: None,
            attacker_species: Some(m.species()),
        },
        weapon: weapon.clone(),
        amount: base_damage,
        knockback: m.attrs.attack_knockback as f32,
        direction: [
            f64::from((m.yaw * to_rad).msin()),
            f64::from(-(m.yaw * to_rad).mcos()),
        ],
        attack: WeaponAttack::Stab {
            deals_damage,
            deals_knockback,
            dismounts,
        },
    })));
    let affected = deals_damage || deals_knockback || dismounts;
    if affected {
        events.push(w.route_player_event(MobEvent::StabMemory {
            attacker: m.id,
            target,
            contact,
            attempted_damage: deals_damage,
            otherwise_affected: deals_knockback || dismounts,
        }));
    }
    affected
}

/// `Mob.chargeSpeedModifier` of the mount: a zombie horse runs 1.4 times as
/// fast with a charging rider, a camel husk 4 times.
fn charge_speed_modifier(m: &Mob, w: &MobWorld) -> f32 {
    match m.extra.mount.and_then(|id| w.seen(id)).map(|s| s.species) {
        Some(Species::ZombieHorse) => 1.4,
        Some(Species::CamelHusk) => 4.0,
        _ => 1.0,
    }
}

/// `Entity.isPassenger` as the 2 blocks it adds to the charge distances.
fn passenger_distance(m: &Mob) -> i32 {
    if m.extra.mount.is_some() { 2 } else { 0 }
}

/// `ableToAttack` of `SpearUseGoal` and the spear behaviours: a target and a
/// kinetic weapon in the main hand.
fn able_to_attack(m: &Mob, w: &MobWorld) -> bool {
    m.target_info(w).is_some() && main_hand_kinetic(m).is_some()
}

/// `SpearUseGoal.SpearUseState`: where a charge is.
#[derive(Clone, Debug)]
struct SpearUseState {
    engage_time: i32,
    fleeing_time: i32,
    away_pos: Option<[f64; 3]>,
    done: bool,
}
impl SpearUseState {
    /// `SpearUseState.<init>`: not engaged and not fleeing yet.
    fn new() -> Self {
        Self {
            engage_time: -1,
            fleeing_time: -1,
            away_pos: None,
            done: false,
        }
    }
    /// `SpearUseState.notEngagedYet`.
    fn not_engaged_yet(&self) -> bool {
        self.engage_time < 0
    }
    /// `SpearUseState.startEngagement`: the spear stays down for `spear_down_time` goal ticks.
    fn start_engagement(&mut self, spear_down_time: i32) {
        self.engage_time = spear_down_time;
    }
    /// `SpearUseState.tickAndCheckEngagement`: whether the time ran out.
    fn tick_and_check_engagement(&mut self) -> bool {
        if self.engage_time > 0 {
            self.engage_time -= 1;
            if self.engage_time == 0 {
                return true;
            }
        }
        false
    }
    /// `SpearUseState.tickAndCheckFleeing`: whether it fled for too long.
    fn tick_and_check_fleeing(&mut self) -> bool {
        if self.fleeing_time > 0 {
            self.fleeing_time += 1;
            if self.fleeing_time > SpearUse::MAX_FLEEING_TIME {
                self.done = true;
                return true;
            }
        }
        false
    }
}

/// `SpearUseGoal`: with a spear in hand and a target, walk up to the
/// approach distance, lower the spear and charge until its use time ends,
/// back off 9 to 11 blocks and start over.
#[derive(Clone, Debug)]
pub struct SpearUse {
    speed_modifier_when_charging: f64,
    speed_modifier_when_repositioning: f64,
    approach_distance_sq: f32,
    target_in_range_radius_sq: f32,
    state: Option<SpearUseState>,
}
impl SpearUse {
    /// `SpearUseGoal.MAX_FLEEING_TIME`: `reducedTickDelay(100)`.
    const MAX_FLEEING_TIME: i32 = 50;

    pub fn new(
        speed_modifier_when_charging: f64,
        speed_modifier_when_repositioning: f64,
        approach_distance: f32,
        target_in_range_radius: f32,
    ) -> Self {
        Self {
            speed_modifier_when_charging,
            speed_modifier_when_repositioning,
            approach_distance_sq: approach_distance * approach_distance,
            target_in_range_radius_sq: target_in_range_radius * target_in_range_radius,
            state: None,
        }
    }

    /// `SpearUseGoal.getKineticWeaponUseDuration`.
    fn kinetic_weapon_use_duration(m: &Mob) -> i32 {
        reduced(main_hand_kinetic(m).map_or(0, |k| k.damage_use_duration()))
    }

    /// The part of `SpearUseGoal.tick` that works on the state.
    fn step(&self, state: &mut SpearUseState, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else { return };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        let target_dist_sqr = m.distance_sqr(t.position);
        let speed_modifier = f64::from(charge_speed_modifier(m, cx.w));
        let mount_distance = passenger_distance(m);
        let at = [t.position.x, t.eye_y, t.position.z];
        m.turn_to(at, 30.0, 30.0);
        m.look_at_speed(at, 30.0, 30.0);
        if state.not_engaged_yet() {
            if target_dist_sqr > f64::from(self.approach_distance_sq) {
                m.nav_move_to_entity(
                    cx.w,
                    [t.position.x, t.position.y, t.position.z],
                    1,
                    speed_modifier * self.speed_modifier_when_repositioning,
                );
                return;
            }
            state.start_engagement(Self::kinetic_weapon_use_duration(m));
            start_using_item(m, false);
        }
        if state.tick_and_check_engagement() {
            stop_using_item(m);
            let distance = target_dist_sqr.sqrt();
            state.away_pos = m.land_random_pos_away_between(
                cx.w,
                0.0_f64.max(f64::from(9 + mount_distance) - distance),
                1.0_f64.max(f64::from(11 + mount_distance) - distance),
                7,
                [t.position.x, t.position.y, t.position.z],
            );
            state.fleeing_time = 1;
        }
        if !state.tick_and_check_fleeing() {
            if let Some(away) = state.away_pos {
                m.nav_move_to(
                    cx.w,
                    away,
                    speed_modifier * self.speed_modifier_when_repositioning,
                );
                if m.ai.nav.is_done() {
                    if state.fleeing_time > 0 {
                        state.done = true;
                        return;
                    }
                    state.away_pos = None;
                }
            } else {
                m.nav_move_to_entity(
                    cx.w,
                    [t.position.x, t.position.y, t.position.z],
                    1,
                    speed_modifier * self.speed_modifier_when_charging,
                );
                if target_dist_sqr < f64::from(self.target_in_range_radius_sq) || m.ai.nav.is_done()
                {
                    let distance = target_dist_sqr.sqrt();
                    state.away_pos = m.land_random_pos_away_between(
                        cx.w,
                        f64::from(6 + mount_distance) - distance,
                        f64::from(7 + mount_distance) - distance,
                        7,
                        [t.position.x, t.position.y, t.position.z],
                    );
                }
            }
        }
    }
}
impl Goal for SpearUse {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    /// `SpearUseGoal.canUse`: able to attack and not using an item.
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        able_to_attack(m, cx.w) && !is_using_item(m)
    }
    /// `SpearUseGoal.canContinueToUse`.
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.state.as_ref().is_some_and(|s| !s.done) && able_to_attack(m, cx.w)
    }
    /// `SpearUseGoal.start`: aggressive, with a new state.
    #[inline]
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.aggressive = true;
        self.state = Some(SpearUseState::new());
    }
    /// `SpearUseGoal.stop`: the navigation stops, the spear goes down.
    #[inline]
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
        m.ai.aggressive = false;
        self.state = None;
        stop_using_item(m);
    }
    /// `SpearUseGoal.tick`.
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(mut state) = self.state.take() {
            self.step(&mut state, m, cx);
            self.state = Some(state);
        }
    }
}

/// `SpearAttack.SpearStatus`: where a piglin is in its charge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpearStatus {
    Approach,
    Charging,
    Retreat,
}

/// The spear memories of a piglin's brain (`SPEAR_STATUS` and the like). None
/// of them is saved: they have no codec.
#[derive(Clone, Debug, Default)]
pub struct Memories {
    /// `SPEAR_STATUS`.
    pub status: Option<SpearStatus>,
    /// `SPEAR_ENGAGE_TIME`: ticks of the charge left.
    pub engage_time: Option<i32>,
    /// `SPEAR_CHARGE_POSITION`: where the charge goes to reposition.
    pub charge_position: Option<[f64; 3]>,
    /// `SPEAR_FLEEING_TIME`: ticks spent backing off.
    pub fleeing_time: Option<i32>,
    /// `SPEAR_FLEEING_POSITION`: where it backs off to.
    pub fleeing_position: Option<[f64; 3]>,
}

/// A piglin's `LOOK_TARGET` of an `EntityTracker(target, true)`: its eyes
/// follow the target's eyes.
fn look_at_target(m: &mut Mob, eye: [f64; 3]) {
    m.look_at(eye);
}

/// `SpearApproach.checkExtraStartConditions`: able to attack and not using an
/// item. The behaviour also needs `SPEAR_STATUS` to be absent.
fn approach_check_extra_start_conditions(m: &Mob, w: &MobWorld) -> bool {
    able_to_attack(m, w) && !is_using_item(m)
}

/// `SpearApproach.start`: aggressive, and the status is APPROACH.
fn approach_start(m: &mut Mob) {
    m.ai.aggressive = true;
    m.extra.piglin.spear.status = Some(SpearStatus::Approach);
}

/// `SpearApproach.canStillUse`: able to attack and still farther from the
/// target than the approach distance (`farEnough`).
fn approach_can_still_use(m: &Mob, w: &MobWorld) -> bool {
    let Some(t) = m.target_info(w) else {
        return false;
    };
    able_to_attack(m, w)
        && m.distance_sqr(t.position) > f64::from(PiglinSpear::APPROACH_DISTANCE_SQ)
}

/// `SpearApproach.tick`: look at the target and walk up to it.
fn approach_tick(m: &mut Mob, cx: &mut Cx) {
    let Some(t) = m.target_info(cx.w) else { return };
    let speed_modifier = f64::from(charge_speed_modifier(m, cx.w));
    look_at_target(m, [t.position.x, t.eye_y, t.position.z]);
    m.nav_move_to_entity(
        cx.w,
        [t.position.x, t.position.y, t.position.z],
        1,
        speed_modifier * PiglinSpear::SPEED_MODIFIER_WHEN_REPOSITIONING,
    );
}

/// `SpearApproach.stop`: the walk ends and the charge is next.
fn approach_stop(m: &mut Mob) {
    m.nav_stop();
    m.extra.piglin.spear.status = Some(SpearStatus::Charging);
}

/// `SpearAttack.checkExtraStartConditions`: the status is CHARGING, able to
/// attack and not using an item.
fn attack_check_extra_start_conditions(m: &Mob, w: &MobWorld) -> bool {
    m.extra.piglin.spear.status.unwrap_or(SpearStatus::Approach) == SpearStatus::Charging
        && able_to_attack(m, w)
        && !is_using_item(m)
}

/// `SpearAttack.getKineticWeaponUseDuration`.
fn attack_kinetic_weapon_use_duration(m: &Mob) -> i32 {
    main_hand_kinetic(m).map_or(0, |k| k.damage_use_duration())
}

/// `SpearAttack.start`: aggressive, the charge lasts the weapon's use time,
/// and the spear goes up.
fn attack_start(m: &mut Mob) {
    m.ai.aggressive = true;
    m.extra.piglin.spear.engage_time = Some(attack_kinetic_weapon_use_duration(m));
    m.extra.piglin.spear.charge_position = None;
    start_using_item(m, false);
}

/// `SpearAttack.canStillUse`: time left to charge and able to attack.
fn attack_can_still_use(m: &Mob, w: &MobWorld) -> bool {
    m.extra.piglin.spear.engage_time.unwrap_or(0) > 0 && able_to_attack(m, w)
}

/// `SpearAttack.tick`: look at the target, count the charge down and keep
/// moving: run at the target, and once within 2 blocks of it (or at the end
/// of the path) pick a spot 6 to 7 blocks past it to reposition to, then run
/// at the target again from there.
fn attack_tick(m: &mut Mob, cx: &mut Cx) {
    let Some(t) = m.target_info(cx.w) else { return };
    let target_dist_sqr = m.distance_sqr(t.position);
    let speed_modifier = f64::from(charge_speed_modifier(m, cx.w));
    let mount_distance = passenger_distance(m);
    look_at_target(m, [t.position.x, t.eye_y, t.position.z]);
    let left = m.extra.piglin.spear.engage_time.unwrap_or(0) - 1;
    m.extra.piglin.spear.engage_time = Some(left);
    if let Some(away) = m.extra.piglin.spear.charge_position {
        m.nav_move_to(
            cx.w,
            away,
            speed_modifier * PiglinSpear::SPEED_MODIFIER_WHEN_REPOSITIONING,
        );
        if m.ai.nav.is_done() {
            m.extra.piglin.spear.charge_position = None;
        }
    } else {
        m.nav_move_to_entity(
            cx.w,
            [t.position.x, t.position.y, t.position.z],
            1,
            speed_modifier * PiglinSpear::SPEED_MODIFIER_WHEN_CHARGING,
        );
        if target_dist_sqr < f64::from(PiglinSpear::TARGET_IN_RANGE_RADIUS_SQ) || m.ai.nav.is_done()
        {
            let distance = target_dist_sqr.sqrt();
            m.extra.piglin.spear.charge_position = m.land_random_pos_away_between(
                cx.w,
                f64::from(6 + mount_distance) - distance,
                f64::from(7 + mount_distance) - distance,
                7,
                [t.position.x, t.position.y, t.position.z],
            );
        }
    }
}

/// `SpearAttack.stop`: the walk ends, the spear goes down and the retreat is next.
fn attack_stop(m: &mut Mob) {
    m.nav_stop();
    stop_using_item(m);
    m.extra.piglin.spear.charge_position = None;
    m.extra.piglin.spear.engage_time = None;
    m.extra.piglin.spear.status = Some(SpearStatus::Retreat);
}

/// `SpearRetreat.checkExtraStartConditions`: able to attack, not using an
/// item, the status RETREAT and a spot 9 to 11 blocks from the target to back
/// off to (2 more on a mount), which becomes `SPEAR_FLEEING_POSITION`.
fn retreat_check_extra_start_conditions(m: &mut Mob, cx: &mut Cx) -> bool {
    if !able_to_attack(m, cx.w) || is_using_item(m) {
        return false;
    }
    if m.extra.piglin.spear.status.unwrap_or(SpearStatus::Approach) != SpearStatus::Retreat {
        return false;
    }
    let Some(t) = m.target_info(cx.w) else {
        return false;
    };
    let mount_distance = passenger_distance(m);
    let distance = m.distance_sqr(t.position).sqrt();
    let Some(away) = m.land_random_pos_away_between(
        cx.w,
        0.0_f64.max(f64::from(9 + mount_distance) - distance),
        1.0_f64.max(f64::from(11 + mount_distance) - distance),
        7,
        [t.position.x, t.position.y, t.position.z],
    ) else {
        return false;
    };
    m.extra.piglin.spear.fleeing_position = Some(away);
    true
}

/// `SpearRetreat.start`: aggressive, with no time spent backing off yet.
fn retreat_start(m: &mut Mob) {
    m.ai.aggressive = true;
    m.extra.piglin.spear.fleeing_time = Some(0);
}

/// `SpearRetreat.canStillUse`: under 100 ticks backing off, with a spot to
/// back off to, a walk going on and able to attack.
fn retreat_can_still_use(m: &Mob, w: &MobWorld) -> bool {
    m.extra.piglin.spear.fleeing_time.unwrap_or(100) < 100
        && m.extra.piglin.spear.fleeing_position.is_some()
        && !m.ai.nav.is_done()
        && able_to_attack(m, w)
}

/// `SpearRetreat.tick`: look at the target, count the time and walk to the spot.
fn retreat_tick(m: &mut Mob, cx: &mut Cx) {
    let Some(t) = m.target_info(cx.w) else { return };
    let speed_modifier = f64::from(charge_speed_modifier(m, cx.w));
    look_at_target(m, [t.position.x, t.eye_y, t.position.z]);
    let time = m.extra.piglin.spear.fleeing_time.unwrap_or(0) + 1;
    m.extra.piglin.spear.fleeing_time = Some(time);
    if let Some(away) = m.extra.piglin.spear.fleeing_position {
        m.nav_move_to(
            cx.w,
            away,
            speed_modifier * PiglinSpear::SPEED_MODIFIER_WHEN_REPOSITIONING,
        );
    }
}

/// `SpearRetreat.stop`: the walk ends, the spear is down and the charge starts over.
fn retreat_stop(m: &mut Mob) {
    m.nav_stop();
    m.ai.aggressive = false;
    stop_using_item(m);
    m.extra.piglin.spear.fleeing_time = None;
    m.extra.piglin.spear.fleeing_position = None;
    m.extra.piglin.spear.status = None;
}

/// The spear behaviours of a piglin's fight activity, `SpearApproach`,
/// `SpearAttack` and `SpearRetreat`: each starts on the `SPEAR_STATUS` the
/// one before it leaves. The goal runs them in the order of `Brain.tick`:
/// the ones not running try to start, then the running ones tick or stop.
/// Fight is the activity while the piglin has a target; a piglin with a
/// spear never melees (`NoMeleeWithSpear`).
#[derive(Clone, Debug)]
pub struct PiglinSpear {
    approach: Timed,
    attack: Timed,
    retreat: Timed,
}
impl PiglinSpear {
    /// `new SpearApproach(1.0, 10.0F)`: the squared approach distance.
    const APPROACH_DISTANCE_SQ: f32 = 10.0 * 10.0;
    /// `new SpearAttack(1.0, 1.0, 2.0F)`: the speed charging and the squared
    /// radius around the target that ends the run at it.
    const SPEED_MODIFIER_WHEN_CHARGING: f64 = 1.0;
    const TARGET_IN_RANGE_RADIUS_SQ: f32 = 2.0 * 2.0;
    /// The `speedModifierWhenRepositioning` of the three behaviours.
    const SPEED_MODIFIER_WHEN_REPOSITIONING: f64 = 1.0;

    pub fn new() -> Self {
        Self {
            // `Behavior(Map)`: a duration of 60 ticks (which `timedOut` ignores).
            approach: Timed::new(60, 60),
            attack: Timed::new(60, 60),
            // `super(Map, 100)`: a timeout of 100 ticks.
            retreat: Timed::new(100, 100),
        }
    }

    /// Whether the piglin is fighting (it has a target) with a kinetic weapon.
    fn fighting_with_spear(m: &Mob) -> bool {
        m.ai.target.is_some() && main_hand_kinetic(m).is_some()
    }
}
impl Default for PiglinSpear {
    fn default() -> Self {
        Self::new()
    }
}
impl Goal for PiglinSpear {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        Self::fighting_with_spear(m)
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        Self::fighting_with_spear(m)
    }
    #[inline]
    fn every_tick(&self) -> bool {
        true
    }
    /// `Behavior.doStop` of the behaviours still running when the fight ends.
    #[inline]
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        if self.approach.running {
            self.approach.running = false;
            approach_stop(m);
        }
        if self.attack.running {
            self.attack.running = false;
            attack_stop(m);
        }
        if self.retreat.running {
            self.retreat.running = false;
            retreat_stop(m);
        }
    }
    /// `Brain.tick`'s `startEachNonRunningBehavior`, then `tickEachRunningBehavior`.
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let now = cx.w.game_time;
        if !self.approach.running
            && m.extra.piglin.spear.status.is_none()
            && approach_check_extra_start_conditions(m, cx.w)
        {
            self.approach.begin(cx.level.random(), now);
            approach_start(m);
        }
        if !self.attack.running
            && m.extra.piglin.spear.status.is_some()
            && attack_check_extra_start_conditions(m, cx.w)
        {
            self.attack.begin(cx.level.random(), now);
            attack_start(m);
        }
        if !self.retreat.running
            && m.extra.piglin.spear.status.is_some()
            && retreat_check_extra_start_conditions(m, cx)
        {
            self.retreat.begin(cx.level.random(), now);
            retreat_start(m);
        }
        if self.approach.running {
            if approach_can_still_use(m, cx.w) {
                approach_tick(m, cx);
            } else {
                self.approach.running = false;
                approach_stop(m);
            }
        }
        if self.attack.running {
            if attack_can_still_use(m, cx.w) {
                attack_tick(m, cx);
            } else {
                self.attack.running = false;
                attack_stop(m);
            }
        }
        if self.retreat.running {
            if !self.retreat.timed_out(now) && retreat_can_still_use(m, cx.w) {
                retreat_tick(m, cx);
            } else {
                self.retreat.running = false;
                retreat_stop(m);
            }
        }
    }
}

/// `MeleeAttack.isHoldingUsableNonMeleeWeapon`: a piglin holding a kinetic
/// weapon (`canUseNonMeleeWeapon`) never melees; its spear behaviours attack.
#[derive(Clone, Debug)]
pub struct NoMeleeWithSpear<G>(pub G);
/// A kinetic weapon or a crossbow in the main hand: the melee goal stays off.
pub(super) fn holds_non_melee_weapon(m: &Mob) -> bool {
    main_hand_kinetic(m).is_some() || m.extra.main_hand_id() == Some("minecraft:crossbow")
}
impl<G: Goal + Clone + 'static> Goal for NoMeleeWithSpear<G> {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    #[inline]
    fn name(&self) -> &'static str {
        self.0.name()
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !holds_non_melee_weapon(m) && self.0.can_use(m, cx)
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !holds_non_melee_weapon(m) && self.0.can_continue(m, cx)
    }
    #[inline]
    fn interruptable(&self) -> bool {
        self.0.interruptable()
    }
    #[inline]
    fn every_tick(&self) -> bool {
        self.0.every_tick()
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Mob, MobEvent, MobWorld, WeaponAttack, WeaponHit, Who, count_use_tick, damage_entities,
        is_using_item, main_hand_kinetic, stab_attack, start_using_item,
    };
    use sunrise_core::geometry::Vec3;

    fn spear_zombie() -> Mob {
        let mut m = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
        m.extra.set_slot_id(
            sunrise_items::equipment::MAINHAND,
            Some("minecraft:iron_spear".into()),
        );
        m
    }

    /// `KineticWeapon.damageEntities`: a mob's charge deals the attack damage
    /// attribute's base value (`getAttributeBaseValue`) plus the speed bonus,
    /// whatever modifiers (Strength, Weakness, a command's) the attribute has.
    #[test]
    #[ignore = "daily"]
    fn a_mob_spear_charge_deals_the_base_attack_damage_without_its_modifiers() {
        use crate::mobs::finalize::{self, Attr, Op};
        let target = Mob::new(
            2,
            crate::mobs::sensor_scan_tests::kind(),
            Vec3::new(0.0, 80.0, 1.6),
            0.0,
        );
        let others = [target.seen(None, &crate::items::Items::empty())];
        let charge = |modifier: Option<f64>| {
            let mut m = spear_zombie();
            m.position = Vec3::new(0.0, 80.0, 0.0);
            finalize::set_base(&mut m, Attr::AttackDamage, 3.0);
            if let Some(amount) = modifier {
                finalize::add_modifier(
                    &mut m,
                    Attr::AttackDamage,
                    "test:strength",
                    amount,
                    Op::AddValue,
                );
            }
            let kinetic = main_hand_kinetic(&m).expect("an iron spear");
            let mut amounts = Vec::new();
            assert!(crate::mobs::world::with_test_world(&others, |w, _| {
                let mut events = Vec::new();
                // Running 0.1 blocks a tick (2 a second) toward the still target.
                damage_entities(
                    &mut m,
                    &kinetic,
                    kinetic.delay_ticks,
                    w,
                    Vec3::new(0.0, 0.0, 0.1),
                    &mut events,
                );
                amounts.extend(events.iter().filter_map(|event| match event {
                    MobEvent::WeaponAttack(WeaponHit {
                        target: Who::Mob(2),
                        amount,
                        ..
                    }) => Some(*amount),
                    _ => None,
                }));
            }));
            (m.attrs.attack_damage, amounts)
        };
        // 3 + floor(2 * 0.95), the iron spear's damage multiplier.
        assert_eq!(charge(None), (3.0, vec![4.0]));
        assert_eq!(charge(Some(10.0)), (13.0, vec![4.0]));
        assert_eq!(charge(Some(-2.0)), (1.0, vec![4.0]));
    }

    /// The counters a mod's `living_set_item_use_remaining` leaves a 72000
    /// tick use at (remaining `i32::MIN`, `i32::MAX`, 0 and -5): the next
    /// ticks count on without overflowing, and the use keeps running.
    #[test]
    fn a_use_counter_at_any_value_a_mod_sets_survives_the_next_ticks() {
        for counter in [i32::MAX, 1, 72_001, 72_006, i32::MAX - 1] {
            let mut m = spear_zombie();
            assert!(start_using_item(&mut m, false));
            m.extra.using_item = counter;
            for tick in 1..=3 {
                count_use_tick(&mut m);
                assert!(is_using_item(&m), "{counter} tick {tick}");
                assert_eq!(
                    m.extra.using_item,
                    counter.saturating_add(tick),
                    "{counter} tick {tick}"
                );
            }
        }
    }

    /// `LivingEntity.stabAttack`: a stab that dismounts leaves a rider whose type is in
    /// `cannot_be_dismounted_by_item_usage` seated and unseats one outside it, and the answer
    /// follows the tag's membership when it is rebound (a `/reload`).
    #[test]
    #[ignore = "daily"]
    fn a_stab_leaves_a_rider_of_the_no_dismount_tag_seated() {
        use crate::items::{EntityTypeTags, Items};
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "test:rider".into();
        // Its place in the one-type registry the tags below bind, as a type's is set at load.
        kind.flags.entity_tag_slot = Some(0);
        let mut rider = Mob::new(2, std::sync::Arc::new(kind), Vec3::new(1.0, 80.0, 0.0), 0.0);
        rider.extra.mount = Some(3);
        let bind = |members: Vec<String>| {
            let mut items = Items::empty();
            items.set_entity_type_tags(EntityTypeTags::from_ordered(
                [(
                    "minecraft:cannot_be_dismounted_by_item_usage".to_owned(),
                    members,
                )]
                .into(),
                vec!["test:rider".to_owned()].into(),
            ));
            items
        };
        let (tagged, untagged) = (bind(vec!["test:rider".into()]), bind(Vec::new()));
        let others = [rider.seen(None, &tagged)];
        assert!(crate::mobs::world::with_test_world(&others, |world, _| {
            for (items, seated) in [(&tagged, true), (&untagged, false), (&tagged, true)] {
                let w = MobWorld {
                    items: Some(items),
                    ..world.clone()
                };
                let mut attacker = spear_zombie();
                let mut events = Vec::new();
                stab_attack(
                    &mut attacker,
                    Who::Mob(2),
                    0,
                    1.0,
                    (false, false, true),
                    &w,
                    &mut events,
                );
                // The stab's handler unseats the target on the decision made here.
                let dismounted = events.iter().any(|event| {
                    matches!(
                        event,
                        MobEvent::WeaponAttack(WeaponHit {
                            attack: WeaponAttack::Stab {
                                dismounts: true,
                                ..
                            },
                            ..
                        })
                    )
                });
                assert_eq!(dismounted, !seated, "seated {seated}");
            }
        }));
    }
}
