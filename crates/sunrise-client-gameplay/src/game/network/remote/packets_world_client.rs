use super::*;
use sunrise_core::geometry::border::BorderError;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `C::BlockDestruction`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_block_destruction_packet(&mut self, p: play::BlockDestruction) {
        // ClientPacketListener.handleBlockDestruction: another player's crack.
        self.destroy_block_progress(p.id, p.pos, i32::from(p.progress));
        self.native_client_break_progress(p.id, p.pos, i32::from(p.progress));
    }
    /// `C::BlockEvent`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_block_event_packet(
        &mut self,
        p: play::BlockEvent,
    ) -> Result<()> {
        // Level.blockEvent on the client level: the block's triggerEvent
        // runs on the client's copy of whatever is at pos (a piston's
        // moveBlocks builds the moving block entities it animates, a note
        // block plays); what it doesn't handle reaches the client copy of
        // the block entity (chest lids, shulker boxes).
        let pos = sunrise_core::position::BlockPos::new(p.pos[0], p.pos[1], p.pos[2]);
        let triggered = self.with_level(|b, level, ticks| {
            b.client_block_event(level, ticks, pos, i32::from(p.b0), i32::from(p.b1))
        });
        self.client.moving_pistons_stale = true;
        self.client_process_level_events()?;
        if !triggered {
            let state = self.common.world.block(p.pos[0], p.pos[1], p.pos[2]);
            let block = &self.common.res.facts().get(state).name;
            let named = self.client.remote.as_ref().and_then(|r| {
                r.builtin
                    .get("minecraft:block")?
                    .name(p.block)
                    .map(str::to_owned)
            });
            if named.as_deref() == Some(&**block) {
                self.block_entity_event(p.pos, i32::from(p.b0), i32::from(p.b1));
            }
        }
        Ok(())
    }
    /// `C::ForgetLevelChunk`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_forget_level_chunk_packet(&mut self, p: play::ForgetLevelChunk) {
        // `poll_remote` forgets a run of them at once.
        let [x, z] = p.pos;
        self.forget_remote_packets(&mut [(x, z)].into_iter().collect());
    }
    /// `C::GameEvent`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_game_event_packet(&mut self, p: play::GameEvent) {
        match p.event {
            // NO_RESPAWN_BLOCK_AVAILABLE: ClientPacketListener.handleGameEvent
            // sends the player this system message.
            0 => self.client_system_message(sunrise_game_data::text::Text::translate(
                "block.minecraft.spawn.not_valid",
                &[],
            )),
            // LEVEL_CHUNKS_LOAD_START: the server has begun sending the level.
            13 => {
                if let Some(r) = self.client.remote.as_mut() {
                    r.level_load.loading_packets_received();
                }
            }
            // GUARDIAN_ELDER_EFFECT: the apparition, and the curse sound
            // unless the elder guardian is silent.
            10 => {
                self.client.particles.0.elder_guardian();
                if p.rounded_param() == 1
                    && let Some(at) = self.common.player.as_ref().map(|m| m.position)
                {
                    self.play_client_local_sound(
                        LevelSound::new(
                            "minecraft:entity.elder_guardian.curse",
                            [at.x, at.y, at.z],
                            1.0,
                            1.0,
                        )
                        .in_source(mod_api::SoundSource::Hostile),
                    );
                }
            }
            // WIN_GAME: the poem and credits, then PERFORM_RESPAWN.
            4 if self.client.screen != Screen::Win => {
                self.open_win_screen(true, super::super::win::Finish::RemoteRespawn);
            }
            // DEMO_EVENT: a parentless popup or a client-system hint.
            5 => self.apply_demo_event(p.param),
            // START_RAINING, STOP_RAINING, RAIN_LEVEL_CHANGE and THUNDER_LEVEL_CHANGE.
            event @ (1 | 2 | 7 | 8) => self.common.world.weather.game_event(event, p.param),
            3 => {
                self.set_local_mode(sunrise_gameplay_common::game_modes::mode_from_id(
                    p.rounded_param(),
                ));
                if self.common.spectator() {
                    self.reset_client_item_use();
                }
            }
            // IMMEDIATE_RESPAWN: LocalPlayer.setShowDeathScreen.
            11 => {
                if let Some(r) = self.client.remote.as_mut() {
                    r.show_death_screen = p.param == 0.0;
                }
            }
            // PLAY_ARROW_HIT_SOUND: ClientPacketListener.handleGameEvent
            // plays ARROW_HIT_PLAYER at player eye height.
            6 => {
                if let Some(at) = self.common.eye_position() {
                    self.play_client_local_sound(
                        LevelSound::new("minecraft:entity.arrow.hit_player", at, 0.18, 0.45)
                            .in_source(mod_api::SoundSource::Players),
                    );
                }
            }
            // PUFFER_FISH_STING: ClientPacketListener.handleGameEvent
            // plays PUFFER_FISH_STING at player position.
            9 => {
                if let Some(at) = self.common.player.as_ref().map(|m| m.position) {
                    self.play_client_local_sound(
                        LevelSound::new(
                            "minecraft:entity.puffer_fish.sting",
                            [at.x, at.y, at.z],
                            1.0,
                            1.0,
                        )
                        .in_source(mod_api::SoundSource::Neutral),
                    );
                }
            }
            // LIMITED_CRAFTING: LocalPlayer.setDoLimitedCrafting.
            12 => {
                if let Some(r) = self.client.remote.as_mut() {
                    r.do_limited_crafting = p.param == 1.0;
                }
            }
            _ => {}
        }
    }
    /// `ClientPacketListener.handleInitializeBorder`: install the level's incoming geometry
    /// and warnings, without running server listeners.
    pub(in crate::game) fn apply_initialize_border_packet(
        &mut self,
        p: play::InitializeBorder,
    ) -> std::result::Result<(), BorderError> {
        let mut border = sunrise_level::worldborder::WorldBorder::default();
        // Validate the incoming extent against the incoming center and clip limit,
        // not against any part of the extent this packet replaces.
        border.set_absolute_max_size(p.absolute_max_size)?;
        border.set_center([p.center_x, p.center_z])?;
        if p.lerp_time > 0 {
            border.lerp_between(p.old_size, p.new_size, p.lerp_time)?;
        } else {
            border.set_size(p.new_size)?;
        }
        border.warning_distance = p.warning_blocks;
        border.warning_time = p.warning_time;
        border.damage_per_block = self.common.border.damage_per_block;
        border.safe_zone = self.common.border.safe_zone;
        self.common.border = border;
        Ok(())
    }
    /// `C::SectionBlocksUpdate`: apply verified states in packet order.
    pub(in crate::game) fn apply_section_blocks_update_packet(
        &mut self,
        p: &play::SectionBlocksUpdate,
        mut groups: crate::game::native_connection_groups::GroupUpdates,
    ) {
        for (pos, state) in super::super::remote_chunks::section_blocks(p) {
            self.server_block(pos, state, groups.remove(&pos));
        }
    }
    /// `ClientPacketListener.handleSetSpawn` / `ClientLevel.setRespawnData`: store the
    /// border-adjusted spawn position with the packet's dimension for the compass.
    pub(in crate::game) fn apply_set_default_spawn_position_packet(
        &mut self,
        p: play::SetDefaultSpawnPosition,
    ) {
        let [mut x, mut y, mut z] = p.pos.pos;
        // Level.getWorldBorderAdjustedRespawnData tests the integer BlockPos against
        // the active level's border, even when the packet names a different level.
        if let Some(remote) = &self.client.remote
            && !self
                .common
                .border
                .geometry()
                .is_within_bounds(f64::from(x), f64::from(z), 0.0)
        {
            [x, z] = self.common.border.center().map(sunrise_core::mth::floor);
            // Level.getHeight: the horizontal range is half-open; outside it use
            // this level's sea level, and for missing chunks use its build minimum.
            const WORLD_RANGE: std::ops::Range<i32> = -30_000_000..30_000_000;
            y = if WORLD_RANGE.contains(&x) && WORLD_RANGE.contains(&z) {
                self.common
                    .world
                    .chunks
                    .get(&(x >> 4, z >> 4))
                    .and_then(|chunk| chunk.packet_motion_blocking_height(x, z))
                    .unwrap_or(remote.dimension.min_y)
            } else {
                remote.sea_level.wrapping_add(1)
            };
        }
        self.common.spawn = Some(Vec3::new(
            f64::from(x) + 0.5,
            f64::from(y),
            f64::from(z) + 0.5,
        ));
        self.common.level.world_spawn_dimension =
            (p.pos.dimension != "minecraft:overworld").then_some(p.pos.dimension);
    }
    /// `C::SetTime`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_set_time_packet(&mut self, p: play::SetTime) {
        // ClientPacketListener.handleSetTime / ClientClockManager.handleUpdates:
        // advance all clocks to the packet's game time before applying its
        // (possibly partial) updates. The sky still uses only the default clock.
        self.common.world.game_time = p.game_time;
        self.remote_clock_tick();
        if let Some(r) = self.client.remote.as_mut() {
            for clock in p.clocks {
                if r.registries
                    .name("minecraft:world_clock", clock.clock)
                    .is_some()
                {
                    r.clocks.insert(clock.clock, clock);
                }
            }
            self.common.world.day_time = r.clock_time(r.default_clock);
        }
    }
    /// `C::SoundEntity`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_sound_entity_packet(&mut self, p: play::SoundEntity) {
        if let Some(name) = self.remote_sound_name(&p.sound)
            && !self.pvp_mutes_sound(&name)
        {
            let mut sound = LevelSound::new(&name, [0.0; 3], p.volume, p.pitch);
            sound.emitter = Some(p.id as u32);
            if let Ok(source) = usize::try_from(p.source)
                && let Some(source) = mod_api::SoundSource::ALL.get(source)
            {
                sound.source = *source;
            }
            self.play_client_level_sound(sound, p.seed);
        }
    }
    /// `C::Sound`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_sound_packet(&mut self, p: play::Sound) {
        if let Some(name) = self.remote_sound_name(&p.sound)
            && !self.pvp_mutes_sound(&name)
        {
            let at = [
                f64::from(p.x) / 8.0,
                f64::from(p.y) / 8.0,
                f64::from(p.z) / 8.0,
            ];
            let mut sound = LevelSound::new(&name, at, p.volume, p.pitch);
            if let Ok(source) = usize::try_from(p.source)
                && let Some(source) = mod_api::SoundSource::ALL.get(source)
            {
                sound.source = *source;
            }
            self.play_client_level_sound(sound, p.seed);
        }
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;
