//! Primed TNT, falling blocks, projectiles, tridents, firework rockets and
//! end crystals on a server's client (`ClientPacketListener.handleAddEntity`
//! and `handleSetEntityData`): the server simulates them, the client draws
//! them where it says and interpolates their moves.
use super::{FallingBlock, fireworks::fireworks_of};
use sunrise_client_render::particles::ParticleLevel;
use sunrise_core::{geometry::Vec3, position::BlockPos, random::RandomSource};
use sunrise_gameplay_common::game::native_ticks::{TickRound, places_of, slot_place};
use sunrise_items::items::ItemStack;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_protocol::entity_data::Value;
use sunrise_sim::join_seq::{JoinList, RestoreOrder};
use sunrise_sim::projectiles::{Projectile, ProjectileKind};
use sunrise_world::registry::BlockStateId;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static HANDBACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[allow(unused_imports)]
pub(in crate::game) use sunrise_client_gameplay::remote::projectiles::{
    RemoteCloud, TRIDENT, cloud_client_tick, default_cloud_particle, projectile_kind,
    remote_projectile_item, remote_projectile_power, remote_projectile_step,
    shulker_bullet_trail_at,
};
#[allow(unused_imports)]
#[cfg(test)]
pub(in crate::game) use sunrise_client_gameplay::remote::projectiles::{
    remote_bullet_tick, remote_projectile_tick,
};

/// `AbstractHurtingProjectile.getTrailParticle`: smoke for every one of them
/// but the wind charges (no trail) and the dragon's fireball (its own).
pub(in crate::game) fn has_smoke_trail(kind: &ProjectileKind) -> bool {
    matches!(
        kind,
        ProjectileKind::SmallFireball
            | ProjectileKind::LargeFireball { .. }
            | ProjectileKind::WitherSkull { .. }
    )
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `Arrow.handleEntityEvent(0)`: a tipped arrow stuck for 600 ticks bursts
    /// into twenty particles in its colour. The server turns it into a plain
    /// arrow and does not send the colour again, so it stops here.
    pub(in crate::game::client_side) fn remote_arrow_puff(&mut self, id: u32) {
        let Some(i) = self
            .common
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == id)
        else {
            return;
        };
        let Some((at, color)) = self.common.creatures.projectiles.update(i, |p| {
            let a = p.arrow.as_mut()?;
            Some((p.position, std::mem::replace(&mut a.effect_color, -1)))
        }) else {
            return;
        };
        self.arrow_makeparticle(at, color, 20);
    }

    /// `handleAddEntity` for these types; false when `kind` isn't one.
    pub fn spawn_remote_projectile(
        &mut self,
        kind: &str,
        packet: &sunrise_protocol::packets::play::AddEntity,
        rotation: (f32, f32),
    ) -> bool {
        let id = packet.id;
        self.remove_remote_projectile(id as u32);
        self.spawn_remote_projectile_inner(kind, packet, rotation)
    }

    /// Whether the client tracks `id` already: asked by `AddEntity` before it tracks the new one.
    pub(in crate::game::client_side) fn tracks_remote_entity(&self, id: i32) -> bool {
        self.client
            .remote
            .as_ref()
            .is_some_and(|r| r.tracked_entity(id).is_some())
    }

    /// `AddEntity`'s `spawn_remote_projectile`. Only an id the client already tracked (`replace`)
    /// can still be in `falling` or `tnts`, so a new id skips those passes; the crystals and the
    /// small lists are filtered every time, since the client adds to some of those itself.
    pub(in crate::game::client_side) fn spawn_added_projectile(
        &mut self,
        p: &sunrise_protocol::packets::play::AddEntity,
        kind: &str,
        rotation: (f32, f32),
        replace: bool,
    ) {
        if replace {
            self.spawn_remote_projectile(kind, p, rotation);
        } else {
            self.common.crystals.retain(|c| c.id != p.id as u32);
            self.remove_remote_small_projectiles(p.id as u32);
            self.spawn_remote_projectile_inner(kind, p, rotation);
        }
    }

    /// `handleSetEntityData` for these types: what changes how they look.
    pub fn remote_projectile_data(&mut self, id: i32, values: &[(u8, i32, Value)]) {
        let id = id as u32;
        self.remote_hook_data(id, values);
        if let Some(at) = self
            .client
            .remote_clouds
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.at)
        {
            // AreaEffectCloud.DATA_RADIUS, DATA_WAITING and DATA_PARTICLE.
            for (index, _, value) in values {
                let particle = match (index, value) {
                    (10, Value::Particle(p)) => self.received_particle(p, at),
                    _ => None,
                };
                let Some(c) = self.client.remote_clouds.iter_mut().find(|c| c.id == id) else {
                    return;
                };
                match (index, value) {
                    (8, Value::Float(r)) => c.radius = *r,
                    (9, Value::Bool(w)) => c.waiting = *w,
                    (10, Value::Particle(_)) => {
                        if let Some(particle) = particle {
                            c.particle = particle;
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
        if self.common.rockets.iter().any(|r| r.id == id) {
            for (index, _, value) in values {
                match (index, value) {
                    // Entity.DATA_SILENT: suppresses client burst blast and twinkle sounds.
                    (4, Value::Bool(silent)) => {
                        if let Some(r) = self.common.rockets.iter_mut().find(|r| r.id == id) {
                            r.silent = *silent;
                        }
                    }
                    // FireworkRocketEntity.DATA_ID_FIREWORKS_ITEM: its
                    // explosions for `createFireworks`.
                    (8, Value::Item(stack)) => {
                        if let Some(stack) = self.to_stack(stack)
                            && let Some(r) = self.common.rockets.iter_mut().find(|r| r.id == id)
                        {
                            let (_, explosions) = fireworks_of(&stack);
                            r.explosions = explosions.into();
                            r.item = stack.into();
                        }
                    }
                    // DATA_ATTACHED_TO_TARGET (OptionalInt: the id plus 1).
                    (9, Value::Id(v)) => {
                        let v = (*v > 0).then(|| (*v - 1) as u32);
                        if let Some(r) = self.common.rockets.iter_mut().find(|r| r.id == id) {
                            r.attached_to = v;
                        }
                    }
                    // DATA_SHOT_AT_ANGLE.
                    (10, Value::Bool(a)) => {
                        if let Some(r) = self.common.rockets.iter_mut().find(|r| r.id == id) {
                            r.angled = *a;
                        }
                    }
                    _ => {}
                }
            }
        }
        if self.common.potions.has_thrown(id) {
            // ThrowableItemProjectile.DATA_ITEM_STACK: the potion it is.
            for (index, _, value) in values {
                if let (8, Value::Item(stack)) = (index, value)
                    && let Some(stack) = self.to_stack(stack)
                {
                    crate::game::client_side::potions::remote_thrown_item(
                        &mut self.common.potions,
                        id,
                        stack,
                    );
                }
            }
            return;
        }
        // Fireball.DATA_ITEM_STACK: the item a fireball is drawn as.
        if let Some(stack) = values
            .iter()
            .find_map(|(index, _, value)| match (index, value) {
                (8, Value::Item(stack)) => Some(stack),
                _ => None,
            })
            && let Some(stack) = self.to_stack(stack)
        {
            remote_projectile_item(&mut self.common.creatures.projectiles, id, stack);
        }
        for (index, _, value) in values {
            if let Some(t) = self.common.tnts.iter_mut().find(|t| t.id == id) {
                // PrimedTnt.DATA_FUSE_ID.
                if let (8, Value::Int(fuse)) = (index, value) {
                    t.fuse = *fuse;
                }
            } else if let Some(i) = self.common.falling.iter().position(|f| f.id == id) {
                // FallingBlockEntity.DATA_START_POS (the model's seed).
                if let (8, Value::Pos(p)) = (index, value) {
                    self.common
                        .falling
                        .update(i, |f| f.start_pos = BlockPos::new(p[0], p[1], p[2]));
                }
            } else if let Some(c) = self.common.crystals.iter_mut().find(|c| c.id == id) {
                // EndCrystal.DATA_BEAM_TARGET and DATA_SHOW_BOTTOM.
                match (index, value) {
                    (8, Value::OptPos(beam)) => c.beam = *beam,
                    (9, Value::Bool(show)) => c.show_bottom = *show,
                    _ => {}
                }
            } else if let Some(t) = self.common.tridents.iter_mut().find(|t| t.id == id) {
                // AbstractArrow.ID_FLAGS (no physics) and IN_GROUND, and
                // ThrownTrident.ID_LOYALTY.
                match (index, value) {
                    // Clients retain the server's fire flag until the next update.
                    (0, Value::Byte(flags)) => t.fire_ticks = i32::from(flags & 1),
                    (8, Value::Byte(flags)) => t.no_physics = flags & 2 != 0,
                    (10, Value::Bool(in_ground)) => t.in_ground = *in_ground,
                    (11, Value::Byte(loyalty)) => t.loyalty = u32::from(*loyalty as u8),
                    _ => {}
                }
            } else if let Some(i) = self
                .common
                .creatures
                .projectiles
                .iter()
                .position(|p| p.id == id)
            {
                self.common.creatures.projectiles.update(i, |p| {
                    match (&mut p.kind, index, value) {
                        // Entity.DATA_SHARED_FLAGS_ID's FLAG_ONFIRE: Entity.isOnFire on
                        // a client, which draws the flames. A client doesn't count the
                        // fire down: the flag goes off when the server's fire is out.
                        (_, 0, Value::Byte(flags)) => {
                            if let Some(a) = p.arrow.as_mut() {
                                a.synced_fire = flags & 1 != 0;
                            } else {
                                p.synced_fire = flags & 1 != 0;
                            }
                        }
                        (_, 1, Value::Int(air_supply)) => p.air_supply = *air_supply,
                        // AbstractArrow.ID_FLAGS (critical) and IN_GROUND.
                        (ProjectileKind::Arrow { critical, .. }, 8, Value::Byte(flags)) => {
                            *critical = flags & 1 != 0;
                            if let Some(a) = p.arrow.as_mut() {
                                a.no_physics = flags & 2 != 0;
                            }
                        }
                        (ProjectileKind::Arrow { .. }, 5, Value::Bool(no_gravity)) => {
                            p.no_gravity = *no_gravity;
                        }
                        (ProjectileKind::Arrow { .. }, 10, Value::Bool(in_ground)) => {
                            // AbstractArrow.onSyncedDataUpdated: IN_GROUND turning true after
                            // the first tick starts the shake.
                            if *in_ground
                                && !p.stuck
                                && p.age > 0
                                && let Some(a) = p.arrow.as_mut()
                                && a.shake <= 0
                            {
                                a.shake = 7;
                            }
                            p.stuck = *in_ground;
                            if let Some(a) = p.arrow.as_mut() {
                                a.in_ground = *in_ground;
                            }
                        }
                        // Arrow.ID_EFFECT_COLOR.
                        (ProjectileKind::Arrow { .. }, 11, Value::Int(color)) => {
                            if let Some(a) = p.arrow.as_mut() {
                                a.effect_color = *color;
                            }
                        }
                        // WitherSkull.DATA_DANGEROUS.
                        (ProjectileKind::WitherSkull { dangerous }, 8, Value::Bool(d)) => {
                            *dangerous = *d;
                        }
                        _ => {}
                    }
                });
            }
        }
    }

    /// A move's rotation for a projectile or trident.
    pub(in crate::game) fn turn_remote_projectile(
        &mut self,
        id: i32,
        rotation: Option<(f32, f32)>,
    ) {
        let (id, Some((yaw, pitch))) = (id as u32, rotation) else {
            return;
        };
        if let Some(i) = self
            .common
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == id)
        {
            self.common.creatures.projectiles.update(i, |p| {
                p.yaw = yaw;
                p.pitch = pitch;
            });
        } else if let Some(t) = self.common.tridents.iter_mut().find(|t| t.id == id) {
            t.yaw = yaw;
            t.pitch = pitch;
        }
    }

    /// `Entity.getEyePosition` of a projectile's owner: this game's player or
    /// another player the server sent.
    pub(in crate::game::client_side) fn remote_eye(&self, id: u32) -> Option<Vec3> {
        if self.remote_player_id() == Some(id) {
            let eye = self.common.eye_position()?;
            return Some(Vec3::new(eye[0], eye[1], eye[2]));
        }
        let m = self.common.mobs.iter().find(|m| m.id == id)?;
        Some(Vec3::new(
            m.position.x,
            m.position.y + m.eye_height(),
            m.position.z,
        ))
    }

    /// This client's own player's entity id on the server.
    pub(in crate::game::client_side) fn remote_player_id(&self) -> Option<u32> {
        self.client
            .remote
            .as_ref()
            .and_then(|r| u32::try_from(r.entity_id).ok())
    }

    /// `FireworkRocketEntity.tick` for the rockets attached to this client's
    /// own player, once the player has moved (it ticks before them in
    /// `ClientLevel.tickEntities`), each with its spark.
    pub(in crate::game) fn tick_own_rockets(&mut self) {
        let Some(me) = self.remote_player_id() else {
            return;
        };
        if !self.common.clock.runs_normally()
            || self.common.player.is_none()
            || !self
                .common
                .rockets
                .iter()
                .any(|r| r.attached_to == Some(me))
        {
            return;
        }
        let mut rockets = self.common.rockets.lift();
        #[cfg(test)]
        nested_spawn_tests::after_lift(self, nested_spawn_tests::Point::OwnRocket);
        for r in rockets.iter_mut().filter(|r| r.attached_to == Some(me)) {
            self.follow_own_player(r);
            self.pick_section_move(r.id, r.position.components());
            self.rocket_spark(r);
        }
        self.common
            .rockets
            .restore(rockets, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
    }

    /// One tick of interpolation, as `tick_remote_objects`.
    pub fn tick_remote_projectiles(&mut self, mut step: impl FnMut(u32, Vec3) -> Vec3) {
        self.tick_remote_fangs();
        // PrimedTnt.tick runs on a client too: it falls, bounces and burns its
        // fuse down (the swell and flash follow it) between the server's
        // updates (every 10 ticks), and a client discards it when the fuse
        // runs out; only the server explodes it.
        let (world, collision, assets, facts, fluids, border, blocks) = (
            &self.common.world,
            self.common.collision,
            &self.common.pack.assets,
            self.common.res.facts(),
            &self.common.res.motion().fluids,
            Some(self.common.border.geometry()),
            Some(&self.common.res.motion().blocks),
        );
        let side_server = self.game_side_client();
        let side = client_game_side!(self, side_server);
        let sections = self.client.client_pick.sections.get_mut();
        self.common.tnts.retain_mut(|t| {
            // A fluid lookup that fails leaves it where its move put it.
            let _ = t.tick_among(
                world,
                collision,
                assets,
                facts,
                fluids,
                &self.common.loop_context,
                border,
                blocks,
                Some(&side),
                &[],
            );
            let kept = t.fuse > 0;
            if kept {
                sections.move_id(t.id, t.position.components());
            } else {
                sections.remove(t.id);
            }
            kept
        });
        // PrimedTnt.tick: while the fuse is still lit, a client adds one SMOKE
        // particle at (x, y + 0.5, z). The server's Level.addParticle never
        // reaches this window.
        let smoking: Vec<Vec3> = self.common.tnts.iter().map(|t| t.position).collect();
        let smoke = ParticleOptions::simple(ParticleType::Smoke);
        for p in smoking {
            self.client
                .particles
                .0
                .add(&smoke, [p.x, p.y + 0.5, p.z], [0.0; 3]);
        }
        // Clients simulate motion and drag; only the server places/removes the landing block.
        let mut falling = self.common.falling.lift();
        #[cfg(test)]
        nested_spawn_tests::after_lift(self, nested_spawn_tests::Point::Falling);
        falling.for_each_mut(|f| {
            let _ = self.client_fall_step(f);
            self.pick_section_move(f.id, f.position.components());
        });
        self.common
            .falling
            .restore(falling, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
        // Projectiles tick on the client too and fly on between the tracker's
        // updates (10 to 20 ticks apart); a block stops them there, and only
        // the server decides what they hit. EntityTickEvent.Pre and .Post bracket each remote
        // projectile and trident individually when subscribed; when unsubscribed, the fast loop
        // runs with zero hand-backs.
        let mut round = self.native_tick_round();
        let told = round.told_listening();
        let mut projectiles = self.common.creatures.projectiles.lift();
        #[cfg(test)]
        nested_spawn_tests::after_lift(self, nested_spawn_tests::Point::Projectile);
        let bubbles: Vec<_> = projectiles
            .iter()
            .filter_map(Projectile::water_bubbles)
            .collect();
        let mut tridents = self.common.tridents.lift();
        #[cfg(test)]
        nested_spawn_tests::after_lift(self, nested_spawn_tests::Point::Trident);
        let mut arrow_events = Vec::new();
        if told {
            (projectiles, tridents) = self.tick_remote_projectiles_told(
                projectiles,
                tridents,
                &mut round,
                &mut arrow_events,
            );
        } else {
            // Their visits, counted once the loop hands the game back: no hook runs between.
            let mut visited = Vec::new();
            {
                let rain = self.arrow_rain();
                let arrow_scene = self.client_arrow_scene(&rain);
                let side = self.client_mob_side();
                let w = self.client_mob_world_base(&[], &side);
                let world = super::creatures::ProjWorld {
                    w: &w,
                    targets: &[],
                    border: Some(self.common.border.bounds()),
                };
                projectiles.for_each_mut(|p| {
                    visited.push(p.id);
                    remote_projectile_step(p, &arrow_scene, &world, &mut arrow_events);
                    self.pick_section_move(p.id, p.position.components());
                });
                for t in &mut tridents {
                    visited.push(t.id);
                    crate::game::client_side::trident_client::remote_tick(
                        t,
                        &world,
                        self.remote_eye(t.owner),
                    );
                    self.pick_section_move(t.id, t.position.components());
                }
            }
            for id in visited {
                self.count_client_tick(id);
            }
        }
        self.common.creatures.projectiles.restore(
            projectiles,
            sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst,
        );
        self.common.tridents.restore(
            tridents,
            sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst,
        );
        for (at, event) in arrow_events {
            self.client_arrow_event(at, event);
        }
        self.arrow_effect_particles();
        let trails: Vec<Vec3> = self
            .common
            .creatures
            .projectiles
            .iter()
            .filter(|p| has_smoke_trail(&p.kind))
            .map(|p| p.position)
            .collect();
        self.smoke_trails(trails);
        self.dragon_breath_trails();
        self.shulker_bullet_trails();
        self.throwable_bubbles(bubbles);
        // FireworkRocketEntity.tick: a rocket attached to another entity
        // follows it, and every rocket leaves its spark trail on a client. One
        // boosting this client's own player ticks after it (`tick_own_rockets`).
        let me = self.remote_player_id();
        let mut rockets = self.common.rockets.lift();
        #[cfg(test)]
        nested_spawn_tests::after_lift(self, nested_spawn_tests::Point::RemoteRocket);
        for r in &mut rockets {
            let side_server = self.game_side_client();
            r.remote_step(
                self.common
                    .body_motion(&client_game_side!(self, side_server)),
            );
            if let Some(target) = r.attached_to {
                if me == Some(target) {
                    self.pick_section_move(r.id, r.position.components());
                    continue;
                }
                if let Some(m) = self.common.mobs.iter().find(|m| m.id == target) {
                    // Another player's motion is not known here: its step.
                    r.velocity = Vec3::new(
                        m.position.x - r.previous.x,
                        m.position.y - r.previous.y,
                        m.position.z - r.previous.z,
                    );
                    r.position = m.position;
                }
            }
            self.pick_section_move(r.id, r.position.components());
            self.rocket_spark(r);
        }
        self.common
            .rockets
            .restore(rockets, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
        for c in &mut self.common.crystals {
            // ClientLevel.tickNonPassenger's setOldPosAndRot, then the interpolation step.
            c.previous = c.position;
            c.time += 1;
            c.position = step(c.id, c.position);
        }
        self.step_remote_thrown();
        self.tick_remote_hooks();
        self.common.end.step_remote_eyes();
        self.tick_remote_clouds();
    }

    pub(in crate::game) fn remove_remote_projectile(&mut self, id: u32) {
        self.common.tnts.retain(|t| t.id != id);
        self.common.falling.retain(|f| f.id != id);
        self.common.crystals.retain(|c| c.id != id);
        self.remove_remote_small_projectiles(id);
    }

    pub(in crate::game::client_side) fn remove_remote_small_projectiles(&mut self, id: u32) {
        self.common.fangs.list.retain(|f| f.id != id);
        self.common.creatures.projectiles.retain(|p| p.id != id);
        self.common.tridents.retain(|t| t.id != id);
        self.common.rockets.retain(|r| r.id != id);
        crate::game::client_side::potions::remove_remote_thrown(&mut self.common.potions, id);
        self.client.remote_clouds.retain(|c| c.id != id);
        self.common.end.remove_remote_eye(id);
        self.remove_remote_hook(id);
    }

    /// Every one a server sent (a new connection, level or dimension).
    pub fn clear_remote_projectiles(&mut self) {
        self.common.fangs.list.clear();
        self.common.tnts.clear();
        self.common.falling.clear();
        self.common.creatures.projectiles.clear();
        self.common.tridents.clear();
        self.common.rockets.clear();
        self.common.crystals.clear();
        // The dragon a server sent goes with its crystals.
        self.common.dragon.discard();
        self.common.potions.clear_thrown();
        self.client.remote_clouds.clear();
        self.common.end.clear_remote_eyes();
        self.clear_remote_hooks();
    }

    /// Their kind and place, for client `status` (including remote fishing hooks).
    pub(in crate::game) fn client_projectiles_status(&self) -> Vec<serde_json::Value> {
        let mut out = self.common.projectiles_status();
        out.extend(self.hooks_status());
        out
    }

    /// Subscribed path for remote projectiles and tridents: each entity is bracketed
    /// individually right before and right after its own tick, with both lists handed back so
    /// hook handles read the live rows. A canceled one keeps its state.
    pub(in crate::game::client_side) fn tick_remote_projectiles_told(
        &mut self,
        mut projectiles: JoinList<Projectile, false>,
        mut tridents: JoinList<sunrise_sim::thrown_trident::ThrownTrident>,
        round: &mut TickRound,
        arrow_events: &mut Vec<(Vec3, sunrise_sim::entities::arrows::ArrowEvent)>,
    ) -> (
        JoinList<Projectile, false>,
        JoinList<sunrise_sim::thrown_trident::ThrownTrident>,
    ) {
        let slots: Vec<u32> = projectiles.iter().map(|p| p.id).collect();
        let mut places = None;
        for slot in 0..slots.len() {
            let Some(mut i) = slot_place(&places, &slots, slot) else {
                continue;
            };
            let (id, kind) = (projectiles[i].id, projectiles[i].entity_type());
            if round.pre() {
                let mut ticks = true;
                let changed;
                (projectiles, tridents, changed) =
                    self.remote_projectiles_told(projectiles, tridents, |game| {
                        told_visit(game, id, |game| {
                            ticks = game.native_tick_pre_one(round, id, kind);
                            ticks
                        });
                    });
                if changed.0 {
                    places = Some(places_of(projectiles.iter().map(|p| p.id)));
                }
                let Some(at) = slot_place(&places, &slots, slot).filter(|_| ticks) else {
                    continue;
                };
                i = at;
            } else {
                // Only Post is subscribed: no Pre can cancel its tick, so its visit counts here.
                self.count_client_tick(id);
            }
            {
                let rain = self.arrow_rain();
                let scene = self.client_arrow_scene(&rain);
                let side = self.client_mob_side();
                let w = self.client_mob_world_base(&[], &side);
                let world = super::creatures::ProjWorld {
                    w: &w,
                    targets: &[],
                    border: Some(self.common.border.bounds()),
                };
                projectiles.update(i, |p| {
                    remote_projectile_step(p, &scene, &world, arrow_events);
                    self.pick_section_move(p.id, p.position.components());
                });
            }
            if round.post() {
                let changed;
                (projectiles, tridents, changed) =
                    self.remote_projectiles_told(projectiles, tridents, |game| {
                        game.native_tick_post_one(round, id, kind);
                    });
                if changed.0 {
                    places = Some(places_of(projectiles.iter().map(|p| p.id)));
                }
            }
        }
        let slots: Vec<u32> = tridents.iter().map(|t| t.id).collect();
        let mut places = None;
        for slot in 0..slots.len() {
            let id = slots[slot];
            let Some(mut i) = slot_place(&places, &slots, slot) else {
                continue;
            };
            if round.pre() {
                let mut ticks = true;
                let changed;
                (projectiles, tridents, changed) =
                    self.remote_projectiles_told(projectiles, tridents, |game| {
                        told_visit(game, id, |game| {
                            ticks = game.native_tick_pre_one(round, id, TRIDENT);
                            ticks
                        });
                    });
                if changed.1 {
                    places = Some(places_of(tridents.iter().map(|t| t.id)));
                }
                let Some(at) = slot_place(&places, &slots, slot).filter(|_| ticks) else {
                    continue;
                };
                i = at;
            } else {
                // Only Post is subscribed: no Pre can cancel its tick, so its visit counts here.
                self.count_client_tick(id);
            }
            {
                let side = self.client_mob_side();
                let w = self.client_mob_world_base(&[], &side);
                let world = super::creatures::ProjWorld {
                    w: &w,
                    targets: &[],
                    border: Some(self.common.border.bounds()),
                };
                let eye = self.remote_eye(tridents[i].owner);
                let t = &mut tridents[i];
                crate::game::client_side::trident_client::remote_tick(t, &world, eye);
                self.pick_section_move(t.id, t.position.components());
            }
            if round.post() {
                let changed;
                (projectiles, tridents, changed) =
                    self.remote_projectiles_told(projectiles, tridents, |game| {
                        game.native_tick_post_one(round, id, TRIDENT);
                    });
                if changed.1 {
                    places = Some(places_of(tridents.iter().map(|t| t.id)));
                }
            }
        }
        (projectiles, tridents)
    }

    /// Reset the per-thread remote projectile handback counter (test support).
    #[cfg(any(test, feature = "test-support"))]
    pub fn reset_remote_projectile_handbacks() {
        HANDBACKS.with(|count| count.set(0));
    }

    /// Read the per-thread remote projectile handback counter (test support).
    #[cfg(any(test, feature = "test-support"))]
    pub fn remote_projectile_handbacks() -> usize {
        HANDBACKS.with(std::cell::Cell::get)
    }

    /// Hands the client's lifted projectiles and tridents back for a hook told
    /// about one of them, and takes them again. Whether each list came back
    /// written (its write stamp): its rows may then have moved. What spawns
    /// meanwhile goes after them, as at the end of the family's tick.
    pub(in crate::game::client_side) fn remote_projectiles_told(
        &mut self,
        projectiles: JoinList<Projectile, false>,
        tridents: JoinList<sunrise_sim::thrown_trident::ThrownTrident>,
        tell: impl FnOnce(&mut Self),
    ) -> (
        JoinList<Projectile, false>,
        JoinList<sunrise_sim::thrown_trident::ThrownTrident>,
        (bool, bool),
    ) {
        #[cfg(any(test, feature = "test-support"))]
        HANDBACKS.with(|count| count.set(count.get() + 1));
        self.common
            .creatures
            .projectiles
            .restore(projectiles, RestoreOrder::SurvivorsFirst);
        self.common
            .tridents
            .restore(tridents, RestoreOrder::SurvivorsFirst);
        let stamps = (
            self.common.creatures.projectiles.stamp(),
            self.common.tridents.stamp(),
        );
        tell(self);
        let changed = (
            self.common.creatures.projectiles.stamp() != stamps.0,
            self.common.tridents.stamp() != stamps.1,
        );
        (
            self.common.creatures.projectiles.lift(),
            self.common.tridents.lift(),
            changed,
        )
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `Arrow.tick` and `SpectralArrow.tick` on a client: a tipped arrow
    /// sheds two entity-effect swirls in its colour while flying, one every
    /// fifth tick in a block; a spectral arrow an effect sparkle at its
    /// position while flying.
    pub(in crate::game::client_side) fn arrow_effect_particles(&mut self) {
        let mut puffs: Vec<(Vec3, i32, u32)> = Vec::new();
        for p in &self.common.creatures.projectiles {
            let Some(a) = p.arrow.as_ref().filter(|_| p.is_arrow()) else {
                continue;
            };
            let at = p.position;
            if p.is_spectral_arrow() {
                if !p.stuck {
                    let options = ParticleOptions::simple(ParticleType::Effect);
                    self.client
                        .particles
                        .0
                        .add(&options, [at.x, at.y, at.z], [0.0; 3]);
                }
                continue;
            }
            let count = match (p.stuck, a.in_ground_time % 5) {
                (true, 0) => 1,
                (true, _) => 0,
                (false, _) => 2,
            };
            puffs.push((at, a.effect_color, count));
        }
        for (at, color, count) in puffs {
            self.arrow_makeparticle(at, color, count);
        }
    }
    /// `Arrow.makeParticle`: `count` entity-effect swirls in `color` (an ARGB
    /// int, -1 for none) inside the arrow's 0.5 x 0.5 box.
    pub(in crate::game::client_side) fn arrow_makeparticle(
        &mut self,
        at: Vec3,
        color: i32,
        count: u32,
    ) {
        // `EntityType.ARROW` is 0.5 x 0.5.
        const SIZE: f64 = 0.5;
        if color == -1 {
            return;
        }
        // ColorParticleOption.create(ENTITY_EFFECT, colour): ARGB as floats.
        let channel = |shift: u32| (color as u32 >> shift & 255) as f32 / 255.0;
        let options = ParticleOptions::colored(
            ParticleType::EntityEffect,
            [channel(16), channel(8), channel(0), channel(24)],
        );
        for _ in 0..count {
            // getRandomX(0.5), getRandomY, getRandomZ(0.5).
            let r = self.client.particles.0.random();
            let x = at.x + SIZE * (2.0 * r.next_double() - 1.0) * 0.5;
            let y = at.y + SIZE * r.next_double();
            let z = at.z + SIZE * (2.0 * r.next_double() - 1.0) * 0.5;
            self.client.particles.0.add(&options, [x, y, z], [0.0; 3]);
        }
    }
    /// `DragonFireball.getTrailParticle`: dragon breath, where
    /// `createParticleTrail` puts every hurting projectile's trail.
    pub(in crate::game) fn dragon_breath_trails(&mut self) {
        let trail = ParticleOptions::simple(ParticleType::DragonBreath);
        let at: Vec<Vec3> = self
            .common
            .creatures
            .projectiles
            .iter()
            .filter(|p| p.kind == ProjectileKind::DragonFireball)
            .map(|p| p.position)
            .collect();
        for p in at {
            self.client
                .particles
                .0
                .add(&trail, [p.x, p.y + 0.5, p.z], [0.0; 3]);
        }
    }
    /// `ShulkerBullet.tick` on a client: two `END_ROD` particles in its
    /// 0.3125 box (`getRandomX(0.5)`, `getRandomY`, `getRandomZ(0.5)`).
    pub(in crate::game::client_side) fn shulker_bullet_trails(&mut self) {
        let bullets: Vec<Vec3> = self
            .common
            .creatures
            .projectiles
            .iter()
            .filter(|p| matches!(p.kind, ProjectileKind::ShulkerBullet { .. }))
            .map(|p| p.position)
            .collect();
        let options = ParticleOptions::simple(ParticleType::EndRod);
        for at in bullets {
            for _ in 0..2 {
                let p = shulker_bullet_trail_at(at, self.client.particles.0.random());
                self.client.particles.0.add(&options, p, [0.0; 3]);
            }
        }
    }
    /// `AbstractHurtingProjectile.createParticleTrail`: one smoke puff half a
    /// block above each position.
    pub(in crate::game) fn smoke_trails(&mut self, at: impl IntoIterator<Item = Vec3>) {
        let smoke = ParticleOptions::simple(ParticleType::Smoke);
        for p in at {
            self.client
                .particles
                .0
                .add(&smoke, [p.x, p.y + 0.5, p.z], [0.0; 3]);
        }
    }
    /// `spawn_remote_projectile` without first removing an entity under the same id: for
    /// `AddEntity` of an id the client does not track yet, which no list holds.
    pub(in crate::game::client_side) fn spawn_remote_projectile_inner(
        &mut self,
        kind: &str,
        packet: &sunrise_protocol::packets::play::AddEntity,
        rotation: (f32, f32),
    ) -> bool {
        let id = packet.id as u32;
        let position = packet.position;
        let velocity = packet.velocity;
        let data = packet.data;
        let at = Vec3::new(position[0], position[1], position[2]);
        let velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        let (yaw, pitch) = rotation;
        match kind {
            "minecraft:evoker_fangs" => self.common.insert_fangs(id, position, yaw, 0, 0),
            "minecraft:tnt" => {
                self.common
                    .tnts
                    .push(sunrise_gameplay_common::game::tnt_common::PrimedTnt {
                        size_answer: None,
                        attachment_lifetime: Default::default(),
                        rotation: [0.0, 0.0],
                        join: Default::default(),
                        no_gravity: false,
                        id,
                        position: at,
                        previous: at,
                        velocity,
                        fuse: 80,
                        ticks: 0,
                        ground: Default::default(),
                        fall_distance: 0.0,
                        stuck: Vec3::ZERO,
                        in_water: false,
                        owner: None,
                        guest: None,
                        power: 4.0,
                        used_portal: false,
                        block_state: None,
                    })
            }
            // FallingBlockEntity.recreateFromPacket: the block state is the data.
            "minecraft:falling_block" => self.common.falling.push(FallingBlock {
                size_answer: None,
                attachment_lifetime: Default::default(),
                rotation: [0.0, 0.0],
                join: Default::default(),
                id,
                state: BlockStateId(data.max(0) as u32),
                blocks_building: true,
                start_pos: BlockPos::default(),
                position: at,
                previous: at,
                velocity,
                age: 0,
                no_gravity: false,
                portal: Default::default(),
                drop_item: false,
                fall_distance: 0.0,
                ground: Default::default(),
                fire_ticks: 0,
                sync_velocity: false,
                stuck: Vec3::ZERO,
                damage: Default::default(),
                saved: None,
            }),
            "minecraft:trident" => {
                let mut t = sunrise_sim::thrown_trident::ThrownTrident::remote(id, at, yaw, pitch);
                t.velocity = velocity;
                // Projectile.getAddEntityPacket: the owner's id is the data.
                t.owner = data as u32;
                self.common.tridents.push(t);
            }
            "minecraft:firework_rocket" => {
                self.common
                    .rockets
                    .push(super::fireworks::Rocket::remote(id, at, velocity));
            }
            "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:experience_bottle" => {
                crate::game::client_side::potions::add_remote_thrown(
                    &mut self.common.potions,
                    id,
                    kind,
                    at,
                    velocity,
                );
            }
            "minecraft:area_effect_cloud" => self.client.remote_clouds.push(RemoteCloud {
                id,
                at: position,
                radius: 3.0,
                waiting: false,
                particle: default_cloud_particle(),
                random: sunrise_level::random_source::entity(),
            }),
            "minecraft:eye_of_ender" => {
                self.common
                    .end
                    .add_remote_eye(id, position, [velocity.x, velocity.y, velocity.z])
            }
            // FishingHook.recreateFromPacket: the owner's id is the data.
            "minecraft:fishing_bobber" => self.spawn_remote_hook(
                id,
                sunrise_sim::lan::playerdata::uuid_ints(packet.uuid),
                at,
                velocity,
                data,
            ),
            "minecraft:end_crystal" => {
                // EndCrystal's constructor starts `time` from the crystal's own
                // random, so each placed crystal spins from its own phase. The id is the
                // server's, given in the order its chunks load, so under `SUNRISE_STILL` the
                // position alone seeds it.
                let drawn = sunrise_sim::mobs::entity_random(id, at)
                    .next_int_bound(100_000)
                    .unwrap_or(0);
                let time = sunrise_gameplay_common::game::dragon_common::Crystal::start_time(
                    at,
                    drawn,
                    self.common.clock.still(),
                );
                self.common.crystals.push(
                    sunrise_gameplay_common::game::dragon_common::Crystal::new(id, at, true, time),
                );
            }
            _ => {
                let Some(k) = projectile_kind(kind) else {
                    return false;
                };
                if k == ProjectileKind::LlamaSpit {
                    self.client
                        .particles
                        .0
                        .llama_spit_burst(position, [velocity.x, velocity.y, velocity.z]);
                }
                let mut p = Projectile::new(k, at, None);
                // SpectralArrow: its own type; its pickup item says so.
                if kind == "minecraft:spectral_arrow"
                    && let Some(a) = p.arrow.as_mut()
                {
                    a.pickup_item = ItemStack::new("minecraft:spectral_arrow", 1);
                }
                p.id = id;
                p.velocity = velocity;
                p.yaw = yaw;
                p.pitch = pitch;
                if let Some(a) = p.arrow.as_mut()
                    && !p.stuck
                {
                    a.spawn_pos = Some(p.position);
                    a.spawn_velocity = Some(p.velocity);
                    a.spawn_rot = Some((p.pitch, p.yaw));
                }
                self.common.creatures.projectiles.push(p);
            }
        }
        true
    }
    pub(in crate::game::client_side) fn throwable_bubbles(
        &mut self,
        at: Vec<([f64; 3], [f64; 3])>,
    ) {
        let bubble = ParticleOptions::simple(ParticleType::Bubble);
        for (position, speed) in at {
            for _ in 0..4 {
                self.client.particles.0.add(&bubble, position, speed);
            }
        }
    }
    /// `AreaEffectCloud.clientTick` for each cloud a server tracks.
    pub(in crate::game::client_side) fn tick_remote_clouds(&mut self) {
        let level = ParticleLevel::new(
            &self.common.world,
            self.common.res.facts(),
            self.common.collision,
        );
        for c in &mut self.client.remote_clouds {
            cloud_client_tick(
                &mut self.client.particles.0,
                &level,
                c.at,
                c.radius,
                c.waiting,
                &c.particle,
                &mut c.random,
            );
        }
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use sunrise_core::geometry::Aabb;
    use sunrise_sim::mobs::Who;
    use sunrise_sim::projectiles::ProjectileWorld;

    /// A world with nothing in it: the client tick of a projectile only
    /// needs the blocks around it.
    struct Empty;
    impl ProjectileWorld for Empty {
        fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            None
        }
        fn in_water(&self, _: Aabb) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &[]
        }
    }

    /// An upward bubble column at (0, 100, 0) with water over it.
    struct Column;
    impl ProjectileWorld for Column {
        fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            None
        }
        fn in_water(&self, _: Aabb) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &[]
        }
        fn bubble_column(&self, p: [i32; 3]) -> Option<(bool, bool)> {
            (p == [0, 100, 0]).then_some((false, false))
        }
    }

    /// `ShulkerBullet.tick` applies the blocks' effects on the client too
    /// (`applyEffectsFromBlocks` after its move), so a bullet in a bubble
    /// column gets `Projectile.onInsideBubbleColumn`'s 0.06 up and its fall
    /// reset there, between the server's updates.
    #[test]
    fn a_client_shulker_bullet_in_a_bubble_column_is_pushed_up() {
        let mut p = Projectile::new(
            ProjectileKind::ShulkerBullet {
                target: Some(Who::Player),
                dir: None,
                steps: 0,
                delta: [0.0; 3],
                seed: 0,
            },
            Vec3::new(0.5, 100.1, 0.5),
            None,
        );
        p.fall_distance = 5.0;
        remote_bullet_tick(&mut p, &Column);
        assert_eq!(p.velocity, Vec3::new(0.0, 0.06, 0.0));
        assert_eq!(p.fall_distance, 0.0);
        assert!(p.bubble_surfaces.is_empty());
    }

    /// A thrown item, as a server's client tracks it: the spawn position and
    /// the motion the server gave it.
    fn thrown(kind: ProjectileKind) -> Projectile {
        let mut p = Projectile::new(kind, Vec3::new(0.0, 100.0, 0.0), None);
        p.velocity = Vec3::new(1.5, 0.0, 0.0);
        p
    }

    /// The same, under the id a server tracks it by.
    fn tracked(kind: ProjectileKind, id: u32) -> Projectile {
        let mut p = thrown(kind);
        p.id = id;
        p
    }

    #[test]
    fn a_shulker_bullet_trail_stays_inside_its_box() {
        let at = Vec3::new(10.0, 20.0, 30.0);
        let mut random = sunrise_core::random::LegacyRandom::new(1);
        for _ in 0..32 {
            let [x, y, z] = shulker_bullet_trail_at(at, &mut random);
            assert!((x - at.x).abs() <= 0.3125 * 0.5 + 1e-12, "{x}");
            assert!((0.0..0.3125 + 1e-12).contains(&(y - at.y)), "{y}");
            assert!((z - at.z).abs() <= 0.3125 * 0.5 + 1e-12, "{z}");
        }
    }

    /// `Entity.setOldPosAndRot`: what the renderer interpolates a projectile's
    /// rotation from is the rotation the last tick ended with. An arrow shot
    /// straight up at the lowest power a bow fires turns over within a few
    /// ticks, so every tick has to carry the value of the one before it.
    #[test]
    fn a_thrown_item_flies_on_between_the_server_updates() {
        // A thrown egg, an ender pearl and a wind charge (#1762): the server
        // sends their position every 10 ticks, and the client ticks them
        // itself in between, as vanilla's no-op interpolation handler does.
        for kind in [
            ProjectileKind::Egg,
            ProjectileKind::EnderPearl,
            ProjectileKind::WindCharge,
        ] {
            let mut p = thrown(kind);
            remote_projectile_tick(&mut p, &Empty);
            assert!(p.position.x > 1.0, "the projectile did not fly on: {p:?}");
            // A second tick moves it further: it keeps flying, not a hop.
            let first = p.position.x;
            remote_projectile_tick(&mut p, &Empty);
            assert!(p.position.x > first, "the projectile stopped: {p:?}");
        }
    }

    #[test]
    fn a_thrown_item_falls_and_drags_like_a_throwable_projectile() {
        // ThrowableProjectile.tick: gravity 0.03, then the air drag 0.99F
        // (`getAirDrag`, a float widened to the double it scales by).
        let drag = f64::from(0.99_f32);
        let mut p = thrown(ProjectileKind::Egg);
        remote_projectile_tick(&mut p, &Empty);
        assert!((p.velocity.x - 1.5 * drag).abs() < 1e-9, "{p:?}");
        assert!((p.velocity.y - (-0.03 * drag)).abs() < 1e-9, "{p:?}");
        // A wind charge keeps its speed: no gravity and no drag
        // (AbstractWindCharge: acceleration 0, inertia 1).
        let mut p = thrown(ProjectileKind::WindCharge);
        remote_projectile_tick(&mut p, &Empty);
        assert!((p.velocity.x - 1.5).abs() < 1e-9, "{p:?}");
        assert!((p.velocity.y - 0.0).abs() < 1e-9, "{p:?}");
    }

    #[test]
    fn a_power_packet_gives_a_fireball_the_power_its_next_tick_uses() {
        // ClientPacketListener.handleProjectilePowerPacket: the fireball with
        // id 12 gets the power the server sends bundled with its motion.
        let mut list: sunrise_sim::join_seq::JoinList<Projectile, false> =
            [tracked(ProjectileKind::LargeFireball { power: 1 }, 12)]
                .into_iter()
                .collect();
        remote_projectile_power(&mut list, 12, 0.5);
        assert_eq!(list[0].acceleration, 0.5);
        // AbstractHurtingProjectile.applyInertia: (1.5 + 0.5) * (double) 0.95f.
        list.update(0, |p| remote_projectile_tick(p, &Empty));
        assert_eq!(list[0].velocity.x, 1.899_999_976_158_142);
    }

    #[test]
    fn a_power_packet_ignores_an_arrow_an_unknown_id_and_a_thrown_item() {
        // Vanilla's instanceof AbstractHurtingProjectile: an arrow and a
        // thrown egg are none, and no entity is id 99.
        let mut list: sunrise_sim::join_seq::JoinList<Projectile, false> = [
            tracked(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                3,
            ),
            tracked(ProjectileKind::Egg, 4),
            tracked(ProjectileKind::LargeFireball { power: 1 }, 12),
        ]
        .into_iter()
        .collect();
        remote_projectile_power(&mut list, 3, 0.5);
        remote_projectile_power(&mut list, 4, 0.5);
        remote_projectile_power(&mut list, 99, 0.5);
        assert_eq!(
            list.iter().map(|p| p.acceleration).collect::<Vec<_>>(),
            [0.1; 3]
        );
    }

    #[test]
    fn a_guest_draws_a_fireball_as_the_item_the_server_sends() {
        // `ClientPacketListener.handleSetEntityData` of
        // Fireball.DATA_ITEM_STACK: the item `Fireball.getItem` returns, which
        // is the fire charge the default was until the server says otherwise.
        let mut list: sunrise_sim::join_seq::JoinList<Projectile, false> = [
            tracked(ProjectileKind::LargeFireball { power: 1 }, 12),
            tracked(ProjectileKind::SmallFireball, 13),
            tracked(ProjectileKind::Snowball, 14),
        ]
        .into_iter()
        .collect();
        remote_projectile_item(&mut list, 12, ItemStack::new("minecraft:diamond", 4));
        assert_eq!(
            list[0].item.as_ref().map(|i| (&*i.item, i.count)),
            Some(("minecraft:diamond", 1)),
            "one of it, as `setItem` keeps"
        );
        // The default fire charge goes back when the server sends an empty
        // stack, and a snowball is no `Fireball`: it has no item to set.
        remote_projectile_item(&mut list, 13, ItemStack::new("minecraft:air", 1));
        assert_eq!(
            list[1].item.as_ref().map(|i| (&*i.item, i.count)),
            Some(("minecraft:fire_charge", 1))
        );
        remote_projectile_item(&mut list, 14, ItemStack::new("minecraft:diamond", 1));
        assert_eq!(list[2].item, None);
    }
}

/// The visit of one entity told alone: a passenger counts before its Pre (`tickPassenger`), a
/// root once `pre` let it through (`tickNonPassenger`).
#[cfg(feature = "client")]
pub(in crate::game::client_side) fn told_visit(
    game: &mut crate::game::ClientGame<'_>,
    id: u32,
    pre: impl FnOnce(&mut crate::game::ClientGame<'_>) -> bool,
) {
    let passenger = game.client_passenger(id);
    if passenger {
        game.count_client_tick(id);
    }
    if pre(game) && !passenger {
        game.count_client_tick(id);
    }
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::fireworks::FireworksGameExt1 as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod nested_spawn_tests {
    use super::*;
    use std::cell::Cell;
    const ID: u32 = 1_900_000_001;
    const AT: Vec3 = Vec3::new(3.5, 80.0, 3.5);
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(in crate::game) enum Point {
        Falling,
        Projectile,
        Trident,
        RemoteRocket,
        OwnRocket,
    }
    type Spawn = fn(&mut crate::game::ClientGame<'_>, u32);
    thread_local! { static PENDING: Cell<Option<(Point, Spawn)>> = const { Cell::new(None) }; }
    pub(super) fn after_lift(game: &mut crate::game::ClientGame<'_>, point: Point) {
        if let Some((expected, spawn)) = PENDING.get()
            && point == expected
        {
            PENDING.set(None);
            spawn(game, ID);
        }
    }
    fn with_spawn(point: Point, spawn: Spawn, action: impl FnOnce()) {
        assert!(PENDING.replace(Some((point, spawn))).is_none());
        action();
        assert!(PENDING.take().is_none(), "requested lift was not exercised");
    }
    fn spawn(game: &mut crate::game::ClientGame<'_>, kind: &str, data: i32, id: u32) {
        assert!(game.spawn_remote_projectile(
            kind,
            &sunrise_protocol::packets::play::AddEntity {
                id: id as i32,
                uuid: u128::from(id),
                kind: 0,
                position: [AT.x, AT.y, AT.z],
                velocity: [1.0, 0.0, 0.0],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data
            },
            (0.0, 0.0),
        ));
    }
    fn falling(game: &mut crate::game::ClientGame<'_>, id: u32) {
        let sand = game
            .pack
            .assets
            .registry
            .default_state("minecraft:sand")
            .unwrap();
        spawn(game, "minecraft:falling_block", sand.0 as i32, id);
    }
    #[test]
    #[ignore = "daily"]
    fn client_falling_block_keeps_collision_bounce_through_drag() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            use std::sync::Arc;
            let slime = game
                .pack
                .assets
                .registry
                .default_state("minecraft:slime_block")
                .unwrap();
            game.common
                .world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            game.world.set(4, 63, 4, slime).unwrap();
            falling(game, 48);
            game.falling.update(0, |f| {
                f.position = Vec3::new(4.5, 64.25, 4.5);
                f.velocity = Vec3::new(0.0, -0.96, 0.0);
            });
            game.tick_remote_projectiles(|_, p| p);
            let f = &game.falling[0];
            assert_eq!(f.position.y.to_bits(), 64.0_f64.to_bits());
            assert_eq!(f.velocity.y.to_bits(), 0.7878808308504466_f64.to_bits());
            assert!(f.ground.on_ground);
        });
    }
    fn projectile(game: &mut crate::game::ClientGame<'_>, id: u32) {
        spawn(game, "minecraft:shulker_bullet", 0, id);
    }
    fn trident(game: &mut crate::game::ClientGame<'_>, id: u32) {
        spawn(game, "minecraft:trident", 0, id);
    }
    fn rocket(game: &mut crate::game::ClientGame<'_>, id: u32) {
        spawn(game, "minecraft:firework_rocket", 0, id);
    }
    fn own_rocket(game: &mut crate::game::ClientGame<'_>, id: u32) {
        rocket(game, id);
        let me = game.client.remote.as_ref().unwrap().entity_id as u32;
        game.rockets.for_each_mut(|r| {
            if r.id == id {
                r.attached_to = Some(me);
            }
        });
    }
    fn identities(
        game: &mut crate::game::ClientGame<'_>,
        point: Point,
        remove: bool,
    ) -> Vec<(u32, sunrise_sim::join_seq::JoinSeq)> {
        macro_rules! rows {
            ($list:expr) => {{
                if remove {
                    let _ = $list.remove(1);
                }
                $list.iter().map(|e| (e.id, e.join)).collect()
            }};
        }
        match point {
            Point::Falling => rows!(game.falling),
            Point::Projectile => rows!(game.creatures.projectiles),
            Point::Trident => rows!(game.tridents),
            Point::RemoteRocket | Point::OwnRocket => rows!(game.rockets),
        }
    }
    struct Case {
        point: Point,
        spawn: Spawn,
        position: fn(&crate::game::ClientGame<'_>) -> Vec3,
    }
    #[test]
    #[ignore = "daily"]
    fn remote_lift_restores_each_nested_spawn_without_ticking_it_early() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.common.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            for case in [
                Case {
                    point: Point::Falling,
                    spawn: falling,
                    position: |g| g.falling.iter().find(|e| e.id == ID).unwrap().position,
                },
                Case {
                    point: Point::Projectile,
                    spawn: projectile,
                    position: |g| {
                        g.creatures
                            .projectiles
                            .iter()
                            .find(|e| e.id == ID)
                            .unwrap()
                            .position
                    },
                },
                Case {
                    point: Point::Trident,
                    spawn: trident,
                    position: |g| g.tridents.iter().find(|e| e.id == ID).unwrap().position,
                },
                Case {
                    point: Point::RemoteRocket,
                    spawn: rocket,
                    position: |g| g.rockets.iter().find(|e| e.id == ID).unwrap().position,
                },
            ] {
                game.clear_remote_projectiles();
                for id in [71, 72, 73] {
                    (case.spawn)(game, id);
                }
                let mut expected = identities(game, case.point, true);
                with_spawn(case.point, case.spawn, || {
                    game.tick_remote_projectiles(|_, at| at)
                });
                assert_eq!(
                    (case.position)(game),
                    AT,
                    "{:?} ticked during its spawn pass",
                    case.point
                );
                let actual = identities(game, case.point, false);
                expected.push(*actual.iter().find(|e| e.0 == ID).unwrap());
                assert_eq!(
                    actual, expected,
                    "{:?} whole-list identity/order",
                    case.point
                );
                game.tick_remote_projectiles(|_, at| at);
                assert!(
                    (case.position)(game).x > AT.x,
                    "{:?} missed the next pass",
                    case.point
                );
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn own_rocket_lift_keeps_a_new_attachment_until_the_next_pass() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "NestedRocket").unwrap();
            game.client.remote.as_mut().unwrap().entity_id = 99;
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(20.0, 80.0, 20.0))
                    .unwrap(),
            );
            game.common.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            for id in [7, 8, 9] {
                own_rocket(game, id);
            }
            let mut expected = identities(game, Point::OwnRocket, true);
            with_spawn(Point::OwnRocket, own_rocket, || game.tick_own_rockets());
            assert_eq!(
                game.rockets.iter().find(|r| r.id == ID).unwrap().position,
                AT
            );
            let actual = identities(game, Point::OwnRocket, false);
            expected.push(*actual.iter().find(|e| e.0 == ID).unwrap());
            assert_eq!(actual, expected);
            game.tick_own_rockets();
            assert_ne!(
                game.rockets.iter().find(|r| r.id == ID).unwrap().position,
                AT
            );
        });
    }
}
