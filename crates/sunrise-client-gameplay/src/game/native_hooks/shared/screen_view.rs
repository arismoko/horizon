use super::*;

/// `view` as the screen `id` (an overlay or a title screen, which shows over
/// the whole window with no menu).
pub(in crate::game) fn screen_view_with_id<'a>(
    view: &ScreenView<'a>,
    id: &'static str,
) -> ScreenView<'a> {
    ScreenView::from(mod_api::__private::built::ScreenView {
        id,
        mod_screen: if id == "mod" { view.mod_screen } else { None },
        menu: view.menu,
        title: view.title,
        rect: view.rect,
        slots: view.slots,
        mouse: view.mouse,
        partial_tick: view.partial_tick,
        // Opening, closing and init come before the screen draws: nothing beside it yet.
        regions: &[],
    })
}

/// `view` as mod `mod_id`'s own screen (`id == "mod"`, `mod_screen == Some(mod_id)`).
pub fn screen_view_for_mod<'a>(view: &ScreenView<'a>, mod_id: &'a str) -> ScreenView<'a> {
    ScreenView::from(mod_api::__private::built::ScreenView {
        id: "mod",
        mod_screen: Some(mod_id),
        menu: None,
        title: None,
        rect: view.rect,
        slots: 0,
        mouse: view.mouse,
        partial_tick: view.partial_tick,
        regions: &[],
    })
}
