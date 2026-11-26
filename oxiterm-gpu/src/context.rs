//! Shared headless `wgpu` context (`Instance`, `Adapter`, `Device`, `Queue`).
//!
//! Per `OxiUI`'s "one device, several users" rule (`docs/oxi-ui.md`), [`GpuContext`] is owned
//! separately from [`crate::GpuSink`] (typically behind an `Arc<GpuContext>`) so that the
//! `DisplayList` renderer (Stage 5.2+), future `OxiScript` GPU kernels (Stage 8.3b), and 3D
//! rendering (Stage 9.3) all share one device and queue without CPU round-trips.

use std::fmt;
use std::sync::{Arc, OnceLock};

/// Errors returned while creating a [`GpuContext`] or rendering a frame.
#[derive(Debug, Clone)]
pub enum GpuError {
    /// No compatible `wgpu` adapter is available on this machine/environment.
    NoAdapter(String),
    /// Creating the logical `wgpu::Device` failed.
    DeviceRequest(String),
    /// The `wgpu::Device` was lost or destroyed.
    DeviceLost(String),
    /// Surface dimensions are zero or exceed the device or safety limits.
    InvalidSurfaceSize { width: u32, height: u32, max_dim: u32 },
    /// Caller-supplied render target texture view is incompatible with the pipeline.
    InvalidTarget(String),
    /// `wgpu` reported a validation error during rendering or resource creation.
    Validation(String),
    /// `wgpu` reported an out-of-memory error during rendering or resource creation.
    OutOfMemory(String),
    /// `wgpu` reported an internal backend error during rendering or resource creation.
    Internal(String),
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
            Self::DeviceLost(msg) => write!(f, "GPU device lost: {msg}"),
            Self::InvalidSurfaceSize { width, height, max_dim } => {
                write!(f, "invalid GPU surface size {width}x{height} (max dimension {max_dim})")
            }
            Self::InvalidTarget(msg) => write!(f, "invalid GPU render target: {msg}"),
            Self::Validation(msg) => write!(f, "GPU validation error: {msg}"),
            Self::OutOfMemory(msg) => write!(f, "GPU out of memory: {msg}"),
            Self::Internal(msg) => write!(f, "GPU internal error: {msg}"),
            Self::Readback(msg) => write!(f, "GPU readback failed: {msg}"),
            Self::Png(msg) => write!(f, "PNG error: {msg}"),
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for GpuError {}

fn map_wgpu_error(err: wgpu::Error, lost: &OnceLock<String>) -> GpuError {
    if let Some(reason) = lost.get() {
        return GpuError::DeviceLost(reason.clone());
    }
    match err {
        wgpu::Error::OutOfMemory { source } => GpuError::OutOfMemory(source.to_string()),
        wgpu::Error::Validation { description, .. } => {
            if is_device_lost_message(&description) {
                let _ = lost.set(description.clone());
                GpuError::DeviceLost(description)
            } else {
                GpuError::Validation(description)
            }
        }
        wgpu::Error::Internal { description, .. } => {
            if is_device_lost_message(&description) {
                let _ = lost.set(description.clone());
                GpuError::DeviceLost(description)
            } else {
                GpuError::Internal(description)
            }
        }
    }
}

fn is_device_lost_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("device")
        && (lower.contains("lost") || lower.contains("destroyed") || lower.contains("invalid"))
}

/// Options for constructing a shared [`GpuContext`].
///
/// [`Default`] selects the GLES 3.0 / WebGL2 baseline (`Features::empty()`,
/// `Limits::downlevel_webgl2_defaults()` raised to the adapter's texture resolution,
/// environment-configured backends, and no power preference). Future subsystems such as
/// `OxiScript` GPU kernels (Stage 8.3b) or 3D rendering (Stage 9.3) can request additional
/// features or higher limits through [`GpuContext::new_headless_with_options`].
#[derive(Debug, Clone)]
pub struct GpuContextOptions {
    /// Backends to enable. `None` uses `wgpu`'s environment selection (`WGPU_BACKEND`).
    pub backends: Option<wgpu::Backends>,
    /// Power preference passed to `request_adapter`.
    pub power_preference: wgpu::PowerPreference,
    /// Required `wgpu::Features` on the logical device (default: `Features::empty()`).
    pub required_features: wgpu::Features,
    /// Required `wgpu::Limits` on the logical device. `None` uses
    /// `Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())`.
    pub required_limits: Option<wgpu::Limits>,
}

impl Default for GpuContextOptions {
    fn default() -> Self {
        Self {
            backends: None,
            power_preference: wgpu::PowerPreference::None,
            required_features: wgpu::Features::empty(),
            required_limits: None,
        }
    }
}

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
    device_lost: Arc<OnceLock<String>>,
}

impl GpuContext {
    /// Creates a headless GPU context synchronously with default WebGL2-level options.
    ///
    /// Respects standard `wgpu` / Vulkan environment variables (such as `WGPU_BACKEND` or Mesa's
    /// software Vulkan ICD) and never opens `/dev/dri`, `/dev/fb*`, or `/dev/input` directly.
    /// Returns `Err(GpuError::NoAdapter(_))` when no adapter exists.
    pub fn new_headless() -> Result<Self, GpuError> {
        Self::new_headless_with_options(&GpuContextOptions::default())
    }

    /// Creates a headless GPU context synchronously with custom [`GpuContextOptions`].
    pub fn new_headless_with_options(options: &GpuContextOptions) -> Result<Self, GpuError> {
        pollster::block_on(Self::new_headless_with_options_async(options))
    }

    /// Convenience helper returning `None` when no adapter is available.
    pub fn try_new_headless() -> Option<Self> {
        Self::new_headless().ok()
    }

    /// Creates a headless GPU context asynchronously with default WebGL2-level options.
    pub async fn new_headless_async() -> Result<Self, GpuError> {
        Self::new_headless_with_options_async(&GpuContextOptions::default()).await
    }

    /// Creates a headless GPU context asynchronously with custom [`GpuContextOptions`].
    pub async fn new_headless_with_options_async(
        options: &GpuContextOptions,
    ) -> Result<Self, GpuError> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        if let Some(backends) = options.backends {
            desc.backends = backends;
        }
        let instance = wgpu::Instance::new(desc);

        // Try the environment's default adapter first; fall back to software/fallback adapters
        // (e.g. Mesa lavapipe in headless CI) if no default hardware adapter is returned.
        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: options.power_preference,
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
        {
            Ok(a) => a,
            Err(first_err) => instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: options.power_preference,
                    force_fallback_adapter: true,
                    ..Default::default()
                })
                .await
                .map_err(|fallback_err| {
                    GpuError::NoAdapter(format!("{first_err}; fallback: {fallback_err}"))
                })?,
        };

        let info = adapter.get_info();

        // By default, stay within GLES 3.0 / WebGL2 downlevel limits (raising texture resolution
        // limits to whatever the adapter supports).
        let required_limits = options.required_limits.clone().unwrap_or_else(|| {
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())
        });

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("oxiterm-gpu-device"),
                required_features: options.required_features,
                required_limits,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| GpuError::DeviceRequest(e.to_string()))?;

        let device_lost = Arc::new(OnceLock::new());
        let lost_flag = Arc::clone(&device_lost);
        device.set_device_lost_callback(move |reason, msg| {
            let _ = lost_flag.set(format!("{reason:?}: {msg}"));
        });

        Ok(Self { instance, adapter, device, queue, info, device_lost })
    }

    /// Returns `true` if the underlying `wgpu::Device` has been lost or destroyed.
    pub fn is_device_lost(&self) -> bool {
        self.device_lost.get().is_some()
    }

    /// Returns `Err(GpuError::DeviceLost(_))` if the device has been lost or destroyed.
    pub fn check_device_lost(&self) -> Result<(), GpuError> {
        if let Some(reason) = self.device_lost.get() {
            return Err(GpuError::DeviceLost(reason.clone()));
        }
        Ok(())
    }

    /// Runs `f` inside thread-local `wgpu` error scopes (`Internal`, `OutOfMemory`, `Validation`)
    /// so errors are captured and returned as [`GpuError`] rather than panicking or leaking into a
    /// co-user's scope or uncaptured-error handler.
    pub(crate) fn with_error_scopes<T>(
        &self,
        f: impl FnOnce() -> Result<T, GpuError>,
    ) -> Result<T, GpuError> {
        self.check_device_lost()?;
        let s_int = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let s_oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let s_val = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let res = f();
        // Pop in strict LIFO order matching push order.
        let val_err = pollster::block_on(s_val.pop());
        let oom_err = pollster::block_on(s_oom.pop());
        let int_err = pollster::block_on(s_int.pop());
        if let Some(wgpu_err) = val_err.or(oom_err).or(int_err) {
            let _ = self.device.poll(wgpu::PollType::Poll);
            return Err(map_wgpu_error(wgpu_err, &self.device_lost));
        }
        if res.is_err() {
            let _ = self.device.poll(wgpu::PollType::Poll);
            self.check_device_lost()?;
        }
        res
    }

    /// Runs `f` inside `wgpu` error scopes even if the device is already lost, returning both the
    /// result of `f` and any captured error (used by non-fallible constructors such as
    /// [`crate::GpuSink::new`]).
    pub(crate) fn capture_error_scopes<T>(&self, f: impl FnOnce() -> T) -> (T, Option<GpuError>) {
        let s_int = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let s_oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let s_val = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let val = f();
        let val_err = pollster::block_on(s_val.pop());
        let oom_err = pollster::block_on(s_oom.pop());
        let int_err = pollster::block_on(s_int.pop());
        let err = if let Some(reason) = self.device_lost.get() {
            Some(GpuError::DeviceLost(reason.clone()))
        } else if let Some(wgpu_err) = val_err.or(oom_err).or(int_err) {
            let _ = self.device.poll(wgpu::PollType::Poll);
            Some(map_wgpu_error(wgpu_err, &self.device_lost))
        } else {
            None
        };
        (val, err)
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
