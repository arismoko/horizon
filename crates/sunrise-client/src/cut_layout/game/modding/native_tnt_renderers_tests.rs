//! `register_entity_renderers` for primed TNT (#1296): a renderer registered for
//! `minecraft:tnt` draws in place of the primed TNT cube, reading the TNT fuse; with no
//! renderer the family's own geometry is unchanged; an `Err` leaves its cube for just that draw.
#![cfg(feature = "client")]
use mod_api::{
    BlockEntityDrawContext, EntityRenderState, EntityRenderer, Hooks, RegisterEntityRenderersEvent,
    RegistrationContext,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use sunrise_assets::model::Direction as Face;
use sunrise_client::mesher::Vertex;
use sunrise_client_gameplay::game::native_entity_renderers::{
    EntityRendererTable, RegisteredEntityRenderer,
};
use sunrise_core::geometry::Vec3;
use sunrise_core::position::BlockPos;

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

/// What the TNT renderer last read, and whether it was asked.
#[derive(Default)]
struct Heard {
    state: Option<EntityRenderState>,
}

/// Draws a quad 100 blocks up and records the frame it read.
struct Tnt(Arc<Mutex<Heard>>);
impl EntityRenderer for Tnt {
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

/// The camera, south of the entity and looking at it.
const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.0, -4.5);
const FEET: [f64; 3] = [0.5, 64.0, 0.5];
const ALPHA: f64 = 0.5;
const FUSE: i32 = 80;

/// How high above the entity's feet a vertex is, in blocks.
fn above(v: &Vertex) -> f32 {
    v.pos[1] - (FEET[1] - CAMERA.y) as f32
}

/// The vertex's fields, for a byte-for-byte comparison (`Vertex` is not `PartialEq`).
fn verts(out: &[Vertex]) -> Vec<([f32; 3], [f32; 2], u32, u32, u32, [i8; 4], [f32; 2])> {
    out.iter()
        .map(|v| (v.pos, v.uv, v.layer, v.color, v.light, v.normal, v.glint_uv))
        .collect()
}

/// Remaining fuse as the cube draws it (`fuse - partial + 1`).
fn remaining_fuse(fuse: i32, alpha: f64) -> f32 {
    fuse as f32 - alpha as f32 + 1.0
}

/// Late-fuse swell: `1 + g^4 * 0.3` when the remaining fuse is under 10 ticks.
fn cube_scale(remaining: f32) -> f32 {
    if remaining < 10.0 {
        let g = (1.0 - remaining / 10.0).clamp(0.0, 1.0);
        1.0 + g * g * g * g * 0.3
    } else {
        1.0
    }
}

/// White flash every 5 ticks of remaining fuse.
fn cube_flashing(remaining: f32) -> bool {
    remaining >= 0.0 && (remaining / 5.0) as i32 % 2 == 0
}

/// An independent baseline of the vanilla TNT cube for `tnt` at `alpha`, not going through
/// `tnt_geometry`. Used so a deterministic change to the cube cannot pass by matching itself.
fn expected_cube(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    tnt: &sunrise_gameplay_common::game::tnt_common::PrimedTnt,
    camera: glam::DVec3,
    alpha: f64,
) -> Vec<Vertex> {
    let Some(state) = game.pack.assets.registry.default_state("minecraft:tnt") else {
        return Vec::new();
    };
    let Ok(models) = game
        .client
        .client_pack
        .render
        .model_catalog
        .collect(state, BlockPos::new(0, 0, 0))
    else {
        return Vec::new();
    };
    let mut sprites = [None; 6];
    for face in models.iter().flat_map(|(_, m)| m.faces.iter()) {
        let index = match face.quad.facing {
            Face::Up => 0,
            Face::Down => 1,
            Face::North => 2,
            Face::South => 3,
            Face::West => 4,
            Face::East => 5,
        };
        if sprites[index].is_none() {
            sprites[index] = game
                .client
                .client_pack
                .layer_names
                .get(&face.layer)
                .and_then(|name| game.client.gui.atlas.sprites.get(name).copied());
        }
    }
    let remaining = remaining_fuse(tnt.fuse, alpha);
    let scale = cube_scale(remaining);
    let lit = cube_flashing(remaining);
    let base = glam::DVec3::new(
        tnt.previous.x + (tnt.position.x - tnt.previous.x) * alpha,
        tnt.previous.y + (tnt.position.y - tnt.previous.y) * alpha + 0.5,
        tnt.previous.z + (tnt.position.z - tnt.previous.z) * alpha,
    ) - camera;
    let o = base.as_vec3();
    let light = game.world.light(
        tnt.position.x.floor() as i32,
        (tnt.position.y + 0.5).floor() as i32,
        tnt.position.z.floor() as i32,
    );
    let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
    let v = |x: f32, y: f32, z: f32| o + glam::Vec3::new(x - 0.5, y - 0.5, z - 0.5) * scale;
    let faces = [
        (
            [v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.), v(0., 1., 0.)],
            1.0,
        ),
        (
            [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
            0.5,
        ),
        (
            [v(1., 0., 0.), v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.)],
            0.8,
        ),
        (
            [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
            0.8,
        ),
        (
            [v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.), v(0., 1., 0.)],
            0.6,
        ),
        (
            [v(1., 0., 1.), v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.)],
            0.6,
        ),
    ];
    let mut out = Vec::new();
    for ((corners, shade), sprite) in faces.into_iter().zip(sprites) {
        let Some(r) = sprite else { continue };
        let c = (255.0 * shade) as u32;
        let color = 0xFF00_0000 | c << 16 | c << 8 | c;
        for (p, uv) in corners
            .iter()
            .zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]])
        {
            out.push(Vertex {
                normal: [0; 4],
                pos: p.to_array(),
                uv,
                layer: if lit { 5 } else { 0 },
                color,
                light: packed,
                glint_uv: [0.0, 0.0],
            });
        }
    }
    out
}

fn primed(id: u32, fuse: i32) -> sunrise_gameplay_common::game::tnt_common::PrimedTnt {
    let at = Vec3::new(FEET[0], FEET[1], FEET[2]);
    sunrise_gameplay_common::game::tnt_common::PrimedTnt {
        size_answer: None,
        attachment_lifetime: Default::default(),
        rotation: [90.0, -10.0],
        join: Default::default(),
        id,
        position: at,
        previous: at,
        velocity: Vec3::ZERO,
        fuse,
        ticks: 0,
        ground: Default::default(),
        fall_distance: 0.0,
        stuck: Vec3::ZERO,
        in_water: false,
        owner: None,
        guest: None,
        no_gravity: false,
        power: 4.0,
        used_portal: false,
        block_state: None,
    }
}

/// The vertices one frame of primed TNT draws for a single entity.
fn tnt_frame(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: u32,
    fuse: i32,
) -> Vec<Vertex> {
    game.tnts.clear();
    game.tnts.push(primed(id, fuse));
    let mut out = Vec::new();
    game.tnt_geometry(
        CAMERA,
        ALPHA,
        &mut out,
        0..game.tnts.len(),
        game.tnt_face_sprites(),
    );
    out
}

#[test]
fn a_tnt_registration_resolves_to_the_tnt_slot() {
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
        "minecraft:tnt".to_owned(),
        RegisteredEntityRenderer::new(Arc::new(Mark), None),
    )]);
    let (table, unknown) = EntityRendererTable::resolve(
        &registered,
        &[],
        &Default::default(),
        &sunrise_gameplay_common::game::general::GeneralTypeCatalog::freeze(&[]).unwrap(),
        |id| ["minecraft:tnt", "minecraft:oak_boat"].contains(&id),
    );
    assert!(table.tnt().is_some());
    assert_eq!(unknown, Vec::<String>::new());
}

/// Runs `check` on a client game with prepared resources and the scene's TNT renderer, resolved
/// as the first load does. In a child process running only `test`: the registrations are kept
/// once per process.
fn in_scene(
    test: &str,
    check: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, &Arc<Mutex<Heard>>, &Flaky),
) {
    if std::env::var_os("SUNRISE_TNT_RENDERERS_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("cut_layout::game::native_tnt_renderers_tests::{test}"),
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_TNT_RENDERERS_CHILD", "1")
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
                .register("minecraft:tnt".into(), Arc::new(Tnt(self.0.clone())));
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
                .register("minecraft:tnt".into(), self.0.clone());
        }
    }
    if test.starts_with("a_failed") {
        let mods = vec![mod_api::ModEntry::new(
            "scene",
            Box::new(RefuseScene(flaky.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_client_gameplay::game::client_side::mods_client::register_entity_renderers(
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            );
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
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
        sunrise_client_gameplay::game::client_side::mods_client::register_entity_renderers(
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        );
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            game.resolve_entity_renderers();
            check(game, &heard, &flaky);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_registered_tnt_draws_its_renderer_in_place_of_the_cube() {
    in_scene(
        "a_registered_tnt_draws_its_renderer_in_place_of_the_cube",
        |game, heard, _| {
            // Previous and current differ so the pose is the interpolated position, not the feet.
            let mut tnt = primed(91, FUSE);
            tnt.previous = Vec3::new(0.5, 64.0, 0.5);
            tnt.position = Vec3::new(1.5, 65.0, 2.5);
            tnt.ticks = 6;
            game.tnts.clear();
            game.tnts.push(tnt);
            let mut drawn = Vec::new();
            game.tnt_geometry(
                CAMERA,
                ALPHA,
                &mut drawn,
                0..game.tnts.len(),
                game.tnt_face_sprites(),
            );
            assert_eq!(drawn.len(), 4, "one quad and no cube: {drawn:?}");
            // Interpolated position [1.0, 64.5, 1.5] minus CAMERA [0.5, 64.0, -4.5] is
            // [0.5, 0.5, 6.0]; the raised quad sits 100..101 above that origin, 0..1 in x, at z.
            for v in &drawn {
                assert!(
                    (0.5..=1.5).contains(&v.pos[0]),
                    "quad x at the interpolated pose: {v:?}"
                );
                assert!(
                    (100.5..=101.5).contains(&v.pos[1]),
                    "quad y 100 blocks above the interpolated pose: {v:?}"
                );
                assert!(
                    (v.pos[2] - 6.0).abs() < 1e-5,
                    "quad z at the interpolated pose: {v:?}"
                );
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
            assert_eq!(state.pitch, -10.0);
            assert_eq!(state.living, None);
            assert_eq!(state.tnt_fuse, Some(80.5));
        },
    );
}

/// With no renderer registered, primed TNT's cube matches an independent baseline for a fixed
/// fixture in each of the cube's modes (normal, flashing, and late-fuse scale), and two frames of
/// the same entity match byte for byte.
#[test]
#[ignore = "daily"]
fn an_unregistered_tnt_is_byte_identical_to_the_vanilla_route() {
    if std::env::var_os("SUNRISE_TNT_RENDERERS_PLAIN").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cut_layout::game::native_tnt_renderers_tests::an_unregistered_tnt_is_byte_identical_to_the_vanilla_route",
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_TNT_RENDERERS_PLAIN", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        game.resolve_entity_renderers();
        // fuse 79 + partial 0.5 → remaining 79.5: scale 1, not flashing.
        // fuse 80 + partial 0.5 → remaining 80.5: scale 1, flashing.
        // fuse 2 + partial 0.5 → remaining 2.5: late-fuse scale, flashing.
        for (fuse, label, expect_flash, late) in [
            (79, "normal", false, false),
            (80, "flashing", true, false),
            (2, "late-fuse scale", true, true),
        ] {
            let tnt = primed(92, fuse);
            let drawn = tnt_frame(game, 92, fuse);
            assert!(!drawn.is_empty(), "{label}: its vanilla cube");
            assert!(
                drawn
                    .iter()
                    .all(|v| above(v) < 2.0 || late && above(v) < 3.0),
                "{label}: near the entity: {drawn:?}"
            );
            let remaining = remaining_fuse(fuse, ALPHA);
            assert_eq!(cube_flashing(remaining), expect_flash, "{label}: flash");
            assert_eq!(cube_scale(remaining) > 1.0, late, "{label}: scale");
            assert_eq!(
                verts(&drawn),
                verts(&expected_cube(game, &tnt, CAMERA, ALPHA)),
                "{label}: matches the independent cube baseline"
            );
        }
        let first = tnt_frame(game, 92, FUSE);
        let second = tnt_frame(game, 92, FUSE);
        assert_eq!(
            verts(&first),
            verts(&second),
            "two frames of the same TNT match byte for byte"
        );
    });
}

/// A renderer returning `Err` falls back to the cube for that frame only and stays active for
/// later frames and after a resource reload.
#[test]
#[ignore = "daily"]
fn a_failed_tnt_renderer_falls_back_to_the_cube_for_that_frame_only() {
    in_scene(
        "a_failed_tnt_renderer_falls_back_to_the_cube_for_that_frame_only",
        |game, _, flaky| {
            let drawn = tnt_frame(game, 93, FUSE);
            assert!(!drawn.is_empty(), "the failing frame: its own cube");
            assert!(
                drawn.iter().all(|v| above(v) < 2.0),
                "the failing frame: no quad kept: {drawn:?}"
            );
            assert_eq!(flaky.calls(), 1);

            let drawn = tnt_frame(game, 93, FUSE);
            assert_eq!(flaky.calls(), 2, "asked again on the next frame");
            assert_eq!(drawn.len(), 4, "one quad and no cube: {drawn:?}");
            assert!(drawn.iter().all(|v| (400.0..=401.0).contains(&above(v))));

            sunrise_client_gameplay::game::client_side::model_baking_completed_tests::reload(game);
            let drawn = tnt_frame(game, 93, FUSE);
            assert_eq!(flaky.calls(), 3, "the reload resolved it again");
            assert_eq!(drawn.len(), 4, "one quad and no cube: {drawn:?}");
            assert!(drawn.iter().all(|v| (400.0..=401.0).contains(&above(v))));
        },
    );
}
