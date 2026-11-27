use super::*;

/// `computeNextAutosaveInterval`'s arithmetic: `Math.max(100, (int)(rate * 300.0F))`.
pub fn autosave_interval(rate: f32) -> i64 {
    i64::from((rate * 300.0) as i32).max(100)
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn finish_player_autosaves(&mut self, wait: bool) -> Result<bool> {
        let saved = self.server.player_autosaves.0.completed(wait);
        if !saved.is_empty() {
            let directory = sunrise_sim::lan::playerdata::directory(&self.server.world_root);
            for player in saved {
                self.native_player_save_to_file(player.uuid, &player.name, &directory);
            }
        }
        self.server.player_autosaves.0.writes_finished()
    }
    /// `MinecraftServer.computeNextAutosaveInterval`: three hundred seconds
    /// of ticks at the rate the server runs at now - its measured rate while
    /// a `/tick sprint` runs - never fewer than a hundred.
    pub(in crate::game) fn next_autosave_interval(&self) -> i64 {
        let rate = if self.common.clock.sprinting() {
            1.0e9 / (self.common.clock.average_nanos() + 1) as f32
        } else {
            self.common.clock.rate()
        };
        autosave_interval(rate)
    }

    /// `ServerTickRateManager.setTickRate`: set the rate, run the hook and autosave
    /// adjustment, then send the current ticking state to clients.
    pub(in crate::game) fn set_tick_rate(&mut self, rate: f32) -> bool {
        if !self.is_logical_server() || !rate.is_finite() {
            let mod_id = mod_api::content::crash::current_native_mod().unwrap_or("unknown");
            log::warn!(
                "mod {mod_id}: set_tick_rate refused {rate}: requires a finite rate on the logical server"
            );
            return false;
        }
        self.common.clock.set_rate(rate);
        self.on_tick_rate_changed();
        // A listener can retune the clock, so send its state after the notification.
        if let Some(host) = &mut self.server.lan {
            host.hub
                .send_ticking_state(self.common.clock.rate(), self.common.clock.frozen());
        }
        true
    }

    /// `MinecraftServer.onTickRateChanged`: mods hear of it, then a rate that
    /// asks for a shorter wait than the one already running shortens it.
    pub(in crate::game) fn on_tick_rate_changed(&mut self) {
        if self.is_logical_server() {
            self.native_server_tick_rate_changed();
        }
        let interval = self.next_autosave_interval();
        if interval < self.server.ticks_until_autosave {
            self.server.ticks_until_autosave = interval;
        }
    }
    pub fn save_game_metadata(&mut self) -> Result<()> {
        let owner_saved = self.save_placed_players();
        self.save_world_metadata(owner_saved)
    }

    pub(in crate::game) fn save_placed_players(&mut self) -> bool {
        let mut owner_saved = false;
        if self
            .common
            .options
            .execution_role
            .writes_singleplayer_player()
            && !self.server.crash_save.keep_owner_file
        {
            let stored_owner = self.store_owner();

            if stored_owner.is_some() || (self.server.lan.is_none() && self.common.player.is_some())
            {
                if stored_owner.is_none() {
                    self.sync_level();
                }

                let _ = self.finish_player_autosaves(true);
                match self
                    .common
                    .level
                    .write_player(&self.server.world_root, self.common.res.items())
                {
                    Ok(()) => {
                        owner_saved = true;
                        self.save_stats();
                        self.save_advancements();
                        if let Some((uuid, name)) = stored_owner {
                            let directory =
                                sunrise_sim::lan::playerdata::directory(&self.server.world_root);
                            self.native_player_save_to_file(uuid, &name, &directory);
                        }
                    }
                    Err(error) => {
                        log::warn!("Sunrise: the owner's playerdata is not saved: {error}")
                    }
                }
            }
        }
        if self.common.options.execution_role.is_logical_server()
            && let Some(host) = self.server.lan.as_mut()
        {
            let owner = host.owner.as_ref().map(|owner| owner.uuid);

            host.hub.guests.update_all(|g| {
                if g.placed
                    && g.participates_in_player_simulation()
                    && !g.released
                    && Some(g.uuid) != owner
                    && !self.server.crash_save.excludes(g.conn)
                {


                    if std::env::var_os("SUNRISE_DEV_ADVANCEMENT_SELECTOR_AUDIT").is_some() {
                        eprintln!(
                            "advancement selector model raw: {}",
                            serde_json::json!({"conn":g.conn,"name":g.name,
                                "uuid":sunrise_protocol::crypto::uuid_string(g.uuid),
                                "game_time":self.common.world.game_time,
                                "progress":g.advancements.to_json(&self.common.adv.tree, sunrise_world::DATA_VERSION)})
                        );
                    }
                    self.server.player_autosaves.0.enqueue(g, &self.server.world_root, self.common.res.items());
                }
            });
            let host = self
                .server
                .lan
                .as_ref()
                .expect("the player list is still installed");
            self.save_connected_guest_statistics(host);
            for guest in host.hub.guests.iter() {
                if Some(guest.uuid) != owner && host.play.contains_key(&guest.conn) {
                    self.save_guest_advancements(host, guest);
                }
            }
        }

        let _ = self.finish_player_autosaves(true);
        owner_saved
    }

    pub(in crate::game) fn save_world_metadata(&mut self, owner_saved: bool) -> Result<()> {
        self.save_maps();
        self.save_random_sequences();
        self.save_fight();
        self.save_wandering_trader();
        self.save_portal_arrivals();
        self.save_scoreboard();
        self.save_bossbars();
        self.save_schedule();
        self.save_warden_tracker();
        self.sync_world_level_data();
        if self
            .common
            .options
            .execution_role
            .writes_singleplayer_player()
            && !owner_saved
        {
            self.common
                .level
                .write_keeping_player(&self.server.world_root)?;
        } else {
            self.common
                .level
                .write_without_player(&self.server.world_root)?;
        }
        self.save_world_state();
        Ok(())
    }
}
