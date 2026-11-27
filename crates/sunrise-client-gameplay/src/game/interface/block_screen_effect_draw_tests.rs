//! Selected block overlay renderers in a real game: what they submit and where those quads land.
#[allow(unused_imports)]
use super::{PackResources, Screen};
#[allow(unused_imports)]
use crate::game::native_entity_refs::tests::{fixture_root, run_fresh_cases, with_client_game};
#[allow(unused_imports)]
use BlockScreenEffect::{Block, Fire, Water};
#[allow(unused_imports)]
use mod_api::{
    BlockScreenEffect, BlockScreenEffectRender, BlockScreenEffectRenderer, CameraSubmersionEvent,
    ClientHost, ExtractBlockScreenEffectEvent, Flow, Hooks, ModEntry, Mods, RenderGuiPostEvent,
};
#[allow(unused_imports)]
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
#[allow(unused_imports)]
use sunrise_client::{
    camera::hand_projection,
    fog::FogType,
    gfx::ScreenshotRequest,
    screen_effects::{Blend, ScreenQuad},
};
#[allow(unused_imports)]
use sunrise_core::{geometry::Vec3, position::BlockPos};
#[allow(unused_imports)]
use sunrise_level::camera_math::CameraType;
#[allow(unused_imports)]
use sunrise_level::chunk::Chunk;
#[allow(unused_imports)]
use sunrise_sim::{
    clock::FixedStep,
    motion::{MotionWorld, PlayerMotion, WATER},
};

pub(in crate::game) const RED: u32 = 0xFFFF0000;
/// Half-transparent blue: it shows what is under it.
pub(in crate::game) const CLEAR_BLUE: u32 = 0x800000FF;
/// A quad a quarter of a block from the view's centre, at vanilla's depth.
pub(in crate::game) const QUAD: [[f32; 3]; 4] = [
    [-0.25, -0.25, -0.5],
    [0.25, -0.25, -0.5],
    [0.25, 0.25, -0.5],
    [-0.25, 0.25, -0.5],
];

/// What the mod does with each overlay.
#[derive(Clone, Copy)]
pub(in crate::game) struct Plan {
    pub(in crate::game) custom: &'static [BlockScreenEffect],
    pub(in crate::game) cancel: &'static [BlockScreenEffect],
    pub(in crate::game) replace: &'static [BlockScreenEffect],
}
pub(in crate::game) const LEAVE: Plan = Plan {
    custom: &[],
    cancel: &[],
    replace: &[],
};
/// One submission of a selected renderer.
#[derive(Debug, PartialEq)]
pub(in crate::game) struct Asked {
    pub(in crate::game) effect: BlockScreenEffect,
    pub(in crate::game) state: mod_api::BlockState,
    pub(in crate::game) pos: BlockPos,
    pub(in crate::game) partial_tick: f32,
    pub(in crate::game) hide_gui: bool,
}
pub(in crate::game) static PLAN: Mutex<Plan> = Mutex::new(LEAVE);
pub(in crate::game) static ASKED: Mutex<Vec<Asked>> = Mutex::new(Vec::new());
/// The partial tick each extract event held.
pub(in crate::game) static EXTRACTED: Mutex<Vec<(BlockScreenEffect, f32)>> = Mutex::new(Vec::new());

/// The mod's colour for what it draws over `effect`.
pub(in crate::game::client_side) fn marker(effect: BlockScreenEffect) -> u32 {
    match effect {
        Block => 0xFF010000,
        Water => 0xFF020000,
        Fire => 0xFF030000,
    }
}
pub(in crate::game) struct Planned;
#[mod_api::__subscriptions]
impl Hooks for Planned {
    fn extract_block_screen_effect(
        &self,
        _: &dyn ClientHost,
        event: &mut ExtractBlockScreenEffectEvent<'_>,
    ) -> Flow {
        let plan = *PLAN.lock().unwrap();
        EXTRACTED
            .lock()
            .unwrap()
            .push((event.effect, event.partial_tick));
        if plan.custom.contains(&event.effect) {
            event.set_custom_renderer(&PlannedRenderer);
        }
        if plan.cancel.contains(&event.effect) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

pub(in crate::game) struct PlannedRenderer;
impl BlockScreenEffectRenderer for PlannedRenderer {
    fn submit(&self, _: &dyn ClientHost, render: BlockScreenEffectRender<'_>) -> bool {
        ASKED.lock().unwrap().push(Asked {
            effect: render.effect,
            state: render.state,
            pos: render.pos,
            partial_tick: render.partial_tick,
            hide_gui: render.hide_gui,
        });
        assert!(render.draw.quad(QUAD, marker(render.effect)));
        PLAN.lock().unwrap().replace.contains(&render.effect)
    }
}

/// A player on fire in first person, with their eye in block (0, 65, 0).
pub(in crate::game::client_side) fn burning_player(game: &mut crate::game::ClientGame<'_>) {
    game.common.world.chunks.insert(
        (0, 0),
        Arc::new(Chunk::with_layout(0, 0, game.common.world.layout())),
    );
    let mut player = PlayerMotion::new(game.common.motion, Vec3::ZERO).unwrap();
    player.position = Vec3::new(0.5, 64.0, 0.5);
    player.fire_ticks = 100;
    game.common.player = Some(player);
}
#[allow(unused_imports)]
pub(in crate::game) use crate::game::server_side::block_screen_effect_draw_tests_server::sample_eye_in_water;

/// The frame's quads at 2:1 with the hand's usual 70 degrees.
pub(in crate::game::client_side) fn quads(game: &crate::game::ClientGame<'_>) -> Vec<ScreenQuad> {
    let projection = hand_projection(70.0, 1000.0, 500.0, 1000.0);
    game.screen_quads(0.25, [1000, 500], 1.0, projection)
}
/// Names the block overlays' quads in draw order, the HUD's camera overlays left out.
pub(in crate::game::client_side) fn names(quads: &[ScreenQuad]) -> Vec<&'static str> {
    quads
        .iter()
        .filter(|quad| quad.blend == Blend::Alpha)
        .filter_map(|quad| {
            let [r, g, b, a] = quad.vertices[0].color;
            Some(match (r, g, b, a) {
                (_, 0.0, 0.0, 1.0) if r == 1.0 / 255.0 => "mod block",
                (_, 0.0, 0.0, 1.0) if r == 2.0 / 255.0 => "mod water",
                (_, 0.0, 0.0, 1.0) if r == 3.0 / 255.0 => "mod fire",
                (_, _, _, 1.0) if r == 25.0 / 255.0 && r == g && g == b => "block",
                (_, _, _, 0.1) => "water",
                (1.0, 1.0, 1.0, _) if a == 229.0 / 255.0 => "fire",
                _ => return None,
            })
        })
        .collect()
}
pub(in crate::game::client_side) fn asked() -> Vec<BlockScreenEffect> {
    let mut asked = ASKED.lock().unwrap();
    asked.drain(..).map(|ask| ask.effect).collect()
}
pub(in crate::game::client_side) fn close_to(actual: [f32; 4], expected: [f32; 4]) {
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-4, "{actual:?} is not {expected:?}");
    }
}

/// Each thing a mod can do with an overlay, in a frame with stone at the eye and fire on the
/// player, then with water at the eye. The expected names are `ScreenEffectRenderer.submit`'s
/// order, each mod's quads right before the overlay they belong to.
#[test]
#[ignore = "daily"]
fn a_mod_draws_prepends_replaces_and_cancels_overlays() {
    with_client_game(|game| {
        burning_player(game);
        let registry = &game.pack.assets.registry;
        let state = |name: &str| registry.default_state(name).unwrap();
        let (stone, water, fire) = (
            state("minecraft:stone"),
            state("minecraft:water"),
            state("minecraft:fire"),
        );
        game.world.set(0, 65, 0, stone).unwrap();
        // No listener: vanilla's overlays, at vanilla's place. `submitBlockSprite`'s (-1, -1) at
        // depth 0.5 under 70 degrees at 2:1 is -1 / (0.5 * tan 35 * 2) and -1 / (0.5 * tan 35).
        let vanilla = quads(game);
        assert_eq!(names(&vanilla), ["block", "fire", "fire"]);
        close_to(vanilla[0].vertices[0].pos, [-1.42815, -2.85630, 0.0, 1.0]);
        close_to(vanilla[0].vertices[2].pos, [1.42815, 2.85630, 0.0, 1.0]);
        let bytes = |quads: &[ScreenQuad]| {
            let vertices = quads.iter().flat_map(|quad| quad.vertices);
            let vertices: Vec<_> = vertices.collect();
            bytemuck::cast_slice::<_, u8>(&vertices).to_vec()
        };
        let mods: Mods = vec![ModEntry::new("screen-effect", Box::new(Planned))].into();
        mod_platform::testing::with_mods(mods, || {
            let frame = |game: &crate::game::ClientGame<'_>, plan: Plan| {
                *PLAN.lock().unwrap() = plan;
                let quads = quads(game);
                (quads, asked())
            };
            // A listener that claims nothing leaves every vertex and is never asked to draw.
            let (left, draws) = frame(game, LEAVE);
            assert_eq!(bytes(&left), bytes(&vanilla));
            assert_eq!(draws, []);
            // `quads` draws at partial tick 0.25: `getWorldPartialTick` of each shown overlay.
            assert_eq!(
                std::mem::take(&mut *EXTRACTED.lock().unwrap()),
                [(Block, 0.25), (Fire, 0.25)]
            );

            // Prepending: the mod's quad, then vanilla's.
            let plan = Plan {
                custom: &[Block],
                ..LEAVE
            };
            *PLAN.lock().unwrap() = plan;
            game.client.chat_ui.hide_gui = true;
            let prepended = quads(game);
            game.client.chat_ui.hide_gui = false;
            assert_eq!(names(&prepended), ["mod block", "block", "fire", "fire"]);
            assert_eq!(
                ASKED.lock().unwrap().drain(..).collect::<Vec<_>>(),
                [Asked {
                    effect: Block,
                    state: sunrise_gameplay_common::game::api_values::TryIntoApi::try_into_api(
                        &stone
                    )
                    .unwrap(),
                    pos: BlockPos::new(0, 65, 0),
                    partial_tick: 0.25,
                    hide_gui: true,
                }]
            );
            // Camera space under the hand's projection: x * 1 / (tan 35 * 2), y / tan 35, and
            // w is the depth. Reversed depth from 0.05 puts z at about 0.05 for any depth.
            let corner = prepended[0].vertices[2].pos;
            close_to(corner, [0.17852, 0.35704, 0.04998, 0.5]);

            // Replacing: only the mod's quad, and the other overlays stay.
            let (replaced, draws) = frame(
                game,
                Plan {
                    custom: &[Block],
                    replace: &[Block],
                    ..LEAVE
                },
            );
            assert_eq!(names(&replaced), ["mod block", "fire", "fire"]);
            assert_eq!(draws, [Block]);

            // Cancellation does not clear the renderer, only the vanilla overlay.
            let (canceled, draws) = frame(
                game,
                Plan {
                    custom: &[Block],
                    cancel: &[Block],
                    ..LEAVE
                },
            );
            assert_eq!(names(&canceled), ["mod block", "fire", "fire"]);
            assert_eq!(draws, [Block]);

            // Canceled alone is NO_OP: nothing drawn, nothing asked.
            let (no_op, draws) = frame(
                game,
                Plan {
                    cancel: &[Block],
                    ..LEAVE
                },
            );
            assert_eq!(names(&no_op), ["fire", "fire"]);
            assert_eq!(draws, []);

            // Fire's own state and place, replaced while the block's overlay stays.
            *PLAN.lock().unwrap() = Plan {
                custom: &[Fire],
                replace: &[Fire],
                ..LEAVE
            };
            assert_eq!(names(&quads(game)), ["block", "mod fire"]);
            assert_eq!(
                ASKED.lock().unwrap().drain(..).collect::<Vec<_>>(),
                [Asked {
                    effect: Fire,
                    state: sunrise_gameplay_common::game::api_values::TryIntoApi::try_into_api(
                        &fire
                    )
                    .unwrap(),
                    pos: BlockPos::new(0, 64, 0),
                    partial_tick: 0.25,
                    hide_gui: false,
                }]
            );

            // Fire cancellation only suppresses vanilla drawing if no renderer was selected.
            let (canceled_fire, draws) = frame(
                game,
                Plan {
                    custom: &[Fire],
                    cancel: &[Fire],
                    ..LEAVE
                },
            );
            assert_eq!(names(&canceled_fire), ["block", "mod fire", "fire", "fire"]);
            assert_eq!(draws, [Fire]);
            let (no_op_fire, draws) = frame(
                game,
                Plan {
                    cancel: &[Fire],
                    ..LEAVE
                },
            );
            assert_eq!(names(&no_op_fire), ["block"]);
            assert_eq!(draws, []);

            // Under water: every overlay claimed, in vanilla's order.
            game.world.set(0, 65, 0, water).unwrap();
            assert!(sample_eye_in_water(game));
            let (under, draws) = frame(
                game,
                Plan {
                    custom: &[Water, Fire],
                    replace: &[Water],
                    ..LEAVE
                },
            );
            assert_eq!(names(&under), ["mod water", "mod fire", "fire", "fire"]);
            assert_eq!(draws, [Water, Fire]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_panicking_extractor_preserves_the_previous_selection_and_other_listeners() {
    use mod_api::__private::{HookPanicReport, take_hook_panics};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Panics(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Panics {
        #[listener(priority = -5)]
        fn extract_block_screen_effect(
            &self,
            _: &dyn ClientHost,
            event: &mut ExtractBlockScreenEffectEvent<'_>,
        ) -> Flow {
            if event.effect == Water {
                self.0.fetch_add(1, Ordering::Relaxed);
                event.set_custom_renderer(&Choice(RED));
                event.cancelled = true;
                panic!("extractor failed after replacing the selection");
            }
            Flow::Continue
        }
    }

    struct After(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for After {
        #[listener(priority = -10)]
        fn extract_block_screen_effect(
            &self,
            _: &dyn ClientHost,
            event: &mut ExtractBlockScreenEffectEvent<'_>,
        ) -> Flow {
            if event.effect == Water {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
            Flow::Continue
        }
    }

    with_client_game(|game| {
        burning_player(game);
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        game.world.set(0, 65, 0, water).unwrap();
        assert!(sample_eye_in_water(game));
        let attempts = Arc::new(AtomicUsize::new(0));
        let later_calls = Arc::new(AtomicUsize::new(0));
        let mods = vec![
            ModEntry::new("first", Box::new(ObserveOverlay(Water, Arc::default()))),
            ModEntry::new("broken-extractor", Box::new(Panics(attempts.clone()))),
            ModEntry::new("after", Box::new(After(later_calls.clone()))),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            for frame in 1..=2 {
                assert_eq!(names(&quads(game)), ["mod water", "water", "fire", "fire"]);
                assert_eq!(attempts.load(Ordering::Relaxed), 1);
                assert_eq!(later_calls.load(Ordering::Relaxed), frame);
                let reports = take_hook_panics();
                if frame == 1 {
                    assert_eq!(
                        reports,
                        [HookPanicReport {
                            mod_id: "broken-extractor",
                            hook: "extract_block_screen_effect",
                        }]
                    );
                } else {
                    assert!(reports.is_empty());
                }
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_panicking_renderer_discards_its_quads_and_is_skipped_on_later_frames() {
    use mod_api::__private::{HookPanicReport, take_hook_panics};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SUBMISSIONS: AtomicUsize = AtomicUsize::new(0);
    struct BrokenFire;
    impl BlockScreenEffectRenderer for BrokenFire {
        fn submit(&self, _: &dyn ClientHost, render: BlockScreenEffectRender<'_>) -> bool {
            assert!(render.draw.quad(QUAD, marker(Fire)));
            SUBMISSIONS.fetch_add(1, Ordering::Relaxed);
            panic!("fire renderer failed after drawing");
        }
    }

    struct Select(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Select {
        fn extract_block_screen_effect(
            &self,
            _: &dyn ClientHost,
            event: &mut ExtractBlockScreenEffectEvent<'_>,
        ) -> Flow {
            match event.effect {
                Fire => {
                    self.0.fetch_add(1, Ordering::Relaxed);
                    event.set_custom_renderer(&BrokenFire);
                }
                Water => event.set_custom_renderer(&Choice(0xFF020000)),
                Block => {}
            }
            Flow::Continue
        }
    }

    with_client_game(|game| {
        burning_player(game);
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        game.world.set(0, 65, 0, water).unwrap();
        assert!(sample_eye_in_water(game));
        let extractions = Arc::new(AtomicUsize::new(0));
        let mods = vec![ModEntry::new(
            "broken-fire",
            Box::new(Select(extractions.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            for frame in 1..=2 {
                // Water from the same mod still submits, and vanilla fire remains visible.
                assert_eq!(names(&quads(game)), ["mod water", "water", "fire", "fire"]);
                assert_eq!(SUBMISSIONS.load(Ordering::Relaxed), 1);
                assert_eq!(extractions.load(Ordering::Relaxed), frame);
                let reports = take_hook_panics();
                if frame == 1 {
                    assert_eq!(
                        reports,
                        [HookPanicReport {
                            mod_id: "broken-fire",
                            hook: "block_screen_effect_renderer::submit",
                        }]
                    );
                } else {
                    assert!(reports.is_empty());
                }
            }
        });
    });
}

pub(in crate::game) struct Choice(pub(in crate::game) u32);
impl BlockScreenEffectRenderer for Choice {
    fn submit(&self, _: &dyn ClientHost, render: BlockScreenEffectRender<'_>) -> bool {
        assert!(render.draw.quad(QUAD, self.0));
        false
    }
}

#[derive(Debug, PartialEq)]
pub(in crate::game) struct OverlayInput {
    pub(in crate::game) state: mod_api::BlockState,
    pub(in crate::game) pos: BlockPos,
    pub(in crate::game) partial_tick: f32,
    pub(in crate::game) player_partial_tick: f32,
}

pub(in crate::game) struct ObserveOverlay(
    pub(in crate::game) BlockScreenEffect,
    pub(in crate::game) Arc<Mutex<Option<OverlayInput>>>,
);
#[mod_api::__subscriptions]
impl Hooks for ObserveOverlay {
    fn extract_block_screen_effect(
        &self,
        _: &dyn ClientHost,
        event: &mut ExtractBlockScreenEffectEvent<'_>,
    ) -> Flow {
        if event.effect == self.0 {
            *self.1.lock().unwrap() = Some(OverlayInput {
                state: event.state,
                pos: event.pos,
                partial_tick: event.partial_tick,
                player_partial_tick: event.player_partial_tick,
            });
            if event.effect == Water {
                event.set_custom_renderer(&Choice(0xFF020000));
            }
        }
        Flow::Continue
    }
}

pub(in crate::game::client_side) fn observe_overlay(
    effect: BlockScreenEffect,
    run: impl FnOnce(),
) -> Option<OverlayInput> {
    let seen = Arc::new(Mutex::new(None));
    let mods = vec![ModEntry::new(
        "observe-overlay",
        Box::new(ObserveOverlay(effect, seen.clone())),
    )];
    mod_platform::testing::with_mods(mods.into(), run);
    seen.lock().unwrap().take()
}

#[test]
#[ignore = "daily"]
fn underwater_third_person_neither_extracts_nor_draws_the_overlay() {
    with_client_game(|game| {
        burning_player(game);
        game.client.camera_type = CameraType::ThirdPersonBack;
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        game.world.set(0, 65, 0, water).unwrap();
        assert!(sample_eye_in_water(game));
        let seen = observe_overlay(Water, || {
            let overlays = names(&quads(game));
            assert!(!overlays.contains(&"mod water") && !overlays.contains(&"water"));
        });
        assert_eq!(seen, None);
    });
}

#[test]
#[ignore = "daily"]
fn waterlogged_eye_reports_default_water_at_the_eye_block() {
    with_client_game(|game| {
        burning_player(game);
        let slab = game
            .block_state_id("minecraft:oak_slab[type=bottom,waterlogged=true]")
            .unwrap();
        game.world.set(0, 65, 0, slab).unwrap();
        assert!(sample_eye_in_water(game));
        let seen = observe_overlay(Water, || {
            assert!(
                names(&quads(game))
                    .windows(2)
                    .any(|pair| pair == ["mod water", "water"])
            );
        })
        .unwrap();
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        assert_eq!(
            seen.state,
            sunrise_gameplay_common::game::api_values::TryIntoApi::try_into_api(&water).unwrap()
        );
        assert_eq!(seen.pos, BlockPos::new(0, 65, 0));
    });
}

#[test]
#[ignore = "daily"]
fn water_extraction_follows_the_tick_sample_at_the_surface() {
    with_client_game(|game| {
        burning_player(game);
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        game.world.set(0, 65, 0, water).unwrap();
        // A source with no water or solid ceiling above it is 8/9 of a block high.
        // EntityFluidInteraction includes the equality case; Camera.getFluidInCamera does not.
        let surface = 65.0 + f64::from(8.0_f32 / 9.0);
        let player = game.player.as_mut().unwrap();
        player.position.y = surface - f64::from(player.dimensions.eye_height);
        assert!(sample_eye_in_water(game));
        assert_eq!(
            game.camera_fog_type([0.5, surface, 0.5]),
            FogType::Atmospheric
        );
        assert!(names(&quads(game)).contains(&"water"));
        assert!(
            observe_overlay(Water, || {
                assert!(names(&quads(game)).contains(&"mod water"));
            })
            .is_some()
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_dead_players_water_overlay_uses_the_last_sample_after_removal() {
    with_client_game(|game| {
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "water-overlay").unwrap();
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.loaded_sent = true;
        remote.joined = true;
        remote.entity_id = 42;
        remote.level_load.finish();
        game.client.screen = Screen::Playing;
        game.client.camera_type = CameraType::FirstPerson;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5)).unwrap());
        let chunk = Chunk::with_layout(0, 0, game.world.layout());
        game.client
            .streaming
            .insert_remote(&mut game.common.world, chunk);
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        let air = game.world.block(8, 65, 8);
        game.world.set(8, 65, 8, water).unwrap();
        game.tick_remote().unwrap();

        game.client.screen = Screen::Dead;
        use sunrise_protocol::packets::play::{Clientbound, SetHealth};
        game.apply_packet(Clientbound::SetHealth(SetHealth {
            health: 0.0,
            food: 20,
            saturation: 5.0,
        }))
        .unwrap();
        let wet = observe_overlay(Water, || {
            assert!(
                names(&quads(game))
                    .windows(2)
                    .any(|pair| pair == ["mod water", "water"])
            );
        });
        assert_eq!(wet.unwrap().pos, BlockPos::new(8, 65, 8));

        game.world.set(8, 65, 8, air).unwrap();
        // Rendering still reads the old sample until the player ticks.
        assert!(names(&quads(game)).contains(&"water"));
        game.tick_remote().unwrap();
        let dry = observe_overlay(Water, || {
            let overlays = names(&quads(game));
            assert!(!overlays.contains(&"water") && !overlays.contains(&"mod water"));
        });
        assert_eq!(dry, None);
        assert_eq!(game.client.chat_ui.death_time, 1);

        // Tick 20 still samples before the local player is removed from the level.
        for _ in 1..19 {
            game.tick_remote().unwrap();
        }
        assert_eq!(game.client.chat_ui.death_time, 19);
        game.world.set(8, 65, 8, water).unwrap();
        game.tick_remote().unwrap();
        assert_eq!(game.client.chat_ui.death_time, 20);
        assert!(names(&quads(game)).contains(&"water"));

        // Extraction still reads that last sample after removal, even as the world ticks.
        game.world.set(8, 65, 8, air).unwrap();
        game.tick_remote().unwrap();
        let still_wet = observe_overlay(Water, || {
            assert!(
                names(&quads(game))
                    .windows(2)
                    .any(|pair| pair == ["mod water", "water"])
            );
        });
        assert_eq!(still_wet.unwrap().pos, BlockPos::new(8, 65, 8));
        assert_eq!(game.client.chat_ui.death_time, 20);
        server.shutdown();
    });
}

pub(in crate::game) struct InvertWater;
#[mod_api::__subscriptions]
impl Hooks for InvertWater {
    fn camera_submersion(&self, _: &dyn ClientHost, event: &mut CameraSubmersionEvent<'_>) {
        *event.fog_type = if *event.fog_type == FogType::Water {
            FogType::Atmospheric
        } else {
            FogType::Water
        };
    }
}

#[test]
#[ignore = "daily"]
fn camera_submersion_can_force_or_clear_the_first_person_water_overlay() {
    with_client_game(|game| {
        burning_player(game);
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        let dry = game.world.block(0, 65, 0);
        let mods: Mods = vec![ModEntry::new("invert-water", Box::new(InvertWater))].into();
        for wet in [false, true] {
            if wet {
                game.world.set(0, 65, 0, water).unwrap();
            }
            assert_eq!(sample_eye_in_water(game), wet);
            // The world can change after the sample. The override still sees the tick's answer.
            game.world.set(0, 65, 0, dry).unwrap();
            mod_platform::testing::with_mods(mods.clone(), || {
                assert_eq!(names(&quads(game)).contains(&"water"), !wet);
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn pack_assembly_reports_a_missing_water_default() {
    with_client_game(|game| {
        let root = sunrise_assets::resource_source::ResourceRoot::directory(fixture_root());
        let client_pack = &game.client.client_pack;
        let registry = sunrise_world::registry::BlockRegistry::from_facts_json(
            br#"{"states":[["minecraft:air",true,false,"","minecraft:air"]],"defaults":{"minecraft:air":0}}"#
                .as_slice(),
        )
        .unwrap();
        let assets = sunrise_level::assets::Assets {
            registry: Box::leak(Box::new(registry)),
            properties: sunrise_level::assets::Properties::for_tests(&[(
                true, false, false, false,
            )]),
            material_states: [sunrise_world::registry::BlockStateId(0); 6],
            blocks_fluid_flow: Default::default(),
        };
        let items = sunrise_client::item_render::ItemRenderer::load_for_server(
            &root,
            client_pack.render.model_library.clone(),
            &game.res.core.item_data,
        )
        .unwrap();
        let assembled_pack = PackResources::assemble(
            &root,
            sunrise_gameplay_common::game::resource_tables::PackParts {
                assets: Arc::new(assets),
                biomes: game.pack.chunk_context.biomes.clone(),
                light: game.pack.chunk_context.light.clone(),
                item_models: Arc::clone(&items.models),
                item_ids: Arc::clone(items.item_ids()),
            },
        )
        .unwrap();
        let Err(error) = super::ClientPackResources::assemble(
            &root,
            &assembled_pack,
            super::resource_tables_client::ClientPackParts {
                render: client_pack.render.clone(),
                tables: client_pack.context.mesh.tables.clone(),
                item_renderer: items,
            },
            true,
        ) else {
            panic!("a registry without water cannot supply the overlay");
        };
        assert_eq!(
            error.to_string(),
            "block registry is missing the default minecraft:water state"
        );
    });
}

#[test]
#[ignore = "daily"]
fn extraction_keeps_player_interpolation_when_world_interpolation_is_frozen() {
    with_client_game(|game| {
        burning_player(game);
        game.clock = FixedStep::client();
        game.clock.set_frozen(true);
        assert!(!game.clock.world_runs());
        game.clock.end_frame(false);
        game.clock.advance(Duration::from_millis(25), 50.0);
        let seen = observe_overlay(Fire, || {
            let projection = hand_projection(70.0, 1000.0, 500.0, 1000.0);
            game.screen_quads(
                game.frame_world_partial_tick() as f32,
                [1000, 500],
                1.0,
                projection,
            );
        })
        .unwrap();
        assert_eq!((seen.partial_tick, seen.player_partial_tick), (1.0, 0.5));
    });
}

pub(in crate::game) struct FirstChoice(pub(in crate::game) bool);
#[mod_api::__subscriptions]
impl Hooks for FirstChoice {
    #[listener(priority = 10)]
    fn extract_block_screen_effect(
        &self,
        _: &dyn ClientHost,
        event: &mut ExtractBlockScreenEffectEvent<'_>,
    ) -> Flow {
        if event.effect == Block {
            event.set_custom_renderer(&Choice(RED));
            if self.0 {
                return Flow::Cancel;
            }
        }
        Flow::Continue
    }
}

pub(in crate::game) struct ReceiveCancelled;
#[mod_api::__subscriptions]
impl Hooks for ReceiveCancelled {
    #[listener(receive_cancelled)]
    fn extract_block_screen_effect(
        &self,
        _: &dyn ClientHost,
        event: &mut ExtractBlockScreenEffectEvent<'_>,
    ) -> Flow {
        if event.effect == Block && event.cancelled {
            event.set_custom_renderer(&Choice(0xFF0000FF));
        }
        Flow::Continue
    }
}

pub(in crate::game) struct LastChoice;
#[mod_api::__subscriptions]
impl Hooks for LastChoice {
    #[listener(priority = -10)]
    fn extract_block_screen_effect(
        &self,
        _: &dyn ClientHost,
        event: &mut ExtractBlockScreenEffectEvent<'_>,
    ) -> Flow {
        if event.effect == Block {
            event.set_custom_renderer(&Choice(0xFF00FF00));
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn only_the_last_eligible_renderer_submits() {
    with_client_game(|game| {
        burning_player(game);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 65, 0, stone).unwrap();
        let frame = |cancel| {
            // Supply entries out of listener-priority order.
            let mods: Mods = vec![
                ModEntry::new("last", Box::new(LastChoice)),
                ModEntry::new("receive", Box::new(ReceiveCancelled)),
                ModEntry::new("first", Box::new(FirstChoice(cancel))),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || quads(game))
        };
        for (cancel, chosen) in [(false, [0.0, 1.0, 0.0, 1.0]), (true, [0.0, 0.0, 1.0, 1.0])] {
            let frame = frame(cancel);
            assert_eq!(frame[0].vertices[0].color, chosen);
            let choices = [
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
            ];
            let custom_colors: Vec<_> = frame
                .iter()
                .map(|quad| quad.vertices[0].color)
                .filter(|color| choices.contains(color))
                .collect();
            assert_eq!(custom_colors, [chosen]);
        }
    });
}

/// A frame's pixels, RGBA.
pub(in crate::game::client_side) fn drawn_frame(
    game: &mut crate::game::ClientGame<'_>,
) -> (Vec<u8>, usize) {
    game.client.renderer.0.as_mut().unwrap().screenshot = Some(ScreenshotRequest {
        path: "unused.png".into(),
        size: None,
        scaled: false,
        background: false,
    });
    game.render_with(|renderer, frame| renderer.render(frame, None))
        .unwrap();
    let result = game
        .client
        .renderer
        .0
        .as_mut()
        .unwrap()
        .screenshot_result
        .take();
    let mut pending = result.unwrap().unwrap();
    let image = pending.image_mut();
    (std::mem::take(&mut image.rgba), image.width as usize)
}

/// Whether the frame's mod draws: off for the frame to compare with.
pub(in crate::game) static DRAWING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
pub(in crate::game) struct Painter;
#[mod_api::__subscriptions]
impl Hooks for Painter {
    fn extract_block_screen_effect(
        &self,
        _: &dyn ClientHost,
        event: &mut ExtractBlockScreenEffectEvent<'_>,
    ) -> Flow {
        if event.effect == Fire {
            event.set_custom_renderer(&Painter);
        }
        Flow::Continue
    }
    fn render_gui_post(&self, _: &dyn ClientHost, event: &mut RenderGuiPostEvent<'_>) {
        if DRAWING.load(std::sync::atomic::Ordering::Relaxed) {
            let [width, height] = event.draw.size();
            event.draw.fill([0.0, 0.0, width / 2.0, height], 0xFF00FF00);
        }
    }
}

impl BlockScreenEffectRenderer for Painter {
    fn submit(&self, _: &dyn ClientHost, render: BlockScreenEffectRender<'_>) -> bool {
        // Leaning away to the right: the right edge is twice as deep, so half as far out.
        let leaning = [
            [-0.25, -0.25, -0.5],
            [0.25, -0.25, -1.0],
            [0.25, 0.25, -1.0],
            [-0.25, 0.25, -0.5],
        ];
        assert!(render.draw.quad(leaning, RED));
        let clear = [
            [0.33, -0.1, -0.5],
            [0.44, -0.1, -0.5],
            [0.44, 0.1, -0.5],
            [0.33, 0.1, -0.5],
        ];
        assert!(render.draw.quad(clear, CLEAR_BLUE));
        true
    }
}

/// On a real renderer: the quads go over the level, perspective included, and the GUI goes over
/// them. At 320x240 the hand's 70 degrees put a corner `(x, y, z)` at pixel
/// `160 + 160 * x * 1.07111 / -z`, `120 - 120 * y * 1.42815 / -z`, so the leaning quad's left edge
/// is at x 74 and its right edge at x 203, and the clear quad covers x 273 to 311.
#[test]
#[ignore = "daily"]
fn screen_effect_quads_draw_over_the_level_and_under_the_gui() {
    let name = "game::client_side::block_screen_effect_draw_tests::screen_effect_quads_draw_over_the_level_and_under_the_gui";
    if run_fresh_cases(name, "SUNRISE_SCREEN_EFFECT_PASS_CHILD", &["pixels"]) {
        return;
    }
    let mods: Mods = vec![ModEntry::new("painter", Box::new(Painter))].into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let mut builder = winit::event_loop::EventLoop::builder();
            #[cfg(target_os = "linux")]
            {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11().with_any_thread(true);
            }
            let event_loop = builder.build().unwrap_or_else(|error| {
                eprintln!("screen-effect event loop error: {error}");
                panic!("screen-effect event loop creation failed")
            });
            #[allow(deprecated)]
            let window = Arc::new(
                event_loop
                    .create_window(
                        winit::window::Window::default_attributes()
                            .with_visible(false)
                            .with_inner_size(winit::dpi::PhysicalSize::new(320, 240)),
                    )
                    .unwrap(),
            );
            game.options.offscreen = true;
            let renderer = game.new_renderer(window).unwrap();
            game.set_renderer(renderer);
            game.client.title.0 = None;
            burning_player(game);
            game.previous_position = game.player.as_ref().unwrap().position;
            // A black vignette is no quad: the HUD's camera overlays would darken the colours.
            game.client.screen_fx.vignette_brightness = 0.0;
            game.player.as_mut().unwrap().fire_ticks = 0;
            let (level, width) = drawn_frame(game);
            game.player.as_mut().unwrap().fire_ticks = 100;
            DRAWING.store(true, std::sync::atomic::Ordering::Relaxed);
            let (frame, _) = drawn_frame(game);
            let pixel = |pixels: &[u8], x: usize, y: usize| {
                let at = (y * width + x) * 4;
                [pixels[at], pixels[at + 1], pixels[at + 2]]
            };
            let green = [0, 255, 0];
            let red = [255, 0, 0];
            assert_eq!(pixel(&frame, 190, 120), red, "inside the leaning quad");
            // The top edge runs from (74, 34) to (203, 77), so it is at y 66 for x 170.
            assert_eq!(pixel(&frame, 170, 70), red, "under the sloping top edge");
            assert_eq!(
                pixel(&frame, 170, 60),
                pixel(&level, 170, 60),
                "over the sloping top edge: perspective, not a rectangle"
            );
            assert_eq!(
                pixel(&frame, 208, 120),
                pixel(&level, 208, 120),
                "past its right edge"
            );
            assert_eq!(pixel(&frame, 100, 120), green, "the GUI is over the quad");
            // The clear quad blends with the level under it.
            let under = pixel(&level, 290, 120);
            let over = pixel(&frame, 290, 120);
            assert_ne!(under, [0, 0, 0], "the level draws something here");
            let blue = [0.0, 0.0, 255.0];
            for i in 0..3 {
                let kept = f32::from(under[i]) * (127.0 / 255.0);
                let expected = kept + blue[i] * (128.0 / 255.0);
                let off = (f32::from(over[i]) - expected).abs();
                assert!(off <= 1.0, "{over:?} is not blue at half over {under:?}");
            }
        });
    });
}
