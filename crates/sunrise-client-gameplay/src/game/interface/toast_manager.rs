use super::{mod_toasts, tutorial};
use crate::game::client_side::advancement_toast;
use std::{cell::RefCell, collections::VecDeque};
use sunrise_client::toasts::{SystemToast, ToastClock};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SystemToastId {
    Narrator,
    PackLoadFailure,
    PackDownload(usize),
    ChunkIo(usize),
    Title,
}

pub enum Toast {
    Advancement(advancement_toast::Toast),
    Tutorial(tutorial::HintToast),
    System(Option<SystemToastId>, SystemToast),
    Diagnostic {
        toast: SystemToast,
        hide_with_f1: bool,
    },
    Mod(mod_toasts::Entry),
}

pub struct VisibleToast {
    pub toast: Toast,
    pub slot: usize,
    // A reset can resize the content, but the instance keeps its original slots.
    pub(in crate::game) slots: u32,
}

#[derive(Default)]
pub struct ToastManager {
    pub pending_mod: RefCell<Vec<mod_toasts::Pending>>,
    pub queued: VecDeque<Toast>,
    pub visible: Vec<VisibleToast>,
    /// The mods holding new toasts back (`set_toasts_held`), as Dynamic FPS makes
    /// `ToastManager.freeSlotCount` 0 while the window is in the background.
    holders: RefCell<Vec<&'static str>>,
}

impl ToastManager {
    /// Mod `id` holds new toasts back, or lets them go. False, changing nothing, with no mod.
    pub(in crate::game) fn hold(&self, id: Option<&'static str>, held: bool) -> bool {
        let Some(id) = id else {
            return false;
        };
        let mut holders = self.holders.borrow_mut();
        holders.retain(|holder| *holder != id);
        if held {
            holders.push(id);
        }
        true
    }

    /// Whether an enabled mod holds new toasts back.
    pub(in crate::game) fn held(&self) -> bool {
        self.holders.borrow().iter().any(|id| {
            mod_api::__private::mod_index(id).is_some_and(mod_api::__private::mod_index_on)
        })
    }

    /// `ToastManager.addToast`: the toast waits in the queue for a free slot. Under `SUNRISE_STILL`
    /// (`ToastClock::Still`) it is done at once instead: it never takes a slot or waits in the queue,
    /// which draws the same nothing a toast held at its first frame drew, and a later dismissal or
    /// replacement finds nothing to change.
    pub(in crate::game) fn post(&mut self, clock: ToastClock, toast: Toast) {
        if clock == ToastClock::Real {
            self.queued.push_back(toast);
        }
    }

    pub(in crate::game) fn all_mut(&mut self) -> impl Iterator<Item = &mut Toast> {
        self.visible
            .iter_mut()
            .map(|entry| &mut entry.toast)
            .chain(&mut self.queued)
    }

    /// `ToastManager.getToast` looks in the visible list before the queue.
    pub(in crate::game) fn find_mut<T>(
        &mut self,
        get: impl FnMut(&mut Toast) -> Option<&mut T>,
    ) -> Option<&mut T> {
        self.all_mut().find_map(get)
    }
}

#[cfg(test)]
mod still_tests {
    use crate::game::{ClientGame, native_entity_refs::tests::with_client_game};
    use sunrise_client::{clock::FixedStep, hud::HudBatch, text::Text, toasts::SystemToast};
    use sunrise_gameplay_common::game::{
        in_own_process, native_hooks::block_entity_on_load::world_for_mods,
    };

    /// One draw of every toast: its vertices' bytes.
    fn frame(game: &mut ClientGame<'_>) -> Vec<u8> {
        let mut batch = HudBatch::scratch([640, 480]);
        game.draw_every_toast(&mut batch);
        bytemuck::cast_slice(&batch.vertices).to_vec()
    }

    /// Under `SUNRISE_STILL` a posted toast is done at once: 1000 of them (system and mod toasts),
    /// through a level change, `SetTime` and the title screen, leave the slots and the queue empty
    /// and draw the frame a game with none draws. Normal play queues and shows them as before.
    #[test]
    #[ignore = "daily"]
    fn a_still_game_posts_toasts_into_nothing() {
        in_own_process(
            "game::toast_manager::still_tests::a_still_game_posts_toasts_into_nothing",
            || {
                with_client_game(|game| {
                    let saves = tempfile::tempdir().unwrap();
                    game.common.clock = FixedStep::still_for_tests();
                    let empty = frame(game);
                    for i in 0..1000 {
                        let title = Text::literal(format!("Saved {i}"));
                        if i % 2 == 0 {
                            assert!(game.add_system_toast(None, SystemToast::new(title, None)));
                        } else {
                            game.add_toast(mod_api::ModToast::new(mod_api::Text::literal(
                                format!("Mod {i}"),
                            )))
                            .unwrap();
                        }
                        match i {
                            300 => {
                                game.common.world = world_for_mods(game.common.res.facts().air, 9)
                            }
                            600 => {
                                game.common.world.game_time = 90_000;
                                game.common.world.day_time = 90_000;
                            }
                            900 => {
                                game.client.title.0 =
                                    Some(game.new_title(saves.path().to_path_buf()));
                            }
                            _ => {}
                        }
                        if i % 100 == 99 {
                            assert_eq!(frame(game), empty, "after {} toasts", i + 1);
                        }
                    }
                    assert_eq!(frame(game), empty);
                    assert!(game.client.toast_manager.visible.is_empty());
                    assert!(game.client.toast_manager.queued.is_empty());

                    // Normal play: five slots fill, the sixth waits.
                    game.common.clock = FixedStep::default();
                    assert!(!game.common.clock.still());
                    for i in 0..6 {
                        let title = Text::literal(format!("Real {i}"));
                        assert!(game.add_system_toast(None, SystemToast::new(title, None)));
                    }
                    frame(game);
                    assert_eq!(game.client.toast_manager.visible.len(), 5);
                    assert_eq!(game.client.toast_manager.queued.len(), 1);
                });
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(manager: &mut ToastManager, title: &str) {
        manager
            .queued
            .push_back(Toast::System(None, SystemToast::new(title.into(), None)));
    }

    /// While an enabled mod holds them, new toasts wait in the queue, those already shown go on,
    /// and they show once the last holder lets go; a call with no mod holds nothing.
    #[test]
    fn held_toasts_wait_in_the_queue_until_released() {
        struct Quiet;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Quiet {}
        let mods = vec![
            mod_api::ModEntry::new("first", Box::new(Quiet)),
            mod_api::ModEntry::new("second", Box::new(Quiet)),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            let mut manager = ToastManager::default();
            notice(&mut manager, "shown");
            manager.update(ToastClock::Real, 1.0);
            assert_eq!(manager.visible.len(), 1);

            assert!(!manager.hold(None, true));
            assert!(manager.hold(Some("first"), true));
            assert!(manager.hold(Some("second"), true));
            notice(&mut manager, "waits");
            manager.update(ToastClock::Real, 1.0);
            assert_eq!((manager.visible.len(), manager.queued.len()), (1, 1));

            assert!(manager.hold(Some("first"), false));
            manager.update(ToastClock::Real, 1.0);
            assert_eq!((manager.visible.len(), manager.queued.len()), (1, 1));

            assert!(manager.hold(Some("second"), false));
            manager.update(ToastClock::Real, 1.0);
            assert_eq!((manager.visible.len(), manager.queued.len()), (2, 0));

            // An unknown (or disabled) mod's hold holds nothing.
            assert!(manager.hold(Some("gone"), true));
            notice(&mut manager, "unheld");
            manager.update(ToastClock::Real, 1.0);
            assert_eq!((manager.visible.len(), manager.queued.len()), (3, 0));
        });
    }
}
