pub mod contours;
pub mod geometry;
pub mod gpu_errors;
pub mod meshes;
pub mod pipeline;
pub mod protocols;
pub mod roi_views;
pub mod view3d_cache;

/// A device for GPU unit tests: a software adapter if there is one, otherwise the machine's own.
/// `None` (the test then does nothing) only when the machine has no adapter at all.
#[cfg(test)]
pub(crate) fn test_device() -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    for force_fallback_adapter in [true, false] {
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter,
            }))
        else {
            continue;
        };
        if let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        {
            return Some((adapter, device, queue));
        }
    }
    eprintln!("no GPU adapter on this machine: GPU unit tests are skipped");
    None
}
