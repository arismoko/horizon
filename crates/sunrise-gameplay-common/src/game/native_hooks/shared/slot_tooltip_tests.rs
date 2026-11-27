use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};

pub const MENU: &str = "tips:chest";

pub(in crate::game) struct SlotTip {
    pub(in crate::game) enabled: bool,
    pub(in crate::game) mode: Arc<AtomicU8>,
    pub(in crate::game) calls: Arc<AtomicUsize>,
}
impl mod_api::MenuScreenFactory for SlotTip {
    fn create(
        &self,
        _: &dyn mod_api::ClientHost,
        open: mod_api::MenuScreenOpen<'_>,
    ) -> Option<mod_api::MenuScreen> {
        let screen = mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
            width: 176.0,
            height: 166.0,
            inventory_x: 8.0,
            inventory_y: 84.0,
            slots: (0..open.backing_slots)
                .map(|index| {
                    Some(mod_api::ScreenSlot {
                        position: [
                            8.0 + (index % 9) as f32 * 18.0,
                            18.0 + (index / 9) as f32 * 18.0,
                        ],
                        no_item_icon: None,
                    })
                })
                .collect(),
            ..Default::default()
        });
        if !self.enabled {
            return Some(screen);
        }
        let (mode, calls) = (self.mode.clone(), self.calls.clone());
        Some(screen.with_tooltip(move |_, frame, slot, lines| {
            // Player inventory slot 9 is menu slot 27 in a three-row chest.
            assert_eq!(slot, Some(27));
            assert_eq!(frame.gui_size, [512, 512]);
            assert_eq!(frame.origin, [168.0, 173.0]);
            assert_eq!(frame.pointer, [184, 265]);
            assert_eq!(frame.at, [16.0, 92.0]);
            assert!(
                lines
                    .iter()
                    .any(|(line, _)| *line == mod_api::Text::literal("item hook tail"))
            );
            calls.fetch_add(1, Ordering::Relaxed);
            match mode.load(Ordering::Relaxed) {
                1 => *lines = vec![(mod_api::Text::literal("replacement"), u32::MAX)],
                2 => lines.clear(),
                3 => lines.push((mod_api::Text::literal("slot hint"), u32::MAX)),
                4 => {
                    lines.clear();
                    panic!("fixture tooltip panic after editing")
                }
                _ => {}
            }
        }))
    }
}
pub(in crate::game) struct SlotImageData;
impl mod_api::TooltipComponentData for SlotImageData {}

pub(in crate::game) struct SlotImageClient(pub(in crate::game) Arc<AtomicUsize>);
impl mod_api::ClientTooltipComponent for SlotImageClient {
    fn height(&self, _: &dyn mod_api::TooltipFont) -> i32 {
        10
    }
    fn width(&self, _: &dyn mod_api::TooltipFont) -> i32 {
        20
    }
    fn draw_image(
        &self,
        _: &dyn mod_api::TooltipFont,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: &mut dyn mod_api::DrawContext,
    ) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ObservedComponent {
    Text(mod_api::Text, u32),
    Custom,
}

#[derive(Clone)]
pub(in crate::game) struct Image;
#[mod_api::sunrise_part]
impl mod_api::ItemPart for Image {
    fn tooltip_image(
        &self,
        _: &dyn mod_api::ClientHost,
        _: &mut mod_api::TooltipImageEvent<'_>,
    ) -> Option<mod_api::TooltipComponent> {
        Some(mod_api::TooltipComponent::custom(SlotImageData))
    }
}
pub struct Listener {
    pub enabled: bool,
    pub painted: Arc<AtomicUsize>,
    pub mode: Arc<AtomicU8>,
    pub calls: Arc<AtomicUsize>,
    pub drawn: Arc<Mutex<Vec<Vec<ObservedComponent>>>>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Listener {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let item = r.item_key("minecraft:stone");
        r.item_part(item, Image);
        r.register_menu_screen(
            MENU,
            Arc::new(SlotTip {
                enabled: self.enabled,
                mode: self.mode.clone(),
                calls: self.calls.clone(),
            }),
        )
        .unwrap();
    }
    fn register_client_tooltip_component_factories(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterClientTooltipComponentFactoriesEvent<'_>,
    ) {
        let painted = self.painted.clone();
        event
            .factories
            .register::<SlotImageData, SlotImageClient>(move |_| SlotImageClient(painted.clone()));
    }
    fn item_tooltip(&self, _: &dyn mod_api::ClientHost, event: &mut mod_api::ItemTooltipEvent<'_>) {
        event
            .lines
            .push(mod_api::Text::literal("item hook tail"), 0xFF124578)
            .unwrap();
    }
    fn render_tooltip_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderTooltipPreEvent<'_>,
    ) -> mod_api::Flow {
        let snapshot = event
            .components
            .iter()
            .filter_map(|component| {
                if let Some((text, color)) = component.as_text() {
                    Some(ObservedComponent::Text(text.clone(), color))
                } else if component.downcast_ref::<SlotImageClient>().is_some() {
                    Some(ObservedComponent::Custom)
                } else {
                    None
                }
            })
            .collect();
        self.drawn.lock().unwrap().push(snapshot);
        mod_api::Flow::Continue
    }
    fn render_tooltip_texture(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderTooltipTextureEvent<'_>,
    ) {
        assert!(event.stack.is_some());
        event.background = 0xFF123456;
    }
}
