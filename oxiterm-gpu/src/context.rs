//! Shared headless `wgpu` context (`Instance`, `Adapter`, `Device`, `Queue`).
//!
//! Per `OxiUI`'s "one device, several users" rule (`docs/oxi-ui.md`), [`GpuContext`] is owned
//! separately from [`crate::GpuSink`] (typically behind an `Arc<GpuContext>`) so that the
//! `DisplayList` renderer (Stage 5.2+), future `OxiScript` GPU kernels (Stage 8.3b), and 3D
//! rendering (Stage 9.3) all share one device and queue without CPU round-trips.

use std::fmt;

/// Errors returned while creating a [`GpuContext`] or rendering a frame.
#[derive(Debug)]
pub enum GpuError {
    /// No compatible `wgpu` adapter is available on this machine/environment.
    NoAdapter(String),
    /// Creating the logical `wgpu::Device` failed.
    DeviceRequest(String),
    /// Surface dimensions are zero or exceed the device or safety limits.
    InvalidSurfaceSize { width: u32, height: u32, max_dim: u32 },
    /// Reading back the rendered texture from the GPU failed.
    Readback(String),
    /// PNG encoding or decoding failed.
    Png(String),
    /// I/O error while reading or writing a file.
    Io(String),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter(msg) => write!(f, "no compatible GPU adapter available: {msg}"),
            Self::DeviceRequest(msg) => write!(f, "failed to create wgpu device: {msg}"),
            Self::InvalidSurfaceSize { width, height, max_dim } => {
                write!(f, "invalid GPU surface size {width}x{height} (max dimension {max_dim})")
            }
            Self::Readback(msg) => write!(f, "GPU readback failed: {msg}"),
            Self::Png(msg) => write!(f, "PNG error: {msg}"),
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for GpuError {}

/// Shared GPU context holding the `wgpu` instance, adapter, device, and queue.
///
/// Created headless (no window or display handle required). Pass `Arc<GpuContext>` to
/// [`crate::GpuSink::new`] and any future GPU subsystems so they share resources on one device.
pub struct GpuContext {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
}

impl GpuContext {
    /// Creates a headless GPU context synchronously (blocking on `pollster`).
    ///
    /// Respects standard `wgpu` / Vulkan environment variables (such as `WGPU_BACKEND` or Mesa's
    /// software Vulkan ICD) and never opens `/dev/dri`, `/dev/fb*`, or `/dev/input` directly.
    /// Returns `Err(GpuError::NoAdapter(_))` when no adapter exists.
    pub fn new_headless() -> Result<Self, GpuError> {
        pollster::block_on(Self::new_headless_async())
    }

    /// Convenience helper returning `None` when no adapter is available.
    pub fn try_new_headless() -> Option<Self> {
        Self::new_headless().ok()
    }

    /// Creates a headless GPU context asynchronously.
    pub async fn new_headless_async() -> Result<Self, GpuError> {
        let desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        let instance = wgpu::Instance::new(desc);

        // Try the environment's default adapter first; fall back to software/fallback adapters
        // (e.g. Mesa lavapipe in headless CI) if no default hardware adapter is returned.
        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
        {
            Ok(a) => a,
            Err(first_err) => instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    force_fallback_adapter: true,
                    ..Default::default()
                })
                .await
                .map_err(|fallback_err| {
                    GpuError::NoAdapter(format!("{first_err}; fallback: {fallback_err}"))
                })?,
        };

        let info = adapter.get_info();

        // Keep required features empty and stay within GLES 3.0 / WebGL2 downlevel limits
        // (raising texture resolution limits to whatever the adapter supports).
        let required_limits =
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("oxiterm-gpu-device"),
                required_features: wgpu::Features::empty(),
                required_limits,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| GpuError::DeviceRequest(e.to_string()))?;

        Ok(Self { instance, adapter, device, queue, info })
    }

    /// The underlying `wgpu::Instance`.
    pub fn instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    /// The selected `wgpu::Adapter`.
    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    /// The shared `wgpu::Device`.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The shared `wgpu::Queue`.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Metadata about the active adapter (name, backend, device type, driver).
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.info
    }

    /// Effective device limits.
    pub fn limits(&self) -> wgpu::Limits {
        self.device.limits()
    }
}

impl fmt::Debug for GpuContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuContext")
            .field("adapter", &self.info.name)
            .field("backend", &self.info.backend)
            .field("device_type", &self.info.device_type)
            .finish_non_exhaustive()
    }
}
