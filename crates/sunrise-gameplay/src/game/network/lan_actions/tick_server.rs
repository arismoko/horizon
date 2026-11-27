use super::*;

fn guest_by_hint(
    guests: &sunrise_sim::lan::guest_list::GuestList,
    hint: usize,
    conn: u32,
) -> Option<&sunrise_sim::lan::guest::Guest> {
    guests
        .get(hint)
        .filter(|g| g.conn == conn)
        .or_else(|| guests.get(guests.connection_index(conn)?))
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn advance_guest_entity_clock(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let advanced = host.hub.update_guest(conn, |guest| {
            guest
                .motion
                .as_mut()
                .map(|motion| motion.health.advance_entity_clock())
        });
        if advanced == Some(None) {
            // Joining players have not borrowed their saved body yet.
            self.as_guest(conn, |game| {
                if let Some(player) = game.player.as_mut() {
                    player.health.advance_entity_clock();
                }
            });
        }
    }
    pub(in crate::game::server_side) fn finish_guest_connection_level(&mut self) {
        self.flush_guest_connection_effects();
        self.flush_attachment_sync();
        self.flush_native_sounds();
    }
    /// Whether guest `g`'s body ticks this tick: `ServerPlayer.doTick`'s
    /// `!isSpectator() || !touchingUnloadedChunk()`. A spectator whose box,
    /// inflated by one block, reaches a chunk the level hasn't loaded skips
    /// `super.tick()` (Player.tick: its effects, death ticks, cooldowns, attack
    /// strength, inventory ticks and PlayerTickEvent.Pre/Post) and doTick's food tick
    /// and statistics; everything else the server does for it still runs.
    pub fn guest_body_ticks(&self, g: &sunrise_sim::lan::guest::Guest) -> bool {
        !g.spectator()
            || !touching_unloaded_chunk(g.position, g.dimensions(), |x, z| {
                self.common.world.chunks.contains_key(&(x, z))
            })
    }
    /// `handlePlayerPositionChange` jumps from the pre-move position and yaw.
    /// A later attack packet reads the resulting velocity even before travel.
    pub fn guest_jump(&mut self, conn: u32, from: [f64; 3], yaw: f32) {
        self.as_guest(conn, |game| {
            let Some(player) = game.common.player.as_mut() else {
                return;
            };
            let (position, yaw_after, ground) =
                (player.position, player.yaw, player.ground.on_ground);
            player.position = Vec3::new(from[0], from[1], from[2]);
            player.yaw = yaw;
            player.ground.on_ground = true;
            let side = server_game_side!(game);
            let mut w = MotionWorld {
                side: &side,
                entities: &[],
                player,
                scene: &mut game.common.world,
                assets: &game.common.pack.assets,
                collision_binding: game.common.collision,
                resources: game.common.motion,
                health: &game.common.health_resources,
                tag_items: Some(server_world_items!(game)),
                loop_context: &game.common.loop_context,
                damage_hooks: &game.common.damage_hooks,
                block_registries: Some(&game.common.pack.block_registries),
            };
            let _ = w.jump_from_ground();
            (w.player.position, w.player.yaw, w.player.ground.on_ground) =
                (position, yaw_after, ground);
        });
    }
    /// Only listeners pay for a guest swap; reuse the loop's resolved index.
    pub(in crate::game::server_side) fn guest_player_tick(
        &mut self,
        gi: usize,
        conn: u32,
        hook: mod_api::HookId,
        fire: fn(&mut Self, mod_api::PlayerRef),
    ) {
        if !mod_api::__private::listened(hook) {
            return;
        }
        let (Some(epoch), Some(host)) = (
            self.common.native_lifecycle.epoch(),
            self.server.lan.as_ref(),
        ) else {
            return;
        };
        let guest = &host.hub.guests[gi];
        let player = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, guest.entity_id as u32),
            guest.uuid,
        );
        let _ = self.as_guest_at(gi, conn, |game| fire(game, player));
    }
    /// `LivingEntity.checkAutoSpinAttack` for a server's copy of a Riptide
    /// spinner (it runs before `aiStep`'s countdown clears the damage): the
    /// first mob touching the player's box takes `Player.attack` with the
    /// spin's damage and the spin ends. A `ServerPlayer` is client
    /// authoritative, so its `aiStep` does not travel and both boxes vanilla
    /// passes are the current one: no sweep over the tick's moves.
    pub(in crate::game::server_side) fn server_spin_attack(&mut self) {
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        if p.auto_spin_attack_ticks <= 0 {
            return;
        }
        let swept = p.bounds();
        let riding = self.common.riding;
        let Some(index) = first_spin_target(
            swept,
            riding,
            self.common
                .mobs
                .iter()
                .map(|m| (m.id, m.removed, m.bounds())),
        ) else {
            return;
        };
        self.attack_mob(index);
        if let Some(p) = self.common.player.as_mut() {
            p.auto_spin_attack_ticks = 0;
            p.auto_spin_attack_dmg = 0.0;
            p.velocity = p.velocity.scale(-0.2);
        }
    }
    pub(in crate::game::server_side) fn tick_guest_connection(&mut self, conn: u32) {
        self.tick_guest_connection_body(conn);
        if !self.on_server_trip(conn) {
            // Like the rest of doTick outside Player.tick, LOCATION also runs
            // for unloaded spectators and canceled entity ticks.
            let tick_count = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn).filter(|g| host.hub.here(g)))
                .map(|guest| guest.motion.as_ref().map(|m| m.health.tick_count));
            match tick_count {
                Some(Some(tick_count)) if tick_count % 20 == 0 => {
                    let _ = self.as_guest(conn, |game| game.trigger_location_advancement());
                }
                // Joining players have not borrowed their saved body yet: read
                // its clock through the swap, as `advance_guest_entity_clock` does.
                Some(None) => {
                    let _ = self.as_guest(conn, |game| {
                        if game
                            .player
                            .as_ref()
                            .is_some_and(|player| player.health.tick_count % 20 == 0)
                        {
                            game.trigger_location_advancement();
                        }
                    });
                }
                _ => {}
            }
            self.guest_vehicle_movement_baseline(conn);
        }
        if self
            .server
            .lan
            .as_ref()
            .is_some_and(|host| !host.hub.native_advancement_grants.is_empty())
        {
            self.run_native_advancement_grants();
        }
    }
    pub(in crate::game::server_side) fn tick_guest_connection_body(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        // ServerGamePacketListenerImpl.tick: the chat and command throttlers
        // decay one a tick, dead or alive.
        let play = host.play.entry(conn).or_default();
        play.chat_spam = play.chat_spam.saturating_sub(1);
        play.command_spam = play.command_spam.saturating_sub(1);
        play.decay_recipe_checks();
        // commandSuggestionsProvider.tick: one answer a tick, to the latest request.
        play.suggestion_budget = true;
        self.guest_try_suggestions(conn);
        // ServerPlayer.doTick runs the body (`super.tick()`: Player.tick,
        // where NeoForge posts PlayerTickEvent.Pre/Post) unless the player
        // is a spectator touching an unloaded chunk. Decided once, before
        // the body, as vanilla's `if` is.
        let Some((gi, body)) = self.server.lan.as_ref().and_then(|host| {
            let gi = host.hub.guests.connection_index(conn)?;
            let guest = &host.hub.guests[gi];
            host.hub
                .here(guest)
                .then(|| (gi, self.guest_body_ticks(guest)))
        }) else {
            return;
        };
        if body {
            self.guest_player_tick(gi, conn, mod_api::HookId::PlayerTickPre, |game, player| {
                game.native_player_tick_pre(mod_api::__private::PlayerTickPreCall { player })
            });
        }
        self.guest_sleep_tick(gi, body);
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if host.hub.guests[gi].dead
            || (host.hub.guests[gi].health <= 0.0 && host.hub.guests[gi].death_canceled)
        {
            // LivingEntity.baseTick's tickDeath is part of the body vanilla skips.
            if body {
                self.guest_tick_death(gi);
            }
            if let Some(host) = self.server.lan.as_ref() {
                refresh_building_player(host, conn);
            }
            if body {
                self.guest_player_tick(
                    gi,
                    conn,
                    mod_api::HookId::PlayerTickPost,
                    |game, player| {
                        game.native_player_tick_post(mod_api::__private::PlayerTickPostCall {
                            player,
                        })
                    },
                );
            }
            return;
        }
        if self.on_server_trip(conn) {
            // LivingEntity.hurt kills at once (die), whether the level
            // has loaded around the player or not: its respawn request
            // is taken as soon as it comes.
            self.guest_die(gi, conn);
            if let Some(host) = self.server.lan.as_ref() {
                refresh_building_player(host, conn);
            }
            if body {
                self.guest_player_tick(
                    gi,
                    conn,
                    mod_api::HookId::PlayerTickPost,
                    |game, player| {
                        game.native_player_tick_post(mod_api::__private::PlayerTickPostCall {
                            player,
                        })
                    },
                );
            }
            return;
        }
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let play = host.play.entry(conn).or_default();
        // ServerGamePacketListenerImpl.tick's dropSpamThrottler.
        play.creative_drops = play.creative_drops.saturating_sub(1);
        let using = play.using.is_some();
        let delayed = play.use_delay > 0 || play.attack_ticks < 100;
        if using || delayed {
            let players = if using {
                self.stab_players_for(conn)
            } else {
                Vec::new()
            };
            let _ = self.as_guest_at(gi, conn, |g| {
                let outer = std::mem::replace(&mut g.server.stab_players, players);
                // Player.tick's attackStrengthTicker, part of the body.
                if body {
                    g.server.attack_ticks = g.server.attack_ticks.wrapping_add(1);
                }
                g.tick_using_server();
                g.server.stab_players = outer;
                if g.use_delay > 0 {
                    g.use_delay -= 1;
                }
            });
            self.land_player_stabs();
        }
        if body {
            // Player.tick's ItemCooldowns.tick, once a tick for each player
            // (the shield an axe disabled, a thrown pearl), after
            // LivingEntity.tick's use: an entry a finished use starts has 20
            // ticks left when the next tick's packets are handled, as the
            // guest's own client counts.
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            let play = host.play.entry(conn).or_default();
            play.shield_cooldown = play.shield_cooldown.saturating_sub(1);
            // ServerItemCooldowns.onCooldownEnded: a finished entry is told to
            // the guest (a duration of 0 removes it).
            for group in play.cooldowns.tick() {
                host.hub.send_to(
                    conn,
                    &C::Cooldown(play::Cooldown {
                        group: group.to_string(),
                        duration: 0,
                    }),
                );
            }
            // Player.tick: a different main-hand item resets the attack
            // strength (its modifiers come with detectEquipmentUpdates below).
            let g = &host.hub.guests[gi];
            let held = sunrise_gameplay_common::game::held_items::held_attack_item(
                &g.inventory,
                g.selected,
            );
            let play = host.play.entry(conn).or_default();
            if play.attack_item.as_ref() != held {
                play.attack_ticks = 0;
                play.attack_item = held.cloned();
            }
        }
        // LivingEntity.tick's detectEquipmentUpdates, part of the body: after the
        // item use above and before aiStep's travel and updatePlayerPose, so a new
        // scale modifier rebuilds the box this tick.
        if body {
            self.detect_guest_equipment_updates(gi, conn);
        }
        self.guest_server_tick(conn, body);
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(gi) = host
            .hub
            .guests
            .connection_index(conn)
            .filter(|&i| host.hub.here(&host.hub.guests[i]))
        else {
            return;
        };
        let sleeping = host.hub.guests[gi].sleeping;
        // ServerPlayer.doTick's updateScoreForCriteria: the health, food,
        // armor, experience and level objectives follow the guest.
        let mut recorded = std::mem::take(&mut host.play.entry(conn).or_default().score_recorded);
        let _ = self.as_guest_at(gi, conn, |g| {
            // Exercise a panic in a normal borrowed tick, not in player placement.
            crate::game::dedicated::crash_fixture("guest", || true);
            g.tick_fishing();
            // Player.aiStep's Inventory.tick and doTick's statistics are the body's.
            if body {
                g.tick_maps();
                g.tick_lodestone_compasses();
                g.native_inventory_ticks();
                g.count_tick_stats(sleeping);
            }
            g.record_player_scores(&mut recorded);
        });
        if let Some(host) = self.server.lan.as_mut() {
            host.play.entry(conn).or_default().score_recorded = recorded;
        }
        self.lan_map_updates(conn);
        // A hand-over freezes this player's inventory until its record is released.
        if self
            .server
            .lan
            .as_ref()
            .is_some_and(|host| host.hub.guests[gi].handover.is_none())
        {
            self.guest_pickups(gi);
            self.guest_arrows(gi);
            self.guest_orbs(gi);
        }
        self.guest_die(gi, conn);
        if let Some(host) = self.server.lan.as_ref() {
            refresh_building_player(host, conn);
        }
        if body {
            self.guest_player_tick(gi, conn, mod_api::HookId::PlayerTickPost, |game, player| {
                game.native_player_tick_post(mod_api::__private::PlayerTickPostCall { player })
            });
        }
    }

    /// `LivingEntity.detectEquipmentUpdates` / `collectEquipmentChanges` for a
    /// server guest inside its body tick, after `tick_using` and before
    /// `guest_server_tick`: compares all eight `EquipmentSlot`s by borrow
    /// against `play.last_equipment`, posts `living_equipment_change` per
    /// changed slot before updating attributes (matching NeoForge), updates
    /// `last_equipment` once per changed slot, and refreshes equipment
    /// attribute modifiers when any slot changed.
    fn detect_guest_equipment_updates(&mut self, gi: usize, conn: u32) {
        use crate::game::native_mobs::{
            equipment_stack_matches, guest_equipment_changed, guest_equipment_slot,
            store_equipment_history,
        };

        let Some((entity_id, changed)) = self.server.lan.as_ref().and_then(|host| {
            let g = guest_by_hint(&host.hub.guests, gi, conn).filter(|g| host.hub.here(g))?;
            let last = host
                .play
                .get(&conn)
                .and_then(|play| play.last_equipment.as_deref());
            Some((g.entity_id as u32, guest_equipment_changed(g, last)))
        }) else {
            return;
        };
        if !changed {
            return;
        }
        let mut any_changed = false;
        for slot in sunrise_core::equipment_slot::EquipmentSlot::ALL {
            let living = self
                .living_equipment_notify_epoch()
                .map(|epoch| mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, entity_id)));
            let index = slot as usize;
            let (from, to) = {
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                let Some(g) =
                    guest_by_hint(&host.hub.guests, gi, conn).filter(|g| host.hub.here(g))
                else {
                    break;
                };
                let play = host.play.entry(conn).or_default();
                let current = guest_equipment_slot(g, slot);
                let prev = play
                    .last_equipment
                    .as_deref()
                    .and_then(|last| last[index].as_ref());
                if equipment_stack_matches(current, prev) {
                    continue;
                }
                any_changed = true;
                let to = current.cloned();
                if living.is_none() {
                    store_equipment_history(&mut play.last_equipment, index, to);
                    continue;
                }
                let from = play
                    .last_equipment
                    .as_deref_mut()
                    .and_then(|last| last[index].take());
                (from, to)
            };
            if let Some(living) = living {
                self.dispatch_living_equipment_change(living, slot, from.as_ref(), to.as_ref());
                if let Some(host) = self.server.lan.as_mut()
                    && let Some(play) = host.play.get_mut(&conn)
                {
                    store_equipment_history(&mut play.last_equipment, index, to);
                }
            }
        }
        if any_changed
            && let Some(gi) = self.server.lan.as_ref().and_then(|host| {
                let gi = host.hub.guests.connection_index(conn)?;
                host.hub.here(&host.hub.guests[gi]).then_some(gi)
            })
        {
            let _ = self.as_guest_at(gi, conn, |g| g.put_equipment_modifiers());
        }
    }
    /// Tick each connection once, in hub order. Resolve its level at dispatch
    /// time: a preceding player tick can move a later player to another level.
    /// `across_levels` is false for standalone ticks of the level in place.
    pub fn tick_guest_connections(&mut self, across_levels: bool) -> sunrise_game_data::Result<()> {
        building_cache(self.server.lan.as_ref());
        let result = self.tick_guest_connections_inner(across_levels);
        building_cache(None);
        self.server.pickup_columns = None;
        result
    }
    pub(in crate::game::server_side) fn tick_guest_connections_inner(
        &mut self,
        across_levels: bool,
    ) -> sunrise_game_data::Result<()> {
        let Some(host) = self.server.lan.as_ref() else {
            return Ok(());
        };
        let conns: Vec<_> = host
            .hub
            .guests
            .iter()
            // hasClientLoaded gates incoming actions, not ServerPlayer.doTick.
            .filter(|g| {
                if across_levels {
                    g.placed && g.participates_in_player_simulation()
                } else {
                    host.hub.here(g)
                }
            })
            .map(|g| g.conn)
            .collect();
        let mut active_level = None;
        let mut collision = None;
        for conn in conns {
            let Some(dim) = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .map(|g| Dim::from_id(&g.dimension))
            else {
                continue;
            };
            if !across_levels && dim != self.common.dim {
                continue;
            }
            if active_level != Some(dim) {
                if across_levels {
                    if active_level.is_some() {
                        self.finish_guest_connection_level();
                        drop(collision.take());
                    }
                    if self.common.dim != dim {
                        self.enter_level(dim)?;
                    }
                    collision = Some((
                        self.placed_moving_colliders(),
                        self.common.collision_parts(),
                    ));
                }
                active_level = Some(dim);
                // Most players aren't in any of the columns items reach.
                self.server.pickup_columns = Some(Default::default());
                if self
                    .server
                    .lan
                    .as_ref()
                    .is_some_and(|h| h.play.values().any(|p| p.hook.is_some()))
                {
                    self.refresh_hook_targets();
                }
            }
            if let Some((colliders, _parts)) = &collision {
                sunrise_world::moving_colliders::in_level(colliders.clone(), || {
                    sunrise_sim::block_contact::in_level(self.common.tick, Some(dim.id()), || {
                        self.tick_guest_connection(conn);
                    });
                });
                self.drain_entity_inside();
                if let Some(host) = self.server.lan.as_mut() {
                    host.hub.sync_guest_after_tick(conn);
                }
            } else {
                self.tick_guest_connection(conn);
            }
        }
        if across_levels && active_level.is_some() {
            self.finish_guest_connection_level();
        }
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn tick_guests_inner(&mut self) {
        self.tick_level_guests_inner();
        self.tick_guest_connections_inner(false).unwrap();
        self.server.pickup_columns = None;
    }
    /// The level path: `ServerLevel.tick` calls `ServerPlayer.tick` before the
    /// server starts its connection ticks.
    pub fn tick_level_guests(&mut self) {
        building_cache(self.server.lan.as_ref());
        self.tick_level_guests_inner();
        building_cache(None);
    }
    pub(in crate::game) fn tick_level_guests_inner(&mut self) {
        // `EntityEvent.Size` for the poses the guests' moves set since the last tick.
        self.native_guest_sizes();
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        host.hub.sync_initial_health_data();
        Self::sync_hub_mod_flags(&mut host.hub);
        let turns: Vec<_> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g))
            .map(|g| (g.conn, g.vehicle.is_some()))
            .collect();
        // A passenger's Entity.commonTick precedes its rideTick and cannot
        // be canceled by the ride tick's EntityTickEvent.Pre.
        for &(conn, passenger) in &turns {
            if self.on_server_trip(conn) {
                continue;
            }
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            host.play.entry(conn).or_default().level_up_sound.tick();
            if passenger {
                self.advance_guest_entity_clock(conn);
            }
        }
        self.apply_guest_camera_requests();
        self.common.creatures.canceled_players.clear();
        let mut round = self.native_tick_round();
        // For a root player the common tick is part of the cancellable round,
        // even for a spectator touching an unloaded chunk. Each guest's Pre,
        // ServerPlayer.tick work, and Post bracket that guest individually.
        for &(conn, passenger) in &turns {
            let Some(entity_id) = self.server.lan.as_ref().and_then(|host| {
                let g = host.hub.guest(conn).filter(|g| host.hub.here(g))?;
                Some(g.entity_id as u32)
            }) else {
                continue;
            };
            if round.pre()
                && !self.native_tick_pre_one(
                    &mut round,
                    entity_id,
                    sunrise_gameplay_common::game::native_ticks::PLAYER,
                )
            {
                self.common.creatures.canceled_players.push(conn);
                continue;
            }
            if self
                .server
                .lan
                .as_ref()
                .is_none_or(|host| host.hub.guest(conn).is_none_or(|g| !host.hub.here(g)))
            {
                continue;
            }
            if !passenger && !self.on_server_trip(conn) {
                self.advance_guest_entity_clock(conn);
            }
            self.tick_guest_camera_one(conn);
            self.lan_merchant_tick_one(conn);
            self.lan_station_tick_one(conn);
            self.lan_table_tick_one(conn);
            self.lan_mod_crafting_tick_one(conn);
            self.lan_menu_links_tick_one(conn);
            self.lan_container_tick_one(conn);
            self.tick_lan_mount_menu_one(conn);
            self.announce_guest_advancements();
            // A player waiting for a teleport destination cannot dig there yet.
            if !self.on_server_trip(conn)
                && let Some(host) = self.server.lan.as_mut()
                && let Some(gi) = host
                    .hub
                    .guests
                    .connection_index(conn)
                    .filter(|&i| host.hub.here(&host.hub.guests[i]))
            {
                if let Some((pos, _)) = host.play.get(&conn).and_then(|play| play.digging)
                    && self
                        .common
                        .res
                        .facts()
                        .is_air(self.common.world.block(pos[0], pos[1], pos[2]))
                    && let Some(play) = host.play.get_mut(&conn)
                {
                    play.digging = None;
                    play.mining = None;
                }
                let play = host.play.entry(conn).or_default();
                let digging = play.digging;
                if digging.is_some() || play.mining.is_some() || play.destroy_delay > 0 {
                    let _ = self.as_guest_at(gi, conn, |g| {
                        if let Some((pos, face)) = digging {
                            g.server.forced_hit = Some(Hit {
                                pos,
                                face,
                                point: [
                                    f64::from(pos[0]) + 0.5,
                                    f64::from(pos[1]) + 0.5,
                                    f64::from(pos[2]) + 0.5,
                                ],
                                inside: false,
                            });
                        }
                        let _ = g.tick_mining();
                        g.server.forced_hit = None;
                    });
                    // Instant STARTs complete without a STOP; do not break a later
                    // block planted in the same position again.
                    if digging.is_some()
                        && let Some(host) = self.server.lan.as_mut()
                        && let Some(play) = host.play.get_mut(&conn)
                        && (host.hub.guest(conn).is_some_and(|g| g.creative())
                            || play.mining.is_none())
                    {
                        play.digging = None;
                    }
                }
                self.guest_delayed_destroy(conn);
                self.guest_dig_feedback(conn);
                let _ = self.as_guest(conn, |g| {
                    if let Some(player) = g.common.player.as_mut() {
                        player.health.advance_damage_cooldown();
                    }
                    // ServerPlayer.tick: the TICK and LEVITATION triggers,
                    // player attributes, then PlayerAdvancements.flushDirty.
                    g.tick_level_advancements();
                    g.update_player_attributes();
                });
                if let Some(host) = self.server.lan.as_mut() {
                    host.hub.flush_progress(conn);
                    if !host.hub.native_advancement_grants.is_empty() {
                        self.run_native_advancement_grants();
                    }
                }
                if let Some(host) = self.server.lan.as_mut()
                    && let Some(gi) = host.hub.guests.connection_index(conn)
                    && host.hub.here(&host.hub.guests[gi])
                    && host.hub.guests[gi].post_effects_dirty
                {
                    host.hub.send_post_effects(gi);
                }
            }
            if round.post() {
                self.native_tick_post_one(
                    &round,
                    entity_id,
                    sunrise_gameplay_common::game::native_ticks::PLAYER,
                );
            }
        }
        self.announce_guest_advancements();
        // Entity.rideTick positions the rider after its tick (and after Post).
        // Canceling the tick does not stop this.
        for (conn, passenger) in turns {
            if passenger && !self.on_server_trip(conn) {
                self.guest_vehicle_tick(conn);
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[path = "tests/building_cache_server.rs"]
pub mod building_cache_tests;

use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;
