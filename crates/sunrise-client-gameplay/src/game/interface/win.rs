//! `WinScreen` in the game: the poem and credits after leaving the End for
//! the first time (`EndPortalBlock.entityInside` -> `ServerPlayer.showEndCredits`,
//! or a server's `WIN_GAME` event), and the credits from Credits &
//! Attribution.
use super::Screen;
use std::time::Instant;
use sunrise_client::{
    hud::{EndPortalFill, HudBatch},
    win_screen::{Key, WinScreen},
};
use sunrise_game_data::Result;
use winit::keyboard::KeyCode;

/// What closing the screen does (`WinScreen.onFinished`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Finish {
    /// `PERFORM_RESPAWN` sent to the server.
    RemoteRespawn,
    /// Back to Credits & Attribution.
    Options,
    /// Back to the game (`sunrise-ctl credits`).
    Closed,
}
pub(in crate::game) struct Win {
    screen: WinScreen,
    finish: Finish,
    /// The last drawn frame, for the scroll's game-time delta.
    frame: Instant,
}
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn open_win_screen(&mut self, poem: bool, finish: Finish) {
        let screen = WinScreen::new(
            poem,
            &self.common.pack.source,
            &self.common.options.player_name(),
            &self.client.gui.atlas.font,
        );
        self.client.win = Some(Win {
            screen,
            finish,
            frame: Instant::now(),
        });
        self.clear_device_keys();
        self.set_screen(Screen::Win);
        self.capture(false);
    }
    /// Stop the scroll `scroll` pixels in (`sunrise-ctl credits ... SCROLL`).
    pub(in crate::game::client_side) fn hold_win_screen(&mut self, scroll: f32) {
        if let Some(win) = self.client.win.as_mut() {
            win.screen.hold(scroll);
        }
    }
    /// `WinScreen.isPauseScreen`: only the credits without the poem pause.
    pub(in crate::game::client_side) fn win_screen_pauses(&self) -> bool {
        self.client.win.as_ref().is_some_and(|w| !w.screen.poem)
    }
    pub(in crate::game::client_side) fn draw_win_screen(&mut self, batch: &mut HudBatch) {
        let Some(win) = self.client.win.as_mut() else {
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
            return;
        };
        sunrise_client::widgets::begin_screen(|| win.screen.screen_info());
        let now = Instant::now();
        // DeltaTracker.getGameTimeDeltaTicks: the frame's length in ticks.
        let delta_ticks = now.duration_since(win.frame).as_secs_f32()
            / self.common.clock.tick_duration().as_secs_f32();
        win.frame = now;
        let sprite = |n: &str| {
            self.client
                .gui
                .atlas
                .sprites
                .get(n)
                .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0])
        };
        let fill = EndPortalFill {
            sprites: [
                sprite("environment/end_sky"),
                sprite("entity/end_portal/end_portal"),
            ],
            game_time: (self.common.world.game_time % 24000) as f32 / 24000.0,
        };
        win.screen
            .draw_background(batch, &self.client.gui.atlas, fill);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        self.client.win.as_mut().unwrap().screen.draw_contents(
            batch,
            &self.client.gui.atlas,
            delta_ticks,
        );
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
    }
    /// `WinScreen.keyPressed` / `keyReleased`; Escape closes it (`onClose`).
    pub(in crate::game::client_side) fn win_key(
        &mut self,
        code: KeyCode,
        pressed: bool,
    ) -> Result<bool> {
        if code == KeyCode::Escape && pressed {
            self.close_win_screen()?;
            return Ok(true);
        }
        let Some(key) = Key::from_code(code) else {
            return Ok(false);
        };
        if let Some(win) = self.client.win.as_mut() {
            win.screen.key(key, pressed);
            return Ok(true);
        }
        Ok(false)
    }
    /// `WinScreen.tick`: done once everything has rolled past the top.
    pub(in crate::game::client_side) fn tick_win_screen(&mut self) -> Result<()> {
        let height = self.gui_size().1;
        if self
            .client
            .win
            .as_ref()
            .is_some_and(|w| w.screen.finished(height))
        {
            self.close_win_screen()?;
        }
        Ok(())
    }
    /// `WinScreen.respawn` (`onFinished`) and `removed`: the credits music stops.
    pub(in crate::game::client_side) fn close_win_screen(&mut self) -> Result<()> {
        let Some(win) = self.client.win.take() else {
            return Ok(());
        };
        let credits = mod_api::client::music::Music::credits();
        self.client
            .music
            .stop_playing_music(&credits, None, &mut self.client.audio.0);
        match win.finish {
            Finish::Options => self.set_screen(Screen::Options),
            Finish::Closed if self.client.title.0.is_some() => self.set_screen(Screen::Playing),
            Finish::RemoteRespawn | Finish::Closed => {
                if win.finish == Finish::RemoteRespawn {
                    self.remote_respawn();
                }
                self.set_screen(Screen::Playing);
                self.capture(true);
            }
        }
        Ok(())
    }
}
