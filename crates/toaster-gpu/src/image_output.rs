use anyhow::{ensure, Context, Result};

use image::RgbaImage;
use std::path::Path;

pub fn pixels_to_rgba8(pixels: &[[f32; 4]], width: u32, height: u32) -> Result<Vec<u8>> {
    let expected_pixels = (width as usize)
        .checked_mul(height as usize)
        .context("image dimensions are too large")?;
    ensure!(
        pixels.len() == expected_pixels,
        "received {} pixels for a {}x{} image",
        pixels.len(),
        width,
        height
    );

    let mut rgba8 = Vec::with_capacity(expected_pixels * 4);
    for color in pixels {
        rgba8.extend(color.map(f32_to_u8));
    }
    Ok(rgba8)
}

pub fn save_pixels_to_png(
    pixels: &[[f32; 4]],
    width: u32,
    height: u32,
    out_path: &Path,
) -> Result<()> {
    let rgba8 = pixels_to_rgba8(pixels, width, height)?;
    let img = RgbaImage::from_raw(width, height, rgba8)
        .context("converted pixel count does not match image dimensions")?;
    img.save(out_path)?;
    Ok(())
}

fn f32_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_float_pixels_to_rgba8() {
        let converted =
            pixels_to_rgba8(&[[0.0, 0.5, 1.0, 1.5], [-1.0, 0.25, 0.75, 1.0]], 2, 1).unwrap();

        assert_eq!(converted, [0, 128, 255, 255, 0, 64, 191, 255]);
        assert!(pixels_to_rgba8(&[[0.0; 4]], 2, 1).is_err());
    }
}
