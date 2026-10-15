struct SceneUniform {
    view_proj: mat4x4<f32>,
    ambient: vec4<f32>,
}

@group(0) @binding(0) var<uniform> scene: SceneUniform;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct InstanceIn {
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
    @location(7) n0: vec3<f32>,
    @location(8) n1: vec3<f32>,
    @location(9) n2: vec3<f32>,
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

@vertex
fn vs_static(v: VertexIn, inst: InstanceIn) -> VsOut {
    let model = mat4x4(inst.m0, inst.m1, inst.m2, inst.m3);
    let world = model * vec4(v.pos, 1.0);
    var out: VsOut;
    out.clip = scene.view_proj * world;
    out.world = world.xyz;
    out.normal = mat3x3(inst.n0, inst.n1, inst.n2) * v.normal;
    out.uv = v.uv;
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv);
    return vec4(t.rgb * scene.ambient.rgb, t.a);
}
