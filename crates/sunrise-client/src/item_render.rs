//! Item rendering through the resolved item model pipeline: the
//! `ItemModelResolver` of `sunrise-assets` fills an [`ItemRenderState`]
//! (`ItemStackRenderState`: layers of baked quads or special renderers, each
//! with its display transform, local transform, tints and foil), and
//! [`ItemRenderState::submit`] turns it into the quads `ItemFeatureRenderer`
//! and the special model renderers submit, in pose space. A sink then draws
//! them: [`gui`] into the HUD batch with the GUI item lighting, [`WorldSink`]
//! into entity vertices with the level's lighting.
use sunrise_assets::resource_source::{ResourceRoot, ResourceSource};
#[cfg(test)]
mod block_light_tests;
pub mod cit;
mod glint_atlas;
#[cfg(test)]
mod glint_tests;
pub mod gui;
mod registered_providers;
mod registered_special;
mod special;
#[cfg(test)]
mod special_parts_tests;
#[cfg(test)]
mod tint_tests;

use crate::{
    block_entity_render::{ModelPart, diffuse},
    hud::Rect,
    mesher::{CULL_BACK, Vertex},
};
use glam::{Mat3, Mat4, Vec3};
use mod_api::{ClientHost, ItemDisplayContext, ItemKey, SpecialModelContext, SpecialModelTints};
pub use registered_providers::{
    BoundItemModel, BoundRangeProperty, BoundSelectProperty, BoundTintSource, ItemModelFactories,
    ItemProviderEntry, ItemProviderTables, ItemTintFactories, RangePropertyFactories,
    SelectPropertyFactories, bind_item_model_factories, bind_item_tint_factories,
    bind_range_property_factories, bind_select_property_factories, decode_item_model,
    decode_range_property, decode_select_property, decode_tint_source,
    install_item_model_factories, install_item_tint_factories, install_range_property_factories,
    install_select_property_factories,
};
pub use registered_special::{
    BakedSpecialGeometry, OwnedSpecialModelFactory, SpecialModelFactories,
    install_special_model_factories,
};
use registered_special::{FACTORIES, StackAncestor};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    num::NonZeroU32,
    ops::ControlFlow,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};
use sunrise_assets::{
    item_bake::{BakedItemModel, SpriteLookup, SpriteMask, bake_item_model, bake_sprite_layer},
    item_model::{
        ClientItem, Context, DisplayContext, FoilType, ItemEnv, ItemModels, Layer, LayerContent,
        LayerTransform, LevelState, OwnerState, SpecialModel, StackView,
    },
    model::Direction,
    resolve::{MISSING_MODEL, ModelLibrary},
};
pub use sunrise_sim::item_models::{stack_view, tag_json};

/// One quad as submitted for drawing: pose-space corners, atlas UVs, the
/// transformed unit normal, ARGB colour (tint, `-1` white) and its glint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemQuad {
    pub pos: [Vec3; 4],
    pub uv: [[f32; 2]; 4],
    /// The model UVs `GLINT_TEXTURING` scrolls for the glint (vanilla's UV0,
    /// or UV3 for a special model).
    pub foil_uv: [[f32; 2]; 4],
    /// Glint UV override (vanilla atlas UV for standard/shield foil, decal UV3 for special foil);
    /// CIT and raw entity models keep sprite-local `foil_uv`.
    pub gui_glint_uv: Option<[[f32; 2]; 4]>,
    pub normal: Vec3,
    pub color: u32,
    pub foil: Foil,
    /// True for entity-model special items (`Trident`, `Shield`) using `ENTITY_GLINT_TEXTURING`.
    pub entity_glint: bool,
    /// `MaterialInfo.lightEmission` (block light floor).
    pub light_emission: u8,
    /// The render type culls back faces (`entitySolid`): world drawing discards them.
    pub cull_back: bool,
    /// The quad's sprite has translucent pixels (`MaterialInfo.itemRenderType`
    /// is a translucent sheet): drawn with the translucent entities.
    pub translucent: bool,
    /// The quad's render type has an `OitPipelineSet` (`entityTranslucent`: a
    /// player head's skin): improved transparency draws it in its stages.
    pub oit: bool,
    /// `RenderTypes.bannerPattern`: translucent blend without depth writes or alpha cutout.
    pub banner_pattern: bool,
    /// Optional explicit packed light coordinates in vanilla `LightCoordsUtil` layout
    /// (block byte in bits `0..8`, sky byte in bits `16..24`). `None` inherits draw light and
    /// `light_emission`.
    pub packed_light: Option<u32>,
    /// Skip directional diffuse face shading while still sampling both lightmap channels.
    pub lightmap_only: bool,
    /// Explicit item render type override (`ItemCutout` / `ItemTranslucent`), using vanilla's
    /// `0.1` alpha cutoff in GUI draws and sorting cutout quads before translucent ones.
    pub explicit_alpha_cutoff: bool,
}
/// `TextureTransform.setupGlintTexturing`'s inputs.
#[derive(Clone, Copy, Debug)]
pub struct Glint {
    /// `options.glintSpeed` (vanilla default 0.5).
    pub speed: f64,
    /// `options.glintStrength` / shader `GlintAlpha` (vanilla default 0.75).
    pub alpha: f32,
    /// `Util.getMillis()`.
    pub millis: i64,
    /// The transform's scale: `GLINT_TEXTURING`'s 8.0 for cuboid items (GUI and world),
    /// `ENTITY_GLINT_TEXTURING`'s 0.5 for entity-model special items (`Trident`, `Shield`), and
    /// `ARMOR_ENTITY_GLINT_TEXTURING`'s 0.16 for armour (entity.wgsl).
    pub scale: f32,
}
impl Glint {
    pub const ITEM_SCALE: f32 = 8.0;
    pub const ENTITY_SCALE: f32 = 0.5;
    pub const ARMOR_SCALE: f32 = 0.16;

    /// Select vanilla's item (`8.0`) or entity (`0.5`) texturing scale for a quad.
    pub fn for_quad(self, entity_glint: bool) -> Self {
        Self {
            scale: if entity_glint {
                Self::ENTITY_SCALE
            } else {
                Self::ITEM_SCALE
            },
            ..self
        }
    }
    /// `Util.getMillis() * glintSpeed * 8.0` as vanilla's long.
    fn ticks(millis: i64, speed: f64) -> i64 {
        (millis as f64 * speed * 8.0) as i64
    }
    /// The scroll clock modulo the common period of both offsets (110000 and
    /// 30000 divide 330000), for `globals.time.w`: the entity shader derives
    /// `ARMOR_ENTITY_GLINT_TEXTURING` from it.
    pub fn scroll_ticks(millis: i64, speed: f64) -> f32 {
        (Self::ticks(millis, speed) % 330_000) as f32
    }
    /// `TextureTransform.setupGlintTexturing(scale)` applied to the model UV:
    /// translation(-o0, o1) * rotateZ(PI/18) * scale. The result repeats:
    /// the shader samples the dedicated glint texture with vanilla's
    /// repeating `GlintSampler`, so the UVs stay unwrapped.
    pub fn uv(&self, model: [f32; 2]) -> [f32; 2] {
        let ticks = Self::ticks(self.millis, self.speed);
        let offset0 = (ticks % 110_000) as f32 / 110_000.0;
        let offset1 = (ticks % 30_000) as f32 / 30_000.0;
        // Compile-time `(float)(Math.PI / 18.0)` in `TextureTransform.setupGlintTexturing`.
        const ANGLE: f32 = f32::from_bits(0x3e32_b8c2);
        let sin = crate::camera::joml::sin(ANGLE);
        let cos = crate::camera::joml::cos_from_sin(sin, ANGLE);
        let (m00, m01) = (cos * self.scale, sin * self.scale);
        let (m10, m11) = (-sin * self.scale, cos * self.scale);
        // JOML `Vector4f.mulAffine`: right-associated unfused sums; `0.0 + -offset0` drops -0.0.
        [
            m00 * model[0] + (m10 * model[1] + (0.0 + -offset0)),
            m01 * model[0] + (m11 * model[1] + (0.0 + offset1)),
        ]
    }
}
/// The glint inputs of one frame, read once: `GameRenderer.extract` snapshots
/// `glintSpeed` and `glintStrength` into `OptionsRenderState`
/// (`extractOptions`), and `TextureTransform.setupGlintTexturing` reads
/// `Util.getMillis()` once per glint render-type batch, not per item.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlintFrame {
    /// `Util.getMillis()` this frame.
    pub millis: i64,
    /// `OptionsRenderState.glintSpeed`.
    pub speed: f64,
    /// `OptionsRenderState.glintStrength`.
    pub alpha: f32,
    /// The atlas has `misc/enchanted_glint_item`.
    pub sprite: bool,
}
impl GlintFrame {
    /// The glint at a texturing scale, if the atlas has its sprite.
    pub fn glint(&self, scale: f32) -> Option<Glint> {
        self.sprite.then_some(Glint {
            speed: self.speed,
            alpha: self.alpha,
            millis: self.millis,
            scale,
        })
    }
    /// [`Glint::scroll_ticks`] for this frame.
    pub fn scroll_ticks(&self) -> f32 {
        Glint::scroll_ticks(self.millis, self.speed)
    }
}
/// Which glint render type a quad uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Foil {
    None,
    /// `GLINT_TEXTURING` over the sprite's UVs (`itemGlintRenderType`,
    /// `entitySolidGlint`).
    Standard,
    /// `itemGlintSpecialRenderType`: compasses and clocks, a sheeted decal.
    Special,
    /// `RenderTypes.patternedShieldGlint`: equal-depth glint-only pass drawn once after patterns.
    GlintOnly,
}
/// Where submitted quads go.
pub trait ItemSink {
    fn quad(&mut self, quad: ItemQuad);
}
impl ItemSink for Vec<ItemQuad> {
    fn quad(&mut self, quad: ItemQuad) {
        self.push(quad);
    }
}

/// Client drawing services in addition to the asset resolver's host services.
pub trait ItemDrawEnv: ItemEnv {
    /// The host and validated input for a registered special draw. The resolver reports
    /// unavailable input and refuses this layer only, not the renderer.
    fn special_model_input(
        &self,
        _stack: &StackView,
        _context: &Context<'_>,
    ) -> Option<SpecialModelInput<'_>> {
        None
    }

    /// Resolve a mod-submitted render type against the current resource load.
    fn render_type(
        &self,
        _handle: mod_api::RenderTypeRef,
    ) -> Result<
        sunrise_client_render::mod_pipelines::ResolvedRenderType<'static>,
        mod_api::RenderTypeError,
    > {
        Err(mod_api::RenderTypeError::Unknown)
    }

    /// Resolve a mod-submitted stack and its display context at the engine boundary.
    fn special_model_stack(
        &self,
        _stack: &mod_api::ItemStack,
        _display: ItemDisplayContext,
    ) -> Option<(StackView, DisplayContext)> {
        None
    }
}

/// The game services and stack identity the special renderer reads during this draw.
pub struct SpecialModelInput<'a> {
    pub host: &'a dyn ClientHost,
    pub item: ItemKey,
    pub count: NonZeroU32,
    pub context: SpecialModelContext,
}

impl ItemDrawEnv for sunrise_assets::item_model::NoEnv {}

/// Data special renderers read from the game (registries, skins).
pub trait SpecialHost {
    /// The banner atlas texture of a banner pattern (`entity/banner/<asset>`).
    fn banner_texture(&self, pattern: &str) -> Option<String>;
    /// The shield atlas texture of a banner pattern (`entity/shield/<asset>`).
    fn shield_texture(&self, pattern: &str) -> Option<String>;
    /// The skin texture of a `minecraft:profile` component, if loaded.
    fn player_skin(&self, _profile: &Value) -> Option<String> {
        None
    }
}
/// A host with no registries: patterns and skins fall back to defaults.
pub struct NoHost;
impl SpecialHost for NoHost {
    fn banner_texture(&self, _: &str) -> Option<String> {
        None
    }
    fn shield_texture(&self, _: &str) -> Option<String> {
        None
    }
}

/// The geometry of one layer.
#[derive(Clone, Debug)]
pub enum LayerGeometry {
    Quads(Arc<BakedItemModel>),
    Special {
        model: SpecialModel,
        argument: Value,
    },
}
/// Tints computed for an ordinary layer, or the palette supplied by a special renderer.
#[derive(Clone, Debug)]
pub enum LayerTints {
    Owned(Vec<i32>),
    Special(SpecialModelTints),
}
impl LayerTints {
    fn at(&self, index: i32) -> u32 {
        match self {
            // `getLayerColorSafe`: an untinted quad or a missing palette entry draws white.
            Self::Owned(tints) => tints
                .get(index as usize)
                .map_or(u32::MAX, |&tint| argb(tint)),
            Self::Special(SpecialModelTints::Uniform(tint)) => *tint,
            Self::Special(SpecialModelTints::Indexed(tints)) => {
                tints.get(index as usize).copied().unwrap_or(u32::MAX)
            }
            Self::Special(_) => u32::MAX,
        }
    }
}
/// `ItemStackRenderState.LayerRenderState`.
#[derive(Clone, Debug)]
pub struct LayerRender {
    pub geometry: LayerGeometry,
    pub tints: LayerTints,
    pub foil: FoilType,
    /// `applyTransform`: the display transform, then the local transform.
    pub matrix: Mat4,
    /// `particleMaterial` (atlas sprite name).
    pub particle: Arc<str>,
    pub uses_block_light: bool,
    /// Untransformed extents (quad corners or the special model's parts).
    pub extents: Arc<Vec<Vec3>>,
    /// The display context the layer was resolved for, which its submission follows (a GUI
    /// glint's atlas coordinates): the state's own, except for the layers of a nested stack.
    pub display: DisplayContext,
    /// Per-submission packed light, render type, and lighting mode from
    /// `SpecialModelCollector::submit_model` (defaults for ordinary item layers).
    pub submit_options: mod_api::SpecialModelSubmitOptions,
    /// The render type `submit_options` named through `SpecialModelRenderType::Custom`,
    /// resolved when the layer was submitted.
    pub render_type: Option<sunrise_client_render::mod_pipelines::ResolvedRenderType<'static>>,
}
/// `ItemStackRenderState`.
#[derive(Clone, Debug)]
pub struct ItemRenderState {
    pub display: DisplayContext,
    pub layers: Vec<LayerRender>,
    pub oversized_in_gui: bool,
    pub animated: bool,
    /// The OptiFine CIT glint layers the stack draws in the GUI (`cit_glint`).
    pub cit_glint: Option<Arc<crate::cit_glint::StackGlint>>,
    /// HDR output's tier of an item that glows (`dynamic_light::Luminance::hdr_tier`).
    pub hdr_tier: u8,
    /// In HDR output, the block light of an item that glows: its block's, as a placed one has
    /// it, so a torch doesn't go dark in the shade while it shines. 0 for the rest.
    pub hdr_light: u8,
    /// A third-person held-item submission's opaque ARGB multiplier, never cached with the model.
    held_tint: Option<u32>,
}

#[derive(Default)]
struct RenderLayerFlags {
    oversized_in_gui: bool,
    animated: bool,
}

impl ItemRenderState {
    /// Apply a held-item Part's RGB answer to this draw only, preserving model alpha.
    pub fn set_held_tint(&mut self, rgb: u32) {
        self.held_tint = Some(rgb | 0xFF00_0000);
    }
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
    /// `usesBlockLight` of the first layer (GUI lighting choice).
    pub fn uses_block_light(&self) -> bool {
        self.layers.first().is_some_and(|l| l.uses_block_light)
    }
    /// `getModelBoundingBox`: the transformed extents of every layer, as
    /// (min, max); a zero box at the origin when there are none. Each extent is
    /// `Vector3f.mulPosition`'s (JOML's `Math.fma` unfused, summed from the right).
    pub fn bounding_box(&self) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for layer in &self.layers {
            let [c0, c1, c2, c3] = layer.matrix.to_cols_array_2d();
            for p in layer.extents.iter() {
                let q = Vec3::from_array(std::array::from_fn(|row| {
                    c0[row] * p.x + (c1[row] * p.y + (c2[row] * p.z + c3[row]))
                }));
                lo = lo.min(q);
                hi = hi.max(q);
            }
        }
        if lo.x > hi.x {
            (Vec3::ZERO, Vec3::ZERO)
        } else {
            (lo, hi)
        }
    }
    /// Bounds of the actual submitted item geometry under each world pose.
    /// Used for dropped-item occlusion because its collision box omits the
    /// render bob, model size, and copies in a stacked item cluster.
    pub fn posed_bounds(&self, poses: &[Mat4]) -> Option<(Vec3, Vec3)> {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for &pose in poses {
            for layer in &self.layers {
                let transform = pose * layer.matrix;
                for &point in layer.extents.iter() {
                    let point = transform.transform_point3(point);
                    if !point.is_finite() {
                        return None;
                    }
                    lo = lo.min(point);
                    hi = hi.max(point);
                }
            }
        }
        (lo.x <= hi.x && lo.y <= hi.y && lo.z <= hi.z).then_some((lo, hi))
    }
    /// `ItemStackRenderState.submit` under `pose`.
    pub fn submit(
        &self,
        renderer: &ItemRenderer,
        sprites: &HashMap<String, Rect>,
        host: &dyn SpecialHost,
        pose: Mat4,
        sink: &mut dyn ItemSink,
    ) {
        let mut tinted;
        let sink: &mut dyn ItemSink = if let Some(tint) = self.held_tint {
            tinted = TintedItemSink { sink, tint };
            &mut tinted
        } else {
            sink
        };
        for layer in &self.layers {
            let m = pose * layer.matrix;
            match &layer.geometry {
                LayerGeometry::Quads(baked) => {
                    submit_quads(renderer, baked, layer, m, sprites, sink);
                }
                LayerGeometry::Special { model, argument } => special::submit(
                    renderer,
                    model,
                    argument,
                    layer.foil != FoilType::None,
                    m,
                    sprites,
                    host,
                    sink,
                ),
            }
        }
    }
    /// [`Self::submit`] preserving JOML `Matrix4f.mulAffine` and `Vector3f.mulPosition`
    /// rounding through baked quad submission (`DisplayRenderer.ItemDisplayRenderer.submitInner`).
    pub fn submit_joml(
        &self,
        renderer: &ItemRenderer,
        sprites: &HashMap<String, Rect>,
        host: &dyn SpecialHost,
        pose: crate::camera::joml::Mat4,
        sink: &mut dyn ItemSink,
    ) {
        let mut tinted;
        let sink: &mut dyn ItemSink = if let Some(tint) = self.held_tint {
            tinted = TintedItemSink { sink, tint };
            &mut tinted
        } else {
            sink
        };
        for layer in &self.layers {
            let m_joml = pose.mul_affine(&crate::camera::joml::Mat4::affine(layer.matrix));
            match &layer.geometry {
                LayerGeometry::Quads(baked) => {
                    submit_quads_joml(renderer, baked, layer, m_joml, sprites, sink);
                }
                LayerGeometry::Special { model, argument } => special::submit(
                    renderer,
                    model,
                    argument,
                    layer.foil != FoilType::None,
                    m_joml.to_glam(),
                    sprites,
                    host,
                    sink,
                ),
            }
        }
    }
    /// [`Self::submit`] into a [`WorldSink`] of `light`, `lights` and `tier`,
    /// posed on the GPU (`gfx::entity_skin`): the same vertices in the same
    /// order. False, with nothing drawn, when a layer cannot take that path: a
    /// special model, a glint, or a quad that glows or is translucent (another
    /// material).
    pub fn submit_skinned(
        &self,
        renderer: &ItemRenderer,
        sprites: &HashMap<String, Rect>,
        pose: Mat4,
        (light, lights, tier): (u32, Option<(Vec3, Vec3)>, u8),
        skin: &mut crate::mobs::skin::Skin,
    ) -> bool {
        use crate::gfx::entity_skin::{SkinJob, SkinPart};
        // The axes' order in a stored vertex (`SkinVertex::axis`).
        const AXES: [Direction; 6] = [
            Direction::East,
            Direction::Up,
            Direction::South,
            Direction::West,
            Direction::Down,
            Direction::North,
        ];
        for layer in &self.layers {
            let LayerGeometry::Quads(baked) = &layer.geometry else {
                return false;
            };
            if layer.foil != FoilType::None || matches!(&layer.tints, LayerTints::Special(_)) {
                return false;
            }
            let Some(stored) = skin.models.item(baked, skin.frame) else {
                return false;
            };
            let translucent = |run: &crate::mobs::skin::ItemRun| {
                let material = &baked.quads[run.quad].material;
                material.force_translucent || renderer.sprites.translucent(&material.sprite)
            };
            if stored.runs.iter().any(translucent) {
                return false;
            }
        }
        for layer in &self.layers {
            let LayerGeometry::Quads(baked) = &layer.geometry else {
                continue;
            };
            let m = pose * layer.matrix;
            let mut normals = FaceNormals::new(m);
            let Some(stored) = skin.models.item(baked, skin.frame) else {
                continue;
            };
            for run in &stored.runs {
                // `submit_quads`' sprite and tint, once for the run's quads.
                let q = &baked.quads[run.quad];
                let Some(r) = sprites
                    .get(sprite_name(&q.material.sprite))
                    .or_else(|| sprites.get("misc/missingno"))
                    .or_else(|| sprites.get("sunrise/missingno"))
                else {
                    continue;
                };
                let color = layer.tints.at(run.tint_index);
                let color = self
                    .held_tint
                    .map_or(color, |tint| crate::hud_layout::multiply(color, tint));
                let colors = AXES.map(|d| shaded(color, normals.face(d), lights));
                let frame = &mut *skin.frame;
                frame.jobs.push(SkinJob {
                    first: run.first,
                    count: run.count,
                    part: frame.parts.len() as u32,
                    dst: frame.vertices,
                    rect: [r.u0, r.v0, r.u1, r.v1],
                    light,
                    overlay: u32::from(tier) << 16,
                    mode: SkinJob::ITEM,
                    ..Default::default()
                });
                frame.parts.push(SkinPart::item(m, colors));
                frame.vertices += run.count;
            }
            let bounds = stored.bounds;
            skin.grow_bounds(m, bounds);
        }
        true
    }
}

/// Multiplies every submitted quad's RGB, including special models, before the world sink lights it.
struct TintedItemSink<'a> {
    sink: &'a mut dyn ItemSink,
    tint: u32,
}
impl ItemSink for TintedItemSink<'_> {
    fn quad(&mut self, mut quad: ItemQuad) {
        if quad.foil != Foil::GlintOnly {
            quad.color = crate::hud_layout::multiply(quad.color, self.tint);
        }
        self.sink.quad(quad);
    }
}

/// The atlas name of a sprite identifier (`minecraft:item/apple` -> `item/apple`).
pub fn sprite_name(sprite: &str) -> &str {
    sprite.strip_prefix("minecraft:").unwrap_or(sprite)
}
/// `Direction.getUnitVec3f`.
pub fn direction_vector(d: Direction) -> Vec3 {
    match d {
        Direction::Down => Vec3::NEG_Y,
        Direction::Up => Vec3::Y,
        Direction::North => Vec3::NEG_Z,
        Direction::South => Vec3::Z,
        Direction::West => Vec3::NEG_X,
        Direction::East => Vec3::X,
    }
}
/// `PoseStack.Pose.transformNormal`: the normal matrix, renormalized.
pub fn transform_normal(m: Mat4, n: Vec3) -> Vec3 {
    FaceNormals::new(m).transform(n)
}
/// [`transform_normal`] of one pose for many quads: its normal matrix inverted once, and
/// the normal of each of the six facings worked out the first time a quad uses it.
pub struct FaceNormals {
    normal: Mat3,
    faces: [Option<Vec3>; 6],
}
impl FaceNormals {
    pub fn new(m: Mat4) -> Self {
        Self {
            normal: Mat3::from_mat4(m).inverse().transpose(),
            faces: [None; 6],
        }
    }
    fn transform(&self, n: Vec3) -> Vec3 {
        (self.normal * n).normalize_or_zero()
    }
    /// The pose's normal of a face pointing `d`.
    pub fn face(&mut self, d: Direction) -> Vec3 {
        let i = d as usize;
        if let Some(n) = self.faces[i] {
            return n;
        }
        let n = self.transform(direction_vector(d));
        self.faces[i] = Some(n);
        n
    }
}
/// An `ARGB` int as the `u32` colour of an [`ItemQuad`].
pub fn argb(color: i32) -> u32 {
    color as u32
}

fn joml_transform_pos(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    [
        m[0] * p[0] + (m[4] * p[1] + (m[8] * p[2] + m[12])),
        m[1] * p[0] + (m[5] * p[1] + (m[9] * p[2] + m[13])),
        m[2] * p[0] + (m[6] * p[1] + (m[10] * p[2] + m[14])),
    ]
}

/// JOML 1.10.9 `Matrix4f.invertAffine` when `MatrixUtil.mulComponentWise` was not called.
fn joml_invert_affine(m: &[f32; 16]) -> [f32; 16] {
    let [
        m00,
        m01,
        m02,
        _,
        m10,
        m11,
        m12,
        _,
        m20,
        m21,
        m22,
        _,
        m30,
        m31,
        m32,
        _,
    ] = *m;
    let t2 = m00 * m11;
    let t3 = m01 * m10;
    let t4 = m02 * m10;
    let t5 = m00 * m12;
    let t6 = m01 * m12;
    let t7 = m02 * m11;
    let det = (t2 - t3) * m22 + (t4 - t5) * m21 + (t6 - t7) * m20;
    let inv = 1.0 / det;
    let t10 = m10 * m22;
    let t11 = m10 * m21;
    let t12 = m11 * m22;
    let t13 = m11 * m20;
    let t14 = m12 * m21;
    let t15 = m12 * m20;
    let t16 = m20 * m02;
    let t17 = m20 * m01;
    let t18 = m21 * m02;
    let t19 = m21 * m00;
    let t20 = m22 * m01;
    let t21 = m22 * m00;
    let nm30 = (t10 * m31 - t11 * m32 + t13 * m32 - t12 * m30 + t14 * m30 - t15 * m31) * inv;
    let nm31 = (t16 * m31 - t17 * m32 + t19 * m32 - t18 * m30 + t20 * m30 - t21 * m31) * inv;
    let nm32 = (t7 * m30 - t6 * m30 + t5 * m31 - t4 * m31 + t3 * m32 - t2 * m32) * inv;
    [
        (t12 - t14) * inv,
        (t18 - t20) * inv,
        (t6 - t7) * inv,
        0.0,
        (t15 - t10) * inv,
        (t21 - t16) * inv,
        (t4 - t5) * inv,
        0.0,
        (t11 - t13) * inv,
        (t17 - t19) * inv,
        (t2 - t3) * inv,
        0.0,
        nm30,
        nm31,
        nm32,
        1.0,
    ]
}

/// JOML 1.10.9 `Matrix4f.invertGenericThis` after `MatrixUtil.mulComponentWise` clears properties.
fn joml_invert_generic(m: &[f32; 16]) -> [f32; 16] {
    let [
        m00,
        m01,
        m02,
        m03,
        m10,
        m11,
        m12,
        m13,
        m20,
        m21,
        m22,
        m23,
        m30,
        m31,
        m32,
        m33,
    ] = *m;
    let a = m00 * m11 - m01 * m10;
    let b = m00 * m12 - m02 * m10;
    let c = m00 * m13 - m03 * m10;
    let d = m01 * m12 - m02 * m11;
    let e = m01 * m13 - m03 * m11;
    let f = m02 * m13 - m03 * m12;
    let g = m20 * m31 - m21 * m30;
    let h = m20 * m32 - m22 * m30;
    let i = m20 * m33 - m23 * m30;
    let j = m21 * m32 - m22 * m31;
    let k = m21 * m33 - m23 * m31;
    let l = m22 * m33 - m23 * m32;
    let det = a * l - b * k + c * j + d * i - e * h + f * g;
    let inv = 1.0 / det;
    [
        (m11 * l + (-m12 * k + m13 * j)) * inv,
        (-m01 * l + (m02 * k + -m03 * j)) * inv,
        (m31 * f + (-m32 * e + m33 * d)) * inv,
        (-m21 * f + (m22 * e + -m23 * d)) * inv,
        (-m10 * l + (m12 * i + -m13 * h)) * inv,
        (m00 * l + (-m02 * i + m03 * h)) * inv,
        (-m30 * f + (m32 * c + -m33 * b)) * inv,
        (m20 * f + (-m22 * c + m23 * b)) * inv,
        (m10 * k + (-m11 * i + m13 * g)) * inv,
        (-m00 * k + (m01 * i + -m03 * g)) * inv,
        (m30 * e + (-m31 * c + m33 * a)) * inv,
        (-m20 * e + (m21 * c + -m23 * a)) * inv,
        (-m10 * j + (m11 * h + -m12 * g)) * inv,
        (m00 * j + (-m01 * h + m02 * g)) * inv,
        (-m30 * d + (m31 * b + -m32 * a)) * inv,
        (m20 * d + (-m21 * b + m22 * a)) * inv,
    ]
}

/// JOML 1.10.9 `Matrix3f.invert` (column-major).
fn joml_mat3_invert(m: &[f32; 9]) -> [f32; 9] {
    let [m00, m01, m02, m10, m11, m12, m20, m21, m22] = *m;
    let a = m00 * m11 + (-m01 * m10);
    let b = m02 * m10 + (-m00 * m12);
    let c = m01 * m12 + (-m02 * m11);
    let d = a * m22 + (b * m21 + c * m20);
    let s = 1.0 / d;
    [
        (m11 * m22 + (-m21 * m12)) * s,
        (m21 * m02 + (-m01 * m22)) * s,
        c * s,
        (m20 * m12 + (-m10 * m22)) * s,
        (m00 * m22 + (-m20 * m02)) * s,
        b * s,
        (m10 * m21 + (-m20 * m11)) * s,
        (m20 * m01 + (-m00 * m21)) * s,
        a * s,
    ]
}

fn joml_mat3_transpose(m: &[f32; 9]) -> [f32; 9] {
    [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]]
}

fn joml_mat3_transform(m: &[f32; 9], v: [f32; 3]) -> [f32; 3] {
    [
        m[0] * v[0] + (m[3] * v[1] + m[6] * v[2]),
        m[1] * v[0] + (m[4] * v[1] + m[7] * v[2]),
        m[2] * v[0] + (m[5] * v[1] + m[8] * v[2]),
    ]
}

/// JOML 1.10.9 `Vector3f.normalize`.
fn joml_vec3_normalize(v: [f32; 3]) -> [f32; 3] {
    let inv = 1.0 / (v[0] * v[0] + (v[1] * v[1] + v[2] * v[2])).sqrt();
    [v[0] * inv, v[1] * inv, v[2] * inv]
}

/// `Direction.getApproximateNearest(float, float, float)`.
fn direction_approximate_nearest(v: [f32; 3]) -> Direction {
    const DIRECTIONS: [(Direction, [f32; 3]); 6] = [
        (Direction::Down, [0.0, -1.0, 0.0]),
        (Direction::Up, [0.0, 1.0, 0.0]),
        (Direction::North, [0.0, 0.0, -1.0]),
        (Direction::South, [0.0, 0.0, 1.0]),
        (Direction::West, [-1.0, 0.0, 0.0]),
        (Direction::East, [1.0, 0.0, 0.0]),
    ];
    let mut result = Direction::North;
    // `Float.MIN_VALUE` (smallest positive subnormal f32).
    let mut highest_dot = f32::from_bits(1);
    for (dir, n) in DIRECTIONS {
        let dot = v[0] * n[0] + v[1] * n[1] + v[2] * n[2];
        if dot > highest_dot {
            highest_dot = dot;
            result = dir;
        }
    }
    result
}

fn direction_unit_vec(d: Direction) -> [f32; 3] {
    match d {
        Direction::Down => [0.0, -1.0, 0.0],
        Direction::Up => [0.0, 1.0, 0.0],
        Direction::North => [0.0, 0.0, -1.0],
        Direction::South => [0.0, 0.0, 1.0],
        Direction::West => [-1.0, 0.0, 0.0],
        Direction::East => [1.0, 0.0, 0.0],
    }
}

fn joml_rotate_x(v: [f32; 3], angle: f32) -> [f32; 3] {
    let sin = crate::camera::joml::sin(angle);
    let cos = crate::camera::joml::cos_from_sin(sin, angle);
    [v[0], v[1] * cos - v[2] * sin, v[1] * sin + v[2] * cos]
}

fn joml_rotate_y(v: [f32; 3], angle: f32) -> [f32; 3] {
    let sin = crate::camera::joml::sin(angle);
    let cos = crate::camera::joml::cos_from_sin(sin, angle);
    [v[0] * cos + v[2] * sin, v[1], -v[0] * sin + v[2] * cos]
}

/// `Direction.getRotation()`.
fn direction_rotation(d: Direction) -> crate::camera::joml::Quat {
    use crate::camera::joml::Quat;
    use std::f32::consts::{FRAC_PI_2, PI};
    let xyz = |ax: f32, ay: f32, az: f32| {
        let [x, y, z, w] = sunrise_assets::math::quaternion_xyz(ax, ay, az);
        Quat { x, y, z, w }
    };
    match d {
        Direction::Down => Quat::rotation_x(PI),
        Direction::Up => Quat::IDENTITY,
        Direction::North => xyz(FRAC_PI_2, 0.0, PI),
        Direction::South => Quat::rotation_x(FRAC_PI_2),
        Direction::West => xyz(FRAC_PI_2, 0.0, FRAC_PI_2),
        Direction::East => xyz(FRAC_PI_2, 0.0, -FRAC_PI_2),
    }
}

/// Position and normal inverses for `VertexConsumer.putBakedQuadWithGlint` +
/// `SheetedDecalTextureGenerator.setSheetedDecalUv`.
struct DecalTransforms {
    camera_inv: [f32; 16],
    normal_mat: [f32; 9],
    normal_inv: [f32; 9],
}

/// `ItemFeatureRenderer.computeFoilDecalPose` + `PoseStack.Pose.computeNormalMatrix` inverted.
fn special_foil_transforms(m: &[f32; 16], display: DisplayContext) -> DecalTransforms {
    let scale = match display {
        DisplayContext::Gui => Some(0.5_f32),
        DisplayContext::FirstPersonLeftHand | DisplayContext::FirstPersonRightHand => {
            Some(0.75_f32)
        }
        _ => None,
    };
    let camera_inv = match scale {
        Some(s) => joml_invert_generic(&m.map(|v| v * s)),
        None => joml_invert_affine(m),
    };
    let upper = [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]];
    let normal_mat = joml_mat3_transpose(&joml_mat3_invert(&upper));
    let normal_inv = joml_mat3_invert(&normal_mat);
    DecalTransforms {
        camera_inv,
        normal_mat,
        normal_inv,
    }
}

/// `VertexConsumer.putBakedQuadWithGlint` + `SheetedDecalTextureGenerator.setSheetedDecalUv`.
fn special_foil_uv(
    m_cols: &[f32; 16],
    decal: &DecalTransforms,
    positions: [[f32; 3]; 4],
    facing: Direction,
) -> [[f32; 2]; 4] {
    let normal = joml_vec3_normalize(joml_mat3_transform(
        &decal.normal_mat,
        direction_unit_vec(facing),
    ));
    let recovered = joml_mat3_transform(&decal.normal_inv, normal);
    let rot = direction_rotation(direction_approximate_nearest(recovered));
    positions.map(|p| {
        let pos = joml_transform_pos(m_cols, p);
        let wp = joml_transform_pos(&decal.camera_inv, pos);
        let wp = joml_rotate_y(wp, std::f32::consts::PI);
        let wp = joml_rotate_x(wp, -std::f32::consts::FRAC_PI_2);
        let wp = rot.transform(wp);
        [-wp[0] * 0.0078125, -wp[1] * 0.0078125]
    })
}

/// `MaterialBaker.get`: explicit `missingno` resolves on blocks; otherwise stitched items, then
/// stitched blocks, then the blocks missing sprite.
fn resolve_cuboid_glint(
    name: &str,
    items: &glint_atlas::GlintAtlas,
    blocks: &glint_atlas::GlintAtlas,
) -> Option<Rect> {
    if name == "missingno" {
        blocks.missing().or_else(|| items.missing())
    } else {
        items
            .get_stitched(name)
            .or_else(|| blocks.get_stitched(name))
            .or_else(|| blocks.missing())
            .or_else(|| items.missing())
    }
}

/// `ItemFeatureRenderer.prepareMainSubmit` for one cuboid layer.
fn submit_quads(
    renderer: &ItemRenderer,
    baked: &BakedItemModel,
    layer: &LayerRender,
    m: Mat4,
    sprites: &HashMap<String, Rect>,
    sink: &mut dyn ItemSink,
) {
    // Non-foil quads keep glam `transform_point3` to match `entity_skin::expand` on the GPU.
    let foil_cols = (layer.foil != FoilType::None).then(|| m.to_cols_array());
    submit_quads_with(
        renderer,
        baked,
        layer,
        m,
        |p| match &foil_cols {
            Some(cols) => Vec3::from_array(joml_transform_pos(cols, p)),
            None => m.transform_point3(Vec3::from(p)),
        },
        sprites,
        sink,
    );
}

fn submit_quads_joml(
    renderer: &ItemRenderer,
    baked: &BakedItemModel,
    layer: &LayerRender,
    m: crate::camera::joml::Mat4,
    sprites: &HashMap<String, Rect>,
    sink: &mut dyn ItemSink,
) {
    submit_quads_with(
        renderer,
        baked,
        layer,
        m.to_glam(),
        |p| Vec3::from_array(m.transform_position(p)),
        sprites,
        sink,
    );
}

fn submit_quads_with(
    renderer: &ItemRenderer,
    baked: &BakedItemModel,
    layer: &LayerRender,
    m: Mat4,
    transform_pos: impl Fn([f32; 3]) -> Vec3,
    sprites: &HashMap<String, Rect>,
    sink: &mut dyn ItemSink,
) {
    let foil = match layer.foil {
        FoilType::None => Foil::None,
        FoilType::Standard => Foil::Standard,
        FoilType::Special => Foil::Special,
    };
    let (item_atlas, block_atlas) = if foil == Foil::Standard {
        (
            Some(
                renderer
                    .glint_atlas
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
            Some(
                renderer
                    .block_glint_atlas
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
        )
    } else {
        (None, None)
    };
    let m_cols = (foil != Foil::None).then(|| m.to_cols_array());
    let decal = (foil == Foil::Special)
        .then(|| {
            m_cols
                .as_ref()
                .map(|c| special_foil_transforms(c, layer.display))
        })
        .flatten();
    let mut normals = FaceNormals::new(m);
    // Quads of one layer mostly share a sprite: look its rect and its
    // translucency up once.
    let mut last: Option<(&str, bool)> = None;
    let mut last_rect: Option<(&str, Option<Rect>)> = None;
    let mut last_glint: Option<(&str, Option<Rect>)> = None;
    for q in &baked.quads {
        let rect = match last_rect {
            Some((sprite, rect)) if sprite == q.material.sprite => rect,
            _ => {
                let rect = sprites
                    .get(sprite_name(&q.material.sprite))
                    .or_else(|| sprites.get("misc/missingno"))
                    .or_else(|| sprites.get("sunrise/missingno"))
                    .copied();
                last_rect = Some((&q.material.sprite, rect));
                rect
            }
        };
        let Some(r) = rect else {
            continue;
        };
        let color = layer.tints.at(q.tint_index);
        let pos = q.positions.map(&transform_pos);
        let uv =
            q.uv.map(|[u, v]| [r.u0 + (r.u1 - r.u0) * u, r.v0 + (r.v1 - r.v0) * v]);
        let translucent = match last {
            Some((sprite, translucent)) if sprite == q.material.sprite => translucent,
            _ => {
                let translucent = renderer.sprites.translucent(&q.material.sprite);
                last = Some((&q.material.sprite, translucent));
                translucent
            }
        };
        // Every item render type culls back faces: `Sheets.solidBlockSheet`
        // (`entitySolid`), `cutoutBlockSheet` (`entityCutout`) and
        // `translucentItemSheet` (`entityTranslucentCull`). Drawing both sides
        // z-fights the coplanar pairs a two-sided model already carries (a
        // calibrated sculk sensor's tendrils and amethyst planes, #4856; big
        // dripleaf, #4855; the inside of a held hay block, #4801) and stacks a
        // jelly shell's partial alpha until it looks opaque (#4508).
        let (translucent, explicit_alpha_cutoff) = match layer.submit_options.render_type {
            mod_api::SpecialModelRenderType::Baked => {
                (translucent || q.material.force_translucent, false)
            }
            mod_api::SpecialModelRenderType::ItemCutout => (false, true),
            mod_api::SpecialModelRenderType::ItemTranslucent => (true, true),
            mod_api::SpecialModelRenderType::Custom(_) => (
                layer
                    .render_type
                    .is_some_and(|resolved| resolved.is_translucent()),
                true,
            ),
            _ => (translucent || q.material.force_translucent, false),
        };
        let uv = match layer
            .render_type
            .map(|resolved| resolved.texture_transform())
        {
            Some(mod_api::TextureTransform::Offset([u, v])) => uv.map(|[x, y]| [x + u, y + v]),
            _ => uv,
        };
        let lightmap_only = matches!(
            layer.submit_options.lighting,
            mod_api::SpecialModelLighting::LightmapOnly
        );
        let gui_glint_uv = match foil {
            Foil::None | Foil::GlintOnly => None,
            Foil::Standard => {
                let rect = match last_glint {
                    Some((sprite, rect)) if sprite == q.material.sprite => rect,
                    _ => {
                        let name = sprite_name(&q.material.sprite);
                        let rect = item_atlas
                            .as_deref()
                            .zip(block_atlas.as_deref())
                            .and_then(|(items, blocks)| resolve_cuboid_glint(name, items, blocks));
                        last_glint = Some((&q.material.sprite, rect));
                        rect
                    }
                };
                rect.map(|r| {
                    q.uv.map(|[u, v]| [r.u0 + (r.u1 - r.u0) * u, r.v0 + (r.v1 - r.v0) * v])
                })
            }
            Foil::Special => m_cols
                .as_ref()
                .zip(decal.as_ref())
                .map(|(cols, d)| special_foil_uv(cols, d, q.positions, q.facing)),
        };
        sink.quad(ItemQuad {
            pos,
            uv,
            foil_uv: q.uv,
            gui_glint_uv,
            normal: normals.face(q.facing),
            color,
            foil,
            entity_glint: false,
            light_emission: q.light_emission,
            cull_back: true,
            translucent,
            oit: false,
            banner_pattern: false,
            packed_light: layer.submit_options.packed_light,
            lightmap_only,
            explicit_alpha_cutoff,
        });
    }
}

// ---------------------------------------------------------------------------
// Sprites for baking
// ---------------------------------------------------------------------------

/// The alpha masks of item and block sprites as the items and blocks atlases finish them (every
/// unique animation frame): the same sprites the block models and the HUD atlas draw.
struct FileSprites {
    root: ResourceSource,
    materials: crate::atlas_sources::Materials,
    cache: Mutex<HashMap<String, Option<SpriteMask>>>,
    /// Sprites with a pixel that is neither fully transparent nor opaque
    /// (`Transparency.hasTranslucent`), filled in as sprites load.
    translucent: Mutex<HashMap<String, bool>>,
}
/// `Identifier.parse` default namespace without allocating when already qualified.
fn qualified_sprite(sprite: &str) -> std::borrow::Cow<'_, str> {
    match sprite.split_once(':') {
        Some(("", path)) => std::borrow::Cow::Owned(format!("minecraft:{path}")),
        Some(_) => std::borrow::Cow::Borrowed(sprite),
        None => std::borrow::Cow::Owned(format!("minecraft:{sprite}")),
    }
}
impl SpriteLookup for FileSprites {
    fn mask(&self, sprite: &str) -> Option<SpriteMask> {
        let id = qualified_sprite(sprite);
        if let Ok(cache) = self.cache.lock()
            && let Some(hit) = cache.get(id.as_ref())
        {
            return hit.clone();
        }
        let mask = self.load(&id);
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(id.into_owned(), mask.clone());
        }
        mask
    }
}
fn argb_pixels(rgba: &[u8]) -> Vec<u32> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from_be_bytes([p[3], p[0], p[1], p[2]]))
        .collect()
}
impl FileSprites {
    fn new(root: ResourceSource) -> Self {
        Self {
            materials: crate::atlas_sources::Materials::new(&root),
            root,
            cache: Mutex::default(),
            translucent: Mutex::default(),
        }
    }
    fn translucent(&self, sprite: &str) -> bool {
        let id = qualified_sprite(sprite);
        let known = |this: &Self| this.translucent.lock().ok()?.get(id.as_ref()).copied();
        known(self).unwrap_or_else(|| {
            self.mask(&id);
            known(self).unwrap_or(false)
        })
    }
    fn load(&self, sprite: &str) -> Option<SpriteMask> {
        // A candidate whose frames don't fit already gave way to the next atlas.
        let crate::atlas_sources::Material {
            image: (iw, ih, rgba),
            animation,
            ..
        } = self.materials.finish(&self.root, sprite)?;
        let (iw, ih, argb) = (iw as usize, ih as usize, argb_pixels(&rgba));
        let translucent = argb.iter().any(|p| matches!(p >> 24, 1..=254));
        if let Ok(mut cache) = self.translucent.lock() {
            cache.insert(sprite.to_owned(), translucent);
        }
        let alpha: Vec<u8> = argb.iter().map(|p| (p >> 24) as u8).collect();
        let (w, h) = match &animation {
            Some(a) => {
                let [w, h] = a.frame_size(iw as i32, ih as i32);
                (w as usize, h as usize)
            }
            None => (iw, ih),
        };
        if w == 0 || h == 0 {
            return None;
        }
        let columns = iw / w;
        let total = columns * (ih / h);
        let clip = animation.and_then(|a| a.clip(total as u32).ok().flatten());
        let frames: Vec<usize> = match clip {
            Some(clip) => {
                let mut unique = Vec::new();
                for (index, _) in clip.sequence() {
                    if !unique.contains(&(*index as usize)) {
                        unique.push(*index as usize);
                    }
                }
                unique
            }
            None => vec![0],
        };
        let frames = frames
            .into_iter()
            .map(|f| {
                let (fx, fy) = (f % columns * w, f / columns * h);
                (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .map(|(x, y)| alpha[(fy + y) * iw + fx + x] == 0)
                    .collect()
            })
            .collect();
        Some(SpriteMask {
            width: w,
            height: h,
            frames,
        })
    }
}

// ---------------------------------------------------------------------------
// The renderer
// ---------------------------------------------------------------------------

/// The baked item models and everything resolution needs.
pub struct ItemRenderer {
    pub models: Arc<ItemModels>,
    library: Arc<ModelLibrary>,
    sprites: FileSprites,
    baked: Mutex<HashMap<String, Arc<BakedItemModel>>>,
    extents: Mutex<HashMap<String, Arc<Vec<Vec3>>>>,
    special_generation: Arc<()>,
    /// GUI items already drawn: the HUD vertices of a static item at a GUI position
    /// (`gui::draw`), which are the same every frame it stays there.
    pub(crate) gui_drawn: Mutex<HashMap<gui::DrawnKey, Arc<[crate::hud::HudVertex]>>>,
    /// Vanilla 26.3's `GuiItemAtlas` / `DynamicAtlasAllocator` physical slot assignment.
    pub(crate) gui_item_atlas: Mutex<gui::GuiItemAtlasSlots>,
    max_texture_size: AtomicU32,
    /// Entity model layers of the special renderers, with the head of each skull type a native
    /// mod registered, under its kind.
    parts: HashMap<String, ModelPart>,
    /// The default texture of each skull type a native mod registered (`SKIN_BY_TYPE`).
    skull_textures: HashMap<&'static str, &'static str>,
    /// `colormap/grass.png` as ARGB.
    grass: Vec<i32>,
    /// `minecraft:item` registry ids (`Item.getId`).
    item_ids: Arc<HashMap<String, i32>>,
    /// The enabled packs' OptiFine CIT rules and their swapped bakes; `None`
    /// without rules.
    pub cit: Option<cit::CitModels>,
    glint_atlas: Mutex<glint_atlas::GlintAtlas>,
    block_glint_atlas: Mutex<glint_atlas::GlintAtlas>,
    shield_glint_atlas: Mutex<glint_atlas::GlintAtlas>,
}

/// Every winning `items/**/*.json` client item definition, by identifier. An error or panic
/// while decoding or baking omits the definition rather than looking in a lower pack.
fn load_definitions(
    root: &ResourceSource,
    decode: impl Fn(&str) -> Result<ClientItem, sunrise_assets::DecodeError>,
) -> BTreeMap<String, ClientItem> {
    let mut out = BTreeMap::new();
    for namespace in root.namespaces("assets") {
        let prefix = format!("assets/{namespace}/items/");
        for name in root.list(&prefix) {
            let Some(id) = name
                .strip_prefix(&prefix)
                .and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            let Ok(text) = root.read_to_string(&name) else {
                continue;
            };
            match catch_unwind(AssertUnwindSafe(|| decode(&text))) {
                Ok(Ok(item)) => {
                    out.insert(format!("{namespace}:{id}"), item);
                }
                Ok(Err(error)) => {
                    log::warn!("Sunrise: item definition {namespace}:{id} not loaded: {error}");
                }
                Err(panic) => {
                    // An arbitrary native panic payload can itself panic when dropped.
                    let reason = match panic.downcast::<String>() {
                        Ok(message) => std::borrow::Cow::Owned(*message),
                        Err(panic) => match panic.downcast::<&'static str>() {
                            Ok(message) => std::borrow::Cow::Borrowed(*message),
                            Err(panic) => {
                                std::mem::forget(panic);
                                std::borrow::Cow::Borrowed("unknown panic payload")
                            }
                        },
                    };
                    log::warn!(
                        "Sunrise: item definition {namespace}:{id} not loaded; decode or bake panicked: {reason}"
                    );
                }
            }
        }
    }
    out
}

fn glint_padding(filtering: i32, bit: i32) -> usize {
    if filtering == 2 {
        bit.wrapping_sub(1).clamp(0, 4) as usize
    } else {
        0
    }
}

impl ItemRenderer {
    #[cfg(any(test, feature = "test-resource-cache"))]
    pub fn has_registered_providers() -> bool {
        FACTORIES.get().is_some()
            || ItemProviderTables::has_registered()
            || crate::skull_models::all().next().is_some()
    }

    /// Cheap clone sharing `Arc<ModelLibrary>` and prototypes with fresh per-game caches.
    #[cfg(any(test, feature = "test-resource-cache"))]
    pub fn clone_for_tests(&self) -> Option<Self> {
        if self.cit.is_some() {
            return None;
        }
        Some(Self {
            models: Arc::clone(&self.models),
            library: Arc::clone(&self.library),
            sprites: FileSprites::new(self.sprites.root.clone()),
            baked: Mutex::default(),
            extents: Mutex::default(),
            special_generation: Arc::new(()),
            gui_drawn: Mutex::default(),
            gui_item_atlas: Mutex::default(),
            max_texture_size: AtomicU32::new(self.max_texture_size.load(Ordering::Relaxed)),
            parts: self.parts.clone(),
            skull_textures: self.skull_textures.clone(),
            grass: self.grass.clone(),
            item_ids: Arc::clone(&self.item_ids),
            cit: None,
            glint_atlas: Mutex::default(),
            block_glint_atlas: Mutex::default(),
            shield_glint_atlas: Mutex::default(),
        })
    }

    /// Prepare item/block/shield glint atlases after the device exists, defaulting `mipmapLevels`
    /// to vanilla's `4`.
    pub fn prepare_glint_atlas(&self, maximum: u32, filtering: (i32, i32)) -> crate::Result<()> {
        self.prepare_glint_atlas_with_mips(maximum, filtering, 4)
    }

    /// Prepare item/block/shield glint atlases with the configured `mipmapLevels` option.
    pub fn prepare_glint_atlas_with_mips(
        &self,
        maximum: u32,
        filtering: (i32, i32),
        mipmap_levels: i32,
    ) -> crate::Result<()> {
        self.max_texture_size
            .store(maximum.max(512), Ordering::Relaxed);
        let aniso_shift = glint_padding(filtering.0, filtering.1) as u32;
        let max_mip = mipmap_levels.clamp(0, 4) as u32;
        let listing = crate::atlas_sources::listing(&self.sprites.root);
        let items = glint_atlas::load_atlas(
            &self.sprites.root,
            "minecraft:items",
            0,
            aniso_shift,
            maximum,
        )?;
        let blocks = if listing.defines("minecraft:blocks") {
            glint_atlas::load_atlas(
                &self.sprites.root,
                "minecraft:blocks",
                max_mip,
                aniso_shift,
                maximum,
            )?
        } else {
            glint_atlas::GlintAtlas::default()
        };
        let shield = if listing.defines("minecraft:shield_patterns") {
            glint_atlas::load_atlas(
                &self.sprites.root,
                "minecraft:shield_patterns",
                0,
                aniso_shift,
                maximum,
            )?
        } else {
            glint_atlas::GlintAtlas::default()
        };
        *self
            .glint_atlas
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = items;
        *self
            .block_glint_atlas
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = blocks;
        *self
            .shield_glint_atlas
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = shield;
        Ok(())
    }

    /// Load item definitions, item prototypes (`items.json`), every model,
    /// the special renderers' entity models and the grass colormap.
    pub fn load(
        root: &ResourceRoot,
        item_data: &sunrise_world::item_defaults::ItemData,
    ) -> crate::Result<Self> {
        Self::load_with_library(
            root,
            crate::model_library::ParsedModels::load(root.source()).into_item_library(),
            item_data,
        )
    }
    /// Reuse the immutable models prepared with the block resources at `root`.
    /// Decode special item definitions against this library and its pack sprites.
    pub fn load_with_library(
        root: &ResourceRoot,
        library: std::result::Result<Arc<ModelLibrary>, String>,
        item_data: &sunrise_world::item_defaults::ItemData,
    ) -> crate::Result<Self> {
        Self::load_tables(
            root,
            library,
            true,
            item_data,
            None,
            ItemProviderTables::installed(FACTORIES.get()),
        )
    }
    /// What a dedicated server reads: the item prototypes (the stacks'
    /// default components) and ids, without the client item definitions,
    /// the special renderers' entity models and the grass colormap, which
    /// only drawing reads (1.4 MB and ~1,500 files).
    pub fn load_for_server(
        root: &ResourceRoot,
        library: std::result::Result<Arc<ModelLibrary>, String>,
        item_data: &sunrise_world::item_defaults::ItemData,
    ) -> crate::Result<Self> {
        Self::load_tables(
            root,
            library,
            false,
            item_data,
            None,
            ItemProviderTables::default(),
        )
    }
    pub fn load_with_library_and_entity_models(
        root: &sunrise_assets::resource_source::ResourceRoot,
        library: std::result::Result<Arc<ModelLibrary>, String>,
        document: &crate::mobs::EntityModelDocument,
        item_data: &sunrise_world::item_defaults::ItemData,
    ) -> crate::Result<Self> {
        Self::load_tables(
            root,
            library,
            true,
            item_data,
            Some(document),
            ItemProviderTables::installed(FACTORIES.get()),
        )
    }
    fn load_tables(
        root: &ResourceRoot,
        library: std::result::Result<Arc<ModelLibrary>, String>,
        drawn: bool,
        item_data: &sunrise_world::item_defaults::ItemData,
        entity_models: Option<&crate::mobs::EntityModelDocument>,
        providers: ItemProviderTables<'_>,
    ) -> crate::Result<Self> {
        let prototypes = Arc::clone(item_data.prototypes()?);
        // CIT models and glint textures only affect drawing. A server still
        // needs item prototypes and ids, but never these resource-pack assets.
        let (library, cit) = if drawn {
            cit::load(root.source(), library?)
        } else {
            (library?, None)
        };
        let mut parts = if drawn {
            entity_models.map_or_else(
                || special::load_parts(root.prepared()),
                special::load_parts_from_document,
            )
        } else {
            HashMap::new()
        };
        // Mod skull types, resolved once here so a head draws as a vanilla one does.
        let mut skull_textures = HashMap::new();
        if drawn {
            for (kind, part, texture) in crate::skull_models::all() {
                parts.insert(kind.to_owned(), part.clone());
                skull_textures.insert(kind, texture);
            }
        }
        let grass = if drawn {
            root.source()
                .read("assets/minecraft/textures/colormap/grass.png")
                .map_err(Into::into)
                .and_then(|bytes| crate::hud::decode_bytes(&bytes))
                .map(|(_, _, rgba)| argb_pixels(&rgba).into_iter().map(|p| p as i32).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let item_ids = Arc::new(sunrise_sim::item_models::load_item_ids(root.prepared()));
        let mut renderer = Self {
            glint_atlas: Mutex::default(),
            block_glint_atlas: Mutex::default(),
            shield_glint_atlas: Mutex::default(),
            item_ids,
            models: Arc::new(ItemModels {
                definitions: BTreeMap::new(),
                prototypes,
            }),
            library,
            sprites: FileSprites::new(root.source().clone()),
            baked: Mutex::default(),
            extents: Mutex::default(),
            special_generation: Arc::new(()),
            gui_drawn: Mutex::default(),
            gui_item_atlas: Mutex::default(),
            max_texture_size: AtomicU32::new(16384),
            parts,
            skull_textures,
            grass,
            cit,
        };
        if drawn {
            let definitions = load_definitions(root.source(), |text| {
                renderer.decode_definition(text, &providers)
            });
            Arc::get_mut(&mut renderer.models)
                .expect("unique ItemModels during ItemRenderer::load_tables")
                .definitions = definitions;
        }
        Ok(renderer)
    }
    /// `GuiItemAtlas.endFrame`: frees animated GUI item slots at frame end while retaining static ones.
    pub fn end_gui_frame(&self) {
        self.gui_item_atlas
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .end_frame();
    }

    pub(crate) fn gui_item_slot(&self, state: &ItemRenderState, gui_scale: f32) -> (u32, u32) {
        let max_tex = self.max_texture_size.load(Ordering::Relaxed);
        self.gui_item_atlas
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .slot_for(state, gui_scale, max_tex)
    }

    /// `ModelBakery` item baking, cached per model.
    pub fn bake(&self, model: &str) -> Arc<BakedItemModel> {
        self.baked_with(model, || {
            bake_item_model(&self.library, &self.sprites, model)
        })
    }
    /// A layer baked from something other than a model id, such as an extruded sprite,
    /// cached under `key` (which no model id can equal: identifiers hold no NUL).
    fn baked_with(
        &self,
        key: &str,
        bake: impl FnOnce() -> std::result::Result<BakedItemModel, sunrise_assets::DecodeError>,
    ) -> Arc<BakedItemModel> {
        if let Ok(cache) = self.baked.lock()
            && let Some(hit) = cache.get(key)
        {
            return hit.clone();
        }
        let baked = bake()
            .or_else(|_| bake_item_model(&self.library, &self.sprites, MISSING_MODEL))
            .map(Arc::new)
            .unwrap_or_else(|_| {
                Arc::new(BakedItemModel {
                    quads: Vec::new(),
                    uses_block_light: false,
                    particle: sunrise_assets::model::Material {
                        sprite: sunrise_assets::item_bake::MISSING_SPRITE.to_owned(),
                        force_translucent: false,
                    },
                    transforms: Default::default(),
                })
            });
        if let Ok(mut cache) = self.baked.lock() {
            cache.insert(key.to_owned(), baked.clone());
        }
        baked
    }
    /// `CuboidItemModelWrapper.computeExtents`: the distinct quad corners.
    fn quad_extents(&self, model: &str, baked: &BakedItemModel) -> Arc<Vec<Vec3>> {
        if let Ok(cache) = self.extents.lock()
            && let Some(hit) = cache.get(model)
        {
            return hit.clone();
        }
        let out = Arc::new(corners(baked));
        if let Ok(mut cache) = self.extents.lock() {
            cache.insert(model.to_owned(), out.clone());
        }
        out
    }
    fn special_extents(&self, model: &SpecialModel) -> Arc<Vec<Vec3>> {
        let key = model.to_json().to_string();
        if let Ok(cache) = self.extents.lock()
            && let Some(hit) = cache.get(&key)
        {
            return hit.clone();
        }
        let out = Arc::new(special::extents(self, model));
        if let Ok(mut cache) = self.extents.lock() {
            cache.insert(key, out.clone());
        }
        out
    }
    /// `ItemModelResolver.updateForTopItem`.
    pub fn resolve(
        &self,
        stack: &StackView,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        level: Option<&LevelState>,
        seed: i32,
        env: &dyn ItemDrawEnv,
    ) -> ItemRenderState {
        self.resolve_with_rule(stack, display, owner, level, seed, env, None)
    }
    /// [`Self::resolve`] for a stack OptiFine CIT item rule `rule` (an index
    /// into `cit.rules.rules`) matched: its model layers draw what the rule
    /// swaps them for.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_with_rule(
        &self,
        stack: &StackView,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        level: Option<&LevelState>,
        seed: i32,
        env: &dyn ItemDrawEnv,
        rule: Option<usize>,
    ) -> ItemRenderState {
        let ctx = Context {
            display,
            owner,
            level,
            seed,
        };
        let mut layers = Vec::new();
        let flags = self
            .append_render_layers(
                &mut layers,
                &StackAncestor {
                    stack,
                    display,
                    parent: None,
                },
                &ctx,
                env,
                rule,
            )
            .continue_value()
            .unwrap_or_default();
        ItemRenderState {
            display,
            layers,
            oversized_in_gui: flags.oversized_in_gui,
            animated: flags.animated,
            cit_glint: None,
            hdr_tier: 0,
            hdr_light: 0,
            held_tint: None,
        }
    }

    /// Append into the top-level state even when resolving a nested stack. Nested refusals
    /// unwind to the outer special layer's checkpoint; only that layer emits a missing model.
    fn append_render_layers(
        &self,
        layers: &mut Vec<LayerRender>,
        ancestor: &StackAncestor<'_>,
        ctx: &Context<'_>,
        env: &dyn ItemDrawEnv,
        rule: Option<usize>,
    ) -> ControlFlow<(), RenderLayerFlags> {
        let stack = ancestor.stack;
        let mut flags = RenderLayerFlags::default();
        if stack.count <= 0 || stack.item == "minecraft:air" {
            return ControlFlow::Continue(flags);
        }
        let display = ctx.display;
        let missing = || {
            let baked = self.bake(MISSING_MODEL);
            let extents = self.quad_extents(MISSING_MODEL, &baked);
            (LayerGeometry::Quads(baked.clone()), baked, extents)
        };
        let mut visit = |layer: Layer, layer_stack: &StackView| {
            let (geometry, baked, extents) = match layer.content {
                LayerContent::Model { model } => {
                    let (baked, extents) = rule
                        .and_then(|rule| self.cit_layer(rule, &model))
                        .unwrap_or_else(|| {
                            let baked = self.bake(&model);
                            let extents = self.quad_extents(&model, &baked);
                            (baked, extents)
                        });
                    (LayerGeometry::Quads(baked.clone()), baked, extents)
                }
                LayerContent::Sprite { sprite, mask } => {
                    let key = sprite_layer_key(&sprite, mask.as_deref());
                    let baked = self.baked_with(&key, || {
                        bake_sprite_layer(&self.library, &self.sprites, &sprite, mask.as_deref())
                    });
                    let extents = self.quad_extents(&key, &baked);
                    (LayerGeometry::Quads(baked.clone()), baked, extents)
                }
                LayerContent::Missing => missing(),
                LayerContent::Special {
                    base,
                    model,
                    argument,
                } => {
                    let baked = self.bake(&base);
                    let extents = self.special_extents(&model);
                    (LayerGeometry::Special { model, argument }, baked, extents)
                }
                LayerContent::RegisteredSpecial { model } => {
                    // Every draw can give a different result; never keep a static GUI image.
                    flags.animated = true;
                    // Vanilla's bundle model can produce a layer for its selected stack.
                    let selected = StackAncestor {
                        stack: layer_stack,
                        display,
                        parent: Some(ancestor),
                    };
                    let source = if std::ptr::eq(stack, layer_stack) {
                        ancestor
                    } else {
                        &selected
                    };
                    if self
                        .append_registered_special(
                            layers,
                            &model,
                            (layer.foil, layer.transform, layer.display_transform),
                            ctx,
                            env,
                            source,
                        )
                        .is_continue()
                    {
                        return ControlFlow::Continue(());
                    }
                    if ancestor.parent.is_some() {
                        return ControlFlow::Break(());
                    }
                    missing()
                }
            };
            flags.animated |= layer.foil != FoilType::None;
            layers.push(LayerRender {
                geometry,
                tints: LayerTints::Owned(layer.tints),
                foil: layer.foil,
                matrix: glam_matrix(&baked.layer_matrix(
                    layer.display_transform,
                    display,
                    &layer.transform,
                )),
                uses_block_light: baked.uses_block_light,
                particle: Arc::from(sprite_name(&baked.particle.sprite)),
                extents,
                display,
                submit_options: mod_api::SpecialModelSubmitOptions::default(),
                render_type: None,
            });
            ControlFlow::Continue(())
        };
        flags.oversized_in_gui = self
            .models
            .visit_layers(stack, ctx, env, LayerTransform::Model, &mut visit)?
            .unwrap_or_default();
        ControlFlow::Continue(flags)
    }
    /// `GrassColor.get` over the loaded colormap.
    pub fn grass_color(&self, temperature: f64, downfall: f64) -> i32 {
        let rain = downfall * temperature;
        let x = ((1.0 - temperature) * 255.0) as i64;
        let y = ((1.0 - rain) * 255.0) as i64;
        let index = (y << 8 | x) as usize;
        self.grass.get(index).copied().unwrap_or(-65281)
    }
    /// Submit a special model renderer directly (worn skulls of
    /// `CustomHeadLayer`, which draw the skull model, not the item).
    #[allow(clippy::too_many_arguments)]
    pub fn submit_special(
        &self,
        model: &SpecialModel,
        argument: &Value,
        has_foil: bool,
        pose: Mat4,
        sprites: &HashMap<String, Rect>,
        host: &dyn SpecialHost,
        sink: &mut dyn ItemSink,
    ) {
        special::submit(self, model, argument, has_foil, pose, sprites, host, sink);
    }
    /// Shared `minecraft:item` registry id table (`Item.getId`).
    pub fn item_ids(&self) -> &Arc<HashMap<String, i32>> {
        &self.item_ids
    }
    /// `Item.getId`, unavailable when the registry report or item is missing.
    pub fn item_registry_id(&self, item: &str) -> Option<u32> {
        sunrise_sim::item_models::item_registry_id(&self.item_ids, item)
    }
    pub(crate) fn part(&self, key: &str) -> Option<&ModelPart> {
        self.parts.get(key)
    }
}
/// A cache key for an extruded sprite layer: `\0` keeps it out of model ids' space.
fn sprite_layer_key(sprite: &str, mask: Option<&str>) -> String {
    format!("\0{sprite}\0{}", mask.unwrap_or_default())
}
/// `CuboidItemModelWrapper.computeExtents`: the distinct quad corners.
fn corners(baked: &BakedItemModel) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = Vec::new();
    for q in &baked.quads {
        for p in q.positions {
            let p = Vec3::from(p);
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}
/// A `sunrise-assets` (row-major, JOML-exact) matrix as glam's.
pub fn glam_matrix(m: &sunrise_assets::math::Mat4) -> Mat4 {
    Mat4::from_cols_array_2d(&m.rows()).transpose()
}

// ---------------------------------------------------------------------------
// World drawing
// ---------------------------------------------------------------------------

/// Entity vertices for submitted quads: the `item`/`entity` shader's
/// `minecraft_mix_light` with the level's diffuse lights on the vertex
/// colour, the packed lightmap coordinates (raised by light emission) and
/// overlay 7 for the glint.
pub struct WorldSink<'a> {
    pub out: &'a mut Vec<Vertex>,
    /// Packed light (`block_entity_render::vertex_light`).
    pub light: u32,
    /// The level's diffuse lights; `None` leaves the colour unshaded.
    pub lights: Option<(Vec3, Vec3)>,
    /// The `GLINT_TEXTURING` sample for foil quads (`None` leaves them unglinted).
    pub glint: Option<Glint>,
    /// HDR output's tier (`ItemRenderState::hdr_tier`): rides in the layer's high bits.
    pub tier: u8,
}
/// The vertex colour [`WorldSink`] gives a quad of `color` (ARGB) facing `normal`:
/// shaded by `lights` (`None`: unshaded), as ABGR bytes.
fn shaded(color: u32, normal: Vec3, lights: Option<(Vec3, Vec3)>) -> u32 {
    let brightness = lights.map_or(1.0, |l| diffuse(normal, l));
    let [a, r, g, b] = color.to_be_bytes();
    let c = |v: u8| (f32::from(v) * brightness) as u32;
    u32::from(a) << 24 | c(b) << 16 | c(g) << 8 | c(r)
}
impl ItemSink for WorldSink<'_> {
    fn quad(&mut self, q: ItemQuad) {
        let lights = if q.lightmap_only { None } else { self.lights };
        let color = shaded(q.color, q.normal, lights);
        let light = if let Some(packed) = q.packed_light {
            // Vanilla `LightCoordsUtil` layout: block byte in bits 0..8, sky byte in bits 16..24.
            // Convert to entity vertex order (sky in bits 0..8, block in bits 8..16) and set
            // `light.z = 1` (bit 16) so `entity.wgsl` bypasses dynamic block light boosts and
            // uses both submitted coordinates verbatim with no baked emission floor.
            let block_byte = packed & 0xFF;
            let sky_byte = (packed >> 16) & 0xFF;
            sky_byte | (block_byte << 8) | (1 << 16)
        } else {
            // `LightCoordsUtil.lightCoordsWithEmission`: block light at least the emission.
            let emission = u32::from(q.light_emission) * 16;
            if (self.light >> 8 & 0xFF) < emission {
                (self.light & 0xFF) | emission << 8
            } else {
                self.light
            }
        };
        let glint = (q.foil != Foil::None)
            .then_some(self.glint)
            .flatten()
            .map(|g| g.for_quad(q.entity_glint));
        if q.foil == Foil::GlintOnly {
            let Some(glint) = glint else {
                return;
            };
            let src_uv = q.gui_glint_uv.unwrap_or(q.foil_uv);
            for (i, (p, uv)) in q.pos.iter().zip(q.uv).enumerate() {
                self.out.push(Vertex {
                    normal: [0; 4],
                    pos: p.to_array(),
                    uv,
                    layer: crate::gfx::GLINT_ONLY_OVERLAY | u32::from(self.tier) << 16,
                    color: u32::MAX,
                    light,
                    glint_uv: glint.uv(src_uv[i]),
                });
            }
            return;
        }
        let overlay = if q.banner_pattern {
            crate::gfx::BANNER_PATTERN_OVERLAY
        } else {
            match (glint.is_some(), q.translucent) {
                (false, true) => crate::gfx::TRANSLUCENT_ITEM_OVERLAY,
                (false, false) => 0,
                (true, true) => crate::gfx::TRANSLUCENT_GLINT_ITEM_OVERLAY,
                (true, false) => 7,
            }
        };
        // Only plain and foil quads: the translucent sheet's overlays are their own material.
        let oit = if q.oit && matches!(overlay, 0 | 7) {
            crate::gfx::OIT_ENTITY
        } else {
            0
        };
        let src_uv = q.gui_glint_uv.unwrap_or(q.foil_uv);
        for (i, (p, uv)) in q.pos.iter().zip(q.uv).enumerate() {
            let glint_uv = glint.map_or([0.0, 0.0], |glint| glint.uv(src_uv[i]));
            self.out.push(Vertex {
                normal: if q.cull_back { CULL_BACK } else { [0; 4] },
                pos: p.to_array(),
                uv,
                layer: overlay | oit | u32::from(self.tier) << 16,
                color,
                light,
                glint_uv,
            });
        }
    }
}

/// `ItemEntity.getSpin`.
pub fn item_spin(age_in_ticks: f32, bob_offset: f32) -> f32 {
    age_in_ticks / 20.0 + bob_offset
}
/// `ItemEntityRenderer`'s bob: how far a dropped stack rises as it spins, or
/// nothing when its item does not bob (NeoForge's `shouldBobAsEntity`).
pub fn item_bob(age_in_ticks: f32, bob_offset: f32, bobs: bool) -> f32 {
    if bobs {
        sunrise_core::geometry::trig::sin(f64::from(age_in_ticks / 10.0 + bob_offset)) * 0.1 + 0.1
    } else {
        0.0
    }
}
/// `ItemClusterRenderState.getRenderedAmount`.
pub fn rendered_amount(count: u32) -> u32 {
    match count {
        0..=1 => 1,
        2..=16 => 2,
        17..=32 => 3,
        33..=48 => 4,
        _ => 5,
    }
}
/// `ItemEntityRenderer.submitMultipleFromCount`: the poses of each copy of
/// a stack (`seed` is `ItemClusterRenderState.getSeedForItemStack`).
pub fn cluster_poses(state: &ItemRenderState, pose: Mat4, amount: u32, seed: i64) -> Vec<Mat4> {
    if amount == 0 {
        return Vec::new();
    }
    let (lo, hi) = state.bounding_box();
    // `AABB.getZsize`, in doubles, then narrowed.
    let depth = (f64::from(hi.z) - f64::from(lo.z)) as f32;
    cluster_poses_with_depth(pose, amount, seed, depth)
}

/// The poses of a dropped stack's copies: spread apart as [`cluster_poses_with_depth`]
/// places them, or one pose for the whole stack when its item's copies do not spread
/// (NeoForge's `shouldSpreadAsEntity`).
pub fn dropped_copy_poses(
    pose: Mat4,
    amount: u32,
    seed: i64,
    depth: f32,
    spreads: bool,
) -> Vec<Mat4> {
    if amount == 0 {
        Vec::new()
    } else if spreads {
        cluster_poses_with_depth(pose, amount, seed, depth)
    } else {
        vec![pose]
    }
}

/// [`cluster_poses`] with the depth of the resolved model's bounding box
/// already computed for this draw (also used to place the stack's bob). Each
/// copy's `PoseStack.translate` is JOML's.
pub fn cluster_poses_with_depth(pose: Mat4, amount: u32, seed: i64, depth: f32) -> Vec<Mat4> {
    use crate::camera::joml;
    use sunrise_core::random::{LegacyRandom, RandomSource};
    let mut out = Vec::new();
    if amount == 0 {
        return out;
    }
    let pose = joml::Mat4::affine(pose);
    let translated = |p: joml::Mat4, x: f32, y: f32, z: f32| {
        let mut p = p;
        p.translate(x, y, z);
        p
    };
    let mut random = LegacyRandom::new(seed);
    let mut next = || random.next_float() * 2.0 - 1.0;
    if depth > 0.0625 {
        out.push(pose.to_glam());
        for _ in 1..amount {
            let (x, y, z) = (next() * 0.15, next() * 0.15, next() * 0.15);
            out.push(translated(pose, x, y, z).to_glam());
        }
    } else {
        let offset = depth * 1.5;
        let mut p = translated(pose, 0.0, 0.0, -(offset * (amount - 1) as f32 / 2.0));
        out.push(p.to_glam());
        p.translate(0.0, 0.0, offset);
        for _ in 1..amount {
            let (x, y) = (next() * 0.15 * 0.5, next() * 0.15 * 0.5);
            out.push(translated(p, x, y, 0.0).to_glam());
            p.translate(0.0, 0.0, offset);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use sunrise_world::native_load_context::DataLoadOrigin;

    /// Item definitions are read again on every load: a node of a registered type keeps its
    /// `models` as the packs now give them, and a definition with a type no mod registered isn't
    /// loaded, so its item draws the missing model.
    /// A renamed file whose animation frames can't tile it is no candidate: the blocks atlas's
    /// sprite of the id is the mask, and nothing divides by zero.
    #[test]
    fn a_renamed_file_with_unfitting_frames_leaves_the_blocks_sprite() {
        let root = std::env::temp_dir().join(format!("sunrise-mask-frames-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let textures = root.join("assets/demo/textures/item");
        let atlases = root.join("assets/minecraft/atlases");
        fs::create_dir_all(&textures).unwrap();
        fs::create_dir_all(&atlases).unwrap();
        let write = |name: &str, width: u32, height: u32, pixels: &[u8]| {
            let mut encoder = png::Encoder::new(
                fs::File::create(textures.join(name)).unwrap(),
                width,
                height,
            );
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(pixels)
                .unwrap();
        };
        // A 1 x 1 file declaring frames 2 wide, and a 2 x 1 sheet: opaque, then transparent.
        write("tiny.png", 1, 1, &[255; 4]);
        fs::write(
            textures.join("tiny.png.mcmeta"),
            r#"{"animation":{"width":2}}"#,
        )
        .unwrap();
        write("sheet.png", 2, 1, &[255, 0, 0, 255, 0, 255, 0, 0]);
        fs::write(
            atlases.join("items.json"),
            r#"{"sources":[{"type":"minecraft:single","resource":"demo:item/tiny","sprite":"demo:item/target"}]}"#,
        )
        .unwrap();
        fs::write(
            atlases.join("blocks.json"),
            r#"{"sources":[{"type":"minecraft:unstitch","resource":"demo:item/sheet","divisor_x":2,
                "regions":[{"sprite":"demo:item/target","x":1,"y":0,"width":1,"height":1}]}]}"#,
        )
        .unwrap();
        let sprites = FileSprites::new(ResourceSource::directory(root.clone()));
        let mask = sprites.mask("demo:item/target").unwrap();
        assert_eq!((mask.width, mask.height), (1, 1));
        assert_eq!(mask.frames, vec![vec![true]]);
        fs::remove_dir_all(&root).unwrap();
    }
    /// The items atlas's cut of an id is the mask, not the blocks atlas's; a sprite only an
    /// atlas cut makes has one at all.
    #[test]
    fn a_mask_follows_the_items_atlas_then_the_blocks_atlas() {
        let root =
            std::env::temp_dir().join(format!("sunrise-mask-atlases-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let textures = root.join("assets/demo/textures/item");
        let atlases = root.join("assets/minecraft/atlases");
        fs::create_dir_all(&textures).unwrap();
        fs::create_dir_all(&atlases).unwrap();
        // 2 x 1 sheet: an opaque texel, then a fully transparent one.
        let mut encoder =
            png::Encoder::new(fs::File::create(textures.join("sheet.png")).unwrap(), 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 0])
            .unwrap();
        let cut = |sprite: &str, x: u8| {
            format!(
                r#"{{"type":"minecraft:unstitch","resource":"demo:item/sheet","divisor_x":2,
                "regions":[{{"sprite":"demo:item/{sprite}","x":{x},"y":0,"width":1,"height":1}}]}}"#
            )
        };
        fs::write(
            atlases.join("items.json"),
            format!(r#"{{"sources":[{}]}}"#, cut("both", 1)),
        )
        .unwrap();
        fs::write(
            atlases.join("blocks.json"),
            format!(
                r#"{{"sources":[{},{}]}}"#,
                cut("both", 0),
                cut("blocks_only", 0)
            ),
        )
        .unwrap();
        let sprites = FileSprites::new(ResourceSource::directory(root.clone()));
        // `true` marks a fully transparent texel.
        assert_eq!(
            sprites.mask("demo:item/both").unwrap().frames,
            vec![vec![true]]
        );
        assert_eq!(
            sprites.mask("demo:item/blocks_only").unwrap().frames,
            vec![vec![false]]
        );
        assert!(
            sprites.mask("demo:item/sheet").is_none(),
            "no atlas lists the sheet"
        );
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn a_registered_item_model_type_is_read_again_on_each_load() {
        use mod_api::{
            __private::game::RegistrationContextNew, Hooks, Identifier, ModEntry,
            RegisterItemModelsEvent, RegistrationContext,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};
        use sunrise_assets::resource_source::Resource;

        struct CellMod(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for CellMod {
            fn register_item_models(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterItemModelsEvent<'_>,
            ) {
                let decodes = Arc::clone(&self.0);
                event.register(
                    Identifier::parse("loadtest:cell").unwrap(),
                    move |_, children| {
                        decodes.fetch_add(1, Ordering::Relaxed);
                        children.add_named_models()?;
                        Ok(
                            |_: &dyn mod_api::ClientHost,
                             _: &mod_api::ItemModelUpdateInput<'_>,
                             _: &mut Vec<mod_api::ItemModelLayer>| {},
                        )
                    },
                );
            }
        }

        let decodes = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "loadtest",
            Box::new(CellMod(Arc::clone(&decodes))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut registered = Vec::new();
            mod_api::dispatch_register_item_models(&RegistrationContext::new(), &mut registered);
            let (factories, kept) = bind_item_model_factories(registered);
            let providers = ItemProviderTables {
                item_model: Some(&factories),
                ..Default::default()
            };
            assert_eq!(kept, ["loadtest:cell"]);

            let source = |full: &str| {
                let cell = format!(
                    r#"{{"model":{{"type":"loadtest:cell","models":{{"full":{{"type":"minecraft:model","model":"{full}"}}}}}}}}"#
                );
                ResourceSource::indexed(BTreeMap::from([
                    (
                        "assets/loadtest/items/cell.json".to_owned(),
                        Resource::bytes(cell.into_bytes()),
                    ),
                    (
                        "assets/loadtest/items/other.json".to_owned(),
                        Resource::bytes(br#"{"model":{"type":"loadtest:unregistered"}}"#.to_vec()),
                    ),
                ]))
            };
            let models = |full: &str| {
                let loaded = load_definitions(&source(full), |text| {
                    registered_providers::decode_definition_without_specials(text, &providers)
                });
                assert!(!loaded.contains_key("loadtest:other"));
                let mut models = Vec::new();
                loaded["loadtest:cell"].model.dependencies(&mut models);
                models
            };
            assert_eq!(models("loadtest:item/a"), ["loadtest:item/a"]);
            assert_eq!(models("loadtest:item/b"), ["loadtest:item/b"]);
            assert_eq!(decodes.load(Ordering::Relaxed), 2);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn archive_item_models_use_real_prepared_tables_without_unpacking() -> crate::Result<()> {
        use std::io::Write;
        use sunrise_assets::resource_source::Resource;
        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let prepared = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").ok_or("resources required")?,
        );
        let scratch = Scratch(std::env::temp_dir().join(format!(
                "sunrise-real-item-source-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            )));
        fs::create_dir(&scratch.0)?;
        let mut image = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut image, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()?
                .write_image_data(&[20, 30, 40, 255])?;
        }
        let jar = scratch.0.join("items.jar");
        let mut writer = zip::ZipWriter::new(fs::File::create(&jar)?);
        for (name, bytes) in [
            ("assets/archive_item_smoke/models/item/panel.json", br#"{"parent":"minecraft:item/generated","textures":{"layer0":"archive_item_smoke:item/panel"}}"#.as_slice()),
            ("assets/archive_item_smoke/items/panel.json", br#"{"model":{"type":"minecraft:model","model":"archive_item_smoke:item/panel"}}"#.as_slice()),
            ("assets/archive_item_smoke/textures/item/panel.png", image.as_slice()),
            ("entity-models.json", b"invalid archive table".as_slice()),
            ("items.json", b"invalid archive table".as_slice()),
        ] {
            writer.start_file(name, zip::write::SimpleFileOptions::default())?;
            writer.write_all(bytes)?;
        }
        writer.finish()?;
        let base = ResourceSource::directory(prepared.clone());
        let mut files: BTreeMap<_, _> = base
            .list("assets/")
            .into_iter()
            .map(|name| (name.clone(), Resource::file(base.clone(), name)))
            .collect();
        let archive = ResourceSource::archive(&jar)?;
        for name in archive.list("") {
            files.insert(name.clone(), Resource::file(archive.clone(), name));
        }
        let root = ResourceRoot::new(
            prepared.clone(),
            scratch.0.join("cache"),
            ResourceSource::indexed(files),
        );
        let data = crate::test_resources::item_data(
            &prepared,
            sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
        )?;
        let renderer = ItemRenderer::load(&root, &data)?;
        assert!(
            renderer
                .models
                .definitions
                .contains_key("archive_item_smoke:panel")
        );
        assert!(
            !renderer.parts.is_empty(),
            "prepared entity models stay separate from archive entries"
        );
        let model = renderer.bake("archive_item_smoke:item/panel");
        assert!(!model.quads.is_empty());
        assert_eq!(model.particle.sprite, "archive_item_smoke:item/panel");
        assert!(!prepared.join("assets/archive_item_smoke").exists());
        assert!(!scratch.0.join("assets").exists());
        Ok(())
    }

    #[test]
    fn archive_item_definitions_masks_and_glint_need_no_extracted_files() {
        use std::io::Write;
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[1, 2, 3, 0, 4, 5, 6, 128])
                .unwrap();
        }
        let folder = std::env::temp_dir().join(format!(
            "sunrise-item-source-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&folder).unwrap();
        let jar = folder.join("items.jar");
        let mut writer = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
        for (name, bytes) in [
            ("assets/example/textures/item/mask.png", png.as_slice()),
            (
                "assets/example/textures/item/mask.png.mcmeta",
                br#"{"animation":{"width":1,"height":1,"frames":[0,1]}}
// High Contrast pack signature"#
                    .as_slice(),
            ),
            (
                "assets/example/items/nested/mask.json",
                br#"{"model":{"type":"minecraft:empty"}}"#.as_slice(),
            ),
            (
                "assets/minecraft/atlases/items.json",
                br#"{"sources":[{"type":"minecraft:single","resource":"example:item/mask"}]}"#
                    .as_slice(),
            ),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        let source = ResourceSource::archive(&jar).unwrap();
        fs::remove_file(&jar).unwrap();
        assert!(
            load_definitions(&source, ClientItem::from_json).contains_key("example:nested/mask")
        );
        let sprites = FileSprites::new(source.clone());
        let mask = sprites.mask("example:item/mask").unwrap();
        assert_eq!((mask.width, mask.height), (1, 1));
        assert_eq!(mask.frames, vec![vec![true], vec![false]]);
        assert!(sprites.translucent("example:item/mask"));
        let atlas = glint_atlas::load_atlas(&source, "minecraft:items", 0, 0, 1024).unwrap();
        assert!(atlas.get_stitched("example:item/mask").is_some());
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
        fs::remove_dir(&folder).unwrap();
    }

    #[test]
    fn posed_item_bounds_include_each_cluster_transform() {
        let state = ItemRenderState {
            display: DisplayContext::Ground,
            layers: vec![LayerRender {
                geometry: LayerGeometry::Special {
                    model: SpecialModel {
                        kind: sunrise_assets::item_model::SpecialKind::Trident,
                    },
                    argument: Value::Null,
                },
                tints: LayerTints::Owned(Vec::new()),
                foil: FoilType::None,
                matrix: Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
                particle: Arc::from(""),
                uses_block_light: false,
                display: DisplayContext::Ground,
                extents: Arc::new(vec![
                    Vec3::new(-1.0, -0.5, -0.25),
                    Vec3::new(0.5, 1.0, 0.25),
                ]),
                submit_options: mod_api::SpecialModelSubmitOptions::default(),
                render_type: None,
            }],
            oversized_in_gui: false,
            animated: false,
            cit_glint: None,
            hdr_tier: 0,
            hdr_light: 0,
            held_tint: None,
        };
        let poses = [
            Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)),
            Mat4::from_translation(Vec3::new(-1.0, 0.0, 0.0)),
        ];
        let (lo, hi) = state.posed_bounds(&poses).expect("item model has vertices");
        assert_eq!(lo, Vec3::new(-2.0, 1.5, -0.25));
        assert_eq!(hi, Vec3::new(2.5, 3.0, 0.25));
    }

    #[test]
    fn stack_copy_positions_keep_the_flat_and_solid_random_sequences() {
        // Captured from the original cluster renderer with seed 7, three
        // copies and an identity pose. Flat items draw two random values
        // per extra copy; solid items draw three.
        for (depth, expected) in [
            (
                0.03125,
                [
                    [0, 0, 3_175_088_128],
                    [1_024_310_737, 1_017_789_485, 0],
                    [1_025_054_461, 3_180_766_298, 1_027_604_480],
                ],
            ),
            (
                0.25,
                [
                    [0, 0, 0],
                    [1_032_699_345, 1_026_178_093, 1_033_443_069],
                    [3_189_154_906, 3_174_720_933, 3_141_056_154],
                ],
            ),
        ] {
            let copies = cluster_poses_with_depth(Mat4::IDENTITY, 3, 7, depth);
            assert_eq!(copies.len(), expected.len());
            for (pose, position) in copies.iter().zip(expected) {
                let expected =
                    Mat4::from_translation(Vec3::from_array(position.map(f32::from_bits)));
                assert_eq!(
                    pose.to_cols_array().map(f32::to_bits),
                    expected.to_cols_array().map(f32::to_bits)
                );
            }
        }
        assert!(cluster_poses_with_depth(Mat4::IDENTITY, 0, 7, 0.25).is_empty());
    }

    /// One frame's glint is read once and shared: every item drawn from the
    /// same `GlintFrame` scrolls by the same phase, and without the atlas
    /// sprite no item gets a glint (`TextureTransform.setupGlintTexturing`
    /// once per batch, `GameRenderer.extractOptions` once per frame).
    #[test]
    fn a_frame_glint_is_shared_by_every_item() {
        let frame = GlintFrame {
            millis: 123_456,
            speed: 0.5,
            alpha: 0.75,
            sprite: true,
        };
        let world = frame.glint(0.5).expect("the atlas has the glint sprite");
        assert_eq!(
            (world.speed, world.alpha, world.millis),
            (0.5, 0.75, 123_456)
        );
        assert_eq!(world.scale, 0.5);
        let uvs: Vec<_> = (0..1000)
            .map(|_| frame.glint(0.5).unwrap().uv([0.25, 0.75]))
            .collect();
        assert!(uvs.iter().all(|uv| *uv == uvs[0]));
        assert_eq!(frame.scroll_ticks(), Glint::scroll_ticks(123_456, 0.5));
        // 123456 ms * 0.5 * 8 = 493824 ticks, modulo 330000.
        assert_eq!(frame.scroll_ticks(), 163_824.0);
        // Whole monotonic milliseconds truncate sub-millisecond elapsed time (`Util.getMillis`).
        let sub_ms = std::time::Duration::from_micros(1_500).as_millis() as i64;
        assert_eq!(sub_ms, 1);
        assert_eq!(Glint::ticks(sub_ms, 0.5), 4);
        assert_eq!(Glint::scroll_ticks(sub_ms, 0.5), 4.0);
        // Double-precision speed avoids narrowing 0.7 to f32 before scaling.
        assert_eq!(Glint::ticks(1_000, 0.7), 5_600);
        assert_eq!(Glint::scroll_ticks(1_000, 0.7), 5_600.0);
        // Whole milliseconds above 2^24 keep single-millisecond resolution.
        let high_ms = (1_i64 << 24) + 1;
        assert_eq!(Glint::ticks(high_ms, 0.5), high_ms * 4);
        assert_eq!(Glint::scroll_ticks(high_ms, 0.5), 118_868.0);
        assert_ne!(
            Glint::scroll_ticks(high_ms, 0.5),
            Glint::scroll_ticks(1_i64 << 24, 0.5)
        );
        let missing = GlintFrame {
            sprite: false,
            ..frame
        };
        assert!(missing.glint(0.5).is_none());
        assert_eq!(missing.scroll_ticks(), frame.scroll_ticks());
    }

    /// A bad item model surfaces only once items.json has loaded, so a broken
    /// items.json still reports itself first.
    #[test]
    fn item_model_errors_wait_for_the_item_definitions() -> crate::Result<()> {
        let root =
            std::env::temp_dir().join(format!("sunrise-item-model-errors-{}", std::process::id()));
        let model = root.join("assets/sunrise/models/INVALID.json");
        fs::create_dir_all(model.parent().unwrap())?;
        fs::write(model, "{}")?;
        let library = crate::model_library::ParsedModels::load(
            &crate::resource_source::ResourceSource::directory(root.clone()),
        )
        .into_item_library();
        assert!(library.is_err());
        fs::write(root.join("items.json"), "{")?;
        let load = || {
            crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload)
                .and_then(|data| {
                    ItemRenderer::load_with_library(
                        &sunrise_assets::resource_source::ResourceRoot::directory(
                            root.to_path_buf(),
                        ),
                        library.clone(),
                        &data,
                    )
                })
                .err()
                .unwrap()
                .to_string()
        };
        let broken_items = load();
        fs::write(root.join("items.json"), r#"{"sunrise:bad":false}"#)?;
        let data = crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload)?;
        let broken_components = ItemRenderer::load_with_library(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            library.clone(),
            &data,
        )
        .err()
        .unwrap()
        .to_string();
        fs::write(root.join("items.json"), "{}")?;
        let bad_model = load();
        let _ = fs::remove_dir_all(&root);
        assert!(!broken_items.starts_with("item models:"));
        assert!(!broken_components.starts_with("item models:"));
        assert!(bad_model.starts_with("item models:"));
        Ok(())
    }

    /// Definitions live at any depth (`items/<dir>/<name>.json`), as
    /// ModelEngine's `modelengine:postman/head` does.
    #[test]
    fn item_definitions_load_from_nested_folders() -> crate::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "sunrise-item-definitions-nested-{}",
            std::process::id()
        ));
        let definitions = root.join("assets/sunrise/items");
        fs::create_dir_all(definitions.join("postman"))?;
        let empty = r#"{"model":{"type":"minecraft:empty"}}"#;
        fs::write(definitions.join("top.json"), empty)?;
        fs::write(definitions.join("postman/head.json"), empty)?;
        let loaded = load_definitions(
            &ResourceSource::directory(root.clone()),
            ClientItem::from_json,
        );
        let _ = fs::remove_dir_all(&root);
        assert!(loaded.contains_key("sunrise:top"));
        assert!(loaded.contains_key("sunrise:postman/head"));
        Ok(())
    }

    /// A dedicated server keeps the stacks' default components but reads
    /// none of the client item definitions.
    #[test]
    fn a_server_keeps_prototypes_without_definitions() -> crate::Result<()> {
        let root = std::env::temp_dir().join(format!("sunrise-item-server-{}", std::process::id()));
        let definitions = root.join("assets/sunrise/items");
        fs::create_dir_all(&definitions)?;
        fs::write(
            definitions.join("top.json"),
            r#"{"model":{"type":"minecraft:empty"}}"#,
        )?;
        fs::write(
            root.join("items.json"),
            r#"{"sunrise:top":{"minecraft:max_stack_size":16}}"#,
        )?;
        let cit = root.join("assets/minecraft/optifine");
        fs::create_dir_all(&cit)?;
        fs::write(cit.join("cit.properties"), "useGlint=false\n")?;
        let library = crate::model_library::ParsedModels::load(
            &crate::resource_source::ResourceSource::directory(root.clone()),
        )
        .into_item_library();
        let data = crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload)?;
        let client = ItemRenderer::load_with_library(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            library.clone(),
            &data,
        )?;
        let server = ItemRenderer::load_for_server(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            library,
            &data,
        )?;
        let _ = fs::remove_dir_all(&root);
        assert!(client.models.definitions.contains_key("sunrise:top"));
        assert!(server.models.definitions.is_empty());
        assert!(client.cit.is_some());
        assert!(server.cit.is_none());
        assert_eq!(server.models.prototypes, client.models.prototypes);
        Ok(())
    }

    #[test]
    fn a_dropped_stack_that_does_not_bob_keeps_its_height() {
        assert_eq!(item_bob(17.5, 0.3, false), 0.0);
        // A quarter turn lands on `Mth.SIN`'s entry 16384, which is exactly 1.
        assert_eq!(item_bob(15.0, 0.070_796_37, true), 0.2);
        // Below one table step the sine is entry 0, exactly 0, where `f32::sin` is not.
        assert_eq!(item_bob(0.0, 0.000_09, true), 0.1);
    }
    #[test]
    fn a_stack_whose_copies_do_not_spread_draws_once_in_its_pose() {
        let pose =
            Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 3.0)) * Mat4::from_rotation_y(0.7);
        let amount = rendered_amount(8);
        for depth in [0.0625, 0.5] {
            assert_eq!(dropped_copy_poses(pose, amount, 42, depth, false), [pose]);
            assert!(dropped_copy_poses(pose, 0, 42, depth, false).is_empty());
            assert_eq!(
                dropped_copy_poses(pose, amount, 42, depth, true),
                cluster_poses_with_depth(pose, amount, 42, depth)
            );
        }
    }
    #[test]
    fn rendered_amounts_follow_the_count_bands() {
        let bands: Vec<u32> = [0, 1, 2, 16, 17, 32, 33, 48, 49, 64]
            .iter()
            .map(|c| rendered_amount(*c))
            .collect();
        assert_eq!(bands, [1, 1, 2, 2, 3, 3, 4, 4, 5, 5]);
    }

    #[test]
    fn palette_trns_partial_alpha_counts_as_translucent() {
        // Slime/honey (and ice) are indexed PNGs with a tRNS chunk. Decode must
        // keep that fractional alpha so FileSprites marks them translucent (#4508).
        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x03, 0x00, 0x00,
            0x00, 0x45, 0x68, 0xfd, 0x16, 0x00, 0x00, 0x00, 0x06, 0x50, 0x4c, 0x54, 0x45, 0x00,
            0xff, 0x00, 0x00, 0x80, 0x00, 0x54, 0x80, 0xe6, 0x08, 0x00, 0x00, 0x00, 0x01, 0x74,
            0x52, 0x4e, 0x53, 0xb4, 0x8c, 0xea, 0xaf, 0xf3, 0x00, 0x00, 0x00, 0x0e, 0x49, 0x44,
            0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x60, 0x64, 0x60, 0x60, 0x04, 0x00, 0x00, 0x0b,
            0x00, 0x03, 0x52, 0x29, 0x78, 0x8f, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
            0xae, 0x42, 0x60, 0x82,
        ];
        let dir = std::env::temp_dir().join("sunrise-trns-alpha");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("indexed.png");
        std::fs::write(&path, png).unwrap();
        let (_w, _h, rgba) = crate::hud::decode(&path).unwrap();
        let partial = rgba.chunks(4).filter(|p| matches!(p[3], 1..=254)).count();
        assert!(partial > 0, "tRNS alpha was stripped: {rgba:?}");
    }

    #[test]
    fn normals_follow_the_inverse_transpose() {
        let m = Mat4::from_scale(Vec3::new(2.0, 1.0, 1.0));
        assert_eq!(transform_normal(m, Vec3::X), Vec3::X);
        let r = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert!((transform_normal(r, Vec3::Z) - Vec3::X).length() < 1e-6);
    }

    /// Every vanilla item on the ground, held in either hand, framed and worn on the
    /// head, turned, shaded and unshaded: the GPU path's vertices (its jobs expanded as
    /// `entity_skin.wgsl` does) are the CPU path's, to the bit and in the same order, for
    /// each state it takes. Needs the prepared resources.
    #[test]
    fn items_posed_on_the_gpu_draw_what_the_cpu_draws() {
        use crate::gfx::entity_skin::{SkinFrame, expand};
        use crate::mobs::skin::{Skin, SkinModels};
        use sunrise_assets::item_model::{NoEnv, StackView};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-models.json").exists() {
            eprintln!("no prepared resources: skipped");
            return;
        }
        let data = crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload).unwrap();
        let r = ItemRenderer::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &data,
        )
        .unwrap();
        let resolve = |id: &str| {
            let view = StackView::new(id, 1);
            r.resolve(&view, DisplayContext::Ground, None, None, 7, &NoEnv)
        };
        // Each display context's transforms (`ItemTransform`s) pose the same quads.
        let contexts = [
            DisplayContext::Ground,
            DisplayContext::ThirdPersonRightHand,
            DisplayContext::ThirdPersonLeftHand,
            DisplayContext::Fixed,
            DisplayContext::Head,
        ];
        let states: Vec<_> = r
            .models
            .definitions
            .keys()
            .flat_map(|id| {
                contexts.map(|context| {
                    let view = StackView::new(id, 1);
                    (id, r.resolve(&view, context, None, None, 7, &NoEnv))
                })
            })
            .collect();
        // Each sprite its own rect, so a run drawn with another's shows.
        let mut sprites: HashMap<String, Rect> = HashMap::new();
        for (_, state) in &states {
            for layer in &state.layers {
                if let LayerGeometry::Quads(baked) = &layer.geometry {
                    for q in &baked.quads {
                        let n = sprites.len() as f32;
                        sprites
                            .entry(sprite_name(&q.material.sprite).to_owned())
                            .or_insert(Rect {
                                u0: n / 4096.0,
                                v0: 0.5 - n / 8192.0,
                                u1: n / 4096.0 + 0.003,
                                v1: 0.5 - n / 8192.0 + 0.007,
                                w: 16,
                                h: 16,
                            });
                    }
                }
            }
        }
        let pose = Mat4::from_translation(Vec3::new(-2.3, 0.7, 5.1))
            * Mat4::from_rotation_y(2.1)
            * Mat4::from_scale(Vec3::splat(0.5));
        let mut models = SkinModels::default();
        let mut frame = SkinFrame::default();
        let (mut taken, mut left) = (0, 0);
        for lights in [
            Some(crate::block_entity_render::diffuse_lights(false)),
            None,
        ] {
            for (id, state) in &states {
                let mut cpu = Vec::new();
                let mut sink = WorldSink {
                    out: &mut cpu,
                    light: 0x50_a0,
                    lights,
                    glint: None,
                    tier: 1,
                };
                state.submit(&r, &sprites, &NoHost, pose, &mut sink);
                frame.clear_draws();
                let mut skin = Skin::new(&mut models, &mut frame);
                if !state.submit_skinned(&r, &sprites, pose, (0x50_a0, lights, 1), &mut skin) {
                    assert!(frame.jobs.is_empty(), "{id}: left to the CPU after drawing");
                    left += 1;
                    continue;
                }
                taken += 1;
                let bounds = skin.bounds;
                let gpu = expand(&frame.new_model, &frame);
                assert_eq!(gpu.len(), cpu.len(), "{id}");
                for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
                    assert_eq!(
                        bytemuck::bytes_of(g),
                        bytemuck::bytes_of(c),
                        "{id} vertex {i}: {g:?} != {c:?}"
                    );
                }
                if let (Some((lo, hi)), false) = (bounds, cpu.is_empty()) {
                    for v in &cpu {
                        let p = Vec3::from_array(v.pos);
                        assert!(
                            p.cmpge(lo - 1e-4).all() && p.cmple(hi + 1e-4).all(),
                            "{id}: {p} outside"
                        );
                    }
                }
            }
        }
        // The 8 kinds of the `stress_items` scene all take it, and resolved again (as
        // every frame does) they draw the models already stored.
        let stored = frame.new_model.len();
        for id in [
            "stone",
            "oak_log",
            "diamond",
            "apple",
            "iron_sword",
            "bread",
            "redstone",
            "torch",
        ] {
            frame.clear_draws();
            let mut skin = Skin::new(&mut models, &mut frame);
            let state = resolve(&format!("minecraft:{id}"));
            assert!(
                state.submit_skinned(&r, &sprites, pose, (0, None, 0), &mut skin),
                "{id}"
            );
        }
        assert_eq!(frame.new_model.len(), stored, "models stored again");
        eprintln!("{taken} item states on the GPU path, {left} on the CPU");
        assert!(taken > left, "{taken} taken, {left} left");
    }

    #[test]
    fn face_normals_of_a_pose_are_its_inverse_transpose() {
        // A dropped item's pose: spun, squashed and moved. Each face's normal is the
        // inverse transpose of the pose's 3x3 applied to the face's axis, renormalized,
        // here worked out in f64 from the cofactors.
        let m = Mat4::from_translation(Vec3::new(3.0, -1.0, 7.5))
            * Mat4::from_rotation_y(0.7)
            * Mat4::from_rotation_x(-0.3)
            * Mat4::from_scale(Vec3::new(0.5, 0.25, 2.0));
        let c = m.to_cols_array();
        let a = |row: usize, col: usize| f64::from(c[col * 4 + row]);
        let cofactor = |row: usize, col: usize| {
            let (r0, r1) = ((row + 1) % 3, (row + 2) % 3);
            let (c0, c1) = ((col + 1) % 3, (col + 2) % 3);
            a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0)
        };
        let mut normals = FaceNormals::new(m);
        // Twice each: a cached face gives what it gave the first time.
        for d in Direction::ALL.into_iter().chain(Direction::ALL) {
            let axis = direction_vector(d).to_array().map(f64::from);
            // inverse^T = cofactors / det: the det's sign is the only part that survives
            // renormalizing, and it is positive here.
            let n: Vec<f64> = (0..3)
                .map(|row| (0..3).map(|col| cofactor(row, col) * axis[col]).sum())
                .collect();
            let len = n.iter().map(|v| v * v).sum::<f64>().sqrt();
            let got = normals.face(d);
            for (g, e) in got.to_array().into_iter().zip(&n) {
                assert!(
                    (f64::from(g) - e / len).abs() < 1e-6,
                    "{d:?}: {got} vs {n:?}"
                );
            }
            assert_eq!(got, transform_normal(m, direction_vector(d)), "{d:?}");
        }
    }
}
