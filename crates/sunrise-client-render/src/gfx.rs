//! GPU renderer: terrain, sky, block outline and HUD. Camera-relative section origins keep world coordinates precise.
use crate::{
    Result,
    block_entity_render::{StaticCacheFrameStats, StaticSectionRevision, StaticSectionSnapshot},
    environment::Sky,
    fast_hash::FxHashMap,
    graphics::GraphicsApi,
    hud::{HudBatch, HudRenderer},
    mesher::{Part as Quads, SectionMesh, Sides, TerrainVertex, Vertex, packed},
    section_mask::SectionMask,
    sprites::SpriteAnimation,
};
#[cfg(test)]
use glam::Vec4;
use glam::{DVec3, Mat4, Vec3};
use pack_post::PackPostExt as _;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};
use wgpu::util::DeviceExt;
use winit::window::Window;
// RenderPipelines.CRUMBLING: dst * src + src * dst, alpha replaced.
const CRUMBLING_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Dst,
        dst_factor: wgpu::BlendFactor::Src,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent::REPLACE,
};
/// `TerrainVertex` for `vs_terrain`: position, UV, layer, light, colour.
const TERRAIN_ATTRIBUTES: [wgpu::VertexAttribute; 5] =
    wgpu::vertex_attr_array![0=>Float32x3,1=>Uint16x2,2=>Uint16,4=>Uint8x2,3=>Unorm8x4];

fn no_pack_hdr_gain(layered: bool, pack: bool, world: bool) -> bool {
    layered && !pack && world
}

fn light_marks(format: wgpu::TextureFormat, hdr: bool) -> bool {
    hdr && format != LINEAR_FRAME
}

fn terrain_shader() -> String {
    include_str!("hdr_light.wgsl").to_owned() + include_str!("terrain.wgsl")
}

pub(super) use border::border_shader;

pub mod border;
#[cfg(test)]
mod crumbling_tests;
mod gpu_checks;
mod section_gpu;
mod static_entity;

pub(crate) use section_gpu::collect_animated_layers;
use section_gpu::{
    Part, SectionGpu, begin_shrink, decide_fade, forget_records, hold_fading, mark_fades,
};
use static_entity::{GpuStaticEntitySection, StaticEntityUniform};
pub mod outline;
mod overlay;
pub use border::BorderDraw;
use border::BorderUniform;
use gpu_checks::GpuChecks;
use outline::{debug_line_opaque, debug_lines_opaque_first, outline_vertices_into};
pub use outline::{outline_params, outline_vertices, outline_width};
use overlay::Overlay;
mod dynamic_entity;
mod entity_batch;
mod entity_outline;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod entity_pixel_test;
mod fluid;
mod prepared_model;
#[cfg(feature = "test-support")]
pub use prepared_model::fail_next_prepared_upload;
pub use prepared_model::{
    PreparedInstanceBuilder, PreparedModelDraw, PreparedModelInstance, PreparedModelVertex,
    PreparedModels, camera_matrix, prepared_instance_room_for,
};
#[cfg(feature = "test-support")]
pub use prepared_model::{
    cap_prepared_instance_slots, capped_prepared_room, set_test_prepared_generation,
    test_prepared_generation,
};
mod block_entity_positions;
mod culling_control;
mod draw_frame;
mod entity_draw;
mod hdr_config;
#[cfg(test)]
mod inter_stage_tests;
mod renderer_construction;
mod section_fade;
mod section_upload;
mod shader_pack;
mod surface_lifecycle;
use entity_draw::*;
#[cfg(test)]
mod sprite_arrays_tests;
mod terrain_model;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_gpu;
#[cfg(feature = "test-support")]
pub use test_gpu::lock as lock_test_gpu;
mod weather;
mod world_text;
use cull::occlusion_culling_requested;
use entity_batch::EntityBatch;
pub use entity_batch::{
    ADDITIVE_PARTICLE_OVERLAY, ARMOR_GLINT_ONLY_OVERLAY, BANNER_PATTERN_OVERLAY, DISSOLVE_OVERLAY,
    DRAGON_RAYS_OVERLAY, DYNAMIC_TEXTURE_CUTOUT_OVERLAY, DYNAMIC_TEXTURE_SOLID_OVERLAY,
    DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY, ENERGY_SWIRL_OVERLAY, EntityQuadCapacity, EntityQuadScope,
    FLUID_OVERLAY, GLINT_ONLY_OVERLAY, OIT_ENTITY, SHADOW_OVERLAY,
    TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY, TERRAIN_MODEL_CUTOUT_OVERLAY, TERRAIN_MODEL_SOLID_OVERLAY,
    TERRAIN_MODEL_TRANSLUCENT_OVERLAY, TEXT_NORMAL_SOLID_OVERLAY, TEXT_POLYGON_OFFSET_OVERLAY,
    TEXT_POLYGON_OFFSET_SOLID_OVERLAY, TRANSLUCENT_DRAW_FLIP, TRANSLUCENT_GLINT_ITEM_OVERLAY,
    TRANSLUCENT_ITEM_OVERLAY, TRANSLUCENT_PARTICLE_OVERLAY, WATER_MASK_OVERLAY, WORLD_TEXT_OVERLAY,
    WORLD_TEXT_POLYGON_OFFSET_OVERLAY, dynamic_texture_bits, is_native_terrain, mark_oit,
};
pub use entity_batch::{NAME_TAG_OVERLAY, NAME_TAG_SOLID_OVERLAY};
use external::{shadow_box, shadow_runs};
pub use frustum::{Cull, JoinedFrusta, ShadowCull};
use frustum::{block_entities_within_reach, frustum, shadow_casters, taa_frustum};
pub use mod_gpu::ModGpuResources;
pub use section_fade::{CHUNK_VISIBILITY_THRESHOLD, chunk_visibility, fade_millis};
use section_fade::{FadeInputs, compiled_and_visible, section_instance, section_visibility};
#[doc(hidden)]
pub use section_upload::section_in_eye_view;
pub use sunrise_render_core::SectionKey;
pub use view::{View, ViewSet, cull_fov, cull_frustum};

/// Whether Translucent Batching is offered (`graphics::offer_translucent_batching`): where
/// indirect multi-draws (`multi_draw`) take its batches, but not OpenGL's, which loop over
/// their entries on the CPU (and without them every section is a call of its own anyway).
/// Frames drawn flat (Metal, Intel Gen11/Gen12's Vulkan driver) take it too: their
/// translucent calls then come one per page of its arena rather than about one per section.
fn translucent_batchable(info: &wgpu::AdapterInfo, multi_draw: bool) -> bool {
    multi_draw && info.backend != wgpu::Backend::Gl
}

/// The vertices flat draws bind for a call's page key: a page of the section arena, or of
/// the translucent arena up to its last range in use (`flat::TRANSLUCENT_KEYS`).
fn flat_vertices<'a>(
    arena: &'a arena::Arena,
    translucent: Option<&'a arena::Arena>,
) -> impl Fn(u32) -> wgpu::BufferSlice<'a> + Copy {
    move |key| match (translucent, key.checked_sub(flat::TRANSLUCENT_KEYS)) {
        (Some(translucent), Some(page)) => translucent.binding(page),
        _ => arena.buffer(key).slice(..),
    }
}

/// Bytes of `texture` with every mip level of every layer.
pub(crate) fn texture_bytes(texture: &wgpu::Texture) -> u64 {
    let size = texture.size();
    let block = u64::from(texture.format().block_copy_size(None).unwrap_or(4));
    (0..texture.mip_level_count())
        .map(|level| {
            let side = |v: u32| u64::from((v >> level).max(1));
            side(size.width) * side(size.height) * block
        })
        .sum::<u64>()
        * u64::from(size.depth_or_array_layers)
}

/// Whether the section arena's pages are storage too: where flat draws read them (`flat`), or
/// while a `shaders/` pack's terrain programs read them per section (`pages`,
/// `pack_atlas::TerrainDraws::Pages`). Otherwise they are vertex buffers alone, which the device's
/// storage binding size does not cap (`Arena::page_limit`).
fn arena_storage(flat: bool, pages: bool) -> bool {
    flat || pages
}

/// Whether translucent quads have an arena of their own: with Translucent Batching
/// (`batching`), unless a shader pack's frames can draw terrain flat (as for `flat_frame`),
/// which reads them in the section arena through its tables: where flat draws exist and
/// are on (`flat_draws`), for a `shaders/` pack's terrain programs (`terrain`) or a shadow
/// pass (`shadow`: a `shaders/` pack's, or the `[shadow]` of one of Sunrise's own). Nor while
/// a `shaders/` pack's terrain or shadow programs read the section arena's pages in per-section
/// draws (`pages`, `pack_atlas::TerrainDraws::Pages`): the translucent arena is no storage.
fn translucent_split(
    batching: bool,
    flat_draws: bool,
    pages: bool,
    terrain: bool,
    shadow: bool,
) -> bool {
    batching && !(flat_draws && (terrain || shadow)) && !pages
}

/// The compact layout of solid quads, unless `SUNRISE_PACKED_QUADS=0` (A/B checks).
fn packed_wanted() -> bool {
    std::env::var("SUNRISE_PACKED_QUADS").as_deref() != Ok("0")
}

pub(crate) mod animate;
mod arena;
pub mod capture;
mod celestial;
mod census;
pub use census::RendererMemorySnapshot;
pub mod compositor;
pub(crate) mod core_clouds;
pub(crate) mod core_lightmap;
pub use core_lightmap::SHADERS as LIGHTMAP_SHADERS;
pub(crate) mod core_lines;
mod core_particles;
mod particle_draws;
pub use core_particles::HEIGHTMAP_SIZE as POLYTONE_HEIGHTMAP_SIZE;
pub(crate) use particle_draws::EXPLICIT_PARTICLE_OVERLAY;
pub(crate) mod core_sky;
pub(crate) mod core_text;
mod cull;
pub(crate) mod display_watch;
mod dx12_compiler;
mod end_sky;
pub mod entity_skin;
pub mod expression_blocks;
pub mod external;
mod flat;
mod frame_gen;
mod frustum;
mod gpu_culling;
mod hdr;
mod hdr_icc;
mod hdr_start;
mod hdr_stats;
pub(crate) mod hud_coverage;
mod in_flight;
mod sourced_geometry;
mod terrain_normals;
use display_watch::DisplayWatch;
#[cfg(test)]
use display_watch::{macos_hdr, os_reported};
pub use hdr::{Calibration as HdrCalibration, Output as HdrOutput};
pub use output::{DisplayReading, EffectiveOutput, FrameConfiguration, FrameRequest};

mod custom_sky;
mod frame_policy;
pub mod lightning;
mod linear_blend;
#[cfg(test)]
use frame_policy::frame_format;
use frame_policy::{frame_configuration, overlay_format, sunrise_tone_curve};
mod lod;
pub mod mod_compute;
pub mod mod_frame_graph;
pub mod mod_gpu;
pub mod mod_meshes;
pub mod mod_pipelines;
pub mod mod_readback;
pub mod mod_render_targets;
pub mod mod_storage;
pub mod mod_uniforms;
use crate::render_slots as native_render;
mod occlusion;
mod oit;
mod oit_features;
pub(crate) mod output;
mod output_apply;
mod pack_entities;
#[doc(hidden)]
pub mod visibility_probe;
// The pack's programs' inputs and helpers, from `sunrise-render-shaderpack`: the client's frame
// only tells them what a `Frame` and the game's own values are.
pub use sunrise_render_shaderpack::{
    pack_atlas, pack_draw, pack_hand, pack_lines, pack_sky, pack_timing, pack_weather,
};
#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod pack_atlas_tests;
mod pack_post;
// The client's `Frame` as the shadow light reads it: a `UniformFrameView`; the uniforms
// themselves come from `sunrise-render-shaderpack`.
pub mod pack_uniforms;
mod pass_timing_api;
use pass_timing_api::{after_culling_pass, pixel_count, world_pass_after_culling};
mod page_runs;
pub mod panorama;
mod readback;
mod resident;
mod reuse;
mod screenshot;
pub use screenshot::{PendingScreenshot, ScreenshotSave};
mod screenshot_readback;
mod stars;
mod surface_setup;
use surface_setup::{AdapterChoice, AdapterFailure, open_adapter, pick_adapter, set_up_surface};
mod off_page;
use off_page::move_off_page;
pub mod submit;
mod sunrise_fan;
mod timing;
pub mod translucency;
pub(crate) mod upload;
mod view;
mod view_sections;
mod visbuffer;
mod world_handover;
mod world_post;
pub mod world_primitives;
mod world_views;

/// A section's instance: its camera-relative origin and, in `w`, the step of
/// its fixed-point UVs (`terrain.wgsl`'s `terrain_uv`).
/// The world pass again after occlusion culling's test: it loads both
/// targets (phase 2 draws over phase 1, reading the test's results).
/// `LevelRenderer.addSkyPass`: fluids and camera effects block the sky pass.
fn sky_open(frame: &Frame) -> bool {
    !frame.sky_blocked
}

/// The Overworld-style sky (disc, sunset, stars, void) draws this frame.
fn sky_visible(frame: &Frame) -> bool {
    !frame.title && frame.skybox && sky_open(frame)
}

/// The End during the dragon fight, whose world fog leaves out vanilla's sky: a shader pack's
/// `gbuffers_skybasic` still draws a fan around the horizon (`pack_sky`), as our probe packs read
/// it in Iris 1.11.7.
fn end_fog_sky(frame: &Frame) -> bool {
    pack_sky::end_fog_sky(frame.title, frame.end_sky, frame.boss_world_fog)
}

/// The point a `shaders/` pack's terrain is drawn relative to: the camera with the fraction of
/// each coordinate rounded to the nearest 1/65536 block. Read back with a probe pack of ours,
/// Iris 1.11.7's terrain `gl_Vertex` is the block's position minus this, for cameras from 13
/// to 40000 blocks out (#8464); `cameraPosition` and the matrices keep the exact camera. Off by
/// up to 2^-17 blocks, a fragment on a texel boundary of the shadow map reads the other texel.
fn pack_terrain_camera(camera: DVec3) -> DVec3 {
    let whole = camera.floor();
    whole + ((camera - whole) * 65536.0).round() / 65536.0
}
/// A new value for `Renderer::block_entity_revision`.
fn next_block_entity_revision() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

const MAX_OUTLINE_EDGES: u64 = 256;
/// Staging buffer size for section meshes: a frame of uploads while
/// exploring fits in a few.
const SECTION_UPLOAD_CHUNK: u64 = 4 << 20;
/// Section copies are submitted early past this many staged bytes, which
/// bounds the staging memory frames that are not drawn can hold.
const SECTION_COPY_LIMIT: u64 = 64 << 20;
/// What compaction copies in a frame, `compact_arena` first and
/// `shrink_translucent` with what it left, and a range more where a call's
/// first is larger (`move_off_page`): a page empties over a few frames, with
/// no hitch.
const COMPACT_BUDGET: u64 = 32 << 20;
/// A large translucent page starts to shrink (`Renderer::shrink_translucent`)
/// this long after terrain last arrived, and `SHRINK_RETRY` after a try found
/// no room.
/// `Renderer::remove_columns` filters each store once rather than looking up every
/// section key from this many keys, or from as many as there are resident sections.
const BULK_REMOVAL_KEYS: usize = 256;
const SHRINK_SETTLE: std::time::Duration = std::time::Duration::from_secs(1);
const SHRINK_RETRY: std::time::Duration = std::time::Duration::from_secs(5);
mod renderer;
pub use renderer::Renderer;
/// Every layer's mip levels, layer major (`first` is the first layer's
/// index into `settings`), made by `MipmapGenerator`.
/// Block sprite layers per texture array at most: the
/// `max_texture_array_layers` of most GPUs. Mods add sprites past it into more
/// arrays (terrain.wgsl's `sprite`); a GPU with fewer layers per array gets a
/// smaller split ([`layer_split`]).
pub(crate) const SPRITE_SPLIT: u32 = 2048;
/// A smaller split from `SUNRISE_SPRITE_SPLIT` (dev only: vanilla's sprites
/// drawn from several arrays must look the same).
fn requested_split() -> Option<u32> {
    std::env::var("SUNRISE_SPRITE_SPLIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| (1..=SPRITE_SPLIT).contains(&n))
}
fn adapter_requested_split(info: &wgpu::AdapterInfo, asked: Option<u32>, n: u32) -> Option<u32> {
    // Metal can fail on big mipmapped arrays (#8471): 512 layers each, more only for n > 8 x 512.
    asked.or((info.backend == wgpu::Backend::Metal).then(|| n.div_ceil(SPRITE_ARRAYS).max(512)))
}
/// The split the CPU mip chains are cut by before the adapter is known.
fn sprite_split() -> u32 {
    requested_split().unwrap_or(SPRITE_SPLIT)
}
/// The block sprite layers per texture array on an adapter that holds
/// `max_layers` layers per array: [`SPRITE_SPLIT`], `requested`
/// (`SUNRISE_SPRITE_SPLIT`) or `max_layers`, whichever is smallest, with a full
/// array still fitting once [`array_layers`] pads it for OpenGL. Errors when
/// `layers` sprites need more than the [`SPRITE_ARRAYS`] arrays terrain.wgsl binds.
fn layer_split(
    max_layers: u32,
    gl: bool,
    requested: Option<u32>,
    layers: u32,
) -> std::result::Result<u32, String> {
    let mut split = requested.unwrap_or(SPRITE_SPLIT).min(max_layers).max(1);
    if split > 1 && array_layers(split, gl) > max_layers {
        split -= 1;
    }
    if layers > split * SPRITE_ARRAYS {
        return Err(format!(
            "GPU supports {max_layers} texture layers per array; {layers} block textures need \
             more than the {SPRITE_ARRAYS} arrays of {split} Sunrise draws from"
        ));
    }
    Ok(split)
}
/// Sprite arrays terrain.wgsl binds (`sprites`, `sprites_1`...).
const SPRITE_ARRAYS: u32 = 8;
/// The layers to give a square texture of `count` layers viewed as a 2D
/// array. wgpu's OpenGL backend picks a texture's target from its layer
/// count when it is created: one layer makes a 2D texture and a multiple of
/// six a cube map, neither of which can be viewed as an array.
pub(crate) fn array_layers(count: u32, gl: bool) -> u32 {
    match count {
        _ if !gl => count,
        0 | 1 => 2,
        n if n % 6 == 0 => n + 1,
        n => n,
    }
}
/// The views of terrain.wgsl's sprite arrays, the unused ones a 1-layer
/// stand-in.
fn sprite_views(
    textures: &[wgpu::Texture],
    unused: &wgpu::TextureView,
    mips: Option<u32>,
) -> Vec<wgpu::TextureView> {
    (0..SPRITE_ARRAYS as usize)
        .map(|i| match textures.get(i) {
            Some(t) => t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                mip_level_count: mips,
                ..Default::default()
            }),
            None => unused.clone(),
        })
        .collect()
}
/// The block sprites sampled sharp, for native mods (`WorldDrawn::block_sprites`): nearest,
/// repeating, the nearest mip level.
const SPRITE_NEAREST: wgpu::SamplerDescriptor<'static> = wgpu::SamplerDescriptor {
    label: Some("Block sprites (nearest)"),
    address_mode_u: wgpu::AddressMode::Repeat,
    address_mode_v: wgpu::AddressMode::Repeat,
    address_mode_w: wgpu::AddressMode::ClampToEdge,
    mag_filter: wgpu::FilterMode::Nearest,
    min_filter: wgpu::FilterMode::Nearest,
    mipmap_filter: wgpu::MipmapFilterMode::Nearest,
    lod_min_clamp: 0.0,
    lod_max_clamp: 32.0,
    compare: None,
    anisotropy_clamp: 1,
    border_color: None,
};
/// The terrain sprites' sampler: LevelRenderer.chunkLayerSampler's linear one (clamped to the
/// edge) in every Texture Filtering mode, with `anisotropy` samples when it is above 1 (wgpu
/// then needs a linear mipmap filter too).
fn sprite_sampler_descriptor(anisotropy: u16) -> wgpu::SamplerDescriptor<'static> {
    wgpu::SamplerDescriptor {
        label: Some("Block sprites (filtered)"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: anisotropy,
        ..Default::default()
    }
}
/// `Options.maxAnisotropyValue` for a `maxAnisotropyBit`: `1 << bit`, at most wgpu's 16.
fn sampler_anisotropy(bit: i32) -> u16 {
    1u16 << bit.clamp(0, 4)
}
/// terrain.wgsl's group 0.
#[allow(clippy::too_many_arguments)]
fn terrain_group(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    sprites: &[wgpu::TextureView],
    sprite_sampler: &wgpu::Sampler,
    lightmap: &wgpu::TextureView,
    lightmap_sampler: &wgpu::Sampler,
    emission_thresholds: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let view = |binding, view| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            view(1, &sprites[0]),
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sprite_sampler),
            },
            view(3, lightmap),
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(lightmap_sampler),
            },
            view(5, &sprites[1]),
            view(6, &sprites[2]),
            view(7, &sprites[3]),
            view(8, &sprites[4]),
            view(9, &sprites[5]),
            view(10, &sprites[6]),
            view(11, &sprites[7]),
            view(12, emission_thresholds),
        ],
    })
}
mod prepared_textures;
pub use prepared_textures::*;
mod solid_pipeline;
pub use solid_pipeline::*;
mod frame;
pub use frame::*;
mod device;
pub use device::*;
/// Logs a step of the renderer's start, for a native crash in the graphics
/// driver: it leaves no panic or crash report, so the log's last line is the
/// only sign of where the game stopped. The log file is written unbuffered;
/// the flush is for loggers that buffer.
fn startup_step(step: std::fmt::Arguments) {
    log::info!("Sunrise renderer: {step}");
    log::logger().flush();
}
/// How far the first frame on a new device got, for `startup_step`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FirstFrame {
    Acquire,
    Submit,
    Drawn,
}
fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}
impl Renderer {
    /// The first-person hand's quads, which follow `first` quads of the entity buffer.
    /// Unmarks the clouds just drawn (`fs_hdr_cloud_mark`).
    fn mark_clouds(&self, pass: &mut wgpu::RenderPass<'_>, quads: u32) {
        if !light_marks(self.output.frame.format, self.hdr.is_some())
            || self.hdr.as_ref().is_none_or(|h| !h.lighting)
        {
            return;
        }
        pass.set_pipeline(&self.hdr_cloud_mark_pipeline);
        pass.set_bind_group(0, &self.entity_group, &[]);
        pass.set_bind_group(1, &self.cloud_group, &[]);
        pass.set_vertex_buffer(0, self.cloud_buffer.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..quads * 6, 0, 0..1);
        pass.set_bind_group(0, &self.group, &[]);
    }
    /// Unmarks translucent terrain, over the draws that just blended it (`fs_translucent_mark`).
    fn mark_translucent_terrain(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        flat: Option<&flat::Flat>,
        (indirect, base): (&wgpu::Buffer, u32),
        solid_draws: u32,
        translucent_draws: u32,
    ) {
        if !light_marks(self.output.frame.format, self.hdr.is_some())
            || self.hdr.as_ref().is_none_or(|h| !h.lighting)
        {
            return;
        }
        match flat {
            Some(flat) => {
                pass.set_pipeline(&flat.translucent_mark);
                flat.draw(
                    pass,
                    flat.translucent_calls(),
                    flat_vertices(&self.arena, self.translucent_arena.as_ref()),
                );
                pass.set_bind_group(0, &self.group, &[]);
            }
            None => {
                pass.set_pipeline(&self.translucent_mark);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                self.draw_sections(pass, indirect, base, solid_draws, translucent_draws);
            }
        }
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
    }
    /// The shared atlas's LabPBR maps (`HudAtlas::material_maps`), uploaded for the shader packs
    /// loaded from now on; a map too large for this GPU is left out.
    pub fn set_shared_material_maps(&mut self, maps: &[Option<crate::paged_maps::PagedMap>; 2]) {
        let limit = self.device.limits().max_texture_dimension_2d;
        let sizes: Vec<String> = (maps.iter().zip(["normals", "specular"]))
            .filter_map(|(m, name)| {
                m.as_ref()
                    .map(|m| format!("{name} {}x{}", m.width, m.height))
            })
            .collect();
        if !sizes.is_empty() {
            let bytes: usize = maps.iter().flatten().map(|m| m.pixels.len()).sum();
            log::info!(
                "shared atlas LabPBR maps: {} ({:.1} MiB)",
                sizes.join(", "),
                bytes as f64 / f64::from(1 << 20)
            );
        }
        self.shared_material_maps = maps.each_ref().map(|map| {
            let map = map.as_ref().filter(|m| m.width.max(m.height) <= limit)?;
            Some(self.device.create_texture_with_data(
                &self.queue,
                &wgpu::TextureDescriptor {
                    label: Some("Shared atlas material map"),
                    size: wgpu::Extent3d {
                        width: map.width,
                        height: map.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &map.pixels,
            ))
        });
    }
    /// The pack's custom sky layer textures (`crate::custom_sky::load`).
    pub fn set_custom_sky(&mut self, pack: &dyn crate::custom_sky::SkyLayers) {
        self.custom_sky.set_pack(&self.device, &self.queue, pack);
    }
    /// The custom sky layers the next frame draws (`SkyLayers::draws`): the game refills it each
    /// frame, empty when the world's sky has none.
    pub fn custom_sky_draws(&mut self) -> &mut Vec<crate::custom_sky::Draw> {
        &mut self.custom_sky_draws
    }
    fn quad_indices(device: &wgpu::Device, quads: u32) -> wgpu::Buffer {
        surface_setup::ray_query::quad_indices(device, quads)
    }
    fn offscreen_target(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Offscreen frame"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: config.usage,
            view_formats: &config.view_formats,
        })
    }
    /// The share of real frames frame generation shows a made frame before (1 while it
    /// keeps up), `None` while it isn't running.
    /// The posed models' store size (`SkinFrame::base`), with entity skinning.
    pub fn skinned_models(&self) -> Option<u32> {
        self.entity_skin
            .as_ref()
            .map(entity_skin::EntitySkin::stored)
    }

    /// Whether ordinary entity models may be posed on the GPU (`Frame::skin`, `entity_skin::poses`).
    pub fn skins_entities(&self) -> bool {
        self.entity_skin.is_some()
            && entity_skin::poses(
                self.frame_gen.on,
                (self.pack_post.as_ref())
                    .map(|(_, p)| p.as_ref().is_some_and(|p| p.skins_entities())),
                || native_render::needs().entity_spans,
            )
    }

    /// Whether the layers tagged `OIT_ENTITY` (the eyes render type's, horse
    /// markings, a slime's outer cube) may be posed on the GPU too
    /// (`mobs::skin::SkinModels::tagged`): only with improved transparency off,
    /// where they are ordinary entity quads. On, `EntityBatch` sorts them into
    /// its stages by the CPU vertices' `OIT_ENTITY` tag.
    pub fn skins_tagged(&self) -> bool {
        self.skins_entities() && !self.improved_transparency && self.pack_post.is_none()
    }

    pub fn frame_generation_share(&self) -> Option<f32> {
        self.frame_gen.made_share
    }
    /// Frame generation on or off (FSR 3.1's frame interpolation).
    pub fn set_frame_generation(&mut self, on: bool) {
        if !on {
            self.frame_gen.stop();
        }
        self.frame_gen.on = on;
    }
    /// `upscale`: output pixels per rendered pixel, for TAA to rebuild (1: native).
    /// `sharpness`: how much TAA's upscaled picture is sharpened, 0..=1.
    pub fn set_world_post(
        &mut self,
        scale: f32,
        antialias: bool,
        taa: bool,
        upscale: f32,
        sharpness: f32,
    ) {
        self.world_post.set_sharpness(&self.device, sharpness);
        let taa = taa && self.world_post.supports_taa();
        self.world_post.scale = scale.clamp(0.25, 2.0);
        self.world_post.antialias = antialias && !taa;
        self.world_post.taa = taa;
        self.world_post.upscale = if taa { upscale.clamp(1.0, 4.0) } else { 1.0 };
    }
    /// The Texture Filtering and Max Anisotropy options (`filtering`: 0 none, 1 RGSS, 2
    /// anisotropic; `bit`: `maxAnisotropyBit`). All take LevelRenderer's chunk layer sampler
    /// (linear, clamped, `bit` anisotropy when anisotropic) and terrain.wgsl's sampleNearest,
    /// or sampleRGSS for RGSS.
    pub fn set_texture_filtering(&mut self, filtering: i32, bit: i32) {
        let filtering = filtering.clamp(0, 2) as u32;
        let anisotropy = if filtering == 2 {
            sampler_anisotropy(bit)
        } else {
            1
        };
        if filtering == self.texture_filtering && anisotropy == self.max_anisotropy {
            return;
        }
        self.texture_filtering = filtering;
        self.max_anisotropy = anisotropy;
        self.sprite_sampler = self
            .device
            .create_sampler(&sprite_sampler_descriptor(anisotropy));
        self.rebuild_terrain_group();
    }
    fn rebuild_terrain_group(&mut self) {
        self.sprite_views = sprite_views(
            &self.sprite_textures,
            &self.sprite_unused,
            Some(self.mipmaps + 1),
        );
        self.group = terrain_group(
            &self.device,
            "Sunrise terrain resources",
            &self.terrain_layout,
            &self.globals,
            &self.sprite_views,
            &self.sprite_sampler,
            &self.lightmap_view,
            &self.lightmap_sampler,
            &self.emission_stats.view,
        );
        self.fluid_group = terrain_group(
            &self.device,
            "Still fluid resources",
            &self.terrain_layout,
            &self.globals,
            &self.sprite_views,
            &self.fluid_sampler,
            &self.lightmap_view,
            &self.lightmap_sampler,
            &self.emission_stats.view,
        );
        self.terrain_model_group = terrain_group(
            &self.device,
            "Native model resources",
            &self.terrain_layout,
            &self.globals,
            &self.sprite_views,
            &self.terrain_model_sampler,
            &self.lightmap_view,
            &self.lightmap_sampler,
            &self.emission_stats.view,
        );
    }
    /// Block sprite layers per array on this device: a sprite's array is its layer over it, its
    /// layer in that array the rest (`block_sprites::Lookup`).
    pub fn sprite_split(&self) -> u32 {
        self.sprite_split
    }
    /// The Mipmap Levels option: block textures sample at most `levels`
    /// reduced copies (0: none).
    pub fn set_mipmap_levels(&mut self, levels: u32) {
        self.mipmap_option = levels;
        let levels = crate::sprites::sprite_mipmap_limit(
            levels,
            self.sprite_textures[0].width(),
            self.sprite_minimum_size,
            self.mip_levels,
        );
        if levels == self.mipmaps {
            return;
        }
        self.mipmaps = levels;
        self.rebuild_terrain_group();
    }
    fn animate(&mut self, encoder: &mut wgpu::CommandEncoder, tick: u64, cull_inactive: bool) {
        let active = cull_inactive.then_some(&self.active_animated_layers[..]);
        self.animations.animate_modes(
            encoder,
            &mut self.uploads,
            tick,
            active,
            self.own_timing.as_mut(),
        );
        if let Some(arrays) = &self.pbr_maps {
            for animation in &self.material_animations {
                if !self.animations.changed().contains(&animation.layer) {
                    continue;
                }
                let Some(pair) = arrays.get((animation.layer / self.sprite_split) as usize) else {
                    continue;
                };
                for kind in 0..2 {
                    let Some(pixels) = animation.material_pixels(kind, tick) else {
                        continue;
                    };
                    let texture = &pair[kind];
                    let from = (pixels.len() / 4).isqrt() as u32;
                    for level in 0..texture.mip_level_count() {
                        let size = texture.width() >> level;
                        let pixels = crate::sprites::resample(pixels, from, size);
                        self.uploads.texture(
                            encoder,
                            wgpu::TexelCopyTextureInfo {
                                texture,
                                mip_level: level,
                                origin: wgpu::Origin3d {
                                    x: 0,
                                    y: 0,
                                    z: animation.layer % self.sprite_split,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            size,
                            size,
                            &pixels,
                        );
                    }
                }
            }
            if let Some(atlases) = &self.pack_material_atlases {
                for kind in 0..2 {
                    let arrays: Vec<_> = arrays.iter().map(|pair| pair[kind].clone()).collect();
                    atlases[kind].copy(
                        &self.device,
                        encoder,
                        &arrays,
                        self.animations.changed().iter().copied(),
                    );
                }
            }
        }
        if let Some(atlas) = &self.pack_atlas {
            atlas.copy(
                &self.device,
                encoder,
                &self.sprite_textures,
                self.animations.changed().iter().copied(),
            );
        }
    }
    /// Device `max_buffer_size`, for instance-room checks while `Renderer` is borrowed by `render`.
    pub fn max_buffer_size(&self) -> u64 {
        self.mod_gpu.max_buffer_size()
    }
    /// See [`ModGpuResources::mod_binding_limits`].
    pub fn mod_binding_limits(&self) -> (u64, u64) {
        self.mod_gpu.mod_binding_limits()
    }
    /// Render `frame`, optionally driving mod world-render hooks (`FrameHooks`) at the renderer's
    /// real visibility, sky, prepare, pass-stage, and post-world boundaries.
    #[sunrise_mod_api_macros::replaceable]
    pub fn render(
        &mut self,
        frame: &Frame,
        hooks: Option<&mut dyn world_primitives::FrameHooks>,
    ) -> Result<bool> {
        // A lost device cannot accept uploads either. Leave recovery to the
        // owner instead of submitting atlas patches to the invalid device.
        if self.device_lost() {
            return Err("the graphics device was lost".into());
        }
        if let Some(request) = self.capture.take() {
            let result = self.write_capture(&request, frame);
            self.capture_result = Some(result.map_err(|e| e.to_string()));
        }
        let scaled = self.screenshot_size();
        let rendered = match scaled {
            Some(size) => self.draw_scaled_frame(frame, size, hooks),
            None => self.draw_frame(frame, None, self.output, hooks),
        };
        self.finish_prepared_frame();
        // A frame that was not drawn still submits the section copies and
        // atlas patches (glyphs, skins, maps, mods' dynamic textures), which the caller
        // already took.
        if self.device_lost() {
            return Err("the graphics device was lost".into());
        }
        let patched = !frame.hud.patches.is_empty() || !frame.hud.dynamic_commands.is_empty();
        if !matches!(rendered, Ok(true)) && patched {
            let section_copies = self.section_copies.take();
            let has_section_copies = section_copies.is_some();
            let mut encoder = section_copies.unwrap_or_else(|| {
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Sunrise atlas patches"),
                    })
            });
            self.hud
                .upload_patches(&self.device, &mut self.uploads, &mut encoder, frame.hud);
            self.uploads.finish(&encoder);
            if has_section_copies {
                self.section_copy_bytes = 0;
                self.section_uploads.finish(&encoder);
            }
            self.queue.submit([encoder.finish()]);
        } else if let Some(copies) = self.take_section_copies() {
            self.queue.submit([copies]);
        }
        // A scaled screenshot's frame never reached the window.
        rendered.map(|drawn| drawn && scaled.is_none())
    }
    /// The device's largest 2D texture side (`max_texture_dimension_2d`), the limit of a dynamic
    /// texture.
    pub fn max_texture_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }
    /// Whether the device's loss callback has fired. A surface reconfigure
    /// alone cannot restore resources belonging to a lost device.
    pub fn device_lost(&self) -> bool {
        self.device_lost.load(std::sync::atomic::Ordering::Relaxed)
    }
}
/// Encodes the copy of `texture`'s `size` pixels into a mappable buffer.
pub(crate) fn copy_frame(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    [width, height]: [u32; 2],
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Sunrise screenshot"),
        size: u64::from(screenshot::padded_row(width)) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(screenshot::padded_row(width)),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    buffer
}
/// Centre-crop the frame to a square and average it down to `size`.
pub(crate) fn square_downscale(pixels: &[u8], width: u32, height: u32, size: [u32; 2]) -> Vec<u8> {
    let (side, cx, cy) = if width > height {
        (height, (width - height) / 2, 0)
    } else {
        (width, 0, (height - width) / 2)
    };
    let [tw, th] = size;
    let mut out = vec![0u8; (tw * th * 4) as usize];
    for ty in 0..th {
        let y0 = cy + ty * side / th;
        let y1 = cy + (ty + 1) * side / th;
        for tx in 0..tw {
            let x0 = cx + tx * side / tw;
            let x1 = cx + (tx + 1) * side / tw;
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = ((y * width + x) * 4) as usize;
                    r += u32::from(pixels[p]);
                    g += u32::from(pixels[p + 1]);
                    b += u32::from(pixels[p + 2]);
                }
            }
            let count = (x1 - x0) * (y1 - y0);
            let o = ((ty * tw + tx) * 4) as usize;
            out[o..o + 4].copy_from_slice(&[
                (r / count) as u8,
                (g / count) as u8,
                (b / count) as u8,
                255,
            ]);
        }
    }
    out
}
mod view_frustum;
pub use view_frustum::Frustum;

#[cfg(feature = "test-support")]
pub use view::test_frustum;
/// Vanilla's `Projection`: reversed depth (near maps to 1, far to 0, tests
/// are GREATER_THAN_OR_EQUAL), which keeps far faces a hair apart, like
/// fire_side's 0.01 pixel in front of its block, from z-fighting.
/// The section the camera is in.
fn camera_section(camera: DVec3, min_y: i32) -> SectionKey {
    let (cx, cz) = resident::camera_column(camera);
    (cx, resident::camera_section_y(camera, min_y), cz)
}

/// The near plane of the world's projection, in blocks.
const NEAR_PLANE: f32 = 0.05;

pub(crate) fn world_projection(fov_degrees: f32, aspect: f32, far: f32) -> Mat4 {
    Mat4::perspective_rh(fov_degrees.to_radians(), aspect, far, NEAR_PLANE)
}

/// Calls `draw(first, quads)` for each run of a mesh's solid quads that can
/// face `eye` (`view::projection_center`), the mesh at the camera-relative
/// `origin`; all of them when there is no eye.
fn facing_runs(sides: &Sides, eye: Option<Vec3>, origin: [f32; 3], draw: impl FnMut(u32, u32)) {
    facing_runs_from(sides, eye, origin, Quads::All, draw)
}
/// `facing_runs` for one `part` of a mesh's solid quads.
fn facing_runs_from(
    sides: &Sides,
    eye: Option<Vec3>,
    origin: [f32; 3],
    part: Quads,
    draw: impl FnMut(u32, u32),
) {
    match eye {
        Some(eye) => {
            let eye = eye - Vec3::from(origin);
            sides.visible_from(eye.to_array(), eye.y, part, draw)
        }
        None => sides.every(part, draw),
    }
}

#[cfg(test)]
mod frame_policy_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod sprite_upload_tests;

#[cfg(test)]
mod shadow_tests;

/// A `shaders/` pack building on its own thread (`Renderer::load_shader_pack`): the dimension it
/// is for, the thread (a panic in it contained), and what the game named it with and when.
/// Dropping it (another pack, none, or another dimension) cancels the build: the thread stops at
/// the next program instead of compiling the rest for nothing.
pub(crate) struct PackLoading {
    dimension: i32,
    /// Held to be dropped with it: that cancels the build.
    _cancel: CancelOnDrop,
    #[allow(clippy::type_complexity)]
    thread: std::thread::JoinHandle<
        std::result::Result<
            (
                std::result::Result<pack_post::PackPost, pack_post::Refused>,
                Vec<i32>,
            ),
            String,
        >,
    >,
    label: String,
    started: std::time::Instant,
    /// What its terrain and shadow programs will draw from the game's sections, as the built
    /// pack's `has_terrain` and `has_shadow` will say (`PackLoading::draws_of`), for the layouts
    /// the frames drawn meanwhile keep (`Renderer::pack_pending`).
    draws: PackDraws,
}

/// A running `shaders/` pack's block ids (`Renderer::pack_block_ids`).
#[derive(Debug)]
pub(crate) struct PackBlockIds(pub(crate) Vec<i32>);

/// What a pack's programs will draw, as the built pack's `has_terrain`, `has_shadow` and
/// `draws_sections` will say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PackDraws {
    pub(crate) terrain: bool,
    pub(crate) shadow: bool,
    pub(crate) sections: bool,
}

impl PackLoading {
    /// What `post`'s programs draw once built, with terrain geometry (`atlas`, the pack's block
    /// atlas) or without: `PackPost::has_terrain` (a solid or cutout terrain program, built with
    /// that geometry, or terrain the pack leaves undrawn), `has_shadow` (the shadow pass, with a
    /// terrain or translucent shadow program built with it) and `draws_sections` (a solid,
    /// cutout or translucent terrain program, or that shadow pass). A program the device then
    /// refuses to build is not foreseen: the layouts follow the built pack once it runs.
    pub(crate) fn draws_of(post: &sunrise_shaders::Post, atlas: bool) -> PackDraws {
        let shadow = atlas
            && post.shadow_pass
            && (post.shadow.is_some()
                || post.shadow_water.is_some()
                || post.shadow_water_plain.is_some());
        let programs = post.terrain.is_some() || post.terrain_cutout.is_some();
        PackDraws {
            terrain: atlas && programs || post.terrain_undrawn,
            shadow,
            sections: atlas && (programs || post.water.is_some()) || shadow,
        }
    }
}

/// Sets its flag when dropped: `PackLoading`'s cancel, which `PackPost::new` checks per program.
struct CancelOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
