#![cfg(test)]
#[cfg(feature = "client")]
use sunrise_client::hud::HudBatch;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "recipe_book/check_mod_menu_tests.rs"]
pub(crate) mod check_mod_menu_tests;

#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) mod screen_room_tests {
    use super::*;
    use sunrise_client::recipe_book_screen as screen;
    use sunrise_client::recipe_book_screen::Call;
    use sunrise_client::recipe_book_screen::Screen as Layout;
    use sunrise_client::recipe_book_screen::Search;
    use sunrise_gameplay_common::game::Screen;
    use sunrise_sim::recipe_book::BookType;
    #[test]
    #[ignore = "daily"]
    fn shifted_book_painter_and_input_share_the_absolute_origin() {
        use mod_api::{ClientHost, Hooks, ModEntry, ScreenRoomEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Panel(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Panel {
            fn screen_room(&self, _: &dyn ClientHost, event: &mut ScreenRoomEvent<'_>) {
                event.right = event.right.max(self.0.load(Ordering::Relaxed) as f32);
            }
        }
        let room = Arc::new(AtomicUsize::new(100));
        let mods: mod_api::Mods =
            vec![ModEntry::new("panel", Box::new(Panel(room.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
                game.client.screen = Screen::Inventory;
                game.container = None;
                game.craft_size = 2;
                game.client.settings.gui_scale = 1;
                game.level.recipe_book.open[BookType::Crafting.index()] = true;
                let batch = HudBatch::new([640, 240], game.client.settings.gui_scale_inputs());
                let (x, y) = game.book_origin(&batch);
                assert_eq!((x, y), (110.0, 37.0));
                assert_eq!(game.tab_pos(x, y, 0), (80.0, 40.0));
                assert_eq!(
                    sunrise_client_gameplay::game::ClientGame::button_pos(x, y, 0),
                    (121.0, 68.0)
                );
                assert_eq!(game.inventory_origin(&batch), (259.0, 37.0));
                let search = Search::default();
                let tabs = [screen::TabView {
                    icons: ("minecraft:stone".into(), None),
                    selected: true,
                    anim: 0.0,
                }];
                let layout = Layout {
                    book: BookType::Crafting,
                    origin: (x as i32, y as i32),
                    mouse: (221, 50),
                    index: 0,
                    filtering: false,
                    filter_look: None,
                    search: &search,
                    tabs: &tabs,
                    buttons: &[],
                    page: 0,
                    pages: 1,
                    overlay: None,
                    ghost: None,
                    translate: &str::to_owned,
                    text_width: &|s| s.len() as i32,
                };
                let calls = screen::draw(&layout);
                assert!(matches!(calls[0].call, Call::Blit { x: 110, y: 37, .. }));
                assert!(calls.iter().any(|c| matches!(&c.call, Call::Sprite { id, x: 78, y: 40, .. } if id == "recipe_book/tab_selected")));
                assert!(calls.iter().any(|c| matches!(&c.call, Call::Sprite { id, x: 220, y: 49, .. } if id == "recipe_book/filter_disabled_highlighted")));
                // A panel-only fit at 510 must be refused because the selected tab would clip.
                let batch = HudBatch::new([510, 240], game.client.settings.gui_scale_inputs());
                room.store(180, Ordering::Relaxed);
                game.client.frames += 1;
                assert_eq!(game.open_book_left(&batch), Some(63.0));
                assert_eq!(game.book_origin(&batch), (95.0, 37.0));
                assert_eq!(game.inventory_origin(&batch), (244.0, 37.0));
                room.store(100, Ordering::Relaxed);
                game.client.frames += 1;
                assert_eq!(game.book_origin(&batch), (45.0, 37.0));
                assert_eq!(game.inventory_origin(&batch), (194.0, 37.0));
            });
        });
    }
}
