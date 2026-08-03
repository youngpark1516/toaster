//! GPU image readback.
use anyhow::Result;

/// Maps a completed readback buffer, copies its float RGBA pixels, and unmaps it.
///
/// The caller must have waited for the output-to-readback copy and must size the
/// buffer as a whole number of `[f32; 4]` pixels.
pub fn readback_pixels(device: &wgpu::Device, readback: &wgpu::Buffer) -> Result<Vec<[f32; 4]>> {
    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).expect("Failed to send map result");
    });

    device.poll(wgpu::PollType::Wait)?;

    receiver.recv().expect("Failed to receive map result")?;

    let data = slice.get_mapped_range();
    let pixels: Vec<[f32; 4]> = bytemuck::cast_slice(&data).to_vec();
    drop(data);
    readback.unmap();

    Ok(pixels)
}
