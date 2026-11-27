use super::*;

impl crate::game::ServerGame<'_> {
    /// `InstrumentItem.play`'s `level.playSound(player, ...)`: the others hear
    /// a guest's horn now, the guest's own client played it as it used it.
    /// The horn is bound to the guest (`playSound(player, player, ...)`).
    pub(in crate::game::server_side) fn guest_instrument_sound(
        &mut self,
        conn: u32,
        heard: usize,
        horn: Option<String>,
    ) {
        let Some(player) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .and_then(|g| u32::try_from(g.entity_id).ok())
        else {
            return;
        };
        let Some(horn) = horn else {
            return;
        };
        if !self.server.audio_capture_active {
            return;
        }
        let capture = &mut self.server.pending_sounds;
        let mut sounds = Vec::new();
        let mut i = heard.min(capture.len());
        while i < capture.len() {
            if capture[i].event == horn {
                sounds.push(capture.remove(i));
            } else {
                i += 1;
            }
        }
        for mut captured in sounds {
            captured.entity = Some(player);
            self.send_captured_level_sound(captured, Some(conn));
        }
    }
    /// `level.playSound(player, ...)` of a guest's use it predicted (`remote::hand_operated`
    /// blocks only, as the guest decides): the others hear it now, the guest already did.
    pub(in crate::game::server_side) fn guest_use_sounds(
        &mut self,
        conn: u32,
        logged: usize,
        heard: usize,
    ) {
        let Some(placer) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return;
        };
        let placer = placer.entity_id as u32;
        let changed = self
            .common
            .world
            .block_log
            .as_ref()
            .and_then(|log| log.get(logged..))
            .unwrap_or_default();
        // BlockItem.place marked the selected event for everyone except the predicting player.
        // Forward that event instead of asking the block part again after the placement.
        let mut sounds = Vec::new();
        if self.server.audio_capture_active {
            let capture = &mut self.server.pending_sounds;
            let mut i = heard.min(capture.len());
            while i < capture.len() {
                if capture[i].except_entity == Some(placer) {
                    sounds.push(capture.remove(i));
                } else {
                    i += 1;
                }
            }
        }
        let behaviours = self.common.res.behaviours();
        let hand_operated = !changed.is_empty()
            && changed.iter().all(|&(x, y, z)| {
                sunrise_gameplay_common::game::remote::hand_operated(
                    behaviours,
                    self.common.world.block(x, y, z),
                )
            });
        if hand_operated && self.server.audio_capture_active {
            let capture = &mut self.server.pending_sounds;
            let start = heard.min(capture.len());
            sounds.extend(capture.drain(start..));
        }
        for sound in sounds {
            self.send_captured_level_sound(sound, Some(conn));
        }
    }
    /// Positional sounds and level events of the tick, for the guests.
    pub fn lan_broadcast_effects(&mut self) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        host.broken.clear();
        let sounds = std::mem::take(&mut self.server.pending_sounds);
        for sound in sounds {
            self.send_captured_level_sound(sound, None);
        }
        self.send_bound_sounds();
        if let Some(host) = self.server.lan.as_ref() {
            host.hub.finish_captured_sounds();
        }
        // A stop whose removal waited for these sounds follows that removal (#4522).
        self.run_tracking_after_sounds();
    }
    #[allow(dead_code)]
    #[inline(always)]
    pub(in crate::game) fn lan_broken_block_positions(&self) -> Vec<[f64; 3]> {
        let Some(host) = self.server.lan.as_ref() else {
            return Vec::new();
        };
        host.broken
            .iter()
            .copied()
            .chain(
                self.server
                    .lan_events
                    .iter()
                    .filter(|event| event.kind == 2001)
                    .map(|event| event.pos.map(f64::from)),
            )
            .collect()
    }
    /// The guests' items in use and Riptide spins, for the players around
    /// (`LivingEntity.startUsingItem`, `stopUsingItem` and `startAutoSpinAttack`
    /// set DATA_LIVING_ENTITY_FLAGS).
    pub fn lan_living_flags(&mut self) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        host.hub.guests.update_all(|g| {
            let using = host.play.get(&g.conn).and_then(GuestPlay::using);
            let spinning = g
                .motion
                .as_ref()
                .is_some_and(|m| m.auto_spin_attack_ticks > 0);
            g.living_flags = crate::game::lan::living_flags(using, spinning);
        });
    }
    /// A level sound's old emitter must still be known at playback. Keep
    /// sounds needing a new pairing queued, with their later removals held.
    pub(in crate::game) fn lan_prepare_captured_sounds(
        &mut self,
        entities: &[sunrise_sim::lan::entity::EntityState],
    ) {
        if !self.server.audio_capture_active {
            return;
        }
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        host.hub
            .prepare_captured_sounds(&mut self.server.pending_sounds, entities);
    }
    /// `LivingEntity.tick`'s arrows and bee stingers falling out of the
    /// players' bodies, and their counts for the players around
    /// (`DATA_ARROW_COUNT_ID`, `DATA_STINGER_COUNT_ID`).
    pub(in crate::game::server_side) fn lan_stuck(&mut self, paused: bool) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if !paused {
            self.server.combat.0.stuck.tick();
        }
        host.hub.guests.update_all(|g| {
            g.shoulders = crate::game::shoulders::shoulder_variants(g.saved.as_ref());
            g.stuck = match self.server.guest_combat.0.get_mut(&g.conn) {
                Some(combat) => {
                    if !paused {
                        combat.stuck.tick();
                    }
                    (combat.stuck.arrows, combat.stuck.stingers)
                }
                None => (0, 0),
            };
        });
    }
    /// A use the server ran swung the guest's hand: their own client is told
    /// too (`swing(hand, true)` for a SwingSource.SERVER result). Sunrise's
    /// client does not run uses itself, so it does not predict the swing; a
    /// client that did (a block placed) is still in the first half of its
    /// swing and ignores it (`SwingState.startIfAble`).
    pub(in crate::game::server_side) fn lan_swing_back(&mut self, conn: u32, swung: Option<bool>) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        if swung != Some(true) {
            return;
        }
        let (Some(g), Some(play)) = (host.hub.guest(conn), host.play.get(&conn)) else {
            return;
        };
        if let Some(c) = play.swing.current {
            host.hub
                .send_to(conn, &swing_packet(g.entity_id, c.hand, c.animation));
        }
    }
    /// The tick's arm swings for the players around (each guest's to everyone
    /// else), and `LivingEntity.updateSwingTime` for the guests' bodies here.
    pub fn lan_swings(&mut self) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let conns: Vec<u32> = host.play.keys().copied().collect();
        for conn in conns {
            let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
                continue;
            };
            let (id, at, here) = (g.entity_id, g.position, host.hub.here(g));
            let play = host.play.get_mut(&conn).expect("play");
            let swung = std::mem::take(&mut play.swings) > 0;
            if let (true, true, Some(c)) = (swung, here, play.swing.current) {
                let packet = swing_packet(id, c.hand, c.animation);
                host.hub.broadcast_near(at, 128.0, &packet, Some(conn));
            }
            play.swing.tick();
        }
    }
    pub(in crate::game::server_side) fn send_captured_level_sound(
        &mut self,
        mut sound: sunrise_game_data::sound_source::ServerSound,
        except: Option<u32>,
    ) {
        let source = mod_api::SoundSource::ALL
            .get(sound.source)
            .copied()
            .unwrap_or(mod_api::SoundSource::Master);
        let level_sound = LevelSound {
            name: &sound.event,
            at: sound.position,
            volume: sound.volume,
            pitch: sound.pitch,
            source,
            emitter: sound.entity,
        };
        let Some(heard) = self.ask_level_sound(self.common.dim.id(), level_sound) else {
            return;
        };
        let name = heard.name.into_owned();
        sound.source = heard.source as usize;
        sound.volume = heard.volume;
        sound.pitch = heard.pitch;
        sound.event = name;
        if let Some(host) = self.server.lan.as_ref() {
            host.hub.captured_sound(&sound, except);
        }
    }
}
