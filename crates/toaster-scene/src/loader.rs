use crate::{
    material::Material,
    object::Sphere,
    scene::{CameraSettings, RenderSettings, Scene},
};
use anyhow::{bail, Context, Result};
use glam::Vec3;
use serde::Deserialize;
use std::{collections::HashMap, path::Path};

#[derive(Deserialize)]
struct SceneFile {
    camera: CameraFile,
    render: RenderFile,
    materials: Vec<MaterialFile>,
    objects: Vec<ObjectFile>,
}

#[derive(Deserialize)]
struct CameraFile {
    position: Vec3,
    look_at: Vec3,
    #[serde(default = "default_up")]
    up: Vec3,
    #[serde(alias = "vertical_fov_degrees")]
    fov_degrees: f32,
}

#[derive(Deserialize)]
struct RenderFile {
    width: u32,
    height: u32,
    #[serde(alias = "samples_per_pixel")]
    samples: u32,
    max_bounces: u32,
}

#[derive(Deserialize)]
struct MaterialFile {
    name: String,
    #[serde(rename = "type")]
    kind: MaterialKind,
    albedo: Vec3,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum MaterialKind {
    Diffuse,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ObjectFile {
    Sphere {
        center: Vec3,
        radius: f32,
        material: String,
    },
}

fn default_up() -> Vec3 {
    Vec3::Y
}

pub fn load_scene(path: impl AsRef<Path>) -> Result<Scene> {
    let path = path.as_ref();
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read scene {}", path.display()))?;
    let file: SceneFile = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse scene {}", path.display()))?;
    build_scene(file)
}

fn build_scene(file: SceneFile) -> Result<Scene> {
    if file.render.width == 0 || file.render.height == 0 {
        bail!("render width and height must be greater than zero");
    }
    if file.render.samples == 0 {
        bail!("render samples must be greater than zero");
    }
    if !(0.0..180.0).contains(&file.camera.fov_degrees) {
        bail!("camera fov_degrees must be between 0 and 180");
    }
    if !file.camera.position.is_finite()
        || !file.camera.look_at.is_finite()
        || !file.camera.up.is_finite()
        || file
            .camera
            .position
            .abs_diff_eq(file.camera.look_at, f32::EPSILON)
    {
        bail!("camera vectors are invalid");
    }
    let view = file.camera.position - file.camera.look_at;
    if file.camera.up.cross(view).length_squared() <= f32::EPSILON {
        bail!("camera up vector must not be parallel to its view direction");
    }

    let mut material_names = HashMap::new();
    let mut materials = Vec::with_capacity(file.materials.len());
    for material in file.materials {
        if !material.albedo.is_finite()
            || material.albedo.cmplt(Vec3::ZERO).any()
            || material.albedo.cmpgt(Vec3::ONE).any()
        {
            bail!(
                "material '{}' albedo must be between 0 and 1",
                material.name
            );
        }
        if material_names.contains_key(&material.name) {
            bail!("duplicate material name '{}'", material.name);
        }
        let _kind = material.kind;
        material_names.insert(material.name, materials.len());
        materials.push(Material {
            albedo: material.albedo,
        });
    }

    let mut spheres = Vec::with_capacity(file.objects.len());
    for object in file.objects {
        match object {
            ObjectFile::Sphere {
                center,
                radius,
                material,
            } => {
                if !center.is_finite() || !radius.is_finite() || radius <= 0.0 {
                    bail!("sphere radius must be positive and its center must be finite");
                }
                let material_index = material_names
                    .get(&material)
                    .copied()
                    .with_context(|| format!("sphere references unknown material '{material}'"))?;
                spheres.push(Sphere {
                    center,
                    radius,
                    material_index,
                });
            }
        }
    }

    Ok(Scene {
        camera: CameraSettings {
            position: file.camera.position,
            look_at: file.camera.look_at,
            up: file.camera.up,
            fov_degrees: file.camera.fov_degrees,
        },
        render: RenderSettings {
            width: file.render.width,
            height: file.render.height,
            samples: file.render.samples,
            max_bounces: file.render.max_bounces,
        },
        materials,
        spheres,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Scene> {
        build_scene(serde_json::from_str(text)?)
    }

    #[test]
    fn accepts_canonical_fields_and_defaults_up() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,0,1],"look_at":[0,0,0],"fov_degrees":60},
            "render":{"width":4,"height":2,"samples":2,"max_bounces":1},
            "materials":[{"name":"red","type":"diffuse","albedo":[1,0,0]}],
            "objects":[{"type":"sphere","center":[0,0,0],"radius":0.5,"material":"red"}]
        }"#,
        )
        .unwrap();
        assert_eq!(scene.render.samples, 2);
        assert_eq!(scene.camera.up, Vec3::Y);
        assert_eq!(scene.spheres[0].material_index, 0);
    }

    #[test]
    fn accepts_legacy_field_aliases() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,0,1],"look_at":[0,0,0],"vertical_fov_degrees":45},
            "render":{"width":1,"height":1,"samples_per_pixel":1,"max_bounces":1},
            "materials":[],"objects":[]
        }"#,
        )
        .unwrap();
        assert_eq!(scene.camera.fov_degrees, 45.0);
    }

    #[test]
    fn rejects_unknown_materials() {
        let error = parse(
            r#"{
            "camera":{"position":[0,0,1],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "materials":[],
            "objects":[{"type":"sphere","center":[0,0,0],"radius":1,"material":"missing"}]
        }"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown material"));
    }
}
