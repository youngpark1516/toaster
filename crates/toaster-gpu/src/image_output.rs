//! Conversion from linear float GPU pixels to shared RGBA8 images and encoders.

use anyhow::{ensure, Context, Result};

use image::{codecs::jpeg::JpegEncoder, ImageBuffer, Rgba, RgbaImage};
use std::path::Path;
use toaster_core::color::{linear_hdr_to_rgb8, DisplaySettings};

/// Converts float pixels once and writes the resulting RGBA image to disk.
pub fn save_pixels_to_png(
    pixels: &[[f32; 4]],
    width: u32,
    height: u32,
    display: DisplaySettings,
    out_path: &Path,
) -> Result<()> {
    let img = pixels_to_rgba_image(pixels, width, height, display)?;
    save_rgba_image_to_png(&img, out_path)
}

/// Saves an already converted RGBA image using the image crate's path format.
pub fn save_rgba_image_to_png(image: &RgbaImage, out_path: &Path) -> Result<()> {
    image.save(out_path)?;
    Ok(())
}

/// Converts float pixels and JPEG-encodes them at quality `1..=100`.
pub fn encode_pixels_to_jpeg(
    pixels: &[[f32; 4]],
    width: u32,
    height: u32,
    quality: u8,
    display: DisplaySettings,
) -> Result<Vec<u8>> {
    ensure!(
        (1..=100).contains(&quality),
        "JPEG quality must be between 1 and 100"
    );

    let img = pixels_to_rgba_image(pixels, width, height, display)?;
    encode_rgba_image_to_jpeg(&img, quality)
}

/// JPEG-encodes an existing RGBA image at quality `1..=100`.
pub fn encode_rgba_image_to_jpeg(image: &RgbaImage, quality: u8) -> Result<Vec<u8>> {
    ensure!(
        (1..=100).contains(&quality),
        "JPEG quality must be between 1 and 100"
    );

    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, quality)
        .encode_image(image)
        .context("failed to encode JPEG frame")?;
    Ok(jpeg)
}

/// Validates dimensions and converts row-major float RGBA into an [`RgbaImage`].
pub fn pixels_to_rgba_image(
    pixels: &[[f32; 4]],
    width: u32,
    height: u32,
    display: DisplaySettings,
) -> Result<RgbaImage> {
    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .context("image dimensions are too large")?;
    ensure!(
        pixels.len() == pixel_count,
        "pixel buffer length {} does not match image dimensions {}x{}",
        pixels.len(),
        width,
        height
    );

    let bytes = pixels
        .iter()
        .flat_map(|color| {
            let rgb = linear_hdr_to_rgb8(glam::Vec3::new(color[0], color[1], color[2]), display);
            [rgb[0], rgb[1], rgb[2], f32_to_u8(color[3])]
        })
        .collect();

    ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, bytes)
        .context("failed to construct image from pixel buffer")
}

/// Clamps a normalized float channel and rounds it to the nearest byte.
fn f32_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    #[test]
    fn converts_float_pixels_to_rgba_once() {
        let pixels = [[-1.0, 0.5, 2.0, 1.0], [0.0, 0.25, 0.75, 0.0]];

        let image = pixels_to_rgba_image(&pixels, 2, 1, DisplaySettings::default()).unwrap();

        assert_eq!(image.as_raw(), &[0, 206, 245, 255, 0, 165, 223, 0]);
    }

    #[test]
    fn rejects_mismatched_pixel_count() {
        let error =
            pixels_to_rgba_image(&[[0.0; 4]], 2, 1, DisplaySettings::default()).unwrap_err();
        assert!(error.to_string().contains("pixel buffer length"));
    }

    #[test]
    fn encodes_decodable_jpeg_with_expected_dimensions() {
        let pixels = vec![[0.25, 0.5, 0.75, 1.0]; 6];

        let jpeg = encode_pixels_to_jpeg(&pixels, 3, 2, 90, DisplaySettings::default()).unwrap();
        let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();

        assert_eq!(decoded.dimensions(), (3, 2));
        assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xff, 0xd9]);
    }

    #[test]
    fn rejects_invalid_jpeg_quality() {
        assert!(encode_pixels_to_jpeg(&[[0.0; 4]], 1, 1, 0, DisplaySettings::default()).is_err());
        assert!(encode_pixels_to_jpeg(&[[0.0; 4]], 1, 1, 101, DisplaySettings::default()).is_err());
    }
}
