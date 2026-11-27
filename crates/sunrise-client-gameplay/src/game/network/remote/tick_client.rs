use super::*;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Whether entity `id` of the client level rides another (`Entity.vehicle`): its tick is a
    /// ride visit, counted whatever its Pre answers.
    pub(in crate::game) fn client_passenger(&self, id: u32) -> bool {
        self.client
            .remote
            .as_ref()
            .and_then(|r| r.entities.get(&(id as i32)))
            .is_some_and(|t| t.vehicle.is_some())
    }
    /// `Entity.commonTick`'s tickCount for an entity of the client level at its own visit: a
    /// root once its `EntityTickEvent.Pre` let it through (`tickNonPassenger`), a passenger at
    /// its ride visit whatever that Pre answered (`tickPassenger`).
    pub(in crate::game) fn count_client_tick(&mut self, id: u32) {
        if let Some(t) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.entities.get_mut(&(id as i32)))
        {
            t.tick_count = t.tick_count.wrapping_add(1);
        }
    }
    /// The visit of each entity of the client level whose kind is told no tick
    /// (`tick_remote_entities`): its `Entity.commonTick` counts it here. A frozen level visits
    /// only the players and what carries them.
    pub(in crate::game::client_side) fn count_untold_client_ticks(
        &mut self,
        running: bool,
        frozen_ticking: &[i32],
    ) {
        let Some(mut r) = self.client.remote.take() else {
            return;
        };
        let flying: sunrise_level::fast_hash::FxHashSet<u32> = self
            .common
            .creatures
            .projectiles
            .iter()
            .map(|p| p.id)
            .chain(self.common.tridents.iter().map(|t| t.id))
            .collect();
        let told = |id: i32| {
            let id = id as u32;
            self.common
                .mob_index
                .get_ref(&self.common.mobs, id)
                .is_some()
                || self.client.text_displays.0.contains_key(&(id as i32))
                || flying.contains(&id)
        };
        if running {
            for (&id, t) in r.entities.iter_mut() {
                if !told(id) {
                    t.tick_count = t.tick_count.wrapping_add(1);
                }
            }
        } else {
            for &id in frozen_ticking {
                if !told(id)
                    && let Some(t) = r.entities.get_mut(&id)
                {
                    t.tick_count = t.tick_count.wrapping_add(1);
                }
            }
        }
        self.client.remote = Some(r);
    }
    #[inline(always)]
    pub(in crate::game) fn game_side_client(
        &self,
    ) -> (Option<u128>, Option<mod_api::LevelRef>, bool) {
        let level = self.common.native_lifecycle.epoch().and_then(|epoch| {
            let key = self.common.dim.key();
            mod_api::__private::dimension_id(key)?;
            Some(mod_api::LevelRef::with_generation(epoch, key, 0))
        });
        (None, level, false)
    }
    /// `ClientClockManager.tick`: the clock moves on by the game time that
    /// passed since its last tick, at its rate. A game time the server sets
    /// forward (its sync every second) moves the clock with it.
    pub(in crate::game::client_side) fn remote_clock_tick(&mut self) {
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let delta = self.common.world.game_time - r.clock_game_time;
        r.clock_game_time = self.common.world.game_time;
        for clock in r.clocks.values_mut() {
            let _ = client_level_tick::advance_clock(clock, delta);
        }
        self.common.world.day_time = r.clock_time(r.default_clock);
    }
    /// `ClientPacketListener.tick`'s `LevelLoadTracker.tickClientLoad`: the
    /// player's section is shown once the renderer has its mesh and it is 30%
    /// faded in (without a renderer: once its column and the 8 around it are
    /// lit, then the same fade); when the level is
    /// ready the loading screen closes and the server hears `PlayerLoaded`
    /// (`notifyPlayerLoaded`).
    pub(in crate::game::client_side) fn tick_client_load(&mut self) {
        let Some(player) = self.common.player.as_ref() else {
            return;
        };
        if !self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.is_active())
        {
            return;
        }
        let cam_pos = self.visual_camera_position().unwrap_or([
            player.position.x,
            player.position.y + f64::from(self.common.eye),
            player.position.z,
        ]);
        let camera = cam_pos.map(|v| v.floor() as i32);
        let camera_y = camera[1];
        // `Util.toMillis(chunkSectionFadeInTime)`.
        let fade_ms = sunrise_client::gfx::fade_millis(self.client.settings.chunk_fade as f32);
        // `LevelRenderer`'s playerCompiledSectionCallback, run once
        // `isSectionCompiledAndVisible(camera)`: the renderer's own mesh and fade
        // for the camera's section. Without a renderer, the section counts as
        // compiled once `LevelExtractor` would compile it
        // (`SectionUpdateTracker.hasAllNeighbors`: its column and the 8 around
        // it lit, as meshing waits for here).
        let shown = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|renderer| renderer.section_compiled_and_visible(camera, fade_ms));
        let (cx, cz) = (camera[0] >> 4, camera[2] >> 4);
        let neighbours_lit = shown.is_none()
            && (-1..=1).all(|dx| {
                (-1..=1).all(|dz| {
                    self.client
                        .streaming
                        .is_lit(cx.wrapping_add(dx), cz.wrapping_add(dz))
                })
            });
        let shape = self.common.world.shape;
        let check = sunrise_client::level_loading::PlayerCheck {
            outside_build_height: camera_y < shape.min_y || camera_y >= shape.min_y + shape.height,
            spectator: player.spectator,
            alive: player.health.health() > 0.0,
        };
        let now = client_millis();
        // LevelLoadingScreen.tick.
        self.tick_load_progress();
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        if r.level_load.wants_player_section() {
            match shown {
                Some(true) => r.level_load.player_section_ready(),
                Some(false) => {}
                None => r.level_load.player_section_compiled(
                    [cx, camera_y >> 4, cz],
                    neighbours_lit,
                    now,
                    fade_ms,
                ),
            }
        }
        r.level_load.tick_client_load(now, check);
        if !r.level_load.is_level_ready(now) {
            return;
        }
        r.level_load.finish();
        if !r.loaded_sent {
            r.loaded_sent = true;
            self.send(S::PlayerLoaded(Empty));
        }
        // LevelLoadingScreen.onClose.
        self.say_system_now(&sunrise_game_data::text::Text::translate(
            "narrator.ready_to_play",
            &[],
        ));
        self.capture_loaded_world();
    }
    /// `LocalPlayer.tick`'s `Player.tick` work: input, riding, motion and
    /// sleep, and the input `sendChanges` sends for it (`None`: nothing ticked).
    /// `fall_flying_before_hooks` is the flag from before the tick's hooks when the caller
    /// took it before them: a glide a `player_tick_pre` listener starts is then reported as
    /// this tick's start, as a jump-started glide is.
    pub fn tick_local_player(
        &mut self,
        entity_collisions: Vec<sunrise_core::geometry::Aabb>,
        sprint_window: i32,
        fall_flying_before_hooks: Option<bool>,
    ) -> Result<Option<u8>> {
        // Player.tick's ItemCooldowns.tick, after Pre and after
        // handleKeybinds: this path replaces `tick_world`'s for a server's
        // client (multiplayer and the default single player), so the entries
        // advance here or the slot stays shaded until the server says the
        // entry ended. `onCooldownEnded` is the plain ItemCooldowns' no-op on
        // a client. Ticking it after the keybinds keeps a held use from
        // trying again a tick before the server's entry ends.
        self.common.cooldowns.tick();
        // Player.tick's sleep counter (the darkening and the wake-up fade).
        self.common.sleep_timer = sunrise_gameplay_common::game::tick_sleep_timer(
            self.common.sleep_timer,
            self.common.sleeping.is_some(),
        );
        if self.common.player.is_none() {
            return Ok(None);
        }
        // The LevelLoadingScreen takes the input until the level is ready.
        let loading = self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.is_active());
        let playing = self.client.screen == Screen::Playing && !loading && !self.dialog_open();
        let stick = self.pad_move();
        let mut keys = if playing {
            self.sample_movement_keys(stick)
        } else {
            Keys::default()
        };
        // On a server (not this game's own server thread), the stick moves as the keys it sets do
        // (`KeyboardInput.tick`: 8 directions, full speed). The server only hears those keys
        // (`ServerboundPlayerInputPacket`), and an anticheat that predicts from them pulls back
        // a player moving at the stick's own angle and speed.
        if self.client.integrated.is_none() && self.server_address().is_some() {
            keys.analog = None;
        }
        let mut packet_keys = self.update_movement_input(keys);
        // LocalPlayer.applyInput and aiStep: watching through another
        // entity, the player does not move itself; its keys still reach the
        // server, whose sneak check lets go of the camera.
        let keys = if self.controls_camera() {
            packet_keys
        } else {
            Keys::default()
        };
        // The held lead's end eases from the look at the start of the tick
        // (`yRotO`/`xRotO`), as the local tick does; `tick` returns before it
        // reaches its own write when this window is a server client.
        self.client.rope_rot_old = (self.common.yaw, self.common.pitch);
        // Entity.rideTick: the vehicle ticks first, then its rider, which moves itself
        // from where it sat before (`rider_tick`, in the riding branch below, once its
        // look and the vehicle's fluid shelter are this tick's); positionRider then puts
        // it on the new seat and changes nothing else.
        let old_seat = self
            .common
            .riding
            .filter(|_| self.client.screen != Screen::Dead)
            .and_then(|_| self.common.player.as_ref().map(|p| p.position));
        // LocalPlayer.aiStep: on a tick without a vehicle that can jump now, the riding jump's
        // scale drops to 0 (the `else` of `jumpableVehicle != null && getJumpCooldown() == 0`),
        // so a charge left when the rider got off is not kept.
        if self.jumpable_mount().is_none() {
            self.common.mount.jump_scale = 0.0;
        }
        let riding = self.client.screen != Screen::Dead && self.remote_ride(keys);
        let mut rider_movement = None;
        // LocalPlayer.isShiftKeyDown is the input's sneak, riding or not: the tick after a
        // dismount crouches (a passenger never does) on the sneak held to get off.
        if riding && let Some(p) = self.common.player.as_mut() {
            p.shift_key_down = keys.shift;
        }
        // The server sends no passenger's position: they sit where their
        // vehicles are now.
        self.position_riders();
        // Entity.getFluidInteractionBox asks the vehicle: a boat afloat shelters
        // its passenger's eyes (AbstractBoat.modifyPassengerFluidInteractionBox).
        let boat = self
            .common
            .riding
            .and_then(|id| self.common.boats.iter().find(|b| b.id == id))
            .map(|b| sunrise_sim::motion::RiddenBoat {
                under_water: b.under_water(),
                max_y: b.bounds().max_position().y,
            });
        let Some(player) = self.common.player.as_mut() else {
            return Ok(None);
        };
        player.fluids.boat = boat;
        player.is_passenger = riding;
        if !riding && !self.client.replay.playing() {
            self.common.previous_position = player.position;
        }
        let before = (
            player.ground.on_ground,
            [player.velocity.x, player.velocity.y, player.velocity.z],
            [player.position.x, player.position.y, player.position.z],
        );
        let was_flying = player.flying;
        self.sync_player_body();
        self.sync_remote_player_game_mode();
        let sleeping = self.common.sleeping;
        if let Some(at) = sleeping.and_then(|head| self.sleeping_position(head))
            && let Some(player) = self.common.player.as_mut()
        {
            let from = std::mem::replace(&mut player.position, at);
            if from != at
                && let Ok(id) = u32::try_from(self.own_entity_id())
            {
                self.remote_entity_section_step(id, from, at);
            }
        }
        let walks = self.can_move_local_player();
        let section_scope = self.player_section_scope();
        let swim_listener = self.common.has_swim_eligibility_listener();
        // Release the field borrows for the read-only Game host, then resume the same body.
        macro_rules! motion_world {
            ($side:ident, $player:expr) => {
                MotionWorld {
                    side: &$side,
                    entities: &entity_collisions,
                    player: $player,
                    scene: &mut self.common.world,
                    assets: &self.common.pack.assets,
                    collision_binding: self.common.collision,
                    resources: self.common.motion,
                    health: &self.common.health_resources,
                    loop_context: &self.common.loop_context,
                    damage_hooks: &self.common.damage_hooks,
                    tag_items: None,
                    block_registries: None,
                }
            };
        }
        let side_server = self.game_side_client();
        let Some(player) = self.common.player.as_mut() else {
            return Ok(None);
        };
        let body = player.entity_id;
        let was_gliding = fall_flying_before_hooks.unwrap_or(player.fall_flying);
        let dead = self.client.screen == Screen::Dead;
        // LivingEntity.isImmobile: a sleeper lies still, even if its bed's
        // shape is unavailable here; only the server decides when to wake.
        if sleeping.is_some() {
            // Entity.baseTick still samples fluids while sleeping, before LocalPlayer.aiStep.
            let side = client_game_side!(self, side_server);
            player.client_side = true;
            motion_world!(side, player).fluid_base_tick()?;
            if let Some(player) = self.common.player.as_mut().filter(|p| p.entity_id == body) {
                player.velocity = Vec3::ZERO;
                player.pitch = 0.0;
            }
            if !dead {
                self.tick_local_water_vision();
            }
        } else if walks {
            let side = client_game_side!(self, side_server);
            player.client_side = true;
            // LivingEntity.checkAutoSpinAttack: a Riptide spin that touches a
            // mob ends and bounces back; the server deals the hit.
            player.spin_targets.clear();
            if player.auto_spin_attack_ticks > 0 {
                player.spin_targets.extend(
                    self.common
                        .mobs
                        .iter()
                        .filter(|m| !m.removed)
                        .map(|m| (m.id, m.bounds())),
                );
            }
            let mut motion = motion_world!(side, &mut *player);
            motion.begin_local_tick()?;
            let decision = if swim_listener {
                motion.swim_entry()
            } else {
                None
            };
            let answer = self.ask_swim_entry(decision);
            let side_server = self.game_side_client();
            let side = client_game_side!(self, side_server);
            let Some(player) = self.common.player.as_mut().filter(|p| p.entity_id == body) else {
                return Ok(None);
            };
            let mut motion = motion_world!(side, player);
            motion.finish_fluid_tick(answer)?;
            let keys = motion.prepare_local_sprint(keys)?;
            self.player_sprint_tick(keys, sprint_window, None)?;
            if !self
                .common
                .player
                .as_ref()
                .is_some_and(|p| p.entity_id == body)
            {
                return Ok(None);
            }
            // LocalPlayer.aiStep: water vision reads this tick's `Entity.baseTick`
            // `isEyeInFluid(FluidTags.WATER)` sample and `this.isSpectator()`, before `super.aiStep()`.
            self.tick_local_water_vision();
            let side_server = self.game_side_client();
            let side = client_game_side!(self, side_server);
            let Some(player) = self.common.player.as_mut().filter(|p| p.entity_id == body) else {
                return Ok(None);
            };
            // Only travel writes positions; the fluid and sprint decisions precede this pass.
            let _sections =
                section_scope.map(|scope| scope.begin(&mut self.common.native_lifecycle));
            motion_world!(side, &mut *player).finish_local_tick(keys)?;
            // LocalPlayer.aiStep's makeJump adds a key press to this tick's input.
            packet_keys.jump |= player.was_auto_jump;
        } else {
            // LivingEntity.baseTick counts hurtTime down for a dead or riding
            // player too (controlled_simulation_tick does it otherwise): left
            // at 10, the hurt tilt restarted every frame and shook the camera.
            player.health.advance_clock();
            let side = client_game_side!(self, side_server);
            player.client_side = true;
            let mut passenger = motion_world!(side, player);
            if riding && !dead {
                // The rider moves from its old seat; the new one is its place after.
                let seat = passenger.player.position;
                if let Some(from) = old_seat {
                    passenger.player.position = from;
                }
                passenger.begin_fluid_tick()?;
                let decision = if swim_listener {
                    passenger.swim_entry()
                } else {
                    None
                };
                let answer = self.ask_swim_entry(decision);
                let side_server = self.game_side_client();
                let side = client_game_side!(self, side_server);
                let Some(player) = self.common.player.as_mut().filter(|p| p.entity_id == body)
                else {
                    return Ok(None);
                };
                motion_world!(side, player).finish_rider_base_tick(answer);
                self.remote_rider_sprint(keys, sprint_window)?;
                if !self
                    .common
                    .player
                    .as_ref()
                    .is_some_and(|p| p.entity_id == body)
                {
                    return Ok(None);
                }
                self.tick_local_water_vision();
                let side_server = self.game_side_client();
                let side = client_game_side!(self, side_server);
                let Some(player) = self.common.player.as_mut().filter(|p| p.entity_id == body)
                else {
                    return Ok(None);
                };
                let _sections =
                    section_scope.map(|scope| scope.begin(&mut self.common.native_lifecycle));
                let mut passenger = motion_world!(side, player);
                let start = passenger.player.position;
                passenger.finish_rider_tick(keys.direction())?;
                rider_movement = Some(passenger.player.position.subtract(start));
                sunrise_world::entity_move::EntityMoveWorld::set_position(&mut passenger, seat)?;
                // LivingEntity.aiStep: a passenger freezes in powder snow too, and the
                // frostbite overlay shows it (#8156).
                passenger.passenger_block_effects_tick()?;
            } else {
                // Entity.baseTick samples fluids even if movement is skipped by death.
                passenger.fluid_base_tick()?;
            }
        }
        // LivingEntity.baseTick reaches LocalPlayer.tickDeath after the fluid update.
        // The last sample is therefore the one taken on death tick 20.
        if dead {
            self.client.chat_ui.death_time = self.client.chat_ui.death_time.saturating_add(1);
            // LivingEntity.tick runs LocalPlayer.aiStep only while `!this.isRemoved()`.
            if !self.local_player_removed_by_death() {
                self.tick_local_water_vision();
            }
        }
        let Some(player) = self.common.player.as_mut().filter(|p| p.entity_id == body) else {
            return Ok(None);
        };
        self.client
            .client_pick
            .sections
            .get_mut()
            .move_id(body as u32, player.position.components());
        // The server decides fire the window cannot predict (blaze fireballs,
        // fire aspect, lightning): Entity.isOnFire reads the synched flag, so
        // the predicted tick's countdown must not end it while it is set.
        if !player.mod_fire_immune {
            player.fire_ticks = hold_server_fire(
                player.fire_ticks,
                self.client.remote.as_ref().is_some_and(|r| r.own_on_fire),
            );
        }
        // LocalPlayer.onUpdateAbilities: flight turned on or off (a double
        // jump, a spectator, landing) is sent to the server.
        let flying = player.flying;
        // LocalPlayer.aiStep: a glide this client started is sent to the
        // server (ServerboundPlayerCommandPacket START_FALL_FLYING).
        let started_gliding = player.fall_flying && !was_gliding;
        let events = std::mem::take(&mut player.events);
        self.client.previous_eye = self.common.eye;
        self.common.eye += (player.dimensions.eye_height - self.common.eye) * 0.5;
        for event in events {
            match event {
                sunrise_sim::motion::MotionEvent::Emission {
                    position,
                    state,
                    muffled,
                } => {
                    let at = [position.x, position.y, position.z];
                    let sound = self.own_block_sound(at, state, mod_api::BlockSoundCause::Step);
                    // Player.playStepSound: muffled under the swim sound in water.
                    let (volume, pitch) = if muffled {
                        (sound.volume * 0.05, sound.pitch * 0.8)
                    } else {
                        (sound.volume * 0.15, sound.pitch)
                    };
                    self.play_client_player_sound(sound.event.as_str(), volume, pitch);
                }
                sunrise_sim::motion::MotionEvent::Swim { volume, pitch } => {
                    self.play_client_player_sound("minecraft:entity.player.swim", volume, pitch);
                }
                // Entity.doWaterSplashEffect on the client's own level: the
                // server's splash skips this player (Player.playSound), and
                // its particles are the client's own.
                sunrise_sim::motion::MotionEvent::Splash {
                    high_speed,
                    volume,
                    pitch,
                } => {
                    let name = if high_speed {
                        "minecraft:entity.player.splash.high_speed"
                    } else {
                        "minecraft:entity.player.splash"
                    };
                    self.play_client_player_sound(name, volume, pitch);
                }
                sunrise_sim::motion::MotionEvent::SplashParticle {
                    bubble,
                    position,
                    velocity,
                } => {
                    use sunrise_level::particle_options::{ParticleOptions, ParticleType};
                    let kind = if bubble {
                        ParticleType::Bubble
                    } else {
                        ParticleType::Splash
                    };
                    self.client.particles.0.add(
                        &ParticleOptions::simple(kind),
                        [position.x, position.y, position.z],
                        [velocity.x, velocity.y, velocity.z],
                    );
                }
                sunrise_sim::motion::MotionEvent::Chime { volume, pitch } => {
                    self.play_client_player_sound(
                        "minecraft:block.amethyst_block.chime",
                        volume,
                        pitch,
                    );
                }
                // NetherPortalBlock.entityInside on the client's own level:
                // the portal overlay and its trigger sound (the server moves
                // the player).
                sunrise_sim::motion::MotionEvent::InsidePortal { .. } => {
                    self.client_inside_nether_portal();
                }
                // HoneyBlock.maybeDoSlideEffects on the client's own level:
                // the slide sound, heard here (LocalPlayer.playSound); the
                // particles come from the server's event 53.
                sunrise_sim::motion::MotionEvent::HoneySlide { sound: true, .. } => {
                    self.play_client_player_sound("minecraft:block.honey_block.slide", 1.0, 1.0);
                }
                // LivingEntity.causeFallDamage and HoneyBlock.fallOn on the
                // client's own level: the fall and honey sounds are heard here
                // (LocalPlayer.playSound); the server's copy skips this player
                // (Player.playSound).
                sunrise_sim::motion::MotionEvent::Fall(event) => self.block_fall(event),
                _ => {}
            }
        }
        // A stand riding the player follows this move (tickPassenger).
        self.seat_own_stands();
        // Other players on this player (possibly through a plugin seat) follow this tick too;
        // the full pass only when something rides this player.
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.has_passengers(r.entity_id as u32))
        {
            self.position_riders();
        }
        // The server owns health and hunger.
        self.apply_remote_health();
        self.tick_avatar(before, false);
        // LivingEntity.tick: a sleeper's xRot is 0.
        if sleeping.is_some() {
            self.common.pitch = 0.0;
            self.snap_player_model_pitch();
        }
        // The player's own model (third person, the inventory's preview).
        if !self.client.replay.playing() {
            self.tick_player_model_with_rider_movement(rider_movement);
        }
        if !dead {
            self.remote_tick_using();
        }
        if flying != was_flying {
            if let Some(r) = self.client.remote.as_mut() {
                r.abilities.flying = flying;
            }
            self.send(S::PlayerAbilities(play::PlayerAbilitiesRequest {
                flags: if flying { 2 } else { 0 },
            }));
        }
        if started_gliding {
            let id = self.client.remote.as_ref().map_or(0, |r| r.entity_id);
            self.send(S::PlayerCommand(play::PlayerCommand {
                id,
                action: 6,
                data: 0,
            }));
        }
        // LocalPlayer.sendChanges sends the final key presses, not the move vector.
        Ok(Some(
            u8::from(packet_keys.forward)
                | u8::from(packet_keys.backward) << 1
                | u8::from(packet_keys.left) << 2
                | u8::from(packet_keys.right) << 3
                | u8::from(packet_keys.jump) << 4
                | u8::from(packet_keys.shift) << 5
                | u8::from(packet_keys.sprint) << 6,
        ))
    }
    /// `LocalPlayer.aiStep` (877-883): water vision reads the local body's `Entity.baseTick`
    /// `isEyeInFluid(FluidTags.WATER)` sample and `this.isSpectator()`.
    pub(in crate::game::client_side) fn tick_local_water_vision(&mut self) {
        let in_water = self
            .common
            .player
            .as_ref()
            .is_some_and(|p| p.is_eye_in_fluid(sunrise_sim::motion::WATER));
        self.client
            .fog
            .tick(in_water, self.own_player_is_spectator());
    }
    /// One client tick while connected (Minecraft.tick with a ClientLevel).
    pub fn tick_remote(&mut self) -> Result<()> {
        // The particle hook follows the enabled mods before anything this tick can add a
        // particle: the level's packets and ambience, frozen ticks and the first tick included.
        self.sync_render_hooks();
        self.tick_remote_level()?;
        // The rockets boosting this player tick after it, also while it waits
        // for chunks.
        self.tick_own_rockets();
        // Far tiles the samplers asked for (`lod_net`).
        if let Some(r) = &self.client.remote {
            r.session.send_far_terrain();
        }
        // Minecraft.tick sends this even when the player cannot tick yet,
        // including while waiting for chunks. Keep it outside early returns.
        if self.client.remote.as_ref().is_some_and(Remote::has_level) {
            self.send(S::ClientTickEnd(Empty));
        }
        Ok(())
    }
    /// Humanoid held arm poses, selected at the frame's extraction from the equipment and use state
    /// the tick set (`AvatarRenderer.extractRenderState` → `getArmPose`).
    pub fn extract_remote_arm_poses(&mut self) {
        for i in 0..self.common.mobs.len() {
            let m = &self.common.mobs[i];
            if !sunrise_sim::mobs::anim::extracts_humanoid(m) {
                continue;
            }
            // Extraction also runs while the world is frozen: derive the use
            // animation from the current equipment rather than last tick's state.
            if &*m.kind.id != "minecraft:player" {
                let slot = if m.extra.using_off_hand {
                    sunrise_items::equipment::OFFHAND
                } else {
                    sunrise_items::equipment::MAINHAND
                };
                let animation = (m.extra.using_item > 0)
                    .then(|| m.extra.slot_stack(slot))
                    .flatten()
                    .map_or_else(Default::default, |stack| self.item_use_animation(stack));
                self.common.mobs[i].extra.use_animation = animation;
            }
            let m = &self.common.mobs[i];
            let poses = self.held_arm_poses(m.id, m);
            let charge_duration = (&*m.kind.id != "minecraft:player").then(|| {
                let using_slot = if m.extra.using_off_hand {
                    sunrise_items::equipment::OFFHAND
                } else {
                    sunrise_items::equipment::MAINHAND
                };
                m.extra.slot_stack(using_slot).map_or(25, |stack| {
                    sunrise_gameplay_common::game::CommonGame::charge_ticks(stack).max(1)
                }) as i32
            });
            self.common.mobs[i].extra.held_arm_pose = poses;
            if let Some(duration) = charge_duration {
                self.common.mobs[i].extra.ints[5] = duration;
            }
        }
    }
    /// `Level.tickBlockEntities` on a client: the block entities' animations
    /// and client ticks, and the moving pistons' pushes.
    pub fn tick_remote_block_entities(&mut self) {
        self.tick_client_block_entities();
        self.with_level(|b, level, ticks| {
            b.client_tick_block_entities(level, ticks);
        });
        let pushes = std::mem::take(&mut self.common.block_ticks.piston_pushes);
        self.push_piston_entities(pushes);
        self.sync_moving_pistons();
    }
    /// `ClientLevel.tickEntities`: a frozen level only ticks the players
    /// (`TickRateManager.isEntityFrozen`); the section index takes each crossing at its entity's
    /// turn.
    pub fn tick_remote_entities(&mut self, running: bool) {
        self.client
            .client_pick
            .sections
            .get_mut()
            .begin_entity_ticks();
        self.tick_remote_entity_turns(running);
        self.client
            .client_pick
            .sections
            .get_mut()
            .end_entity_ticks();
    }
    pub(in crate::game::client_side) fn tick_remote_entity_turns(&mut self, running: bool) {
        use sunrise_gameplay_common::game::native_ticks::{PLAYER, TickKind};

        let frozen_ticking: Vec<i32> = if running {
            Vec::new()
        } else {
            self.client
                .remote
                .as_ref()
                .map(|r| {
                    r.entities
                        .keys()
                        .copied()
                        .filter(|&id| self.entity_ticks_while_frozen(r, id as u32))
                        .collect()
                })
                .unwrap_or_default()
        };
        // Entity.commonTick's tickCount, at each entity's own visit. The kinds a client tells
        // their tick (mobs and players, projectiles, tridents, text displays) count in their own
        // loops (`count_client_tick`): a root once its Pre let it through, a passenger at its
        // ride visit whatever either Pre answers. The others are visited here, where their
        // setOldPosAndRot runs.
        self.count_untold_client_ticks(running, &frozen_ticking);
        self.tick_text_displays(running);
        if running {
            // Entity.commonTick's setOldPosAndRot for each entity the level ticks.
            if let Some(r) = self.client.remote.as_mut() {
                for t in r.entities.values_mut() {
                    t.previous = t.position;
                    t.previous_yaw = t.yaw;
                    t.previous_pitch = t.pitch;
                }
            }
            for i in self.client.interactions.values_mut() {
                crate::game::client_side::interactions::snapshot_previous(i);
            }
            self.tick_remote_ominous_item_spawners();
        } else {
            // ClientLevel.tickPassenger: non-players in a player-bearing passenger tree still tick.
            if let Some(r) = self.client.remote.as_mut() {
                for &id in &frozen_ticking {
                    if let Some(t) = r.entities.get_mut(&id) {
                        t.previous = t.position;
                        t.previous_yaw = t.yaw;
                        t.previous_pitch = t.pitch;
                    }
                }
            }
            for &id in &frozen_ticking {
                if let Some(i) = self.client.interactions.get_mut(&id) {
                    crate::game::client_side::interactions::snapshot_previous(i);
                }
            }
        }
        // EntityTickEvent.Pre and .Post bracket each mob and remote player individually right
        // before and right after its own turn below.
        let mut round = self.native_tick_round();
        // The mount this client moves ticks in `remote_ride` instead, as its
        // authoritative side (LivingEntity.tick with travelRidden).
        let driven = self.driven_mount().map(|m| m.id);
        let told = round.told_listening();
        // EntityFluidInteraction.update for the entities this window only displays: the
        // server never sends the fluid contact, and the renderers read it (cod, salmon and
        // tropical fish lie on their side without `in_water`, #1591).
        // A frozen level ticks only the players, as below.
        // Entity.isInRain for the wolves, which dry off by their own `Wolf.tick`.
        let (mut fluids, mut rain, mut standings, mut water_fx) = {
            let r = self
                .client
                .remote
                .as_mut()
                .expect("remote entity tick without remote state");
            (
                std::mem::take(&mut r.tick_fluids),
                std::mem::take(&mut r.tick_rain),
                std::mem::take(&mut r.tick_standings),
                std::mem::take(&mut r.tick_water_fx),
            )
        };
        let thundering = {
            let side = self.client_mob_side();
            let w = self.client_mob_world_base(&[], &side);
            let players = self.client.remote.as_ref().map(|r| &r.level_players);
            fluids.clear();
            fluids.extend(self.common.mobs.iter().map(|m| {
                (running
                    || players.is_some_and(|p| p.contains_key(&(m.id as i32)))
                    || self
                        .client
                        .remote
                        .as_ref()
                        .is_some_and(|r| self.entity_ticks_while_frozen(r, m.id)))
                .then(|| w.fluids(m.bounds(), m.eye_y()))
            }));
            rain.clear();
            rain.extend(self.common.mobs.iter().map(|m| {
                m.species() == sunrise_sim::mobs::Species::Wolf
                    && sunrise_sim::mobs::tame::in_rain(m, &w)
            }));
            // The block under each mob the packs' random-model rules read (a grazing
            // cow), one hair below the feet like `Entity.getBlockStateOn`; nothing
            // without rules.
            standings.clear();
            standings.extend(self.common.mobs.iter().map(|m| {
                sunrise_client::mobs::cem::tracks_standing(&m.kind.id).then(|| {
                    let (x, y, z) = (
                        m.position.x.floor() as i32,
                        (m.position.y - 0.0001).floor() as i32,
                        m.position.z.floor() as i32,
                    );
                    w.block_name(x, y, z).to_owned()
                })
            }));
            w.thundering
        };
        // The window's own player already splashes from its motion tick.
        let own = self.client.remote.as_ref().map(|r| r.entity_id);
        for i in 0..self.common.mobs.len() {
            let id = self.common.mobs[i].id;
            // When Pre is subscribed, each entity takes its fluid step in its own turn after Pre.
            if round.pre() && fluids[i].is_some() && driven != Some(id) {
                continue;
            }
            client_fluid_step(
                &mut self.common.mobs[i],
                fluids[i].take(),
                standings[i].take(),
                own,
                &mut water_fx,
            );
        }
        let game_time = self.common.world.game_time;
        // `AbstractPiglin.isConverting`: the dimension is not piglin safe.
        let nether = !self.dimension_type().piglins_zombify;
        // What a wither's side heads can look at, by network entity id: the
        // mobs and the player (position and eye height).
        let wither_targets: Vec<(i32, [f64; 3], f64)> = if self
            .common
            .mobs
            .iter()
            .any(|m| m.species() == sunrise_sim::mobs::Species::Wither)
        {
            let own = self
                .client
                .remote
                .as_ref()
                .zip(self.common.eye_position())
                .map(|(r, eye)| (r.entity_id, eye, eye[1]));
            self.common
                .mobs
                .iter()
                .map(|m| {
                    (
                        m.id as i32,
                        [m.position.x, m.position.y, m.position.z],
                        m.eye_y(),
                    )
                })
                .chain(own)
                .collect()
        } else {
            Vec::new()
        };
        // ExperienceOrb.followNearbyPlayer: the players an orb may go for,
        // their feet and the point `eyeHeight / 2` above them.
        let followed: Vec<([f64; 3], [f64; 3])> = {
            let own = self
                .common
                .player
                .as_ref()
                .filter(|p| !p.health.dead_or_dying() && !p.spectator)
                .map(|p| {
                    let feet = [p.position.x, p.position.y, p.position.z];
                    let eye = f64::from(p.dimensions.eye_height);
                    (feet, [feet[0], feet[1] + eye / 2.0, feet[2]])
                });
            let others = self.client.remote.as_ref().into_iter().flat_map(|r| {
                self.common
                    .mobs
                    .iter()
                    .filter(move |m| m.alive() && r.level_players.contains_key(&(m.id as i32)))
                    .filter(move |m| r.remote_player_info_game_mode(m.id as i32) != Some(SPECTATOR))
                    .map(|m| {
                        let feet = [m.position.x, m.position.y, m.position.z];
                        (
                            feet,
                            [feet[0], feet[1] + (m.eye_y() - feet[1]) / 2.0, feet[2]],
                        )
                    })
            });
            own.into_iter().chain(others).collect()
        };
        // Cat.handleLieDown: the sleeping players (another's, drawn lying in
        // a bed, and this client's own) a lying cat can lie on top of.
        let sleepers: Vec<sunrise_core::geometry::Aabb> = self
            .client
            .remote
            .as_ref()
            .map(|r| {
                self.common
                    .mobs
                    .iter()
                    .filter(|m| {
                        r.level_players.contains_key(&(m.id as i32)) && m.extra.ints[1] != 0
                    })
                    .map(|m| {
                        let p = m.position;
                        sunrise_core::geometry::Aabb::new(
                            sunrise_core::geometry::Vec3::new(p.x - 0.1, p.y, p.z - 0.1),
                            sunrise_core::geometry::Vec3::new(p.x + 0.1, p.y + 0.2, p.z + 0.1),
                        )
                    })
                    .chain(self.common.sleeping.map(|bed| {
                        let p = sunrise_core::geometry::Vec3::new(
                            f64::from(bed[0]) + 0.5,
                            f64::from(bed[1]) + 0.6875,
                            f64::from(bed[2]) + 0.5,
                        );
                        sunrise_core::geometry::Aabb::new(
                            sunrise_core::geometry::Vec3::new(p.x - 0.1, p.y, p.z - 0.1),
                            sunrise_core::geometry::Vec3::new(p.x + 0.1, p.y + 0.2, p.z + 0.1),
                        )
                    }))
                    .collect()
            })
            .unwrap_or_default();
        if self.client.remote.is_none() {
            return;
        }
        let mut using = Vec::new();
        let mut mob_fx = water_fx;
        let tick_listened = mod_api::__private::listened(mod_api::HookId::PlayerTickPre)
            || mod_api::__private::listened(mod_api::HookId::PlayerTickPost);
        // When tick hooks are subscribed, each entity hears them with the list in place: keep
        // each slot's id so row shifts during hooks still resolve the remaining slots.
        let slots: Vec<u32> = if told {
            self.common.mobs.iter().map(|m| m.id).collect()
        } else {
            Vec::new()
        };
        let mut places = None;
        for (slot, rain) in rain.iter().copied().enumerate() {
            let Some(mut i) =
                sunrise_gameplay_common::game::native_ticks::slot_place(&places, &slots, slot)
            else {
                continue;
            };
            let id = self.common.mobs[i].id;
            let player = self
                .client
                .remote
                .as_ref()
                .unwrap()
                .level_players
                .contains_key(&(id as i32));
            let ticks_frozen = !running
                && (player
                    || self
                        .client
                        .remote
                        .as_ref()
                        .is_some_and(|r| self.entity_ticks_while_frozen(r, id)));
            if !running && !ticks_frozen {
                continue;
            }
            // The mount this client drives ticks in `remote_ride`, told nothing: its visit.
            if driven == Some(id) {
                self.count_client_tick(id);
                continue;
            }
            // A passenger's ride visit counts it before its own Pre is told, whatever it answers.
            let passenger = self.client_passenger(id);
            if passenger {
                self.count_client_tick(id);
            }
            if round.canceled(id) {
                continue;
            }
            // Its own Pre, then its fluid step and its tick.
            let kind = told.then(|| {
                if player {
                    TickKind::Fixed(PLAYER)
                } else {
                    TickKind::Mob(self.common.mobs[i].kind.clone())
                }
            });
            if let Some(kind) = &kind
                && round.pre()
            {
                let stamp = self.common.mobs.stamp();
                let ticks = self.native_tick_pre_one(&mut round, id, kind.id());
                if self.common.mobs.stamp() != stamp {
                    places = Some(sunrise_gameplay_common::game::native_ticks::places_of(
                        self.common.mobs.iter().map(|m| m.id),
                    ));
                }
                match sunrise_gameplay_common::game::native_ticks::slot_place(&places, &slots, slot)
                {
                    Some(at) if ticks => i = at,
                    _ => continue,
                }
                client_fluid_step(
                    &mut self.common.mobs[i],
                    fluids[slot].take(),
                    standings[slot].take(),
                    own,
                    &mut mob_fx,
                );
            }
            // A root's own visit, once its Pre let it through.
            if !passenger {
                self.count_client_tick(id);
            }
            let animation = if player && self.common.mobs[i].extra.using_item > 0 {
                let extra = &self.common.mobs[i].extra;
                let slot = if extra.using_off_hand {
                    sunrise_items::equipment::OFFHAND
                } else {
                    sunrise_items::equipment::MAINHAND
                };
                extra
                    .slot_stack(slot)
                    .map_or_else(Default::default, |stack| self.item_use_animation(stack))
            } else {
                Default::default()
            };
            let player_ref = tick_listened
                .then(|| {
                    self.client.remote.as_ref().and_then(|r| {
                        let uuid = *r.level_players.get(&(id as i32))?;
                        Some(mod_api::__private::game::player_ref(
                            mod_api::EntityRef::new(self.common.native_lifecycle.epoch()?, id),
                            uuid,
                        ))
                    })
                })
                .flatten();
            let spectator_player =
                player
                    && self.client.remote.as_ref().is_some_and(|r| {
                        r.remote_player_info_game_mode(id as i32) == Some(SPECTATOR)
                    });
            let r = self.client.remote.as_mut().unwrap();
            let stepped = &mut r.stepped;
            let m = &mut self.common.mobs[i];
            let section_from = m.position;
            if player {
                // AvatarRenderer.extractRenderState: isSpectator, from the player list.
                m.extra.spectator = spectator_player;
            }
            m.previous = m.position;
            // `Entity.deltaMovementOnPreviousTick`: the elytra's flight yaw
            // lerps from it, so another player's model does not snap when a
            // new tick's velocity arrives.
            m.extra.previous_velocity = m.velocity;
            m.previous_yaw = m.yaw;
            m.previous_body_yaw = m.body_yaw;
            m.previous_head_yaw = m.head_yaw;
            m.previous_head_pitch = m.head_pitch;
            m.previous_limb_speed = m.limb_speed;
            m.age += 1;
            if m.species() == sunrise_sim::mobs::Species::Rabbit {
                sunrise_sim::mobs::rabbit::base_tick(m);
            }
            if matches!(
                m.species(),
                sunrise_sim::mobs::Species::Horse
                    | sunrise_sim::mobs::Species::Donkey
                    | sunrise_sim::mobs::Species::Mule
                    | sunrise_sim::mobs::Species::SkeletonHorse
                    | sunrise_sim::mobs::Species::ZombieHorse
            ) {
                sunrise_sim::mobs::equine::tail_tick(m);
            }
            if m.species() == sunrise_sim::mobs::Species::Creeper {
                sunrise_sim::mobs::hostile::creeper_client_tick(m);
            }
            if matches!(
                m.species(),
                sunrise_sim::mobs::Species::Guardian | sunrise_sim::mobs::Species::ElderGuardian
            ) {
                sunrise_sim::mobs::guardian::client_tick(m);
            }
            // PolarBear.tick: the stand animation follows the synched flag.
            if m.species() == sunrise_sim::mobs::Species::PolarBear {
                sunrise_sim::mobs::polar_bear::client_tick(m);
            }
            // The particles these mobs' own ticks draw on the client side.
            {
                use sunrise_sim::mobs::{
                    Species as S, cube, illager, illusioner, nautilus, tame, wither,
                };
                match m.species() {
                    S::Wolf => tame::client_tick(m, rain, &mut mob_fx),
                    S::Slime | S::MagmaCube | S::SulfurCube => cube::client_tick(m, &mut mob_fx),
                    S::Nautilus | S::ZombieNautilus => nautilus::client_tick(m, &mut mob_fx),
                    S::Illusioner => {
                        illusioner::client_tick(m, &mut mob_fx);
                        illager::cast_particles(m, &mut mob_fx);
                    }
                    S::Evoker => illager::cast_particles(m, &mut mob_fx),
                    S::Wither => {
                        wither::client_smoke(m, &mut mob_fx);
                        // `getInvulnerableTicks() > 0`: the charge's cloud,
                        // after the heads' smoke as in `aiStep`.
                        wither::charging_particles(m, &mut mob_fx);
                    }
                    _ => {}
                }
            }
            if m.invulnerable > 0 {
                m.invulnerable -= 1;
            }
            if m.hurt_time > 0 {
                m.hurt_time -= 1;
            }
            if m.extra.max_health_dirty {
                m.extra.max_health_dirty = false;
                m.health = m.health.min(m.attrs.max_health);
            }
            if !m.alive() {
                m.death_time += 1;
            }
            // LivingEntity.updateSwingTime.
            m.anim.attack_anim_o = m.anim.attack_anim;
            sunrise_sim::mobs::ai::tick_swing(m);
            // Entity.tick: the SteppedInterpolationHandler's step.
            if let Some(at) = stepped
                .get_mut(&(m.id as i32))
                .and_then(|h| h.interpolate())
            {
                place_interpolated(m, at);
            }
            // `ClientLevel.tickNonPassenger`'s `commonTick` interpolated the
            // entity before `Player.tick`, which `Pre` precedes.
            if let Some(player) = player_ref {
                self.native_player_tick_pre(mod_api::__private::PlayerTickPreCall { player });
            }
            let head_lerps = &mut self.client.remote.as_mut().unwrap().head_lerps;
            let m = &mut self.common.mobs[i];
            // LivingEntity.baseTick's tickEffects, inside this tick (after its Pre, and a
            // player's tick Pre): its effects count down and fade.
            super::super::potions::client_tick_effects(
                &mut m.extra.client_effects,
                &mut m.extra.darkness_blend,
            );
            // LivingEntity.aiStep: the head's own interpolation.
            if let Some((yaw, steps)) = head_lerps.get_mut(&(m.id as i32))
                && *steps > 0
            {
                m.head_yaw += wrap_degrees(*yaw - m.head_yaw) / *steps as f32;
                *steps -= 1;
            }
            // Fox.tick: the head tilt and crouch follow the synched flags.
            if m.species() == sunrise_sim::mobs::Species::Fox {
                sunrise_sim::mobs::fox::client_tick(m);
            }
            // Cat.tick: the lying and relaxing amounts follow the synched flags.
            if m.species() == sunrise_sim::mobs::Species::Cat {
                m.anim.vo = m.anim.v;
                sunrise_sim::mobs::tame::cat_amounts(m);
                sunrise_sim::mobs::tame::cat_lying_on_sleeper(m, &sleepers);
            }
            // Chicken.aiStep: the wings follow the server's on-ground flag.
            if m.species() == sunrise_sim::mobs::Species::Chicken {
                sunrise_sim::mobs::farm::chicken_flap(m);
            }
            // Parrot.calculateFlapping: the wings follow the server's
            // on-ground flag and the shoulder it sits on (the glide is the
            // server's).
            if m.species() == sunrise_sim::mobs::Species::Parrot {
                sunrise_sim::mobs::parrot::flap(m);
            }
            // LivingEntity.updatingUsingItem: the ticks the item has been
            // used before this one. Another player's use particles; a mob's
            // drawn bow only needs the count (its pull follows it).
            if player {
                // AvatarRenderer.getArmPose: the used item's use animation
                // poses the arm.
                m.extra.use_animation = animation;
            }
            if m.extra.using_item > 0 {
                if player {
                    let item = if m.extra.using_off_hand {
                        m.extra.slot_stack(sunrise_items::equipment::OFFHAND)
                    } else {
                        m.extra.slot_stack(sunrise_items::equipment::MAINHAND)
                    }
                    .map(|s| s.item.clone());
                    if let Some(item) = item {
                        let eye = m.eye_height();
                        let at = [m.position.x, m.position.y, m.position.z];
                        using.push((
                            item,
                            m.extra.using_item as u32 - 1,
                            at,
                            eye,
                            m.head_pitch,
                            m.yaw,
                        ));
                    }
                }
                m.extra.using_item += 1;
            }
            // LivingEntity.tick's tickHeadTurn: a mob's BodyRotationControl,
            // a player's body following its walk and look.
            if player {
                remote_player_head_turn(m);
                // LivingEntity.tick: the glide count, wings and swim amount.
                m.tick_avatar_pose(m.fluids.in_water);
                // LivingEntity.tick: a sleeper's xRot is 0.
                if m.extra.ints[1] != 0 {
                    m.head_pitch = 0.0;
                    m.previous_head_pitch = 0.0;
                }
            } else if m.alive() {
                m.body_rotation_tick();
            }
            // Squid.aiStep; WalkAnimationState.update from the horizontal distance.
            sunrise_sim::mobs::aquatic::squid_client_step(m, &mut mob_fx);
            let d = sunrise_core::geometry::length(
                m.position.x - m.previous.x,
                m.position.z - m.previous.z,
            ) as f32;
            let (target, factor) = if m.species() == sunrise_sim::mobs::Species::Frog {
                (sunrise_sim::mobs::frog::walk_target(m, d), 0.4)
            } else if sunrise_sim::mobs::camel::is_camel(m.species()) {
                sunrise_sim::mobs::camel::walk_target(m, d)
            } else {
                ((d * 4.0).min(1.0), 0.4)
            };
            m.limb_speed += (target - m.limb_speed) * factor;
            m.limb_position += m.limb_speed;
            if m.species() == sunrise_sim::mobs::Species::Rabbit {
                sunrise_sim::mobs::rabbit::post_tick(m);
            }
            if m.species() == sunrise_sim::mobs::Species::Bat {
                sunrise_sim::mobs::bat::setup_animation_states(m);
            }
            // The animation states a server computes in its own `ai_step`,
            // and the particles vanilla draws on the client.
            sunrise_sim::mobs::sync::client_tick(m, game_time, &mut mob_fx);
            match m.species() {
                // WitherBoss.aiStep: the side heads turn toward their targets.
                sunrise_sim::mobs::Species::Wither => {
                    sunrise_sim::mobs::wither::client_tick(m, |id| {
                        wither_targets
                            .iter()
                            .find(|t| t.0 == id)
                            .map(|t| (t.1, t.2))
                    });
                }
                // Hoglin and piglin `isConverting` outside the Nether.
                sunrise_sim::mobs::Species::Hoglin
                | sunrise_sim::mobs::Species::Piglin
                | sunrise_sim::mobs::Species::PiglinBrute => {
                    sunrise_sim::mobs::piglin::client_converting(m, nether);
                }
                sunrise_sim::mobs::Species::Panda => {
                    sunrise_sim::mobs::panda::client_tick(m, thundering);
                }
                // Shulker.tick's onPeekAmountChange: the lid moves this
                // client's player, whose movement is its own.
                sunrise_sim::mobs::Species::Shulker => {
                    sunrise_sim::mobs::shulker::client_push(m, &mut mob_fx);
                }
                _ => {}
            }
            let section_id = m.id;
            let section_to = m.position;
            // The committed body is where the index finds it before any hook reads it; the
            // section events stay at their own turn below.
            self.pick_section_move(section_id, section_to.components());
            // `refreshDimensions` from its tick: a `minecraft:scale` the server sent since
            // (`LivingEntity.refreshDirtyAttributes`), a shulker's lid (`onPeekAmountChange`).
            self.native_client_copy_size(section_id, None);
            if let Some(player) = player_ref {
                self.native_player_tick_post(mod_api::__private::PlayerTickPostCall { player });
            }
            if section_from != section_to {
                self.remote_entity_section_step(section_id, section_from, section_to);
            }
            // Its own Post, right after its turn.
            if let Some(kind) = &kind
                && round.post()
            {
                let stamp = self.common.mobs.stamp();
                self.native_tick_post_one(&round, id, kind.id());
                if self.common.mobs.stamp() != stamp {
                    places = Some(sunrise_gameplay_common::game::native_ticks::places_of(
                        self.common.mobs.iter().map(|m| m.id),
                    ));
                }
            }
        }
        // ExperienceOrb.tick runs on a client too (all but the merges and the
        // push out of a block, which are the server's): gravity, buoyancy,
        // collisions and the pull of the nearest player between the server's
        // updates; the server removes it on pickup.
        let (world, facts, collision) = (
            &self.common.world,
            self.common.res.facts(),
            self.common.collision,
        );
        let border = Some(self.common.border.geometry());
        let mut lava_random = None;
        let mut next_float = || {
            lava_random
                .get_or_insert_with(random_source::entity)
                .next_float()
        };
        if running {
            let side_server = self.game_side_client();
            let sections = self.client.client_pick.sections.get_mut();
            self.common.orbs.retain_mut(|o| {
                let distance = |feet: &[f64; 3]| -> f64 {
                    (feet[0] - o.position.x).powi(2)
                        + (feet[1] - o.position.y).powi(2)
                        + (feet[2] - o.position.z).powi(2)
                };
                let target = followed
                    .iter()
                    .filter(|(feet, _)| distance(feet) < 64.0)
                    .min_by(|a, b| distance(&a.0).total_cmp(&distance(&b.0)))
                    .map(|(_, target)| *target);
                // Entity.commonTick: setOldPosAndRot, then
                // LinearInterpolationHandler.doInterpolate moves 1/remaining of
                // the way to the server's position, so the frame draws the step
                // from the old position.
                let old = o.position;
                if let Some(l) = self
                    .client
                    .remote
                    .as_mut()
                    .and_then(|r| r.lerps.get_mut(&(o.id as i32)))
                    .filter(|l| l.steps > 0)
                {
                    let alpha = 1.0 / f64::from(l.steps);
                    o.position = Vec3::new(
                        o.position.x + (l.target[0] - o.position.x) * alpha,
                        o.position.y + (l.target[1] - o.position.y) * alpha,
                        o.position.z + (l.target[2] - o.position.z) * alpha,
                    );
                    l.steps -= 1;
                    sections.move_id(o.id, o.position.components());
                }
                let before = o.position;
                let side = client_game_side!(self, side_server);
                let kept = o.tick(
                    world,
                    facts,
                    collision,
                    target,
                    false,
                    &mut next_float,
                    border,
                    &self.common.res.motion().fluids,
                    &self.common.loop_context,
                    Some(&self.common.res.motion().blocks),
                    &side,
                    &mut |_, _| {},
                );
                // `tick` set `previous` to the stepped position: keep the
                // pre-step one, as `xo` is in vanilla.
                o.previous = old;
                // AbstractInterpolationHandler.applyPredictedMovement: the orb's
                // own move carries the target along (vanilla also checks the
                // moved target for collisions; that check is left out here).
                if let Some(l) = self
                    .client
                    .remote
                    .as_mut()
                    .and_then(|r| r.lerps.get_mut(&(o.id as i32)))
                    .filter(|l| l.steps > 0)
                {
                    l.target[0] += o.position.x - before.x;
                    l.target[1] += o.position.y - before.y;
                    l.target[2] += o.position.z - before.z;
                }
                if kept {
                    sections.move_id(o.id, o.position.components());
                } else {
                    sections.remove(o.id);
                }
                kept
            });
        }
        // ItemEntity.tick runs on a client too: gravity, collisions and
        // friction move the item between the server's updates (every 20
        // ticks). Ordinary removal remains server-authoritative. Mods can opt
        // into predicting consumption at block contact.
        self.client
            .remote
            .as_mut()
            .unwrap()
            .head_lerps
            .retain(|_, (_, steps)| *steps > 0);
        let mut entered = Vec::new();
        if running {
            entity_section_steps::tick_items(self, &mut entered);
        }
        // Entity.doWaterSplashEffect on this window: Level.addParticle draws
        // the bubbles and splashes. ClientLevel.playSeededSound drops
        // Entity.playSound, so the sound stays the server's.
        for splash in entered {
            self.apply_item_splash(splash);
        }
        let _ = self.handle_mob_events_reusable(&mut mob_fx);
        if let Some(r) = self.client.remote.as_mut() {
            r.tick_fluids = fluids;
            r.tick_rain = rain;
            r.tick_standings = standings;
            r.tick_water_fx = mob_fx;
        }
        for (item, ticks, at, eye, pitch, yaw) in using {
            self.remote_player_eating(&item, ticks, at, eye, pitch, yaw);
        }
        // ClientLevel.tick: the sky flash fades in a frozen level too.
        self.common.flash = self.common.flash.saturating_sub(1);
        // A shulker's lid in mob_fx can move an observing boat and shift
        // its interpolation target. Lend the targets only after that push.
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let mut lerps = std::mem::take(&mut r.lerps);
        let mut cart_lerps = std::mem::take(&mut r.cart_lerps);
        if running {
            self.guardian_bubbles();
            // The dragon: LivingEntity's interpolation, inside its aiStep.
            let dragon_lerp = self
                .common
                .dragon
                .as_ref()
                .and_then(|d| lerps.get_mut(&(d.id as i32)))
                .filter(|l| l.steps > 0)
                .map(|l| {
                    let steps = l.steps;
                    l.steps -= 1;
                    (l.target, l.rotation.map(|r| r.0), steps)
                });
            self.tick_remote_dragon(dragon_lerp);
            if let Some(dragon) = self.common.dragon.as_ref() {
                self.pick_section_move(dragon.id, dragon.position.components());
            }
            // A mod's general rows: the non-living interpolation, and no mod code.
            if let Some(r) = self.client.remote.as_mut() {
                general::tick_remote_general(&mut r.general.rows, &mut lerps);
            }
            self.tick_remote_armor_stands(|_| true);
            // The yaw of a boat eases toward the server's over the same steps
            // as its position, read before `step` counts one off.
            let turns: HashMap<i32, (f32, u32)> = lerps
                .iter()
                .filter(|(_, l)| l.steps > 0)
                .filter_map(|(id, l)| l.rotation.map(|r| (*id, (r.0, l.steps))))
                .collect();
            // Boats, minecarts, projectiles and the rest ease toward the
            // server's position over three ticks (`lerps`).
            let mut step = |id: u32, at: Vec3| match lerps.get_mut(&(id as i32)) {
                Some(l) if l.steps > 0 => {
                    let n = f64::from(l.steps);
                    l.steps -= 1;
                    Vec3::new(
                        at.x + (l.target[0] - at.x) / n,
                        at.y + (l.target[1] - at.y) / n,
                        at.z + (l.target[2] - at.z) / n,
                    )
                }
                _ => at,
            };
            self.tick_remote_objects(
                &mut step,
                |id, yaw| {
                    turns.get(&(id as i32)).map_or(yaw, |&(target, steps)| {
                        super::super::remote_objects::lerp_yaw_step(yaw, target, steps)
                    })
                },
                |c| {
                    cart_lerps
                        .get_mut(&(c.id as i32))
                        .is_some_and(|l| l.tick(c))
                },
            );
            self.tick_remote_projectiles(&mut step);
            // LightningBolt.tick on a client.
            self.client_tick_bolts();
        } else if !frozen_ticking.is_empty() {
            let ticking_set: sunrise_level::fast_hash::FxHashSet<u32> =
                frozen_ticking.iter().map(|&id| id as u32).collect();
            self.tick_remote_armor_stands(|id| ticking_set.contains(&id));
        }
        lerps.retain(|_, l| l.steps > 0);
        if let Some(r) = self.client.remote.as_mut() {
            for (id, l) in lerps {
                r.lerps.entry(id).or_insert(l);
            }
            for (id, l) in cart_lerps {
                r.cart_lerps.entry(id).or_insert(l);
            }
        }
    }
    /// SteppedInterpolationHandler's step, as the mobs', `LivingEntity.baseTick`'s
    /// tickEffects, and the pick index's move.
    fn tick_remote_armor_stands(&mut self, ticks: impl Fn(u32) -> bool) {
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        for a in &mut self.common.armor_stands {
            if !ticks(a.id) {
                continue;
            }
            let step = r
                .stepped
                .get_mut(&(a.id as i32))
                .and_then(|h| h.interpolate());
            a.interpolate(step.map(|at| {
                let p = at.position;
                (Vec3::new(p[0], p[1], p[2]), at.yaw, at.pitch)
            }));
            // LivingEntity.baseTick's tickEffects.
            super::super::potions::client_tick_effects(
                &mut a.client_effects,
                &mut a.darkness_blend,
            );
            self.client
                .client_pick
                .sections
                .get_mut()
                .move_id(a.id, a.position.components());
        }
    }
    /// `GuardianRenderer.getEntityToLookAt`: a guardian's eye follows its
    /// synched attack target, else the camera.
    pub(in crate::game::client_side) fn tick_remote_guardian_eyes(&mut self, running: bool) {
        use sunrise_sim::mobs::{Species, Who, guardian};
        if !running {
            return;
        }
        let camera = self.camera_eye_position();
        let mut looks = std::mem::take(&mut self.client.guardian_look_scratch);
        looks.clear();
        looks.extend(
            self.common
                .mobs
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    matches!(m.species(), Species::Guardian | Species::ElderGuardian) && m.alive()
                })
                .filter_map(|(i, m)| {
                    let look = match m.ai.target.filter(|_| m.extra.flag) {
                        Some(Who::Mob(id)) => self
                            .common
                            .mobs
                            .iter()
                            .find(|t| t.id == id)
                            .map(|t| [t.position.x, t.eye_y(), t.position.z]),
                        _ => camera,
                    };
                    look.map(|look| (i, look))
                }),
        );
        for &(i, look) in &looks {
            guardian::look_at(&mut self.common.mobs[i], look);
        }
        self.client.guardian_look_scratch = looks;
    }
    /// The local player's visit in `ClientLevel.tickEntities`: `EntityTickEvent.Pre`,
    /// `LocalPlayer.tick` (once loaded), then `.Post`. The input `sendChanges` sends after the
    /// block entities ticked, if it runs.
    pub fn tick_remote_own_player(&mut self) -> Result<Option<u8>> {
        use sunrise_gameplay_common::game::native_ticks::{PLAYER, TickRound};
        // ClientLevel.tickEntities skips a removed entity. Keep the last player state
        // for LevelExtractor.extractPlayerState, which still sees it on the death screen.
        if self.local_player_removed_by_death() {
            return Ok(None);
        }
        // LocalPlayer.tick and sendChanges run once the client has loaded (hasClientLoaded),
        // wherever the player is: a chunk that hasn't come yet has no blocks here, as vanilla's
        // empty chunk has none, so the player falls through it until the chunk comes.
        let loaded = self.common.player.is_some()
            && self.client.remote.as_ref().is_some_and(|r| r.loaded_sent);
        // The entities that tick before the player pushed it before its turn began, so
        // before its Pre.
        if loaded {
            let pushed = self.remote_pushes(true);
            self.apply_remote_push(pushed);
        }
        // NeoForge's ClientLevel.tickNonPassenger: Pre, then unless it was canceled
        // Entity.commonTick, LocalPlayer.tick and Post. A passenger's tickPassenger runs
        // commonTick first and tells Pre inside Entity.rideTick, so it counts whatever Pre
        // answers. The admitted player is told and counted whether it has its body yet, has
        // loaded, sleeps or rides.
        let own = self
            .client
            .remote
            .as_ref()
            .filter(|r| r.has_logical_player)
            .map(|r| r.entity_id as u32);
        let mut round = TickRound::with_told(self.native_tick_told());
        let ticks = own.is_none_or(|id| {
            let passenger = self.common.riding.is_some();
            if passenger {
                self.count_own_tick();
            }
            let ticks = self.native_tick_pre_one(&mut round, id, PLAYER);
            if ticks && !passenger {
                self.count_own_tick();
            }
            ticks
        });
        let result = if ticks && loaded {
            self.client.drop_spam_throttler.tick();
            let entity_collisions = self.entity_collisions(Mover::Player);
            let sprint_window = self.client.settings.sprint_window;
            // PlayerTickEvent.Pre and Post bracket Player.tick, inside the entity's turn.
            let player = (mod_api::__private::listened(mod_api::HookId::PlayerTickPre)
                || mod_api::__private::listened(mod_api::HookId::PlayerTickPost))
            .then(|| self.local_player())
            .flatten();
            // The flag a glide starts from is the one before the hook, so a listener that
            // starts the glide from `player_tick_pre` is reported as this tick's start.
            let fall_flying_before_hooks =
                self.common.player.as_ref().is_some_and(|p| p.fall_flying);
            if let Some(player) = player {
                self.native_player_tick_pre(mod_api::__private::PlayerTickPreCall { player });
            }
            self.native_local_player_fit_sizes();
            let result = self.tick_local_player(
                entity_collisions,
                sprint_window,
                Some(fall_flying_before_hooks),
            );
            // `EntityEvent.Size` for the pose the tick set.
            self.native_local_player_size();
            if let Some(player) = player {
                self.native_player_tick_post(mod_api::__private::PlayerTickPostCall { player });
            }
            result
        } else {
            Ok(None)
        };
        if ticks && let Some(id) = own {
            self.native_tick_post_one(&round, id, PLAYER);
        }
        // The entities added after the player tick after it, so after Post: `Entity.push`
        // adds each to the player's deltaMovement during their tick, from where they and
        // the player ended it. A velocity or teleport the server sends before the next
        // tick replaces that deltaMovement, push included
        // (`ClientPacketListener.handleSetEntityMotion`, `handleMovePlayer`), so the push
        // lands on the velocity now, not on the next move.
        if loaded {
            let after = self.remote_pushes(false);
            self.apply_remote_push(after);
        }
        result
    }
    /// `Entity.commonTick`'s tickCount for the local player.
    fn count_own_tick(&mut self) {
        if let Some(r) = self.client.remote.as_mut() {
            r.own_tick_count = r.own_tick_count.wrapping_add(1);
        }
    }
    pub fn tick_remote_player(&mut self, running: bool, hold: bool) -> Result<()> {
        // Inventory.tick's ItemStack.inventoryTick: the pop time runs down.
        if !hold {
            for pop in &mut self.client.hotbar_pop {
                *pop = pop.saturating_sub(1);
            }
        }
        // ClientLevel.tick: the border's size change, while the level runs.
        if running && let Err(error) = self.common.border.tick() {
            log::warn!("client world border tick refused: {error}");
        }
        self.common.player_border();
        self.tick_remote_entities(running);
        self.tick_random_textures();
        // Breeze.tick's block dust: drawn by each client for the breezes it sees.
        if running {
            self.tick_breeze_effects();
            self.tick_sniffer_client();
            self.update_song_volumes();
        }
        self.tick_remote_guardian_eyes(running);
        self.tick_hand();
        if running {
            self.prepare_particle_sources();
        } else if self.common.clock.still() {
            self.settle_rain_fog();
        }
        self.tick_frame_visuals(running);
        // Under `SUNRISE_STILL` they wait for the world too: endermen's and blazes' particles
        // draw on the particles' random, and effects count down, on every tick a frozen
        // client runs, as many as the run took to get there.
        if !hold {
            self.tick_client_ambient();
            self.tick_client_effects();
        }
        // Gui.tick: the titles, the action bar and the held item's name.
        if !hold {
            self.tick_hud();
        } else {
            // Only its countdowns wait: lying down still opens the bed's chat, waking closes it.
            self.tick_in_bed_chat();
        }
        self.tick_subtitles();
        self.tick_client_load();
        // Minecraft.tick: Level.tickEntities (this player among them), then
        // Level.tickBlockEntities, then LocalPlayer.sendChanges. A piston's push
        // lands after the player's own move, and the packet carries what it left:
        // the position and the ground and collision flags `Entity.move(PISTON, ...)`
        // set. A frozen level ticks no block entities; a step advances the
        // moving pistons created by the block event once, rather than finishing
        // them during the frozen ticks between steps.
        let sent = self.tick_remote_own_player();
        if running {
            self.tick_remote_block_entities();
        }
        let input = sent?;
        // Like the block-entity push above, collider displacement belongs in this tick's
        // LocalPlayer.sendChanges, before the client closes the tick on the wire.
        let start = sunrise_world::moving_colliders::Scope::placed();
        self.carry_pass(&start);
        self.observe_carry_age(&start);
        if let Some(input) = input {
            self.remote_send_state(input);
        }
        Ok(())
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::block_props::BlockPropsGameExt1 as _;

use sunrise_gameplay_common::game::collision_source::CollisionSourceGameExt1 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::player::PlayerGameExt1 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::can_start_swimming::GameHooks as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;

use sunrise_gameplay_common::game::piston_push::PistonPushGameExt1 as _;

use sunrise_gameplay_common::game::piston_push::carrying::CarryingGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

use crate::game::client_side::remote_client::minecarts::CartLerpClientExt as _;

use sunrise_gameplay_common::game::native_hooks::can_start_swimming::SprintGameExt as _;
