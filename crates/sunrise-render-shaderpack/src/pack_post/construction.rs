//! `PackPost::new` and the pieces it builds a pack's passes with: the formats and clears its
//! buffers get, the bind group layouts its programs declare, the pipelines of its passes and
//! compute dispatches (`compile` makes them), its samplers and its blits. It ends where the
//! frame's own work starts (`execution.rs`).

use std::collections::{HashMap, HashSet};

use sunrise_shaders::{BUFFERS, Format, Group, Kind, Post, SHADOW_COLORS};

use super::mod_pack::{
    ModDepthTargetDecl, ModDrawChunk, ModPackPass, ModPackRuntime, ModPassSlot, ModProgramMode,
    ModUniformDecl, ModUniformMatch, ModUniformTable, ModVertexStage, match_mod_uniform,
    mod_depth_source,
};
use super::packed;
use super::{image_binding, image_format, sampled_image, storage_formats_of};
use crate::pack_draw::DrawUniforms;
use crate::pack_entities::PackEntities;
use crate::pack_images;
use crate::pack_sky::PackSky;
use crate::pack_textures;
use crate::pack_uniforms::{self, FrameUniforms};

pub use crate::pack_post::types::*;

/// The wgpu format of a pack format (6.7), the nearest one wgpu can render to.
pub fn texture_format(f: Format, features: wgpu::Features) -> wgpu::TextureFormat {
    use wgpu::TextureFormat as T;
    let pick = |r, rg, rgba| match f.channels {
        1 => r,
        2 => rg,
        _ => rgba,
    };
    let half = pick(T::R16Float, T::Rg16Float, T::Rgba16Float);
    match (f.kind, f.bits) {
        (Kind::Unorm, 10) => T::Rgb10a2Unorm,
        (Kind::Unorm, 16)
            if features.contains(
                wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                    | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            ) =>
        {
            pick(T::R16Unorm, T::Rg16Unorm, T::Rgba16Unorm)
        }
        (Kind::Unorm, b) if b <= 8 => pick(T::R8Unorm, T::Rg8Unorm, T::Rgba8Unorm),
        (Kind::Float, 32) => pick(T::R32Float, T::Rg32Float, T::Rgba32Float),
        (Kind::Float, 11) if features.contains(wgpu::Features::RG11B10UFLOAT_RENDERABLE) => {
            T::Rg11b10Ufloat
        }
        (Kind::Int, 8) => pick(T::R8Sint, T::Rg8Sint, T::Rgba8Sint),
        (Kind::Int, 16) => pick(T::R16Sint, T::Rg16Sint, T::Rgba16Sint),
        (Kind::Int, _) => pick(T::R32Sint, T::Rg32Sint, T::Rgba32Sint),
        (Kind::Uint, 8) => pick(T::R8Uint, T::Rg8Uint, T::Rgba8Uint),
        (Kind::Uint, 10) => T::Rgb10a2Uint,
        (Kind::Uint, 16) => pick(T::R16Uint, T::Rg16Uint, T::Rgba16Uint),
        (Kind::Uint, _) => pick(T::R32Uint, T::Rg32Uint, T::Rgba32Uint),
        // Snorm, 16-bit norm without the feature, RGB9_E5: half floats hold them.
        _ => half,
    }
}

/// `format` where `renders` says it can be a colour target, else the nearest float format that can:
/// the same channels at 16 bits, then `Rgba8Unorm`. On OpenGL the 32-bit and 16-bit float formats
/// render only with `GL_EXT_color_buffer_float`/`half_float`, and wgpu aborts on creating a target
/// without. Other formats pass through. Float colour buffers are only sampled (never blended), so
/// RENDER_ATTACHMENT is the only usage that decides.
pub fn renderable_float(
    format: wgpu::TextureFormat,
    renders: impl Fn(wgpu::TextureFormat) -> bool,
) -> wgpu::TextureFormat {
    use wgpu::TextureFormat as T;
    let half = match format {
        T::R32Float | T::R16Float => T::R16Float,
        T::Rg32Float | T::Rg16Float => T::Rg16Float,
        T::Rgba32Float | T::Rgba16Float => T::Rgba16Float,
        _ => return format,
    };
    if renders(format) {
        format
    } else if renders(half) {
        half
    } else {
        T::Rgba8Unorm
    }
}

/// [`texture_format`] made renderable on this adapter (see [`renderable_float`]).
pub fn target_format(
    f: Format,
    features: wgpu::Features,
    adapter: &wgpu::Adapter,
) -> wgpu::TextureFormat {
    renderable_float(texture_format(f, features), |format| {
        (adapter.get_texture_format_features(format).allowed_usages)
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
    })
}

/// The SNORM wgpu format of SNORM pack format `f`, where this adapter can render to it. Iris
/// stores such a buffer clamped to [-1, 1] and quantised to its bits; elsewhere the half floats of
/// [`texture_format`] stand in.
pub fn snorm_format(
    f: Format,
    features: wgpu::Features,
    adapter: &wgpu::Adapter,
) -> Option<wgpu::TextureFormat> {
    use wgpu::TextureFormat as T;
    let format = match (f.kind, f.bits, f.channels) {
        (Kind::Snorm, 8, 1) => T::R8Snorm,
        (Kind::Snorm, 8, 2) => T::Rg8Snorm,
        (Kind::Snorm, 8, _) => T::Rgba8Snorm,
        (Kind::Snorm, 16, 1) => T::R16Snorm,
        (Kind::Snorm, 16, 2) => T::Rg16Snorm,
        (Kind::Snorm, 16, _) => T::Rgba16Snorm,
        _ => return None,
    };
    let mut needs = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    if f.bits == 16 {
        needs |= wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
    }
    let renders = (adapter.get_texture_format_features(format).allowed_usages)
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT);
    (features.contains(needs) && renders).then_some(format)
}

/// Whether a `ty` sampler can read a custom texture of `sample_type`: custom textures hold
/// colours, never depth to compare against.
pub fn fits(ty: &str, sample_type: wgpu::TextureSampleType) -> bool {
    use wgpu::TextureSampleType as T;
    !ty.contains("Shadow")
        && match sample_type {
            T::Sint => ty.starts_with('i'),
            T::Uint => ty.starts_with('u'),
            _ => ty.starts_with('s'),
        }
}

/// `hz_depth_linear` for a program: 1 for each of shadowtex0, shadowtex1, `shadow` and
/// `watershadow` whose depth filters (`linear`, bit n: shadowtex n). `shadow` is shadowtex1 in a
/// program that also declares `watershadow` (6.3).
pub fn depth_linear(interface: &sunrise_shaders::Interface, linear: u8) -> [f32; 4] {
    let watershadow = interface.samplers.iter().any(|s| s.name == "watershadow");
    [0, 1, usize::from(watershadow), 0].map(|n| f32::from(linear >> n & 1))
}

/// The texture behind sampler `name` in a program of the shadow buffers (`shadow_space`: buffer
/// n is shadowcolor n) or of the others, when the shadow pass runs (`shadow_pass`). `shadow` is
/// `shadowtex1` when the program also declares `watershadow` (6.3).
pub fn source(name: &str, shadow_space: bool, shadow_pass: bool, watershadow: bool) -> Source {
    let shadow_color = sunrise_shaders::shadow_color(name);
    let shadow_depth = match name {
        "shadowtex0" | "watershadow" | "shadowtex0HW" => Some(0),
        "shadowtex1" | "shadowtex1HW" => Some(1),
        "shadow" => Some(usize::from(watershadow)),
        _ => None,
    };
    if shadow_pass {
        if let Some(n) = shadow_depth {
            return Source::ShadowDepth(n, name.ends_with("HW"));
        }
        if let Some(n) = shadow_color {
            return if shadow_space {
                Source::Color(n)
            } else {
                Source::ShadowColor(n)
            };
        }
    }
    if shadow_space && sunrise_shaders::buffer_index(name).is_some() {
        // colortex buffers are not this pass's.
        return Source::Dummy;
    }
    match name {
        "depthtex0" | "gdepthtex" => Source::Depth,
        "depthtex1" => Source::Depth1,
        "depthtex2" => Source::Depth2,
        "noisetex" => Source::Noise,
        "texture" | "gtexture" | "tex" => Source::Atlas,
        "lightmap" => Source::Lightmap,
        "normals" => Source::Normals,
        "specular" => Source::Specular,
        sunrise_shaders::WATER_MASK => Source::WaterMask,
        n => sunrise_shaders::buffer_index(n).map_or(Source::Dummy, Source::Color),
    }
}

/// `size`×`size` RGB white noise (6.4), from a fixed seed so every run looks the same.
pub fn noise(device: &wgpu::Device, queue: &wgpu::Queue, size: u32) -> wgpu::TextureView {
    let size = size.min(device.limits().max_texture_dimension_2d);
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let texels: Vec<u8> = (0..size as usize * size as usize * 4)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            if i % 4 == 3 { 255 } else { (state >> 56) as u8 }
        })
        .collect();
    texture(device, queue, "Shader pack noisetex", size, &texels)
}

pub fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    side: u32,
    texels: &[u8],
) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: side,
        height: side,
        depth_or_array_layers: 1,
    };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        tex.as_image_copy(),
        texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(side * 4),
            rows_per_image: None,
        },
        size,
    );
    tex.create_view(&Default::default())
}

pub fn sampler(
    device: &wgpu::Device,
    filter: wgpu::FilterMode,
    address: wgpu::AddressMode,
) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Shader pack"),
        address_mode_u: address,
        address_mode_v: address,
        address_mode_w: address,
        mag_filter: filter,
        min_filter: filter,
        ..Default::default()
    })
}

/// Runs `create` and returns its validation error, if any, or the backend's (a shader the
/// driver's compiler refuses is an internal error). A panic while it runs is an error too:
/// naga's backends turn a module they cannot write into a panic as the driver gets the pipeline
/// (#3525), which a pack's program must not turn into a crash.
pub fn checked<T>(device: &wgpu::Device, create: impl FnOnce() -> T) -> Result<T, String> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(create))
        .map_err(|payload| panic_text(&*payload));
    let internal = pollster::block_on(internal.pop());
    match pollster::block_on(validation.pop()).or(internal) {
        Some(e) => {
            // The details are in the error's description or its sources.
            let mut message = match &e {
                wgpu::Error::Validation { source, .. } => format!("{source:?}"),
                e => e.to_string(),
            };
            let mut source = std::error::Error::source(&e);
            while let Some(s) = source {
                message = format!("{message}: {s}");
                source = s.source();
            }
            Err(message)
        }
        None => value.map_err(|panic| format!("the shader compiler failed: {panic}")),
    }
}

/// The panic's message, for `checked`'s error.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".to_owned())
}

/// A full-screen helper pipeline of ours (`pack_post.wgsl`).
pub fn helper(
    device: &wgpu::Device,
    vertex: &str,
    entry: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
    targets: &[Option<wgpu::ColorTargetState>],
    depth: Option<wgpu::DepthStencilState>,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Shader pack helpers"),
        source: wgpu::ShaderSource::Wgsl(include_str!("pack_post.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(entry),
        entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(entry),
        bind_group_layouts: &[Some(&layout)],
        ..Default::default()
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some(vertex),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: depth,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets,
        }),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, layout)
}

pub fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}

pub fn texture_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    texture_entry_of(binding, sample_type, wgpu::TextureViewDimension::D2)
}

pub fn texture_entry_of(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
    view_dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type,
            view_dimension,
            multisampled: false,
        },
    )
}

/// `blend.<program>`'s factors as wgpu's, adding both terms.
pub fn blend_state(b: sunrise_shaders::Blend) -> wgpu::BlendState {
    use sunrise_shaders::Factor as F;
    use wgpu::BlendFactor as W;
    let f = |f| match f {
        F::Zero => W::Zero,
        F::One => W::One,
        F::SrcColor => W::Src,
        F::OneMinusSrcColor => W::OneMinusSrc,
        F::DstColor => W::Dst,
        F::OneMinusDstColor => W::OneMinusDst,
        F::SrcAlpha => W::SrcAlpha,
        F::OneMinusSrcAlpha => W::OneMinusSrcAlpha,
        F::DstAlpha => W::DstAlpha,
        F::OneMinusDstAlpha => W::OneMinusDstAlpha,
        F::SrcAlphaSaturate => W::SrcAlphaSaturated,
    };
    let component = |src, dst| wgpu::BlendComponent {
        src_factor: f(src),
        dst_factor: f(dst),
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: component(b[0], b[1]),
        alpha: component(b[2], b[3]),
    }
}

/// A buffer whose texels are floats (normalized or not), not integers.
pub fn holds_floats(format: wgpu::TextureFormat) -> bool {
    matches!(
        format.sample_type(None, None),
        Some(wgpu::TextureSampleType::Float { .. })
    )
}

pub fn target(format: wgpu::TextureFormat) -> Option<wgpu::ColorTargetState> {
    Some(wgpu::ColorTargetState {
        format,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    })
}

/// Color-attachment cost includes component alignment, not only the copied texel size.
pub fn attachment_bytes(bytes: u32, format: wgpu::TextureFormat) -> u32 {
    let alignment = format
        .target_component_alignment()
        .expect("color attachment");
    bytes.next_multiple_of(alignment) + format.target_pixel_byte_cost().expect("color attachment")
}

/// The viewport of `scale.<program>` (scale, offset x, offset y) on buffers of `size`, in whole
/// pixels, at least one and inside them. Buffer rows are GL's, so the offsets count from GL's
/// bottom-left corner as they are.
pub fn viewport(size: [u32; 2], [scale, x, y]: [f32; 3]) -> [f32; 4] {
    let [w, h] = size.map(|n| n as f32);
    let (x, y) = ((x * w).floor().min(w - 1.0), (y * h).floor().min(h - 1.0));
    [
        x,
        y,
        (scale * w).floor().clamp(1.0, w - x),
        (scale * h).floor().clamp(1.0, h - y),
    ]
}

pub fn attach(
    view: &wgpu::TextureView,
    ops: wgpu::Operations<wgpu::Color>,
) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        depth_slice: None,
        ops,
    })
}

/// Which of the frame's terrain draws: the camera's solid or translucent ones, or the shadow
/// camera's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainList {
    Solid,
    Translucent,
    ShadowSolid,
    ShadowTranslucent,
}

impl TerrainList {
    pub fn translucent(self) -> bool {
        matches!(self, Self::Translucent | Self::ShadowTranslucent)
    }
}

/// Draws the game's terrain of one list into a pass; calls its third argument with each arena
/// page before drawing from it.
/// Draws a list's terrain: indexed, or for a program with a geometry stage, without indices and
/// with that many instances (and its index buffer bound in set 5 by this layout).
pub type TerrainDraw<'a> = dyn FnMut(
        &mut wgpu::RenderPass<'_>,
        TerrainList,
        &dyn Fn(&mut wgpu::RenderPass<'_>, u32),
        Option<(u32, &wgpu::BindGroupLayout)>,
    ) + 'a;

/// An arena page's key, native vertices and optional source records (one per slot: a quad's at
/// the slot that names it, `pack_atlas::PULL_CORNERS`'s `hz_slot`).
pub type TerrainPage = (u32, wgpu::Buffer, Option<wgpu::Buffer>);

/// Header (sprite-rect words, state count), sprite rectangles, then model/fluid ids.
/// Keeping immutable tables together leaves the existing three storage bindings intact.
pub fn terrain_tables(table: &[i32], rects: &[u32]) -> Vec<i32> {
    let rects = if rects.is_empty() { &[0, 0][..] } else { rects };
    [rects.len() as i32, (table.len() / 2) as i32]
        .into_iter()
        .chain(rects.iter().map(|&v| v as i32))
        .chain(table.iter().copied())
        .collect()
}

/// `create_buffer` with an out-of-memory failure returned instead of raised (raised errors are
/// fatal). The client's arena had this; a pack's tables are small, so it is made here.
pub fn try_create(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> Option<wgpu::Buffer> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    });
    // Native error scopes resolve at once.
    pollster_poll(scope.pop()).then_some(buffer)
}

/// Blocks until `scope`'s result is in, then whether it held no error.
fn pollster_poll<F>(errors: F) -> bool
where
    F: std::future::Future<Output = Option<wgpu::Error>>,
{
    pollster::block_on(errors).is_none()
}

pub fn block_ids(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    table: &[i32],
    rects: &[u32],
) -> Option<(wgpu::BindGroupLayout, wgpu::Buffer, wgpu::Buffer)> {
    let buffer = |label, contents: &[u8]| {
        if contents.len() as u64 > device.limits().max_storage_buffer_binding_size {
            return None;
        }
        let buffer = try_create(
            device,
            label,
            contents.len() as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        )?;
        queue.write_buffer(&buffer, 0, contents);
        Some(buffer)
    };
    let ids = buffer("Unknown terrain sources", &[0; 8])?;
    let rects = buffer(
        "Shader pack terrain tables",
        bytemuck::cast_slice(&terrain_tables(table, rects)),
    )?;
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Shader pack terrain"),
        entries: &[storage(0), storage(1), storage(2)],
    });
    Some((layout, ids, rects))
}

/// The game's geometry a gbuffers program draws: its vertex buffers and, for flat draws
/// (flat.rs), the layout of their bind group 1.
#[derive(Clone)]
pub struct Geometry<'a> {
    pub buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    pub table: Option<&'a wgpu::BindGroupLayout>,
    /// Writes depth (the sky does not).
    pub depth_write: bool,
    /// What `texture` reads instead of the block atlas.
    pub atlas: Option<wgpu::TextureView>,
    /// This draw uses the block atlas's surface identities.
    pub material_maps: bool,
    /// Set 3's layout, for terrain that reads its arena page (`pages`): the quad sources and
    /// `mc_Entity`'s table flat terrain reads, and the page itself. Filled in by `PackPost::new`.
    pub ids: Option<&'a wgpu::BindGroupLayout>,
    /// Back faces are culled, as for the game's own terrain.
    pub cull: bool,
    /// Terrain whose program reads its arena page at set 3
    /// (`pack_atlas::TerrainDraws::reads_pages`): its passes get set 3's layout (`ids`), and each
    /// page's group before its draws.
    pub pages: bool,
}

/// What a pack brings besides its programs: custom textures, images and storage buffers.
pub struct Resources<'a> {
    pub textures: &'a [pack_textures::PackTexture],
    pub images: &'a [pack_images::PackImage],
    /// The images this device can't make.
    pub left_out: &'a [sunrise_shaders::CustomImage],
    pub buffers: &'a pack_images::Buffers,
    /// Set 5 of a program with a geometry stage: the draw's indices.
    pub indices: Option<&'a wgpu::BindGroupLayout>,
    pub gpu: Gpu,
    /// The buffers whose format can be bound as an image: colortex N at bit N, shadowcolor N at
    /// bit `BUFFERS + N`.
    pub storable: u32,
    /// The buffers of a three-channel format (`rgb`): they have no alpha to write, and read 1.
    pub rgb: u16,
    /// The bits per channel a buffer of a small packed format keeps ([`packed::bits`]).
    pub packed: [Option<[u8; 4]>; BUFFERS],
    /// The stage (`texture.<stage>`) of the programs built so far, and the buffers one of them
    /// flipped: a `texture.<stage>.colortexN` no longer stands in for buffer N in the stage's
    /// later programs, which read what was written, as in Iris. `final` continues
    /// `composite`'s. (Super Duper Vanilla's `texture.composite.colortex0` is its clouds, read
    /// before `composite4` writes bloom there; Bliss's `colortex6` is blue noise until
    /// `composite5` writes it.)
    pub released: std::cell::Cell<(&'static str, u16)>,
    /// The formats and dimensions of the images no `image.*` gives, for their stand-ins.
    pub stand_ins: &'a std::cell::RefCell<Vec<(wgpu::TextureFormat, wgpu::TextureViewDimension)>>,
    /// Bit n: shadowtex n read through a plain sampler filters linearly (`shadowtexNNearest`
    /// unset): the translator's helpers blend a gather of it (`hz_depth_linear`).
    pub shadow_linear: u8,
    /// While the pack is laid out for `compile`: the pipelines its programs need. `None`
    /// compiles each as its program is built.
    pub jobs: Option<std::cell::RefCell<Vec<Job>>>,
    /// Mod-declared auxiliary depth targets for sampler resolution.
    pub mod_depth_targets: &'a [ModDepthTargetDecl],
    /// Mod-declared per-frame uniforms for uniform resolution and type validation.
    pub mod_uniforms: &'a [ModUniformDecl],
    /// The pack is served Sunrise's HDR extension (`Post::hdr`): its programs read the display
    /// uniforms (`pack_uniforms::DISPLAY`).
    pub display_uniforms: bool,
    /// Deduplicates warnings when a pack declares a mod uniform with an incompatible GLSL type.
    pub mod_uniform_mismatches: &'a std::cell::RefCell<HashSet<(String, String)>>,
    /// Auxiliary depth target index written by the mod geometry pass currently being built, if any.
    pub mod_depth_target: std::cell::Cell<Option<usize>>,
    /// Depth comparison function for the pass currently being built.
    pub depth_compare: std::cell::Cell<wgpu::CompareFunction>,
    /// The mod's own vertex stage for the pass about to be built, which `build_pass` takes and
    /// links to that pass's fragment stage ([`ModProgramMode::ModVertexPackFragment`]).
    pub mod_vertex: std::cell::RefCell<Option<ModVertexStage>>,
}

impl Resources<'_> {
    /// The slot of the render pipeline `job` makes: filled now, or by `compile` when the pack
    /// is laid out.
    pub fn render_pipeline(
        &self,
        device: &wgpu::Device,
        job: RenderJob,
    ) -> Slot<wgpu::RenderPipeline> {
        let slot = Slot::default();
        let job = Job::Render(slot.clone(), job);
        match &self.jobs {
            Some(jobs) => jobs.borrow_mut().push(job),
            None => job.create(device),
        }
        slot
    }
    /// As `render_pipeline`, for a compute pipeline.
    pub fn compute_pipeline(
        &self,
        device: &wgpu::Device,
        job: ComputeJob,
    ) -> Slot<wgpu::ComputePipeline> {
        let slot = Slot::default();
        let job = Job::Compute(slot.clone(), job);
        match &self.jobs {
            Some(jobs) => jobs.borrow_mut().push(job),
            None => job.create(device),
        }
        slot
    }
}

/// A pipeline, made once: `Pipeline`'s, filled by its `Job`.
pub type Slot<T> = std::sync::Arc<std::sync::OnceLock<T>>;

/// What a render pipeline of a pack's program is made of, kept to make it later.
#[derive(Clone)]
pub struct RenderJob {
    pub label: String,
    pub layout: wgpu::PipelineLayout,
    pub vs: wgpu::ShaderModule,
    pub fs: wgpu::ShaderModule,
    /// The vertex buffers: stride, step mode and attributes.
    pub buffers: Vec<Option<(u64, wgpu::VertexStepMode, Vec<wgpu::VertexAttribute>)>>,
    pub primitive: wgpu::PrimitiveState,
    pub depth_stencil: Option<wgpu::DepthStencilState>,
    pub targets: Vec<Option<wgpu::ColorTargetState>>,
}

/// What a compute pipeline of a pack's program is made of.
pub struct ComputeJob {
    pub label: String,
    pub layout: wgpu::PipelineLayout,
    pub module: wgpu::ShaderModule,
}

/// A pipeline to make, and the slot it goes in.
pub enum Job {
    Render(Slot<wgpu::RenderPipeline>, RenderJob),
    Compute(Slot<wgpu::ComputePipeline>, ComputeJob),
}

impl Job {
    /// Makes the pipeline (the driver compiles its shaders here) into its slot.
    pub fn create(&self, device: &wgpu::Device) {
        match self {
            Job::Render(slot, j) => {
                let buffers: Vec<Option<wgpu::VertexBufferLayout>> = j
                    .buffers
                    .iter()
                    .map(|b| {
                        b.as_ref()
                            .map(|(stride, step, attributes)| wgpu::VertexBufferLayout {
                                array_stride: *stride,
                                step_mode: *step,
                                attributes,
                            })
                    })
                    .collect();
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&j.label),
                    layout: Some(&j.layout),
                    vertex: wgpu::VertexState {
                        module: &j.vs,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        buffers: &buffers,
                    },
                    primitive: j.primitive,
                    depth_stencil: j.depth_stencil.clone(),
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &j.fs,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        targets: &j.targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                let _ = slot.set(pipeline);
            }
            Job::Compute(slot, j) => {
                let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(&j.label),
                    layout: Some(&j.layout),
                    module: &j.module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });
                let _ = slot.set(pipeline);
            }
        }
    }
    pub fn label(&self) -> &str {
        match self {
            Job::Render(_, j) => &j.label,
            Job::Compute(_, j) => &j.label,
        }
    }
}

/// Makes the pipelines of `jobs` on as many threads as the machine has cores (up to 8): the
/// driver compiles a big pack's programs for tens of seconds one at a time, on one core. The
/// first one wgpu or the driver refuses, with why; `None` when all are made.
///
/// A refusal doesn't stop the others: every job is made, so that the serial rebuild `new` then
/// runs finds the driver's shader cache warm instead of compiling the rest cold, one at a time.
/// `cancel` set stops handing out jobs: the caller then drops the pack (`PackPost::new`).
pub fn compile(
    device: &wgpu::Device,
    jobs: &[Job],
    cancel: &std::sync::atomic::AtomicBool,
) -> Option<(String, String)> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 8)
        .min(jobs.len().max(1));
    let next = AtomicUsize::new(0);
    let refused = std::sync::Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while !cancel.load(Ordering::Relaxed)
                    && let Some(job) = jobs.get(next.fetch_add(1, Ordering::Relaxed))
                {
                    if let Err(e) = checked(device, || job.create(device)) {
                        let mut refused = refused.lock().unwrap_or_else(|e| e.into_inner());
                        refused.get_or_insert((job.label().to_owned(), e));
                    }
                }
            });
        }
    });
    refused.into_inner().unwrap_or_else(|e| e.into_inner())
}

/// Vertex buffer layouts owned, for a `RenderJob`.
pub fn owned_buffers(
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
) -> Vec<Option<(u64, wgpu::VertexStepMode, Vec<wgpu::VertexAttribute>)>> {
    buffers
        .iter()
        .map(|b| {
            b.as_ref()
                .map(|b| (b.array_stride, b.step_mode, b.attributes.to_vec()))
        })
        .collect()
}

/// A program's bind group layout (its uniform block, samplers and images) and how each sampler
/// and image binds. `None` (with a note) for an image the game can't give it.
pub fn bindings(
    features: wgpu::Features,
    interface: &sunrise_shaders::Interface,
    name: &str,
    group: Group,
    buffers: &[u32],
    geometry: Option<&Geometry<'_>>,
    formats: &[wgpu::TextureFormat; BUFFERS],
    shadow: Option<&[wgpu::TextureFormat; SHADOW_COLORS]>,
    notes: &mut Vec<String>,
    pack: &Resources<'_>,
) -> Option<(
    Vec<wgpu::BindGroupLayoutEntry>,
    Vec<Binding>,
    HashMap<u32, u32>,
    u16,
    u16,
    Vec<(u32, Image)>,
)> {
    let textures = pack.textures;
    if let Some(b) = interface.buffers.iter().find(|&&b| !pack.buffers.has(b)) {
        notes.push(format!("{name}: buffer {b} has no `bufferObject.{b}`"));
        return None;
    }
    let mut entries = vec![entry(
        0,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )];
    let mut samplers = Vec::new();
    let (mut kinds, mut slots) = (HashMap::new(), HashMap::new());
    let mut reads = 0u16;
    let mut shadow_reads = 0u16;
    let watershadow = interface.samplers.iter().any(|s| s.name == "watershadow");
    // Opaque gbuffers draws cannot sample their own attachments. Translucent draws can
    // read colortex4 and above from the preceding deferred stage; those reads use a copy
    // taken before the draw, so the sampled texture is never also an attachment.
    let in_place = match geometry {
        Some(_) => buffers.iter().fold(0u16, |m, &b| m | 1 << b),
        None => 0,
    };
    for s in &interface.samplers {
        let float = wgpu::TextureSampleType::Float { filterable: true };
        // A stand-in of the sampler's own dimension.
        let stand_in = if s.ty.contains("1D") {
            wgpu::TextureViewDimension::D1
        } else if s.ty.contains("3D") {
            wgpu::TextureViewDimension::D3
        } else {
            wgpu::TextureViewDimension::D2
        };
        if let Some(def) = pack
            .left_out
            .iter()
            .find(|d| d.sampler.as_deref() == Some(s.name.as_str()))
        {
            notes.push(format!(
                "{name}: samples image `{}`, which is left out",
                def.name
            ));
            return None;
        }
        let released = pack.released.get();
        let custom = pack_textures::find(textures, group, &s.ty, &s.name).filter(|&i| {
            let t = &textures[i];
            // A PNG under `texture.<stage>.<name>` whose name is no sampler of the game's
            // isn't bound: the sampler reads as if it had no texture. Iris 1.11.7 with a probe
            // pack of ours (#9064) bound it over `gaux2` and `noisetex` but not over names of its
            // own, where a raw texture was bound; this rule is a fit to those cases.
            let unbound = t.png
                && t.stage != sunrise_shaders::EVERY_STAGE
                && source(&s.name, false, true, false) == Source::Dummy;
            !unbound
                && (t.stage != pack_textures::stage(group)
                    || released.0 != t.stage
                    || sunrise_shaders::buffer_index(&t.name)
                        .is_none_or(|b| released.1 & 1 << b == 0))
        });
        let image = pack
            .images
            .iter()
            .position(|i| i.def.sampler.as_deref() == Some(s.name.as_str()));
        let (src, sample_type, dimension) = match custom {
            _ if let Some(k) = image => {
                let i = &pack.images[k];
                let sample_type = i.sample_type(features);
                // A sampler of another class than its image's texture (`usampler2D` over an
                // `R32F` image) refuses the program (decision D-6, #5223).
                if let Err(note) =
                    sampled_image(name, &i.def.name, &s.ty, &s.name, i.format, sample_type)
                {
                    notes.push(note);
                    return None;
                }
                (Source::Image(k), sample_type, i.dimension)
            }
            Some(i) if !fits(&s.ty, textures[i].sample_type) => {
                notes.push(format!(
                    "{name}: `{} {}` can't read texture.{}.{}",
                    s.ty, s.name, textures[i].stage, textures[i].name
                ));
                (Source::Dummy, float, stand_in)
            }
            Some(i) => {
                let t = &textures[i];
                let src = match t.image {
                    pack_textures::Image::Lightmap => Source::Lightmap,
                    pack_textures::Image::Atlas => Source::Atlas,
                    pack_textures::Image::View(_) => Source::Custom {
                        index: i,
                        blur: t.blur,
                        clamp: t.clamp,
                    },
                };
                (src, t.sample_type, t.dimension)
            }
            None => {
                let mut src =
                    mod_depth_source(pack.mod_depth_targets, &s.name).unwrap_or_else(|| {
                        source(&s.name, group.shadow(), shadow.is_some(), watershadow)
                    });
                // A sampler the game gives no texture reads texture unit 0, as do the atlas's
                // names: the atlas in gbuffers programs, `colortex0` in deferred, composite and
                // final (OptiFine's shaders.txt). Iris 1.11.7 read the atlas through a name of
                // ours in gbuffers_terrain, and the scene in `colortex0` through such a name and
                // through `gtexture`, `tex` and `texture` in those three, with or without
                // `colortex0` declared (probe packs of ours, #9307). Translucent programs and
                // the other groups weren't measured and keep the stand-in.
                let unit_0 = match group {
                    Group::Gbuffers => Some(Source::Atlas),
                    Group::Deferred | Group::Composite | Group::Final => Some(Source::Color(0)),
                    _ => None,
                };
                // A name of the game's that has no texture here (a shadow sampler with no
                // shadow pass) stays blank; so does a sampler that isn't 2D, and one under a
                // `texture.<stage>.<name>` left unbound (#9064's probe read a 16-texel-wide
                // texture there in final, undiagnosed).
                let unknown = src == Source::Dummy
                    && source(&s.name, false, true, false) == Source::Dummy
                    && s.ty == "sampler2D"
                    && !textures.iter().any(|t| t.name == s.name);
                // colortex0 to colortex3 (gcolor, gdepth, gnormal, composite) are no gbuffers
                // samplers in shaders.txt's table: Iris 1.11.7 read the atlas through each of
                // colortex0-3 and gnormal in gbuffers_terrain (our probe, #9307).
                let not_gbuffers =
                    group == Group::Gbuffers && matches!(src, Source::Color(n) if n < 4);
                if let Some(unit_0) = unit_0
                    && (src == Source::Atlas || unknown || not_gbuffers)
                {
                    src = unit_0;
                }
                let active_mod_depth = pack.mod_depth_target.get();
                let blank = match src {
                    // The shadow pass writes the shadow map.
                    Source::ShadowDepth(..) => group == Group::Shadow,
                    Source::Atlas => geometry.is_none(),
                    // A mod pass writing to its own auxiliary depth target can legally sample
                    // the pack's main `depthtex0`; passes writing to `depthtex0` cannot.
                    Source::Depth => geometry.is_some() && active_mod_depth.is_none(),
                    // Translucent terrain reads the copy; opaque terrain would read itself.
                    Source::Depth1 | Source::Depth2 => {
                        group == Group::Gbuffers && active_mod_depth.is_none()
                    }
                    Source::ModDepth(idx) => active_mod_depth == Some(idx),
                    Source::ModOpaqueDepth(idx) => {
                        group == Group::Gbuffers && active_mod_depth == Some(idx)
                    }
                    Source::Color(n) => {
                        (group != Group::Translucent || n < 4) && in_place & 1 << n != 0
                    }
                    _ => false,
                };
                let mut sample_type = match src {
                    Source::Color(n) => formats[n]
                        .sample_type(None, Some(features))
                        .unwrap_or(float),
                    Source::ShadowColor(n) => shadow
                        .and_then(|f| f[n].sample_type(None, Some(features)))
                        .unwrap_or(float),
                    Source::Depth
                    | Source::Depth1
                    | Source::Depth2
                    | Source::ShadowDepth(..)
                    | Source::WaterMask
                    | Source::ModDepth(_)
                    | Source::ModOpaqueDepth(_) => {
                        wgpu::TextureSampleType::Float { filterable: false }
                    }
                    // Unfilterable on WebGPU-level devices; the lightmap is sampled at texel centres.
                    Source::Lightmap => float,
                    _ => float,
                };
                let int = !matches!(sample_type, wgpu::TextureSampleType::Float { .. });
                if s.ty == "sampler2DShadow" {
                    src = match src {
                        Source::ShadowDepth(n, _) => Source::ShadowDepth(n, true),
                        _ => Source::Shadow,
                    };
                    sample_type = wgpu::TextureSampleType::Depth;
                }
                let wanted = match s.ty.as_str() {
                    "sampler2DShadow" => true,
                    "sampler2D" => !int,
                    "isampler2D" => sample_type == wgpu::TextureSampleType::Sint,
                    "usampler2D" => sample_type == wgpu::TextureSampleType::Uint,
                    _ => false,
                };
                if !wanted {
                    notes.push(format!(
                        "{}: `{} {}` is not provided yet",
                        name, s.ty, s.name
                    ));
                    (src, sample_type) = (Source::Dummy, float);
                }
                if blank && sample_type == wgpu::TextureSampleType::Depth {
                    src = Source::Shadow;
                } else if blank {
                    // The blank texture is float: an integer sampler then fails the pipeline (a note).
                    src = Source::Dummy;
                    if int {
                        sample_type = float;
                    }
                }
                let dimension = match src {
                    Source::Dummy => stand_in,
                    _ => wgpu::TextureViewDimension::D2,
                };
                (src, sample_type, dimension)
            }
        };
        let filtering = sample_type == float;
        match src {
            Source::Color(n) => reads |= 1 << n,
            Source::ShadowColor(n) => shadow_reads |= 1 << n,
            _ => {}
        }
        entries.push(texture_entry_of(s.binding, sample_type, dimension));
        let kind = SamplerKind::of(src, filtering);
        let sampler = *kinds.entry(kind).or_insert_with(|| {
            entries.push(entry(
                s.binding + 1,
                wgpu::BindingType::Sampler(kind.binding_type()),
            ));
            s.binding + 1
        });
        slots.insert(s.binding + 1, sampler);
        samplers.push(Binding {
            source: src,
            binding: s.binding,
            filtering,
            sampler,
            dimension,
        });
    }
    let mut images = Vec::new();
    for i in &interface.images {
        let buffer = i
            .name
            .strip_prefix("colorimg")
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|&n| n < BUFFERS && !group.shadow());
        let shadow_buffer = shadow_image(&i.name, shadow);
        let own = pack.images.iter().position(|c| c.def.name == i.name);
        let Some(format) = image_format(&i.format) else {
            notes.push(format!("{name}: `{}` has format {}", i.name, i.format));
            return None;
        };
        let dimension = if i.ty.ends_with("1D") {
            wgpu::TextureViewDimension::D1
        } else if i.ty.ends_with("3D") {
            wgpu::TextureViewDimension::D3
        } else {
            wgpu::TextureViewDimension::D2
        };
        // An image no `image.*` gives reads zero, also after the program stored into it
        // (Iris 1.11.7, observed with a probe pack).
        let stand_in = |of: &str, notes: &mut Vec<String>| {
            notes.push(format!(
                "{name}: image `{of}` has no `image.{of}`; it reads zero"
            ));
            let mut stand_ins = pack.stand_ins.borrow_mut();
            if !stand_ins.contains(&(format, dimension)) {
                stand_ins.push((format, dimension));
            }
            (Image::StandIn(format, dimension), format)
        };
        let (target, actual) = match (buffer, own) {
            (_, Some(k)) => (Image::Custom(k), pack.images[k].format),
            (Some(b), None) => (Image::Color(b), formats[b]),
            (None, None) if let Some((n, f)) = shadow_buffer => (Image::ShadowColor(n), f),
            (None, None) if i.before.is_some() => {
                // Bound as the image it copies.
                let of = i.before.as_deref().unwrap_or_default();
                let buffer = of
                    .strip_prefix("colorimg")
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|&n| n < BUFFERS && !group.shadow());
                match (buffer, pack.images.iter().position(|c| c.def.name == of)) {
                    (_, Some(k)) => (Image::CustomBefore(k), pack.images[k].format),
                    (Some(b), None) => (Image::ColorBefore(b), formats[b]),
                    (None, None) => match shadow_image(of, shadow) {
                        Some((n, f)) => (Image::ShadowColorBefore(n), f),
                        None => stand_in(of, notes),
                    },
                }
            }
            (None, None) => stand_in(&i.name, notes),
        };
        // The colortex and shadowcolor buffers are plain 2D textures: an `image3D` (or 1D,
        // array, cube...) declaration would bind a 2D view to a layout of another dimension,
        // which wgpu refuses. A dispatch's copy is declared as the image it copies.
        let built_in = match target {
            Image::Color(n) | Image::ColorBefore(n) => Some(("colortex", n)),
            Image::ShadowColor(n) | Image::ShadowColorBefore(n) => Some(("shadowcolor", n)),
            _ => None,
        };
        if let Some((kind, n)) = built_in
            && !i.ty.ends_with("image2D")
        {
            notes.push(format!(
                "{name}: `{}` is declared {} but {kind}{n} is 2D; skipped",
                i.name, i.ty
            ));
            return None;
        }
        // A qualifier `rebinds` pairs with its texture's format binds the texture's format,
        // with a note (`storage_formats_of` retypes the image): `layout(rgba16)` on a pack's
        // `RGBA16F` buffer (decision D-3, #548), and `layout(r11f_g11f_b10f)` on the `RGBA16F`
        // stand-in for an `R11F_G11F_B10F` image or buffer (a float stand-in that neither clamps
        // nor drops NaN), which Iris 1.11.7 stored into and read back with a probe pack of ours
        // (#10490). Any other pair that differs refuses the program.
        match image_binding(name, &i.name, &i.format, format, actual) {
            Ok(None) => {}
            Ok(Some(note)) => notes.push(note),
            Err(note) => {
                notes.push(note);
                return None;
            }
        }
        let format = actual;
        let access = match i.access.as_str() {
            "writeonly" => wgpu::StorageTextureAccess::WriteOnly,
            "readonly" => wgpu::StorageTextureAccess::ReadOnly,
            "atomic" => wgpu::StorageTextureAccess::Atomic,
            _ => wgpu::StorageTextureAccess::ReadWrite,
        };
        let storable = match target {
            Image::Color(b) | Image::ColorBefore(b) => pack.storable & 1 << b != 0,
            Image::ShadowColor(n) | Image::ShadowColorBefore(n) => {
                pack.storable & 1 << (BUFFERS + n) != 0
            }
            _ => true,
        };
        if !storable {
            notes.push(format!(
                "{name}: `{}` can't be an image on this GPU; skipped",
                i.name
            ));
            return None;
        }
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: i.binding,
            visibility: pack.gpu.storage_stages(),
            ty: wgpu::BindingType::StorageTexture {
                access,
                format,
                view_dimension: dimension,
            },
            count: None,
        });
        images.push((i.binding, target));
    }
    Some((entries, samplers, slots, reads, shadow_reads, images))
}

/// The value slot of the pack's custom uniform `name` for a program declaring it as GLSL type
/// `ty`: after the frame's uniforms, in the order of `custom` (each with the type
/// `shaders.properties` gives it). A `float`, `int`, `vec2`, `vec3` or `vec4` custom uniform
/// declared as another of these types gets none, and reads 0: Iris 1.11.7 logs "Wrong uniform
/// type ... Disabling that uniform" and our probes read 0 there (float as int, int as float,
/// vec3 as vec2, vec2 as vec3; the vec4 cases were not probed). Other pairs (a `bool` on either
/// side) keep the conversion `Value::write` makes: our probe read a float of 2.5 declared
/// `bool` as true in both games.
pub fn custom_index(custom: &[(String, String)], name: &str, ty: &str) -> Option<usize> {
    const STRICT: [&str; 5] = ["float", "int", "vec2", "vec3", "vec4"];
    let k = custom.iter().position(|(c, _)| c == name)?;
    let declared = custom[k].1.as_str();
    let disabled = declared != ty && STRICT.contains(&declared) && STRICT.contains(&ty);
    (!disabled).then_some(pack_uniforms::NAMES.len() + k)
}

/// The depth bias of vanilla 26.3's polygon-offset pipelines: `RenderPipelines.TEXT_POLYGON_OFFSET`
/// (`DepthStencilState(GREATER_THAN_OR_EQUAL, true, 1.0, 10.0)` over reversed depth), the same
/// bias as `RenderPipelines.CRUMBLING_SNIPPET`'s (which writes no depth): toward the camera in a
/// pack's forward depth.
const POLYGON_OFFSET: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: -10,
    slope_scale: -1.0,
    clamp: 0.0,
};

/// Match a pass's declared GLSL uniforms against mod-declared per-frame uniforms.
/// Incompatible GLSL types leave the uniform unbound, record a note, and log a warning once
/// per `(name, declared_ty)` without panicking. In a pack served Sunrise's HDR extension the
/// display uniforms (`pack_uniforms::DISPLAY`) are the display's, never a mod's: a mod uniform
/// of such a name binds only where the pack isn't served.
fn resolve_mod_uniforms<'a>(
    interface: &'a sunrise_shaders::Interface,
    pass_name: &str,
    pack: &Resources<'_>,
    notes: &mut Vec<String>,
) -> (Vec<(usize, bool, u32)>, HashSet<&'a str>) {
    let mut bound = Vec::new();
    let mut handled = HashSet::new();
    let display = |name: &str| pack.display_uniforms && pack_uniforms::DISPLAY.contains(&name);
    for u in interface.uniforms.iter().filter(|u| !display(&u.name)) {
        match match_mod_uniform(pack.mod_uniforms, &u.name, &u.ty) {
            ModUniformMatch::None => {}
            ModUniformMatch::Bound { index, previous } => {
                bound.push((index, previous, u.offset));
                handled.insert(u.name.as_str());
            }
            ModUniformMatch::TypeMismatch { expected } => {
                handled.insert(u.name.as_str());
                let key = (u.name.clone(), u.ty.clone());
                let first = pack.mod_uniform_mismatches.borrow_mut().insert(key);
                let msg = format!(
                    "{pass_name}: mod uniform `{}` declared as `{}` (mod provides `{}`); left unbound",
                    u.name,
                    u.ty,
                    expected.glsl_name()
                );
                if first {
                    log::warn!("shader pack: {msg}");
                }
                notes.push(msg);
            }
        }
    }
    (bound, handled)
}

/// The pass of program `p`: a quad, or `geometry` drawn by the game. `None` (with a note) when
/// wgpu refuses it or it writes nothing. `formats` are its buffers' (for a program of the shadow
/// buffers, buffer n is shadowcolor n); `shadow` the shadowcolor formats when the shadow pass
/// runs. `water_mask` and `polygon_offset` also make the pass's depth-only pipeline
/// (`Pass::water_mask`) and its polygon-offset one (`Pass::polygon_offset`).
pub fn build_pass(
    device: &wgpu::Device,
    p: sunrise_shaders::PostProgram,
    formats: &[wgpu::TextureFormat; BUFFERS],
    shadow: Option<&[wgpu::TextureFormat; SHADOW_COLORS]>,
    screen: wgpu::TextureFormat,
    notes: &mut Vec<String>,
    geometry: Option<&Geometry<'_>>,
    custom: &[(String, String)],
    draws: &wgpu::BindGroupLayout,
    pack: &Resources<'_>,
    water_mask: bool,
    polygon_offset: bool,
) -> Option<Pass> {
    let features = device.features();
    // The frame's uniforms, then the pack's custom ones.
    let index = |name: &str, ty: &str| {
        pack_uniforms::index_in(name, geometry.is_some(), pack.display_uniforms)
            .or_else(|| custom_index(custom, name, ty))
    };
    let mask = |flips: &[(u32, bool)]| flips.iter().filter(|f| f.1).fold(0u16, |m, f| m | 1 << f.0);
    let mut p = p;
    let mut program = p.program.take()?;
    // A mod's own vertex stage replaces the pack's, when it fits the pack's fragment stage.
    let mod_vertex = match pack.mod_vertex.borrow_mut().take() {
        Some(stage) => {
            // The pack's vertex stage carries a geometry stage's inputs: the mod's can't replace it.
            if p.geometry_triangles > 0 {
                notes.push(format!(
                    "{}: the mod's own vertex stage is not used: the pack's program has a geometry stage",
                    p.name
                ));
                return None;
            }
            match super::mod_link::link_vertex_stage(
                &stage.module,
                &program.fragment,
                &program.varyings,
                stage.set1_layout.is_some(),
                device.limits().max_bind_groups,
            ) {
                Ok(linked) => {
                    program.vertex = linked;
                    Some(stage)
                }
                Err(reason) => {
                    notes.push(format!(
                        "{}: the mod's own vertex stage is not used: {reason}",
                        p.name
                    ));
                    return None;
                }
            }
        }
        None => None,
    };
    let interface = &program.interface;
    let draw = interface.draw;
    let storage = !interface.images.is_empty() || !interface.buffers.is_empty();
    if storage && !pack.gpu.fragment_storage {
        notes.push(format!(
            "{}: this GPU can't write images from a fragment shader; skipped",
            p.name
        ));
        return None;
    }
    // Terrain's geometry stage reads the draw's indices at set 5; entities' walk their quads.
    let reads_indices = p.geometry_triangles > 0
        && (program.vertex.global_variables.iter())
            .any(|(_, v)| v.binding.as_ref().is_some_and(|b| b.group == 5));
    if p.geometry_triangles > 0
        && ((reads_indices && pack.indices.is_none()) || !pack.gpu.vertex_storage)
    {
        notes.push(format!(
            "{}: this GPU can't run a geometry stage; skipped",
            p.name
        ));
        return None;
    }
    let (entries, samplers, slots, reads, shadow_reads, images) = bindings(
        features, interface, &p.name, p.group, &p.buffers, geometry, formats, shadow, notes, pack,
    )?;
    let final_pass = p.group == Group::Final;
    // Outputs into an integer buffer, and its integer kind (`packed::bitcast_outputs`).
    let mut int_outputs = Vec::new();
    let mut targets = Vec::new();
    let mut writes = Vec::new();
    // Outputs into a buffer of a small packed format, and the bits it keeps.
    let mut packed_outputs = Vec::new();
    // Each output's own `blend.<program>.<buffer>`, and whether its buffer blends at all: what
    // the sky's part pipelines keep (`PostProgram::vanilla_sky_blends`).
    let mut own_blends: Vec<(Option<wgpu::BlendState>, bool)> = Vec::new();
    for &location in &interface.outputs {
        let l = location as usize;
        // Only an output with somewhere to go takes a target slot: the location is the pack's
        // (up to `u32::MAX`), the slots only as many as the buffers it can write.
        let buffer = if final_pass {
            (location == 0).then_some(None)
        } else {
            p.buffers.get(l).map(Some)
        };
        let Some(buffer) = buffer else {
            continue;
        };
        if targets.len() <= l {
            targets.resize(l + 1, None);
        }
        if final_pass {
            targets[0] = target(screen);
        } else if let Some(&buffer) = buffer {
            let format = formats[buffer as usize];
            match format.sample_type(None, Some(features)) {
                Some(wgpu::TextureSampleType::Uint) => {
                    int_outputs.push((location, naga::ScalarKind::Uint));
                }
                Some(wgpu::TextureSampleType::Sint) => {
                    int_outputs.push((location, naga::ScalarKind::Sint));
                }
                _ => {}
            }
            let mut state = target(format);
            if !p.group.shadow()
                && pack.rgb & 1 << buffer != 0
                && let Some(t) = &mut state
            {
                t.write_mask = wgpu::ColorWrites::COLOR;
            }
            let own = p.buffer_blends.iter().find(|(n, _)| *n == buffer);
            let blendable = format
                .guaranteed_format_features(features)
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE);
            if let (Some(t), Some(b)) = (&mut state, own.map_or(p.blend, |(_, b)| *b))
                && blendable
            {
                t.blend = Some(blend_state(b));
            }
            if own_blends.len() <= l {
                own_blends.resize(l + 1, (None, false));
            }
            let own = own.and_then(|(_, b)| *b).filter(|_| blendable);
            own_blends[l] = (own.map(blend_state), blendable);
            if !p.group.shadow()
                && let Some(bits) = pack.packed[buffer as usize]
            {
                packed_outputs.push((location, bits));
            }
            targets[l] = state;
            writes.push((location, buffer as usize));
        }
    }
    // Outputs past the last buffer the program writes go nowhere, as in GL; a pass has no
    // attachment for them.
    while targets.last().is_some_and(Option::is_none) {
        targets.pop();
    }
    // A program that only writes images draws into a sink that keeps nothing; one drawing
    // geometry has the depth buffer, and draws with no colour target at all.
    let sink = !final_pass && writes.is_empty() && !program.interface.images.is_empty();
    if sink && geometry.is_none() {
        targets = vec![Some(wgpu::ColorTargetState {
            format: SINK,
            blend: None,
            write_mask: wgpu::ColorWrites::empty(),
        })];
    } else if !final_pass && writes.is_empty() && !sink {
        // It draws nothing a later program could see: its compute stages and forced flips are
        // all it does, and the caller drops it when it has neither.
        if geometry.is_none() {
            let flip = mask(&p.flips);
            return Some(no_draw(
                device,
                p.name,
                p.group,
                flip,
                !interface.buffers.is_empty(),
            ));
        }
        notes.push(format!("{}: writes no buffer; skipped", p.name));
        return None;
    }
    let written = writes.iter().fold(0u16, |m, w| m | 1 << w.1);
    let kept = p
        .flips
        .iter()
        .filter(|f| !f.1)
        .fold(0u16, |m, f| m | 1 << f.0);
    // Gbuffers programs write in place (6.1.6). A forced flip of a buffer the program writes
    // cancels the write's own flip, as in Iris 1.11.7 (probed with packs of our own).
    let flip = if final_pass || geometry.is_some() {
        0
    } else {
        (written & !kept) ^ mask(&p.flips)
    };
    let created = checked(device, || {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(&p.name),
            entries: &entries,
        });
        let mut sets = vec![
            Some(&layout),
            geometry.and_then(|g| g.table),
            draw.then_some(draws),
            geometry.and_then(|g| g.ids),
            (!interface.buffers.is_empty()).then_some(&pack.buffers.layout),
            pack.indices.filter(|_| reads_indices),
        ];
        // The mod's own sets go after the pack's (`MOD_SET_BASE`).
        if let Some(stage) = &mod_vertex {
            sets.push(Some(&stage.globals_layout));
            sets.extend(stage.set1_layout.as_ref().map(Some));
        }
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&p.name),
            bind_group_layouts: &sets,
            ..Default::default()
        });
        let module = |mut m: naga::Module| {
            share_samplers(&mut m, &slots);
            storage_formats_of(&mut m, &entries);
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&p.name),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(m)),
            })
        };
        let mut fragment = program.fragment;
        packed::quantise_outputs(&mut fragment, &packed_outputs);
        packed::bitcast_outputs(&mut fragment, &int_outputs);
        let (vs, fs) = (module(program.vertex), module(fragment));
        let job = RenderJob {
            label: p.name.clone(),
            layout: pipeline_layout,
            vs,
            fs,
            // A geometry stage reads its vertices itself.
            buffers: geometry
                .filter(|_| p.geometry_triangles == 0)
                .map_or_else(Vec::new, |g| owned_buffers(g.buffers)),
            // Rows flipped to GL's turn front faces clockwise.
            primitive: wgpu::PrimitiveState {
                // The mod's own vertex stage winds as its pipeline does.
                front_face: if mod_vertex.is_some() {
                    wgpu::FrontFace::Ccw
                } else {
                    wgpu::FrontFace::Cw
                },
                cull_mode: geometry
                    .filter(|g| g.cull && p.geometry_triangles == 0)
                    .map(|_| wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: geometry.map(|g| wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(g.depth_write),
                depth_compare: Some(pack.depth_compare.get()),
                stencil: Default::default(),
                bias: if p.name == "gbuffers_damagedblock" {
                    POLYGON_OFFSET
                } else {
                    Default::default()
                },
            }),
            targets: targets.to_vec(),
        };
        // The program's own blend replaced by vanilla's for the part; a buffer's own
        // `blend.<program>.<buffer>` still applies (Iris 1.11.7 blended the sky disc into that
        // buffer only, with a probe pack of ours). Two more pipelines of the program, made
        // with the others (`compile`).
        let sky_parts = p.vanilla_sky_blends.then(|| {
            [None, Some(blend_state(sunrise_shaders::STARS_BLEND))].map(|part| {
                let targets: Vec<_> = (job.targets.iter().enumerate())
                    .map(|(l, t)| {
                        let (own, blendable) = own_blends.get(l).copied().unwrap_or((None, false));
                        t.clone().map(|t| wgpu::ColorTargetState {
                            blend: own.or(part.filter(|_| blendable)),
                            ..t
                        })
                    })
                    .collect();
                pack.render_pipeline(
                    device,
                    RenderJob {
                        targets,
                        ..job.clone()
                    },
                )
            })
        });
        // The water patches' pipeline: the same program, depth only.
        let water_mask = water_mask.then(|| {
            let targets = (job.targets.iter())
                .map(|t| {
                    t.clone().map(|t| wgpu::ColorTargetState {
                        write_mask: wgpu::ColorWrites::empty(),
                        ..t
                    })
                })
                .collect();
            pack.render_pipeline(
                device,
                RenderJob {
                    targets,
                    ..job.clone()
                },
            )
        });
        // Polygon-offset sign text's pipeline (`Draw::polygon_offset`): the same program, its
        // depth biased toward the camera.
        let polygon_offset = polygon_offset.then(|| {
            pack.render_pipeline(
                device,
                RenderJob {
                    depth_stencil: job.depth_stencil.clone().map(|d| wgpu::DepthStencilState {
                        bias: POLYGON_OFFSET,
                        ..d
                    }),
                    ..job.clone()
                },
            )
        });
        (
            layout,
            pack.render_pipeline(device, job),
            sky_parts,
            water_mask,
            polygon_offset,
        )
    });
    let (layout, pipeline, sky_parts, water_mask, polygon_offset) = match created {
        Ok(created) => created,
        Err(e) => {
            notes.push(format!("{}: skipped: {e}", p.name));
            return None;
        }
    };
    let interface = &program.interface;
    let (mod_uniforms, mod_handled) = resolve_mod_uniforms(interface, &p.name, pack, notes);
    let missing: Vec<&str> = interface
        .uniforms
        .iter()
        .filter(|u| {
            !u.name.starts_with("hz_")
                && !mod_handled.contains(u.name.as_str())
                && index(&u.name, &u.ty).is_none()
        })
        .map(|u| u.name.as_str())
        .collect();
    if !missing.is_empty() {
        notes.push(format!(
            "{}: shader defaults or zero: {}",
            p.name,
            missing.join(" ")
        ));
    }
    let image_copies = image_copies(&samplers, &images);
    let (shadow_own, shadow_before) = shadow_images(&images);
    Some(Pass {
        ubo: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&p.name),
            size: u64::from(interface.uniform_size.max(16)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        block: interface.initial_values.clone(),
        initial_block: interface.initial_values.clone(),
        uniforms: interface
            .uniforms
            .iter()
            .map(|u| (index(&u.name, &u.ty), u.ty.clone(), u.offset))
            .collect(),
        mod_uniforms,
        name: p.name,
        group: p.group,
        pipeline: Pipeline::Render(pipeline),
        sky_parts,
        water_mask,
        polygon_offset,
        layout,
        samplers,
        writes,
        flip,
        depth_linear: depth_linear(interface, pack.shadow_linear),
        shadow_reads: shadow_reads | shadow_own | shadow_before,
        shadow_copies: shadow_copies(p.group, reads, shadow_reads, &images),
        written,
        mipmaps: p.mipmaps,
        draw,
        atlas: geometry.and_then(|g| g.atlas.clone()),
        material_maps: geometry.is_some_and(|g| g.material_maps),
        paged: false,
        ids: geometry.is_some_and(|g| g.ids.is_some()),
        groups: HashMap::new(),
        portal_groups: HashMap::new(),
        dynamic_groups: HashMap::new(),
        block_atlas_groups: HashMap::new(),
        copies: (image_buffers(&images)
            | if p.group == Group::Translucent {
                written
            } else {
                0
            })
            & reads,
        reads: reads | image_buffers(&images),
        buffers: !interface.buffers.is_empty(),
        images,
        compute: Vec::new(),
        sink,
        geometry: p.geometry_triangles,
        viewport: (!final_pass && p.scale != [1.0, 0.0, 0.0]).then_some(p.scale),
        image_copies,
        snapshots: 0,
    })
}

/// A pass that draws nothing (`Pipeline::NoDraw`): it runs the compute stages its caller adds,
/// then flips `flip`. `storage`: its fragment stage binds storage buffers.
fn no_draw(device: &wgpu::Device, name: String, group: Group, flip: u16, storage: bool) -> Pass {
    Pass {
        ubo: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&name),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(&name),
            entries: &[],
        }),
        block: Vec::new(),
        initial_block: Vec::new(),
        uniforms: Vec::new(),
        mod_uniforms: Vec::new(),
        name,
        group,
        pipeline: Pipeline::NoDraw { storage },
        sky_parts: None,
        water_mask: None,
        polygon_offset: None,
        samplers: Vec::new(),
        writes: Vec::new(),
        flip,
        depth_linear: [0.0; 4],
        shadow_reads: 0,
        shadow_copies: 0,
        written: 0,
        mipmaps: 0,
        draw: false,
        atlas: None,
        material_maps: false,
        paged: false,
        ids: false,
        groups: HashMap::new(),
        portal_groups: HashMap::new(),
        dynamic_groups: HashMap::new(),
        block_atlas_groups: HashMap::new(),
        copies: 0,
        reads: 0,
        buffers: false,
        images: Vec::new(),
        compute: Vec::new(),
        sink: false,
        geometry: 0,
        viewport: None,
        image_copies: 0,
        snapshots: 0,
    }
}

/// The pack's images a pass both samples and stores.
pub fn image_copies(samplers: &[Binding], images: &[(u32, Image)]) -> u64 {
    samplers
        .iter()
        .filter_map(|s| match s.source {
            Source::Image(k) if images.iter().any(|i| i.1 == Image::Custom(k)) => Some(k),
            _ => None,
        })
        .fold(0, |m, k| m | 1 << k)
}

/// The pass of compute program `c` of a `group` program: `None` (with a note) when wgpu refuses
/// it or it needs what the game can't give.
pub fn build_compute(
    device: &wgpu::Device,
    name: String,
    group: Group,
    c: sunrise_shaders::ComputeProgram,
    formats: &[wgpu::TextureFormat; BUFFERS],
    shadow: Option<&[wgpu::TextureFormat; SHADOW_COLORS]>,
    notes: &mut Vec<String>,
    custom: &[(String, String)],
    pack: &Resources<'_>,
) -> Option<Pass> {
    let index = |name: &str, ty: &str| {
        pack_uniforms::index_in(name, false, pack.display_uniforms)
            .or_else(|| custom_index(custom, name, ty))
    };
    let interface = &c.interface;
    let (entries, samplers, slots, reads, shadow_reads, images) = bindings(
        device.features(),
        interface,
        &name,
        group,
        &[],
        None,
        formats,
        shadow,
        notes,
        pack,
    )?;
    let local = c.module.entry_points[0].workgroup_size;
    let created = checked(device, || {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(&name),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&name),
            bind_group_layouts: &[
                Some(&layout),
                None,
                None,
                None,
                (!interface.buffers.is_empty()).then_some(&pack.buffers.layout),
            ],
            ..Default::default()
        });
        let mut module = c.module;
        share_samplers(&mut module, &slots);
        storage_formats_of(&mut module, &entries);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&name),
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        });
        let job = ComputeJob {
            label: name.clone(),
            layout: pipeline_layout,
            module,
        };
        (layout, pack.compute_pipeline(device, job))
    });
    let (layout, pipeline) = match created {
        Ok(created) => created,
        Err(e) => {
            notes.push(format!("{name}: skipped: {e}"));
            return None;
        }
    };
    let written = image_buffers(&images);
    let image_copies = image_copies(&samplers, &images);
    let (before_colors, snapshots) = befores(&images);
    let (shadow_own, shadow_before) = shadow_images(&images);
    let (mod_uniforms, _) = resolve_mod_uniforms(interface, &name, pack, notes);
    Some(Pass {
        ubo: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&name),
            size: u64::from(interface.uniform_size.max(16)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        block: interface.initial_values.clone(),
        initial_block: interface.initial_values.clone(),
        uniforms: interface
            .uniforms
            .iter()
            .map(|u| (index(&u.name, &u.ty), u.ty.clone(), u.offset))
            .collect(),
        mod_uniforms,
        name,
        group,
        pipeline: Pipeline::Compute {
            pipeline,
            groups: c.work_groups,
            render: c.work_groups_render,
            local,
        },
        sky_parts: None,
        water_mask: None,
        polygon_offset: None,
        layout,
        samplers,
        writes: Vec::new(),
        flip: 0,
        depth_linear: depth_linear(interface, pack.shadow_linear),
        copies: written & reads | before_colors,
        shadow_copies: shadow_copies(group, reads, shadow_reads, &images),
        reads: reads | written,
        shadow_reads: shadow_reads | shadow_own | shadow_before,
        written,
        mipmaps: 0,
        draw: false,
        atlas: None,
        material_maps: false,
        paged: false,
        ids: false,
        groups: HashMap::new(),
        portal_groups: HashMap::new(),
        dynamic_groups: HashMap::new(),
        block_atlas_groups: HashMap::new(),
        buffers: !interface.buffers.is_empty(),
        images,
        compute: Vec::new(),
        sink: false,
        geometry: 0,
        image_copies,
        snapshots,
        viewport: None,
    })
}

/// Why a pack can't run, and the notes of loading it until then (why programs were skipped).
#[derive(Debug)]
pub struct Refused {
    pub reason: String,
    pub notes: Vec<String>,
}

impl Refused {
    /// The reason of a build stopped by its `cancel` flag (`PackPost::new`).
    pub const CANCELLED: &str = "cancelled";
    pub fn cancelled(notes: Vec<String>) -> Self {
        Self {
            reason: Self::CANCELLED.into(),
            notes,
        }
    }
    /// Whether the build was cancelled (another pack or dimension asked for): not the pack's
    /// fault, so nothing to report to the player.
    pub fn is_cancelled(&self) -> bool {
        self.reason == Self::CANCELLED
    }
}

/// A pack's post-processing pipelines and what they run on: its passes, buffers, samplers and
/// uniforms (`new` makes them) and the frame's own work on them (`execution.rs`).
pub struct PackPost {
    pub setup_pending: bool,
    pub notes: Vec<String>,
    pub passes: Vec<Pass>,
    /// Shadow cache for temporal reprojection.
    pub shadow_cache: ShadowCache,
    /// What draws terrain (`gbuffers_terrain` or its fallback), between `prepare` and `deferred`;
    /// when the pack draws terrain in two parts, solid terrain only (`gbuffers_terrain_solid` or
    /// its fallback).
    pub terrain: Option<Pass>,
    /// What draws cutout terrain (`gbuffers_terrain_cutout` or its fallback), after `terrain`,
    /// when the pack draws terrain in two parts. A pack with no program for one part has only
    /// the other.
    pub terrain_cutout: Option<Pass>,
    /// What draws translucent terrain (`gbuffers_water` or its fallback), after `deferred`.
    pub water: Option<Pass>,
    /// The pack has no program for solid and cutout terrain, or for translucent terrain:
    /// nothing draws it (`sunrise_shaders::Post::terrain_undrawn`).
    pub terrain_undrawn: bool,
    pub water_undrawn: bool,
    /// What draws the sky (`gbuffers_skybasic` or its fallback), before terrain, and its vertices.
    pub sky: Option<Sky>,
    /// What draws entities and block entities, after terrain, and their vertices.
    pub entities: Option<Entities>,
    /// What draws the clouds, after entities, and this frame's clouds.
    pub clouds: Option<Pass>,
    /// What draws the block outline (`gbuffers_line`, else `gbuffers_basic`), and this frame's.
    pub lines: Option<Pass>,
    pub line_draw: Option<Lines>,
    pub cloud_draw: Option<Clouds>,
    /// The pack draws the clouds (`gbuffers_clouds` or its fallback), even with a program that
    /// writes nothing (a pack drawing its own clouds discards vanilla's), or `clouds=off`: the
    /// game's clouds are not drawn.
    pub hides_clouds: bool,
    /// The pack has a sky (`gbuffers_skybasic`) or sun and moon (`gbuffers_skytextured`)
    /// program: vanilla's stays hidden even when the program writes nothing.
    pub hides_sky: bool,
    pub hides_celestial: bool,
    /// `sky`, `stars`, `sun` and `moon`: the parts of the vanilla sky left out, by the pack's sky
    /// programs and by the game's sky alike.
    pub sky_switches: sunrise_shaders::SkySwitches,
    /// `colortex0ClearColor`, when the pack sets it.
    pub sky_clear: Option<[f32; 4]>,
    /// What draws rain and snow (the same program, each with its kind's sprite as `texture`),
    /// after translucent terrain, this frame's weather and the sprites' generation bound.
    pub weather: Option<[Pass; 2]>,
    pub weather_draw: Option<Weather>,
    pub weather_generation: u64,
    /// What draws the first-person hand (`gbuffers_hand` or its fallback), after the opaque
    /// world, and its vertices.
    pub hand: Option<(Pass, crate::pack_hand::PackHand)>,
    /// `gbuffers_hand_water` (or its fallback): the hand's translucent quads, after deferred.
    pub hand_water: Option<Pass>,
    /// The pre-translucent copy: `depthtex1`, or `depthtex2` without a shader hand.
    pub depth1: bool,
    /// A program after the hand reads `depthtex2`: it is copied before the hand.
    pub depth2: bool,
    /// Translucent terrain reads the boats' water patches (`Targets::water_mask`).
    pub water_mask: bool,
    /// The water patches' depth holds patches (or was never cleared): cleared before the next
    /// frame's are drawn.
    pub water_mask_dirty: bool,
    /// The shadow buffers, when the shadow pass runs (2.5), and the `shadowcomp` programs.
    pub shadow_targets: Option<crate::pack_shadow::ShadowTargets>,
    /// What draws terrain from the light (`shadow`), and translucent terrain (`shadow_water`).
    pub shadow_terrain: Option<Pass>,
    /// `end_portal.png`'s atlas rect this frame, and a texture of its own copied from it that
    /// End portal draws bind as their atlas.
    pub portal_rect: [f32; 4],
    pub portal_texture: Option<([f32; 4], wgpu::TextureView)>,
    pub shadow_water: Option<Pass>,
    pub shadow_comp: Vec<Pass>,
    /// Buffers with mipmaps (filterable ones a program asks them for).
    pub mipped: u16,
    /// Buffers whose mip chain matches their first level, this frame.
    pub mips_valid: u16,
    /// The downsampling pipeline of each format with mipmaps.
    pub down: HashMap<wgpu::TextureFormat, (wgpu::RenderPipeline, wgpu::BindGroupLayout)>,
    /// The per-draw uniforms of gbuffers programs.
    pub draws: DrawUniforms,
    /// Set 3 of flat terrain: an unknown-source fallback and immutable state/sprite tables.
    /// Each page's bind group tracks both its vertex buffer and optional source sidecar.
    pub block_ids: Option<(wgpu::BindGroupLayout, wgpu::Buffer, wgpu::Buffer)>,
    pub page_groups: HashMap<u32, (wgpu::Buffer, Option<wgpu::Buffer>, wgpu::BindGroup)>,
    pub gui_size: [u32; 2],
    /// GPU time of each pass, while the renderer asks for it (`pack_timing.rs`).
    pub timing: Option<crate::pack_timing::PassTiming>,
    /// `depthtex0` at the screen's centre, read back for `centerDepthSmooth`.
    pub center_depth: Option<crate::pack_center_depth::CenterDepth>,
    /// `shaders.properties`' custom uniforms, after the frame's in `uniforms.values`.
    pub custom: sunrise_shaders::Custom,
    pub atlas: Option<wgpu::TextureView>,
    pub material_maps: Option<[wgpu::TextureView; 2]>,
    /// The shared atlas's normal and specular paged maps (`set_paged_maps`).
    pub paged_maps: [Option<wgpu::TextureView>; 2],
    pub lightmap: wgpu::TextureView,
    pub deferred_pre: u16,
    pub composite_pre: u16,
    pub formats: [wgpu::TextureFormat; BUFFERS],
    pub clears: [Option<[f32; 4]>; BUFFERS],
    /// Buffers of a three-channel format, and whether their textures are new: then they get
    /// alpha 1 before the next frame.
    pub rgb: u16,
    pub rgb_unset: bool,
    pub used: u16,
    /// Buffers some pass draws into the "alt" side of or flips; the others' B is their A.
    pub doubled: u16,
    /// `size.buffer.*`.
    pub sizes: [Option<sunrise_shaders::BufferSize>; BUFFERS],
    /// Buffers bound as images, and those also copied for a pass that samples them.
    pub storage: u16,
    pub copied: u16,
    /// Bit n set: B is buffer n's "main".
    pub main: u16,
    pub targets: Option<Targets>,
    pub noise: wgpu::TextureView,
    pub dummy: wgpu::TextureView,
    pub dummy_black: wgpu::TextureView,
    pub dummy_normal: wgpu::TextureView,
    pub dummy_3d: wgpu::TextureView,
    pub dummy_1d: wgpu::TextureView,
    /// `image.*` (the storage buffers, `bufferObject.*`, are `buffers`).
    pub images: Vec<pack_images::PackImage>,
    /// The shadowcolor buffers a pass outside the shadow group binds as images (bit n:
    /// shadowcolor n): such a pass changes them after the shadow pass, so the Shadow Cache
    /// can't keep them.
    pub shadow_images_outside: u16,
    /// The shadowcolor buffers a shadow program binds as images: cleared before the shadow pass
    /// draws (`ShadowTargets::clear_images`).
    pub shadow_pass_images: u16,
    /// The shadowcolor buffers a shadow program, or a compute stage attached to one, draws
    /// into, flips or binds as an image: the Shadow Cache keeps the pass only while all of them
    /// are cleared every frame.
    pub shadow_writes: u16,
    /// A shadow program, or a compute stage attached to one, binds the pack's storage buffers
    /// (`bufferObject.*`, set 4): they are zeroed once and then kept between frames, and the
    /// cache key doesn't hold them, so the Shadow Cache never keeps such a pass.
    pub shadow_buffers: bool,
    /// Set 5 of programs with a geometry stage.
    pub indices: Option<wgpu::BindGroupLayout>,
    pub shadow_culling: sunrise_shaders::ShadowCulling,
    pub gpu: Gpu,
    /// Zeros to clear images by copying, where the device has no `clear_texture`.
    pub zeros: Option<wgpu::Buffer>,
    /// What the images no `image.*` gives bind.
    pub stand_ins: Vec<pack_images::StandIn>,
    pub dump: Option<pack_images::Dump>,
    pub buffers: pack_images::Buffers,
    pub textures: Vec<pack_textures::PackTexture>,
    /// The stand-in shadow map and its comparison sampler; cleared on the first frame.
    pub shadow: (wgpu::TextureView, wgpu::Sampler),
    pub shadow_cleared: bool,
    pub linear: wgpu::Sampler,
    pub nearest: wgpu::Sampler,
    pub repeat: wgpu::Sampler,
    pub nearest_repeat: wgpu::Sampler,
    pub atlas_sampler: wgpu::Sampler,
    /// colortex buffers' (`SamplerKind::Buffer`).
    pub buffer_sampler: wgpu::Sampler,
    /// None when it would write nothing: colortex0 holds integers and there is no depth to copy
    /// nor buffer to clear.
    pub blit: Option<(wgpu::RenderPipeline, wgpu::BindGroupLayout)>,
    /// The blit also writes depthtex0 (some program reads it) and clears these buffers.
    /// Whether any gbuffer pass in this pack writes `targets.depth` (and therefore
    /// `fs_export_depth` should write the pack's final depth back to Sunrise's `world_depth`).
    pub draws_depth: bool,
    /// Whether `targets.depth` receives the world's depth (`blit_depth` includes post-only
    /// depth readers in addition to `draws_depth`).
    pub blit_depth: bool,
    pub blit_clears: Vec<usize>,
    /// Buffers cleared in passes of their own (more than the blit's eight attachments).
    pub other_clears: Vec<usize>,
    /// Source views identify the world target, including replacements at the same size.
    pub blit_group: Option<([wgpu::TextureView; 2], wgpu::BindGroup)>,
    /// Shows colortex0 when the pack has no `final` (OQ-5); served Sunrise's HDR (`hdr`), it
    /// encodes the linear light colortex0 holds as a served `final` does (`fs_copy_linear`),
    /// loading texels only: its layout has no sampler and takes an unfilterable colortex0.
    pub copy: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    /// Served Sunrise's HDR (`hdr`), the copy of colortex0 when it is the window's size: each
    /// pixel its own texel (`fs_copy_linear_texel`), filtered by nothing.
    pub copy_texel: Option<(wgpu::RenderPipeline, wgpu::BindGroupLayout)>,
    /// Sunrise's HDR extension as the pack was served it (`Post::hdr`): what its picture on
    /// the frame was encoded from, and who owns the tone curve.
    pub hdr: Option<sunrise_shaders::Tonemap>,
    /// Copies the picture `final` drew at the world's size onto the window: no flip, it is
    /// already in the screen's rows. Served Sunrise's HDR, alpha isn't filtered
    /// (`fs_upscale_linear`).
    pub upscale: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    /// Copies the pack's GL-rows, GL-depth `targets.depth` back into Sunrise's screen-rows,
    /// reversed-Z `world_depth` so post-pack stages (`AfterLevel`, `world_drawn`) see valid
    /// terrain, water, entity, and hand depth.
    pub export_depth: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    /// Cached bind group for `export_depth`, keyed by the pack's `targets.depth` view.
    pub export_depth_group: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    /// The picture `final` draws while the world's size is not the window's.
    pub final_target: Option<FinalTarget>,
    pub screen: wgpu::TextureFormat,
    pub uniforms: FrameUniforms,
    /// Mod-declared render pipelines bound to translated pack programs.
    pub mod_passes: Vec<ModPackPass>,
    /// Pipeline slots bound to `mod_passes`, cached once at build time for zero-alloc queries.
    pub bound_mod_slots: Vec<usize>,
    /// Uploaded quad draw chunks for bound mod pipelines on the current frame.
    pub mod_draws: Vec<ModDrawChunk>,
    /// Mod-declared auxiliary depth targets and lazy usage flags (allocated only when sampled).
    pub mod_depth_decls: Vec<ModDepthTargetDecl>,
    pub mod_depth_used: Vec<bool>,
    pub mod_opaque_depth_used: Vec<bool>,
    /// Mod-declared per-frame uniforms with automatic previous-frame tracking.
    pub mod_uniforms: ModUniformTable,
}

impl PackPost {
    /// Builds the pipelines of `post` for a screen of `screen` format. An error when the pack
    /// can't show anything on the screen: the game turns it off, as a pack that fails to load.
    ///
    /// The pack is laid out first and its pipelines are made after, all at once (`compile`).
    /// When wgpu or the driver refuses one, the pack is built again a program at a time, so that
    /// the program is skipped with its note as it always was.
    ///
    /// `cancel` set (the pack isn't wanted any more) stops the build between programs: a refusal
    /// whose `is_cancelled` is true.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        adapter: &wgpu::Adapter,
        post: Post,
        screen: wgpu::TextureFormat,
        world: World<'_>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<PackPost, Refused> {
        Self::new_with_mods(
            (device, queue, adapter),
            post,
            screen,
            world,
            ModPackRuntime::default(),
            cancel,
        )
    }

    /// As [`Self::new`], with mod-declared auxiliary depth targets, frame uniforms, and
    /// pipeline pack bindings.
    pub fn new_with_mods(
        gpu: (&wgpu::Device, &wgpu::Queue, &wgpu::Adapter),
        post: Post,
        screen: wgpu::TextureFormat,
        world: World<'_>,
        mods: ModPackRuntime<'_>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<PackPost, Refused> {
        use std::sync::atomic::Ordering;
        let spare = (post.clone(), world.clone(), mods.clone());
        let (built, jobs) = PackPost::build(gpu, post, screen, world, mods, true, cancel)?;
        let refused = compile(gpu.0, &jobs, cancel);
        // Some pipelines may be left unmade: the pack can't draw.
        if cancel.load(Ordering::Relaxed) {
            return Err(Refused::cancelled(built.notes));
        }
        let Some((program, refused)) = refused else {
            return Ok(built);
        };
        log::warn!(
            "shader pack: {program} was refused ({refused}): building it again a program at a time"
        );
        drop((built, jobs));
        let (post, world, mods) = spare;
        PackPost::build(gpu, post, screen, world, mods, false, cancel).map(|(built, _)| built)
    }

    /// `new`'s work: with `defer`, the pipelines are left to make (the jobs returned).
    fn build(
        (device, queue, adapter): (&wgpu::Device, &wgpu::Queue, &wgpu::Adapter),
        mut post: Post,
        screen: wgpu::TextureFormat,
        world: World<'_>,
        mods: ModPackRuntime<'_>,
        defer: bool,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<(PackPost, Vec<Job>), Refused> {
        // What the pack's shaders need of its storage buffers, before its parts move out.
        let minimums = post.buffer_minimums();
        // Pre-scan every loaded program before `post`'s parts move out so mod geometry passes
        // targeting an actively sampled auxiliary depth buffer can legally sample `depthtex0`.
        let mut pre_mod_depth_sampled = vec![false; mods.depth_targets.len()];
        if !mods.depth_targets.is_empty() {
            let samplers = post
                .every_program()
                .flat_map(|p| {
                    p.program
                        .iter()
                        .map(|q| &q.interface)
                        .chain(p.compute.iter().map(|(_, c)| &c.interface))
                })
                .flat_map(|i| &i.samplers);
            for s in samplers {
                if let Some(Source::ModDepth(idx) | Source::ModOpaqueDepth(idx)) =
                    mod_depth_source(mods.depth_targets, &s.name)
                {
                    pre_mod_depth_sampled[idx] = true;
                }
            }
        }
        let features = device.features();
        let formats: [wgpu::TextureFormat; BUFFERS] = std::array::from_fn(|i| {
            let f = post.config.buffers[i].format;
            snorm_format(f, features, adapter)
                .unwrap_or_else(|| target_format(f, features, adapter))
        });
        let sky_clear = Some(&post.config.buffers[0])
            .filter(|b| b.clear)
            .and_then(|b| b.clear_color);
        // A three-channel buffer reads alpha 1 whatever is written or cleared, as in Iris: its
        // passes leave alpha alone, and its clears and first contents set it to 1.
        let rgb = (0..BUFFERS)
            .filter(|&i| post.config.buffers[i].format.channels == 3)
            .fold(0u16, |m, i| m | 1 << i);
        // A small packed format's buffer reads back fewer bits, as in Iris: its clears and
        // what its passes write keep only those.
        let packed: [Option<[u8; 4]>; BUFFERS] =
            std::array::from_fn(|i| packed::bits(post.config.buffers[i].format));
        let clears = std::array::from_fn(|i| {
            let b = &post.config.buffers[i];
            b.clear.then(|| {
                let mut c = (b.clear_color).unwrap_or(if i == 1 { [1.0; 4] } else { [0.0; 4] });
                if rgb & 1 << i != 0 {
                    c[3] = 1.0;
                }
                packed[i].map_or(c, |bits| packed::quantise(c, bits))
            })
        });
        let hdr = post.hdr;
        let mut notes = post.notes;
        notes.extend(post.config.notes);
        let mut sizes = post.sizes;
        // The blit writes colortex0 beside the depth buffer, at the render size.
        if sizes[0].take().is_some() {
            notes.push("size.buffer.colortex0: colortex0 keeps the render size".into());
        }
        let textures = pack_textures::upload(device, queue, post.textures, &mut notes);
        let shadow_config = post.config.shadow.clone();
        let noise_size = post.config.noise_resolution.unwrap_or(256);
        let half_lives = post.config.half_lives;
        for (name, ticks) in sunrise_shaders::HALF_LIVES.iter().zip(half_lives) {
            if let Some(t) = ticks {
                notes.push(format!("`{name}` is {t} ticks"));
            }
        }
        // shadowcolor2 to 7 only for a pack that names `HIGHER_SHADOWCOLOR`.
        let shadow_colors = (1u16 << post.shadow_colors) - 1;
        let mut custom = post.custom;
        let custom_names: Vec<(String, String)> = (custom.uniforms())
            .map(|(name, ty)| (name.to_owned(), ty.to_owned()))
            .collect();
        // A first run finds the names neither the frame nor the pack defines.
        custom.eval(0.0, &pack_uniforms::zero);
        // Its zeros are no start for `smooth`: the first frame's values are, as in Iris.
        custom.restart_smoothing();
        if !custom.notes.is_empty() {
            notes.push(format!("custom uniforms: {}", custom.notes.join("; ")));
            custom.notes.clear();
        }
        let mask =
            |flips: &[(u32, bool)]| flips.iter().filter(|f| f.1).fold(0u16, |m, f| m | 1 << f.0);
        let mut used = 1u16;
        // Buffers some pass draws into the "alt" side of, or flips: only these get a B texture.
        let mut doubled = mask(&post.deferred_pre) | mask(&post.composite_pre);
        let mut shadow_doubled = 0u16;
        // shadowcolor buffers bound as images, and those copied before a pass.
        let (mut shadow_storage, mut shadow_copied) = (0u16, 0u16);
        let (mut shadow_images_outside, mut shadow_pass_images) = (0u16, 0u16);
        let mut shadow_writes = 0u16;
        let mut shadow_buffers = false;
        let draws = DrawUniforms::new(device);
        // The shadow buffers' formats when the shadow pass runs; their programs see them as
        // buffers 0 to 7.
        let shadow_formats: Option<[wgpu::TextureFormat; SHADOW_COLORS]> =
            post.shadow_pass.then(|| {
                std::array::from_fn(|n| {
                    target_format(shadow_config.colors[n].format, features, adapter)
                })
            });
        let shadow_space: [wgpu::TextureFormat; BUFFERS] = std::array::from_fn(|i| {
            shadow_formats
                .filter(|_| i < SHADOW_COLORS)
                .map_or(formats[i], |f| f[i])
        });
        let (mut storage, mut copied, mut image_copied, mut snapshotted) = (0u16, 0u16, 0u64, 0u64);
        let gpu = Gpu::of(adapter, features);
        if !gpu.compute {
            notes.push("this GPU has no compute shaders: compute programs are skipped".into());
        }
        // An image this device can't make or store into: the programs using it are skipped.
        let mut image_defs = std::mem::take(&mut post.images);
        let mut left_out = Vec::new();
        let packed_float = gpu.storable(adapter, wgpu::TextureFormat::Rg11b10Ufloat, features);
        image_defs.retain(|def| {
            let made = pack_images::makeable(device, def, packed_float).and_then(|()| {
                if gpu.storable(
                    adapter,
                    pack_images::format(def.format, packed_float),
                    features,
                ) {
                    Ok(())
                } else {
                    Err("this GPU can't store into its format".into())
                }
            });
            if let Err(e) = &made {
                notes.push(format!("image `{}`: {e}", def.name));
                left_out.push(def.clone());
            }
            made.is_ok()
        });
        // A mask of a pass's images holds this many.
        let kept = image_defs.len().min(pack_images::IMAGES);
        for def in image_defs.drain(kept..) {
            notes.push(format!(
                "image `{}`: past the {} images a pack may have",
                def.name,
                pack_images::IMAGES
            ));
            left_out.push(def);
        }
        let images = pack_images::images(image_defs, packed_float);
        let mut objects = std::mem::take(&mut post.buffer_objects);
        if gpu.storage_stages().is_empty() {
            objects.clear();
        }
        let buffers = pack_images::Buffers::new(device, objects, minimums, gpu.storage_stages());
        let shadow_storable = shadow_formats.map_or(0, |f| {
            (0..SHADOW_COLORS)
                .filter(|&n| gpu.storable(adapter, f[n], features))
                .fold(0u32, |m, n| m | 1 << (BUFFERS + n))
        });
        let storable = (0..BUFFERS)
            .filter(|&i| gpu.storable(adapter, formats[i], features))
            .fold(shadow_storable, |m, i| m | 1 << i);
        let indices = gpu.vertex_storage.then(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Shader pack geometry indices"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            })
        });
        let stand_ins = std::cell::RefCell::new(Vec::new());
        let mod_uniform_mismatches = std::cell::RefCell::new(HashSet::new());
        let pack = Resources {
            textures: &textures,
            images: &images,
            left_out: &left_out,
            buffers: &buffers,
            indices: indices.as_ref(),
            gpu,
            storable,
            rgb,
            packed,
            released: std::cell::Cell::new(("", 0)),
            stand_ins: &stand_ins,
            shadow_linear: (0..2)
                .filter(|&n| !shadow_config.nearest[n])
                .fold(0, |m, n| m | 1 << n),
            jobs: defer.then(Default::default),
            mod_depth_targets: mods.depth_targets,
            mod_uniforms: mods.uniforms,
            display_uniforms: hdr.is_some(),
            mod_uniform_mismatches: &mod_uniform_mismatches,
            mod_depth_target: std::cell::Cell::new(None),
            depth_compare: std::cell::Cell::new(wgpu::CompareFunction::LessEqual),
            mod_vertex: Default::default(),
        };
        // Flat terrain resolves each quad's source state against the current pack; terrain that
        // reads its pages per section binds them by the same layout.
        let block_ids = world
            .terrain
            .as_ref()
            .filter(|g| g.pages)
            .map(|_| {
                block_ids(device, queue, world.block_ids, world.sprite_rects).ok_or_else(|| {
                    Refused {
                        reason: "terrain source tables could not be allocated".into(),
                        notes: notes.clone(),
                    }
                })
            })
            .transpose()?;
        // The next pass built also gets a depth-only pipeline for the boats' water patches, and
        // a polygon-offset one for sign text.
        let water_mask_next = std::cell::Cell::new(false);
        let polygon_offset_next = std::cell::Cell::new(false);
        let mut build = |mut p: sunrise_shaders::PostProgram, geometry: Option<&Geometry<'_>>| {
            // Not wanted any more: the rest is skipped and the build refused below.
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return None;
            }
            let in_shadow = p.group.shadow();
            let mut compute = std::mem::take(&mut p.compute);
            if !gpu.compute {
                compute.clear();
                p.program.as_ref()?;
            }
            let group = p.group;
            let stage = pack_textures::stage(group);
            if pack.released.get().0 != stage {
                pack.released.set((stage, 0));
            }
            let mut computes: Vec<Pass> = compute
                .into_iter()
                .filter_map(|(name, c)| {
                    build_compute(
                        device,
                        name,
                        group,
                        c,
                        if in_shadow { &shadow_space } else { &formats },
                        shadow_formats.as_ref(),
                        &mut notes,
                        &custom_names,
                        &pack,
                    )
                })
                .collect();
            let mut pass = if p.program.is_none() {
                // Compute stages only: the last is the pass, the others run before it.
                let mut last = computes.pop()?;
                last.compute = computes;
                computes = Vec::new();
                last
            } else {
                build_pass(
                    device,
                    p,
                    if in_shadow { &shadow_space } else { &formats },
                    shadow_formats.as_ref(),
                    screen,
                    &mut notes,
                    geometry,
                    &custom_names,
                    &draws.layout,
                    &pack,
                    water_mask_next.get(),
                    polygon_offset_next.get(),
                )?
            };
            // A program whose fragment stage writes nothing keeps its compute stages and its
            // forced flips; with neither, nothing of it is left.
            if let Pipeline::NoDraw { storage } = pass.pipeline {
                if pass.flip == 0 && computes.is_empty() {
                    notes.push(format!("{}: writes no buffer; skipped", pass.name));
                    return None;
                }
                if storage {
                    notes.push(format!(
                        "{}: writes no buffer; its fragment stage, which binds storage buffers, \
                         is left out",
                        pass.name
                    ));
                }
            }
            pass.compute.append(&mut computes);
            // A shadow program with geometry draws into its buffers' "main" side in place, the
            // side `shadowcolorimgN` binds, and wgpu binds a texture one way per pass.
            let drawn_images = shadow_images(&pass.images).0 & pass.written;
            if in_shadow && geometry.is_some() && drawn_images != 0 {
                let n = drawn_images.trailing_zeros();
                notes.push(format!(
                    "{}: draws into shadowcolor{n} and binds it as `shadowcolorimg{n}`; skipped",
                    pass.name
                ));
                return None;
            }
            // A pass's attachments share a size: the depth buffer's, for the passes that draw
            // the world.
            let size =
                |b: usize| sizes[b].unwrap_or(sunrise_shaders::BufferSize::Relative([1.0; 2]));
            let depth = matches!(group, Group::Gbuffers | Group::Translucent);
            let mut written = pass.writes.iter().map(|w| size(w.1));
            let first = if depth {
                Some(sunrise_shaders::BufferSize::Relative([1.0; 2]))
            } else {
                written.next()
            };
            // Sizes of one kind that differ never meet. A size relative to the render size and
            // one in pixels can: the pass then draws while they resolve to one size
            // (`PackPost::sizes_differ`). Iris 1.11.7 drew a composite program writing
            // colortex2 at `0.25 0.25` and colortex3 at `320 180` at 1280×720 (#10454).
            let apart = |s: sunrise_shaders::BufferSize| {
                first.is_some_and(|f| {
                    f != s && (depth || std::mem::discriminant(&f) == std::mem::discriminant(&s))
                })
            };
            if !in_shadow && group != Group::Final && written.any(apart) {
                notes.push(format!(
                    "{}: writes buffers of different sizes (size.buffer); skipped",
                    pass.name
                ));
                return None;
            }
            // Composite-style programs draw into the "alt" side of what they write; gbuffers,
            // shadow and `final` programs write in place, compute programs through images.
            let alt = if geometry.is_none()
                && group != Group::Final
                && matches!(pass.pipeline, Pipeline::Render(_))
            {
                pass.written
            } else {
                0
            };
            if in_shadow {
                shadow_doubled |= alt;
            } else {
                doubled |= alt;
            }
            for p in std::iter::once(&pass).chain(&pass.compute) {
                let (stage, flipped) = pack.released.get();
                pack.released.set((stage, flipped | p.flip));
                if in_shadow {
                    shadow_doubled |= p.flip;
                } else {
                    used |= p.reads | p.written | p.flip;
                    doubled |= p.flip;
                }
                storage |= image_buffers(&p.images);
                let (own, before) = shadow_images(&p.images);
                shadow_storage |= own | before;
                if in_shadow {
                    shadow_pass_images |= own | before;
                    shadow_writes |= p.written | p.flip | own;
                    shadow_buffers |= p.buffers;
                } else {
                    shadow_images_outside |= own | before;
                }
                shadow_copied |= p.shadow_copies;
                image_copied |= p.image_copies;
                snapshotted |= p.snapshots;
                copied |= p.copies;
            }
            Some(pass)
        };
        let terrain_geometry = world.terrain.as_ref().map(|g| Geometry {
            buffers: g.buffers,
            table: g.table,
            depth_write: g.depth_write,
            atlas: g.atlas.clone(),
            material_maps: g.material_maps,
            ids: block_ids.as_ref().map(|i| &i.0),
            cull: g.cull,
            pages: g.pages,
        });
        let split = post.terrain.is_some() && post.terrain_cutout.is_some();
        let (terrain_undrawn, water_undrawn) = (post.terrain_undrawn, post.water_undrawn);
        let mut terrain = post
            .terrain
            .zip(terrain_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)));
        let mut terrain_cutout = post
            .terrain_cutout
            .zip(terrain_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)));
        let water = post
            .water
            .zip(terrain_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)));
        // The shadow pass draws terrain faces from both sides, as Iris 1.11.7 does (#9033): a
        // cave ceiling faces away from the sun, and is often all that shadows the cave.
        let shadow_geometry = terrain_geometry.as_ref().map(|g| Geometry {
            buffers: g.buffers,
            table: g.table,
            depth_write: g.depth_write,
            atlas: g.atlas.clone(),
            material_maps: g.material_maps,
            ids: g.ids,
            cull: false,
            pages: g.pages,
        });
        let shadow_terrain = post
            .shadow
            .zip(shadow_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)));
        // A GPU that refuses translucent terrain's program with the boats' water patches gets it
        // without them (`Post::shadow_water_plain`), rather than no translucent shadows at all.
        let mut plain = post.shadow_water_plain;
        let shadow_water = post
            .shadow_water
            .zip(shadow_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)));
        let unpatched = shadow_water.is_none().then(|| plain.take()).flatten();
        let unpatched_name = unpatched.as_ref().map(|p| p.name.clone());
        let shadow_water = shadow_water.or_else(|| {
            unpatched
                .zip(shadow_geometry.as_ref())
                .and_then(|(p, g)| build(p, Some(g)))
        });
        let water_unpatched = unpatched_name.filter(|_| shadow_water.is_some());
        let (hides_sky, hides_celestial) = (post.sky.is_some(), post.sky_textured.is_some());
        let sky_switches = post.sky_switches;
        let sky = (post.sky.is_some() || post.sky_textured.is_some()).then(|| {
            let geometry = PackSky::new(device);
            let basic = Geometry {
                buffers: &PackSky::buffers(),
                table: Some(&geometry.layout),
                depth_write: false,
                atlas: None,
                material_maps: false,
                ids: None,
                cull: false,
                pages: false,
            };
            let basic = post.sky.and_then(|p| build(p, Some(&basic)));
            let textured = Geometry {
                buffers: &PackSky::celestial_buffers(),
                table: None,
                depth_write: false,
                atlas: world.celestial.clone(),
                material_maps: false,
                ids: None,
                cull: false,
                pages: false,
            };
            let textured = post.sky_textured.and_then(|p| build(p, Some(&textured)));
            Sky {
                geometry,
                basic,
                textured,
            }
        });
        let sky = sky.filter(|s| s.basic.is_some() || s.textured.is_some());
        let programs = [
            post.entities,
            post.block,
            post.entities_translucent,
            post.particles,
            post.damaged_block,
            post.beacon_beam,
            post.block_translucent,
            post.particles_translucent,
        ];
        let mut late = [
            false,
            false,
            post.translucent_late,
            post.particles_late,
            false,
            true,
            post.translucent_late,
            true,
        ];
        let shadow_only = (post.shadow_only_player, post.shadow_only_lit_blocks);
        let shadow_programs = [post.shadow_entities, post.shadow_block];
        let entity_vertices = (programs.iter().chain(&shadow_programs))
            .any(Option::is_some)
            .then(|| {
                let mut entities = PackEntities::new(device, &draws.layout);
                // Native terrain quads of block entities and entities are placed in the block
                // atlas, the one the pack's terrain samples (`pack_entities::BlockAtlas`).
                entities.set_block_atlas(world.atlas.as_ref().and_then(|(_, size)| {
                    crate::pack_entities::BlockAtlas::new(world.sprite_rects, *size)
                }));
                entities
            });
        let entity_buffers = crate::pack_entities::buffers();
        let entity_geometry = entity_vertices.as_ref().map(|e| Geometry {
            buffers: &entity_buffers,
            table: Some(&e.table),
            depth_write: true,
            atlas: world.celestial.clone(),
            material_maps: false,
            ids: None,
            cull: false,
            pages: false,
        });
        // Programs drawing the shared atlas's geometry read its paged maps (`pack_entities::inputs`).
        let paged = |pass: Option<Pass>| {
            pass.map(|mut pass| {
                pass.paged = true;
                pass
            })
        };
        let mut kind = 0;
        let entity_passes = programs.map(|p| {
            // The entity program draws the boats, and their water patches (`Draw::water_mask`).
            water_mask_next.set(kind == crate::pack_entities::ENTITY);
            // The block programs draw sign text, its POLYGON_OFFSET quads biased
            // (`Draw::polygon_offset`).
            polygon_offset_next.set(matches!(
                kind,
                crate::pack_entities::BLOCK | crate::pack_entities::BLOCK_TRANSLUCENT
            ));
            kind += 1;
            p.zip(entity_geometry.as_ref()).and_then(|(p, g)| {
                if p.name == "gbuffers_damagedblock" {
                    let decal = Geometry {
                        buffers: g.buffers,
                        table: g.table,
                        depth_write: false,
                        atlas: g.atlas.clone(),
                        material_maps: g.material_maps,
                        ids: g.ids,
                        cull: false,
                        pages: g.pages,
                    };
                    paged(build(p, Some(&decal)))
                } else {
                    paged(build(p, Some(g)))
                }
            })
        });
        // Translation alone is insufficient: a refused GPU pipeline must retain the
        // legacy single particle pass and its timing as well as its geometry grouping.
        if entity_passes[crate::pack_entities::PARTICLE_TRANSLUCENT].is_some() {
            late[crate::pack_entities::PARTICLE] = false;
        }
        water_mask_next.set(false);
        polygon_offset_next.set(false);
        let shadow_entities = shadow_programs.map(|p| {
            p.zip(entity_geometry.as_ref())
                .and_then(|(p, g)| paged(build(p, Some(g))))
        });
        let entities = entity_vertices
            .filter(|_| (entity_passes.iter().chain(&shadow_entities)).any(Option::is_some))
            .map(|geometry| Entities {
                geometry,
                passes: entity_passes,
                late,
                shadow: shadow_entities,
                only: shadow_only,
            });
        let cloud_geometry = world.clouds.map(|layout| Geometry {
            buffers: &entity_buffers,
            table: Some(layout),
            depth_write: true,
            atlas: world.celestial.clone(),
            material_maps: false,
            ids: None,
            cull: false,
            pages: false,
        });
        let hides_clouds = post.clouds_off || post.clouds.is_some() && cloud_geometry.is_some();
        // Rain and snow test depth but never write it: Iris 1.11.7 left none of either in
        // `depthtex0`, whatever `rain.depth` said (probe packs of ours, #10492).
        let weather_geometry = world.weather.then(|| Geometry {
            buffers: &entity_buffers,
            table: None,
            depth_write: false,
            atlas: None,
            material_maps: false,
            ids: None,
            cull: false,
            pages: false,
        });
        let hand_geometry = world
            .hand
            .then(|| crate::pack_hand::PackHand::new(device, &draws.layout));
        let hand = post.hand.zip(hand_geometry).and_then(|(p, geometry)| {
            let g = Geometry {
                buffers: &entity_buffers,
                table: Some(&geometry.layout),
                depth_write: true,
                atlas: world.celestial.clone(),
                material_maps: false,
                ids: None,
                cull: false,
                pages: false,
            };
            paged(build(p, Some(&g))).map(|pass| (pass, geometry))
        });
        // Translucent held items, drawn from the hand's own buffer.
        let hand_water = post.hand_water.and_then(|p| {
            let (_, geometry) = hand.as_ref()?;
            let g = Geometry {
                buffers: &entity_buffers,
                table: Some(&geometry.layout),
                depth_write: true,
                atlas: world.celestial.clone(),
                material_maps: false,
                ids: None,
                cull: false,
                pages: false,
            };
            paged(build(p, Some(&g)))
        });
        let weather = post
            .weather
            .zip(weather_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)))
            .map(|rain| {
                let mut snow = rain.clone();
                snow.name = format!("{} (snow)", rain.name);
                [rain, snow]
            });
        let clouds = post
            .clouds
            .zip(cloud_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)));
        let line_buffers = [
            Some(crate::pack_lines::layout()),
            Some(crate::pack_lines::over_layout()),
        ];
        let line_geometry = Geometry {
            buffers: &line_buffers,
            table: None,
            depth_write: false,
            atlas: None,
            material_maps: false,
            ids: None,
            cull: false,
            pages: false,
        };
        let lines = post.lines.and_then(|p| build(p, Some(&line_geometry)));
        // Build mod-declared pipeline passes that resolved to a pack program.
        let mut mod_passes = Vec::new();
        if !mods.vertex_buffers.is_empty() {
            for resolved in post.mod_programs {
                let Some(program) = resolved.program else {
                    continue;
                };
                let Some(spec) = mods.pipelines.iter().find(|s| s.slot == resolved.slot) else {
                    continue;
                };
                if spec.pass_slot == ModPassSlot::Shadow && shadow_formats.is_none() {
                    continue;
                }
                let active_depth = spec.depth_target.filter(|&idx| {
                    spec.pass_slot != ModPassSlot::Shadow
                        && pre_mod_depth_sampled.get(idx).copied().unwrap_or(false)
                });
                let mod_set1_layout = mods.bind_group_layouts.get(spec.slot).copied().flatten();
                // The mod's own vertex stage, its layouts and its vertices, or the pack's
                // program over the engine's mod vertex format.
                let own_stage = match spec.program_mode {
                    ModProgramMode::PackVertexAndFragment => None,
                    ModProgramMode::ModVertexPackFragment => {
                        let stage = mods.vertex_stages.get(spec.slot).and_then(Option::as_ref);
                        // A pipeline the GPU refused has no stage: its slot stays unbound.
                        if stage.is_none() {
                            continue;
                        }
                        stage
                    }
                };
                let mut own_buffers: Vec<Option<wgpu::VertexBufferLayout<'_>>> = Vec::new();
                for buffer in own_stage.iter().flat_map(|s| &s.buffers) {
                    let slot = buffer.binding as usize;
                    if own_buffers.len() <= slot {
                        own_buffers.resize(slot + 1, None);
                    }
                    own_buffers[slot] = Some(wgpu::VertexBufferLayout {
                        array_stride: buffer.stride,
                        step_mode: buffer.step_mode,
                        attributes: &buffer.attributes,
                    });
                }
                let mod_geometry = Geometry {
                    buffers: if own_stage.is_some() {
                        &own_buffers[..]
                    } else {
                        mods.vertex_buffers
                    },
                    // The mod's own sets go after the pack's, not into set 1.
                    table: mod_set1_layout.filter(|_| own_stage.is_none()),
                    depth_write: spec.depth_write,
                    atlas: None,
                    material_maps: true,
                    ids: None,
                    cull: spec.cull,
                    pages: false,
                };
                pack.mod_depth_target.set(active_depth);
                pack.depth_compare.set(spec.depth_compare);
                *pack.mod_vertex.borrow_mut() = own_stage.cloned();
                let built_pass = build(program, Some(&mod_geometry));
                pack.mod_vertex.borrow_mut().take();
                pack.mod_depth_target.set(None);
                pack.depth_compare.set(wgpu::CompareFunction::LessEqual);
                if let Some(pass) = built_pass {
                    mod_passes.push(ModPackPass {
                        slot: spec.slot,
                        pass_slot: spec.pass_slot,
                        depth_target: active_depth,
                        has_mod_bind_group: own_stage.map_or_else(
                            || mod_set1_layout.is_some(),
                            |stage| stage.set1_layout.is_some(),
                        ),
                        mod_vertex: own_stage.is_some(),
                        pass,
                    });
                }
            }
        }
        // Developers' aid: `SUNRISE_SHADER_PACK_SKIP=composite,final` leaves programs out.
        let skip = std::env::var("SUNRISE_SHADER_PACK_SKIP").unwrap_or_default();
        let passes: Vec<Pass> = post
            .programs
            .into_iter()
            .filter(|p| !skip.split(',').any(|s| s == p.name))
            .filter_map(|p| build(p, None))
            .collect();
        // The screen gets colortex0 through `final`, or else through Sunrise's copy, which reads
        // floats: an integer colortex0 (`RGBA32UI`...) with no `final` left to draw can't reach
        // it (#9165).
        let shows = passes
            .iter()
            .any(|p| p.group == Group::Final && matches!(p.pipeline, Pipeline::Render(_)));
        if !shows && !holds_floats(formats[0]) {
            return Err(Refused {
                reason: format!(
                    "colortex0 has an integer format ({:?}) and no `final` program draws it to \
                     the screen",
                    formats[0]
                ),
                notes,
            });
        }
        let shadow_comp: Vec<Pass> = post
            .shadow_comp
            .into_iter()
            .filter(|p| !skip.split(',').any(|s| s == p.name))
            .filter_map(|p| build(p, None))
            .collect();
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Refused::cancelled(notes));
        }
        let stand_ins = pack_images::stand_ins(device, stand_ins.take());
        // Terrain in two parts draws both or neither: one alone would leave the other part's
        // terrain undrawn. Without either, the game draws all terrain, as when the terrain
        // program of a pack that doesn't split can't draw.
        if split
            && terrain.is_some() != terrain_cutout.is_some()
            && let Some(p) = terrain.take().or(terrain_cutout.take())
        {
            notes.push(format!(
                "{}: the other part of terrain can't draw; the game draws all terrain",
                p.name
            ));
        }
        match (&terrain, &terrain_cutout) {
            (Some(t), None) => notes.push(format!("terrain draws with {}", t.name)),
            (t, Some(c)) => {
                if let Some(t) = t {
                    notes.push(format!("solid terrain draws with {}", t.name));
                }
                notes.push(format!("cutout terrain draws with {}", c.name));
            }
            (None, None) => {}
        }
        for (part, pass) in [
            ("terrain", &shadow_terrain),
            ("translucent terrain", &shadow_water),
        ] {
            if let Some(p) = pass {
                notes.push(format!("{part} casts shadows with {}", p.name));
            }
        }
        if let Some(w) = &water {
            notes.push(format!("translucent terrain draws with {}", w.name));
        }
        // Timed (and labelled) apart from the solid shadow pass that runs the same program.
        let mut shadow_water = shadow_water;
        if let Some(p) = &mut shadow_water {
            p.name.push_str(" (translucent)");
        }
        for (part, pass) in sky
            .iter()
            .flat_map(|s| [("sky", &s.basic), ("sun and moon", &s.textured)])
        {
            if let Some(p) = pass {
                notes.push(format!("the {part} draw with {}", p.name));
            }
        }
        let parts = [
            "entities",
            "block entities",
            "translucent entities",
            "particles",
        ];
        if let Some(l) = &lines {
            notes.push(format!("the block outline draws with {}", l.name));
        }
        if let Some(c) = &clouds {
            notes.push(format!("clouds draw with {}", c.name));
        }
        if let Some((h, _)) = &hand {
            notes.push(format!("the hand draws with {}", h.name));
        }
        if let Some(h) = &hand_water {
            notes.push(format!("translucent held items draw with {}", h.name));
        }
        if let Some([w, _]) = &weather {
            notes.push(format!("rain and snow draw with {}", w.name));
        }
        for (part, pass) in entities.iter().flat_map(|e| parts.iter().zip(&e.passes)) {
            if let Some(p) = pass {
                notes.push(format!("{part} draw with {}", p.name));
            }
        }
        // Timed (and labelled) apart from the terrain's shadow pass, which may run the same
        // program.
        let mut entities = entities;
        for e in entities.iter_mut() {
            let only = [
                (e.only.0, " (only the player)"),
                (e.only.1, " (only those that emit light)"),
            ];
            for ((part, pass), (only, which)) in parts.iter().zip(&mut e.shadow).zip(only) {
                if let Some(p) = pass {
                    let which = if only { which } else { "" };
                    notes.push(format!("{part}{which} cast shadows with {}", p.name));
                    p.name.push_str(&format!(" ({part})"));
                }
            }
        }
        let reads_depth = |source| {
            passes
                .iter()
                .filter(|p| matches!(p.group, Group::Deferred | Group::Composite | Group::Final))
                .chain(&water)
                .chain(weather.iter().take(1).flatten())
                .chain(
                    mod_passes
                        .iter()
                        .filter(|m| {
                            matches!(
                                m.pass_slot,
                                ModPassSlot::AfterTranslucentGbuffers | ModPassSlot::AfterWeather
                            ) || m.depth_target.is_some()
                        })
                        .map(|m| &m.pass),
                )
                .flat_map(|p| std::iter::once(p).chain(&p.compute))
                .any(|p| p.samplers.iter().any(|s| s.source == source))
        };
        let reads_depth2 = reads_depth(Source::Depth2);
        let mod_writes_post_opaque_depth0 = mod_passes.iter().any(|m| {
            matches!(
                m.pass_slot,
                ModPassSlot::AfterTranslucentGbuffers | ModPassSlot::AfterWeather
            ) && m.depth_target.is_none()
        });
        // Without a shader hand, depth1 also serves depth2: both exclude only translucency.
        let depth1 = (water.is_some() || hand.is_some() || mod_writes_post_opaque_depth0)
            && (reads_depth(Source::Depth1) || (hand.is_none() && reads_depth2));
        let depth2 = hand.is_some() && reads_depth2;
        let water_mask = (water.iter())
            .flat_map(|p| &p.samplers)
            .any(|s| s.source == Source::WaterMask);
        let every_base_pass = || {
            (passes
                .iter()
                .chain(&shadow_comp)
                .chain(&terrain)
                .chain(&terrain_cutout)
                .chain(&water))
            .chain(shadow_terrain.iter().chain(&shadow_water))
            .chain(sky.iter().flat_map(|s| s.basic.iter().chain(&s.textured)))
            .chain(entities.iter().flat_map(|e| e.passes.iter().flatten()))
            .chain(entities.iter().flat_map(|e| e.shadow.iter().flatten()))
            .chain(&clouds)
            .chain(&lines)
            .chain(weather.iter().flatten())
            .chain(hand.iter().map(|h| &h.0))
            .chain(&hand_water)
        };
        // Lazy allocation for mod-declared auxiliary depth targets: allocate a depth target
        // (and its opaque-depth snapshot) only when at least one built pass actually samples it.
        let mut mod_depth_used = vec![false; mods.depth_targets.len()];
        let mut mod_opaque_depth_used = vec![false; mods.depth_targets.len()];
        let mut record_mod_depth_samplers = |p: &Pass| {
            for s in &p.samplers {
                match s.source {
                    Source::ModDepth(idx) => {
                        if let Some(flag) = mod_depth_used.get_mut(idx) {
                            *flag = true;
                        }
                    }
                    Source::ModOpaqueDepth(idx) => {
                        if let Some(flag) = mod_depth_used.get_mut(idx) {
                            *flag = true;
                        }
                        if let Some(flag) = mod_opaque_depth_used.get_mut(idx) {
                            *flag = true;
                        }
                    }
                    _ => {}
                }
            }
        };
        for p in every_base_pass().flat_map(|p| std::iter::once(p).chain(&p.compute)) {
            record_mod_depth_samplers(p);
        }
        for p in mod_passes
            .iter()
            .map(|m| &m.pass)
            .flat_map(|p| std::iter::once(p).chain(&p.compute))
        {
            record_mod_depth_samplers(p);
        }
        for m in &mut mod_passes {
            if let Some(idx) = m.depth_target
                && !mod_depth_used.get(idx).copied().unwrap_or(false)
            {
                if m.pass
                    .samplers
                    .iter()
                    .any(|s| matches!(s.source, Source::Depth | Source::Depth1 | Source::Depth2))
                {
                    mod_depth_used[idx] = true;
                } else {
                    m.depth_target = None;
                }
            }
        }
        let every_pass = || every_base_pass().chain(mod_passes.iter().map(|m| &m.pass));
        let shadow_targets = shadow_formats.map(|f| {
            // `shadow_storage` covers compute stages attached to a pass, which `every_pass`
            // leaves out: one may be the only program binding shadowcolorimgN.
            let colors = (every_pass().fold(0u16, |m, p| {
                let own = if p.group.shadow() {
                    p.reads | p.written | p.flip
                } else {
                    0
                };
                m | own | p.shadow_reads
            }) | shadow_storage)
                & shadow_colors;
            let depth1 = every_pass()
                .flat_map(|p| &p.samplers)
                .any(|s| matches!(s.source, Source::ShadowDepth(1, _)));
            let t = crate::pack_shadow::ShadowTargets::new(
                device,
                &shadow_config,
                f,
                crate::pack_shadow::ShadowUse {
                    colors,
                    doubled: shadow_doubled & colors,
                    storage: shadow_storage & colors,
                    copied: shadow_copied & colors,
                    sink: shadow_comp.iter().any(|p| p.sink),
                },
                depth1,
            );
            notes.push(format!(
                "shadow map {0}x{0} over {2} blocks, sunPathRotation {3}, shadowcolor buffers {colors:#b} (two textures: {4:#b}){1}",
                t.size,
                if depth1 { ", shadowtex1" } else { "" },
                shadow_config.distance,
                shadow_config.sun_path_rotation,
                shadow_doubled & colors,
            ));
            t
        });
        log::info!(
            "shader pack: colortex buffers {used:#b}, with two textures {:#b}",
            doubled & used
        );
        // Any program can leave pixels untouched. Composite-style passes also write the
        // alternate image, so their first write does not replace the main image's clear.
        let mut cleared = Vec::new();
        for b in 1..BUFFERS {
            let bit = 1u16 << b;
            let touched = |p: &&Pass| (p.reads | p.flip | p.written) & bit != 0;
            // Gbuffers passes cover part of their targets: a clear is needed if they touch it.
            let first = sky
                .iter()
                .flat_map(|s| s.basic.iter().chain(&s.textured))
                .chain(&terrain)
                .chain(&terrain_cutout)
                .chain(entities.iter().flat_map(|e| e.passes.iter().flatten()))
                .chain(&clouds)
                .chain(&lines)
                .chain(&water)
                .chain(weather.iter().take(1).flatten())
                .chain(hand.iter().map(|h| &h.0))
                .chain(&hand_water)
                .chain(
                    mod_passes
                        .iter()
                        .filter(|m| m.pass_slot != ModPassSlot::Shadow)
                        .map(|m| &m.pass),
                )
                .find(touched)
                .map(|_| true)
                .or_else(|| passes.iter().find(touched).map(|_| true));
            if clears[b].is_some() && first == Some(true) {
                cleared.push(b);
            }
        }
        // Terrain tests against the world's depth (entities...), so the blit copies it then.
        let draws_depth = terrain.is_some()
            || terrain_cutout.is_some()
            || water.is_some()
            || sky.is_some()
            || entities.as_ref().is_some_and(Entities::drawn)
            || clouds.is_some()
            || lines.is_some()
            || weather.is_some()
            || hand.is_some()
            || mod_passes
                .iter()
                .any(|m| m.pass_slot != ModPassSlot::Shadow && m.depth_target.is_none());
        let blit_depth = draws_depth
            || every_pass()
                .flat_map(|p| std::iter::once(p).chain(&p.compute))
                .any(|p| {
                    p.samplers.iter().any(|s| {
                        matches!(s.source, Source::Depth | Source::Depth1 | Source::Depth2)
                    })
                });
        // Buffers of a size of their own clear apart from the blit, which writes colortex0.
        let (mut cleared, mut other_clears): (Vec<usize>, Vec<usize>) =
            cleared.into_iter().partition(|&b| sizes[b].is_none());
        let limits = device.limits();
        let max_attachments = (limits.max_color_attachments as usize).min(8);
        let mut bytes = attachment_bytes(0, formats[0]);
        let mut attachments = 1;
        cleared.retain(|&b| {
            let next = attachment_bytes(bytes, formats[b]);
            if attachments < max_attachments && next <= limits.max_color_attachment_bytes_per_sample
            {
                attachments += 1;
                bytes = next;
                true
            } else {
                other_clears.push(b);
                false
            }
        });
        let float = wgpu::TextureSampleType::Float { filterable: true };
        // The world's colour is floats: an integer colortex0 (`RGBA32UI`...) gets only what the
        // pack writes, and is cleared in a pass of its own.
        let world_color = holds_floats(formats[0]);
        if !world_color {
            other_clears.push(0);
        }
        let mut world_target = world_color.then(|| target(formats[0])).flatten();
        if rgb & 1 != 0
            && let Some(t) = &mut world_target
        {
            t.write_mask = wgpu::ColorWrites::COLOR;
        }
        let blit_targets = std::iter::once(world_target);
        // Cleared by the pass's load operation; the shader writes nothing there.
        let blit_targets: Vec<_> = blit_targets
            .chain(cleared.iter().map(|&b| {
                Some(wgpu::ColorTargetState {
                    format: formats[b],
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })
            }))
            .collect();
        let blit = (world_color || blit_depth || !cleared.is_empty()).then(|| {
            helper(
                device,
                "vs_full",
                if blit_depth {
                    "fs_blit"
                } else {
                    "fs_blit_color"
                },
                &[
                    texture_entry(0, float),
                    texture_entry(1, wgpu::TextureSampleType::Float { filterable: false }),
                ],
                &blit_targets,
                // GL's window depth, written for depthtex0 and the terrain's depth test.
                blit_depth.then_some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
            )
        });
        // The colortex0 copy's bindings: served Sunrise's HDR it only loads texels, so colortex0
        // needs no filtering (an `RGBA32F` one isn't filterable without `FLOAT32_FILTERABLE`);
        // otherwise `fs_copy` samples it.
        let unfiltered = [texture_entry(
            0,
            wgpu::TextureSampleType::Float { filterable: false },
        )];
        let sampled = [
            texture_entry(0, float),
            entry(
                2,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ),
        ];
        let copy = match hdr {
            // Served Sunrise's HDR, colortex0 holds the linear light `final` would write.
            Some(_) => helper(
                device,
                "vs_uv",
                "fs_copy_linear",
                &unfiltered,
                &[target(screen)],
                None,
            ),
            None => helper(
                device,
                "vs_uv",
                "fs_copy",
                &sampled,
                &[target(screen)],
                None,
            ),
        };
        // Served, a colortex0 copy without a render scale reads each pixel's own texel.
        let copy_texel = hdr.is_some().then(|| {
            helper(
                device,
                "vs_uv",
                "fs_copy_linear_texel",
                &unfiltered,
                &[target(screen)],
                None,
            )
        });
        let upscale = helper(
            device,
            "vs_screen",
            // Served, the picture's alpha is the centre texel's, as for the copy.
            if hdr.is_some() {
                "fs_upscale_linear"
            } else {
                "fs_copy"
            },
            &[
                texture_entry(0, float),
                entry(
                    2,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
            &[target(screen)],
            None,
        );
        let export_depth = helper(
            device,
            "vs_full",
            "fs_export_depth",
            &[texture_entry(
                1,
                wgpu::TextureSampleType::Float { filterable: false },
            )],
            &[],
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
        );
        let filterable = |b: usize| {
            formats[b].sample_type(None, Some(device.features()))
                == Some(wgpu::TextureSampleType::Float { filterable: true })
        };
        let asked = passes.iter().fold(0u16, |m, p| m | p.mipmaps);
        let mipped = (0..BUFFERS)
            .filter(|&b| asked & 1 << b != 0 && filterable(b))
            .fold(0u16, |m, b| m | 1 << b);
        if asked & !mipped != 0 {
            notes.push(format!(
                "mipmaps of unfilterable buffers are not generated (buffers {:#b})",
                asked & !mipped
            ));
        }
        let mut down = HashMap::new();
        for b in (0..BUFFERS).filter(|b| mipped & 1 << b != 0) {
            down.entry(formats[b]).or_insert_with(|| {
                helper(
                    device,
                    "vs_full",
                    "fs_down",
                    &[
                        texture_entry(0, float),
                        entry(
                            2,
                            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        ),
                    ],
                    &[target(formats[b])],
                    None,
                )
            });
        }
        let white = [255u8; 4];
        let black = [0u8; 4];
        let flat_normal = [128u8, 128, 255, 255];
        // Said as what it is, not as a program skipped.
        if let Some(name) = water_unpatched {
            let skipped = format!("{name}: skipped: ");
            if let Some(n) = notes.iter_mut().rev().find(|n| n.starts_with(&skipped)) {
                *n = format!(
                    "{name}: translucent terrain without the boats' water patches, as the GPU \
                     refused it with them ({})",
                    &n[skipped.len()..]
                );
            }
        }
        let jobs = (pack.jobs.as_ref())
            .map(std::cell::RefCell::take)
            .unwrap_or_default();
        let bound_mod_slots = mod_passes.iter().map(|m| m.slot).collect();
        let built = PackPost {
            notes,
            setup_pending: true,
            passes,
            terrain,
            terrain_cutout,
            water,
            terrain_undrawn,
            water_undrawn,
            sky,
            entities,
            clouds,
            cloud_draw: None,
            lines,
            line_draw: None,
            hides_clouds,
            hides_sky,
            hides_celestial,
            sky_switches,
            sky_clear,
            weather,
            weather_draw: None,
            weather_generation: 0,
            hand,
            hand_water,
            depth1,
            depth2,
            water_mask,
            water_mask_dirty: true,
            shadow_targets,
            shadow_terrain,
            portal_rect: [0.0; 4],
            portal_texture: None,
            shadow_water,
            shadow_comp,
            mipped,
            mips_valid: 0,
            down,
            draws,
            block_ids,
            page_groups: HashMap::new(),
            gui_size: world.gui_size,
            timing: None,
            center_depth: None,
            custom,
            atlas: world.atlas.as_ref().map(|a| a.0.clone()),
            material_maps: world.material_maps,
            paged_maps: [None, None],
            lightmap: world.lightmap,
            deferred_pre: mask(&post.deferred_pre),
            composite_pre: mask(&post.composite_pre),
            formats,
            clears,
            rgb,
            rgb_unset: true,
            used,
            doubled: doubled & used,
            storage,
            copied,
            sizes,
            draws_depth,
            blit_depth,
            blit_clears: cleared,
            other_clears,
            blit_group: None,
            main: 0,
            targets: None,
            noise: pack_textures::noise(&textures)
                .cloned()
                .unwrap_or_else(|| noise(device, queue, noise_size)),
            dummy: texture(device, queue, "Shader pack stand-in", 1, &white),
            dummy_black: texture(device, queue, "Shader pack black stand-in", 1, &black),
            dummy_normal: texture(
                device,
                queue,
                "Shader pack normal stand-in",
                1,
                &flat_normal,
            ),
            images: {
                let mut images = images;
                for (k, image) in images.iter_mut().enumerate() {
                    image.copied = image_copied & 1 << k != 0;
                    image.snapshotted = snapshotted & 1 << k != 0;
                }
                images
            },
            buffers,
            indices,
            shadow_culling: post.shadow_culling,
            gpu,
            zeros: None,
            stand_ins,
            dump: pack_images::Dump::new(),
            dummy_3d: device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Shader pack 3D stand-in"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default()),
            dummy_1d: device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Shader pack 1D stand-in"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D1,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default()),
            textures,
            shadow: (
                crate::pack_shadow::stand_in(device),
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("Shader pack shadow"),
                    compare: Some(wgpu::CompareFunction::LessEqual),
                    ..Default::default()
                }),
            ),
            shadow_cleared: false,
            linear: sampler(
                device,
                wgpu::FilterMode::Linear,
                wgpu::AddressMode::ClampToEdge,
            ),
            nearest: sampler(
                device,
                wgpu::FilterMode::Nearest,
                wgpu::AddressMode::ClampToEdge,
            ),
            repeat: sampler(device, wgpu::FilterMode::Linear, wgpu::AddressMode::Repeat),
            nearest_repeat: sampler(device, wgpu::FilterMode::Nearest, wgpu::AddressMode::Repeat),
            atlas_sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Shader pack atlas"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            }),
            buffer_sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Shader pack buffers"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            }),
            blit,
            copy,
            copy_texel,
            hdr,
            upscale,
            export_depth,
            export_depth_group: None,
            final_target: None,
            screen,
            uniforms: {
                let mut u = FrameUniforms::default();
                u.atlas = world.atlas.map_or([0; 2], |a| a.1);
                u.shadow = shadow_config;
                u.half_lives = half_lives;
                u
            },
            shadow_cache: Default::default(),
            shadow_images_outside,
            shadow_pass_images,
            shadow_writes,
            shadow_buffers,
            mod_passes,
            bound_mod_slots,
            mod_draws: Vec::new(),
            mod_depth_decls: mods.depth_targets.to_vec(),
            mod_depth_used,
            mod_opaque_depth_used,
            mod_uniforms: ModUniformTable::new(mods.uniforms),
        };
        Ok((built, jobs))
    }
}

impl PackPost {
    /// The shared atlas's normal and specular paged maps (`paged_maps.rs`), which the programs
    /// drawing its geometry read as `normals` and `specular`; set before the first frame (a pass's
    /// bind groups are kept once made). A missing one reads as the dummy (flat, no specular).
    pub fn set_paged_maps(&mut self, maps: [Option<wgpu::TextureView>; 2]) {
        self.paged_maps = maps;
    }
}
