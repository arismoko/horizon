//! The client ticks of one frame (`Minecraft.runTick`'s tick loop), after the frame has handled
//! what the server sent (`update_world`).
use super::*;

impl crate::game::ClientGame<'_> {
    /// Runs `steps` client ticks: each tells `client_tick_pre`, ticks the game unless it is
    /// paused or on the title screen (the level, its block entities' client tickers included),
    /// does the client's own per-tick work and tells `client_tick_post`. With `event_loop`, an
    /// armed tick script plays its step before the game ticks.
    pub fn client_tick_steps(
        &mut self,
        steps: u32,
        event_loop: Option<&dyn EventLoopCtl>,
    ) -> Result<()> {
        if steps != 0 {
            // Client hooks run in menus too. Their Host can read the world-rest tables even
            // before a level is open, so every entry path (not only the desktop loading
            // window) must publish those tables before the first tick is dispatched.
            self.common.join_world_rest()?;
        }
        for _ in 0..steps {
            let started = Instant::now();
            sunrise_client::social::set_presence_status(self.presence_status());
            sunrise_client::social::tick();
            // Minecraft.tick: every client tick, paused or not, in menus too.
            self.native_client_tick_pre();
            if self.client.title.0.is_none() && !self.common.pause {
                if let Some(event_loop) = event_loop
                    && self.client.tick_script.armed()
                {
                    self.tick_script_step(event_loop);
                }
                sunrise_level::spans::time("tick", || self.tick())?;
                self.tick_friends();
                if self.client.remote.is_some() {
                    // Cave mood (a shader pack's playerMood) samples on a random
                    // seeded from the clock: under `SUNRISE_STILL` it waits for the
                    // world and draws from its tick.
                    if !self.common.clock.still() {
                        self.tick_ambience();
                    } else if self.common.clock.runs_normally() {
                        self.client.ambience.reseed(self.visual_ticks() as i64);
                        self.tick_ambience();
                    }
                    self.tick_fov();
                }
            } else if self.common.pause {
                // Minecraft.tick still picks while the game mode and level are paused.
                self.refresh_client_pick(1.0);
            }
            self.tick_win_screen()?;
            self.native_configs_tick_client();
            // Gui.tick runs ChatListener.tick paused or not; Hud.tickCount advances
            // only when unpaused, including when there is no level on the title screen.
            if !self.common.pause {
                self.client.gui_ticks = self.client.gui_ticks.wrapping_add(1);
            }
            self.tick_chat_queue();
            self.tick_music();
            self.tick_sound_engine();
            // ClientTickEvent.Post, the tick's work done.
            self.native_client_tick_post();
            self.common.clock.record(started.elapsed());
        }
        self.common
            .clock
            .update_animation_phase(self.client.texture_ticks);
        Ok(())
    }
    /// The window's own lead of a tick: debug keys and charts, the tutorial and the creative screen.
    pub(in crate::game) fn tick_client_lead(&mut self) -> Result<()> {
        self.tick_debug_keys()?;
        self.tick_debug_charts();
        self.tick_tutorial();
        if self.client.screen == Screen::Creative {
            self.refresh_creative_tabs();
            // CreativeModeInventoryScreen.containerTick: a player without
            // infinite materials gets the survival inventory instead.
            if !self.common.creative {
                if let Some(c) = self.common.carried.take() {
                    let _ = self.add_stack_to_inventory(c);
                }
                self.set_screen(Screen::Inventory);
            }
        }
        Ok(())
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use winit::{event::WindowEvent, event_loop::ControlFlow};
    use {
        crate::game::native_entity_refs::tests::with_client_game,
        crate::game::startup_menu::StartupMenu,
    };

    struct NoLoop(std::cell::Cell<bool>);
    impl EventLoopCtl for NoLoop {
        fn exit(&self) {
            self.0.set(true);
        }
        fn set_control_flow(&self, _: ControlFlow) {}
    }

    #[cfg(any())]
    #[test]
    #[ignore = "daily"]
    fn tick_body_dispatches_by_role_and_remote() {
        use crate::game::client_side::native_client_session::tests::logged_client;
        use crate::game::native_entity_refs::tests::with_game;
        with_client_game(|game| {
            let time_before = game.world.game_time;
            game.destroy_block_progress(-1, [0, 64, 0], 5);
            game.world.game_time = time_before + 401;
            game.tick().unwrap();
            assert_eq!(game.ticks, 0, "client without remote must not tick a level");
            assert_eq!(game.world.game_time, time_before + 401);
            assert!(
                game.client.destroying.is_empty(),
                "client-only crack expiry still runs without a remote"
            );

            let _server = logged_client(game);
            game.tick().unwrap();
            assert_eq!(game.ticks, 1, "client with remote ticks its remote level");
        });

        with_game(|game| {
            let time_before = game.world.game_time;
            game.tick().unwrap();
            assert_eq!(game.ticks, 1);
            assert_eq!(game.world.game_time, time_before + 1);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn client_redraw_without_title_or_remote_fails_loudly() {
        with_client_game(|game| {
            game.client.title.0 = None;
            assert!(game.client.remote.is_none());
            let loop_ctl = NoLoop(std::cell::Cell::new(false));
            game.handle_window_event_inner(&loop_ctl, WindowEvent::RedrawRequested, None, None);
            assert!(
                game.error.is_some(),
                "client RedrawRequested without title or remote must record an error"
            );
            assert!(
                loop_ctl.0.get(),
                "client RedrawRequested without title or remote must exit the event loop"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn startup_menu_handoff_without_title_opens_title_on_disconnected_client() {
        with_client_game(|game| {
            game.client.title.0 = None;
            assert!(game.client.remote.is_none());
            StartupMenu::default().handoff(game);
            assert!(
                game.client.title.0.is_some(),
                "handoff must never leave a disconnected client without a title screen"
            );
        });
    }
}
