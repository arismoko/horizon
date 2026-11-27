//! `register_entity_renderers` for paintings (#1296): a renderer registered for
//! `minecraft:painting` draws in place of the painting's quads, reading the facing and size; with no
//! renderer the family's own geometry is unchanged; an `Err` leaves its quads for just that draw.
use super::native_entity_renderers::{EntityRendererTable, RegisteredEntityRenderer};
use mod_api::{
    BlockEntityDrawContext, Direction, EntityRenderState, EntityRenderer, Hooks,
    RegisterEntityRenderersEvent, RegistrationContext,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use sunrise_client::mesher::Vertex;
use sunrise_sim::entities::Painting;

/// One stone quad a block wide, `height` blocks above the entity.
fn raised_quad(draw: &mut dyn BlockEntityDrawContext, height: f32) {
    let (low, high) = (height, height + 1.0);
    let corners = [
        [0.0, high, 0.0],
        [0.0, low, 0.0],
        [1.0, low, 0.0],
        [1.0, high, 0.0],
    ];
    draw.quad("block/stone", corners, [0.0, 0.0, 16.0, 16.0], 0xFF_FFFF);
}

/// What the painting renderer last read, and whether it was asked.
#[derive(Default)]
struct Heard {
    state: Option<EntityRenderState>,
}

/// Draws a quad 100 blocks up and records the frame it read.
struct Paint(Arc<Mutex<Heard>>);
impl EntityRenderer for Paint {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        self.0.lock().unwrap().state = Some(entity.clone());
        raised_quad(draw, 100.0);
        Ok(())
    }
}

/// Refuses its first call with an `Err`, then draws a quad 400 blocks up; `calls` counts the
/// calls.
#[derive(Default)]
struct Flaky {
    calls: AtomicU32,
}
impl Flaky {
    fn calls(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }
}
impl EntityRenderer for Flaky {
    fn submit(
        &self,
        _: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        let first = self.calls.fetch_add(1, Ordering::Relaxed) == 0;
        raised_quad(draw, 400.0);
        if first {
            return Err("refused its first frame".into());
        }
        Ok(())
    }
}

/// The camera, west of a west-facing painting and looking at its front.
const CAMERA: glam::DVec3 = glam::DVec3::new(-4.0, 64.5, 0.5);
/// The hanging entity's position: the centre of a 1x1 painting at `[0, 64, 0]` facing west.
const CENTRE: [f64; 3] = [0.96875, 64.5, 0.5];
const FACING: u8 = 1;
const WIDTH: i32 = 1;
const HEIGHT: i32 = 1;
/// The frame's partial tick, passed through to `EntityRenderState`.
const ALPHA: f64 = 0.5;

/// How high above the entity a vertex is, in blocks.
fn above(v: &Vertex) -> f32 {
    v.pos[1] - (CENTRE[1] - CAMERA.y) as f32
}

/// The vertex's fields, for a byte-for-byte comparison (`Vertex` is not `PartialEq`).
fn verts(out: &[Vertex]) -> Vec<([f32; 3], [f32; 2], u32, u32, u32, [i8; 4], [f32; 2])> {
    out.iter()
        .map(|v| (v.pos, v.uv, v.layer, v.color, v.light, v.normal, v.glint_uv))
        .collect()
}

fn hanging(id: u32) -> Painting {
    Painting {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id,
        pos: [0, 64, 0],
        facing: FACING,
        variant: Arc::from("minecraft:kebab"),
        asset: None,
        width: WIDTH,
        height: HEIGHT,
        check_interval: 0,
        ticks: 0,
    }
}

/// The vertices one frame of paintings draws for a single entity.
fn painting_frame(game: &mut crate::game::ClientGame<'_>, id: u32) -> Vec<Vertex> {
    game.common.paintings.clear();
    game.common.paintings.push(hanging(id));
    let mut out = Vec::new();
    game.painting_geometry(CAMERA, ALPHA, &mut out);
    out
}

/// An independent baseline of the vanilla painting quads for `p` at `camera`, not going through
/// `painting_geometry`. Used so a deterministic change to the quads cannot pass by matching itself.
fn expected_quads(
    game: &crate::game::ClientGame<'_>,
    p: &Painting,
    camera: glam::DVec3,
) -> Vec<Vertex> {
    let sprites = &game.client.gui.atlas.sprites;
    let Some(back) = sprites.get("painting/back").copied() else {
        return Vec::new();
    };
    let Some(v) = world_paintings!(game).get(&p.variant) else {
        return Vec::new();
    };
    let Some(front) = sprites.get(&format!("painting/{}", v.asset)).copied() else {
        return Vec::new();
    };
    let (lo, hi) = p.bounds();
    let c = glam::DVec3::new(
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    ) - camera;
    let s = p.step().map(f64::from);
    let l = p.left().map(f64::from);
    let out_dir = glam::DVec3::new(s[0], 0.0, s[2]);
    let right = glam::DVec3::new(l[0], 0.0, l[2]);
    let up = glam::DVec3::Y;
    let (hw, hh, d) = (
        f64::from(p.width) / 2.0,
        f64::from(p.height) / 2.0,
        1.0 / 32.0,
    );
    let light = game.common.world.light(p.pos[0], p.pos[1], p.pos[2]);
    let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
    let faces_camera = |corners: &[glam::DVec3; 4], normal: glam::DVec3| {
        let centre = (corners[0] + corners[1] + corners[2] + corners[3]) / 4.0;
        normal.dot(-centre) > 0.0
    };
    let mut out = Vec::new();
    let mut quad =
        |corners: [glam::DVec3; 4], normal: glam::DVec3, uv: [[f32; 2]; 4], shade: f32| {
            if !faces_camera(&corners, normal) {
                return;
            }
            let k = (255.0 * shade) as u32;
            let color = 0xFF00_0000 | k << 16 | k << 8 | k;
            for (p, uv) in corners.iter().zip(uv) {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: p.as_vec3().to_array(),
                    uv,
                    layer: 0,
                    color,
                    light: packed,
                    glint_uv: [0.0, 0.0],
                });
            }
        };
    let full =
        |r: sunrise_client::hud::Rect| [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]];
    let f = c + out_dir * d;
    quad(
        [
            f - right * hw - up * hh,
            f + right * hw - up * hh,
            f + right * hw + up * hh,
            f - right * hw + up * hh,
        ],
        out_dir,
        full(front),
        1.0,
    );
    let b = c - out_dir * d;
    quad(
        [
            b + right * hw - up * hh,
            b - right * hw - up * hh,
            b - right * hw + up * hh,
            b + right * hw + up * hh,
        ],
        -out_dir,
        full(back),
        0.8,
    );
    let strip = [
        [back.u0, back.v0 + (back.v1 - back.v0) / 16.0],
        [back.u1, back.v0 + (back.v1 - back.v0) / 16.0],
        [back.u1, back.v0],
        [back.u0, back.v0],
    ];
    let (top, bt) = (up * hh, -up * hh);
    quad(
        [
            b + top + right * hw,
            b + top - right * hw,
            f + top - right * hw,
            f + top + right * hw,
        ],
        up,
        strip,
        1.0,
    );
    quad(
        [
            f + bt + right * hw,
            f + bt - right * hw,
            b + bt - right * hw,
            b + bt + right * hw,
        ],
        -up,
        strip,
        0.5,
    );
    quad(
        [
            b - right * hw - up * hh,
            f - right * hw - up * hh,
            f - right * hw + up * hh,
            b - right * hw + up * hh,
        ],
        -right,
        strip,
        0.6,
    );
    quad(
        [
            f + right * hw - up * hh,
            b + right * hw - up * hh,
            b + right * hw + up * hh,
            f + right * hw + up * hh,
        ],
        right,
        strip,
        0.6,
    );
    out
}

#[test]
fn a_painting_registration_resolves_to_the_painting_slot() {
    struct Mark;
    impl EntityRenderer for Mark {
        fn submit(
            &self,
            _: &EntityRenderState,
            _: &mut dyn BlockEntityDrawContext,
        ) -> Result<(), String> {
            Ok(())
        }
    }
    let registered = std::collections::BTreeMap::from([(
        "minecraft:painting".to_owned(),
        RegisteredEntityRenderer::new(Arc::new(Mark), None),
    )]);
    let (table, unknown) = EntityRendererTable::resolve(
        &registered,
        &[],
        &Default::default(),
        &sunrise_gameplay_common::game::general::GeneralTypeCatalog::freeze(&[]).unwrap(),
        |id| ["minecraft:painting", "minecraft:oak_boat"].contains(&id),
    );
    assert!(table.painting().is_some());
    assert_eq!(unknown, Vec::<String>::new());
}

/// Runs `check` on a client game with prepared resources and the scene's painting renderer,
/// resolved as the first load does. In a child process running only `test`: the registrations are
/// kept once per process.
fn in_scene(
    test: &str,
    check: impl FnOnce(&mut crate::game::ClientGame<'_>, &Arc<Mutex<Heard>>, &Flaky),
) {
    if std::env::var_os("SUNRISE_PAINTING_RENDERERS_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("game::native_painting_renderers_tests::{test}"),
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_PAINTING_RENDERERS_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let heard = Arc::new(Mutex::new(Heard::default()));
    let flaky = Arc::new(Flaky::default());
    struct CallScene(Arc<Mutex<Heard>>);
    #[mod_api::__subscriptions]
    impl Hooks for CallScene {
        fn register_entity_renderers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterEntityRenderersEvent<'_>,
        ) {
            event
                .renderers
                .register("minecraft:painting".into(), Arc::new(Paint(self.0.clone())));
        }
    }
    struct RefuseScene(Arc<Flaky>);
    #[mod_api::__subscriptions]
    impl Hooks for RefuseScene {
        fn register_entity_renderers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterEntityRenderersEvent<'_>,
        ) {
            event
                .renderers
                .register("minecraft:painting".into(), self.0.clone());
        }
    }
    if test.starts_with("a_failed") {
        let mods = vec![mod_api::ModEntry::new(
            "scene",
            Box::new(RefuseScene(flaky.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::client_side::mods_client::register_entity_renderers(
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            );
            super::native_entity_refs::tests::with_client_game(|game| {
                game.resolve_entity_renderers();
                check(game, &heard, &flaky);
            });
        });
        return;
    }
    let mods = vec![mod_api::ModEntry::new(
        "scene",
        Box::new(CallScene(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::client_side::mods_client::register_entity_renderers(
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        );
        super::native_entity_refs::tests::with_client_game(|game| {
            game.resolve_entity_renderers();
            check(game, &heard, &flaky);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_registered_painting_draws_its_renderer_in_place_of_the_quads() {
    in_scene(
        "a_registered_painting_draws_its_renderer_in_place_of_the_quads",
        |game, heard, _| {
            // A lived-ticks counter and a non-zero partial so age is ticks + partial, not 0.
            let mut painting = hanging(91);
            painting.ticks = 6;
            game.paintings.clear();
            game.paintings.push(painting);
            let mut drawn = Vec::new();
            game.painting_geometry(CAMERA, ALPHA, &mut drawn);
            assert_eq!(drawn.len(), 4, "one quad and no painting: {drawn:?}");
            // Centre [0.96875, 64.5, 0.5] minus CAMERA [-4.0, 64.5, 0.5] is [4.96875, 0.0, 0.0];
            // the raised quad sits 100..101 above that origin, 0..1 in x, at z.
            for v in &drawn {
                assert!(
                    (4.96875..=5.96875).contains(&v.pos[0]),
                    "quad x at the hanging pose: {v:?}"
                );
                assert!(
                    (100.0..=101.0).contains(&v.pos[1]),
                    "quad y 100 blocks above the hanging pose: {v:?}"
                );
                assert!(v.pos[2].abs() < 1e-5, "quad z at the hanging pose: {v:?}");
            }
            let state = heard
                .lock()
                .unwrap()
                .state
                .take()
                .expect("the renderer was asked");
            assert_eq!(state.id, 91);
            assert_eq!(state.partial, 0.5);
            assert_eq!(state.age, 6.5);
            assert_eq!(state.yaw, 90.0);
            assert_eq!(state.pitch, 0.0);
            assert_eq!(state.living, None);
            let painting = state.painting.expect("painting state");
            assert_eq!(painting.facing, Direction::West);
            assert_eq!((painting.width, painting.height), (WIDTH, HEIGHT));
        },
    );
}

/// With no renderer registered, a painting's quads match an independent baseline for a fixed
/// fixture, and two frames of the same entity match byte for byte.
#[test]
#[ignore = "daily"]
fn an_unregistered_painting_is_byte_identical_to_the_vanilla_route() {
    if std::env::var_os("SUNRISE_PAINTING_RENDERERS_PLAIN").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "game::native_painting_renderers_tests::an_unregistered_painting_is_byte_identical_to_the_vanilla_route",
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_PAINTING_RENDERERS_PLAIN", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    super::native_entity_refs::tests::with_client_game(|game| {
        game.resolve_entity_renderers();
        let first = painting_frame(game, 92);
        assert!(!first.is_empty(), "its vanilla quads");
        assert!(
            first.iter().all(|v| above(v) < 2.0),
            "near the entity: {first:?}"
        );
        assert_eq!(
            verts(&first),
            verts(&expected_quads(game, &hanging(92), CAMERA)),
            "matches the independent painting baseline"
        );
        let second = painting_frame(game, 92);
        assert_eq!(
            verts(&first),
            verts(&second),
            "two frames of the same painting match byte for byte"
        );
    });
}

/// A renderer returning `Err` falls back to the quads for that frame only and stays active for
/// later frames and after a resource reload.
#[test]
#[ignore = "daily"]
fn a_failed_painting_renderer_falls_back_to_the_quads_for_that_frame_only() {
    in_scene(
        "a_failed_painting_renderer_falls_back_to_the_quads_for_that_frame_only",
        |game, _, flaky| {
            let drawn = painting_frame(game, 93);
            assert!(!drawn.is_empty(), "the failing frame: its own quads");
            assert!(
                drawn.iter().all(|v| above(v) < 2.0),
                "the failing frame: no quad kept: {drawn:?}"
            );
            assert_eq!(flaky.calls(), 1);

            let drawn = painting_frame(game, 93);
            assert_eq!(flaky.calls(), 2, "asked again on the next frame");
            assert_eq!(drawn.len(), 4, "one quad and no painting: {drawn:?}");
            assert!(drawn.iter().all(|v| (400.0..=401.0).contains(&above(v))));

            crate::game::client_side::model_baking_completed_tests::reload(game);
            let drawn = painting_frame(game, 93);
            assert_eq!(flaky.calls(), 3, "the reload resolved it again");
            assert_eq!(drawn.len(), 4, "one quad and no painting: {drawn:?}");
            assert!(drawn.iter().all(|v| (400.0..=401.0).contains(&above(v))));
        },
    );
}
