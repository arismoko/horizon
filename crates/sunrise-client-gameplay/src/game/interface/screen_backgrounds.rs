/// `view` with no regions: a cancelled render drew nothing beside the panel.
pub(in crate::game) fn without_regions<'a>(
    view: &mod_api::ScreenView<'a>,
) -> mod_api::ScreenView<'a> {
    mod_api::ScreenView::from(mod_api::__private::built::ScreenView {
        id: view.id,
        mod_screen: view.mod_screen,
        menu: view.menu,
        title: view.title,
        rect: view.rect,
        slots: view.slots,
        mouse: view.mouse,
        partial_tick: view.partial_tick,
        regions: &[],
    })
}
