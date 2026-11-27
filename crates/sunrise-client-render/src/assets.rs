use crate::Result;
use serde_json::Value;
use std::sync::Arc;
use sunrise_world::registry::BlockStateId;

pub use sunrise_level::assets::{
    Assets, MATERIALS, Properties, StateFacts, block_palette_bits, block_states_count,
    block_states_generation, set_block_states,
};
/// The block models and their baked sprites, read from the pack at `root` over
/// an existing block registry: everything the mesher and renderer need.
pub struct RenderAssets {
    pub sprite_size: u32,
    pub sprite_minimum_size: u32,
    /// The block sprites' RGBA, layer by layer: as built until a renderer has
    /// them, then kept compressed ([`SpritePixels`]).
    pub pixels: SpritePixels,
    /// How many array layers the block sprites take, whatever maps name them.
    pub texture_layers: u32,
    pub model_catalog: crate::models::StateModels,
    /// Shared parsed resources; item-only validation errors surface at item preparation.
    pub model_library: std::result::Result<Arc<sunrise_assets::resolve::ModelLibrary>, String>,
    pub model_placements: Vec<(sunrise_core::position::BlockPos, BlockStateId)>,
    pub model_gallery: Vec<Value>,
    pub animations: Vec<crate::sprites::SpriteAnimation>,
    pub sprite_layers: std::collections::BTreeMap<String, u32>,
    /// The first block sprite name at each layer, in `sprite_layers`' name order.
    pub(crate) sprite_names: Vec<Option<String>>,
    /// The layers of the texture files read on their own (emissive overlays), by raw name.
    pub file_layers: std::collections::BTreeMap<String, u32>,
    /// Emissive block overlays resolved during bake/reload (mod-registered and OptiFine),
    /// sorted by base sprite layer for `Globals.emissive_layers` and bound to [`Self::generation`].
    pub emissive_layers: Vec<[u32; 4]>,
    pub translucent_layers: Vec<bool>,
    /// Each layer's `MipmapGenerator` settings.
    pub sprite_mips: Vec<crate::mipmap::Settings>,
    /// The first frame, at its own side, of each layer larger than `sprite_size`
    /// (`Sprites::oversized`), which the layer holds shrunk.
    pub oversized_sprites: std::collections::BTreeMap<u32, (u32, Vec<u8>)>,
    /// The resource packs' LabPBR maps (normals, specular), layer by layer at `pbr_size`:
    /// empty when they ship none.
    pub pbr_normals: Vec<u8>,
    pub pbr_specular: Vec<u8>,
    pub pbr_size: u32,
    /// OptiFine connected textures (empty without `optifine/ctm` files).
    pub connected: crate::connected::ConnectedTextures,
    /// Fusion connected textures (empty without Fusion's connecting textures and models).
    pub fusion: crate::fusion::FusionTextures,
    /// The keyframe animations mods and packs ship (`neoforge/animations/entity`), by id.
    pub mod_animations: crate::mod_animations::Animations,
    /// The mods' registered render pipelines as this load built them.
    pub mod_pipelines: crate::mod_pipelines::Pipelines,
    /// Counts up on every successful resource load, including model-only changes.
    /// Only publishing this store invalidates the previous store's model handles and sprites.
    pub generation: u64,
}
impl RenderAssets {
    /// The empty render generation before the first reload's apply.
    pub fn unpublished() -> Self {
        Self {
            sprite_size: 1,
            sprite_minimum_size: 1,
            pixels: SpritePixels::new(Vec::new()),
            texture_layers: 0,
            model_catalog: Default::default(),
            model_library: Err("models have not been published".into()),
            model_placements: Vec::new(),
            model_gallery: Vec::new(),
            animations: Vec::new(),
            sprite_layers: Default::default(),
            sprite_names: Vec::new(),
            file_layers: Default::default(),
            emissive_layers: Vec::new(),
            translucent_layers: Vec::new(),
            sprite_mips: Vec::new(),
            oversized_sprites: Default::default(),
            pbr_normals: Vec::new(),
            pbr_specular: Vec::new(),
            pbr_size: 0,
            connected: Default::default(),
            fusion: Default::default(),
            mod_animations: Default::default(),
            mod_pipelines: Default::default(),
            generation: 0,
        }
    }

    /// Resolve a registered standalone model in this published store.
    pub fn model(
        &self,
        id: &mod_api::Identifier,
    ) -> std::result::Result<mod_api::ModelRef, mod_api::ModelError> {
        let slot = self
            .model_catalog
            .standalone_slot(&id.to_string())
            .ok_or(mod_api::ModelError::UnknownModel)?;
        Ok(mod_api::__private::built::model_ref(self.generation, slot))
    }
    /// Block-atlas sprite `sprite`'s first frame at its own resolution, for `RenderAssets::atlas_sprite_pixels`:
    /// `split` is the layers to an array (`Renderer::sprite_split`). The layer pixels are the frame
    /// the sprite starts on (an animation's first `.mcmeta` frame), scaled up to the array edge
    /// by whole pixels when the sprite is smaller (`Sprites::finish`), so a sprite whose own side
    /// (`Settings::native_size`) is not above the edge is read back exactly. A larger one, which the
    /// layer holds shrunk to the edge, comes back from the frame `Sprites::finish` kept for it at
    /// its own size.
    pub fn atlas_sprite_pixels(
        &self,
        sprite: mod_api::SpriteRef,
        split: u32,
    ) -> std::result::Result<mod_api::AtlasSpritePixels, mod_api::AtlasSpriteError> {
        use mod_api::AtlasSpriteError::{NoSuchSprite, NotReady};
        if self.generation == 0 {
            return Err(NotReady);
        }
        let split = u64::from(split.max(1));
        if u64::from(sprite.layer) >= split {
            return Err(NoSuchSprite);
        }
        let layer = u64::from(sprite.array) * split + u64::from(sprite.layer);
        if layer >= u64::from(self.texture_layers) {
            return Err(NoSuchSprite);
        }
        if let Some((side, rgba)) = u32::try_from(layer)
            .ok()
            .and_then(|layer| self.oversized_sprites.get(&layer))
        {
            return Ok(mod_api::__private::built::atlas_sprite_pixels(
                *side,
                *side,
                rgba.clone(),
            ));
        }
        let edge = self.sprite_size as usize;
        let bytes = edge * edge * 4;
        let Some(held) = (layer as usize)
            .checked_mul(bytes)
            .and_then(|from| self.pixels.range(from, bytes))
        else {
            return Err(NoSuchSprite);
        };
        let side = self
            .sprite_mips
            .get(layer as usize)
            .map_or(edge, |m| m.native_size as usize);
        if side == 0 || side >= edge {
            return Ok(mod_api::__private::built::atlas_sprite_pixels(
                edge as u32,
                edge as u32,
                held,
            ));
        }
        // `Sprites::finish` read the pixel at row `r * side / edge` for each of the `edge` rows;
        // the first row that reads source row `y` is `ceil(y * edge / side)`.
        let at = |y: usize| (y * edge).div_ceil(side);
        let mut rgba = Vec::with_capacity(side * side * 4);
        for y in 0..side {
            let row = at(y) * edge;
            for x in 0..side {
                let offset = (row + at(x)) * 4;
                rgba.extend_from_slice(&held[offset..offset + 4]);
            }
        }
        Ok(mod_api::__private::built::atlas_sprite_pixels(
            side as u32,
            side as u32,
            rgba,
        ))
    }
    /// The keyframe animation `id` of this published store
    /// (`assets/<namespace>/neoforge/animations/entity/<path>.json`).
    pub fn animation(&self, id: &mod_api::Identifier) -> Option<mod_api::AnimationHolder> {
        self.mod_animations.get(id).cloned()
    }

    /// The handle of the registered render pipeline `id` in this published store.
    pub fn render_pipeline(
        &self,
        id: &mod_api::Identifier,
    ) -> std::result::Result<mod_api::RenderPipelineRef, mod_api::RenderPipelineError> {
        self.mod_pipelines.resolve(id)
    }

    /// The handle of the built-in or registered custom render type `id` in this published store.
    pub fn render_type(
        &self,
        id: &mod_api::Identifier,
    ) -> std::result::Result<mod_api::RenderTypeRef, mod_api::RenderTypeError> {
        self.mod_pipelines.resolve_render_type(id)
    }

    /// The handle of the built-in entity render type `kind` drawing the dynamic texture
    /// `texture` in this published store (`Pipelines::textured_render_type`).
    pub fn textured_render_type(
        &self,
        kind: crate::mod_pipelines::BuiltinRenderType,
        texture: &mod_api::Identifier,
    ) -> std::result::Result<mod_api::RenderTypeRef, mod_api::RenderTypeError> {
        self.mod_pipelines.textured_render_type(kind, texture)
    }

    /// The built-in entity render type and dynamic texture a handle from
    /// [`Self::textured_render_type`] names.
    pub fn textured_render_type_of(
        &self,
        handle: mod_api::RenderTypeRef,
    ) -> std::result::Result<
        (crate::mod_pipelines::BuiltinRenderType, mod_api::Identifier),
        mod_api::RenderTypeError,
    > {
        self.mod_pipelines.textured_render_type_of(handle)
    }

    /// The handle of the registered compute pipeline `id` in this published store.
    pub fn compute_pipeline(
        &self,
        id: &mod_api::Identifier,
    ) -> std::result::Result<mod_api::ComputePipelineRef, mod_api::ComputePipelineError> {
        self.mod_pipelines.compute().resolve(id)
    }

    /// The handle of the registered pipeline modifier `id` (static across resource reloads).
    pub fn pipeline_modifier(
        &self,
        id: &mod_api::Identifier,
    ) -> std::result::Result<mod_api::PipelineModifierRef, mod_api::RenderPipelineError> {
        crate::mod_pipelines::resolve_modifier(id)
    }

    /// Validate a handle for an API that only accepts registered standalone models.
    pub fn validate_standalone_model(
        &self,
        model: mod_api::ModelRef,
    ) -> std::result::Result<(), mod_api::ModelError> {
        use mod_api::__private::game::Fields;
        let fields = model.fields();
        if fields.generation != self.generation {
            return Err(mod_api::ModelError::StaleModel);
        }
        if !self.model_catalog.is_standalone_slot(fields.slot) {
            return Err(mod_api::ModelError::UnknownModel);
        }
        Ok(())
    }

    /// Validate a handle once at consumption, before appending any geometry.
    pub fn resolve_model(
        &self,
        model: mod_api::ModelRef,
    ) -> std::result::Result<&crate::models::catalog::BakedModel, mod_api::ModelError> {
        use mod_api::__private::game::Fields;
        let fields = model.fields();
        if fields.generation != self.generation {
            return Err(mod_api::ModelError::StaleModel);
        }
        self.model_catalog
            .model_by_slot(fields.slot)
            .ok_or(mod_api::ModelError::UnknownModel)
    }
}

/// The block sprites' pixels for a renderer to upload. Once a renderer has
/// them ([`Self::pack`]) they are kept lz4-compressed in blocks: the GPU holds the
/// texture array, and the copy only serves a renderer made again (another
/// window, a lost device, another graphics API), which waits for
/// [`Self::get`] to uncompress them, or a read of one range ([`Self::range`]),
/// which uncompresses only the blocks it touches. Shared by the window and the workers'
/// `RenderAssets`, so the compressed form replaces the raw one in place.
pub struct SpritePixels(Arc<PixelStore>);
struct PixelStore {
    len: usize,
    /// Set by the first `pack`: later calls cost one atomic read.
    packing: std::sync::atomic::AtomicBool,
    /// As built; dropped once `packed` holds them.
    raw: std::sync::Mutex<Option<Arc<Vec<u8>>>>,
    /// `raw` in lz4 blocks of [`PACKED_BLOCK`] bytes (the last may be shorter).
    packed: std::sync::OnceLock<Vec<Vec<u8>>>,
}
/// Uncompressed bytes per lz4 block of packed sprite pixels: a read of one layer
/// uncompresses this much at most beyond the layer itself.
const PACKED_BLOCK: usize = 64 * 1024;
impl SpritePixels {
    pub fn new(pixels: Vec<u8>) -> Self {
        Self(Arc::new(PixelStore {
            len: pixels.len(),
            packing: std::sync::atomic::AtomicBool::new(false),
            raw: std::sync::Mutex::new(Some(Arc::new(pixels))),
            packed: std::sync::OnceLock::new(),
        }))
    }
    /// Their size in bytes, uncompressed.
    pub fn len(&self) -> usize {
        self.0.len
    }
    pub fn is_empty(&self) -> bool {
        self.0.len == 0
    }
    /// The pixels as built, uncompressed again once packed.
    pub fn get(&self) -> Arc<Vec<u8>> {
        let raw = self.0.raw.lock().unwrap_or_else(|e| e.into_inner()).clone();
        raw.unwrap_or_else(|| {
            Arc::new(
                self.unpack(0, self.0.len)
                    .unwrap_or_else(|| vec![0; self.0.len]),
            )
        })
    }
    /// The `bytes` bytes from byte `from` of the pixels, uncompressing only the blocks they
    /// lie in once packed. None when the range reaches past [`Self::len`].
    pub fn range(&self, from: usize, bytes: usize) -> Option<Vec<u8>> {
        let end = from.checked_add(bytes).filter(|end| *end <= self.0.len)?;
        let raw = self.0.raw.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match raw {
            Some(raw) => raw.get(from..end).map(<[u8]>::to_vec),
            None => Some(self.unpack(from, end).unwrap_or_else(|| vec![0; bytes])),
        }
    }
    /// Bytes `from..end` (within `len`) out of the packed blocks; None when they are not packed
    /// yet or a block does not decode to its size.
    fn unpack(&self, from: usize, end: usize) -> Option<Vec<u8>> {
        let packed = self.0.packed.get()?;
        let mut out = Vec::with_capacity(end - from);
        if from == end {
            return Some(out);
        }
        for index in from / PACKED_BLOCK..=(end - 1) / PACKED_BLOCK {
            let start = index * PACKED_BLOCK;
            let size = PACKED_BLOCK.min(self.0.len - start);
            let block = lz4_flex::decompress_size_prepended(packed.get(index)?)
                .ok()
                .filter(|block| block.len() == size)?;
            out.extend_from_slice(&block[from.max(start) - start..end.min(start + size) - start]);
        }
        Some(out)
    }
    /// Keep them compressed from now on, once a renderer has uploaded them:
    /// compressed on a thread of their own, the raw copy dropped after.
    pub fn pack(&self) {
        use std::sync::atomic::Ordering;
        if self.0.len == 0 || self.0.packing.swap(true, Ordering::Relaxed) {
            return;
        }
        let store = self.0.clone();
        let job = move || {
            let raw = store.raw.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(raw) = raw {
                let _ = store.packed.set(
                    raw.chunks(PACKED_BLOCK)
                        .map(lz4_flex::compress_prepend_size)
                        .collect(),
                );
                // `get` reads the compressed copy from here on.
                *store.raw.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
        };
        let fallback = job.clone();
        if std::thread::Builder::new()
            .name("Sprite packing".into())
            .spawn(job)
            .is_err()
        {
            fallback();
        }
    }
    /// Compress raw sprite pixels synchronously so cached test resources keep lz4 bytes.
    #[cfg(any(test, feature = "test-support"))]
    pub fn pack_sync(&self) {
        use std::sync::atomic::Ordering;
        if self.0.len == 0 || self.0.packing.swap(true, Ordering::Relaxed) {
            return;
        }
        let raw = self.0.raw.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(raw) = raw {
            let _ = self.0.packed.set(
                raw.chunks(PACKED_BLOCK)
                    .map(lz4_flex::compress_prepend_size)
                    .collect(),
            );
            *self.0.raw.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
}
/// The last `RenderAssets::generation` handed out.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A controlled blockstate scene populated with `render`'s gallery placements.
pub fn test_scene_world(
    assets: &Assets,
    render: &RenderAssets,
    radius: i32,
) -> Result<crate::world::World> {
    crate::world::World::test_scene_with_placements(assets, &render.model_placements, radius)
}

impl RenderAssets {
    pub fn load(root: &crate::resource_source::ResourceRoot, assets: &Assets) -> Result<Self> {
        Self::load_with(root, assets, true)
    }
    /// [`RenderAssets::load`] with mod-registered emissive overlays collected for this reload.
    pub fn load_with_emissive(
        root: &crate::resource_source::ResourceRoot,
        assets: &Assets,
        emissive_sprites: &[mod_api::Answered<mod_api::EmissiveSpriteRegistration>],
    ) -> Result<Self> {
        Self::load_with_parsed(root, assets, true, None, emissive_sprites)
    }
    /// [`RenderAssets::load`] with the model library already parsed.
    pub fn load_with_models(
        root: &crate::resource_source::ResourceRoot,
        assets: &Assets,
        parsed: crate::model_library::ParsedModels,
    ) -> Result<Self> {
        Self::load_with_parsed(root, assets, true, Some(parsed), &[])
    }
    /// [`RenderAssets::load_with_models`] with mod-registered emissive overlays collected for this reload.
    pub fn load_with_models_and_emissive(
        root: &crate::resource_source::ResourceRoot,
        assets: &Assets,
        parsed: crate::model_library::ParsedModels,
        emissive_sprites: &[mod_api::Answered<mod_api::EmissiveSpriteRegistration>],
    ) -> Result<Self> {
        Self::load_with_parsed(root, assets, true, Some(parsed), emissive_sprites)
    }
    /// [`RenderAssets::load`] for a server, which has no sprites or baked models.
    pub fn load_for_server(
        root: &crate::resource_source::ResourceRoot,
        assets: &Assets,
    ) -> Result<Self> {
        Self::load_with(root, assets, false)
    }
    #[sunrise_mod_api_macros::replaceable]
    fn load_with(
        root: &crate::resource_source::ResourceRoot,
        assets: &Assets,
        draws: bool,
    ) -> Result<Self> {
        Self::load_with_parsed(root, assets, draws, None, &[])
    }
    /// `parsed`: the model library when a caller parsed it ahead of this
    /// (see [`RenderAssets::load_with_models`]); otherwise it is parsed here.
    fn load_with_parsed(
        root: &crate::resource_source::ResourceRoot,
        assets: &Assets,
        draws: bool,
        parsed: Option<crate::model_library::ParsedModels>,
        emissive_sprites: &[mod_api::Answered<mod_api::EmissiveSpriteRegistration>],
    ) -> Result<Self> {
        let registry = assets.registry;
        let mut sprites = if draws {
            crate::sprites::Sprites::with_disk_cache(root)
        } else {
            crate::sprites::Sprites::default()
        };
        if draws {
            for name in MATERIALS {
                sprites.insert(&format!("minecraft:block/{name}"))?;
            }
        }
        // A server draws no block and no item: its block models (about 24 MB
        // baked) and item model library (7 MB) stay empty.
        let parsed = match parsed {
            Some(parsed) => parsed,
            None if draws => {
                let _step = crate::spans::startup::step("core.models_parse");
                crate::model_library::ParsedModels::load(root.source())
            }
            None => crate::model_library::ParsedModels::default(),
        };
        // Connected-texture rules find their faces by sprite layer: what they match by name keeps a
        // layer of its own, decided before the models insert their textures.
        let ctm_rules = if draws {
            crate::connected::read_rules(root.source())
        } else {
            Vec::new()
        };
        sprites.keep_apart(crate::connected::ConnectedTextures::matched_sprites(
            &ctm_rules,
        ));
        if draws {
            // After the rules' names are kept apart, so a sprite a source made keeps its own layer
            // where a rule matches it.
            sprites.insert_mod_sprites()?;
        }
        let mut models = if draws {
            let _step = crate::spans::startup::step("core.models_bake");
            crate::models::prepare_with_models(root, registry, &mut sprites, &parsed)?
        } else {
            crate::models::PreparedModels::default()
        };
        let model_library = {
            let _step = crate::spans::startup::step("core.item_library");
            parsed.into_item_library()
        };
        if draws {
            // Fluids are drawn by the fluid renderer, not block models; breaking
            // progress uses the destroy-stage overlays. Ensure the missing texture
            // sprite is always present in the blocks atlas for missing-sprite fallbacks.
            sprites.insert_in_blocks("minecraft:missingno")?;
            for name in [
                "water_still",
                "water_flow",
                "water_overlay",
                "lava_still",
                "lava_flow",
            ] {
                sprites.insert_in_blocks(&format!("minecraft:block/{name}"))?;
            }
            for model in crate::fluid_models::resolved() {
                for name in [
                    Some(&model.still),
                    Some(&model.flowing),
                    model.overlay.as_ref(),
                ]
                .into_iter()
                .flatten()
                {
                    sprites.insert_in_blocks(name)?;
                }
            }
            if crate::fluid_models::any_flat_look() {
                sprites.insert_flat(crate::fluid_models::FLAT_SPRITE)?;
            }
            for stage in 0..10 {
                sprites.insert(&format!("minecraft:block/destroy_stage_{stage}"))?;
                sprites.insert(&format!("minecraft:block/destroy_oit_stage_{stage}"))?;
            }
        }
        let fusion = if draws {
            let _step = crate::spans::startup::step("core.fusion");
            crate::fusion::FusionTextures::load(
                root.source(),
                registry,
                &mut sprites,
                &mut models.catalog,
            )
        } else {
            Default::default()
        };
        let mut connected = if draws {
            // A side's sprite, for rules that connect by tile: the first quad facing it.
            let catalog = &models.catalog;
            let mut found = Vec::new();
            // Which states are panes, read only when a rule can draw on a pane by its sprite.
            let panes = if ctm_rules.iter().any(|rule| rule.match_blocks.is_empty()) {
                Some(crate::blocks::BlockFacts::load(root.prepared(), registry)?)
            } else {
                None
            };
            crate::connected::ConnectedTextures::load(
                root.source(),
                ctm_rules,
                registry,
                &mut sprites,
                |state, face| {
                    let side = sunrise_assets::model::Direction::ALL[face as usize];
                    catalog
                        .models_at(
                            state,
                            sunrise_core::position::BlockPos::new(0, 0, 0),
                            &mut found,
                        )
                        .ok()?;
                    found
                        .iter()
                        .flat_map(|m| &m.faces)
                        .find(|f| f.quad.facing == side)
                        .map(|f| f.layer)
                },
                |state| {
                    panes
                        .as_ref()
                        .and_then(|facts| facts.states.get(state.0 as usize))
                        .is_some_and(|facts| facts.is_pane())
                },
            )
        } else {
            Default::default()
        };
        // Rules that join by height read one number per block state: the highest point of its
        // collision shape. Nothing is read when no rule does.
        if connected.joins_by_height() {
            connected.set_heights(crate::connected::collision_heights(
                root.prepared(),
                registry,
            )?);
        }
        // After the connected tiles: a tile identical to a block shares its layer, and the
        // overlays skip shared layers.
        let emissive_layers = if draws {
            crate::native_emissive::load_layers(root.source(), emissive_sprites, &mut sprites)
        } else {
            Vec::new()
        };
        if draws {
            sprites.save_disk_cache();
            sprites.finish();
        }
        // `finish` grows the buffer by doubling: keep only the bytes in use.
        sprites.pixels.shrink_to_fit();
        // Multiple block sprites can share one layer. Keep the same first name that looking up
        // the layer in the name-sorted map would give.
        let mut sprite_names = vec![None; sprites.count()];
        for (name, &layer) in &sprites.layers {
            sprite_names[layer as usize].get_or_insert_with(|| name.clone());
        }
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        Ok(Self {
            sprite_size: sprites.size,
            sprite_minimum_size: sprites.minimum_size,
            texture_layers: u32::try_from(sprites.count())?,
            pixels: SpritePixels::new(sprites.pixels),
            model_catalog: models.catalog,
            model_library,
            model_placements: models.placements,
            model_gallery: models.states,
            animations: sprites.animations,
            sprite_layers: sprites.layers,
            sprite_names,
            file_layers: sprites.files,
            emissive_layers,
            translucent_layers: sprites.translucent,
            sprite_mips: sprites.mips,
            oversized_sprites: sprites.oversized,
            pbr_normals: sprites.normals,
            pbr_specular: sprites.specular,
            pbr_size: sprites.pbr_size,
            connected,
            fusion,
            mod_animations: if draws {
                crate::mod_animations::Animations::load(root.source())
            } else {
                Default::default()
            },
            // A required pipeline that doesn't build fails the load: nothing of it is published.
            mod_pipelines: if draws {
                crate::mod_pipelines::Pipelines::load(root.source(), generation)?
            } else {
                Default::default()
            },
            generation,
        })
    }
}

#[cfg(test)]
#[path = "assets_registry_tests.rs"]
mod registry_tests;

#[cfg(test)]
mod tests {
    use crate::items::ConsumeEffect;
    use sunrise_world::loot::PackOverlay;

    #[test]
    #[ignore = "daily"]
    fn archive_world_override_loads_over_real_prepared_tables_without_unpacking()
    -> crate::Result<()> {
        use crate::{
            packs::{self, Pack, Source},
            resource_source::{ResourceRoot, ResourceSource},
        };
        use std::{fs, io::Write};
        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let prepared = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .ok_or("SUNRISE_TEST_RESOURCES is required")?,
        );
        let scratch = Scratch(
            std::env::temp_dir().join(format!("sunrise-real-archive-world-{}", std::process::id())),
        );
        fs::create_dir_all(&scratch.0)?;
        let model = "assets/minecraft/models/block/stone.json";
        let original_model = fs::read(prepared.join(model))?;
        let jar = scratch.0.join("world.jar");
        let mut writer = zip::ZipWriter::new(fs::File::create(&jar)?);
        writer.start_file(model, zip::write::SimpleFileOptions::default())?;
        writer.write_all(br##"{"textures":{"all":"archive:block/stone","particle":"#all"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#all"}}}]}"##)?;
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()?
                .write_image_data(&[217, 34, 87, 255].repeat(256))?;
        }
        writer.start_file(
            "assets/archive/textures/block/stone.png",
            zip::write::SimpleFileOptions::default(),
        )?;
        writer.write_all(&png)?;
        writer.finish()?;
        let source = packs::resources(
            ResourceSource::directory(prepared.clone()),
            &[Pack::mod_layer("archive".into(), Source::Zip(jar))],
        )?;
        let root = ResourceRoot::new(prepared.clone(), scratch.0.join("cache"), source);
        let assets = super::Assets::load(root.prepared())?;
        let render = super::RenderAssets::load(&root, &assets)?;
        let stone = assets
            .registry
            .default_state("minecraft:stone")
            .ok_or("missing stone")?;
        let models = render
            .model_catalog
            .collect(stone, sunrise_core::position::BlockPos::new(0, 0, 0))?;
        assert_eq!(models[0].1.faces.len(), 1);
        let layer = render.sprite_layers["archive:block/stone"];
        assert_eq!(models[0].1.faces[0].layer, layer);
        let offset = layer as usize * (render.sprite_size * render.sprite_size * 4) as usize;
        assert_eq!(render.pixels.get()[offset..offset + 4], [217, 34, 87, 255]);
        assert_eq!(fs::read(prepared.join(model))?, original_model);
        assert!(!prepared.join("assets/archive").exists());
        assert!(!scratch.0.join("assets").exists());
        eprintln!(
            "archive-backed render loaded {} prepared states and {} sprite layers without unpacking",
            assets.registry.len(),
            render.texture_layers
        );
        Ok(())
    }

    #[test]
    fn always_edible_and_consume_effects_loaded() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("items.json").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = crate::items::Items::load(
            &root,
            assets.registry,
            &facts,
            &PackOverlay::NONE,
            &defaults,
        )
        .unwrap();

        let apple = items.get("minecraft:golden_apple").expect("golden apple");
        assert!(apple.can_always_eat);
        assert!(apple.food.is_some());
        assert!(!apple.consumable.as_ref().unwrap().effects.is_empty());

        let enchanted = items
            .get("minecraft:enchanted_golden_apple")
            .expect("enchanted golden apple");
        assert!(enchanted.can_always_eat);
        assert!(enchanted.food.is_some());
        assert!(!enchanted.consumable.as_ref().unwrap().effects.is_empty());

        let beef = items.get("minecraft:cooked_beef").expect("cooked beef");
        assert!(!beef.can_always_eat);
        assert!(beef.food.is_some());

        // Consumables without food, their style and their remainders.
        use sunrise_core::use_animation::UseAnimation;
        let milk = items.get("minecraft:milk_bucket").expect("milk bucket");
        let drink = milk.consumable.as_ref().unwrap();
        assert!(milk.food.is_none());
        assert_eq!(drink.effects, [ConsumeEffect::ClearAllEffects]);
        assert_eq!((drink.ticks(), drink.animation), (32, UseAnimation::Drink));
        assert_eq!(drink.sound, "minecraft:entity.generic.drink");
        assert!(!drink.particles);
        let remainder = |info: &sunrise_items::items::ItemInfo| {
            info.component_prototype
                .data()?
                .template_stack("minecraft:use_remainder")
                .map(|stack| stack.item)
        };
        assert_eq!(remainder(milk).as_deref(), Some("minecraft:bucket"));
        let honey = items.get("minecraft:honey_bottle").expect("honey bottle");
        let sip = honey.consumable.as_ref().unwrap();
        assert_eq!(sip.ticks(), 40);
        assert_eq!(sip.sound, "minecraft:item.honey_bottle.drink");
        assert_eq!(
            sip.effects,
            [ConsumeEffect::RemoveEffects(vec![
                "minecraft:poison".into()
            ])]
        );
        assert_eq!(remainder(honey).as_deref(), Some("minecraft:glass_bottle"));
        let kelp = items.get("minecraft:dried_kelp").unwrap();
        let bite = kelp.consumable.as_ref().unwrap();
        assert_eq!((bite.ticks(), bite.animation), (16, UseAnimation::Eat));
        assert_eq!(bite.sound, "minecraft:entity.generic.eat");
        assert!(bite.particles && remainder(kelp).is_none());
        let chorus = items.get("minecraft:chorus_fruit").unwrap();
        assert_eq!(
            chorus.consumable.as_ref().unwrap().effects,
            [ConsumeEffect::TeleportRandomly(
                sunrise_world::teleport_consume::TeleportConsume {
                    diameter: 16.0,
                    directional_particles: true,
                }
            )]
        );
        let chicken = items.get("minecraft:chicken").unwrap();
        assert!(matches!(
            chicken.consumable.as_ref().unwrap().effects[..],
            [ConsumeEffect::ApplyEffects { probability, .. }] if probability == 0.3
        ));
    }

    #[test]
    fn sprite_pixels_come_back_the_same_once_packed() {
        // Two layers of 16x16 RGBA with some structure, as sprites have.
        let pixels: Vec<u8> = (0..2 * 16 * 16 * 4).map(|i| (i * 7 % 251) as u8).collect();
        let stored = super::SpritePixels::new(pixels.clone());
        assert_eq!(*stored.get(), pixels);
        stored.pack();
        stored.pack();
        // The packing thread drops the raw copy once the compressed one is set.
        let started = std::time::Instant::now();
        while stored.0.raw.lock().unwrap().is_some() {
            assert!(started.elapsed().as_secs() < 10, "packing never finished");
            std::thread::yield_now();
        }
        assert!(
            stored
                .0
                .packed
                .get()
                .is_some_and(|p| p.iter().map(Vec::len).sum::<usize>() < pixels.len())
        );
        assert_eq!(*stored.get(), pixels);
        assert_eq!(stored.len(), pixels.len());
    }

    #[test]
    fn a_range_of_packed_pixels_reads_across_blocks() {
        // Three blocks and a bit: the ranges lie in one block, across a boundary, over
        // two boundaries, at the very end, and past it.
        let pixels: Vec<u8> = (0..3 * super::PACKED_BLOCK + 100)
            .map(|i| (i * 13 % 251) as u8)
            .collect();
        let stored = super::SpritePixels::new(pixels.clone());
        stored.pack();
        let started = std::time::Instant::now();
        while stored.0.raw.lock().unwrap().is_some() {
            assert!(started.elapsed().as_secs() < 10, "packing never finished");
            std::thread::yield_now();
        }
        for (from, bytes) in [
            (10, 20),
            (65_530, 12),
            (65_000, 70_000),
            (3 * 65_536, 100),
            (7, 0),
        ] {
            assert_eq!(
                stored.range(from, bytes).as_deref(),
                Some(&pixels[from..from + bytes]),
                "{bytes} bytes from {from}"
            );
        }
        assert_eq!(stored.range(3 * 65_536 + 1, 100), None);
        assert_eq!(stored.range(usize::MAX, 2), None);
    }
}

#[cfg(test)]
mod atlas_sprite_pixels_tests {
    use super::{RenderAssets, SpritePixels};
    use crate::{
        mipmap::Settings,
        resource_source::ResourceSource,
        sprites::{Sprites, resample},
    };
    use mod_api::{AtlasSpriteError, SpriteRef};

    fn sprite(array: u32, layer: u32) -> SpriteRef {
        SpriteRef {
            array,
            layer,
            animated: false,
            cutout: false,
        }
    }

    /// An atlas of `edge`-pixel layers holding each native frame scaled up as `Sprites::finish` does.
    fn atlas(edge: u32, natives: &[(u32, Vec<u8>)]) -> RenderAssets {
        let mut atlas = RenderAssets::unpublished();
        atlas.generation = 1;
        atlas.sprite_size = edge;
        atlas.texture_layers = natives.len() as u32;
        atlas.sprite_mips = natives
            .iter()
            .map(|(side, _)| Settings {
                native_size: *side,
                ..Default::default()
            })
            .collect();
        atlas.pixels = SpritePixels::new(
            natives
                .iter()
                .flat_map(|(side, frame)| resample(frame, *side, edge))
                .collect(),
        );
        atlas
    }

    fn frame(side: u32, seed: u8) -> Vec<u8> {
        (0..side * side * 4)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    #[test]
    fn a_small_sprite_comes_back_at_its_own_size() {
        let (small, whole, odd) = (frame(4, 1), frame(16, 2), frame(3, 3));
        let atlas = atlas(
            16,
            &[(4, small.clone()), (16, whole.clone()), (3, odd.clone())],
        );
        for (layer, side, expected) in [(0, 4, small), (1, 16, whole), (2, 3, odd)] {
            let read = atlas
                .atlas_sprite_pixels(sprite(0, layer), u32::MAX)
                .unwrap();
            assert_eq!((read.width, read.height), (side, side));
            assert_eq!(read.rgba, expected, "layer {layer}");
        }
    }

    /// A sprite larger than the array's edge comes back at its own size from the frame
    /// `Sprites::finish` kept, not from the layer it holds shrunk (#5814).
    #[test]
    fn a_sprite_above_the_edge_comes_back_at_its_own_size() {
        let big = frame(8, 9);
        let mut shrunk = atlas(2, &[(2, frame(2, 1)), (2, frame(2, 2))]);
        shrunk.sprite_mips[1].native_size = 8;
        shrunk.oversized_sprites.insert(1, (8, big.clone()));
        let read = shrunk.atlas_sprite_pixels(sprite(0, 1), u32::MAX).unwrap();
        assert_eq!((read.width, read.height), (8, 8));
        assert_eq!(read.rgba, big);
        // The layer beside it still reads from the array.
        let other = shrunk.atlas_sprite_pixels(sprite(0, 0), u32::MAX).unwrap();
        assert_eq!(other.rgba, frame(2, 1));
    }

    /// `Sprites::finish` keeps the frame each layer larger than the edge starts on, at its own
    /// size, and nothing of a layer that fits.
    #[test]
    fn the_frame_a_shrunk_layer_starts_on_is_kept_at_its_own_size() {
        use std::sync::Arc;
        let (small, big, second) = (frame(16, 3), frame(32, 4), frame(32, 5));
        let native = [
            (16, Arc::new(vec![small]), 0),
            (32, Arc::new(vec![big, second.clone()]), 1),
            (16, Arc::new(vec![frame(16, 6)]), 0),
        ];
        let kept = crate::sprites::oversized_frames(&native, 16);
        assert_eq!(kept.into_iter().collect::<Vec<_>>(), [(1, (32, second))]);
        assert!(crate::sprites::oversized_frames(&native, 32).is_empty());
    }

    #[test]
    fn arrays_split_the_layers() {
        let atlas = atlas(2, &[(2, frame(2, 1)), (2, frame(2, 2)), (2, frame(2, 3))]);
        let read = atlas.atlas_sprite_pixels(sprite(1, 0), 2).unwrap();
        assert_eq!(read.rgba, frame(2, 3));
        let second = atlas.atlas_sprite_pixels(sprite(0, 1), 2).unwrap();
        assert_eq!(second.rgba, frame(2, 2));
    }

    #[test]
    fn an_animated_sprite_reads_back_the_frame_it_starts_on() {
        // A sheet of two solid 16 x 16 frames, the animation starting on the second.
        let root =
            std::env::temp_dir().join(format!("sunrise-sprite-start-{}", std::process::id()));
        let directory = root.join("assets/test/textures");
        std::fs::create_dir_all(&directory).unwrap();
        let sheet: Vec<u8> = [[10, 20, 30, 255], [200, 100, 50, 255]]
            .iter()
            .flat_map(|texel| texel.repeat(16 * 16))
            .collect();
        let path = directory.join("start.png");
        let mut encoder = png::Encoder::new(std::fs::File::create(&path).unwrap(), 16, 32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&sheet)
            .unwrap();
        std::fs::write(
            path.with_extension("png.mcmeta"),
            r#"{"animation":{"frames":[1,0]}}"#,
        )
        .unwrap();
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert("test:start").unwrap();
        sprites.finish();
        std::fs::remove_dir_all(&root).unwrap();
        let mut atlas = RenderAssets::unpublished();
        atlas.generation = 1;
        atlas.sprite_size = sprites.size;
        atlas.texture_layers = sprites.count() as u32;
        atlas.sprite_mips = sprites.mips;
        atlas.pixels = SpritePixels::new(sprites.pixels);
        let read = atlas
            .atlas_sprite_pixels(sprite(0, layer), u32::MAX)
            .unwrap();
        assert_eq!((read.width, read.height), (16, 16));
        assert_eq!(read.rgba, [200, 100, 50, 255].repeat(16 * 16));
    }

    #[test]
    fn a_reference_with_no_sprite_is_refused_and_nothing_published_is_not_ready() {
        let atlas = atlas(2, &[(2, frame(2, 1))]);
        for bad in [
            sprite(0, 1),
            sprite(1, 0),
            sprite(0, 2),
            sprite(u32::MAX, u32::MAX),
        ] {
            assert_eq!(
                atlas.atlas_sprite_pixels(bad, 2),
                Err(AtlasSpriteError::NoSuchSprite)
            );
        }
        assert_eq!(
            RenderAssets::unpublished().atlas_sprite_pixels(sprite(0, 0), 1),
            Err(AtlasSpriteError::NotReady)
        );
    }
}
