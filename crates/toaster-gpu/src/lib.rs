pub mod animation;
pub mod buffers;
pub mod device;
pub mod dispatch;
pub mod gpu_types;
pub mod gradient;
pub mod image_output;
pub mod path_tracer;
pub mod pipeline;
pub mod readback;
pub mod scene_upload;
mod video_output;

pub use animation::{frame_output_path, AnimationConfig};
pub use device::{create_gpu_context, GpuAdapterInfo, GpuContext};
pub use gradient::render_gradient;
pub use path_tracer::{
    benchmark_gpu_scene, render_gpu_animation_with_sink, render_gpu_progressive_with_sink,
    render_scene_gpu_animation, render_scene_gpu_animation_with_sink, render_scene_gpu_video,
    CompletedFrame, FramePacing, FrameSink, GpuBenchmarkConfig, GpuBenchmarkResult,
    GpuFrameTimings, ProgressiveRenderConfig,
};
