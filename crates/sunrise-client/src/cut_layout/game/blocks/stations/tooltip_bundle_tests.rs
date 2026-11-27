use mod_api::TooltipElement;
use mod_api::{ClientHost, Hooks, ModEntry, Mods, RenderTooltipGatherComponentsEvent};
use sunrise_client::hud::HudBatch;
use sunrise_client::hud::rgba;
use sunrise_client_gameplay::game::client_item_extensions::DEFAULT_TOOLTIP_FONT_ID;
use sunrise_client_gameplay::game::client_side::stations::tooltip::PreparedTooltipItem;
use sunrise_client_gameplay::game::host::draw::EngineTooltipFont;
use sunrise_gameplay_common::game::TryIntoApi;
use sunrise_items::items::ItemStack;

struct Edit(bool);
#[mod_api::__subscriptions]
impl Hooks for Edit {
    fn render_tooltip_gather_components(
        &self,
        _: &dyn ClientHost,
        e: &mut RenderTooltipGatherComponentsEvent<'_>,
    ) -> mod_api::Flow {
        assert!(matches!(
            &e.elements[1],
            TooltipElement::Component(c) if c.as_bundle().is_some()
        ));
        let image = e.elements.remove(1);
        if self.0 {
            e.elements.push(image);
        }
        mod_api::Flow::Continue
    }
}

fn run_bundle_case(name: &str) -> bool {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        &format!("cut_layout::game::client_side::stations::tooltip::bundle_tests::{name}"),
        "SUNRISE_BUNDLE_TOOLTIP_CASE",
        &["bundle"],
    )
}

fn run(mode: Option<bool>, test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>)) {
    let mods: Mods = mode
        .map(|mode| ModEntry::new("bundle-test", Box::new(Edit(mode))))
        .into_iter()
        .collect::<Vec<_>>()
        .into();
    run_mods(mods, test);
}

fn run_mods(mods: Mods, test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>)) {
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        sunrise_client_gameplay::game::client_side::native_entity_refs::tests::with_client_game(
            |game| {
                game.client.gui.atlas.sprites.insert(
                    "container/bundle/slot_background".into(),
                    sunrise_client::hud::Rect {
                        u0: 0.25,
                        v0: 0.25,
                        u1: 0.5,
                        v1: 0.5,
                        w: 24,
                        h: 24,
                    },
                );
                test(game);
            },
        )
    });
}
fn bundle() -> ItemStack {
    let mut stack = ItemStack::new("minecraft:bundle", 1);
    stack.data_mut().bundle_contents = vec![ItemStack::new("minecraft:stone", 1)];
    stack
}
fn draw(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, stack: &ItemStack) -> HudBatch {
    let mut batch = HudBatch::scratch([320, 240]);
    game.draw_stack_tooltip_with(
        &mut batch,
        stack,
        false,
        10.0,
        20.0,
        vec![(sunrise_game_data::text::Text::literal("Rest"), u32::MAX)],
    );
    batch
}
fn grid(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    batch: &HudBatch,
    y: f32,
    present: bool,
) {
    // Positioner: x + 12, y - 12. Slot right-aligned: 22 + 72 = 94.
    let mut expected = HudBatch::scratch([320, 240]);
    sunrise_client::ui::nine_slice(
        &mut expected,
        &game.client.gui.atlas,
        "container/bundle/slot_background",
        94.0,
        y,
        24.0,
        24.0,
        u32::MAX,
    );
    assert!(!expected.vertices.is_empty());
    let found = batch
        .vertices
        .windows(expected.vertices.len())
        .any(|window| {
            window
                .iter()
                .zip(&expected.vertices)
                .all(|(a, b)| a.pos == b.pos && a.uv == b.uv && a.color == b.color)
        });
    assert_eq!(found, present);
}
fn bottom(batch: &HudBatch) -> f32 {
    batch.vertices.iter().map(|v| v.pos[1]).fold(0.0, f32::max)
}
#[test]
#[ignore = "daily"]
fn bundle_no_listener_order_and_draw_position() {
    if run_bundle_case("bundle_no_listener_order_and_draw_position") {
        return;
    }
    run(None, |game| {
        let stack = bundle();
        let lines = vec![
            (sunrise_game_data::text::Text::literal("Name"), u32::MAX),
            (sunrise_game_data::text::Text::literal("Rest"), u32::MAX),
        ];
        let elements = game
            .gather_tooltip_components(&stack, lines, 10, (320.0, 240.0))
            .unwrap_ready();
        let font = EngineTooltipFont::new(&game.client.gui.atlas.font, &DEFAULT_TOOLTIP_FONT_ID);
        assert_eq!(elements.len(), 3);
        assert!(elements[0].as_text().is_some());
        assert!(elements[1].as_bundle().is_some());
        assert_eq!(
            game.measure_prepared_item(&elements[1], &font),
            Some((96, 45))
        );
        assert!(elements[2].as_text().is_some());
        let batch = draw(game, &stack);
        // top 8 + name 10 + first-component padding 2 = 20.
        grid(game, &batch, 20.0, true);
    });
}
#[test]
#[ignore = "daily"]
fn bundle_gather_moves_image_to_end() {
    run(Some(true), |game| {
        let batch = draw(game, &bundle());
        // top 8 + name 10 + padding 2 + rest 10 = 30.
        grid(game, &batch, 30.0, true);
        grid(game, &batch, 20.0, false);
    });
}
#[test]
#[ignore = "daily"]
fn bundle_gather_keeps_vanillas_selected_slot() {
    if run_bundle_case("bundle_gather_keeps_vanillas_selected_slot") {
        return;
    }
    run(Some(true), |game| {
        let mut stack = bundle();
        stack.data_mut().bundle_selected = sunrise_items::item_data::BundleSelected(0);
        let lines = vec![(sunrise_game_data::text::Text::literal("Name"), u32::MAX)];
        let elements = game
            .gather_tooltip_components(&stack, lines, 10, (320.0, 240.0))
            .unwrap_ready();
        let PreparedTooltipItem::Bundle { contents, .. } = &elements[1] else {
            panic!("the unchanged bundle must still be prepared");
        };
        // The transient selection isn't in the API stack patch. Keeping the original
        // vanilla source is necessary for ClientBundleTooltip's selected-item tooltip.
        assert_eq!(contents.selected(), 0);
    });
}

#[test]
#[ignore = "daily"]
fn bundle_gather_removes_grid_and_height() {
    let mut full_bottom = 0.0;
    run(None, |game| {
        full_bottom = bottom(&draw(game, &bundle()));
    });
    run(Some(false), |game| {
        let batch = draw(game, &bundle());
        grid(game, &batch, 20.0, false);
        // One 24px row + 13px bar + 8px gap = 45px.
        assert_eq!(full_bottom - bottom(&batch), 45.0);
    });
}
#[test]
#[ignore = "daily"]
fn non_bundle_keeps_text_only_geometry() {
    run(None, |game| {
        let stack = ItemStack::new("minecraft:stick", 1);
        let batch = draw(game, &stack);
        grid(game, &batch, 20.0, false);
        assert_eq!(game.stack_tooltip_lines(&stack, false, false).len(), 1);
        // Two 10px lines: top 8, content height 20, outer border extends through y + h + 4.
        assert_eq!(bottom(&batch), 32.0);
    });
}

#[test]
#[ignore = "daily"]
fn bundle_empty_text_gathers_image_at_zero() {
    if run_bundle_case("bundle_empty_text_gathers_image_at_zero") {
        return;
    }
    run(None, |game| {
        let elements = game
            .gather_tooltip_components(&bundle(), vec![], 10, (320.0, 240.0))
            .unwrap_ready();
        let font = EngineTooltipFont::new(&game.client.gui.atlas.font, &DEFAULT_TOOLTIP_FONT_ID);
        assert_eq!(elements.len(), 1);
        assert!(elements[0].as_bundle().is_some());
        assert_eq!(
            game.measure_prepared_item(&elements[0], &font),
            Some((96, 45))
        );
    });
}

#[test]
#[ignore = "daily"]
fn bundle_empty_contents_keeps_image() {
    if run_bundle_case("bundle_empty_contents_keeps_image") {
        return;
    }
    run(None, |game| {
        let stack = ItemStack::new("minecraft:bundle", 1);
        let lines = vec![(sunrise_game_data::text::Text::literal("Name"), u32::MAX)];
        let elements = game
            .gather_tooltip_components(&stack, lines, 10, (320.0, 240.0))
            .unwrap_ready();
        let font = EngineTooltipFont::new(&game.client.gui.atlas.font, &DEFAULT_TOOLTIP_FONT_ID);
        // Vanilla maps the present bundle-contents component even when empty.
        // Empty description: two 9px lines + 13px bar + 8px gap = 39px.
        assert_eq!(elements.len(), 2);
        assert!(elements[0].as_text().is_some());
        assert!(elements[1].as_bundle().is_some());
        assert_eq!(
            game.measure_prepared_item(&elements[1], &font),
            Some((96, 39))
        );
        let batch = draw(game, &stack);
        grid(game, &batch, 20.0, false);
        // Content top 8 + two text lines 20 + image 39 + outer extent 4.
        assert_eq!(bottom(&batch), 71.0);
    });
}

struct Boundary {
    insert: bool,
    observed: std::sync::Arc<std::sync::Mutex<Vec<TooltipElement>>>,
}
#[mod_api::__subscriptions]
impl Hooks for Boundary {
    fn render_tooltip_gather_components(
        &self,
        _: &dyn ClientHost,
        event: &mut RenderTooltipGatherComponentsEvent<'_>,
    ) -> mod_api::Flow {
        if self.insert {
            event.elements.insert(
                0,
                TooltipElement::Text(mod_api::Text::literal("Inserted"), u32::MAX),
            );
        } else {
            let image = event.elements.remove(1);
            event.elements.push(image);
        }
        mod_api::Flow::Continue
    }

    fn render_tooltip_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut mod_api::RenderTooltipPreEvent<'_>,
    ) -> mod_api::Flow {
        *self.observed.lock().unwrap() = event
            .components
            .iter()
            .filter_map(|component| {
                if let Some((text, color)) = component.as_text() {
                    Some(TooltipElement::Text(text.clone(), color))
                } else {
                    component.as_bundle().cloned().map(|stack| {
                        TooltipElement::Component(mod_api::TooltipComponent::bundle(stack))
                    })
                }
            })
            .collect();
        mod_api::Flow::Continue
    }
}

fn boundary(insert: bool) {
    let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "bundle-boundary",
        Box::new(Boundary {
            insert,
            observed: observed.clone(),
        }),
    )]
    .into();
    run_mods(mods, |game| {
        let stack = bundle();
        let lines = game.stack_tooltip_lines(&stack, false, false);
        assert_eq!(lines.len(), 1);
        let name = TooltipElement::Text(lines[0].0.try_into_api().unwrap(), lines[0].1);
        let rest = TooltipElement::Text(mod_api::Text::literal("Rest"), u32::MAX);
        let image = TooltipElement::Component(mod_api::TooltipComponent::bundle(
            stack.try_into_api().unwrap(),
        ));
        let expected = if insert {
            vec![
                TooltipElement::Text(mod_api::Text::literal("Inserted"), u32::MAX),
                name,
                image,
                rest,
            ]
        } else {
            vec![name, rest, image]
        };
        let batch = draw(game, &stack);
        assert!(*observed.lock().unwrap() == expected);
        // Top 8 + first line 10 + first-element gap 2 + second line 10 = 30.
        // Inserting before the name transfers the gap to the inserted line.
        grid(game, &batch, 30.0, true);
        grid(game, &batch, 20.0, false);
    });
}

#[test]
#[ignore = "daily"]
fn bundle_listener_inserts_before_name() {
    boundary(true);
}

#[test]
#[ignore = "daily"]
fn bundle_pre_observes_gather_reordered_list() {
    boundary(false);
}

struct ClearLines(std::sync::Arc<std::sync::atomic::AtomicBool>);
#[mod_api::__subscriptions]
impl Hooks for ClearLines {
    fn item_tooltip(&self, _: &dyn ClientHost, event: &mut mod_api::ItemTooltipEvent<'_>) {
        event.lines.clear();
    }

    fn render_tooltip_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut mod_api::RenderTooltipPreEvent<'_>,
    ) -> mod_api::Flow {
        assert_eq!(event.components.len(), 1);
        assert!(event.components[0].as_bundle().is_some());
        let size = event.draw.with_tooltip_font(event.font, |font| {
            (
                event.components[0].width(font),
                event.components[0].height(font),
            )
        });
        assert_eq!(size, (Some(96), Some(45)));
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn bundle_cleared_lines_still_draws_image_at_content_top() {
    let observed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mods: Mods = vec![ModEntry::new(
        "clear-lines",
        Box::new(ClearLines(observed.clone())),
    )]
    .into();
    run_mods(mods, |game| {
        let mut batch = HudBatch::scratch([320, 240]);
        game.draw_stack_tooltip_with(&mut batch, &bundle(), false, 10.0, 20.0, vec![]);
        // Positioner: cursor y 20 - 12 = content top 8. No name or gap above the image.
        grid(game, &batch, 8.0, true);
        assert!(observed.load(std::sync::atomic::Ordering::Relaxed));
    });
}

#[test]
#[ignore = "daily"]
fn bundle_hidden_tooltip_still_draws_nothing() {
    run(None, |game| {
        let mut stack = bundle();
        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(sunrise_core::nbt::Tag::compound([(
                "hide_tooltip",
                sunrise_core::nbt::Tag::Byte(1),
            )])),
        );
        // Also pins that the supplied after-name text cannot revive a hidden tooltip.
        let batch = draw(game, &stack);
        assert!(batch.vertices.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn custom_bundle_item_part_draws_tooltip_image_and_bar() {
    use mod_api::{
        __private::game::RegistrationContextNew, BundleConfig, ClientTooltipComponent,
        ComponentTypesEvent, DrawContext, ItemBarEvent, ItemPart,
        RegisterClientTooltipComponentFactoriesEvent, Registrar, RegistrationContext,
        TooltipComponent, TooltipComponentData, TooltipFont, TooltipImageEvent,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_client::tooltip_components::{TooltipComponentFactories, with_test_factories};
    use sunrise_core::nbt::Tag;
    use sunrise_items::bundle::{BundleSpec, CustomBundleContents};

    const CUSTOM_COMPONENT: &str = "pouch:contents";

    /// The pouch's tooltip data. `tooltip_image` reads the contents and metrics through the host
    /// once, as `BundleItem.getTooltipImage` hands a `BundleTooltip` its contents, and the client
    /// component below draws from this data alone.
    struct PouchTooltip {
        rows: usize,
        fullness: f32,
        is_full: bool,
    }
    impl TooltipComponentData for PouchTooltip {}

    /// A `ClientBundleTooltip`-shaped component: a 96 pixel wide grid and a 13 pixel bar.
    struct ClientPouchTooltip {
        rows: usize,
        fullness: f32,
        is_full: bool,
        drawn: Arc<AtomicUsize>,
        seen_fullness: Arc<Mutex<Vec<f32>>>,
    }
    impl ClientTooltipComponent for ClientPouchTooltip {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            self.rows as i32 * 24 + 13 + 8
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            96
        }
        fn draw_image(
            &self,
            _: &dyn TooltipFont,
            x: i32,
            y: i32,
            _: i32,
            _: i32,
            draw: &mut dyn DrawContext,
        ) {
            self.seen_fullness.lock().unwrap().push(self.fullness);
            let fill = (self.fullness * 94.0).trunc().clamp(0.0, 94.0);
            let color = if self.is_full { 0xFFFF5454 } else { 0xFF7087FF };
            draw.fill([x as f32, y as f32, fill, 13.0], color);
            self.drawn.fetch_add(1, Ordering::Relaxed);
        }
    }

    struct CustomBundleUi {
        config: BundleConfig,
    }
    #[mod_api::sunrise_part]
    impl ItemPart for CustomBundleUi {
        fn item_bar(&self, host: &dyn ClientHost, event: &mut ItemBarEvent<'_>) {
            if let Ok(Some(metrics)) = self.config.metrics(host, event.stack) {
                *event.bar = metrics.bar;
            }
        }
        fn tooltip_image(
            &self,
            host: &dyn ClientHost,
            event: &mut TooltipImageEvent<'_>,
        ) -> Option<TooltipComponent> {
            let slots = self.config.slots(host, event.stack).ok()??;
            let metrics = self.config.metrics(host, event.stack).ok()??;
            Some(TooltipComponent::custom(PouchTooltip {
                rows: slots.len().min(12).div_ceil(4).max(1),
                fullness: metrics.fullness,
                is_full: metrics.is_full,
            }))
        }
    }

    struct CustomBundleUiMod {
        config: BundleConfig,
        drawn: Arc<AtomicUsize>,
        seen_fullness: Arc<Mutex<Vec<f32>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for CustomBundleUiMod {
        fn component_types(&self, _: &RegistrationContext, event: &mut ComponentTypesEvent<'_>) {
            event.register(CUSTOM_COMPONENT.into());
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let key = r.item_key("minecraft:paper");
            r.item_part(
                key,
                CustomBundleUi {
                    config: self.config.clone(),
                },
            );
        }
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let (drawn, seen_fullness) = (self.drawn.clone(), self.seen_fullness.clone());
            event
                .factories
                .register::<PouchTooltip, ClientPouchTooltip>(move |data| ClientPouchTooltip {
                    rows: data.rows,
                    fullness: data.fullness,
                    is_full: data.is_full,
                    drawn: drawn.clone(),
                    seen_fullness: seen_fullness.clone(),
                });
        }
    }

    let config = BundleConfig::custom(CUSTOM_COMPONENT, 4);
    let drawn = Arc::new(AtomicUsize::new(0));
    let seen_fullness = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "custom-bundle-ui",
        Box::new(CustomBundleUiMod {
            config: config.clone(),
            drawn: drawn.clone(),
            seen_fullness: seen_fullness.clone(),
        }),
    )]
    .into();
    run_mods(mods, |game| {
        // The client's tooltip component factories, as client startup collects them.
        let mut factories = TooltipComponentFactories::default();
        sunrise_client_gameplay::game::client_side::native_hooks_client::register_client_tooltip_component_factories::native_register_client_tooltip_component_factories(
            &RegistrationContext::new(),
            &mut factories,
        );
        with_test_factories(factories, || {
            let spec = BundleSpec::custom(CUSTOM_COMPONENT, 4);

            // Empty custom pouch: no fullness bar.
            let mut empty = ItemStack::new("minecraft:paper", 1);
            empty
                .data_mut()
                .set_other(CUSTOM_COMPONENT, Some(Tag::List(Vec::new())));
            let mut bar = None;
            game.native_item_bar(&empty, &mut bar);
            assert_eq!(bar, None);

            // Half-full custom pouch (128/256 stones): blue bar width 7 and custom tooltip image.
            let mut half = empty.clone();
            let mut contents = CustomBundleContents::of(game.items(), &half, spec).unwrap();
            let mut stones = ItemStack::new("minecraft:stone", 128);
            assert_eq!(contents.try_insert(game.items(), &mut stones, true), 128);
            contents.store(game.items(), &mut half);
            game.native_item_bar(&half, &mut bar);
            assert_eq!(bar, Some((7, 0x7087FF)));

            let batch = draw(game, &half);
            assert_eq!(drawn.load(Ordering::Relaxed), 1);
            assert_eq!(*seen_fullness.lock().unwrap(), vec![0.5]);
            assert!(
                batch
                    .vertices
                    .iter()
                    .any(|v| v.color == rgba(0x70, 0x87, 0xFF, 0xFF))
            );

            // Full custom pouch (256/256 stones): red bar width 13.
            let mut full = half.clone();
            let mut contents = CustomBundleContents::of(game.items(), &full, spec).unwrap();
            let mut more = ItemStack::new("minecraft:stone", 128);
            assert_eq!(contents.try_insert(game.items(), &mut more, true), 128);
            contents.store(game.items(), &mut full);
            game.native_item_bar(&full, &mut bar);
            assert_eq!(bar, Some((13, 0xFF5454)));
        });
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;
