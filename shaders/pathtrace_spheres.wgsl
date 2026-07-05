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

struct Ray {
    origin: vec3<f32>,
    direction: vec3<f32>
};

struct HitRecord {
    distance: f32,
    point: vec3<f32>,
    normal: vec3<f32>,
    front_face: bool,
    material_index: u32,
};

fn at(ray: Ray, d: f32) -> vec3<f32> {
    return ray.origin + d * ray.direction;
}

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
    let x: u32 = gid.x;
    let y: u32 = gid.y;

    if (x >= params.width || y >= params.height) {
        return;
    }

    let idx: u32 = y * params.width + x;

    let u: f32 = f32(x) / f32(params.width);
    let v: f32 = 1 - (f32(y) / f32(params.height));

    let ray: Ray = Ray(
        camera.origin.xyz,
        camera.lower_left_corner.xyz 
            + u * camera.horizontal.xyz
            + v *camera.vertical.xyz
            - camera.origin.xyz
    );

    let color: vec4<f32> = ray_color(ray);

    output[idx] = color;
}

fn hit_sphere(ray: Ray, sphere: Sphere) -> HitRecord {
    let no_hit: HitRecord = HitRecord(
        3.4028235e38f,
        vec3<f32>(0, 0, 0),
        vec3<f32>(0, 0, 0),
        false,
        0
    );

    let offset: vec3<f32> = ray.origin - sphere.center_radius.xyz;
    let a: f32 = dot(ray.direction, ray.direction);
    let half_b: f32 = dot(offset, ray.direction);
    let c: f32 = dot(offset, offset) - sphere.center_radius.w * sphere.center_radius.w;
    let discriminant: f32 = half_b * half_b - a * c;

    if discriminant < 0.0 {
        return no_hit;
    }

    let sqrt_disc = sqrt(discriminant);
    var distance: f32 = (-1 * half_b - sqrt_disc) / a;

    if distance < 0 {
        distance = (-1 * half_b + sqrt_disc) / a;

        if distance < 0 {
            return no_hit;
        }
    }

    let point: vec3<f32> = at(ray, distance);
    let test_norm: vec3<f32>  = (point - sphere.center_radius.xyz);
    let front_face: bool = dot(ray.direction, test_norm) < 0.0;

    let normal: vec3<f32>  = select(-1 * test_norm, test_norm, front_face);

    return HitRecord(
        distance,
        point,
        normal,
        front_face,
        sphere.material_index
    );
}

fn ray_color(ray: Ray) -> vec4<f32> {
    let MAX_DISTANCE: f32 = 10000; //ARBITRARY MAGIC NUMBER CHANGE LATER
    let num_sphere: u32 = arrayLength(&spheres);
    var record: HitRecord = HitRecord(
        3.4028235e38f,
        vec3<f32>(0, 0, 0),
        vec3<f32>(0, 0, 0),
        false,
        0
    );

    for (var i: u32 = 0; i < num_sphere; i++) {
        let sphere: Sphere = spheres[i];
        let tempRecord: HitRecord = hit_sphere(ray, sphere);

        if (record.distance > tempRecord.distance) {
            record = tempRecord;
        }
    }

    if (record.distance > MAX_DISTANCE) {
        return vec4f(0.5, 0.7, 1.0, 1.0);
    }

    return vec4f((normalize(record.normal) + vec3f(1, 1, 1)) * 0.5, 1);
}