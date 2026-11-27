//! `register_entity_renderers` for experience orbs (#1296): a renderer registered for
//! `minecraft:experience_orb` draws in place of the billboarded orb quad, reading the orb's
//! value; with no renderer the family's own geometry is unchanged; a failing renderer leaves that
//! quad until a resource reload resolves it again.
use super::native_entity_renderers::{EntityRendererTable, RegisteredEntityRenderer};
use mod_api::{
    BlockEntityDrawContext, EntityRenderState, EntityRenderer, Hooks, RegisterEntityRenderersEvent,
    RegistrationContext,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use sunrise_client::mesher::Vertex;
use sunrise_core::geometry::Vec3;
use sunrise_sim::entities::ExperienceOrb;

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

/// What the orb renderer last read, and whether it was asked.
#[derive(Default)]
struct Heard {
    state: Option<EntityRenderState>,
}

/// Draws a quad 100 blocks up and records the frame it read.
struct Orb(Arc<Mutex<Heard>>);
impl EntityRenderer for Orb {
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

/// The camera, south of the orb and looking at it.
const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.0, -4.5);
const FEET: [f64; 3] = [0.5, 64.0, 0.5];
const ALPHA: f32 = 0.5;
const VALUE: i32 = 11;
const AGE: i32 = 6;

/// How high above the entity's position a vertex is, in blocks.
fn above(v: &Vertex) -> f32 {
    v.pos[1] - (FEET[1] - CAMERA.y) as f32
}

/// The vertex's fields, for a byte-for-byte comparison (`Vertex` is not `PartialEq`).
fn verts(out: &[Vertex]) -> Vec<([f32; 3], [f32; 2], u32, u32, u32, [i8; 4], [f32; 2])> {
    out.iter()
        .map(|v| (v.pos, v.uv, v.layer, v.color, v.light, v.normal, v.glint_uv))
        .collect()
}

fn hanging(id: u32, previous: Vec3) -> ExperienceOrb {
    let at = Vec3::new(FEET[0], FEET[1], FEET[2]);
    ExperienceOrb {
        size_answer: None,
        attachment_lifetime: Default::default(),
        fall_distance: 0.0,
        rotation: [90.0, -10.0],
        join: Default::default(),
        id,
        position: at,
        previous,
        velocity: Vec3::ZERO,
        value: VALUE,
        age: AGE,
        ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
        count: 1,
        fire_ticks: 0,
        ticks: 0,
        health: 5,
        air: 300,
        no_gravity: true,
        following: None,
    }
}

/// The vertices one frame of experience orbs draws for a single entity.
fn orb_frame(game: &mut crate::game::ClientGame<'_>, id: u32) -> Vec<Vertex> {
    orb_frame_from(game, id, Vec3::new(FEET[0], FEET[1], FEET[2]))
}

/// [`orb_frame`] for an orb that moved to its position from `previous`.
fn orb_frame_from(game: &mut crate::game::ClientGame<'_>, id: u32, previous: Vec3) -> Vec<Vertex> {
    game.common.orbs.clear();
    game.common.orbs.push(hanging(id, previous));
    let live = game.common.orbs.iter().map(|o| {
        let at = [
            o.previous.x + (o.position.x - o.previous.x) * f64::from(ALPHA),
            o.previous.y + (o.position.y - o.previous.y) * f64::from(ALPHA),
            o.previous.z + (o.position.z - o.previous.z) * f64::from(ALPHA),
        ];
        (o, at, ALPHA)
    });
    let mut out = Vec::new();
    let frame = game.orb_frame();
    game.orb_geometry(CAMERA, &frame, live, None, &mut out);
    out
}

#[test]
fn an_experience_orb_registration_resolves_to_the_orb_slot() {
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
        "minecraft:experience_orb".to_owned(),
        RegisteredEntityRenderer::new(Arc::new(Mark), None),
    )]);
    let (table, unknown) = EntityRendererTable::resolve(
        &registered,
        &[],
        &Default::default(),
        &sunrise_gameplay_common::game::general::GeneralTypeCatalog::freeze(&[]).unwrap(),
        |id| ["minecraft:experience_orb", "minecraft:oak_boat"].contains(&id),
    );
    assert!(table.experience_orb().is_some());
    assert_eq!(unknown, Vec::<String>::new());
}

/// Runs `check` on a client game with prepared resources and the scene's orb renderer, resolved
/// as the first load does. In a child process running only `test`: the registrations are kept
/// once per process.
fn in_scene(
    test: &str,
    check: impl FnOnce(&mut crate::game::ClientGame<'_>, &Arc<Mutex<Heard>>, &Flaky),
) {
    if std::env::var_os("SUNRISE_EXPERIENCE_ORB_RENDERERS_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("game::native_experience_orb_renderers_tests::{test}"),
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_EXPERIENCE_ORB_RENDERERS_CHILD", "1")
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
                "minecraft:experience_orb".into(),
                Arc::new(Orb(self.0.clone())),
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
                .register("minecraft:experience_orb".into(), self.0.clone());
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
fn a_registered_experience_orb_draws_its_renderer_in_place_of_the_quad() {
    in_scene(
        "a_registered_experience_orb_draws_its_renderer_in_place_of_the_quad",
        |game, heard, _| {
            // Moved from (-0.5, 60, -1.5) to (0.5, 64, 0.5), half way (ALPHA) it stands at
            // (0, 62, -0.5); the camera at (0.5, 64, -4.5) puts it at (-0.5, -2, 4) from there.
            // The quad's corners, in blocks from the entity: x 0 and 1, y 100 and 101, z 0.
            let drawn = orb_frame_from(game, 91, Vec3::new(-0.5, 60.0, -1.5));
            assert_eq!(drawn.len(), 4, "one quad and no orb: {drawn:?}");
            let corners: Vec<[f32; 3]> = drawn.iter().map(|v| v.pos).collect();
            assert_eq!(
                corners,
                [
                    [-0.5, 99.0, 4.0],
                    [-0.5, 98.0, 4.0],
                    [0.5, 98.0, 4.0],
                    [0.5, 99.0, 4.0],
                ]
            );
            let state = heard
                .lock()
                .unwrap()
                .state
                .take()
                .expect("the renderer was asked");
            assert_eq!(state.id, 91);
            assert_eq!(state.partial, ALPHA);
            assert_eq!(state.age, AGE as f32 + ALPHA);
            assert_eq!(state.yaw, 90.0);
            assert_eq!(state.pitch, -10.0);
            assert_eq!(state.living, None);
            assert_eq!(state.orb_value, Some(VALUE));
        },
    );
}

/// With no renderer registered, an experience orb's geometry is byte-identical to the family's
/// own route (the quad near the entity, no raised quad), and two frames of the same entity match.
#[test]
#[ignore = "daily"]
fn an_unregistered_experience_orb_is_byte_identical_to_the_vanilla_route() {
    if std::env::var_os("SUNRISE_EXPERIENCE_ORB_RENDERERS_PLAIN").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "game::native_experience_orb_renderers_tests::an_unregistered_experience_orb_is_byte_identical_to_the_vanilla_route",
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_EXPERIENCE_ORB_RENDERERS_PLAIN", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    super::native_entity_refs::tests::with_client_game(|game| {
        game.resolve_entity_renderers();
        let first = orb_frame(game, 92);
        assert!(!first.is_empty(), "its vanilla quad");
        assert!(
            first.iter().all(|v| above(v) < 2.0),
            "near the entity: {first:?}"
        );
        let second = orb_frame(game, 92);
        assert_eq!(
            verts(&first),
            verts(&second),
            "two frames of the same orb match byte for byte"
        );
    });
}

/// A renderer returning `Err` falls back to the quad for that frame only and stays active for
/// later frames and after a resource reload.
#[test]
#[ignore = "daily"]
fn a_failed_experience_orb_renderer_falls_back_to_the_quad_for_that_frame_only() {
    in_scene(
        "a_failed_experience_orb_renderer_falls_back_to_the_quad_for_that_frame_only",
        |game, _, flaky| {
            let drawn = orb_frame(game, 93);
            assert!(!drawn.is_empty(), "the failing frame: its own quad");
            assert!(
                drawn.iter().all(|v| above(v) < 2.0),
                "the failing frame: no quad kept: {drawn:?}"
            );
            assert_eq!(flaky.calls(), 1);

            let drawn = orb_frame(game, 93);
            assert_eq!(flaky.calls(), 2, "asked again on the next frame");
            assert_eq!(drawn.len(), 4, "one quad and no orb: {drawn:?}");
            assert!(drawn.iter().all(|v| (400.0..=401.0).contains(&above(v))));

            crate::game::client_side::model_baking_completed_tests::reload(game);
            let drawn = orb_frame(game, 93);
            assert_eq!(flaky.calls(), 3, "the reload resolved it again");
            assert_eq!(drawn.len(), 4, "one quad and no orb: {drawn:?}");
            assert!(drawn.iter().all(|v| (400.0..=401.0).contains(&above(v))));
        },
    );
}
