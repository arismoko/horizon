use super::*;

impl crate::game::ClientGame<'_> {
    #[inline(always)]
    #[allow(dead_code)]
    pub(in crate::game) fn spawn_player_client(
        &mut self,
        player: PlayerMotion,
        feet: Vec3,
        entering: bool,
    ) {
        // A saved death already dropped inventory/XP and counted the death.
        if player.health.dead_or_dying() {
            self.set_screen(Screen::Dead);
            self.client.death_focus = None;
            self.client.chat_ui.died_at = Some(Instant::now());
            self.client.chat_ui.death_text = None;
            self.client.chat_ui.death_confirm = None;
            self.capture(false);
        }
        self.common.previous_position = feet;
        if entering {
            // The world's first player: a server's client gets it with its
            // level (Minecraft.setLevel), which stops every sound.
            self.stop_sounds_for_new_level();
        }
        self.common.hook.discard();
        self.common.player_state.attachments.retire();
        self.common.resident_entities.local_player.renew();
        self.common.player = Some(player);
        self.bind_local_player_residency();
        self.input_player_replaced();
        self.client.sway = Default::default();
        self.apply_equipment();
        self.common.eye = 1.62;
        self.client.previous_eye = 1.62;
        self.common.clock.reset();
        self.capture_loaded_world();
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
