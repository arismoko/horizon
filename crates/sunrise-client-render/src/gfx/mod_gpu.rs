//! The GPU state mod handles resolve against, shareable apart from the [`Renderer`].
//!
//! `Game::render` takes the renderer out of the game while it draws, and the mod hooks that fire
//! during the draw still create, write, resize and resolve meshes, uniform buffers, storage
//! buffers and render targets. They go through a clone of [`ModGpuResources`], whose tables are
//! shared with the renderer that made it.

use super::mod_meshes::{ModMeshes, ResolvedMeshDraw};
use super::mod_render_targets::{
    ModRenderTargets, ResolvedOffscreenTarget, TargetMips, probe_render_capabilities,
};
use super::mod_storage::{ModStorageBuffers, ResolvedStorageRange};
use super::mod_uniforms::{
    ATLAS_VIEW_ID, DYNAMIC_VIEW_ID_BIT, LIGHTMAP_VIEW_ID, ModUniformBuffers, WORLD_DEPTH_VIEW_ID,
};
use mod_api::{
    BufferError, Identifier, MeshDescriptor, MeshDraw, MeshError, MeshRef, PipelineDrawError,
    PrimitiveTopology, RenderCapabilities, RenderTargetDescriptor, RenderTargetError,
    RenderTargetRef, StorageBufferDescriptor, StorageBufferRef, StorageBufferSlice,
    StorageImageAccess, StorageImageBinding, UniformBufferRef, VertexFormat,
};
use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, Mutex};

/// The renderer's per-frame values a mod hook can read while the renderer is out of the game:
/// the texture views a mod pipeline can bind, the entity batch capacity and whether a `shaders/`
/// pack is loaded. Refreshed when the renderer draws a frame or resizes.
#[derive(Clone)]
pub(crate) struct ModFrameState {
    pub(crate) entity_atlas: wgpu::TextureView,
    pub(crate) lightmap: wgpu::TextureView,
    pub(crate) world_depth: wgpu::TextureView,
    pub(crate) entity_quad_capacity: super::EntityQuadCapacity,
    pub(crate) pack_iris: bool,
}

/// Handles on the device and the mod tables of one [`Renderer`](super::Renderer); cloning shares
/// them.
///
/// Frame semantics, for mod hooks that run inside `Renderer::render`:
/// - No table lock is held while a `FrameHooks` method runs.
/// - Releasing or resizing removes the entry and drops this table's handle; nothing calls
///   `wgpu::Buffer::destroy` or `wgpu::Texture::destroy`, so wgpu's reference counting keeps
///   what the frame already recorded alive until its submission completes. A resize makes a new
///   resource: draws already recorded keep the old one and later draws use the new one.
/// - Mesh and uniform writes call `Queue::write_buffer`, which lands before the frame's command
///   buffers are submitted: a write from a frame hook applies to the whole frame and the last
///   write wins. Storage buffer writes are queued in call order and applied by
///   `Renderer::render` right after `prepare_render_buffers`; one made later in the frame is
///   applied before the next frame's draws.
#[derive(Clone)]
pub struct ModGpuResources {
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    /// Distinct frame-graph cycle pass-label sets already logged once since last reload.
    pub(crate) warned_frame_graph_cycles: Arc<Mutex<HashSet<Vec<String>>>>,
    render_capabilities: RenderCapabilities,
    max_buffer_size: u64,
    /// Retained GPU vertex/index buffers backing [`mod_api::MeshRef`].
    pub(crate) meshes: Arc<Mutex<ModMeshes>>,
    /// Retained GPU uniform buffers backing [`mod_api::UniformBufferRef`] and cached mod textures.
    pub(crate) uniforms: Arc<Mutex<ModUniformBuffers>>,
    /// Retained GPU storage buffers backing [`mod_api::StorageBufferRef`]; they outlive a resource
    /// reload and a world close.
    pub(crate) storage: Arc<Mutex<ModStorageBuffers>>,
    /// Mod-owned offscreen GPU render targets backing [`mod_api::RenderTargetRef`].
    pub(crate) render_targets: Arc<Mutex<ModRenderTargets>>,
    /// The mods' dynamic textures, shared with the renderer's HUD.
    dynamic_textures: crate::hud::dynamic_textures::Store,
    frame_views: Arc<Mutex<ModFrameState>>,
}

impl ModGpuResources {
    pub(crate) fn new(
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        dynamic_textures: crate::hud::dynamic_textures::Store,
        frame_views: ModFrameState,
    ) -> Self {
        Self {
            render_capabilities: probe_render_capabilities(&adapter, &device),
            max_buffer_size: device.limits().max_buffer_size,
            adapter,
            device,
            queue,
            warned_frame_graph_cycles: Arc::default(),
            meshes: Arc::default(),
            uniforms: Arc::default(),
            storage: Arc::default(),
            render_targets: Arc::default(),
            dynamic_textures,
            frame_views: Arc::new(Mutex::new(frame_views)),
        }
    }

    pub(crate) fn set_frame_views(&self, views: ModFrameState) {
        *self.frame_views.lock().unwrap_or_else(|e| e.into_inner()) = views;
    }

    /// The block/item atlas view for `TextureSource::AtlasSprite`.
    pub fn mod_atlas_texture_view(&self) -> (u64, wgpu::TextureView) {
        let views = self.frame_views.lock().unwrap_or_else(|e| e.into_inner());
        (ATLAS_VIEW_ID, views.entity_atlas.clone())
    }

    /// The active lightmap view for `TextureSource::Lightmap`.
    pub fn mod_lightmap_texture_view(&self) -> (u64, wgpu::TextureView) {
        let views = self.frame_views.lock().unwrap_or_else(|e| e.into_inner());
        (LIGHTMAP_VIEW_ID, views.lightmap.clone())
    }

    /// The active world depth view for `TextureSource::WorldDepth`.
    pub fn mod_world_depth_texture_view(&self) -> (u64, wgpu::TextureView) {
        let views = self.frame_views.lock().unwrap_or_else(|e| e.into_inner());
        (WORLD_DEPTH_VIEW_ID, views.world_depth.clone())
    }

    /// A mod's dynamic texture by its handle, with the revision that keys its bind group.
    pub fn dynamic_texture_view(&self, handle: u64) -> Option<(u64, wgpu::TextureView)> {
        let (view, revision) = self.dynamic_textures.view(handle)?;
        Some((DYNAMIC_VIEW_ID_BIT | revision, view))
    }

    /// The combined entity batch capacity of the renderer's last frame (it depends on whether GPU
    /// entity skinning is on).
    pub fn entity_quad_capacity(&self) -> super::EntityQuadCapacity {
        self.frame_views
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entity_quad_capacity
    }

    /// Whether a `shaders/` pack (not one of Sunrise's native ones) was loaded at the last frame.
    pub fn pack_iris(&self) -> bool {
        self.frame_views
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pack_iris
    }

    /// `device.limits().max_buffer_size`, the bound on every mod buffer.
    pub fn max_buffer_size(&self) -> u64 {
        self.max_buffer_size
    }

    /// The optional device features mods can ask about, read once when the renderer was made.
    pub fn render_capabilities(&self) -> RenderCapabilities {
        self.render_capabilities.clone()
    }

    /// The largest inline uniform and texel-buffer payloads a mod's set-1 draw may bind: the
    /// device's `max_uniform_buffer_binding_size` and `max_storage_buffer_binding_size`, each
    /// capped by `max_buffer_size`.
    pub fn mod_binding_limits(&self) -> (u64, u64) {
        let limits = self.device.limits();
        (
            limits
                .max_uniform_buffer_binding_size
                .min(limits.max_buffer_size),
            limits
                .max_storage_buffer_binding_size
                .min(limits.max_buffer_size),
        )
    }
}

/// Renderer methods that forward to its [`ModGpuResources`].
macro_rules! forward {
    ($($vis:vis fn $name:ident(&self $(, $arg:ident: $ty:ty)*) $(-> $ret:ty)?;)*) => {
        impl super::Renderer {
            $(
                $vis fn $name(&self $(, $arg: $ty)*) $(-> $ret)? {
                    self.mod_gpu.$name($($arg),*)
                }
            )*
        }
    };
}

forward! {
    pub fn create_mesh(&self, owner: Option<&'static str>, desc: &MeshDescriptor<'_>) -> Result<MeshRef, MeshError>;
    pub fn mesh_write_vertices(&self, owner: Option<&'static str>, mesh: MeshRef, binding: u8, byte_offset: u64, data: &[u8]) -> Result<(), MeshError>;
    pub fn mesh_write_indices(&self, owner: Option<&'static str>, mesh: MeshRef, byte_offset: u64, data: &[u8]) -> Result<(), MeshError>;
    pub fn mesh_replace(&self, owner: Option<&'static str>, mesh: MeshRef, desc: &MeshDescriptor<'_>) -> Result<(), MeshError>;
    pub fn mesh_release(&self, owner: Option<&'static str>, mesh: MeshRef) -> Result<(), MeshError>;
    pub fn clear_mod_meshes(&self);
    pub fn retire_disabled_mod_meshes(&self, disabled: &BTreeSet<String>);
    pub fn resolve_mod_mesh_indirect(&self, owner: Option<&'static str>, mesh: MeshRef, bindings: &[(u8, VertexFormat)]) -> Result<ResolvedMeshDraw, PipelineDrawError>;
    pub fn resolve_mod_mesh_draw(&self, owner: Option<&'static str>, mesh: MeshRef, bindings: &[(u8, VertexFormat)], topology: PrimitiveTopology, draw: &MeshDraw) -> Result<ResolvedMeshDraw, PipelineDrawError>;

    pub fn create_uniform_buffer(&self, owner: Option<&'static str>, bytes: &[u8]) -> Result<UniformBufferRef, BufferError>;
    pub fn uniform_buffer_write(&self, owner: Option<&'static str>, buffer: UniformBufferRef, byte_offset: u32, bytes: &[u8]) -> Result<(), BufferError>;
    pub fn uniform_buffer_release(&self, owner: Option<&'static str>, buffer: UniformBufferRef) -> Result<(), BufferError>;
    pub fn clear_mod_uniforms(&self);
    pub fn retire_disabled_mod_uniforms(&self, disabled: &BTreeSet<String>);
    pub fn resolve_mod_uniform_buffer(&self, owner: Option<&'static str>, handle: UniformBufferRef, min_size: u32) -> Result<(u64, wgpu::Buffer, u64), PipelineDrawError>;
    pub fn resolve_mod_standalone_texture(&self, id: &Identifier, read_png: impl FnOnce() -> Option<Vec<u8>>) -> Result<(u64, wgpu::TextureView), PipelineDrawError>;

    pub fn create_storage_buffer(&self, owner: Option<&'static str>, desc: &StorageBufferDescriptor) -> Result<StorageBufferRef, BufferError>;
    pub fn storage_buffer_write_range(&self, owner: Option<&'static str>, buffer: StorageBufferRef, offset: u64, data: &[u8]) -> Result<(), BufferError>;
    pub fn storage_buffer_size(&self, owner: Option<&'static str>, buffer: StorageBufferRef) -> Result<u64, BufferError>;
    pub fn storage_buffer_release(&self, owner: Option<&'static str>, buffer: StorageBufferRef) -> Result<(), BufferError>;
    pub fn retire_unloaded_mod_storage(&self, unloaded: &BTreeSet<String>);
    pub fn resolve_mod_storage_range(&self, owner: Option<&'static str>, slice: StorageBufferSlice, min_size: u32) -> Result<ResolvedStorageRange, BufferError>;
    pub fn resolve_mod_storage_buffer(&self, owner: Option<&'static str>, handle: StorageBufferRef) -> Result<wgpu::Buffer, BufferError>;
    pub(super) fn flush_mod_storage(&self);
    pub(super) fn take_released_mod_storage(&self) -> Vec<u64>;

    pub fn render_capabilities(&self) -> RenderCapabilities;
    pub fn max_texture_dimension_2d(&self) -> u32;
    pub fn max_compute_workgroups_per_dimension(&self) -> u32;
    pub fn float32_filterable(&self) -> bool;
    pub fn warn_frame_graph_cycle_once(&self, passes: &[String]);
    pub fn create_render_target(&self, owner: Option<&'static str>, desc: &RenderTargetDescriptor) -> Result<RenderTargetRef, RenderTargetError>;
    pub fn render_target_resize(&self, owner: Option<&'static str>, target: RenderTargetRef, size: [u32; 2]) -> Result<(), RenderTargetError>;
    pub fn render_target_size(&self, owner: Option<&'static str>, target: RenderTargetRef) -> Result<[u32; 2], RenderTargetError>;
    pub fn render_target_release(&self, owner: Option<&'static str>, target: RenderTargetRef) -> Result<(), RenderTargetError>;
    pub fn retire_unloaded_mod_render_targets(&self, unloaded: &BTreeSet<String>);
    pub fn resolve_mod_render_target_color(&self, owner: Option<&'static str>, target: RenderTargetRef, attachment: u8, filterable: bool) -> Result<(u64, wgpu::TextureView, TargetMips), PipelineDrawError>;
    pub fn resolve_mod_render_target_color_mips(&self, owner: Option<&'static str>, view: mod_api::RenderTargetMipView, filterable: bool) -> Result<(u64, wgpu::TextureView, TargetMips), PipelineDrawError>;
    pub fn resolve_mod_storage_image(&self, owner: Option<&'static str>, access: StorageImageAccess, image: StorageImageBinding) -> Result<(u64, wgpu::TextureView, wgpu::TextureFormat, TargetMips), PipelineDrawError>;
    pub fn resolve_mod_render_target_depth(&self, owner: Option<&'static str>, target: RenderTargetRef, filterable: bool) -> Result<(u64, wgpu::TextureView), PipelineDrawError>;
    pub fn resolve_mod_render_target(&self, owner: Option<&'static str>, target: RenderTargetRef, mip_level: u32) -> Result<ResolvedOffscreenTarget, RenderTargetError>;
    pub(super) fn take_released_mod_render_targets(&self) -> Vec<u64>;
}

impl super::Renderer {
    /// Points the mod handles at the views this renderer draws with now.
    pub(super) fn refresh_mod_frame_views(&self) {
        self.mod_gpu.set_frame_views(ModFrameState {
            entity_atlas: self.entity_atlas.clone(),
            lightmap: self.lightmap_view.clone(),
            world_depth: self.depth.clone(),
            entity_quad_capacity: self.entity_quad_capacity(),
            pack_iris: self.shader_pack_kind() == Some("iris"),
        });
    }

    /// The shareable handle on this renderer's device and mod tables.
    pub fn mod_gpu(&self) -> &ModGpuResources {
        &self.mod_gpu
    }
}
