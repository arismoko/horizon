//! Vexes (`Vex`): summoned by evokers, flying through walls around their
//! bound origin, charging at their target, and wasting away after their
//! limited life.
use super::{
    Damage, DamageKind, Mob, MobEvent, Species, Who,
    anim::{Poses, State},
    control::Operation,
    goals::{
        Cx, Float, Goal, HurtByTarget, LookAtPlayer, MOVE, NearestTarget, goal_clone, reduced,
    },
};
use crate::health::DamageLevel;
use std::f32::consts::PI;
use sunrise_core::geometry::trig::Mth;
use sunrise_core::{geometry::Vec3, random::RandomSource};

// Vex state: `flag` charging, `counter2` the owner's mob id (0 none),
// `home` the bound origin, `egg_time` the life left (-1 unlimited).

/// The `Vex` constructor and `registerGoals`. Gravity stays on: `Vex.tick`
/// turns it off only after its first `super.tick()`, so a new vex with AI
/// falls for one tick.
pub fn register(m: &mut Mob) {
    m.ai.mov.kind = super::control::MoveKind::Custom;
    m.extra.set_main_hand(Some("minecraft:iron_sword".into()));
    if m.extra.egg_time == 0 {
        m.extra.egg_time = -1;
    }
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(4, Charge);
    g.add(8, RandomMove);
    g.add(9, LookAtPlayer::new(3.0).probability(1.0));
    g.add(10, LookAtPlayer::mobs(8.0));
    // `HurtByTargetGoal(this, Raider.class)`: raiders are ignored, vexes are not.
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![
        Species::Pillager,
        Species::Vindicator,
        Species::Evoker,
        Species::Illusioner,
        Species::Witch,
        Species::Ravager,
    ];
    m.ai.targets.add(1, hurt);
    m.ai.targets.add(2, CopyOwnerTarget);
    m.ai.targets.add(3, NearestTarget::player(true));
}
fn has_wanted(m: &Mob) -> bool {
    m.ai.mov.operation == Operation::MoveTo
}
fn set_wanted(m: &mut Mob, at: [f64; 3], speed: f64) {
    m.ai.mov.wanted = at;
    m.ai.mov.speed_modifier = speed;
    m.ai.mov.operation = Operation::MoveTo;
}
/// `VexChargeAttackGoal`.
#[derive(Clone, Debug)]
struct Charge;
impl Goal for Charge {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE
    }
    #[inline]
    fn every_tick(&self) -> bool {
        true
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(t) = m.target_info(cx.w).filter(|t| t.alive) else {
            return false;
        };
        !has_wanted(m)
            && m.random.next_int_bound(reduced(7)).unwrap_or(1) == 0
            && m.distance_sqr(t.position) > 4.0
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        has_wanted(m) && m.extra.flag && m.target_info(cx.w).is_some_and(|t| t.alive)
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(t) = m.target_info(cx.w) {
            set_wanted(m, [t.position.x, t.eye_y, t.position.z], 1.0);
        }
        m.extra.flag = true;
        cx.events.push(MobEvent::Sound(
            "entity.vex.charge",
            [m.position.x, m.position.y, m.position.z],
            1.0,
            1.0,
        ));
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        m.extra.flag = false;
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let (Some(who), Some(t)) = (m.ai.target, m.target_info(cx.w)) else {
            return;
        };
        if m.bounds().intersects(t.bounds) {
            super::ai::do_hurt_target(m, who, cx);
            m.extra.flag = false;
        } else if m.distance_sqr(t.position) < 9.0 {
            set_wanted(m, [t.position.x, t.eye_y, t.position.z], 1.0);
        }
    }
}
/// `VexRandomMoveGoal`: drift to an empty block near the bound origin.
#[derive(Clone, Debug)]
struct RandomMove;
impl Goal for RandomMove {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        !has_wanted(m) && m.random.next_int_bound(reduced(7)).unwrap_or(1) == 0
    }
    #[inline]
    fn can_continue(&mut self, _m: &mut Mob, _cx: &mut Cx) -> bool {
        false
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        // Ticked once: the goal never continues.
        let origin = m.extra.home.unwrap_or([
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ]);
        for _ in 0..3 {
            let r = &mut m.random;
            let p = [
                origin[0] + r.next_int_bound(15).unwrap_or(7) - 7,
                origin[1] + r.next_int_bound(11).unwrap_or(5) - 5,
                origin[2] + r.next_int_bound(15).unwrap_or(7) - 7,
            ];
            if cx.w.facts.is_air(cx.w.world.block(p[0], p[1], p[2])) {
                let at = [
                    f64::from(p[0]) + 0.5,
                    f64::from(p[1]) + 0.5,
                    f64::from(p[2]) + 0.5,
                ];
                set_wanted(m, at, 0.25);
                if m.ai.target.is_none() {
                    m.look_at_speed(at, 180.0, 20.0);
                }
                break;
            }
        }
    }
}
/// `Vex.getOwner`: the mob the vex is bound to. `counter2` holds its id while
/// the evoker that summoned it is in the level; a vex loaded from a save knows
/// only the UUID it was saved with, and vanilla's `EntityReference.getEntity`
/// asks the level for that entity every time it is called
/// (`MobWorld::who_by_uuid`), so an owner that loaded later is found too.
pub fn owner(m: &Mob, lookup: Option<&dyn super::WhoLookup>) -> Option<u32> {
    if m.extra.counter2 != 0 {
        return Some(m.extra.counter2 as u32);
    }
    match lookup?.who_by_uuid(m.extra.owner?)? {
        Who::Mob(id) => Some(id),
        // `Vex.setOwner` takes a `Mob`: a UUID naming the player is no owner.
        Who::Player => None,
    }
}
/// `VexCopyOwnerTargetGoal`: the evoker's target.
#[derive(Clone, Debug)]
struct CopyOwnerTarget;
impl Goal for CopyOwnerTarget {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        super::goals::TARGET
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let seen = owner(m, cx.w.who_lookup).and_then(|owner| {
            cx.w.others
                .iter()
                .find(|s| s.id == owner)
                .and_then(|s| s.target)
        });
        seen.is_some_and(|t| t != Who::Mob(m.id) && copyable(cx.w, t))
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.target_info(cx.w).is_some_and(|t| t.alive)
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let target = owner(m, cx.w.who_lookup).and_then(|owner| {
            cx.w.others
                .iter()
                .find(|s| s.id == owner)
                .and_then(|s| s.target)
        });
        m.set_ai_target(cx.w, target);
    }
}
/// `TargetingConditions.test` for the owner's target: `canBeSeenByAnyone`
/// leaves out the dead and spectators.
fn copyable(w: &crate::mobs::MobWorld, t: Who) -> bool {
    match t {
        Who::Player => w.player.as_ref().is_some_and(|p| p.alive && !p.spectator),
        Who::Mob(id) => w.seen(id).is_some_and(|s| s.alive),
    }
}
/// `VexMoveControl`: accelerate straight at the wanted point.
pub fn move_control(m: &mut Mob) {
    if m.ai.mov.operation != Operation::MoveTo {
        return;
    }
    let [wx, wy, wz] = m.ai.mov.wanted;
    let d = Vec3::new(wx - m.position.x, wy - m.position.y, wz - m.position.z);
    let len = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
    let b = m.bounds();
    let (lo, hi) = (b.min_position(), b.max_position());
    let size = ((hi.x - lo.x) + (hi.y - lo.y) + (hi.z - lo.z)) / 3.0;
    if len < size {
        m.ai.mov.operation = Operation::Wait;
        m.velocity = m.velocity.scale(0.5);
        return;
    }
    m.velocity = m
        .velocity
        .add_vector(d.scale(m.ai.mov.speed_modifier * 0.05 / len));
    let yaw = match m.extra.point.filter(|_| m.ai.target.is_some()) {
        Some(t) => -((t[0] - m.position.x).atan2(t[2] - m.position.z) as f32).to_degrees(),
        None => -(m.velocity.x.atan2(m.velocity.z) as f32).to_degrees(),
    };
    m.yaw = yaw;
    m.body_yaw = yaw;
}
/// `Vex` travel with `noPhysics`: straight through blocks, air friction, and
/// `LivingEntity.travelInAir`'s gravity — the effective gravity comes off the
/// moved velocity's y, then the `0.91F`/`0.98F` drags widen from float. While
/// the `NoGravity` of the tick's end stands, `effective_gravity` is 0 and the
/// vex keeps gliding (`Vex.tick` sets the flag only after `super.tick()`).
pub fn travel(m: &mut Mob, w: &super::MobWorld) {
    let from = m.position;
    m.position = m.position.add_vector(m.velocity);
    super::living::section_writes::record_with(
        m.id,
        m.join,
        from,
        m.position,
        w.section_writer.clone(),
    );
    let y = m.velocity.y - m.effective_gravity();
    m.set_velocity(Vec3::new(
        m.velocity.x * f64::from(0.91_f32),
        y * f64::from(0.98_f32),
        m.velocity.z * f64::from(0.91_f32),
    ));
    m.ground.on_ground = false;
}
/// `Vex.tick`: the limited life costs a heart a second once spent.
pub fn ai_step(m: &mut Mob, w: &super::MobWorld, level: &mut DamageLevel<'_>) {
    m.extra.point = m
        .target_info(w)
        .map(|t| [t.position.x, t.position.y, t.position.z]);
    if m.extra.egg_time > 0 {
        m.extra.egg_time -= 1;
        if m.extra.egg_time == 0 {
            m.extra.egg_time = 20;
            m.damage(level, 1.0, Damage::of(DamageKind::Generic));
        }
    }
}
/// `VexModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    p.yrot("head", s.y_rot.to_radians());
    p.xrot("head", s.x_rot.to_radians());
    let bob = (s.age * 5.5 * sunrise_core::geometry::trig::DEG_TO_RAD).mcos() * 0.1;
    p.zrot("right_arm", PI / 5.0 + bob);
    p.zrot("left_arm", -(PI / 5.0 + bob));
    if m.extra.flag {
        p.xrot("body", 0.0);
        // setArmsCharging
        let (right, left) = (
            m.extra.main_hand_id().is_some(),
            m.extra.off_hand_id().is_some(),
        );
        if !right && !left {
            p.xrot("right_arm", -1.221_730_5);
            p.yrot("right_arm", PI / 12.0);
            p.zrot("right_arm", -0.471_238_88 - bob);
            p.xrot("left_arm", -1.221_730_5);
            p.yrot("left_arm", -PI / 12.0);
            p.zrot("left_arm", 0.471_238_88 + bob);
        } else {
            let sword = (std::f64::consts::PI * 7.0 / 6.0) as f32;
            if right {
                p.xrot("right_arm", sword);
                p.yrot("right_arm", PI / 12.0);
                p.zrot("right_arm", -0.471_238_88 - bob);
            }
            if left {
                p.xrot("left_arm", sword);
                p.yrot("left_arm", -PI / 12.0);
                p.zrot("left_arm", 0.471_238_88 + bob);
            }
        }
    } else {
        p.xrot("body", PI / 20.0);
    }
    let deg = sunrise_core::geometry::trig::DEG_TO_RAD;
    let wing = 1.099_557_4 + (s.age * 45.836_624 * deg).mcos() * deg * 16.2;
    p.yrot("left_wing", wing);
    p.yrot("right_wing", -wing);
    p.xrot("left_wing", 0.471_238_88);
    p.zrot("left_wing", -0.471_238_88);
    p.xrot("right_wing", 0.471_238_88);
    p.zrot("right_wing", 0.471_238_88);
}
/// The vex's texture: brighter while charging.
pub fn texture(m: &Mob) -> &'static str {
    if m.extra.flag {
        "entity/illager/vex_charging"
    } else {
        "entity/illager/vex"
    }
}

/// `Vex.addAdditionalSaveData`: the bound origin, the limited life and the
/// owner reference's UUID (`EntityReference.store`, left out when there is no
/// owner). The level answers for an owner still in it; a reference the vex was
/// loaded with stands when the owner is gone.
pub fn write_nbt(
    m: &Mob,
    map: &mut crate::entity_nbt::EntityTagWriter,
    uuid_of: &mut impl FnMut(u32) -> Option<[i32; 4]>,
) {
    use sunrise_core::nbt::Tag;
    if let Some(h) = m.extra.home {
        map.insert("bound_pos".into(), Tag::IntArray(h.to_vec()));
    }
    if m.extra.egg_time > 0 {
        map.insert("life_ticks".into(), Tag::Int(m.extra.egg_time));
    }
    let resolved = if m.extra.counter2 != 0 {
        crate::entity_nbt::who_uuid(Who::Mob(m.extra.counter2 as u32), uuid_of)
    } else {
        None
    };
    match resolved.or(m.extra.owner) {
        Some(uuid) => {
            map.insert("owner".into(), Tag::IntArray(uuid.to_vec()));
        }
        None => {
            map.remove(&"owner".into());
        }
    }
}

/// `Vex.readAdditionalSaveData`: the bound origin, the life left (-1 when
/// absent) and the owner's UUID, kept as the UUID it is - an id would name
/// whatever mob has it now - and resolved through the level when the vex asks
/// for its owner (`owner`).
pub fn read_nbt(m: &mut Mob, tag: &sunrise_core::nbt::Tag) {
    use sunrise_core::nbt::Tag;
    if let Some(Tag::IntArray(v)) = tag.get("bound_pos")
        && v.len() == 3
    {
        m.extra.home = Some([v[0], v[1], v[2]]);
    }
    m.extra.egg_time = crate::save::number(tag.get("life_ticks")).map_or(-1, |v| v as i32);
    m.extra.owner = crate::save::uuid_from_tag(tag.get("owner"));
}

#[cfg(test)]
mod owner_save_tests {
    use super::*;
    use crate::entity_nbt::{mob_from_nbt, mob_to_nbt};
    use crate::mobs::WhoLookup;
    use std::collections::HashMap;
    use std::sync::Arc;
    use sunrise_core::nbt::Tag;

    /// A vex with the id `EVOKER` in the level: what `Vex.setOwner` records
    /// when an evoker summons it.
    const EVOKER: u32 = 7;

    fn vex() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:vex".into();
        ty.species = Species::Vex;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// The `Vex` constructor leaves `DATA_NO_GRAVITY` at its default, false:
    /// only the end of `Vex.tick` sets it.
    #[test]
    fn a_new_vex_has_gravity_until_its_first_tick() {
        assert!(!vex().no_gravity);
    }

    /// The first tick of a vex with AI (`travel` runs only under
    /// `isEffectiveAi`): the constructor's gravity is still on, so the
    /// travel step turns a standing vex's `Motion[1]` into
    /// `(0 - 0.08) * 0.98F` without moving it yet, and the tick's end (which
    /// is where vanilla's `Vex.tick` sets the flag after `super.tick()`)
    /// saves `NoGravity:1b`. A vex summoned with `NoGravity:1b` — the
    /// gravity-disabled control — keeps `Motion[1]` at 0. The travel pin is
    /// on a vex with no goal drift yet (its goals may draw a drift target on
    /// a first tick, as vanilla's 1-in-4 `VexRandomMoveGoal` can), which is
    /// the stationary vex the fix is about.
    #[test]
    #[ignore = "daily"]
    fn a_vexs_first_tick_falls_and_then_saves_no_gravity() {
        use crate::mobs::world::with_test_world;
        with_test_world(&[], |w, _| {
            for with_gravity in [true, false] {
                let mut vex = vex();
                register(&mut vex);
                vex.no_gravity = !with_gravity;
                travel(&mut vex, w);
                if with_gravity {
                    // The moved velocity was 0, so the first tick has not
                    // moved it yet, and `travelInAir` leaves the gravity
                    // step in the float-widened drag.
                    assert_eq!(vex.position, Vec3::ZERO, "no motion yet");
                    assert_eq!(
                        vex.velocity.y.to_bits(),
                        (-0.08_f64 * f64::from(0.98_f32)).to_bits(),
                        "the first tick's Motion[1]"
                    );
                } else {
                    assert_eq!(vex.velocity.y, 0.0, "NoGravity vex never falls");
                }
            }
            // One whole tick saves the flag: the vex's own ai tick, whose
            // end is `Vex.tick`'s setNoGravity(true) (AI or not).
            let mut vex = vex();
            register(&mut vex);
            vex.persistent = true;
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            crate::mobs::ai::tick(&mut vex, w, &mut test_level.level(), &mut events);
            assert!(vex.no_gravity, "the tick's end set the flag");
            let saved = mob_to_nbt(&vex, [0; 4], None, None, |_| None, &Default::default());
            assert_eq!(saved.get("NoGravity"), Some(&Tag::Byte(1)));
        });
    }

    /// `Level.getEntity(uuid)` for the test: `Game` answers this with
    /// `Game::who_by_uuid`.
    struct Level(HashMap<[i32; 4], Who>);
    impl WhoLookup for Level {
        fn who_by_uuid(&self, uuid: [i32; 4]) -> Option<Who> {
            self.0.get(&uuid).copied()
        }

        fn mob_uuid(&self, id: u32) -> Option<[i32; 4]> {
            self.0
                .iter()
                .find_map(|(&uuid, &who)| (who == Who::Mob(id)).then_some(uuid))
        }
    }

    /// `Vex.addAdditionalSaveData` stores its owner reference's UUID
    /// (`EntityReference.store`, nothing at all without an owner) and
    /// `readAdditionalSaveData` reads it back (`EntityReference.read`), so a
    /// summoned vex keeps the evoker whose target it copies across a chunk
    /// save. Sunrise models the owner as the owner's mob id and wrote neither,
    /// so a reloaded vex had no owner to copy a target from.
    #[test]
    fn a_vexs_owner_uuid_survives_the_chunk_save() {
        let evoker_uuid = [3, 1, 4, 1];
        let mut summoned = vex();
        assert_eq!(owner(&summoned, None), None, "a vex nobody summoned");
        summoned.extra.counter2 = EVOKER as i32;
        assert_eq!(owner(&summoned, None), Some(EVOKER), "the evoker's id");
        let uuid_of = |id: u32| (id == EVOKER).then_some(evoker_uuid);
        let saved = mob_to_nbt(&summoned, [0; 4], None, None, uuid_of, &Default::default());
        assert_eq!(
            saved.get("owner"),
            Some(&Tag::IntArray(evoker_uuid.to_vec()))
        );
        // The loaded vex knows the UUID: no id survives a save, and its owner
        // is found again only through the level (`EntityReference.getEntity`).
        let types = [("minecraft:vex".into(), summoned.kind.clone())].into();
        let back = mob_from_nbt(&saved, &types, 2, None).unwrap();
        assert_eq!(back.extra.owner, Some(evoker_uuid));
        assert_eq!(back.extra.counter2, 0);
        let level = Level(HashMap::from([(evoker_uuid, Who::Mob(42))]));
        assert_eq!(owner(&back, Some(&level)), Some(42), "the evoker, found");
        // An owner the level does not have, and a UUID naming the player, are
        // no owner: `Vex.setOwner` takes a `Mob`.
        assert_eq!(owner(&back, Some(&Level(HashMap::new()))), None);
        let player = Level(HashMap::from([(evoker_uuid, Who::Player)]));
        assert_eq!(owner(&back, Some(&player)), None);
        assert_eq!(owner(&back, None), None, "a level with no UUIDs");
        // A save read again keeps the reference even when nothing in the level
        // answers for it, as `EntityReference` does.
        let again = mob_to_nbt(&back, [0; 4], None, None, |_| None, &Default::default());
        assert_eq!(
            again.get("owner"),
            Some(&Tag::IntArray(evoker_uuid.to_vec()))
        );
    }

    /// `EntityReference.store` writes nothing for a vex with no owner, and a
    /// save that names none loads with none - neither the `owner` key a loaded
    /// compound carried nor an id left over may stand.
    #[test]
    fn a_vex_with_no_owner_saves_no_owner_key() {
        let mut loose = vex();
        let stale = Tag::compound([
            ("id", Tag::String("minecraft:vex".into())),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(0.5_f64.to_bits()),
                    Tag::Double(64.0_f64.to_bits()),
                    Tag::Double(0.5_f64.to_bits()),
                ]),
            ),
            ("owner", Tag::IntArray(vec![9, 9, 9, 9])),
        ]);
        let types = [("minecraft:vex".into(), loose.kind.clone())].into();
        let loaded = mob_from_nbt(&stale, &types, 3, None).unwrap();
        assert_eq!(loaded.extra.owner, Some([9, 9, 9, 9]));
        loose.extra.owner = None;
        loose.extra.counter2 = 0;
        let saved = mob_to_nbt(
            &loose,
            [0; 4],
            Some(&stale),
            None,
            |_| None,
            &Default::default(),
        );
        assert_eq!(saved.get("owner"), None, "no owner, no key");
    }
    /// `VexCopyOwnerTargetGoal.canUse` runs the owner's target through
    /// `TargetingConditions.forNonCombat()`, whose `canBeSeenByAnyone` drops a
    /// dead target. A dead target is not copied.
    #[test]
    #[ignore = "daily"]
    fn a_vex_does_not_copy_a_dead_owners_target() {
        use crate::mobs::world::with_test_world;
        let mut outcomes = Vec::new();
        if !with_test_world(&[], |w, _| {
            for alive in [true, false] {
                let mut owner_kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
                owner_kind.species = Species::Evoker;
                let mut owner = Mob::new(2, Arc::new(owner_kind), Vec3::new(1.5, 64.0, 0.5), 0.0);
                owner.ai.target = Some(Who::Mob(3));
                let mut target_kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
                target_kind.species = Species::Cow;
                let mut target = Mob::new(3, Arc::new(target_kind), Vec3::new(2.5, 64.0, 0.5), 0.0);
                target.health = if alive { target.health } else { 0.0 };
                let seen = [
                    owner.seen(None, &crate::items::Items::empty()),
                    target.seen(None, &crate::items::Items::empty()),
                ];
                let world = crate::mobs::MobWorld {
                    side: crate::motion::dry_side(),
                    others: &seen,
                    ..w.clone()
                };
                let mut vex = vex();
                vex.position = Vec3::new(0.5, 64.0, 0.5);
                vex.extra.counter2 = 2;
                vex.ai.targets.add(2, CopyOwnerTarget);
                let mut lr = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut cx = Cx {
                    w: &world,
                    level: &mut lr.level(),
                    events: &mut events,
                };
                let mut goals = std::mem::take(&mut vex.ai.targets);
                goals.tick(&mut vex, &mut cx);
                outcomes.push(goals.is_running("CopyOwnerTarget"));
                vex.ai.targets = goals;
            }
        }) {
            return;
        }
        assert_eq!(
            outcomes,
            vec![true, false],
            "an alive target is copied; a dead one is not"
        );
    }
}
