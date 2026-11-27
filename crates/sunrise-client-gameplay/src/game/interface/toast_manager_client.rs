use crate::game::ClientGame;
use crate::game::toast_manager::SystemToastId;
use crate::game::toast_manager::Toast;
use crate::game::toast_manager::ToastManager;
use crate::game::toast_manager::VisibleToast;
use sunrise_client::{
    hud::HudBatch,
    toasts::{SystemToast, ToastClock},
};

impl ClientGame<'_> {
    pub(in crate::game::client_side) fn add_or_update_system_toast(
        &mut self,
        id: SystemToastId,
        new: SystemToast,
    ) -> bool {
        if let Some(toast) = self.client.toast_manager.system_mut(id) {
            toast.reset_from(new);
            true
        } else {
            self.add_system_toast(Some(id), new)
        }
    }
    pub fn add_system_toast(&mut self, id: Option<SystemToastId>, mut toast: SystemToast) -> bool {
        if !self.admit_toast(
            mod_api::ToastKind::System,
            &mut toast.title,
            &mut toast.message,
        ) {
            return false;
        }
        let clock = ToastClock::of(self.common.clock.still());
        self.client
            .toast_manager
            .post(clock, Toast::System(id, toast));
        true
    }
    /// `ToastManager.extractRenderState`: F1 hides the draw, not the update.
    pub fn draw_every_toast(&mut self, batch: &mut HudBatch) {
        let clock = ToastClock::of(self.common.clock.still());
        let replay = self.client.replay.playing();
        let hidden = self.client.chat_ui.player_hides_gui || replay;
        let diagnostic_hidden = replay || self.overlay_holds_view();
        self.admit_mod_toasts();
        self.add_server_report_toasts();
        self.add_hook_panic_toasts();
        self.update_tutorial_toasts();
        let multiplier = self.client.settings.notification_display_time;
        let (visibility_sound, challenge_sound) =
            self.client.toast_manager.update(clock, multiplier);
        if let Some(show) = visibility_sound {
            let event = if show {
                "minecraft:ui.toast.in"
            } else {
                "minecraft:ui.toast.out"
            };
            self.play_audio(event, None, 1.0, 1.0);
        }
        if challenge_sound {
            self.play_audio("minecraft:ui.toast.challenge_complete", None, 1.0, 1.0);
        }
        let atlas = &self.client.gui.atlas;
        for entry in &self.client.toast_manager.visible {
            let y = entry.slot as f32 * 32.0;
            match &entry.toast {
                Toast::Diagnostic {
                    toast,
                    hide_with_f1,
                } if !diagnostic_hidden
                    && !(*hide_with_f1 && self.client.chat_ui.player_hides_gui) =>
                {
                    toast.draw(atlas, batch, y)
                }
                Toast::Advancement(t) if !hidden => {
                    self.draw_advancement_toast(batch, t, y, multiplier)
                }
                Toast::Tutorial(t) if !hidden => t.draw(atlas, batch, y),
                Toast::System(_, t) if !hidden => t.draw(atlas, batch, y),
                Toast::Mod(t) if !hidden => self.draw_mod_toast(batch, t, entry.slot),
                _ => {}
            }
        }
        self.draw_controller_notice(batch, clock);
        self.draw_music_toast(batch, hidden, clock);
    }
}

impl ToastManager {
    pub fn system_mut(&mut self, id: SystemToastId) -> Option<&mut SystemToast> {
        self.find_mut(|entry| match entry {
            Toast::System(Some(key), toast) if *key == id => Some(toast),
            _ => None,
        })
    }
    /// `ToastManager.update`: finish/update the visible list before offering
    /// vacancies to the queue, in insertion order. Admission cannot sound a
    /// visibility change until the next update of the promoted instance.
    pub fn update(&mut self, clock: ToastClock, multiplier: f64) -> (Option<bool>, bool) {
        let mut visibility_sound = None;
        self.visible.retain_mut(|entry| {
            let (sound, finished) = match &mut entry.toast {
                Toast::Advancement(t) => t.update(clock, multiplier),
                Toast::Tutorial(t) => t.update(clock),
                Toast::System(_, t) | Toast::Diagnostic { toast: t, .. } => {
                    t.update(clock, multiplier)
                }
                Toast::Mod(t) => t.request.toast.update(clock, multiplier),
            };
            visibility_sound = visibility_sound.or(sound);
            !finished
        });

        // ToastManager.SLOT_COUNT and findFreeSlotsIndex: first consecutive run that fits.
        const SLOT_COUNT: usize = 5;
        let mut used = self
            .visible
            .iter()
            .fold(0u32, |used, entry| used | entry.slots);
        let mut challenge_sound = false;
        // A held queue admits nothing: its toasts wait, as they do for a free slot.
        if !self.held() && used.count_ones() < SLOT_COUNT as u32 {
            for _ in 0..self.queued.len() {
                let Some(toast) = self.queued.pop_front() else {
                    break;
                };
                let count = match &toast {
                    Toast::Tutorial(t) => t.slots(),
                    Toast::Mod(t) => t.layout.slots(),
                    _ => 1,
                };
                let slot = SLOT_COUNT.checked_sub(count).and_then(|last| {
                    let mask = (1 << count) - 1;
                    (0..=last).find(|slot| used & (mask << slot) == 0)
                });
                let Some(slot) = slot else {
                    self.queued.push_back(toast);
                    continue;
                };
                let slots = ((1 << count) - 1) << slot;
                used |= slots;
                challenge_sound |= matches!(&toast, Toast::Advancement(t) if t.is_challenge());
                self.visible.push(VisibleToast { toast, slot, slots });
            }
        }
        (visibility_sound, challenge_sound)
    }
}
