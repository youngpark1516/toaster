struct RenderParams {
    width: u32,
    height: u32,
    samples: u32,
    max_bounces: u32,
};

@group(0) @binding(0)
var<storage, read_write> output: array<vec4<f32>>;

@group(0) @binding(1)
var<uniform> params: RenderParams;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;

    if (x >= params.width || y >= params.height) {
        return;
    }

    let idx = y * params.width + x;

    let u = f32(x) / f32(params.width - 1u);
    let v = f32(y) / f32(params.height - 1u);

    output[idx] = vec4<f32>(u, v, 0.25, 1.0);
}