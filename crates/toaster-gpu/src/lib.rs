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
pub mod animation;

pub use device::{create_gpu_context, GpuContext};
pub use gradient::render_gradient;
pub use path_tracer::render_scene_gpu_animation;
pub use animation::{frame_output_path, AnimationConfig};
