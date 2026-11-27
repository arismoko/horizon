//! Mod block entities drawn on the client: `native_block_entity_submit`
//! (vanilla's `BlockEntityRenderer.submit`) gets a [`BlockEntityDraw`], whose
//! pose stack (`PoseStack`) places what the helpers here add to the entity
//! batch, as vanilla's block entity renderers draw.
pub use mod_api::game::CubeFace;

use super::Dim;
use glam::{DVec3, Mat4, Quat, Vec3};
#[cfg(test)]
use mod_api::prelude::*;
use mod_api::{Identifier, ModelError, ModelPart, ModelRef};
use sunrise_assets::{
    bake::{bake_flat_face, default_uv},
    item_model::DisplayContext,
    model::{Direction, Material},
};
use sunrise_client::{
    block_entity_render::{diffuse, diffuse_lights, vertex_light},
    gfx::Cull,
    hud::Rect,
    mesher::Vertex,
    mobs::{ModelNormals, model_layers},
};
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::{banners::layer_textures, tables::GameItems as _};
use sunrise_items::items::ItemStack;
use sunrise_level::{
    containers::{BlockEntity, ModRevision},
    fast_hash::FxHashMap,
};
use sunrise_world::registry::BlockStateId;

// The native block-entity draw helper submits the outline and fill as one material.
const TEXT_OVER_OUTLINE: f32 = 0.05;

/// Atlas-resolved standalone geometry, indexed by model slot and resource generation.
pub(in crate::game) struct StandaloneGeometry {
    generation: u64,
    slots: Vec<Option<Vec<crate::game::client_side::minecarts_client::PreparedModelFace>>>,
    translucent: Vec<bool>,
    bounds: Vec<[[f32; 3]; 2]>,
    /// What each fluid draws as in a `fluid_box`, by fluid slot, resolved on its first box of
    /// this generation (the table is rebuilt with the resources, so a reload resolves anew).
    fluid_looks: std::cell::RefCell<Vec<Option<Option<super::native_fluid_render::FluidBoxLook>>>>,
    /// The first slot past the baked store's: block state `n` is slot `states_from + n`.
    states_from: u32,
    /// Every block state's model, bound when the generation is prepared so a draw only reads.
    states: Box<[StateModel]>,
}

/// A block state's faces as vanilla draws it outside the terrain: default tints in the colour.
pub(in crate::game) struct StateModel {
    faces: Vec<sunrise_client::models::catalog::BakedFace>,
    prepared: Vec<crate::game::client_side::minecarts_client::PreparedModelFace>,
    tints: FxHashMap<i32, u32>,
    /// The state is also drawn by its block's special renderer, with default data
    /// (`BuiltInBlockModels`' `CompositeBlockModel`: the block-state model, then the special one).
    special: bool,
    bounds: [[f32; 3]; 2],
}

/// Whether a resolved slot can take instances, and the local box used to cull copies.
pub(in crate::game) struct InstanceSlot {
    pub translucent: bool,
    /// A block state's model: drawn on the CPU route only, never as a GPU instance.
    pub state: bool,
    /// The block state whose special renderer draws after the model's faces.
    pub special: Option<BlockStateId>,
    pub bounds: [[f32; 3]; 2],
}

fn point_bounds(points: impl IntoIterator<Item = [f32; 3]>) -> Option<[[f32; 3]; 2]> {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for pos in points {
        for axis in 0..3 {
            min[axis] = min[axis].min(pos[axis]);
            max[axis] = max[axis].max(pos[axis]);
        }
    }
    min[0].is_finite().then_some([min, max])
}

fn face_bounds(positions: impl IntoIterator<Item = [f32; 3]>) -> [[f32; 3]; 2] {
    point_bounds(positions).unwrap_or_default()
}

impl StandaloneGeometry {
    /// The standalone models and every block state's model of `pack`.
    ///
    /// With `block_entity_render`, a state whose block has a special renderer also records it
    /// (`StateModel::special`) and widens its bounds by that part's default geometry. With `None`
    /// no state records a special part and its bounds hold the block-state faces alone: only
    /// for a caller that reads `prepared_upload`, which holds the standalone slots and no state.
    /// A `Game` always passes its renderer, joined before the first world frame.
    pub(in crate::game) fn new(
        pack: &super::PackResources,
        client_pack: &crate::game::client_side::resource_tables_client::ClientPackResources,
        gui: &super::UiResources,
        block_entity_render: Option<&sunrise_client::block_entity_render::BlockEntityRenderer>,
    ) -> Self {
        let catalog = &client_pack.render.model_catalog;
        let layer_sprites: FxHashMap<_, _> = client_pack
            .layer_names
            .iter()
            .filter_map(|(&layer, name)| Some((layer, *gui.atlas.sprites.get(name)?)))
            .collect();
        let mut slots = Vec::new();
        let mut translucent = Vec::new();
        let mut bounds = Vec::new();
        for slot in catalog.standalone_slots() {
            let index = slot as usize;
            if slots.len() <= index {
                slots.resize_with(index + 1, || None);
                translucent.resize(index + 1, false);
                bounds.resize(index + 1, [[0.0; 3], [0.0; 3]]);
            }

            let model = catalog.model_by_slot(slot).expect("standalone slot exists");
            let prepared: Vec<_> = model
                .faces
                .iter()
                .filter_map(|face| {
                    let sprite = layer_sprites.get(&face.layer)?;
                    Some(
                        crate::game::client_side::minecarts_client::PreparedModelFace::new(
                            &face.quad, *sprite,
                        ),
                    )
                })
                .collect();
            translucent[index] = model.faces.iter().any(|face| face.translucent);
            bounds[index] = face_bounds(prepared.iter().flat_map(
                crate::game::client_side::minecarts_client::PreparedModelFace::positions,
            ));
            slots[index] = Some(prepared);
        }
        // The special parts are sized in an empty world, with no level loaded yet.
        let special_renderer = block_entity_render.map(|renderer| {
            (
                renderer,
                sunrise_level::world::World::new(BlockStateId(0), 0),
            )
        });
        let mut special_scratch = Vec::new();
        let states = (0..pack.assets.registry.len() as u32)
            .map(|state| {
                let id = BlockStateId(state);
                // `BuiltInBlockModels.addDefaults`: only run the special renderer for blocks
                // that have one, widening the state's bounds with its default geometry.
                let special = special_renderer.as_ref().and_then(|(renderer, world)| {
                    if !pack
                        .assets
                        .registry
                        .state(id)?
                        .name
                        .strip_prefix("minecraft:")
                        .is_some_and(super::text_displays::has_special_model)
                    {
                        return None;
                    }
                    special_scratch.clear();
                    crate::game::client_side::minecarts_client::with_default_scene(
                        gui,
                        world,
                        false,
                        |scene| {
                            renderer.render_state(
                                scene,
                                id.0,
                                Mat4::IDENTITY,
                                0,
                                &mut special_scratch,
                            );
                        },
                    );
                    point_bounds(special_scratch.iter().map(|vertex| vertex.pos))
                });
                Self::bind_state(client_pack, &layer_sprites, id, special)
            })
            .collect();
        Self {
            generation: client_pack.render.generation,
            slots,
            translucent,
            bounds,
            fluid_looks: Default::default(),
            states_from: catalog.model_count(),
            states,
        }
    }

    /// Block state `id` as one model: the blockstate file's parts at `BLOCK_MODEL_SEED` with
    /// each tinted face holding the block's default tint (no position) in its colour, bound to
    /// its atlas regions, plus `special`'s bounds when the state also draws with a special
    /// renderer.
    fn bind_state(
        client_pack: &crate::game::client_side::resource_tables_client::ClientPackResources,
        layer_sprites: &FxHashMap<u32, Rect>,
        id: BlockStateId,
        special: Option<[[f32; 3]; 2]>,
    ) -> StateModel {
        use sunrise_client::models::catalog::BakedFace;
        let mut faces = Vec::new();
        let mut prepared = Vec::new();
        let mut tints = FxHashMap::default();
        let grass = client_pack.item_renderer.grass_color(0.5, 1.0) as u32;
        let default_tint = |index| {
            client_pack
                .context
                .mesh
                .tables
                .default_tint(id, index, grass)
        };
        if let Ok(models) = client_pack.render.model_catalog.collect_seeded(
            id,
            sunrise_gameplay_common::game::CommonGame::BLOCK_MODEL_SEED,
        ) {
            for face in models.iter().flat_map(|(_, model)| &model.faces) {
                let mut quad = face.quad.clone();
                if let Some(tint) = default_tint(quad.tint_index) {
                    tints.insert(quad.tint_index, tint);
                    let base = quad.color.map_or(u32::MAX, |v| v as u32);
                    quad.color =
                        Some(sunrise_client::hud_layout::multiply(base, 0xFF00_0000 | tint) as i32);
                }
                if let Some(&sprite) = layer_sprites.get(&face.layer) {
                    prepared.push(
                        crate::game::client_side::minecarts_client::PreparedModelFace::new(
                            &quad, sprite,
                        ),
                    );
                }
                faces.push(BakedFace::new(
                    quad,
                    face.layer,
                    face.opaque,
                    face.translucent,
                    None,
                ));
            }
        }
        let corners = special.into_iter().flatten();
        let bounds = face_bounds(
            faces
                .iter()
                .flat_map(|face| face.quad.positions)
                .chain(corners),
        );
        StateModel {
            faces,
            prepared,
            tints,
            special: special.is_some(),
            bounds,
        }
    }

    pub(in crate::game::client_side) fn instance_slot(&self, slot: u32) -> Option<InstanceSlot> {
        if let Some(state) = slot.checked_sub(self.states_from) {
            let model = self.states.get(state as usize)?;
            return Some(InstanceSlot {
                translucent: false,
                state: true,
                special: model.special.then_some(BlockStateId(state)),
                bounds: model.bounds,
            });
        }
        let index = slot as usize;
        self.slots.get(index)?.is_some().then_some(InstanceSlot {
            translucent: self.translucent[index],
            state: false,
            special: None,
            bounds: self.bounds[index],
        })
    }

    /// A state's prepared seed-42 faces and atlas bindings.
    fn state_model(&self, state: BlockStateId) -> Option<&StateModel> {
        self.states.get(state.0 as usize)
    }

    /// The baked faces of a block state's slot.
    pub(in crate::game::client_side) fn state_faces(
        &self,
        slot: u32,
    ) -> Option<&[sunrise_client::models::catalog::BakedFace]> {
        let state = slot.checked_sub(self.states_from)?;
        Some(&self.states.get(state as usize)?.faces)
    }

    pub(in crate::game::client_side) fn faces(
        &self,
        slot: u32,
    ) -> Option<&[crate::game::client_side::minecarts_client::PreparedModelFace]> {
        self.slots.get(slot as usize)?.as_deref()
    }

    /// The `fluid_box` look of fluid `slot`, from `resolve` the first time.
    pub(in crate::game::client_side) fn fluid_look(
        &self,
        slot: u32,
        resolve: impl FnOnce() -> Option<super::native_fluid_render::FluidBoxLook>,
    ) -> Option<super::native_fluid_render::FluidBoxLook> {
        let mut looks = self.fluid_looks.borrow_mut();
        let at = slot as usize;
        if looks.len() <= at {
            looks.resize(at + 1, None);
        }
        *looks[at].get_or_insert_with(resolve)
    }

    pub(in crate::game::client_side) fn generation(&self) -> u64 {
        self.generation
    }

    /// Vertices and per-slot index ranges for the generation's GPU buffer.
    pub(in crate::game::client_side) fn prepared_upload(
        &self,
    ) -> (
        Vec<sunrise_client::gfx::PreparedModelVertex>,
        Vec<Option<(u32, u32)>>,
    ) {
        let mut vertices = Vec::new();
        let mut ranges = Vec::new();
        for slot in &self.slots {
            let Some(faces) = slot else {
                ranges.push(None);
                continue;
            };
            let index_start = vertices.len() as u32 / 4 * 6;
            for face in faces {
                for corner in 0..4 {
                    vertices.push(face.gpu_vertex(corner));
                }
            }
            let index_count = vertices.len() as u32 / 4 * 6 - index_start;
            ranges.push((index_count > 0).then_some((index_start, index_count)));
        }
        (vertices, ranges)
    }
}

/// `BlockEntityRenderer.getViewDistance`'s default, in blocks.
const VIEW_DISTANCE: f64 = 64.0;

/// Each mod block entity's view distance (`BlockEntityRenderer.getViewDistance`), by block
/// state, as the parts answered `native_block_entity_view_distance` once for each mod block;
/// states with none keep [`VIEW_DISTANCE`]. `furthest` is how far block entities are gathered.
pub(in crate::game) struct ViewDistances {
    by_state: FxHashMap<u32, f64>,
    furthest: f64,
}

impl ViewDistances {
    /// The distances of the block states `states` (id, block): `ask` is asked once for each
    /// mod block, never for vanilla's, and an answer that isn't finite and above 0 keeps
    /// [`VIEW_DISTANCE`].
    fn of_states<'a>(
        states: impl Iterator<Item = (u32, &'a str)>,
        mut ask: impl FnMut(&str) -> f64,
    ) -> Self {
        let mut by_block: FxHashMap<&str, f64> = FxHashMap::default();
        let mut by_state = FxHashMap::default();
        for (state, block) in states {
            if !mod_api::__private::is_content_block(block) {
                continue;
            }
            let distance = *by_block.entry(block).or_insert_with(|| {
                let distance = ask(block);
                if distance.is_finite() && distance > 0.0 {
                    distance
                } else {
                    VIEW_DISTANCE
                }
            });
            if distance != VIEW_DISTANCE {
                by_state.insert(state, distance);
            }
        }
        let furthest = by_state.values().copied().fold(VIEW_DISTANCE, f64::max);
        Self { by_state, furthest }
    }
    /// How far the block entity of `state` draws.
    fn of(&self, state: u32) -> f64 {
        self.by_state.get(&state).copied().unwrap_or(VIEW_DISTANCE)
    }
}
/// A mod block entity's bounds until its mod says otherwise: the block with
/// the block of margin Sunrise culls vanilla's block entities with.
const DEFAULT_BOUNDS: [f32; 6] = [-1.0, -1.0, -1.0, 2.0, 2.0, 2.0];

/// Whether `bounds` (min then max, in blocks from `origin`, the block's corner
/// relative to the camera) meet the frustum; an infinite bound always does
/// (NeoForge's `INFINITE_EXTENT_AABB`), as does a frame without a frustum.
fn in_frustum(frustum: Option<&impl Cull>, origin: Vec3, bounds: [f32; 6]) -> bool {
    if bounds.iter().any(|b| !b.is_finite()) {
        return true;
    }
    let min = origin + Vec3::new(bounds[0], bounds[1], bounds[2]);
    let max = origin + Vec3::new(bounds[3], bounds[4], bounds[5]);
    frustum.is_none_or(|f| f.intersects(min, max))
}

/// Whether a mod block entity with `bounds` is visible in the eye pass: global renderers
/// (infinite bounds or `shouldRenderOffScreen`) bypass the eye-section filter, while ordinary
/// renderers follow the same eye-visible section check as built-in block entities.
fn mod_block_entity_visible(
    frustum: Option<&impl Cull>,
    origin: Vec3,
    bounds: [f32; 6],
    eye_visible: bool,
    off_screen: bool,
) -> bool {
    let is_global = off_screen || bounds.iter().any(|b| !b.is_finite());
    if !is_global && !eye_visible {
        return false;
    }
    in_frustum(frustum, origin, bounds)
}

/// Default camera for unit tests constructing `BlockEntityDraw::posed` without a frame view.
static DEFAULT_DRAW_CAMERA: mod_api::Camera = mod_api::Camera {
    position: [0.0; 3],
    eye_height: 0.0,
    partial_tick: 0.0,
    yaw: 0.0,
    pitch: 0.0,
    roll: 0.0,
    fov_degrees: 70.0,
    near: 0.05,
    far: 128.0,
    view: [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ],
    projection: [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ],
    dimension: String::new(),
};

/// One mod block entity's frame, as `submit` gets it: the render state's
/// partial tick, game time and light, and the pose stack the helpers draw
/// under (`native_draw_quad`, `native_draw_cube`, `native_draw_block`).
// Native mods read and drive it; nothing in Sunrise does.
pub struct BlockEntityDraw<'v> {
    /// Read-only frame state for the world view this draw belongs to.
    view: mod_api::WorldRenderView<'v>,
    /// The light the helpers draw with: the block's own
    /// (`LevelRenderer.getLightCoords`), 16 times sky light in bits 0..8 and
    /// 16 times block light in bits 8..16; set it to light a part otherwise.
    light: u32,
    /// The block drawn, for `native_draw_item`'s owner.
    block: [i32; 3],
    pose: Mat4,
    /// First-person arm normals follow the listener's pose operations separately.
    normal_pose: Option<ModelNormals>,
    /// World entity draws track the local translation separately from their camera-relative
    /// render pose. The affine pose's basis is the same in both spaces.
    local_light_translation: Option<DVec3>,
    parent_poses: Vec<(Mat4, Option<ModelNormals>, Option<DVec3>)>,
    lights: (Vec3, Vec3),
    pub(super) vertices: Vec<Vertex>,
    /// Earlier frame vertices are not owned by this draw and must keep their offsets.
    first_vertex: usize,
    kept: bool,
    /// Where each turning part starts in `vertices` (`spin`), and where the
    /// parts standing still start again (`still`, none).
    spins: Vec<(usize, Option<Spin>)>,
    /// `level_light` is on: elements drawn now are lit by the level.
    pub(super) level_lit: bool,
    /// `unlit` is on: elements drawn now skip face shading and draw full bright.
    pub(super) unlit: bool,
    /// `crumbling` is on: elements drawn now are covered by the breaking crack.
    crumbling: bool,
    /// The breaking stage at the block drawn, when it is cracked (`crumbling_stage`).
    crumbling_stage: Option<u8>,
    /// The vertex ranges drawn under `crumbling`, beside `spins`.
    crumbled: Vec<(usize, usize)>,
    /// Each element drawn under `level_light` (vertex range and local centre),
    /// beside `spins`.
    lit: Vec<LitElement>,
    /// World block entities record a prepared copy instead of CPU vertices.
    /// Shader packs, GUI scenes and mobs leave this clear, so the model stays
    /// in the vertex stream those paths and their spans already cover.
    prepared: bool,
    records: Vec<PreparedRecord>,
    /// Device-derived vertex and index capacity for the combined entity batch.
    quad_capacity: sunrise_client::gfx::EntityQuadCapacity,
    /// Complete quads already reserved by other draw sources in the same combined batch.
    reserved_quads: usize,
    /// Inside a mod's submit that `block_entity` dispatched: another `block_entity` is refused.
    nested: bool,
}

/// One element (`block`, `model`, `terrain_model`, `cube` or `quad`) drawn under
/// `level_light`: its vertices and posed centre in draw coordinates, before any `spin`.
struct LitElement {
    start: usize,
    end: usize,
    centre: DVec3,
    /// Fixed ambient fallback. The vertex light stores only the intrinsic floor until
    /// placement, so turning cannot ratchet it.
    ambient: u32,
}

impl LitElement {
    fn apply_light(vertices: &mut [Vertex], ambient: u32) {
        let sky = ambient & 0xff;
        let block = (ambient >> 8) & 0xff;
        for v in vertices {
            v.light = (ambient & 0xffff_0000)
                | sky.max(v.light & 0xff)
                | (block.max((v.light >> 8) & 0xff) << 8);
        }
    }
}

/// Where the draw's coordinate origin is in the level and how to look up its packed light.
pub(in crate::game) struct LevelLight<'a> {
    pub(in crate::game) origin: DVec3,
    pub(in crate::game) light_at: &'a dyn Fn([i32; 3]) -> u8,
}

impl LevelLight<'_> {
    fn sample(&self, centre: DVec3) -> Option<u32> {
        let absolute = (self.origin + centre).floor();
        if !absolute.cmpge(DVec3::splat(f64::from(i32::MIN))).all()
            || !absolute.cmple(DVec3::splat(f64::from(i32::MAX))).all()
        {
            return None;
        }
        Some(vertex_light((self.light_at)(
            absolute.as_ivec3().to_array(),
        )))
    }
}

/// Walks the ordered light records alongside the spin segments. Kept stationary
/// vertices store their first sample; kept turning vertices store their intrinsic floors.
struct PlacementLight<'a> {
    elements: &'a [LitElement],
    level: &'a LevelLight<'a>,
    pass: LightPass<'a>,
}

enum LightPass<'a> {
    Fresh(Option<&'a mut [Vertex]>),
    Retained,
}

impl PlacementLight<'_> {
    fn apply(&mut self, vertices: &mut [Vertex], start: usize, turn: Option<Mat4>) {
        let count = self
            .elements
            .partition_point(|e| e.start < start + vertices.len());
        let (elements, rest) = self.elements.split_at(count);
        self.elements = rest;
        if turn.is_none() && matches!(self.pass, LightPass::Retained) {
            return;
        }
        let light_turn = turn.map(|m| m.as_dmat4());
        for e in elements {
            let lo = e.start - start;
            let hi = (e.end - start).min(vertices.len());
            if lo >= hi {
                continue;
            }
            let centre = light_turn.map_or(e.centre, |m| m.transform_point3(e.centre));
            let ambient = self.level.sample(centre).unwrap_or(e.ambient);
            let span = &mut vertices[lo..hi];
            LitElement::apply_light(span, ambient);
            if turn.is_none()
                && let LightPass::Fresh(Some(kept)) = &mut self.pass
            {
                let kept_end = e.end.min(kept.len());
                if e.start < kept_end {
                    for (kept, drawn) in kept[e.start..kept_end].iter_mut().zip(span) {
                        kept.light = drawn.light;
                    }
                }
            }
        }
    }
}

struct PreparedRecord {
    at: usize,
    slot: u32,
    generation: u64,
    pose: Mat4,
    tint: u32,
    light: u32,
    /// Drawn under `crumbling`: its baked vertices join `crumbled`.
    crumbling: bool,
}

/// Instance tint in the same bytes the CPU vertex colour uses.
pub(in crate::game::client_side) fn prepared_tint(rgb: u32) -> [f32; 4] {
    sunrise_client::hud_layout::multiply(
        u32::MAX,
        sunrise_client::mobs::render::abgr(rgb & 0xFF_FFFF),
    )
    .to_le_bytes()
    .map(|byte| byte as f32 / 255.0)
}

/// An Iris pack (`pack_ids`) or one of Sunrise's own packs owns the CPU entity stream.
pub(in crate::game::client_side) fn prepared_route(
    iris_pack: bool,
    native_pack: bool,
    uploaded: Option<u64>,
    generation: u64,
) -> bool {
    !iris_pack && !native_pack && uploaded == Some(generation)
}

pub(in crate::game::client_side) fn prepared_instance(
    pose: Mat4,
    origin: DVec3,
    camera: DVec3,
    tint: u32,
    light: u32,
) -> sunrise_client::gfx::PreparedModelInstance {
    sunrise_client::gfx::PreparedModelInstance::new(
        pose,
        origin,
        camera,
        prepared_tint(tint),
        light,
    )
}

/// A part of a block entity's frame that turns by itself (`BlockEntityDraw::spin`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::game) struct Spin {
    axis: Vec3,
    pivot: Vec3,
    degrees_per_tick: f32,
    phase: f32,
}

impl Spin {
    /// The turn at animation time `time` (ticks with the partial tick), about
    /// the pivot, in the block's coordinates. Counted in f64: a long session
    /// has more ticks than an f32 holds exactly.
    fn turn(&self, time: f64) -> Mat4 {
        let t = time;
        let degrees =
            (t * f64::from(self.degrees_per_tick) + f64::from(self.phase)).rem_euclid(360.0);
        Mat4::from_translation(self.pivot)
            * Mat4::from_quat(Quat::from_axis_angle(
                self.axis,
                (degrees as f32).to_radians(),
            ))
            * Mat4::from_translation(-self.pivot)
    }
}

/// Places one draw in place: turns each spin segment and its light samples with the same
/// matrix, then makes the vertices camera-relative.
fn place_geometry(
    vertices: &mut [Vertex],
    spins: &[(usize, Option<Spin>)],
    time: f64,
    origin: Vec3,
    mut lighting: Option<PlacementLight<'_>>,
) {
    let mut start = 0;
    let mut spin: Option<Spin> = None;
    for (end, next_spin) in spins.iter().copied().chain([(vertices.len(), None)]) {
        if start >= vertices.len() {
            break;
        }
        let end = end.min(vertices.len());
        let span = &mut vertices[start..end];
        if !span.is_empty() {
            let turn = spin.map(|s| s.turn(time));
            if let Some(light) = &mut lighting {
                light.apply(span, start, turn);
            }
            for v in span {
                if let Some(m) = turn {
                    v.pos = m.transform_point3(Vec3::from_array(v.pos)).to_array();
                    if v.normal[3] != 0 {
                        let n = Vec3::new(
                            f32::from(v.normal[0]),
                            f32::from(v.normal[1]),
                            f32::from(v.normal[2]),
                        );
                        let n = m.transform_vector3(n);
                        v.normal = [
                            n.x.round().clamp(-127.0, 127.0) as i8,
                            n.y.round().clamp(-127.0, 127.0) as i8,
                            n.z.round().clamp(-127.0, 127.0) as i8,
                            v.normal[3],
                        ];
                    }
                }
                v.pos = (Vec3::from_array(v.pos) + origin).to_array();
            }
        }
        start = end;
        spin = next_spin;
    }
}

/// Adds to `overlay` the placed vertices of one block's `ranges` (offsets into `placed`,
/// camera-relative, turned by any `spin`) projected onto the crack stage whose sprite starts at
/// `sprite`: what `submitCrumblingOverlay` draws again over the parts a renderer chose.
fn crack_overlay(
    placed: &[Vertex],
    ranges: &[(usize, usize)],
    origin: Vec3,
    sprite: [f32; 2],
    overlay: &mut Vec<Vertex>,
) {
    for &(from, to) in ranges {
        let to = to.min(placed.len());
        if from >= to {
            continue;
        }
        let at = overlay.len();
        overlay.extend_from_slice(&placed[from..to]);
        sunrise_client::breaking::entity_geometry(&mut overlay[at..], origin, sprite);
    }
}

#[allow(dead_code)]
impl<'v> BlockEntityDraw<'v> {
    /// A frame drawn from `pose` (relative to the camera), borrowing `view`, lit by `light` and
    /// `lights`, adding to `vertices`; `block` is where `native_draw_item`'s owner stands.
    pub(in crate::game) fn posed_with_view(
        view: mod_api::WorldRenderView<'v>,
        light: u32,
        block: [i32; 3],
        pose: Mat4,
        lights: (Vec3, Vec3),
        vertices: Vec<Vertex>,
    ) -> Self {
        Self {
            view,
            light,
            block,
            pose,
            normal_pose: None,
            local_light_translation: None,
            parent_poses: Vec::new(),
            lights,
            first_vertex: vertices.len(),
            vertices,
            kept: false,
            spins: Vec::new(),
            level_lit: false,
            unlit: false,
            crumbling: false,
            crumbling_stage: None,
            crumbled: Vec::new(),
            lit: Vec::new(),
            prepared: false,
            records: Vec::new(),
            quad_capacity: sunrise_client::gfx::EntityQuadCapacity::default_device(),
            reserved_quads: 0,
            nested: false,
        }
    }

    /// Borrow the active world render view for this draw.
    pub(crate) fn view(&self) -> &mod_api::WorldRenderView<'_> {
        &self.view
    }

    /// Sets the renderer-owned combined entity quad capacity and quads already reserved by other
    /// draw sources in the same batch.
    pub(in crate::game) fn with_quad_budget(
        mut self,
        quad_capacity: sunrise_client::gfx::EntityQuadCapacity,
        reserved_quads: usize,
    ) -> Self {
        self.quad_capacity = quad_capacity;
        self.reserved_quads = reserved_quads;
        self
    }

    /// Opens the renderer-owned scoped quad submission sink for this draw.
    pub(in crate::game) fn quad_scope(&mut self) -> sunrise_client::gfx::EntityQuadScope<'_> {
        sunrise_client::gfx::EntityQuadScope::new(
            &mut self.vertices,
            self.quad_capacity,
            self.reserved_quads,
        )
    }

    /// Number of vertices currently recorded in this draw.
    pub(in crate::game) fn vertices_len(&self) -> usize {
        self.vertices.len()
    }

    /// This frame on the client's animation clock (`Game::animation_time`),
    /// which turns its `spin` parts and `animation_time` reads.
    pub(in crate::game) fn animated(mut self, time: f64) -> Self {
        self.view.animation_time = time;
        self
    }
    fn spinning(&self) -> bool {
        self.spins.last().is_some_and(|(_, spin)| spin.is_some())
    }

    /// Record a prepared copy at the current vertex, unless this draw is CPU-only or spinning,
    /// or the cracked block's overlay needs the model's vertices.
    fn defer_model(&mut self, slot: u32, generation: u64, tint: u32) -> bool {
        self.defer_model_at(slot, generation, self.pose(), tint, self.light)
    }

    /// Record a prepared copy with an explicit pose and light, unless this draw is CPU-only,
    /// spinning, level-lit or unlit.
    pub(in crate::game) fn defer_model_at(
        &mut self,
        slot: u32,
        generation: u64,
        pose: Mat4,
        tint: u32,
        light: u32,
    ) -> bool {
        if !self.prepared
            || self.spinning()
            || self.level_lit
            || self.unlit
            || (self.crumbling && self.crumbling_stage.is_some())
        {
            return false;
        }
        self.records.push(PreparedRecord {
            at: self.vertices.len(),
            slot,
            generation,
            pose,
            tint,
            light,
            crumbling: self.crumbling,
        });
        true
    }

    fn bake_recorded(
        &mut self,
        slots: &[Option<Vec<crate::game::client_side::minecarts_client::PreparedModelFace>>],
    ) {
        while let Some(record) = self.records.pop() {
            let Some(faces) = slots.get(record.slot as usize).and_then(Option::as_deref) else {
                continue;
            };
            let mut extra = Vec::new();
            crate::game::client_side::minecarts_client::emit_model_faces(
                faces.iter().copied().map(|face| (face, record.light)),
                record.pose,
                true,
                |index| (index >= 0).then_some(record.tint & 0xFF_FFFF),
                &mut extra,
            );
            crate::game::client_side::minecarts_client::scale_tint_alpha(
                &mut extra,
                faces,
                record.tint,
            );
            let count = extra.len();
            for (at, _) in &mut self.spins {
                if *at >= record.at {
                    *at += count;
                }
            }
            for e in &mut self.lit {
                if e.start >= record.at {
                    e.start += count;
                    e.end += count;
                }
            }
            let mut shifted = Vec::with_capacity(self.crumbled.len() + 1);
            for &(start, end) in &self.crumbled {
                if end <= record.at {
                    shifted.push((start, end));
                } else if start >= record.at {
                    shifted.push((start + count, end + count));
                } else {
                    shifted.push((start, record.at));
                    shifted.push((record.at + count, end + count));
                }
            }
            self.crumbled = shifted;
            if record.crumbling && count > 0 {
                self.crumbled.push((record.at, record.at + count));
            }
            self.vertices.splice(record.at..record.at, extra);
        }
    }

    fn push_instances(
        &mut self,
        origin: DVec3,
        camera: DVec3,
        out: &mut Vec<sunrise_client::gfx::PreparedModelDraw>,
    ) {
        out.extend(
            self.records
                .drain(..)
                .map(|record| sunrise_client::gfx::PreparedModelDraw {
                    cpu_vertices: record.at as u32,
                    generation: record.generation,
                    slot: record.slot,
                    instance: {
                        let mut instance = prepared_instance(
                            record.pose,
                            origin,
                            camera,
                            record.tint,
                            record.light,
                        );
                        instance.tint[3] = (record.tint >> 24) as f32 / 255.0;
                        instance
                    },
                }),
        );
    }
    /// The vertices drawn in a level-less frame, with those it was made with first.
    pub fn into_vertices(self) -> Vec<Vertex> {
        self.into_vertices_with_light(None)
    }

    /// Finish a posed frame drawn at rest, sampling the level when supplied.
    pub(in crate::game) fn into_vertices_with_light(
        mut self,
        level: Option<&LevelLight<'_>>,
    ) -> Vec<Vertex> {
        self.finish_light(level);
        self.vertices
    }

    /// Place the vertices of a level-less scene, including any turning parts.
    pub(in crate::game::client_side) fn into_placed(mut self, origin: Vec3) -> Vec<Vertex> {
        self.finish_light(None);
        self.spins.retain(|(at, _)| *at <= self.vertices.len());
        place_geometry(
            &mut self.vertices,
            &self.spins,
            self.view.animation_time,
            origin,
            None,
        );
        self.vertices
    }

    fn finish_light(&mut self, level: Option<&LevelLight<'_>>) {
        let first = self.first_vertex;
        self.quad_scope().truncate_to_budget_since(first);
        for e in &self.lit {
            let end = e.end.min(self.vertices.len());
            if e.start >= end {
                break;
            }
            let ambient = level.and_then(|l| l.sample(e.centre)).unwrap_or(e.ambient);
            LitElement::apply_light(&mut self.vertices[e.start..end], ambient);
        }
    }
    /// Track the pose from the entity's world origin, not its starting render translation.
    pub(in crate::game) fn track_entity_local_light_pose(&mut self) {
        self.local_light_translation = Some(DVec3::ZERO);
    }

    /// Track normals from the rigid first-person view transform through listener draws.
    pub(in crate::game) fn track_normal_pose(mut self) -> Self {
        self.normal_pose = Some(ModelNormals::from_rotation(self.pose));
        self
    }

    /// `PoseStack.pushPose`: `pop` comes back to the pose as it is now.
    fn push(&mut self) {
        self.parent_poses
            .push((self.pose, self.normal_pose, self.local_light_translation));
    }
    /// `PoseStack.popPose`; the block's own pose is never popped.
    fn pop(&mut self) {
        if let Some((pose, normals, light_translation)) = self.parent_poses.pop() {
            self.pose = pose;
            self.normal_pose = normals;
            self.local_light_translation = light_translation;
        }
    }
    /// `Err` inside a submit that [`Self::one_level`] runs: a carried block entity is drawn one
    /// level deep, so input a mod controls cannot recurse without end.
    fn refuse_nested(&self) -> Result<(), mod_api::BlockEntityDrawRefusal> {
        if self.nested {
            Err(mod_api::BlockEntityDrawRefusal::Nested)
        } else {
            Ok(())
        }
    }
    /// Runs `submit` as a carried block entity's: under its own pose level, with
    /// [`Self::refuse_nested`] refusing what it draws that way.
    fn one_level(&mut self, submit: impl FnOnce(&mut Self)) {
        self.nested = true;
        self.push();
        submit(self);
        self.pop();
        self.nested = false;
    }
    /// Moves what is drawn next by `by`, in blocks.
    fn translate(&mut self, by: [f32; 3]) {
        self.apply(Mat4::from_translation(Vec3::from_array(by)));
    }
    /// Turns what is drawn next by `degrees` about `axis` (counterclockwise
    /// seen from the axis' positive end, as `Axis.rotationDegrees`) through
    /// `pivot`, in blocks from the pose's origin: `[0.5; 3]` is the block's centre.
    fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, pivot: [f32; 3]) {
        let axis = Vec3::from_array(axis).normalize_or(Vec3::Y);
        let pivot = Vec3::from_array(pivot);
        let rotation = Mat4::from_quat(Quat::from_axis_angle(axis, degrees.to_radians()));
        self.apply(Mat4::from_translation(pivot) * rotation * Mat4::from_translation(-pivot));
        if let Some(normal) = &mut self.normal_pose {
            normal.rotate(glam::Mat3::from_mat4(rotation));
        }
    }
    /// Scales what is drawn next by `by` about the pose's origin.
    fn scale(&mut self, by: [f32; 3]) {
        let scale = Vec3::from_array(by);
        self.apply(Mat4::from_scale(scale));
        if let Some(normal) = &mut self.normal_pose {
            normal.scale(scale);
        }
    }
    /// Keeps what this call drew: the block is drawn the same again, with
    /// neither `native_block_entity_submit` nor
    /// `native_block_entity_render_bounding_box` called, until its state,
    /// its block entity (what the server sent), its light or the resource
    /// packs change. Call it only when what you drew reads none of
    /// `partial`, `game_time`, the blocks around it or anything else of
    /// yours, and moves nothing by itself but through `spin`: an idle machine,
    /// or a shaft turning at a steady speed, not one whose speed changes from
    /// frame to frame. Kept geometry stands still: an enchanted item's glint, and an
    /// item drawn with `native_draw_item` whose model reads the time (a clock,
    /// a compass), stop where they were.
    fn keep(&mut self) {
        self.kept = true;
    }
    /// What is drawn next turns by itself: `degrees_per_tick` degrees each
    /// tick of this client's animation clock (`animation`; a shaft at `rpm`
    /// turns `rpm * 0.3`, as Create's do), about `axis` through `pivot`, both
    /// in blocks from the block's corner whatever the pose, counterclockwise
    /// seen from the axis' positive end. The angle is `animation *
    /// degrees_per_tick + phase`. The clock is the client's own
    /// (`Game::animation_time`), not the game time, which a server's time
    /// updates set back: so a part turns smoothly. Draw the part at rest; the
    /// game turns it each frame. With `keep` it goes on turning with no call
    /// of `native_block_entity_submit` until what it was drawn with changes.
    /// With a new speed, pass `old_angle - animation * new_speed` as the
    /// phase, or it jumps. The shading of a quad drawn while it is active follows
    /// the turn (its normal turns); its light is fixed, and follows the turn
    /// only under `level_light`. Until `still` or the next `spin`. A block entity's
    /// frame only: a mob's (`native_render_living_pre`) draws it at rest. API
    /// @API:spinning_geometry@.
    pub(in crate::game::client_side) fn spin(
        &mut self,
        axis: [f32; 3],
        pivot: [f32; 3],
        degrees_per_tick: f32,
        phase: f32,
    ) {
        let spin = Spin {
            axis: Vec3::from_array(axis).normalize_or(Vec3::Y),
            pivot: Vec3::from_array(pivot),
            degrees_per_tick: if degrees_per_tick.is_finite() {
                degrees_per_tick
            } else {
                0.0
            },
            phase: if phase.is_finite() { phase } else { 0.0 },
        };
        self.spins.push((self.vertices.len(), Some(spin)));
    }
    /// What is drawn next stands still again, after `spin`.
    /// API @API:spinning_geometry@.
    pub(in crate::game::client_side) fn still(&mut self) {
        self.spins.push((self.vertices.len(), None));
    }
    fn set_light(&mut self, light: u32) {
        self.light = light;
        self.level_lit = false;
    }

    /// What is drawn next is lit by the level (`native_draw_level_light`).
    fn level_light(&mut self) {
        self.level_lit = true;
    }
    /// What is drawn next skips directional shading and draws full bright (`native_draw_unlit`).
    pub(in crate::game::client_side) fn unlit(&mut self, on: bool) {
        self.unlit = on;
    }
    /// What is drawn next by `quad`, `cube`, `block`, `model` and `terrain_model` gets the
    /// breaking crack while `on` (`native_draw_crumbling`).
    fn crumbling(&mut self, on: bool) {
        self.crumbling = on;
    }
    /// Marks the vertices from `start` as drawn under `crumbling`, joining the range before
    /// them when they follow it.
    pub(in crate::game) fn crumbling_since(&mut self, start: usize) {
        let end = self.vertices.len();
        if !self.crumbling || end <= start {
            return;
        }
        match self.crumbled.last_mut() {
            Some((_, last_end)) if *last_end == start => *last_end = end,
            _ => self.crumbled.push((start, end)),
        }
    }
    /// The vertices from `start` of an element `unlit` covers: under `unlit` they lose their
    /// shading normal and draw full bright (their colours carry no shade), else `lit_since`.
    /// `terrain_model` keeps its own shading and calls `lit_since` directly.
    pub(in crate::game) fn drawn_since(
        &mut self,
        start: usize,
        centre: impl FnOnce() -> DVec3,
        ambient: u32,
    ) {
        if !self.unlit {
            return self.lit_since(start, centre, ambient);
        }
        for v in &mut self.vertices[start..] {
            v.normal = [0; 4];
            v.light = sunrise_client::mobs::render::FULL_BRIGHT;
        }
    }
    /// Records one element and its fixed fallback only if there are vertices to relight.
    pub(in crate::game::client_side) fn lit_since(
        &mut self,
        start: usize,
        centre: impl FnOnce() -> DVec3,
        ambient: u32,
    ) {
        if self.level_lit && self.vertices.len() > start {
            let pose = self.pose.as_dmat4();
            let translation = self
                .local_light_translation
                .unwrap_or(pose.w_axis.truncate());
            self.lit.push(LitElement {
                start,
                end: self.vertices.len(),
                centre: pose.transform_vector3(centre()) + translation,
                ambient,
            });
        }
    }

    /// Under `level_light`, the emitter writes only the face's intrinsic light floors.
    pub(in crate::game::client_side) fn element_ambient(&self) -> u32 {
        if self.level_lit { 0 } else { self.light }
    }
    /// The centre of the unit cell a model is drawn in, before posing.
    pub(in crate::game::client_side) fn cell_centre() -> DVec3 {
        DVec3::splat(0.5)
    }
    fn apply(&mut self, m: Mat4) {
        if let Some(translation) = &mut self.local_light_translation {
            *translation += self
                .pose
                .as_dmat4()
                .transform_vector3(m.w_axis.truncate().as_dvec3());
        }
        self.pose *= m;
    }
    pub(in crate::game) fn pose(&self) -> Mat4 {
        self.pose
    }
    pub(in crate::game) fn normal_pose(&self) -> Option<ModelNormals> {
        self.normal_pose
    }
    pub(in crate::game) fn vertices_mut(&mut self) -> &mut Vec<Vertex> {
        &mut self.vertices
    }
    /// The light the helpers draw with (`native_draw_light`).
    pub(in crate::game) fn light(&self) -> u32 {
        self.light
    }
    fn quad(&mut self, corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], sprite: Rect, tint: u32) {
        let start = self.vertices.len();
        self.emit_quad(corners, uv, sprite, tint, self.element_ambient());
        self.drawn_since(
            start,
            || {
                corners
                    .into_iter()
                    .map(|c| DVec3::from_array(c.map(f64::from)) / 4.0)
                    .sum()
            },
            self.light,
        );
        self.crumbling_since(start);
    }

    /// A quad of `corners` showing a dynamic texture: `quad`'s shading and light, UVs in the
    /// texture's own unit square, and the texture named in the vertices (`overlay` is the
    /// render type, `handle` the texture) for the renderer to bind when it draws. False, with
    /// nothing appended, for a first-person hand draw, a coordinate that is not finite, or a
    /// quad the batch has no room for.
    pub(in crate::game) fn textured_quad(
        &mut self,
        corners: [[f32; 3]; 4],
        uv: [[f32; 2]; 4],
        overlay: u32,
        handle: u64,
        tint: u32,
    ) -> bool {
        // The hand's vertices are drawn from the shared atlas by a pass of their own.
        if self.normal_pose.is_some()
            || !corners
                .iter()
                .flatten()
                .chain(uv.iter().flatten())
                .all(|c| c.is_finite())
        {
            return false;
        }
        let start = self.vertices.len();
        let whole = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1,
            h: 1,
        };
        self.emit_quad(corners, uv, whole, tint, self.element_ambient());
        if self.vertices.len() != start + 4 {
            return false;
        }
        let glint_uv = sunrise_client::gfx::dynamic_texture_bits(handle);
        for v in &mut self.vertices[start..] {
            v.layer = overlay;
            v.glint_uv = glint_uv;
        }
        self.drawn_since(
            start,
            || {
                corners
                    .into_iter()
                    .map(|c| DVec3::from_array(c.map(f64::from)) / 4.0)
                    .sum()
            },
            self.light,
        );
        true
    }

    /// A quad of `corners` (blocks, under the pose) with unit UVs, shaded as
    /// vanilla's block entity models are (`Lighting`'s two lights).
    fn emit_quad(
        &mut self,
        corners: [[f32; 3]; 4],
        uv: [[f32; 2]; 4],
        sprite: Rect,
        tint: u32,
        light: u32,
    ) {
        let pose = self.pose();
        let at = corners.map(|c| pose.transform_point3(Vec3::from_array(c)));
        let normal = (at[1] - at[0]).cross(at[2] - at[0]).normalize_or_zero();
        let normal = if normal == Vec3::ZERO {
            (at[2] - at[1]).cross(at[3] - at[1]).normalize_or(Vec3::Y)
        } else {
            normal
        };
        // A turning part is shaded where it turns to (`place_geometry` turns the
        // normal, the shader shades it, as `draw_block`'s faces are); a part
        // standing still keeps its shading in its colour. Its light is fixed
        // unless `level_light` is on.
        let (color, normal) = if self.unlit {
            (shaded(tint, 1.0), [0; 4])
        } else if self.spinning() {
            let n = (normal * 127.0).round();
            (shaded(tint, 1.0), [n.x as i8, n.y as i8, n.z as i8, 127])
        } else {
            (shaded(tint, diffuse(normal, self.lights)), [0; 4])
        };
        let light = if self.unlit {
            sunrise_client::mobs::render::FULL_BRIGHT
        } else {
            light
        };
        let quad = std::array::from_fn(|i| Vertex {
            pos: at[i].to_array(),
            uv: [
                sprite.u0 + (sprite.u1 - sprite.u0) * uv[i][0],
                sprite.v0 + (sprite.v1 - sprite.v0) * uv[i][1],
            ],
            layer: 0,
            color,
            light,
            normal,
            glint_uv: [0.0, 0.0],
        });
        let _ = self.quad_scope().submit_quad(quad);
    }

    fn model(
        &mut self,
        faces: &[crate::game::client_side::minecarts_client::PreparedModelFace],
        tint: u32,
    ) {
        let ambient = self.element_ambient();
        let start = self.vertices.len();
        let room = self.quad_scope().remaining_quads();
        crate::game::client_side::minecarts_client::emit_model_faces(
            faces.iter().take(room).copied().map(|face| (face, ambient)),
            self.pose(),
            true,
            |index| (index >= 0).then_some(tint & 0xFF_FFFF),
            &mut self.vertices,
        );
        crate::game::client_side::minecarts_client::scale_tint_alpha(
            &mut self.vertices[start..],
            &faces[..faces.len().min(room)],
            tint,
        );
        self.quad_scope().retain_valid_since(start);
        self.drawn_since(start, Self::cell_centre, self.light & 0xffff);
        self.crumbling_since(start);
    }
}

#[allow(dead_code)]
impl BlockEntityDraw<'static> {
    /// A frame drawn from `pose` with a default static view, for unit tests that only exercise
    /// local geometry helpers without a frame camera.
    pub(in crate::game) fn posed(
        partial: f32,
        game_time: i64,
        light: u32,
        block: [i32; 3],
        pose: Mat4,
        lights: (Vec3, Vec3),
        vertices: Vec<Vertex>,
    ) -> Self {
        Self::posed_with_view(
            mod_api::WorldRenderView::new(
                &DEFAULT_DRAW_CAMERA,
                None,
                partial,
                game_time,
                0.0,
                false,
                &[],
            ),
            light,
            block,
            pose,
            lights,
            vertices,
        )
    }
}

/// 0xRRGGBB `tint` times `brightness`, as an opaque vertex colour.
fn shaded(tint: u32, brightness: f32) -> u32 {
    let c = |shift: u32| (((tint >> shift) & 255) as f32 * brightness) as u32;
    0xFF00_0000 | c(0) << 16 | c(8) << 8 | c(16)
}

/// Game ticks kept geometry stays after it was last drawn, as signs' text.
const KEPT_FOR: i64 = 100;

/// What kept geometry was drawn with besides its block entity: drawn again
/// when any of it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeptKey {
    dim: Dim,
    state: u32,
    light: u32,
    /// `Font::epoch` when it was drawn: a cache reset moves glyphs in the
    /// atlas, so text a mod kept (`native_draw_text`) is drawn again, as a
    /// sign's kept text is.
    glyphs: u64,
}

struct KeptEntry {
    key: KeptKey,
    /// Of the block entity it was drawn from: equal, nothing changed it.
    revision: ModRevision,
    bounds: [f32; 6],
    /// Game time it was last drawn.
    used: i64,
    /// Relative to the block's corner.
    vertices: Vec<Vertex>,
    /// Its turning parts (`BlockEntityDraw::spin`), from the start of `vertices`.
    spins: Vec<(usize, Option<Spin>)>,
    /// Its elements lit by the level (`BlockEntityDraw::level_light`), from the
    /// start of `vertices`.
    lit: Vec<LitElement>,
    /// The vertex ranges drawn under `BlockEntityDraw::crumbling`, from the start of `vertices`.
    crumbled: Vec<(usize, usize)>,
}

/// Geometry mods kept (`BlockEntityDraw::keep`), by position. As
/// `SignTexts` keeps a sign's text, it is kept relative to the block and
/// moved to the camera each frame.
#[derive(Default)]
pub(in crate::game) struct KeptGeometry {
    entries: FxHashMap<[i32; 3], KeptEntry>,
    /// Game time of the last sweep of what is out of view.
    swept: i64,
}

impl KeptGeometry {
    /// Drops what was not drawn for `KEPT_FOR` ticks, once every `KEPT_FOR`.
    fn sweep(&mut self, now: i64) {
        if (now - self.swept).abs() >= KEPT_FOR {
            self.swept = now;
            self.entries
                .retain(|_, e| (now - KEPT_FOR..=now).contains(&e.used));
        }
    }
    /// What is kept at `pos` when it was drawn with `key` and its block
    /// entity at `revision`, marked drawn at `now`.
    fn get(
        &mut self,
        pos: [i32; 3],
        key: KeptKey,
        revision: ModRevision,
        now: i64,
    ) -> Option<&KeptEntry> {
        let entry = self
            .entries
            .get_mut(&pos)
            .filter(|e| e.key == key && e.revision == revision)?;
        entry.used = now;
        Some(entry)
    }
    fn keep(
        &mut self,
        pos: [i32; 3],
        key: KeptKey,
        revision: ModRevision,
        bounds: [f32; 6],
        vertices: &[Vertex],
        spins: Vec<(usize, Option<Spin>)>,
        lit: Vec<LitElement>,
        crumbled: Vec<(usize, usize)>,
        now: i64,
    ) -> &mut KeptEntry {
        use std::collections::hash_map::Entry;
        let mut entry = KeptEntry {
            key,
            revision,
            bounds,
            used: now,
            vertices: Vec::new(),
            spins,
            lit,
            crumbled,
        };
        let entry = match self.entries.entry(pos) {
            Entry::Vacant(slot) => slot.insert(entry),
            Entry::Occupied(mut slot) => {
                std::mem::swap(&mut entry.vertices, &mut slot.get_mut().vertices);
                *slot.get_mut() = entry;
                slot.into_mut()
            }
        };
        entry.vertices.clear();
        entry.vertices.extend_from_slice(vertices);
        entry
    }

    /// Place the kept draw if its inputs still match; only a miss asks for bounds and submits.
    fn render<'v>(
        &mut self,
        pos: [i32; 3],
        key: KeptKey,
        revision: ModRevision,
        draw: &mut BlockEntityDraw<'v>,
        origin: Vec3,
        level: &LevelLight<'_>,
        crack: Option<(u8, Option<[f32; 2]>)>,
        overlay: &mut Vec<Vertex>,
        bounds: impl FnOnce() -> [f32; 6],
        visible: impl FnOnce([f32; 6], usize) -> bool,
        submit: impl FnOnce(&mut BlockEntityDraw<'v>),
    ) {
        let start = draw.vertices.len();
        let sprite = crack.and_then(|(_, sprite)| sprite);
        if let Some(entry) = self.get(pos, key, revision, draw.view.game_time) {
            if visible(entry.bounds, start) {
                let take = entry
                    .vertices
                    .len()
                    .min(draw.quad_scope().remaining_quads().saturating_mul(4))
                    / 4
                    * 4;
                if entry.spins.is_empty() {
                    draw.vertices
                        .extend(entry.vertices[..take].iter().map(|v| Vertex {
                            pos: (Vec3::from_array(v.pos) + origin).to_array(),
                            ..*v
                        }));
                    draw.quad_scope().retain_valid_since(start);
                } else {
                    draw.vertices.extend_from_slice(&entry.vertices[..take]);
                    place_geometry(
                        &mut draw.vertices[start..],
                        &entry.spins,
                        draw.view.animation_time,
                        origin,
                        (!entry.lit.is_empty()).then_some(PlacementLight {
                            elements: &entry.lit,
                            level,
                            pass: LightPass::Retained,
                        }),
                    );
                    draw.quad_scope().retain_valid_since(start);
                }
                if let Some(sprite) = sprite {
                    crack_overlay(
                        &draw.vertices[start..],
                        &entry.crumbled,
                        origin,
                        sprite,
                        overlay,
                    );
                }
            }
            return;
        }
        let bounds = bounds();
        if !visible(bounds, start) {
            return;
        }
        draw.block = pos;
        draw.pose = Mat4::IDENTITY;
        draw.parent_poses.clear();
        draw.light = key.light;
        draw.kept = false;
        draw.spins.clear();
        draw.level_lit = false;
        draw.unlit = false;
        draw.crumbling = false;
        draw.crumbling_stage = crack.map(|(stage, _)| stage);
        draw.crumbled.clear();
        draw.lit.clear();
        draw.records.clear();
        submit(draw);
        // Offsets in the shared scratch draw become local to this block's vertices.
        for (at, _) in &mut draw.spins {
            *at -= start;
        }
        for e in &mut draw.lit {
            e.start -= start;
            e.end -= start;
        }
        for (from, to) in &mut draw.crumbled {
            *from -= start;
            *to -= start;
        }
        // The scope drops quads only once it is full, so a draw that leaves room dropped none:
        // only a complete extraction is retained. A full one may have been cut short, and a
        // later frame with more room must extract it again.
        let complete = draw.quad_scope().has_room();
        let vertices = &mut draw.vertices[start..];
        let (spins, lit, crumbled, cached) = if draw.kept && complete {
            // Keep the intrinsic floors, not the sampled light written into the frame.
            let entry = self.keep(
                pos,
                key,
                revision,
                bounds,
                vertices,
                std::mem::take(&mut draw.spins),
                std::mem::take(&mut draw.lit),
                std::mem::take(&mut draw.crumbled),
                draw.view.game_time,
            );
            (
                &entry.spins,
                &entry.lit,
                &entry.crumbled,
                Some(entry.vertices.as_mut_slice()),
            )
        } else {
            self.forget(pos);
            (&draw.spins, &draw.lit, &draw.crumbled, None)
        };
        place_geometry(
            vertices,
            spins,
            draw.view.animation_time,
            origin,
            (!lit.is_empty()).then_some(PlacementLight {
                elements: lit,
                level,
                pass: LightPass::Fresh(cached),
            }),
        );
        if let Some(sprite) = sprite {
            crack_overlay(vertices, crumbled, origin, sprite, overlay);
        }
    }

    fn forget(&mut self, pos: [i32; 3]) {
        self.entries.remove(&pos);
    }
    /// Forgets everything: the atlas's sprites moved.
    pub(in crate::game::client_side) fn clear(&mut self) {
        self.entries.clear();
    }
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn api_block_entity_render_bounding_box(
        &self,
        pos: [i32; 3],
        block: &str,
        state: sunrise_world::registry::BlockStateId,
        entity: &sunrise_level::containers::ModBlockEntity,
        bounds: &mut [f32; 6],
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityRenderBoundingBox) {
            return;
        }
        let data =
            sunrise_gameplay_common::game::native_hooks::ResidentReadBlockEntity::stored(entity);
        self.native_block_entity_render_bounding_box(
            pos,
            block,
            state,
            mod_api::BlockEntityDataView::new(&data),
            bounds,
        );
    }
    /// The client's mod animation clock: level-running texture ticks plus a phase that holds
    /// across pauses and freezes. Server time updates never change it.
    pub(in crate::game) fn animation_time(&self) -> f64 {
        self.client.texture_ticks as f64 + self.common.clock.animation_phase()
    }

    /// The camera entity's interpolated eye position for the current world view, or `None`
    /// when there is no camera entity (title screen, scenes, or no player/spectated entity).
    pub(in crate::game) fn world_camera_entity_eye(&self) -> Option<[f64; 3]> {
        if self.shows_panorama() {
            return None;
        }
        if let Some(eye) = self.client.native_camera_entity_eye {
            return Some(eye);
        }
        if self.client.view.initialized {
            self.camera_subject().map(|_| self.client.view.eye)
        } else {
            self.camera_eye_position()
        }
    }

    /// Build a frame camera snapshot for `position` and `partial` when a draw runs outside
    /// `Game::render` or for a secondary view whose position differs from `native_camera_frame`.
    pub(in crate::game) fn fallback_camera(
        &self,
        position: DVec3,
        partial: f32,
    ) -> mod_api::Camera {
        let (yaw, pitch, roll) = self.client.posed_rotation.unwrap_or((
            self.client.view.y_rot,
            self.client.view.x_rot,
            self.client.view.roll,
        ));
        let view = if roll == 0.0 {
            sunrise_client::camera::view_matrix(yaw, pitch)
        } else {
            sunrise_client::camera::view_matrix_rolled(yaw, pitch, roll)
        };
        mod_api::Camera {
            position: position.to_array(),
            eye_height: self.client.view.eye_height_old
                + (self.client.view.eye_height - self.client.view.eye_height_old) * partial,
            partial_tick: partial,
            yaw,
            pitch,
            roll,
            fov_degrees: self.client.settings.fov,
            near: 0.05,
            far: sunrise_client::camera::depth_far(
                self.terrain_distance(),
                self.client.settings.cloud_range,
            ),
            view: view.to_cols_array(),
            projection: glam::Mat4::IDENTITY.to_cols_array(),
            dimension: self.common.dim.id().to_owned(),
        }
    }

    /// Borrow the active world view for `camera` and `partial`, preferring an explicit
    /// per-view [`mod_api::WorldRenderView`] (such as a scene or secondary view) or the
    /// current frame's cached camera snapshot with zero allocation.
    pub(in crate::game) fn with_world_view<R>(
        &self,
        camera: DVec3,
        partial: f32,
        explicit: Option<&mod_api::WorldRenderView<'_>>,
        f: impl FnOnce(mod_api::WorldRenderView<'_>) -> R,
    ) -> R {
        if let Some(&view) = explicit {
            return f(view);
        }
        let eye = self.world_camera_entity_eye();
        let anim = self.animation_time();
        let oit = self.client.settings.improved_transparency;
        if let Some(frame_cam) = self
            .client
            .native_camera_frame
            .as_ref()
            .filter(|c| c.position == camera.to_array())
        {
            return f(mod_api::WorldRenderView::new(
                frame_cam,
                eye,
                partial,
                self.common.world.game_time,
                anim,
                oit,
                &[],
            ));
        }
        let owned = self.fallback_camera(camera, partial);
        f(mod_api::WorldRenderView::new(
            &owned,
            eye,
            partial,
            self.common.world.game_time,
            anim,
            oit,
            &[],
        ))
    }
    /// The mod block entities within view distance of `camera` whose block's
    /// template has none, which the mesher's block entity lists (made from
    /// the template) leave out: where they are and their block's state, for
    /// `mod_block_entity_geometry`. From the per-chunk list of
    /// `BlockEntities::mods_near`, not a scan of every block entity.
    pub(in crate::game::client_side) fn untemplated_mod_block_entities(
        &self,
        camera: DVec3,
    ) -> Vec<([i32; 3], u32)> {
        self.common
            .world
            .block_entities
            .mods_near(camera.x, camera.z, self.view_distances().furthest)
            .map(|p| ([p.0, p.1, p.2], self.common.world.block(p.0, p.1, p.2).0))
            .filter(|&(_, state)| {
                !self
                    .common
                    .res
                    .facts()
                    .get(BlockStateId(state))
                    .block_entity
            })
            .collect()
    }
    /// The mod block entities' view distances: the parts are asked once for each mod block
    /// (`native_block_entity_view_distance`) the first time they're needed, and kept.
    fn view_distances(&self) -> &ViewDistances {
        self.client.mod_view_distances.get_or_init(|| {
            let registry = self.common.pack.assets.registry;
            let states = (0..registry.len() as u32).filter_map(|i| {
                registry
                    .state(BlockStateId(i))
                    .map(|state| (i, state.name.as_str()))
            });
            ViewDistances::of_states(states, |block| {
                let mut distance = VIEW_DISTANCE;
                self.native_block_entity_view_distance(block, &mut distance);
                distance
            })
        })
    }

    /// Device-derived combined entity quad capacity for this frame's renderer.
    pub(in crate::game) fn entity_quad_capacity(&self) -> sunrise_client::gfx::EntityQuadCapacity {
        self.mod_gpu().map_or_else(
            sunrise_client::gfx::EntityQuadCapacity::default_device,
            sunrise_client::gfx::ModGpuResources::entity_quad_capacity,
        )
    }

    /// Complete quads already occupied by other draw sources in the same combined entity batch
    /// (GPU-posed entities and outlines, see-through text, shadow-only player geometry, and the
    /// active frame's additive, hand, or entity streams outside the current target buffer).
    pub(in crate::game) fn entity_batch_reserved_quads(&self) -> usize {
        let (posed, outlined) = {
            let skin = self.client.skin_frame.borrow();
            (
                (skin.vertices / 4) as usize,
                (skin.outline_vertices / 4) as usize,
            )
        };
        let see_through = self.client.see_through_vertices.len() / 4;
        let shadow_only = self.client.shadow_only_vertices.borrow().len() / 4;
        self.client
            .frame_batch_reserved_quads
            .get()
            .saturating_add(posed)
            .saturating_add(outlined)
            .saturating_add(see_through)
            .saturating_add(shadow_only)
    }
    /// `native_block_entity_submit` for each of `blocks` (paired with whether its section is
    /// eye-visible, drawn by no vanilla renderer) whose block entity is a mod's, closer than
    /// `BlockEntityRenderer.getViewDistance` to the camera (`shouldRender`) and with its bounds
    /// (`native_block_entity_render_bounding_box`) in the frustum. Ordinary (non-global)
    /// renderers require an eye-visible section, matching built-in block entities.
    /// Returns the crack overlay of the geometry its mods drew under `crumbling` (the
    /// `DESTROY` stage's vertices, for the caller to draw after marking that stage), empty
    /// when nothing is being mined.
    pub(in crate::game::client_side) fn mod_block_entity_geometry<'a>(
        &self,
        blocks: impl Iterator<Item = (&'a ([i32; 3], u32), bool)>,
        camera: DVec3,
        partial: f32,
        out: &mut Vec<Vertex>,
    ) -> Vec<Vertex> {
        self.mod_block_entity_geometry_with_view(blocks, camera, partial, None, out)
    }

    /// [`Self::mod_block_entity_geometry`] with an explicit [`mod_api::WorldRenderView`] for
    /// multi-view rendering.
    pub(in crate::game::client_side) fn mod_block_entity_geometry_with_view<'a>(
        &self,
        blocks: impl Iterator<Item = (&'a ([i32; 3], u32), bool)>,
        camera: DVec3,
        partial: f32,
        explicit_view: Option<&mod_api::WorldRenderView<'_>>,
        out: &mut Vec<Vertex>,
    ) -> Vec<Vertex> {
        self.with_world_view(camera, partial, explicit_view, |view| {
            let mut draw = BlockEntityDraw {
                view,
                light: 0,
                block: [0; 3],
                pose: Mat4::IDENTITY,
                normal_pose: None,
                local_light_translation: None,
                parent_poses: Vec::new(),
                lights: diffuse_lights(self.nether_lighting()),
                first_vertex: out.len(),
                vertices: std::mem::take(out),
                kept: false,
                spins: Vec::new(),
                level_lit: false,
                unlit: false,
                crumbling: false,
                crumbling_stage: None,
                crumbled: Vec::new(),
                lit: Vec::new(),
                prepared: prepared_route(
                    self.client.pack_ids.is_some(),
                    !self.client.native_shaders.picked.is_empty(),
                    self.renderer_prepared_generation(),
                    self.client.standalone_geometry.generation(),
                ),
                records: Vec::new(),
                quad_capacity: self.entity_quad_capacity(),
                reserved_quads: self.entity_batch_reserved_quads(),
                nested: false,
            };
            let now = view.game_time;
            let mut kept = self.client.kept_block_entities.borrow_mut();
            kept.sweep(now);
            // The stage stored at each cracked position: empty, and never looked in, with no crack.
            let cracks = self.crack_stage_map();
            let mut overlay = Vec::new();
            let distances = self.view_distances();
            let light_at = |b: [i32; 3]| self.common.world.light(b[0], b[1], b[2]);
            for (&(pos, state), eye_visible) in blocks {
                let corner = DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
                // Vec3.closerThan: strictly closer.
                let distance = distances.of(state);
                if (corner + 0.5).distance_squared(camera) >= distance * distance {
                    continue;
                }
                let Some(BlockEntity::Mod(entity)) = self
                    .common
                    .world
                    .block_entities
                    .get(&(pos[0], pos[1], pos[2]))
                else {
                    continue;
                };
                let origin = (corner - camera).as_vec3();
                let light = vertex_light(self.common.world.light(pos[0], pos[1], pos[2]));
                let key = KeptKey {
                    dim: self.common.dim,
                    state,
                    light,
                    glyphs: self.client.gui.atlas.font.epoch(),
                };
                let id = BlockStateId(state);
                let block = || &*self.common.res.facts().get(id).name;
                kept.render(
                    pos,
                    key,
                    entity.revision(),
                    &mut draw,
                    origin,
                    &LevelLight {
                        origin: corner,
                        light_at: &light_at,
                    },
                    (!cracks.is_empty())
                        .then(|| cracks.get(&pos))
                        .flatten()
                        .map(|&stage| (stage, self.crack_origin(u32::from(stage)))),
                    &mut overlay,
                    || {
                        let mut bounds = DEFAULT_BOUNDS;
                        self.api_block_entity_render_bounding_box(
                            pos,
                            block(),
                            id,
                            entity,
                            &mut bounds,
                        );
                        bounds
                    },
                    |bounds, start| {
                        if !mod_block_entity_visible(
                            self.client.view_frustum.as_ref(),
                            origin,
                            bounds,
                            eye_visible,
                            self.block_entity_reach(state).off_screen(),
                        ) {
                            return false;
                        }
                        self.mark_block_entity(start, state);
                        true
                    },
                    |draw| {
                        self.api_block_entity_submit(pos, block(), id, entity, draw);
                        self.finish_prepared_draw(draw, corner, camera);
                    },
                );
            }
            *out = draw.vertices;
            overlay
        })
    }

    pub(in crate::game::client_side) fn prepared_copy_room(
        &self,
        already: u64,
        incoming: usize,
    ) -> Option<usize> {
        let room = self
            .renderer_prepared_room_info()
            .map(|(max_buffer, prior)| {
                sunrise_client::gfx::prepared_instance_room_for(
                    already.saturating_add(prior),
                    incoming,
                    max_buffer,
                )
            });
        #[cfg(test)]
        let room = room.or_else(|| sunrise_client::gfx::capped_prepared_room(already, incoming));
        room
    }

    /// Keep, spin, or no renderer bakes every copy into the vertex span.
    /// Otherwise copies that fit the instance buffer are staged, and the rest are
    /// baked at their own cut so they stay after the instanced prefix.
    fn finish_prepared_draw(&self, draw: &mut BlockEntityDraw, origin: DVec3, camera: DVec3) {
        if draw.records.is_empty() {
            return;
        }
        if draw.kept || !draw.spins.is_empty() {
            draw.bake_recorded(&self.client.standalone_geometry.slots);
            return;
        }
        let already = self.client.prepared_model_draws.borrow().len() as u64;
        let incoming = draw.records.len();
        let Some(room) = self.prepared_copy_room(already, incoming) else {
            draw.bake_recorded(&self.client.standalone_geometry.slots);
            return;
        };
        if room == 0 {
            draw.bake_recorded(&self.client.standalone_geometry.slots);
            return;
        }
        let overflow = (room < draw.records.len()).then(|| draw.records.split_off(room));
        draw.push_instances(
            origin,
            camera,
            &mut self.client.prepared_model_draws.borrow_mut(),
        );
        if let Some(overflow) = overflow {
            draw.records = overflow;
            draw.bake_recorded(&self.client.standalone_geometry.slots);
        }
    }
}

/// Helpers for drawing mod block entities, kept as stable as the hook points: the
/// methods of `BlockEntityDrawContext`.
// Native mods use these; nothing in Sunrise does.
#[allow(dead_code)]
impl crate::game::ClientGame<'_> {
    /// `PoseStack.pushPose`: `pop` comes back to the pose as it is now.
    pub(in crate::game) fn native_draw_push(&self, draw: &mut BlockEntityDraw) {
        draw.push();
    }
    /// `PoseStack.popPose`; the block's own pose is never popped.
    pub(in crate::game) fn native_draw_pop(&self, draw: &mut BlockEntityDraw) {
        draw.pop();
    }
    /// Moves what is drawn next by `by`, in blocks.
    pub(in crate::game) fn native_draw_translate(&self, draw: &mut BlockEntityDraw, by: [f32; 3]) {
        draw.translate(by);
    }
    /// Turns what is drawn next by `degrees` about `axis` (counterclockwise
    /// seen from the axis' positive end, as `Axis.rotationDegrees`) through
    /// `pivot`, in blocks from the pose's origin: `[0.5; 3]` is the block's centre.
    pub(in crate::game) fn native_draw_rotate_about(
        &self,
        draw: &mut BlockEntityDraw,
        axis: [f32; 3],
        degrees: f32,
        pivot: [f32; 3],
    ) {
        draw.rotate_about(axis, degrees, pivot);
    }
    /// Scales what is drawn next by `by` about the pose's origin.
    pub(in crate::game) fn native_draw_scale(&self, draw: &mut BlockEntityDraw, by: [f32; 3]) {
        draw.scale(by);
    }
    /// What is drawn next turns by itself on the animation clock (`BlockEntityDraw::spin`).
    pub(in crate::game) fn native_draw_spin(
        &self,
        draw: &mut BlockEntityDraw,
        axis: [f32; 3],
        pivot: [f32; 3],
        degrees_per_tick: f32,
        phase: f32,
    ) {
        draw.spin(axis, pivot, degrees_per_tick, phase);
    }
    /// What is drawn next stands still again, after `spin`.
    pub(in crate::game) fn native_draw_still(&self, draw: &mut BlockEntityDraw) {
        draw.still();
    }
    /// Keeps what this call drew, with neither `native_block_entity_submit` nor
    /// `native_block_entity_render_bounding_box` called until its state, block entity,
    /// light or resource packs change. Parts marked with `spin` keep turning on the
    /// animation clock; other geometry stays as submitted. Do not bake frame-dependent
    /// values into kept vertices: item glint and time-dependent item models stop
    /// where they were, and neighboring blocks do not invalidate the cache.
    pub(in crate::game) fn native_draw_keep(&self, draw: &mut BlockEntityDraw) {
        draw.keep();
    }
    /// The frame's partial tick, 0 to 1 (`BlockEntityRenderState`).
    pub(in crate::game) fn native_draw_partial(&self, draw: &BlockEntityDraw<'_>) -> f32 {
        draw.view.partial_tick
    }
    /// The level's game time, for animations.
    pub(in crate::game) fn native_draw_game_time(&self, draw: &BlockEntityDraw<'_>) -> i64 {
        draw.view.game_time
    }
    /// The frame on this client's animation clock: whole level ticks plus the animation phase,
    /// held through a tick freeze and carried on without a jump. This is what turns `spin` parts.
    pub(in crate::game) fn native_draw_animation_time(&self, draw: &BlockEntityDraw<'_>) -> f64 {
        draw.view.animation_time
    }
    /// The light the helpers draw with: the block's own
    /// (`LevelRenderer.getLightCoords`), 16 times sky light in bits 0..8 and
    /// 16 times block light in bits 8..16.
    pub(in crate::game) fn native_draw_light(&self, draw: &BlockEntityDraw) -> u32 {
        draw.light
    }
    /// Lights what is drawn next with `light` (as `native_draw_light` reads it).
    pub(in crate::game) fn native_draw_set_light(&self, draw: &mut BlockEntityDraw, light: u32) {
        draw.set_light(light);
    }
    /// Lights what is drawn next by the level at each element's centre
    /// (`BlockEntityDrawContext::level_light`).
    pub(in crate::game) fn native_draw_level_light(&self, draw: &mut BlockEntityDraw) {
        draw.level_light();
    }
    /// What is drawn next skips directional shading and draws full bright while `on`
    /// (`BlockEntityDrawContext::unlit`).
    pub(in crate::game) fn native_draw_unlit(&self, draw: &mut BlockEntityDraw, on: bool) {
        draw.unlit(on);
    }

    /// The breaking stage at the block drawn (`BlockEntityDrawContext::crumbling_stage`).
    pub(in crate::game) fn native_draw_crumbling_stage(
        &self,
        draw: &BlockEntityDraw,
    ) -> Option<u8> {
        draw.crumbling_stage
    }
    /// What is drawn next by `quad`, `cube`, `block`, `model` and `terrain_model` gets the
    /// breaking crack while `on` (`BlockEntityDrawContext::crumbling`).
    pub(in crate::game) fn native_draw_crumbling(&self, draw: &mut BlockEntityDraw, on: bool) {
        draw.crumbling(on);
    }
    /// Draws a quad with `corners` in blocks under `draw`'s pose, in a model
    /// face's order (top left, bottom left, bottom right, top right as seen
    /// from its front), textured with the `uv` rectangle (u0, v0, u1, v1 in
    /// texels of 16 per side) of `sprite` (`mymod:entity/screen`,
    /// `mymod:block/casing`, `block/stone`) times the 0xRRGGBB `tint`
    /// (0xFFFFFF for none). False for an unknown sprite.
    pub(in crate::game) fn native_draw_quad(
        &self,
        draw: &mut BlockEntityDraw,
        sprite: &str,
        corners: [[f32; 3]; 4],
        uv: [f32; 4],
        tint: u32,
    ) -> bool {
        let Some(rect) = self.client.gui.atlas.sprites.get(sprite).copied() else {
            return false;
        };
        let [u0, v0, u1, v1] = uv.map(|t| t / 16.0);
        draw.quad(
            corners,
            [[u0, v0], [u0, v1], [u1, v1], [u1, v0]],
            rect,
            tint,
        );
        true
    }

    /// Draws a quad of `corners` under `draw`'s pose showing the dynamic texture `render_type`
    /// names, with unit `uv`s (`BlockEntityDrawContext::textured_quad`). False for a handle that
    /// is not one of the three texture-taking built-ins' of this resource load, a texture that
    /// is not live, or input `draw` refuses.
    pub(in crate::game) fn native_draw_textured_quad(
        &self,
        draw: &mut BlockEntityDraw,
        render_type: mod_api::RenderTypeRef,
        corners: [[f32; 3]; 4],
        uv: [[f32; 2]; 4],
        tint: u32,
    ) -> bool {
        use sunrise_client::gfx::{
            DYNAMIC_TEXTURE_CUTOUT_OVERLAY, DYNAMIC_TEXTURE_SOLID_OVERLAY,
            DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY,
        };
        use sunrise_client_render::mod_pipelines::BuiltinRenderType;
        let Ok((kind, texture)) = self
            .client
            .client_pack
            .render
            .textured_render_type_of(render_type)
        else {
            return false;
        };
        let overlay = match kind {
            BuiltinRenderType::EntitySolid => DYNAMIC_TEXTURE_SOLID_OVERLAY,
            BuiltinRenderType::EntityCutout => DYNAMIC_TEXTURE_CUTOUT_OVERLAY,
            BuiltinRenderType::EntityTranslucent => DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY,
            _ => return false,
        };
        // A live texture when the quad is submitted; one released after it draws the missing
        // texture (`TextureManager.getTexture`).
        let Some((handle, _)) = self.client.dynamic_textures.get(&texture) else {
            return false;
        };
        draw.textured_quad(corners, uv, overlay, handle, tint)
    }

    /// Draws `text` under `draw`'s pose as vanilla's `Font.drawInBatch` draws a
    /// component in the world (a sign's line, a text display's): in the pose's
    /// x-y plane, one unit a font pixel, y down, its left end at `at`, facing
    /// +z. Scale the pose first as a sign does (`draw.scale([1.0 / 64.0,
    /// -1.0 / 64.0, 1.0 / 64.0])` for its letters, 8 pixels high). It is drawn
    /// with its styles, through the path name tags and text displays use
    /// (`HudBatch::text_styled_world`): each span's colour (the rest in the
    /// 0xRRGGBB `color`), bold, italic, underline, strikethrough, obfuscated,
    /// its font, inline objects and `§` codes, right-to-left text in visual
    /// order. `glowing` text is full bright (a glow ink sac's), the rest lit by
    /// `draw.light`. With `outline`, a 0xRRGGBB colour (`Some(0)` is black),
    /// each glyph is ringed with it as glowing sign text is
    /// (`Font.drawInBatch8xOutline`). Returns the width drawn, in font pixels
    /// (`Font.width`): centre a line by starting it at minus half its width.
    /// Obfuscated glyphs change only when the block is drawn again, so kept
    /// text (`keep`) stands still. API @API:draw_text@.
    pub(in crate::game) fn native_draw_text(
        &self,
        draw: &mut BlockEntityDraw,
        text: &Text,
        at: [f32; 2],
        color: u32,
        glowing: bool,
        outline: Option<u32>,
    ) -> f32 {
        let opaque =
            |c: u32| sunrise_client::hud::rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, 255);
        let atlas = &self.client.gui.atlas;
        let mut batch = sunrise_client::hud::HudBatch::scratch([1, 1]);
        let (width, front) = match outline {
            Some(ring) => batch.text_styled_outline_world(
                atlas,
                text,
                at[0],
                at[1],
                opaque(color),
                opaque(ring),
            ),
            None => (
                batch.text_styled_world(atlas, text, at[0], at[1], opaque(color), false),
                0,
            ),
        };
        // Glyphs new to the atlas upload with this frame (`Game::text_patches`).
        self.client
            .text_patches
            .borrow_mut()
            .append(&mut batch.patches);
        let light = if glowing { 240 | 240 << 8 } else { draw.light };
        let pose = draw.pose();
        let mut scope = draw.quad_scope();
        for (i, quad) in batch.vertices.as_chunks::<6>().0.iter().enumerate() {
            let lift = if outline.is_some() && i * 6 >= front {
                TEXT_OVER_OUTLINE
            } else {
                0.0
            };
            let corners = sunrise_client::text_display::CORNERS.map(|c| quad[c]);
            let submitted = corners.map(|v| Vertex {
                pos: pose
                    .transform_point3(Vec3::new(v.pos[0], v.pos[1], v.depth + lift))
                    .to_array(),
                uv: v.uv,
                layer: 0,
                color: v.color,
                light,
                normal: [0; 4],
                glint_uv: [0.0, 0.0],
            });
            if !scope.submit_quad(submitted) {
                break;
            }
        }
        width.ceil()
    }
    /// The width of `text` in font pixels, as `draw_text` draws it, styles
    /// included (`Font.width`), to lay a line out before drawing it.
    pub(in crate::game) fn native_draw_text_width(
        &self,
        _draw: &BlockEntityDraw,
        text: &Text,
    ) -> f32 {
        self.client.gui.atlas.font.width_text(text)
    }

    /// Draws a box from `from` to `to` (texels of 16 per block, as a block
    /// model's element) under `draw`'s pose, with the faces given, by
    /// `Direction` (down, up, north, south, west, east), baked as a model
    /// element's are (`FaceBakery`), times the 0xRRGGBB `tint`. A turned box
    /// is drawn under a turned pose (`BlockEntityDraw::rotate_about`). False
    /// when a face's sprite is unknown; that face is left out.
    pub(in crate::game) fn native_draw_cube(
        &self,
        draw: &mut BlockEntityDraw,
        from: [f32; 3],
        to: [f32; 3],
        faces: [Option<CubeFace<'_>>; 6],
        tint: u32,
    ) -> bool {
        let mut all = true;
        let start = draw.vertices.len();
        let ambient = draw.element_ambient();
        for (direction, face) in Direction::ALL.into_iter().zip(faces) {
            let Some(face) = face else { continue };
            let Some(rect) = self.client.gui.atlas.sprites.get(face.sprite).copied() else {
                all = false;
                continue;
            };
            let uv = face.uv.unwrap_or_else(|| default_uv(from, to, direction));
            let material = Material {
                sprite: String::new(),
                force_translucent: false,
            };
            if let Ok(quad) = bake_flat_face(from, to, uv, direction, material, -1) {
                draw.emit_quad(quad.positions, quad.uv, rect, tint, ambient);
            }
        }
        draw.drawn_since(
            start,
            || {
                (DVec3::from_array(from.map(f64::from)) + DVec3::from_array(to.map(f64::from)))
                    / 32.0
            },
            draw.light,
        );
        draw.crumbling_since(start);
        all
    }

    /// Draws resolved block state `state`'s baked model under `draw`'s pose
    /// with its default tints, matching vanilla's
    /// `SubmitNodeCollector.submitBlockModel` (`BlockEntityDrawContext::block`
    /// lowers an API `BlockState` to `BlockStateId` before calling here). A
    /// moving part that is not a block is a standalone model
    /// (`native_draw_model`). Returns `false` without emitting vertices when
    /// `state` is not in the active block registry.
    pub(in crate::game) fn native_draw_block(
        &self,
        draw: &mut BlockEntityDraw,
        state: BlockStateId,
    ) -> bool {
        let Some(model) = self.resolved_block_model(state) else {
            return false;
        };
        let start = draw.vertices.len();
        let light = draw.element_ambient();
        crate::game::client_side::minecarts_client::emit_model_faces(
            model.prepared.iter().map(|&face| (face, light)),
            draw.pose(),
            true,
            |index| model.tints.get(&index).copied(),
            &mut draw.vertices,
        );
        draw.drawn_since(start, BlockEntityDraw::cell_centre, draw.light & 0xffff);
        draw.crumbling_since(start);
        true
    }

    /// `BlockEntityRenderDispatcher`'s extract and submit for `entity`, a block entity of
    /// block state `state` (None: the state is not in the registry) that the caller carries
    /// (`BlockEntityDrawContext::block_entity`), under `draw`'s pose and light, with no overlay
    /// (`NO_OVERLAY`). The entity's own position is never read and `shouldRender`'s culling is
    /// skipped: the caller's entity was culled already.
    ///
    /// A state with a vanilla renderer draws through `BlockEntityRenderer::render_state`
    /// (`sunrise_client::block_entity_render`), the renderer's no-level form, from the data in
    /// `entity.fields.nbt`; any other state's mod Part submits, one level deep, with the entity
    /// as its data. A state with neither draws nothing.
    pub(in crate::game) fn native_draw_block_entity(
        &self,
        draw: &mut BlockEntityDraw<'_>,
        state: Option<BlockStateId>,
        entity: &mod_api::SceneBlockEntity,
    ) -> Result<(), mod_api::BlockEntityDrawRefusal> {
        draw.refuse_nested()?;
        let Some(state) = state else {
            return Ok(());
        };
        let block = &*self.common.res.facts().get(state).name;
        let renderer = self.client_res().block_entity_render();
        if !renderer.renders(state.0) {
            draw.one_level(|draw| {
                self.native_block_entity_submit(
                    draw.block,
                    block,
                    state,
                    mod_api::BlockEntityDataView::new(entity),
                    draw,
                );
            });
            return Ok(());
        }
        // The saved tag as `BlockEntity.loadWithComponents` reads it, at the origin.
        let mut fields = entity.fields.nbt.clone();
        fields.insert(
            "id".into(),
            sunrise_core::nbt::Tag::String(entity.id.as_str().into()),
        );
        for key in ["x", "y", "z"] {
            fields.insert(key.into(), sunrise_core::nbt::Tag::Int(0));
        }
        let saved = sunrise_core::nbt::Tag::Compound(fields);
        let decoded = sunrise_sim::save::decode_block_entity_for_block(&saved, block)
            .map(|(_, entity)| entity);
        let anim = sunrise_client::block_entity_render::BlockAnim {
            sherds: if block == "minecraft:decorated_pot" {
                sunrise_gameplay_common::game::block_entity_anims_common::pot_sherds(Some(&saved))
            } else {
                Default::default()
            },
            ..Default::default()
        };
        let banner = match &decoded {
            Some(BlockEntity::Banner(banner)) => {
                layer_textures(&self.stations().banners, &banner.layers)
            }
            _ => Vec::new(),
        };
        let skin = match &decoded {
            Some(BlockEntity::Skull(skull)) => self.skull_skin(skull),
            _ => None,
        };
        let gui = &self.client.gui;
        let scene = sunrise_client::block_entity_render::Scene {
            camera: DVec3::ZERO,
            sign_outline: sunrise_client::block_entity_render::SignOutline::None,
            orientation: Quat::IDENTITY,
            anim: &|_| anim.clone(),
            texts: None,
            partial: draw.view.partial_tick,
            game_time: draw.view.game_time,
            max_y: self.common.world.shape.max_y() - 1,
            world: &self.common.world,
            sprites: &gui.atlas.sprites,
            emissive: &gui.atlas.emissive,
            translucent_sprites: &gui.atlas.translucent,
            atlas_revision: gui.atlas.revision,
            nether: self.nether_lighting(),
            lid: &|_| 0.0,
            sign: &|_| match &decoded {
                Some(BlockEntity::Sign(sign)) => Some(sign),
                _ => None,
            },
            inline_images: None,
            banner: &|_| banner.clone(),
            skull: &|_| skin.clone(),
            glyph: &|c| gui.atlas.glyph_cell(c),
            glyphs: gui.atlas.font.epoch(),
            font: Some(&gui.atlas.font),
            dynamic_visible: None,
        };
        if !draw.quad_scope().has_room() {
            return Ok(());
        }
        let start = draw.vertices.len();
        renderer.render_state(
            &scene,
            state.0,
            draw.pose(),
            crate::game::client_side::minecarts_client::special_light(draw.light),
            &mut draw.vertices,
        );
        draw.quad_scope().retain_valid_since(start);
        draw.drawn_since(start, BlockEntityDraw::cell_centre, draw.light & 0xffff);
        Ok(())
    }

    /// Resolved block state `id` as one model: what `native_draw_block` draws
    /// for a `BlockState` lowered to `BlockStateId`, combining the blockstate
    /// file's parts at `BLOCK_MODEL_SEED` (42) with each tinted face holding
    /// the block's default tint (no position) in its colour. Bound when the generation is
    /// prepared.
    pub fn native_block_state_model(&self, id: BlockStateId) -> Result<ModelRef, ModelError> {
        let _ = self
            .resolved_block_model(id)
            .filter(|model| !model.faces.is_empty() || model.special)
            .ok_or(ModelError::UnknownModel)?;
        Ok(mod_api::__private::built::model_ref(
            self.client.standalone_geometry.generation(),
            self.client.standalone_geometry.states_from + id.0,
        ))
    }

    /// `CompositeBlockModel`'s second part for block state `state`: what its special renderer
    /// draws with default data, after the block-state model's faces, under the same pose and
    /// packed `light`. It takes no tint and no hidden faces; `color` (`0xAARRGGBB`) multiplies
    /// it, and below opaque it blends as `entityTranslucent` does. The end portal and gateway
    /// draw their starfield whatever `color` is, as `rendertype_end_portal` does. Non-finite
    /// quads are skipped individually and the contribution stays within `scope`'s budget.
    pub(in crate::game) fn special_model_part(
        &self,
        state: BlockStateId,
        pose: Mat4,
        light: u32,
        color: u32,
        scope: &mut sunrise_client::gfx::EntityQuadScope<'_>,
    ) {
        use sunrise_client::block_entity_render::{END_GATEWAY_LAYER, END_PORTAL_LAYER};
        if !scope.has_room() {
            return;
        }
        let start = scope.vertices_mut().len();
        self.special_block_model(
            state,
            pose,
            crate::game::client_side::minecarts_client::special_light(light),
            scope.vertices_mut(),
        );
        scope.retain_valid_since(start);
        let out = scope.vertices_mut();
        if color != u32::MAX {
            for vertex in &mut out[start..] {
                vertex.color =
                    sunrise_client::terrain_model::multiply_draw_color(vertex.color, color);
            }
        }
        // The portal shader writes an opaque starfield, so a faded portal stays opaque.
        let portal = out[start..]
            .iter()
            .any(|vertex| matches!(vertex.layer & 0xFFFF, END_PORTAL_LAYER | END_GATEWAY_LAYER));
        if color >> 24 != 0xFF && !portal {
            sunrise_client::gfx::mark_oit(&mut out[start..]);
        }
    }

    /// Share the same prepared state model for terrain submissions and block-entity draws.
    fn resolved_block_model(&self, id: BlockStateId) -> Option<&StateModel> {
        self.client.standalone_geometry.state_model(id)
    }

    /// The baked faces a slot draws through the terrain materials: a standalone model's, or a
    /// block state's from `native_block_state_model`.
    pub fn model_faces(&self, slot: u32) -> Option<&[sunrise_client::models::catalog::BakedFace]> {
        let catalog = &self.client.client_pack.render.model_catalog;
        match catalog.model_by_slot(slot) {
            Some(model) => Some(&model.faces),
            None => self.client.standalone_geometry.state_faces(slot),
        }
    }

    /// Resolve a registered standalone model once in `client_setup` and again in
    /// `client_resources_reloaded`, not each frame. Resource-pack overrides and other
    /// namespaces use the normal shared model store; no resource is read or baked here.
    pub(in crate::game) fn native_model(&self, id: &Identifier) -> Result<ModelRef, ModelError> {
        self.client.client_pack.render.model(id)
    }

    /// Draws the resolved standalone model under `draw`'s pose: unturned, from
    /// the block's lower north-west corner, lit by `draw.light`. Faces with a tint
    /// index are multiplied by `tint` (`0xAARRGGBB`, the same as `WorldQuad::color`;
    /// `0xFFFFFFFF` for none), alpha included. Faces with no tint index stay opaque.
    /// A stale generation or absent slot returns an error without appending geometry.
    pub(in crate::game) fn native_draw_model(
        &self,
        draw: &mut BlockEntityDraw,
        model: ModelRef,
        tint: u32,
    ) -> Result<(), ModelError> {
        use mod_api::__private::game::Fields;
        let fields = model.fields();
        if fields.generation != self.client.standalone_geometry.generation() {
            return Err(ModelError::StaleModel);
        }
        let faces = self
            .client
            .standalone_geometry
            .slots
            .get(fields.slot as usize)
            .and_then(Option::as_ref)
            .ok_or(ModelError::UnknownModel)?;
        if draw.defer_model(fields.slot, fields.generation, tint) {
            return Ok(());
        }
        draw.model(faces, tint);
        Ok(())
    }

    /// Draws model part `part` and the parts under it under `draw`'s pose, as
    /// vanilla's `ModelPart.render` does: each part moved to its place (in
    /// texels, 16 to a block), turned about z, then y, then x, and scaled,
    /// its faces textured from `sprite` (`mymod:entity/golem`, a content
    /// mod's `assets/mymod/textures/entity/golem.png`, or vanilla's
    /// `entity/zombie/zombie`) times the 0xRRGGBB `tint`. A part that isn't
    /// `visible` draws nothing, one that skips drawing (`skip_draw`) only its
    /// children. A mob's model is built in the model's own frame (+y down,
    /// its front towards -z, its feet 24 texels down): in `native_render_living_pre`,
    /// `draw.scale([1.0, -1.0, -1.0])` then `draw.translate([0.0, -1.501,
    /// 0.0])` first, as `LivingEntityRenderer` does. False for an unknown
    /// sprite. API @API:model_layers@.
    pub fn native_draw_model_part(
        &self,
        draw: &mut BlockEntityDraw,
        part: &ModelPart,
        sprite: &str,
        tint: u32,
    ) -> bool {
        let Some(rect) = self.client.gui.atlas.sprites.get(sprite).copied() else {
            return false;
        };
        draw_model_part(draw, part, rect, tint);
        true
    }

    /// Entity model layer `layer` baked into its `ModelPart` tree, each part
    /// at its pose, as `EntityModelSet.bakeLayer` bakes it: one a native mod
    /// registered (`native_register_layer_definitions`), or vanilla's, by the name
    /// `entity-models.json` gives it (`minecraft:zombie#main`,
    /// `minecraft:player#main`). The tree is the caller's own copy: bake it
    /// once, as a renderer does, and set its parts' poses each frame. None
    /// for a layer neither has. A vanilla layer no mob has (a player's, a
    /// chest's) is read from the resources each call, so slow. API
    /// @API:model_layers@.
    pub fn bake_layer(&self, layer: &str) -> Option<ModelPart> {
        if let Some(definition) = model_layers::registered(layer) {
            return Some(definition.bake_root());
        }
        self.client.entity_models.layers.get(layer).cloned()
    }

    /// Draws `stack` under `draw`'s pose as an item is drawn in the world
    /// (`ItemModelResolver.updateForTopItem` and `ItemStackRenderState.submit`), centred on the
    /// pose's origin, in its model's `display` transform: `Fixed` as a
    /// campfire's or an item frame's, `Ground` as a dropped item's. `seed`
    /// picks among random models (a campfire passes its slot plus its
    /// position's hash). `owner_yaw` makes the block entity the item's
    /// owner (`ItemOwner`), at the block's centre and turned that many
    /// degrees (`getVisualRotationYInDegrees`), as a shelf is: a compass on
    /// it points to its target from there. None draws it with no owner, as
    /// a campfire does. False for an empty stack or one with no model. The
    /// client's copy of a block entity has no item slots (`entity.items` is
    /// empty there): send the stacks you draw in
    /// `native_block_entity_update_tag` and read them from `entity.nbt`.
    pub(in crate::game) fn native_draw_item(
        &self,
        draw: &mut BlockEntityDraw,
        stack: &ItemStack,
        display: DisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        // ShelfBlockEntity.position(): Vec3.atCenterOf(getBlockPos()).
        let owner = owner_yaw.map(|yaw| {
            crate::game::client_side::item_draw::owner_at(
                draw.block.map(|c| f64::from(c) + 0.5),
                yaw,
                "",
            )
        });
        let item = self.resolve_item(stack, display, owner.as_ref(), seed);
        if item.is_empty() {
            return false;
        }
        let pose = draw.pose();
        if draw.unlit {
            let full = sunrise_client::mobs::render::FULL_BRIGHT;
            self.submit_item_unlit(&item, pose, full, &mut draw.vertices);
        } else {
            self.submit_item_world(&item, pose, draw.light, &mut draw.vertices);
        }
        true
    }

    /// Property `key` of block state `state` (a machine's `facing`, from the
    /// `state` `native_block_entity_submit` gets), if it has one: no lookup in
    /// the world, so cheap each frame.
    pub(in crate::game) fn block_state_property(
        &self,
        state: BlockStateId,
        key: &str,
    ) -> Option<&str> {
        self.common
            .pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .map(String::as_str)
    }
    /// The block `state` is a state of: its name, as `GameLevel`'s helper of
    /// the same name gives it for the hooks that get a state in the world.
    pub(in crate::game::client_side) fn native_state_block(
        &self,
        state: BlockStateId,
    ) -> Option<&str> {
        self.common
            .pack
            .assets
            .registry
            .state(state)
            .map(|s| &*s.name)
    }
}

/// `ModelPart.render`: `part`'s faces under its own pose
/// (`translateAndRotate`), then its children under theirs.
fn draw_model_part(draw: &mut BlockEntityDraw, part: &ModelPart, sprite: Rect, tint: u32) {
    if !part.visible || (part.is_empty() && part.children().next().is_none()) {
        return;
    }
    draw.push();
    draw.apply(part_transform(part));
    if !part.skip_draw {
        for polygon in &part.polygons {
            let corners = polygon.vertices.map(|v| v.pos.map(|c| c / 16.0));
            let uv = polygon.vertices.map(|v| [v.u, v.v]);
            draw.emit_quad(corners, uv, sprite, tint, draw.light);
        }
    }
    for (_, child) in part.children() {
        draw_model_part(draw, child, sprite, tint);
    }
    draw.pop();
}

/// `ModelPart.translateAndRotate`: to the part's place in blocks, turned
/// about z, then y, then x (`rotationZYX`), then scaled.
fn part_transform(part: &ModelPart) -> Mat4 {
    let mut m = Mat4::from_translation(Vec3::new(part.x, part.y, part.z) / 16.0);
    if part.x_rot != 0.0 || part.y_rot != 0.0 || part.z_rot != 0.0 {
        m *= Mat4::from_quat(Quat::from_euler(
            glam::EulerRot::ZYX,
            part.z_rot,
            part.y_rot,
            part.x_rot,
        ));
    }
    if part.x_scale != 1.0 || part.y_scale != 1.0 || part.z_scale != 1.0 {
        m *= Mat4::from_scale(Vec3::new(part.x_scale, part.y_scale, part.z_scale));
    }
    m
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use sunrise_level::containers::ModBlockEntity;
    #[cfg(test)]
    use sunrise_level::containers::ModBlockEntityFields;

    pub fn sprite() -> Rect {
        Rect {
            u0: 0.5,
            v0: 0.25,
            u1: 0.75,
            v1: 0.5,
            w: 16,
            h: 16,
        }
    }
    pub fn marker(pos: f32) -> Vertex {
        Vertex {
            pos: [pos, 0.0, 0.0],
            uv: [0.0, 0.0],
            layer: 0,
            color: 0,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0, 0.0],
        }
    }
    pub fn rows(m: Mat4) -> [f32; 12] {
        let c = m.to_cols_array();
        [
            c[0], c[4], c[8], c[12], c[1], c[5], c[9], c[13], c[2], c[6], c[10], c[14],
        ]
    }
    #[cfg(test)]
    /// A frustum that sees nothing.
    struct Blind;
    #[cfg(test)]
    impl Cull for Blind {
        fn intersects(&self, _: Vec3, _: Vec3) -> bool {
            false
        }
        fn contains(&self, _: Vec3, _: Vec3) -> bool {
            false
        }
    }
    pub fn machine() -> ModRevision {
        ModBlockEntity::new("mymod:machine".into(), Default::default()).revision()
    }
    pub fn vertex(x: f32) -> Vertex {
        Vertex {
            pos: [x, 0.0, 0.0],
            ..bytemuck::Zeroable::zeroed()
        }
    }
    pub const LIGHT_QUAD: [[f32; 3]; 4] = [
        [0.0, 1.0, 0.5],
        [0.0, 0.0, 0.5],
        [1.0, 0.0, 0.5],
        [1.0, 1.0, 0.5],
    ];

    pub fn draw() -> BlockEntityDraw<'static> {
        BlockEntityDraw::posed(
            0.0,
            0,
            0xF0F0,
            [0; 3],
            Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)),
            diffuse_lights(false),
            Vec::new(),
        )
    }

    #[cfg(test)]
    /// A quad standing still has its light baked into its colour, as before; one drawn under
    /// `spin` keeps its plain tint and its outward normal (`w = 127`, the shader shades it), and
    /// once turned its normal points where the face does: up turned a quarter about +x faces +z.
    #[test]
    fn a_turning_quad_is_shaded_where_it_turns_to() {
        let mut draw = draw();
        let top = [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ];
        let tint = 0x80_40_20;
        draw.quad(top, [[0.0; 2]; 4], sprite(), tint);
        draw.spin([1.0, 0.0, 0.0], [0.5; 3], 90.0, 0.0);
        draw.quad(top, [[0.0; 2]; 4], sprite(), tint);
        let (still, turning) = draw.vertices.split_at(4);
        let lit = shaded(tint, diffuse(Vec3::Y, draw.lights));
        assert!(still.iter().all(|v| v.normal == [0; 4] && v.color == lit));
        assert!(
            turning
                .iter()
                .all(|v| v.normal == [0, 127, 0, 127] && v.color == shaded(tint, 1.0))
        );
        let out = draw.animated(1.0).into_placed(Vec3::ZERO);
        assert!(out[..4].iter().all(|v| v.normal == [0; 4]));
        assert!(out[4..].iter().all(|v| v.normal == [0, 0, 127, 127]));
    }

    #[cfg(test)]
    #[test]
    fn non_finite_quad_submission_leaves_geometry_unchanged() {
        let mut surface = draw();
        let corners = [
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let uv = [[0.0; 2]; 4];
        surface.quad(corners, uv, sprite(), 0xFFFFFF);
        assert_eq!(surface.vertices.len(), 4);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut bad_corners = corners;
            bad_corners[1][2] = bad;
            surface.quad(bad_corners, uv, sprite(), 0xFFFFFF);
            let mut bad_uv = uv;
            bad_uv[2][1] = bad;
            surface.quad(corners, bad_uv, sprite(), 0xFFFFFF);
        }
        assert_eq!(surface.into_vertices().len(), 4);
    }

    #[cfg(test)]
    #[test]
    fn an_overfull_draw_keeps_earlier_entity_vertices_and_offsets() {
        use sunrise_client::gfx::EntityQuadCapacity;

        let capacity =
            EntityQuadCapacity::from_max_buffer_size(EntityQuadCapacity::VERTEX_QUAD_BYTES * 2);
        let mut draw = BlockEntityDraw::posed(
            0.0,
            0,
            0,
            [0; 3],
            Mat4::IDENTITY,
            diffuse_lights(false),
            [marker(1.0); 8].to_vec(),
        )
        .with_quad_budget(capacity, 1);
        draw.vertices.extend([marker(2.0); 4]);
        let out = draw.into_vertices();
        assert_eq!(out.len(), 8);
        assert!(out.iter().all(|vertex| vertex.pos[0] == 1.0));
    }

    #[cfg(test)]
    #[test]
    fn prepared_faces_transform_and_preserve_tint_and_emission() {
        use crate::game::client_side::minecarts_client::{PreparedModelFace, emit_model_faces};
        let mut quad = bake_flat_face(
            [0.0; 3],
            [16.0; 3],
            [0.0, 0.0, 16.0, 16.0],
            Direction::Up,
            Material {
                sprite: String::new(),
                force_translucent: false,
            },
            0,
        )
        .unwrap();
        quad.positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        quad.uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        quad.light_emission = 12;
        let tinted = PreparedModelFace::new(&quad, sprite());
        quad.tint_index = -1;
        let plain = PreparedModelFace::new(&quad, sprite());
        let pose = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 3.0, 4.0),
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(10.0, 20.0, 30.0),
        );
        let mut out = Vec::new();
        emit_model_faces(
            [(tinted, 0x2030), (plain, 0x2030)],
            pose,
            true,
            |index| (index >= 0).then_some(0x123456),
            &mut out,
        );
        let positions = [
            [10.0, 20.0, 30.0],
            [10.0, 22.0, 30.0],
            [7.0, 22.0, 30.0],
            [7.0, 20.0, 30.0],
        ];
        let uvs = [[0.5, 0.25], [0.75, 0.25], [0.75, 0.5], [0.5, 0.5]];
        for (i, v) in out.iter().enumerate() {
            for axis in 0..3 {
                assert!((v.pos[axis] - positions[i % 4][axis]).abs() < 0.00001);
            }
            assert_eq!(v.uv, uvs[i % 4]);
            assert_eq!(v.normal, [-127, 0, 0, 127]);
            assert_eq!(v.light, 0xC030);
            assert_eq!(v.color, if i < 4 { 0xFF563412 } else { u32::MAX });
        }
        assert_eq!(out.len(), 8);
    }

    #[cfg(test)]
    #[test]
    fn prepared_route_stays_cpu_for_either_shader_pack() {
        assert!(super::prepared_route(false, false, Some(4), 4));
        assert!(!super::prepared_route(true, false, Some(4), 4), "iris pack");
        assert!(
            !super::prepared_route(false, true, Some(4), 4),
            "sunrise pack"
        );
        assert!(!super::prepared_route(false, false, None, 4));
        assert!(!super::prepared_route(false, false, Some(3), 4));
    }

    #[cfg(test)]
    #[test]
    fn prepared_models_stay_ordered_and_cpu_inside_the_block_span() {
        use crate::game::client_side::minecarts_client::{PreparedModelFace, emit_model_faces};
        let mut quad = bake_flat_face(
            [0.0; 3],
            [16.0; 3],
            [0.0, 0.0, 16.0, 16.0],
            Direction::South,
            Material {
                sprite: String::new(),
                force_translucent: false,
            },
            0,
        )
        .unwrap();
        quad.positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        quad.uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let face = PreparedModelFace::new(&quad, sprite());
        let slots = vec![Some(vec![face])];
        let mut gpu = draw();
        gpu.prepared = true;
        gpu.light = 0xF030;
        gpu.vertices.extend((0..4).map(|i| marker(i as f32)));
        let span = gpu.vertices.len();
        assert!(gpu.defer_model(0, 3, 0xFF80_C040));
        gpu.vertices.extend((0..4).map(|i| marker(10.0 + i as f32)));
        let origin = DVec3::new(8_000_000.5, 70.0, -4_000_000.25);
        let camera = DVec3::new(8_000_000.0, 70.2, -4_000_000.0);
        let mut instances = Vec::new();
        gpu.push_instances(origin, camera, &mut instances);
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].cpu_vertices, span as u32);
        assert_eq!(gpu.vertices.len(), 8, "a prepared copy is not in the span");
        assert!((0..4).all(|i| gpu.vertices[i].pos[0] < 4.0));
        assert!((4..8).all(|i| gpu.vertices[i].pos[0] >= 10.0));

        let mut cpu = draw();
        cpu.prepared = false;
        assert!(
            !cpu.defer_model(0, 3, 0xFF80_C040),
            "packs, gui and mobs stay on the cpu"
        );
        cpu.prepared = true;
        cpu.vertices.extend((0..4).map(|i| marker(i as f32)));
        assert!(cpu.defer_model(0, 3, 0xFF80_C040));
        let pose = cpu.records[0].pose;
        cpu.spin([0.0, 1.0, 0.0], [0.5; 3], 12.0, 0.0);
        cpu.bake_recorded(&slots);
        let mut expected = Vec::new();
        emit_model_faces(
            [(face, cpu.light)],
            pose,
            true,
            |index| (index >= 0).then_some(0xFF80_C040),
            &mut expected,
        );
        assert_eq!(cpu.vertices.len(), 4 + expected.len());
        assert_eq!(cpu.spins[0].0, 4 + expected.len());
        for (got, want) in cpu.vertices[4..].iter().zip(&expected) {
            assert_eq!(got.pos, want.pos);
            assert_eq!(got.uv, want.uv);
            assert_eq!(got.color, want.color);
            assert_eq!(got.light, want.light);
            assert_eq!(got.normal, want.normal);
        }
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn prepared_model_entity_pixels() {
        use mod_api::__private::game::Fields;
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let model = game
                .render_assets()
                .model(&Identifier::parse("minecraft:block/grass_block").unwrap())
                .unwrap();
            let prepared = game.client.standalone_geometry.slots[model.fields().slot as usize]
                .as_ref()
                .unwrap();
            let face_index = prepared
                .iter()
                .position(|face| face.facing == Direction::South && face.tint_index >= 0)
                .unwrap();
            let mut surface = draw();
            surface.pose = Mat4::from_rotation_z(0.25);
            surface.light = 0x7030;
            game.native_draw_model(&mut surface, model, 0xFF80_C040)
                .unwrap();
            let vertices = &surface.vertices[face_index * 4..face_index * 4 + 4];
            assert!(vertices.iter().all(|v| v.light == 0x7030));
            let atlas = &game.client.gui.atlas;
            let mut pixels = vec![255; (atlas.width * atlas.height * 4) as usize];
            // Prepared indices exclude baked faces without an atlas sprite.
            let source = game.client.client_pack.render.resolve_model(model).unwrap();
            let sprite = source
                .faces
                .iter()
                .filter_map(|face| {
                    game.client
                        .client_pack
                        .layer_names
                        .get(&face.layer)
                        .and_then(|name| atlas.sprites.get(name))
                        .copied()
                })
                .nth(face_index)
                .unwrap();
            let x0 = (sprite.u0 * atlas.width as f32).round() as u32;
            let x1 = (sprite.u1 * atlas.width as f32).round() as u32;
            let y0 = (sprite.v0 * atlas.height as f32).round() as u32;
            let y1 = (sprite.v1 * atlas.height as f32).round() as u32;
            for y in y0..y1 {
                for x in x0..(x0 + x1) / 2 {
                    pixels[((y * atlas.width + x) * 4 + 3) as usize] = 0;
                }
            }
            let view = Mat4::from_scale_rotation_translation(
                Vec3::new(1.4, 1.4, 0.2),
                Quat::IDENTITY,
                Vec3::new(-0.55, -0.85, 0.5),
            );
            let rendered = sunrise_client::gfx::entity_pixel_test::render(
                vertices,
                [atlas.width, atlas.height],
                &pixels,
                view,
                false,
            )
            .expect("software Vulkan entity pixels");
            let coverage = rendered
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
                .count();
            let pixel = |x: usize, y: usize| &rendered[(y * 64 + x) * 4..(y * 64 + x + 1) * 4];
            assert_eq!(pixel(32, 32), [48, 107, 47, 255]);
            assert_eq!(pixel(0, 0), [0, 0, 0, 255]);
            assert!((950..1050).contains(&coverage), "coverage was {coverage}");
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn a_full_instance_buffer_bakes_the_tail_after_the_cut() {
        use crate::game::client_side::minecarts_client::PreparedModelFace;
        use sunrise_client::gfx::cap_prepared_instance_slots;
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                cap_prepared_instance_slots(u64::MAX);
            }
        }
        let _reset = Reset;
        let mut quad = bake_flat_face(
            [0.0; 3],
            [0.0; 3],
            [0.0; 4],
            Direction::South,
            Material {
                sprite: String::new(),
                force_translucent: false,
            },
            0,
        )
        .unwrap();
        quad.positions = [[0.0; 3]; 4];
        let face = PreparedModelFace::new(&quad, sprite());
        // kind 0 is an ordinary quad at x; kind 1 is a prepared copy (x, tint, light).
        let scene = [
            (0, 1.0, 0, 0),                 // A
            (1, 10.0, 0xFF01_0203, 0x1110), // B
            (1, 20.0, 0xFF04_0506, 0x2220), // C, same cut as B
            (0, 2.0, 0, 0),                 // D
            (1, 30.0, 0xFF07_0809, 0x3330), // E
            (0, 3.0, 0, 0),                 // F
        ];
        let split = [
            (0, 1.0, 0, 0),
            (1, 10.0, 0xFF01_0203, 0x1110),
            (1, 20.0, 0xFF04_0506, 0x2220),
            (1, 40.0, 0xFF0A_0B0C, 0x4440),
            (0, 2.0, 0, 0),
        ];
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.client.standalone_geometry.slots = vec![Some(vec![face])];
            let play = |cap, steps: &[_]| {
                cap_prepared_instance_slots(cap);
                let mut draw = draw();
                draw.prepared = true;
                for (kind, x, tint, light) in steps {
                    if *kind == 0 {
                        draw.vertices.extend(std::iter::repeat_n(marker(*x), 4));
                    } else {
                        draw.light = *light;
                        draw.pose = Mat4::from_translation(Vec3::new(*x, 0.0, 0.0));
                        assert!(draw.defer_model(0, 1, *tint));
                    }
                }
                game.client.prepared_model_draws.borrow_mut().clear();
                game.finish_prepared_draw(&mut draw, DVec3::ZERO, DVec3::ZERO);
                assert!(draw.records.is_empty());
                draw
            };
            let xs = |d: &BlockEntityDraw| {
                d.vertices
                    .iter()
                    .step_by(4)
                    .map(|v| v.pos[0])
                    .collect::<Vec<_>>()
            };
            let paint = |d: &BlockEntityDraw, i: usize, color: u32, light: u32| {
                assert_eq!((d.vertices[i].color, d.vertices[i].light), (color, light));
            };
            let cuts = |game: &crate::game::ClientGame<'_>| {
                game.client
                    .prepared_model_draws
                    .borrow()
                    .iter()
                    .map(|c| c.cpu_vertices)
                    .collect::<Vec<_>>()
            };
            let same = |got: Vec<f32>, want: &[f32]| assert_eq!(got.as_slice(), want);
            let staged = |game: &crate::game::ClientGame<'_>, want: &[u32]| {
                assert_eq!(cuts(game).as_slice(), want)
            };
            let draw = play(1, &scene);
            same(xs(&draw), &[1.0, 20.0, 2.0, 30.0, 3.0]);
            paint(&draw, 4, 0xFF06_0504, 0x2220);
            paint(&draw, 12, 0xFF09_0807, 0x3330);
            staged(game, &[4]);
            assert_eq!(
                game.client.prepared_model_draws.borrow()[0]
                    .instance
                    .matrix()
                    .w_axis
                    .x,
                10.0
            );
            same(xs(&play(0, &scene)), &[1.0, 10.0, 20.0, 2.0, 30.0, 3.0]);
            assert!(cuts(game).is_empty());
            same(xs(&play(3, &scene)), &[1.0, 2.0, 3.0]);
            staged(game, &[4, 4, 8]);
            let over = play(2, &scene);
            same(xs(&over), &[1.0, 2.0, 30.0, 3.0]);
            paint(&over, 8, 0xFF09_0807, 0x3330);
            staged(game, &[4, 4]);
            let tail = play(1, &split);
            same(xs(&tail), &[1.0, 20.0, 40.0, 2.0]);
            paint(&tail, 4, 0xFF06_0504, 0x2220);
            paint(&tail, 8, 0xFF0C_0B0A, 0x4440);
            staged(game, &[4]);
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn prepared_instance_matches_cpu_pixels() {
        use glam::DVec3;
        use mod_api::__private::game::Fields;
        use sunrise_client::gfx::fail_next_prepared_upload;
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let model = game
                .render_assets()
                .model(&Identifier::parse("minecraft:block/grass_block").unwrap())
                .unwrap();
            let slot = model.fields().slot;
            let local = Mat4::from_scale_rotation_translation(
                Vec3::new(1.2, 0.5, 1.7),
                Quat::from_rotation_z(0.25),
                Vec3::ZERO,
            );
            let origin = DVec3::new(8_000_000.5, 70.0, -4_000_000.25);
            let camera = DVec3::new(8_000_000.0, 70.2, -4_000_000.0);
            let pose = sunrise_client::gfx::camera_matrix(local, origin, camera);
            let mut surface = draw();
            surface.pose = pose;
            surface.light = 0xF030;
            game.native_draw_model(&mut surface, model, 0xFF80_C040)
                .unwrap();
            let instance = super::prepared_instance(local, origin, camera, 0xFF80_C040, 0xF030);
            assert_eq!(instance.matrix(), pose);
            let (vertices, ranges) = game.client.standalone_geometry.prepared_upload();
            let atlas = &game.client.gui.atlas;
            let mut pixels = atlas.pixels().to_vec();
            let face_index = game.client.standalone_geometry.slots[slot as usize]
                .as_ref()
                .unwrap()
                .iter()
                .position(|face| face.facing == Direction::South && face.tint_index >= 0)
                .unwrap();
            let sprite = game
                .client
                .client_pack
                .render
                .resolve_model(model)
                .unwrap()
                .faces
                .iter()
                .filter_map(|face| {
                    game.client
                        .client_pack
                        .layer_names
                        .get(&face.layer)
                        .and_then(|name| atlas.sprites.get(name))
                        .copied()
                })
                .nth(face_index)
                .unwrap();
            let x0 = (sprite.u0 * atlas.width as f32).round() as u32;
            let x1 = (sprite.u1 * atlas.width as f32).round() as u32;
            let y0 = (sprite.v0 * atlas.height as f32).round() as u32;
            let y1 = (sprite.v1 * atlas.height as f32).round() as u32;
            for y in y0..y1 {
                for x in x0..(x0 + x1) / 2 {
                    pixels[((y * atlas.width + x) * 4 + 3) as usize] = 0;
                }
            }
            let view = Mat4::from_scale_rotation_translation(
                Vec3::new(1.4, 1.4, 0.2),
                Quat::IDENTITY,
                Vec3::new(-0.55, -0.85, 0.5),
            );
            let cpu = sunrise_client::gfx::entity_pixel_test::render(
                &surface.vertices,
                [atlas.width, atlas.height],
                &pixels,
                view,
                true,
            )
            .expect("software Vulkan entity pixels");
            let uploaded = game.client.standalone_geometry.generation();
            let instance_pixels = |ranges, requested| {
                sunrise_client::gfx::entity_pixel_test::render_instance(
                    sunrise_client::gfx::entity_pixel_test::PreparedInstanceDraw {
                        vertices: &vertices,
                        ranges,
                        uploaded,
                        requested,
                        slot,
                        instances: std::slice::from_ref(&instance),
                        per_copy: false,
                        hdr: true,
                        atlas_size: [atlas.width, atlas.height],
                        atlas: &pixels,
                        view_proj: view,
                    },
                )
            };
            let gpu = instance_pixels(ranges, uploaded).expect("software Vulkan prepared instance");
            let mut worst = 0u8;
            for (cpu, gpu) in cpu.iter().zip(gpu.iter()) {
                worst = worst.max(cpu.abs_diff(*gpu));
            }
            assert!(worst <= 2, "cpu and instance pixels differed by {worst}");
            // The HDR mark uses bias 0 on both routes, so it does not land on the
            // colour pass's biased depth. Coverage is the colour, not a mid alpha.
            assert!(
                cpu.as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
            );
            fail_next_prepared_upload();
            assert!(
                instance_pixels(
                    game.client.standalone_geometry.prepared_upload().1,
                    uploaded
                )
                .is_err(),
                "a forced upload failure must not draw"
            );
            assert!(
                instance_pixels(
                    game.client.standalone_geometry.prepared_upload().1,
                    uploaded.wrapping_add(1),
                )
                .is_err(),
                "a stale generation must not draw"
            );
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn resolved_model_draws_and_resource_replacement_invalidates_it() {
        use crate::game::host::draw::GameBlockEntityDrawContext;
        use mod_api::BlockEntityDrawContext;

        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let id = Identifier::parse("minecraft:block/grass_block").unwrap();
            let model = game.render_assets().model(&id).unwrap();
            assert_eq!(game.render_assets().model(&id), Ok(model));
            assert_eq!(
                game.render_assets()
                    .model(&Identifier::parse("mymod:block/absent").unwrap()),
                Err(ModelError::UnknownModel)
            );
            let mut surface = draw();
            surface.pose = Mat4::from_scale_rotation_translation(
                Vec3::new(2.0, 0.5, 1.5),
                Quat::from_rotation_y(0.7) * Quat::from_rotation_x(0.3),
                Vec3::new(8.0, -4.0, 3.0),
            );
            surface.light = 0x7030;
            // Compare the consumer with the existing shared emitter, under the same pose,
            // tint, atlas and light: handle resolution must not change any vertex bytes.
            let mut expected = Vec::new();
            game.baked_model_vertices(
                [game
                    .client
                    .client_pack
                    .render
                    .model_catalog
                    .standalone("minecraft:block/grass_block")
                    .unwrap()],
                surface.pose(),
                |_| surface.light,
                true,
                |index| (index >= 0).then_some(0xFF12_3456),
                &mut expected,
            );
            assert!(!expected.is_empty());
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            assert_eq!(drawn.model(model, 0xFF12_3456), Ok(()));
            let bytes = |vertices: &[Vertex]| bytemuck::cast_slice::<_, u8>(vertices).to_vec();
            assert_eq!(bytes(&surface.vertices), bytes(&expected));
            assert!(surface.vertices.iter().all(|v| v.light == 0x7030));
            // An explicitly emissive tinted face and its untinted twin exercise both
            // branches independently of the vanilla grass model's current contents.
            let source = game.client.client_pack.render.resolve_model(model).unwrap();
            let face = &source.faces[0];
            let sprite = game
                .client
                .client_pack
                .layer_names
                .get(&face.layer)
                .and_then(|name| game.client.gui.atlas.sprites.get(name))
                .copied()
                .unwrap();
            let mut tinted = face.quad.clone();
            tinted.tint_index = 0;
            tinted.light_emission = 12;
            let mut plain = tinted.clone();
            plain.tint_index = -1;
            let mut emitted = Vec::new();
            crate::game::client_side::minecarts_client::emit_model_faces(
                [
                    (
                        crate::game::client_side::minecarts_client::PreparedModelFace::new(
                            &tinted, sprite,
                        ),
                        0x2030,
                    ),
                    (
                        crate::game::client_side::minecarts_client::PreparedModelFace::new(
                            &plain, sprite,
                        ),
                        0x2030,
                    ),
                ],
                Mat4::IDENTITY,
                true,
                |index| (index >= 0).then_some(0x123456),
                &mut emitted,
            );
            assert_eq!(emitted.len(), 8);
            assert!(emitted.iter().all(|v| v.light == 0xC030));
            assert!(emitted[..4].iter().all(|v| v.color == 0xFF563412));
            assert!(emitted[4..].iter().all(|v| v.color == u32::MAX));

            // Same content and emitter on each side; this times CPU work only.
            let mut scratch = Vec::with_capacity(expected.len());
            let rounds = 20_000;
            let start = std::time::Instant::now();
            for _ in 0..rounds {
                scratch.clear();
                game.baked_model_vertices(
                    [source],
                    surface.pose(),
                    |_| surface.light,
                    true,
                    |index| (index >= 0).then_some(0x123456),
                    &mut scratch,
                );
                std::hint::black_box(&scratch);
            }
            let before_time = start.elapsed();
            let start = std::time::Instant::now();
            for _ in 0..rounds {
                surface.vertices.clear();
                game.native_draw_model(&mut surface, model, 0xFF12_3456)
                    .unwrap();
                std::hint::black_box(&surface.vertices);
            }
            eprintln!(
                "CPU model {rounds} draws: lookup {before_time:?}, prepared {:?}",
                start.elapsed()
            );
            let before = bytes(&surface.vertices);
            let invalid = mod_api::__private::built::model_ref(
                game.client.client_pack.render.generation,
                u32::MAX,
            );
            assert_eq!(
                game.native_draw_model(&mut surface, invalid, 0xFFFF_FFFF),
                Err(ModelError::UnknownModel)
            );
            assert_eq!(bytes(&surface.vertices), before);

            // Even a nested model decode failure must leave the live generations intact.
            let active_layers = [
                "minecraft:leash_knot#main",
                "minecraft:standing_banner#main",
                "minecraft:standing_banner#flag",
                "minecraft:chest#main",
            ]
            .map(|name| (name, game.bake_layer(name).unwrap()));
            let bad_root = tempfile::tempdir().unwrap();
            let path = bad_root.path().join("entity-models.json");
            for (document, reason) in [
                ("[]", "top-level JSON object"),
                ("{}", "no entity model definitions"),
                (
                    r#"{"minecraft:chest#main": {"children": {"lid": {
                        "polygons": [[[0,0,0,0], [1,0,0,1,0], [1,1,0,1,1], [0,1,0,0,1]]]
                    }}}}"#,
                    "layer minecraft:chest#main: child lid: polygons",
                ),
            ] {
                std::fs::write(&path, document).unwrap();
                let listeners = game.register_client_reload_listeners(
                    &game
                        .capture_candidate_settings(&game.options.resources)
                        .unwrap(),
                );
                let error = super::super::packs::stage_pack(
                    bad_root.path(),
                    &[],
                    &[],
                    &game.mods,
                    listeners,
                    &[],
                    &game.pack.assets,
                    &game.res,
                    game.client_res(),
                    &game.client.client_pack.context,
                    false,
                    false,
                    &super::super::Progress::new(),
                )
                .err()
                .expect("a malformed definition set must fail staging");
                let error = error.to_string();
                assert!(
                    error.contains("loading vanilla entity model definitions"),
                    "{error}"
                );
                assert!(error.contains(reason), "{error}");
                for (name, layer) in &active_layers {
                    assert_eq!(game.bake_layer(name).as_ref(), Some(layer), "{name}");
                }
                assert!(game.client.entity_models.leash_knot.is_some());
                assert!(game.client.entity_models.banner.is_some());
            }
            assert_eq!(
                game.native_draw_model(&mut surface, model, 0xFF12_3456),
                Ok(())
            );
            let before = bytes(&surface.vertices);
            // Identical texture bytes still produce a new model generation.
            let old_generation = game.client.client_pack.render.generation;
            let listeners = game.register_client_reload_listeners(
                &game
                    .capture_candidate_settings(&game.options.resources)
                    .unwrap(),
            );
            let staged = super::super::packs::stage_pack(
                &game.options.resources,
                &[],
                &[],
                &game.mods,
                listeners,
                &[],
                &game.pack.assets,
                &game.res,
                game.client_res(),
                &game.client.client_pack.context,
                false,
                false,
                &super::super::Progress::new(),
            )
            .unwrap();
            game.apply_staged_pack(staged).unwrap();
            assert_ne!(game.client.client_pack.render.generation, old_generation);
            assert_eq!(
                game.native_draw_model(&mut surface, model, 0xFF12_3456),
                Err(ModelError::StaleModel)
            );
            assert_eq!(bytes(&surface.vertices), before);
            let fresh = game.render_assets().model(&id).unwrap();
            assert_ne!(fresh, model);
            assert_eq!(
                game.native_draw_model(&mut surface, fresh, 0xFF12_3456),
                Ok(())
            );
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn model_tint_alpha_agrees_on_keep_deferred_and_batch() {
        use crate::game::host::draw::{GameBlockEntityDrawContext, GameWorldDrawContext};
        use mod_api::{
            BlockEntityDrawContext, Identifier, LightCoords, ModelInstance, Overlay,
            WorldDrawContext,
        };
        use sunrise_client::gfx::entity_pixel_test::{self, PreparedInstanceDraw};
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                sunrise_client::gfx::cap_prepared_instance_slots(u64::MAX);
                sunrise_client::gfx::set_test_prepared_generation(None);
            }
        }
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let _restore = Restore;
            let id = Identifier::parse("minecraft:block/grass_block").unwrap();
            let model = game.render_assets().model(&id).unwrap();
            let generation = mod_api::__private::game::Fields::fields(&model).generation;
            sunrise_client::gfx::set_test_prepared_generation(Some(generation));
            sunrise_client::gfx::cap_prepared_instance_slots(8);
            let local = Mat4::from_scale_rotation_translation(
                Vec3::new(0.85, 1.15, 0.7),
                Quat::from_rotation_y(0.6),
                Vec3::new(0.12, -0.05, 0.08),
            );
            let origin = DVec3::new(8_000_000.5, 70.0, -4_000_000.25);
            let camera = DVec3::new(8_000_000.0, 70.2, -4_000_000.0);
            let tint = 0x40FF_3040;
            let mut spun = draw();
            spun.prepared = true;
            spun.light = 0xF0F0;
            spun.pose = local;
            spun.spin([0.0, 1.0, 0.0], [0.5; 3], 0.0, 0.0);
            {
                let mut ctx = GameBlockEntityDrawContext {
                    game,
                    surface: &mut spun,
                };
                ctx.model(model, tint).unwrap();
            }
            let mut kept = draw();
            kept.prepared = true;
            kept.light = 0xF0F0;
            kept.pose = local;
            {
                let mut ctx = GameBlockEntityDrawContext {
                    game,
                    surface: &mut kept,
                };
                ctx.model(model, tint).unwrap();
                ctx.keep();
            }
            game.finish_prepared_draw(&mut kept, origin, camera);
            assert_eq!(spun.vertices.len(), kept.vertices.len());
            for (spun, kept) in spun.vertices.iter().zip(kept.vertices.iter()) {
                assert_eq!(spun.color, kept.color, "spin and keep tint alpha differ");
            }
            let slot = mod_api::__private::game::Fields::fields(&model).slot;
            let faces = game.client.standalone_geometry.faces(slot).unwrap();
            let (quads, _) = kept.vertices.as_chunks::<4>();
            assert!(
                quads
                    .iter()
                    .zip(faces)
                    .any(|(quad, face)| face.tint_index < 0 && quad[0].color >> 24 == 255),
                "an untinted face stays opaque"
            );
            assert!(
                quads
                    .iter()
                    .zip(faces)
                    .any(|(quad, face)| face.tint_index >= 0 && quad[0].color >> 24 == 0x40),
                "a tinted face takes the tint alpha"
            );
            let shift = (origin - camera).as_vec3();
            for vertex in &mut kept.vertices {
                vertex.pos = (Vec3::from(vertex.pos) + shift).to_array();
            }
            let mut deferred = draw();
            deferred.prepared = true;
            deferred.light = 0xF0F0;
            deferred.pose = local;
            {
                let mut ctx = GameBlockEntityDrawContext {
                    game,
                    surface: &mut deferred,
                };
                ctx.model(model, tint).unwrap();
            }
            game.client.prepared_model_draws.borrow_mut().clear();
            game.finish_prepared_draw(&mut deferred, origin, camera);
            let deferred_instance = game.client.prepared_model_draws.borrow()[0].instance;
            game.client.prepared_model_draws.borrow_mut().clear();
            let mut batch = crate::world_geometry::WorldGeometry::default();
            batch.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
            let copy = ModelInstance::new(
                origin.to_array(),
                rows(local),
                tint,
                LightCoords::FULL_BRIGHT,
                Overlay::NONE,
            );
            {
                let mut ctx = GameWorldDrawContext {
                    game,
                    surface: &mut batch,
                };
                ctx.draw_model_instances(model, &[copy]).unwrap();
            }
            let mut entities = Vec::new();
            game.commit_model_instances(&mut batch, camera, &mut entities);
            let batch_instance = game.client.prepared_model_draws.borrow()[0].instance;
            let alpha = 0x40 as f32 / 255.0;
            assert!((deferred_instance.tint[3] - alpha).abs() < 1e-6);
            assert!((batch_instance.tint[3] - alpha).abs() < 1e-6);
            let atlas = &game.client.gui.atlas;
            let pixels = atlas.pixels();
            let uploaded = game.client.standalone_geometry.generation();
            let (vertices, ranges) = game.client.standalone_geometry.prepared_upload();
            let view = Mat4::from_scale_rotation_translation(
                Vec3::new(1.4, 1.4, 0.2),
                Quat::IDENTITY,
                Vec3::new(-0.55, -0.85, 0.5),
            );
            let cpu = entity_pixel_test::render(
                &kept.vertices,
                [atlas.width, atlas.height],
                &pixels,
                view,
                true,
            )
            .expect("keep pixels");
            let gpu = |instance: &sunrise_client::gfx::PreparedModelInstance| {
                entity_pixel_test::render_instance(PreparedInstanceDraw {
                    vertices: &vertices,
                    ranges: ranges.clone(),
                    uploaded,
                    requested: uploaded,
                    slot,
                    instances: std::slice::from_ref(instance),
                    per_copy: false,
                    hdr: true,
                    atlas_size: [atlas.width, atlas.height],
                    atlas: &pixels,
                    view_proj: view,
                })
                .expect("instance pixels")
            };
            let deferred_pixels = gpu(&deferred_instance);
            let batch_pixels = gpu(&batch_instance);
            let worst = |a: &[u8], b: &[u8]| {
                a.iter()
                    .zip(b)
                    .fold(0u8, |worst, (p, q)| worst.max(p.abs_diff(*q)))
            };
            assert_eq!(worst(&cpu, &deferred_pixels), 0, "keep and deferred differ");
            assert_eq!(worst(&cpu, &batch_pixels), 0, "keep and batch differ");
        });
    }

    #[cfg(test)]
    /// Under a `shaders/` pack, a batch copy and the per-copy `model()` route draw the same
    /// vertices in a block-entity run, so our probe pack (one flat colour per program) paints
    /// both with `gbuffers_block`. Without a pack the batch marks no run and bakes the same.
    #[test]
    #[ignore = "daily"]
    fn a_pack_shades_batch_and_per_copy_models_with_one_program() {
        use crate::game::host::draw::{GameBlockEntityDrawContext, GameWorldDrawContext};
        use mod_api::{BlockEntityDrawContext, LightCoords, ModelInstance, Overlay};
        use mod_api::{Identifier, WorldDrawContext};
        use sunrise_client::gfx::entity_pixel_test::{self, PACK_COLOURS};
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let id = Identifier::parse("minecraft:block/grass_block").unwrap();
            let model = game.render_assets().model(&id).unwrap();
            let local = Mat4::from_scale_rotation_translation(
                Vec3::new(0.85, 1.15, 0.7),
                Quat::from_rotation_y(0.6),
                Vec3::new(0.12, -0.05, 0.08),
            );
            let corner = DVec3::new(8_000_000.0, 70.0, -4_000_001.0);
            let camera = DVec3::new(8_000_000.3, 70.4, -4_000_003.0);
            let tint = 0xFF80_C040;
            let light = LightCoords::new(3, 15).unwrap();
            let take_spans = |game: &crate::game::ClientGame<'_>| {
                std::mem::take(&mut *game.client.entity_spans.borrow_mut())
            };
            let batch = |game: &mut crate::game::ClientGame<'_>| {
                let mut draw = crate::world_geometry::WorldGeometry::default();
                draw.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
                let copy =
                    ModelInstance::new(corner.to_array(), rows(local), tint, light, Overlay::NONE);
                GameWorldDrawContext {
                    game,
                    surface: &mut draw,
                }
                .draw_model_instances(model, &[copy])
                .unwrap();
                let mut entities = Vec::new();
                game.commit_model_instances(&mut draw, camera, &mut entities);
                (entities, take_spans(game))
            };
            take_spans(game);
            let (plain, plain_spans) = batch(game);
            assert!(
                !plain.is_empty() && plain_spans.is_empty(),
                "no pack, no run"
            );

            game.client.pack_ids = Some(Default::default());
            // What `mod_block_entity_geometry` does around a mod's `model()` call.
            let mut per_copy = draw();
            per_copy.pose = local;
            per_copy.light = 0x30F0;
            game.mark_block_entity(0, 1);
            GameBlockEntityDrawContext {
                game,
                surface: &mut per_copy,
            }
            .model(model, tint)
            .unwrap();
            game.finish_prepared_draw(&mut per_copy, corner, camera);
            let shift = (corner - camera).as_vec3();
            for vertex in &mut per_copy.vertices {
                vertex.pos = (Vec3::from(vertex.pos) + shift).to_array();
            }
            let per_copy_spans = take_spans(game);
            let (copies, copy_spans) = batch(game);
            // Independent run-boundary expectations for equal and alternating overlays.
            for (overlays, expected) in [
                (vec![], vec![]),
                (vec![Overlay::NONE; 3], vec![0]),
                (
                    vec![Overlay::NONE, Overlay::new(0, true).unwrap(), Overlay::NONE],
                    vec![0, 1, 2],
                ),
            ] {
                let mut surface = crate::world_geometry::WorldGeometry::default();
                surface.reset(camera.to_array(), Mat4::IDENTITY, [0.0, 0.0]);
                let instances: Vec<_> = overlays
                    .into_iter()
                    .map(|overlay| {
                        ModelInstance::new(corner.to_array(), rows(local), tint, light, overlay)
                    })
                    .collect();
                GameWorldDrawContext {
                    game,
                    surface: &mut surface,
                }
                .draw_model_instances(model, &instances)
                .unwrap();
                let mut vertices = Vec::new();
                game.commit_model_instances(&mut surface, camera, &mut vertices);
                let spans = take_spans(game);
                assert_eq!(
                    spans.iter().map(|s| s.start as usize).collect::<Vec<_>>(),
                    expected
                        .into_iter()
                        .map(|i| i * copies.len())
                        .collect::<Vec<_>>()
                );
                assert!(spans.iter().all(
                    |s| s.id == 0 && s.stage == sunrise_shaders::render_stage("BLOCK_ENTITIES")
                ));
            }
            game.client.pack_ids = None;
            let bytes = |v: &[Vertex]| bytemuck::cast_slice::<Vertex, u8>(v).to_vec();
            assert_eq!(bytes(&copies), bytes(&per_copy.vertices), "route vertices");
            assert_eq!(bytes(&copies), bytes(&plain), "the pack changed the bake");
            let pixels = |vertices: &[Vertex], spans: &[_]| {
                entity_pixel_test::render_pack(vertices, spans).expect("software Vulkan pack")
            };
            let reference = pixels(&per_copy.vertices, &per_copy_spans);
            let painted: Vec<_> = reference.iter().filter(|p| p[..3] != [0; 3]).collect();
            assert!(
                painted.len() > 20,
                "the probe pack drew {} pixels",
                painted.len()
            );
            assert!(
                painted.iter().all(|p| **p == PACK_COLOURS[1]),
                "gbuffers_block"
            );
            let batch = pixels(&copies, &copy_spans);
            let differing = batch.iter().zip(&reference).filter(|(a, b)| a != b).count();
            assert_eq!(differing, 0, "batch pixels differ from per-copy ones");
        });
    }

    #[cfg(test)]
    /// Each mod block is asked once, whatever its number of states; vanilla blocks never are;
    /// an answer that isn't finite and above 0 keeps the default; the gather reaches the
    /// furthest.
    #[test]
    fn view_distances_are_asked_once_per_mod_block() {
        mod_platform::testing::with_mods(Default::default(), || {
            mod_api::__private::install_content_owners(
                [
                    ("minecraft:p3_beam", "mymod"),
                    ("mymod:screw", "mymod"),
                    ("mymod:gear", "mymod"),
                ],
                [],
                [],
                [],
            );
            let states = [
                (1, "minecraft:stone"),
                (2, "minecraft:p3_beam"),
                (3, "minecraft:p3_beam"),
                (4, "mymod:screw"),
                (5, "mymod:gear"),
            ];
            let mut asked = Vec::new();
            let distances = ViewDistances::of_states(states.into_iter(), |block| {
                asked.push(block.to_owned());
                match block {
                    "minecraft:p3_beam" => 256.0,
                    "mymod:screw" => f64::NAN,
                    _ => VIEW_DISTANCE,
                }
            });
            assert_eq!(asked, ["minecraft:p3_beam", "mymod:screw", "mymod:gear"]);
            assert_eq!(distances.of(2), 256.0);
            assert_eq!(distances.of(3), 256.0);
            assert_eq!(distances.of(4), VIEW_DISTANCE);
            assert_eq!(distances.of(5), VIEW_DISTANCE);
            assert_eq!(distances.of(1), VIEW_DISTANCE);
            assert_eq!(distances.furthest, 256.0);
        });
        let none = ViewDistances::of_states(std::iter::empty(), |_| unreachable!());
        assert_eq!(none.furthest, VIEW_DISTANCE);
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn block_draws_resolved_states_and_rejects_unknown_states() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::client_side::native_block_entity_draw::tests::block_draws_resolved_states_and_rejects_unknown_states",
            "SUNRISE_BLOCK_DRAW_STATES_CHILD",
            &["case"],
        ) {
            return;
        }
        use crate::game::host::draw::GameBlockEntityDrawContext;
        use mod_api::{BlockEntityDrawContext, BlockState, HostBlockStates as _, StateConstraint};
        use sunrise_gameplay_common::game::api_values::TryIntoEngine as _;

        super::super::native_entity_refs::tests::with_client_game(|game| {
            let mut surface = draw();
            assert!(!game.native_draw_block(&mut surface, BlockStateId(u32::MAX)));
            assert!(surface.vertices.is_empty());

            let log_key = game.block_key("minecraft:oak_log").expect("oak_log key");
            let states = game.block_states();
            let log_x = BlockState::of(&states, log_key, &[StateConstraint::new("axis", "x")])
                .expect("oak_log[axis=x]");
            let log_y = BlockState::of(&states, log_key, &[StateConstraint::new("axis", "y")])
                .expect("oak_log[axis=y]");
            let log_x_id = log_x.try_into_engine(&()).expect("engine id for axis=x");
            let log_y_id = log_y.try_into_engine(&()).expect("engine id for axis=y");

            let invalid_ordinal = mod_api::__private::block_state_from_ordinal(log_key, u16::MAX);
            let unknown_block = mod_api::__private::block_state_from_ordinal(
                mod_api::__private::parts_installation::block_key(u32::MAX),
                0,
            );
            {
                let mut drawn = GameBlockEntityDrawContext {
                    game,
                    surface: &mut surface,
                };
                assert!(!drawn.block(invalid_ordinal));
                assert!(!drawn.block(unknown_block));
            }
            assert!(surface.vertices.is_empty());

            let bytes = |vertices: &[Vertex]| bytemuck::cast_slice::<_, u8>(vertices).to_vec();
            let top_sprite = game.client.gui.atlas.sprites["block/oak_log_top"];
            let side_sprite = game.client.gui.atlas.sprites["block/oak_log"];
            let face_in_sprite = |quad: &[Vertex], rect: Rect| {
                quad.iter().all(|v| {
                    (rect.u0..=rect.u1).contains(&v.uv[0]) && (rect.v0..=rect.v1).contains(&v.uv[1])
                })
            };

            let render = |game: &crate::game::ClientGame<'_>, state: BlockState| {
                let mut surface = draw();
                surface.light = 0x7030;
                {
                    let mut drawn = GameBlockEntityDrawContext {
                        game,
                        surface: &mut surface,
                    };
                    assert!(drawn.block(state));
                }
                surface
            };

            // Sprites were bound when the generation was built, not on the first block draw.
            game.client.gui.atlas.sprites.remove("block/oak_log_top");
            let drawn_y = render(game, log_y);
            game.client
                .gui
                .atlas
                .sprites
                .insert("block/oak_log_top".into(), top_sprite);
            let drawn_x = render(game, log_x);

            let air_key = game.block_key("minecraft:air").unwrap();
            let air = BlockState::of(&game.block_states(), air_key, &[]).unwrap();
            assert!(
                render(game, air).vertices.is_empty(),
                "valid empty states draw nothing"
            );
            assert_eq!(drawn_y.vertices.len(), 24, "upright oak_log has 6 faces");
            assert_eq!(drawn_x.vertices.len(), 24, "horizontal oak_log has 6 faces");
            assert!(drawn_y.vertices.iter().all(|v| v.light == 0x7030));
            assert!(drawn_x.vertices.iter().all(|v| v.light == 0x7030));

            let y_top = drawn_y
                .vertices
                .as_chunks::<4>()
                .0
                .iter()
                .find(|q| q.iter().all(|v| v.pos[1] == 1.0))
                .expect("top face on axis=y");
            let y_east = drawn_y
                .vertices
                .as_chunks::<4>()
                .0
                .iter()
                .find(|q| q.iter().all(|v| v.pos[0] == 11.0))
                .expect("east face on axis=y");
            assert!(face_in_sprite(y_top, top_sprite));
            assert!(face_in_sprite(y_east, side_sprite));

            let x_top = drawn_x
                .vertices
                .as_chunks::<4>()
                .0
                .iter()
                .find(|q| q.iter().all(|v| v.pos[1] == 1.0))
                .expect("top face on axis=x");
            let x_east = drawn_x
                .vertices
                .as_chunks::<4>()
                .0
                .iter()
                .find(|q| q.iter().all(|v| v.pos[0] == 11.0))
                .expect("east face on axis=x");
            assert!(face_in_sprite(x_top, side_sprite));
            assert!(face_in_sprite(x_east, top_sprite));

            for (state_id, drawn) in [(log_y_id, &drawn_y), (log_x_id, &drawn_x)] {
                let mut expected = Vec::new();
                game.block_model_vertices(
                    state_id,
                    sunrise_gameplay_common::game::CommonGame::BLOCK_MODEL_SEED,
                    drawn.pose(),
                    drawn.element_ambient(),
                    true,
                    &mut expected,
                );
                assert_eq!(bytes(&drawn.vertices), bytes(&expected));
            }
        });
    }

    #[cfg(test)]
    /// `BlockEntityDrawContext::text` lays the font's glyphs out under the pose, a quad per
    /// glyph from left to right, in the frame's light and the opaque colour it was given, with
    /// the text's styles: bold widens it and doubles its glyphs, and an outline (black too)
    /// rings each glyph with eight copies behind the text.
    #[test]
    #[ignore = "daily"]
    fn text_lays_glyphs_out_under_the_pose_in_the_frames_light() {
        use crate::game::host::draw::GameBlockEntityDrawContext;
        use mod_api::BlockEntityDrawContext;

        super::super::native_entity_refs::tests::with_client_game(|game| {
            // The server fixture only measures fonts; this test needs drawable glyphs.
            game.client.gui.atlas = sunrise_client::hud::HudAtlas::build(
                &sunrise_assets::resource_source::ResourceSource::directory(
                    game.options.resources.to_path_buf(),
                ),
            )
            .unwrap();
            let plain = mod_api::Text::literal("AÁ");
            let mut style = mod_api::text::TextStyle::default();
            style.bold = Some(true);
            let bold = mod_api::Text::literal("A").with_style(style);
            let mut surface = draw();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            assert_eq!(drawn.text_width(&mod_api::Text::literal("A")), 6.0);
            assert_eq!(drawn.text_width(&plain), 12.0);
            // Font.width: bold adds a pixel a glyph.
            assert_eq!(drawn.text_width(&bold), 7.0);
            let mut deep = mod_api::Text::literal("A");
            for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
                deep = mod_api::Text::literal("").append(deep);
            }
            assert_eq!(drawn.text_width(&deep), 6.0);
            assert_eq!(
                drawn.text(&plain, [0.0, 0.0], 0xFF0000, false, None),
                Ok(12.0)
            );
            assert_eq!(surface.vertices.len(), 8, "a quad per glyph");
            // Vanilla's bitmap providers: A is 8x8 with ascent 7; Á is 9x12 with ascent 10.
            // GlyphBitmap places the top at 7 - ascent; BakedSheetGlyph adds the pen's position.
            let mut at: Vec<[f32; 3]> = surface.vertices.iter().map(|v| v.pos).collect();
            at.sort_by(|a, b| a.partial_cmp(b).unwrap());
            assert_eq!(
                at,
                [
                    [10.0, 0.0, 0.0],
                    [10.0, 8.0, 0.0],
                    [16.0, -3.0, 0.0],
                    [16.0, 9.0, 0.0],
                    [18.0, 0.0, 0.0],
                    [18.0, 8.0, 0.0],
                    [25.0, -3.0, 0.0],
                    [25.0, 9.0, 0.0],
                ]
            );
            // Red, as a vertex colour stores it (ABGR), opaque.
            assert!(surface.vertices.iter().all(|v| v.color == 0xFF00_00FF));
            assert!(surface.vertices.iter().all(|v| v.light == 0xF0F0));

            // Bold: each glyph again a pixel to the right (Font's bold offset).
            surface.vertices.clear();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            assert_eq!(drawn.text(&bold, [0.0, 0.0], 0xFFFFFF, true, None), Ok(7.0));
            assert_eq!(surface.vertices.len(), 8, "the glyph twice");
            assert!(surface.vertices.iter().all(|v| v.light == 240 | 240 << 8));

            // Deep component trees (> 512 nodes) match vanilla Font.width / Font.drawInBatch.
            surface.vertices.clear();
            let mut deep = mod_api::Text::literal("A");
            for _ in 0..600 {
                deep = mod_api::Text::literal("").append(deep);
            }
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            assert_eq!(drawn.text_width(&deep), 6.0);
            assert_eq!(drawn.text(&deep, [0.0, 0.0], 0xFFFFFF, true, None), Ok(6.0));
            assert_eq!(surface.vertices.len(), 4);

            // A black outline: eight rings behind, then the text a hair in front of them.
            surface.vertices.clear();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            drawn
                .text(
                    &mod_api::Text::literal("A"),
                    [0.0, 0.0],
                    0xFFFFFF,
                    true,
                    Some(0),
                )
                .unwrap();
            assert_eq!(surface.vertices.len(), 9 * 4);
            let (rings, front) = surface.vertices.split_at(8 * 4);
            assert!(
                rings
                    .iter()
                    .all(|v| v.color == 0xFF00_0000 && v.pos[2] == 0.0)
            );
            assert!(
                front
                    .iter()
                    .all(|v| v.color == u32::MAX && v.pos[2] == TEXT_OVER_OUTLINE)
            );
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn fractional_text_advance_rounds_up_for_drawing_and_measurement() {
        use crate::game::host::draw::GameBlockEntityDrawContext;
        use mod_api::BlockEntityDrawContext;
        use sunrise_assets::resource_source::{Resource, ResourceSource};
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
                "assets/minecraft/font/default.json".into(),
                Resource::bytes(
                    br#"{"providers":[{"type":"space","advances":{"X":2.25}}]}"#.to_vec(),
                ),
            )]));
            game.client.gui.atlas.font = sunrise_client::font::Font::metrics(&source);
            let mut surface = draw();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            let text = mod_api::Text::literal("XX");
            assert_eq!(drawn.text_width(&text), 5.0);
            assert_eq!(drawn.text(&text, [0.0; 2], 0xFFFFFF, false, None), Ok(5.0));
            assert_eq!(
                drawn.text(&text, [0.0; 2], 0xFFFFFF, true, Some(0)),
                Ok(5.0)
            );
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn animation_clock_survives_pause_freeze_resume_and_time_updates() {
        use std::time::Duration;
        use sunrise_protocol::packets::play;
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.clock = sunrise_sim::clock::FixedStep::client();
            game.client.texture_ticks = 0;
            // Run just the fixture's visual work, then the real frame's phase sampling.
            let frame = |game: &mut crate::game::ClientGame<'_>, ms, target, pause| {
                let steps = game.clock.advance(Duration::from_millis(ms), target);
                if !game.clock.paused() {
                    for _ in 0..steps {
                        let running = game.clock.world_runs();
                        game.tick_frame_visuals(running);
                    }
                }
                game.client_tick_steps(0, None).unwrap();
                game.clock.end_frame(pause);
            };
            frame(game, 75, 50.0, true);
            assert_eq!(game.animation_time(), 1.5);
            frame(game, 25, 50.0, false);
            assert_eq!(game.animation_time(), 1.5);
            game.clock.set_frozen(true);
            frame(game, 25, 50.0, false);
            assert_eq!(game.animation_time(), 1.5);
            game.world.game_time = 40;
            game.handle_packet(play::Clientbound::SetTime(play::SetTime {
                game_time: 0,
                clocks: vec![],
            }))
            .unwrap();
            assert_eq!(game.world.game_time, 0);
            assert_eq!(game.animation_time(), 1.5);

            assert!(game.clock.start_step(1));
            frame(game, 50, 50.0, false);
            assert_eq!(game.animation_time(), 2.5);

            game.clock.set_frozen(false);
            frame(game, 50, 50.0, false);
            assert_eq!(game.client.texture_ticks, 3);
            assert_eq!(game.animation_time(), 2.5);
            frame(game, 25, 100.0, false);
            assert_eq!(game.animation_time(), 2.75);
            frame(game, 75, 100.0, false);
            assert_eq!(game.animation_time(), 3.5);
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn atlas_reset_discards_queued_text_uploads() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.client
                .text_patches
                .borrow_mut()
                .push(sunrise_client::font::Patch {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    rgba: vec![255; 4],
                });
            game.forget_atlas_slots();
            assert!(game.client.text_patches.borrow().is_empty());
        });
    }

    #[cfg(test)]
    /// A face a mod builds with `CubeFace::new` and `with_uv` draws in its sprite through
    /// `BlockEntityDrawContext::cube`, its UVs cut from the sprite; an unknown sprite's face
    /// is left out and the call says so.
    #[test]
    #[ignore = "daily"]
    fn a_mod_built_cube_face_draws_in_its_sprite() {
        use crate::game::host::draw::GameBlockEntityDrawContext;
        use mod_api::BlockEntityDrawContext;

        super::super::native_entity_refs::tests::with_client_game(|game| {
            // A server-side game holds only the font, so the sprite is given its place.
            game.client
                .gui
                .atlas
                .sprites
                .insert("mymod:block/casing".into(), sprite());
            let casing = sprite();
            let mut surface = draw();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            let mut faces = [None; 6];
            // Up, the casing's top-left quarter.
            faces[1] =
                Some(mod_api::CubeFace::new("mymod:block/casing").with_uv([0.0, 0.0, 8.0, 8.0]));
            assert!(drawn.cube([0.0; 3], [16.0; 3], faces, 0xFFFFFF));
            assert_eq!(surface.vertices.len(), 4, "one face");
            let (mid_u, mid_v) = ((casing.u0 + casing.u1) / 2.0, (casing.v0 + casing.v1) / 2.0);
            for v in &surface.vertices {
                assert_eq!(v.pos[1], 1.0, "a 16-texel box's top, in blocks");
                assert!((casing.u0..=mid_u).contains(&v.uv[0]), "{:?}", v.uv);
                assert!((casing.v0..=mid_v).contains(&v.uv[1]), "{:?}", v.uv);
            }

            let mut surface = draw();
            let mut drawn = GameBlockEntityDrawContext {
                game,
                surface: &mut surface,
            };
            faces[0] = Some(mod_api::CubeFace::new("mymod:block/missing"));
            assert!(!drawn.cube([0.0; 3], [16.0; 3], faces, 0xFFFFFF));
            assert_eq!(surface.vertices.len(), 4, "the casing face alone");
        });
    }

    #[cfg(test)]
    #[test]
    fn the_pose_stack_turns_about_its_pivot_and_pops_back() {
        let mut d = draw();
        d.push();
        d.rotate_about([0.0, 1.0, 0.0], 90.0, [0.5, 0.5, 0.5]);
        // (1, 0, 0.5) turns a quarter about the block's centre to (0.5, 0, 0).
        let p = d.pose().transform_point3(Vec3::new(1.0, 0.0, 0.5));
        assert!(p.abs_diff_eq(Vec3::new(10.5, 0.0, 0.0), 1e-5), "{p}");
        d.pop();
        d.pop();
        // Popping past the block's own pose keeps it.
        let p = d.pose().transform_point3(Vec3::ZERO);
        assert_eq!(p, Vec3::new(10.0, 0.0, 0.0));
        d.translate([0.0, 2.0, 0.0]);
        d.scale([2.0, 2.0, 2.0]);
        let p = d.pose().transform_point3(Vec3::ONE);
        assert_eq!(p, Vec3::new(12.0, 4.0, 2.0));
    }

    #[cfg(test)]
    /// An arm a block east of the frame's origin, turned a quarter about z,
    /// with a hand a block below its pivot: `translateAndRotate` turns each
    /// part about its own pivot, then moves it, and a child goes with its
    /// parent. Expected corners are worked out by hand from JOML's
    /// `rotationZYX` (a right-handed quarter turn takes +x to +y).
    #[test]
    fn a_model_part_draws_turned_about_its_pivot_and_carries_its_children() {
        use mod_api::{CubeListBuilder, LayerDefinition, MeshDefinition, PartPose};
        use sunrise_core::geometry::Direction as Face;
        let mut mesh = MeshDefinition::new();
        let root = mesh.root();
        let cube = |face| CubeListBuilder::create().add_box_faces([0.0; 3], [16.0; 3], &[face]);
        let arm = mesh.add_or_replace_child(
            root,
            "arm",
            cube(Face::South),
            PartPose::offset_and_rotation(16.0, 0.0, 0.0, 0.0, 0.0, std::f32::consts::FRAC_PI_2),
        );
        mesh.add_or_replace_child(
            arm,
            "hand",
            cube(Face::North),
            PartPose::offset(0.0, 16.0, 0.0),
        );
        let mut model = LayerDefinition::create(mesh, 64, 64).bake_root();
        let drawn = |model: &ModelPart| {
            let mut d = draw();
            draw_model_part(&mut d, model, sprite(), 0xFF_FFFF);
            d.into_vertices()
        };
        let near = |v: &Vertex, at: [f32; 3]| {
            assert!(
                Vec3::from(v.pos).abs_diff_eq(Vec3::from(at), 1e-5),
                "{:?} not {at:?}",
                v.pos
            );
        };

        let all = drawn(&model);
        assert_eq!(all.len(), 8);
        // The arm's south face, (0, 0, 1) (1, 0, 1) (1, 1, 1) (0, 1, 1) in
        // its frame, turned and moved a block east, then the frame's 10.
        for (v, at) in all.iter().zip([
            [11.0, 0.0, 1.0],
            [11.0, 1.0, 1.0],
            [10.0, 1.0, 1.0],
            [10.0, 0.0, 1.0],
        ]) {
            near(v, at);
        }
        // Its net's last strip, 48 to 64 across and 16 to 32 down, in the sprite.
        assert_eq!(all[0].uv, [0.75, 0.3125]);
        // The hand's north face starts at its (1, 0, 0), the arm's (1, 1, 0).
        near(&all[4], [10.0, 1.0, 0.0]);
        near(&all[5], [10.0, 0.0, 0.0]);

        let arm = model.get_child_mut("arm").unwrap();
        arm.skip_draw = true;
        let hand_only = drawn(&model);
        assert_eq!(hand_only.len(), 4);
        near(&hand_only[0], [10.0, 1.0, 0.0]);
        model.get_child_mut("arm").unwrap().visible = false;
        assert!(drawn(&model).is_empty());
    }

    #[cfg(test)]
    #[test]
    fn a_quad_lands_in_its_sprite_lit_and_shaded() {
        let mut d = draw();
        // An upward face: full brightness under vanilla's level lights is
        // below 1 (two lights), so the colour is shaded but opaque.
        d.quad(
            [
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, 1.0, 0.0],
            ],
            [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            sprite(),
            0xFFFFFF,
        );
        assert_eq!(d.vertices.len(), 4);
        let v = d.vertices[2];
        assert_eq!(v.pos, [11.0, 1.0, 1.0]);
        assert_eq!(v.uv, [0.75, 0.5]);
        assert_eq!(v.light, 0xF0F0);
        let up = diffuse(Vec3::Y, diffuse_lights(false));
        assert_eq!(v.color, shaded(0xFFFFFF, up));
        assert_eq!(v.color >> 24, 0xFF);
    }

    /// Each quad carries its render type and texture in its vertices, so quads of two textures
    /// stay apart whatever the batch does with them.
    #[test]
    fn textured_quads_name_their_render_type_and_texture() {
        use sunrise_client::gfx::{
            DYNAMIC_TEXTURE_CUTOUT_OVERLAY, DYNAMIC_TEXTURE_SOLID_OVERLAY, dynamic_texture_bits,
        };
        let mut d = draw();
        assert!(d.textured_quad(
            CORNERS,
            UNIT_UV,
            DYNAMIC_TEXTURE_CUTOUT_OVERLAY,
            7,
            0xFFFFFF
        ));
        assert!(d.textured_quad(
            CORNERS,
            UNIT_UV,
            DYNAMIC_TEXTURE_CUTOUT_OVERLAY,
            7,
            0xFFFFFF
        ));
        assert!(d.textured_quad(CORNERS, UNIT_UV, DYNAMIC_TEXTURE_SOLID_OVERLAY, 9, 0xFFFFFF));
        assert_eq!(d.vertices.len(), 12);
        let expected = [
            (DYNAMIC_TEXTURE_CUTOUT_OVERLAY, 7),
            (DYNAMIC_TEXTURE_CUTOUT_OVERLAY, 7),
            (DYNAMIC_TEXTURE_SOLID_OVERLAY, 9),
        ];
        for (quad, (overlay, handle)) in d.vertices.chunks(4).zip(expected) {
            for v in quad {
                assert_eq!(v.layer, overlay);
                assert_eq!(v.glint_uv, dynamic_texture_bits(handle));
            }
        }
        assert_eq!(d.vertices[2].uv, [1.0, 1.0]);
    }

    /// A coordinate that is not finite refuses the quad and appends nothing; so does a
    /// first-person hand draw.
    #[test]
    fn a_textured_quad_refuses_non_finite_input_and_the_hand() {
        use sunrise_client::gfx::DYNAMIC_TEXTURE_SOLID_OVERLAY as SOLID;
        let mut d = draw();
        let mut bad = CORNERS;
        bad[1][2] = f32::NAN;
        assert!(!d.textured_quad(bad, UNIT_UV, SOLID, 7, 0xFFFFFF));
        let mut uv = UNIT_UV;
        uv[3][0] = f32::INFINITY;
        assert!(!d.textured_quad(CORNERS, uv, SOLID, 7, 0xFFFFFF));
        assert!(d.vertices.is_empty());
        d.normal_pose = Some(ModelNormals::from_rotation(Mat4::IDENTITY));
        assert!(!d.textured_quad(CORNERS, UNIT_UV, SOLID, 7, 0xFFFFFF));
        assert!(d.vertices.is_empty());
    }

    /// Under `unlit(true)` a textured quad has full bright light and no shading normal.
    #[test]
    fn an_unlit_textured_quad_is_full_bright() {
        use sunrise_client::gfx::DYNAMIC_TEXTURE_SOLID_OVERLAY as SOLID;
        use sunrise_client::mobs::render::FULL_BRIGHT;
        let mut d = draw();
        d.set_light(0x3070);
        d.unlit(true);
        assert!(d.textured_quad(CORNERS, UNIT_UV, SOLID, 7, 0xFFFFFF));
        for v in &d.vertices {
            assert_eq!(v.light, FULL_BRIGHT);
            assert_eq!(v.normal, [0; 4]);
        }
    }

    /// `textured_quad` draws nothing for a render type whose texture is not a live dynamic
    /// texture, and for a handle that is not one of the three texture-taking constructors'
    /// (`entity_solid`, `entity_cutout`, `entity_translucent`), here `minecraft:eyes`'.
    #[test]
    #[ignore = "daily"]
    fn a_textured_quad_of_a_texture_that_is_not_live_or_a_plain_render_type_draws_nothing() {
        use sunrise_client_render::mod_pipelines::{BuiltinRenderType, Pipelines};
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let mut render = sunrise_client_render::assets::RenderAssets::unpublished();
            render.mod_pipelines = Pipelines::load(
                &sunrise_assets::resource_source::ResourceSource::directory(
                    "/nonexistent/sunrise-test-assets".into(),
                ),
                7,
            )
            .unwrap();
            game.client.client_pack.render = std::sync::Arc::new(render);
            let texture = mod_api::Identifier::parse("example:dynamic/none").unwrap();
            let mut d = draw();
            for kind in [
                BuiltinRenderType::EntitySolid,
                BuiltinRenderType::EntityCutout,
                BuiltinRenderType::EntityTranslucent,
            ] {
                let made = game
                    .client
                    .client_pack
                    .render
                    .textured_render_type(kind, &texture)
                    .unwrap();
                assert!(!game.native_draw_textured_quad(&mut d, made, CORNERS, UNIT_UV, 0xFFFFFF));
            }
            let eyes = game
                .client
                .client_pack
                .render
                .render_type(&mod_api::Identifier::parse("minecraft:eyes").unwrap())
                .unwrap();
            assert!(!game.native_draw_textured_quad(&mut d, eyes, CORNERS, UNIT_UV, 0xFFFFFF));
            assert!(d.vertices.is_empty());
        });
    }

    #[cfg(test)]
    /// A quad (what `cube` and `model_part` emit) drawn after `unlit(true)` has no shading
    /// normal, its plain tint and full bright light, even under `level_light`; after
    /// `unlit(false)` a bottom face is shaded (vanilla's 0.4) and lit by the draw's light again.
    #[test]
    fn an_unlit_quad_is_unshaded_and_full_bright_until_unlit_false() {
        use sunrise_client::mobs::render::FULL_BRIGHT;
        let bottom = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ];
        let uv = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        let mut d = draw();
        d.set_light(0x3070);
        d.level_light();
        d.unlit(true);
        d.quad(bottom, uv, sprite(), 0x80C0FF);
        // A turning part keeps a shading normal, so `unlit` must clear that one too.
        d.spin([1.0, 0.0, 0.0], [0.5; 3], 1.0, 0.0);
        d.quad(bottom, uv, sprite(), 0x80C0FF);
        d.still();
        assert_eq!(d.vertices.len(), 8);
        assert!(d.lit.is_empty(), "nothing left for the level to relight");
        for v in &d.vertices {
            assert_eq!(v.normal, [0; 4]);
            assert_eq!(v.color, 0xFFFF_C080);
            assert_eq!(v.light, FULL_BRIGHT);
        }

        d.unlit(false);
        d.set_light(0x3070);
        d.quad(bottom, uv, sprite(), 0x80C0FF);
        for v in &d.vertices[8..] {
            assert_eq!(v.normal, [0; 4], "a still quad's shade is in its colour");
            assert_eq!(v.color, 0xFF66_4C33);
            assert_eq!(v.light, 0x3070);
        }
    }

    #[cfg(test)]
    /// A model drawn under `unlit` is drawn on the CPU (no prepared copy), with no shading
    /// normal, its plain colour and full bright light, and nothing for `level_light` to relight.
    #[test]
    fn an_unlit_model_skips_its_prepared_copy_shading_and_light() {
        use sunrise_client::mobs::render::FULL_BRIGHT;
        let quad = bake_flat_face(
            [0.0; 3],
            [16.0; 3],
            [0.0, 0.0, 16.0, 16.0],
            Direction::Down,
            Material {
                sprite: String::new(),
                force_translucent: false,
            },
            -1,
        )
        .unwrap();
        let face =
            crate::game::client_side::minecarts_client::PreparedModelFace::new(&quad, sprite());
        let mut d = draw();
        d.prepared = true;
        d.set_light(0x3070);
        d.unlit(true);
        assert!(
            !d.defer_model(0, 1, u32::MAX),
            "unlit alone keeps it on the CPU"
        );
        d.level_light();
        d.model(&[face], u32::MAX);
        assert_eq!(d.vertices.len(), 4);
        assert!(d.lit.is_empty());
        for v in &d.vertices {
            assert_eq!(v.normal, [0; 4]);
            assert_eq!(v.color, u32::MAX);
            assert_eq!(v.light, FULL_BRIGHT);
        }
    }

    #[cfg(test)]
    /// `unlit` lasts to the end of one block entity's draw: the next one kept in the frame,
    /// drawn through the same scratch draw, is lit again.
    #[test]
    fn unlit_ends_with_the_block_entity_that_turned_it_on() {
        use sunrise_client::mobs::render::FULL_BRIGHT;
        let level = LevelLight {
            origin: DVec3::ZERO,
            light_at: &|_| 0,
        };
        let mut kept = KeptGeometry::default();
        let mut draw = draw();
        let key = KeptKey {
            light: 0x3070,
            ..KEY
        };
        for (pos, unlit) in [([0, 64, 0], true), ([1, 64, 0], false)] {
            kept.render(
                pos,
                key,
                machine(),
                &mut draw,
                Vec3::ZERO,
                &level,
                None,
                &mut Vec::new(),
                || DEFAULT_BOUNDS,
                |_, _| true,
                |draw| {
                    if unlit {
                        draw.unlit(true);
                    }
                    draw.quad(LIGHT_QUAD, [[0.0; 2]; 4], sprite(), 0xFFFFFF);
                },
            );
        }
        let lights = draw.vertices.iter().map(|v| v.light).collect::<Vec<_>>();
        assert_eq!(lights, [[FULL_BRIGHT; 4], [0x3070; 4]].concat());
    }

    #[cfg(test)]
    /// An item drawn under `unlit` comes out the same in the dark (block 0, sky 0) as at
    /// full light (15/15), while a lit item's light follows the setting.
    #[test]
    #[ignore = "daily"]
    fn an_unlit_item_draws_the_same_in_the_dark_as_at_full_light() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.client
                .gui
                .atlas
                .sprites
                .insert("misc/missingno".into(), sprite());
            let stack = ItemStack::new("minecraft:stick", 1);
            let item = |game: &crate::game::ClientGame<'_>, light: u32, unlit: bool| {
                let mut d = draw();
                d.set_light(light);
                d.unlit(unlit);
                assert!(game.native_draw_item(&mut d, &stack, DisplayContext::Fixed, None, 0));
                d.vertices
                    .iter()
                    .map(|v| (v.color, v.light, v.normal))
                    .collect::<Vec<_>>()
            };
            let dark = item(game, 0, true);
            assert!(!dark.is_empty());
            assert_eq!(dark, item(game, 0xF0F0, true));
            let full = sunrise_client::mobs::render::FULL_BRIGHT;
            // An untinted stick unshaded: every face its plain white (its normal is only
            // the back-face marker, `CULL_BACK`).
            assert!(
                dark.iter().all(|&(c, l, _)| (c, l) == (u32::MAX, full)),
                "{dark:?}"
            );
            let lit = item(game, 0xF0F0, false);
            assert!(
                lit.iter().any(|&(color, ..)| color != u32::MAX),
                "lit faces are shaded"
            );
            assert!(item(game, 0, false).iter().all(|&(_, light, _)| light == 0));
        });
    }

    #[cfg(test)]
    #[test]
    fn infinite_bounds_and_frames_without_a_frustum_are_never_culled() {
        let origin = Vec3::new(1.0e6, 0.0, 0.0);
        let infinite = [f32::NEG_INFINITY, 0.0, 0.0, f32::INFINITY, 1.0, 1.0];
        assert!(!in_frustum(Some(&Blind), origin, DEFAULT_BOUNDS));
        assert!(in_frustum(Some(&Blind), origin, infinite));
        assert!(in_frustum(None::<&Blind>, origin, DEFAULT_BOUNDS));
    }

    #[cfg(test)]
    #[test]
    fn a_tint_keeps_its_channels_in_vertex_order() {
        // The vertex colour is RGBA8 read little-endian: red in the low byte.
        assert_eq!(shaded(0x10_20_30, 1.0), 0xFF_30_20_10);
    }

    #[cfg(test)]
    const KEY: KeptKey = KeptKey {
        dim: Dim::Overworld,
        state: 7,
        light: 0xF0F0,
        glyphs: 0,
    };

    #[cfg(test)]
    /// A part drawn after `spin` turns about its pivot by the animation clock
    /// (90 degrees a tick about +y takes +x to -z, by the right
    /// hand), its normal with it; what is drawn after `still` and before the
    /// spin stays; all of it is moved by the origin.
    #[test]
    fn a_spinning_part_turns_with_the_animation_clock_and_the_rest_stays() {
        let mut draw = draw();
        let at = |x: f32, y: f32, z: f32, normal: [i8; 4]| Vertex {
            pos: [x, y, z],
            normal,
            ..bytemuck::Zeroable::zeroed()
        };
        draw.vertices.push(at(1.0, 0.5, 0.5, [0; 4]));
        draw.spin([0.0, 1.0, 0.0], [0.5, 0.5, 0.5], 90.0, 0.0);
        draw.vertices.push(at(1.0, 0.5, 0.5, [127, 0, 0, 127]));
        draw.still();
        draw.vertices.push(at(0.0, 0.0, 0.0, [0; 4]));
        let origin = Vec3::new(10.0, 0.0, 0.0);
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4);
        // A tick and a half: 135 degrees.
        let mut out = draw.vertices.clone();
        place_geometry(&mut out, &draw.spins, 1.5, origin, None);
        assert!(close(out[0].pos, [11.0, 0.5, 0.5]), "{:?}", out[0].pos);
        let h = 0.5 * std::f32::consts::FRAC_1_SQRT_2;
        assert!(
            close(out[1].pos, [10.5 - h, 0.5, 0.5 - h]),
            "{:?}",
            out[1].pos
        );
        assert!(close(out[2].pos, [10.0, 0.0, 0.0]), "{:?}", out[2].pos);
        // One tick: a quarter turn, +x to -z, the normal along.
        out.copy_from_slice(&draw.vertices);
        place_geometry(&mut out, &draw.spins, 1.0, origin, None);
        assert!(close(out[1].pos, [10.5, 0.5, 0.0]), "{:?}", out[1].pos);
        assert_eq!(out[1].normal, [0, 0, -127, 127]);
        // Four ticks: all the way round, and a long session's late clock is no
        // less exact.
        out.copy_from_slice(&draw.vertices);
        place_geometry(&mut out, &draw.spins, 4_000_000_000.0, origin, None);
        assert!(close(out[1].pos, [11.0, 0.5, 0.5]), "{:?}", out[1].pos);
    }

    #[cfg(test)]
    #[test]
    fn kept_spun_model_follows_level_light_without_losing_emission_or_resubmitting() {
        use std::cell::{Cell, RefCell};

        let mut quad = bake_flat_face(
            [0.0; 3],
            [16.0; 3],
            [0.0, 0.0, 16.0, 16.0],
            Direction::South,
            Material {
                sprite: "test:casing".into(),
                force_translucent: false,
            },
            -1,
        )
        .unwrap();
        let plain =
            crate::game::client_side::minecarts_client::PreparedModelFace::new(&quad, sprite());
        quad.light_emission = 12;
        let emissive =
            crate::game::client_side::minecarts_client::PreparedModelFace::new(&quad, sprite());
        let pos = [10, 64, 20];
        let samples = RefCell::new(Vec::new());
        let level = LevelLight {
            origin: DVec3::from_array(pos.map(f64::from)),
            light_at: &|cell| {
                samples.borrow_mut().push(cell);
                match cell {
                    [11, 64, 20] => 0x7F,
                    [10, 64, 19] => 0,
                    [10, 64, 20] => 0x29,
                    _ => panic!("unexpected sample at {cell:?}"),
                }
            },
        };
        let mut kept = KeptGeometry::default();
        let mut draw = draw();
        let revision = machine();
        let submits = Cell::new(0);
        let bounds_calls = Cell::new(0);
        for (tick, expected) in [
            (0, [0x9020, 0xF070, 0xF070, 0xF0F0]),
            (1, [0x9020, 0xC000, 0x0000, 0xF0F0]),
        ] {
            draw.view.animation_time = f64::from(tick);
            draw.vertices.clear();
            kept.render(
                pos,
                KEY,
                revision,
                &mut draw,
                Vec3::new(4.0, -2.0, 3.0),
                &level,
                None,
                &mut Vec::new(),
                || {
                    bounds_calls.set(bounds_calls.get() + 1);
                    DEFAULT_BOUNDS
                },
                |_, _| true,
                |draw| {
                    submits.set(submits.get() + 1);
                    // A stationary lit neighbour keeps its first sample. The model's two
                    // faces share a lookup, but only one face has an intrinsic block floor.
                    draw.level_light();
                    draw.quad(LIGHT_QUAD, [[0.0; 2]; 4], sprite(), 0xFFFFFF);
                    draw.spin([0.0, 1.0, 0.0], [0.5; 3], 90.0, 0.0);
                    draw.push();
                    draw.translate([1.0, 0.0, 0.0]);
                    draw.model(&[emissive, plain], u32::MAX);
                    draw.pop();
                    draw.still();
                    draw.set_light(KEY.light);
                    draw.quad(LIGHT_QUAD, [[0.0; 2]; 4], sprite(), 0xFFFFFF);
                    draw.keep();
                },
            );
            assert_eq!(
                draw.vertices.iter().map(|v| v.light).collect::<Vec<_>>(),
                expected
                    .into_iter()
                    .flat_map(|light| [light; 4])
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(
            *samples.borrow(),
            [[10, 64, 20], [11, 64, 20], [10, 64, 19]]
        );
        assert_eq!(bounds_calls.get(), 1);
        assert_eq!(submits.get(), 1);
    }

    #[cfg(test)]
    #[test]
    fn entity_level_light_samples_the_same_cell_at_two_camera_distances() {
        const DELTA: f32 = 1.0 / 1_048_576.0; // 2^-20, exact in both f32 and f64.
        let origin = DVec3::new(10.5 - f64::from(DELTA), 64.25, 20.0);
        let level = LevelLight {
            origin,
            light_at: &|cell| {
                assert_eq!(cell, [11, 64, 20]);
                0x37
            },
        };
        for relative_x in [1.0, 64.0] {
            let camera = origin - DVec3::new(relative_x, 0.0, 0.0);
            let mut draw = BlockEntityDraw::posed(
                0.0,
                0,
                0x50A0,
                [10, 64, 20],
                Mat4::from_translation((origin - camera).as_vec3()),
                diffuse_lights(false),
                Vec::new(),
            );
            draw.track_entity_local_light_pose();
            draw.translate([DELTA, 0.0, 0.0]);
            draw.push();
            draw.translate([1.0, 0.0, 0.0]);
            draw.pop();
            draw.level_light();
            // The local centre is 0.5 + DELTA, so it lies on the X=11 boundary.
            // With the camera 64 blocks away, the f32 render pose loses DELTA.
            draw.quad(
                [
                    [0.5, 1.0, 0.0],
                    [0.5, 0.0, 0.0],
                    [0.5, 0.0, 1.0],
                    [0.5, 1.0, 1.0],
                ],
                [[0.0; 2]; 4],
                sprite(),
                0xFFFFFF,
            );
            let out = draw.into_vertices_with_light(Some(&level));
            assert_eq!(out.len(), 4);
            assert!(out.iter().all(|v| v.light == 0x7030));
        }
    }

    #[cfg(test)]
    #[test]
    fn a_huge_translation_samples_only_when_the_absolute_cell_fits() {
        for (anchor_x, cell, expected) in [(10, None, 0x3070), (-1, Some(i32::MAX), 0x2040)] {
            let pos = [anchor_x, 64, 20];
            let mut draw = draw();
            KeptGeometry::default().render(
                pos,
                KeptKey {
                    light: 0x3070,
                    ..KEY
                },
                machine(),
                &mut draw,
                Vec3::ZERO,
                &LevelLight {
                    origin: DVec3::from_array(pos.map(f64::from)),
                    light_at: &|[x, y, z]| {
                        assert_eq!(Some(x), cell);
                        assert_eq!([y, z], [64, 20]);
                        0x42
                    },
                },
                None,
                &mut Vec::new(),
                || DEFAULT_BOUNDS,
                |_, _| true,
                |draw| {
                    draw.level_light();
                    draw.translate([2_147_483_648.0, 0.0, 0.0]);
                    draw.quad(LIGHT_QUAD, [[0.0; 2]; 4], sprite(), 0xFFFFFF);
                },
            );
            assert_eq!(draw.vertices.len(), 4);
            assert!(draw.vertices.iter().all(|v| v.light == expected));
        }
    }

    #[cfg(test)]
    #[test]
    fn kept_geometry_is_drawn_again_until_what_it_was_drawn_with_changes() {
        let mut kept = KeptGeometry::default();
        let pos = [1, 2, 3];
        let mut entity = ModBlockEntity::new(
            "mymod:machine".into(),
            ModBlockEntityFields {
                data: vec![0],
                ..Default::default()
            },
        );
        let drawn = entity.revision();
        kept.keep(
            pos,
            KEY,
            drawn,
            DEFAULT_BOUNDS,
            &[vertex(0.5)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            0,
        );
        let entry = kept.get(pos, KEY, entity.revision(), 1).expect("kept");
        assert_eq!(entry.vertices.len(), 1);
        assert_eq!(entry.vertices[0].pos, [0.5, 0.0, 0.0]);
        assert_eq!(entry.bounds, DEFAULT_BOUNDS);
        // New data from the server, another state, light or dimension, or
        // another position: none of them is the kept geometry.
        entity.data[0] = 1;
        assert!(kept.get(pos, KEY, entity.revision(), 1).is_none());
        for changed in [
            KeptKey { state: 8, ..KEY },
            KeptKey {
                light: 0x00F0,
                ..KEY
            },
            KeptKey {
                dim: Dim::Nether,
                ..KEY
            },
            KeptKey { glyphs: 1, ..KEY },
        ] {
            assert!(kept.get(pos, changed, drawn, 1).is_none());
        }
        assert!(kept.get([1, 2, 4], KEY, drawn, 1).is_none());
        // Another block entity in its place, as the old one was: not it.
        let replaced = ModBlockEntity::new(
            "mymod:machine".into(),
            ModBlockEntityFields {
                data: vec![0],
                ..Default::default()
            },
        );
        assert!(kept.get(pos, KEY, replaced.revision(), 1).is_none());
        // Drawn again and kept with the new data, it replaces the old.
        kept.keep(
            pos,
            KEY,
            entity.revision(),
            DEFAULT_BOUNDS,
            &[vertex(0.25)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            2,
        );
        assert!(kept.get(pos, KEY, drawn, 2).is_none());
        let entry = kept
            .get(pos, KEY, entity.revision(), 2)
            .expect("kept again");
        assert_eq!(entry.vertices[0].pos, [0.25, 0.0, 0.0]);
        // Drawn without keeping: forgotten.
        kept.forget(pos);
        assert!(kept.get(pos, KEY, entity.revision(), 3).is_none());
    }

    #[cfg(test)]
    #[test]
    fn a_crowded_first_frame_does_not_keep_its_prefix() {
        use sunrise_client::gfx::EntityQuadCapacity;
        let pos = [10, 64, 20];
        let level = LevelLight {
            origin: DVec3::from_array(pos.map(f64::from)),
            light_at: &|_| 0,
        };
        let mut kept = KeptGeometry::default();
        let revision = machine();
        let submits = std::cell::Cell::new(0);
        let mut frame = |quads: u64| {
            let capacity = EntityQuadCapacity::from_max_buffer_size(
                EntityQuadCapacity::VERTEX_QUAD_BYTES * quads,
            );
            let mut draw = draw().with_quad_budget(capacity, 0);
            kept.render(
                pos,
                KEY,
                revision,
                &mut draw,
                Vec3::ZERO,
                &level,
                None,
                &mut Vec::new(),
                || DEFAULT_BOUNDS,
                |_, _| true,
                |draw| {
                    submits.set(submits.get() + 1);
                    for _ in 0..2 {
                        draw.quad(LIGHT_QUAD, [[0.0; 2]; 4], sprite(), 0xFFFFFF);
                    }
                    draw.keep();
                },
            );
            draw.vertices.len() / 4
        };
        assert_eq!(frame(1), 1, "the crowded frame draws what fits");
        assert_eq!(frame(8), 2, "an uncrowded one draws all of it");
        assert_eq!(submits.get(), 2, "the prefix was extracted again, not kept");
        assert_eq!(frame(1), 1);
        assert_eq!(submits.get(), 2, "the complete extraction is kept");
    }

    #[cfg(test)]
    #[test]
    fn kept_geometry_out_of_view_for_its_ticks_is_swept() {
        let mut kept = KeptGeometry::default();
        let (seen, gone) = ([0, 0, 0], [5, 0, 0]);
        let m = machine();
        kept.keep(
            seen,
            KEY,
            m,
            DEFAULT_BOUNDS,
            &[vertex(0.0)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            0,
        );
        kept.keep(
            gone,
            KEY,
            m,
            DEFAULT_BOUNDS,
            &[vertex(0.0)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            0,
        );
        kept.get(gone, KEY, m, 50);
        // A sweep at 100 keeps both: each was drawn in the last 100 ticks.
        kept.sweep(100);
        assert!(kept.entries.contains_key(&gone));
        assert!(kept.entries.contains_key(&seen));
        kept.get(seen, KEY, m, 150);
        // At 199 no sweep is due yet; at 200 the one last drawn at 50 goes.
        kept.sweep(199);
        assert!(kept.entries.contains_key(&gone));
        kept.sweep(200);
        assert!(!kept.entries.contains_key(&gone));
        assert!(kept.get(seen, KEY, m, 200).is_some());
    }

    #[cfg(test)]
    /// Ordinary (non-global) native block entities go through the same eye-section filter as
    /// built-in chests: from Y=111 looking down at render distance 2, Y=60 is excluded (fresh
    /// and cached), the inclusive edge Y=64 submits geometry, and global renderers at Y=60 stay.
    #[test]
    fn ordinary_native_block_entities_follow_eye_visible_sections() {
        use std::cell::Cell;
        use sunrise_client::gfx::{Frustum, JoinedFrusta, section_in_eye_view};

        let min_y = -64;
        let sections = 24;
        let rd = 2;
        let camera = DVec3::new(8.5, 111.0, 8.5);
        let proj = Mat4::perspective_rh(70.0_f32.to_radians(), 16.0 / 9.0, 192.0, 0.05);
        let view = sunrise_client::camera::view_matrix(0.0, 90.0);
        let frustum = JoinedFrusta::from(Frustum::from_view_projection(proj * view));

        let eye_visible = |pos: [i32; 3], cam: DVec3, view: &JoinedFrusta| {
            let key = (pos[0] >> 4, (pos[1] >> 4) - (min_y >> 4), pos[2] >> 4);
            section_in_eye_view(key, cam, min_y, sections, rd, Some(view), |_| true)
        };

        let outside_pos = [8, 60, 8];
        let edge_pos = [8, 64, 8];
        assert!(
            !eye_visible(outside_pos, camera, &frustum),
            "Y=60 is outside the vertical band from Y=111 at distance 2"
        );
        assert!(
            eye_visible(edge_pos, camera, &frustum),
            "Y=64 lies on the inclusive bottom edge of the band"
        );

        let mut kept = KeptGeometry::default();
        let revision = machine();
        let mut draw = draw();
        let submits = Cell::new(0);
        let render_at = |kept: &mut KeptGeometry,
                         draw: &mut BlockEntityDraw,
                         pos: [i32; 3],
                         cam: DVec3,
                         bounds: [f32; 6]| {
            draw.vertices.clear();
            let corner = DVec3::from_array(pos.map(f64::from));
            let origin = (corner - cam).as_vec3();
            let in_eye = eye_visible(pos, cam, &frustum);
            kept.render(
                pos,
                KEY,
                revision,
                draw,
                origin,
                &LevelLight {
                    origin: corner,
                    light_at: &|_| 0xF0,
                },
                None,
                &mut Vec::new(),
                || bounds,
                |b, _| mod_block_entity_visible(Some(&frustum), origin, b, in_eye, false),
                |draw| {
                    submits.set(submits.get() + 1);
                    draw.keep();
                    draw.quad(LIGHT_QUAD, [[0.0; 2]; 4], sprite(), 0xFFFFFF);
                },
            );
            draw.vertices.len()
        };

        // Non-global renderer at Y=60 never submits geometry from Y=111 looking down at rd=2.
        assert_eq!(
            render_at(&mut kept, &mut draw, outside_pos, camera, DEFAULT_BOUNDS),
            0
        );
        assert_eq!(submits.get(), 0);

        // Even after caching geometry while the camera was near Y=60, Y=111 at rd=2 hides it.
        let near_camera = DVec3::new(8.5, 65.0, 8.5);
        assert_eq!(
            render_at(
                &mut kept,
                &mut draw,
                outside_pos,
                near_camera,
                DEFAULT_BOUNDS
            ),
            4
        );
        assert_eq!(submits.get(), 1);
        assert_eq!(
            render_at(&mut kept, &mut draw, outside_pos, camera, DEFAULT_BOUNDS),
            0
        );
        assert_eq!(submits.get(), 1);

        // Non-global renderer at the inclusive edge Y=64 still submits geometry.
        assert_eq!(
            render_at(&mut kept, &mut draw, edge_pos, camera, DEFAULT_BOUNDS),
            4
        );
        assert_eq!(submits.get(), 2);

        // Global renderer (infinite bounds) at Y=60 bypasses the eye-section filter.
        let global_pos = [9, 60, 8];
        let infinite = [
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::INFINITY,
            f32::INFINITY,
        ];
        assert_eq!(
            render_at(&mut kept, &mut draw, global_pos, camera, infinite),
            4
        );
        assert_eq!(submits.get(), 3);
    }

    #[cfg(test)]
    const CORNERS: [[f32; 3]; 4] = [
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
    ];
    #[cfg(test)]
    const UNIT_UV: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];

    /// Kept geometry marked with `crumbling` shows the crack as it is each frame without a new
    /// submit: it starts, advances to another stage sprite, turns with `spin` and clears, and
    /// only the marked part is covered.
    #[cfg(test)]
    #[test]
    fn kept_crumbling_follows_the_stage_and_the_spin_without_a_new_submit() {
        use std::cell::Cell;
        use sunrise_client::breaking::stage_light;

        const TURNING: [[f32; 3]; 4] = [
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        let pos = [1, 2, 3];
        let mut kept = KeptGeometry::default();
        let mut draw = draw();
        let revision = machine();
        let submits = Cell::new(0);
        let seen = Cell::new(None);
        let mut frame = |tick: f64, crack: Option<(u8, Option<[f32; 2]>)>| {
            draw.view =
                mod_api::WorldRenderView::new(&DEFAULT_DRAW_CAMERA, None, 0.0, 0, tick, false, &[]);
            draw.vertices.clear();
            let mut overlay = Vec::new();
            kept.render(
                pos,
                KEY,
                revision,
                &mut draw,
                Vec3::ZERO,
                &LevelLight {
                    origin: DVec3::from_array(pos.map(f64::from)),
                    light_at: &|_| 0xF0,
                },
                crack,
                &mut overlay,
                || DEFAULT_BOUNDS,
                |_, _| true,
                |draw| {
                    submits.set(submits.get() + 1);
                    seen.set(draw.crumbling_stage);
                    draw.keep();
                    draw.quad(LIGHT_QUAD, UNIT_UV, sprite(), 0xFFFFFF);
                    draw.crumbling(true);
                    draw.spin([0.0, 1.0, 0.0], [0.5; 3], 90.0, 0.0);
                    draw.quad(TURNING, UNIT_UV, sprite(), 0xFFFFFF);
                    draw.crumbling(false);
                    draw.still();
                    draw.quad(LIGHT_QUAD, UNIT_UV, sprite(), 0xFFFFFF);
                },
            );
            (draw.vertices.clone(), overlay)
        };
        let positions = |vertices: &[Vertex]| vertices.iter().map(|v| v.pos).collect::<Vec<_>>();

        // Submitted once, with the stage the frame gave the block.
        let (vertices, overlay) = frame(0.0, Some((2, Some([0.25, 0.5]))));
        assert_eq!((submits.get(), seen.get()), (1, Some(2)));
        assert_eq!(vertices.len(), 12);
        assert_eq!(overlay.len(), 4, "only the part drawn under crumbling");
        assert!(overlay.iter().all(|v| {
            v.layer == 11 && v.light == stage_light([0.25, 0.5]) && v.color == 0xFFFF_FFFF
        }));
        let at_rest = positions(&vertices[4..8]);
        assert_eq!(positions(&overlay), at_rest);

        // The stage advances: no new submit, the new stage's sprite, and the part's turn.
        let (vertices, overlay) = frame(1.0, Some((3, Some([0.5, 0.5]))));
        assert_eq!(submits.get(), 1);
        assert_eq!(overlay.len(), 4);
        assert!(overlay.iter().all(|v| v.light == stage_light([0.5, 0.5])));
        let turned = positions(&vertices[4..8]);
        assert_ne!(turned, at_rest, "the part turned with the clock");
        assert_eq!(positions(&overlay), turned, "and the crack with it");

        // The crack clears, then starts again, still with no new submit.
        let (vertices, overlay) = frame(1.0, None);
        assert_eq!((submits.get(), vertices.len(), overlay.len()), (1, 12, 0));
        let (_, overlay) = frame(2.0, Some((9, Some([0.0, 0.5]))));
        assert_eq!((submits.get(), overlay.len()), (1, 4));
        // A stage whose sprite the atlas lacks draws no crack.
        let (_, overlay) = frame(2.0, Some((4, None)));
        assert!(overlay.is_empty());
    }

    /// A prepared model recorded between two crumbling draws is spliced in at its record: the
    /// range the two draws merged into splits around it, so the model is not cracked and both
    /// quads are.
    #[cfg(test)]
    #[test]
    fn a_deferred_model_between_crumbling_draws_splits_the_merged_range() {
        let quad = bake_flat_face(
            [0.0; 3],
            [16.0; 3],
            [0.0, 0.0, 16.0, 16.0],
            Direction::South,
            Material {
                sprite: String::new(),
                force_translucent: false,
            },
            -1,
        )
        .unwrap();
        let slots = vec![Some(vec![
            crate::game::client_side::minecarts_client::PreparedModelFace::new(&quad, sprite()),
        ])];
        let mut d = draw();
        d.prepared = true;
        d.crumbling(true);
        d.quad(CORNERS, UNIT_UV, sprite(), 0xFFFFFF);
        d.crumbling(false);
        assert!(d.defer_model(0, 1, u32::MAX));
        d.crumbling(true);
        d.quad(CORNERS, UNIT_UV, sprite(), 0xFFFFFF);
        assert_eq!(d.crumbled, [(0, 8)], "the two quads' ranges join");
        d.bake_recorded(&slots);
        assert_eq!(d.vertices.len(), 12);
        assert_eq!(
            d.crumbled,
            [(0, 4), (8, 12)],
            "the model at 4..8 is left out"
        );
    }

    /// `crumbling` lasts to the end of one block entity's draw: the next one drawn through the
    /// same scratch draw is not covered unless it turns it on.
    #[cfg(test)]
    #[test]
    fn crumbling_ends_with_the_block_entity_that_turned_it_on() {
        let level = LevelLight {
            origin: DVec3::ZERO,
            light_at: &|_| 0,
        };
        let mut kept = KeptGeometry::default();
        let mut draw = draw();
        let mut overlay = Vec::new();
        for (pos, crumbling) in [([0, 64, 0], true), ([1, 64, 0], false)] {
            kept.render(
                pos,
                KEY,
                machine(),
                &mut draw,
                Vec3::ZERO,
                &level,
                Some((1, Some([0.0, 0.0]))),
                &mut overlay,
                || DEFAULT_BOUNDS,
                |_, _| true,
                |draw| {
                    if crumbling {
                        draw.crumbling(true);
                    }
                    draw.quad(LIGHT_QUAD, UNIT_UV, sprite(), 0xFFFFFF);
                },
            );
        }
        assert_eq!(draw.vertices.len(), 8);
        assert_eq!(overlay.len(), 4, "the first block's quad alone");
    }

    /// A carried block entity's submit draws one level deep: inside it `refuse_nested` refuses,
    /// and the pose and the guard are back as they were once it ends.
    #[cfg(test)]
    #[test]
    fn a_carried_submit_refuses_a_nested_one_and_restores_the_pose() {
        let mut d = draw();
        let pose = d.pose();
        assert_eq!(d.refuse_nested(), Ok(()));
        let mut inside = None;
        d.one_level(|d| {
            d.translate([1.0, 2.0, 3.0]);
            inside = Some(d.refuse_nested());
        });
        assert_eq!(inside, Some(Err(mod_api::BlockEntityDrawRefusal::Nested)));
        assert_eq!(d.refuse_nested(), Ok(()));
        assert_eq!(d.pose(), pose);
    }

    pub fn test_crack_overlay(
        placed: &[Vertex],
        ranges: &[(usize, usize)],
        origin: Vec3,
        sprite: [f32; 2],
        overlay: &mut Vec<Vertex>,
    ) {
        crack_overlay(placed, ranges, origin, sprite, overlay);
    }
}

#[cfg(any(test, feature = "test-support"))]
impl BlockEntityDraw<'static> {
    pub fn test_posed(
        partial: f32,
        game_time: i64,
        light: u32,
        block: [i32; 3],
        pose: Mat4,
        lights: (Vec3, Vec3),
        vertices: Vec<Vertex>,
    ) -> Self {
        Self::posed(partial, game_time, light, block, pose, lights, vertices)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl BlockEntityDraw<'_> {
    pub fn test_set_crumbling_stage(&mut self, stage: Option<u8>) {
        self.crumbling_stage = stage;
    }
    pub fn test_crumbling(&mut self, on: bool) {
        self.crumbling(on);
    }
    pub fn test_crumbled(&self) -> &[(usize, usize)] {
        &self.crumbled
    }
    pub fn test_vertices(&self) -> &[Vertex] {
        &self.vertices
    }
    pub fn test_set_pose_light(&mut self, pose: Mat4, light: u32) {
        self.pose = pose;
        self.light = light;
    }
    pub fn test_into_vertices_raw(self) -> Vec<Vertex> {
        self.vertices
    }
}

#[cfg(any(test, feature = "test-support"))]
impl crate::game::ClientGame<'_> {
    pub fn test_with_draw_context<R>(
        &self,
        surface: &mut BlockEntityDraw<'_>,
        f: impl FnOnce(&mut dyn mod_api::BlockEntityDrawContext) -> R,
    ) -> R {
        let mut drawn = crate::game::host::draw::GameBlockEntityDrawContext {
            game: self,
            surface,
        };
        f(&mut drawn)
    }
    pub fn test_native_draw_block(
        &self,
        draw: &mut BlockEntityDraw<'_>,
        state: BlockStateId,
    ) -> bool {
        self.native_draw_block(draw, state)
    }
    pub fn test_native_draw_crumbling_stage(&self, draw: &BlockEntityDraw<'_>) -> Option<u8> {
        self.native_draw_crumbling_stage(draw)
    }
}
