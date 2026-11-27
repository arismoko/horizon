#![cfg(test)]
use sunrise_client::hud::HudBatch;
use sunrise_client_gameplay::game::client_side::container_hooks::BUILT;
use sunrise_items::items::ItemStack;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;
#[cfg(test)]
pub(crate) mod tests {

    use super::*;

    use sunrise_gameplay_common::game::Screen;

    use crate::game::native_entity_refs::tests::with_client_game;

    use sunrise_hooks::containers::{
        Button, Click, Container, ContainerHooks, SlotId, engine::with_registrants,
    };
    use sunrise_hooks::hud;

    /// Takes every press, drag and key.
    struct Take;
    impl ContainerHooks for Take {
        fn press(&self, _: &mut dyn Container, _: Option<SlotId>, _: Button) -> bool {
            true
        }
        fn drag(&self, _: &mut dyn Container, _: Option<SlotId>, _: Button) -> bool {
            true
        }
        fn key(&self, _: &mut dyn Container, _: Option<SlotId>, _: &str) -> bool {
            true
        }
    }
    static TAKE: Take = Take;
    /// Picks up slot 9 on every press.
    struct PickNine;
    impl ContainerHooks for PickNine {
        fn press(&self, c: &mut dyn Container, _: Option<SlotId>, _: Button) -> bool {
            c.click(SlotId(9), Click::Pick { right: false })
        }
    }
    static PICK_NINE: PickNine = PickNine;
    /// Takes every release.
    struct TakeRelease;
    impl ContainerHooks for TakeRelease {
        fn release(&self, _: &mut dyn Container, _: Option<SlotId>, _: Button) -> bool {
            true
        }
    }
    static TAKE_RELEASE: TakeRelease = TakeRelease;
    /// Reads slots no menu has, counting each call whose reads all come back empty.
    struct ReadFar(std::sync::atomic::AtomicUsize);
    impl ReadFar {
        fn read(&self, c: &dyn Container) -> bool {
            // None of these is a slot of the player's inventory screen (500 to 504, its armour, are).
            let far = [SlotId(36), SlotId(600), SlotId(9999)];
            let empty = far
                .iter()
                .all(|&s| c.stack(s).is_none() && c.name(s).is_none());
            if empty {
                self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            empty
        }
    }
    impl ContainerHooks for ReadFar {
        fn press(&self, c: &mut dyn Container, _: Option<SlotId>, _: Button) -> bool {
            self.read(c)
        }
        fn draw(&self, c: &dyn Container, _: &mut dyn hud::Canvas) {
            self.read(c);
        }
    }
    static READ_FAR: ReadFar = ReadFar(std::sync::atomic::AtomicUsize::new(0));
    /// Takes nothing.
    struct Pass;
    impl ContainerHooks for Pass {}
    static PASS: Pass = Pass;

    #[test]
    #[ignore = "daily"]
    fn a_hooks_click_goes_through_the_menu() {
        with_client_game(|game| {
            let _server = sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(game);
            game.client.screen = Screen::Inventory;
            let stone = ItemStack::new("minecraft:stone", 5);
            game.inventory[9] = Some(stone.clone());
            with_registrants(&[&PICK_NINE], || {
                assert!(game.container_mouse(MouseButton::Left, true, false))
            });
            assert_eq!(game.carried, Some(stone));
            assert!(game.inventory[9].is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_taken_press_keeps_its_release_from_vanilla() {
        with_client_game(|game| {
            game.client.screen = Screen::Inventory;
            with_registrants(&[&TAKE], || {
                assert!(game.container_mouse(MouseButton::Left, true, false));
                // Take takes no release: the button-up says the press was the hooks'.
                let owned = game.container_button_up(MouseButton::Left);
                assert!(owned);
                assert!(game.container_mouse(MouseButton::Left, false, owned));
                assert!(
                    !game.container_button_up(MouseButton::Left),
                    "the latch is cleared"
                );
            });
            with_registrants(&[&PASS], || {
                assert!(!game.container_mouse(MouseButton::Left, true, false));
                assert!(!game.container_button_up(MouseButton::Left));
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_release_taken_from_vanillas_press_ends_its_drag_unclicked() {
        with_client_game(|game| {
            game.client.screen = Screen::Inventory;
            game.client.inv_drag = Some((0, vec![9]));
            with_registrants(&[&TAKE_RELEASE], || {
                assert!(game.container_mouse(MouseButton::Left, false, false))
            });
            assert!(game.client.inv_drag.is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_slot_outside_the_open_menu_reads_nothing() {
        with_client_game(|game| {
            game.client.screen = Screen::Inventory;
            let reads = || READ_FAR.0.load(std::sync::atomic::Ordering::Relaxed);
            // The press is taken only when the hook ran and every far read came back empty.
            with_registrants(&[&READ_FAR], || {
                assert!(game.container_mouse(MouseButton::Left, true, false))
            });
            assert_eq!(reads(), 1);
            let mut batch = HudBatch::new([854, 480], game.client.settings.gui_scale_inputs());
            with_registrants(&[&READ_FAR], || game.draw_container_hooks(&mut batch));
            assert_eq!(reads(), 2, "the draw view reads nothing there either");
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_key_is_not_dispatched_while_the_recipe_search_is_focused() {
        with_client_game(|game| {
            game.client.screen = Screen::Inventory;
            with_registrants(&[&TAKE], || assert!(game.container_key(KeyCode::KeyR)));
            game.level.recipe_book.open[sunrise_sim::recipe_book::BookType::Crafting.index()] =
                true;
            game.ensure_recipe_search();
            game.recipe_focus_search();
            assert!(game.recipe_search_focused());
            BUILT.with(|n| n.set(0));
            with_registrants(&[&TAKE], || assert!(!game.container_key(KeyCode::KeyR)));
            assert_eq!(BUILT.with(|n| n.get()), 0);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn no_view_is_built_without_a_registrant_or_a_held_button() {
        with_client_game(|game| {
            game.client.screen = Screen::Inventory;
            BUILT.with(|n| n.set(0));
            with_registrants(&[], || {
                assert!(!game.container_mouse(MouseButton::Left, true, false))
            });
            assert_eq!(BUILT.with(|n| n.get()), 0);
            // No button is held: a pointer move is no drag, however a press went.
            with_registrants(&[&TAKE], || assert!(!game.container_drag()));
            assert_eq!(BUILT.with(|n| n.get()), 0);
            with_registrants(&[&TAKE], || {
                assert!(game.container_mouse(MouseButton::Left, true, false))
            });
            assert_eq!(BUILT.with(|n| n.get()), 1);
        });
    }
}
