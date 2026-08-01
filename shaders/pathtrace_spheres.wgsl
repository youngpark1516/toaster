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
    light_count: u32,
    total_light_area: f32,
    _pad1: u32,
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
    texture_offset: u32,
    texture_width: u32,
    texture_height: u32,

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
    tex_coord: vec2<f32>,
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

struct TriangleAttributes {
    n0: vec4<f32>,
    n1: vec4<f32>,
    n2: vec4<f32>,

    uv0: vec2<f32>,
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    flags: u32,
    _pad0: u32,
};

struct Light {
    kind: u32,
    material_index: u32,
    _pad0: u32,
    _pad1: u32,

    v0: vec4<f32>,
    v1: vec4<f32>,
    v2: vec4<f32>,
    center_radius: vec4<f32>,

    // x = area, y = cumulative_area, z/w = unused
    area_cumulative: vec4<f32>,
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

@group(0) @binding(6)
var<storage, read> lights: array<Light>;

@group(0) @binding(7)
var<storage, read> texture_pixels: array<u32>;

@group(0) @binding(8)
var<storage, read> triangle_attributes: array<TriangleAttributes>;

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

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        return value / 12.92;
    }
    return pow((value + 0.055) / 1.055, 2.4);
}

fn unpack_srgb_texel(packed: u32) -> vec3<f32> {
    let scale: f32 = 1.0 / 255.0;
    let srgb = vec3f(
        f32(packed & 255u),
        f32((packed >> 8u) & 255u),
        f32((packed >> 16u) & 255u)
    ) * scale;
    return vec3f(
        srgb_to_linear(srgb.x),
        srgb_to_linear(srgb.y),
        srgb_to_linear(srgb.z)
    );
}

fn wrap_texel(value: i32, size: u32) -> u32 {
    let signed_size = i32(size);
    return u32(((value % signed_size) + signed_size) % signed_size);
}

fn read_texture_texel(material: Material, x: i32, y: i32) -> vec3<f32> {
    let wrapped_x = wrap_texel(x, material.texture_width);
    let wrapped_y = wrap_texel(y, material.texture_height);
    let index = material.texture_offset + wrapped_y * material.texture_width + wrapped_x;
    return unpack_srgb_texel(texture_pixels[index]);
}

fn sample_base_color(material: Material, uv: vec2<f32>) -> vec3<f32> {
    if material.texture_width == 0u || material.texture_height == 0u {
        return material.albedo.xyz;
    }

    let repeated_uv = fract(uv);
    let position = repeated_uv * vec2f(
        f32(material.texture_width),
        f32(material.texture_height)
    ) - vec2f(0.5);
    let base = vec2<i32>(floor(position));
    let amount = fract(position);
    let top = mix(
        read_texture_texel(material, base.x, base.y),
        read_texture_texel(material, base.x + 1, base.y),
        amount.x
    );
    let bottom = mix(
        read_texture_texel(material, base.x, base.y + 1),
        read_texture_texel(material, base.x + 1, base.y + 1),
        amount.x
    );
    return material.albedo.xyz * mix(top, bottom, amount.y);
}

var<private> MIN_DISTANCE: f32 = 0.001;
var<private> MAX_DISTANCE: f32 = 10000; //ARBITRARY MAGIC NUMBER, CHANGE LATER?
var<private> EPSILON: f32 = 1e-8;

const PI : f32 = 3.14159265359;

fn sample_light() -> Light {
    let random_area_sample: f32 = random_f32() * params.total_light_area;
    var selected: Light = lights[0];

    for (var i: u32 = 0; i < params.light_count; i++) {
        let light: Light = lights[i];
        selected = light;
        if light.area_cumulative.y >= random_area_sample {
            break;
        }
    }

    return selected;
}

fn direct_light(hit: HitRecord, albedo: vec3<f32>) ->vec3<f32> {
    if params.light_count == 0 || params.total_light_area <= 0.0 {
        return vec3f(0, 0, 0);
    }

    let light_record: Light = sample_light();
    var position: vec3<f32>;
    var emissive_material: Material;
    var light_normal: vec3<f32>;

    if light_record.kind == 0u {
        let u: f32 = sqrt(random_f32());
        let v: f32 = random_f32();
        let weights: vec3<f32> = vec3f(1.0 - u, u * (1.0 - v), u * v);

        position = weights.x * light_record.v0.xyz
            + weights.y * light_record.v1.xyz
            + weights.z * light_record.v2.xyz;
            
        light_normal = normalize(cross(
            light_record.v1.xyz - light_record.v0.xyz,
            light_record.v2.xyz - light_record.v0.xyz
        ));
        emissive_material = materials[light_record.material_index];
    } else {
        light_normal = random_unit_vector();
        position = light_record.center_radius.xyz + light_record.center_radius.w * light_normal;
        emissive_material = materials[light_record.material_index];

    }

    let shadow_origin: vec3<f32> = hit.point + hit.normal * MIN_DISTANCE;
    let to_light: vec3<f32> = position - shadow_origin;

    let distance_squared: f32 = dot(to_light, to_light);
    if distance_squared <= MIN_DISTANCE * MIN_DISTANCE {
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

    if shadow_intersection.distance < distance - 2.0 * MIN_DISTANCE {
        return vec3f(0, 0, 0);
    }

    let diffuse_brdf: vec3<f32> = albedo / PI;

    return diffuse_brdf * emissive_material.albedo.xyz * emissive_material.params.z
        * surface_cosine * light_cosine * params.total_light_area / (distance_squared);
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
        0,
        vec2f(0)
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
        sphere.material_index,
        vec2f(0)
    );
}

fn hit_triangle(ray: Ray, triangle: Triangle, attributes: TriangleAttributes) -> HitRecord {
    let no_hit: HitRecord = HitRecord(
        3.4028235e38f,
        vec3<f32>(0, 0, 0),
        vec3<f32>(0, 0, 0),
        false,
        0,
        vec2f(0)
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

    let geometric_normal: vec3<f32> = normalize(cross(edge1, edge2));
    let front_face: bool = dot(ray.direction, geometric_normal) < 0.0;
    let weight0: f32 = 1.0 - u - v;
    var outward_normal: vec3<f32> = geometric_normal;
    if (attributes.flags & 1u) != 0u {
        outward_normal = normalize(
            weight0 * attributes.n0.xyz
            + u * attributes.n1.xyz
            + v * attributes.n2.xyz
        );
        if dot(outward_normal, geometric_normal) < 0.0 {
            outward_normal = -outward_normal;
        }
    }
    let normal = select(-1 * outward_normal, outward_normal, front_face);
    var tex_coord: vec2<f32> = vec2f(0);
    if (attributes.flags & 2u) != 0u {
        tex_coord = weight0 * attributes.uv0 + u * attributes.uv1 + v * attributes.uv2;
    }

    return HitRecord(
        distance,
        at(ray, distance),
        normal,
        front_face,
        triangle.material_index,
        tex_coord
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
        0,
        vec2f(0)
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
        let tempRecord: HitRecord = hit_triangle(ray, triangle, triangle_attributes[i]);

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
    var include_emissive: bool = true;

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
                let albedo = sample_base_color(material, record.tex_coord);
                radiance += throughput * direct_light(record, albedo);

                var direction: vec3<f32> = record.normal + random_unit_vector();
                if dot(direction, direction) < EPSILON {
                    direction = record.normal;
                }
                temp_ray = Ray(record.point, normalize(direction));
                attenuation = albedo;
                include_emissive = false;
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
                include_emissive = true;
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
                include_emissive = true;
            }
            case 3: { // emissive
                if include_emissive {
                    radiance += throughput * material.albedo.xyz * material.params.z;
                }
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
