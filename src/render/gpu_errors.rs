//! GPU errors that wgpu reports outside any error scope, and device loss.
//!
//! Without a handler one validation error panics the module (on the web, the whole viewer). The
//! handler records the message instead, and the frame loop surfaces it (status line, QA
//! `lastError`) and keeps running.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Shared between the wgpu callbacks (which may run on any thread) and the frame loop.
#[derive(Clone, Default)]
pub struct GpuErrorSink {
    messages: Arc<Mutex<Vec<String>>>,
    device_lost: Arc<AtomicBool>,
}

impl GpuErrorSink {
    /// Installs the uncaptured-error and device-lost handlers on `device`.
    pub fn install(device: &wgpu::Device) -> Self {
        let sink = Self::default();
        let errors = sink.clone();
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            log::error!("GPU error: {error}");
            errors.push(format!("GPU error: {error}"));
        }));
        let lost = sink.clone();
        device.set_device_lost_callback(move |reason, message| {
            log::error!("GPU device lost ({reason:?}): {message}");
            lost.device_lost.store(true, Ordering::Relaxed);
            lost.push(format!("GPU device lost ({reason:?}): {message}"));
        });
        sink
    }

    fn push(&self, message: String) {
        if let Ok(mut messages) = self.messages.lock() {
            // A failing draw repeats every frame; keep the first reports.
            if messages.len() < 16 {
                messages.push(message);
            }
        }
    }

    /// The errors reported since the last call, oldest first.
    pub fn take(&self) -> Vec<String> {
        self.messages
            .lock()
            .map(|mut messages| std::mem::take(&mut *messages))
            .unwrap_or_default()
    }

    pub fn device_lost(&self) -> bool {
        self.device_lost.load(Ordering::Relaxed)
    }
}

/// Limits to request from `adapter`: what a WebGL2 adapter can give when that is the backend
/// (the defaults normally fail there), the defaults otherwise.
pub fn required_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    if adapter.get_info().backend == wgpu::Backend::Gl {
        wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())
    } else {
        wgpu::Limits::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn software_device() -> Option<(wgpu::Adapter, wgpu::Device)> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: true,
        }))
        .ok()?;
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some((adapter, device))
    }

    #[test]
    fn test_an_invalid_gpu_call_is_reported_and_not_fatal() {
        let Some((_, device)) = software_device() else {
            return;
        };
        let sink = GpuErrorSink::install(&device);

        // Invalid: a zero-usage buffer.
        let _buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("deliberately invalid"),
            size: 16,
            usage: wgpu::BufferUsages::empty(),
            mapped_at_creation: false,
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());

        let reported = sink.take();
        assert!(
            !reported.is_empty(),
            "the validation error must be reported"
        );
        assert!(reported[0].starts_with("GPU error"));
        assert!(sink.take().is_empty(), "reports are drained once");
        assert!(!sink.device_lost());
    }

    #[test]
    fn test_reports_are_bounded() {
        let sink = GpuErrorSink::default();
        for index in 0..100 {
            sink.push(format!("error {index}"));
        }
        assert_eq!(sink.take().len(), 16);
    }

    #[test]
    fn test_required_limits_suit_the_adapter_backend() {
        let Some((adapter, _)) = software_device() else {
            return;
        };
        let limits = required_limits(&adapter);
        assert!(limits.max_texture_dimension_3d >= 256);
    }
}
