use super::{Hit, dragon_common::FightWho};
use std::sync::Arc;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_gameplay_common::game::fireworks::FIREWORK_EXPLODE_EVENT;
use sunrise_gameplay_common::game::fireworks::Rocket;
use sunrise_gameplay_common::game::fireworks::blast_damage;
use sunrise_gameplay_common::game::fireworks::fireworks_of;
use sunrise_gameplay_common::game::fireworks::rocket_draws;
use sunrise_gameplay_common::game::fireworks::use_on_passes;
#[allow(unused_imports)]
use {
    sunrise_game_data::text::Text, sunrise_items::firework::FireworkExplosion,
    sunrise_items::items::ItemStack, sunrise_level::collisions::SceneCollisions,
    sunrise_level::random_source, sunrise_sim::mobs::Damage, sunrise_sim::mobs::Species,
    sunrise_sim::mobs::Who, sunrise_sim::projectiles::Hit as ProjectileHit,
    sunrise_sim::projectiles::ProjectileWorld,
};

#[path = "fireworks/impact_server.rs"]
pub(in crate::game) mod impact;

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn advance_rocket(&mut self, r: &mut Rocket) -> ProjectileHit {
        // Entity.commonTick's setOldPosAndRot: where updateRotation eases from.
        r.previous = r.position;
        r.previous_rotation = r.rotation;
        // Projectile.tick: the first tick posts the shot, with its owner.
        if !r.has_been_shot {
            let at = [r.position.x, r.position.y, r.position.z];
            self.projectile_shoot_event(r.owner, r.guest, at);
            r.has_been_shot = true;
        }
        // Projectile.tick: whether it has left its owner's vehicle tree, before it moves.
        let owner = self.rocket_owner(r);
        if !r.left_owner {
            let reach = r
                .bounds()
                .expand_towards(r.velocity)
                .inflate(Vec3::new(1.0, 1.0, 1.0));
            r.left_owner = owner.is_none_or(|o| self.outside_owner_collision_range(o, reach));
        }
        // Entity.baseTick (updateFluidInteraction): water resets the fall,
        // entering it after the first tick splashes, currents push it; lava
        // halves the fall.
        let before = r.velocity;
        let (water, lava, _) = sunrise_sim::entities::update_entity_fluids(
            r.bounds(),
            r.position,
            r.position.y + f64::from(0.2125_f32),
            &mut r.velocity,
            &self.common.world,
            &self.common.res.motion().fluids,
            &self.common.loop_context,
        );
        if water > 0.0 {
            r.fall_distance = 0.0;
            if !r.was_touching_water && !r.first_tick {
                self.rocket_splash(r, before);
            }
        }
        r.was_touching_water = water > 0.0;
        // Entity.isInLava is false on the first tick (firstTick).
        if lava > 0.0 && !r.first_tick {
            r.fall_distance *= 0.5;
        }
        r.first_tick = false;
        let hit;
        if r.attached
            && let Some(conn) = r.guest
        {
            // A guest's rocket follows them and sweeps their motion (`FireworkRocketEntity.tick`).
            let guest = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn))
                .map(|g| {
                    (
                        Vec3::new(g.position[0], g.position[1], g.position[2]),
                        g.motion.as_deref().map_or(Vec3::ZERO, |m| m.velocity),
                    )
                });
            if let Some((pos, vel)) = guest {
                r.position = pos;
                r.velocity = vel;
            }
            hit = self.rocket_struck(r, r.velocity, owner);
        } else if r.attached {
            self.follow_own_player(r);
            hit = self.rocket_struck(r, r.velocity, owner);
        } else {
            // getHitResultOnMoveVector before the move.
            hit = self.rocket_struck(r, r.next_motion(), owner);
            let mut scene = SceneCollisions {
                world: &self.common.world,
                binding: self.common.collision,
                subject: None,
            };
            if let Some(flight) = r.free_flight(&mut scene, &self.common.loop_context) {
                self.rocket_moved(r, flight, water > 0.0);
            }
        }
        self.entity_powder_snow_particles(r.previous, r.position, r.bounds(), r.velocity, false);
        hit
    }
    pub fn new_rocket(
        &mut self,
        at: Vec3,
        stack: &ItemStack,
        attached: bool,
        own: &mut sunrise_core::random::LegacyRandom,
    ) -> Rocket {
        let (flight, explosions) = fireworks_of(stack);
        let id = self.take_entity_id();
        self.common
            .persist
            .uuids
            .insert(id, sunrise_sim::save::insecure_uuid(own));
        let (velocity, lifetime) = rocket_draws(own, flight);
        Rocket {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            previous_rotation: [0.0, 0.0],
            join: Default::default(),
            id,
            position: at,
            previous: at,
            velocity,
            life: 0,
            lifetime,
            explosions: explosions.into(),
            attached,
            guest: None,
            owner: None,
            owner_uuid: None,
            left_owner: false,
            last_deflected_by: None,
            attached_to: None,
            angled: false,
            item: Arc::new(stack.clone()),
            random: own.clone(),
            ground: Default::default(),
            has_been_shot: false,
            silent: false,
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            was_touching_water: false,
            first_tick: true,
        }
    }
    /// `FireworkRocketItem.use` while gliding: drop the player's leads, then a
    /// rocket attached to the player.
    pub fn boost_firework(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:firework_rocket"
            || !self.common.player.as_ref().is_some_and(|p| p.fall_flying)
        {
            return false;
        }
        // Server path of `use`: `dropAllLeashConnections(null)`, and when that
        // drops anything `playSound(null, player, LEAD_BREAK, NEUTRAL, 1, 1)`.
        // The entity overload follows the player and excludes nobody, including
        // the guest whose action is running while its hub is swapped out.
        if self.drop_all_leash_connections() {
            self.play_player_bound_sound("minecraft:item.lead.break", 6, 1.0, 1.0, false);
        }
        let at = self
            .common
            .player
            .as_ref()
            .map_or(Vec3::ZERO, |p| p.position);
        let mut rocket = self.new_rocket(at, stack, true, &mut random_source::entity());
        rocket.owner = Some(Who::Player);
        rocket.guest = self.server.lan_acting;
        self.common.rockets.push(rocket);
        if !self.common.creative {
            self.consume_held();
        }
        self.award_stat("used", "minecraft:firework_rocket", 1);
        self.swing_arm();
        if let (Some(player), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) {
            self.native_player_elytra_boost(player, &name);
        }
        true
    }
    /// `FireworkRocketEntity.explode` and `dealExplosionDamage`.
    pub fn explode_rocket(&mut self, r: &Rocket) {
        let at = [r.position.x, r.position.y, r.position.z];
        // FireworkRocketEntity.explode broadcasts before discard. Send now:
        // tick_lan removes missing tracked entities before flushing effects.
        if r.id != 0
            && let Some(host) = &self.server.lan
        {
            host.hub.entity_event(r.id as i32, FIREWORK_EXPLODE_EVENT);
        }
        // `this.gameEvent(GameEvent.EXPLODE, this.getOwner())`, before the blast.
        self.rocket_explode_event(r.owner, r.guest, at);
        if !r.explosions.is_empty() {
            let damage = 5.0 + r.explosions.len() as f32 * 2.0;
            let from = r.position;
            // `attachedToEntity` takes the full damage first, before the AABB LivingEntity sweep.
            let boosted = r.guest.filter(|_| r.attached);
            if r.attached && boosted.is_none() {
                self.rocket_hurt_player(from, damage);
            }
            if let Some(conn) = boosted {
                self.as_guest(conn, |g| g.rocket_hurt_player(from, damage));
            }
            let reach = r.bounds().inflate(Vec3::new(5.0, 5.0, 5.0));
            enum SplashTarget {
                Host,
                Guest(u32),
                Mob(u32),
                Dragon,
                Stand(u32),
            }
            let mut candidates: Vec<(sunrise_sim::projectiles::TargetOrder, SplashTarget)> =
                Vec::new();
            if !(r.attached && boosted.is_none())
                && !self.common.spectator()
                && let Some(p) = self.player_view()
                && p.bounds.intersects(reach)
            {
                let pos = self
                    .common
                    .player
                    .as_ref()
                    .map_or(Vec3::ZERO, |pl| pl.position);
                candidates.push((
                    sunrise_sim::projectiles::TargetOrder::entity(pos, 0),
                    SplashTarget::Host,
                ));
            }
            if let Some(host) = self.server.lan.as_ref() {
                for g in &host.hub.guests {
                    if host.hub.here(g)
                        && !g.spectator()
                        && Some(g.conn) != boosted
                        && super::lan_mobs::guest_bounds(g).intersects(reach)
                    {
                        let pos = Vec3::new(g.position[0], g.position[1], g.position[2]);
                        candidates.push((
                            sunrise_sim::projectiles::TargetOrder::entity(pos, g.entity_id as u64),
                            SplashTarget::Guest(g.conn),
                        ));
                    }
                }
            }
            for m in self.common.mobs.iter() {
                if !m.removed && m.bounds().intersects(reach) {
                    candidates.push((
                        sunrise_sim::projectiles::TargetOrder::entity(m.position, m.join.get()),
                        SplashTarget::Mob(m.id),
                    ));
                }
            }
            if let Some(d) = self.common.dragon.as_ref().filter(|d| d.alive())
                && d.body_bounds().intersects(reach)
            {
                candidates.push((
                    sunrise_sim::projectiles::TargetOrder::entity(d.position, u64::from(d.id)),
                    SplashTarget::Dragon,
                ));
            }
            for a in self.common.armor_stands.iter() {
                if a.bounds().intersects(reach) {
                    candidates.push((
                        sunrise_sim::projectiles::TargetOrder::entity(a.position, a.join.get()),
                        SplashTarget::Stand(a.id),
                    ));
                }
            }
            candidates.sort_by_key(|(order, _)| *order);
            let owner = r.owner;
            let by = (owner == Some(Who::Player)).then(|| {
                r.guest
                    .map_or_else(|| self.acting_player(), FightWho::Guest)
            });
            let causing = match owner {
                Some(Who::Player) => Some(r.guest.map_or(
                    super::commands::Target::Player,
                    super::commands::Target::Guest,
                )),
                Some(Who::Mob(id)) => Some(super::commands::Target::Mob(id)),
                None => None,
            };
            for (_, target) in candidates {
                match target {
                    SplashTarget::Host => self.rocket_blast_player(from, damage),
                    SplashTarget::Guest(conn) => {
                        self.as_guest(conn, |g| g.rocket_blast_player(from, damage));
                    }
                    SplashTarget::Mob(id) => {
                        let hit = {
                            let world = self.mob_world_base(&[]);
                            self.common
                                .mobs
                                .iter()
                                .find(|m| m.id == id && !m.removed)
                                .and_then(|m| {
                                    let d2 = (m.position.x - from.x).powi(2)
                                        + (m.position.y - from.y).powi(2)
                                        + (m.position.z - from.z).powi(2);
                                    let sees = d2 <= 25.0
                                        && (0..2).any(|step| {
                                            let to = Vec3::new(
                                                m.position.x,
                                                m.position.y
                                                    + f64::from(m.height()) * 0.5 * f64::from(step),
                                                m.position.z,
                                            );
                                            world.clear_between(from, to, false)
                                        });
                                    sees.then(|| (m.id, blast_damage(damage, from, m.position)))
                                })
                        };
                        if let Some(hit) = hit {
                            let hits = [hit];
                            if let Some(conn) = r.guest.filter(|_| owner.is_some()) {
                                let ran = self
                                    .as_guest(conn, |g| g.rocket_hurt_mobs(&hits, r.id, owner, at))
                                    .is_some();
                                if ran
                                    && let Some(host) = self.server.lan.as_mut()
                                    && host
                                        .hub
                                        .guest(conn)
                                        .is_some_and(|guest| guest.participates_online())
                                    && self.common.mobs.iter().any(|m| m.id == hit.0)
                                {
                                    host.mob_players.insert(hit.0, conn);
                                }
                                if !ran {
                                    self.rocket_hurt_mobs(&hits, r.id, owner, at);
                                }
                            } else {
                                self.rocket_hurt_mobs(&hits, r.id, owner, at);
                            }
                        }
                    }
                    SplashTarget::Dragon => {
                        self.rocket_blast_dragon(from, damage, reach, by, r.id, owner);
                    }
                    SplashTarget::Stand(id) => {
                        let sees = {
                            let world = self.mob_world_base(&[]);
                            self.common
                                .armor_stands
                                .iter()
                                .find(|a| a.id == id)
                                .is_some_and(|a| {
                                    let d2 = (a.position.x - from.x).powi(2)
                                        + (a.position.y - from.y).powi(2)
                                        + (a.position.z - from.z).powi(2);
                                    if d2 > 25.0 {
                                        return false;
                                    }
                                    let bounds = a.bounds();
                                    let height = bounds.max_position().y - bounds.min_position().y;
                                    (0..2).any(|step| {
                                        let to = Vec3::new(
                                            a.position.x,
                                            a.position.y + height * 0.5 * f64::from(step),
                                            a.position.z,
                                        );
                                        world.clear_between(from, to, false)
                                    })
                                })
                        };
                        if sees {
                            self.hurt_armor_stand(
                                id,
                                "minecraft:fireworks",
                                causing,
                                r.id,
                                Some(at),
                            );
                        }
                    }
                }
            }
        }
        self.native_firework_explode(r.id, at, r.explosions.len());
    }
    /// A rocket shot at an angle (`DATA_SHOT_AT_ANGLE`): out of a dispenser
    /// (`FireworkRocketItem.asProjectile`) or a crossbow.
    ///
    /// `shoot` gives the shot's velocity from the rocket's own random, after the
    /// constructor's draws (`asProjectile`, then `spawnProjectileUsingShoot`).
    pub(in crate::game::server_side) fn launch_angled_rocket(
        &mut self,
        at: Vec3,
        stack: &ItemStack,
        owner: Option<Who>,
        shoot: impl FnOnce(&mut sunrise_core::random::LegacyRandom) -> Vec3,
    ) {
        let mut own = random_source::entity();
        let mut rocket = self.new_rocket(at, stack, false, &mut own);
        rocket.velocity = shoot(&mut own);
        rocket.turn_to_shot();
        rocket.angled = true;
        rocket.owner = owner;
        rocket.guest = self
            .server
            .lan_acting
            .filter(|_| owner == Some(Who::Player));
        self.common.rockets.push(rocket);
    }
    /// `FireworkRocketItem.useOn`: launch from the clicked point.
    pub fn launch_firework(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:firework_rocket"
            || use_on_passes(self.common.player.as_ref().is_some_and(|p| p.fall_flying))
        {
            return false;
        }
        let at = Vec3::new(
            hit.point[0] + f64::from(hit.face[0]) * 0.15,
            hit.point[1] + f64::from(hit.face[1]) * 0.15,
            hit.point[2] + f64::from(hit.face[2]) * 0.15,
        );
        let mut rocket = self.new_rocket(at, stack, false, &mut random_source::entity());
        rocket.owner = Some(Who::Player);
        rocket.guest = self.server.lan_acting;
        self.common.rockets.push(rocket);
        if !self.common.creative {
            self.consume_held();
        }
        // `ItemStack.useOn` awards ITEM_USED for `FireworkRocketItem.useOn`'s SUCCESS.
        self.award_stat("used", "minecraft:firework_rocket", 1);
        self.swing_arm();
        true
    }
    /// `FireworkRocketEntity.dealExplosionDamage` on a player that is not the rocket's
    /// `attachedToEntity`: within 5 blocks, when the rocket sees its feet or
    /// its middle (`ClipContext.Block.COLLIDER`, no fluids).
    pub(in crate::game::server_side) fn rocket_blast_player(&mut self, from: Vec3, damage: f32) {
        // The splash query uses EntitySelector.NO_SPECTATORS. The attached
        // target is hurt directly, without that query's exclusion.
        if self.common.spectator() {
            return;
        }
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let feet = p.position;
        let d2 = (feet.x - from.x).powi(2) + (feet.y - from.y).powi(2) + (feet.z - from.z).powi(2);
        if d2 > 25.0 {
            return;
        }
        let height = f64::from(p.dimensions.height);
        let sees = {
            let world = self.mob_world_base(&[]);
            (0..2).any(|step| {
                let to = Vec3::new(feet.x, feet.y + height * 0.5 * f64::from(step), feet.z);
                world.clear_between(from, to, false)
            })
        };
        if sees {
            self.rocket_hurt_player(from, blast_damage(damage, from, feet));
        }
    }
    /// `FireworkRocketEntity.explode`'s game event: EXPLODE at the rocket, from
    /// its owner (`Projectile.getOwner`), a guest's rocket as theirs.
    pub(in crate::game::server_side) fn rocket_explode_event(
        &mut self,
        mut owner: Option<Who>,
        guest: Option<u32>,
        at: [f64; 3],
    ) {
        if let Some(conn) = guest.filter(|conn| self.server.lan_acting != Some(*conn)) {
            let posted = self
                .as_guest(conn, |g| {
                    let ctx = g.who_context(owner, None);
                    g.post_game_event("minecraft:explode", at, ctx);
                })
                .is_some();
            if posted {
                return;
            }
            // An owner who left cannot resolve to the host: no source entity.
            owner = None;
        }
        let ctx = self.who_context(owner, None);
        self.post_game_event("minecraft:explode", at, ctx);
    }
    pub fn rocket_hurt_mobs(
        &mut self,
        hits: &[(u32, f32)],
        rocket_id: u32,
        attacker: Option<Who>,
        at: [f64; 3],
    ) {
        let mut attack = self.attack_of_who(attacker);
        attack.projectile = Some(rocket_id as i32);
        attack.enchantment_direct = Some(sunrise_sim::health::EnchantmentActor {
            id: rocket_id as i32,
            kind: "minecraft:firework_rocket".into(),
            motion: None,
        });
        attack.projectile_kind = sunrise_sim::health::DirectEntity::Projectile;
        attack.projectile_type = Some("minecraft:firework_rocket");
        for &(id, amount) in hits {
            let plan = self.prepare_knockback_credit(id as i32, attacker, &attack);
            // The hit's causing entity, lent to its listeners beside the victim.
            let causing = self.causer_id(attacker);
            let decision = crate::game::entity_server::causer_decision!(self, causing);
            let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
            if let Some(index) = self.common.mob_index.find(&self.common.mobs, id) {
                let before = self.common.mobs[index].health;
                let join = self.common.mobs[index].join;
                let (victim, lend) =
                    crate::game::entity_server::victim_and_causer!(self, index, causing, decision);
                let hurt = victim.damage(
                    crate::game::entity_server::damage_level_of!(self, &attack, &mut credit, lend),
                    amount,
                    Damage::fireworks(attacker, at),
                );
                let Some((index, hurt)) = self.complete_mob_hurt(index, hurt) else {
                    continue;
                };
                // LivingEntity.actuallyHurt posts after health loss, before death settlement.
                if self.common.mobs[index].health < before {
                    let pos = self.common.mobs[index].position.components();
                    let ctx = self.who_context(Some(Who::Mob(id)), None);
                    self.post_game_event("minecraft:entity_damage", pos, ctx);
                }
                // LivingEntity.hurtServer: a player's rocket makes them the
                // last player to hurt it (`setLastHurtByPlayer`), so its death
                // message names them and their item.
                if let Some(index) = self.common.mob_index.find(&self.common.mobs, id)
                    && self.common.mobs[index].join == join
                {
                    if hurt && attacker == Some(Who::Player) {
                        self.remember_mob_player_credit(id);
                    }
                    self.finish_mob_damage_at(index);
                }
            }
            self.settle_mob_hurt(id);
        }
    }
    /// `hurtServer` with `damageSources().fireworks` from the rocket at
    /// `from`: the rocket is the direct entity, so the player is knocked away
    /// from it and the death message names it; creative and spectator
    /// players are immune.
    pub(in crate::game::server_side) fn rocket_hurt_player(&mut self, from: Vec3, amount: f32) {
        // Ability, spectator and death refusals belong to hurtServer, after
        // its invulnerability observation (also for an attached target).
        // No mob ever owns a rocket.
        self.server.combat.0.projectile_owner = None;
        self.server.combat.0.rocket_hit = true;
        self.server.combat.0.projectile_direct =
            Some(Text::translate("entity.minecraft.firework_rocket", &[]));
        let at = [from.x, from.y, from.z];
        self.hurt_player_from(
            amount,
            Some(at),
            "minecraft:fireworks",
            sunrise_gameplay_common::game::creatures::KnownCause::None,
            false,
        );
    }
    /// `Entity.move` after a free rocket's move (`from`, the move it asked
    /// for, the one it made): a move of a block or more casts for a reset
    /// block or water, `checkFallDamage` adds the drop outside water and on
    /// the ground posts HIT_GROUND after a fall and resets it; then
    /// `applyEffectsFromBlocks`: a cobweb or powder snow makes it stuck and
    /// resets the fall, as does a bubble column (whose push the rocket's
    /// `setDeltaMovement(movement2)` undoes).
    pub(in crate::game::server_side) fn rocket_moved(
        &mut self,
        r: &mut Rocket,
        (from, wanted, moved, restored): (Vec3, Vec3, Vec3, Vec3),
        water: bool,
    ) {
        let movement = r.flight_contact(self.common.body_motion(&game_side!(self)), wanted, moved);
        r.fall_distance = self.common.vehicle_fall_reset(r.fall_distance, from, moved);
        if !water && moved.y < 0.0 {
            r.fall_distance -= f64::from(moved.y as f32);
        }
        if r.ground.on_ground {
            if r.fall_distance > 0.0 {
                // Block.fallOn (farmland's random draw, honey's slide) is not
                // run for a rocket yet, nor for the other entities that are
                // not living.
                let p = r.position;
                let state = self.landing_state(p, r.bounds());
                let at = [p.x, p.y, p.z];
                self.projectile_own_event(
                    "minecraft:hit_ground",
                    r.owner,
                    r.guest,
                    at,
                    Some(state),
                );
            }
            r.fall_distance = 0.0;
        }
        let entity = (r.id, "minecraft:firework_rocket");
        let response = self.common.body_motion(&game_side!(self)).restitute(
            entity,
            movement,
            0.0,
            0.98_f32,
            r.velocity,
            r.fall_distance,
        );
        r.velocity = self.finish_body_restitution(entity, r.position, response);
        let side = game_side!(self);
        let motion = self.common.body_motion(&side);
        motion.apply_block_speed(r.position, &mut r.velocity);
        let writes =
            self.body_step_on(entity, (r.position, r.bounds()), &r.ground, &mut r.velocity);
        super::entity::apply_contact_writes(
            writes,
            &mut r.velocity,
            Some(&mut r.fall_distance),
            Some(&mut r.stuck),
            None,
        );
        let length = moved.length_squared();
        let recorded = length > 1.0e-7 || wanted.length_squared() - length < 1.0e-7;
        let (world, facts) = (&self.common.world, self.common.res.facts());
        let (registry, collision) = (self.common.pack.assets.registry, self.common.collision);
        let mut tops = Vec::new();
        let rocket = Rocket::bounds_at;
        let effects = sunrise_sim::falling_block::Effects {
            bounds: rocket,
            from,
            to: r.position,
            original: recorded.then_some(wanted),
        };
        let stuck = sunrise_sim::falling_block::apply_effects_from_blocks_of(
            effects,
            &mut r.velocity,
            &mut r.fall_distance,
            &mut tops,
            |p| {
                use sunrise_sim::falling_block::Inside;
                match &*facts.get(world.block(p.x, p.y, p.z)).name {
                    "minecraft:cobweb" => Inside::Cobweb,
                    "minecraft:powder_snow" => Inside::PowderSnow,
                    _ => {
                        sunrise_sim::motion::bubble_column_at(world, registry, facts, collision, p)
                            .map_or(Inside::Nothing, Inside::BubbleColumn)
                    }
                }
            },
        );
        if let Some(multiplier) = stuck {
            r.stuck = multiplier;
        }
        for pos in tops {
            self.bubble_column_particles(pos);
        }
        // FireworkRocketEntity.tick restores the pre-move delta after block effects.
        r.velocity = restored;
    }
    /// The rocket's owner in projectile queries: a guest by their entity id.
    pub(in crate::game::server_side) fn rocket_owner(&self, r: &Rocket) -> Option<Who> {
        self.shooter_who(r.owner, r.guest)
    }
    /// `Entity.doWaterSplashEffect` of a rocket moving at `velocity`: the
    /// splash sound and particles (from a stand-in for the rocket's own
    /// random, as a dropped item's), then SPLASH.
    pub(in crate::game::server_side) fn rocket_splash(&mut self, r: &Rocket, velocity: Vec3) {
        let seed = i64::from(r.id)
            .wrapping_mul(0x5DEECE66D)
            .wrapping_add(i64::from(r.life));
        let mut random = sunrise_core::random::LegacyRandom::new(seed);
        let mut splashes = Vec::new();
        let mut push = |s| splashes.push(s);
        sunrise_sim::entities::water_splash_effect(
            r.position,
            velocity,
            0.25,
            &mut random,
            &mut push,
        );
        for splash in splashes {
            self.server_apply_item_splash(splash);
        }
        let at = [r.position.x, r.position.y, r.position.z];
        self.projectile_own_event("minecraft:splash", r.owner, r.guest, at, None);
    }
    /// `ProjectileUtil.getHitResultOnMoveVector`: the nearest target on the
    /// move, short of the block it clips (`ClipContext.Block.COLLIDER`), each
    /// box grown by `computeMargin` (`tickCount` is the rocket's life plus
    /// one); else that block.
    pub(in crate::game::server_side) fn rocket_struck(
        &self,
        r: &Rocket,
        motion: Vec3,
        owner: Option<Who>,
    ) -> ProjectileHit {
        use sunrise_sim::projectiles::{NearestHit, compute_margin};
        let from = r.position;
        let to = from.add_vector(motion);
        let query = sunrise_gameplay_common::game::creatures::TargetQuery {
            include_guests: true,
            ..Default::default()
        };
        let live = self.live_proj_world(query, &[]);
        let block = live.clip(from, to);
        let entity_end = block.map_or(to, |(at, ..)| at);
        let search_area = r
            .bounds()
            .expand_towards(motion)
            .inflate(Vec3::new(1.0, 1.0, 1.0));
        let margin = compute_margin(r.life + 1);
        let mut best = NearestHit::new();
        self.visit_projectile_targets(query, &[], |c| {
            if !r.left_owner
                && let Some(o) = owner
                && self.in_same_vehicle_tree(o, c.who)
            {
                return;
            }
            best.consider(c, search_area, from, entity_end, margin);
        });
        best.finish()
            .map(|(who, at)| ProjectileHit::Entity { who, at })
            .or(block.map(|(at, pos, face)| ProjectileHit::Block { pos, face, at }))
            .unwrap_or(ProjectileHit::None)
    }
    /// `Projectile.getOwner` as a target's `Who`: a guest's `Who::Player`
    /// (kept with the connection that shot it) is their entity, not the host.
    pub(in crate::game) fn shooter_who(
        &self,
        owner: Option<Who>,
        guest: Option<u32>,
    ) -> Option<Who> {
        let owner = owner?;
        let Some(conn) = guest.filter(|_| owner == Who::Player) else {
            return Some(owner);
        };
        let host = self.server.lan.as_ref()?;
        let g = host.hub.guests.iter().find(|g| g.conn == conn)?;
        Some(Who::Mob(g.entity_id as u32))
    }
    /// Refuse a removed target before carrying its owning slot into a tick clone.
    pub(in crate::game::server_side) fn store_rocket_back(
        &mut self,
        index: usize,
        rocket: Rocket,
    ) -> bool {
        !self
            .common
            .persist
            .removals
            .contains(&super::commands::Target::Rocket(rocket.id))
            && self.common.rockets.store_back(index, rocket)
    }
    /// One tick of `r`, a clone of the resident rocket at `index`. Returns whether the
    /// rocket stays in the list; one that explodes or was removed has left it.
    pub(in crate::game::server_side) fn tick_rocket(
        &mut self,
        index: usize,
        mut r: Rocket,
    ) -> bool {
        let (id, join) = (r.id, r.join);
        let hit = self.with_entity_held_out(id, |game| game.advance_rocket(&mut r));
        let deflected = matches!(hit, ProjectileHit::Entity { who: Who::Mob(id), .. }
            if self.mobs.iter().any(|m| m.id == id && m.alive() && m.species() == Species::Breeze));
        let canceled =
            if !deflected && !matches!(hit, ProjectileHit::None) && self.native_hears_impacts() {
                let Some((rocket, canceled)) = self.rocket_impact(index, r, hit) else {
                    return false;
                };
                r = rocket;
                canceled
            } else {
                false
            };
        let explodes_on_hit = !canceled
            && !deflected
            && match hit {
                ProjectileHit::Entity { .. } => true,
                ProjectileHit::Block { .. } => !r.explosions.is_empty(),
                ProjectileHit::None => false,
            };
        let terminal = explodes_on_hit || r.life + 1 > r.lifetime;
        let _save_lease = terminal.then(sunrise_sim::join_seq::visibility::PassLease::enter);
        if terminal {
            let Some(mut owner) = self.common.rockets.extract(join) else {
                return false;
            };
            sunrise_sim::join_seq::store_back(&mut owner, r);
            r = owner;
        }
        if !canceled {
            if let ProjectileHit::Entity {
                who: Who::Mob(id),
                at,
            } = hit
                && deflected
            {
                if r.last_deflected_by != Some(id) {
                    let rot = 170.0 + r.random.next_float() * 20.0;
                    r.rotation[0] += rot;
                    r.previous_rotation[0] += rot;
                    r.velocity = r.velocity.scale(-0.5);
                    r.last_deflected_by = Some(id);
                    let sound = super::level_sounds_common::LevelSound::new(
                        "minecraft:entity.breeze.deflect",
                        [at.x, at.y, at.z],
                        1.0,
                        1.0,
                    )
                    .in_source(mod_api::SoundSource::Hostile);
                    self.play_bound_sound(sound, id, None);
                }
            } else {
                match hit {
                    ProjectileHit::Entity { who, at } => {
                        if let Who::Mob(id) = who {
                            self.redirect_projectile(id, r.owner, r.owner_uuid, r.guest, None);
                        }
                        self.explode_rocket(&r);
                        self.projectile_landed_event(r.id, r.owner, [at.x, at.y, at.z]);
                    }
                    ProjectileHit::Block { pos, face, at } => {
                        if face != [0; 3] {
                            let hitter =
                                sunrise_gameplay_common::game::creatures::ProjectileHitter {
                                    owner: r.owner,
                                    guest: r.guest,
                                    arrow: false,
                                    kind: "minecraft:firework_rocket",
                                    position: r.position,
                                };
                            let _ = self.projectile_block_impact(&hitter, pos, at, face);
                            self.target_block_hit(&hitter, pos, at, face);
                        }
                        if !r.explosions.is_empty() {
                            self.explode_rocket(&r);
                        }
                        self.projectile_block_landed_event(r.id, r.owner, pos);
                    }
                    ProjectileHit::None => {}
                }
            }
        }
        // FireworkRocketEntity.tick runs updateRotation, launch sound, life++ even after impact.
        r.update_rotation();
        let at = [r.position.x, r.position.y, r.position.z];
        if r.life == 0 && !r.silent {
            self.server_play_sound("minecraft:entity.firework_rocket.launch", at, 3.0, 1.0);
        }
        r.life += 1;
        if terminal {
            if !explodes_on_hit {
                self.explode_rocket(&r);
            }
            self.common.rockets.retire_extracted(id, join);
            return false;
        }
        let (from, to) = (r.previous, r.position);
        if !self.store_rocket_back(index, r) {
            return false;
        }
        self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
        true
    }
    pub fn tick_rockets(&mut self) {
        let ticking = self.entity_ticking_area();
        let mut i = 0;
        while i < self.common.rockets.len() {
            // ServerLevel.tick: only entities in entity-ticking chunks tick.
            let at = self.common.rockets[i].position;
            if !ticking(at.x.floor() as i32, at.z.floor() as i32) {
                self.common.rockets[i].previous = at;
                i += 1;
                continue;
            }
            // Like falling blocks and boats, tick an unbound clone while the resident
            // owner keeps its slot: extracting and reinserting every rocket shifts the
            // list tail twice per entity, which is O(n^2) for a crowd of rockets.
            let id = self.common.rockets[i].id;
            let rocket = self.common.rockets[i].clone();
            let kept = self.tick_rocket(i, rocket);
            // A hook may have added or removed rockets: resume right after this one
            // where it stands, or at `i` when it is gone.
            i = if !kept {
                i
            } else if self
                .common
                .rockets
                .get(i)
                .is_some_and(|rocket| rocket.id == id)
            {
                i + 1
            } else {
                self.common
                    .rockets
                    .iter()
                    .position(|rocket| rocket.id == id)
                    .map_or(i, |index| index + 1)
            };
        }
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::fireworks::FireworksGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
