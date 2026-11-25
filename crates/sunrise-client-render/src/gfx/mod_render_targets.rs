//! Mod-owned offscreen GPU render targets backing [`mod_api::RenderTargetRef`].
//!
//! Target dimensions are bounded strictly by `device.limits().max_texture_dimension_2d` and
//! colour attachment counts by [`mod_api::MAX_COLOR_TARGETS`] (zero uncited per-mod cap).
//! Targets retain their contents across frames until cleared by an
//! [`mod_api::OffscreenPass`] or reallocated by [`mod_api::RenderTargetRef::resize`]. Calling `resize` with the current dimensions is a no-op
//! that preserves existing pixels.
//!
//! Colour attachments can carry a mip chain (`mip_levels`, vanilla's `GpuTexture.getMipLevels`)
//! and storage usage; each mip level has its own single-level view (`GpuTextureView` with
//! `baseMipLevel` and `mipLevels`) for rendering into, sampling and storage binding.
//!
//! Handles are generational and owner-checked. A target survives a resource reload and a world
//! close: it is freed by [`mod_api::RenderTargetRef::release`] or when its mod unloads.

use mod_api::__private::game::Fields;
use mod_api::{
    OffscreenPass, PipelineDrawError, RenderCapabilities, RenderTargetDepth,
    RenderTargetDescriptor, RenderTargetError, RenderTargetFormat, RenderTargetRef,
    StorageImageAccess, StorageImageBinding,
};
use std::collections::{BTreeMap, BTreeSet};

/// How a draw binding uses a mip range of a render target colour attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetUse {
    /// Sampled through a texture view.
    Sampled,
    /// A read-only storage image.
    StorageReadOnly,
    /// A write-only storage image.
    StorageWriteOnly,
    /// A read-write storage image.
    StorageReadWrite,
}

impl TargetUse {
    /// This use as one bit, for a set of the uses of one level.
    pub const fn bit(self) -> u8 {
        1 << self as u8
    }

    /// Whether this use writes the level.
    pub const fn writes(self) -> bool {
        matches!(self, Self::StorageWriteOnly | Self::StorageReadWrite)
    }

    /// Whether the uses in `bits` (see [`Self::bit`]) cannot share one render pass: wgpu gives
    /// each texture subresource one usage per pass, and a write usage combines with no other
    /// (read-only storage and sampling are both read-only usages and combine).
    pub const fn conflict(bits: u8) -> bool {
        let writes = Self::StorageWriteOnly.bit() | Self::StorageReadWrite.bit();
        bits & writes != 0 && !bits.is_power_of_two()
    }
}

/// The mip range of one colour attachment that a draw's binding uses, for the check that no
/// render pass gives a level two usages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetMips {
    /// Identifies the attachment's texture: unique per allocation, so a resize makes a new one.
    pub texture: u64,
    /// First mip level of the range.
    pub base_mip: u32,
    /// Number of mip levels from `base_mip`.
    pub mip_levels: u32,
    /// How the binding uses the range.
    pub usage: TargetUse,
}

impl TargetMips {
    /// The levels of the range.
    pub fn levels(&self) -> std::ops::Range<u32> {
        self.base_mip..self.base_mip + self.mip_levels
    }

    /// Whether `self` and `other` use a shared level in ways one render pass cannot combine.
    pub fn conflicts_with(&self, other: &Self) -> bool {
        self.texture == other.texture
            && self.levels().start < other.levels().end
            && other.levels().start < self.levels().end
            && TargetUse::conflict(self.usage.bit() | other.usage.bit())
    }
}

/// Starting view ID for mod render target views, disjoint from [`super::mod_uniforms`] view IDs
/// so `ModPipelines`' set-1 bind-group cache keys never collide.
const FIRST_TARGET_VIEW_ID: u64 = 1 << 32;

/// Convert a mod API colour attachment format to its `wgpu::TextureFormat`.
pub fn wgpu_color_format(format: RenderTargetFormat) -> wgpu::TextureFormat {
    match format {
        RenderTargetFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        RenderTargetFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        RenderTargetFormat::R32Float => wgpu::TextureFormat::R32Float,
        _ => wgpu::TextureFormat::Rgba8Unorm,
    }
}

/// Convert a mod API depth/stencil format to its `wgpu::TextureFormat`.
pub fn wgpu_depth_format(depth: RenderTargetDepth) -> Option<wgpu::TextureFormat> {
    match depth {
        RenderTargetDepth::None => None,
        RenderTargetDepth::Depth32Float => Some(wgpu::TextureFormat::Depth32Float),
        RenderTargetDepth::Depth24PlusStencil8 => Some(wgpu::TextureFormat::Depth24PlusStencil8),
        _ => None,
    }
}

/// Cloned texture views and format metadata for executing an [`OffscreenPass`] without holding
/// the [`ModRenderTargets`] mutex across GPU pass recording.
pub struct ResolvedOffscreenTarget {
    pub handle: RenderTargetRef,
    pub size: [u32; 2],
    pub color_views: Vec<wgpu::TextureView>,
    pub color_formats: Vec<wgpu::TextureFormat>,
    pub depth_view: Option<wgpu::TextureView>,
    pub depth_format: Option<wgpu::TextureFormat>,
    pub has_stencil: bool,
}

struct ColorAttachment {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    /// The view over every mip level.
    view: wgpu::TextureView,
    view_id: u64,
    /// One single-level view per mip level: its render attachment, storage image and
    /// single-level sampled view.
    mips: Vec<(u64, wgpu::TextureView)>,
    /// Views over other mip ranges `(base_mip, mip_levels)`, made when first asked for.
    ranges: BTreeMap<(u32, u32), (u64, wgpu::TextureView)>,
    format: wgpu::TextureFormat,
}

struct DepthAttachment {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    attachment_view: wgpu::TextureView,
    sample_view: wgpu::TextureView,
    view_id: u64,
    format: wgpu::TextureFormat,
    has_stencil: bool,
}

struct ModRenderTarget {
    generation: u64,
    owner: Option<&'static str>,
    size: [u32; 2],
    color_formats: Vec<RenderTargetFormat>,
    depth_format: RenderTargetDepth,
    mip_levels: u32,
    storage: Option<StorageImageAccess>,
    colors: Vec<ColorAttachment>,
    depth: Option<DepthAttachment>,
}

/// Generational store of mod-owned offscreen GPU render targets (`RenderTargetRef`).
pub struct ModRenderTargets {
    next_generation: u64,
    slots: Vec<Option<ModRenderTarget>>,
    next_view_id: u64,
    /// The textures of attachments freed since the last [`Self::take_released`].
    released: Vec<u64>,
}

impl Default for ModRenderTargets {
    fn default() -> Self {
        Self {
            next_generation: 0,
            slots: Vec::new(),
            next_view_id: FIRST_TARGET_VIEW_ID,
            released: Vec::new(),
        }
    }
}

impl ModRenderTargets {
    fn alloc_view_id(&mut self) -> u64 {
        let id = self.next_view_id;
        self.next_view_id = self
            .next_view_id
            .checked_add(1)
            .expect("mod render target view id exhausted");
        id
    }

    /// Make the attachments' textures and views. Everything was checked before this is called;
    /// the allocation itself sits in error scopes so a device refusal is an `Err`.
    fn allocate_attachments(
        &mut self,
        device: &wgpu::Device,
        size: [u32; 2],
        color_formats: &[RenderTargetFormat],
        depth_format: RenderTargetDepth,
        mip_levels: u32,
        storage: Option<StorageImageAccess>,
    ) -> Result<(Vec<ColorAttachment>, Option<DepthAttachment>), RenderTargetError> {
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let mut color_usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        if storage.is_some() {
            color_usage |= wgpu::TextureUsages::STORAGE_BINDING;
        }
        let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut colors = Vec::with_capacity(color_formats.len());
        for (idx, &fmt) in color_formats.iter().enumerate() {
            let format = wgpu_color_format(fmt);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("Mod render target color {idx}")),
                size: extent,
                mip_level_count: mip_levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: color_usage,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let view_id = self.alloc_view_id();
            let mips = (0..mip_levels)
                .map(|mip| {
                    let view = texture.create_view(&wgpu::TextureViewDescriptor {
                        label: Some("Mod render target mip"),
                        base_mip_level: mip,
                        mip_level_count: Some(1),
                        ..Default::default()
                    });
                    (self.alloc_view_id(), view)
                })
                .collect();
            colors.push(ColorAttachment {
                texture,
                view,
                view_id,
                mips,
                ranges: BTreeMap::new(),
                format,
            });
        }

        let depth = wgpu_depth_format(depth_format).map(|format| {
            let has_stencil = matches!(format, wgpu::TextureFormat::Depth24PlusStencil8);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Mod render target depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let attachment_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let sample_view = texture.create_view(&wgpu::TextureViewDescriptor {
                aspect: wgpu::TextureAspect::DepthOnly,
                ..Default::default()
            });
            let view_id = self.alloc_view_id();
            DepthAttachment {
                texture,
                attachment_view,
                sample_view,
                view_id,
                format,
                has_stencil,
            }
        });
        let validation = pollster::block_on(validation.pop());
        let out_of_memory = pollster::block_on(out_of_memory.pop());
        if validation.or(out_of_memory).is_some() {
            return Err(RenderTargetError::Device);
        }
        Ok((colors, depth))
    }

    /// Allocate a mod-owned offscreen GPU render target.
    pub fn create(
        &mut self,
        device: &wgpu::Device,
        owner: Option<&'static str>,
        desc: &RenderTargetDescriptor,
    ) -> Result<RenderTargetRef, RenderTargetError> {
        desc.validate(device.limits().max_texture_dimension_2d)?;
        let (colors, depth) = self.allocate_attachments(
            device,
            desc.size,
            &desc.color_targets,
            desc.depth,
            desc.mip_levels,
            desc.storage,
        )?;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("mod render target generation exhausted");
        let generation = self.next_generation;
        let entry = ModRenderTarget {
            generation,
            owner,
            size: desc.size,
            color_formats: desc.color_targets.clone(),
            depth_format: desc.depth,
            mip_levels: desc.mip_levels,
            storage: desc.storage,
            colors,
            depth,
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
        Ok(mod_api::__private::built::render_target_ref(
            generation,
            slot as u32,
        ))
    }

    /// Resize `handle` to `size`. Calling with the target's current dimensions is a no-op that
    /// preserves existing attachment contents.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
        size: [u32; 2],
    ) -> Result<(), RenderTargetError> {
        let max_dim = device.limits().max_texture_dimension_2d;
        if size[0] == 0 || size[1] == 0 || size[0] > max_dim || size[1] > max_dim {
            return Err(RenderTargetError::InvalidSize);
        }
        let (current_size, color_formats, depth_format, mip_levels, storage) = {
            let entry = self.get_owned(owner, handle)?;
            (
                entry.size,
                entry.color_formats.clone(),
                entry.depth_format,
                entry.mip_levels,
                entry.storage,
            )
        };
        if current_size == size {
            return Ok(());
        }
        if mip_levels > RenderTargetDescriptor::max_mip_levels(size) {
            return Err(RenderTargetError::InvalidMipLevels);
        }
        let (colors, depth) = self.allocate_attachments(
            device,
            size,
            &color_formats,
            depth_format,
            mip_levels,
            storage,
        )?;
        let slot = handle.fields().slot as usize;
        if let Some(entry) = self.slots.get_mut(slot).and_then(Option::as_mut) {
            entry.size = size;
            self.released
                .extend(entry.colors.iter().map(|color| color.view_id));
            entry.colors = colors;
            entry.depth = depth;
        }
        Ok(())
    }

    /// Current `[width, height]` pixel dimensions of `handle`.
    pub fn size(
        &self,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
    ) -> Result<[u32; 2], RenderTargetError> {
        Ok(self.get_owned(owner, handle)?.size)
    }

    /// Free `handle`'s GPU textures and retire it (`RenderTargetError::Stale` thereafter).
    pub fn release(
        &mut self,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
    ) -> Result<(), RenderTargetError> {
        self.get_owned(owner, handle)?;
        let slot = handle.fields().slot as usize;
        if let Some(entry) = self.slots[slot].take() {
            self.released
                .extend(entry.colors.iter().map(|color| color.view_id));
        }
        Ok(())
    }

    /// The textures (see [`TargetMips::texture`]) freed since the last call.
    pub fn take_released(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.released)
    }

    /// Free every render target owned by a mod in `unloaded`.
    pub fn retire_unloaded(&mut self, unloaded: &BTreeSet<String>) {
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|rt| rt.owner.is_some_and(|id| unloaded.contains(id)))
                && let Some(entry) = slot.take()
            {
                self.released
                    .extend(entry.colors.iter().map(|color| color.view_id));
            }
        }
    }

    /// Resolve every mip level of a colour attachment for shader sampling
    /// (`TextureSource::RenderTargetColor`).
    pub fn resolve_color(
        &mut self,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
        attachment: u8,
        filterable: bool,
        float32_filterable: bool,
    ) -> Result<(u64, wgpu::TextureView, TargetMips), PipelineDrawError> {
        let mip_levels = self
            .get_owned(owner, handle)
            .map_err(|_| PipelineDrawError::InvalidBinding)?
            .mip_levels;
        self.resolve_color_mips(
            owner,
            mod_api::RenderTargetMipView::new(handle, attachment, 0, mip_levels),
            filterable,
            float32_filterable,
        )
    }

    /// Resolve a mip range of a colour attachment for shader sampling
    /// (`TextureSource::RenderTargetColorMip`). Refused with
    /// `PipelineDrawError::InvalidBinding` for a range outside the chain.
    pub fn resolve_color_mips(
        &mut self,
        owner: Option<&'static str>,
        view: mod_api::RenderTargetMipView,
        filterable: bool,
        float32_filterable: bool,
    ) -> Result<(u64, wgpu::TextureView, TargetMips), PipelineDrawError> {
        let (handle, base, count) = (view.target, view.base_mip, view.mip_levels);
        let entry = self
            .get_owned(owner, handle)
            .map_err(|_| PipelineDrawError::InvalidBinding)?;
        let mip_levels = entry.mip_levels;
        let end = base
            .checked_add(count)
            .ok_or(PipelineDrawError::InvalidBinding)?;
        if count == 0 || end > mip_levels {
            return Err(PipelineDrawError::InvalidBinding);
        }
        let color = entry
            .colors
            .get(view.attachment as usize)
            .ok_or(PipelineDrawError::InvalidBinding)?;
        if filterable && color.format == wgpu::TextureFormat::R32Float && !float32_filterable {
            return Err(PipelineDrawError::InvalidBinding);
        }
        let mips = TargetMips {
            texture: color.view_id,
            base_mip: base,
            mip_levels: count,
            usage: TargetUse::Sampled,
        };
        if count == 1 {
            let (id, view) = &color.mips[base as usize];
            return Ok((*id, view.clone(), mips));
        }
        if base == 0 && count == mip_levels {
            return Ok((color.view_id, color.view.clone(), mips));
        }
        let new_id = (!color.ranges.contains_key(&(base, count))).then(|| self.alloc_view_id());
        let color = self
            .slots
            .get_mut(handle.fields().slot as usize)
            .and_then(Option::as_mut)
            .and_then(|entry| entry.colors.get_mut(view.attachment as usize))
            .ok_or(PipelineDrawError::InvalidBinding)?;
        let texture = &color.texture;
        let (id, view) = color.ranges.entry((base, count)).or_insert_with(|| {
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("Mod render target mip range"),
                base_mip_level: base,
                mip_level_count: Some(count),
                ..Default::default()
            });
            (new_id.unwrap_or_default(), view)
        });
        Ok((*id, view.clone(), mips))
    }

    /// Resolve one mip level of a colour attachment as a storage image
    /// ([`mod_api::BindingKind::StorageImage`]) of `access`, with the format of its texture.
    /// Refused with `PipelineDrawError::InvalidBinding` when the target was not created for
    /// `access` or the level does not exist.
    pub fn resolve_storage_image(
        &self,
        owner: Option<&'static str>,
        access: StorageImageAccess,
        image: StorageImageBinding,
    ) -> Result<(u64, wgpu::TextureView, wgpu::TextureFormat, TargetMips), PipelineDrawError> {
        let entry = self
            .get_owned(owner, image.target)
            .map_err(|_| PipelineDrawError::InvalidBinding)?;
        let created_for = entry.storage.ok_or(PipelineDrawError::InvalidBinding)?;
        if created_for != StorageImageAccess::ReadWrite && created_for != access {
            return Err(PipelineDrawError::InvalidBinding);
        }
        let color = entry
            .colors
            .get(image.attachment as usize)
            .ok_or(PipelineDrawError::InvalidBinding)?;
        let (id, view) = color
            .mips
            .get(image.mip_level as usize)
            .ok_or(PipelineDrawError::InvalidBinding)?;
        let mips = TargetMips {
            texture: color.view_id,
            base_mip: image.mip_level,
            mip_levels: 1,
            usage: match access {
                StorageImageAccess::ReadOnly => TargetUse::StorageReadOnly,
                StorageImageAccess::WriteOnly => TargetUse::StorageWriteOnly,
                _ => TargetUse::StorageReadWrite,
            },
        };
        Ok((*id, view.clone(), color.format, mips))
    }

    /// Resolve the depth aspect for shader sampling (`TextureSource::RenderTargetDepth`).
    pub fn resolve_depth(
        &self,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
        filterable: bool,
    ) -> Result<(u64, wgpu::TextureView), PipelineDrawError> {
        if filterable {
            return Err(PipelineDrawError::InvalidBinding);
        }
        let entry = self
            .get_owned(owner, handle)
            .map_err(|_| PipelineDrawError::InvalidBinding)?;
        let depth = entry
            .depth
            .as_ref()
            .ok_or(PipelineDrawError::InvalidBinding)?;
        Ok((depth.view_id, depth.sample_view.clone()))
    }

    /// Snapshot the views of mip level `mip_level` and format metadata for executing an
    /// [`OffscreenPass`]; `size` is that level's size
    /// (`GpuTextureView.getWidth(mipLevel)`, `getHeight(mipLevel)`).
    pub fn resolve_target(
        &self,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
        mip_level: u32,
    ) -> Result<ResolvedOffscreenTarget, RenderTargetError> {
        let entry = self.get_owned(owner, handle)?;
        if mip_level >= entry.mip_levels {
            return Err(RenderTargetError::InvalidMipLevels);
        }
        Ok(ResolvedOffscreenTarget {
            handle,
            size: [
                (entry.size[0] >> mip_level).max(1),
                (entry.size[1] >> mip_level).max(1),
            ],
            color_views: entry
                .colors
                .iter()
                .map(|c| c.mips[mip_level as usize].1.clone())
                .collect(),
            color_formats: entry.colors.iter().map(|c| c.format).collect(),
            depth_view: entry.depth.as_ref().map(|d| d.attachment_view.clone()),
            depth_format: entry.depth.as_ref().map(|d| d.format),
            has_stencil: entry.depth.as_ref().is_some_and(|d| d.has_stencil),
        })
    }

    /// Access the underlying colour texture of `handle` (used by GPU readback tests).
    #[cfg(test)]
    pub(crate) fn color_texture(
        &self,
        handle: RenderTargetRef,
        attachment: u8,
    ) -> Option<&wgpu::Texture> {
        let fields = handle.fields();
        let entry = self.slots.get(fields.slot as usize)?.as_ref()?;
        if entry.generation != fields.generation {
            return None;
        }
        entry.colors.get(attachment as usize).map(|c| &c.texture)
    }

    fn get_owned(
        &self,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
    ) -> Result<&ModRenderTarget, RenderTargetError> {
        let fields = handle.fields();
        let Some(entry) = self
            .slots
            .get(fields.slot as usize)
            .and_then(Option::as_ref)
        else {
            return Err(RenderTargetError::Stale);
        };
        if entry.generation != fields.generation {
            return Err(RenderTargetError::Stale);
        }
        if entry.owner.is_some() && entry.owner != owner {
            return Err(RenderTargetError::NotOwned);
        }
        Ok(entry)
    }
}

/// The optional device features mods can ask about: one read of the adapter.
pub(super) fn probe_render_capabilities(
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
) -> RenderCapabilities {
    let read_write_storage = [
        RenderTargetFormat::Rgba8Unorm,
        RenderTargetFormat::Rgba16Float,
        RenderTargetFormat::R32Float,
    ]
    .into_iter()
    .filter(|&format| {
        adapter
            .get_texture_format_features(wgpu_color_format(format))
            .flags
            .contains(wgpu::TextureFormatFeatureFlags::STORAGE_READ_WRITE)
    })
    .collect();
    mod_api::__private::built::render_capabilities(
        device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE),
        read_write_storage,
        device.features().contains(wgpu::Features::SUBGROUP),
        device
            .features()
            .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
    )
}

impl super::mod_gpu::ModGpuResources {
    /// Allocate a mod-owned offscreen [`RenderTargetRef`] on this renderer's device. A storage
    /// access the adapter does not support for a colour format (`TextureFormatFeatures`
    /// `allowed_usages` without `STORAGE_BINDING`, or `flags` without the access's
    /// `STORAGE_READ_ONLY`, `STORAGE_WRITE_ONLY` or `STORAGE_READ_WRITE`) is refused before any
    /// GPU call.
    pub fn create_render_target(
        &self,
        owner: Option<&'static str>,
        desc: &RenderTargetDescriptor,
    ) -> Result<RenderTargetRef, RenderTargetError> {
        if let Some(access) = desc.storage {
            let flag = match access {
                StorageImageAccess::ReadOnly => wgpu::TextureFormatFeatureFlags::STORAGE_READ_ONLY,
                StorageImageAccess::WriteOnly => {
                    wgpu::TextureFormatFeatureFlags::STORAGE_WRITE_ONLY
                }
                _ => wgpu::TextureFormatFeatureFlags::STORAGE_READ_WRITE,
            };
            let supported = |format: RenderTargetFormat| {
                let features = self
                    .adapter
                    .get_texture_format_features(wgpu_color_format(format));
                features
                    .allowed_usages
                    .contains(wgpu::TextureUsages::STORAGE_BINDING)
                    && features.flags.contains(flag)
            };
            if !desc.color_targets.iter().all(|&format| supported(format)) {
                return Err(RenderTargetError::UnsupportedStorage);
            }
        }
        self.render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .create(&self.device, owner, desc)
    }

    /// Resize a mod-owned offscreen [`RenderTargetRef`] (no-op if unchanged).
    pub fn render_target_resize(
        &self,
        owner: Option<&'static str>,
        target: RenderTargetRef,
        size: [u32; 2],
    ) -> Result<(), RenderTargetError> {
        self.render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resize(&self.device, owner, target, size)
    }

    /// Query the current pixel dimensions of a mod-owned offscreen [`RenderTargetRef`].
    pub fn render_target_size(
        &self,
        owner: Option<&'static str>,
        target: RenderTargetRef,
    ) -> Result<[u32; 2], RenderTargetError> {
        self.render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .size(owner, target)
    }

    /// Free a mod-owned offscreen [`RenderTargetRef`] and retire its handle.
    pub fn render_target_release(
        &self,
        owner: Option<&'static str>,
        target: RenderTargetRef,
    ) -> Result<(), RenderTargetError> {
        self.render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .release(owner, target)
    }

    /// Free the render targets of mods that unloaded.
    pub fn retire_unloaded_mod_render_targets(&self, unloaded: &BTreeSet<String>) {
        self.render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retire_unloaded(unloaded);
    }

    /// Maximum 2D texture dimension supported by this renderer's GPU device.
    pub fn max_texture_dimension_2d(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    /// Maximum workgroup count of one compute dispatch dimension on this renderer's GPU device.
    pub fn max_compute_workgroups_per_dimension(&self) -> u32 {
        self.device.limits().max_compute_workgroups_per_dimension
    }

    /// Whether this renderer's GPU device supports linear filtering on `R32Float` textures.
    pub fn float32_filterable(&self) -> bool {
        self.device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
    }

    /// Record a distinct frame-graph cycle warning once per reload/world session.
    pub fn warn_frame_graph_cycle_once(&self, passes: &[String]) {
        let mut warned = self
            .warned_frame_graph_cycles
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if warned.insert(passes.to_vec()) {
            log::warn!(
                "Sunrise: frame graph cycle detected between {}",
                passes.join(", ")
            );
        }
    }

    /// Resolve a mod render target colour attachment for set-1 sampling.
    pub fn resolve_mod_render_target_color(
        &self,
        owner: Option<&'static str>,
        target: RenderTargetRef,
        attachment: u8,
        filterable: bool,
    ) -> Result<(u64, wgpu::TextureView, TargetMips), PipelineDrawError> {
        let float32_filterable = self
            .device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE);
        self.render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_color(owner, target, attachment, filterable, float32_filterable)
    }

    /// Resolve a mip range of a mod render target colour attachment for set-1 sampling.
    pub fn resolve_mod_render_target_color_mips(
        &self,
        owner: Option<&'static str>,
        view: mod_api::RenderTargetMipView,
        filterable: bool,
    ) -> Result<(u64, wgpu::TextureView, TargetMips), PipelineDrawError> {
        let float32_filterable = self
            .device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE);
        self.render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_color_mips(owner, view, filterable, float32_filterable)
    }

    /// Resolve one mip level of a mod render target colour attachment as a set-1 storage image.
    pub fn resolve_mod_storage_image(
        &self,
        owner: Option<&'static str>,
        access: StorageImageAccess,
        image: StorageImageBinding,
    ) -> Result<(u64, wgpu::TextureView, wgpu::TextureFormat, TargetMips), PipelineDrawError> {
        self.render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_storage_image(owner, access, image)
    }

    /// Resolve a mod render target depth aspect for set-1 sampling.
    pub fn resolve_mod_render_target_depth(
        &self,
        owner: Option<&'static str>,
        target: RenderTargetRef,
        filterable: bool,
    ) -> Result<(u64, wgpu::TextureView), PipelineDrawError> {
        self.render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_depth(owner, target, filterable)
    }

    /// Snapshot attachment views for executing an [`OffscreenPass`].
    pub fn resolve_mod_render_target(
        &self,
        owner: Option<&'static str>,
        target: RenderTargetRef,
        mip_level: u32,
    ) -> Result<ResolvedOffscreenTarget, RenderTargetError> {
        self.render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_target(owner, target, mip_level)
    }

    /// The textures of render target attachments freed since the last call.
    pub(super) fn take_released_mod_render_targets(&self) -> Vec<u64> {
        self.render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take_released()
    }
}

impl super::Renderer {
    /// Record an offscreen pass into an active `CommandEncoder` (shared seam for
    /// `frame_graph_setup` and picture-in-picture GUI renderers).
    #[allow(clippy::too_many_arguments)]
    pub fn record_mod_offscreen_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        owner: Option<&'static str>,
        pass: &OffscreenPass,
        draws: &[super::mod_pipelines::PipelineDraw],
        transient_vertices: &[u8],
        transient_indices: &[u8],
        inline_uniforms: &[u8],
        texel_data: &[u8],
    ) -> Result<(), RenderTargetError> {
        let target = self.resolve_mod_render_target(owner, pass.target, pass.mip_level)?;
        let Some(primitives) = self.world_primitives.as_mut() else {
            return Err(RenderTargetError::Unavailable);
        };
        primitives.execute_offscreen_pass(
            &self.device,
            &self.queue,
            encoder,
            &super::mod_pipelines::OffscreenPassData {
                pass,
                target: &target,
                draws,
                transient_vertices,
                transient_indices,
                inline_uniforms,
                texel_data,
            },
        );
        Ok(())
    }

    /// Execute and submit a standalone offscreen pass onto a mod-owned [`RenderTargetRef`]
    /// (for picture-in-picture GUI renderers running outside the world frame encoder).
    #[allow(clippy::too_many_arguments)]
    pub fn execute_mod_offscreen_pass(
        &mut self,
        owner: Option<&'static str>,
        pass: &OffscreenPass,
        draws: &[super::mod_pipelines::PipelineDraw],
        transient_vertices: &[u8],
        transient_indices: &[u8],
        inline_uniforms: &[u8],
        texel_data: &[u8],
    ) -> Result<(), RenderTargetError> {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Mod offscreen render pass"),
            });
        self.record_mod_offscreen_pass(
            &mut encoder,
            owner,
            pass,
            draws,
            transient_vertices,
            transient_indices,
            inline_uniforms,
            texel_data,
        )?;
        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::RenderTargetMipView as View;

    /// Clear mip `mip` of the first colour attachment to `value` and read its first texel back.
    fn clear_and_read_first_texel(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: &mut ModRenderTargets,
        rt: RenderTargetRef,
        mip: u32,
        value: f64,
    ) -> f32 {
        let (_, view, _) = targets
            .resolve_color_mips(None, View::single(rt, 0, mip), false, false)
            .unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: value,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        }));
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let texture = targets.color_texture(rt, 0).unwrap();
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        f32::from_ne_bytes(mapped[..4].try_into().unwrap())
    }

    #[test]
    #[ignore = "daily"]
    fn a_mip_chain_has_a_view_per_level_and_refuses_the_storage_images_it_was_not_made_for() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };
        let mut targets = ModRenderTargets::default();
        let desc = RenderTargetDescriptor::new([64, 64])
            .color_target(RenderTargetFormat::R32Float)
            .mip_levels(7)
            .storage(StorageImageAccess::WriteOnly);
        let rt = targets.create(&device, None, &desc).unwrap();

        // Each level has a view of its own that holds what was drawn into it.
        for mip in 0..7 {
            clear_and_read_first_texel(
                &device,
                &queue,
                &mut targets,
                rt,
                mip,
                f64::from(mip) / 8.0,
            );
        }
        assert_eq!(
            clear_and_read_first_texel(&device, &queue, &mut targets, rt, 2, 0.5),
            0.5
        );
        assert_eq!(
            clear_and_read_first_texel(&device, &queue, &mut targets, rt, 3, 0.25),
            0.25
        );

        // A range view is made once and keeps its id; a single level and the whole chain reuse
        // the views the target made.
        let id = |targets: &mut ModRenderTargets, view| {
            targets
                .resolve_color_mips(None, view, false, false)
                .map(|(id, _, _)| id)
        };
        let range = id(&mut targets, View::new(rt, 0, 2, 3)).unwrap();
        assert_eq!(id(&mut targets, View::new(rt, 0, 2, 3)), Ok(range));
        assert_ne!(Some(range), id(&mut targets, View::single(rt, 0, 2)).ok());
        assert_eq!(
            id(&mut targets, View::new(rt, 0, 0, 7)),
            targets
                .resolve_color(None, rt, 0, false, false)
                .map(|(id, _, _)| id)
        );
        for bad in [
            View::new(rt, 0, 5, 3),
            View::new(rt, 0, 0, 0),
            View::single(rt, 1, 0),
        ] {
            assert_eq!(
                id(&mut targets, bad),
                Err(PipelineDrawError::InvalidBinding),
                "{bad:?}"
            );
        }

        let image = |mip| StorageImageBinding::new(rt, 0, mip);
        assert!(
            targets
                .resolve_storage_image(None, StorageImageAccess::WriteOnly, image(5))
                .is_ok()
        );

        // The target was created for write-only storage: read-only or read-write use of it, a
        // level it lacks and a colour attachment it lacks are all refused.
        for refused in [
            (StorageImageAccess::ReadOnly, image(0)),
            (StorageImageAccess::ReadWrite, image(0)),
            (StorageImageAccess::WriteOnly, image(7)),
            (
                StorageImageAccess::WriteOnly,
                StorageImageBinding::new(rt, 1, 0),
            ),
        ] {
            assert_eq!(
                targets
                    .resolve_storage_image(None, refused.0, refused.1)
                    .map(|_| ()),
                Err(PipelineDrawError::InvalidBinding),
                "{refused:?}"
            );
        }
        let plain = targets
            .create(
                &device,
                None,
                &RenderTargetDescriptor::new([8, 8]).color_target(RenderTargetFormat::Rgba8Unorm),
            )
            .unwrap();
        assert_eq!(
            targets
                .resolve_storage_image(
                    None,
                    StorageImageAccess::WriteOnly,
                    StorageImageBinding::new(plain, 0, 0)
                )
                .map(|_| ()),
            Err(PipelineDrawError::InvalidBinding)
        );

        // A resize keeps the chain while the new size has room for it, and refuses otherwise.
        targets.resize(&device, None, rt, [128, 128]).unwrap();
        assert_eq!(targets.size(None, rt), Ok([128, 128]));
        assert_eq!(
            targets.resize(&device, None, rt, [32, 32]),
            Err(RenderTargetError::InvalidMipLevels)
        );
        targets.release(None, rt).unwrap();
        assert_eq!(targets.size(None, rt), Err(RenderTargetError::Stale));
        // The resize freed the first allocation's texture and the release the second's.
        assert_eq!(targets.take_released().len(), 2);
    }
}
