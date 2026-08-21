//! Renderer-independent mathematical and image primitives shared by Toaster's
//! CPU-side components.

/// Validated pinhole-camera construction and primary-ray generation.
pub mod camera;
/// Linear-HDR exposure, tone mapping, and display-color conversion helpers.
pub mod color;
/// CPU linear-color image storage and PNG output.
pub mod image_buffer;
/// Rays used by the CPU renderer and intersection routines.
pub mod ray;
