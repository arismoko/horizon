//! Retained GPU storage buffers backing [`mod_api::StorageBufferRef`] (vanilla's `GpuBuffer`).
//!
//! A buffer is capped at the device's `max_buffer_size`; a binding takes a range
//! ([`mod_api::StorageBufferSlice`], vanilla's `GpuBufferSlice`) of at most
//! `max_storage_buffer_binding_size` bytes whose offset is a multiple of
//! `min_storage_buffer_offset_alignment`. Writes are queued by `write_range` and applied in call
//! order by [`ModStorageBuffers::flush`] before the frame, as region updates to dynamic textures
//! are. A buffer survives a resource reload and a world close: it is freed by an explicit release
//! or when its mod unloads.

use mod_api::__private::game::Fields;
use mod_api::{BufferError, StorageBufferDescriptor, StorageBufferRef, StorageBufferSlice};
use std::collections::BTreeSet;

struct ModStorageBuffer {
    generation: u64,
    owner: Option<&'static str>,
    buffer: wgpu::Buffer,
    byte_len: u64,
}

struct PendingWrite {
    slot: usize,
    generation: u64,
    offset: u64,
    data: Vec<u8>,
}

/// A bound range of a retained buffer, resolved for a draw.
pub struct ResolvedStorageRange {
    /// The buffer's unique generation, the key of cached bind groups.
    pub id: u64,
    pub buffer: wgpu::Buffer,
    pub offset: u64,
    pub length: u64,
}

/// Generational store of retained GPU [`StorageBufferRef`] buffers owned by `Renderer`.
#[derive(Default)]
pub struct ModStorageBuffers {
    next_generation: u64,
    slots: Vec<Option<ModStorageBuffer>>,
    pending: Vec<PendingWrite>,
    /// Generations freed since the pipelines last dropped their cached bind groups.
    released: Vec<u64>,
}

impl ModStorageBuffers {
    /// Allocate a storage buffer for `owner`. Everything is checked before the device is asked;
    /// the allocation itself sits in error scopes so a device refusal is an `Err`.
    pub fn create(
        &mut self,
        device: &wgpu::Device,
        owner: Option<&'static str>,
        desc: &StorageBufferDescriptor,
    ) -> Result<StorageBufferRef, BufferError> {
        if desc.size == 0 {
            return Err(BufferError::Empty);
        }
        if !desc.size.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT) {
            return Err(BufferError::InvalidAlignment);
        }
        if desc.size > device.limits().max_buffer_size {
            return Err(BufferError::ExceedsDeviceLimit);
        }
        let generation = self
            .next_generation
            .checked_add(1)
            .ok_or(BufferError::Unavailable)?;
        let mut usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        if desc.indirect {
            usage |= wgpu::BufferUsages::INDIRECT;
        }
        let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mod storage buffer"),
            size: desc.size,
            usage,
            mapped_at_creation: false,
        });
        let validation = pollster::block_on(validation.pop());
        let out_of_memory = pollster::block_on(out_of_memory.pop());
        if let Some(error) = validation {
            log::warn!("Sunrise: a mod storage buffer was refused by the device: {error}");
            return Err(BufferError::ExceedsDeviceLimit);
        }
        if let Some(error) = out_of_memory {
            log::warn!("Sunrise: the device has no memory for a mod storage buffer: {error}");
            return Err(BufferError::ExceedsDeviceLimit);
        }
        self.next_generation = generation;
        let entry = ModStorageBuffer {
            generation,
            owner,
            buffer,
            byte_len: desc.size,
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
        Ok(mod_api::__private::built::storage_buffer_ref(
            generation,
            slot as u32,
        ))
    }

    /// Queue `data` for `offset` of `handle`, applied by the next [`Self::flush`].
    pub fn write_range(
        &mut self,
        owner: Option<&'static str>,
        handle: StorageBufferRef,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BufferError> {
        let buf = self.get_owned(owner, handle)?;
        if !offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
            || !(data.len() as u64).is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
        {
            return Err(BufferError::InvalidAlignment);
        }
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or(BufferError::OutOfRange)?;
        if end > buf.byte_len {
            return Err(BufferError::OutOfRange);
        }
        if !data.is_empty() {
            let fields = handle.fields();
            self.pending.push(PendingWrite {
                slot: fields.slot as usize,
                generation: fields.generation,
                offset,
                data: data.to_vec(),
            });
        }
        Ok(())
    }

    /// Apply the queued writes in call order, skipping those of buffers released since.
    pub fn flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.pending.is_empty() {
            return;
        }
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        for write in self.pending.drain(..) {
            if let Some(Some(buf)) = self.slots.get(write.slot)
                && buf.generation == write.generation
            {
                queue.write_buffer(&buf.buffer, write.offset, &write.data);
            }
        }
        if let Some(error) = pollster::block_on(validation.pop()) {
            log::warn!("Sunrise: a mod storage buffer write was refused by the device: {error}");
        }
    }

    /// The size `handle` was created with.
    pub fn size(
        &self,
        owner: Option<&'static str>,
        handle: StorageBufferRef,
    ) -> Result<u64, BufferError> {
        self.get_owned(owner, handle).map(|buf| buf.byte_len)
    }

    /// Free `handle`'s buffer and retire it (`BufferError::Stale` thereafter), dropping its
    /// queued writes. A draw already recorded keeps the buffer alive until the frame is done.
    pub fn release(
        &mut self,
        owner: Option<&'static str>,
        handle: StorageBufferRef,
    ) -> Result<(), BufferError> {
        let generation = self.get_owned(owner, handle)?.generation;
        self.slots[handle.fields().slot as usize] = None;
        self.pending.retain(|w| w.generation != generation);
        self.released.push(generation);
        Ok(())
    }

    /// Release every buffer owned by a mod in `unloaded`.
    pub fn retire_unloaded(&mut self, unloaded: &BTreeSet<String>) {
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|buf| buf.owner.is_some_and(|id| unloaded.contains(id)))
                && let Some(buf) = slot.take()
            {
                self.pending.retain(|w| w.generation != buf.generation);
                self.released.push(buf.generation);
            }
        }
    }

    /// The generations released since the last call.
    pub fn take_released(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.released)
    }

    /// Resolve `slice` for a binding declared with `min_size`, checking ownership, liveness, the
    /// declared minimum and both device limits.
    pub fn resolve(
        &self,
        device_limits: &wgpu::Limits,
        owner: Option<&'static str>,
        slice: StorageBufferSlice,
        min_size: u32,
    ) -> Result<ResolvedStorageRange, BufferError> {
        let buf = self.get_owned(owner, slice.buffer)?;
        if slice.length == 0 {
            return Err(BufferError::Empty);
        }
        if !slice.length.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
            || !slice
                .offset
                .is_multiple_of(u64::from(device_limits.min_storage_buffer_offset_alignment))
        {
            return Err(BufferError::InvalidAlignment);
        }
        if slice.length < u64::from(min_size) {
            return Err(BufferError::BelowMinSize);
        }
        if slice.length > device_limits.max_storage_buffer_binding_size {
            return Err(BufferError::ExceedsBindingLimit);
        }
        let end = slice
            .offset
            .checked_add(slice.length)
            .ok_or(BufferError::OutOfRange)?;
        if end > buf.byte_len {
            return Err(BufferError::OutOfRange);
        }
        Ok(ResolvedStorageRange {
            id: buf.generation,
            buffer: buf.buffer.clone(),
            offset: slice.offset,
            length: slice.length,
        })
    }

    /// The whole buffer of `handle`, checking ownership and liveness (an indirect dispatch reads
    /// its counts from it).
    pub fn resolve_buffer(
        &self,
        owner: Option<&'static str>,
        handle: StorageBufferRef,
    ) -> Result<wgpu::Buffer, BufferError> {
        self.get_owned(owner, handle).map(|buf| buf.buffer.clone())
    }

    fn get_owned(
        &self,
        owner: Option<&'static str>,
        handle: StorageBufferRef,
    ) -> Result<&ModStorageBuffer, BufferError> {
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

impl super::mod_gpu::ModGpuResources {
    /// Allocate a retained GPU [`StorageBufferRef`] on this renderer's device.
    pub fn create_storage_buffer(
        &self,
        owner: Option<&'static str>,
        desc: &StorageBufferDescriptor,
    ) -> Result<StorageBufferRef, BufferError> {
        self.storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .create(&self.device, owner, desc)
    }

    /// Queue a write into a retained storage buffer.
    pub fn storage_buffer_write_range(
        &self,
        owner: Option<&'static str>,
        buffer: StorageBufferRef,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BufferError> {
        self.storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_range(owner, buffer, offset, data)
    }

    /// The size a retained storage buffer was created with.
    pub fn storage_buffer_size(
        &self,
        owner: Option<&'static str>,
        buffer: StorageBufferRef,
    ) -> Result<u64, BufferError> {
        self.storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .size(owner, buffer)
    }

    /// Free a retained storage buffer.
    pub fn storage_buffer_release(
        &self,
        owner: Option<&'static str>,
        buffer: StorageBufferRef,
    ) -> Result<(), BufferError> {
        self.storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .release(owner, buffer)
    }

    /// Free the retained storage buffers of mods that unloaded.
    pub fn retire_unloaded_mod_storage(&self, unloaded: &BTreeSet<String>) {
        self.storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retire_unloaded(unloaded);
    }

    /// Resolve a storage buffer range for a draw call whose entry declares `min_size`.
    pub fn resolve_mod_storage_range(
        &self,
        owner: Option<&'static str>,
        slice: StorageBufferSlice,
        min_size: u32,
    ) -> Result<ResolvedStorageRange, BufferError> {
        self.storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve(&self.device.limits(), owner, slice, min_size)
    }

    /// The buffer of `handle` for an indirect dispatch.
    pub fn resolve_mod_storage_buffer(
        &self,
        owner: Option<&'static str>,
        handle: StorageBufferRef,
    ) -> Result<wgpu::Buffer, BufferError> {
        self.storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_buffer(owner, handle)
    }

    /// Apply the queued storage buffer writes, in order, before the frame draws.
    pub(super) fn flush_mod_storage(&self) {
        self.storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .flush(&self.device, &self.queue);
    }

    /// The generations of the storage buffers released since the last call.
    pub(super) fn take_released_mod_storage(&self) -> Vec<u64> {
        self.storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take_released()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> wgpu::Limits {
        wgpu::Limits {
            min_storage_buffer_offset_alignment: 256,
            max_storage_buffer_binding_size: 1024,
            ..wgpu::Limits::default()
        }
    }

    #[test]
    #[ignore = "daily"]
    fn writes_and_bindings_refuse_what_the_device_limits_and_the_handle_forbid() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((_, device, _queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };
        let mut store = ModStorageBuffers::default();
        let handle = store
            .create(&device, None, &StorageBufferDescriptor::new(4096))
            .unwrap();
        assert_eq!(
            store.create(&device, None, &StorageBufferDescriptor::new(0)),
            Err(BufferError::Empty)
        );
        assert_eq!(
            store.create(&device, None, &StorageBufferDescriptor::new(6)),
            Err(BufferError::InvalidAlignment)
        );
        assert_eq!(
            store.create(
                &device,
                None,
                &StorageBufferDescriptor::new(device.limits().max_buffer_size + 4)
            ),
            Err(BufferError::ExceedsDeviceLimit)
        );
        // write_range: alignment, range, then queued in order and dropped by release.
        assert_eq!(
            store.write_range(None, handle, 2, &[0; 4]),
            Err(BufferError::InvalidAlignment)
        );
        assert_eq!(
            store.write_range(None, handle, 0, &[0; 6]),
            Err(BufferError::InvalidAlignment)
        );
        assert_eq!(
            store.write_range(None, handle, 4092, &[0; 8]),
            Err(BufferError::OutOfRange)
        );
        assert_eq!(
            store.write_range(None, handle, u64::MAX - 3, &[0; 8]),
            Err(BufferError::OutOfRange)
        );
        assert_eq!(
            store.write_range(Some("other"), handle, 0, &[0; 4]),
            Err(BufferError::NotOwned)
        );
        store.write_range(None, handle, 0, &[1; 8]).unwrap();
        store.write_range(None, handle, 4, &[2; 8]).unwrap();
        assert_eq!(store.pending.len(), 2);
        assert_eq!(store.size(None, handle), Ok(4096));

        // Bindings: the range limit and the offset alignment are separate from the buffer size.
        let limits = limits();
        assert!(
            store
                .resolve(&limits, None, handle.slice(256, 1024), 16)
                .is_ok()
        );
        assert_eq!(
            store
                .resolve(&limits, None, handle.slice(0, 2048), 16)
                .err(),
            Some(BufferError::ExceedsBindingLimit)
        );
        assert_eq!(
            store
                .resolve(&limits, None, handle.slice(128, 16), 16)
                .err(),
            Some(BufferError::InvalidAlignment)
        );
        assert_eq!(
            store
                .resolve(&limits, None, handle.slice(3840, 512), 16)
                .err(),
            Some(BufferError::OutOfRange)
        );
        assert_eq!(
            store.resolve(&limits, None, handle.slice(0, 0), 16).err(),
            Some(BufferError::Empty)
        );
        // A range shorter than the entry's `min_size` is refused here, not inside the render pass.
        assert_eq!(
            store.resolve(&limits, None, handle.slice(0, 4), 16).err(),
            Some(BufferError::BelowMinSize)
        );
        assert!(
            store
                .resolve(&limits, None, handle.slice(0, 16), 16)
                .is_ok()
        );

        store.release(None, handle).unwrap();
        assert!(store.pending.is_empty());
        assert_eq!(store.take_released(), vec![1]);
        assert_eq!(store.size(None, handle), Err(BufferError::Stale));
        assert_eq!(
            store.write_range(None, handle, 0, &[0; 4]),
            Err(BufferError::Stale)
        );
        assert_eq!(store.release(None, handle), Err(BufferError::Stale));
    }
}
