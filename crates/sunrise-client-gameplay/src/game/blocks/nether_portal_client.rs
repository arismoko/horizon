use super::Screen;
use crate::game::client_side::screens::ScreenClientExt as _;
use sunrise_core::random::RandomSource;
use sunrise_sim::nether_portal::PLAYER_DELAY;

#[path = "nether_portal/respawn_attributes_client.rs"]
pub(in crate::game) mod respawn_attributes;
#[path = "nether_portal/respawn_attributes.rs"]
pub(in crate::game) mod respawn_attributes_state;

impl crate::game::ClientGame<'_> {
    /// Whether the inventory key opens nothing while standing in a nether portal.
    pub(in crate::game) fn in_portal_transition(&self) -> bool {
        self.client.client_was_inside
    }

    /// Client-side `setAsInsidePortal`, driving `client_portal_effect`.
    pub(in crate::game) fn client_inside_nether_portal(&mut self) {
        if !self.common.player_can_use_portal() {
            return;
        }
        if self.common.portal_player.cooldown > 0 {
            self.common.portal_player.cooldown = PLAYER_DELAY;
        } else {
            self.client.client_inside = true;
        }
    }

    /// `LocalPlayer.handlePortalTransitionEffect`: whether the confusion
    /// effect grows this tick. Entering a portal closes the screen only when
    /// it does not allow it (`Screen.isAllowedInPortal`), and plays
    /// `block.portal.trigger` when the effect starts from nothing.
    pub fn client_portal_effect(&mut self) -> bool {
        let inside = std::mem::take(&mut self.client.client_inside);
        self.client.client_was_inside = inside;
        if !inside {
            return false;
        }
        // No screen at all (`screen() == null`) is nothing to close.
        if self.client.screen != Screen::Playing && !self.client.screen.allowed_in_portal() {
            if matches!(self.client.screen, Screen::Creative | Screen::Inventory) {
                self.close_container_client();
                self.common.carried = None;
            }
            self.set_screen(Screen::Playing);
            self.capture(true);
        }
        if self.client.screen_fx.portal == 0.0 {
            let pitch = self.common.world.random.next_float() * 0.4 + 0.8;
            self.play_audio("minecraft:block.portal.trigger", None, 0.25, pitch);
        }
        true
    }
}
