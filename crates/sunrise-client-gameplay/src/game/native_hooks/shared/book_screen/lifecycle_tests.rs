use super::*;
use std::sync::{Mutex, OnceLock};
use sunrise_sim::recipe_book::{settings_codec, settings_sync::Message};

pub static KEY: OnceLock<mod_api::RecipeBookTypeKey> = OnceLock::new();
pub struct BookMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for BookMod {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar.register_recipe_type("geometry:crafting");
        registrar.register_raw_recipe_serializer("geometry:crafting", "geometry:crafting");
        let key = registrar.recipe_book_type("geometry:book").unwrap();
        assert_eq!(KEY.get_or_init(|| key.clone()), &key);
    }
    fn recipe_book_categories(
        &self,
        _: mod_api::HostSide,
        event: &mut mod_api::RecipeBookCategoriesEvent<'_>,
    ) {
        event.categories.push(mod_api::RecipeBookCategory {
            id: "geometry:initial".into(),
            recipe_type: "geometry:crafting".into(),
            icon: "minecraft:stone".into(),
            secondary_icon: None,
            target_menu: "geometry:menu".into(),
            book: KEY.get().unwrap().clone(),
            search_group: "geometry:search".into(),
            displays: Vec::new(),
        });
    }
}
pub fn receive(game: &mut crate::game::ClientGame<'_>, message: Message) {
    game.remote_book_settings(&settings_codec::encode(&message).unwrap());
}
pub type BoundLayoutEdit = fn(&mut mod_api::MenuScreenLayout);

#[derive(Default)]
pub struct BoundBookMod {
    pub edit: Arc<Mutex<Option<BoundLayoutEdit>>>,
    pub no_factory: bool,
}

pub(in crate::game) struct BoundBookScreen {
    pub(in crate::game) book: mod_api::RecipeBookTypeKey,
    pub(in crate::game) edit: Arc<Mutex<Option<BoundLayoutEdit>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for BoundBookMod {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let book = registrar.recipe_book_type("bound:book").unwrap();
        registrar
            .menu_recipe_book(
                "bound:menu",
                &book,
                "bound:recipe",
                mod_api::RecipeBookLayout::Crafting {
                    width: 1,
                    height: 1,
                    slots: vec![0],
                    result: 1,
                },
            )
            .unwrap();
        if !self.no_factory {
            registrar
                .register_menu_screen(
                    "bound:menu",
                    Arc::new(BoundBookScreen {
                        book,
                        edit: self.edit.clone(),
                    }),
                )
                .unwrap();
        }
    }
}

impl mod_api::MenuScreenFactory for BoundBookScreen {
    fn create(
        &self,
        _: &dyn mod_api::ClientHost,
        open: mod_api::MenuScreenOpen<'_>,
    ) -> Option<mod_api::MenuScreen> {
        assert_eq!(open.block, None);
        let mut layout = mod_api::MenuScreenLayout {
            width: 176.0,
            height: 166.0,
            slots: vec![
                Some(mod_api::ScreenSlot {
                    position: [8.0, 18.0],
                    ..Default::default()
                }),
                Some(mod_api::ScreenSlot {
                    position: [80.0, 18.0],
                    ..Default::default()
                }),
            ],
            recipe_book: Some(self.book.clone()),
            recipe_book_toggle: Some(mod_api::RecipeBookToggle {
                x: 153.0,
                y: 60.0,
                from: mod_api::ToggleOrigin::Menu,
            }),
            ..Default::default()
        };
        if let Some(edit) = *self.edit.lock().unwrap() {
            edit(&mut layout);
        }
        Some(mod_api::MenuScreen::new(layout))
    }
}
pub fn with_bound_menu_type(test: impl FnOnce()) {
    sunrise_level::containers::with_mod_menu_types(
        [(
            "bound:menu".into(),
            "bound".into(),
            3,
            0,
            vec![1],
            vec![0],
            None,
            None,
            true,
            None,
        )],
        test,
    );
}

pub const STONE: &str = "minecraft:stone";
pub const MENU: &str = "minecraft:generic_9x1";
pub fn geometry(game: &crate::game::ClientGame<'_>) -> ((f32, f32), Vec<(usize, [f32; 4])>) {
    let batch =
        sunrise_client::hud::HudBatch::new([1280, 720], game.client.settings.gui_scale_inputs());
    let layout = game.built_container(&batch).expect("cached container");
    (layout.origin, layout.slots.clone())
}

pub const REOPENED_MENU: &str = "reopened:menu";
pub type ScreenTrace = Arc<Mutex<Vec<(&'static str, i32, i32)>>>;

pub(in crate::game) struct OpeningProbe {
    pub(in crate::game) trace: ScreenTrace,
    pub(in crate::game) window: i32,
    pub(in crate::game) marker: i32,
}
impl OpeningProbe {
    pub(in crate::game) fn record(&self, event: &'static str) {
        self.trace
            .lock()
            .unwrap()
            .push((event, self.window, self.marker));
    }
}
impl Drop for OpeningProbe {
    fn drop(&mut self) {
        self.record("drop");
    }
}

pub struct ReopenedMod(pub ScreenTrace);
#[mod_api::__subscriptions]
impl mod_api::Hooks for ReopenedMod {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let trace = self.0.clone();
        registrar
            .register_menu_screen(
                REOPENED_MENU,
                Arc::new(
                    move |_: &dyn mod_api::ClientHost, open: mod_api::MenuScreenOpen<'_>| {
                        let marker = i32::from_be_bytes(open.extra.try_into().ok()?);
                        let probe = Arc::new(OpeningProbe {
                            trace: trace.clone(),
                            window: open.window,
                            marker,
                        });
                        probe.record("open");
                        let draw = probe.clone();
                        let press = probe.clone();
                        Some(
                            mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                                width: 176.0,
                                height: 166.0,
                                own_labels: true,
                                own_background: true,
                                ..Default::default()
                            })
                            .with_draw(move |_, _, _| draw.record("draw"))
                            .with_click(move |_, frame, right| {
                                assert!(right);
                                assert_eq!(frame.at, [17.0, 25.0]);
                                press.record("press");
                                Some(press.marker)
                            })
                            .with_tooltip(move |_, _, slot, lines| {
                                assert_eq!(slot, None);
                                probe.record("tooltip");
                                lines.push((
                                    mod_api::Text::literal(probe.marker.to_string()),
                                    u32::MAX,
                                ));
                            }),
                        )
                    },
                ),
            )
            .unwrap();
    }
}

#[test]
fn open_native_screen_set_data_accepts_full_signed_and_mod_data_ranges() {
    let mut stock = OpenNativeScreen {
        menu: OpenNativeMenu {
            key: (0, 0, 0),
            window: 7,
            generation: 1,
            name: "minecraft:hopper".into(),
            title: None,
            block: None,
            pos: None,
            mod_type: None,
            data: Vec::new(),
        },
        slot_moves: Default::default(),
        layout_revision: 0,
        shown: true,
        detached: false,
        screen: mod_api::__private::OwnedMenuScreen::new(
            "data-range",
            0,
            mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                width: 176.0,
                height: 166.0,
                slots: Vec::new(),
                inventory_y: 84.0,
                inventory_x: 8.0,
                title_label: None,
                inventory_label: None,
                recipe_book: None,
                ..Default::default()
            }),
        ),
    };
    assert!(stock.set_data(7, 0, 11));
    assert!(stock.set_data(7, 32_767, 22));
    assert_eq!(stock.menu.data[0], 11);
    assert_eq!(stock.menu.data[32_767], 22);
    assert_eq!(stock.menu.data.len(), 32_768);
    assert!(stock.set_data(7, 32_768, 33));
    assert_eq!(stock.menu.data.len(), 32_768);
}
