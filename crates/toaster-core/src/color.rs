//! Shared conversion from linear HDR radiance to display-referred sRGB.

use glam::Vec3;

/// Filmic operator applied after exposure and before the sRGB transfer function.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToneMapper {
    /// Academy Color Encoding System fitted curve by Krzysztof Narkowicz.
    #[default]
    Aces,
}

/// Renderer-neutral controls for converting linear HDR radiance to display RGB.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DisplaySettings {
    /// Exposure compensation in stops. One stop doubles linear radiance.
    pub exposure_stops: f32,
    /// Filmic curve used to compress HDR radiance into the display range.
    pub tone_mapper: ToneMapper,
}

/// Applies exposure compensation to one linear HDR RGB value.
pub fn apply_exposure(color: Vec3, exposure_stops: f32) -> Vec3 {
    color * exposure_stops.exp2()
}

/// Applies the ACES fitted filmic curve to one linear HDR RGB value.
pub fn aces_fitted(color: Vec3) -> Vec3 {
    color.map(aces_fitted_channel)
}

/// Converts one nonnegative linear-light channel to sRGB.
pub fn linear_to_srgb(channel: f32) -> f32 {
    if channel <= 0.003_130_8 {
        12.92 * channel
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

/// Converts one linear HDR RGB value into display-referred sRGB bytes.
pub fn linear_hdr_to_rgb8(color: Vec3, settings: DisplaySettings) -> [u8; 3] {
    let exposed = apply_exposure(color, settings.exposure_stops);
    let tone_mapped = match settings.tone_mapper {
        ToneMapper::Aces => aces_fitted(exposed),
    }
    .clamp(Vec3::ZERO, Vec3::ONE);
    let srgb = tone_mapped.map(linear_to_srgb);
    [
        channel_to_u8(srgb.x),
        channel_to_u8(srgb.y),
        channel_to_u8(srgb.z),
    ]
}

/// Backwards-compatible display conversion using default settings.
pub fn linear_to_rgb8(color: Vec3) -> [u8; 3] {
    linear_hdr_to_rgb8(color, DisplaySettings::default())
}

fn aces_fitted_channel(channel: f32) -> f32 {
    if channel.is_nan() {
        return 0.0;
    }
    if !channel.is_finite() {
        return if channel.is_sign_positive() { 1.0 } else { 0.0 };
    }
    let channel = channel.max(0.0);
    (channel * (2.51 * channel + 0.03)) / (channel * (2.43 * channel + 0.59) + 0.14)
}

fn channel_to_u8(channel: f32) -> u8 {
    (channel.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_to_srgb_matches_known_values() {
        assert_eq!(linear_to_srgb(0.0), 0.0);
        assert!((linear_to_srgb(0.003_130_8) - 0.040_449_936).abs() < 1.0e-7);
        assert!((linear_to_srgb(0.18) - 0.461_356_13).abs() < 1.0e-6);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn exposure_scales_by_powers_of_two() {
        let color = Vec3::new(0.125, 0.5, 2.0);
        assert_eq!(apply_exposure(color, 2.0), color * 4.0);
        assert_eq!(apply_exposure(color, -1.0), color * 0.5);
    }

    #[test]
    fn aces_is_finite_monotonic_and_rolls_off_bright_values() {
        let mapped = [0.0, 0.18, 1.0, 4.0, 16.0].map(|value| aces_fitted(Vec3::splat(value)).x);
        assert!(mapped.iter().all(|value| value.is_finite()));
        assert!(mapped.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(mapped[3] < 1.0);
        assert!(mapped[4] > mapped[3]);
    }

    #[test]
    fn display_conversion_handles_nonfinite_radiance() {
        assert_eq!(
            linear_hdr_to_rgb8(
                Vec3::new(f32::NAN, f32::NEG_INFINITY, f32::INFINITY),
                DisplaySettings::default(),
            ),
            [0, 0, 255]
        );
    }
}
