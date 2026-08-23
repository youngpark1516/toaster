//! Renderer-independent emitted-power approximations for light sampling.

use glam::Vec3;

/// Returns Rec.709 luminance for a linear RGB value.
pub fn rec709_luminance(color: Vec3) -> f32 {
    0.2126 * color.x + 0.7152 * color.y + 0.0722 * color.z
}

/// Approximates an emissive primitive's power for discrete light selection.
pub fn emissive_light_weight(area: f32, color: Vec3, strength: f32) -> f32 {
    area * rec709_luminance(color) * strength
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rec709_luminance_uses_linear_rgb_coefficients() {
        assert!((rec709_luminance(Vec3::X) - 0.2126).abs() < 1.0e-6);
        assert!((rec709_luminance(Vec3::Y) - 0.7152).abs() < 1.0e-6);
        assert!((rec709_luminance(Vec3::Z) - 0.0722).abs() < 1.0e-6);
    }

    #[test]
    fn light_weight_combines_area_luminance_and_strength() {
        let weight = emissive_light_weight(2.5, Vec3::splat(0.4), 3.0);
        assert!((weight - 3.0).abs() < 1.0e-6);
    }
}
