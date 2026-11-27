//! The mesh visibility design's acceptance cases that need the real mesher and prepared
//! resources (`SUNRISE_MESH_TEST_RESOURCES`), like the other mesher geometry tests.
use super::super::{BiomeZoom, LeafCullingMode, MeshScratch, MeshTables, Mesher};
use super::*;
use crate::{
    assets::{Assets, RenderAssets},
    biome::Biomes,
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y},
    models,
};

struct Fixture {
    assets: Assets,
    biomes: Biomes,
    render: RenderAssets,
    visibility: sunrise_world::block_render::FaceVisibility,
    tables: MeshTables,
    hidden_states: Vec<bool>,
}

fn fixture() -> Fixture {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_MESH_TEST_RESOURCES").expect("SUNRISE_MESH_TEST_RESOURCES"),
    );
    let source = sunrise_assets::resource_source::ResourceRoot::directory(root.clone());
    let assets = Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let biomes = Biomes::load(&source).unwrap();
    let render = crate::assets::RenderAssets::load(&source, &assets).unwrap();
    let visibility = models::load_visibility(&root, assets.registry).unwrap();
    let tables = MeshTables::new(&assets, &render, &facts, visibility.clone(), None).unwrap();
    Fixture {
        assets,
        biomes,
        render,
        visibility,
        tables,
        hidden_states: Vec::new(),
    }
}

fn state(f: &Fixture, name: &str) -> u32 {
    f.assets.registry.default_state(name).unwrap().0
}

const BASE: i32 = MIN_Y + 4;

fn mesh(f: &Fixture, chunk: &Chunk, plan: Option<&SectionVisibilityPlan>) -> SectionMesh {
    mesh_region(f, &[[Some(chunk); 3]; 3], plan)
}

fn mesh_region(
    f: &Fixture,
    region: &[[Option<&Chunk>; 3]; 3],
    plan: Option<&SectionVisibilityPlan>,
) -> SectionMesh {
    let mut mesher = Mesher {
        nether_shading: false,
        shader_pack_lighting: false,
        separate_ao: false,
        shader_pack: false,
        assets: &f.assets,
        render: &f.render,
        tables: &f.tables,
        biomes: &f.biomes,
        visibility: f.visibility.clone(),
        zoom: BiomeZoom::new(0),
        blend_radius: 0,
        smooth_lighting: true,
        cutout_leaves: true,
        opaque_ice: false,
        translucent_layers: &[],
        layer_overrides: &[],
        leaf_culling: LeafCullingMode::Off,
        hidden_states: &f.hidden_states,
        part_tables: Default::default(),
    };
    let mut scratch = MeshScratch::default();
    mesher.mesh_planned(region, 0, SectionMesh::default(), &mut scratch, plan, 0)
}

fn quads(mesh: &SectionMesh) -> usize {
    (mesh.solid.len() + mesh.translucent.len()) / 4
}

fn plan(edit: impl FnOnce(&mut SectionPlanBuilder)) -> SectionVisibilityPlan {
    let mut builder = SectionPlanBuilder::new();
    edit(&mut builder);
    plan_section(builder.finish().as_ref(), |_| None).unwrap()
}

fn stone(f: &Fixture, cells: &[[i32; 3]]) -> Chunk {
    let mut chunk = Chunk::empty(0, 0);
    for &[x, y, z] in cells {
        chunk.set(x, BASE + y, z, state(f, "minecraft:stone"));
    }
    chunk
}

#[test]
#[ignore = "daily"]
fn a_face_mask_removes_faces_and_hidden_cells_open_their_neighbours() {
    let f = fixture();
    let one = stone(&f, &[[8, 0, 8]]);
    assert_eq!(quads(&mesh(&f, &one, None)), 6);
    let mut keep_all = SectionPlanBuilder::new();
    keep_all.restrict([8, 4, 8], true, true, 63);
    let answer = keep_all.finish();
    assert!(answer.is_none(), "keep-63 changes nothing");
    assert_eq!(quads(&mesh(&f, &one, None)), 6);
    let no_up = plan(|b| b.restrict([8, 4, 8], true, true, 61));
    assert_eq!(quads(&mesh(&f, &one, Some(&no_up))), 5);
    let two = stone(&f, &[[8, 0, 8], [9, 0, 8]]);
    assert_eq!(quads(&mesh(&f, &two, None)), 10);
    let hidden = plan(|b| b.restrict([9, 4, 8], false, false, 0));
    assert_eq!(
        quads(&mesh(&f, &two, Some(&hidden))),
        6,
        "the hidden cube draws nothing; its neighbour's face toward it shows"
    );
    // A cube whose west face was removed: it loses that face (already culled), and its
    // neighbour's east face toward it shows, as it no longer occludes there.
    let open_west = plan(|b| b.restrict([9, 4, 8], true, true, ALL_FACES_BUT_WEST));
    assert_eq!(quads(&mesh(&f, &two, Some(&open_west))), 11);
}

/// Every face kept but the west one (bit 4).
const ALL_FACES_BUT_WEST: u8 = 0b10_1111;

#[test]
#[ignore = "daily"]
fn hide_terrain_composes_with_state_visibility_and_section_geometry() {
    use mod_api::{SectionGeometry, SectionGeometryContext, SectionQuad};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct Geometry(AtomicUsize);
    impl SectionGeometry for Geometry {
        fn add(&self, _: [i32; 3], context: &mut dyn SectionGeometryContext) {
            self.0.fetch_add(1, Ordering::Relaxed);
            assert!(context.add(
                "minecraft:block/gold_block",
                SectionQuad {
                    positions: [[2., 8., 2.], [2., 8., 3.], [3., 8., 3.], [3., 8., 2.]],
                    uvs: [[0., 0.], [0., 1.], [1., 1.], [1., 0.]],
                    color: 0xFFFF_FFFF,
                    light: [15, 0],
                    translucent: false,
                }
            ));
        }
    }
    let mut f = fixture();
    let geometry = Arc::new(Geometry::default());
    f.tables.section_geometry =
        Arc::new([super::super::mod_geometry::Renderer::new(geometry.clone())]);
    let mut chunk = stone(&f, &[[1, 0, 1]]);
    let chest = state(&f, "minecraft:chest");
    let barrel = state(&f, "minecraft:barrel");
    let glowstone = state(&f, "minecraft:glowstone");
    let torch = state(&f, "minecraft:torch");
    chunk.set(3, BASE, 3, state(&f, "minecraft:water"));
    chunk.set(5, BASE, 5, chest);
    chunk.set(6, BASE, 6, barrel);
    chunk.set(7, BASE, 7, glowstone);
    chunk.set(9, BASE, 9, torch);
    f.hidden_states = vec![false; f.assets.registry.len()];
    f.hidden_states[barrel as usize] = true;
    f.hidden_states[glowstone as usize] = true;

    let full = mesh(&f, &chunk, None);
    assert!(quads(&full) > 6);
    assert!(
        full.solid
            .as_chunks::<4>()
            .0
            .iter()
            .any(|quad| quad[0].pos == [2., 8., 2.])
    );
    let hidden_cell = plan(|b| b.restrict([5, 4, 5], false, false, 0));
    let cell_only = mesh(&f, &chunk, Some(&hidden_cell));
    assert_eq!(cell_only.block_entities, full.block_entities);
    let hidden = mesh(&f, &chunk, Some(&plan(SectionPlanBuilder::hide_terrain)));
    assert_eq!(quads(&hidden), 0);
    assert_eq!(
        geometry.0.load(Ordering::Relaxed),
        2,
        "no geometry callback for HideTerrain"
    );
    assert_eq!(hidden.block_entities, full.block_entities);
    assert_eq!(hidden.block_entities, [([5, BASE, 5], chest)]);
    assert_eq!(hidden.lights, full.lights);
    assert_eq!(hidden.lights, [([9, BASE, 9], torch)]);
    assert_eq!(hidden.visibility, ALL_VISIBLE);
}

#[test]
#[ignore = "daily"]
fn fully_hidden_neighbours_do_not_change_fluid_surface_geometry() {
    use super::super::TerrainVertex;

    let f = fixture();
    let water = state(&f, "minecraft:water");
    let rock = state(&f, "minecraft:stone");
    for neighbour in [water, rock] {
        for across_border in [false, true] {
            let x = if across_border { 15 } else { 7 };
            let mut centre = Chunk::empty(0, 0);
            let mut east = Chunk::empty(1, 0);
            centre.set(x, BASE, 8, water);
            if across_border {
                east.set(0, BASE, 8, neighbour);
            } else {
                centre.set(x + 1, BASE, 8, neighbour);
            }
            let mut region = [[None; 3]; 3];
            region[1][1] = Some(&centre);
            region[1][2] = Some(&east);
            let mut builder = SectionPlanBuilder::new();
            builder.restrict(
                [if across_border { 0 } else { x as u8 + 1 }, 4, 8],
                false,
                false,
                0,
            );
            let answer = builder.finish();
            let masked = if across_border {
                plan_section(None, |d| {
                    (d == [1, 0, 0]).then_some(answer.as_ref()).flatten()
                })
            } else {
                plan_section(answer.as_ref(), |_| None)
            };
            let before = mesh_region(&f, &region, None);
            let after = mesh_region(&f, &region, masked.as_ref());
            let surface = |mesh: &SectionMesh| {
                mesh.translucent
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|quad| {
                        quad.iter().all(|v| v.pos[1] > 4.25)
                            && quad.iter().any(|v| v.pos[0] == x as f32)
                            && quad.iter().any(|v| v.pos[0] == x as f32 + 1.0)
                    })
                    .flatten()
                    .copied()
                    .collect::<Vec<TerrainVertex>>()
            };
            let before_surface = surface(&before);
            assert!(!before_surface.is_empty());
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&surface(&after)),
                bytemuck::cast_slice::<_, u8>(&before_surface)
            );
            let east_face = |mesh: &SectionMesh| {
                mesh.translucent.as_chunks::<4>().0.iter().any(|quad| {
                    let at = quad[0].pos[0];
                    at > x as f32 + 0.5
                        && at < x as f32 + 1.0
                        && quad.iter().all(|v| v.pos[0] == at)
                })
            };
            assert!(!east_face(&before));
            assert!(
                east_face(&after),
                "visible water exposes its face beside a hidden neighbour"
            );
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_masked_opening_connects_both_sides_of_an_opaque_wall() {
    let f = fixture();
    let mut chunk = Chunk::empty(0, 0);
    for y in 0..16 {
        for z in 0..16 {
            chunk.set(8, MIN_Y + y, z, state(&f, "minecraft:stone"));
        }
    }
    // VisGraph bits are from * 6 + to, with west = 4 and east = 5.
    let west_to_east = 1 << (4 * 6 + 5);
    assert_eq!(mesh(&f, &chunk, None).visibility & west_to_east, 0);
    let open = plan(|b| b.restrict([8, 8, 8], false, false, 0));
    assert_eq!(
        mesh(&f, &chunk, Some(&open)).visibility & west_to_east,
        west_to_east
    );
}

#[test]
#[ignore = "daily"]
fn masked_translucent_faces_sort_far_to_near() {
    use crate::gfx::translucency::{Sorter, quad_center};
    let f = fixture();
    let mut chunk = Chunk::empty(0, 0);
    let ice = state(&f, "minecraft:ice");
    chunk.set(2, BASE, 2, ice);
    chunk.set(5, BASE, 2, ice);
    let tops = plan(|b| {
        b.restrict([2, 4, 2], true, true, 1 << 1);
        b.restrict([5, 4, 2], true, true, 1 << 1);
    });
    let masked = mesh(&f, &chunk, Some(&tops));
    assert!(masked.solid.is_empty());
    let centres: Vec<_> = masked
        .translucent
        .as_chunks::<4>()
        .0
        .iter()
        .map(|q| quad_center(q[0].pos, q[2].pos))
        .collect();
    assert_eq!(centres.len(), 2);
    let mut sorter = Sorter::default();
    let indices = sorter.sort(&centres, [0.5, 5.0, 2.5]);
    assert_eq!(
        [
            centres[indices[0] as usize / 4][0],
            centres[indices[6] as usize / 4][0]
        ],
        [5.5, 2.5]
    );
}

/// The no-mod path is the mesher of before: no plan meshes byte for byte as `mesh_reusing`, and
/// a plan that keeps every cell (which the world thread never sends; it sends none) meshes the
/// same bytes through the masked path.
#[test]
#[ignore = "daily"]
fn without_a_plan_meshes_are_byte_identical() {
    let f = fixture();
    let mut chunk = Chunk::empty(0, 0);
    let names = [
        "minecraft:stone",
        "minecraft:glass",
        "minecraft:water",
        "minecraft:oak_leaves",
        "minecraft:chest",
        "minecraft:torch",
        "minecraft:oak_slab",
        "minecraft:grass_block",
    ];
    for x in 0..16 {
        for z in 0..16 {
            for y in 0..6 {
                let pick = (x * 7 + z * 3 + y * 5) as usize % (names.len() + 2);
                if let Some(name) = names.get(pick) {
                    chunk.set(x, BASE + y, z, state(&f, name));
                }
            }
        }
    }
    let region = [[Some(&chunk); 3]; 3];
    let mut mesher = Mesher {
        nether_shading: false,
        shader_pack_lighting: false,
        separate_ao: false,
        shader_pack: false,
        assets: &f.assets,
        render: &f.render,
        tables: &f.tables,
        biomes: &f.biomes,
        visibility: f.visibility.clone(),
        zoom: BiomeZoom::new(0),
        blend_radius: 0,
        smooth_lighting: true,
        cutout_leaves: true,
        opaque_ice: false,
        translucent_layers: &[],
        layer_overrides: &[],
        leaf_culling: LeafCullingMode::Off,
        hidden_states: &[],
        part_tables: Default::default(),
    };
    let mut scratch = MeshScratch::default();
    let before = mesher.mesh_reusing(&region, 0, SectionMesh::default(), &mut scratch);
    let none = mesher.mesh_planned(&region, 0, SectionMesh::default(), &mut scratch, None, 0);
    let keep_all = SectionVisibilityPlan {
        hide_terrain: false,
        cells: vec![ALL; P * P * P].into_boxed_slice(),
    };
    let kept = mesher.mesh_planned(
        &region,
        0,
        SectionMesh::default(),
        &mut scratch,
        Some(&keep_all),
        0,
    );
    for other in [&none, &kept] {
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&before.solid),
            bytemuck::cast_slice::<_, u8>(&other.solid)
        );
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&before.translucent),
            bytemuck::cast_slice::<_, u8>(&other.translucent)
        );
        assert_eq!(before.solid_sources, other.solid_sources);
        assert_eq!(before.visibility, other.visibility);
        assert_eq!(before.block_entities, other.block_entities);
        assert_eq!(before.lights, other.lights);
        assert_eq!(before.sprite_layers, other.sprite_layers);
    }
    assert!(quads(&before) > 100);
}
