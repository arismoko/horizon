use super::*;
use crate::game::ServerGame;

impl ServerGame<'_> {
    /// `ServerLevel.levelEvent` or `globalLevelEvent` (`global`) is about to send event `id`.
    pub(in crate::game) fn native_server_level_event(
        &mut self,
        dimension: &str,
        id: i32,
        pos: [i32; 3],
        data: i32,
        global: bool,
        canceled: &mut bool,
    ) {
        self.fire_server_level_event(dimension, id, pos, data, global, canceled);
    }

    /// Whether a mod's `server_level_event` cancels this event. False, asking nobody, off the.
    pub(in crate::game) fn server_level_event_canceled(
        &mut self,
        id: i32,
        pos: [i32; 3],
        data: i32,
        global: bool,
    ) -> bool {
        if !self.is_logical_server()
            || !mod_api::__private::listened(mod_api::HookId::ServerLevelEvent)
        {
            return false;
        }
        let mut canceled = false;
        let dimension = self.dim.id();
        self.native_server_level_event(dimension, id, pos, data, global, &mut canceled);
        canceled
    }
}

thread_local! {
    static LEVEL_EVENTS_ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks level events whose `server_level_event` was already asked (one delivered at once),.
pub(in crate::game) struct LevelEventsAsked(bool);

impl LevelEventsAsked {
    pub(in crate::game) fn mark() -> Self {
        Self(LEVEL_EVENTS_ASKED.with(|asked| asked.replace(true)))
    }
    pub(in crate::game) fn marked() -> bool {
        LEVEL_EVENTS_ASKED.with(std::cell::Cell::get)
    }
}

impl Drop for LevelEventsAsked {
    fn drop(&mut self) {
        LEVEL_EVENTS_ASKED.with(|asked| asked.set(self.0));
    }
}
