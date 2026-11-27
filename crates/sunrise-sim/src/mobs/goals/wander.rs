// Included in goals.rs to retain its private scope.
/// FloatGoal: jump (swim up) while deeper than the jump threshold or in lava.
#[derive(Clone, Debug)]
pub struct Float;
impl Goal for Float {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        JUMP
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.floats()
    }
    #[inline]
    fn every_tick(&self) -> bool {
        true
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        if m.random.next_float() < 0.8 {
            m.ai.jump = true;
        }
    }
}
/// `ClimbOnTopOfPowderSnowGoal`: a powder-snow walker standing in powder snow
/// jumps while the block above is powder snow or has no collision.
#[derive(Clone, Debug)]
pub struct ClimbOnTopOfPowderSnow;
impl Goal for ClimbOnTopOfPowderSnow {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        JUMP
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // wasInPowderSnow || isInPowderSnow: the freeze tick has read the
        // powder snow the mob's box touches before the goals run.
        if !super::ai::in_powder_snow(m, cx.w)
            || !cx
                .w
                .entity_tag_bit(&m.kind, crate::items::EntityTypeTags::POWDER_SNOW_WALKABLE)
        {
            return false;
        }
        // Mob.blockPosition().above()
        let above = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32 + 1,
            m.position.z.floor() as i32,
        ];
        cx.w.block_name(above[0], above[1], above[2]) == "minecraft:powder_snow"
            || cx.w.collision_bounds(above).is_none()
    }
    #[inline]
    fn every_tick(&self) -> bool {
        true
    }
    /// `ClimbOnTopOfPowderSnowGoal.tick`: ask the mob to jump.
    #[inline]
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.jump = true;
    }
}
/// PanicGoal: flee to a random position after panic-causing damage (or
/// towards water while burning).
#[derive(Clone, Debug)]
pub struct Panic {
    pub speed: f64,
    /// Only the damage of `#panic_environmental_causes` (an armadillo).
    pub environmental: bool,
    /// `TurtlePanicGoal`: always look for water this far away, not only
    /// when on fire.
    pub water_range: Option<i32>,
    pos: [f64; 3],
}
impl Panic {
    /// The position `canUse` chose (`PanicGoal.posX/Y/Z`).
    pub fn target(&self) -> [f64; 3] {
        self.pos
    }
    pub fn new(speed: f64) -> Self {
        Self {
            speed,
            environmental: false,
            water_range: None,
            pos: [0.0; 3],
        }
    }
    /// `TurtlePanicGoal`.
    pub fn looking_for_water(mut self, range: i32) -> Self {
        self.water_range = Some(range);
        self
    }
}
pub fn should_panic(m: &Mob) -> bool {
    m.last_damage
        .is_some_and(|(d, t)| d.causes_panic() && i64::from(m.age) - t <= 40)
        || m.is_on_fire()
}
impl Goal for Panic {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let wanted = if self.environmental {
            m.last_damage.is_some_and(|(d, t)| {
                super::armadillo::environmental(d.kind) && i64::from(m.age) - t <= 40
            }) || m.is_on_fire()
        } else {
            should_panic(m)
        };
        if !wanted {
            return false;
        }
        let range = self.water_range.or((m.fire_ticks > 0).then_some(5));
        if let Some(range) = range {
            let (bx, by, bz) = (
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            );
            // lookForWater: only when `getCollisionShape(level, pos).isEmpty()`. A state that
            // takes its shapes from its block entity asks for the published one (#1710): a
            // box ending at the block's bottom is not empty though its top is 0. Vanilla's
            // query cannot fail; a failed one here is not taken for empty, so the goal goes
            // on to a random position as over any other shape.
            let state = cx.w.world.block(bx, by, bz);
            let empty = if cx.w.collision.publishes_shapes(state) {
                cx.w.bound_collision_shape(state, [bx, by, bz])
                    .is_some_and(|shape| shape.is_empty())
            } else {
                cx.w.data.state(state).top == 0.0
            };
            if empty {
                // findBlocksInBoxByManhattanDistance(pos, range, 1) for water.
                let mut best: Option<([i32; 3], i32)> = None;
                for dy in -1i32..=1 {
                    for dx in -range..=range {
                        for dz in -range..=range {
                            let d = dx.abs() + dy.abs() + dz.abs();
                            if cx.w.fluid(bx + dx, by + dy, bz + dz) == crate::blocks::Fluid::Water
                                && best.is_none_or(|b| d < b.1)
                            {
                                best = Some(([bx + dx, by + dy, bz + dz], d));
                            }
                        }
                    }
                }
                if let Some((p, _)) = best {
                    self.pos = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
                    return true;
                }
            }
        }
        match m.default_random_pos(cx.w, 5, 4) {
            Some(p) => {
                self.pos = p;
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
        m.nav_move_to(cx.w, self.pos, self.speed);
        m.ai.panicking = true;
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.panicking = false;
    }
}
/// RandomStrollGoal / WaterAvoidingRandomStrollGoal.
#[derive(Clone, Debug)]
pub struct Stroll {
    pub speed: f64,
    pub interval: i32,
    pub check_no_action: bool,
    /// WaterAvoidingRandomStrollGoal probability of a plain stroll.
    pub water_avoiding: Option<f32>,
    /// RandomSwimmingGoal: BehaviorUtils.getRandomSwimmablePos.
    pub swim: bool,
    /// WaterAvoidingRandomFlyingGoal: a hover spot, else an air or water pos.
    pub flying: bool,
    /// `Parrot.ParrotWanderGoal`: to a tree, else as `flying`.
    pub parrot: bool,
    pub force: bool,
    wanted: [f64; 3],
    /// A subclass's `getPosition` in place of the default pick
    /// (`GolemRandomStrollInVillageGoal`, `MoveBackToVillageGoal`).
    position: Option<fn(&mut Mob, &mut Cx) -> Option<[f64; 3]>>,
    /// A subclass's own `canUse` test, run before the interval's draw.
    gate: Option<fn(&Mob, &MobWorld) -> bool>,
    label: Option<&'static str>,
}
impl Stroll {
    /// `RandomStrollGoal.<init>` (interval 120, checks the no-action time).
    pub fn new(speed: f64) -> Self {
        Self {
            speed,
            interval: 120,
            check_no_action: true,
            water_avoiding: None,
            swim: false,
            flying: false,
            parrot: false,
            force: false,
            wanted: [0.0; 3],
            position: None,
            gate: None,
            label: None,
        }
    }
    /// The stroll's spot comes from `f` (an overridden `getPosition`).
    pub fn position(mut self, f: fn(&mut Mob, &mut Cx) -> Option<[f64; 3]>) -> Self {
        self.position = Some(f);
        self
    }
    /// The stroll may only start while `f` holds (an overridden `canUse`).
    pub fn gate(mut self, f: fn(&Mob, &MobWorld) -> bool) -> Self {
        self.gate = Some(f);
        self
    }
    /// The goal's class name, for debugging.
    pub fn named(mut self, label: &'static str) -> Self {
        self.label = Some(label);
        self
    }
    pub fn avoiding_water(speed: f64) -> Self {
        Self {
            water_avoiding: Some(0.001),
            ..Self::new(speed)
        }
    }
    pub fn interval(mut self, i: i32) -> Self {
        self.interval = i;
        self
    }
    /// WaterAvoidingRandomFlyingGoal (a nearby air position).
    pub fn flying(speed: f64) -> Self {
        Self {
            flying: true,
            ..Self::new(speed)
        }
    }
    /// `Parrot.ParrotWanderGoal(speed)`, a WaterAvoidingRandomFlyingGoal.
    pub fn parrot(speed: f64) -> Self {
        Self {
            water_avoiding: Some(0.001),
            parrot: true,
            ..Self::new(speed)
        }
    }
    /// RandomSwimmingGoal(speed, interval)
    pub fn swimming(speed: f64, interval: i32) -> Self {
        Self {
            swim: true,
            ..Self::new(speed).interval(interval)
        }
    }
}
impl Goal for Stroll {
    goal_clone!();
    #[inline]
    fn name(&self) -> &'static str {
        if let Some(label) = self.label {
            label
        } else if self.parrot {
            "ParrotWander"
        } else if self.water_avoiding.is_some() {
            "Stroll"
        } else if self.swim {
            "RandomSwimming"
        } else {
            "RandomStroll"
        }
    }
    #[inline]
    fn flags(&self) -> u8 {
        MOVE
    }
    /// RandomStrollGoal.canUse.
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if super::riders::mob_controlled(m, cx.w) {
            return false;
        }
        if self.gate.is_some_and(|gate| !gate(m, cx.w)) {
            return false;
        }
        if !self.force {
            if self.check_no_action && m.ai.no_action_time >= 100 {
                return false;
            }
            if m.random.next_int_bound(reduced(self.interval)).unwrap_or(1) != 0 {
                return false;
            }
        }
        let pos = match self.water_avoiding {
            _ if self.position.is_some() => self.position.and_then(|pick| pick(m, cx)),
            Some(_) if self.parrot => super::parrot::wander_pos(m, cx.w),
            Some(prob) => {
                if m.fluids.in_water {
                    m.land_random_pos(cx.w, 15, 7)
                        .or_else(|| m.default_random_pos(cx.w, 10, 7))
                } else if m.random.next_float() >= prob {
                    m.land_random_pos(cx.w, 10, 7)
                } else {
                    m.default_random_pos(cx.w, 10, 7)
                }
            }
            None if self.swim => {
                let mut p = m.default_random_pos(cx.w, 10, 7);
                let mut count = 0;
                while let Some(q) = p {
                    let s = cx.w.world.block(
                        q[0].floor() as i32,
                        q[1].floor() as i32,
                        q[2].floor() as i32,
                    );
                    if cx
                        .w
                        .data
                        .pathfindable(s, super::data::PathComputationType::Water)
                        || count >= 10
                    {
                        break;
                    }
                    count += 1;
                    p = m.default_random_pos(cx.w, 10, 7);
                }
                p
            }
            None if self.flying => m.flying_stroll_pos(cx.w),
            None => m.default_random_pos(cx.w, 10, 7),
        };
        let Some(p) = pos else { return false };
        self.wanted = p;
        self.force = false;
        true
    }
    /// RandomStrollGoal.canContinueToUse.
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !m.ai.nav.is_done() && !super::riders::mob_controlled(m, cx.w)
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.nav_move_to(cx.w, self.wanted, self.speed);
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
}
/// LookAtPlayerGoal (player targets only).
#[derive(Clone, Debug)]
pub struct LookAtPlayer {
    pub range: f32,
    pub probability: f32,
    pub only_horizontal: bool,
    /// `LookAtPlayerGoal(Mob.class, ...)`: the nearest mob, not the player.
    pub mobs: bool,
    /// `LookAtPlayerGoal(Strider.class, ...)`: only mobs of this species.
    pub only: Option<Species>,
    /// InteractGoal: LOOK and MOVE.
    pub interact: bool,
    look_at: Option<Who>,
    look_time: i32,
}
impl LookAtPlayer {
    pub fn new(range: f32) -> Self {
        Self {
            range,
            probability: 0.02,
            only_horizontal: false,
            mobs: false,
            only: None,
            interact: false,
            look_at: None,
            look_time: 0,
        }
    }
    /// `LookAtPlayerGoal(this, Mob.class, range)`.
    pub fn mobs(range: f32) -> Self {
        Self {
            mobs: true,
            ..Self::new(range)
        }
    }
    /// `LookAtPlayerGoal(this, <species class>, range)`.
    pub fn species(species: Species, range: f32) -> Self {
        Self {
            only: Some(species),
            ..Self::mobs(range)
        }
    }
    /// `InteractGoal(this, Player.class, range, probability)`.
    pub fn interact(range: f32, probability: f32) -> Self {
        Self {
            interact: true,
            ..Self::new(range).probability(probability)
        }
    }
    pub fn probability(mut self, p: f32) -> Self {
        self.probability = p;
        self
    }
    /// The goal's look target, alive, with its position and eye height.
    fn look_target(&self, w: &MobWorld) -> Option<(Vec3, f64)> {
        match self.look_at? {
            Who::Player => w
                .selected_player()
                .filter(|p| p.alive)
                .map(|p| (p.position, p.eye[1])),
            Who::Mob(id) => w
                .seen(id)
                .filter(|s| s.alive)
                .map(|s| (s.position, s.eye_y)),
        }
    }
}
impl Goal for LookAtPlayer {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        if self.interact { LOOK | MOVE } else { LOOK }
    }
    #[inline]
    fn name(&self) -> &'static str {
        if self.interact {
            "InteractGoal"
        } else {
            "LookAtPlayer"
        }
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.random.next_float() >= self.probability {
            return false;
        }
        self.look_at = None;
        let range = f64::from(self.range);
        if !self.mobs {
            // level.getNearestPlayer(forNonCombat().range(lookDistance)).
            if m.can_see_player(cx.w, range) {
                self.look_at = Some(Who::Player);
            }
            return self.look_at.is_some();
        }
        // level.getNearestEntity(mobs in the box inflated by (range, 3, range)).
        let area = m.bounds().inflate(Vec3::new(range, 3.0, range));
        let (id, only) = (m.id, self.only);
        let at = Vec3::new(m.position.x, m.eye_y(), m.position.z);
        let candidates: Vec<(u32, Vec3)> =
            cx.w.others_near(area)
                .filter(|s| s.id != id && s.alive && s.bounds.intersects(area))
                .filter(|s| only.is_none_or(|o| s.species == o))
                .map(|s| (s.id, s.position))
                .collect();
        let mut best: Option<(u32, f64)> = None;
        for (cid, pos) in candidates {
            let reach = range.max(2.0);
            if m.distance_sqr(pos) > reach * reach || !m.sees(cx.w, Who::Mob(cid)) {
                continue;
            }
            let d = pos.distance_squared(at);
            if best.is_none_or(|(_, b)| d < b) {
                best = Some((cid, d));
            }
        }
        self.look_at = best.map(|(cid, _)| Who::Mob(cid));
        self.look_at.is_some()
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some((pos, _)) = self.look_target(cx.w) else {
            return false;
        };
        m.distance_sqr(pos) <= f64::from(self.range * self.range) && self.look_time > 0
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        self.look_time = reduced(40 + m.random.next_int_bound(40).unwrap_or(0));
    }
    #[inline]
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.look_at = None;
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some((pos, eye)) = self.look_target(cx.w) {
            let y = if self.only_horizontal { m.eye_y() } else { eye };
            m.look_at([pos.x, y, pos.z]);
            self.look_time -= 1;
        }
    }
}
/// RandomLookAroundGoal.
#[derive(Clone, Debug, Default)]
pub struct RandomLookAround {
    rel: [f64; 2],
    time: i32,
}
impl Goal for RandomLookAround {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.random.next_float() < 0.02
    }
    #[inline]
    fn can_continue(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        self.time >= 0
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        let r = std::f64::consts::TAU * m.random.next_double();
        self.rel = [jmath::cos(r), jmath::sin(r)];
        self.time = 20 + m.random.next_int_bound(20).unwrap_or(0);
    }
    #[inline]
    fn every_tick(&self) -> bool {
        true
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        self.time -= 1;
        let at = [
            m.position.x + self.rel[0],
            m.eye_y(),
            m.position.z + self.rel[1],
        ];
        m.look_at(at);
    }
}
