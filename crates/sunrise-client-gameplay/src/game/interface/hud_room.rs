//! The HUD's `Room` this frame (`docs/ui-kit.md`): what vanilla's HUD shows that Sunrise's own
//! overlays keep clear of. `hud` empties it each frame and `hud_layers` fills it in before the
//! world's HUD draws, into `ui.room`; the controller notice and radial menu read it. With it, the frame's `motion::Clock` (`hud_clock`), and whether an
//! overlay holds the view (`overlay_holds_view`), which what vanilla draws there gives way to.
use crate::game::ClientGame;
use sunrise_client::hud::HudVertex;
use sunrise_client::hud_kit::motion::Clock;
use sunrise_client::hud_kit::room::{Keep, Room};
use sunrise_client::{boss_overlay, hud_layout, hud_overlays};

impl ClientGame<'_> {
    /// The HUD's clock this frame (`hud_kit::motion`): still, so overlays show their motion
    /// settled, while the game is paused, frame-stepped (`SUNRISE_STILL`, a screenshot
    /// sequence) or a repro recording or replay runs.
    pub(in crate::game::client_side) fn hud_clock(&self) -> Clock {
        let still = self.common.pause
            || self.common.clock.still()
            || self.client.screenshot_sequence.0.is_some()
            || self.client.repro.0.recording_active()
            || self.client.repro.0.replaying();
        Clock::new(std::time::Instant::now(), still)
    }

    /// What the HUD shows on a `w` x `h` GUI px view this frame, as keep-outs: the boss bars
    /// while the HUD shows (`show_hud`: not F1, not Replay Studio), as wide as the bar or its
    /// widest name.
    pub(in crate::game::client_side) fn hud_room(&self, w: f32, h: f32, show_hud: bool) -> Room {
        let mut room = Room::new(w, h);
        if !show_hud {
            return room;
        }
        let (wi, hi) = (w as i32, h as i32);
        let bars = self.client.boss_overlay.bars(boss_overlay::now_millis());
        let drawn = hud_overlays::boss_bars_drawn(hi, bars.len());
        if drawn > 0 {
            let font = &self.client.gui.atlas.font;
            let half = bars[..drawn]
                .iter()
                .map(|b| (hud_layout::width(font, &b.name.resolve_legacy()) + 1) / 2)
                .fold(hud_overlays::BOSS_BAR_WIDTH / 2, i32::max);
            let bottom = hud_overlays::boss_bars_bottom(hi, bars.len());
            room.add(
                Keep::BossBars,
                [
                    (wi / 2 - half) as f32,
                    0.0,
                    (2 * half) as f32,
                    bottom as f32,
                ],
            );
        }
        room
    }

    /// Reserve the highest painted status row, vanilla or replaced, down to the bottom
    /// of the view. Measure only the installed body's paint, not the surrounding hook draws.
    pub fn keep_status_rows(&mut self, vertices: &[HudVertex], w: f32, h: f32) {
        let top = vertices.iter().map(|v| v.pos[1]).fold(h, f32::min);
        if top < self.client.chat_ui.room.bottom() {
            let half = hud_overlays::HOTBAR_HALF_WIDTH;
            self.client.chat_ui.room.add(
                Keep::StatusRows,
                [
                    ((w as i32) / 2 - half) as f32,
                    top,
                    (2 * half) as f32,
                    h - top,
                ],
            );
        }
    }

    /// An overlay holds the middle of the view (the radial menu, once it shows: a quick tap of
    /// its opener draws nothing, so nothing blinks). The 2D and F3 crosshairs, the chat's lines,
    /// the toasts and the first-person hands give way to it.
    pub(in crate::game::client_side) fn overlay_holds_view(&self) -> bool {
        self.pad_radial_shown()
    }

    /// The hands drawn in first person this frame: none while an overlay holds the view, as the
    /// radial menu's tiles sit where they are.
    pub(in crate::game::client_side) fn hands_shown(&self) -> [bool; 2] {
        if self.overlay_holds_view() {
            [false; 2]
        } else {
            self.hands_to_render()
        }
    }
}
