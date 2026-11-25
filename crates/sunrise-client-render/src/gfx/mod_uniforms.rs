//! Retained GPU uniform buffers backing [`mod_api::UniformBufferRef`], plus cached standalone
//! GPU textures backing [`mod_api::TextureSource::StandaloneTexture`].
//!
//! Uniform buffer sizes are bounded strictly by `device.limits().max_uniform_buffer_binding_size`
//! and `device.limits().max_buffer_size` (zero uncited per-mod cap). Handles are generational,
//! owner-checked, and retired on mod disable, world close, resource reload, or explicit
//! [`mod_api::UniformBufferRef::release`].

use mod_api::__private::game::Fields;
use mod_api::{BufferError, Identifier, PipelineDrawError, UniformBufferRef};
use std::collections::{BTreeMap, BTreeSet};

/// Reserved view IDs for engine-owned textures (never issued to cached standalone views).
pub const ATLAS_VIEW_ID: u64 = 1;
pub const LIGHTMAP_VIEW_ID: u64 = 2;
pub const WORLD_DEPTH_VIEW_ID: u64 = 3;
const FIRST_CUSTOM_VIEW_ID: u64 = 16;
/// Set on the view ID of a dynamic texture, whose ID is its GPU texture's revision from the HUD's
/// own counter: that keeps it apart from the IDs above, which a bind group's cache key shares.
pub const DYNAMIC_VIEW_ID_BIT: u64 = 1 << 63;

struct ModUniformBuffer {
    generation: u64,
    owner: Option<&'static str>,
    buffer: wgpu::Buffer,
    byte_len: u64,
}

struct CachedStandaloneTexture {
    view_id: u64,
    view: wgpu::TextureView,
}

/// Generational store of retained GPU [`UniformBufferRef`] buffers and cached mod pipeline
/// textures owned by `Renderer`.
pub struct ModUniformBuffers {
    next_generation: u64,
    slots: Vec<Option<ModUniformBuffer>>,
    next_view_id: u64,
    standalone_textures: BTreeMap<Identifier, Option<CachedStandaloneTexture>>,
}

impl Default for ModUniformBuffers {
    fn default() -> Self {
        Self {
            next_generation: 0,
            slots: Vec::new(),
            next_view_id: FIRST_CUSTOM_VIEW_ID,
            standalone_textures: BTreeMap::new(),
        }
    }
}

impl ModUniformBuffers {
    /// Allocate a retained GPU uniform buffer initialized with `bytes`.
    pub fn create_uniform_buffer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        owner: Option<&'static str>,
        bytes: &[u8],
    ) -> Result<UniformBufferRef, BufferError> {
        if bytes.is_empty() {
            return Err(BufferError::Empty);
        }
        if !bytes.len().is_multiple_of(4) {
            return Err(BufferError::InvalidAlignment);
        }
        let byte_len = bytes.len() as u64;
        let limit = device
            .limits()
            .max_uniform_buffer_binding_size
            .min(device.limits().max_buffer_size);
        if byte_len > limit {
            return Err(BufferError::ExceedsDeviceLimit);
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mod uniform buffer"),
            size: byte_len,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytes);
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("retained uniform buffer generation exhausted");
        let generation = self.next_generation;
        let entry = ModUniformBuffer {
            generation,
            owner,
            buffer,
            byte_len,
        };
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.slots.len());
        if slot == self.slots.len() {
            self.slots.push(Some(entry));
        } else {
            self.slots[slot] = Some(entry);
        }
        Ok(mod_api::__private::built::uniform_buffer_ref(
            generation,
            slot as u32,
        ))
    }

    /// Partially or fully update retained uniform buffer `handle`.
    pub fn write(
        &self,
        queue: &wgpu::Queue,
        owner: Option<&'static str>,
        handle: UniformBufferRef,
        byte_offset: u32,
        bytes: &[u8],
    ) -> Result<(), BufferError> {
        let buf = self.get_owned(owner, handle)?;
        if !byte_offset.is_multiple_of(4) || !bytes.len().is_multiple_of(4) {
            return Err(BufferError::InvalidAlignment);
        }
        let end = u64::from(byte_offset)
            .checked_add(bytes.len() as u64)
            .ok_or(BufferError::OutOfRange)?;
        if end > buf.byte_len {
            return Err(BufferError::OutOfRange);
        }
        if !bytes.is_empty() {
            queue.write_buffer(&buf.buffer, u64::from(byte_offset), bytes);
        }
        Ok(())
    }

    /// Free `handle`'s GPU buffer and retire it (`BufferError::Stale` thereafter).
    pub fn release(
        &mut self,
        owner: Option<&'static str>,
        handle: UniformBufferRef,
    ) -> Result<(), BufferError> {
        self.get_owned(owner, handle)?;
        let slot = handle.fields().slot as usize;
        self.slots[slot] = None;
        Ok(())
    }

    /// Retire all retained uniform buffers and clear cached standalone textures (called on
    /// world close and resource reload).
    pub fn clear(&mut self) {
        self.slots.clear();
        self.standalone_textures.clear();
    }

    /// Retire all retained uniform buffers owned by any mod in `disabled`.
    pub fn retire_disabled(&mut self, disabled: &BTreeSet<String>) {
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|buf| buf.owner.is_some_and(|id| disabled.contains(id)))
            {
                *slot = None;
            }
        }
    }

    /// Resolve a retained [`UniformBufferRef`] for a draw call, validating ownership, liveness,
    /// and `min_size`. Returns `(unique_id, buffer, byte_len)`.
    pub fn resolve(
        &self,
        owner: Option<&'static str>,
        handle: UniformBufferRef,
        min_size: u32,
    ) -> Result<(u64, wgpu::Buffer, u64), PipelineDrawError> {
        let buf = self
            .get_owned(owner, handle)
            .map_err(PipelineDrawError::Buffer)?;
        if buf.byte_len < u64::from(min_size) {
            return Err(PipelineDrawError::InvalidBinding);
        }
        Ok((buf.generation, buf.buffer.clone(), buf.byte_len))
    }

    /// Resolve or lazily decode+upload a standalone PNG texture asset (`id`), using `read_png`
    /// to fetch raw PNG bytes from the active resource stack on a cache miss.
    pub fn resolve_standalone_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: &Identifier,
        read_png: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Result<(u64, wgpu::TextureView), PipelineDrawError> {
        if let Some(cached) = self.standalone_textures.get(id) {
            return cached
                .as_ref()
                .map(|c| (c.view_id, c.view.clone()))
                .ok_or(PipelineDrawError::InvalidBinding);
        }
        let Some(png_bytes) = read_png() else {
            self.standalone_textures.insert(id.clone(), None);
            return Err(PipelineDrawError::InvalidBinding);
        };
        let Ok((w, h, rgba)) = crate::sprites::decode_rgba(&png_bytes) else {
            self.standalone_textures.insert(id.clone(), None);
            return Err(PipelineDrawError::InvalidBinding);
        };
        let max_dim = device.limits().max_texture_dimension_2d;
        if w == 0 || h == 0 || w > max_dim || h > max_dim {
            self.standalone_textures.insert(id.clone(), None);
            return Err(PipelineDrawError::InvalidBinding);
        }
        let (_texture, view) =
            create_rgba_texture(device, queue, "Mod standalone texture", [w, h], &rgba);
        let view_id = self.alloc_view_id();
        self.standalone_textures.insert(
            id.clone(),
            Some(CachedStandaloneTexture {
                view_id,
                view: view.clone(),
            }),
        );
        Ok((view_id, view))
    }

    fn alloc_view_id(&mut self) -> u64 {
        let id = self.next_view_id;
        self.next_view_id = self
            .next_view_id
            .checked_add(1)
            .expect("mod texture view id exhausted");
        id
    }

    fn get_owned(
        &self,
        owner: Option<&'static str>,
        handle: UniformBufferRef,
    ) -> Result<&ModUniformBuffer, BufferError> {
        let fields = handle.fields();
        let Some(Some(buf)) = self.slots.get(fields.slot as usize) else {
            return Err(BufferError::Stale);
        };
        if buf.generation != fields.generation {
            return Err(BufferError::Stale);
        }
        if buf.owner != owner {
            return Err(BufferError::NotOwned);
        }
        Ok(buf)
    }
}

pub(crate) fn create_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    size: [u32; 2],
    rgba: &[u8],
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    upload_rgba_texture(queue, &texture, size, rgba);
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn upload_rgba_texture(queue: &wgpu::Queue, texture: &wgpu::Texture, size: [u32; 2], rgba: &[u8]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * 4),
            rows_per_image: Some(size[1]),
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
}

impl super::mod_gpu::ModGpuResources {
    /// Allocate a retained GPU [`UniformBufferRef`] on this renderer's device.
    pub fn create_uniform_buffer(
        &self,
        owner: Option<&'static str>,
        bytes: &[u8],
    ) -> Result<UniformBufferRef, BufferError> {
        self.uniforms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .create_uniform_buffer(&self.device, &self.queue, owner, bytes)
    }

    /// Update a retained [`UniformBufferRef`] starting at `byte_offset`.
    pub fn uniform_buffer_write(
        &self,
        owner: Option<&'static str>,
        buffer: UniformBufferRef,
        byte_offset: u32,
        bytes: &[u8],
    ) -> Result<(), BufferError> {
        self.uniforms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write(&self.queue, owner, buffer, byte_offset, bytes)
    }

    /// Free a retained [`UniformBufferRef`] and retire its handle.
    pub fn uniform_buffer_release(
        &self,
        owner: Option<&'static str>,
        buffer: UniformBufferRef,
    ) -> Result<(), BufferError> {
        self.uniforms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .release(owner, buffer)
    }

    /// Retire all retained uniform buffers and cached standalone textures (called on world
    /// close and resource reload).
    pub fn clear_mod_uniforms(&self) {
        self.uniforms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Retire all retained uniform buffers owned by any mod in `disabled`.
    pub fn retire_disabled_mod_uniforms(&self, disabled: &BTreeSet<String>) {
        self.uniforms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retire_disabled(disabled);
    }

    /// Resolve a retained [`UniformBufferRef`] for a draw call.
    pub fn resolve_mod_uniform_buffer(
        &self,
        owner: Option<&'static str>,
        handle: UniformBufferRef,
        min_size: u32,
    ) -> Result<(u64, wgpu::Buffer, u64), PipelineDrawError> {
        self.uniforms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve(owner, handle, min_size)
    }

    /// Resolve or lazily decode+upload a standalone PNG texture asset.
    pub fn resolve_mod_standalone_texture(
        &self,
        id: &Identifier,
        read_png: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Result<(u64, wgpu::TextureView), PipelineDrawError> {
        self.uniforms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_standalone_texture(&self.device, &self.queue, id, read_png)
    }
}
