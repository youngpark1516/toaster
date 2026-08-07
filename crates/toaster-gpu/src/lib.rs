//! Headless `wgpu` path tracing, frame sinks, image conversion, streaming
//! support, video output, and repeatable benchmarking.

/// Finite and indefinite frame schedules plus output naming.
pub mod animation;
/// GPU buffer allocation and binding resources.
pub mod buffers;
/// Adapter and device selection.
pub mod device;
/// Synchronous compute dispatch and output-buffer copying.
pub mod dispatch;
/// Rust structures whose layouts mirror active WGSL structures.
pub mod gpu_types;
/// Small standalone compute-gradient demonstration.
pub mod gradient;
/// Float-pixel conversion and PNG/JPEG encoding.
pub mod image_output;
/// Main path-tracing loop, frame sinks, progressive rendering, and benchmarks.
pub mod path_tracer;
/// Bind-group layouts, bind groups, shaders, and compute pipelines.
pub mod pipeline;
/// Mapping and copying GPU readback buffers into CPU memory.
pub mod readback;
/// Renderer-neutral scene conversion into GPU structures.
pub mod scene_upload;
mod video_output;

pub use animation::{frame_output_path, AnimationConfig};
pub use device::{create_gpu_context, GpuAdapterInfo, GpuContext};
pub use gradient::render_gradient;
pub use path_tracer::{
    benchmark_gpu_scene, render_evaluated_gpu_animation, render_evaluated_gpu_animation_with_sink,
    render_evaluated_gpu_video, render_gpu_animation_with_sink, render_gpu_progressive_with_sink,
    render_scene_gpu_animation, render_scene_gpu_animation_with_sink, render_scene_gpu_video,
    CompletedFrame, FramePacing, FrameSink, GpuBenchmarkConfig, GpuBenchmarkResult,
    GpuFrameTimings, ProgressiveRenderConfig,
};
