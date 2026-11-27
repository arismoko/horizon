use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(in crate::game) struct IconScreen(pub(in crate::game) Arc<AtomicBool>);
impl mod_api::MenuScreenFactory for IconScreen {
    fn create(
        &self,
        _: &dyn mod_api::ClientHost,
        _: mod_api::MenuScreenOpen<'_>,
    ) -> Option<mod_api::MenuScreen> {
        Some(mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
            width: 176.0,
            height: 166.0,
            slots: vec![Some(mod_api::ScreenSlot {
                position: [8.0, 18.0],
                no_item_icon: Some(
                    if self.0.load(Ordering::Relaxed) {
                        "icon_open:missing"
                    } else {
                        "minecraft:container/slot/shield"
                    }
                    .into(),
                ),
            })],
            ..Default::default()
        }))
    }
}
pub const ICON_MENU: &str = "icon_open:menu";

pub struct IconMod(pub Arc<AtomicBool>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for IconMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key("icon_open:machine");
        r.block_container(
            &block,
            mod_api::ModContainer {
                slots: 9,
                menu: Some(ICON_MENU.into()),
                ..Default::default()
            },
        );
        r.register_menu_screen(ICON_MENU, Arc::new(IconScreen(self.0.clone())))
            .unwrap();
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new("icon_open:machine", "minecraft:barrel").with_item(false))
            .unwrap();
    }
}
