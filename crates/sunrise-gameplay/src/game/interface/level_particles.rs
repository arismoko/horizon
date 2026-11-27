//! Level events whose client handler (`LevelEventHandler`) spawns particles
//! on a block: bone meal's growth (1505), dragon egg teleportation (2015), a
//! consumable's teleport (2017), an enderman's (2018), electric sparks (3002),
//! waxing (3003), wax off (3004) and scraping (3005), plus the smoke, drips,
//! egg and dragon fireball particles of 1501, 1502, 1504, 2006, 2009, 2012
//! and 3009.
use crate::game::ServerGame;
use sunrise_core::position::BlockPos;
use sunrise_world::level::LevelEvent;

impl ServerGame<'_> {
    /// `ServerLevel.globalLevelEvent`: heard by every player on the server
    /// (an end portal opening, the dragon's death roar): here, and sent to
    /// every guest whatever their dimension, each at their own position
    /// (`Hub::global_level_event`).
    pub(in crate::game) fn global_level_event(&mut self, id: i32, pos: [i32; 3], data: i32) {
        let global = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::GLOBAL_SOUND_EVENTS);
        if global && self.server_level_event_canceled(id, pos, data, true) {
            return;
        }
        // ServerLevel.globalLevelEvent: without global_sound_events it is a
        // plain level event (`levelEvent(null, ...)`), sent to the players
        // near `pos`; the vanilla client has no plain sound for 1023, 1028
        // and 1038 (they are only in `LevelEventHandler.globalLevelEvent`).
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::GLOBAL_SOUND_EVENTS)
        {
            self.server_level_event_at(id, pos, data);
            return;
        }
        if self.server.audio_capture_active {
            self.server
                .lan_global_events
                .push((self.common.dim.id(), id, pos, data));
        }
    }
    /// Deliver one committed global level event immediately without touching unrelated
    /// `lan_global_events` or `lan_events`, preserving packet order across effect kinds.
    pub(in crate::game::server_side) fn deliver_global_level_event(
        &mut self,
        id: i32,
        pos: [i32; 3],
        data: i32,
    ) {
        let global = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::GLOBAL_SOUND_EVENTS);
        if global && self.server_level_event_canceled(id, pos, data, true) {
            return;
        }
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::GLOBAL_SOUND_EVENTS)
        {
            self.deliver_level_event(id, pos, data);
            return;
        }
        if let Some(host) = self.server.lan.as_ref() {
            host.hub
                .global_level_event(self.common.dim.id(), id, pos, data);
        } else if self.server.audio_capture_active {
            self.server
                .lan_global_events
                .push((self.common.dim.id(), id, pos, data));
        }
    }
    /// Queue and play level event `id` at `pos` (`Level.levelEvent`).
    pub fn server_level_event_at(&mut self, id: i32, pos: [i32; 3], data: i32) {
        self.level_event_from(None, id, pos, data);
    }

    #[cfg(test)]
    pub(in crate::game) fn level_event_at(&mut self, id: i32, pos: [i32; 3], data: i32) {
        self.server_level_event_at(id, pos, data);
    }

    /// Deliver one committed level event immediately without draining unrelated `level_events` or
    /// `lan_events`, sending its packet directly when a LAN host is active so packet order matches
    /// commit order across effect kinds.
    pub(in crate::game::server_side) fn deliver_level_event(
        &mut self,
        id: i32,
        pos: [i32; 3],
        data: i32,
    ) {
        if self.server_level_event_canceled(id, pos, data, false) {
            return;
        }
        let saved_level_events = std::mem::take(&mut self.common.level_events);
        let saved_lan_events = std::mem::take(&mut self.server.lan_events);
        self.common.level_events.push(LevelEvent::LevelEvent {
            id,
            pos: BlockPos::new(pos[0], pos[1], pos[2]),
            data,
            source: None,
        });
        let asked = crate::game::native_hooks::server_level_event::LevelEventsAsked::mark();
        let _ = self.server_process_level_events();
        drop(asked);
        let recorded = std::mem::replace(&mut self.server.lan_events, saved_lan_events);
        let extra_level_events =
            std::mem::replace(&mut self.common.level_events, saved_level_events);
        self.common.level_events.extend(extra_level_events);
        if let Some(host) = self.server.lan.as_mut() {
            if id == 2001 {
                host.broken
                    .push([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])]);
            }
            host.hub.level_event(id, pos, data, None);
        } else {
            self.server.lan_events.extend(recorded);
        }
    }

    pub(in crate::game::server_side) fn level_event_from(
        &mut self,
        source: Option<sunrise_world::level::LevelEventPlayer>,
        id: i32,
        pos: [i32; 3],
        data: i32,
    ) {
        self.common.level_events.push(LevelEvent::LevelEvent {
            id,
            pos: BlockPos::new(pos[0], pos[1], pos[2]),
            data,
            source,
        });
        let _ = self.server_process_level_events();
    }
}
