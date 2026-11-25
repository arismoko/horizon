//! Retained GPU vertex and index buffers backing [`mod_api::MeshRef`].
//!
//! Each live mesh holds real `wgpu::Buffer`s allocated on the client GPU device, bounded only
//! by `device.limits().max_buffer_size` (zero uncited per-mod cap). Drawing a retained mesh
//! resolves its `Arc`-backed `wgpu::Buffer`s directly without copying vertex bytes on the CPU.
//! Handles are generational, owner-checked, and retired on mod disable, world close, resource
//! reload, or explicit [`mod_api::MeshRef::release`].

use mod_api::__private::game::Fields;
use mod_api::{
    DrawRange, IndexFormat, MAX_VERTEX_BINDINGS, MeshDescriptor, MeshDraw, MeshError, MeshRef,
    PipelineDrawError, PrimitiveTopology, VERTEX_ALIGNMENT, VertexFormat, VertexStepMode,
};
use std::collections::BTreeSet;

/// One retained GPU vertex or instance buffer.
#[derive(Clone, Debug)]
pub struct ModMeshBuffer {
    pub buffer: wgpu::Buffer,
    pub stride: u32,
    pub byte_len: u64,
}

/// One retained GPU index buffer.
#[derive(Clone, Debug)]
pub struct ModMeshIndexBuffer {
    pub buffer: wgpu::Buffer,
    pub format: IndexFormat,
    pub byte_len: u64,
}

struct ModMesh {
    generation: u64,
    owner: Option<&'static str>,
    vertex_buffers: Vec<ModMeshBuffer>,
    index_buffer: Option<ModMeshIndexBuffer>,
}

/// Zero-copy resolved GPU buffers for one retained [`MeshRef`] draw call.
#[derive(Clone, Debug)]
pub struct ResolvedMeshDraw {
    /// `(binding_index, buffer, byte_len)` matching the pipeline's `VertexFormat` bindings.
    pub vertex_buffers: Vec<(u32, wgpu::Buffer, u64)>,
    /// Bound index buffer when `draw.range` is [`DrawRange::Indexed`].
    pub index_buffer: Option<(wgpu::Buffer, wgpu::IndexFormat)>,
}

/// Generational store of retained GPU [`MeshRef`] buffers owned by `Renderer`.
#[derive(Default)]
pub struct ModMeshes {
    next_generation: u64,
    slots: Vec<Option<ModMesh>>,
}

impl ModMeshes {
    /// Allocate GPU vertex buffer(s) and optional index buffer described by `desc`.
    pub fn create_mesh(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        owner: Option<&'static str>,
        desc: &MeshDescriptor<'_>,
    ) -> Result<MeshRef, MeshError> {
        let (vertex_buffers, index_buffer) = build_buffers(device, queue, desc)?;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("retained mesh generation exhausted");
        let generation = self.next_generation;
        let mesh = ModMesh {
            generation,
            owner,
            vertex_buffers,
            index_buffer,
        };
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.slots.len());
        if slot == self.slots.len() {
            self.slots.push(Some(mesh));
        } else {
            self.slots[slot] = Some(mesh);
        }
        Ok(mod_api::__private::built::mesh_ref(generation, slot as u32))
    }

    /// Partially or fully update vertex buffer `binding` of `handle`.
    pub fn write_vertices(
        &self,
        queue: &wgpu::Queue,
        owner: Option<&'static str>,
        handle: MeshRef,
        binding: u8,
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), MeshError> {
        let mesh = self.get_owned(owner, handle)?;
        let vb = mesh
            .vertex_buffers
            .get(usize::from(binding))
            .ok_or(MeshError::InvalidBinding)?;
        validate_write_range(byte_offset, data.len(), vb.byte_len)?;
        if !data.is_empty() {
            queue.write_buffer(&vb.buffer, byte_offset, data);
        }
        Ok(())
    }

    /// Partially or fully update the index buffer of `handle`.
    pub fn write_indices(
        &self,
        queue: &wgpu::Queue,
        owner: Option<&'static str>,
        handle: MeshRef,
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), MeshError> {
        let mesh = self.get_owned(owner, handle)?;
        let ib = mesh.index_buffer.as_ref().ok_or(MeshError::NoIndexBuffer)?;
        validate_write_range(byte_offset, data.len(), ib.byte_len)?;
        if !data.is_empty() {
            queue.write_buffer(&ib.buffer, byte_offset, data);
        }
        Ok(())
    }

    /// Reallocate backing GPU buffers in place without changing `handle`'s generation or slot.
    pub fn replace(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        owner: Option<&'static str>,
        handle: MeshRef,
        desc: &MeshDescriptor<'_>,
    ) -> Result<(), MeshError> {
        self.get_owned(owner, handle)?;
        let (vertex_buffers, index_buffer) = build_buffers(device, queue, desc)?;
        let slot = handle.fields().slot as usize;
        let mesh = self.slots[slot].as_mut().expect("validated live mesh slot");
        mesh.vertex_buffers = vertex_buffers;
        mesh.index_buffer = index_buffer;
        Ok(())
    }

    /// Immediately free `handle`'s GPU buffers and retire it (`MeshError::Stale` thereafter).
    pub fn release(
        &mut self,
        owner: Option<&'static str>,
        handle: MeshRef,
    ) -> Result<(), MeshError> {
        self.get_owned(owner, handle)?;
        let slot = handle.fields().slot as usize;
        self.slots[slot] = None;
        Ok(())
    }

    /// Retire all live meshes (called on world close and resource reload).
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    /// Retire every mesh owned by a mod in `disabled`.
    pub fn retire_disabled(&mut self, disabled: &BTreeSet<String>) {
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|mesh| mesh.owner.is_some_and(|id| disabled.contains(id)))
            {
                *slot = None;
            }
        }
    }

    /// Validate `handle` against `bindings`, `topology`, and `draw`, returning `Arc`-cloned GPU
    /// buffers ready to bind with zero CPU vertex copying.
    pub fn resolve_draw(
        &self,
        owner: Option<&'static str>,
        handle: MeshRef,
        bindings: &[(u8, VertexFormat)],
        topology: PrimitiveTopology,
        draw: &MeshDraw,
    ) -> Result<ResolvedMeshDraw, PipelineDrawError> {
        let mesh = self
            .get_owned(owner, handle)
            .map_err(PipelineDrawError::Mesh)?;
        if draw.instances.start > draw.instances.end {
            return Err(PipelineDrawError::InvalidRange);
        }
        let mut resolved_vbs = Vec::with_capacity(bindings.len());
        for (binding_idx, fmt) in bindings {
            let vb = mesh
                .vertex_buffers
                .get(usize::from(*binding_idx))
                .ok_or(PipelineDrawError::VertexFormatMismatch)?;
            if vb.stride != fmt.stride {
                return Err(PipelineDrawError::VertexFormatMismatch);
            }
            let record_count = vb.byte_len / u64::from(vb.stride);
            match fmt.step_mode {
                VertexStepMode::Vertex => {
                    if let DrawRange::Vertices(ref range) = draw.range
                        && (range.start > range.end || u64::from(range.end) > record_count)
                    {
                        return Err(PipelineDrawError::InvalidRange);
                    }
                }
                VertexStepMode::Instance => {
                    if u64::from(draw.instances.end) > record_count {
                        return Err(PipelineDrawError::InvalidRange);
                    }
                }
                _ => return Err(PipelineDrawError::VertexFormatMismatch),
            }
            resolved_vbs.push((u32::from(*binding_idx), vb.buffer.clone(), vb.byte_len));
        }

        let index_buffer = match &draw.range {
            DrawRange::Vertices(range) => {
                if range.start > range.end {
                    return Err(PipelineDrawError::InvalidRange);
                }
                let count = range.end - range.start;
                if matches!(
                    topology,
                    PrimitiveTopology::Quads | PrimitiveTopology::Lines
                ) && count % 4 != 0
                {
                    return Err(PipelineDrawError::InvalidRange);
                }
                None
            }
            DrawRange::Indexed { indices, .. } => {
                let ib = mesh
                    .index_buffer
                    .as_ref()
                    .ok_or(PipelineDrawError::MissingIndexBuffer)?;
                if indices.start > indices.end {
                    return Err(PipelineDrawError::InvalidRange);
                }
                let index_count = ib.byte_len / u64::from(ib.format.byte_size());
                if u64::from(indices.end) > index_count {
                    return Err(PipelineDrawError::InvalidRange);
                }
                Some((ib.buffer.clone(), wgpu_index_format(ib.format)))
            }
            _ => return Err(PipelineDrawError::InvalidRange),
        };

        Ok(ResolvedMeshDraw {
            vertex_buffers: resolved_vbs,
            index_buffer,
        })
    }

    /// Validate `handle` against the pipeline's vertex `bindings` for an indirect draw and return
    /// its buffers. The draw's commands are indexed iff the mesh has an index buffer. A vertex
    /// range is the GPU's to choose, so only the strides are checked.
    pub fn resolve_indirect(
        &self,
        owner: Option<&'static str>,
        handle: MeshRef,
        bindings: &[(u8, VertexFormat)],
    ) -> Result<ResolvedMeshDraw, PipelineDrawError> {
        let mesh = self
            .get_owned(owner, handle)
            .map_err(PipelineDrawError::Mesh)?;
        let mut vertex_buffers = Vec::with_capacity(bindings.len());
        for (binding_idx, fmt) in bindings {
            let vb = mesh
                .vertex_buffers
                .get(usize::from(*binding_idx))
                .ok_or(PipelineDrawError::VertexFormatMismatch)?;
            if vb.stride != fmt.stride {
                return Err(PipelineDrawError::VertexFormatMismatch);
            }
            vertex_buffers.push((u32::from(*binding_idx), vb.buffer.clone(), vb.byte_len));
        }
        Ok(ResolvedMeshDraw {
            vertex_buffers,
            index_buffer: mesh
                .index_buffer
                .as_ref()
                .map(|ib| (ib.buffer.clone(), wgpu_index_format(ib.format))),
        })
    }

    fn get_owned(
        &self,
        owner: Option<&'static str>,
        handle: MeshRef,
    ) -> Result<&ModMesh, MeshError> {
        let fields = handle.fields();
        let mesh = self
            .slots
            .get(fields.slot as usize)
            .and_then(Option::as_ref)
            .filter(|m| m.generation == fields.generation)
            .ok_or(MeshError::Stale)?;
        if mesh.owner.is_some() && mesh.owner != owner {
            return Err(MeshError::NotOwned);
        }
        Ok(mesh)
    }
}

pub(crate) fn wgpu_index_format(format: IndexFormat) -> wgpu::IndexFormat {
    match format {
        IndexFormat::Uint16 => wgpu::IndexFormat::Uint16,
        IndexFormat::Uint32 => wgpu::IndexFormat::Uint32,
        _ => wgpu::IndexFormat::Uint32,
    }
}

fn validate_write_range(byte_offset: u64, data_len: usize, byte_len: u64) -> Result<(), MeshError> {
    if !byte_offset.is_multiple_of(4) || !data_len.is_multiple_of(4) {
        return Err(MeshError::InvalidAlignment);
    }
    let end = byte_offset
        .checked_add(data_len as u64)
        .ok_or(MeshError::OutOfRange)?;
    if end > byte_len {
        return Err(MeshError::OutOfRange);
    }
    Ok(())
}

fn build_buffers(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    desc: &MeshDescriptor<'_>,
) -> Result<(Vec<ModMeshBuffer>, Option<ModMeshIndexBuffer>), MeshError> {
    if desc.vertex_buffers.is_empty() || desc.vertex_buffers.len() > MAX_VERTEX_BINDINGS {
        return Err(MeshError::InvalidBinding);
    }
    let limit = super::upload::legal_buffer_limit(device);

    for vb in desc.vertex_buffers {
        if vb.stride == 0 || vb.stride % VERTEX_ALIGNMENT != 0 {
            return Err(MeshError::InvalidStride);
        }
        if vb.byte_len == 0 {
            return Err(MeshError::OutOfRange);
        }
        if vb.byte_len % u64::from(vb.stride) != 0 || vb.byte_len % 4 != 0 {
            return Err(MeshError::InvalidAlignment);
        }
        if vb.byte_len > limit {
            return Err(MeshError::ExceedsDeviceLimit);
        }
        if let Some(init) = vb.initial {
            validate_write_range(0, init.len(), vb.byte_len)?;
        }
    }
    if let Some(idx) = &desc.indices {
        if idx.byte_len == 0 {
            return Err(MeshError::OutOfRange);
        }
        let elem = u64::from(idx.format.byte_size());
        if idx.byte_len % elem != 0 || idx.byte_len % 4 != 0 {
            return Err(MeshError::InvalidAlignment);
        }
        if idx.byte_len > limit {
            return Err(MeshError::ExceedsDeviceLimit);
        }
        if let Some(init) = idx.initial {
            validate_write_range(0, init.len(), idx.byte_len)?;
        }
    }

    let mut vertex_buffers = Vec::with_capacity(desc.vertex_buffers.len());
    for vb in desc.vertex_buffers {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mod retained mesh vertex buffer"),
            size: vb.byte_len,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if let Some(init) = vb.initial
            && !init.is_empty()
        {
            queue.write_buffer(&buffer, 0, init);
        }
        vertex_buffers.push(ModMeshBuffer {
            buffer,
            stride: vb.stride,
            byte_len: vb.byte_len,
        });
    }

    let index_buffer = desc.indices.map(|idx| {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mod retained mesh index buffer"),
            size: idx.byte_len,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if let Some(init) = idx.initial
            && !init.is_empty()
        {
            queue.write_buffer(&buffer, 0, init);
        }
        ModMeshIndexBuffer {
            buffer,
            format: idx.format,
            byte_len: idx.byte_len,
        }
    });

    Ok((vertex_buffers, index_buffer))
}

impl super::mod_gpu::ModGpuResources {
    /// Allocate a retained GPU [`MeshRef`] owned by `owner`.
    pub fn create_mesh(
        &self,
        owner: Option<&'static str>,
        desc: &MeshDescriptor<'_>,
    ) -> Result<MeshRef, MeshError> {
        self.meshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .create_mesh(&self.device, &self.queue, owner, desc)
    }

    /// Update vertex buffer `binding` of retained `mesh`.
    pub fn mesh_write_vertices(
        &self,
        owner: Option<&'static str>,
        mesh: MeshRef,
        binding: u8,
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), MeshError> {
        self.meshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_vertices(&self.queue, owner, mesh, binding, byte_offset, data)
    }

    /// Update the index buffer of retained `mesh`.
    pub fn mesh_write_indices(
        &self,
        owner: Option<&'static str>,
        mesh: MeshRef,
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), MeshError> {
        self.meshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_indices(&self.queue, owner, mesh, byte_offset, data)
    }

    /// Reallocate backing GPU buffers of retained `mesh` in place.
    pub fn mesh_replace(
        &self,
        owner: Option<&'static str>,
        mesh: MeshRef,
        desc: &MeshDescriptor<'_>,
    ) -> Result<(), MeshError> {
        self.meshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(&self.device, &self.queue, owner, mesh, desc)
    }

    /// Free retained `mesh`'s GPU buffers and retire the handle.
    pub fn mesh_release(
        &self,
        owner: Option<&'static str>,
        mesh: MeshRef,
    ) -> Result<(), MeshError> {
        self.meshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .release(owner, mesh)
    }

    /// Retire all retained GPU meshes (called on world close and resource reload).
    pub fn clear_mod_meshes(&self) {
        self.meshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Retire all retained GPU meshes owned by any mod in `disabled`.
    pub fn retire_disabled_mod_meshes(&self, disabled: &BTreeSet<String>) {
        self.meshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retire_disabled(disabled);
    }

    /// Resolve a retained [`MeshRef`] for an indirect draw (see [`ModMeshes::resolve_indirect`]).
    pub fn resolve_mod_mesh_indirect(
        &self,
        owner: Option<&'static str>,
        mesh: MeshRef,
        bindings: &[(u8, VertexFormat)],
    ) -> Result<ResolvedMeshDraw, PipelineDrawError> {
        self.meshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_indirect(owner, mesh, bindings)
    }

    /// Validate and resolve a retained [`MeshRef`] draw call into zero-copy GPU buffer handles.
    pub fn resolve_mod_mesh_draw(
        &self,
        owner: Option<&'static str>,
        mesh: MeshRef,
        bindings: &[(u8, VertexFormat)],
        topology: PrimitiveTopology,
        draw: &MeshDraw,
    ) -> Result<ResolvedMeshDraw, PipelineDrawError> {
        self.meshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_draw(owner, mesh, bindings, topology, draw)
    }
}
