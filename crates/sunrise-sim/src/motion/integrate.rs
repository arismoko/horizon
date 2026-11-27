//! One tick of movement integration, split out of `motion.rs`.

use super::*;

impl MotionWorld<'_, '_, '_> {
    pub(super) fn context(&self) -> CollisionContext<'_> {
        player_context(
            self.player.entity_id as u32,
            self.player.position.y,
            self.player.shift_key_down,
            self.player.main_hand.as_deref(),
        )
    }
    pub(super) fn collisions(&self) -> SceneCollisions<'_, '_> {
        SceneCollisions {
            world: self.scene,
            binding: self.collision_binding,
            subject: Some(self.player.powder_snow_subject()),
        }
    }
    pub(super) fn movement_collisions(
        &self,
    ) -> crate::collisions::EntityCollisions<'_, SceneCollisions<'_, '_>> {
        let at = [self.player.position.x, self.player.position.z];
        crate::collisions::EntityCollisions {
            blocks: self.collisions(),
            entities: self.entities,
            border: self.player.border.as_ref().map(|b| (b, at)),
        }
    }
    pub(super) fn block(&self, p: BlockPos) -> BlockStateId {
        self.scene.block(p.x, p.y, p.z)
    }
    pub(super) fn support(&mut self, offset: f32) -> Result<BlockPos> {
        let position = self.player.position;
        let support = self.player.ground.supporting_block;
        Ok(movement_support::support_position(
            &mut RegistryMovementWorld {
                properties: &self.resources.blocks,
                world: self,
                edits: &NoFactorEdits,
            },
            position,
            support,
            offset,
        )?)
    }
    pub(super) fn location(&self) -> MovementLocation {
        MovementLocation {
            position: self.player.position,
            block_position: BlockPos::containing(self.player.position),
            supporting_block: self.player.ground.supporting_block,
        }
    }
    /// `Player.onClimbable` (never while flying) over `LivingEntity.onClimbable`
    /// at the player's feet: the climbable tag or an open trapdoor above a
    /// matching ladder. It remembers the block.
    pub fn climbable(&mut self) -> Result<bool> {
        if self.player.flying {
            self.player.on_climbable = false;
            return Ok(false);
        }
        let resources = self.resources;
        let found: Result<bool> =
            climbing::on_climbable(self, &resources.climbing).map_err(|e| e.to_string().into());
        let climbable = found?;
        self.player.on_climbable = climbable;
        Ok(climbable)
    }
    /// Dry, standing movement phases of LivingEntity.aiStep, including powder
    /// snow's block-inside effects and server freezing (thawing, the frost
    /// modifier, freeze damage). Equipment, interpolation, effects and other
    /// world/entity effects remain unbound. Inputs are the native
    /// float xxa/yya/zza fields, before LivingEntity.applyInput damping.
    pub fn movement_tick(&mut self, input: [f32; 3]) -> Result<()> {
        self.dry_tick(TickInput::Living(input))
    }
    /// Dry Player.aiStep movement with a prepared local input vector. The
    /// caller applies LocalPlayer input preparation once. Empty inventory,
    /// equipment and nearby-entity sets are the current scene domain.
    pub fn player_movement_tick(&mut self, direction: [f32; 2], jumping: bool) -> Result<()> {
        self.fluid_base_tick()?;
        self.player_movement_tick_with_fluids(direction, jumping)
    }
    fn player_movement_tick_with_fluids(
        &mut self,
        direction: [f32; 2],
        jumping: bool,
    ) -> Result<()> {
        // Entity.baseTick
        self.player.was_in_powder_snow = self.player.in_powder_snow;
        self.player.in_powder_snow = false;
        self.player.old_position = self.player.position;
        if self.player.jump_trigger_time > 0 {
            self.player.jump_trigger_time -= 1;
        }
        let side = self.side;
        self.player.peaceful_regeneration(
            self.loop_context,
            &|| side.hit_reads(),
            self.scene.health_rules,
        )?;
        // LocalPlayer.aiStep: double-tap jump toggles creative flight.
        let was_jumping = std::mem::replace(&mut self.player.was_jumping, jumping);
        let flying_before = self.player.flying;
        if self.player.spectator {
            self.player.ground.on_ground = false;
        }
        // LocalPlayer.aiStep forces flight for a local spectator only when may_fly is set.
        if self.player.may_fly && self.player.local_spectator {
            self.player.flying = true;
        } else if self.player.may_fly && !was_jumping && jumping && !self.player.was_auto_jump {
            // `LocalPlayer.aiStep`: mayfly gates the flight toggle, not the
            // flying state independently supplied by the server's abilities.
            if self.player.jump_trigger_time == 0 {
                self.player.jump_trigger_time = 7;
            } else if !self.player.swimming {
                self.player.flying = !self.player.flying;
                if self.player.flying && self.player.ground.on_ground {
                    self.jump_from_ground()?;
                }
                self.player.jump_trigger_time = 0;
            }
        }
        let toggled = flying_before != self.player.flying;
        // The server clears the glide flag from the position the client sent
        // last tick, and the client applies it a tick later, before aiStep: a
        // landing glide lasts one more tick on the ground, with no ground
        // friction, so a jump then keeps the speed (the elytra hop).
        // A client of a server takes the flag from the server itself: vanilla's
        // LocalPlayer never ends a glide of its own, and keeping the local
        // prediction there would clear the flag before the server's
        // SetEntityData does (#7616).
        let ending = std::mem::take(&mut self.player.glide_ending);
        if ending && !self.player.server_owns_glide {
            self.player.fall_flying = false;
        }
        // Player.tryToStartFallFlying (canGlide: airborne, not flying, a glider;
        // never from inside a liquid).
        let can_glide = self.player.glider
            && !self.player.ground.on_ground
            && !self.player.flying
            && self.player.status.levitation.is_none()
            && !self.player.fluids.was_touching_water
            && !self.player.is_in_lava();
        // LocalPlayer.aiStep: `!onClimbable()` before tryToStartFallFlying.
        if jumping
            && !toggled
            && !was_jumping
            && !self.climbable()?
            && !self.player.fall_flying
            && can_glide
        {
            self.player.fall_flying = true;
        }
        // LivingEntity.updateFallFlying (fallFlyTicks is counted after the move).
        if self.player.fall_flying {
            // Entity.checkFallDistanceAccumulation: a glide that isn't diving
            // (falling slower than 0.5 a tick) keeps the fall distance at 1, so
            // landing from it hurts no more than a one-block drop.
            if self.player.velocity.y > -0.5 && self.player.fall_distance > 1.0 {
                self.player.fall_distance = 1.0;
            }
            // canGlide alone: water and lava don't end a glide, levitation does.
            let still = self.player.glider
                && !self.player.ground.on_ground
                && !self.player.flying
                && self.player.status.levitation.is_none();
            if !still {
                // The server clears the flag; a client of one waits for its
                // synced data (the SetEntityData handler sets fall_flying).
                if !self.player.server_owns_glide {
                    self.player.glide_ending = true;
                }
            } else if (self.player.fall_fly_ticks + 1) % 10 == 0
                && ((self.player.fall_fly_ticks + 1) / 10) % 2 == 0
            {
                self.player.glider_wear += 1;
            }
        }
        // LocalPlayer.goDownInWater, before LivingEntity.aiStep's velocity cleanup.
        if self.player.fluids.was_touching_water
            && self.player.shift_key_down
            && !self.player.flying
        {
            self.player.velocity.y += f64::from(-0.04f32);
        }
        if self.player.flying {
            // Vertical flight: jump up, shift down, three times the flying speed.
            let ya = i32::from(jumping) - i32::from(self.player.shift_key_down);
            if ya != 0 {
                self.player.velocity.y += f64::from(ya as f32 * self.player.flying_speed * 3.0);
            }
        }
        // LocalPlayer.aiStep: a player inside a block is nudged out of it.
        let [x, z] = std::mem::take(&mut self.player.nudge);
        if let Some(x) = x {
            self.player.velocity.x = x;
        }
        if let Some(z) = z {
            self.player.velocity.z = z;
        }
        self.dry_tick(TickInput::PreparedLocal {
            direction,
            jumping: jumping && !self.player.flying,
        })?;
        // LivingEntity.tick counts fallFlyTicks after aiStep.
        if self.player.fall_flying {
            self.player.fall_fly_ticks += 1;
        } else {
            self.player.fall_fly_ticks = 0;
        }
        if self.player.spectator {
            self.player.ground.on_ground = false;
        }
        if self.player.flying {
            self.player.fall_distance = 0.0;
            if self.player.ground.on_ground && !self.player.local_spectator {
                self.player.flying = false;
            }
        }
        self.player.head_yaw = self.player.yaw;
        // Player.aiStep refreshes speed after movement, not before travel.
        self.player.speed = self.player.attribute("minecraft:movement_speed")? as f32;
        Ok(())
    }
    /// Resident simulation phases in native order. Enclosing entity/world tick
    /// services are still incomplete; this composes clocks, movement and hunger.
    pub fn simulation_tick(&mut self, direction: [f32; 2], jumping: bool) -> Result<()> {
        self.player.health.advance_clock();
        self.fluid_base_tick()?;
        self.simulation_tick_with_fluids(direction, jumping)
    }
    fn simulation_tick_with_fluids(&mut self, direction: [f32; 2], jumping: bool) -> Result<()> {
        self.tick_environment_damage()?;
        let previous = self.player.position;
        self.player_movement_tick_with_fluids(direction, jumping)?;
        self.reset_unlit_fire();
        self.player.refresh_attributes()?;
        self.update_player_pose()?;
        // The resident scene accepts the resolved local movement directly.
        // Vanilla dispatches this separately from travel, on packet acceptance.
        self.check_movement_statistics(previous.vector_to(self.player.position))?;
        let rules = self.scene.health_rules;
        self.player.food_tick(
            self.loop_context,
            &mut crate::health::DamageLevel::of(
                self.scene,
                self.assets.registry,
                self.tag_items,
                self.block_registries,
                self.damage_hooks,
            )
            .with_player_equipment(self.side.player_equipment())
            .with_reads(self.side.hit_reads()),
            self.health,
            rules,
        )
    }
    pub fn controlled_simulation_tick(&mut self, keys: crate::sprint::Keys) -> Result<()> {
        self.local_tick(keys, |player, keys| player.sprint_input_tick(keys))
    }
    /// A client's tick played again: the client's own keys, and the sprinting it said it had
    /// (its ServerboundPlayerCommandPacket comes before the tick's move). It tells the mods nothing
    /// of what it did (`living_jump`, `living_fall`, `player_flyable_fall`: `replaying`), but
    /// their answers to what the movement asks (a bounce's restitution, a block's friction) count.
    pub fn replayed_tick(&mut self, keys: crate::sprint::Keys, sprinting: bool) -> Result<()> {
        self.player.replaying = true;
        let result = self.local_tick(keys, |player, _| {
            if player.sprinting == sprinting {
                Ok(())
            } else {
                player.set_sprinting(sprinting)
            }
        });
        self.player.replaying = false;
        result
    }
    /// `LocalPlayer.aiStep` and the travel it ends with; `sprint` is the
    /// sprint phase.
    fn local_tick(
        &mut self,
        keys: crate::sprint::Keys,
        sprint: impl FnOnce(&mut PlayerMotion, crate::sprint::Keys) -> Result<()>,
    ) -> Result<()> {
        self.begin_local_tick()?;
        self.finish_fluid_tick(None)?;
        let keys = self.prepare_local_sprint(keys)?;
        sprint(self.player, keys)?;
        self.finish_local_tick_body(keys)
    }
    /// The base tick starts before any `LocalPlayer.aiStep` work.
    pub fn begin_local_tick(&mut self) -> Result<()> {
        self.player.health.advance_clock();
        self.begin_fluid_tick()
    }
    pub fn prepare_local_sprint(
        &mut self,
        mut keys: crate::sprint::Keys,
    ) -> Result<crate::sprint::Keys> {
        self.update_local_crouching()?;
        // LocalPlayer.aiStep: a pending auto-jump presses jump for this tick
        // (input.makeJump) and keeps the jump from toggling flight.
        self.player.was_auto_jump = self.player.auto_jump_time > 0;
        if self.player.was_auto_jump {
            self.player.auto_jump_time -= 1;
            keys.jump = true;
        }
        Ok(keys)
    }
    pub(super) fn finish_local_tick_body(&mut self, keys: crate::sprint::Keys) -> Result<()> {
        self.player.shift_key_down = keys.shift;
        let direction = keys.direction();
        self.player.move_vector = direction;
        let sneak = if direction[0] * direction[0] + direction[1] * direction[1] != 0.0
            && self.player.moving_slowly()
        {
            Some(self.player.attribute("minecraft:sneaking_speed")? as f32)
        } else {
            None
        };
        let modified = sunrise_game_data::move_input::modify_input(
            direction,
            self.player.item_use_speed,
            sneak,
        );
        self.player.applied_input = modified;
        self.simulation_tick_with_fluids(modified, keys.jump)
    }
    pub fn check_movement_statistics(&mut self, delta: Vec3) -> Result<()> {
        movement_statistics::check(self, delta)
    }
    pub(super) fn dry_tick(&mut self, input: TickInput) -> Result<()> {
        if self.player.no_jump_delay > 0 {
            self.player.no_jump_delay -= 1;
        }
        self.player.drop_tiny_velocity();
        let mut input = match input {
            TickInput::Living(v) => Vec3::new(
                f64::from(v[0] * 0.98),
                f64::from(v[1]),
                f64::from(v[2] * 0.98),
            ),
            TickInput::PreparedLocal { direction, jumping } => {
                self.player.jumping = jumping;
                Vec3::new(f64::from(direction[0]), 0.0, f64::from(direction[1]))
            }
        };
        if self.player.health.dead_or_dying() {
            self.player.jumping = false;
            input.x = 0.0;
            input.z = 0.0;
        }
        self.jump_phase()?;
        let before_travel = self.player.bounds();
        self.player_travel(input)?;
        if self.player.block_effects {
            self.apply_effects_from_blocks()?;
        }
        // LivingEntity.aiStep "freezing", the server's alone (`level instanceof
        // ServerLevel`): a client's player gets its frost slowdown from the
        // server's attributes and its frozen ticks from the server's entity data.
        if !self.player.client_side {
            if !self.player.in_powder_snow || !self.player.can_freeze() {
                self.player.ticks_frozen = (self.player.ticks_frozen - 2).max(0);
            }
            self.player.remove_frost()?;
            let on = self.support(movement_support::LEGACY_SUPPORT_OFFSET)?;
            let on_air = self.assets.registry.state(self.block(on)).is_none_or(|s| {
                matches!(
                    s.name.as_str(),
                    "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
                )
            });
            self.player.try_add_frost(on_air)?;
        }
        self.update_soul_speed_block()?;
        if !self.player.client_side
            && self.player.health.tick_count % 40 == 0
            && self.player.is_fully_frozen()
            && self.player.can_freeze()
        {
            self.freeze_damage()?;
        }
        if self.player.auto_spin_attack_ticks > 0 {
            self.player.auto_spin_attack_ticks -= 1;
            self.check_auto_spin_attack(before_travel);
        }
        Ok(())
    }
    /// Refresh whether the player stands on a soul speed block, for the
    /// `location_changed` enchantment effects (a server's player too, whose
    /// client does the walking).
    pub fn update_soul_speed_block(&mut self) -> Result<()> {
        self.player.location_effects.on_soul_speed_block = self.on_soul_speed_block()?;
        Ok(())
    }
    /// `MovementAffectedByPredicate` for `#minecraft:soul_speed_blocks`.
    fn on_soul_speed_block(&mut self) -> Result<bool> {
        let on = self.support(movement_support::MOVEMENT_SUPPORT_OFFSET)?;
        Ok(self
            .assets
            .registry
            .state(self.block(on))
            .is_some_and(|s| self.resources.soul_speed_blocks.contains(&s.name)))
    }
    /// `Entity.rideTick` for a rider this client moves (`LocalPlayer`): its
    /// deltaMovement starts at zero, then `LivingEntity.aiStep` travels it as
    /// it does any entity it simulates (`Player.travel` for a passenger is
    /// `LivingEntity.travel`, with no swimming or flight change), from where
    /// it sat, with its own input (`LocalPlayer.applyInput`: a passenger slows
    /// for neither an item nor sneaking); `LivingEntity.rideTick` then forgets
    /// the fall. The caller seats it after (`positionRider`). The move leaves
    /// the onGround and horizontalCollision its Rot packet carries, and the
    /// deltaMovement a dismount teleport keeps (its DELTA_* are relative).
    pub(crate) fn rider_travel_tick(&mut self, direction: [f32; 2]) -> Result<()> {
        self.player.velocity = Vec3::ZERO;
        let input = sunrise_game_data::move_input::modify_input(direction, None, None);
        let mut gate = MoveGate::new();
        self.living_travel(
            &mut gate,
            Vec3::new(f64::from(input[0]), 0.0, f64::from(input[1])),
        )?;
        self.player.fall_distance = 0.0;
        Ok(())
    }
    /// ServerPlayer's own travel: the server simulates a player too
    /// (`MoveSimulationType.AUTHORITATIVE_SIDE_AND_SERVER`), with no input
    /// (it never sets xxa/zza). Only the delta movement stays, for knockback
    /// and later move validation.
    pub fn server_travel(&mut self) -> Result<()> {
        let p = &mut *self.player;
        let (position, ground, collision) = (p.position, p.ground, p.collision);
        let (history, fall_distance, steps) = (p.history.clone(), p.fall_distance, p.steps);
        let events = p.events.len();
        p.drop_tiny_velocity();
        let velocity = p.velocity;
        p.server_copy = true;
        let travelled = self.player_travel(Vec3::ZERO);
        let p = &mut *self.player;
        p.server_copy = false;
        (p.position, p.ground, p.collision) = (position, ground, collision);
        (p.history, p.fall_distance, p.steps) = (history, fall_distance, steps);
        // The wall hit is the server's own: its sound and damage events stay.
        let kept: Vec<_> = p
            .wall_hit_events
            .take()
            .map(|span| p.events.drain(span).collect())
            .unwrap_or_default();
        p.events.truncate(events);
        p.events.extend(kept);
        // A travel the world cannot solve (a swept box past the collision scan's domain) publishes
        // none of its partial work: the velocity it was handed stands as it was.
        if travelled.is_err() {
            p.set_velocity(velocity);
        }
        Ok(())
    }
    /// `LivingEntity.checkAutoSpinAttack`: the first living entity in the
    /// box swept this tick takes the hit and ends the spin, bouncing the
    /// player back at a fifth of its speed; a wall ends it too.
    fn check_auto_spin_attack(&mut self, before: Aabb) {
        let swept = before.union_bounds(self.player.bounds());
        let p = &mut self.player;
        if let Some(&(id, _)) = p.spin_targets.iter().find(|(_, b)| b.intersects(swept)) {
            p.spin_hit = Some(id);
            p.auto_spin_attack_ticks = 0;
            p.velocity = p.velocity.scale(-0.2);
        } else if p.collision.horizontal {
            p.auto_spin_attack_ticks = 0;
        }
    }
    /// `hurtServer(damageSources().freeze(), 1)` for a fully frozen player.
    fn freeze_damage(&mut self) -> Result<()> {
        let id = self
            .resources
            .health
            .damage
            .id("minecraft:freeze")
            .ok_or("missing freeze damage type")?;
        self.player.events.push(MotionEvent::Health(
            crate::health::HealthEvent::FreezeDamage(1.0),
        ));
        let rules = self.scene.health_rules;
        self.player.hurt(
            &mut crate::health::DamageLevel::of(
                self.scene,
                self.assets.registry,
                self.tag_items,
                self.block_registries,
                self.damage_hooks,
            )
            .with_player_equipment(self.side.player_equipment())
            .with_reads(self.side.hit_reads()),
            self.health,
            rules,
            id,
            1.0,
        )?;
        Ok(())
    }
    /// `Entity.applyEffectsFromBlocks` for the blocks Sunrise's player reacts
    /// to inside its movement path: powder snow (`PowderSnowBlock.entityInside`
    /// and the FREEZE / EXTINGUISH effects), cobwebs, sweet berry bushes,
    /// cactus and lit campfires, and the magma and slime blocks it stands on
    /// (`stepOn`). Pressure plates, tripwires and rails are the world tick's
    /// scan.
    pub fn apply_effects_from_blocks(&mut self) -> Result<()> {
        self.effects_from_blocks(BlockPass::Client)
    }
    /// The part of `applyEffectsFromBlocks` a server's player must see before
    /// its landing is checked: `ServerGamePacketListenerImpl.handleMovePlayer`
    /// runs `move` and then `doCheckFallDamage`, and `checkInsideBlocks` lets a
    /// cobweb, a sweet berry bush or powder snow (while the block at its feet
    /// is powder snow) call `makeStuckInBlock`, forgetting the fall, before
    /// that check reads it. Only the fall and the impulse are applied; the
    /// slowing is the client's, and `server_block_damage` does the rest. The
    /// moves stay in the history for it.
    pub fn server_stuck_in_blocks(&mut self, from: Vec3) -> Result<()> {
        self.player.old_position = from;
        self.effects_from_blocks(BlockPass::ServerStuck)
    }
    /// What `applyEffectsFromBlocks` does to a server's player along the
    /// moves its client sent since `from` (in its history): the damage of
    /// magma, cactus, lit campfires and grown sweet berry bushes, and powder
    /// snow's FREEZE, then `LivingEntity.aiStep`'s freezing and its damage.
    /// Its client moves it, so the slowing and the rest are the client's.
    pub fn server_block_damage(&mut self, from: Vec3) -> Result<()> {
        let effects = self.collect_server_block_damage(from)?;
        self.flush_inside_hostless(effects)?;
        self.finish_server_block_damage_freezing()
    }

    /// Walk the server player's movement history and return the queued
    /// `InsideEffects` collector so a game-owned turn can flush it with a
    /// live `&mut Game` loan before [`Self::finish_server_block_damage_freezing`].
    pub fn collect_server_block_damage(&mut self, from: Vec3) -> Result<mod_api::InsideEffects> {
        self.server_block_damage_walk(from, &mut None, false)
            .map(Option::unwrap_or_default)
    }

    /// Stop a guest's block walk after a hit needing death or protection on its acting body.
    /// `Ok(None)` leaves the next contact in `cursor` for the owning game; `Ok(Some(effects))`
    /// returns the collected `InsideEffects` ready to flush against the live level.
    pub fn server_block_damage_until_hit(
        &mut self,
        from: Vec3,
        cursor: &mut Option<PlayerBlockWalk>,
    ) -> Result<Option<mod_api::InsideEffects>> {
        self.server_block_damage_walk(from, cursor, true)
    }

    fn server_block_damage_walk(
        &mut self,
        from: Vec3,
        cursor: &mut Option<PlayerBlockWalk>,
        pause: bool,
    ) -> Result<Option<mod_api::InsideEffects>> {
        let mut walk = if let Some(walk) = cursor.take() {
            walk
        } else {
            self.player.old_position = from;
            // Entity.baseTick
            self.player.was_in_powder_snow = self.player.in_powder_snow;
            self.player.in_powder_snow = false;
            let walk = self.start_block_walk(BlockPass::ServerDamage)?;
            if pause && self.hit_needs_owner() {
                *cursor = walk;
                return Ok(None);
            }
            if let Some(walk) = walk {
                walk
            } else {
                return Ok(Some(mod_api::InsideEffects::default()));
            }
        };
        if !walk.run(self, pause)? {
            *cursor = Some(walk);
            return Ok(None);
        }
        Ok(Some(walk.effects))
    }

    /// `LivingEntity.aiStep`'s freezing countdown and damage after the
    /// server player's inside-block collector has flushed.
    pub fn finish_server_block_damage_freezing(&mut self) -> Result<()> {
        // LivingEntity.aiStep "freezing" (server).
        if !self.player.in_powder_snow || !self.player.can_freeze() {
            self.player.ticks_frozen = (self.player.ticks_frozen - 2).max(0);
        }
        if self.player.health.tick_count % 40 == 0
            && self.player.is_fully_frozen()
            && self.player.can_freeze()
        {
            self.freeze_damage()?;
        }
        Ok(())
    }
    fn effects_from_blocks(&mut self, pass: BlockPass) -> Result<()> {
        if let Some(mut walk) = self.start_block_walk(pass)? {
            walk.run(self, false)?;
            self.flush_inside_hostless(walk.effects)?;
        }
        Ok(())
    }
    fn flush_inside_hostless(&mut self, mut effects: mod_api::InsideEffects) -> Result<()> {
        if effects.is_empty() {
            return Ok(());
        }
        let mut access = PlayerInsideAccess {
            m: self,
            error: None,
        };
        effects.apply_and_clear(&mut access);
        if let Some(err) = access.error {
            Err(err)
        } else {
            Ok(())
        }
    }
    fn start_block_walk(&mut self, pass: BlockPass) -> Result<Option<PlayerBlockWalk>> {
        use sunrise_world::entity_move::MovementRecord;
        let damage_only = pass != BlockPass::Client;
        let mut movements: Vec<MovementRecord> =
            self.player.history.records().iter().copied().collect();
        if pass != BlockPass::ServerStuck {
            self.player.history.clear();
        }
        let position = self.player.position;
        match movements.last() {
            None => movements.push(MovementRecord {
                from: self.player.old_position,
                to: position,
                original: None,
            }),
            Some(last) if last.to.distance_squared(position) > f64::from(9.999_999_4e-11_f32) => {
                movements.push(MovementRecord {
                    from: last.to,
                    to: position,
                    original: None,
                })
            }
            _ => {}
        }
        // isAffectedByBlocks: no physics for spectators.
        if self.player.spectator {
            return Ok(None);
        }
        // Block.stepOn for the block under an entity on the ground:
        // MagmaBlock's hot floor hurts unless it steps carefully (sneaks).
        if self.player.ground.on_ground && pass != BlockPass::ServerStuck {
            let on = self.legacy_on_position()?;
            let state = self.block(on);
            let under = self.assets.registry.state(state);
            let magma = under.is_some_and(|s| s.name == "minecraft:magma_block");
            let slime = under.is_some_and(|s| s.name == "minecraft:slime_block");
            if magma && !self.player.shift_key_down {
                self.environment_hurt("minecraft:hot_floor", 1.0)?;
            }
            // TurtleEggBlock.stepOn: a step that is not careful may break an egg.
            if !self.player.shift_key_down {
                self.resources
                    .landing
                    .step_on(
                        self,
                        state,
                        on,
                        sunrise_world::block_landing::LandingEntityKind::Player,
                    )
                    .map_err(|e| e.to_string())?;
            }
            // SlimeBlock.stepOn: a slow walk on slime drags the horizontal
            // speed (the client moves its own player, so not the server's copy).
            if slime
                && !damage_only
                && !self.player.shift_key_down
                && let Some(scale) = crate::entities::slime_step_on_scale(self.player.velocity.y)
            {
                self.player.velocity.x *= scale;
                self.player.velocity.z *= scale;
            }
            self.part_step_on(state, on)?;
        }
        Ok(Some(PlayerBlockWalk {
            pass,
            movements,
            sweep: Default::default(),
            visited: Default::default(),
            effects: Default::default(),
        }))
    }

    pub(crate) fn hit_needs_owner(&self) -> bool {
        self.player.health.damage_completion.is_some()
    }
    /// The block effects and the frost of a window's own player while a vehicle carries it. Its
    /// own move is `rider_tick`'s; after it, `LivingEntity.aiStep` still applies the effects of
    /// the blocks a passenger is in (`applyEffectsFromBlocks`: powder snow's FREEZE adds a tick of
    /// frost) and thaws it when it is out of powder snow. The frostbite overlay reads these ticks.
    /// The server's copy does this in `server_block_damage`.
    pub fn passenger_block_effects_tick(&mut self) -> Result<()> {
        // Entity.baseTick
        self.player.was_in_powder_snow = self.player.in_powder_snow;
        self.player.in_powder_snow = false;
        if self.player.block_effects {
            self.apply_effects_from_blocks()?;
        }
        // LivingEntity.aiStep "freezing" (server), as the client predicts it on foot.
        if !self.player.in_powder_snow || !self.player.can_freeze() {
            self.player.ticks_frozen = (self.player.ticks_frozen - 2).max(0);
        }
        // The vehicle moves the passenger between ticks: the next sweep starts here.
        self.player.old_position = self.player.position;
        Ok(())
    }
    /// The player's view of the shared inside walk: its box, its life, its
    /// blocks and shapes, and its own `entityInside` moments, which keep their
    /// existing code and queue into the shared collector instead of the
    /// hand-written group flush this replaces.
    pub(super) fn mod_block_inside(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) {
        let entity = self.player.entity_id;
        let Some(dimension) = crate::block_contact::current_level() else {
            return;
        };
        if !crate::block_contact::HOOKS
            .get()
            .is_some_and(|hooks| (hooks.inside_listens)(state))
        {
            return;
        }
        // Generated dispatch isolates each part into its own staged collector
        // and absorbs it only on normal return.
        let mut level = crate::health::DamageLevel::of(
            &mut *self.scene,
            self.assets.registry,
            self.tag_items,
            self.block_registries,
            self.damage_hooks,
        )
        .with_reads(self.side.hit_reads());
        if let Some(hooks) = crate::block_contact::HOOKS.get() {
            (hooks.inside_fire)(
                &mut level,
                dimension,
                entity,
                "minecraft:player",
                state,
                p,
                precise,
                effects,
            );
        }
    }

    /// `HoneyBlock.entityInside`: a falling player touching a side of the block
    /// slides down it (`isSlidingDown`, `doSlideMovement`, `maybeDoSlideEffects`).
    pub(super) fn honey_block_inside(&mut self, pos: BlockPos) -> Result<()> {
        let old_delta_y = |dy: f64| dy / f64::from(0.98_f32) + 0.08;
        let new_delta_y = |dy: f64| (dy - 0.08) * f64::from(0.98_f32);
        let position = self.player.position;
        let velocity = self.player.velocity;
        if self.player.ground.on_ground
            || position.y > f64::from(pos.y) + 0.9375 - 1.0e-7
            || old_delta_y(velocity.y) >= -0.08
        {
            return Ok(());
        }
        let dx = (f64::from(pos.x) + 0.5 - position.x).abs();
        let dz = (f64::from(pos.z) + 0.5 - position.z).abs();
        let overlap = 0.4375 + f64::from(self.player.dimensions.width / 2.0);
        if !(dx + 1.0e-7 > overlap || dz + 1.0e-7 > overlap) {
            return Ok(());
        }
        // doSlideMovement
        let old = old_delta_y(velocity.y);
        if old < -0.13 {
            let factor = -0.05 / old;
            self.player.velocity =
                Vec3::new(velocity.x * factor, new_delta_y(-0.05), velocity.z * factor);
        } else {
            self.player.velocity = Vec3::new(velocity.x, new_delta_y(-0.05), velocity.z);
        }
        self.player.fall_distance = 0.0;
        // maybeDoSlideEffects: the sound, then the server's event 53.
        let random = &mut self.scene.random;
        let sound = random.next_int_bound(5).map_err(|e| format!("{e:?}"))? == 0;
        // Only the server broadcasts event 53 (`!level.isClientSide()`).
        let particles = !self.player.client_side
            && random.next_int_bound(5).map_err(|e| format!("{e:?}"))? == 0;
        // The server also checks the slide criterion every 20 game ticks,
        // including ticks without a sound or a particle event.
        let criterion = !self.player.client_side && self.scene.game_time % 20 == 0;
        if sound || particles || criterion {
            self.player.events.push(MotionEvent::HoneySlide {
                pos,
                sound,
                particles,
            });
        }
        Ok(())
    }
    /// `Entity.getInBlockState`: whether the block at the player's own position
    /// (its feet) is powder snow.
    pub(super) fn feet_in_powder_snow(&mut self) -> bool {
        let feet = BlockPos::containing(self.player.position);
        let state = self.assets.registry.state(self.block(feet));
        state.is_some_and(|s| s.name == "minecraft:powder_snow")
    }
    /// `Player.makeStuckInBlock` as a server's player sees it: its client
    /// moves it and applies the slowing, but the fall is forgotten and the
    /// impulse context ends here too.
    pub(super) fn server_stuck_in_block(&mut self) {
        if !self.player.flying {
            self.player.fall_distance = 0.0;
        }
        self.player.impulse.try_reset();
    }
    pub(super) fn apply_entity_write(&mut self, write: mod_api::EntityWrite) -> Result<()> {
        let triple = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        match write {
            mod_api::EntityWrite::Push(v) => {
                self.player.velocity = self.player.velocity.add_vector(triple(v));
            }
            mod_api::EntityWrite::SetMovement(v) => self.player.velocity = triple(v),
            mod_api::EntityWrite::StuckIn(v) => {
                self.make_stuck_in_block(triple(v));
            }
            mod_api::EntityWrite::Hurt {
                damage_type,
                amount,
            } => {
                if let Some(kind) =
                    mod_api::__private::parts_installation::damage_type_name(damage_type)
                        .and_then(|name| self.health.damage.id(name))
                {
                    self.environment_hurt_type(kind, amount)?;
                }
            }
            mod_api::EntityWrite::ResetFallDistance => self.player.fall_distance = 0.0,
            mod_api::EntityWrite::SetFireTicks(ticks) => self.player.fire_ticks = ticks,
            mod_api::EntityWrite::AddEffect {
                effect,
                duration,
                amplifier,
            } => {
                if let Some(id) = mod_api::__private::effect_key_id(effect) {
                    self.player.events.push(MotionEvent::InsideEffect {
                        id,
                        duration,
                        amplifier,
                    });
                }
            }
        }
        Ok(())
    }
    /// `Player.makeStuckInBlock`: the next move is scaled by `multiplier`
    /// (and the fall forgotten) unless flying; the impulse context resets
    /// either way.
    pub(super) fn make_stuck_in_block(&mut self, multiplier: Vec3) {
        self.player.apply_stuck_in_block(multiplier);
    }
    /// `WebBlock.entityInside`: stuck at a quarter of the speed, half with
    /// the Weaving effect.
    pub(super) fn web_inside(&mut self) {
        let multiplier = if self.player.status.weaving {
            Vec3::new(0.5, 0.25, 0.5)
        } else {
            Vec3::new(0.25, f64::from(0.05_f32), 0.25)
        };
        self.make_stuck_in_block(multiplier);
    }
    /// `SweetBerryBushBlock.entityInside` for the player: slowed, and pricked
    /// (`sweetBerryBush` damage, 1) by a grown bush while its known movement
    /// is at least 0.003 on X or Z.
    pub(super) fn sweet_berry_bush_inside(&mut self, state: BlockStateId) -> Result<()> {
        self.make_stuck_in_block(Vec3::new(f64::from(0.8_f32), 0.75, f64::from(0.8_f32)));
        self.sweet_berry_bush_damage(state)
    }
    /// The server's half of `SweetBerryBushBlock.entityInside`: the prick.
    pub(super) fn sweet_berry_bush_damage(&mut self, state: BlockStateId) -> Result<()> {
        let age = self
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("age"))
            .map_or("0", |a| a.as_str());
        if age == "0" {
            return Ok(());
        }
        // Player.isClientAuthoritative: ServerPlayer.getKnownMovement, this
        // tick's movement.
        let known = self.player.position.subtract(self.player.old_position);
        if known.x * known.x + known.z * known.z > 0.0
            && (known.x.abs() >= f64::from(0.003_f32) || known.z.abs() >= f64::from(0.003_f32))
        {
            let id = self
                .resources
                .health
                .damage
                .id("minecraft:sweet_berry_bush")
                .ok_or("missing sweet_berry_bush damage type")?;
            self.player.events.push(MotionEvent::Health(
                crate::health::HealthEvent::SweetBerryBushDamage(1.0),
            ));
            let rules = self.scene.health_rules;
            self.player.hurt(
                &mut crate::health::DamageLevel::of(
                    self.scene,
                    self.assets.registry,
                    self.tag_items,
                    self.block_registries,
                    self.damage_hooks,
                )
                .with_player_equipment(self.side.player_equipment())
                .with_reads(self.side.hit_reads()),
                self.health,
                rules,
                id,
                1.0,
            )?;
        }
        Ok(())
    }
    /// `PowderSnowBlock.entityInside` for the player: stuck while the block
    /// at its feet is powder snow, with a snowflake now and then (server).
    pub(super) fn powder_snow_inside(&mut self, p: BlockPos) -> Result<()> {
        if !self.feet_in_powder_snow() {
            return Ok(());
        }
        self.make_stuck_in_block(Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32)));
        // `ServerLevel.sendParticles` is server-only (`!level.isClientSide()`).
        if !self.player.client_side {
            self.powder_snow_snowflake(p);
        }
        Ok(())
    }
    /// `PowderSnowBlock.entityInside`'s server particle: one snowflake at
    /// the top of `p` when the player moved horizontally this tick.
    pub(super) fn powder_snow_snowflake(&mut self, p: BlockPos) {
        if !self.feet_in_powder_snow() {
            return;
        }
        // ServerPlayer.getKnownMovement: this tick's movement.
        let known = self.player.position.subtract(self.player.old_position);
        let random = &mut self.scene.random;
        if (known.x != 0.0 || known.z != 0.0) && random.next_boolean() {
            // Mth.randomBetween(random, -1, 1) * (1 / 12)
            let xa = (random.next_float() * 2.0 - 1.0) * 0.083_333_336_f32;
            let za = (random.next_float() * 2.0 - 1.0) * 0.083_333_336_f32;
            self.player.events.push(MotionEvent::Snowflake {
                position: Vec3::new(
                    self.player.position.x,
                    f64::from(p.y + 1),
                    self.player.position.z,
                ),
                velocity: Vec3::new(f64::from(xa), f64::from(0.05_f32), f64::from(za)),
            });
        }
    }
    pub fn jump_from_ground(&mut self) -> Result<()> {
        let jumped = living_motion::jump_from_ground(self)?;
        if jumped && !self.player.replaying {
            let v = self.player.velocity;
            native_living_jump(
                self.loop_context,
                self.player.entity_id as u32,
                "minecraft:player",
                [v.x, v.y, v.z],
            );
        }
        self.player
            .events
            .push(MotionEvent::Health(crate::health::HealthEvent::Statistic {
                name: "jump",
                amount: 1,
            }));
        self.player
            .cause_food_exhaustion(if self.player.sprinting { 0.2 } else { 0.05 });
        Ok(())
    }
    /// One ordinary `Entity.move(mover, movement)`, outside travel. The collision
    /// scan reads the ground flag to step, and the move's end updates the ground
    /// and collision state (`Entity.setOnGroundWithMovement`). A piston's move
    /// uses its per-tick budget (`limitPistonMovement`).
    pub fn move_with_cause(&mut self, mover: MoverType, movement: Vec3) -> Result<()> {
        self.move_ordinary(&mut MoveGate::new(), mover, movement, false, false)
            .map(|_| ())
    }
    /// An ordinary move; its acting server player can hand off a landing to Game.
    pub fn move_ordinary(
        &mut self,
        gate: &mut MoveGate,
        mover: MoverType,
        movement: Vec3,
        auto_jump: bool,
        handoff: bool,
    ) -> Result<Option<PlayerMovePause>> {
        crate::move_gate::arm(
            gate,
            self.loop_context,
            self.player.entity_id as u32,
            "minecraft:player",
            mover,
            movement,
        );
        if gate.skipped() {
            return Ok(None);
        }
        let auto_jump_from = auto_jump.then_some(self.player.position);
        if handoff {
            if let Some((landing, movement)) = entity_move::move_entity_until_landing(
                self,
                mover,
                gate.delta(),
                |world, dy, grounded, state, p| {
                    world.check_guest_fall_damage(dy, grounded, state, p)
                },
            )? {
                return Ok(Some(PlayerMovePause {
                    landing,
                    movement: PlayerMoveTail {
                        movement,
                        auto_jump_from,
                    },
                }));
            }
        } else {
            entity_move::move_entity(self, mover, gate.delta())?;
        }
        if let Some(before) = auto_jump_from {
            self.auto_jump_after_move(before)?;
        }
        Ok(None)
    }

    pub(super) fn auto_jump_after_move(&mut self, before: Vec3) -> Result<()> {
        let dx = (self.player.position.x - before.x) as f32;
        let dz = (self.player.position.z - before.z) as f32;
        self.update_auto_jump(dx, dz)
    }
}
impl crate::auto_jump::AutoJumpWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    #[inline]
    fn collision_shape(&mut self, p: BlockPos) -> Result<sunrise_core::geometry::VoxelShape> {
        let context = self.context();
        let state = self.block(p);
        let binding = self.collision_binding;
        Ok(binding.collision_shape_with_context(&mut self.collisions(), state, p, &context)?)
    }
    #[inline]
    fn collision_boxes(&mut self, bounds: Aabb) -> Result<Vec<Aabb>> {
        // EntityGetter.getEntityCollisions queries with a small inflation, and
        // CollisionGetter.getCollisions returns these before block shapes.
        let query = bounds.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
        let mut boxes: Vec<_> = self
            .entities
            .iter()
            .copied()
            .filter(|b| b.intersects(query))
            .collect();
        let context = self.context();
        let mut scene = self.collisions();
        for hit in sunrise_world::block_collision_scan::BlockCollisionScan::new(
            &mut scene, context, bounds, false, 1_000_000,
        )? {
            boxes.extend_from_slice(hit?.shape.boxes());
        }
        Ok(boxes)
    }
}
// Current resident movement domain: dry, standing, no vehicles or gliding.
// The shared kernel also handles swimming/crouching/flight when those states
// are supplied by their eventual runtime bindings.
impl MovementStatisticsWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    #[inline]
    fn passenger(&mut self) -> Result<bool> {
        Ok(false)
    }
    #[inline]
    fn swimming(&mut self) -> Result<bool> {
        Ok(self.player.swimming)
    }
    #[inline]
    fn eyes_in_water(&mut self) -> Result<bool> {
        Ok(self.player.is_eye_in_fluid(WATER))
    }
    #[inline]
    fn in_water(&mut self) -> Result<bool> {
        Ok(self.player.fluids.was_touching_water)
    }
    #[inline]
    fn on_climbable(&mut self) -> Result<bool> {
        self.climbable()
    }
    #[inline]
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    #[inline]
    fn sprinting(&mut self) -> Result<bool> {
        Ok(self.player.sprinting)
    }
    #[inline]
    fn crouching(&mut self) -> Result<bool> {
        Ok(self.player.pose == crate::pose::Pose::Crouching)
    }
    #[inline]
    fn fall_flying(&mut self) -> Result<bool> {
        Ok(false)
    }
    #[inline]
    fn award(&mut self, stat: MovementStat, amount: i32) -> Result<()> {
        self.player
            .events
            .push(MotionEvent::Health(crate::health::HealthEvent::Statistic {
                name: stat.name(),
                amount,
            }));
        Ok(())
    }
    #[inline]
    fn exhaust(&mut self, amount: f32) -> Result<()> {
        self.player.cause_food_exhaustion(amount);
        Ok(())
    }
}
impl MovementBlockProvider for MotionWorld<'_, '_, '_> {
    type Error = io::Error;
    #[inline]
    fn movement_block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        Ok(self.block(p))
    }
}
impl EdgeBackoffWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    #[inline]
    fn max_up_step(&mut self) -> Result<f32> {
        Ok(self.player.attribute("minecraft:step_height")? as f32)
    }
    #[inline]
    fn staying_on_ground_surface(&mut self) -> Result<bool> {
        let mut staying = self.player.shift_key_down;
        native_player_stays_on_ground_surface(
            self.loop_context,
            self.player.entity_id as u32,
            &mut staying,
        );
        Ok(staying)
    }
    #[inline]
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    #[inline]
    fn no_collision(&mut self, bounds: Aabb) -> Result<bool> {
        self.no_player_collision(bounds)
    }
}
impl EntityMoveWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    #[inline]
    fn no_physics(&self) -> bool {
        self.player.spectator
    }
    #[inline]
    fn position(&self) -> Vec3 {
        self.player.position
    }
    #[inline]
    fn set_position(&mut self, p: Vec3) -> Result<()> {
        let from = self.player.position;
        self.player.position = p;
        if from != p {
            section_writes::position_written(self.player, from);
        }
        Ok(())
    }
    #[inline]
    fn record_movement(&mut self, mover: MoverType, movement: Vec3) -> Result<()> {
        self.player
            .events
            .push(MotionEvent::Recorded { mover, movement });
        Ok(())
    }
    #[inline]
    fn collision_flags(&self) -> CollisionFlags {
        self.player.collision
    }
    #[inline]
    fn set_collision_flags(&mut self, value: CollisionFlags) {
        self.player.collision = value;
    }
    /// `Entity.limitPistonMovement`: the per-tick 0.51-block budget shared by
    /// every move on an axis. A failed game-time read leaves the accumulator
    /// as it was, as native does.
    #[inline]
    fn limit_piston_movement(&mut self, movement: Vec3) -> Result<Vec3> {
        let time = self.scene.game_time;
        self.player
            .piston
            .limit(movement, || Ok::<i64, Box<dyn std::error::Error>>(time))
    }
    #[inline]
    fn stuck_multiplier(&self) -> Vec3 {
        self.player.stuck_multiplier
    }
    #[inline]
    fn set_stuck_multiplier(&mut self, value: Vec3) {
        self.player.stuck_multiplier = value;
    }
    #[inline]
    fn velocity(&mut self) -> Result<Vec3> {
        Ok(self.player.velocity)
    }
    #[inline]
    fn set_velocity(&mut self, value: Vec3) -> Result<()> {
        self.player.set_velocity(value);
        Ok(())
    }
    #[inline]
    fn back_off_edge(&mut self, movement: Vec3, mover: MoverType) -> Result<Vec3> {
        let input = EdgeBackoffInput {
            movement,
            bounds: self.player.bounds(),
            // Player.maybeBackOffFromEdge: not while flying.
            flying: self.player.flying,
            self_or_player: matches!(mover, MoverType::SelfMovement | MoverType::Player),
            fall_distance: self.player.fall_distance,
            query_budget: 1_000_000,
        };
        edge_backoff::back_off_from_edge(self, input)
            .map_err(|e| io::Error::other(e.to_string()).into())
    }
    #[inline]
    fn collide(&mut self, movement: Vec3) -> Result<Vec3> {
        let max_step = self.player.attribute("minecraft:step_height")? as f32;
        Ok(collision_exclusions::resolve_world_movement(
            &mut self.movement_collisions(),
            &self.player.collision_exclusions,
            MovementRequest {
                entity: player_collision_identity(self.player),
                context: self.context(),
                bounds: self.player.bounds(),
                movement,
                max_step,
                on_ground: self.player.ground.on_ground,
                candidate_budget: 1_000_000,
            },
        )?)
    }
    #[inline]
    fn fall_distance(&self) -> f64 {
        self.player.fall_distance
    }
    #[inline]
    fn fall_reset_clip(&mut self, from: Vec3, to: Vec3) -> Result<bool> {
        // ClipContext.Block.FALLDAMAGE_RESETTING: tagged reset blocks are a full
        // block, not their collider; ClipContext.Fluid.WATER: the water's shape.
        // Both shapes are queried for every visited block (the water's first
        // query fixes its cached shape), either one hitting ends the ray.
        let hit = traverse_blocks(from, to, 1_000_000, |p| {
            let water = match self.water_clip(from, to, p) {
                Ok(hit) => hit,
                Err(e) => return Some(Err(e)),
            };
            let name = &self.assets.registry.state(self.block(p)).unwrap().name;
            let reset = self.resources.fall_reset.contains(name)
                && VoxelShape::block().clip(from, to, p).is_some();
            (water || reset).then_some(Ok(()))
        })?;
        hit.transpose().map(|h| h.is_some())
    }
    #[inline]
    fn reset_fall_distance(&mut self) -> Result<()> {
        self.player.fall_distance = 0.0;
        Ok(())
    }
    #[inline]
    fn movement_history(&mut self) -> &mut MovementHistory {
        &mut self.player.history
    }
    #[inline]
    fn locally_authoritative(&mut self) -> Result<bool> {
        Ok(!self.player.server_copy)
    }
    #[inline]
    fn set_on_ground_with_movement(
        &mut self,
        below: bool,
        horizontal: bool,
        movement: Vec3,
    ) -> Result<()> {
        self.player.ground.on_ground = below;
        self.player.ground.horizontal_collision = horizontal;
        self.check_supporting_block(below, movement)
    }
    /// `LocalPlayer.isHorizontalCollisionMinor`: the move that hit a wall was
    /// within 8 degrees of the direction the input asked for.
    #[inline]
    fn minor_horizontal_collision(&mut self, movement: Vec3) -> Result<bool> {
        Ok(horizontal_collision_minor(
            self.player.yaw,
            self.player.applied_input,
            movement,
        ))
    }
    #[inline]
    fn legacy_on_position(&mut self) -> Result<BlockPos> {
        self.support(movement_support::LEGACY_SUPPORT_OFFSET)
    }
    #[inline]
    fn block_state(&mut self, p: BlockPos) -> Result<BlockStateId> {
        Ok(self.block(p))
    }
    #[inline]
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    #[inline]
    fn check_fall_damage(
        &mut self,
        dy: f64,
        grounded: bool,
        state: BlockStateId,
        position: BlockPos,
    ) -> Result<()> {
        self.before_fall_check(dy, grounded, state, position);
        sunrise_world::living_landing::check_living_fall_damage(self, dy, grounded, state, position)
    }
    #[inline]
    fn removed(&mut self) -> Result<bool> {
        Ok(false)
    }
    #[inline]
    fn can_simulate(&mut self) -> Result<bool> {
        Ok(true)
    }
    #[inline]
    fn restitute(
        &mut self,
        state: BlockStateId,
        position: BlockPos,
        x: bool,
        z: bool,
        resolved: Vec3,
    ) -> Result<()> {
        let input = RestitutionInput {
            block: state,
            effect_position: position,
            collided_x: x,
            collided_z: z,
            vertical_collision: self.player.collision.vertical,
            collision_below: self.player.collision.below,
            living: true,
            resolved_movement: resolved,
            fall_distance: self.player.fall_distance,
        };
        movement_restitution::restitute_movement(self, input)
    }
    #[inline]
    fn client_side(&mut self) -> Result<bool> {
        Ok(false)
    }
    #[inline]
    fn movement_emission(&mut self) -> Result<MovementEmission> {
        <Self as MovementEmissionWorld>::movement_emission(self)
    }
    #[inline]
    fn passenger(&mut self) -> Result<bool> {
        Ok(false)
    }
    /// `Entity.applyMovementEmissionAndPlaySound`: a footstep (`Emission`)
    /// only once the walked distance passes `nextStep`. Emitting on every
    /// move played a step sound each tick, even standing still on the
    /// ground (gravity moves the player every tick), as an endless loop.
    #[inline]
    fn apply_movement_emission(
        &mut self,
        kind: MovementEmission,
        movement: Vec3,
        position: BlockPos,
        state: BlockStateId,
    ) -> Result<()> {
        emission::apply_movement_emission(self, kind, movement, position, state)
    }
    #[inline]
    fn block_speed_factor(&mut self) -> Result<f32> {
        self.player_block_speed_factor()
    }
}
// The player's footsteps and swim sounds. Flapping and the step game event
// are not bound here (the sculk listeners track the player's steps themselves).
impl MotionWorld<'_, '_, '_> {
    /// `Entity.checkSupportingBlock`: also called by `doCheckFallDamage`,
    /// independently of (and without changing) the movement flags.
    pub fn check_supporting_block(&mut self, on_ground: bool, movement: Vec3) -> Result<()> {
        let position = self.player.position;
        let bounds = self.player.bounds();
        let held = self.player.main_hand.clone();
        let context = player_context(
            self.player.entity_id as u32,
            position.y,
            self.player.shift_key_down,
            held.as_deref(),
        );
        let mut collisions = SceneCollisions {
            world: self.scene,
            binding: self.collision_binding,
            subject: Some(self.player.powder_snow_subject()),
        };
        self.player
            .ground
            .check_support(on_ground, bounds, Some(movement), |bounds| {
                find_supporting_block(&mut collisions, context, position, bounds, 1_000_000)
            })?;
        Ok(())
    }

    pub fn check_guest_fall_damage(
        &mut self,
        dy: f64,
        grounded: bool,
        state: BlockStateId,
        position: BlockPos,
    ) -> Result<Option<crate::landing::PlayerLandingPause>> {
        self.before_fall_check(dy, grounded, state, position);
        sunrise_world::living_landing::check_living_fall_damage_until(
            self,
            dy,
            grounded,
            state,
            position,
            |world, state, position, distance| world.block_fall(state, position, distance, true),
        )
    }

    fn before_fall_check(
        &mut self,
        dy: f64,
        grounded: bool,
        state: BlockStateId,
        position: BlockPos,
    ) {
        // LivingEntity.aiStep: Slow Falling and Levitation reset the fall
        // distance every tick, so landing under either does not hurt.
        if self.player.status.slow_falling || self.player.status.levitation.is_some() {
            self.player.fall_distance = 0.0;
        }
        // LivingEntity.checkFallDamage: a landing runs onChangedBlock.
        if grounded && self.player.fall_distance > 0.0 {
            self.player.location_effects.landed = true;
        }
        if self.player.spawn_extra_particles_on_fall && grounded && self.player.fall_distance > 0.0
        {
            self.player.events.push(MotionEvent::SmashLanding {
                state,
                position,
                fall_distance: self.player.fall_distance,
            });
            self.player.spawn_extra_particles_on_fall = false;
        }
        self.player.events.push(MotionEvent::Landing {
            dy,
            grounded,
            state,
            position,
        });
    }
}
