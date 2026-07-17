use crate::{
    animation::{validate_animation, Animation},
    material::Material,
    object::{Sphere, Triangle},
    scene::{Background, CameraSettings, RenderSettings, Scene},
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
    #[serde(default)]
    animation: Animation,
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
    #[serde(default)]
    background: BackgroundFile,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BackgroundFile {
    #[default]
    Sky,
    Black,
}

#[derive(Deserialize)]
struct MaterialFile {
    name: String,
    #[serde(flatten)]
    material: MaterialData,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MaterialData {
    Diffuse { albedo: Vec3 },
    Metal { albedo: Vec3, roughness: f32 },
    Dielectric { ior: f32 },
    Emissive { color: Vec3, strength: f32 },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ObjectFile {
    Sphere {
        center: Vec3,
        radius: f32,
        material: String,
        group: Option<String>,
    },
    Triangle {
        vertices: [Vec3; 3],
        material: String,
        group: Option<String>,
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
        if material_names.contains_key(&material.name) {
            bail!("duplicate material name '{}'", material.name);
        }

        let runtime_material = match material.material {
            MaterialData::Diffuse { albedo } => {
                validate_albedo(&material.name, albedo)?;
                Material::Diffuse { albedo }
            }
            MaterialData::Metal { albedo, roughness } => {
                validate_albedo(&material.name, albedo)?;
                if !roughness.is_finite() {
                    bail!("material '{}' roughness must be finite", material.name);
                }
                Material::Metal {
                    albedo,
                    roughness: roughness.clamp(0.0, 1.0),
                }
            }
            MaterialData::Dielectric { ior } => {
                if !ior.is_finite() || ior <= 0.0 {
                    bail!(
                        "material '{}' ior must be finite and greater than zero",
                        material.name
                    );
                }
                Material::Dielectric { ior }
            }
            MaterialData::Emissive { color, strength } => {
                validate_color(&material.name, "color", color)?;
                if !strength.is_finite() || strength < 0.0 {
                    bail!(
                        "material '{}' strength must be finite and non-negative",
                        material.name
                    );
                }
                Material::Emissive { color, strength }
            }
        };

        material_names.insert(material.name, materials.len());
        materials.push(runtime_material);
    }

    let mut spheres = Vec::with_capacity(file.objects.len());
    let mut triangles = Vec::with_capacity(file.objects.len());
    for object in file.objects {
        match object {
            ObjectFile::Sphere {
                center,
                radius,
                material,
                group,
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
                    group: validate_group(group)?,
                });
            }
            ObjectFile::Triangle {
                vertices,
                material,
                group,
            } => {
                if vertices.iter().any(|vertex| !vertex.is_finite()) {
                    bail!("triangle vertices must be finite");
                }
                let edges = [vertices[1] - vertices[0], vertices[2] - vertices[0]];
                if edges[0].cross(edges[1]).length_squared() <= f32::EPSILON {
                    bail!("triangle vertices must not be collinear");
                }
                let material_index = material_names.get(&material).copied().with_context(|| {
                    format!("triangle references unknown material '{material}'")
                })?;
                triangles.push(Triangle {
                    vertices,
                    material_index,
                    group: validate_group(group)?,
                });
            }
        }
    }

    let mut animation = file.animation;
    animation.normalize_rotation_axes()?;
    let scene = Scene {
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
            background: match file.render.background {
                BackgroundFile::Sky => Background::Sky,
                BackgroundFile::Black => Background::Black,
            },
        },
        materials,
        spheres,
        triangles,
        animation,
    };
    validate_animation(&scene)?;
    Ok(scene)
}

fn validate_group(group: Option<String>) -> Result<Option<String>> {
    if group.as_deref().is_some_and(str::is_empty) {
        bail!("object group name must not be empty");
    }
    Ok(group)
}

fn validate_albedo(name: &str, albedo: Vec3) -> Result<()> {
    validate_color(name, "albedo", albedo)
}

fn validate_color(name: &str, field: &str, color: Vec3) -> Result<()> {
    if !color.is_finite() || color.cmplt(Vec3::ZERO).any() || color.cmpgt(Vec3::ONE).any() {
        bail!("material '{name}' {field} must be between 0 and 1");
    }
    Ok(())
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
    fn accepts_all_material_types_and_clamps_roughness() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,0,1],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "materials":[
                {"name":"matte","type":"diffuse","albedo":[0.2,0.3,0.4]},
                {"name":"metal","type":"metal","albedo":[0.8,0.8,0.8],"roughness":2.0},
                {"name":"glass","type":"dielectric","ior":1.5}
            ],
            "objects":[]
        }"#,
        )
        .unwrap();

        assert_eq!(
            scene.materials[0],
            Material::Diffuse {
                albedo: Vec3::new(0.2, 0.3, 0.4)
            }
        );
        assert_eq!(
            scene.materials[1],
            Material::Metal {
                albedo: Vec3::splat(0.8),
                roughness: 1.0
            }
        );
        assert_eq!(scene.materials[2], Material::Dielectric { ior: 1.5 });
    }

    #[test]
    fn accepts_emissive_material_triangle_and_black_background() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,0,1],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1,"background":"black"},
            "materials":[{"name":"light","type":"emissive","color":[1,0.8,0.6],"strength":5}],
            "objects":[{
                "type":"triangle",
                "vertices":[[-1,-1,0],[1,-1,0],[0,1,0]],
                "material":"light"
            }]
        }"#,
        )
        .unwrap();

        assert_eq!(scene.render.background, Background::Black);
        assert_eq!(scene.triangles.len(), 1);
        assert_eq!(scene.triangles[0].material_index, 0);
        assert_eq!(
            scene.materials[0],
            Material::Emissive {
                color: Vec3::new(1.0, 0.8, 0.6),
                strength: 5.0
            }
        );
    }

    #[test]
    fn rejects_invalid_material_parameters() {
        for material in [
            r#"{"name":"bad","type":"diffuse","albedo":[1.1,0,0]}"#,
            r#"{"name":"bad","type":"dielectric","ior":0}"#,
        ] {
            let json = format!(
                r#"{{
                "camera":{{"position":[0,0,1],"look_at":[0,0,0],"fov_degrees":45}},
                "render":{{"width":1,"height":1,"samples":1,"max_bounces":1}},
                "materials":[{material}],"objects":[]
            }}"#
            );
            assert!(parse(&json).is_err());
        }
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

    #[test]
    fn parses_grouped_animation_tracks() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,0,2],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "materials":[{"name":"red","type":"diffuse","albedo":[1,0,0]}],
            "objects":[{"type":"sphere","group":"ball","center":[1,0,0],"radius":0.5,"material":"red"}],
            "animation":{"tracks":[
                {"type":"translation","target":{"type":"group","name":"ball"},"interpolation":"step","keyframes":[{"time":0,"value":[0,1,0]}]},
                {"type":"rotation","target":{"type":"camera"},"axis":[0,2,0],"pivot":[0,0,0],"interpolation":"linear","keyframes":[{"time":0,"degrees":0},{"time":1,"degrees":90}]}
            ]}
        }"#,
        )
        .unwrap();

        assert_eq!(scene.spheres[0].group.as_deref(), Some("ball"));
        assert_eq!(scene.animation.tracks.len(), 2);
        let crate::AnimationTrack::Rotation { axis, .. } = &scene.animation.tracks[1] else {
            panic!("expected rotation track");
        };
        assert_eq!(*axis, Vec3::Y);
    }

    #[test]
    fn rejects_bad_animation_definitions() {
        for track in [
            r#"{"type":"rotation","target":{"type":"camera"},"axis":[0,0,0],"pivot":[0,0,0],"interpolation":"linear","keyframes":[{"time":0,"degrees":0}]}"#,
            r#"{"type":"translation","target":{"type":"group","name":"missing"},"interpolation":"linear","keyframes":[{"time":0,"value":[0,0,0]}]}"#,
            r#"{"type":"translation","target":{"type":"camera"},"interpolation":"linear","keyframes":[{"time":1,"value":[0,0,0]},{"time":1,"value":[1,0,0]}]}"#,
        ] {
            let json = format!(
                r#"{{
                "camera":{{"position":[0,0,2],"look_at":[0,0,0],"fov_degrees":45}},
                "render":{{"width":1,"height":1,"samples":1,"max_bounces":1}},
                "materials":[],"objects":[],"animation":{{"tracks":[{track}]}}
            }}"#
            );
            assert!(parse(&json).is_err());
        }
    }

    #[test]
    fn loads_rotating_cube_demo() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/006_rotating_cube.json");
        let scene = load_scene(path).unwrap();

        assert_eq!(scene.triangles.len(), 12);
        assert!(scene
            .triangles
            .iter()
            .all(|triangle| triangle.group.as_deref() == Some("cube")));
        assert_eq!(scene.animation.tracks.len(), 1);
    }
}
