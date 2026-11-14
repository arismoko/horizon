//! Shared GPU test setup.

use std::sync::{Mutex, MutexGuard};

static GPU_TEST: Mutex<()> = Mutex::new(());

/// Serialize an entire GPU test, including driver initialization and teardown.
///
/// Bind this guard before creating any GPU objects, in the test itself rather
/// than a helper returning those objects. Locals then drop before the guard,
/// including during unwinding; one guard also covers tests opening several devices.
pub(crate) fn lock() -> MutexGuard<'static, ()> {
    GPU_TEST.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
/// Default setup for tests that skip when either the adapter or device is unavailable.
/// The caller must hold [`lock`] until every returned object and GPU resource drops.
pub(crate) fn optional_device() -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let instance = instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).ok()?;
    Some((adapter, device, queue))
}

#[cfg(test)]
/// Find an adapter, reporting an unavailable adapter without hiding other failures.
pub(crate) fn adapter() -> Option<wgpu::Adapter> {
    let instance = instance();
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error @ wgpu::RequestAdapterError::NotFound { .. }) => {
            eprintln!("skipped: no adapter: {error}");
            return None;
        }
        Err(error) => panic!("GPU test adapter request failed: {error}"),
    };
    Some(adapter)
}

#[cfg(test)]
/// Check format usages before constructing a pipeline that needs them.
pub(crate) fn supports_texture_usage(
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> bool {
    if adapter
        .get_texture_format_features(format)
        .allowed_usages
        .contains(usage)
    {
        return true;
    }
    let info = adapter.get_info();
    eprintln!(
        "skipped: {} / {:?} lacks {format:?} {usage:?}",
        info.name, info.backend
    );
    false
}

#[cfg(test)]
/// Naga's GLSL writer cannot lower depth textureLoad (no downlevel flag describes it).
pub(crate) fn supports_depth_load(adapter: &wgpu::Adapter) -> bool {
    let info = adapter.get_info();
    if info.backend != wgpu::Backend::Gl {
        return true;
    }
    eprintln!(
        "skipped: {} / {:?} lacks depth textureLoad in GLSL",
        info.name, info.backend
    );
    false
}

#[cfg(test)]
/// Once capabilities are checked, device errors must fail rather than skip.
pub(crate) fn device(adapter: &wgpu::Adapter) -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(adapter.request_device(&Default::default()))
        .expect("GPU test device creation failed")
}

#[cfg(test)]
/// A new instance for one GPU test. On Windows it opens Vulkan and DirectX 12,
/// not OpenGL: WGL instances made and dropped from many test threads at once
/// deadlocked NVIDIA's driver, and the render tests hung most runs (#5238).
/// The game runs there on Vulkan or DirectX 12 unless neither works.
/// `WGPU_BACKEND` and the other `WGPU_*` variables still apply, as in the game.
pub(crate) fn instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        descriptor.backends = wgpu::Backends::PRIMARY;
    }
    wgpu::Instance::new(descriptor.with_env())
}

#[cfg(test)]
pub(crate) mod culling {
    use crate::gfx::submit::Queue;

    /// A device of `backends` that draws counts with first instances, if this
    /// machine has one; on DirectX 12, `compiler` compiles its shaders. Not
    /// Intel's Gen11 and Gen12 Vulkan driver, which gets first instances wrong
    /// and is never offered GPU culling (`supported`).
    fn device(
        backends: wgpu::Backends,
        compiler: wgpu::Dx12Compiler,
        name: &str,
    ) -> Option<(wgpu::Device, wgpu::Queue)> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = backends;
        desc.backend_options.dx12.shader_compiler = compiler;
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let needed =
            wgpu::Features::MULTI_DRAW_INDIRECT_COUNT | wgpu::Features::INDIRECT_FIRST_INSTANCE;
        let timing = adapter.features()
            & (wgpu::Features::TIMESTAMP_QUERY
                | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES
                | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
                | wgpu::Features::SHADER_EARLY_DEPTH_TEST);
        if !adapter.features().contains(needed)
            || super::super::flat::is_broken_intel_vulkan(&adapter.get_info())
        {
            return None;
        }
        eprintln!("GPU culling on {} ({name})", adapter.get_info().name);
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: needed | timing,
            ..Default::default()
        }))
        .ok()
    }

    /// `test` on each device here that draws counts with first instances:
    /// Vulkan, and DirectX 12 with each of its compilers. Players' DirectX 12
    /// compiles with FXC, part of Windows, and with DXC (`dxcompiler.dll`,
    /// beside the game in the Windows x64 zip) on integrated GPUs
    /// (`gfx/dx12_compiler.rs`).
    pub(crate) fn on_each_device(test: impl Fn(&wgpu::Device, &Queue)) {
        let _gpu = super::lock();
        let mut tested = false;
        for (backends, compiler, name) in [
            (wgpu::Backends::VULKAN, Default::default(), "Vulkan"),
            (
                wgpu::Backends::DX12,
                wgpu::Dx12Compiler::Fxc,
                "DirectX 12, FXC",
            ),
            (
                wgpu::Backends::DX12,
                wgpu::Dx12Compiler::default_dynamic_dxc(),
                "DirectX 12, DXC",
            ),
        ] {
            if let Some((device, queue)) = device(backends, compiler, name) {
                test(&device, &Queue::new(queue, false));
                tested = true;
            }
        }
        if !tested {
            eprintln!("no GPU with indirect count draws: skipped");
        }
    }
}
