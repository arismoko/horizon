//! Guardians and elder guardians (`Guardian`, `ElderGuardian`): the laser
//! charging for 80 (60) ticks before it strikes, spikes that hurt melee
//! attackers while still, flopping on land, the swimming move control, and
//! the elder's mining fatigue curse every minute.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    control::{MoveKind, Operation, rotlerp},
    goals::{
        Cx, Goal, LOOK, LookAtPlayer, MOVE, NearestTarget, RandomLookAround, Stroll, goal_clone,
    },
    nav::NavKind,
    path::PathType,
    trader::MoveTowardsRestriction,
};
use crate::health::DamageLevel;
use std::f32::consts::PI;
use std::ops::ControlFlow;
use sunrise_core::geometry::trig::Mth;
use sunrise_core::{
    geometry::{Axis, Vec3},
    random::RandomSource,
};

// Guardian state: `flag` an active beam target (DATA_ID_ATTACK_TARGET),
// `flag2` moving (DATA_ID_MOVING), `counter2` the client attack time.
// AnimState slots: tail animation, its speed, spikes animation.
pub const TAIL: usize = 0;
const TAIL_SPEED: usize = 1;
pub const SPIKES: usize = 2;
const EYE_X: usize = 3;
const EYE_Y: usize = 4;

pub fn is_elder(m: &Mob) -> bool {
    m.species() == Species::ElderGuardian
}
fn attack_duration(m: &Mob) -> i32 {
    if is_elder(m) { 60 } else { 80 }
}
pub fn register(m: &mut Mob) {
    m.ai.mov.kind = MoveKind::Custom;
    m.ai.nav.kind = NavKind::Water { breach: false };
    m.set_malus(PathType::Water, 0.0);
    let t = m.random.next_float();
    m.anim.v[TAIL] = t;
    m.anim.vo[TAIL] = t;
    let elder = is_elder(m);
    if elder {
        m.persistent = true;
    }
    let g = &mut m.ai.goals;
    g.add(4, GuardianAttack { attack_time: 0 });
    g.add(5, MoveTowardsRestriction::new(1.0).looking());
    g.add(7, Stroll::new(1.0).interval(if elder { 400 } else { 80 }));
    g.add(8, LookAtPlayer::new(8.0));
    g.add(9, RandomLookAround::default());
    // GuardianAttackSelector: players, squid and axolotls beyond 3 blocks.
    m.ai.targets.add(
        1,
        NearestTarget::player(true).when(|m, w| {
            w.selected_player()
                .is_some_and(|p| p.position.distance_squared(m.position) > 9.0)
        }),
    );
    m.ai.targets.add(
        1,
        NearestTarget::species(
            &[Species::Squid, Species::GlowSquid, Species::Axolotl],
            true,
        )
        .beyond(9.0),
    );
}
/// `GuardianAttackGoal`.
#[derive(Clone, Debug)]
struct GuardianAttack {
    attack_time: i32,
}
impl Goal for GuardianAttack {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    #[inline]
    fn every_tick(&self) -> bool {
        true
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.target_info(cx.w).is_some_and(|t| t.alive)
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.can_use(m, cx)
            && (is_elder(m)
                || m.target_info(cx.w)
                    .is_some_and(|t| t.position.distance_squared(m.position) > 9.0))
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.attack_time = -10;
        m.ai.nav.stop();
        if let Some(t) = m.target_info(cx.w) {
            m.look_at_speed([t.position.x, t.eye_y, t.position.z], 90.0, 90.0);
        }
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.extra.flag = false;
        m.set_ai_target(cx.w, None);
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let (Some(who), Some(t)) = (m.ai.target, m.target_info(cx.w)) else {
            return;
        };
        m.ai.nav.stop();
        m.look_at_speed([t.position.x, t.eye_y, t.position.z], 90.0, 90.0);
        if !m.sees(cx.w, who) {
            m.set_ai_target(cx.w, None);
            return;
        }
        self.attack_time += 1;
        if self.attack_time == 0 {
            m.extra.flag = true;
            m.extra.counter2 = 0;
            // Entity event 21: the attack sound loop starts on the clients.
            if !m.silent {
                cx.events.push(MobEvent::EntityEvent {
                    mob: m.id,
                    event: 21,
                    at: [m.position.x, m.position.y, m.position.z],
                });
            }
        } else if self.attack_time >= attack_duration(m) {
            // Indirect magic (1, +2 on Hard, +2 for the elder), then
            // doHurtTarget.
            let magic = 1.0
                + if cx.w.difficulty.hard { 2.0 } else { 0.0 }
                + if is_elder(m) { 2.0 } else { 0.0 };
            let from = [m.position.x, m.position.y, m.position.z];
            match who {
                Who::Player => cx
                    .events
                    .push(cx.w.route_player_event(MobEvent::DamagePlayer {
                        amount: magic,
                        from,
                        kind: "minecraft:indirect_magic",
                        wielder: None,
                    })),
                Who::Mob(id) => cx.events.push(MobEvent::DamageMob {
                    target: id,
                    amount: magic,
                    source: Damage {
                        kind: DamageKind::Magic,
                        attacker: Some(Who::Mob(m.id)),
                        from: Some(from),
                        direct: None,
                        attacker_species: Some(m.species()),
                    },
                }),
            }
            super::ai::do_hurt_target(m, who, cx);
            m.set_ai_target(cx.w, None);
        }
    }
}
/// `GuardianMoveControl`: swim straight at the wanted point with a wobble,
/// looking ahead.
pub fn move_control(m: &mut Mob) {
    let moving = m.ai.mov.operation == Operation::MoveTo && !m.ai.nav.is_done();
    if !moving {
        m.set_speed(0.0);
        m.extra.flag2 = false;
        return;
    }
    let [wx, wy, wz] = m.ai.mov.wanted;
    let d = Vec3::new(wx - m.position.x, wy - m.position.y, wz - m.position.z);
    let length = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
    let (xd, yd, zd) = (d.x / length, d.y / length, d.z / length);
    let y_rot = (d.z.atan2(d.x).to_degrees() as f32) - 90.0;
    m.yaw = rotlerp(m.yaw, y_rot, 90.0);
    m.body_yaw = m.yaw;
    let target = (m.ai.mov.speed_modifier * m.movement_speed()) as f32;
    let speed = m.ai.speed + (target - m.ai.speed) * 0.125;
    m.set_speed(speed);
    let t = f64::from(m.age as i32 + m.id as i32);
    let push = (t * 0.5).sin() * 0.05;
    let (cos, sin) = (
        f64::from(m.yaw.to_radians().mcos()),
        f64::from(m.yaw.to_radians().msin()),
    );
    let y_push = (t * 0.75).sin() * 0.05;
    m.velocity = m.velocity.add_vector(Vec3::new(
        push * cos,
        y_push * (sin + cos) * 0.25 + f64::from(speed) * yd * 0.1,
        push * sin,
    ));
    let new_look = [
        m.position.x + xd * 2.0,
        m.eye_y() + yd / length,
        m.position.z + zd * 2.0,
    ];
    let old = if m.ai.look.cooldown > 0 {
        m.ai.look.wanted
    } else {
        new_look
    };
    let lerp = |a: f64, b: f64| a + (b - a) * 0.125;
    m.look_at_speed(
        [
            lerp(old[0], new_look[0]),
            lerp(old[1], new_look[1]),
            lerp(old[2], new_look[2]),
        ],
        10.0,
        40.0,
    );
    m.extra.flag2 = true;
}
/// `Guardian.travelInWater`; on land the usual travel.
pub(super) fn travel(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    input: Vec3,
    events: &mut Vec<MobEvent>,
    pause: bool,
) -> ControlFlow<super::living::TravelPause> {
    m.move_relative(0.1, input);
    m.travel_move(
        w,
        level,
        events,
        pause,
        sunrise_world::travel::TravelMode::Water,
        |m, _, _, _| {
            m.set_velocity(m.velocity.scale(0.9));
            if !m.extra.flag2 && m.ai.target.is_none() {
                m.set_velocity(m.velocity.with_component(Axis::Y, m.velocity.y - 0.005));
            }
        },
    )
}
/// `Guardian.aiStep`'s client-only tail and spikes animation update.
pub fn tail_and_spikes(m: &mut Mob) {
    let moving = m.extra.flag2;
    let in_water = m.fluids.in_water;
    // Tail and spikes (clientSideTailAnimation / clientSideSpikesAnimation).
    let a = &mut m.anim;
    let speed = &mut a.v[TAIL_SPEED];
    if !in_water {
        *speed = 2.0;
    } else if moving {
        if *speed < 0.5 {
            *speed = 4.0;
        } else {
            *speed += (0.5 - *speed) * 0.1;
        }
    } else {
        *speed += (0.125 - *speed) * 0.2;
    }
    a.v[TAIL] += a.v[TAIL_SPEED];
    if !in_water {
        a.v[SPIKES] = m.random.next_float();
    } else if moving {
        a.v[SPIKES] += (0.0 - a.v[SPIKES]) * 0.25;
    } else {
        a.v[SPIKES] += (1.0 - a.v[SPIKES]) * 0.06;
    }
}
/// `Guardian.aiStep`: air, flopping on land and the beam's clock. The
/// client alone draws random spike animation; defensive spikes are in [`hurt_server`].
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if !m.alive() {
        return;
    }
    let in_water = m.fluids.in_water;
    client_tick(m);
    if in_water {
        m.air = 300;
    } else if m.ground.on_ground {
        let r = &mut m.random;
        let (x, z) = (r.next_float(), r.next_float());
        m.velocity = m.velocity.add_vector(flop_kick(x, z));
        m.yaw = m.random.next_float() * 360.0;
        m.ground.on_ground = false;
        if !m.silent {
            let flop = if is_elder(m) {
                "entity.elder_guardian.flop"
            } else {
                "entity.guardian.flop"
            };
            events.push(MobEvent::Sound(
                flop,
                [m.position.x, m.position.y, m.position.z],
                1.0,
                1.0,
            ));
        }
    }
    if m.extra.flag {
        m.yaw = m.head_yaw;
    }
    // The eye follows the active target, else the player (the camera).
    let look = match m.target_info(w).filter(|_| m.extra.flag) {
        Some(t) => Some([t.position.x, t.eye_y, t.position.z]),
        None => w.selected_player().map(|p| p.eye),
    };
    if let Some(look) = look {
        look_at(m, look);
    }
}
/// `Guardian.hurtServer`: a guardian that is not moving hurts the living thing
/// that hit it for 2, before `LivingEntity.hurtServer`'s guards (a blow it
/// refuses draws the spikes too). `#avoids_guardian_thorns` (magic, thorns,
/// explosions) is exempt, and so is a projectile: its direct entity is not
/// living, though its owner is what `Damage::attacker` names. The kinds below
/// are the ones whose direct entity is the living attacker itself (a player's
/// blow, a mob's melee, a bee's sting, a warden's boom).
pub fn hurt_server(m: &mut Mob, d: &Damage) {
    if m.extra.flag2
        || !matches!(
            d.kind,
            DamageKind::PlayerAttack
                | DamageKind::MaceSmash
                | DamageKind::Spear
                | DamageKind::MobAttack
                | DamageKind::MobAttackNoAggro
                | DamageKind::Sting
                | DamageKind::SonicBoom
        )
    {
        return;
    }
    let from = [m.position.x, m.position.y, m.position.z];
    match d.attacker {
        Some(Who::Player) => m.outbox.push(MobEvent::DamagePlayer {
            amount: 2.0,
            from,
            kind: "minecraft:thorns",
            wielder: None,
        }),
        Some(Who::Mob(id)) => m.outbox.push(MobEvent::DamageMob {
            target: id,
            amount: 2.0,
            source: Damage {
                kind: DamageKind::Thorns,
                attacker: Some(Who::Mob(m.id)),
                from: Some(from),
                direct: None,
                attacker_species: Some(m.species()),
            },
        }),
        None => {}
    }
}
/// `GuardianModel.setupAnim`'s eye: it follows `look` (the attack target's
/// eye, else the camera's), from where the guardian faces.
pub fn look_at(m: &mut Mob, look: [f64; 3]) {
    m.anim.v[EYE_Y] = if look[1] - m.eye_y() > 0.0 { 0.0 } else { 1.0 };
    let yaw = m.head_yaw.to_radians();
    let pitch = m.head_pitch.to_radians();
    let view = [-yaw.msin() * pitch.mcos(), yaw.mcos() * pitch.mcos()];
    let d = [m.position.x - look[0], m.position.z - look[2]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
    let (dx, dz) = ((d[0] / len) as f32, (d[1] / len) as f32);
    // Vec3.yRot(pi / 2) of the direction from the target.
    let side = [dz, -dx];
    let dot = view[0] * side[0] + view[1] * side[1];
    m.anim.v[EYE_X] = dot.abs().sqrt() * 2.0 * dot.signum();
}
/// The GUARDIAN_ELDER_EFFECT parameter of an elder's curse
/// (`isSilent() ? 0.0F : 1.0F`); the client plays the curse sound only for 1.
pub fn elder_curse_param(silent: bool) -> f32 {
    if silent { 0.0 } else { 1.0 }
}
/// `ElderGuardian.customServerAiStep`: every minute, mining fatigue III for
/// players within 50 blocks; a home within 16 blocks.
pub fn elder_server_ai_step(m: &mut Mob, cx: &mut Cx) {
    if (m.age as i64 + i64::from(m.id)) % 1200 == 0 {
        cx.events.push(MobEvent::ElderCurse {
            at: [m.position.x, m.position.y, m.position.z],
            silent: m.silent,
        });
    }
    if m.ai.home.is_none() {
        m.ai.home = Some((
            [
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ],
            16,
        ));
    }
}
#[cfg(test)]
mod elder_home_tests {
    use super::*;
    use crate::mobs::world::with_test_world;
    use std::sync::Arc;

    /// `ElderGuardian.customServerAiStep`: with no home yet, `setHomeTo(blockPosition(), 16)`,
    /// the restriction its wandering goals read (`Mob::restricted`); a home it has is kept.
    #[test]
    #[ignore = "daily"]
    fn an_elder_guardian_homes_where_it_stands() {
        assert!(
            with_test_world(&[], |w, types| {
                let kind = Arc::new(types.types["minecraft:elder_guardian"].clone());
                let mut test_level = crate::health::TestLevel::new(0);
                let mut level = test_level.level();
                let mut events = Vec::new();
                let mut m = Mob::new(1, kind.clone(), Vec3::new(10.7, 64.2, -20.9), 0.0);
                elder_server_ai_step(
                    &mut m,
                    &mut Cx {
                        w,
                        level: &mut level,
                        events: &mut events,
                    },
                );
                assert_eq!(m.ai.home, Some(([10, 64, -21], 16)));
                assert!(!m.restricted(0.0, [20, 64, -21]));
                assert!(m.restricted(0.0, [27, 64, -21]));

                let mut kept = Mob::new(2, kind, Vec3::new(10.0, 64.0, 20.0), 0.0);
                kept.ai.home = Some(([100, 50, 100], 24));
                elder_server_ai_step(
                    &mut kept,
                    &mut Cx {
                        w,
                        level: &mut level,
                        events: &mut events,
                    },
                );
                assert_eq!(kept.ai.home, Some(([100, 50, 100], 24)));
            }),
            "vanilla data is missing"
        );
    }

    /// `ElderGuardian.customServerAiStep`: on its minute (`(tickCount + getId()) % 1200 == 0`)
    /// the curse goes out with `isSilent() ? 0.0F : 1.0F`, and
    /// `ClientPacketListener.handleGameEvent` plays the curse sound only for 1.
    #[test]
    #[ignore = "daily"]
    fn a_silent_elder_guardian_curses_without_the_sound() {
        assert!(
            with_test_world(&[], |w, types| {
                let kind = Arc::new(types.types["minecraft:elder_guardian"].clone());
                let mut fixture = crate::health::TestLevel::new(0);
                let mut level = fixture.level();
                for silent in [false, true] {
                    let mut events = Vec::new();
                    let mut m = Mob::new(1200, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
                    m.silent = silent;
                    elder_server_ai_step(
                        &mut m,
                        &mut Cx {
                            w,
                            level: &mut level,
                            events: &mut events,
                        },
                    );
                    let curses: Vec<_> = events
                        .iter()
                        .filter_map(|e| match e {
                            MobEvent::ElderCurse { silent, .. } => Some(*silent),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(curses, [silent]);
                }
            }),
            "vanilla data is missing"
        );
        assert_eq!(elder_curse_param(false), 1.0);
        assert_eq!(elder_curse_param(true), 0.0);
    }
}
/// Guardian.aiStep's visual attack clock, also used by server clients.
/// Does not run attack goals, movement, damage or random draws.
pub fn client_tick(m: &mut Mob) {
    if m.extra.flag && m.extra.counter2 < attack_duration(m) {
        m.extra.counter2 += 1;
    }
}
/// The beam's clock for rendering: (attack time with the partial tick,
/// attack scale), while a target is locked.
pub fn beam(m: &Mob, partial: f32) -> Option<(f32, f32)> {
    let guardian = matches!(m.species(), Species::Guardian | Species::ElderGuardian);
    if !guardian || !m.extra.flag || m.ai.target.is_none() {
        return None;
    }
    let time = m.extra.counter2 as f32 + partial;
    Some((time, time / attack_duration(m) as f32))
}
/// `GuardianModel.setupAnim`: head, spikes by withdrawal, the eye toward
/// what it looks at, and the tail.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    p.yrot("head", s.y_rot.to_radians());
    p.xrot("head", s.x_rot.to_radians());
    let spikes = m.anim.lerp(SPIKES, s.partial);
    let withdrawal = (1.0 - spikes) * 0.55;
    const X: [f32; 12] = [
        0.0, 0.0, 8.0, -8.0, -8.0, 8.0, 8.0, -8.0, 0.0, 0.0, 8.0, -8.0,
    ];
    const Y: [f32; 12] = [
        -8.0, -8.0, -8.0, -8.0, 0.0, 0.0, 0.0, 0.0, 8.0, 8.0, 8.0, 8.0,
    ];
    const Z: [f32; 12] = [
        8.0, -8.0, 0.0, 0.0, -8.0, -8.0, 8.0, 8.0, 8.0, -8.0, 0.0, 0.0,
    ];
    for i in 0..12 {
        let offset = 1.0 + (s.age * 1.5 + i as f32).mcos() * 0.01 - withdrawal;
        let part = p.get(&format!("spike{i}"));
        part[0] = X[i] * offset;
        part[1] = 16.0 + Y[i] * offset;
        part[2] = Z[i] * offset;
    }
    let e = p.get("eye");
    e[0] = m.anim.v[EYE_X];
    e[1] = m.anim.v[EYE_Y];
    let tail = m.anim.lerp(TAIL, s.partial);
    p.yrot("tail0", tail.msin() * PI * 0.05);
    p.yrot("tail1", tail.msin() * PI * 0.1);
    p.yrot("tail2", tail.msin() * PI * 0.15);
}
/// `Guardian.getAttackAnimationScale(0)`: how far the beam has charged. A
/// beam that ended counts from zero again.
pub fn attack_scale(m: &Mob) -> f32 {
    if m.extra.flag {
        m.extra.counter2 as f32 / attack_duration(m) as f32
    } else {
        0.0
    }
}
/// `Guardian.aiStep`'s flop on land: `(nextFloat() * 2 - 1) * 0.4F` for x and
/// z, in float and only then widened to double, and 0.5 up.
fn flop_kick(x: f32, z: f32) -> Vec3 {
    let kick = |f: f32| f64::from((f * 2.0 - 1.0) * 0.4);
    Vec3::new(kick(x), 0.5, kick(z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    /// `0.5F * 0.4F` widened is `(double) 0.2F`, not the double 0.2.
    #[test]
    fn a_flop_kicks_in_float() {
        let kick = flop_kick(0.75, 0.25);
        assert_eq!(kick.x.to_bits(), 0.20000000298023224f64.to_bits());
        assert_eq!(kick.y, 0.5);
        assert_eq!(kick.z.to_bits(), (-0.20000000298023224f64).to_bits());
    }

    /// `Guardian.aiStep` on the server draws x, z and yaw, without the
    /// client-only spike animation draw.
    #[test]
    #[ignore = "daily"]
    fn a_seeded_flop_pins_the_motion() {
        assert!(
            with_test_world(&[], |w, types| {
                let mut mob = Mob::new(
                    1,
                    Arc::new(types.types["minecraft:guardian"].clone()),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                register(&mut mob);
                mob.random.set_seed(42);
                mob.ground.on_ground = true;
                mob.fluids.in_water = false;
                mob.silent = true;
                ai_step(&mut mob, w, &mut Vec::new());
                assert_eq!(
                    [mob.velocity.x, mob.velocity.y, mob.velocity.z].map(f64::to_bits),
                    [
                        0x3fc7_4d72_0000_0000,
                        0x3fe0_0000_0000_0000,
                        0xbfd6_cd17_a000_0000
                    ]
                );
                assert_eq!(mob.yaw.to_bits(), 0x4375_f5df);
            }),
            "vanilla data is missing"
        );
    }
    use std::sync::Arc;

    fn flop(kind: &str, silent: bool) -> Option<(Mob, Vec<MobEvent>)> {
        let mut result = None;
        let ran = with_test_world(&[], |w, types| {
            let mut mob = Mob::new(
                1,
                Arc::new(types.types[kind].clone()),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            );
            register(&mut mob);
            mob.ground.on_ground = true;
            mob.fluids.in_water = false;
            mob.silent = silent;
            let mut events = Vec::new();
            ai_step(&mut mob, w, &mut events);
            result = Some((mob, events));
        });
        if !ran {
            return None;
        }
        result
    }

    /// `Guardian.aiStep`: a guardian out of water flops, and the flop is heard
    /// (the elder's own sound) unless the guardian is silent (`!isSilent()`).
    #[test]
    #[ignore = "daily"]
    fn a_silent_guardian_flops_without_a_sound() {
        for (kind, sound) in [
            ("minecraft:guardian", "entity.guardian.flop"),
            ("minecraft:elder_guardian", "entity.elder_guardian.flop"),
        ] {
            for silent in [false, true] {
                let Some((mob, events)) = flop(kind, silent) else {
                    return;
                };
                assert!(
                    !mob.ground.on_ground && mob.velocity.y > 0.0,
                    "{kind} flops"
                );
                let sounds: Vec<_> = events
                    .iter()
                    .filter_map(|e| match e {
                        MobEvent::Sound(s, ..) => Some(*s),
                        _ => None,
                    })
                    .collect();
                let heard: &[&str] = if silent { &[] } else { &[sound] };
                assert_eq!(sounds, heard, "{kind} silent={silent}");
            }
        }
    }
}
