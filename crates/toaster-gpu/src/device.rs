//! GPU adapter and device setup.
use anyhow::Result;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuAdapterInfo {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
}

impl From<&wgpu::AdapterInfo> for GpuAdapterInfo {
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

pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: GpuAdapterInfo,
}

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

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("Toaster GPU Device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
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
