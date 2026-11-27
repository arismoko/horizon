//! What special model renderers cost the frame (see the measurement in
//! `docs/performance/special-item-renderers.md`). Each test runs alone in its own process, the
//! measurement by hand: it registers the process's renderers.
use mod_api::{
    ClientHost, SpecialModelCollector, SpecialModelContext, SpecialModelFactory,
    SpecialModelGeometry, SpecialModelRenderer, SpecialModelStack, SpecialModelTints,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{
    item_render::{ItemRenderState, OwnedSpecialModelFactory, install_special_model_factories},
    mesher::Vertex,
};
use sunrise_items::items::ItemStack;

/// Full sky and block light, as the frame packs it.
const LIGHT: u32 = 0x00F0_00F0;

/// A built-in scene: a plain model, a tinted one and vanilla's own special models.
const SCENE: [&str; 6] = [
    "minecraft:stick",
    "minecraft:leather_chestplate",
    "minecraft:shield",
    "minecraft:chest",
    "minecraft:red_bed",
    "minecraft:skeleton_skull",
];

/// `stack` resolved and submitted as a frame draws an item in the world, into `out`.
fn draw(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    stack: &ItemStack,
    out: &mut Vec<Vertex>,
) -> ItemRenderState {
    let state = game.resolve_item(stack, DisplayContext::Gui, None, 0);
    game.submit_item_unlit(&state, glam::Mat4::IDENTITY, LIGHT, out);
    state
}

/// The game's GUI atlas with only the missing-texture sprite, which every quad then draws
/// with: the test game has no window to build the atlas in.
fn give_every_quad_a_sprite(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    let whole = sunrise_client::hud::Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    game.client
        .gui
        .atlas
        .sprites
        .insert("misc/missingno".into(), whole);
}

/// One frame of the scene: the frame's start, then every item, with the allocations each
/// made and the vertices drawn.
fn frame(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    stacks: &[ItemStack],
    out: &mut Vec<Vertex>,
) -> (usize, usize) {
    out.clear();
    let ((), started) = sunrise_threads::allocation_count::measure(|| game.begin_frame_draws());
    let ((), drawn) = sunrise_threads::allocation_count::measure(|| {
        for stack in stacks {
            draw(game, stack, out);
        }
    });
    (started, drawn)
}

/// Never asked: no drawn item's model is of its type.
struct Unmatched;
static UNMATCHED_ASKED: AtomicBool = AtomicBool::new(false);
impl SpecialModelRenderer for Unmatched {
    type Argument = ();

    fn extract_argument(&self, _: &dyn ClientHost, _: &SpecialModelStack<'_>) {
        UNMATCHED_ASKED.store(true, Ordering::Relaxed);
    }

    fn submit(
        &self,
        _: &(),
        _: &SpecialModelContext,
        _: &mut SpecialModelCollector<'_>,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// Row 13: with no special renderer registered, a built-in scene drawn for 10 frames draws the
/// same vertices every frame, starts each frame with no allocation, and
/// allocates as many times each frame as the frame before. Vanilla's resolver allocates for
/// its own layers, so the scene can't be 0 as a whole; nothing the extension adds grows from
/// frame to frame. A renderer registered for a type no drawn model names changes none of it,
/// down to the allocation count.
#[test]
#[ignore = "daily"]
fn built_in_scenes_cost_nothing_more_without_a_listener() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::native_special_renderers::cost_tests::built_in_scenes_cost_nothing_more_without_a_listener",
        built_in_scenes_cost_nothing_more_without_a_listener_isolated,
    );
}

fn built_in_scenes_cost_nothing_more_without_a_listener_isolated() {
    fn scene(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) -> (Vec<u8>, usize) {
        let stacks: Vec<ItemStack> = SCENE.iter().map(|id| ItemStack::new(id, 1)).collect();
        give_every_quad_a_sprite(game);
        let mut out = Vec::new();
        // Warm the game's caches, then compare steady frames and allocation counts.
        frame(game, &stacks, &mut out);
        let first = bytemuck::cast_slice::<Vertex, u8>(&out).to_vec();
        assert!(!first.is_empty(), "the scene drew nothing");
        let mut counts = Vec::new();
        for _ in 1..10 {
            let (started, drawn) = frame(game, &stacks, &mut out);
            assert_eq!(started, 0, "the frame's start allocated");
            assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&out), first);
            counts.push(drawn);
        }
        assert!(
            counts.windows(2).all(|w| w[0] == w[1]),
            "a frame allocated more than the one before: {counts:?}"
        );
        (first, counts[0])
    }
    let mut unregistered = None;
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        unregistered = Some(scene(game))
    });
    assert!(install_special_model_factories(BTreeMap::from([(
        mod_api::Identifier::parse("test:unmatched").unwrap(),
        OwnedSpecialModelFactory::new(
            "test",
            SpecialModelFactory::new(|_| Ok(()), |(), _| Ok(Unmatched)),
        ),
    )])));
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        assert_eq!(scene(game), unregistered.unwrap());
    });
    assert!(!UNMATCHED_ASKED.load(Ordering::Relaxed));
}

/// Draws one stone cube for every stack.
struct Stone(SpecialModelGeometry);
impl SpecialModelRenderer for Stone {
    type Argument = ();

    fn extract_argument(&self, _: &dyn ClientHost, _: &SpecialModelStack<'_>) {}

    fn submit(
        &self,
        _: &(),
        _: &SpecialModelContext,
        collector: &mut SpecialModelCollector<'_>,
    ) -> Result<(), String> {
        collector.submit_model(
            &self.0,
            SpecialModelTints::Untinted,
            mod_api::SpecialModelSubmitOptions::default(),
        )
    }
}

/// Nanoseconds per resolve and submit of the stone cube: stone's own `minecraft:model` versus a
/// registered special renderer submitting the same geometry directly. Prints median, p95 and p99
/// after warm-up. Run by hand with an optimized build and `SUNRISE_TEST_RESOURCES`:
/// `cargo test --profile fast -p sunrise-client --lib -- --ignored --nocapture special_item_draw_cost`.
#[test]
#[ignore = "measurement; needs SUNRISE_TEST_RESOURCES"]
fn special_item_draw_cost() {
    const WARM: usize = 1_000;
    const SAMPLES: usize = 10_000;
    let factories = BTreeMap::from([(
        mod_api::Identifier::parse("test:stone").unwrap(),
        OwnedSpecialModelFactory::new(
            "test",
            SpecialModelFactory::new(
                |_| Ok(()),
                |(), bake| bake.bake_model("minecraft:block/stone").map(Stone),
            ),
        ),
    )]);
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        // Dirt becomes a special model drawing stone's cube under stone's display transforms.
        let special = game.client.client_pack.item_renderer.decode_definition(
            r#"{"model":{"type":"minecraft:special","base":"minecraft:block/stone","model":{"type":"test:stone"}}}"#,
            &sunrise_client::item_render::ItemProviderTables {
                special: Some(&factories),
                ..Default::default()
            },
        ).unwrap();
        let renderer = Arc::get_mut(&mut game.client.client_pack.item_renderer)
            .expect("the game's item renderer is shared");
        Arc::make_mut(&mut renderer.models)
            .definitions
            .insert("minecraft:dirt".into(), special);
        game.pack.item_models = Arc::clone(&renderer.models);
        give_every_quad_a_sprite(game);
        let (stone, dirt) = (
            ItemStack::new("minecraft:stone", 1),
            ItemStack::new("minecraft:dirt", 1),
        );
        let mut out = Vec::new();
        let cube_vertices = {
            draw(game, &stone, &mut out);
            std::mem::take(&mut out).len()
        };
        draw(game, &dirt, &mut out);
        assert_eq!(out.len(), cube_vertices, "the special draws another shape");
        let mut measure = |what: &str, stack: &ItemStack| {
            let mut times = Vec::with_capacity(SAMPLES);
            for sample in 0..WARM + SAMPLES {
                out.clear();
                let start = std::time::Instant::now();
                std::hint::black_box(draw(game, stack, &mut out));
                let elapsed = start.elapsed().as_nanos();
                if sample >= WARM {
                    times.push(elapsed);
                }
            }
            times.sort_unstable();
            println!(
                "SPECIAL_ITEM_COST draw={what} samples={SAMPLES} median_ns={} p95_ns={} p99_ns={}",
                times[SAMPLES / 2],
                times[SAMPLES * 95 / 100],
                times[SAMPLES * 99 / 100]
            );
        };
        measure("built_in_model", &stone);
        measure("special_direct_submission", &dirt);
    });
}
