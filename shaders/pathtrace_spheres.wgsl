// Sphere path-tracing compute shader placeholder.

struct RenderParams {
    width: u32,
    height: u32,
    samples: u32,
    max_bounces: u32,

    sphere_count: u32,
    material_count: u32,
    frame_index: u32,
    _pad0: u32,
};

struct Camera {
    origin: vec4<f32>,
    lower_left_corner: vec4<f32>,
    horizontal: vec4<f32>,
    vertical: vec4<f32>,
};

struct Sphere {
    center_radius: vec4<f32>,

    material_index: u32,
    _pad0: vec3<u32>,
};

struct Material {
    kind: u32,
    _pad0: vec4<f32>,

    albedo: vec4<f32>,

    // x = roughness, y = ior, z = emission_strength, w = unused
    params: vec4<f32>,
};

@group(0) @binding(0)
var<storage, read_write> output: array<vec4<f32>>;

@group(0) @binding(1)
var<uniform> params: RenderParams;

@group(0) @binding(2)
var<uniform> camera: Camera;

@group(0) @binding(3)
var<storage, read> spheres: array<Sphere>;

@group(0) @binding(4)
var<storage, read> materials: array<Material>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;

    if (x >= params.width || y >= params.height) {
        return;
    }

    let idx = y * params.width + x;

    output[idx] = vec4<f32>(0.5, 0.5, 0.25, 1.0);
}