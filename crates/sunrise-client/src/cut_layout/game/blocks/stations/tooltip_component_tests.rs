//! Unit tests for typed client tooltip components: registration & override, NeoForge wrapping
//! fixtures A–K, two-pass draw order & geometry (including audit [87] zero-sized components),
//! `render_tooltip_pre` font replacement & cancellation, `render_tooltip_texture` & line-only
//! `ItemResource::tooltip`, `show_tooltip_with_item_in_hand`, audit [88], and panic containment.
use mod_api::DefaultTooltipPositioner;
use mod_api::{
    __private::game::RegistrationContextNew, ClientHost, ClientTooltipComponent, DrawContext, Flow,
    Hooks, Identifier, ItemPart, ItemResource, ModEntry, Mods,
    RegisterClientTooltipComponentFactoriesEvent, Registrar, RegistrationContext,
    RenderTooltipGatherComponentsEvent, RenderTooltipPreEvent, RenderTooltipTextureEvent,
    TooltipComponent, TooltipComponentData, TooltipElement, TooltipFont, TooltipImageEvent,
};
use std::borrow::Cow;
use std::cell::Cell;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicUsize, Ordering},
};
use sunrise_client::hud::HudBatch;
use sunrise_client::hud::rgba;
use sunrise_client::tooltip_components::TooltipComponentPrepareError;
use sunrise_client::tooltip_components::{TooltipComponentFactories, with_test_factories};
use sunrise_client_gameplay::game::client_item_extensions::DEFAULT_TOOLTIP_FONT_ID;
use sunrise_client_gameplay::game::client_side::stations::tooltip::{
    GatheredTooltip, PreparedTooltipItem, WidgetTooltipPositioner, quantize_tooltip_anchor,
};
use sunrise_client_gameplay::game::host::draw::EngineTooltipFont;
use sunrise_gameplay_common::game::Screen;
use sunrise_gameplay_common::game::TryIntoApi;
use sunrise_items::items::ItemStack;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

fn collect_factories() -> TooltipComponentFactories {
    let mut factories = TooltipComponentFactories::default();
    sunrise_client_gameplay::game::client_side::native_hooks_client::register_client_tooltip_component_factories::native_register_client_tooltip_component_factories(
        &RegistrationContext::new(),
        &mut factories,
    );
    factories
}

#[test]
fn a_panicking_factory_registration_publishes_none_of_its_edits() {
    struct Data;
    impl TooltipComponentData for Data {}
    struct Unpublished;
    impl TooltipComponentData for Unpublished {}

    struct Component(i32);
    impl ClientTooltipComponent for Component {
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            self.0
        }
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            0
        }
    }

    struct Register(i32);
    #[mod_api::__subscriptions]
    impl Hooks for Register {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let value = self.0;
            event
                .factories
                .register::<Data, Component>(move |_| Component(value));
            if value == 2 {
                event
                    .factories
                    .register::<Unpublished, Component>(|_| Component(3));
                panic!("failed tooltip factory registration");
            }
        }
    }

    use sunrise_client::tooltip_components::{TooltipComponentPrepareError, create};
    let mods = vec![
        ModEntry::new("first", Box::new(Register(1))),
        ModEntry::new("failing", Box::new(Register(2))),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        with_test_factories(collect_factories(), || {
            let original = create(&TooltipComponent::custom(Data)).unwrap();
            assert_eq!(original.downcast_ref::<Component>().unwrap().0, 1);
            assert!(matches!(
                create(&TooltipComponent::custom(Unpublished)),
                Err(TooltipComponentPrepareError::MissingFactory { .. })
            ));
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "failing",
                    hook: "register_client_tooltip_component_factories",
                }]
            );
        });
    });
}

const TOOLTIP_CHILD: &str = "SUNRISE_TOOLTIP_COMPONENT_CASE";

fn run_tooltip_cases(test: &str, cases: &[&str]) -> bool {
    run_fresh_cases(
        &format!("cut_layout::game::client_side::stations::tooltip::component_tests::{test}"),
        TOOLTIP_CHILD,
        cases,
    )
}

fn with_mod_factories(
    mods: Mods,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let factories = collect_factories();
        with_test_factories(factories, || {
            with_client_game(test);
        });
    });
}

fn populated_bundle() -> ItemStack {
    let mut stack = ItemStack::new("minecraft:bundle", 1);
    stack.data_mut().bundle_contents = vec![ItemStack::new("minecraft:stone", 1)];
    stack
}

fn text_lines(components: &[PreparedTooltipItem]) -> Vec<String> {
    components
        .iter()
        .filter_map(|component| component.as_text().map(|(text, _)| text.plain()))
        .collect()
}

/// Pack-independent ASCII metrics: each lowercase letter and the space have the given advance.
fn synthetic_ascii_fonts<const N: usize>(fonts: [(&str, i32); N]) -> sunrise_client::font::Font {
    use sunrise_assets::resource_source::{Resource, ResourceSource};
    let files = fonts.into_iter().map(|(name, advance)| {
        let advances: std::collections::BTreeMap<_, _> = ('a'..='z')
            .chain([' '])
            .map(|c| (c.to_string(), advance))
            .collect();
        let definition =
            serde_json::json!({"providers": [{"type": "space", "advances": advances}]});
        (
            format!("assets/minecraft/font/{name}.json"),
            Resource::bytes(definition.to_string().into_bytes()),
        )
    });
    sunrise_client::font::Font::metrics(&ResourceSource::indexed(files.collect()))
}

/// Covers NeoForge wrapping fixtures A–K end-to-end through `gather_tooltip_components`,
/// factory registration & last-wins override, and graceful omission of unregistered components.
#[test]
#[ignore = "daily"]
fn gather_wrapping_fixtures_preserve_empty_lines_and_exclude_components() {
    if run_tooltip_cases(
        "gather_wrapping_fixtures_preserve_empty_lines_and_exclude_components",
        &["tooltip"],
    ) {
        return;
    }
    struct WideData(i32, i32);
    impl TooltipComponentData for WideData {}

    struct UnregisteredData;
    impl TooltipComponentData for UnregisteredData {}

    struct WideClient(i32, i32);
    impl ClientTooltipComponent for WideClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            self.1
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            self.0
        }
    }

    struct FirstFactory;
    #[mod_api::__subscriptions]
    impl Hooks for FirstFactory {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            event
                .factories
                .register::<WideData, WideClient>(|d| WideClient(d.0, d.1));
        }
    }

    struct OverrideFactory;
    #[mod_api::__subscriptions]
    impl Hooks for OverrideFactory {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            // Last registration wins for the same TooltipComponentData type.
            event
                .factories
                .register::<WideData, WideClient>(|d| WideClient(d.0 * 2, d.1));
        }
    }

    struct GatherConfig {
        max_width: AtomicI32,
        mode: AtomicU8,
        bundle: Mutex<Option<mod_api::ItemStack>>,
    }

    struct GatherMod(Arc<GatherConfig>);
    #[mod_api::__subscriptions]
    impl Hooks for GatherMod {
        fn render_tooltip_gather_components(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipGatherComponentsEvent<'_>,
        ) -> Flow {
            event.max_width = self.0.max_width.load(Ordering::Relaxed);
            match self.0.mode.load(Ordering::Relaxed) {
                1 => {
                    // Fixtures I & J: empty line + custom component + bundle + tail.
                    let bundle_stack = self.0.bundle.lock().unwrap().clone().unwrap();
                    event
                        .elements
                        .push(TooltipElement::Text(mod_api::Text::literal(""), u32::MAX));
                    event
                        .elements
                        .push(TooltipElement::Component(TooltipComponent::custom(
                            WideData(250, 12),
                        )));
                    event
                        .elements
                        .push(TooltipElement::Component(TooltipComponent::bundle(
                            bundle_stack,
                        )));
                    event.elements.push(TooltipElement::Text(
                        mod_api::Text::literal("tail"),
                        u32::MAX,
                    ));
                }
                2 => {
                    // Fixture K: wide custom (500px) & bundle (96px) beside short text.
                    let bundle_stack = self.0.bundle.lock().unwrap().clone().unwrap();
                    event
                        .elements
                        .push(TooltipElement::Component(TooltipComponent::custom(
                            WideData(250, 12),
                        )));
                    event
                        .elements
                        .push(TooltipElement::Component(TooltipComponent::bundle(
                            bundle_stack,
                        )));
                }
                3 => {
                    // Unregistered custom component aborts preparation with MissingFactory.
                    event
                        .elements
                        .push(TooltipElement::Component(TooltipComponent::custom(
                            UnregisteredData,
                        )));
                }
                _ => {}
            }
            Flow::Continue
        }
    }

    let config = Arc::new(GatherConfig {
        max_width: AtomicI32::new(-1),
        mode: AtomicU8::new(0),
        bundle: Mutex::new(None),
    });
    let mods: Mods = vec![
        ModEntry::new("first_factory", Box::new(FirstFactory)),
        ModEntry::new("override_factory", Box::new(OverrideFactory)),
        ModEntry::new("gather_mod", Box::new(GatherMod(config.clone()))),
    ]
    .into();

    with_mod_factories(mods, |game| {
        let stick = ItemStack::new("minecraft:stick", 1);
        *config.bundle.lock().unwrap() = Some(populated_bundle().try_into_api().unwrap());
        game.client.gui.atlas.font = synthetic_ascii_fonts([("default", 5)]);
        let font = &game.client.gui.atlas.font;
        let long_text = sunrise_game_data::text::Text::literal(
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron",
        );
        const RIGHT_WRAP: [&str; 3] = [
            "alpha beta gamma delta epsilon",
            "zeta eta theta iota kappa lambda",
            "mu nu xi omicron",
        ];

        // Fixtures A & B: fits right and right-edge flip without wrapping.
        let short = sunrise_game_data::text::Text::literal("alpha beta");
        for mouse_x in [10, 260] {
            let gathered = game
                .gather_tooltip_components(
                    &stick,
                    vec![(short.clone(), u32::MAX)],
                    mouse_x,
                    (320.0, 240.0),
                )
                .unwrap_ready();
            assert_eq!(text_lines(&gathered), ["alpha beta"]);
        }

        // Fixture C: right-half narrow screen wrap (mouse_x > screen_width / 2 -> 180 - 12 - 8 = 160).
        let right_wrap = game
            .gather_tooltip_components(
                &stick,
                vec![(long_text.clone(), u32::MAX)],
                180,
                (320.0, 240.0),
            )
            .unwrap_ready();
        assert_eq!(text_lines(&right_wrap), RIGHT_WRAP);

        // Fixture D: left-half narrow screen wrap (mouse_x < screen_width / 2 -> 320 - 16 - 100 = 204).
        let left_wrap = game
            .gather_tooltip_components(
                &stick,
                vec![(long_text.clone(), u32::MAX)],
                100,
                (320.0, 240.0),
            )
            .unwrap_ready();
        assert_eq!(
            text_lines(&left_wrap),
            [
                "alpha beta gamma delta epsilon zeta eta",
                "theta iota kappa lambda mu nu xi omicron",
            ]
        );

        // Fixture E: exact midpoint (mouse_x == screen_width / 2 -> 320 - 16 - 160 = 144).
        let mid_wrap = game
            .gather_tooltip_components(
                &stick,
                vec![(long_text.clone(), u32::MAX)],
                160,
                (320.0, 240.0),
            )
            .unwrap_ready();
        assert_eq!(
            text_lines(&mid_wrap),
            [
                "alpha beta gamma delta",
                "epsilon zeta eta theta iota",
                "kappa lambda mu nu xi",
                "omicron"
            ]
        );

        // Fixture F: explicit max_width wraps a tooltip that otherwise fits.
        config.max_width.store(50, Ordering::Relaxed);
        let max_wrap = game
            .gather_tooltip_components(
                &stick,
                vec![(
                    sunrise_game_data::text::Text::literal("alpha beta gamma delta"),
                    u32::MAX,
                )],
                10,
                (320.0, 240.0),
            )
            .unwrap_ready();
        assert_eq!(text_lines(&max_wrap), ["alpha beta", "gamma", "delta"]);

        // Fixture G: max_width tighter than screen wrap (80 < 160) wins.
        config.max_width.store(80, Ordering::Relaxed);
        let tighter = game
            .gather_tooltip_components(
                &stick,
                vec![(long_text.clone(), u32::MAX)],
                180,
                (320.0, 240.0),
            )
            .unwrap_ready();
        assert_eq!(
            text_lines(&tighter),
            [
                "alpha beta gamma",
                "delta epsilon",
                "zeta eta theta",
                "iota kappa",
                "lambda mu nu xi",
                "omicron"
            ]
        );

        // Fixture H: max_width wider than screen wrap (200 > 160) keeps screen wrap width.
        config.max_width.store(200, Ordering::Relaxed);
        let wider = game
            .gather_tooltip_components(
                &stick,
                vec![(long_text.clone(), u32::MAX)],
                180,
                (320.0, 240.0),
            )
            .unwrap_ready();
        assert_eq!(text_lines(&wider), RIGHT_WRAP);

        // Fixtures I & J: empty line preserved, custom override wins, bundle kept.
        config.max_width.store(60, Ordering::Relaxed);
        config.mode.store(1, Ordering::Relaxed);
        let mixed = game
            .gather_tooltip_components(
                &stick,
                vec![(long_text.clone(), u32::MAX)],
                10,
                (320.0, 240.0),
            )
            .unwrap_ready();
        let n = 7;
        assert_eq!(mixed.len(), 11);
        assert_eq!(
            text_lines(&mixed[..n]),
            [
                "alpha beta",
                "gamma delta",
                "epsilon zeta",
                "eta theta",
                "iota kappa",
                "lambda mu nu",
                "xi omicron",
            ]
        );
        // Empty text line preserved as a single empty text component right after wrapped text.
        assert_eq!(text_lines(&mixed[n..=n]), [""]);
        // Override factory won: WideData(250, 12) -> WideClient(500, 12).
        let tooltip_font = EngineTooltipFont::new(font, &DEFAULT_TOOLTIP_FONT_ID);
        let wide = mixed[n + 1]
            .downcast_ref::<WideClient>()
            .expect("override factory should prepare WideClient");
        assert_eq!((wide.0, wide.1), (500, 12));
        assert_eq!(
            game.measure_prepared_item(&mixed[n + 1], &tooltip_font),
            Some((500, 12))
        );
        // Bundle component kept in order.
        assert!(mixed[n + 2].as_bundle().is_some());
        assert_eq!(text_lines(&mixed[n + 3..]), ["tail"]);

        // Fixture K: wide custom (500px) and bundle (96px) excluded from tooltipTextWidth.
        config.max_width.store(-1, Ordering::Relaxed);
        config.mode.store(2, Ordering::Relaxed);
        let excluded = game
            .gather_tooltip_components(
                &stick,
                vec![(sunrise_game_data::text::Text::literal("short"), u32::MAX)],
                10,
                (100.0, 240.0),
            )
            .unwrap_ready();
        assert_eq!(excluded.len(), 3);
        assert_eq!(text_lines(&excluded[..1]), ["short"]);
        assert!(excluded[1].downcast_ref::<WideClient>().is_some());
        assert!(excluded[2].as_bundle().is_some());

        // Unregistered custom component aborts preparation with MissingFactory.
        config.mode.store(3, Ordering::Relaxed);
        assert!(matches!(
            game.gather_tooltip_components(
                &stick,
                vec![(sunrise_game_data::text::Text::literal("short"), u32::MAX)],
                10,
                (320.0, 240.0),
            ),
            GatheredTooltip::PrepareError(TooltipComponentPrepareError::MissingFactory {
                owner: Some("gather_mod"),
                ..
            })
        ));
    });
}

/// Verifies two-pass draw order (all `draw_text` before any `draw_image`), exact `(x, local_y, w, h)`
/// geometry (including `+2` after index 0 when `len > 1` and `-2` start when `len == 1`), and
/// retention of zero-width and zero-height components (audit [87]).
#[test]
#[ignore = "daily"]
fn two_pass_draw_order_geometry_and_zero_sized_components() {
    if run_tooltip_cases(
        "two_pass_draw_order_geometry_and_zero_sized_components",
        &["tooltip"],
    ) {
        return;
    }
    struct ZeroWidthData;
    impl TooltipComponentData for ZeroWidthData {}
    struct ZeroHeightData;
    impl TooltipComponentData for ZeroHeightData {}
    struct PanelData;
    impl TooltipComponentData for PanelData {}
    struct SingleData;
    impl TooltipComponentData for SingleData {}

    struct RecordingClient {
        tag: &'static str,
        w: i32,
        h: Cell<i32>,
        log: Arc<Mutex<Vec<String>>>,
    }
    impl ClientTooltipComponent for RecordingClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            self.h.get()
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            self.w
        }
        fn draw_text(&self, _: &dyn TooltipFont, x: i32, y: i32, _: &mut dyn DrawContext) {
            if self.tag == "zero_w" {
                self.h.set(10);
            }
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:text({x},{y})", self.tag));
        }
        fn draw_image(
            &self,
            _: &dyn TooltipFont,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            _: &mut dyn DrawContext,
        ) {
            if self.tag == "zero_w" {
                self.h.set(17);
            }
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:image({x},{y},{width},{height})", self.tag));
        }
    }

    struct FactoryMod(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for FactoryMod {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let log = self.0.clone();
            event
                .factories
                .register::<ZeroWidthData, RecordingClient>(move |_| RecordingClient {
                    tag: "zero_w",
                    w: 0,
                    h: Cell::new(6),
                    log: log.clone(),
                });
            let log = self.0.clone();
            event
                .factories
                .register::<ZeroHeightData, RecordingClient>(move |_| RecordingClient {
                    tag: "zero_h",
                    w: 12,
                    h: Cell::new(0),
                    log: log.clone(),
                });
            let log = self.0.clone();
            event
                .factories
                .register::<PanelData, RecordingClient>(move |_| RecordingClient {
                    tag: "panel",
                    w: 80,
                    h: Cell::new(16),
                    log: log.clone(),
                });
            let log = self.0.clone();
            event
                .factories
                .register::<SingleData, RecordingClient>(move |_| RecordingClient {
                    tag: "single",
                    w: 40,
                    h: Cell::new(16),
                    log: log.clone(),
                });
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "order_mod",
        Box::new(FactoryMod(log.clone())),
    )]
    .into();

    with_mod_factories(mods, |game| {
        let mut components = vec![
            PreparedTooltipItem::Text {
                text: sunrise_game_data::text::Text::literal("A"),
                color: u32::MAX,
            },
            game.prepare_tooltip_component(&TooltipComponent::custom(ZeroWidthData))
                .unwrap(),
            game.prepare_tooltip_component(&TooltipComponent::custom(ZeroHeightData))
                .unwrap(),
            game.prepare_tooltip_component(&TooltipComponent::custom(PanelData))
                .unwrap(),
        ];
        let mut batch = HudBatch::scratch([320, 240]);
        let rect = game.draw_prepared_tooltip(
            &mut batch,
            None,
            &mut components,
            [10, 20],
            &DefaultTooltipPositioner,
            true,
        );
        // x = 10 + 12 = 22, y = 20 - 12 = 8, w = 80, h = 0 + 10 + 6 + 0 + 16 = 32.
        assert_eq!(rect, Some([22.0, 8.0, 80.0, 32.0]));
        // The box keeps the original height, but the next component is placed using
        // the zero-width component's new height after each draw callback.
        assert_eq!(
            std::mem::take(&mut *log.lock().unwrap()),
            [
                "zero_w:text(22,20)",
                "zero_h:text(22,30)",
                "panel:text(22,30)",
                "zero_w:image(22,20,80,32)",
                "zero_h:image(22,37,80,32)",
                "panel:image(22,37,80,32)",
            ]
        );

        // Single-component tooltip: height starts at -2 (16 - 2 = 14) with no +2 gap.
        let mut single = vec![
            game.prepare_tooltip_component(&TooltipComponent::custom(SingleData))
                .unwrap(),
        ];
        let mut single_batch = HudBatch::scratch([320, 240]);
        let single_rect = game.draw_prepared_tooltip(
            &mut single_batch,
            None,
            &mut single,
            [10, 20],
            &DefaultTooltipPositioner,
            true,
        );
        assert_eq!(single_rect, Some([22.0, 8.0, 40.0, 14.0]));
        assert_eq!(
            std::mem::take(&mut *log.lock().unwrap()),
            ["single:text(22,8)", "single:image(22,8,40,14)"]
        );
    });
}

/// Verifies `render_tooltip_pre`: inspects prepared components (`as_text`, `as_bundle`,
/// `downcast_ref`), checks `positioner`, moves `place`, replaces `font` without rewrapping, and
/// cancels drawing while keeping custom draws submitted in `Pre`.
#[test]
#[ignore = "daily"]
fn render_tooltip_pre_observes_components_replaces_font_without_rewrap_and_cancels() {
    if run_tooltip_cases(
        "render_tooltip_pre_observes_components_replaces_font_without_rewrap_and_cancels",
        &["tooltip"],
    ) {
        return;
    }
    struct FontAwareData;
    impl TooltipComponentData for FontAwareData {}

    struct FontState {
        fonts_seen: Mutex<Vec<String>>,
        draws: AtomicUsize,
        pre_mode: AtomicU8,
        pre_observed_texts: AtomicUsize,
        pre_saw_bundle: AtomicBool,
        pre_saw_custom: AtomicBool,
        bundle_stack: Mutex<Option<mod_api::ItemStack>>,
    }

    struct FontAwareClient(Arc<FontState>);
    impl ClientTooltipComponent for FontAwareClient {
        fn height(&self, font: &dyn TooltipFont) -> i32 {
            self.0
                .fonts_seen
                .lock()
                .unwrap()
                .push(format!("h:{}", font.id()));
            12
        }
        fn width(&self, font: &dyn TooltipFont) -> i32 {
            self.0
                .fonts_seen
                .lock()
                .unwrap()
                .push(format!("w:{}", font.id()));
            if font.id().path() == "alt" { 130 } else { 30 }
        }
        fn draw_text(&self, font: &dyn TooltipFont, _: i32, _: i32, _: &mut dyn DrawContext) {
            self.0
                .fonts_seen
                .lock()
                .unwrap()
                .push(format!("t:{}", font.id()));
        }
        fn draw_image(
            &self,
            font: &dyn TooltipFont,
            x: i32,
            y: i32,
            _: i32,
            _: i32,
            draw: &mut dyn DrawContext,
        ) {
            self.0
                .fonts_seen
                .lock()
                .unwrap()
                .push(format!("i:{}", font.id()));
            self.0.draws.fetch_add(1, Ordering::Relaxed);
            draw.fill([x as f32, y as f32, 6.0, 6.0], 0xFF11_2233);
        }
    }

    struct PreMod(Arc<FontState>);
    #[mod_api::__subscriptions]
    impl Hooks for PreMod {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let state = self.0.clone();
            event
                .factories
                .register::<FontAwareData, FontAwareClient>(move |_| {
                    FontAwareClient(state.clone())
                });
        }
        fn render_tooltip_gather_components(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipGatherComponentsEvent<'_>,
        ) -> Flow {
            event.max_width = 55;
            let bundle = self.0.bundle_stack.lock().unwrap().clone().unwrap();
            event
                .elements
                .push(TooltipElement::Component(TooltipComponent::bundle(bundle)));
            event
                .elements
                .push(TooltipElement::Component(TooltipComponent::custom(
                    FontAwareData,
                )));
            Flow::Continue
        }
        fn render_tooltip_pre(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipPreEvent<'_>,
        ) -> Flow {
            assert_eq!(
                event
                    .positioner
                    .position_tooltip([320, 240], [10, 20], [50, 20]),
                [22, 8]
            );
            let texts = event
                .components
                .iter()
                .filter(|c| c.as_text().is_some())
                .count();
            let has_bundle = event.components.iter().any(|c| c.as_bundle().is_some());
            let has_custom = event
                .components
                .iter()
                .any(|c| c.downcast_ref::<FontAwareClient>().is_some());
            self.0.pre_observed_texts.store(texts, Ordering::Relaxed);
            self.0.pre_saw_bundle.store(has_bundle, Ordering::Relaxed);
            self.0.pre_saw_custom.store(has_custom, Ordering::Relaxed);

            match self.0.pre_mode.load(Ordering::Relaxed) {
                1 => {
                    event.place.x = 40;
                    event.place.y = 50;
                    *event.font = Identifier::parse("minecraft:alt").unwrap();
                    Flow::Continue
                }
                2 => {
                    event.draw.fill([5.0, 5.0, 10.0, 10.0], 0xFFAA_BBCC);
                    Flow::Cancel
                }
                _ => Flow::Continue,
            }
        }
    }

    let state = Arc::new(FontState {
        fonts_seen: Mutex::new(Vec::new()),
        draws: AtomicUsize::new(0),
        pre_mode: AtomicU8::new(1),
        pre_observed_texts: AtomicUsize::new(0),
        pre_saw_bundle: AtomicBool::new(false),
        pre_saw_custom: AtomicBool::new(false),
        bundle_stack: Mutex::new(None),
    });
    let mods: Mods = vec![ModEntry::new("pre_mod", Box::new(PreMod(state.clone())))].into();

    with_mod_factories(mods, |game| {
        let stick = ItemStack::new("minecraft:stick", 1);
        *state.bundle_stack.lock().unwrap() = Some(populated_bundle().try_into_api().unwrap());
        game.client.gui.atlas.font = synthetic_ascii_fonts([("default", 5), ("alt", 14)]);
        let line = sunrise_game_data::text::Text::literal("one two three four five six");

        let mut prepared = game
            .gather_tooltip_components(&stick, vec![(line, u32::MAX)], 10, (320.0, 240.0))
            .unwrap_ready();
        assert_eq!(prepared.len(), 5);
        assert_eq!(text_lines(&prepared), ["one two", "three four", "five six"]);

        // Mode 1: move place and switch font to minecraft:alt without rewrapping.
        let mut batch = HudBatch::scratch([320, 240]);
        let rect = game.draw_prepared_tooltip(
            &mut batch,
            Some(&stick),
            &mut prepared,
            [10, 20],
            &DefaultTooltipPositioner,
            true,
        );
        // "three four" is 140px in alt; the box has three text lines, the bundle and custom panel.
        assert_eq!(rect, Some([52.0, 38.0, 140.0, 87.0]));
        assert_eq!(state.pre_observed_texts.load(Ordering::Relaxed), 3);
        assert!(state.pre_saw_bundle.load(Ordering::Relaxed));
        assert!(state.pre_saw_custom.load(Ordering::Relaxed));
        assert_eq!(
            std::mem::take(&mut *state.fonts_seen.lock().unwrap()),
            [
                "w:minecraft:alt",
                "h:minecraft:alt",
                "t:minecraft:alt",
                "h:minecraft:alt",
                "i:minecraft:alt",
                "h:minecraft:alt",
            ]
        );
        assert_eq!(state.draws.load(Ordering::Relaxed), 1);
        assert!(
            batch
                .vertices
                .iter()
                .any(|v| v.color == rgba(0x11, 0x22, 0x33, 0xFF))
        );

        // Mode 2: cancel in Pre skips tooltip background and components, keeping Pre's custom draw.
        state.pre_mode.store(2, Ordering::Relaxed);
        let mut cancelled_batch = HudBatch::scratch([320, 240]);
        let cancelled = game.draw_prepared_tooltip(
            &mut cancelled_batch,
            Some(&stick),
            &mut prepared,
            [10, 20],
            &DefaultTooltipPositioner,
            true,
        );
        assert_eq!(cancelled, None);
        assert_eq!(state.draws.load(Ordering::Relaxed), 1);
        assert!(state.fonts_seen.lock().unwrap().is_empty());
        assert!(
            cancelled_batch
                .vertices
                .iter()
                .any(|v| v.color == rgba(0xAA, 0xBB, 0xCC, 0xFF)),
            "custom Pre draw must remain after cancel"
        );
        assert!(
            !cancelled_batch
                .vertices
                .iter()
                .any(|v| v.color == rgba(0x11, 0x22, 0x33, 0xFF)),
            "cancelled tooltip must not draw custom component"
        );
        assert!(
            !cancelled_batch
                .vertices
                .iter()
                .any(|v| v.color == rgba(0x10, 0x00, 0x10, 0xF0)),
            "cancelled tooltip must not draw vanilla background"
        );
    });
}

/// Verifies `render_tooltip_texture` still runs for tooltips with custom components and
/// `ItemResource::tooltip` still returns only `TooltipElement::Text` lines.
#[test]
#[ignore = "daily"]
fn tooltip_texture_and_item_resource_tooltip_with_custom_components() {
    if run_tooltip_cases(
        "tooltip_texture_and_item_resource_tooltip_with_custom_components",
        &["tooltip"],
    ) {
        return;
    }
    struct BadgeData;
    impl TooltipComponentData for BadgeData {}

    struct BadgeClient;
    impl ClientTooltipComponent for BadgeClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            12
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            24
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
            draw.fill([x as f32, y as f32, 8.0, 8.0], 0xFF33_4455);
        }
    }

    #[derive(Clone)]
    struct BadgePart;
    #[mod_api::sunrise_part]
    impl ItemPart for BadgePart {
        fn tooltip_image(
            &self,
            _: &dyn ClientHost,
            _: &mut TooltipImageEvent<'_>,
        ) -> Option<TooltipComponent> {
            Some(TooltipComponent::custom(BadgeData))
        }
    }

    struct TextureMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for TextureMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let stick = r.item_key("minecraft:stick");
            r.item_part(stick, BadgePart);
        }
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            event
                .factories
                .register::<BadgeData, BadgeClient>(|_| BadgeClient);
        }
        fn render_tooltip_texture(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipTextureEvent<'_>,
        ) {
            assert!(event.stack.is_some());
            self.0.fetch_add(1, Ordering::Relaxed);
            event.background = 0xFF22_6699;
        }
    }

    let texture_calls = Arc::new(AtomicUsize::new(0));
    let mods: Mods = vec![ModEntry::new(
        "texture_mod",
        Box::new(TextureMod(texture_calls.clone())),
    )]
    .into();

    with_mod_factories(mods, |game| {
        let stick = ItemStack::new("minecraft:stick", 1);
        let mut batch = HudBatch::scratch([320, 240]);
        game.draw_stack_tooltip(&mut batch, &stick, 10.0, 20.0);
        assert_eq!(texture_calls.load(Ordering::Relaxed), 1);
        assert!(
            batch
                .vertices
                .iter()
                .any(|v| v.color == rgba(0x22, 0x66, 0x99, 0xFF)),
            "render_tooltip_texture background must apply to custom-component tooltips"
        );
        assert!(
            batch
                .vertices
                .iter()
                .any(|v| v.color == rgba(0x33, 0x44, 0x55, 0xFF)),
            "custom component must draw alongside the textured box"
        );

        // ItemResource::tooltip stays line-only (TooltipElement::Text) for both custom and bundle items.
        for stack in [stick, populated_bundle()] {
            let resource = ItemResource::of(
                &sunrise_gameplay_common::game::api_values::test_snapshot(&stack),
            );
            let lines = resource.tooltip(game, false, false).unwrap();
            assert!(!lines.is_empty());
            assert!(
                lines
                    .iter()
                    .all(|element| matches!(element, TooltipElement::Text(..))),
                "ItemResource::tooltip must return only TooltipElement::Text: {lines:?}"
            );
        }
    });
}

/// Verifies `ClientTooltipComponent::show_tooltip_with_item_in_hand()` controls whether a hovered
/// slot tooltip stays visible while `game.carried` holds an item stack.
#[test]
#[ignore = "daily"]
fn show_tooltip_with_item_in_hand_controls_carried_visibility() {
    if run_tooltip_cases(
        "show_tooltip_with_item_in_hand_controls_carried_visibility",
        &["tooltip"],
    ) {
        return;
    }
    struct AlwaysShowData;
    impl TooltipComponentData for AlwaysShowData {}
    struct HideWithHandData;
    impl TooltipComponentData for HideWithHandData {}

    struct AlwaysShowClient;
    impl ClientTooltipComponent for AlwaysShowClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            20
        }
        fn show_tooltip_with_item_in_hand(&self) -> bool {
            true
        }
    }

    struct HideWithHandClient;
    impl ClientTooltipComponent for HideWithHandClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            20
        }
        fn show_tooltip_with_item_in_hand(&self) -> bool {
            false
        }
    }

    #[derive(Clone)]
    struct AlwaysShowPart;
    #[mod_api::sunrise_part]
    impl ItemPart for AlwaysShowPart {
        fn tooltip_image(
            &self,
            _: &dyn ClientHost,
            _: &mut TooltipImageEvent<'_>,
        ) -> Option<TooltipComponent> {
            Some(TooltipComponent::custom(AlwaysShowData))
        }
    }

    #[derive(Clone)]
    struct HideWithHandPart;
    #[mod_api::sunrise_part]
    impl ItemPart for HideWithHandPart {
        fn tooltip_image(
            &self,
            _: &dyn ClientHost,
            _: &mut TooltipImageEvent<'_>,
        ) -> Option<TooltipComponent> {
            Some(TooltipComponent::custom(HideWithHandData))
        }
    }

    struct HandMod;
    #[mod_api::__subscriptions]
    impl Hooks for HandMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let stick = r.item_key("minecraft:stick");
            r.item_part(stick, AlwaysShowPart);
            let apple = r.item_key("minecraft:apple");
            r.item_part(apple, HideWithHandPart);
        }
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            event
                .factories
                .register::<AlwaysShowData, AlwaysShowClient>(|_| AlwaysShowClient);
            event
                .factories
                .register::<HideWithHandData, HideWithHandClient>(|_| HideWithHandClient);
        }
    }

    let mods: Mods = vec![ModEntry::new("hand_mod", Box::new(HandMod))].into();
    with_mod_factories(mods, |game| {
        let stick = ItemStack::new("minecraft:stick", 1);
        let apple = ItemStack::new("minecraft:apple", 1);
        game.set_screen(Screen::Inventory);
        game.inventory[9] = Some(stick.clone());
        game.inventory[10] = Some(apple.clone());

        game.carried = None;
        assert!(game.tooltip_with_carried(&stick));
        assert!(game.tooltip_with_carried(&apple));

        game.carried = Some(ItemStack::new("minecraft:stone", 1));
        assert!(game.tooltip_with_carried(&stick));
        assert!(!game.tooltip_with_carried(&apple));

        let mut batch = HudBatch::scratch([512, 512]);
        let (ox, oy) = game.inventory_origin(&batch);

        let (sx, sy) = game.slot_position(9);
        game.client.cursor = (ox + sx + 8.0, oy + sy + 8.0);
        assert_eq!(
            game.slot_at(&batch, game.client.cursor.0, game.client.cursor.1),
            Some(9)
        );
        game.draw_slot_tooltip(&mut batch);
        assert!(
            !batch.vertices.is_empty(),
            "show_tooltip_with_item_in_hand = true keeps slot tooltip visible while carrying"
        );

        batch.vertices.clear();
        let (ax, ay) = game.slot_position(10);
        game.client.cursor = (ox + ax + 8.0, oy + ay + 8.0);
        assert_eq!(
            game.slot_at(&batch, game.client.cursor.0, game.client.cursor.1),
            Some(10)
        );
        game.draw_slot_tooltip(&mut batch);
        assert!(
            batch.vertices.is_empty(),
            "show_tooltip_with_item_in_hand = false hides slot tooltip while carrying"
        );
    });
}

/// Audit [88]: verifies no-listener fast path skips stack API conversion without cancelling,
/// conversion error in `render_tooltip_pre` skips only that hook without cancelling, and an
/// in-memory text deeper than the serialization cap still renders with its siblings.
#[test]
#[ignore = "daily"]
fn audit_88_no_listener_fast_path_and_deep_local_text() {
    if run_tooltip_cases(
        "audit_88_no_listener_fast_path_and_deep_local_text",
        &["tooltip"],
    ) {
        return;
    }
    // This one line is 198 pixels wide. Neither side of x=145 on a 160-pixel screen can fit
    // it, but vanilla's item tooltip still passes it to the positioner intact.
    const WIDE: &str = "long item name near a screen edge";
    let assert_unwrapped = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                            stack: &ItemStack| {
        let original_font = std::mem::replace(
            &mut game.client.gui.atlas.font,
            synthetic_ascii_fonts([("default", 6)]),
        );
        let gathered = game
            .gather_tooltip_components(
                stack,
                vec![(sunrise_game_data::text::Text::literal(WIDE), u32::MAX)],
                145,
                (160.0, 240.0),
            )
            .unwrap_ready();
        assert_eq!(text_lines(&gathered), [WIDE]);
        game.client.gui.atlas.font = original_font;
        gathered
    };
    let stick = ItemStack::new("minecraft:stick", 1);

    // Part 1: no listeners at all — an unconvertible stack still gathers and renders its lines.
    let empty_mods: Mods = Vec::<ModEntry>::new().into();
    with_mod_factories(empty_mods, |game| {
        let unconvertible = ItemStack::new("missing:unregistered_item", 1);
        assert!(unconvertible.try_into_api().is_err());
        let lines = vec![(sunrise_game_data::text::Text::literal("Plain"), u32::MAX)];
        let mut gathered = game
            .gather_tooltip_components(&unconvertible, lines.clone(), 10, (320.0, 240.0))
            .unwrap_ready();
        assert_eq!(text_lines(&gathered), ["Plain"]);

        let mut batch = HudBatch::scratch([320, 240]);
        let pre = game.tooltip_pre_components(
            &mut batch,
            Some(&unconvertible),
            &mut gathered,
            [10, 20],
            &DefaultTooltipPositioner,
        );
        assert_eq!(
            pre,
            Some(([10, 20], Cow::Borrowed(&*DEFAULT_TOOLTIP_FONT_ID)))
        );
        game.draw_stack_tooltip_lines(&mut batch, &unconvertible, lines, 10.0, 20.0);
        assert!(!batch.vertices.is_empty());
        assert_unwrapped(game, &stick);
    });

    // Part 2: listeners active — unconvertible stack skips Pre without cancelling, and a
    // deep TooltipElement::Text remains alongside its siblings.
    struct SiblingData;
    impl TooltipComponentData for SiblingData {}

    struct SiblingClient;
    impl ClientTooltipComponent for SiblingClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            24
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
            draw.fill([x as f32, y as f32, 6.0, 6.0], 0xFF44_88CC);
        }
    }

    struct DeepTextMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for DeepTextMod {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            event
                .factories
                .register::<SiblingData, SiblingClient>(|_| SiblingClient);
        }
        fn render_tooltip_gather_components(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipGatherComponentsEvent<'_>,
        ) -> Flow {
            let mut deep = mod_api::Text::literal("deep");
            for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
                let mut parent = mod_api::Text::literal("");
                parent.children.push(deep);
                deep = parent;
            }
            *event.elements = vec![
                TooltipElement::Text(mod_api::Text::literal("Before"), u32::MAX),
                TooltipElement::Text(deep, u32::MAX),
                TooltipElement::Component(TooltipComponent::custom(SiblingData)),
                TooltipElement::Text(mod_api::Text::literal("After"), u32::MAX),
            ];
            Flow::Continue
        }
        fn render_tooltip_pre(
            &self,
            _: &dyn ClientHost,
            _: &mut RenderTooltipPreEvent<'_>,
        ) -> Flow {
            self.0.fetch_add(1, Ordering::Relaxed);
            Flow::Continue
        }
    }

    /// Listens to Pre alone and keeps the literal text of each component it is shown.
    struct PreOnlyMod(Arc<Mutex<Vec<Option<String>>>>);
    #[mod_api::__subscriptions]
    impl Hooks for PreOnlyMod {
        fn render_tooltip_pre(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipPreEvent<'_>,
        ) -> Flow {
            *self.0.lock().unwrap() = event
                .components
                .iter()
                .map(|component| match component.as_text() {
                    Some((text, _)) => match &text.content {
                        mod_api::text::TextContent::Literal(literal) => Some(literal.clone()),
                        _ => None,
                    },
                    None => None,
                })
                .collect();
            Flow::Continue
        }
    }

    let pre_calls = Arc::new(AtomicUsize::new(0));
    let pre_only_seen = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![
        ModEntry::new("deep_text_mod", Box::new(DeepTextMod(pre_calls.clone()))),
        ModEntry::new("pre_only_mod", Box::new(PreOnlyMod(pre_only_seen.clone()))),
    ]
    .into();

    with_mod_factories(mods, |game| {
        // With Pre alone listening, Gather never runs, so Pre sees the wide line unsplit.
        assert!(mod_api::__private::set_mod_enabled("deep_text_mod", false));
        let mut gathered = assert_unwrapped(game, &stick);
        let mut pre_batch = HudBatch::scratch([160, 240]);
        assert!(
            game.tooltip_pre_components(
                &mut pre_batch,
                Some(&stick),
                &mut gathered,
                [145, 20],
                &DefaultTooltipPositioner,
            )
            .is_some()
        );
        assert_eq!(*pre_only_seen.lock().unwrap(), [Some(WIDE.to_owned())]);
        assert!(mod_api::__private::set_mod_enabled("pre_only_mod", false));
        assert!(mod_api::__private::set_mod_enabled("deep_text_mod", true));

        // An enabled Gather listener cannot take a stack that does not cross the API
        // boundary: Gather is skipped and the wide line survives unwrapped.
        assert_unwrapped(game, &ItemStack::new("missing:unregistered_item", 1));

        let unconvertible = ItemStack::new("missing:unregistered_item", 1);
        let mut one_line = vec![PreparedTooltipItem::Text {
            text: sunrise_game_data::text::Text::literal("Keep"),
            color: u32::MAX,
        }];
        let mut pre_batch = HudBatch::scratch([320, 240]);
        let pre = game.tooltip_pre_components(
            &mut pre_batch,
            Some(&unconvertible),
            &mut one_line,
            [10, 20],
            &DefaultTooltipPositioner,
        );
        assert_eq!(
            pre,
            Some(([10, 20], Cow::Borrowed(&*DEFAULT_TOOLTIP_FONT_ID)))
        );
        assert_eq!(
            pre_calls.load(Ordering::Relaxed),
            0,
            "stack conversion failure skips Pre without cancelling"
        );

        let mut gathered = game
            .gather_tooltip_components(
                &stick,
                vec![(sunrise_game_data::text::Text::literal("Seed"), u32::MAX)],
                10,
                (320.0, 240.0),
            )
            .unwrap_ready();
        assert_eq!(gathered.len(), 4);
        assert_eq!(text_lines(&gathered), ["Before", "deep", "After"]);
        assert!(gathered[2].downcast_ref::<SiblingClient>().is_some());

        let mut draw_batch = HudBatch::scratch([320, 240]);
        assert!(
            game.draw_prepared_tooltip(
                &mut draw_batch,
                Some(&stick),
                &mut gathered,
                [10, 20],
                &DefaultTooltipPositioner,
                true
            )
            .is_some()
        );
        assert_eq!(pre_calls.load(Ordering::Relaxed), 1);
        assert!(
            draw_batch
                .vertices
                .iter()
                .any(|v| v.color == rgba(0x44, 0x88, 0xCC, 0xFF)),
            "valid sibling custom component must still render"
        );
    });
}

/// A failed component aborts only its tooltip: no background or component paint survives,
/// including paint from a successful text pass before the image pass panics. Pre's own paint
/// survives, and healthy owners can still draw other tooltips.
#[test]
#[ignore = "daily"]
fn panic_containment_rolls_back_partial_paint_and_disables_only_failing_factory() {
    if run_tooltip_cases(
        "panic_containment_rolls_back_partial_paint_and_disables_only_failing_factory",
        &["tooltip"],
    ) {
        return;
    }
    struct CreatePanicData;
    impl TooltipComponentData for CreatePanicData {}
    struct WidthPanicData;
    impl TooltipComponentData for WidthPanicData {}
    struct HeightPanicData<const N: usize>;
    impl<const N: usize> TooltipComponentData for HeightPanicData<N> {}
    struct HandPanicData;
    impl TooltipComponentData for HandPanicData {}
    struct DrawTextPanicData;
    impl TooltipComponentData for DrawTextPanicData {}
    struct DrawImagePanicData;
    impl TooltipComponentData for DrawImagePanicData {}
    struct SiblingData;
    impl TooltipComponentData for SiblingData {}

    struct WidthPanicClient(Arc<AtomicUsize>);
    impl ClientTooltipComponent for WidthPanicClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            self.0.fetch_add(1, Ordering::Relaxed);
            panic!("tooltip width panic");
        }
    }

    struct HeightPanicClient<const N: usize> {
        query: Cell<usize>,
        calls: Arc<AtomicUsize>,
    }
    impl<const N: usize> ClientTooltipComponent for HeightPanicClient<N> {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let next = self.query.get() + 1;
            self.query.set(next);
            assert_ne!(next, N, "tooltip height panic on query {N}");
            10
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn draw_text(&self, _: &dyn TooltipFont, x: i32, y: i32, draw: &mut dyn DrawContext) {
            draw.fill([x as f32, y as f32, 8.0, 8.0], 0xFFDE_AD04);
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
            draw.fill([x as f32, y as f32, 8.0, 8.0], 0xFFDE_AD05);
        }
    }

    struct HandPanicClient(Arc<AtomicUsize>);
    impl ClientTooltipComponent for HandPanicClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn show_tooltip_with_item_in_hand(&self) -> bool {
            self.0.fetch_add(1, Ordering::Relaxed);
            panic!("tooltip hand panic");
        }
    }

    struct DrawTextPanicClient(Arc<AtomicUsize>);
    impl ClientTooltipComponent for DrawTextPanicClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            20
        }
        fn draw_text(&self, _: &dyn TooltipFont, x: i32, y: i32, draw: &mut dyn DrawContext) {
            self.0.fetch_add(1, Ordering::Relaxed);
            draw.fill([x as f32, y as f32, 8.0, 8.0], 0xFFDE_AD01);
            panic!("tooltip draw_text panic");
        }
    }

    struct DrawImagePanicClient(Arc<AtomicUsize>, Arc<AtomicBool>);
    impl ClientTooltipComponent for DrawImagePanicClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            20
        }
        fn draw_text(&self, _: &dyn TooltipFont, x: i32, y: i32, draw: &mut dyn DrawContext) {
            draw.fill([x as f32, y as f32, 8.0, 8.0], 0xFFDE_AD03);
            self.1.store(true, Ordering::Relaxed);
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
            assert!(self.1.load(Ordering::Relaxed));
            self.0.fetch_add(1, Ordering::Relaxed);
            draw.fill([x as f32, y as f32, 8.0, 8.0], 0xFFDE_AD02);
            panic!("tooltip draw_image panic");
        }
    }

    struct SiblingClient(Arc<AtomicUsize>);
    impl ClientTooltipComponent for SiblingClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            30
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
            self.0.fetch_add(1, Ordering::Relaxed);
            draw.fill([x as f32, y as f32, 8.0, 8.0], 0xFF12_EF34);
        }
    }

    struct CreatePanicMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for CreatePanicMod {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let calls = self.0.clone();
            event
                .factories
                .register::<CreatePanicData, SiblingClient>(move |_| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    panic!("tooltip factory panic");
                });
        }
    }

    struct WidthPanicMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for WidthPanicMod {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let calls = self.0.clone();
            event
                .factories
                .register::<WidthPanicData, WidthPanicClient>(move |_| {
                    WidthPanicClient(calls.clone())
                });
        }
    }

    struct HeightPanicMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for HeightPanicMod {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            fn register<const N: usize>(
                event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
                calls: Arc<AtomicUsize>,
            ) {
                event
                    .factories
                    .register::<HeightPanicData<N>, HeightPanicClient<N>>(move |_| {
                        HeightPanicClient {
                            query: Cell::new(0),
                            calls: calls.clone(),
                        }
                    });
            }
            register::<1>(event, self.0.clone());
            register::<2>(event, self.0.clone());
            register::<3>(event, self.0.clone());
        }
    }

    struct HandPanicMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for HandPanicMod {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let calls = self.0.clone();
            event
                .factories
                .register::<HandPanicData, HandPanicClient>(move |_| {
                    HandPanicClient(calls.clone())
                });
        }
    }

    struct DrawTextPanicMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for DrawTextPanicMod {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let calls = self.0.clone();
            event
                .factories
                .register::<DrawTextPanicData, DrawTextPanicClient>(move |_| {
                    DrawTextPanicClient(calls.clone())
                });
        }
    }

    struct DrawImagePanicMod(Arc<AtomicUsize>, Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for DrawImagePanicMod {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let calls = self.0.clone();
            let text_drawn = self.1.clone();
            event
                .factories
                .register::<DrawImagePanicData, DrawImagePanicClient>(move |_| {
                    DrawImagePanicClient(calls.clone(), text_drawn.clone())
                });
        }
    }

    struct SiblingMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for SiblingMod {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let calls = self.0.clone();
            event
                .factories
                .register::<SiblingData, SiblingClient>(move |_| SiblingClient(calls.clone()));
        }
        fn render_tooltip_pre(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipPreEvent<'_>,
        ) -> Flow {
            event.draw.fill([3.0, 3.0, 2.0, 2.0], 0xFFAA_BBCC);
            Flow::Continue
        }
    }

    let create_calls = Arc::new(AtomicUsize::new(0));
    let width_calls = Arc::new(AtomicUsize::new(0));
    let height_calls = Arc::new(AtomicUsize::new(0));
    let hand_calls = Arc::new(AtomicUsize::new(0));
    let text_calls = Arc::new(AtomicUsize::new(0));
    let image_calls = Arc::new(AtomicUsize::new(0));
    let image_component_text_drawn = Arc::new(AtomicBool::new(false));
    let sibling_calls = Arc::new(AtomicUsize::new(0));

    let mods: Mods = vec![
        ModEntry::new(
            "panic_create",
            Box::new(CreatePanicMod(create_calls.clone())),
        ),
        ModEntry::new("panic_width", Box::new(WidthPanicMod(width_calls.clone()))),
        ModEntry::new(
            "panic_height",
            Box::new(HeightPanicMod(height_calls.clone())),
        ),
        ModEntry::new("panic_hand", Box::new(HandPanicMod(hand_calls.clone()))),
        ModEntry::new(
            "panic_draw_text",
            Box::new(DrawTextPanicMod(text_calls.clone())),
        ),
        ModEntry::new(
            "panic_draw_image",
            Box::new(DrawImagePanicMod(
                image_calls.clone(),
                image_component_text_drawn.clone(),
            )),
        ),
        ModEntry::new(
            "healthy_sibling",
            Box::new(SiblingMod(sibling_calls.clone())),
        ),
    ]
    .into();

    with_mod_factories(mods, |game| {
        let assert_aborts = |mut component: PreparedTooltipItem| {
            let mut batch = HudBatch::scratch([320, 240]);
            assert_eq!(
                game.draw_prepared_tooltip(
                    &mut batch,
                    None,
                    std::slice::from_mut(&mut component),
                    [10, 20],
                    &DefaultTooltipPositioner,
                    true,
                ),
                None
            );
            assert!(
                !batch.vertices.is_empty(),
                "Pre listener paint must survive"
            );
            assert!(
                batch
                    .vertices
                    .iter()
                    .all(|v| v.color == rgba(0xAA, 0xBB, 0xCC, 0xFF)),
                "neither the background nor either component pass may leave paint"
            );
        };

        // 1. Factory creation panic: caught and disabled on subsequent tooltips.
        assert!(matches!(
            game.prepare_tooltip_component(&TooltipComponent::custom(CreatePanicData)),
            Err(TooltipComponentPrepareError::FactoryFailed { .. })
        ));
        assert_eq!(create_calls.load(Ordering::Relaxed), 1);
        assert!(matches!(
            game.prepare_tooltip_component(&TooltipComponent::custom(CreatePanicData)),
            Err(TooltipComponentPrepareError::FactoryFailed { .. })
        ));
        assert_eq!(create_calls.load(Ordering::Relaxed), 1);

        // 2. Width failure aborts instead of producing a 0px measurement.
        let width_comp = game
            .prepare_tooltip_component(&TooltipComponent::custom(WidthPanicData))
            .unwrap();
        assert_aborts(width_comp);
        assert_eq!(width_calls.load(Ordering::Relaxed), 1);
        assert!(matches!(
            game.prepare_tooltip_component(&TooltipComponent::custom(WidthPanicData)),
            Err(TooltipComponentPrepareError::FactoryFailed { .. })
        ));

        // Height failure aborts at the initial measurement or after either pass. Even
        // a failure after the image callback must discard all the tooltip's paint.
        for (query, payload) in [
            (1, TooltipComponent::custom(HeightPanicData::<1>)),
            (2, TooltipComponent::custom(HeightPanicData::<2>)),
            (3, TooltipComponent::custom(HeightPanicData::<3>)),
        ] {
            let before = height_calls.load(Ordering::Relaxed);
            assert_aborts(game.prepare_tooltip_component(&payload).unwrap());
            assert_eq!(height_calls.load(Ordering::Relaxed) - before, query);
            assert!(matches!(
                game.prepare_tooltip_component(&payload),
                Err(TooltipComponentPrepareError::FactoryFailed { .. })
            ));
        }

        // 4. show_tooltip_with_item_in_hand panic: caught, returns false, disables factory.
        let hand_comp = game
            .prepare_tooltip_component(&TooltipComponent::custom(HandPanicData))
            .unwrap();
        assert!(
            !hand_comp
                .as_custom()
                .unwrap()
                .show_tooltip_with_item_in_hand()
        );
        assert_eq!(hand_calls.load(Ordering::Relaxed), 1);
        assert!(matches!(
            game.prepare_tooltip_component(&TooltipComponent::custom(HandPanicData)),
            Err(TooltipComponentPrepareError::FactoryFailed { .. })
        ));

        // 5 & 6. Either pass can fail; after a successful text pass the image failure must
        // still discard the text, the partial image, and the background together.
        for payload in [
            TooltipComponent::custom(DrawTextPanicData),
            TooltipComponent::custom(DrawImagePanicData),
        ] {
            assert_aborts(game.prepare_tooltip_component(&payload).unwrap());
        }
        assert_eq!(text_calls.load(Ordering::Relaxed), 1);
        assert_eq!(image_calls.load(Ordering::Relaxed), 1);
        assert!(image_component_text_drawn.load(Ordering::Relaxed));

        // Subsequent tooltips skip disabled factories while the healthy owner can still draw.
        assert!(matches!(
            game.prepare_tooltip_component(&TooltipComponent::custom(DrawTextPanicData)),
            Err(TooltipComponentPrepareError::FactoryFailed { .. })
        ));
        assert!(matches!(
            game.prepare_tooltip_component(&TooltipComponent::custom(DrawImagePanicData)),
            Err(TooltipComponentPrepareError::FactoryFailed { .. })
        ));
        let mut batch = HudBatch::scratch([320, 240]);
        let mut sibling = game
            .prepare_tooltip_component(&TooltipComponent::custom(SiblingData))
            .unwrap();
        assert!(
            game.draw_prepared_tooltip(
                &mut batch,
                None,
                std::slice::from_mut(&mut sibling),
                [10, 20],
                &DefaultTooltipPositioner,
                true,
            )
            .is_some()
        );
        assert_eq!(sibling_calls.load(Ordering::Relaxed), 1);
        assert!(
            batch
                .vertices
                .iter()
                .any(|v| v.color == rgba(0x12, 0xEF, 0x34, 0xFF))
        );
    });
}

/// Finding 1: original engine tooltip text and vanilla bundles that cannot convert across the
/// mod-API boundary are preserved end-to-end (both with and without listeners, and for both item
/// and non-item tooltips) without dropping lines or bundles.
#[test]
#[ignore = "daily"]
fn unconvertible_engine_text_and_bundle_preserved_without_api_roundtrip() {
    if run_tooltip_cases(
        "unconvertible_engine_text_and_bundle_preserved_without_api_roundtrip",
        &["tooltip"],
    ) {
        return;
    }
    let pre_calls = Arc::new(AtomicUsize::new(0));
    let gather_calls = Arc::new(AtomicUsize::new(0));

    struct ObserverMod {
        gather_calls: Arc<AtomicUsize>,
        pre_calls: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for ObserverMod {
        fn render_tooltip_gather_components(
            &self,
            _: &dyn ClientHost,
            _: &mut RenderTooltipGatherComponentsEvent<'_>,
        ) -> Flow {
            self.gather_calls.fetch_add(1, Ordering::Relaxed);
            Flow::Continue
        }
        fn render_tooltip_pre(
            &self,
            _: &dyn ClientHost,
            _: &mut RenderTooltipPreEvent<'_>,
        ) -> Flow {
            self.pre_calls.fetch_add(1, Ordering::Relaxed);
            Flow::Continue
        }
    }

    let make_unconvertible_text = || {
        let text = sunrise_game_data::text::Text::from(serde_json::json!({
            "object": "player",
            "player": 7,
            "fallback": "Visible"
        }));
        assert_eq!(text.plain(), "Visible");
        assert!(text.try_into_api().is_err());
        text
    };
    let make_unconvertible_bundle = || {
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        bundle.data_mut().bundle_contents = vec![ItemStack::new("minecraft:stick", 1)];
        bundle
            .data_mut()
            .set_other("!minecraft:damage", Some(sunrise_core::nbt::Tag::Int(7)));
        assert!(bundle.try_into_api().is_err());
        bundle
    };

    // Without listeners: unconvertible text & unconvertible bundle both survive Gather & draw.
    let empty_mods: Mods = Vec::<ModEntry>::new().into();
    with_mod_factories(empty_mods, |game| {
        let stick = ItemStack::new("minecraft:stick", 1);
        let text = make_unconvertible_text();
        let gathered = game
            .gather_tooltip_components(&stick, vec![(text.clone(), u32::MAX)], 10, (320.0, 240.0))
            .unwrap_ready();
        assert_eq!(text_lines(&gathered), ["Visible"]);
        let mut batch = HudBatch::scratch([320, 240]);
        let rect = game.draw_tooltip_lines(&mut batch, &[(text.clone(), u32::MAX)], 10.0, 20.0);
        assert!(rect.is_some());
        assert!(!batch.vertices.is_empty());

        let bundle = make_unconvertible_bundle();
        let gathered_bundle = game
            .gather_tooltip_components(&bundle, vec![(text, u32::MAX)], 10, (320.0, 240.0))
            .unwrap_ready();
        assert_eq!(gathered_bundle.len(), 2);
        assert_eq!(text_lines(&gathered_bundle[..1]), ["Visible"]);
        assert!(gathered_bundle[1].as_bundle().is_some());
    });

    // With Gather + Pre listeners attached: unconvertible engine text skips Gather and Pre
    // cleanly without dropping the original text or bundle.
    let mods: Mods = vec![ModEntry::new(
        "observer_mod",
        Box::new(ObserverMod {
            gather_calls: gather_calls.clone(),
            pre_calls: pre_calls.clone(),
        }),
    )]
    .into();
    with_mod_factories(mods, |game| {
        let stick = ItemStack::new("minecraft:stick", 1);
        let text = make_unconvertible_text();
        let mut gathered = game
            .gather_tooltip_components(&stick, vec![(text.clone(), u32::MAX)], 10, (320.0, 240.0))
            .unwrap_ready();
        assert_eq!(text_lines(&gathered), ["Visible"]);
        assert_eq!(gather_calls.load(Ordering::Relaxed), 0);

        let mut item_batch = HudBatch::scratch([320, 240]);
        let rect = game.draw_prepared_tooltip(
            &mut item_batch,
            Some(&stick),
            &mut gathered,
            [10, 20],
            &DefaultTooltipPositioner,
            true,
        );
        assert!(rect.is_some());
        assert!(!item_batch.vertices.is_empty());
        assert_eq!(pre_calls.load(Ordering::Relaxed), 0);

        let mut non_item_batch = HudBatch::scratch([320, 240]);
        let non_item_rect =
            game.draw_tooltip_lines(&mut non_item_batch, &[(text.clone(), u32::MAX)], 10.0, 20.0);
        assert!(non_item_rect.is_some());
        assert!(!non_item_batch.vertices.is_empty());
        assert_eq!(pre_calls.load(Ordering::Relaxed), 0);

        let bundle = make_unconvertible_bundle();
        let mut gathered_bundle = game
            .gather_tooltip_components(
                &bundle,
                vec![(sunrise_game_data::text::Text::literal("Bundle"), u32::MAX)],
                10,
                (320.0, 240.0),
            )
            .unwrap_ready();
        assert_eq!(gathered_bundle.len(), 2);
        assert!(gathered_bundle[1].as_bundle().is_some());
        let mut bundle_batch = HudBatch::scratch([320, 240]);
        assert!(
            game.draw_prepared_tooltip(
                &mut bundle_batch,
                Some(&bundle),
                &mut gathered_bundle,
                [10, 20],
                &DefaultTooltipPositioner,
                true,
            )
            .is_some()
        );
        assert_eq!(pre_calls.load(Ordering::Relaxed), 0);
    });
}

/// Finding 2: unregistered custom tooltip component aborts tooltip preparation with
/// `MissingFactory` before `render_tooltip_pre`, `render_tooltip_texture`, or painting — without
/// reporting cancellation or latching so the next ordinary tooltip still renders cleanly.
#[test]
#[ignore = "daily"]
fn unregistered_custom_component_aborts_before_pre_texture_and_paint_without_latching() {
    if run_tooltip_cases(
        "unregistered_custom_component_aborts_before_pre_texture_and_paint_without_latching",
        &["tooltip"],
    ) {
        return;
    }
    struct UnregisteredPayload;
    impl TooltipComponentData for UnregisteredPayload {}

    struct ProbeState {
        inject_unregistered: AtomicBool,
        pre_calls: AtomicUsize,
        texture_calls: AtomicUsize,
    }
    struct ProbeMod(Arc<ProbeState>);
    #[mod_api::__subscriptions]
    impl Hooks for ProbeMod {
        fn render_tooltip_gather_components(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipGatherComponentsEvent<'_>,
        ) -> Flow {
            if self.0.inject_unregistered.load(Ordering::Relaxed) {
                event
                    .elements
                    .push(TooltipElement::Component(TooltipComponent::custom(
                        UnregisteredPayload,
                    )));
            }
            Flow::Continue
        }
        fn render_tooltip_pre(
            &self,
            _: &dyn ClientHost,
            _: &mut RenderTooltipPreEvent<'_>,
        ) -> Flow {
            self.0.pre_calls.fetch_add(1, Ordering::Relaxed);
            Flow::Continue
        }
        fn render_tooltip_texture(
            &self,
            _: &dyn ClientHost,
            _: &mut RenderTooltipTextureEvent<'_>,
        ) {
            self.0.texture_calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    let state = Arc::new(ProbeState {
        inject_unregistered: AtomicBool::new(true),
        pre_calls: AtomicUsize::new(0),
        texture_calls: AtomicUsize::new(0),
    });
    let mods: Mods = vec![ModEntry::new(
        "probe_mod",
        Box::new(ProbeMod(state.clone())),
    )]
    .into();

    with_mod_factories(mods, |game| {
        let stick = ItemStack::new("minecraft:stick", 1);
        let lines = vec![(sunrise_game_data::text::Text::literal("Stick"), u32::MAX)];
        assert!(matches!(
            game.gather_tooltip_components(&stick, lines.clone(), 10, (320.0, 240.0)),
            GatheredTooltip::PrepareError(TooltipComponentPrepareError::MissingFactory {
                owner: Some("probe_mod"),
                ..
            })
        ));

        let mut aborted_batch = HudBatch::scratch([320, 240]);
        game.draw_stack_tooltip_lines(&mut aborted_batch, &stick, lines.clone(), 10.0, 20.0);
        assert!(
            aborted_batch.vertices.is_empty(),
            "missing factory must abort before painting"
        );
        assert_eq!(state.pre_calls.load(Ordering::Relaxed), 0);
        assert_eq!(state.texture_calls.load(Ordering::Relaxed), 0);

        // Non-latching: the next ordinary tooltip renders normally through Pre & Texture.
        state.inject_unregistered.store(false, Ordering::Relaxed);
        let mut next_batch = HudBatch::scratch([320, 240]);
        game.draw_stack_tooltip_lines(&mut next_batch, &stick, lines, 10.0, 20.0);
        assert!(!next_batch.vertices.is_empty());
        assert_eq!(state.pre_calls.load(Ordering::Relaxed), 1);
        assert_eq!(state.texture_calls.load(Ordering::Relaxed), 1);
    });
}

/// A retained destructor panic disables its publisher after preparation, cancellation or
/// removal. It does not disable a healthy listener or a sibling item provider of the same mod.
#[test]
#[ignore = "daily"]
fn panicking_tooltip_payload_drop_contained_across_prepare_cancel_and_removal() {
    if run_tooltip_cases(
        "panicking_tooltip_payload_drop_contained_across_prepare_cancel_and_removal",
        &["prepare", "cancel", "remove", "item"],
    ) {
        return;
    }
    struct PanicDropPayload(Arc<AtomicUsize>);
    impl TooltipComponentData for PanicDropPayload {}
    impl Drop for PanicDropPayload {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
            panic!("custom payload drop boom");
        }
    }

    struct HealthyPayload;
    impl TooltipComponentData for HealthyPayload {}

    struct SimpleClient;
    impl ClientTooltipComponent for SimpleClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            20
        }
    }

    #[derive(Clone)]
    struct ImageProvider {
        calls: Arc<AtomicUsize>,
        drops: Option<Arc<AtomicUsize>>,
    }
    #[mod_api::sunrise_part]
    impl ItemPart for ImageProvider {
        fn tooltip_image(
            &self,
            _: &dyn ClientHost,
            _: &mut TooltipImageEvent<'_>,
        ) -> Option<TooltipComponent> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Some(match &self.drops {
                Some(drops) => TooltipComponent::custom(PanicDropPayload(drops.clone())),
                None => TooltipComponent::custom(HealthyPayload),
            })
        }
    }

    struct ProducerMod {
        calls: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
        healthy_item_calls: Arc<AtomicUsize>,
        item_case: bool,
    }
    #[mod_api::__subscriptions]
    impl Hooks for ProducerMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            if self.item_case {
                let stick = r.item_key("minecraft:stick");
                r.item_part(
                    stick,
                    ImageProvider {
                        calls: self.calls.clone(),
                        drops: Some(self.drops.clone()),
                    },
                );
                let apple = r.item_key("minecraft:apple");
                r.item_part(
                    apple,
                    ImageProvider {
                        calls: self.healthy_item_calls.clone(),
                        drops: None,
                    },
                );
            }
        }
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            event
                .factories
                .register::<PanicDropPayload, SimpleClient>(|_| SimpleClient);
            event
                .factories
                .register::<HealthyPayload, SimpleClient>(|_| SimpleClient);
        }
        fn render_tooltip_gather_components(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipGatherComponentsEvent<'_>,
        ) -> Flow {
            if !self.item_case {
                self.calls.fetch_add(1, Ordering::Relaxed);
                event
                    .elements
                    .push(TooltipElement::Component(TooltipComponent::custom(
                        PanicDropPayload(self.drops.clone()),
                    )));
            }
            Flow::Continue
        }
    }

    struct SiblingMod {
        mode: Arc<AtomicU8>,
        calls: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for SiblingMod {
        fn render_tooltip_gather_components(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipGatherComponentsEvent<'_>,
        ) -> Flow {
            self.calls.fetch_add(1, Ordering::Relaxed);
            match self.mode.swap(0, Ordering::Relaxed) {
                1 => return Flow::Cancel,
                2 => event.elements.retain(|e| {
                    !matches!(e, TooltipElement::Component(c) if c.downcast_ref::<PanicDropPayload>().is_some())
                }),
                _ => {}
            }
            event
                .elements
                .push(TooltipElement::Component(TooltipComponent::custom(
                    HealthyPayload,
                )));
            Flow::Continue
        }
    }

    let case = std::env::var(TOOLTIP_CHILD).unwrap();
    let (mode, item_case) = match case.as_str() {
        "prepare" => (0, false),
        "cancel" => (1, false),
        "remove" => (2, false),
        "item" => (0, true),
        _ => unreachable!("unknown case {case}"),
    };
    let drops = Arc::new(AtomicUsize::new(0));
    let producer_calls = Arc::new(AtomicUsize::new(0));
    let sibling_calls = Arc::new(AtomicUsize::new(0));
    let healthy_item_calls = Arc::new(AtomicUsize::new(0));
    let mods: Mods = vec![
        ModEntry::new(
            "drop_producer",
            Box::new(ProducerMod {
                calls: producer_calls.clone(),
                drops: drops.clone(),
                healthy_item_calls: healthy_item_calls.clone(),
                item_case,
            }),
        ),
        ModEntry::new(
            "drop_sibling",
            Box::new(SiblingMod {
                mode: Arc::new(AtomicU8::new(mode)),
                calls: sibling_calls.clone(),
            }),
        ),
    ]
    .into();

    with_mod_factories(mods, |game| {
        let _ = mod_api::__private::take_hook_panics();
        let stick = ItemStack::new("minecraft:stick", 1);
        let hover = |stack: &ItemStack| {
            let lines = vec![(sunrise_game_data::text::Text::literal("Item"), u32::MAX)];
            game.gather_tooltip_components(stack, lines, 10, (320.0, 240.0))
        };

        let first = hover(&stick);
        if mode == 1 {
            assert!(matches!(first, GatheredTooltip::Cancelled));
        } else {
            assert_eq!(first.unwrap_ready().len(), if mode == 2 { 2 } else { 3 });
        }
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(producer_calls.load(Ordering::Relaxed), 1);

        // Nothing manually disables the failed producer between hovers. Only the
        // item line and the healthy Gather sibling's component should remain.
        assert_eq!(hover(&stick).unwrap_ready().len(), 2);
        assert_eq!(producer_calls.load(Ordering::Relaxed), 1);
        assert_eq!(sibling_calls.load(Ordering::Relaxed), 2);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(
            mod_api::__private::take_hook_panics(),
            [mod_api::__private::HookPanicReport {
                mod_id: "drop_producer",
                hook: "data_drop",
            }]
        );

        if item_case {
            let apple = ItemStack::new("minecraft:apple", 1);
            assert_eq!(hover(&apple).unwrap_ready().len(), 3);
            assert_eq!(healthy_item_calls.load(Ordering::Relaxed), 1);
        }
    });
}

/// Findings 4 & 5: item-owned tooltip font (`ClientItemExtensions::tooltip_font`) drives Gather's
/// wrapping and seeds `render_tooltip_pre`'s `font`, while a replacement font chosen in Pre is
/// used only for measurement and drawing without rewrapping; fractional cursor coordinates are
/// quantized once so Pre editing only `y` preserves the quantized `x`.
#[test]
#[ignore = "daily"]
fn item_tooltip_font_wraps_in_gather_pre_replaces_without_rewrap_and_anchor_quantizes_once() {
    if run_tooltip_cases(
        "item_tooltip_font_wraps_in_gather_pre_replaces_without_rewrap_and_anchor_quantizes_once",
        &["tooltip"],
    ) {
        return;
    }
    struct FontFlowState {
        phase: AtomicU8,
        item_font: Identifier,
        pre_font: Identifier,
        seen_pre_place: Mutex<Option<(i32, i32)>>,
    }

    #[derive(Clone)]
    struct Image(Arc<FontFlowState>);
    #[mod_api::sunrise_part]
    impl ItemPart for Image {
        fn tooltip_image(
            &self,
            _: &dyn ClientHost,
            _: &mut TooltipImageEvent<'_>,
        ) -> Option<TooltipComponent> {
            assert_eq!(self.0.phase.fetch_add(1, Ordering::Relaxed), 0);
            None
        }
    }

    struct ItemFontProvider(Arc<FontFlowState>);
    impl mod_api::ClientItemExtensions for ItemFontProvider {
        fn tooltip_font(&self, _: &dyn ClientHost, _: &mod_api::ItemStack) -> Option<Identifier> {
            match self.0.phase.fetch_add(1, Ordering::Relaxed) {
                1 => Some(self.0.item_font.clone()),
                3 => None,
                step => panic!("unexpected tooltip font query at step {step}"),
            }
        }
    }

    struct FontFlowMod(Arc<FontFlowState>);
    #[mod_api::__subscriptions]
    impl Hooks for FontFlowMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let key = r.item_key("minecraft:stick");
            r.item_part(key, Image(self.0.clone()));
        }

        fn register_client_item_extensions(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterClientItemExtensionsEvent<'_>,
        ) {
            let stick_key = mod_api::__private::item_key("minecraft:stick").unwrap();
            event
                .extensions
                .client_item_extensions(stick_key, Arc::new(ItemFontProvider(self.0.clone())))
                .unwrap();
        }
        fn render_tooltip_gather_components(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipGatherComponentsEvent<'_>,
        ) -> Flow {
            assert_eq!(self.0.phase.fetch_add(1, Ordering::Relaxed), 2);
            // 25px fits "aa aa" in default (14px), wraps to 2 lines in item_font (36px).
            event.max_width = 25;
            Flow::Continue
        }
        fn render_tooltip_pre(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipPreEvent<'_>,
        ) -> Flow {
            assert_eq!(*event.font, *DEFAULT_TOOLTIP_FONT_ID);
            assert_eq!(self.0.phase.fetch_add(1, Ordering::Relaxed), 4);
            *self.0.seen_pre_place.lock().unwrap() = Some((event.place.x, event.place.y));
            *event.font = self.0.pre_font.clone();
            event.place.y = 40;
            Flow::Continue
        }
    }

    let item_font = Identifier::parse("minecraft:item_wide").unwrap();
    let pre_font = Identifier::parse("minecraft:pre_wider").unwrap();
    let state = Arc::new(FontFlowState {
        phase: AtomicU8::new(0),
        item_font,
        pre_font,
        seen_pre_place: Mutex::new(None),
    });
    let mods: Mods = vec![ModEntry::new(
        "font_flow_mod",
        Box::new(FontFlowMod(state.clone())),
    )]
    .into();

    with_mod_factories(mods, |game| {
        // Install three distinct space-advance fonts so Gather wrapping vs Pre measurement have
        // unambiguous pixel widths:
        // - default:    'a'=3, ' '=2  -> "aa aa" = 14px (1 line at max_width=25)
        // - item_wide:  'a'=8, ' '=4  -> "aa aa" = 36px -> splits at max_width=25 into ["aa", "aa"] (each 16px)
        // - pre_wider:  'a'=15,' '=4  -> each split "aa" measures 30px (no rewrap to 4 lines!)
        let default_json = r#"{"providers":[{"type":"space","advances":{" ":2,"a":3}}]}"#;
        let item_json = r#"{"providers":[{"type":"space","advances":{" ":4,"a":8}}]}"#;
        let pre_json = r#"{"providers":[{"type":"space","advances":{" ":4,"a":15}}]}"#;
        let source = sunrise_assets::resource_source::ResourceSource::indexed(
            std::collections::BTreeMap::from([
                (
                    "assets/minecraft/font/default.json".into(),
                    sunrise_assets::resource_source::Resource::bytes(
                        default_json.as_bytes().to_vec(),
                    ),
                ),
                (
                    "assets/minecraft/font/item_wide.json".into(),
                    sunrise_assets::resource_source::Resource::bytes(item_json.as_bytes().to_vec()),
                ),
                (
                    "assets/minecraft/font/pre_wider.json".into(),
                    sunrise_assets::resource_source::Resource::bytes(pre_json.as_bytes().to_vec()),
                ),
            ]),
        );
        game.client.gui.atlas.font = sunrise_client::font::Font::metrics(&source);
        game.client.armor_stand_assets.0.extensions =
            sunrise_client_gameplay::game::ClientGame::prepare_client_item_extensions(
                &game.client.gui,
                &mut game.client.client_item_extension_owners,
            );

        let stick = ItemStack::new("minecraft:stick", 1);
        let text = sunrise_game_data::text::Text::literal("aa aa");
        let anchor = quantize_tooltip_anchor((10.75, 20.25));
        assert_eq!(anchor, [10, 20]);

        let mut components = game
            .gather_tooltip_components(&stick, vec![(text, u32::MAX)], anchor[0], (320.0, 240.0))
            .unwrap_ready();
        // Wrapped using item_wide (36px > 25px -> 2 lines of "aa"), NOT default (14px -> 1 line).
        assert_eq!(text_lines(&components), ["aa", "aa"]);

        // Pre receives the newly queried default font and anchor [10, 20], replaces font with pre_wider, and edits
        // only y -> 40. Final rect must use truncated x = 10 (+12 = 22), edited y = 40 (-12 = 28),
        // pre_wider width of "aa" = 30, and 2 lines height = 20 (never rewrapped in pre_wider).
        let mut batch = HudBatch::scratch([320, 240]);
        let rect = game.draw_prepared_tooltip(
            &mut batch,
            Some(&stick),
            &mut components,
            anchor,
            &DefaultTooltipPositioner,
            true,
        );
        assert_eq!(state.phase.load(Ordering::Relaxed), 5);
        assert_eq!(*state.seen_pre_place.lock().unwrap(), Some((10, 20)));
        assert_eq!(rect, Some([22.0, 28.0, 30.0, 20.0]));
    });
}

#[test]
#[ignore = "daily"]
fn gather_wraps_logical_arabic_advances_but_measures_visual_lines() {
    if run_tooltip_cases(
        "gather_wraps_logical_arabic_advances_but_measures_visual_lines",
        &["tooltip"],
    ) {
        return;
    }
    struct Width25;
    #[mod_api::__subscriptions]
    impl Hooks for Width25 {
        fn render_tooltip_gather_components(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipGatherComponentsEvent<'_>,
        ) -> Flow {
            event.max_width = 25;
            Flow::Continue
        }
    }

    use sunrise_assets::resource_source::{Resource, ResourceSource};
    // The two words measure 38px in logical order and 12px after shaping.
    let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
        "assets/minecraft/font/default.json".into(),
        Resource::bytes(br#"{"providers":[{"type":"space","advances":{"\u0644":9,"\u0627":9,"\ufefb":5," ":2}}]}"#.to_vec()),
    )]));
    for listened in [false, true] {
        let mods = if listened {
            vec![ModEntry::new("tooltip_wrap_25", Box::new(Width25))]
        } else {
            Vec::new()
        };
        with_mod_factories(mods.into(), |game| {
            game.client.gui.atlas.font = sunrise_client::font::Font::metrics(&source);
            let stack = ItemStack::new("minecraft:stick", 1);
            let line = sunrise_game_data::text::Text::literal("\u{644}\u{627} \u{644}\u{627}");
            let width = if listened { 320.0 } else { 51.0 };
            let mut components = game
                .gather_tooltip_components(&stack, vec![(line, u32::MAX)], 10, (width, 240.0))
                .unwrap_ready();
            let (lines, expected_rect): (&[&str], _) = if listened {
                (
                    &["\u{644}\u{627}", "\u{644}\u{627}"],
                    [22.0, 8.0, 5.0, 20.0],
                )
            } else {
                (&["\u{644}\u{627} \u{644}\u{627}"], [22.0, 8.0, 12.0, 8.0])
            };
            assert_eq!(text_lines(&components), lines);
            let mut batch = HudBatch::scratch([width as u32, 240]);
            let rect = game.draw_prepared_tooltip(
                &mut batch,
                Some(&stack),
                &mut components,
                [10, 20],
                &DefaultTooltipPositioner,
                true,
            );
            assert_eq!(rect, Some(expected_rect));
        });
    }
}

/// A hovered slot checks carried-item visibility once; stonecutter and merchant previews do
/// not check it at all, even for a component that would refuse a second query.
#[test]
#[ignore = "daily"]
fn only_hovered_slots_check_show_tooltip_with_item_in_hand() {
    if run_tooltip_cases(
        "only_hovered_slots_check_show_tooltip_with_item_in_hand",
        &["tooltip"],
    ) {
        return;
    }
    struct OnceHandData;
    impl TooltipComponentData for OnceHandData {}

    struct OnceHandClient(Arc<AtomicUsize>);
    impl ClientTooltipComponent for OnceHandClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            10
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            24
        }
        fn show_tooltip_with_item_in_hand(&self) -> bool {
            self.0.fetch_add(1, Ordering::Relaxed) == 0
        }
    }

    #[derive(Clone)]
    struct OnceImage;
    #[mod_api::sunrise_part]
    impl ItemPart for OnceImage {
        fn tooltip_image(
            &self,
            _: &dyn ClientHost,
            _: &mut TooltipImageEvent<'_>,
        ) -> Option<TooltipComponent> {
            Some(TooltipComponent::custom(OnceHandData))
        }
    }

    struct StationHandMod {
        hand_calls: Arc<AtomicUsize>,
        gather_calls: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for StationHandMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let stick = r.item_key("minecraft:stick");
            r.item_part(stick, OnceImage);
            let slab = r.item_key("minecraft:deepslate_tile_slab");
            r.item_part(slab, OnceImage);
        }
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let calls = self.hand_calls.clone();
            event
                .factories
                .register::<OnceHandData, OnceHandClient>(move |_| OnceHandClient(calls.clone()));
        }
        fn render_tooltip_gather_components(
            &self,
            _: &dyn ClientHost,
            _: &mut RenderTooltipGatherComponentsEvent<'_>,
        ) -> Flow {
            self.gather_calls.fetch_add(1, Ordering::Relaxed);
            Flow::Continue
        }
    }

    let hand_calls = Arc::new(AtomicUsize::new(0));
    let gather_calls = Arc::new(AtomicUsize::new(0));
    let mods: Mods = vec![ModEntry::new(
        "station_hand_mod",
        Box::new(StationHandMod {
            hand_calls: hand_calls.clone(),
            gather_calls: gather_calls.clone(),
        }),
    )]
    .into();

    with_mod_factories(mods, |game| {
        game.open_station_client(
            sunrise_gameplay_common::game::stations_common::Station::new(
                sunrise_gameplay_common::game::stations_common::Kind::Anvil,
                [0, 64, 0],
            ),
        );
        game.station_changed();
        game.inventory[9] = Some(ItemStack::new("minecraft:stick", 1));
        game.carried = Some(ItemStack::new("minecraft:stone", 1));

        let mut batch = HudBatch::scratch([512, 512]);
        let (ox, oy) = game.inventory_origin(&batch);
        // Find the inventory slot holding our stick in the station's slot table.
        let slot = (0..46)
            .find(|&i| {
                game.slot_view(i)
                    .as_ref()
                    .is_some_and(|s| s.item.as_ref() == "minecraft:stick")
            })
            .expect("stick should be visible in station player inventory");
        let (sx, sy) = game.slot_position_any(slot).unwrap();
        game.client.cursor = (ox + sx + 8.0, oy + sy + 8.0);
        assert_eq!(
            game.slot_at(&batch, game.client.cursor.0, game.client.cursor.1),
            Some(slot)
        );

        assert!(game.draw_station(&mut batch));
        assert_eq!(
            hand_calls.load(Ordering::Relaxed),
            1,
            "station draw must check show_tooltip_with_item_in_hand once per hovered slot"
        );
        assert_eq!(
            gather_calls.load(Ordering::Relaxed),
            1,
            "tooltip must render on first true show_tooltip_with_item_in_hand answer"
        );

        // Both previews bypass that same callback (which would now answer false).
        game.open_station_client(
            sunrise_gameplay_common::game::stations_common::Station::new(
                sunrise_gameplay_common::game::stations_common::Kind::Stonecutter,
                [0, 64, 0],
            ),
        );
        game.station_changed();
        let recipe = game
            .station_recipes()
            .stonecutting
            .iter()
            .position(|r| r.result.item.as_ref() == "minecraft:deepslate_tile_slab")
            .expect("stonecutter must have a deepslate tile slab recipe");
        let station = game.station.as_mut().unwrap();
        station.inputs[0] = Some(ItemStack::new("minecraft:deepslate", 1));
        station.stone.recipes = vec![recipe];
        game.carried = Some(ItemStack::new("minecraft:stone", 1));
        let mut batch = HudBatch::scratch([512, 512]);
        let (ox, oy) = game.inventory_origin(&batch);
        game.client.cursor = (ox + 60.0, oy + 25.0);
        assert!(game.draw_station(&mut batch));
        assert_eq!(hand_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            gather_calls.load(Ordering::Relaxed),
            2,
            "stonecutter preview gathers"
        );

        use sunrise_sim::merchant::{ItemCost, MerchantOffer, Session};
        game.open_remote_merchant(
            Session::new(
                vec![MerchantOffer {
                    base_cost_a: ItemCost {
                        stack: ItemStack::new("minecraft:stick", 1),
                    },
                    cost_b: None,
                    result: ItemStack::new("minecraft:bread", 1),
                    uses: 0,
                    max_uses: 12,
                    reward_exp: true,
                    special_price_diff: 0,
                    demand: 0,
                    price_multiplier: 0.0,
                    xp: 1,
                }],
                true,
            ),
            None,
        );
        game.carried = Some(ItemStack::new("minecraft:stone", 1));
        let mut batch = HudBatch::scratch([512, 512]);
        let (ox, oy) = game.inventory_origin(&batch);
        // Merchant screen is shifted left 50px; hover the first payment in the first offer.
        game.client.cursor = (ox - 40.0, oy + 26.0);
        assert!(game.draw_station(&mut batch));
        assert_eq!(hand_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            gather_calls.load(Ordering::Relaxed),
            3,
            "merchant preview gathers"
        );
    });
}

/// Finding 8: tooltip height accumulation and per-component `local_y` advance wrap as Java
/// signed 32-bit `int` rather than saturating.
#[test]
#[ignore = "daily"]
fn tooltip_height_and_local_y_use_wrapping_signed_i32_arithmetic() {
    if run_tooltip_cases(
        "tooltip_height_and_local_y_use_wrapping_signed_i32_arithmetic",
        &["tooltip"],
    ) {
        return;
    }
    struct WrapData {
        tag: &'static str,
        height: i32,
    }
    impl TooltipComponentData for WrapData {}

    struct WrapClient {
        tag: &'static str,
        height: i32,
        seen: Arc<Mutex<Vec<(&'static str, i32, i32)>>>,
    }
    impl ClientTooltipComponent for WrapClient {
        fn height(&self, _: &dyn TooltipFont) -> i32 {
            self.height
        }
        fn width(&self, _: &dyn TooltipFont) -> i32 {
            20
        }
        fn draw_image(
            &self,
            _: &dyn TooltipFont,
            _: i32,
            y: i32,
            _: i32,
            total_height: i32,
            _: &mut dyn DrawContext,
        ) {
            self.seen.lock().unwrap().push((self.tag, y, total_height));
        }
    }

    struct WrapMod(Arc<Mutex<Vec<(&'static str, i32, i32)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for WrapMod {
        fn register_client_tooltip_component_factories(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterClientTooltipComponentFactoriesEvent<'_>,
        ) {
            let seen = self.0.clone();
            event
                .factories
                .register::<WrapData, WrapClient>(move |d| WrapClient {
                    tag: d.tag,
                    height: d.height,
                    seen: seen.clone(),
                });
        }
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new("wrap_mod", Box::new(WrapMod(seen.clone())))].into();

    with_mod_factories(mods, |game| {
        let mut components = vec![
            game.prepare_tooltip_component(&TooltipComponent::custom(WrapData {
                tag: "first",
                height: i32::MAX,
            }))
            .unwrap(),
            game.prepare_tooltip_component(&TooltipComponent::custom(WrapData {
                tag: "second",
                height: 10,
            }))
            .unwrap(),
        ];
        let mut batch = HudBatch::scratch([320, 240]);
        let _ = game.draw_prepared_tooltip(
            &mut batch,
            None,
            &mut components,
            [10, 20],
            &DefaultTooltipPositioner,
            true,
        );
        // Total height wraps: 0_i32.wrapping_add(i32::MAX).wrapping_add(10) == i32::MIN + 9.
        let expected_h = 0_i32.wrapping_add(i32::MAX).wrapping_add(10);
        // DefaultTooltipPositioner keeps y = 20 - 12 = 8 (since 8 + expected_h + 6 < 240).
        // Pass 2 local_y after index 0: 8_i32.wrapping_add(i32::MAX).wrapping_add(2) == i32::MIN + 9.
        let expected_second_y = 8_i32.wrapping_add(i32::MAX).wrapping_add(2);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                ("first", 8, expected_h),
                ("second", expected_second_y, expected_h)
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn widget_tooltip_uses_the_same_positioner_for_pre_and_paint() {
    if run_tooltip_cases(
        "widget_tooltip_uses_the_same_positioner_for_pre_and_paint",
        &["tooltip"],
    ) {
        return;
    }

    struct Observe(Arc<Mutex<Vec<[i32; 2]>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn render_tooltip_pre(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderTooltipPreEvent<'_>,
        ) -> Flow {
            self.0
                .lock()
                .unwrap()
                .push(event.positioner.position_tooltip(
                    [320, 240],
                    [event.place.x, event.place.y],
                    [0, 8],
                ));
            event.place.x = 80;
            Flow::Continue
        }
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "widget_place",
        Box::new(Observe(seen.clone())),
    )]
    .into();
    with_mod_factories(mods, |game| {
        let lines = [(sunrise_game_data::text::Text::literal(""), u32::MAX)];
        let mut batch = HudBatch::scratch([320, 240]);
        let widget = [50, 40, 22, 22];
        assert_eq!(
            game.draw_tooltip_lines_positioned(
                &mut batch,
                &lines,
                [61, 51],
                &WidgetTooltipPositioner::Menu(widget)
            ),
            Some([92.0, 64.0, 0.0, 8.0]),
        );
        assert_eq!(
            game.draw_tooltip_lines_positioned(
                &mut batch,
                &lines,
                [10, 20],
                &WidgetTooltipPositioner::BelowOrAbove(widget)
            ),
            Some([53.0, 66.0, 0.0, 8.0]),
        );
        assert_eq!(*seen.lock().unwrap(), [[73, 64], [53, 66]]);
    });
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::stations_common::StationsCommonGameExt1 as _;
