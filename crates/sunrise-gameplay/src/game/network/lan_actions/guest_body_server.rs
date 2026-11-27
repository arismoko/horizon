use super::*;
use sunrise_core::geometry::Vec3;
pub(in crate::game::server_side) use sunrise_gameplay_common::game::lan_actions::guest_body::initialize;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_sim::{lan::hub::CommittedMove, landing::PlayerLandingPause};

/// The server copy of the acting guest's body, borrowing its player and the level from `game`.
/// A macro so that the borrow can end where the game itself is needed (a hook's owning host)
/// and begin again after it.
macro_rules! motion_world {
    ($game:ident, $side:ident, $player:ident) => {
        MotionWorld {
            side: &$side,
            entities: &[],
            player: $player,
            scene: &mut $game.common.world,
            assets: &$game.common.pack.assets,
            collision_binding: $game.common.collision,
            resources: $game.common.motion,
            health: &$game.common.health_resources,
            tag_items: Some(server_world_items!($game)),
            loop_context: &$game.common.loop_context,
            damage_hooks: &$game.common.damage_hooks,
            block_registries: Some(&$game.common.pack.block_registries),
        }
    };
}

// End the low-level body loan before its death or protection has to use Game. At a
// distinct hurt site the next operation must already see the same hit's settlement.
macro_rules! settle_guest_hit {
    ($game:ident, $side:ident, $world:ident, $id:ident) => {
        if $world.player.health.damage_completion.is_some() {
            if let Err(collected) = $game.finish_guest_damage_before_resume() {
                return (collected, None);
            }
            let Some(player) = $game
                .common
                .player
                .as_mut()
                .filter(|p| p.entity_id as u32 == $id)
            else {
                return (Vec::new(), None);
            };
            $side = server_game_side!($game);
            $world = motion_world!($game, $side, player);
        }
    };
}

impl crate::game::ServerGame<'_> {
    /// `handlePlayerPositionChange`: complete the accepted movement before later
    /// packets and `ServerPlayer.tick` can observe the player.
    pub fn guest_accepted_move(&mut self, conn: u32, commit: CommittedMove) {
        let CommittedMove {
            movement: accepted,
            horizontal_collision,
            from,
            jump_yaw,
        } = commit;
        let Some(g) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return;
        };
        let id = g.entity_id as u32;
        let spectator = g.spectator();
        let dimensions = g.dimensions();
        let at = accepted.position;
        if let Some(yaw) = jump_yaw {
            self.guest_jump(conn, from, yaw);
        }
        if sunrise_gameplay_common::game::entity_sections::crossed(from, at) {
            self.entity_section_step(id, from, at);
        }
        if spectator {
            return;
        }
        let Some(events) = self.as_guest(conn, |game| {
            let leather_boots = super::super::worn_boots::leather_boots(&game.armor);
            let boat = game.riding_boat_fluid_box();
            let Some(player) = game.common.player.as_mut() else {
                return Vec::new();
            };
            player.fluids.boat = boat;
            player.leather_boots = leather_boots;
            player.shift_key_down = accepted.sneaking;
            player.record_fall_reset = game.common.adv.marks.fall.is_some();
            let mut side = server_game_side!(game);
            let mut w = motion_world!(game, side, player);
            let [x, y, z] = accepted.delta;
            let movement = Vec3::new(x, y, z);
            if w.set_on_ground_with_movement(accepted.on_ground, horizontal_collision, movement)
                .is_ok()
                && !touching_unloaded_chunk(at, dimensions, |x, z| {
                    w.scene.chunks.contains_key(&(x, z))
                })
            {
                let landed = (|| -> sunrise_game_data::Result<_> {
                    // Entity.doCheckFallDamage queries support again before fallOn.
                    w.check_supporting_block(accepted.on_ground, movement)?;
                    let pos = w.legacy_on_position()?;
                    let state = w.block_state(pos)?;
                    w.check_guest_fall_damage(y, accepted.on_ground, state, pos)
                })();
                let tail = match landed {
                    Ok(Some(PlayerLandingPause::Trample(trample))) => {
                        let mut collected = std::mem::take(&mut w.player.events);
                        game.trample_farmland(id, true, trample);
                        let Some(player) = game
                            .common
                            .player
                            .as_mut()
                            .filter(|p| p.entity_id as u32 == id)
                        else {
                            return collected;
                        };
                        collected.append(&mut player.events);
                        player.events = collected;
                        side = server_game_side!(game);
                        w = motion_world!(game, side, player);
                        w.finish_trample_fall(trample).ok().flatten()
                    }
                    Ok(Some(PlayerLandingPause::Damage(tail))) => Some(tail),
                    Ok(None) | Err(_) => None,
                };
                if let Some(tail) = tail {
                    if let Err(collected) = game.finish_guest_damage_before_resume() {
                        return collected;
                    }
                    let Some(player) = game
                        .common
                        .player
                        .as_mut()
                        .filter(|p| p.entity_id as u32 == id)
                    else {
                        return Vec::new();
                    };
                    side = server_game_side!(game);
                    w = motion_world!(game, side, player);
                    let _ = w.finish_guest_landing(tail);
                }
            }
            let player = w.player;
            if y > 0.0 {
                player.reset_fall_distance();
            }
            player.record_fall_reset = false;
            let moved = player
                .position
                .subtract(Vec3::new(from[0], from[1], from[2]));
            let side = server_game_side!(game);
            let _ = motion_world!(game, side, player).check_movement_statistics(moved);
            game.movement_stats(moved);
            let Some(player) = game.common.player.as_mut() else {
                return Vec::new();
            };
            let events = std::mem::take(&mut player.events);
            game.record_guest_motion_combat(&events);
            events
        }) else {
            return;
        };
        let fall_end = events.iter().find_map(|event| match event {
            MotionEvent::FallDistanceReset(at) => Some(at.components()),
            _ => None,
        });
        self.guest_motion_events(conn, at, events);
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            return;
        };
        // LivingEntity.hurtServer: the turn already applied death protection.
        // If it did not save the guest, finish ServerPlayer.die before the
        // next packet can move them or change their drops.
        if host.hub.guests[gi].health <= 0.0 {
            self.guest_die(gi, conn);
        }
        // ServerPlayer.resetFallDistance: the first reset in this packet ends
        // the tracked fall. Use its position and the health after protection.
        if let Some(end) = fall_end
            && self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.play.get(&conn))
                .is_some_and(|play| play.marks.fall.is_some())
        {
            self.as_guest(conn, |game| {
                if let Some(start) = game.common.adv.marks.fall.take()
                    && game
                        .player
                        .as_ref()
                        .is_some_and(|p| p.health.health() > 0.0)
                {
                    game.trigger_distance_at("minecraft:fall_from_height", start, Some(end));
                }
            });
        }
    }
    pub(in crate::game::server_side) fn guest_motion_events(
        &mut self,
        conn: u32,
        at: [f64; 3],
        events: Vec<MotionEvent>,
    ) {
        for event in events {
            match event {
                MotionEvent::Health(sunrise_sim::health::HealthEvent::DamageBroadcast {
                    source,
                }) => self.guest_damage_broadcast(conn, source),
                MotionEvent::HurtSound(sound) => self.guest_hurt_sound(conn, sound),
                MotionEvent::Fall(sunrise_sim::landing::FallEvent::Particles {
                    state,
                    position,
                    count,
                    speed,
                }) => self.server_landing_dust(state, position, count, speed),
                MotionEvent::Fall(sunrise_sim::landing::FallEvent::Sound {
                    sound,
                    position: Some(pos),
                    volume,
                    pitch,
                }) if sound.block_sound().is_some() => {
                    if let Some(name) = sound.block_sound() {
                        let at = [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 0.5,
                            f64::from(pos.z) + 0.5,
                        ];
                        self.send_level_sound(LevelSound::new(name, at, volume, pitch), None);
                    }
                }
                MotionEvent::Fall(sunrise_sim::landing::FallEvent::Sound {
                    sound,
                    volume,
                    pitch,
                    ..
                }) => {
                    if let Some(name) = sound.player_fall_sound() {
                        self.send_level_sound(LevelSound::new(name, at, volume, pitch), Some(conn));
                    }
                }
                MotionEvent::Fall(sunrise_sim::landing::FallEvent::FlyableFall {
                    distance,
                    multiplier,
                }) => {
                    if let Some((uuid, id, name)) = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.uuid, g.entity_id, g.name.clone()))
                    {
                        self.as_guest(conn, |game| {
                            game.native_player_flyable_fall(uuid, id, &name, distance, multiplier);
                        });
                    }
                }
                MotionEvent::Fall(sunrise_sim::landing::FallEvent::Broadcast(event))
                    if sunrise_sim::landing::honey_particle_count(event as i8).is_some() =>
                {
                    if let Some(host) = self.server.lan.as_ref()
                        && let Some(id) = host.hub.guest(conn).map(|g| g.entity_id)
                    {
                        host.hub.entity_event(id, event as i8);
                    }
                }
                MotionEvent::Fall(sunrise_sim::landing::FallEvent::BlockFallSound {
                    position,
                    state,
                }) => {
                    let entity = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.entity_id as u32, "minecraft:player"));
                    let sound = self.selected_block_sound(
                        [position.x, position.y, position.z],
                        state,
                        entity,
                        mod_api::BlockSoundCause::Fall,
                    );
                    self.send_level_sound(
                        // `Player.playSound` plays in the players' source.
                        LevelSound::new(
                            sound.event.as_str(),
                            at,
                            sound.volume * 0.5,
                            sound.pitch * 0.75,
                        )
                        .in_source(mod_api::SoundSource::Players),
                        Some(conn),
                    );
                }
                MotionEvent::Fall(event) => {
                    let player = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| g.uuid);
                    if let Some(vibration) = self.landing_block_updates(player, &event) {
                        self.as_guest(conn, |game| game.post_landing_vibration(vibration));
                    }
                }
                MotionEvent::Snowflake { position, velocity } => {
                    if let Some(host) = self.server.lan.as_ref() {
                        crate::game::lan_particles::send_snowflake_particle(
                            host,
                            [position.x, position.y, position.z],
                            [velocity.x, velocity.y, velocity.z],
                        );
                    }
                }
                MotionEvent::Drown(bubbles) => {
                    if !self.drown_bubbles(at, [0.0; 3], bubbles) {
                        continue;
                    }
                    if let Some(host) = self.server.lan.as_ref()
                        && let Some(g) = host.hub.guest(conn)
                    {
                        host.hub.entity_event(
                            g.entity_id,
                            sunrise_gameplay_common::game::combat_common::DROWN_EVENT,
                        );
                    }
                }
                MotionEvent::EnterBlock { state } => {
                    self.as_guest(conn, |game| game.trigger_enter_block(state));
                }
                MotionEvent::HoneySlide {
                    pos,
                    sound,
                    particles,
                } => {
                    if self.common.world.game_time % 20 == 0 {
                        self.as_guest(conn, |game| game.trigger_honey_slide([pos.x, pos.y, pos.z]));
                    }
                    let Some((id, at)) = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.entity_id, g.position))
                    else {
                        continue;
                    };
                    if sound {
                        let name = "minecraft:block.honey_block.slide";
                        self.send_level_sound(LevelSound::new(name, at, 1.0, 1.0), Some(conn));
                    }
                    if particles && let Some(host) = self.server.lan.as_ref() {
                        host.hub.entity_event(id, 53);
                    }
                }
                _ => {}
            }
        }
    }
    /// The guest's connection-path body: fluid and block effects, hazards,
    /// regeneration, food and travel (`ServerPlayer.doTick`). Accepted reports
    /// have already run their fall checks; the body still walks their history
    /// for `Entity.applyEffectsFromBlocks` and the movement a glide reads.
    pub fn guest_server_tick(&mut self, conn: u32, body: bool) {
        // The last move's fall stands in for the server's delta movement.
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let fell = host
            .hub
            .guest(conn)
            .and_then(|g| g.moves.last().map(|m| m.delta[1]))
            .unwrap_or(0.0);
        self.guest_glide_tick(conn, fell);
        let Some((moves, from_ground)) = self.server.lan.as_mut().and_then(|host| {
            host.hub.update_guest(conn, |g| {
                let moves = std::mem::take(&mut g.moves);
                let from_ground = if moves.is_empty() {
                    g.on_ground
                } else {
                    g.moves_from_ground
                };
                (moves, from_ground)
            })
        }) else {
            return;
        };
        let g = self.server.lan.as_ref().unwrap().hub.guest(conn).unwrap();
        let id = g.entity_id as u32;
        if g.spectator() {
            // ServerPlayer.doTick runs the player's whole tick for a spectator
            // too, unless its box touches an unloaded chunk
            // (`touchingUnloadedChunk`, `guest_body_ticks`): its effects
            // (LivingEntity.baseTick), Player.aiStep's shoulder entities and
            // tickRegeneration, and foodData.tick go on while it watches
            // (#11312). The hazards, the travel and the movement statistics
            // below are what a spectator's body skips.
            if !body {
                return;
            }
            self.as_guest(conn, |game| game.tick_mob_effects());
            self.as_guest(conn, |game| {
                game.handle_shoulder_entities();
                let rules = game.world.health_rules;
                let blocking = game.player_blocking_component().is_some();
                let Some(player) = game.common.player.as_mut() else {
                    return;
                };
                player.health.advance_body_clock();
                let _ = player.peaceful_regeneration(
                    &game.common.loop_context,
                    &|| crate::game::entity_server::hit_reads_of!(game),
                    rules,
                );
                let _ = player.food_tick(
                    &game.common.loop_context,
                    crate::game::entity_server::damage_level_of!(
                        game,
                        &sunrise_sim::health::NO_ATTACK,
                        no_push,
                        None,
                        acting_equipment!(game.common)
                    ),
                    &game.common.health_resources,
                    rules,
                );
                game.finish_guest_damage();
                let Some(player) = game
                    .common
                    .player
                    .as_mut()
                    .filter(|p| p.entity_id as u32 == id)
                else {
                    return;
                };
                player.body_yaw = sunrise_sim::mobs::living::player_body_yaw(
                    player.body_yaw,
                    game.common.yaw,
                    0.0,
                    0.0,
                    game.common.swing.animation > 0.0,
                    blocking,
                );
                let events = std::mem::take(&mut player.events);
                game.record_combat(&events);
            });
            return;
        }
        let at = g.position;
        let dimensions = g.dimensions();
        let (sneaking, sleeping) = (g.sneaking, g.sleeping);
        let riding = g.vehicle.is_some();
        if g.motion
            .as_ref()
            .is_some_and(|m| m.auto_spin_attack_ticks > 0)
        {
            self.as_guest(conn, |game| game.server_spin_attack());
        }
        let in_wall = !sleeping && self.in_wall(conn, at, dimensions);
        let events = self.as_guest(conn, |game| {
            let (rules, motion) = (game.world.health_rules, game.motion);
            let health = &motion.health;
            let leather_boots = super::super::worn_boots::leather_boots(&game.armor);
            let freeze_immune = game.wears_freeze_immune();
            let boat = game.riding_boat_fluid_box();
            let swim_listener = game.has_swim_eligibility_listener();
            let Some(player) = game.common.player.as_mut() else {
                return (Vec::new(), None);
            };
            player.fluids.boat = boat;
            player.leather_boots = leather_boots;
            let mut side = server_game_side!(game);
            let mut w = motion_world!(game, side, player);
            w.player.health.advance_body_clock();
            if w.begin_fluid_tick().is_ok() {
                let decision = if swim_listener { w.swim_entry() } else { None };
                let answer = game.ask_swim_entry(decision);
                let Some(player) = game
                    .common
                    .player
                    .as_mut()
                    .filter(|p| p.entity_id as u32 == id)
                else {
                    return (Vec::new(), None);
                };
                side = server_game_side!(game);
                w = motion_world!(game, side, player);
                if w.swim_and_burn_tick(answer).is_ok() {
                    settle_guest_hit!(game, side, w, id);
                    w.finish_fluid_base_tick();
                }
            }
            // Player.aiStep: a flying player that isn't riding starts every tick
            // with no fall distance, so flying down in creative and switching to
            // survival lands without the flight's distance.
            if w.player.flying && !riding {
                w.player.fall_distance = 0.0;
            }
            // The tick's whole movement, for the delta movement a glide's wall
            // hit reads (`handleMovePlayer` gives the server copy the client's
            // own movement).
            let vector = |[x, y, z]: [f64; 3]| Vec3::new(x, y, z);
            let tick_movement = moves.iter().fold(Vec3::ZERO, |sum, accepted| {
                sum.add_vector(vector(accepted.delta))
            });
            // Entity.move's movementThisTick, using the actual accepted endpoints.
            let end = w.player.position;
            let start = moves.first().map_or(end, |first| {
                vector(first.position).subtract(vector(first.delta))
            });
            w.player.history.clear();
            let mut from = start;
            for accepted in &moves {
                let to = vector(accepted.position);
                w.player
                    .history
                    .add(sunrise_world::entity_move::MovementRecord {
                        from,
                        to,
                        original: Some(vector(accepted.delta)),
                    });
                from = to;
            }
            let _ = w.server_stuck_in_blocks(start);
            // A ServerPlayer travels on the server too
            // (MoveSimulationType.AUTHORITATIVE_SIDE_AND_SERVER), so every tick
            // on a ladder or vine LivingEntity.handleOnClimbable resets its fall
            // distance: climbing down doesn't hurt where it lands (#1255).
            if w.climbable().unwrap_or(false) {
                w.player.fall_distance = 0.0;
            }
            // ServerGamePacketListenerImpl.handlePlayerPositionChange ends the
            // impulse context on LivingEntity.hasLandedInLiquid: a move into a
            // liquid that does not rise, or a wind charge that lands in water
            // would waive the fall after it.
            let liquid = w.player.fluids.was_touching_water || w.player.is_in_lava();
            let delta_y = w.player.velocity.y;
            w.player.impulse.landed_in_liquid(delta_y, liquid);
            // Entity.applyEffectsFromBlocks along those moves: magma, cactus,
            // lit campfires and grown sweet berry bushes hurt (#1305).
            w.player.shift_key_down = sneaking;
            let _ = w.player.set_sleeping(sleeping);
            w.player.freeze_immune = freeze_immune;
            let mut contact = None;
            let inside_effects = loop {
                let progress = w.server_block_damage_until_hit(start, &mut contact);
                settle_guest_hit!(game, side, w, id);
                match progress {
                    Ok(Some(effects)) => break effects,
                    Ok(None) => continue,
                    Err(_) => break mod_api::InsideEffects::default(),
                }
            };
            if !inside_effects.is_empty() {
                let mut collected = std::mem::take(&mut w.player.events);
                let _ = w;
                let _ = side;
                game.flush_player_inside(inside_effects);
                let Some(player) = game.common.player.as_mut() else {
                    return (collected, None);
                };
                collected.append(&mut player.events);
                player.events = collected;
                side = server_game_side!(game);
                w = motion_world!(game, side, player);
            }
            let _ = w.finish_server_block_damage_freezing();
            settle_guest_hit!(game, side, w, id);
            let _ = w.update_soul_speed_block();
            // LivingEntity.baseTick's air supply: decided here, then `living_breathe` is
            // asked with the game itself, which `w` borrows, and its answer applied.
            if let Ok(step) = w.decide_air() {
                // This tick's events so far, kept if the hook leaves no acting player.
                let mut collected = std::mem::take(&mut w.player.events);
                let answer = match step {
                    Some(step) if sunrise_sim::breathing::heard() => {
                        game.ask_player_breath(step).map(|breath| (step, breath))
                    }
                    step => step.map(|step| (step, step.vanilla)),
                };
                let Some(player) = game.common.player.as_mut() else {
                    return (collected, None);
                };
                collected.append(&mut player.events);
                player.events = collected;
                side = server_game_side!(game);
                w = motion_world!(game, side, player);
                if let Some((step, breath)) = answer {
                    let _ = w.apply_air(step, breath);
                    settle_guest_hit!(game, side, w, id);
                }
                let _ = w.tick_lava_contact();
                settle_guest_hit!(game, side, w, id);
            }
            w.reset_unlit_fire();
            if w.check_below_world().is_ok() {
                settle_guest_hit!(game, side, w, id);
                if in_wall {
                    let _ = w.suffocation_hurt();
                    settle_guest_hit!(game, side, w, id);
                }
            }
            let mut events = std::mem::take(&mut w.player.events);
            // Powder snow and cauldrons put the fire out along those moves.
            game.block_extinguish_player(&events);
            // Entity.baseTick: rain puts out a burning player.
            game.rain_extinguish_player();
            // WitherRoseBlock.entityInside along those moves.
            game.apply_player_inside_events(&events);
            // `LivingEntity.baseTick` runs the equipment tick before location effects.
            game.tick_equipment_enchantments(crate::game::commands::Target::Player, id);
            // LivingEntity.baseTick: a new block position runs the worn
            // enchantments' location_changed effects (frost walker).
            game.location_changed_effects([start.x, start.y, start.z], from_ground);
            // LivingEntity.baseTick: outside the world border, then tickEffects.
            game.border_damage();
            game.tick_mob_effects();
            // `LivingEntity.baseTick` ran effects before the remaining dirty
            // attributes refresh in `LivingEntity.tick`. `Player.tick` then
            // chooses its pose, including for a passenger (#4509 / #3308).
            let Some(player) = game.common.player.as_mut() else {
                return (events, None);
            };
            let side = server_game_side!(game);
            let mut w = motion_world!(game, side, player);
            if riding {
                let _ = w.player.refresh_attributes();
                let _ = w.passenger_pose_tick();
            } else {
                let _ = w.server_player_pose_tick();
            }
            game.apply_player_waypoint_updates();
            game.update_soul_speed_location();
            game.handle_shoulder_entities();
            let Some(player) = game.common.player.as_mut() else {
                return (Vec::new(), None);
            };
            let _ = player.peaceful_regeneration(
                &game.common.loop_context,
                &|| crate::game::entity_server::hit_reads_of!(game),
                rules,
            );
            let _ = player.food_tick(
                &game.common.loop_context,
                crate::game::entity_server::damage_level_of!(
                    game,
                    &sunrise_sim::health::NO_ATTACK,
                    no_push,
                    None,
                    acting_equipment!(game.common)
                ),
                health,
                rules,
            );
            game.finish_guest_damage();
            let Some(player) = game
                .common
                .player
                .as_mut()
                .filter(|p| p.entity_id as u32 == id)
            else {
                return (events, None);
            };
            events.append(&mut player.events);
            player.carry_client_movement(tick_movement);
            let side = server_game_side!(game);
            let _ = motion_world!(game, side, player).server_travel();
            game.finish_guest_damage();
            // Player.aiStep's entity contacts run with the player's body,
            // even when the level and its mobs are frozen. Flush their damage
            // events and marked velocity through the existing guest path.
            game.touch_pufferfish();
            let blocking = body && game.player_blocking_component().is_some();
            let Some(player) = game.common.player.as_mut() else {
                return (events, None);
            };
            if body {
                let movement = player.position.subtract(vector(at));
                player.body_yaw = sunrise_sim::mobs::living::player_body_yaw(
                    player.body_yaw,
                    game.common.yaw,
                    movement.x,
                    movement.z,
                    game.common.swing.animation > 0.0,
                    blocking,
                );
            }
            // Landing damage from this travel reaches the tracker too.
            events.append(&mut player.events);
            // A hurt since the last tick marked the velocity (Entity.markHurt):
            // ServerEntity.sendChanges sends it as it is after this travel.
            let marked = std::mem::take(&mut player.health.sync_velocity);
            let velocity = marked.then_some(player.velocity);
            game.record_guest_motion_combat(&events);
            (events, velocity)
        });
        let Some((events, marked)) = events else {
            return;
        };
        self.as_guest(conn, |game| {
            game.sculk_step_on_player();
            game.tick_player_sculk_events();
            // Entity.restituteMovementAfterCollisions: GameEvent.BOUNCE.
            for event in &events {
                if matches!(event, MotionEvent::Bounce { .. }) {
                    game.post_entity_event("minecraft:bounce", sunrise_sim::mobs::Who::Player);
                }
            }
        });
        self.guest_motion_events(conn, at, events);
        if let Some(v) = marked {
            if mod_api::__private::listened(mod_api::HookId::PlayerVelocity)
                && let Some((uuid, name)) = self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guest(conn))
                    .map(|g| (g.uuid, g.name.clone()))
            {
                self.native_player_velocity(uuid, &name, [v.x, v.y, v.z]);
            }
            if let Some(g) = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
            {
                g.send(&C::SetEntityMotion(play::SetEntityMotion {
                    id: g.entity_id,
                    velocity: [v.x, v.y, v.z],
                }));
            }
        }
    }
    pub(in crate::game::server_side) fn record_guest_motion_combat(
        &mut self,
        events: &[MotionEvent],
    ) {
        // Player.causeFallDamage: a fall of 2 blocks or more counts.
        for event in events {
            if let MotionEvent::Fall(sunrise_sim::landing::FallEvent::Statistic(cm)) = event {
                self.award_custom_stat("fall_one_cm", *cm);
            }
        }
        self.record_combat(events);
        self.wear_armor(events);
    }

    /// Flush a server player's collected `InsideEffects` with a live `&mut ServerGame`
    /// loan (`InsideBlockEffectApplier.StepBasedCollector.applyAndClear`), then
    /// play the extinguish hiss once if the sweep put a burning player out.
    pub(in crate::game) fn flush_player_inside(&mut self, mut effects: mod_api::InsideEffects) {
        if effects.is_empty() {
            return;
        }
        let was_on_fire = self.common.player.as_ref().is_some_and(|p| p.is_on_fire());
        let mut access = GamePlayerInsideAccess { game: self };
        effects.apply_and_clear(&mut access);
        if was_on_fire
            && let Some(p) = self.common.player.as_ref()
            && !p.is_on_fire()
        {
            let at = [p.position.x, p.position.y, p.position.z];
            self.extinguish_sound(at);
        }
    }

    /// `WitherRoseBlock.entityInside` and `step_on`'s `EntityWrite::AddEffect`
    /// queued along a server player's moves: vanilla's Peaceful check applies
    /// only to the wither rose, not to general `LivingEntity.addEffect`.
    pub(in crate::game) fn apply_player_inside_events(&mut self, events: &[MotionEvent]) {
        for event in events {
            match event {
                MotionEvent::InsideWitherRose => self.wither_rose_contact(),
                MotionEvent::InsideEffect {
                    id,
                    duration,
                    amplifier,
                } => {
                    self.give_player_effect(id, *duration, i32::from(*amplifier));
                }
                _ => {}
            }
        }
    }
}

/// Live server backing for [`mod_api::InsideEntityView`] while flushing a server
/// player's `InsideEffects` collector directly against `ServerGame`.
struct GamePlayerInsideAccess<'a, 'g> {
    game: &'a mut crate::game::ServerGame<'g>,
}

impl mod_api::InsideEntityAccess for GamePlayerInsideAccess<'_, '_> {
    #[inline]
    fn id(&self) -> u32 {
        self.game
            .common
            .player
            .as_ref()
            .map_or(0, |p| p.entity_id as u32)
    }

    #[inline]
    fn kind(&self) -> &str {
        "minecraft:player"
    }

    #[inline]
    fn is_alive(&self) -> bool {
        self.game
            .common
            .player
            .as_ref()
            .is_some_and(|p| !p.health.dead_or_dying())
    }

    #[inline]
    fn position(&self) -> [f64; 3] {
        self.game
            .common
            .player
            .as_ref()
            .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z])
    }

    #[inline]
    fn velocity(&self) -> [f64; 3] {
        self.game
            .common
            .player
            .as_ref()
            .map_or([0.0; 3], |p| [p.velocity.x, p.velocity.y, p.velocity.z])
    }

    #[inline]
    fn fire_ticks(&self) -> i32 {
        self.game.common.player.as_ref().map_or(0, |p| p.fire_ticks)
    }

    #[inline]
    fn is_on_fire(&self) -> bool {
        self.game
            .common
            .player
            .as_ref()
            .is_some_and(|p| p.is_on_fire())
    }

    #[inline]
    fn fall_distance(&self) -> f64 {
        self.game
            .common
            .player
            .as_ref()
            .map_or(0.0, |p| p.fall_distance)
    }

    #[inline]
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        use sunrise_gameplay_common::game::api_values::TryIntoApi as _;
        let level = self.game.level(self.game.common.dim.id())?;
        self.game
            .level_block_state(level, pos)
            .and_then(|id| id.try_into_api().ok())
    }

    #[inline]
    fn push(&mut self, delta: [f64; 3]) -> bool {
        if !delta.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(p) = self.game.common.player.as_mut() else {
            return false;
        };
        p.velocity = p
            .velocity
            .add_vector(Vec3::new(delta[0], delta[1], delta[2]));
        true
    }

    #[inline]
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        if !velocity.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(p) = self.game.common.player.as_mut() else {
            return false;
        };
        p.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        true
    }

    #[inline]
    fn make_stuck_in_block(&mut self, speed_multiplier: [f64; 3]) -> bool {
        if !speed_multiplier.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(p) = self.game.common.player.as_mut() else {
            return false;
        };
        p.apply_stuck_in_block(Vec3::new(
            speed_multiplier[0],
            speed_multiplier[1],
            speed_multiplier[2],
        ));
        true
    }

    #[inline]
    fn reset_fall_distance(&mut self) {
        if let Some(p) = self.game.common.player.as_mut() {
            p.fall_distance = 0.0;
        }
    }

    #[inline]
    fn set_remaining_fire_ticks(&mut self, ticks: i32) {
        if let Some(p) = self.game.common.player.as_mut() {
            p.set_remaining_fire_ticks(ticks);
        }
    }

    #[inline]
    fn add_effect(&mut self, effect: mod_api::EffectKey, duration: i32, amplifier: u8) -> bool {
        let Some(def) = mod_api::__private::effect_index(effect)
            .and_then(sunrise_game_data::potions::effect_by_index)
        else {
            return false;
        };
        if self.game.common.player.is_none() {
            return false;
        }
        let instance =
            sunrise_level::effects::EffectInstance::from_def(def, duration, i32::from(amplifier));
        self.game.add_player_effect(instance)
    }

    #[inline]
    fn hurt_with_source(
        &mut self,
        damage_type: &str,
        amount: f32,
        direct: Option<mod_api::EntityRef>,
        causing: Option<mod_api::EntityRef>,
    ) -> bool {
        let Some(p) = self.game.common.player.as_ref() else {
            return false;
        };
        let subject_id = p.entity_id as u32;
        let subject_pos = [p.position.x, p.position.y, p.position.z];
        let Some((direct, causing, from)) = self.game.resolve_inside_damage_sources(
            subject_id,
            crate::game::commands::Target::Player,
            subject_pos,
            direct,
            causing,
            None,
        ) else {
            return false;
        };
        self.game.hurt_target(
            crate::game::commands::Target::Player,
            amount,
            damage_type,
            direct,
            causing,
            from,
        )
    }

    #[inline]
    fn set_block(&mut self, pos: [i32; 3], state: mod_api::BlockState) -> bool {
        use sunrise_gameplay_common::game::api_values::TryIntoEngine as _;
        let Some(level) = self.game.level(self.game.common.dim.id()) else {
            return false;
        };
        let Ok(id) = state.try_into_engine(&self.game.common) else {
            return false;
        };
        let flags = mod_api::BlockUpdateFlags::NEIGHBORS | mod_api::BlockUpdateFlags::CLIENTS;
        self.game
            .level_set_block(level, pos, id, flags)
            .unwrap_or(false)
    }

    #[inline]
    fn destroy_block(&mut self, pos: [i32; 3], drop: bool) -> bool {
        let Some(level) = self.game.level(self.game.common.dim.id()) else {
            return false;
        };
        self.game
            .level_destroy_block(level, pos, drop, None, None, false)
            .is_some()
    }

    #[inline]
    fn apply_effect(&mut self, effect: mod_api::InsideEffect) {
        match effect {
            mod_api::InsideEffect::Freeze => {
                if let Some(p) = self.game.common.player.as_mut() {
                    p.in_powder_snow = true;
                    if p.can_freeze() {
                        p.ticks_frozen =
                            (p.ticks_frozen + 1).min(PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
                    }
                }
            }
            mod_api::InsideEffect::ClearFreeze => {
                if let Some(p) = self.game.common.player.as_mut() {
                    p.ticks_frozen = 0;
                }
            }
            mod_api::InsideEffect::FireIgnite => {
                let Some(p) = self.game.common.player.as_mut() else {
                    return;
                };
                p.fire_ignite_contact(&mut self.game.common.world.random);
            }
            mod_api::InsideEffect::LavaIgnite => {
                if let Some(p) = self.game.common.player.as_mut()
                    && !p.mod_fire_immune
                {
                    let _ = p.ignite_for_ticks(300);
                }
            }
            mod_api::InsideEffect::Extinguish => {
                if let Some(p) = self.game.common.player.as_mut() {
                    p.fire_ticks = p.fire_ticks.min(0);
                }
            }
        }
    }

    #[inline]
    fn apply_builtin(&mut self, action: mod_api::__private::BuiltinInsideAction) {
        match action {
            mod_api::__private::BuiltinInsideAction::PowderSnowMelt(pos) => {
                if self
                    .game
                    .common
                    .player
                    .as_ref()
                    .is_some_and(|p| p.is_on_fire() && !p.spectator)
                {
                    self.game
                        .powder_snow_melt(sunrise_core::position::BlockPos::new(
                            pos[0], pos[1], pos[2],
                        ));
                }
            }
            mod_api::__private::BuiltinInsideAction::LayeredCauldronExtinguish { pos, state } => {
                let [x, y, z] = pos;
                let pos = sunrise_core::position::BlockPos::new(x, y, z);
                if self
                    .game
                    .common
                    .player
                    .as_ref()
                    .is_some_and(|p| p.is_on_fire())
                    && self.game.may_interact_at(pos)
                {
                    self.game
                        .cauldron_extinguish_at(pos, sunrise_world::registry::BlockStateId(state));
                }
            }
            mod_api::__private::BuiltinInsideAction::FireHurt(damage) => {
                self.hurt_with_source("minecraft:in_fire", damage, None, None);
            }
            mod_api::__private::BuiltinInsideAction::LavaHurt => {
                if self
                    .game
                    .common
                    .player
                    .as_ref()
                    .is_some_and(|p| !p.mod_fire_immune)
                {
                    self.hurt_with_source("minecraft:lava", 4.0, None, None);
                }
            }
        }
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::native_block_sounds::NativeBlockSoundsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::can_start_swimming::GameHooks as _;
