use super::*;
#[cfg(feature = "client")]
use crate::game::client_side::render_tick::tick_client_weather_effects;
pub(in crate::game::client_side) use sunrise_gameplay_common::game::remote::client_level_tick::advance_clock;

/// Per-level client work bracketed by the shared LevelTickEvent hooks.
/// No level means no event; failed ticks never emit the post event.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub fn tick_remote_level(&mut self) -> Result<()> {
        self.tick_remote_level_with(Self::tick_remote_level_body)?;
        if self.common.clock.runs_normally()
            && self.client.remote.as_ref().is_some_and(Remote::has_level)
        {
            self.tick_particle_sources();
            self.tick_particle_engine();
        }
        Ok(())
    }
    pub(in crate::game::client_side) fn tick_remote_level_body(&mut self) -> Result<()> {
        // MultiPlayerGameMode.tick, first in Minecraft.tick, before the GUI and its screens tick:
        // the slot chosen since the last tick (the wheel, the last tick's number keys, the
        // server's SetHeldSlot), then its Connection.tick's ClientPacketListener.tick (the key
        // refresh and setKeyPair: a new chat session), ahead of a closing screen's packet, this
        // tick's actions and its move.
        self.remote_send_carried();
        self.tick_chat_session();
        self.tick_debug_connection();
        self.fire_pending_block_entity_loads();
        self.common.ticks += 1;
        self.repro_tick();
        // Hud.tick ends with ChatComponent.tick.
        let chat_ticks = self.visual_ticks();
        self.client
            .chat_ui
            .chat
            .tick(&self.client.gui.atlas, chat_ticks);
        self.tick_sign_editor();
        self.track_experience_display();
        self.tick_enchanting_book();
        self.tick_inventory_screen();
        // Leaving a slot clears its bundle's selection here and on the server.
        self.tick_bundle_hover();
        // Options.broadcastOptions: a changed render distance or Chat option
        // is sent again.
        let info = self
            .client
            .settings
            .client_information(client_view_distance(self.client.settings.render_distance));
        if let Some(r) = self.client.remote.as_mut()
            && r.joined
            && r.sent_info != info
        {
            r.sent_info = info.clone();
            self.send(S::ClientInformation(info));
        }
        // Minecraft.tick: TickRateManager.tick; ClientLevel.tickTime advances
        // the time only while the level runs normally.
        let running = self.common.clock.world_runs();
        self.client.boss_overlay.tick_darken(running);
        // Under `SUNRISE_STILL` the frame's countdowns (this one, the hotbar's pop, the HUD's
        // below) wait for the world, as the border's size change does in any frozen level: a
        // frozen client keeps ticking, as many ticks as the run took.
        let hold = !running && self.common.clock.still();
        if !hold {
            self.tick_item_activation();
        }
        // Weather levels drift toward their targets between server updates.
        let w = &mut self.common.world.weather;
        w.rain_level.0 = w.rain_level.1;
        w.thunder_level.0 = w.thunder_level.1;
        if self.common.use_delay > 0 {
            self.common.use_delay -= 1;
        }
        // Minecraft.handleKeybinds precedes entity ticking and sendChanges.
        // Window callbacks only queue clicks, so an interaction cannot leak
        // between CLIENT_TICK_END and a later pong.
        self.remote_handle_input();
        self.tick_remote_player(running, hold)?;
        // ClientLevel.tick advances time and sweeps only after input, entity ticks and player sync.
        if running && self.client.remote.as_ref().is_some_and(Remote::has_level) {
            self.common.world.game_time += 1;
            self.remote_clock_tick();
            tick_client_weather_effects(self);
            self.expire_client_cracks();
        }
        self.tick_end_flash();
        Ok(())
    }
    pub fn tick_remote_level_with(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {
        let level = (mod_api::__private::listened(mod_api::HookId::LevelTickPre)
            || mod_api::__private::listened(mod_api::HookId::LevelTickPost))
        .then(|| self.native_client_level())
        .flatten();
        if let Some(level) = level {
            self.native_level_tick_pre(level);
        }
        let result = body(self);
        // Each entity's own move this tick (`EntityEvent.EnteringSection`).
        self.client_entity_sections_stepped();
        // `ClientLevel.tick`'s RETURN, which a failed tick does not reach.
        if result.is_ok()
            && let Some(level) = level
        {
            self.native_level_tick_post(level);
        }
        result
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::block_entity_on_load::BlockEntityOnLoadGameExt1 as _;
