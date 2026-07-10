// Sphere path-tracing compute shader placeholder.

struct RenderParams {
    width: u32,
    height: u32,
    samples: u32,
    max_bounces: u32,

    sphere_count: u32,
    triangle_count: u32,
    material_count: u32,
    frame_index: u32,

    background_kind: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
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
    _pad0: array<u32, 3>,
};

struct Material {
    kind: u32,
    _pad0: array<u32, 3>,

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

struct Triangle {
    v0: vec4<f32>,
    v1: vec4<f32>,
    v2: vec4<f32>,

    material_index: u32,
    _pad0: array<u32, 3>,
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

@group(0) @binding(5)
var<storage, read> triangles: array<Triangle>;

var<private> rng_state: u32;

fn pcg_hash(input: u32) -> u32 {
    var state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn random_f32() -> f32 {
    rng_state = pcg_hash(rng_state);
    return f32(rng_state) / 4294967295.0;
}

fn random_unit_vector() -> vec3<f32> {
    let z = random_f32() * 2.0 - 1.0;
    let a = random_f32() * 6.2831853;
    let r = sqrt(max(0.0, 1.0 - z * z));
    return vec3<f32>(r * cos(a), r * sin(a), z);
}

var<private> MIN_DISTANCE: f32 = 0.001;
var<private> MAX_DISTANCE: f32 = 10000; //ARBITRARY MAGIC NUMBER, CHANGE LATER?
var<private> EPSILON: f32 = 1e-8;
var<private> light_area_tot: f32 = 0;
const PI : f32 = 3.14159265359;

fn collect_lights() {
    light_area_tot = 0;
    let num_triangle: u32 = params.triangle_count;

    for (var i: u32 = 0; i < num_triangle; i++) {
        let triangle: Triangle = triangles[i];
        if triangle.material_index != 3 {
            continue;
        }

        let edge1: vec3<f32> = triangle.v1.xyz - triangle.v0.xyz;
        let edge2: vec3<f32> = triangle.v2.xyz - triangle.v0.xyz;
        light_area_tot += length(cross(edge1, edge2)) * 0.5;
    }
}

fn sample_light() -> Triangle {
    let random_area_sample: f32 = random_f32() * light_area_tot;
    let num_triangle: u32 = params.triangle_count;

    var temp_area: f32 = 0;
    var triangle: Triangle;

    for (var i: u32 = 0; i < num_triangle; i++) {
        triangle = triangles[i];
        if triangle.material_index != 3 {
            continue;
        }

        let edge1: vec3<f32> = triangle.v1.xyz - triangle.v0.xyz;
        let edge2: vec3<f32> = triangle.v2.xyz - triangle.v0.xyz;
        temp_area += length(cross(edge1, edge2)) * 0.5;

        if (temp_area >= random_area_sample) {
            break;
        }
    }

    return triangle;
}

fn direct_light(hit: HitRecord, albedo: vec3<f32>) ->vec3<f32> {
    if light_area_tot == 0 {
        return vec3f(0, 0, 0);
    }

    let triangle: Triangle = sample_light();
    let u: f32 = sqrt(random_f32());
    let v: f32 = random_f32();
    let weights: vec3<f32> = vec3f(1.0 - u, u * (1.0 - v), u * v);
    let position: vec3<f32> = weights.x * triangle.v0.xyz
        + weights.y * triangle.v1.xyz
        + weights.z * triangle.v2.xyz;
            // let edge2 = ;
            // let cross = edge1.cross(edge2);
    let light_normal: vec3<f32> = normalize(cross(
        triangle.v1.xyz - triangle.v0.xyz,
        triangle.v2.xyz - triangle.v0.xyz
    ));
    
    let shadow_origin: vec3<f32> = hit.point + hit.normal * EPSILON;
    let to_light: vec3<f32> = position - shadow_origin;

    let distance_squared: f32 = dot(to_light, to_light);
    if distance_squared <= EPSILON * EPSILON {
        return vec3f(0, 0, 0);
    }

    let distance: f32 = sqrt(distance_squared);
    let direction: vec3<f32> = to_light / distance;


    let surface_cosine: f32 = dot(direction, hit.normal);
    let light_cosine:f32 = dot(-direction, light_normal);

    if surface_cosine <= 0.0 || light_cosine <= 0.0 {
        return vec3f(0, 0, 0);
    }

    let shadow_ray: Ray = Ray(shadow_origin, direction);
    let shadow_intersection: HitRecord = calc_intersections(shadow_ray);

    if shadow_intersection.distance < distance - 2.0 * EPSILON
        && shadow_intersection.distance > MAX_DISTANCE {
        return vec3f(0, 0, 0);
    }

    let material: Material = materials[triangle.material_index];

    let diffuse_brdf: vec3<f32> = albedo / PI;

    return diffuse_brdf * material.albedo.xyz * material.params.z * surface_cosine * light_cosine * light_area_tot
        / (distance_squared);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    
    let x: u32 = gid.x;
    let y: u32 = gid.y;

    if (x >= params.width || y >= params.height) {
        return;
    }

    let idx: u32 = y * params.width + x;
    rng_state = pcg_hash(idx ^ (params.frame_index * 9781u));
    let num_samples: u32 = params.samples;
    var color: vec4<f32> = vec4f(0, 0, 0, 0); 

    collect_lights();

    for (var i: u32 = 0; i < num_samples; i++) {

        let u: f32 = (f32(x) + random_f32()) / f32(params.width);
        let v: f32 = 1.0 - ((f32(y) + random_f32()) / f32(params.height));

        let ray: Ray = Ray(
        camera.origin.xyz,
        normalize(camera.lower_left_corner.xyz 
            + u * camera.horizontal.xyz
            + v *camera.vertical.xyz
            - camera.origin.xyz)
        );

        color += vec4f(ray_color(ray), 1.0);
    }
    output[idx] = color / f32(num_samples);
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

    let sqrt_disc: f32 = sqrt(discriminant);
    var distance: f32 = (-half_b - sqrt_disc) / a;

    if distance < MIN_DISTANCE {
        distance = (-1 * half_b + sqrt_disc) / a;

        if distance < MIN_DISTANCE {
            return no_hit;
        }
    }

    let point: vec3<f32> = at(ray, distance);
    let test_norm: vec3<f32>  = (point - sphere.center_radius.xyz) / sphere.center_radius.w;
    let front_face: bool = dot(ray.direction, test_norm) < 0.0;

    let normal: vec3<f32> = select(-test_norm, test_norm, front_face);

    return HitRecord(
        distance,
        point,
        normal,
        front_face,
        sphere.material_index
    );
}

fn hit_triangle(ray: Ray, triangle: Triangle) -> HitRecord {
    let no_hit: HitRecord = HitRecord(
        3.4028235e38f,
        vec3<f32>(0, 0, 0),
        vec3<f32>(0, 0, 0),
        false,
        0
    );

    let edge1: vec3<f32> = triangle.v1.xyz - triangle.v0.xyz;
    let edge2: vec3<f32> = triangle.v2.xyz - triangle.v0.xyz;
    let direction_cross_edge2: vec3<f32> = cross(ray.direction, edge2);
    let determinant: f32 = dot(direction_cross_edge2, edge1);

    if abs(determinant) < EPSILON {
        return no_hit;
    }

    let inv_determinant: f32 = 1.0 / determinant;
    let origin_offset: vec3<f32> = ray.origin - triangle.v0.xyz;
    let u: f32 = dot(origin_offset, direction_cross_edge2) * inv_determinant;
    if u < 0.0 || u > 1.0 {
        return no_hit;
    }

    let origin_cross_edge1: vec3<f32> = cross(origin_offset, edge1);
    let v: f32 = dot(ray.direction, origin_cross_edge1) * inv_determinant;
    if v < 0.0 || (u + v) > 1.0 {
        return no_hit;
    }

    let distance: f32 = dot(edge2, origin_cross_edge1) * inv_determinant;
    if distance < MIN_DISTANCE || distance > MAX_DISTANCE {
        return no_hit;
    }

    let outward_normal: vec3<f32> = normalize(cross(edge1, edge2));
    let front_face: bool = dot(ray.direction, outward_normal) < 0.0;
    let normal = select(-1 * outward_normal, outward_normal, front_face);

    return HitRecord(
        distance,
        at(ray, distance),
        normal,
        front_face,
        triangle.material_index
    );
}

fn calc_intersections(ray: Ray) -> HitRecord {
    let num_sphere: u32 = params.sphere_count;
    let num_triangle: u32 = params.triangle_count;
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

    for (var i: u32 = 0; i < num_triangle; i++) {
        let triangle: Triangle = triangles[i];
        let tempRecord: HitRecord = hit_triangle(ray, triangle);

        if (record.distance > tempRecord.distance) {
            record = tempRecord;
        }
    }

    return record;
}

fn ray_color(ray: Ray) -> vec3<f32> {
    let max_bounces: u32 = params.max_bounces;

    var throughput: vec3<f32> = vec3f(1, 1, 1);
    var cur_ray: Ray = ray;
    var radiance: vec3<f32> = vec3f(0,0,0);
    var break_loop: bool = false;

    for (var i: u32 = 0; i < max_bounces; i++) {
        let record: HitRecord = calc_intersections(cur_ray);

        if (record.distance > MAX_DISTANCE) {
            if params.background_kind == 0u {
                let lerp_t: f32 = (normalize(cur_ray.direction).y + 1.0) * 0.5;
                radiance += throughput * mix(vec3f(1.0, 1.0, 1.0), vec3f(0.35, 0.65, 1.0), lerp_t);
            }
            break;
        }

        let material: Material = materials[record.material_index];
        var attenuation: vec3<f32> = vec3f(0, 0, 0);
        var temp_ray: Ray = Ray(
            record.point,
            normalize(reflect_vec(cur_ray.direction, record.normal))
        );

        switch(material.kind) {
            case 0: { // diffuse
                radiance += throughput * direct_light(record, material.albedo.xyz);
                
                var direction: vec3<f32> = record.normal + random_unit_vector();
                if dot(direction, direction) < EPSILON {
                    direction = record.normal;
                }
                temp_ray = Ray(record.point, normalize(direction));
                attenuation = material.albedo.xyz;
            }
            case 1: { // metal
                let reflected = reflect_vec(cur_ray.direction, record.normal);
                let roughness = clamp(material.params.x, 0.0, 1.0);
                let direction = reflected + roughness * random_unit_vector();
                if dot(direction, record.normal) <= 0.0 {
                    break_loop = true;
                }
                temp_ray = Ray(
                    record.point,
                    normalize(direction)
                );
                attenuation = material.albedo.xyz;
            }
            case 2: { // dielectric
                let refraction_ratio: f32 = select(
                    material.params.y, 
                    1.0 / material.params.y, 
                    record.front_face
                );
                let unit_direction: vec3<f32> = normalize(cur_ray.direction);
                let cos_theta: f32 = min(1.0, dot(-unit_direction, record.normal));
                let sin_theta: f32 = sqrt(1.0 - (cos_theta * cos_theta));
                let cannot_refract: bool = (refraction_ratio * sin_theta) > 1.0;
                var direction: vec3<f32>;
                if cannot_refract || reflectance(cos_theta, refraction_ratio) > random_f32() {
                    direction = reflect_vec(unit_direction, record.normal);
                } else {
                    direction = refract_vec(unit_direction, record.normal, refraction_ratio);
                }
                temp_ray = Ray(record.point, normalize(direction));
                attenuation = vec3f(1, 1, 1);
            }
            case 3: { // emissive
                radiance += throughput * material.albedo.xyz * material.params.z;
                break_loop = true;
            }
            default: {
                break_loop = true;
            }
        }

        if break_loop {
            break;
        }

        throughput = throughput * attenuation;
        cur_ray = temp_ray;

    }

    return radiance;
}

fn reflect_vec(dir: vec3<f32>, norm: vec3<f32>) -> vec3 <f32> {
    return dir - dot(norm, dir) * 2.0 * norm;
}

fn refract_vec(dir: vec3<f32>, norm: vec3<f32>, ratio: f32) -> vec3<f32> {
    let cos_theta: f32 = min(dot(-dir, norm), 1.0);
    let perpendicular: vec3<f32> = ratio * (dir + cos_theta * norm);
    let parallel: vec3<f32> = -sqrt(abs(1.0 - dot(perpendicular, perpendicular))) * norm;
    return perpendicular + parallel;
}

fn reflectance(cosine: f32, refraction_ratio: f32) -> f32 {
    var r0 = ((1.0 - refraction_ratio) / (1.0 + refraction_ratio));
    r0 = r0 * r0;
    var temp: f32 = (1.0 - cosine);
    return r0 + (1.0 - r0) * temp * temp * temp * temp * temp;
}
