//! Tridents: `TridentItem.releaseUsing` (throw after 10 ticks, or Riptide in
//! water or rain) and `ThrownTrident` (arrow flight with 0.99 water inertia,
//! 8 damage plus Impaling, the reversed bounce off a hit, sticking, Loyalty's
//! return to the owner, pickup) with `ThrownTridentRenderer`, and Channeling's
//! lightning on a hit (`channeling.json`).
use super::commands::Target;
use mod_api::{__private::TransactionFrame, SoundSource};
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_gameplay_common::game::{dragon_common::SPECIAL, level_sounds_common::LevelSound};
use sunrise_sim::{
    entities::arrows::{Pickup, back_off, clip_block_hit},
    join_seq::JoinSeq,
    mobs::{Damage, DamageKind, Species, Who},
    projectiles::Hit,
};

use sunrise_gameplay::trident::breeze_deflect;
use sunrise_sim::thrown_trident::{HALF, ThrownTrident};
/// A player a trident can return to (its owner) or that can pick it up: this
/// game's own, or a server's guest in this level, which is all a server has.
pub(in crate::game) struct Catcher {
    /// The guest's connection; `None` for this game's player.
    pub(in crate::game) guest: Option<u32>,
    /// The guest's entity id (unused for this game's player).
    pub(in crate::game) id: i32,
    pub(in crate::game) feet: Vec3,
    pub(in crate::game) eye: Vec3,
    /// `Player.aiStep` touches things only while alive and not a spectator.
    pub(in crate::game) touches: bool,
    pub(in crate::game) creative: bool,
    pub(in crate::game) uuid: Option<[i32; 4]>,
}
impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn server_in_rain(&mut self, at: Vec3, top: f64) -> bool {
        self.in_rain(at, top)
    }
    /// `TridentItem.releaseUsing`.
    pub fn release_trident(&mut self, ticks: u32) {
        if ticks < 10 {
            return;
        }
        let Some(stack) = self.common.held().cloned() else {
            return;
        };
        let Some(eye) = self.common.eye_position() else {
            return;
        };
        let riptide = stack.enchantment_level("minecraft:riptide");
        // `releaseUsing` checks the wet/passenger conditions, then nextDamageWillBreak again.
        if riptide > 0 && (!self.in_water_or_rain() || self.common.riding.is_some()) {
            return;
        }
        if stack.damage >= self.trident_max_damage(&stack).saturating_sub(1) {
            return;
        }
        let at = self
            .common
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        // Counted on the server before the throw or the spin.
        self.award_stat("used", "minecraft:trident", 1);
        if riptide > 0 {
            self.hurt_held_without_breaking(1);
            // trident_spin_attack_strength: 1.5 + 0.75 per level above the first.
            let spin_stack = self.common.held().cloned();
            let strength = 1.5 + 0.75 * (riptide as f32 - 1.0);
            // TridentItem.releaseUsing: the look in floats from Mth's sine table, scaled to
            // the strength by its float length (Mth.sqrt), then Entity.push.
            use sunrise_core::geometry::trig;
            let (y, x) = (
                f64::from(self.common.yaw * trig::DEG_TO_RAD),
                f64::from(self.common.pitch * trig::DEG_TO_RAD),
            );
            let xd = -trig::sin(y) * trig::cos(x);
            let yd = -trig::sin(x);
            let zd = trig::cos(y) * trig::cos(x);
            let scale = strength / f64::from(xd * xd + yd * yd + zd * zd).sqrt() as f32;
            if let Some(p) = self.common.player.as_mut() {
                p.velocity = Vec3::new(
                    p.velocity.x + f64::from(xd * scale),
                    p.velocity.y + f64::from(yd * scale),
                    p.velocity.z + f64::from(zd * scale),
                );
                // Player.startAutoSpinAttack(20, 8, stack).
                p.auto_spin_attack_ticks = 20;
                p.auto_spin_attack_dmg = 8.0;
                p.auto_spin_attack_item = spin_stack;
            }
            // `player.move(SELF, (0, 1.1999999, 0))` when on the ground: the player's own move
            // (LocalPlayer.move, through the move gate, #2414), which stops at a ceiling and whose
            // end writes the ground state, so the rest of the tick travels in the air.
            if let Some(before) = self
                .common
                .player
                .as_ref()
                .filter(|p| p.ground.on_ground)
                .map(|p| p.position.components())
            {
                let lift = self.move_local_player(
                    sunrise_world::entity_move::MoverType::SelfMovement,
                    Vec3::new(0.0, f64::from(1.1999999f32), 0.0),
                    true,
                );
                if matches!(lift, Ok(Some(_)))
                    && let (Some(conn), Some(body), Some(host)) = (
                        self.server.lan_acting,
                        self.common.player.as_ref(),
                        self.server.lan.as_mut(),
                    )
                {
                    // ServerPlayer inherits Entity.move: commit the lift to its server record,
                    // not as a teleport in leave_guest. The client runs the same move and still
                    // has its own push; it reports its position after travelling.
                    host.hub.update_guest(conn, |g| {
                        // Leave a teleport by a listener for the normal guest turn sync.
                        if !body.teleported && g.position == before {
                            let lifted = body.position.components();
                            g.moved |= g.position != lifted;
                            g.position = lifted;
                            g.on_ground = body.ground.on_ground;
                            g.rests_on_ground = body.collision.below;
                        }
                    });
                }
            }
            let sound = format!("minecraft:item.trident.riptide_{}", riptide.min(3));
            // level.playSound(null, player, sound, PLAYERS, 1, 1).
            match (self.server.lan_acting_entity, at) {
                (Some(player), Some(at)) => {
                    let sound =
                        LevelSound::new(&sound, at, 1.0, 1.0).in_source(SoundSource::Players);
                    self.play_bound_sound(sound, player, None);
                }
                _ => {
                    if let Some(at) = at {
                        self.server_play_sound(&sound, at, 1.0, 1.0);
                    }
                }
            }
            if let (Some(player), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            ) {
                self.native_player_riptide(player, &name);
            }
            return;
        }
        // hurtWithoutBreaking (Unbreaking applies), then the thrown copy.
        self.hurt_held_without_breaking(1);
        let thrown = self.common.held().cloned().unwrap_or(stack);
        if !self.common.creative {
            *self.common.held_mut() = None;
        }
        let start = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let mut projectile = sunrise_sim::projectiles::Projectile::new(
            sunrise_sim::projectiles::ProjectileKind::Snowball,
            start,
            Some(Who::Player),
        );
        let (v, on_ground) = self.common.player.as_ref().map_or((Vec3::ZERO, true), |p| {
            (self.known_player_movement(), p.ground.on_ground)
        });
        let id = self.take_entity_id();
        if let Some(uuid) = projectile.uuid {
            self.common.persist.uuids.insert(id, uuid);
        }
        let owner_uuid = self.acting_player_uuid();
        let mut trident = ThrownTrident {
            size_answer: None,
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            loyalty: 0,
            stack: thrown,
            position: start,
            previous: start,
            velocity: Vec3::ZERO,
            needs_sync: false,
            yaw: 0.0,
            pitch: 0.0,
            previous_yaw: 0.0,
            previous_pitch: 0.0,
            in_ground: false,
            in_ground_time: 0,
            life: 0,
            shake: 0,
            dealt_damage: false,
            no_physics: false,
            returning: 0,
            pickup: if self.common.creative {
                Pickup::CreativeOnly
            } else {
                Pickup::Allowed
            },
            left_owner: false,
            has_been_shot: false,
            owner_uuid: Some(owner_uuid),
            no_gravity: false,
            ground: Default::default(),
            fire_ticks: 0,
            fall_distance: 0.0,
            sound_event: sunrise_sim::entity_nbt::TRIDENT_HIT_GROUND.to_owned(),
            was_touching_water: false,
            age: 0,
            owner: 0,
            guest: self.server.lan_acting,
            random: projectile.random.0.clone(),
            deflected_by: None,
            last_state: None,
        };
        self.resolve_trident_loyalty(&mut trident);
        // The constructor resolves Loyalty before shootFromRotation, on the same entity stream.
        projectile.random.0 = trident.random;
        projectile.shoot_from_rotation(
            self.common.pitch,
            self.common.yaw,
            0.0,
            2.5,
            1.0,
            v,
            on_ground,
        );
        trident.random = projectile.random.0;
        trident.velocity = projectile.velocity;
        trident.yaw = projectile.yaw;
        trident.pitch = projectile.pitch;
        // Projectile.shoot ends with yRotO = getYRot(), xRotO = getXRot().
        trident.previous_yaw = trident.yaw;
        trident.previous_pitch = trident.pitch;
        self.common.tridents.push(trident);
        self.player_projectile_shot();
        // level.playSound(null, trident, sound, PLAYERS, 1, 1).
        let throw = LevelSound::new(
            "minecraft:item.trident.throw",
            [start.x, start.y, start.z],
            1.0,
            1.0,
        )
        .in_source(SoundSource::Players);
        self.play_bound_sound(throw, id, None);
    }
    /// `ThrownTrident.getLoyaltyFromItem`, at spawn and after deserializing.
    pub(in crate::game) fn resolve_trident_loyalty(&mut self, trident: &mut ThrownTrident) {
        if !self.common.is_logical_server() {
            return;
        }
        let ctx = sunrise_items::enchant::EffectContext {
            this: Some("minecraft:trident"),
            this_motion: Some((trident.fall_distance, false)),
            ..Default::default()
        };
        let acceleration = world_stations!(self).enchantments.modify_entity_value(
            &trident.stack.all_enchantments(),
            "minecraft:trident_return_acceleration",
            &ctx,
            &mut self.common.world.random,
            &mut trident.random,
            0.0,
        );
        // Synched data stores the nonnegative acceleration in a signed byte.
        trident.loyalty = (acceleration as i32).clamp(0, i8::MAX as i32) as u32;
    }
    /// `settle_dealt_absorbed` for the player who threw a trident: a LAN guest's own
    /// statistics are only reachable inside `as_guest`, so the host is not credited.
    pub(in crate::game::server_side) fn thrower_dealt_absorbed(&mut self, guest: Option<u32>) {
        let Some(conn) = guest.filter(|c| self.server.lan_acting != Some(*c)) else {
            self.settle_dealt_absorbed(true);
            return;
        };
        let entered = self.as_guest(conn, |g| g.settle_dealt_absorbed(true));
        if entered.is_none() {
            self.settle_dealt_absorbed(false);
        }
    }
    /// The thrower's `PLAYER_HURT_ENTITY` for a trident hit: run as the
    /// guest that threw it, and not at all once they have left.
    pub(in crate::game::server_side) fn thrower_hit(
        &mut self,
        guest: Option<u32>,
        victim: &sunrise_sim::advancements::EntityView,
        direct: sunrise_sim::advancements::EntityView,
        damage: f32,
    ) {
        let Some(conn) = guest.filter(|c| self.server.lan_acting != Some(*c)) else {
            self.player_hit(victim, "minecraft:trident", Some(direct), damage);
            return;
        };
        self.as_guest(conn, |g| {
            g.player_hit(victim, "minecraft:trident", Some(direct), damage)
        });
    }
    /// The connection of the guest whose entity id is `id`.
    pub(in crate::game::server_side) fn trident_guest_conn(&self, id: u32) -> Option<u32> {
        self.server
            .lan
            .as_ref()?
            .hub
            .guests
            .iter()
            .find(|g| g.entity_id as u32 == id)
            .map(|g| g.conn)
    }
    /// `ThrownTrident.onHitEntity` on the guest `conn`: the hurt runs as them and
    /// `ServerPlayer.hurtServer` refuses it when the `pvp` game rule or their
    /// teams forbid it (`canHarmPlayer`); the trident bounces off either way.
    pub(in crate::game::server_side) fn trident_hits_guest(
        &mut self,
        t: &ThrownTrident,
        conn: u32,
        damage: f32,
        attack: sunrise_sim::health::Attack,
    ) -> bool {
        if self.server.lan.is_none() {
            return false;
        }
        let refused = !self.guest_may_harm(t.guest, conn);
        let shooter = self.player_attacker(t.guest);
        let from = [t.previous.x, t.previous.y, t.previous.z];
        let victim = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| g.entity_id as u32);
        let hurt = self.guest_hit_by(conn, |g| {
            g.with_attack(attack.clone(), |g| {
                g.server.combat.0.refuse_player_hit = refused;
                g.server.combat.0.projectile_attacker = shooter;
                let hurt = g.mob_hits_player(damage, from, "minecraft:trident");
                g.server.combat.0.refuse_player_hit = false;
                g.server.combat.0.projectile_attacker = None;
                hurt
            })
        });
        let hurt = hurt.unwrap_or(false);
        if hurt && let Some(victim) = victim {
            let source = attack.player_source("minecraft:trident", attack.attacker);
            self.post_attack(
                victim,
                "minecraft:player",
                &attack,
                source,
                Some(&t.stack),
                true,
            );
        }
        hurt
    }
    /// The players tridents fly back to or land at: this game's player and the
    /// guests in this level. A server has no player of its own, so its
    /// tridents belong to guests.
    pub(in crate::game::server_side) fn trident_catchers(&self) -> Vec<Catcher> {
        let mut out = Vec::new();
        if let Some(eye) = self.common.eye_position() {
            let feet = self
                .common
                .player
                .as_ref()
                .map_or(Vec3::ZERO, |p| p.position);
            out.push(Catcher {
                guest: None,
                id: 0,
                feet,
                eye: Vec3::new(eye[0], eye[1], eye[2]),
                touches: true,
                creative: self.common.creative,
                uuid: self.common.level.player_uuid,
            });
        }
        if let Some(host) = self.server.lan.as_ref() {
            for g in host.hub.guests.iter().filter(|g| host.hub.here(g)) {
                let [x, y, z] = g.position;
                out.push(Catcher {
                    guest: Some(g.conn),
                    id: g.entity_id,
                    feet: Vec3::new(x, y, z),
                    eye: Vec3::new(x, y + g.eye_height(), z),
                    touches: !g.dead && !g.spectator(),
                    creative: g.creative(),
                    uuid: Some(sunrise_sim::lan::playerdata::uuid_ints(g.uuid)),
                });
            }
        }
        out
    }
    /// Notify at the contact with the live trident in its owning slot. Return the live row
    /// and whether to skip the hit (veto or invalidated contact). Only removal ends the tick.
    pub(in crate::game::server_side) fn trident_impact(
        &mut self,
        index: &mut usize,
        t: ThrownTrident,
        hit: Hit,
        through: &mut Option<JoinSeq>,
        transaction: &mut Option<TransactionFrame>,
    ) -> Option<(ThrownTrident, bool)> {
        if !self.native_hears_impacts() {
            return Some((t, false));
        }
        through.get_or_insert_with(JoinSeq::latest);
        let (id, join, guest) = (t.id, t.join, t.guest);
        let motion = |t: &ThrownTrident| {
            (
                t.position,
                t.velocity,
                t.in_ground,
                t.dealt_damage,
                t.no_physics,
                t.owner,
                t.owner_uuid,
                t.guest,
            )
        };
        let before = motion(&t);
        let target = self.native_impact_target(hit);
        self.common.tridents.put_back(*index, t);
        let frame = self.open_storage_transaction();
        *transaction = Some(frame);
        let canceled = self.native_trident_impact(id, guest, hit);
        if self.entity(id).is_some()
            && let Some(at) = self.common.tridents.locate(join)
        {
            let refused = canceled
                || motion(&self.common.tridents[at]) != before
                || target.is_none()
                || target != self.native_impact_target(hit);
            if let Some(live) = self.common.tridents.extract(join) {
                *index = at;
                return Some((live, refused));
            }
        }
        *transaction = None;
        self.close_storage_frame(frame, true);
        if let Some(at) = self.common.tridents.locate(join) {
            *index = at + 1;
        }
        None
    }

    pub fn tick_tridents(&mut self) {
        if !self.common.tridents.is_empty() {
            self.with_late_storage_scope(|game, transaction| {
                game.tick_tridents_in_scope(transaction)
            });
        }
    }

    pub(in crate::game::server_side) fn tick_tridents_in_scope(
        &mut self,
        transaction: &mut Option<TransactionFrame>,
    ) {
        let catchers = self.trident_catchers();
        let ticking = self.entity_ticking_area();
        let mut round = self.native_tick_round();
        // The trident whose tick just ended, whatever ended it: it hears its Post before the
        // next one's Pre.
        let mut told = None;
        // EntityTickList copy-on-iterate: while a listener may remove tridents, the round visits
        // the tridents that were there when it began, each found again by its join.
        let joins: Option<Vec<_>> = round
            .told_listening()
            .then(|| self.common.tridents.iter().map(|t| t.join).collect());
        let mut turns = joins.iter().flatten();
        let mut i = 0;
        let mut through = None;
        loop {
            if let Some(id) = told.take() {
                self.native_tick_post_one(&round, id, "minecraft:trident");
            }
            if joins.is_some() {
                let Some(&join) = turns.next() else { break };
                let Some(at) = self.common.tridents.locate(join) else {
                    continue;
                };
                i = at;
            } else if i >= self.common.tridents.len() {
                break;
            }
            let t = &self.common.tridents[i];
            let target = Target::Trident(t.id);
            if through.is_some_and(|last| t.join > last)
                || self.removal_pending(&target)
                || self.common.persist.removals.contains(&target)
            {
                i += 1;
                continue;
            }
            let at = self.common.tridents[i].position;
            if !ticking(at.x.floor() as i32, at.z.floor() as i32) {
                self.common.tridents[i].previous = at;
                i += 1;
                continue;
            }
            let (id, join) = (self.common.tridents[i].id, self.common.tridents[i].join);
            if !self.native_tick_pre_one(&mut round, id, "minecraft:trident") {
                i += 1;
                continue;
            }
            // The Pre's listener may have removed tridents.
            if self.common.tridents.get(i).is_none_or(|t| t.join != join) {
                let Some(at) = self.common.tridents.locate(join) else {
                    continue;
                };
                i = at;
            }
            // A removal the listener queued: nothing ticks, nothing hears a Post.
            if self.removal_pending(&target) || self.common.persist.removals.contains(&target) {
                i += 1;
                continue;
            }
            told = Some(id);
            let mut t = self.common.tridents.remove(i);
            t.previous = t.position;
            t.previous_yaw = t.yaw;
            t.previous_pitch = t.pitch;
            t.age += 1;
            // AbstractArrow.tick: `if (shakeTime > 0) --shakeTime;`.
            t.shake -= i32::from(t.shake > 0);
            if t.in_ground_time > 4 {
                t.dealt_damage = true;
            }
            // ThrownTrident.tick: Loyalty needs its owner, who is in this level.
            let owner = catchers
                .iter()
                .find(|c| t.owner_uuid.is_some() && c.uuid == t.owner_uuid);
            let owner_eye = owner.map(|c| c.eye);
            let owner_who = owner
                .map(|c| c.guest.map_or(Who::Player, |_| Who::Mob(c.id as u32)))
                .or_else(|| t.owner_uuid.and_then(|uuid| self.who_by_uuid(uuid)));
            if let Some(owner) = owner {
                t.guest = owner.guest;
            }
            // `isAcceptibleReturnOwner` fails for a dead or spectating owner: the
            // trident drops as an item when it can be picked up, then goes.
            if t.loyalty > 0
                && (t.dealt_damage || t.no_physics)
                && owner.is_some_and(|c| !c.touches)
            {
                if t.pickup == Pickup::Allowed {
                    self.spawn_at_location(t.position, f64::from(0.1f32), t.stack.clone());
                }
                continue;
            }
            if t.loyalty > 0
                && (t.dealt_damage || t.no_physics)
                && let Some(owner_eye) = owner_eye
            {
                t.no_physics = true;
                t.in_ground = false;
                let from = t.position;
                t.loyalty_pull(owner_eye);
                // The pull is a position update of its own, before the trident's move.
                let to = t.position;
                self.entity_section_step(t.id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
                if t.returning == 0 {
                    self.server_play_sound(
                        "minecraft:item.trident.return",
                        [t.position.x, t.position.y, t.position.z],
                        10.0,
                        1.0,
                    );
                }
                t.returning += 1;
            }
            // AbstractArrow clears cached water/rain before its stuck branch.
            let in_rain = {
                let rain = self.arrow_rain();
                self.arrow_scene(&rain).in_rain(t.position)
            };
            if t.was_touching_water || in_rain {
                t.fire_ticks = t.fire_ticks.min(0);
            }
            let stuck_tick = t.in_ground && !t.no_physics;
            // AbstractArrow.tick checks this before moving, and only its
            // flight branch reaches Projectile.tick (a stuck arrow does not).
            if !stuck_tick && !t.left_owner {
                let swept = Aabb::new(
                    t.position.add_vector(Vec3::new(-HALF, 0.0, -HALF)),
                    t.position.add_vector(Vec3::new(HALF, 0.5, HALF)),
                )
                .expand_towards(t.velocity)
                .inflate(Vec3::new(1.0, 1.0, 1.0));
                t.left_owner =
                    owner_who.is_none_or(|o| self.outside_owner_collision_range(o, swept));
            }
            if t.in_ground {
                // AbstractArrow.tick: `if (lastState != blockState && shouldFall())
                // startFalling();` (`blockState` is the block at its position).
                // Its `else tickDespawn()` counts `life` and discards at 1200
                // (`ThrownTrident.tickDespawn`: not for a Loyalty trident that
                // can be picked up).
                let mut falls = false;
                if !t.no_physics {
                    let rain = self.arrow_rain();
                    let scene = self.arrow_scene(&rain);
                    let state = scene.block(BlockPos::containing(t.position));
                    falls = t.last_state != Some(state) && t.should_fall(&scene);
                }
                if falls {
                    t.start_falling();
                } else if t.tick_despawn() {
                    // `discard()`: gone for everything after, so no block
                    // effects (`isAlive`) and no pickup this tick.
                    continue;
                }
                t.in_ground_time += 1;
                if !t.no_physics {
                    let at = t.position;
                    self.trident_block_effects(&mut t, at, at);
                }
            } else if t.no_physics {
                t.turn_towards_movement();
                t.position = Vec3::new(
                    t.position.x + t.velocity.x,
                    t.position.y + t.velocity.y,
                    t.position.z + t.velocity.z,
                );
            } else {
                t.turn_towards_movement();
                // AbstractArrow.tick: clip blocks first, then search pickable entities up to end.
                let from = t.position;
                let flight = t.velocity;
                let clip = clip_block_hit(
                    &self.common.world,
                    self.common.res.facts(),
                    self.common.collision,
                    from,
                    flight,
                );
                let end = clip.map_or(from.add_vector(flight), |(at, _, _)| at);
                // ProjectileUtil.computeMargin + NearestHit broad-phase over live targets.
                let margin = sunrise_sim::projectiles::compute_margin(t.age as i32);
                let search_area = Aabb::new(
                    from.add_vector(Vec3::new(-HALF, 0.0, -HALF)),
                    from.add_vector(Vec3::new(HALF, 0.5, HALF)),
                )
                .expand_towards(flight)
                .inflate(Vec3::new(1.0, 1.0, 1.0));
                let mut best = sunrise_sim::projectiles::NearestHit::new();
                if !t.dealt_damage {
                    let query =
                        sunrise_gameplay_common::game::creatures::projectile_targets::TargetQuery {
                            include_guests: true,
                            arrow_friendly_fire: owner_who.map(|o| (o, t.guest)),
                            ..Default::default()
                        };
                    self.visit_projectile_targets(query, &[], |c| {
                        if !t.left_owner
                            && let Some(o) = owner_who
                            && self.in_same_vehicle_tree(o, c.who)
                        {
                            return;
                        }
                        best.consider(c, search_area, from, end, margin);
                    });
                }
                let hit_entity = best.finish();
                // Preserve the captured contact through ProjectileImpactEvent.
                let refused_entity = if let Some((who, at)) = hit_entity {
                    t.position = at;
                    self.trident_block_effects(&mut t, from, at);
                    let hit = Hit::Entity { who, at };
                    let Some((live, refused)) =
                        self.trident_impact(&mut i, t, hit, &mut through, transaction)
                    else {
                        continue;
                    };
                    t = live;
                    refused
                } else {
                    false
                };
                if refused_entity {
                    // No damage, sound or deflection: it flies on from the contact.
                } else if let Some((who, _)) = hit_entity {
                    // `Breeze.deflection`: turns a trident back without dealing damage.
                    let breeze =
                        match who {
                            Who::Mob(id) => self.common.mobs.iter().find(|m| {
                                m.id == id && m.alive() && m.species() == Species::Breeze
                            }),
                            Who::Player => None,
                        };
                    if let Some(breeze) = breeze {
                        let breeze_id = breeze.id;
                        if t.deflected_by != Some(breeze_id) {
                            let sound_at =
                                [breeze.position.x, breeze.position.y, breeze.position.z];
                            self.server_play_sound(
                                "minecraft:entity.breeze.deflect",
                                sound_at,
                                1.0,
                                1.0,
                            );
                            let jitter = t.random.next_float();
                            let (velocity, yaw) = breeze_deflect(t.velocity, t.yaw, jitter);
                            t.velocity = velocity;
                            t.yaw = yaw;
                            // ProjectileDeflection.REVERSE turns the old yaw by the same angle.
                            t.previous_yaw += 170.0 + jitter * 20.0;
                            t.deflected_by = Some(breeze_id);
                        }
                    } else {
                        if let Who::Mob(id) = who {
                            self.redirect_projectile(id, owner_who, t.owner_uuid, t.guest, None);
                        }
                        let victim = self.entity_kind_for_who(who);
                        // The thrower's entity id, resolved while a guest victim acts too.
                        let attacker_id = match owner_who {
                            Some(Who::Mob(oid)) => Some(oid),
                            Some(Who::Player) => Some(
                                self.player_entity_id_by_connection(t.guest)
                                    .map_or(0, |id| id as u32),
                            ),
                            None => None,
                        };
                        let victim_id = match who {
                            Who::Mob(id) => id,
                            Who::Player => self.own_entity_id() as u32,
                        };
                        let causing = attacker_id.map_or(0, |id| id as i32);
                        let mut hurt_mob = None;
                        let mut hurt_player = false;
                        let mut hurt_stand = None;
                        let mut dodged = false;
                        let aim = self.attacker_aim(owner_who);
                        let mut attack =
                            attacker_id.map_or_else(Default::default, |a| self.attack_of(a as i32));
                        attack.projectile = Some(t.id as i32);
                        attack.projectile_kind = sunrise_sim::health::DirectEntity::Arrow;
                        attack.projectile_type = Some("minecraft:trident");
                        attack.enchantment_direct = Some(sunrise_sim::health::EnchantmentActor {
                            id: t.id as i32,
                            kind: "minecraft:trident".into(),
                            motion: None,
                        });
                        self.set_projectile_weapon(&mut attack, Some(&t.stack));
                        // ThrownTrident.getDamage: `EnchantmentHelper.modifyDamage` on 8
                        // (Impaling hits `#sensitive_to_impaling`, the aquatic tag).
                        let mut ctx = attack.enchantment_context(
                            attack.player_source("minecraft:trident", causing),
                            victim.as_deref().unwrap_or(""),
                        );
                        ctx.this = victim.as_deref();
                        let damage =
                            self.evaluate_weapon_damage(&t.stack, Some(victim_id), ctx, 8.0);
                        t.dealt_damage = true;
                        let trident_source = DamageKind::Named(
                            sunrise_sim::mobs::NamedDamage::registered(
                                &self.common.health_resources.damage,
                                "minecraft:trident",
                            )
                            .expect("registered vanilla trident damage"),
                        );
                        match who {
                            Who::Player => {
                                let from_pos = [t.previous.x, t.previous.y, t.previous.z];
                                hurt_player = self.with_attack(attack.clone(), |g| {
                                    g.mob_hits_player(damage, from_pos, "minecraft:trident")
                                });
                            }
                            Who::Mob(id) if self.is_special_target(id) => {
                                let guest = t.guest;
                                let direct = super::dragon::DirectHit::trident(
                                    t.id,
                                    t.position,
                                    (id >= SPECIAL
                                        && !mod_api::__private::mods().is_empty()
                                        && mod_api::__private::enabled_listened(
                                            mod_api::HookId::LivingDeath,
                                        ))
                                    .then_some(&attack),
                                );
                                let hurt = |game: &mut Self| {
                                    game.hit_special_full(
                                        id,
                                        damage,
                                        owner_who,
                                        guest,
                                        trident_source,
                                        Some("minecraft:trident"),
                                        Some(direct),
                                    );
                                };
                                if self.common.is_crystal_id(id) {
                                    // EndCrystal.hurtServer explodes before ThrownTrident.onHitEntity
                                    // deflects. Keep the in-flight owner in the explosion's level walk.
                                    let join = t.join;
                                    self.common.tridents.put_back(i, t);
                                    hurt(self);
                                    let Some(index) = self
                                        .common
                                        .tridents
                                        .locate(join)
                                        .filter(|_| !self.removal_pending(&target))
                                    else {
                                        if let Some(frame) = transaction.take() {
                                            self.close_storage_frame(frame, true);
                                        }
                                        continue;
                                    };
                                    i = index;
                                    t = if transaction.is_some() {
                                        self.common.tridents.extract(join).unwrap()
                                    } else {
                                        self.common.tridents.remove(i)
                                    };
                                } else {
                                    hurt(self);
                                }
                            }
                            Who::Mob(id) => {
                                if let Some(mob_idx) =
                                    self.common.mob_index.find(&self.common.mobs, id)
                                {
                                    let plan = self
                                        .prepare_knockback_credit(id as i32, owner_who, &attack);
                                    // The hit's causing entity, lent to its listeners beside the victim.
                                    let causing = self.causer_id(owner_who);
                                    let decision = crate::game::server_side::entity_server::causer_decision!(
                                        self, causing
                                    );
                                    let mut credit =
                                        plan.lend(&mut self.server.last_hurt_by_player);
                                    let from_pos = Some([t.position.x, t.position.y, t.position.z]);
                                    let join = self.common.mobs[mob_idx].join;
                                    let (victim, lend) = crate::game::server_side::entity_server::victim_and_causer!(
                                        self, mob_idx, causing, decision
                                    );
                                    let hurt = victim.damage_projectile(
                                        crate::game::server_side::entity_server::damage_level_of!(
                                            self,
                                            &attack,
                                            &mut credit,
                                            lend
                                        ),
                                        damage,
                                        Damage {
                                            kind: trident_source,
                                            attacker: owner_who,
                                            from: from_pos,
                                            direct: Some("minecraft:trident"),
                                            attacker_species: None,
                                        },
                                        aim,
                                        [t.velocity.x, t.velocity.z],
                                    );
                                    if let Some((mob_idx, hurt)) =
                                        self.complete_mob_hurt(mob_idx, hurt)
                                    {
                                        self.finish_mob_damage_at(mob_idx);
                                        if hurt {
                                            hurt_mob = Some((id, join));
                                        } else if self
                                            .common
                                            .mobs
                                            .iter()
                                            .find(|m| m.id == id && m.join == join && !m.removed)
                                            .is_some_and(|m| m.species() == Species::Enderman)
                                        {
                                            // Enderman.projectileReceivesSideEffectsOnHit(wasHurt) == wasHurt.
                                            dodged = true;
                                        }
                                    }
                                    if owner_who.is_some() {
                                        self.thrower_dealt_absorbed(t.guest);
                                    }
                                    self.settle_mob_hurt(id);
                                    let _ = self.settle_hurt_teleports();
                                } else {
                                    if owner_who.is_some() {
                                        self.thrower_dealt_absorbed(t.guest);
                                    }
                                    if let Some(conn) = self.trident_guest_conn(id) {
                                        self.trident_hits_guest(&t, conn, damage, attack.clone());
                                    } else {
                                        let stand_pos = self
                                            .common
                                            .armor_stands
                                            .iter()
                                            .find(|a| a.id == id)
                                            .map(|a| a.position.components());
                                        let info = sunrise_gameplay_common::game::creatures::projectile_targets::ProjectileHitInfo {
                                            direct_id: t.id,
                                            owner: owner_who,
                                            guest: t.guest,
                                            damage_type: "minecraft:trident",
                                            amount: damage,
                                            velocity: t.velocity,
                                            position: t.previous,
                                            is_arrow: true,
                                            burning_arrow: false,
                                            ignite_seconds: None,
                                        };
                                        if self.projectile_hurts_non_mob(&info, id)
                                            == sunrise_gameplay_common::game::creatures::projectile_targets::NonMobHit::Hurt
                                            && let Some(pos) = stand_pos
                                        {
                                            hurt_stand = Some((id, pos));
                                        }
                                    }
                                }
                            }
                        }
                        // `doPostAttackEffectsWithItemSourceOnBreak`: the victim's equipment,
                        // then the trident's own effects, Channeling's among them. A guest
                        // player's continuation ran inside the hit, in their own turn.
                        let source = sunrise_sim::health::HurtSource {
                            damage_type: "minecraft:trident",
                            causing,
                            direct: t.id as i32,
                            direct_type: Some("minecraft:trident"),
                            position: Some([t.previous.x, t.previous.y, t.previous.z]),
                        };
                        let hurt_living = hurt_mob
                            .map(|(id, _)| (id, victim.clone()))
                            .or_else(|| {
                                hurt_stand
                                    .map(|(id, _)| (id, Some("minecraft:armor_stand".to_owned())))
                            })
                            .or_else(|| {
                                hurt_player.then(|| {
                                    (
                                        self.own_entity_id() as u32,
                                        Some("minecraft:player".to_owned()),
                                    )
                                })
                            });
                        if let Some((id, Some(kind))) = hurt_living {
                            self.post_attack(id, &kind, &attack, source, Some(&t.stack), true);
                        }
                        if owner_who.is_some()
                            && let Some((id, join)) = hurt_mob
                            && let Some(m) = self
                                .common
                                .mobs
                                .iter()
                                .find(|m| m.id == id && m.join == join && !m.removed)
                        {
                            // LivingEntity.hurtServer: PLAYER_HURT_ENTITY.
                            let victim = self.mob_entity(m);
                            let direct = sunrise_sim::advancements::EntityView {
                                kind: "minecraft:trident".into(),
                                position: [t.position.x, t.position.y, t.position.z],
                                ..Default::default()
                            };
                            self.thrower_hit(t.guest, &victim, direct, damage);
                        }
                        // ThrownTrident.onHitEntity: REVERSE deflection (RNG draw + yaw) and sound
                        // run before Projectile.onHit posts PROJECTILE_LAND.
                        if !dodged {
                            let rot = 170.0 + t.random.next_float() * 20.0;
                            t.yaw += rot;
                            t.velocity = Vec3::new(
                                t.velocity.x * (-0.5 * 0.02),
                                t.velocity.y * (-0.5 * 0.2),
                                t.velocity.z * (-0.5 * 0.02),
                            );
                            self.server_play_sound(
                                "minecraft:item.trident.hit",
                                [t.position.x, t.position.y, t.position.z],
                                1.0,
                                1.0,
                            );
                        }
                        self.projectile_landed_event(
                            t.id,
                            owner_who,
                            [t.position.x, t.position.y, t.position.z],
                        );
                    }
                } else {
                    // Move to contact and apply block effects before ProjectileImpactEvent or onHitBlock.
                    let contact = clip.map_or(from.add_vector(flight), |(at, _, _)| at);
                    t.position = contact;
                    self.trident_block_effects(&mut t, from, contact);
                    // Only onHitBlock backs away from the clipped point.
                    let refused_block = if let Some((at, block, face)) = clip {
                        let hit = Hit::Block {
                            pos: [block.x, block.y, block.z],
                            face,
                            at,
                        };
                        let Some((live, refused)) =
                            self.trident_impact(&mut i, t, hit, &mut through, transaction)
                        else {
                            continue;
                        };
                        t = live;
                        refused
                    } else {
                        false
                    };
                    if !refused_block && let Some((at, block, _)) = clip {
                        let stop = back_off(at, flight);
                        // 0.05 back along signum(movement), stopped, stuck, shaking.
                        t.position = stop;
                        t.velocity = Vec3::ZERO;
                        // Projectile.onHit on a block: PROJECTILE_LAND at block center with post-callback state.
                        self.projectile_block_landed_event(
                            t.id,
                            owner_who,
                            [block.x, block.y, block.z],
                        );
                        t.in_ground = true;
                        t.last_state = Some(self.common.world.block(block.x, block.y, block.z));
                        t.shake = 7;
                        let sound = t.hit_ground_sound();
                        self.server_play_sound(&sound, [stop.x, stop.y, stop.z], 1.0, 1.0);
                        let (id, join) = (t.id, t.join);
                        let holder = owner_who.and_then(|owner| match owner {
                            Who::Player => self
                                .player_entity_id_by_connection(t.guest)
                                .map(|id| id as u32),
                            Who::Mob(id) => Some(id),
                        });
                        let stack = t.stack.clone();
                        if transaction.is_some() {
                            self.common.tridents.put_back(i, t);
                        } else {
                            self.common.tridents.insert(i, t);
                        }
                        self.hit_block_enchantment_effects(
                            &stack,
                            holder,
                            id,
                            "minecraft:trident",
                            at,
                            [block.x, block.y, block.z],
                        );
                        let Some(row) = self.common.tridents.locate(join) else {
                            if let Some(frame) = transaction.take() {
                                self.close_storage_frame(frame, true);
                            }
                            continue;
                        };
                        i = row;
                        t = if transaction.is_some() {
                            self.common
                                .tridents
                                .extract(join)
                                .expect("the trident was located")
                        } else {
                            self.common.tridents.remove(row)
                        };
                    }
                }
                if !t.in_ground {
                    // AbstractArrow.tick scales by 0.99f then applyGravity adds (0, -0.05, 0).
                    let drag = f64::from(0.99f32);
                    t.velocity = t.velocity.scale(drag);
                    if !t.no_gravity {
                        t.velocity = t.velocity.add_vector(Vec3::new(0.0, -0.05, 0.0));
                    }
                }
            }
            if !t.no_physics {
                let half = 0.25;
                let bounds = Aabb::new(
                    t.position.add_vector(Vec3::new(-half, 0.0, -half)),
                    t.position.add_vector(Vec3::new(half, 0.5, half)),
                );
                self.entity_powder_snow_particles(
                    t.previous, t.position, bounds, t.velocity, false,
                );
            }
            // The stuck branch returns before Entity.baseTick. A flight that
            // just hit a block still reaches it, as does a noPhysics return.
            if !stuck_tick {
                t.has_been_shot = true;
                let rain = self.arrow_rain();
                t.was_touching_water = self.arrow_scene(&rain).in_water(t.position);
                // Entity.updateInWaterStateAndDoFluidPushing: resetFallDistance in water.
                if t.was_touching_water {
                    t.fall_distance = 0.0;
                }
                if t.fire_ticks > 0 {
                    t.fire_ticks -= 1;
                }
            }
            // playerTouch: stuck or returning tridents go back to a player.
            // ThrownTrident.playerTouch: only its owner takes it, or anyone
            // once the owner is not in this level (`getOwner() == null`), and
            // not while `AbstractArrow.shakeTime` still runs.
            if (t.in_ground || t.no_physics) && t.shake <= 0 {
                let near = |c: &&Catcher| {
                    c.touches
                        && (owner_who.is_none() || c.uuid == t.owner_uuid)
                        && (t.position.x - c.feet.x).abs() < 1.0
                        && (t.position.z - c.feet.z).abs() < 1.0
                        && t.position.y > c.feet.y - 1.0
                        && t.position.y < c.feet.y + 2.5
                };
                if let Some(by) = catchers.iter().find(near)
                    && self.trident_taken(&t, by)
                {
                    if let Some(frame) = transaction.take() {
                        self.common.tridents.retire_extracted(t.id, t.join);
                        self.close_storage_frame(frame, true);
                    }
                    continue;
                }
            }
            let (id, from, to) = (t.id, t.previous, t.position);
            if transaction.is_some() {
                self.common.tridents.put_back(i, t);
            } else {
                self.common.tridents.insert(i, t);
            }
            self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
            if let Some(frame) = transaction.take() {
                self.close_storage_frame(frame, true);
            }
            i += 1;
        }
        if let Some(id) = told {
            self.native_tick_post_one(&round, id, "minecraft:trident");
        }
    }
    /// Apply the environment before impact callbacks can change the world.
    pub(in crate::game::server_side) fn trident_block_effects(
        &mut self,
        t: &mut ThrownTrident,
        from: Vec3,
        to: Vec3,
    ) {
        let extinguished = {
            let rain = self.arrow_rain();
            // Block-contact hooks are already delivered by the game's inside pass.
            let (ticks, extinguished) =
                self.arrow_scene(&rain)
                    .apply_block_effects(from, to, t.fire_ticks, |_, _, _| {});
            t.fire_ticks = ticks;
            extinguished
        };
        if extinguished {
            let pitch = 1.6 + (t.random.next_float() - t.random.next_float()) * 0.4;
            self.server_play_sound(
                "minecraft:entity.generic.extinguish_fire",
                [to.x, to.y, to.z],
                0.7,
                pitch,
            );
        }
    }
    /// `ThrownTrident.tryPickup` for a trident that has landed or is
    /// returning: the player takes the item (a creative thrower's trident is
    /// only removed) and the rest that does not fit drops at their feet.
    /// False when it could not be given.
    pub(in crate::game::server_side) fn trident_taken(
        &mut self,
        t: &ThrownTrident,
        by: &Catcher,
    ) -> bool {
        let creative_pickup = t.pickup == Pickup::CreativeOnly && by.creative;
        let returning_to_owner = t.no_physics && t.owner_uuid.is_some() && t.owner_uuid == by.uuid;
        if !creative_pickup {
            if t.pickup != Pickup::Allowed && !returning_to_owner {
                return false;
            }
            let rest = match by.guest {
                None => self.add_stack_to_inventory(t.stack.clone()),
                Some(conn) => {
                    let rest = self.as_guest(conn, |g| g.add_stack_to_inventory(t.stack.clone()));
                    let Some(rest) = rest else {
                        return false;
                    };
                    rest
                }
            };
            if let Some(rest) = rest {
                self.spawn_item(rest, [by.feet.x, by.feet.y, by.feet.z], [0.0; 3], 0);
            }
        }
        // Player.take, for a creative thrower's trident too: the clients near
        // play the pickup sound and fly the trident to its collector.
        if by.guest.is_some()
            && let Some(host) = self.server.lan.as_ref()
        {
            let at = [t.position.x, t.position.y, t.position.z];
            host.hub.take_item(t.id as i32, by.id, 1, at);
        }
        if by.guest.is_none() {
            let pitch = ((self.common.world.random.next_float()
                - self.common.world.random.next_float())
                * 0.7
                + 1.0)
                * 2.0;
            self.server_play_sound(
                "minecraft:entity.item.pickup",
                [by.feet.x, by.feet.y, by.feet.z],
                0.2,
                pitch,
            );
        }
        true
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
use sunrise_gameplay_common::game::transactions::entities::EntityRemovalGameExt as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

use sunrise_gameplay_common::game::piston_push::PistonPushGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
