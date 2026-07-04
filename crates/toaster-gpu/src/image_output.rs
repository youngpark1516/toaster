use anyhow::Result;

use image::{ImageBuffer, Rgba};
use std::path::Path;

pub fn save_pixels_to_png(
    pixels: &[[f32; 4]],
    width: u32,
    height: u32,
    out_path: &Path,
) -> Result<()> {
    let mut img = ImageBuffer::new(width, height);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let idx = (y * width + x) as usize;
        let color = pixels[idx];
        *pixel = Rgba([
            f32_to_u8(color[0]),
            f32_to_u8(color[1]),
            f32_to_u8(color[2]),
            f32_to_u8(color[3]),
        ]);
    }
    img.save(out_path)?;
    Ok(())
}

fn f32_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
