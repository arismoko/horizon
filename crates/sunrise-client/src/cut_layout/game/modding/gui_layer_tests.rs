//! HUD slots in a real client: placement, scoped wrapping and replacement, and layer cancellation.
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use mod_api::{
    ClientHost, Flow, GuiLayerPlacement, GuiLayerPostEvent, GuiLayerPreEvent, Hooks, ModEntry,
    PlayerHeartTypeEvent, RegisterGuiLayersEvent, RegistrationContext, RenderGuiPostEvent,
    RenderGuiPreEvent, ScreenRenderBackgroundEvent, ScreenRenderPostEvent, ScreenRenderPreEvent,
    content::crash::current_native_mod,
};
use std::sync::{Arc, Mutex};
use sunrise_client::{gui_layers::VanillaGuiLayer, hud::HudBatch};
use sunrise_client_gameplay::game::ClientGame;
use sunrise_command::args::GameMode;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::{Screen, execution_role::ExecutionRole};
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::{MotionWorld, PlayerMotion, WATER, dry_side};

#[path = "gui_layer_tests/hidden_panic.rs"]
mod hidden_panic;
#[path = "gui_layer_tests/status_heights.rs"]
mod status_heights;

fn set_fixture_mode(game: &mut ClientGame<'_>, mode: GameMode) {
    if mode != game.mode {
        game.previous_mode = Some(game.mode);
    }
    game.mode = mode;
    game.creative = mode == GameMode::Creative;
    let spectator = mode == GameMode::Spectator;
    if let Some(player) = game.player.as_mut() {
        player.invulnerable = mode == GameMode::Creative || spectator;
        player.may_fly = player.invulnerable;
        player.spectator = spectator;
        player.local_spectator = spectator;
        if mode != GameMode::Creative {
            player.flying = spectator;
        }
    }
}

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log(entry: impl Into<String>) {
    LOG.lock().unwrap().push(entry.into());
}

fn assert_owner(id: &'static str) {
    assert_eq!(current_native_mod(), Some(id));
}

struct Compass;
#[mod_api::__subscriptions]
impl Hooks for Compass {
    fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
        use GuiLayerPlacement::*;
        // The layer namespace is deliberately different from the registering mod.
        for (id, placement) in [
            (
                "hud_test:under_crosshair",
                Below("minecraft:crosshair".into()),
            ),
            (
                "hud_test:over_crosshair",
                Above("minecraft:crosshair".into()),
            ),
            ("hud_test:bar", Above("minecraft:hotbar".into())),
            (
                "hud_test:above_health",
                Above("minecraft:player_health".into()),
            ),
            (
                "hud_test:below_contextual",
                Below("minecraft:contextual_info_bar".into()),
            ),
            ("hud_test:top", AboveAll),
        ] {
            assert!(event.layers.register(
                id.into(),
                placement,
                Box::new(move |_, draw| {
                    assert_owner("compass");
                    log(format!("draw {id}"));
                    if id == "hud_test:under_crosshair" {
                        draw.fill([0.0, 0.0, 4.0, 4.0], 0xFF00_FF00);
                    }
                }),
            ));
        }
    }
    fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        log(format!("pre {}", event.layer));
        Flow::Continue
    }
    fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
        log(format!("post {}", event.layer));
    }
}

struct Inner;
#[mod_api::__subscriptions]
impl Hooks for Inner {
    fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
        assert!(event.layers.wrap_layer(
            "hud_test:under_crosshair",
            Box::new(|ctx| {
                assert_owner("inner");
                log("inner before");
                ctx.draw_previous();
                assert_owner("inner");
                log("inner after");
            }),
        ));
        assert!(event.layers.wrap_layer(
            "minecraft:hotbar",
            Box::new(|ctx| {
                log("hotbar wrapper before");
                ctx.draw_previous();
                log("hotbar wrapper after");
            }),
        ));
        for (id, label) in [
            ("minecraft:crosshair", "crosshair wrapper"),
            ("minecraft:contextual_info_bar", "contextual wrapper"),
        ] {
            assert!(event.layers.wrap_layer(
                id,
                Box::new(move |ctx| {
                    log(label);
                    ctx.draw_previous();
                })
            ));
        }
    }
}

struct Editor;
#[mod_api::__subscriptions]
impl Hooks for Editor {
    fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
        assert!(
            !event
                .layers
                .wrap_layer("hud_test:missing", Box::new(|_| {}))
        );
        assert!(
            !event
                .layers
                .replace_layer("hud_test:missing", Box::new(|_, _| {}))
        );
        assert!(event.layers.wrap_layer(
            "hud_test:under_crosshair",
            Box::new(|ctx| {
                assert_owner("editor");
                log("outer before");
                ctx.with_draw(&mut |draw| draw.fill([0.0, 0.0, 4.0, 4.0], 0xFFFF_0000));
                ctx.draw_previous();
                assert_owner("editor");
                ctx.with_draw(&mut |draw| draw.fill([0.0, 0.0, 4.0, 4.0], 0xFF00_00FF));
                log("outer after");
            }),
        ));
        assert!(event.layers.replace_layer(
            "minecraft:contextual_info_bar",
            Box::new(|_, _| {
                assert_owner("editor");
                log("contextual replacement");
            }),
        ));
    }
    fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        if matches!(event.layer, "hud_test:bar" | "minecraft:effects") {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn take() -> Vec<String> {
    std::mem::take(&mut *LOG.lock().unwrap())
}

const PLAYER_HEALTH: &str = "minecraft:player_health";
const AIR_LEVEL: &str = "minecraft:air_level";
const BUBBLE_POP: &str = "ui.hud.bubble_pop";
// Equal RGB channels make the marker's packed vertex colour independent of channel order.
const HEALTH_MARKER: u32 = 0xFF2D_2D2D;

#[derive(Default)]
struct HealthTrace {
    cancel_gui: bool,
    cancel_health: bool,
    cancel_air: bool,
    calls: Vec<&'static str>,
}

struct HealthLayer {
    replace: bool,
    trace: Arc<Mutex<HealthTrace>>,
}

#[mod_api::__subscriptions]
impl Hooks for HealthLayer {
    fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
        if self.replace {
            let trace = self.trace.clone();
            assert!(event.layers.replace_layer(
                PLAYER_HEALTH,
                Box::new(move |_, draw| {
                    trace.lock().unwrap().calls.push("replacement");
                    draw.draw_gui(&mut |canvas| {
                        canvas.fill([80.0, 100.0, 70.0, 4.0], HEALTH_MARKER)
                    })
                    .unwrap();
                }),
            ));
        }
        let trace = self.trace.clone();
        assert!(event.layers.wrap_layer(
            AIR_LEVEL,
            Box::new(move |ctx| {
                trace.lock().unwrap().calls.push("air wrapper");
                ctx.draw_previous();
            }),
        ));
    }

    fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
        if self.trace.lock().unwrap().cancel_gui {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        let mut trace = self.trace.lock().unwrap();
        match event.layer {
            PLAYER_HEALTH => {
                trace.calls.push("health pre");
                if trace.cancel_health {
                    return Flow::Cancel;
                }
            }
            "minecraft:armor_level" => trace.calls.push("armor pre"),
            "minecraft:food_level" => trace.calls.push("food pre"),
            "minecraft:vehicle_health" => trace.calls.push("vehicle pre"),
            AIR_LEVEL => {
                trace.calls.push("air pre");
                if trace.cancel_air {
                    return Flow::Cancel;
                }
            }
            _ => {}
        }
        Flow::Continue
    }

    fn player_heart_type(&self, _: &mut dyn ClientHost, _: &mut PlayerHeartTypeEvent) {
        self.trace.lock().unwrap().calls.push("hearts");
    }

    fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
        let call = match event.layer {
            PLAYER_HEALTH => "health post",
            "minecraft:armor_level" => "armor post",
            "minecraft:food_level" => "food post",
            "minecraft:vehicle_health" => "vehicle post",
            AIR_LEVEL => "air post",
            _ => return,
        };
        self.trace.lock().unwrap().calls.push(call);
    }
}

#[test]
#[ignore = "daily"]
fn health_extraction_runs_only_in_the_vanilla_layer_body() {
    const CASE: &str = "SUNRISE_HEALTH_LAYER_FIXTURE";
    if run_fresh_cases(
        "cut_layout::game::client_side::gui_layer_tests::health_extraction_runs_only_in_the_vanilla_layer_body",
        CASE,
        &["vanilla", "replace"],
    ) {
        return;
    }
    let replace = std::env::var(CASE).unwrap() == "replace";
    let trace = Arc::new(Mutex::new(HealthTrace::default()));
    with_client_game(|game| {
        mod_platform::__private::install(
            vec![ModEntry::new(
                "health-test",
                Box::new(HealthLayer {
                    replace,
                    trace: trace.clone(),
                }),
            )]
            .into(),
        );
        sunrise_client_gameplay::game::client_side::mods_client::register_gui_layers(
            ExecutionRole::Client,
        );
        game.client.title.0 = None;
        game.client.screen = Screen::Playing;
        game.client.chat_ui.hide_gui = false;
        set_fixture_mode(game, GameMode::Survival);
        let mut player = PlayerMotion::new(game.motion, Vec3::new(2.0, 64.0, 2.0)).unwrap();
        player.set_health(12.0, false).unwrap();
        player
            .set_modifier(
                "minecraft:armor",
                "health-test:armor",
                2.0,
                "add_value",
                true,
            )
            .unwrap();
        // At 181/300, Hud.extractAirBubbles has six full bubbles and one popping.
        // Above water the same depleted air instead resets the pop latch, with no sound.
        player.air = 181;
        game.player = Some(player);
        for (under_water, creative, hidden) in [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (true, false, true),
        ] {
            set_fixture_mode(
                game,
                if creative {
                    GameMode::Creative
                } else {
                    GameMode::Survival
                },
            );
            game.client.chat_ui.hide_gui = hidden;
            if under_water && !game.player.as_ref().unwrap().is_eye_in_fluid(WATER) {
                let water = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:water")
                    .unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                for y in 64..=66 {
                    game.world.set(2, y, 2, water).unwrap();
                }
                let common = &mut game.common;
                let player = common.player.as_mut().unwrap();
                MotionWorld {
                    entities: &[],
                    player,
                    scene: &mut common.world,
                    assets: &common.pack.assets,
                    collision_binding: common.collision,
                    resources: common.motion,
                    health: &common.health_resources,
                    tag_items: None,
                    loop_context: &common.loop_context,
                    side: dry_side(),
                    damage_hooks: &common.damage_hooks,
                    block_registries: None,
                }
                .update_fluids()
                .unwrap();
                assert!(player.is_eye_in_fluid(WATER));
            }
            let vanilla_eligible = !creative && !hidden;
            let vetoes: &[(bool, bool)] = if !vanilla_eligible {
                &[(false, false)]
            } else {
                &[(true, false), (true, true), (false, false), (false, true)]
            };
            for &(cancel_health, cancel_air) in vetoes {
                game.client.health_hud.last_health = 17;
                game.client.health_hud.last_bubble_pop_sound = 3;
                game.client.audio.0.heard.remove(BUBBLE_POP);
                *trace.lock().unwrap() = HealthTrace {
                    cancel_health,
                    cancel_air,
                    ..HealthTrace::default()
                };
                let batch = game.hud_layers([640, 480]);
                let calls = std::mem::take(&mut trace.lock().unwrap().calls);
                let mut expected = vec!["health pre"];
                if !cancel_health {
                    if replace {
                        expected.push("replacement");
                    } else if vanilla_eligible {
                        expected.push("hearts");
                    }
                    expected.push("health post");
                }
                expected.extend([
                    "armor pre",
                    "armor post",
                    "food pre",
                    "food post",
                    "vehicle pre",
                    "vehicle post",
                    "air pre",
                ]);
                if !cancel_air {
                    expected.extend(["air wrapper", "air post"]);
                }
                assert_eq!(calls, expected);
                let painted_replacement = !cancel_health && replace;
                assert_eq!(
                    batch.vertices.iter().any(|v| v.color == HEALTH_MARKER),
                    painted_replacement,
                );
                if painted_replacement {
                    assert_eq!(game.client.chat_ui.room.bottom(), 100.0);
                } else if !vanilla_eligible {
                    assert_eq!(
                        game.client.chat_ui.room.bottom(),
                        batch.height,
                        "no vanilla status paint"
                    );
                } else {
                    assert!(game.client.chat_ui.room.bottom() > 100.0);
                }
                assert_eq!(
                    game.client.health_hud.last_health,
                    if cancel_health || replace || !vanilla_eligible {
                        17
                    } else {
                        12
                    }
                );
                let draws_air = !cancel_air && vanilla_eligible;
                let pops = draws_air && under_water;
                assert_eq!(
                    game.client.health_hud.last_bubble_pop_sound,
                    if !draws_air {
                        3
                    } else if pops {
                        7
                    } else {
                        0
                    }
                );
                // The sound is local-only; `heard` counts it even with the device disabled.
                assert_eq!(
                    game.client.audio.0.heard.get(BUBBLE_POP).copied(),
                    pops.then_some(1)
                );
            }
        }
        // The last frame hid the vanilla HUD. The global veto still applies to its slots.
        *trace.lock().unwrap() = HealthTrace {
            cancel_gui: true,
            ..HealthTrace::default()
        };
        let batch = game.hud_layers([640, 480]);
        assert!(trace.lock().unwrap().calls.is_empty());
        assert!(!batch.vertices.iter().any(|v| v.color == HEALTH_MARKER));
    });
}

#[test]
#[ignore = "daily"]
fn mod_layers_draw_in_their_place_and_hide_like_vanilla_layers() {
    // The layer order is kept once per process.
    if run_fresh_cases(
        "cut_layout::game::client_side::gui_layer_tests::mod_layers_draw_in_their_place_and_hide_like_vanilla_layers",
        "SUNRISE_GUI_LAYER_FIXTURE",
        &["run"],
    ) {
        return;
    }
    with_client_game(|game| {
        mod_platform::__private::install(
            vec![
                ModEntry::new("compass", Box::new(Compass)),
                ModEntry::new("inner", Box::new(Inner)),
                ModEntry::new("editor", Box::new(Editor)),
            ]
            .into(),
        );
        sunrise_client_gameplay::game::client_side::mods_client::register_gui_layers(
            ExecutionRole::Client,
        );
        let mut batch = HudBatch::new([320, 240], game.client.settings.gui_scale_inputs());

        // Simulate the HUD paint sites with the crosshair skipped (third person).
        // Its wrapper doesn't draw, but both neighboring layers still do.
        game.client.hud_layer_next = Some(0);
        for (layer, paint) in [
            (VanillaGuiLayer::Hotbar, "hotbar paint"),
            (VanillaGuiLayer::Effects, "effects paint"),
            (VanillaGuiLayer::PlayerHealth, "health paint"),
            (VanillaGuiLayer::ContextualInfoBar, "contextual paint"),
            (VanillaGuiLayer::Chat, "chat paint"),
        ] {
            game.gui_layer(layer, &mut batch, &mut |_, _| log(paint));
        }
        game.draw_mod_gui_layers(None, &mut batch);
        game.client.hud_layer_next = None;
        assert_eq!(current_native_mod(), None);
        assert_eq!(
            take(),
            [
                "pre hud_test:under_crosshair",
                "outer before",
                "inner before",
                "draw hud_test:under_crosshair",
                "inner after",
                "outer after",
                "post hud_test:under_crosshair",
                "pre hud_test:over_crosshair",
                "draw hud_test:over_crosshair",
                "post hud_test:over_crosshair",
                "pre minecraft:hotbar",
                "hotbar wrapper before",
                "hotbar paint",
                "hotbar wrapper after",
                "post minecraft:hotbar",
                "pre hud_test:bar",
                "pre minecraft:effects",
                "pre minecraft:player_health",
                "health paint",
                "post minecraft:player_health",
                "pre hud_test:above_health",
                "draw hud_test:above_health",
                "post hud_test:above_health",
                "pre hud_test:below_contextual",
                "draw hud_test:below_contextual",
                "post hud_test:below_contextual",
                "pre minecraft:contextual_info_bar",
                "contextual replacement",
                "post minecraft:contextual_info_bar",
                "pre minecraft:chat",
                "chat paint",
                "post minecraft:chat",
                "pre hud_test:top",
                "draw hud_test:top",
                "post hud_test:top",
            ]
        );
        // The outer wrapper and the underlying painter append into the same batch in order.
        let mut colors: Vec<_> = batch
            .vertices
            .iter()
            .map(|v| v.color.to_le_bytes())
            .collect();
        colors.dedup();
        assert_eq!(
            colors,
            [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]
        );

        // A vanilla layer still paints when the inserted-layer cursor is inactive.
        game.gui_layer(VanillaGuiLayer::SleepOverlay, &mut batch, &mut |_, _| {
            log("sleep paint")
        });
        game.draw_mod_gui_layers(None, &mut batch);
        assert_eq!(
            take(),
            [
                "pre minecraft:sleep_overlay",
                "sleep paint",
                "post minecraft:sleep_overlay"
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn failed_gui_layer_keeps_completed_paint_and_isolates_following_layers() {
    use mod_api::{
        __private::{HookPanicReport, take_hook_panics},
        GuiClipRefusal, GuiDrawError,
        client::hud_layout::PoseOp,
    };
    use sunrise_client::hud::rgba;

    fn color_bounds(batch: &HudBatch, color: u32) -> Option<[f32; 4]> {
        batch
            .vertices
            .iter()
            .filter(|v| v.color == color)
            .fold(None, |acc, v| {
                let [x0, y0, x1, y1] = acc.unwrap_or([v.pos[0], v.pos[1], v.pos[0], v.pos[1]]);
                Some([
                    x0.min(v.pos[0]),
                    y0.min(v.pos[1]),
                    x1.max(v.pos[0]),
                    y1.max(v.pos[1]),
                ])
            })
    }

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::gui_layer_tests::failed_gui_layer_keeps_completed_paint_and_isolates_following_layers",
        || {
            struct CheckedLayer;
            #[mod_api::__subscriptions]
            impl Hooks for CheckedLayer {
                fn register_gui_layers(
                    &self,
                    _: &RegistrationContext,
                    event: &mut RegisterGuiLayersEvent<'_>,
                ) {
                    assert!(event.layers.register(
                        "hud_test:checked".into(),
                        GuiLayerPlacement::AboveAll,
                        Box::new(|_, draw| {
                            // Pose stays lexical to draw_gui/scope; malformed draw/clip checks
                            // stay independent of layer containment.
                            assert_eq!(
                                draw.draw_gui(&mut |canvas| {
                                    canvas.scope(
                                        &[PoseOp::Translate(50.0, 50.0)],
                                        None,
                                        &mut |_| Ok(()),
                                    )
                                }),
                                Ok(())
                            );
                            assert_eq!(
                                draw.with_scissor([f32::NAN, 0.0, 1.0, 1.0], &mut |_| {
                                    unreachable!()
                                }),
                                Err(GuiClipRefusal::NotFinite)
                            );
                            draw.with_scissor([10.0, 10.0, 5.0, 5.0], &mut |draw| {
                                draw.fill([0.0, 0.0, 100.0, 100.0], 0xFFFF_0000);
                                assert_eq!(
                                    draw.draw_gui(&mut |canvas| {
                                        canvas.fill([10.0, 10.0, 5.0, 5.0], 0xFFFF_FF00)?;
                                        Err(GuiDrawError::Abort)
                                    }),
                                    Err(GuiDrawError::Abort)
                                );
                            })
                            .unwrap();
                            log("checked");
                        }),
                    ));
                }
            }

            struct PanickingLayer;
            #[mod_api::__subscriptions]
            impl Hooks for PanickingLayer {
                fn register_gui_layers(
                    &self,
                    _: &RegistrationContext,
                    event: &mut RegisterGuiLayersEvent<'_>,
                ) {
                    assert!(event.layers.register(
                        "hud_test:panicking".into(),
                        GuiLayerPlacement::AboveAll,
                        Box::new(|_, draw| {
                            let _ = draw.with_scissor([20.0, 20.0, 4.0, 4.0], &mut |draw| {
                                draw.fill([0.0, 0.0, 100.0, 100.0], 0xFF00_00FF);
                                log("panicking");
                                panic!("broken layer");
                            });
                        }),
                    ));
                }
            }

            struct NextLayer;
            #[mod_api::__subscriptions]
            impl Hooks for NextLayer {
                fn register_gui_layers(
                    &self,
                    _: &RegistrationContext,
                    event: &mut RegisterGuiLayersEvent<'_>,
                ) {
                    assert!(event.layers.register(
                        "hud_test:next".into(),
                        GuiLayerPlacement::AboveAll,
                        Box::new(|_, draw| {
                            draw.fill([0.0, 0.0, 100.0, 100.0], 0xFF00_FF00);
                            log("next");
                        }),
                    ));
                }
                fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
                    log(format!("post {}", event.layer));
                }
            }

            with_client_game(|game| {
                mod_platform::__private::install(
                    vec![
                        ModEntry::new("checked_layer", Box::new(CheckedLayer)),
                        ModEntry::new("panicking_layer", Box::new(PanickingLayer)),
                        ModEntry::new("next_layer", Box::new(NextLayer)),
                    ]
                    .into(),
                );
                sunrise_client_gameplay::game::client_side::mods_client::register_gui_layers(
                    ExecutionRole::Client,
                );
                let red = rgba(255, 0, 0, 255);
                let blue = rgba(0, 0, 255, 255);
                let green = rgba(0, 255, 0, 255);

                let mut batch = HudBatch::scratch([320, 240]);
                game.client.hud_layer_next = Some(0);
                game.draw_mod_gui_layers(None, &mut batch);
                assert_eq!(
                    take(),
                    [
                        "checked",
                        "post hud_test:checked",
                        "panicking",
                        "post hud_test:panicking",
                        "next",
                        "post hud_test:next",
                    ]
                );
                assert_eq!(
                    batch.vertices.iter().map(|v| v.color).collect::<Vec<_>>(),
                    [[red; 6], [blue; 6], [green; 6]].concat()
                );
                assert_eq!(color_bounds(&batch, red), Some([0.0, 0.0, 100.0, 100.0]));
                assert_eq!(color_bounds(&batch, blue), Some([0.0, 0.0, 100.0, 100.0]));
                assert_eq!(color_bounds(&batch, green), Some([0.0, 0.0, 100.0, 100.0]));
                assert_eq!(color_bounds(&batch, rgba(255, 255, 0, 255)), None);
                assert_eq!(
                    batch.scissors_in(0..18).collect::<Vec<_>>(),
                    [
                        (0..6, Some([10.0, 10.0, 15.0, 15.0])),
                        (6..12, Some([20.0, 20.0, 24.0, 24.0])),
                        (12..18, None),
                    ]
                );
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "panicking_layer",
                        hook: "gui_layer_draw",
                    }]
                );

                // The layer with a draw error still runs; the panicking layer stays disabled.
                let mut batch = HudBatch::scratch([320, 240]);
                game.client.hud_layer_next = Some(0);
                game.draw_mod_gui_layers(None, &mut batch);
                assert_eq!(
                    take(),
                    [
                        "checked",
                        "post hud_test:checked",
                        "post hud_test:panicking",
                        "next",
                        "post hud_test:next",
                    ]
                );
                assert_eq!(
                    batch.vertices.iter().map(|v| v.color).collect::<Vec<_>>(),
                    [[red; 6], [green; 6]].concat()
                );
                assert_eq!(color_bounds(&batch, red), Some([0.0, 0.0, 100.0, 100.0]));
                assert_eq!(color_bounds(&batch, blue), None);
                assert_eq!(color_bounds(&batch, green), Some([0.0, 0.0, 100.0, 100.0]));
                assert_eq!(
                    batch.scissors_in(0..12).collect::<Vec<_>>(),
                    [(0..6, Some([10.0, 10.0, 15.0, 15.0])), (6..12, None)]
                );
                assert!(take_hook_panics().is_empty());
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_gui_wrapper_panic_is_contained_but_delegated_engine_paint_is_fatal() {
    use mod_api::__private::{HookPanicReport, take_hook_panics};
    use sunrise_client::hud::{Blend, rgba};

    let test = "cut_layout::game::client_side::gui_layer_tests::a_gui_wrapper_panic_is_contained_but_delegated_engine_paint_is_fatal";
    sunrise_gameplay_common::game::in_own_process(test, || {
        struct Wrapper(&'static str, bool);
        #[mod_api::__subscriptions]
        impl Hooks for Wrapper {
            fn register_gui_layers(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterGuiLayersEvent<'_>,
            ) {
                let Self(id, broken) = *self;
                assert!(event.layers.wrap_layer(
                    "minecraft:hotbar",
                    Box::new(move |ctx| {
                        assert_owner(id);
                        log(id);
                        if broken {
                            ctx.with_draw(&mut |draw| draw.fill([0.0, 0.0, 4.0, 4.0], 0xFFFF_0000));
                            ctx.draw_previous();
                            ctx.with_draw(&mut |draw| draw.fill([0.0, 0.0, 4.0, 4.0], 0xFF00_00FF));
                            ctx.draw_previous();
                            ctx.with_draw(&mut |draw| {
                                let _ = draw.with_scissor([0.0, 0.0, 4.0, 4.0], &mut |draw| {
                                    draw.fill([0.0, 0.0, 20.0, 20.0], 0xFFFF_0000);
                                    panic!("broken wrapper");
                                });
                            });
                            return;
                        }
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            ctx.draw_previous();
                        }))
                        .is_err()
                        {
                            log(format!("caught by {id}"));
                        }
                        assert_owner(id);
                    }),
                ));
            }
        }
        with_client_game(|game| {
            mod_platform::__private::install(
                vec![
                    ModEntry::new("inner", Box::new(Wrapper("inner", false))),
                    ModEntry::new("broken", Box::new(Wrapper("broken", true))),
                    ModEntry::new("outer", Box::new(Wrapper("outer", false))),
                ]
                .into(),
            );
            sunrise_client_gameplay::game::client_side::mods_client::register_gui_layers(
                ExecutionRole::Client,
            );
            let mut batch = HudBatch::scratch([320, 240]);
            let white = rgba(255, 255, 255, 255);
            let red = rgba(255, 0, 0, 255);
            let blue = rgba(0, 0, 255, 255);
            let green = rgba(0, 255, 0, 255);
            let yellow = rgba(255, 255, 0, 255);
            let mut paint_calls = 0;
            batch.fill(&game.client.gui.atlas, 0.0, 0.0, 4.0, 4.0, white);
            game.gui_layer(VanillaGuiLayer::Hotbar, &mut batch, &mut |g, batch| {
                log("vanilla paint");
                let start = batch.vertices.len();
                let color = [green, yellow][paint_calls];
                paint_calls += 1;
                batch.fill(&g.client.gui.atlas, 0.0, 0.0, 4.0, 4.0, color);
                batch.invert_since(start);
            });
            assert_eq!(
                take(),
                [
                    "outer",
                    "broken",
                    "inner",
                    "vanilla paint",
                    "inner",
                    "vanilla paint"
                ]
            );
            assert_eq!(
                batch.vertices.iter().map(|v| v.color).collect::<Vec<_>>(),
                [
                    [white; 6],
                    [red; 6],
                    [green; 6],
                    [blue; 6],
                    [yellow; 6],
                    [red; 6],
                ]
                .concat()
            );
            assert_eq!(
                batch.scissors_in(0..36).collect::<Vec<_>>(),
                [(0..30, None), (30..36, Some([0.0, 0.0, 4.0, 4.0]))]
            );
            assert_eq!(
                batch.blends,
                [(12..18, Blend::Invert), (24..30, Blend::Invert)]
            );
            assert_eq!(
                take_hook_panics(),
                [HookPanicReport {
                    mod_id: "broken",
                    hook: "gui_layer_wrapper",
                }]
            );

            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.gui_layer(VanillaGuiLayer::Hotbar, &mut batch, &mut |_, _| {
                    assert_eq!(current_native_mod(), None);
                    assert!(!mod_api::__private::game::retained_callback_active());
                    panic!("vanilla paint failed");
                });
            }))
            .unwrap_err();
            assert_eq!(panic.downcast_ref::<&str>(), Some(&"vanilla paint failed"));
            assert_eq!(
                take(),
                ["outer", "inner", "caught by inner", "caught by outer"]
            );
            assert_eq!(current_native_mod(), None);
            assert!(take_hook_panics().is_empty());

            game.gui_layer(VanillaGuiLayer::Hotbar, &mut batch, &mut |_, _| {
                log("vanilla paint");
            });
            assert_eq!(take(), ["outer", "inner", "vanilla paint"]);
        });
    });
}

fn enter_test_world(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    game.client.title.0 = None;
    game.set_screen(Screen::Playing);
    game.player = Some(
        sunrise_sim::motion::PlayerMotion::new(
            game.motion,
            sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
        )
        .unwrap(),
    );
}

#[test]
#[ignore = "daily"]
fn hotbar_attack_indicator_progress_edits_only_stock_hotbar_paint() {
    use mod_api::{HotbarAttackIndicatorProgressEvent, ItemDecorationsEvent, ItemPart, Registrar};
    use sunrise_items::items::ItemStack;
    use sunrise_sim::mobs::Mob;

    const HOTBAR: &str = "minecraft:hotbar";
    const CASE: &str = "SUNRISE_HOTBAR_ATTACK_INDICATOR_FIXTURE";
    if run_fresh_cases(
        "cut_layout::game::client_side::gui_layer_tests::hotbar_attack_indicator_progress_edits_only_stock_hotbar_paint",
        CASE,
        &["stock", "replace", "delegate", "skip"],
    ) {
        return;
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Body {
        Stock,
        Replace,
        Delegate,
        Skip,
    }
    let body = match std::env::var(CASE).unwrap().as_str() {
        "stock" => Body::Stock,
        "replace" => Body::Replace,
        "delegate" => Body::Delegate,
        "skip" => Body::Skip,
        case => panic!("unknown hotbar fixture: {case}"),
    };

    #[derive(Debug, PartialEq)]
    enum Call {
        Pre,
        Before,
        Items,
        Progress(f32),
        After,
        Replaced,
        Post,
    }

    #[derive(Default)]
    struct Trace {
        cancel: bool,
        proposals: [Option<f32>; 2],
        calls: Vec<Call>,
    }

    #[derive(Clone)]
    struct Decoration(Arc<Mutex<Trace>>);

    #[mod_api::sunrise_part]
    impl ItemPart for Decoration {
        fn item_decorations(&self, _: &dyn ClientHost, _: &mut ItemDecorationsEvent<'_>) {
            self.0.lock().unwrap().calls.push(Call::Items);
        }
    }

    struct Layer {
        body: Body,
        trace: Arc<Mutex<Trace>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Layer {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let key = r.item_key("minecraft:stick");
            r.item_part(key, Decoration(self.trace.clone()));
        }

        fn register_gui_layers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterGuiLayersEvent<'_>,
        ) {
            let trace = self.trace.clone();
            match self.body {
                Body::Stock => {}
                Body::Replace => assert!(event.layers.replace_layer(
                    HOTBAR,
                    Box::new(move |_, _| trace.lock().unwrap().calls.push(Call::Replaced)),
                )),
                Body::Delegate | Body::Skip => {
                    let delegate = self.body == Body::Delegate;
                    assert!(event.layers.wrap_layer(
                        HOTBAR,
                        Box::new(move |ctx| {
                            trace.lock().unwrap().calls.push(Call::Before);
                            if delegate {
                                ctx.draw_previous();
                            }
                            trace.lock().unwrap().calls.push(Call::After);
                        }),
                    ));
                }
            }
        }

        fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
            if event.layer == HOTBAR {
                let mut trace = self.trace.lock().unwrap();
                trace.calls.push(Call::Pre);
                if trace.cancel {
                    return Flow::Cancel;
                }
            }
            Flow::Continue
        }

        fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
            if event.layer == HOTBAR {
                self.trace.lock().unwrap().calls.push(Call::Post);
            }
        }
    }

    struct Listener {
        proposal: Option<usize>,
        trace: Arc<Mutex<Trace>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn hotbar_attack_indicator_progress(
            &self,
            _: &dyn ClientHost,
            event: &mut HotbarAttackIndicatorProgressEvent,
        ) {
            let mut trace = self.trace.lock().unwrap();
            trace.calls.push(Call::Progress(event.progress));
            if let Some(value) = self.proposal.and_then(|i| trace.proposals[i]) {
                event.progress = value;
            }
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Variant {
        Stock,
        Crosshair,
        Off,
        Hidden,
        Spectator,
        Mob,
        Veto,
    }

    #[derive(Clone, Copy)]
    struct Frame {
        name: &'static str,
        ticker: i32,
        variant: Variant,
        proposals: [Option<f32>; 2],
        seen: [f32; 3],
        fill: Option<i32>,
    }

    impl Frame {
        fn edit(
            name: &'static str,
            ticker: i32,
            proposals: [Option<f32>; 2],
            seen: [f32; 3],
            fill: Option<i32>,
        ) -> Self {
            Self {
                name,
                ticker,
                proposals,
                seen,
                fill,
                ..PARTIAL
            }
        }

        fn gated(name: &'static str, variant: Variant) -> Self {
            Self {
                name,
                variant,
                fill: None,
                ..PARTIAL
            }
        }
    }

    // The local player's 4.0 base attack speed gives five ticks to full charge.
    const PARTIAL: Frame = Frame {
        name: "partial",
        ticker: 2,
        variant: Variant::Stock,
        proposals: [None, None],
        seen: [0.4; 3],
        fill: Some(7),
    };

    let trace = Arc::new(Mutex::new(Trace::default()));
    let mut entries = vec![ModEntry::new(
        "hotbar-layer",
        Box::new(Layer {
            body,
            trace: trace.clone(),
        }),
    )];
    entries.extend(
        [
            ("first-edit", Some(0)),
            ("second-edit", Some(1)),
            ("observer", None),
        ]
        .map(|(id, proposal)| {
            ModEntry::new(
                id,
                Box::new(Listener {
                    proposal,
                    trace: trace.clone(),
                }),
            )
        }),
    );
    // Install before the game fixture snapshots registrations and resolves the stick's Part.
    mod_platform::__private::install(entries.into());
    with_client_game(|game| {
        sunrise_client_gameplay::game::client_side::mods_client::register_gui_layers(
            ExecutionRole::Client,
        );
        enter_test_world(game);
        game.client.settings.main_hand_left = false;
        game.selected = 0;
        game.inventory[1] = Some(ItemStack::new("minecraft:stick", 1));
        game.clock
            .advance(std::time::Duration::from_millis(25), 50.0);
        assert_eq!(game.clock.fraction(), 0.5, "exercise a draw between ticks");

        let mut frames = vec![PARTIAL, Frame::gated("Pre veto", Variant::Veto)];
        if body == Body::Stock {
            frames.extend(
                [
                    ("NaN", f32::NAN),
                    ("+infinity", f32::INFINITY),
                    ("-infinity", f32::NEG_INFINITY),
                    ("above full", 1.25),
                    ("negative", -0.25),
                ]
                .map(|(name, invalid)| Frame {
                    name,
                    proposals: [Some(invalid), None],
                    ..PARTIAL
                }),
            );
            frames.extend([
                Frame::edit("full charge", 5, [None; 2], [1.0; 3], None),
                Frame::edit(
                    "full, edited then refused",
                    5,
                    [Some(0.5), Some(f32::NAN)],
                    [1.0, 0.5, 0.5],
                    Some(9),
                ),
                Frame::edit(
                    "edited to full",
                    2,
                    [Some(1.0), None],
                    [0.4, 1.0, 1.0],
                    None,
                ),
                Frame::edit(
                    "edited to zero",
                    2,
                    [Some(0.0), None],
                    [0.4, 0.0, 0.0],
                    Some(0),
                ),
                Frame {
                    proposals: [Some(1.0), None],
                    ..Frame::gated("Crosshair", Variant::Crosshair)
                },
                Frame::gated("Off", Variant::Off),
                Frame::gated("F1", Variant::Hidden),
                Frame::gated("spectator", Variant::Spectator),
                Frame::gated("non-player camera", Variant::Mob),
            ]);
        }

        // Only the non-player camera case needs a connection, and it runs last.
        let mut server = None;
        for frame in frames {
            if frame.variant == Variant::Mob {
                server = Some(sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(game));
                game.client.remote.as_mut().unwrap().level_load.finish();
                enter_test_world(game);
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                game.mobs.push(Mob::new(31, cow, Vec3::ZERO, 0.0));
                game.remote_set_camera(31);
                assert_eq!(
                    game.camera_player(),
                    sunrise_client_gameplay::game::remote_camera::CameraPlayer::Absent
                );
            }
            let mode = if frame.variant == Variant::Spectator {
                GameMode::Spectator
            } else {
                GameMode::Survival
            };
            // A client of a server takes its mode from the server: only its local mode changes.
            if game.client.remote.is_some() {
                game.set_local_mode(mode);
            } else {
                set_fixture_mode(game, mode);
            }
            game.client.chat_ui.hide_gui = frame.variant == Variant::Hidden;
            game.client.settings.attack_indicator = match frame.variant {
                Variant::Off => 0,
                Variant::Crosshair => 1,
                _ => 2,
            };
            game.client.hand_strength.ticker = frame.ticker;
            let cancel = frame.variant == Variant::Veto;
            *trace.lock().unwrap() = Trace {
                cancel,
                proposals: frame.proposals,
                calls: Vec::new(),
            };

            let batch = game.hud_layers([640, 480]);
            let stock = !cancel
                && matches!(body, Body::Stock | Body::Delegate)
                && matches!(
                    frame.variant,
                    Variant::Stock | Variant::Crosshair | Variant::Off
                );
            let mut expected = vec![Call::Pre];
            if !cancel {
                if matches!(body, Body::Delegate | Body::Skip) {
                    expected.push(Call::Before);
                }
                if stock {
                    expected.push(Call::Items);
                    if frame.variant == Variant::Stock {
                        expected.extend(frame.seen.map(Call::Progress));
                    }
                }
                match body {
                    Body::Delegate | Body::Skip => expected.push(Call::After),
                    Body::Replace => expected.push(Call::Replaced),
                    Body::Stock => {}
                }
                expected.push(Call::Post);
            }
            assert_eq!(
                trace.lock().unwrap().calls,
                expected,
                "{body:?}: {}",
                frame.name
            );
            assert_eq!(
                game.client.hand_strength.ticker, frame.ticker,
                "paint must not change attack charge"
            );

            // Read screen and source rectangles, keyed by the sprite atlas coordinates.
            type Quad = ([[f32; 2]; 2], [[i32; 2]; 2]);
            let assert_sprite = |sprite, expected: Option<Quad>| {
                let r = *game.client.gui.atlas.gui_rect(sprite).unwrap();
                // Integer atlas pixels come back exactly once the f32 UV quantisation is rounded off.
                let source = |[u, v]: [f32; 2]| {
                    [
                        ((u - r.u0) / (r.u1 - r.u0) * r.w as f32).round() as i32,
                        ((v - r.v0) / (r.v1 - r.v0) * r.h as f32).round() as i32,
                    ]
                };
                let actual: Vec<_> = batch
                    .vertices
                    .as_chunks::<6>()
                    .0
                    .iter()
                    .filter_map(|q| {
                        let [u, v] = q[0].uv;
                        if (r.u0..=r.u1).contains(&u)
                            && (r.v0..=r.v1).contains(&v)
                            && q[0].pos[1] != q[2].pos[1]
                        {
                            Some(([q[0].pos, q[2].pos], [source(q[0].uv), source(q[2].uv)]))
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(
                    actual.as_slice(),
                    expected.as_slice(),
                    "{body:?}: {}, {sprite}",
                    frame.name
                );
            };

            let mid = (batch.width as i32 / 2) as f32;
            let (x, y) = (mid + 97.0, batch.height - 20.0);
            let fill = frame.fill.filter(|_| stock);
            assert_sprite(
                "hud/hotbar_attack_indicator_background",
                fill.map(|_| ([[x, y], [x + 18.0, y + 18.0]], [[0, 0], [18, 18]])),
            );
            assert_sprite(
                "hud/hotbar_attack_indicator_progress",
                fill.filter(|&fill| fill > 0).map(|fill| {
                    (
                        [[x, y + (18 - fill) as f32], [x + 18.0, y + 18.0]],
                        [[0, 18 - fill], [18, 18]],
                    )
                }),
            );

            let (x, y) = (mid - 8.0, (batch.height as i32 / 2) as f32 + 9.0);
            let crosshair = frame.variant == Variant::Crosshair;
            assert_sprite(
                "hud/crosshair_attack_indicator_background",
                crosshair.then_some(([[x, y], [x + 16.0, y + 4.0]], [[0, 0], [16, 4]])),
            );
            assert_sprite(
                "hud/crosshair_attack_indicator_progress",
                crosshair.then_some(([[x, y], [x + 6.0, y + 4.0]], [[0, 0], [6, 4]])),
            );
        }
        drop(server);
    });
}

#[test]
#[ignore = "daily"]
fn f1_hides_vanilla_paint_without_skipping_the_hud_walk() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use sunrise_client::{chat::ChatSource, hud::rgba};
    use sunrise_game_data::text::{Style, Text};

    let test = "cut_layout::game::client_side::gui_layer_tests::f1_hides_vanilla_paint_without_skipping_the_hud_walk";
    sunrise_gameplay_common::game::in_own_process(test, || {
        const MOD_PAINT: u32 = 0xFF13_579B;
        static CANCEL_HUD: AtomicBool = AtomicBool::new(false);

        struct Observer;
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
                log("hud pre");
                if CANCEL_HUD.load(Ordering::Relaxed) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }

            fn register_gui_layers(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterGuiLayersEvent<'_>,
            ) {
                assert!(event.layers.register(
                    "hud_test:indicator".into(),
                    GuiLayerPlacement::AboveAll,
                    Box::new(|_, draw| {
                        log("indicator paint");
                        draw.fill([14.0, 18.0, 4.0, 4.0], MOD_PAINT);
                    }),
                ));
                assert!(event.layers.wrap_layer(
                    "minecraft:chat",
                    Box::new(|ctx| {
                        log("chat wrapper");
                        ctx.draw_previous();
                    }),
                ));
            }

            fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
                log(format!("pre {}", event.layer));
                Flow::Continue
            }

            fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
                log(format!("post {}", event.layer));
            }

            fn render_gui_post(&self, _: &dyn ClientHost, _: &mut RenderGuiPostEvent<'_>) {
                log("hud post");
            }

            fn screen_render_pre(
                &self,
                _: &dyn ClientHost,
                _: &mut ScreenRenderPreEvent<'_>,
            ) -> Flow {
                log("screen pre");
                Flow::Continue
            }

            fn screen_render_background(
                &self,
                _: &dyn ClientHost,
                _: &mut ScreenRenderBackgroundEvent<'_>,
            ) {
                log("screen background");
            }

            fn screen_render_post(&self, _: &dyn ClientHost, _: &mut ScreenRenderPostEvent<'_>) {
                log("screen post");
            }
        }

        with_client_game(|game| {
            mod_platform::__private::install(
                vec![ModEntry::new("observer", Box::new(Observer))].into(),
            );
            sunrise_client_gameplay::game::client_side::mods_client::register_gui_layers(
                ExecutionRole::Client,
            );
            let size = [640, 480];
            let screen_events = ["screen pre", "screen background", "screen post"];
            let saves = tempfile::tempdir().unwrap();
            game.client.title.0 = Some(game.new_title(saves.path().to_path_buf()));
            game.open_options(true);
            game.hud_layers(size);
            assert_eq!(take(), screen_events, "title Options has no HUD walk");

            enter_test_world(game);
            let rect = game.client.gui.atlas.gui_rect("hud/hotbar").unwrap();
            let hotbar_uv = [rect.u0, rect.v0];
            let has_hotbar = |frame: &HudBatch| {
                frame
                    .vertices
                    .iter()
                    .any(|v| v.uv == hotbar_uv && v.pos[1] == frame.height - 22.0)
            };
            let has_indicator = |frame: &HudBatch| {
                frame
                    .vertices
                    .iter()
                    .any(|v| v.color == sunrise_client::hud_layout::abgr(MOD_PAINT))
            };
            let mut expected = vec!["hud pre".to_owned()];
            // NeoForge visits hidden and visible slots the same way. The literal IDs also
            // cover normally absent vanilla bodies, e.g. vehicle health with no mount.
            for id in [
                "crosshair",
                "hotbar",
                "effects",
                "boss_overlay",
                "player_health",
                "armor_level",
                "food_level",
                "vehicle_health",
                "air_level",
                "contextual_info_bar",
                "selected_item_name",
                "sleep_overlay",
                "overlay_message",
                "title",
                "scoreboard_sidebar",
                "subtitle_overlay",
                "chat",
                "tab_list",
            ] {
                expected.push(format!("pre minecraft:{id}"));
                if id == "chat" {
                    expected.push("chat wrapper".to_owned());
                }
                expected.push(format!("post minecraft:{id}"));
            }
            expected.extend(
                [
                    "pre hud_test:indicator",
                    "indicator paint",
                    "post hud_test:indicator",
                    "hud post",
                ]
                .map(str::to_owned),
            );

            let chat_paint = |frame: &HudBatch| {
                frame
                    .vertices
                    .iter()
                    .position(|v| v.color & 0x00FF_FFFF == rgba(0x27, 0xB4, 0xD9, 0))
                    .map(|first_paint| first_paint >= frame.blur_at as usize)
            };
            for screen in [Screen::Playing, Screen::Options, Screen::Chat] {
                let after_hud: &[&str] = match screen {
                    Screen::Options => {
                        game.set_screen(Screen::Paused);
                        game.open_options(false);
                        &screen_events
                    }
                    Screen::Chat => {
                        game.set_screen(Screen::Playing);
                        game.begin_chat_screen("", false);
                        assert!(game.add_chat_line(
                            Text::styled("Focused chat", Style::color(0x27_B4_D9)),
                            ChatSource::Client
                        ));
                        &screen_events
                    }
                    _ => &[],
                };
                let expected: Vec<_> = expected
                    .iter()
                    .map(String::as_str)
                    .chain(after_hud.iter().copied())
                    .collect();
                for hidden in [false, true] {
                    game.client.chat_ui.hide_gui = hidden;
                    let frame = game.hud_layers(size);
                    assert_eq!(take(), expected, "{screen:?}, F1 hidden: {hidden}");
                    assert_eq!(has_hotbar(&frame), !hidden);
                    assert!(has_indicator(&frame));
                    assert_eq!(chat_paint(&frame), (screen == Screen::Chat).then_some(true));
                }
            }
            CANCEL_HUD.store(true, Ordering::Relaxed);
            let frame = game.hud_layers(size);
            assert_eq!(
                take(),
                ["hud pre", "screen pre", "screen background", "screen post"]
            );
            assert!(!has_hotbar(&frame));
            assert!(!has_indicator(&frame));
            assert_eq!(
                chat_paint(&frame),
                Some(true),
                "HUD cancellation leaves focused chat on the screen"
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn f1_subtitles_paint_in_the_container_background_without_redispatch() {
    use sunrise_client::{hud::rgba, options_file::V, sound_engine::Notification};

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::gui_layer_tests::f1_subtitles_paint_in_the_container_background_without_redispatch",
        || {
            const MARKER: u32 = 0xFFAE_2818;
            struct Observer;
            #[mod_api::__subscriptions]
            impl Hooks for Observer {
                fn gui_layer_pre(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut GuiLayerPreEvent<'_>,
                ) -> Flow {
                    if event.layer == "minecraft:subtitle_overlay" {
                        log("subtitle pre");
                    }
                    Flow::Continue
                }
                fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
                    if event.layer == "minecraft:subtitle_overlay" {
                        log("subtitle post");
                    }
                }
                fn screen_render_background(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut ScreenRenderBackgroundEvent<'_>,
                ) {
                    event.draw.fill([0.0, 0.0, 1.0, 1.0], MARKER);
                }
            }

            with_client_game(|game| {
                mod_platform::__private::install(
                    vec![ModEntry::new("observer", Box::new(Observer))].into(),
                );
                enter_test_world(game);
                game.client.settings.show_subtitles = true;
                game.client
                    .settings
                    .file
                    .set("backgroundForChatOnly", V::Bool(false));
                game.client.settings.text_background_opacity = 0.4;
                let size = [640, 480];
                game.hud(size); // Start listening before the sound plays.
                assert_eq!(take(), ["subtitle pre", "subtitle post"]);
                game.client.audio.0.engine.notifications.push(Notification {
                    identifier: "minecraft:entity.player.levelup".into(),
                    subtitle: Some("subtitles.entity.player.levelup".into()),
                    position: game.client.view.position,
                    relative: false,
                    looping: false,
                    range: 16.0,
                });
                game.tick_subtitles();
                game.toggle_hide_gui();
                let is_subtitle =
                    |v: &sunrise_client::hud::HudVertex| v.color == rgba(0, 0, 0, 102);

                let frame = game.hud(size);
                assert!(
                    !frame.vertices.iter().any(is_subtitle),
                    "F1 without a screen"
                );
                assert_eq!(take(), ["subtitle pre", "subtitle post"]);

                game.set_screen(Screen::Inventory);
                let frame = game.hud(size);
                let mut subtitle = frame
                    .vertices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| is_subtitle(v));
                let (first, v) = subtitle
                    .next()
                    .expect("F1 keeps subtitles in the inventory");
                assert_eq!(v.pos[1], frame.height - 40.0);
                assert_eq!(subtitle.count(), 5, "one subtitle backdrop quad");
                let dim = frame
                    .vertices
                    .iter()
                    .position(|v| v.color == rgba(16, 16, 16, 160))
                    .unwrap();
                let screen = frame
                    .vertices
                    .iter()
                    .position(|v| v.color == sunrise_client::hud_layout::abgr(MARKER))
                    .unwrap();
                assert!(
                    dim < first && first < screen,
                    "subtitles draw over the dimming before screen widgets"
                );
                assert_eq!(take(), ["subtitle pre", "subtitle post"]);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn options_controller_notice_paints_after_screen_post_despite_hud_cancel() {
    use sunrise_client::{controller_notice::Notice, hud::rgba, hud_layout::abgr};

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::gui_layer_tests::options_controller_notice_paints_after_screen_post_despite_hud_cancel",
        || {
            const MARKER: u32 = 0xFFAE_2818;
            struct CancelHud;
            #[mod_api::__subscriptions]
            impl Hooks for CancelHud {
                fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
                    Flow::Cancel
                }
                fn screen_render_post(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut ScreenRenderPostEvent<'_>,
                ) {
                    event.draw.fill([0.0, 0.0, 1.0, 1.0], MARKER);
                }
            }

            with_client_game(|game| {
                mod_platform::__private::install(
                    vec![ModEntry::new("cancel_hud", Box::new(CancelHud))].into(),
                );
                enter_test_world(game);
                game.toggle_hide_gui();
                game.set_screen(Screen::Paused);
                game.open_options(false);
                game.client.controller_notice.raise(
                    Notice::new("Controller disconnected", "Use the keyboard"),
                    sunrise_client::toasts::ToastClock::Real,
                );
                let frame = game.hud([640, 480]);
                let post = frame
                    .vertices
                    .iter()
                    .position(|v| v.color == abgr(MARKER))
                    .unwrap();
                // The notice plate's 1px border is emitted even at the start of the slide.
                let notice = frame
                    .vertices
                    .iter()
                    .position(|v| v.color == rgba(255, 255, 255, 0x5A))
                    .unwrap();
                assert!(notice > post);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn panicking_hud_listeners_under_f1_disable_only_themselves_and_report_their_owner() {
    use std::time::{Duration, Instant};
    use sunrise_client::hud::Rect;

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::gui_layer_tests::panicking_hud_listeners_under_f1_disable_only_themselves_and_report_their_owner",
        || {
            const OWNER: &str = "broken_hud_listener";
            struct Broken;
            #[mod_api::__subscriptions]
            impl Hooks for Broken {
                fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
                    log("broken hud pre");
                    panic!("HUD pre failed");
                }
                fn gui_layer_pre(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut GuiLayerPreEvent<'_>,
                ) -> Flow {
                    if event.layer == "minecraft:crosshair" {
                        log("broken crosshair pre");
                        panic!("HUD layer pre failed");
                    }
                    Flow::Continue
                }
                fn render_gui_post(&self, _: &dyn ClientHost, _: &mut RenderGuiPostEvent<'_>) {
                    log("same mod hud post");
                }
            }
            struct Observer;
            #[mod_api::__subscriptions]
            impl Hooks for Observer {
                fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
                    log("observer hud pre");
                    Flow::Continue
                }
                fn gui_layer_pre(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut GuiLayerPreEvent<'_>,
                ) -> Flow {
                    if event.layer == "minecraft:crosshair" {
                        log("observer crosshair pre");
                    }
                    Flow::Continue
                }
            }

            with_client_game(|game| {
                mod_platform::__private::install(
                    vec![
                        ModEntry::new(OWNER, Box::new(Broken)),
                        ModEntry::new("observing_hud_listener", Box::new(Observer)),
                    ]
                    .into(),
                );
                enter_test_world(game);
                game.toggle_hide_gui();
                assert!(game.client.chat_ui.player_hides_gui && game.client.chat_ui.hide_gui);
                let size = [640, 480];
                let _ = game.hud(size);
                assert_eq!(
                    take(),
                    [
                        "broken hud pre",
                        "observer hud pre",
                        "broken crosshair pre",
                        "observer crosshair pre",
                        "same mod hud post"
                    ]
                );
                let _ = game.hud(size); // Start both slides after promotion.
                assert_eq!(
                    take(),
                    [
                        "observer hud pre",
                        "observer crosshair pre",
                        "same mod hud post"
                    ]
                );
                let [whole, layer] = game.client.toast_manager.visible.as_mut_slice() else {
                    panic!("one shared error toast per panicking listener");
                };
                let rect = *game
                    .client
                    .gui
                    .atlas
                    .sprites
                    .entry("toast/system".into())
                    .or_insert(Rect {
                        u0: 0.25,
                        v0: 0.25,
                        u1: 0.5,
                        v1: 0.5,
                        w: 160,
                        h: 32,
                    });
                let y =
                    [(whole, "render_gui_pre"), (layer, "gui_layer_pre")].map(|(entry, hook)| {
                        let sunrise_client_gameplay::game::toast_manager::Toast::Diagnostic {
                            toast,
                            ..
                        } = &mut entry.toast
                        else {
                            panic!("expected panic toast")
                        };
                        let message = toast.message.as_ref().unwrap().component();
                        assert_eq!(
                            message,
                            serde_json::json!({
                                "translate": "sunrise.mods.hook_panic.description",
                                "with": [{"text": OWNER}, {"text": hook}],
                            })
                        );
                        toast.slide.origin = Some(Instant::now() - Duration::from_secs(1));
                        32.0 * entry.slot as f32
                    });
                let frame = game.hud(size);
                for y in y {
                    assert!(
                        frame.vertices.iter().any(|v| v.uv == [rect.u0, rect.v0]
                            && v.pos[1] == y
                            && v.pos[0] < frame.width),
                        "F1 must not hide the shared callback error toast"
                    );
                }
                assert_eq!(
                    take(),
                    [
                        "observer hud pre",
                        "observer crosshair pre",
                        "same mod hud post"
                    ]
                );
                assert_eq!(game.client.toast_manager.visible.len(), 2);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_panicking_hud_painter_shows_its_error_toast_under_f1() {
    use std::time::{Duration, Instant};
    use sunrise_client::toasts::SystemToast;
    use sunrise_game_data::text::Text;

    const OWNER: &str = "broken_hud";
    let test = "cut_layout::game::client_side::gui_layer_tests::a_panicking_hud_painter_shows_its_error_toast_under_f1";
    sunrise_gameplay_common::game::in_own_process(test, || {
        struct BrokenHud;
        #[mod_api::__subscriptions]
        impl Hooks for BrokenHud {
            fn register_gui_layers(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterGuiLayersEvent<'_>,
            ) {
                assert!(event.layers.register(
                    "hud_test:broken".into(),
                    GuiLayerPlacement::AboveAll,
                    Box::new(|_, _| {
                        log("broken painter called");
                        panic!("broken HUD painter");
                    }),
                ));
            }
        }

        with_client_game(|game| {
            mod_platform::__private::install(
                vec![ModEntry::new(OWNER, Box::new(BrokenHud))].into(),
            );
            sunrise_client_gameplay::game::client_side::mods_client::register_gui_layers(
                ExecutionRole::Client,
            );
            enter_test_world(game);
            game.toggle_hide_gui();
            assert!(game.client.chat_ui.player_hides_gui && game.client.chat_ui.hide_gui);
            use {
                sunrise_client_gameplay::game::toast_manager::SystemToastId,
                sunrise_client_gameplay::game::toast_manager::Toast,
            };
            game.add_system_toast(
                Some(SystemToastId::PackLoadFailure),
                SystemToast::new(Text::tr("resourcePack.load_fail", &[]), None),
            );
            let size = [640, 480];
            let _ = game.hud(size);

            // Backdate the slide to test fully visible toasts without waiting for animation.
            let slide_started = Instant::now() - Duration::from_millis(700);
            let _ = game.hud(size); // Start both slides after promotion.
            let [ordinary, error] = game.client.toast_manager.visible.as_mut_slice() else {
                panic!("expected an ordinary toast and one panic diagnostic");
            };
            let Toast::Diagnostic { toast, .. } = &mut error.toast else {
                panic!("expected panic toast")
            };
            assert_eq!(
                toast.title.translation_key(),
                Some("sunrise.mods.hook_panic.title")
            );
            assert_eq!(
                toast.message.as_ref().unwrap().component()["with"][0]["text"],
                OWNER
            );
            let error_y = 32.0 * error.slot as f32;
            toast.slide.origin = Some(slide_started);
            let ordinary_y = 32.0 * ordinary.slot as f32;
            let Toast::System(_, toast) = &mut ordinary.toast else {
                panic!("expected ordinary toast")
            };
            assert_ne!(ordinary_y, error_y);
            toast.slide.origin = Some(slide_started);
            let rect = *game
                .client
                .gui
                .atlas
                .sprites
                .entry("toast/system".into())
                .or_insert(sunrise_client::hud::Rect {
                    u0: 0.25,
                    v0: 0.25,
                    u1: 0.5,
                    v1: 0.5,
                    w: 160,
                    h: 32,
                });

            let frame = game.hud(size);
            let has_toast = |y| {
                frame
                    .vertices
                    .iter()
                    .any(|v| v.uv == [rect.u0, rect.v0] && v.pos[1] == y && v.pos[0] < frame.width)
            };
            assert!(
                has_toast(error_y),
                "F1 must not hide the callback error toast"
            );
            assert!(!has_toast(ordinary_y), "F1 still hides ordinary toasts");
            assert_eq!(
                take(),
                ["broken painter called"],
                "a panic disables that painter"
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn render_gui_post_runs_after_hud_layers_respects_hiding_and_exposes_layer_bounds() {
    use mod_api::LevelReadError;
    use std::sync::atomic::{AtomicBool, Ordering};

    let test = "cut_layout::game::client_side::gui_layer_tests::render_gui_post_runs_after_hud_layers_respects_hiding_and_exposes_layer_bounds";
    sunrise_gameplay_common::game::in_own_process(test, || {
        static CANCEL_PRE: AtomicBool = AtomicBool::new(false);
        type Bounds = Result<Option<[f32; 4]>, LevelReadError>;
        static HUD_BOUNDS: Mutex<Option<[Bounds; 4]>> = Mutex::new(None);
        static SCREEN_BOUNDS: Mutex<Option<Bounds>> = Mutex::new(None);

        struct PostHudObserver;
        #[mod_api::__subscriptions]
        impl Hooks for PostHudObserver {
            fn register_gui_layers(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterGuiLayersEvent<'_>,
            ) {
                assert!(event.layers.register(
                    "post_test:top".into(),
                    GuiLayerPlacement::AboveAll,
                    Box::new(|_, draw| {
                        log("layer post_test:top");
                        draw.fill([12.0, 18.0, 20.0, 10.0], 0xFF00_FF00);
                    }),
                ));
            }
            fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
                log("render_gui_pre");
                if CANCEL_PRE.load(Ordering::Relaxed) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
            fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
                if matches!(
                    event.layer,
                    "minecraft:boss_overlay"
                        | "minecraft:sleep_overlay"
                        | "minecraft:subtitle_overlay"
                        | "minecraft:tab_list"
                        | "post_test:top"
                ) {
                    log(format!("layer_post {}", event.layer));
                }
            }
            fn render_gui_post(&self, host: &dyn ClientHost, event: &mut RenderGuiPostEvent<'_>) {
                log("render_gui_post");
                *HUD_BOUNDS.lock().unwrap() = Some([
                    host.hud_layer_bounds("minecraft:effects"),
                    host.hud_layer_bounds("minecraft:sleep_overlay"),
                    host.hud_layer_bounds("post_test:top"),
                    host.hud_layer_bounds("post_test:missing"),
                ]);
                event.draw.fill([4.0, 4.0, 8.0, 8.0], 0xFFFF_00FF);
            }
            fn screen_render_pre(
                &self,
                _: &dyn ClientHost,
                event: &mut ScreenRenderPreEvent<'_>,
            ) -> Flow {
                log(format!("screen_pre {}", event.screen.id));
                Flow::Continue
            }
            fn screen_render_post(
                &self,
                host: &dyn ClientHost,
                event: &mut ScreenRenderPostEvent<'_>,
            ) {
                *SCREEN_BOUNDS.lock().unwrap() = Some(host.hud_layer_bounds("post_test:top"));
                log(format!("screen_post {}", event.screen.id));
            }
        }

        with_client_game(|game| {
            mod_platform::__private::install(
                vec![ModEntry::new("post_hud", Box::new(PostHudObserver))].into(),
            );
            sunrise_client_gameplay::game::client_side::mods_client::register_gui_layers(
                ExecutionRole::Client,
            );
            enter_test_world(game);
            game.set_screen(Screen::Paused);
            let speed = sunrise_sim::brewing::effect("speed")
                .expect("vanilla speed effect")
                .id;
            game.potions
                .effects
                .add(sunrise_sim::effects::EffectInstance::new(speed, 200, 0));

            // Post comes after every HUD layer, but before even ScreenEvent.Render.Pre.
            let _ = game.hud([320, 240]);
            assert_eq!(
                take(),
                [
                    "render_gui_pre",
                    "layer_post minecraft:boss_overlay",
                    "layer_post minecraft:sleep_overlay",
                    "layer_post minecraft:subtitle_overlay",
                    "layer_post minecraft:tab_list",
                    "layer post_test:top",
                    "layer_post post_test:top",
                    "render_gui_post",
                    "screen_pre paused",
                    "screen_post paused",
                ]
            );
            let [effects, sleep, custom, missing] = HUD_BOUNDS.lock().unwrap().take().unwrap();
            assert_eq!(effects, Ok(Some([295.0, 1.0, 24.0, 24.0])));
            assert_eq!(sleep, Ok(None));
            assert_eq!(custom, Ok(Some([12.0, 18.0, 20.0, 10.0])));
            assert_eq!(missing, Ok(None));
            assert_eq!(SCREEN_BOUNDS.lock().unwrap().take(), Some(Ok(None)));

            // F1 hides ordinary HUD paint while sleep and deferred subtitles keep their slots.
            game.client.chat_ui.hide_gui = true;
            game.sleep_timer = 50;
            game.client.screen = Screen::BlockEditor;
            let _ = game.hud([320, 240]);
            assert_eq!(
                take(),
                [
                    "render_gui_pre",
                    "layer_post minecraft:boss_overlay",
                    "layer_post minecraft:sleep_overlay",
                    "layer_post minecraft:subtitle_overlay",
                    "layer_post minecraft:tab_list",
                    "layer post_test:top",
                    "layer_post post_test:top",
                    "render_gui_post",
                    "screen_pre block_editor",
                    "screen_post block_editor",
                ]
            );
            let [effects, sleep, _, _] = HUD_BOUNDS.lock().unwrap().take().unwrap();
            assert_eq!(effects, Ok(None));
            assert_eq!(sleep, Ok(Some([0.0, 0.0, 320.0, 240.0])));

            // A canceled Pre skips even those two slots, but still draws the screen.
            CANCEL_PRE.store(true, Ordering::Relaxed);
            let canceled = game.hud([320, 240]);
            assert_eq!(canceled.blur_at, 0, "HUD paint leaked into a canceled pass");
            assert_eq!(
                take(),
                [
                    "render_gui_pre",
                    "screen_pre block_editor",
                    "screen_post block_editor"
                ]
            );
            assert!(HUD_BOUNDS.lock().unwrap().take().is_none());
            CANCEL_PRE.store(false, Ordering::Relaxed);
            game.client.chat_ui.hide_gui = false;

            // Options runs the HUD pass only in-world, and its screen hook cannot see HUD bounds.
            game.open_options(false);
            let _ = game.hud([320, 240]);
            let events = take();
            assert!(
                events.iter().map(String::as_str).rev().take(3).eq([
                    "screen_post options",
                    "screen_pre options",
                    "render_gui_post",
                ]),
                "unexpected Options order: {events:?}"
            );
            assert_eq!(SCREEN_BOUNDS.lock().unwrap().take(), Some(Ok(None)));

            let saves = tempfile::tempdir().unwrap();
            game.client.title.0 = Some(game.new_title(saves.path().to_path_buf()));
            game.open_options(true);
            let _ = game.hud([320, 240]);
            assert_eq!(take(), ["screen_pre options", "screen_post options"]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn hud_layer_bounds_rejects_dedicated_server_role() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::gui_layer_tests::hud_layer_bounds_rejects_dedicated_server_role",
        || {
            with_client_game(|game| {
                game.common.options.execution_role = ExecutionRole::DedicatedServer;
                assert_eq!(
                    game.hud_layer_bounds("minecraft:effects"),
                    Err(mod_api::LevelReadError::Unavailable)
                );
            });
        },
    );
}
