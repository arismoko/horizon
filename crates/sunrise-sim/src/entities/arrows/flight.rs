// Included in arrows.rs to retain its private scope.
/// Reusable scratch buffer for `AbstractArrow.findHitEntities` sorting.
pub type ArrowHitScratch = Vec<(Who, Vec3, f64, TargetOrder)>;

/// Live entity targets queried by `AbstractArrow` without allocating a snapshot vector.
pub trait ArrowTargets {
    fn visit(&self, visit: impl FnMut(TargetCandidate));
    fn owner_tree_contains(&self, owner: Who, candidate: Who) -> bool {
        candidate == owner
    }
    fn outside_owner_collision_range(&self, owner: Who, swept: Aabb) -> bool;
}

struct SliceTargets<'a>(&'a [(Who, Aabb)]);

impl ArrowTargets for SliceTargets<'_> {
    fn visit(&self, mut visit: impl FnMut(TargetCandidate)) {
        for (idx, &(who, bounds)) in self.0.iter().enumerate() {
            let c = bounds.center();
            let position = Vec3::new(c.x, bounds.min_position().y, c.z);
            visit(TargetCandidate {
                who,
                bounds,
                position,
                order: TargetOrder::entity(position, idx as u64),
            });
        }
    }
    fn outside_owner_collision_range(&self, owner: Who, swept: Aabb) -> bool {
        !self
            .0
            .iter()
            .any(|&(who, b)| who == owner && b.intersects(swept))
    }
}

/// One arrow tick: whether to keep it and what it hit. An arrow that hit an
/// entity stops there and is not kept: the game hands it to
/// `finish_entity_hit` once it has hurt the entity.
pub fn tick(p: &mut Projectile, scene: &ArrowScene, targets: &[(Who, Aabb)]) -> (bool, Hit) {
    tick_to_block_contact(p, scene, targets, false)
}

/// With block impact listeners, stop after moving to the captured contact and
/// applying the flight's block effects. The caller applies the hit response, if
/// accepted, then runs the remainder of the tick.
pub fn tick_to_block_contact(
    p: &mut Projectile,
    scene: &ArrowScene,
    targets: &[(Who, Aabb)],
    defer_block_hit: bool,
) -> (bool, Hit) {
    let mut scratch = ArrowHitScratch::new();
    tick_with(
        p,
        scene,
        &SliceTargets(targets),
        true,
        false,
        defer_block_hit,
        &mut scratch,
    )
}

/// Zero-allocation live-target entry for `AbstractArrow.tick`.
pub fn tick_to_block_contact_live(
    p: &mut Projectile,
    scene: &ArrowScene,
    targets: &impl ArrowTargets,
    defer_block_hit: bool,
    scratch: &mut ArrowHitScratch,
) -> (bool, Hit) {
    tick_with(p, scene, targets, true, false, defer_block_hit, scratch)
}

/// `AbstractArrow.onHitBlock` for an accepted block contact.
pub fn apply_block_contact(p: &mut Projectile, scene: &ArrowScene, block: BlockPos) {
    let Some(mut st) = p.arrow.take() else {
        return;
    };
    Ticker {
        p,
        st: &mut st,
        scene,
        client: false,
    }
    .on_hit_block(block, scene.block(block));
    p.stuck = st.in_ground;
    p.arrow = Some(st);
}

/// Run drag, gravity and the projectile base tick after a block contact. Returns
/// false if the arrow expired. The caller publishes `PROJECTILE_LAND` first
/// when the contact was accepted.
pub fn finish_block_contact(p: &mut Projectile, scene: &ArrowScene) -> bool {
    let Some(mut st) = p.arrow.take() else {
        return false;
    };
    Ticker {
        p,
        st: &mut st,
        scene,
        client: false,
    }
    .finish();
    p.stuck = st.in_ground;
    let keep = !st.removed;
    p.arrow = Some(st);
    keep
}

/// One arrow tick whose hit is canceled (`ProjectileImpactEvent`): the arrow
/// flies through its full movement: not stuck, nothing hurt, no hit.
pub fn tick_through(
    p: &mut Projectile,
    scene: &ArrowScene,
    targets: &[(Who, Aabb)],
) -> (bool, Hit) {
    let mut scratch = ArrowHitScratch::new();
    tick_with(
        p,
        scene,
        &SliceTargets(targets),
        false,
        false,
        false,
        &mut scratch,
    )
}
/// Live-target variant of [`tick_through`].
pub fn tick_through_live(
    p: &mut Projectile,
    scene: &ArrowScene,
    targets: &impl ArrowTargets,
    scratch: &mut ArrowHitScratch,
) -> (bool, Hit) {
    tick_with(p, scene, targets, false, false, false, scratch)
}
/// Simulate a remote arrow without server-only despawn or block reactions.
/// Entity hits remain authoritative on the server.
pub fn tick_client(p: &mut Projectile, scene: &ArrowScene) -> (bool, Hit) {
    let mut scratch = ArrowHitScratch::new();
    let out = tick_with(
        p,
        scene,
        &SliceTargets(&[]),
        true,
        true,
        false,
        &mut scratch,
    );
    // `Entity.playSound` passes no player to `Level.playSound`, and
    // `ClientLevel.playSeededSound` plays only for its own player: the server's
    // sound packets carry the hit and splash. The sounds' draws still happened.
    if let Some(a) = p.arrow.as_mut() {
        a.events.retain(|e| !matches!(e, ArrowEvent::Sound { .. }));
    }
    out
}
fn tick_with(
    p: &mut Projectile,
    scene: &ArrowScene,
    targets: &impl ArrowTargets,
    impact: bool,
    client: bool,
    defer_block_hit: bool,
    scratch: &mut ArrowHitScratch,
) -> (bool, Hit) {
    let Some(mut st) = p.arrow.take() else {
        return (false, Hit::None);
    };
    // Entity.setOldPosAndRot: the renderer interpolates from here, in flight
    // and stuck alike; ServerLevel.tickNonPassenger counts the tick.
    p.set_old_pos_and_rot();
    p.age += 1;
    let hit = Ticker {
        p,
        st: &mut st,
        scene,
        client,
    }
    .tick(targets, impact, defer_block_hit, scratch);
    p.stuck = st.in_ground;
    let keep = !st.removed && !matches!(hit, Hit::Entity { .. });
    p.arrow = Some(st);
    (keep, hit)
}
/// What became of the entity an arrow hit (`AbstractArrow.onHitEntity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityHit {
    /// `hurtOrSimulate` succeeded: the arrow is spent, with its hit sound.
    Hurt,
    /// It failed (a raised shield, invulnerability): the arrow turns back.
    Refused,
    /// The entity turned it first (`Entity.deflection`: breezes).
    Deflected,
    /// The target teleported away from it (`EnderMan.hurtServer` returns true,
    /// `onHitEntity` returns before the hit sound and the discard): the arrow
    /// flies on and falls to the ground.
    Dodged,
}
/// `ProjectileDeflection.REVERSE` with power `0.2`: each axis of the movement
/// is multiplied by `-0.2 * 0.5` (`-0.1`), and the yaw turns `170 + nextFloat * 20`
/// degrees. No hit sound. One left almost still (`lengthSqr < 1e-7`) drops its
/// pickup when that is ALLOWED, then is discarded.
pub fn deflect_reverse(
    velocity: &mut Vec3,
    yaw: &mut f32,
    st: &mut ArrowState,
    random: &mut impl RandomSource,
) {
    let power = 0.2;
    let rotation = 170.0 + random.next_float() * 20.0;
    let v = *velocity;
    let k = -power * 0.5;
    *velocity = Vec3::new(v.x * k, v.y * k, v.z * k);
    *yaw += rotation;
    if velocity.length_squared() < 1.0e-7 {
        if st.pickup == Pickup::Allowed {
            st.events.push(ArrowEvent::Drop);
        }
        st.removed = true;
    }
}
/// The rest of the tick of an arrow that hit an entity: whether to keep it.
pub fn finish_entity_hit(p: &mut Projectile, scene: &ArrowScene, outcome: EntityHit) -> bool {
    let Some(mut st) = p.arrow.take() else {
        return false;
    };
    let trident = matches!(p.kind, ProjectileKind::Trident);
    let mut ticker = Ticker {
        p,
        st: &mut st,
        scene,
        client: false,
    };
    match outcome {
        EntityHit::Hurt | EntityHit::Refused if trident => ticker.trident_bounce(),
        EntityHit::Hurt => {
            ticker.hit_sound();
            // A piercing arrow flies on (`getPierceLevel() <= 0` discards).
            if (ticker.st.pierce as i8) <= 0 {
                ticker.st.removed = true;
            }
        }
        EntityHit::Refused => ticker.turn_back(),
        EntityHit::Dodged => {
            if trident {
                ticker.st.dealt_damage = true;
            }
        }
        EntityHit::Deflected => {}
    }
    ticker.finish();
    p.stuck = st.in_ground;
    let keep = !st.removed;
    p.arrow = Some(st);
    keep
}

/// `AbstractArrow.onHitEntity` pierce limit: records `who` or discards past `pierce + 1`.
pub fn record_pierce_hit(p: &mut Projectile, who: Who) -> bool {
    let Some(a) = p.arrow.as_mut() else {
        return true;
    };
    if (a.pierce as i8) <= 0 {
        return true;
    }
    if a.pierced.len() > usize::from(a.pierce) {
        a.removed = true;
        return false;
    }
    a.pierced.push(who);
    true
}

/// Apply one entity hit's side effects inside `AbstractArrow.hitTargetsOrDeflectSelf`.
pub fn apply_pierce_hit(p: &mut Projectile, outcome: EntityHit) {
    let Some(mut st) = p.arrow.take() else {
        return;
    };
    match outcome {
        EntityHit::Hurt => {
            let pitch = 1.2 / (p.entity_random().next_float() * 0.2 + 0.9);
            let sound = if matches!(p.kind, ProjectileKind::Trident) {
                ArrowSound::TridentHitGround
            } else if st.crossbow_hit_sound {
                ArrowSound::CrossbowHit
            } else {
                ArrowSound::HitGround
            };
            st.events.push(ArrowEvent::Sound {
                sound,
                volume: 1.0,
                pitch,
            });
            if (st.pierce as i8) <= 0 {
                st.removed = true;
            }
        }
        EntityHit::Refused => {
            deflect_reverse(&mut p.velocity, &mut p.yaw, &mut st, &mut p.random.0);
        }
        EntityHit::Dodged | EntityHit::Deflected => {}
    }
    p.arrow = Some(st);
}

/// Next iteration of `AbstractArrow.stepMoveAndHit` for a piercing arrow.
pub fn step_pierce_next(
    p: &mut Projectile,
    scene: &ArrowScene,
    targets: &[(Who, Aabb)],
    end: Vec3,
    block: Option<(Vec3, BlockPos, [i32; 3])>,
) -> (Hit, Vec<(Who, Vec3)>) {
    let mut scratch = ArrowHitScratch::new();
    step_pierce_next_live(
        p,
        scene,
        &SliceTargets(targets),
        end,
        block,
        false,
        &mut scratch,
    )
}

/// Live-target continuation sweep of `AbstractArrow.stepMoveAndHit`.
pub fn step_pierce_next_live(
    p: &mut Projectile,
    scene: &ArrowScene,
    targets: &impl ArrowTargets,
    end: Vec3,
    block: Option<(Vec3, BlockPos, [i32; 3])>,
    defer_block_hit: bool,
    scratch: &mut ArrowHitScratch,
) -> (Hit, Vec<(Who, Vec3)>) {
    let Some(mut st) = p.arrow.take() else {
        return (Hit::None, Vec::new());
    };
    let out = {
        let mut ticker = Ticker {
            p,
            st: &mut st,
            scene,
            client: false,
        };
        let from = ticker.position();
        ticker.step_once(from, end, block, targets, defer_block_hit, scratch)
    };
    p.stuck = st.in_ground;
    p.arrow = Some(st);
    out
}

/// Run drag, gravity, fluids, fire, and shoot event once after a piercing arrow's `stepMoveAndHit`.
pub fn finish_pierce_tick(p: &mut Projectile, scene: &ArrowScene) -> bool {
    let Some(mut st) = p.arrow.take() else {
        return false;
    };
    {
        let mut ticker = Ticker {
            p,
            st: &mut st,
            scene,
            client: false,
        };
        ticker.finish();
    }
    p.stuck = st.in_ground;
    let keep = !st.removed;
    p.arrow = Some(st);
    keep
}
struct Ticker<'p, 's, 'a, 'r> {
    p: &'p mut Projectile,
    st: &'s mut ArrowState,
    scene: &'s ArrowScene<'a, 'r>,
    client: bool,
}
impl Ticker<'_, '_, '_, '_> {
    fn position(&self) -> Vec3 {
        self.p.position
    }
    fn block_position(&self) -> BlockPos {
        BlockPos::containing(self.p.position)
    }
    /// `Entity.isInRain`: at the block position or the top of the box.
    fn in_rain(&self) -> bool {
        self.scene.in_rain(self.p.position)
    }
    /// Whether this is a `ThrownTrident` (`AbstractArrow` with its own water
    /// inertia, sounds and hit).
    fn trident(&self) -> bool {
        matches!(self.p.kind, ProjectileKind::Trident)
    }
    fn tick(
        &mut self,
        targets: &impl ArrowTargets,
        impact: bool,
        defer_block_hit: bool,
        scratch: &mut ArrowHitScratch,
    ) -> Hit {
        // ThrownTrident.tick.
        if self.trident() && self.st.in_ground_time > 4 {
            self.st.dealt_damage = true;
        }
        let physics = !self.st.no_physics;
        let movement = self.p.velocity;
        let block_pos = self.block_position();
        let state = self.scene.block(block_pos);
        if !self.scene.is_air(state) && physics {
            let origin = Vec3::new(
                f64::from(block_pos.x),
                f64::from(block_pos.y),
                f64::from(block_pos.z),
            );
            let at = self.position();
            let inside = self
                .scene
                .collision_boxes(state, block_pos)
                .iter()
                .any(|b| b.move_by(origin).contains(at));
            if inside && !self.st.canceled_impact_inside {
                self.p.velocity = Vec3::ZERO;
                self.st.in_ground = true;
            }
            self.st.canceled_impact_inside &= inside;
        } else {
            self.st.canceled_impact_inside = false;
        }
        if self.st.shake > 0 {
            self.st.shake -= 1;
        }
        if self.st.was_touching_water || self.in_rain() {
            self.st.clear_fire();
        }
        if self.st.in_ground && physics {
            if !self.client {
                if self.st.last_state != Some(state) && self.should_fall() {
                    self.start_falling();
                } else {
                    // AbstractArrow.tickDespawn.
                    self.st.life += 1;
                    if self.st.life >= 1200 {
                        self.st.removed = true;
                    }
                }
            }
            self.st.in_ground_time += 1;
            if !self.st.removed {
                let at = self.position();
                self.apply_effects_from_blocks(at, at);
            }
            // Arrow.tick on the server: a tipped arrow stuck for 600 ticks sends
            // entity event 0 (the clients' burst of particles in its colour,
            // `handleEntityEvent`) and turns into a plain arrow. The flight
            // and ground particles are the clients' own (`Level.isClientSide`).
            if !self.client
                && self.st.in_ground_time >= 600
                && let Some(color) = self.st.potion_color()
            {
                self.st.events.push(ArrowEvent::PotionPuff {
                    id: self.p.id,
                    color,
                });
                self.st.pickup_item = crate::items::ItemStack::new("minecraft:arrow", 1);
            }
            return Hit::None;
        }
        self.st.in_ground_time = 0;
        let original = self.position();
        if self.st.was_touching_water {
            // getWaterInertia (0.99 for a trident), then the bubbles behind it.
            let inertia = if self.trident() { 0.99f32 } else { 0.6f32 };
            self.p.velocity = self.p.velocity.scale(f64::from(inertia));
            let v = self.p.velocity;
            for _ in 0..4 {
                self.st.events.push(ArrowEvent::Particle {
                    kind: ArrowParticle::Bubble,
                    position: Vec3::new(
                        original.x - v.x * 0.25,
                        original.y - v.y * 0.25,
                        original.z - v.z * 0.25,
                    ),
                    velocity: v,
                });
            }
        }
        if self.p.critical() {
            for i in 0..4 {
                let f = f64::from(i) / 4.0;
                self.st.events.push(ArrowEvent::Particle {
                    kind: ArrowParticle::Crit,
                    position: Vec3::new(
                        original.x + movement.x * f,
                        original.y + movement.y * f,
                        original.z + movement.z * f,
                    ),
                    velocity: Vec3::new(-movement.x, -movement.y + 0.2, -movement.z),
                });
            }
        }
        let y_rot = if physics {
            degrees(movement.x, movement.z)
        } else {
            degrees(-movement.x, -movement.z)
        };
        let x_rot = degrees(movement.y, movement.horizontal_distance());
        self.p.pitch = lerp_rotation(self.p.pitch, x_rot);
        self.p.yaw = lerp_rotation(self.p.yaw, y_rot);
        self.check_left_owner(targets);
        let mut hit = Hit::None;
        let end = Vec3::new(
            original.x + movement.x,
            original.y + movement.y,
            original.z + movement.z,
        );
        if physics && impact {
            hit = self.step_move_and_hit(original, end, targets, defer_block_hit, scratch);
            if matches!(hit, Hit::Entity { .. })
                || defer_block_hit && matches!(hit, Hit::Block { .. })
            {
                return hit;
            }
        } else {
            self.p.position = end;
            self.apply_effects_from_blocks(original, end);
            if physics {
                self.st.canceled_impact_inside = true;
            }
        }
        self.finish();
        hit
    }
    /// The end of a tick in flight: drag and gravity, then `Projectile.tick`
    /// and `Entity.baseTick`'s fluid and fire phases.
    fn finish(&mut self) {
        self.st.pierce_step = None;
        let physics = !self.st.no_physics;
        if !self.st.was_touching_water {
            self.p.velocity = self.p.velocity.scale(f64::from(0.99f32));
        }
        let gravity = self.p.get_gravity();
        if physics && !self.st.in_ground && gravity != 0.0 {
            // Entity.applyGravity skips the vector addition when gravity is zero.
            self.add_velocity(-gravity);
        }
        // Projectile.tick, then Entity.baseTick's fluid and fire phases.
        if !self.st.has_been_shot {
            if !self.client {
                self.st.events.push(ArrowEvent::ShootEvent);
            }
            self.st.has_been_shot = true;
        }
        self.st.was_eye_in_water = self.st.fluids.is_eye_in_fluid(&Tags, WATER);
        let mut fluids = std::mem::take(&mut self.st.fluids);
        let _ = entity_fluid_tick::update(&mut fluids, self, WATER, LAVA, 1_000_000);
        self.st.fluids = fluids;
        if self.client {
            self.st.clear_fire();
        } else if self.st.fire_ticks > 0 {
            self.st.fire_ticks -= 1;
        }
        if !self.st.first_tick && self.st.fluids.is_in_fluid(&Tags, LAVA) {
            self.st.fall_distance *= 0.5;
        }
        if self.position().y < self.scene.world.below_world_y() {
            self.st.removed = true;
        }
        self.st.first_tick = false;
    }
    /// `getDeltaMovement().add(0, y, 0)`: the zeros added to x and z also
    /// turn a negative zero positive.
    fn add_velocity(&mut self, y: f64) {
        let v = self.p.velocity;
        self.p.velocity = Vec3::new(v.x + 0.0, v.y + y, v.z + 0.0);
    }
    /// `Projectile.checkLeftOwner`: out of the owner's vehicle tree once the arrow's
    /// box, swept by its motion and grown by 1, no longer touches any pickable member.
    fn check_left_owner(&mut self, targets: &impl ArrowTargets) {
        if self.p.left_owner {
            return;
        }
        let swept = bounds(self.p.position)
            .expand_towards(self.p.velocity)
            .inflate(Vec3::new(1.0, 1.0, 1.0));
        self.p.left_owner = self
            .p
            .collision_owner()
            .is_none_or(|o| targets.outside_owner_collision_range(o, swept));
    }
    /// `AbstractArrow.shouldFall`.
    fn should_fall(&self) -> bool {
        let at = self.position();
        self.st.in_ground
            && self
                .scene
                .no_block_collision(Aabb::new(at, at).inflate(Vec3::new(0.06, 0.06, 0.06)))
    }
    /// `AbstractArrow.startFalling`.
    fn start_falling(&mut self) {
        self.st.in_ground = false;
        let v = self.p.velocity;
        let x = f64::from(self.p.entity_random().next_float() * 0.2);
        let y = f64::from(self.p.entity_random().next_float() * 0.2);
        let z = f64::from(self.p.entity_random().next_float() * 0.2);
        self.p.velocity = Vec3::new(v.x * x, v.y * y, v.z * z);
        self.st.life = 0;
    }
    /// `AbstractArrow.findHitEntities` (or `ThrownTrident.findHitEntity` singleton sweep)
    /// into reusable `scratch`, sorted by distance squared to actual entity position.
    fn find_hit_entities(
        &self,
        from: Vec3,
        end: Vec3,
        targets: &impl ArrowTargets,
        scratch: &mut ArrowHitScratch,
    ) {
        scratch.clear();
        // ThrownTrident.findHitEntity: none once it has dealt damage.
        if self.st.dealt_damage {
            return;
        }
        let margin = compute_margin(self.p.age as i32);
        let area = bounds(from)
            .expand_towards(self.p.velocity)
            .inflate(Vec3::new(1.0, 1.0, 1.0));
        let owner = self.p.collision_owner();
        let left_owner = self.p.left_owner;
        // ThrownTrident.findHitEntities wraps the singleton ProjectileUtil.getEntityHitResult.
        if self.trident() {
            let mut nearest = NearestHit::new();
            targets.visit(|c| {
                if !left_owner && owner.is_some_and(|o| targets.owner_tree_contains(o, c.who)) {
                    return;
                }
                if self.st.pierced.contains(&c.who) {
                    return;
                }
                nearest.consider(c, area, from, end, margin);
            });
            if let Some((who, at)) = nearest.finish() {
                scratch.push((who, at, 0.0, TargetOrder::dragon_part(0)));
            }
            return;
        }
        targets.visit(|c| {
            if !c.bounds.intersects(area) || self.st.pierced.contains(&c.who) {
                return;
            }
            // Vehicle ancestry is only relevant for candidates in the arrow's sweep.
            if !left_owner && owner.is_some_and(|o| targets.owner_tree_contains(o, c.who)) {
                return;
            }
            if let Some(at) = self.entity_hit(&c.bounds, from, end, margin) {
                let d = from.distance_squared(c.position);
                scratch.push((c.who, at, d, c.order));
            }
        });
        scratch.sort_by(|a, b| a.2.total_cmp(&b.2).then_with(|| a.3.cmp(&b.3)));
    }
    /// One iteration of `AbstractArrow.stepMoveAndHit` from `from` towards `end`.
    fn step_once(
        &mut self,
        from: Vec3,
        end: Vec3,
        block: Option<(Vec3, BlockPos, [i32; 3])>,
        targets: &impl ArrowTargets,
        defer_block_hit: bool,
        scratch: &mut ArrowHitScratch,
    ) -> (Hit, Vec<(Who, Vec3)>) {
        self.find_hit_entities(from, end, targets, scratch);
        let next = scratch.first().map_or(end, |h| h.1);
        self.p.position = next;
        self.apply_effects_from_blocks(from, next);
        if let Some(&(who, at, _, _)) = scratch.first() {
            let rest = if scratch.len() > 1 {
                scratch.drain(1..).map(|(w, p, _, _)| (w, p)).collect()
            } else {
                Vec::new()
            };
            scratch.clear();
            return (Hit::Entity { who, at }, rest);
        }
        let Some((at, pos, face)) = block else {
            return (Hit::None, Vec::new());
        };
        if self.st.removed {
            return (Hit::None, Vec::new());
        }
        let state = self.scene.block(pos);
        if self.scene.is_air(state) {
            return (Hit::None, Vec::new());
        }
        if !defer_block_hit {
            self.on_hit_block(pos, state);
        }
        (
            Hit::Block {
                pos: [pos.x, pos.y, pos.z],
                at,
                face,
            },
            Vec::new(),
        )
    }
    /// `AbstractArrow.stepMoveAndHit`: first entity (plus batch continuation) or block.
    fn step_move_and_hit(
        &mut self,
        from: Vec3,
        to: Vec3,
        targets: &impl ArrowTargets,
        defer_block_hit: bool,
        scratch: &mut ArrowHitScratch,
    ) -> Hit {
        let ignored = self
            .st
            .canceled_impact_inside
            .then(|| self.block_position());
        let block = self.scene.clip(from, to, ignored);
        let end = block.map_or(to, |b| b.0);
        let (hit, rest) = self.step_once(from, end, block, targets, defer_block_hit, scratch);
        // Keep the hit batch even at pierce 0: a refused hit (e.g. FallingBlock) continues the batch.
        if matches!(hit, Hit::Entity { .. }) && !self.trident() {
            self.st.pierce_step = Some(PierceStep {
                end,
                block,
                remaining_hits: rest,
            });
        }
        hit
    }
    /// `ProjectileUtil.getManyEntityHitResult` for one entity box: where the
    /// path enters it, else where it enters the box grown by `margin`, if a
    /// line from there towards the centre (stopped by blocks) reaches the box.
    fn entity_hit(&self, b: &Aabb, from: Vec3, to: Vec3, margin: f64) -> Option<Vec3> {
        if let Some(at) = b.clip(from, to) {
            return Some(at);
        }
        if margin <= 0.0 {
            return None;
        }
        let outside = b
            .inflate(Vec3::new(margin, margin, margin))
            .clip(from, to)?;
        let centre = b.center();
        let towards = self
            .scene
            .clip(outside, centre, None)
            .map_or(centre, |h| h.0);
        b.clip(outside, towards).map(|_| outside)
    }
    /// `playSound(soundEvent, 1, 1.2 / (random * 0.2 + 0.9))`.
    fn hit_sound(&mut self) {
        let pitch = 1.2 / (self.p.entity_random().next_float() * 0.2 + 0.9);
        // CrossbowItem.createProjectile: its arrows hit with the crossbow's sound.
        let crossbow = self.st.crossbow_hit_sound;
        let sound = if self.trident() {
            ArrowSound::TridentHitGround
        } else if crossbow {
            ArrowSound::CrossbowHit
        } else {
            ArrowSound::HitGround
        };
        self.st.events.push(ArrowEvent::Sound {
            sound,
            volume: 1.0,
            pitch,
        });
    }
    /// `ThrownTrident.onHitEntity` after the hurt, whether it landed or not:
    /// the hit sound, then `deflect(REVERSE, ..., (0.02, 0.2, 0.02))`. The
    /// trident keeps flying (it has dealt its damage) and falls.
    fn trident_bounce(&mut self) {
        self.st.dealt_damage = true;
        self.st.events.push(ArrowEvent::Sound {
            sound: ArrowSound::TridentHit,
            volume: 1.0,
            pitch: 1.0,
        });
        // ProjectileDeflection.REVERSE.
        let rotation = 170.0 + self.p.entity_random().next_float() * 20.0;
        let v = self.p.velocity;
        self.p.velocity = Vec3::new(v.x * (-0.02 * 0.5), v.y * (-0.2 * 0.5), v.z * (-0.02 * 0.5));
        self.p.yaw += rotation;
    }
    /// `onHitEntity` when the entity took no damage: `deflect(REVERSE, 0.2)`,
    /// a half turn give or take ten degrees at a tenth of the speed; one
    /// left almost still drops its pickup (ALLOWED) and is discarded.
    fn turn_back(&mut self) {
        deflect_reverse(
            &mut self.p.velocity,
            &mut self.p.yaw,
            self.st,
            &mut self.p.random.0,
        );
    }
    /// `AbstractArrow.onHitBlock`: back off the face by 0.05 along each axis
    /// it moved on, stop, and stick.
    fn on_hit_block(&mut self, pos: BlockPos, state: BlockStateId) {
        self.st.last_state = Some(state);
        // BlockState.onProjectileHit still sees the deltaMovement it hit at.
        self.st.hit_speed = self.p.velocity.length();
        // Projectile.onHitBlock: BlockState.onProjectileHit.
        if !self.client
            && self.st.on_fire()
            && let Some(reaction) = self.block_reaction(state)
        {
            self.st.events.push(ArrowEvent::Block {
                pos,
                reaction,
                owner: self.p.owner,
            });
        }
        let v = self.p.velocity;
        let offset = f64::from(0.05f32);
        self.p.position = Vec3::new(
            self.p.position.x - signum(v.x) * offset,
            self.p.position.y - signum(v.y) * offset,
            self.p.position.z - signum(v.z) * offset,
        );
        self.p.velocity = Vec3::ZERO;
        self.hit_sound();
        self.st.in_ground = true;
        self.st.shake = 7;
        // setSoundEvent(SoundEvents.ARROW_HIT): later hits and the save.
        self.st.crossbow_hit_sound = false;
        self.p.set_critical(false);
        self.st.pierce = 0;
        self.st.pierced.clear();
        self.st.pierced_killed.clear();
        self.st.pierce_step = None;
    }
    /// `Projectile.mayInteract`: a mob's arrows need `mob_griefing`, or a
    /// mod's yes for the owner (NeoForge's `canEntityGrief(level, owner)`).
    fn may_interact(&self) -> bool {
        match self.p.owner {
            Some(Who::Mob(owner)) => match self.scene.mob_hooks {
                Some(hooks) => hooks.can_grief(owner, self.scene.mob_griefing),
                None => self.scene.mob_griefing,
            },
            None => {
                !self
                    .p
                    .owner_uuid
                    .is_some_and(|u| self.scene.owners.non_player_entity(u))
                    || self.scene.mob_griefing
            }
            Some(Who::Player) => true,
        }
    }
    /// `onProjectileHit` of the blocks a burning projectile lights.
    fn block_reaction(&self, state: BlockStateId) -> Option<BlockReaction> {
        let name = self.scene.registry.state(state)?.name.as_str();
        let flag = |key: &str| self.scene.property(state, key) == Some("true");
        let lit = || {
            let mut definition = self.scene.registry.state(state)?.clone();
            definition.properties.insert("lit".into(), "true".into());
            self.scene.registry.id(&definition)
        };
        match name {
            "minecraft:tnt" if self.may_interact() && self.scene.tnt_explodes => {
                Some(BlockReaction::PrimeTnt)
            }
            "minecraft:campfire" | "minecraft:soul_campfire"
                if self.may_interact() && !flag("lit") && !flag("waterlogged") =>
            {
                lit().map(BlockReaction::Light)
            }
            // AbstractCandleBlock: candles also must be dry (CandleBlock.canBeLit).
            _ if name.ends_with("candle_cake") && !flag("lit") => lit().map(BlockReaction::Light),
            _ if name.ends_with("candle") && !flag("lit") && !flag("waterlogged") => {
                lit().map(BlockReaction::Light)
            }
            _ => None,
        }
    }
    /// `Entity.applyEffectsFromBlocks`: the sweep keeps the arrow's
    /// bubble-column pushes and applies the fire effects; the part's contact
    /// fires at the batch that ticks it (Game::sweep_projectile_inside), on
    /// the shared walk, as items' and the other projectiles' do — this own
    /// walk no longer records it for the drain.
    fn apply_effects_from_blocks(&mut self, from: Vec3, to: Vec3) {
        if self.st.removed || self.st.no_physics {
            return;
        }
        let scene = self.scene;
        let (fire_ticks, extinguished) =
            scene.apply_block_effects(from, to, self.st.fire_ticks, |state, p, precise| {
                let name = &*scene.facts.get(state).name;
                if name == "minecraft:bubble_column" && precise {
                    self.bubble_column(state, p);
                }
            });
        self.st.fire_ticks = fire_ticks;
        if extinguished {
            // Entity.playEntityOnFireExtinguishedSound: keep the entity RNG order.
            let pitch = 1.6
                + (self.p.entity_random().next_float() - self.p.entity_random().next_float()) * 0.4;
            self.st.events.push(ArrowEvent::Sound {
                sound: ArrowSound::Extinguish,
                volume: 0.7,
                pitch,
            });
        }
    }
    /// `BubbleColumnBlock.entityInside` with `Projectile`'s pushes.
    fn bubble_column(&mut self, state: BlockStateId, p: BlockPos) {
        let drag = self.scene.property(state, "drag") == Some("true");
        let above = self.scene.block(BlockPos::new(p.x, p.y + 1, p.z));
        // The state above's collision shape, queried at this position.
        let nothing_above = self.scene.collision_boxes(above, p).is_empty()
            && self.scene.fluids.fluid(above).kind() == FluidKind::Empty;
        if self.st.in_ground {
            return;
        }
        if nothing_above {
            self.add_velocity(if drag { -0.03 } else { 0.1 });
            self.st
                .events
                .push(ArrowEvent::BubbleColumnParticles { pos: p });
        } else {
            self.add_velocity(if drag { -0.03 } else { 0.06 });
            self.st.fall_distance = 0.0;
        }
    }
}
impl ArrowScene<'_, '_> {
    /// `Entity.isInRain`: at the feet or the top of the arrow-sized box.
    pub fn in_rain(&self, at: Vec3) -> bool {
        let p = BlockPos::containing(at);
        (self.raining)(p)
            || (self.raining)(BlockPos::new(
                p.x,
                bounds(at).max_position().y.floor() as i32,
                p.z,
            ))
    }
    /// The water membership used by `AbstractArrow.tick` on the next tick.
    /// Only query the fluid; callers keep their own movement and splash logic.
    pub fn in_water(&self, at: Vec3) -> bool {
        let b = bounds(at).deflate(Vec3::new(0.001, 0.001, 0.001));
        let lo = b.min_position();
        let hi = b.max_position();
        let low = BlockPos::containing(lo);
        let high = BlockPos::new(
            hi.x.ceil() as i32 - 1,
            hi.y.ceil() as i32 - 1,
            hi.z.ceil() as i32 - 1,
        );
        for x in (low.x - 1) >> 4..=(high.x + 1) >> 4 {
            for z in (low.z - 1) >> 4..=(high.z + 1) >> 4 {
                if !self.world.chunks.contains_key(&(x, z)) {
                    return false;
                }
            }
        }
        for x in low.x..=high.x {
            for y in low.y..=high.y {
                for z in low.z..=high.z {
                    let fluid = self.fluids.fluid(self.block(BlockPos::new(x, y, z)));
                    let mut scene = self;
                    if matches!(fluid.kind(), FluidKind::Water | FluidKind::FlowingWater)
                        && let Ok(height) = fluid.height_at(&mut scene, [x, y, z])
                        && f64::from(y) + f64::from(height) >= lo.y
                        && f64::from(y) + f64::from(height) > at.y
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
    /// `Entity.applyEffectsFromBlocks` for arrows and player-thrown tridents.
    /// The caller handles `isAffectedByBlocks`, initial water/rain clearing and
    /// `baseTick`. `visit` preserves arrow contact hooks and bubble pushes.
    /// Returns the fire timer and whether to play the extinguishing sound.
    pub fn apply_block_effects(
        &self,
        from: Vec3,
        to: Vec3,
        mut fire_ticks: i32,
        mut visit: impl FnMut(BlockStateId, BlockPos, bool),
    ) -> (i32, bool) {
        let previous_fire = fire_ticks;
        let mut visited = crate::fast_hash::FxHashSet::default();
        let mut effects = Collector::default();
        let mut walk = ArrowInside {
            scene: self,
            visit: &mut visit,
        };
        // The shared walk: one movement with no original, its 16-iteration
        // budget, then the target alone when the budget ran out.
        let _ = crate::block_contact::sweep_movements(
            &mut walk,
            &[sunrise_world::entity_move::MovementRecord {
                from,
                to,
                original: None,
            }],
            &mut visited,
            &mut effects,
        );
        for effect in effects.finish() {
            match effect {
                Effect::ClearFreeze => {}
                Effect::FireIgnite => {
                    // BaseFireBlock.fireIgnite for a non-player entity.
                    if fire_ticks < 0 {
                        fire_ticks += 1;
                    }
                    if fire_ticks >= 0 {
                        fire_ticks = fire_ticks.max(160);
                    }
                }
                Effect::LavaIgnite => fire_ticks = fire_ticks.max(300),
                Effect::Extinguish => fire_ticks = fire_ticks.min(0),
            }
        }
        if self.in_rain(to) {
            fire_ticks = fire_ticks.min(0);
        }
        let extinguished = previous_fire > 0 && fire_ticks <= 0;
        let ignited = fire_ticks > previous_fire;
        if fire_ticks <= 0 && !ignited {
            // -getFireImmuneTicks(): zero for AbstractArrow.
            fire_ticks = 0;
        }
        (fire_ticks, extinguished)
    }
    /// `Entity.collidedWithFluid`: the fluid's box swept from `from` to `to`.
    fn collided_with_fluid(&self, fluid: FluidState, p: BlockPos, from: Vec3, to: Vec3) -> bool {
        if fluid.kind() == FluidKind::Empty {
            return false;
        }
        let mut scene = self;
        let Ok(height) = fluid.height_at(&mut scene, [p.x, p.y, p.z]) else {
            return false;
        };
        let b = Aabb::new(
            Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)),
            Vec3::new(
                f64::from(p.x) + 1.0,
                f64::from(p.y) + f64::from(height),
                f64::from(p.z) + 1.0,
            ),
        );
        bounds(from).collided_along_vector(to.subtract(from), &[b])
    }
}
/// An arrow's view of the shared inside walk: the scene's blocks, every block
/// the whole block, the bubble column and contact hooks through `visit`, the
/// fire's and the fluid's own effects queued into the arrow's collector.
struct ArrowInside<'s, 'a, 'b, F> {
    scene: &'s ArrowScene<'a, 'b>,
    visit: &'s mut F,
}
impl<F: FnMut(BlockStateId, BlockPos, bool)> crate::block_contact::InsideWalk
    for ArrowInside<'_, '_, '_, F>
{
    type Effects = Collector;
    type Error = std::convert::Infallible;
    const DEFLATE: f64 = 1.0e-5_f32 as f64;
    fn bounds_at(&self, at: Vec3) -> Aabb {
        bounds(at)
    }
    fn is_alive(&self) -> bool {
        true
    }
    fn block(&self, at: BlockPos) -> BlockStateId {
        self.scene.block(at)
    }
    fn is_air(&self, state: BlockStateId) -> bool {
        self.scene.is_air(state)
    }
    fn inside_shape(&mut self, _: BlockStateId, _: BlockPos) -> crate::block_contact::InsideShape {
        // getEntityInsideCollisionShape: the whole block for these.
        crate::block_contact::InsideShape::Full
    }
    fn collided_with_fluid(&self, at: BlockPos, from: Vec3, to: Vec3) -> bool {
        let fluid = self.scene.fluids.fluid(self.scene.block(at));
        self.scene.collided_with_fluid(fluid, at, from, to)
    }
    fn entity_inside(
        &mut self,
        at: BlockPos,
        state: BlockStateId,
        _: i32,
        precise: bool,
        effects: &mut Collector,
    ) -> Result<(), std::convert::Infallible> {
        (self.visit)(state, at, precise);
        if matches!(
            &*self.scene.facts.get(state).name,
            "minecraft:fire" | "minecraft:soul_fire"
        ) {
            // BaseFireBlock.entityInside (its inFire hurt does nothing to arrows).
            effects.apply(Effect::ClearFreeze);
            effects.apply(Effect::FireIgnite);
        }
        Ok(())
    }
    fn fluid_inside(
        &mut self,
        at: BlockPos,
        _: i32,
        effects: &mut Collector,
    ) -> Result<(), std::convert::Infallible> {
        match self.scene.fluids.fluid(self.scene.block(at)).kind() {
            // WaterFluid.entityInside.
            FluidKind::Water | FluidKind::FlowingWater => effects.apply(Effect::Extinguish),
            // LavaFluid.entityInside (lavaHurt does nothing to arrows).
            FluidKind::Lava | FluidKind::FlowingLava => {
                effects.apply(Effect::ClearFreeze);
                effects.apply(Effect::LavaIgnite);
            }
            // A mod's fluid does nothing to an entity inside it.
            FluidKind::Empty | FluidKind::Mod(_) | FluidKind::ModFlowing(_) => {}
        }
        Ok(())
    }
}
impl FluidHeightWorld for &ArrowScene<'_, '_> {
    type Error = Error;
    fn fluid(&mut self, p: [i32; 3]) -> Result<FluidState, Error> {
        Ok(self
            .fluids
            .fluid(self.block(BlockPos::new(p[0], p[1], p[2]))))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> Result<bool, Error> {
        let state = self.block(BlockPos::new(p[0], p[1], p[2]));
        Ok(self.fluids.sturdy(state, Direction::Down))
    }
}
/// `InsideBlockEffectType`s an arrow meets, in their apply order (FREEZE
/// needs powder snow, which an arrow's collision context passes through).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Effect {
    ClearFreeze,
    FireIgnite,
    LavaIgnite,
    Extinguish,
}
const EFFECT_ORDER: [Effect; 4] = [
    Effect::ClearFreeze,
    Effect::FireIgnite,
    Effect::LavaIgnite,
    Effect::Extinguish,
];
/// `InsideBlockEffectApplier.StepBasedCollector`: each step's effects once,
/// in type order, steps in the order they were reached.
struct Collector {
    last_step: i32,
    step: [bool; 4],
    effects: Vec<Effect>,
}
impl Default for Collector {
    fn default() -> Self {
        Self {
            last_step: -1,
            step: [false; 4],
            effects: Vec::new(),
        }
    }
}
impl crate::block_contact::StepCollector for Collector {
    fn advance_step(&mut self, step: i32) {
        if self.last_step != step {
            self.last_step = step;
            self.flush();
        }
    }
}
impl Collector {
    fn apply(&mut self, effect: Effect) {
        self.step[effect as usize] = true;
    }
    fn flush(&mut self) {
        for effect in EFFECT_ORDER {
            if std::mem::take(&mut self.step[effect as usize]) {
                self.effects.push(effect);
            }
        }
    }
    fn finish(mut self) -> Vec<Effect> {
        self.flush();
        self.effects
    }
}
struct Tags;
impl FluidMembership for Tags {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        if tag == WATER {
            matches!(kind, FluidKind::Water | FluidKind::FlowingWater)
        } else {
            matches!(kind, FluidKind::Lava | FluidKind::FlowingLava)
        }
    }
}
type Error = Box<dyn std::error::Error>;
impl FluidMembership for Ticker<'_, '_, '_, '_> {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        Tags.fluid_in_tag(kind, tag)
    }
}
impl CurrentTarget for Ticker<'_, '_, '_, '_> {
    type Error = Error;
    fn is_player_class(&self) -> bool {
        false
    }
    fn velocity(&mut self) -> Result<Vec3, Error> {
        Ok(self.p.velocity)
    }
    fn add_delta_movement(&mut self, delta: Vec3) -> Result<(), Error> {
        let v = self.p.velocity;
        let sum = Vec3::new(v.x + delta.x, v.y + delta.y, v.z + delta.z);
        if delta.is_finite() && sum.is_finite() {
            self.p.velocity = sum;
        }
        Ok(())
    }
}
impl FluidInteractionWorld for Ticker<'_, '_, '_, '_> {
    type Error = Error;
    type Chunk = ();
    fn interaction_box(&mut self) -> Result<Option<Aabb>, Error> {
        Ok(Some(
            bounds(self.p.position).deflate(Vec3::new(0.001, 0.001, 0.001)),
        ))
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> Result<Option<()>, Error> {
        Ok(self.scene.world.chunks.contains_key(&(x, z)).then_some(()))
    }
    fn chunk_sections(&mut self, _: ()) -> Result<FluidSections<'_>, Error> {
        let layout = self.scene.world.layout();
        Ok(FluidSections {
            minimum_section_y: layout.min_y() >> 4,
            has_fluid: crate::chunk::every_section(layout),
        })
    }
    fn bounding_box_min_y(&self) -> f64 {
        self.p.position.y
    }
    fn block_x(&self) -> i32 {
        self.p.position.x.floor() as i32
    }
    fn eye_y(&mut self) -> Result<f64, Error> {
        Ok(self.p.position.y + f64::from(EYE_HEIGHT))
    }
    fn block_z(&self) -> i32 {
        self.p.position.z.floor() as i32
    }
    fn fluid_state(&mut self, p: BlockPos) -> Result<FluidState, Error> {
        Ok(self.scene.fluids.fluid(self.scene.block(p)))
    }
    fn fluid_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32, Error> {
        f.height_at(self, [p.x, p.y, p.z])
    }
    fn camera_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32, Error> {
        f.camera_height_at(self, [p.x, p.y, p.z])
    }
    fn flow(&mut self, f: FluidState, p: BlockPos) -> Result<Vec3, Error> {
        crate::motion::pushing_flow(self.scene.loop_context, self, f, p)
    }
}
impl EntityFluidTickWorld for Ticker<'_, '_, '_, '_> {
    /// `AbstractArrow.isPushedByFluid`: not when stuck.
    fn is_pushed_by_fluid(&mut self) -> Result<bool, Error> {
        Ok(!self.st.in_ground)
    }
    fn reset_fall_distance(&mut self) -> Result<(), Error> {
        self.st.fall_distance = 0.0;
        Ok(())
    }
    fn was_touching_water(&self) -> bool {
        self.st.was_touching_water
    }
    fn first_tick(&self) -> bool {
        self.st.first_tick
    }
    fn water_splash(&mut self) -> Result<(), Error> {
        water_splash::water_splash(self, 1_000_000).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn set_was_touching_water(&mut self, value: bool) {
        self.st.was_touching_water = value;
    }
    fn fast_lava(&mut self) -> Result<bool, Error> {
        Ok(false)
    }
}
impl FluidHeightWorld for Ticker<'_, '_, '_, '_> {
    type Error = Error;
    fn fluid(&mut self, p: [i32; 3]) -> Result<FluidState, Error> {
        Ok(self
            .scene
            .fluids
            .fluid(self.scene.block(BlockPos::new(p[0], p[1], p[2]))))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> Result<bool, Error> {
        let state = self.scene.block(BlockPos::new(p[0], p[1], p[2]));
        Ok(self.scene.fluids.sturdy(state, Direction::Down))
    }
}
impl FluidFlowWorld for Ticker<'_, '_, '_, '_> {
    type Error = Error;
    type Block = BlockStateId;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState, Error> {
        Ok(self.scene.fluids.fluid(self.scene.block(p)))
    }
    fn block(&mut self, p: BlockPos) -> Result<BlockStateId, Error> {
        Ok(self.scene.block(p))
    }
    fn blocks_fluid_flow(&self, s: BlockStateId) -> bool {
        self.scene.fluids.blocks_flow(s)
    }
    fn is_ice_class(&self, s: BlockStateId) -> bool {
        self.scene.fluids.ice(s)
    }
    fn is_face_sturdy(
        &mut self,
        s: BlockStateId,
        _: BlockPos,
        d: Direction,
    ) -> Result<bool, Error> {
        Ok(self.scene.fluids.sturdy(s, d))
    }
}
/// `Entity.doWaterSplashEffect` with the arrow's own random.
impl WaterSplashWorld for Ticker<'_, '_, '_, '_> {
    type Error = Error;
    type Entity = ();
    type Sound = ();
    fn this_entity(&self) {}
    fn controlling_passenger(&mut self) -> Result<Option<()>, Error> {
        Ok(None)
    }
    fn velocity(&mut self, _: ()) -> Result<Vec3, Error> {
        Ok(self.p.velocity)
    }
    fn splash_sound(&mut self, _: bool) -> Result<(), Error> {
        Ok(())
    }
    fn random_float(&mut self) -> Result<f32, Error> {
        Ok(self.p.entity_random().next_float())
    }
    fn random_double(&mut self) -> Result<f64, Error> {
        Ok(self.p.entity_random().next_double())
    }
    fn play_sound(&mut self, _: (), volume: f32, pitch: f32) -> Result<(), Error> {
        self.st.events.push(ArrowEvent::Sound {
            sound: ArrowSound::Splash,
            volume,
            pitch,
        });
        Ok(())
    }
    fn position(&self) -> Vec3 {
        self.p.position
    }
    fn width(&self) -> f32 {
        WIDTH
    }
    fn particle(
        &mut self,
        kind: SplashParticle,
        position: Vec3,
        velocity: Vec3,
    ) -> Result<(), Error> {
        self.st.events.push(ArrowEvent::Particle {
            kind: match kind {
                SplashParticle::Bubble => ArrowParticle::Bubble,
                SplashParticle::Splash => ArrowParticle::Splash,
            },
            position,
            velocity,
        });
        Ok(())
    }
    fn splash_event(&mut self) -> Result<(), Error> {
        if !self.client {
            self.st.events.push(ArrowEvent::SplashEvent);
        }
        Ok(())
    }
}
