//! A client mod's extra models (`Chunk::extra_models`) drawn as part of their block: placed,
//! lit and culled as its own faces, each quad in its own pass, and nothing for a stale handle.
use super::mod_geometry_tests::{World, quads};
use super::*;

const GOLD: &str = "minecraft:block/gold_block";
const GLASS: &str = "minecraft:block/white_stained_glass";

/// The handle of the first baked model block `name`'s default state draws, in `world`'s store
/// of `generation`.
fn model_of(world: &World, name: &str, generation: u64) -> mod_api::ModelRef {
    let state = world.assets.registry.default_state(name).unwrap();
    let slot = world.render.model_catalog.state_slots(state).unwrap()[0];
    mod_api::__private::built::model_ref(generation, slot)
}

/// `chunk` with `models` drawn as part of the block at (2, MIN_Y + 4, 2), `world.cube`'s stone.
fn with_extra(mut chunk: Chunk, models: &[mod_api::ModelRef]) -> Chunk {
    let cell = chunk.dynamic_cell(2, MIN_Y + 4, 2).unwrap();
    chunk.extra_models.insert(cell, models.into());
    chunk
}

fn layer(world: &World, sprite: &str) -> u16 {
    world.render.sprite_layers[sprite] as u16
}

fn on_layer(vertices: &[TerrainVertex], layer: u16) -> Vec<TerrainVertex> {
    vertices
        .iter()
        .filter(|v| v.layer & !OPAQUE_LAYER == layer)
        .copied()
        .collect()
}

#[test]
#[ignore = "daily"]
fn extra_models_draw_with_the_blocks_place_and_light() {
    if !crate::test_process::isolated(concat!(
        "mesher::extra_models_tests::",
        stringify!(extra_models_draw_with_the_blocks_place_and_light)
    )) {
        return;
    }
    let world = World::load().expect("prepared Sunrise resources are required");
    let tables = world.tables(Vec::new());
    let mut mesher = world.mesher(&tables);
    let gold = model_of(&world, "minecraft:gold_block", world.render.generation);
    let plain = world.cube();
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&plain);
    let stone = mesher.mesh(&region, 0);
    assert_eq!(quads(&stone.solid), 6);
    let chunk = with_extra(world.cube(), &[gold]);
    region[1][1] = Some(&chunk);
    let mesh = mesher.mesh(&region, 0);
    assert_eq!(
        quads(&mesh.solid),
        12,
        "the stone's six faces and the gold's six"
    );
    let added = on_layer(&mesh.solid, layer(&world, GOLD));
    assert_eq!(quads(&added), 6);
    // A full cube over the stone's: each gold corner where a stone corner is, lit as it is.
    let own: Vec<_> = stone.solid.iter().map(|v| (v.pos, v.light)).collect();
    assert!(added.iter().all(|v| own.contains(&(v.pos, v.light))));
    // The same model twice is drawn twice.
    let chunk = with_extra(world.cube(), &[gold, gold]);
    region[1][1] = Some(&chunk);
    assert_eq!(quads(&mesher.mesh(&region, 0).solid), 18);

    // SectionCompiler.compile skips all air states, not only the ordinary air palette ID.
    let mut mixed = with_extra(world.cube(), &[gold]);
    for (x, name) in [
        (5, "minecraft:air"),
        (8, "minecraft:cave_air"),
        (11, "minecraft:void_air"),
    ] {
        let air = world.assets.registry.default_state(name).unwrap();
        mixed.set(x, MIN_Y + 4, 2, air.0);
        let cell = mixed.dynamic_cell(x, MIN_Y + 4, 2).unwrap();
        mixed.extra_models.insert(cell, Arc::from([gold]));
    }
    region[1][1] = Some(&mixed);
    let gold = on_layer(&mesher.mesh(&region, 0).solid, layer(&world, GOLD));
    assert_eq!(quads(&gold), 6);
    assert!(gold.iter().all(|v| (2.0..=3.0).contains(&v.pos[0])));
}

#[test]
#[ignore = "daily"]
fn extra_faces_are_culled_by_neighbours_and_keep_their_pass() {
    if !crate::test_process::isolated(concat!(
        "mesher::extra_models_tests::",
        stringify!(extra_faces_are_culled_by_neighbours_and_keep_their_pass)
    )) {
        return;
    }
    let world = World::load().expect("prepared Sunrise resources are required");
    let tables = world.tables(Vec::new());
    let mut mesher = world.mesher(&tables);
    let generation = world.render.generation;
    let gold = model_of(&world, "minecraft:gold_block", generation);
    let glass = model_of(&world, "minecraft:white_stained_glass", generation);
    let mut chunk = with_extra(world.cube(), &[gold, glass]);
    // Stone east of the block hides its east faces, the gold's and the glass's with them.
    let stone = world
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    chunk.set(3, MIN_Y + 4, 2, stone.0);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    let mesh = mesher.mesh(&region, 0);
    assert_eq!(
        quads(&on_layer(&mesh.solid, layer(&world, GOLD))),
        5,
        "no gold face toward the neighbour"
    );
    let glass_quads = on_layer(&mesh.translucent, layer(&world, GLASS));
    assert_eq!(quads(&glass_quads), 5, "the glass in the translucent pass");
    assert!(on_layer(&mesh.solid, layer(&world, GLASS)).is_empty());
    assert_eq!(quads(&mesh.solid), 5 + 5 + 5, "both stones and the gold");

    mesher.opaque_ice = true;
    let ice = world
        .assets
        .registry
        .default_state("minecraft:ice")
        .unwrap();
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(2, MIN_Y + 4, 2, ice.0);
    let chunk = with_extra(chunk, &[glass]);
    region[1][1] = Some(&chunk);
    let mesh = mesher.mesh(&region, 0);
    assert_eq!(
        quads(&on_layer(&mesh.solid, layer(&world, "minecraft:block/ice"))),
        6
    );
    assert_eq!(quads(&on_layer(&mesh.translucent, layer(&world, GLASS))), 6);
    assert!(on_layer(&mesh.solid, layer(&world, GLASS)).is_empty());
}

#[test]
#[ignore = "daily"]
fn a_stale_handle_draws_nothing() {
    if !crate::test_process::isolated(concat!(
        "mesher::extra_models_tests::",
        stringify!(a_stale_handle_draws_nothing)
    )) {
        return;
    }
    let world = World::load().expect("prepared Sunrise resources are required");
    let tables = world.tables(Vec::new());
    let mut mesher = world.mesher(&tables);
    let stale = model_of(&world, "minecraft:gold_block", world.render.generation + 1);
    let chunk = with_extra(world.cube(), &[stale]);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    let mesh = mesher.mesh(&region, 0);
    assert_eq!(quads(&mesh.solid), 6, "the stone alone");
    assert!(on_layer(&mesh.solid, layer(&world, GOLD)).is_empty());
}

fn bytes(vertices: &[TerrainVertex]) -> &[u8] {
    bytemuck::cast_slice(vertices)
}

/// A checkerboard section, every face of its 2,048 stones open but those toward the absent
/// neighbouring columns, with enough gold cubes on each stone to exceed the upload budget.
/// The mesh keeps every stone face, draws attachment faces in cell and list order, and is the
/// same on every build.
#[test]
#[ignore = "daily"]
fn extra_models_stay_within_the_sections_limits() {
    if !crate::test_process::isolated(concat!(
        "mesher::extra_models_tests::",
        stringify!(extra_models_stay_within_the_sections_limits)
    )) {
        return;
    }
    use super::extra_models::FACE_QUADS;
    use super::mod_geometry::{MAX_QUADS, MAX_SECTION_BYTES};

    let world = World::load().expect("prepared Sunrise resources are required");
    let tables = world.tables(Vec::new());
    let mut mesher = world.mesher(&tables);
    let stone = world
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    let gold = model_of(&world, "minecraft:gold_block", world.render.generation);
    let mut plain = Chunk::empty(0, 0);
    let mut cells = Vec::new();
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                if (x + y + z) % 2 == 0 {
                    plain.set(x, MIN_Y + y, z, stone.0);
                    cells.push([x, y, z]);
                }
            }
        }
    }
    let mut chunk = plain.clone();
    let copies = 64;
    let heavy: Arc<[_]> = vec![gold; copies].into();
    for &[x, y, z] in &cells {
        let cell = chunk.dynamic_cell(x, MIN_Y + y, z).unwrap();
        chunk.extra_models.insert(cell, heavy.clone());
    }
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&plain);
    let base = mesher.mesh(&region, 0);
    assert_eq!(quads(&base.solid), cells.len() * 6 - 4 * 128);
    region[1][1] = Some(&chunk);
    let mesh = mesher.mesh(&region, 0);

    let vertices = mesh.solid.len() + mesh.translucent.len();
    let used = vertices * size_of::<TerrainVertex>()
        + mesh.solid_tints.len() * size_of::<u32>()
        + (mesh.solid_sources.len() + mesh.translucent_sources.len()) * size_of::<TerrainSource>();
    assert!(vertices / 4 <= MAX_QUADS, "{} quads", vertices / 4);
    assert!(used <= MAX_SECTION_BYTES, "{used} bytes");
    assert!(
        vertices / 4 > MAX_QUADS - 2 * FACE_QUADS,
        "the extra models fill the section up to its limit: {} quads",
        vertices / 4
    );
    // Every stone face, unchanged and first.
    assert_eq!(
        bytes(&mesh.solid[..base.solid.len()]),
        bytes(&base.solid),
        "the block geometry is kept whole"
    );
    // The extra models after it, from the first cell on: its whole list, and none for the last.
    let extra = &mesh.solid[base.solid.len()..];
    let gold_layer = layer(&world, GOLD);
    assert!(extra.iter().all(|v| v.layer & !OPAQUE_LAYER == gold_layer));
    // A quad of a cell's cube: its four corners on it (a neighbour's meets it at an edge at most).
    let of = |quad: &[TerrainVertex], [x, y, z]: [i32; 3]| {
        let lo = [x as f32, y as f32, z as f32];
        quad.iter()
            .all(|v| (0..3).all(|k| (lo[k]..=lo[k] + 1.0).contains(&v.pos[k])))
    };
    let faces =
        |vertices: &[TerrainVertex], cell| vertices.chunks(4).filter(|quad| of(quad, cell)).count();
    let first = 4 * faces(&base.solid, cells[0]) * copies;
    assert!(first > 0);
    assert!(extra[..first].chunks(4).all(|quad| of(quad, cells[0])));
    assert_eq!(faces(extra, *cells.last().unwrap()), 0);
    // The same section meshes the same, quad for quad.
    let again = mesher.mesh(&region, 0);
    assert_eq!(bytes(&again.solid), bytes(&mesh.solid));
    assert_eq!(bytes(&again.translucent), bytes(&mesh.translucent));

    // Put the only tinted attachment past the actual cliff. Its tint source must not run
    // (including with the shader-pack source records and visibility charge in the budget).
    let &[x, y, z] = cells.last().unwrap();
    let key = chunk.dynamic_cell(x, MIN_Y + y, z).unwrap();
    let leaves = model_of(&world, "minecraft:oak_leaves", world.render.generation);
    chunk.extra_models.insert(key, Arc::from([leaves]));
    let source = Arc::new(Counted {
        asked: Default::default(),
        answer: Answer::Green,
    });
    let mut tables = tables;
    tables
        .apply_tint_layers(
            world.assets.registry,
            &std::collections::BTreeMap::from([(
                "minecraft:stone".to_string(),
                Arc::from([mod_api::TintLayer::Source(source.clone())]),
            )]),
            Some(super::mod_tint_tests::tint_values(world.assets.registry)),
        )
        .unwrap();
    let mut mesher = world.mesher(&tables);
    mesher.shader_pack = true;
    let mut preparation = AttachedTintScratch::default();
    let mut prepare = |chunk: &Chunk| {
        let mut region = [[None; 3]; 3];
        region[1][1] = Some(chunk);
        mesher.prepare_attached_tints(&region, 0, Some(512), None, &mut preparation)
    };
    assert!(prepare(&chunk).is_none());
    assert_eq!(source.asked.load(std::sync::atomic::Ordering::Relaxed), 0);

    // The exact same attachment does ask when earlier extra models no longer fill the budget.
    chunk.extra_models.retain(|&cell, _| cell == key);
    assert!(prepare(&chunk).is_some());
    assert_eq!(source.asked.load(std::sync::atomic::Ordering::Relaxed), 1);
}

/// A mod's own tint source: green, an ordinary panic, or a panic with a hostile payload.
#[derive(Clone, Copy)]
enum Answer {
    Green,
    Panic,
    DropPanic,
}

struct PanicsOnDrop;
impl Drop for PanicsOnDrop {
    fn drop(&mut self) {
        // A second payload can also have a panicking destructor.
        std::panic::panic_any(PanicsOnDrop);
    }
}

struct Counted {
    asked: std::sync::atomic::AtomicUsize,
    answer: Answer,
}
impl mod_api::BlockTintSource for Counted {
    fn color(&self, _: &mod_api::BlockState, _: &dyn mod_api::BlockMetadata) -> u32 {
        0x00_FF00
    }
    fn color_in_world(
        &self,
        _: &mod_api::BlockState,
        _: &dyn mod_api::BlockMetadata,
        _: &dyn mod_api::BlockAndTintGetter,
        _: [i32; 3],
    ) -> u32 {
        self.asked
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert_ne!(std::thread::current().name(), Some("sunrise-worker-test"));
        match self.answer {
            Answer::Green => 0x00_FF00,
            Answer::Panic => panic!("no neighbour data at the chunk edge"),
            Answer::DropPanic => std::panic::panic_any(PanicsOnDrop),
        }
    }
}

/// Oak leaves use the same tint index in their base mesh and in an attached copy. The
/// provider is asked once, before the worker runs, and failure colours both sets white.
#[test]
#[ignore = "daily"]
fn a_mod_tint_source_is_asked_before_the_mesh_not_by_the_worker() {
    if !crate::test_process::isolated(concat!(
        "mesher::extra_models_tests::",
        stringify!(a_mod_tint_source_is_asked_before_the_mesh_not_by_the_worker)
    )) {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    crate::crash::install(home.path().to_path_buf(), "client", || None);
    // Exercise the fatal-client branch of the production panic hook if containment is missing.
    std::thread::Builder::new()
        .name("main".into())
        .spawn(run_tint_cases)
        .unwrap()
        .join()
        .unwrap();
    assert!(
        !home.path().join("crash-reports").exists(),
        "a recoverable tint panic wrote a fatal report"
    );
    assert!(
        crate::crash::report_error(
            home.path(),
            "client",
            &crate::crash::DebugCrash::new(),
            || None
        )
        .is_some(),
        "a recoverable tint panic consumed the fatal report slot"
    );
}

fn run_tint_cases() {
    use std::sync::atomic::Ordering;

    let world = World::load().expect("prepared Sunrise resources are required");
    let registry = world.assets.registry;
    let values = super::mod_tint_tests::tint_values(registry);
    let leaves = model_of(&world, "minecraft:oak_leaves", world.render.generation);
    let mut chunk = world.cube();
    chunk.set(
        2,
        MIN_Y + 4,
        2,
        registry.default_state("minecraft:oak_leaves").unwrap().0,
    );
    let chunk = with_extra(chunk, &[leaves]);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    let leaf_layer = layer(&world, "minecraft:block/oak_leaves");
    let mut tables = world.tables(Vec::new());
    tables
        .apply_tint_layers(
            registry,
            &std::collections::BTreeMap::from([
                (
                    "test:missing_block".to_owned(),
                    Arc::from([mod_api::TintLayer::Source(Arc::new(Counted {
                        asked: Default::default(),
                        answer: Answer::Panic,
                    }))]),
                ),
                (
                    "minecraft:stone".to_owned(),
                    Arc::from([mod_api::TintLayer::Vanilla(mod_api::VanillaTint::Constant(
                        0x00_FF00,
                    ))]),
                ),
            ]),
            Some(ModTintValues {
                states: values.states.clone(),
                metadata: values.metadata,
            }),
        )
        .unwrap();
    assert!(
        !tables.has_custom_tint_sources(),
        "constant layers and absent-block registrations need no scheduler pass"
    );

    let mut preparation = AttachedTintScratch::default();
    for answer in [Answer::Green, Answer::Panic, Answer::DropPanic] {
        let source = Arc::new(Counted {
            asked: Default::default(),
            answer,
        });
        let unused = Arc::new(Counted {
            asked: Default::default(),
            answer: Answer::Panic,
        });
        let mut tables = world.tables(Vec::new());
        let layers = std::collections::BTreeMap::from([
            (
                "minecraft:oak_leaves".to_string(),
                Arc::from([
                    mod_api::TintLayer::Source(source.clone()),
                    mod_api::TintLayer::Source(unused.clone()),
                ]),
            ),
            (
                "minecraft:stone".to_string(),
                Arc::from([mod_api::TintLayer::Source(source.clone())]),
            ),
        ]);
        tables
            .apply_tint_layers(
                registry,
                &layers,
                Some(ModTintValues {
                    states: values.states.clone(),
                    metadata: values.metadata,
                }),
            )
            .unwrap();
        let mut mesher = world.mesher(&tables);
        if matches!(answer, Answer::Green) {
            let untinted_base = with_extra(world.cube(), &[leaves]);
            let mut unprepared = region;
            unprepared[1][1] = Some(&untinted_base);
            let mesh = mesh_on_worker(&world, &tables, &unprepared, None);
            assert_eq!(
                source.asked.load(Ordering::Relaxed),
                0,
                "no tint on the worker"
            );
            assert_top_color(&mesh, leaf_layer, 2, 4, 0xFF_FFFF);
        }
        let prepared = mesher
            .prepare_attached_tints(&region, 0, None, None, &mut preparation)
            .expect("the source colours the extra model");
        assert_eq!(
            source.asked.load(Ordering::Relaxed),
            1,
            "asked once while preparing"
        );
        let mesh = mesh_on_worker(&world, &tables, &region, Some(prepared));
        assert_eq!(
            source.asked.load(Ordering::Relaxed),
            1,
            "the base and attachment reused the answer"
        );
        assert_eq!(
            unused.asked.load(Ordering::Relaxed),
            0,
            "the unused tint layer was never asked"
        );
        let mut vertices = on_layer(&mesh.solid, leaf_layer);
        vertices.extend(on_layer(&mesh.translucent, leaf_layer));
        assert_eq!(quads(&vertices), 12, "six base and six attached leaf faces");
        // With AO off the upward quads are unshaded, so their RGB is literal green or white.
        let expected = if matches!(answer, Answer::Green) {
            0x00_FF00
        } else {
            0xFF_FFFF
        };
        let tops: Vec<_> = vertices
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|quad| quad.iter().all(|v| v.pos[1] == 5.0))
            .flatten()
            .collect();
        assert_eq!(
            tops.len(),
            8,
            "the base and attachment each have a top face"
        );
        assert!(tops.iter().all(|v| v.color & 0xFF_FFFF == expected));
    }
}

fn mesh_on_worker(
    world: &World,
    tables: &MeshTables,
    region: &[[Option<&Chunk>; 3]; 3],
    prepared: Option<Arc<AttachedTints>>,
) -> SectionMesh {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("sunrise-worker-test".into())
            .spawn_scoped(scope, move || {
                let mut scratch = MeshScratch {
                    attached_tints: prepared,
                    ..Default::default()
                };
                world
                    .mesher(tables)
                    .mesh_reusing(region, 0, SectionMesh::default(), &mut scratch)
            })
            .unwrap()
            .join()
            .expect("the mesh worker kept running")
    })
}

fn assert_top_color(mesh: &SectionMesh, layer: u16, x: i32, y: i32, rgb: u32) {
    let tops: Vec<_> = mesh
        .solid
        .as_chunks::<4>()
        .0
        .iter()
        .chain(mesh.translucent.as_chunks::<4>().0)
        .filter(|quad| {
            quad.iter().all(|v| {
                v.layer & !OPAQUE_LAYER == layer
                    && v.pos[1] == (y + 1) as f32
                    && (x as f32..=(x + 1) as f32).contains(&v.pos[0])
            })
        })
        .flatten()
        .collect();
    assert!(!tops.is_empty(), "layer {layer} at x={x} has a top face");
    assert!(tops.iter().all(|v| v.color & 0xFF_FFFF == rgb));
}

struct SharedTint {
    lock: Arc<std::sync::Mutex<()>>,
    asked: std::sync::atomic::AtomicUsize,
    fails: bool,
}

impl mod_api::BlockTintSource for SharedTint {
    fn color(&self, _: &mod_api::BlockState, _: &dyn mod_api::BlockMetadata) -> u32 {
        0x00_FF00
    }

    fn color_in_world(
        &self,
        _: &mod_api::BlockState,
        _: &dyn mod_api::BlockMetadata,
        _: &dyn mod_api::BlockAndTintGetter,
        _: [i32; 3],
    ) -> u32 {
        self.asked
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _lock = self.lock.lock().unwrap();
        assert!(
            !self.fails,
            "the overlay provider panics with the shared lock held"
        );
        0x00_FF00
    }
}

/// Only the gold attachment uses this CTM overlay. Its `tintBlock` supplies a different
/// provider than the leaves' own one. Both providers share the poisoned mutex, even in
/// sections with no preparation.
#[test]
#[ignore = "daily"]
fn an_attachment_overlay_panic_does_not_call_a_different_source_on_the_worker() {
    if !crate::test_process::isolated(concat!(
        "mesher::extra_models_tests::",
        stringify!(an_attachment_overlay_panic_does_not_call_a_different_source_on_the_worker)
    )) {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    crate::crash::install(home.path().to_path_buf(), "client", || None);
    std::thread::Builder::new()
        .name("main".into())
        .spawn(run_overlay_panic_case)
        .unwrap()
        .join()
        .unwrap();
    assert!(!home.path().join("crash-reports").exists());
}

fn run_overlay_panic_case() {
    use crate::resource_source::{Resource, ResourceRoot, ResourceSource};
    use std::collections::BTreeMap;
    use std::sync::atomic::Ordering;

    let mut world = World::load().expect("prepared Sunrise resources are required");
    let registry = world.assets.registry;
    crate::pack_formats::install();
    let pack = ResourceSource::directory(world.root.clone()).extended(BTreeMap::from([(
        "assets/minecraft/optifine/ctm/test/overlay.properties".to_owned(),
        Resource::bytes(
            concat!(
                "matchTiles=gold_block\n",
                "method=overlay_fixed\n",
                "tiles=textures/block/diamond_block.png\n",
                "tintIndex=0\n",
                "tintBlock=stone\n",
            )
            .as_bytes()
            .to_vec(),
        ),
    )]));
    let cache = tempfile::tempdir().unwrap();
    world.render = crate::assets::RenderAssets::load(
        &ResourceRoot::new(world.root.clone(), cache.path().to_path_buf(), pack),
        &world.assets,
    )
    .unwrap();
    let mut tables = world.tables(Vec::new());
    let lock = Arc::new(std::sync::Mutex::new(()));
    let leaf_source = Arc::new(SharedTint {
        lock: lock.clone(),
        asked: Default::default(),
        fails: false,
    });
    let overlay_source = Arc::new(SharedTint {
        lock: lock.clone(),
        asked: Default::default(),
        fails: true,
    });
    tables
        .apply_tint_layers(
            registry,
            &BTreeMap::from([
                (
                    "minecraft:oak_leaves".into(),
                    Arc::from([mod_api::TintLayer::Source(leaf_source.clone())]),
                ),
                (
                    "minecraft:stone".into(),
                    Arc::from([mod_api::TintLayer::Source(overlay_source.clone())]),
                ),
            ]),
            Some(super::mod_tint_tests::tint_values(registry)),
        )
        .unwrap();
    let leaves = model_of(&world, "minecraft:oak_leaves", world.render.generation);
    let gold = model_of(&world, "minecraft:gold_block", world.render.generation);
    let oak = registry.default_state("minecraft:oak_leaves").unwrap();
    let mut chunk = world.cube();
    chunk.set(2, MIN_Y + 4, 2, oak.0);
    chunk.set(7, MIN_Y + 4, 2, oak.0);
    let mut chunk = with_extra(chunk, &[leaves]);
    let late = chunk.dynamic_cell(7, MIN_Y + 4, 2).unwrap();
    chunk.extra_models.insert(late, Arc::from([gold]));
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    let prepared = world
        .mesher(&tables)
        .prepare_attached_tints(&region, 0, None, None, &mut AttachedTintScratch::default())
        .expect("the attachments have custom tints");
    assert!(
        lock.is_poisoned(),
        "the overlay used the different provider before meshing"
    );
    assert_eq!(leaf_source.asked.load(Ordering::Relaxed), 1);
    assert_eq!(overlay_source.asked.load(Ordering::Relaxed), 1);

    let mesh = mesh_on_worker(&world, &tables, &region, Some(prepared));
    assert_eq!(
        leaf_source.asked.load(Ordering::Relaxed),
        1,
        "no base retry on the worker"
    );
    assert_eq!(overlay_source.asked.load(Ordering::Relaxed), 1);
    let leaf = layer(&world, "minecraft:block/oak_leaves");
    assert_top_color(&mesh, leaf, 2, 4, 0x00_FF00);
    assert_top_color(&mesh, leaf, 7, 4, 0xFF_FFFF);
    assert_top_color(
        &mesh,
        layer(&world, "minecraft:block/diamond_block"),
        7,
        4,
        0xFF_FFFF,
    );

    // An ordinary section with no attachments gets no prepared answer. Its base provider still
    // reaches the shared poisoned mutex, but the failure yields white once for the whole block.
    let mut plain = Chunk::empty(1, 0);
    plain.set(7, MIN_Y + 4, 2, oak.0);
    let mut next_region = [[None; 3]; 3];
    next_region[1][1] = Some(&plain);
    let mesh = mesh_on_worker(&world, &tables, &next_region, None);
    assert_top_color(&mesh, leaf, 7, 4, 0xFF_FFFF);
    assert_eq!(leaf_source.asked.load(Ordering::Relaxed), 2);
    assert_eq!(overlay_source.asked.load(Ordering::Relaxed), 1);
}
