use crate::game::ClientGame;

impl ClientGame<'_> {
    /// The window's frame step: changed config files reload once a second.
    pub fn native_configs_tick_client(&mut self) {
        self.native_configs_poll();
    }
}

use sunrise_gameplay_common::game::native_configs::NativeConfigsGameExt1 as _;
