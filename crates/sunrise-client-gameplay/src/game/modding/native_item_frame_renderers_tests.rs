//! `register_entity_renderers` for item frames and glow item frames (#1296): a renderer
//! registered for `minecraft:item_frame` or `minecraft:glow_item_frame` draws in place of the
//! frame model and the item or map on its face, reading the frame's own state; with no renderer
//! the family's own geometry is unchanged; an `Err` leaves that geometry for just that draw.
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
use sunrise_items::items::ItemStack;
use sunrise_sim::entities::ItemFrame;

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

/// What the frame renderer last read, and whether it was asked.
#[derive(Default)]
struct Heard {
    state: Option<EntityRenderState>,
}

/// Draws a quad 100 blocks up and records the frame it read.
struct Frame(Arc<Mutex<Heard>>);
impl EntityRenderer for Frame {
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

/// The camera, south of a south-facing frame and looking at its front.
const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.5, 4.5);
/// The hanging entity's position: the centre of a frame at `[0, 64, 0]` facing south.
const CENTRE: [f64; 3] = [0.5, 64.5, 0.03125];
const FACING: u8 = 3;
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

fn hanging(id: u32, glow: bool) -> ItemFrame {
    ItemFrame {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id,
        pos: [0, 64, 0],
        facing: FACING,
        glow,
        item: None,
        rotation: 0,
        drop_chance: 1.0,
        fixed: false,
        invisible: false,
        check_interval: 0,
        ticks: 0,
        custom_name: None,
        random: sunrise_level::random_source::entity(),
    }
}

/// The vertices one frame of item frames draws for a single entity.
fn frame_frame(game: &mut crate::game::ClientGame<'_>, id: u32, glow: bool) -> Vec<Vertex> {
    game.common.item_frames.clear();
    game.common.item_frames.push(hanging(id, glow));
    let mut out = Vec::new();
    game.frame_geometry(CAMERA, ALPHA, &mut out);
    out
}

/// An independent baseline of the vanilla item-frame model for `f` at `camera`, not going through
/// `frame_geometry`. Used so a deterministic change to the model cannot pass by matching itself.
/// The framed item, if any, is not included: the registered-renderer test checks that a
/// successful replacement drops the contents, and the fallback test checks that they stay.
fn expected_frame(
    game: &crate::game::ClientGame<'_>,
    f: &ItemFrame,
    camera: glam::DVec3,
) -> Vec<Vertex> {
    let sprites = &game.client.gui.atlas.sprites;
    let s = f.step();
    let (x_rot, y_rot) = if s[1] == 0 {
        let to_y = match f.facing {
            3 => 0.0,
            4 => 90.0,
            2 => 180.0,
            _ => 270.0,
        };
        (0.0f64, 180.0f64 - to_y)
    } else {
        (-90.0 * f64::from(s[1]), 180.0)
    };
    let pose = glam::DMat4::from_translation(
        glam::DVec3::new(
            f64::from(f.pos[0]) + 0.5,
            f64::from(f.pos[1]) + 0.5,
            f64::from(f.pos[2]) + 0.5,
        ) - camera,
    ) * glam::DMat4::from_rotation_x(x_rot.to_radians())
        * glam::DMat4::from_rotation_y(y_rot.to_radians());
    let light = game.common.world.light(f.pos[0], f.pos[1], f.pos[2]);
    let frame_light = if f.glow {
        (light & 0xF0) | (light & 15).max(5)
    } else {
        light
    };
    let frame_packed =
        (u32::from(frame_light >> 4) * 16) | ((u32::from(frame_light & 15) * 16) << 8);
    // These fixtures never hold a map, so the ordinary (non-map) frame models apply.
    let model = if f.glow {
        &game.client.glow_frame_model
    } else {
        &game.client.frame_model
    };
    let mut out = Vec::new();
    if !f.invisible {
        for (corners, uv, sprite) in model {
            let Some(r) = sprites.get(sprite) else {
                continue;
            };
            let at = corners.map(|c| {
                pose.transform_point3(glam::DVec3::new(
                    f64::from(c[0]) - 0.5,
                    f64::from(c[1]) - 0.5,
                    f64::from(c[2]) - 0.5,
                ))
            });
            let n = (at[1] - at[0]).cross(at[2] - at[0]).normalize_or_zero();
            let normal = [0, 1, 2].map(|k| (n[k].clamp(-1.0, 1.0) * 127.0) as i8);
            let mapped = uv.map(|[u, v]| [r.u0 + u * (r.u1 - r.u0), r.v0 + v * (r.v1 - r.v0)]);
            for (point, uv) in at.into_iter().zip(mapped) {
                out.push(Vertex {
                    normal: [normal[0], normal[1], normal[2], -127],
                    pos: point.as_vec3().to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: frame_packed,
                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }
    if f.glow {
        for v in &mut out {
            v.layer |= u32::from(sunrise_client::mesher::INCANDESCENT) << 16;
        }
    }
    out
}

#[test]
fn an_item_frame_registration_resolves_to_the_frame_slots() {
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
    let registered = std::collections::BTreeMap::from([
        (
            "minecraft:item_frame".to_owned(),
            RegisteredEntityRenderer::new(Arc::new(Mark), None),
        ),
        (
            "minecraft:glow_item_frame".to_owned(),
            RegisteredEntityRenderer::new(Arc::new(Mark), None),
        ),
    ]);
    let (table, unknown) = EntityRendererTable::resolve(
        &registered,
        &[],
        &Default::default(),
        &sunrise_gameplay_common::game::general::GeneralTypeCatalog::freeze(&[]).unwrap(),
        |id| {
            [
                "minecraft:item_frame",
                "minecraft:glow_item_frame",
                "minecraft:oak_boat",
            ]
            .contains(&id)
        },
    );
    assert!(table.item_frame(false).is_some());
    assert!(table.item_frame(true).is_some());
    assert_eq!(unknown, Vec::<String>::new());
}

/// Runs `check` on a client game with prepared resources and the scene's frame renderers,
/// resolved as the first load does. In a child process running only `test`: the registrations are
/// kept once per process.
fn in_scene(
    test: &str,
    check: impl FnOnce(&mut crate::game::ClientGame<'_>, &Arc<Mutex<Heard>>, &Flaky),
) {
    if std::env::var_os("SUNRISE_ITEM_FRAME_RENDERERS_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("game::client_side::native_item_frame_renderers_tests::{test}"),
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_ITEM_FRAME_RENDERERS_CHILD", "1")
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
            event.renderers.register(
                "minecraft:item_frame".into(),
                Arc::new(Frame(self.0.clone())),
            );
            event.renderers.register(
                "minecraft:glow_item_frame".into(),
                Arc::new(Frame(self.0.clone())),
            );
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
                .register("minecraft:item_frame".into(), self.0.clone());
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
                super::execution_role::ExecutionRole::Client,
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
            super::execution_role::ExecutionRole::Client,
        );
        super::native_entity_refs::tests::with_client_game(|game| {
            game.resolve_entity_renderers();
            check(game, &heard, &flaky);
        });
    });
}

fn assert_frame_state(state: EntityRenderState, id: u32, glow: bool, age: f32, has_item: bool) {
    assert_eq!(state.id, id);
    assert_eq!(state.partial, 0.5);
    assert_eq!(state.age, age);
    assert_eq!(state.yaw, 0.0);
    assert_eq!(state.pitch, 0.0);
    assert_eq!(state.living, None);
    let frame = state.item_frame.expect("item frame state");
    assert_eq!(frame.facing, Direction::South);
    assert_eq!(frame.glow, glow);
    assert!(!frame.invisible);
    assert_eq!(frame.rotation, 0);
    assert_eq!(frame.has_item, has_item);
    assert!(!frame.has_map);
}

/// The hanging pose of a south-facing frame at `[0, 64, 0]`, relative to CAMERA.
/// Centre [0.5, 64.5, 0.03125] minus CAMERA [0.5, 64.5, 4.5] is [0.0, 0.0, -4.46875].
fn assert_raised_at_hanging_pose(drawn: &[Vertex]) {
    for v in drawn {
        assert!(
            (0.0..=1.0).contains(&v.pos[0]),
            "quad x at the hanging pose: {v:?}"
        );
        assert!(
            (100.0..=101.0).contains(&v.pos[1]),
            "quad y 100 blocks above the hanging pose: {v:?}"
        );
        assert!(
            (v.pos[2] + 4.46875).abs() < 1e-5,
            "quad z at the hanging pose: {v:?}"
        );
    }
}

#[test]
#[ignore = "daily"]
fn a_registered_item_frame_draws_its_renderer_in_place_of_the_model() {
    in_scene(
        "a_registered_item_frame_draws_its_renderer_in_place_of_the_model",
        |game, heard, _| {
            // A lived-ticks counter, a non-zero partial, and a held item: age is ticks +
            // partial, and a successful replacement drops the frame and its contents.
            let mut frame = hanging(91, false);
            frame.ticks = 6;
            frame.item = Some(ItemStack::new("minecraft:diamond", 1));
            game.item_frames.clear();
            game.item_frames.push(frame);
            let mut drawn = Vec::new();
            game.frame_geometry(CAMERA, ALPHA, &mut drawn);
            assert_eq!(drawn.len(), 4, "one quad, no frame and no item: {drawn:?}");
            assert_raised_at_hanging_pose(&drawn);
            let state = heard
                .lock()
                .unwrap()
                .state
                .take()
                .expect("the renderer was asked");
            assert_frame_state(state, 91, false, 6.5, true);

            heard.lock().unwrap().state = None;
            let mut glow = hanging(94, true);
            glow.ticks = 3;
            game.item_frames.clear();
            game.item_frames.push(glow);
            let mut drawn = Vec::new();
            game.frame_geometry(CAMERA, ALPHA, &mut drawn);
            assert_eq!(drawn.len(), 4, "glow: one quad and no frame: {drawn:?}");
            assert_raised_at_hanging_pose(&drawn);
            let state = heard
                .lock()
                .unwrap()
                .state
                .take()
                .expect("the glow renderer was asked");
            assert_frame_state(state, 94, true, 3.5, false);
        },
    );
}

/// With no renderer registered, an item frame's model matches an independent baseline for a
/// fixed fixture (ordinary and glow), two frames of the same entity match byte for byte, and a
/// frame that holds an item keeps those contents in the fallback.
#[test]
#[ignore = "daily"]
fn an_unregistered_item_frame_is_byte_identical_to_the_vanilla_route() {
    if std::env::var_os("SUNRISE_ITEM_FRAME_RENDERERS_PLAIN").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "game::client_side::native_item_frame_renderers_tests::an_unregistered_item_frame_is_byte_identical_to_the_vanilla_route",
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_ITEM_FRAME_RENDERERS_PLAIN", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    super::native_entity_refs::tests::with_client_game(|game| {
        game.resolve_entity_renderers();
        let first = frame_frame(game, 92, false);
        assert!(!first.is_empty(), "its vanilla frame model");
        assert!(
            first.iter().all(|v| above(v) < 2.0),
            "near the entity: {first:?}"
        );
        assert_eq!(
            verts(&first),
            verts(&expected_frame(game, &hanging(92, false), CAMERA)),
            "matches the independent item-frame baseline"
        );
        let second = frame_frame(game, 92, false);
        assert_eq!(
            verts(&first),
            verts(&second),
            "two frames of the same item frame match byte for byte"
        );
        let glow = frame_frame(game, 95, true);
        assert!(!glow.is_empty(), "its vanilla glow frame model");
        assert!(
            glow.iter().all(|v| above(v) < 2.0),
            "glow near the entity: {glow:?}"
        );
        assert_eq!(
            verts(&glow),
            verts(&expected_frame(game, &hanging(95, true), CAMERA)),
            "glow matches the independent item-frame baseline"
        );
        let mut loaded = hanging(96, false);
        loaded.item = Some(ItemStack::new("minecraft:diamond", 1));
        game.item_frames.clear();
        game.item_frames.push(loaded);
        let mut with_item = Vec::new();
        game.frame_geometry(CAMERA, ALPHA, &mut with_item);
        assert!(
            with_item.len() > first.len(),
            "fallback keeps the framed item: empty {} verts, with item {} verts: {with_item:?}",
            first.len(),
            with_item.len()
        );
        assert!(
            with_item.iter().all(|v| above(v) < 2.0),
            "item stays near the entity: {with_item:?}"
        );
    });
}

/// A renderer returning `Err` falls back to the frame model for that frame only and stays active
/// for later frames and after a resource reload.
#[test]
#[ignore = "daily"]
fn a_failed_item_frame_renderer_falls_back_to_the_model_for_that_frame_only() {
    in_scene(
        "a_failed_item_frame_renderer_falls_back_to_the_model_for_that_frame_only",
        |game, _, flaky| {
            let drawn = frame_frame(game, 93, false);
            assert!(!drawn.is_empty(), "the failing frame: its own model");
            assert!(
                drawn.iter().all(|v| above(v) < 2.0),
                "the failing frame: no quad kept: {drawn:?}"
            );
            assert_eq!(flaky.calls(), 1);

            let drawn = frame_frame(game, 93, false);
            assert_eq!(flaky.calls(), 2, "asked again on the next frame");
            assert_eq!(drawn.len(), 4, "one quad and no frame: {drawn:?}");
            assert!(drawn.iter().all(|v| (400.0..=401.0).contains(&above(v))));

            crate::game::client_side::model_baking_completed_tests::reload(game);
            let drawn = frame_frame(game, 93, false);
            assert_eq!(flaky.calls(), 3, "the reload resolved it again");
            assert_eq!(drawn.len(), 4, "one quad and no frame: {drawn:?}");
            assert!(drawn.iter().all(|v| (400.0..=401.0).contains(&above(v))));
        },
    );
}
