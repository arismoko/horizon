//! `register_entity_renderers` on the client (#860's acceptance): a content mod's thrown type with
//! a renderer draws that renderer's geometry in place of its flying item, one without keeps its
//! item, a vanilla boat is unchanged, the later registration for a type wins, an unknown type is
//! refused by name, and a resource reload keeps the registered renderer in use.
#![cfg(feature = "client")]
use mod_api::{BlockEntityDrawContext, EntityRenderState, EntityRenderer};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};
use sunrise_client::mesher::Vertex;
use sunrise_client_gameplay::game::native_entity_renderers::{
    EntityRendererTable, RegisteredEntityRenderer,
};
use sunrise_sim::projectiles::ThrownType;

/// Draws one stone quad a block wide, `height` blocks above the entity; `drew` says whether it was
/// asked and the sprite was there.
struct Raised {
    pub height: f32,
    drew: AtomicBool,
}
impl Raised {
    fn new(height: f32) -> Arc<Self> {
        Arc::new(Self {
            height,
            drew: AtomicBool::new(false),
        })
    }
    fn drew(&self) -> bool {
        self.drew.load(Ordering::Relaxed)
    }
}
impl EntityRenderer for Raised {
    fn submit(
        &self,
        _: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        let drew = raised_quad(draw, self.height);
        self.drew.store(drew, Ordering::Relaxed);
        Ok(())
    }
}

/// Refuses its first call with an `Err`, then draws one stone quad 400 blocks above the entity;
/// `calls` counts the calls.
#[derive(Default)]
struct Flaky {
    pub calls: AtomicU32,
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

/// One stone quad a block wide, `height` blocks above the entity; whether the sprite was there.
fn raised_quad(draw: &mut dyn BlockEntityDrawContext, height: f32) -> bool {
    let (low, high) = (height, height + 1.0);
    let corners = [
        [0.0, high, 0.0],
        [0.0, low, 0.0],
        [1.0, low, 0.0],
        [1.0, high, 0.0],
    ];
    draw.quad("block/stone", corners, [0.0, 0.0, 16.0, 16.0], 0xFF_FFFF)
}

static TYPES: [ThrownType; 2] = [
    ThrownType {
        id: "ruby:bolt",
        item: "minecraft:snowball",
        damage: 0.0,
        slot: 0,
    },
    ThrownType {
        id: "ruby:dart",
        item: "minecraft:snowball",
        damage: 0.0,
        slot: 1,
    },
];

/// Resolves `ids`, each with a renderer of its own, against [`TYPES`] and the entity types
/// `minecraft:oak_boat`, `ruby:bolt` and `ruby:dart`.
fn resolve(ids: &[&str]) -> (EntityRendererTable, Vec<String>) {
    let registered: BTreeMap<String, RegisteredEntityRenderer> = ids
        .iter()
        .map(|id| {
            (
                (*id).to_owned(),
                RegisteredEntityRenderer::new(Raised::new(1.0), None),
            )
        })
        .collect();
    EntityRendererTable::resolve(
        &registered,
        &TYPES,
        &Default::default(),
        &sunrise_gameplay_common::game::general::GeneralTypeCatalog::freeze(&[]).unwrap(),
        |id| ["minecraft:oak_boat", "ruby:bolt", "ruby:dart"].contains(&id),
    )
}

#[test]
fn a_registered_thrown_type_resolves_to_its_slot_and_an_unregistered_one_to_none() {
    let (table, _) = resolve(&["ruby:bolt"]);
    assert!(table.thrown(&TYPES[0]).is_some());
    assert!(table.thrown(&TYPES[1]).is_none());
}

#[test]
fn a_registration_for_an_unknown_entity_type_is_refused_by_name() {
    let (table, unknown) = resolve(&["nope:ghost", "ruby:bolt"]);
    assert_eq!(unknown, ["nope:ghost"]);
    assert!(
        table.thrown(&TYPES[0]).is_some(),
        "the known one still resolves"
    );
}

#[test]
fn a_registration_for_a_vanilla_type_is_kept_silently() {
    let (_, unknown) = resolve(&["minecraft:oak_boat"]);
    assert_eq!(unknown, Vec::<String>::new());
}

/// The renderers the scene's mod registers: `ruby:bolt` twice (`first` 100 blocks up, then
/// `second` 200 up), `minecraft:oak_boat` (`boat`, 300 up) and `ruby:spike` (`spike`, failing its
/// first call). `ruby:dart` has none.
struct Scene {
    pub first: Arc<Raised>,
    second: Arc<Raised>,
    boat: Arc<Raised>,
    spike: Arc<Flaky>,
}

/// The camera, south of the entities and looking at them.
const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.0, -4.5);
/// Where the scene's entities stand.
const FEET: [f64; 3] = [0.5, 64.0, 0.5];

/// How high above the scene's entities a vertex is, in blocks.
fn above(v: &Vertex) -> f32 {
    v.pos[1] - (FEET[1] - CAMERA.y) as f32
}

fn feet() -> sunrise_core::geometry::Vec3 {
    sunrise_core::geometry::Vec3::new(FEET[0], FEET[1], FEET[2])
}

/// The vertices one frame of mobs and projectiles draws with these thrown types in flight.
fn thrown_frame(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    kinds: &[&str],
) -> Vec<Vertex> {
    use sunrise_sim::projectiles::{Projectile, ProjectileKind, thrown_type};
    game.creatures.projectiles.clear();
    for (i, kind) in kinds.iter().enumerate() {
        let t = thrown_type(kind).unwrap();
        let mut p = Projectile::new(ProjectileKind::Thrown(t), feet(), None);
        p.id = 90 + i as u32;
        game.creatures.projectiles.push(p);
    }
    let mut out = Vec::new();
    game.mob_geometry(CAMERA, 1.0, &mut out);
    out
}

/// Runs `check` on a client game with prepared resources, the thrown types `ruby:bolt`,
/// `ruby:dart` and `ruby:spike` and the [`Scene`]'s registrations, resolved as the first load does. In a child
/// process running only `test`: the thrown types and the registrations are kept once per process.
fn in_scene(
    test: &str,
    check: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, &Scene),
) {
    if std::env::var_os("SUNRISE_ENTITY_RENDERERS_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("cut_layout::game::native_entity_renderers_tests::{test}"),
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_ENTITY_RENDERERS_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    use mod_api::{Hooks, RegisterEntityRenderersEvent, RegistrationContext};
    assert!(sunrise_sim::projectiles::register_thrown_types([
        ("ruby:bolt".into(), "minecraft:snowball".into(), 0.0),
        ("ruby:dart".into(), "minecraft:snowball".into(), 0.0),
        ("ruby:spike".into(), "minecraft:snowball".into(), 0.0),
    ]));
    let scene = Scene {
        first: Raised::new(100.0),
        second: Raised::new(200.0),
        boat: Raised::new(300.0),
        spike: Arc::default(),
    };
    struct Renderers(Arc<Raised>, Arc<Raised>, Arc<Raised>, Arc<Flaky>);
    #[mod_api::__subscriptions]
    impl Hooks for Renderers {
        fn register_entity_renderers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterEntityRenderersEvent<'_>,
        ) {
            event.renderers.register("ruby:bolt".into(), self.0.clone());
            event.renderers.register("ruby:bolt".into(), self.1.clone());
            event
                .renderers
                .register("minecraft:oak_boat".into(), self.2.clone());
            event
                .renderers
                .register("ruby:spike".into(), self.3.clone());
        }
    }
    let renderers = Renderers(
        scene.first.clone(),
        scene.second.clone(),
        scene.boat.clone(),
        scene.spike.clone(),
    );
    let mods = vec![mod_api::ModEntry::new("ruby", Box::new(renderers))].into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_client_gameplay::game::client_side::mods_client::register_entity_renderers(
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        );
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            game.resolve_entity_renderers();
            check(game, &scene);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_registered_thrown_type_draws_the_later_renderer_and_no_item() {
    in_scene(
        "a_registered_thrown_type_draws_the_later_renderer_and_no_item",
        |game, scene| {
            let bolt = thrown_frame(game, &["ruby:bolt"]);
            assert_eq!(bolt.len(), 4, "one quad and no item: {bolt:?}");
            assert!(bolt.iter().all(|v| (200.0..=201.0).contains(&above(v))));
            assert!(scene.second.drew());
            assert!(!scene.first.drew(), "the replaced registration never draws");
        },
    );
}

#[test]
#[ignore = "daily"]
fn an_unregistered_thrown_type_still_draws_its_item() {
    in_scene(
        "an_unregistered_thrown_type_still_draws_its_item",
        |game, _| {
            let dart = thrown_frame(game, &["ruby:dart"]);
            assert!(!dart.is_empty(), "its snowball item");
            assert!(
                dart.iter().all(|v| above(v) < 2.0),
                "near the entity: {dart:?}"
            );
            let both = thrown_frame(game, &["ruby:bolt", "ruby:dart"]);
            assert_eq!(both.len(), 4 + dart.len());
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_vanilla_boat_is_unchanged_by_a_registration_for_its_type() {
    in_scene(
        "a_vanilla_boat_is_unchanged_by_a_registration_for_its_type",
        |game, scene| {
            game.boats
                .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                    100,
                    "oak".into(),
                    feet(),
                    0.0,
                ));
            let mut boat = Vec::new();
            game.boat_geometry(CAMERA, 1.0, &mut boat, 0..game.boats.len());
            assert!(!boat.is_empty(), "its vanilla model");
            assert!(
                boat.iter().all(|v| above(v) < 2.0),
                "near the boat: {boat:?}"
            );
            assert!(
                !scene.boat.drew(),
                "the boat route does not read the table yet"
            );
        },
    );
}

/// A renderer returning `Err` falls back to the item for that frame only and stays active for
/// later frames and after a resource reload.
#[test]
#[ignore = "daily"]
fn a_failed_renderer_falls_back_for_that_frame_and_recovers_on_the_next() {
    in_scene(
        "a_failed_renderer_falls_back_for_that_frame_and_recovers_on_the_next",
        |game, scene| {
            let _ = mod_api::__private::take_hook_panics();
            let spike = thrown_frame(game, &["ruby:spike"]);
            assert!(!spike.is_empty(), "the failing frame: its snowball item");
            assert!(
                spike.iter().all(|v| above(v) < 2.0),
                "the failing frame: no quad kept: {spike:?}"
            );
            assert_eq!(scene.spike.calls(), 1);
            assert!(
                mod_api::__private::take_hook_panics().is_empty(),
                "an Err return never queues a panic notice"
            );

            let spike = thrown_frame(game, &["ruby:spike"]);
            assert_eq!(scene.spike.calls(), 2, "asked again on the next frame");
            assert_eq!(spike.len(), 4, "one quad and no item: {spike:?}");
            assert!(spike.iter().all(|v| (400.0..=401.0).contains(&above(v))));

            sunrise_client_gameplay::game::client_side::model_baking_completed_tests::reload(game);
            let spike = thrown_frame(game, &["ruby:spike"]);
            assert_eq!(scene.spike.calls(), 3, "the reload resolved it again");
            assert_eq!(spike.len(), 4, "one quad and no item: {spike:?}");
            assert!(spike.iter().all(|v| (400.0..=401.0).contains(&above(v))));
        },
    );
}
