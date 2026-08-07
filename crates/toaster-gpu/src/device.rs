//! GPU adapter and device setup.
use anyhow::{ensure, Result};

const REQUIRED_STORAGE_BUFFERS_PER_SHADER_STAGE: u32 = 10;

#[derive(Clone, Debug, Eq, PartialEq)]
/// Stable, serializable-style metadata for the selected adapter.
pub struct GpuAdapterInfo {
    /// Adapter-reported device name.
    pub name: String,
    /// Backend name such as `Vulkan`, `Metal`, or `Gl`.
    pub backend: String,
    /// Adapter class such as discrete, integrated, virtual, or CPU.
    pub device_type: String,
    /// Adapter-reported driver name.
    pub driver: String,
    /// Adapter-reported driver version or descriptive details.
    pub driver_info: String,
}

impl From<&wgpu::AdapterInfo> for GpuAdapterInfo {
    /// Copies wgpu metadata while converting enums to stable debug strings.
    fn from(info: &wgpu::AdapterInfo) -> Self {
        Self {
            name: info.name.clone(),
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
            driver: info.driver.clone(),
            driver_info: info.driver_info.clone(),
        }
    }
}

/// Instance, selected adapter, logical device, queue, and copied metadata.
pub struct GpuContext {
    /// Backend instance kept alive for the adapter and device lifetime.
    pub instance: wgpu::Instance,
    /// Selected high-performance headless adapter.
    pub adapter: wgpu::Adapter,
    /// Logical device used for all resources and polling.
    pub device: wgpu::Device,
    /// Submission and buffer-update queue.
    pub queue: wgpu::Queue,
    /// Copied adapter description used by logs and benchmark reports.
    pub info: GpuAdapterInfo,
}

/// Requests a high-performance headless adapter and the renderer's buffer limits.
pub async fn create_gpu_context() -> Result<GpuContext> {
    let instance = wgpu::Instance::default();

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await?;

    let adapter_info = adapter.get_info();
    let info = GpuAdapterInfo::from(&adapter_info);
    tracing::info!(
        gpu.name = %info.name,
        gpu.backend = %info.backend,
        gpu.device_type = %info.device_type,
        gpu.driver = %info.driver,
        "selected GPU adapter"
    );

    let adapter_limits = adapter.limits();
    ensure!(
        adapter_limits.max_storage_buffers_per_shader_stage
            >= REQUIRED_STORAGE_BUFFERS_PER_SHADER_STAGE,
        "selected GPU supports {} compute storage buffers, but Toaster requires {}",
        adapter_limits.max_storage_buffers_per_shader_stage,
        REQUIRED_STORAGE_BUFFERS_PER_SHADER_STAGE
    );
    let required_limits = wgpu::Limits {
        max_storage_buffers_per_shader_stage: REQUIRED_STORAGE_BUFFERS_PER_SHADER_STAGE,
        ..wgpu::Limits::default()
    };

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("Toaster GPU Device"),
            required_features: wgpu::Features::empty(),
            required_limits,
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        })
        .await?;

    Ok(GpuContext {
        instance,
        adapter,
        device,
        queue,
        info,
    })
}
