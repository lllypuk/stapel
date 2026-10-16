struct Light {
    pos: vec3<f32>,
    range: f32,
    color: vec4<f32>,
}

struct SceneUniform {
    view_proj: mat4x4<f32>,
    ambient: vec4<f32>,
    fog_color: vec4<f32>,
    sun_color: vec4<f32>,
    eye: vec3<f32>,
    fog_start: f32,
    // Нормирован на CPU; нулевой при `sun: None`.
    sun_dir: vec3<f32>,
    fog_end: f32,
    light_count: u32,
    lights: array<Light, 16>,
}

@group(0) @binding(0) var<uniform> scene: SceneUniform;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(2) @binding(0) var<uniform> palette: array<mat4x4<f32>, 64>;

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
    @location(12) tint: vec4<f32>,
}

struct SkinIn {
    @location(10) joints: vec4<u32>,
    @location(11) weights: vec4<f32>,
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tint: vec4<f32>,
}

fn vs_out(pos: vec4<f32>, normal: vec3<f32>, uv: vec2<f32>, inst: InstanceIn) -> VsOut {
    let model = mat4x4(inst.m0, inst.m1, inst.m2, inst.m3);
    let world = model * pos;
    var out: VsOut;
    out.clip = scene.view_proj * world;
    out.world = world.xyz;
    out.normal = mat3x3(inst.n0, inst.n1, inst.n2) * normal;
    out.uv = uv;
    out.tint = inst.tint;
    return out;
}

@vertex
fn vs_static(v: VertexIn, inst: InstanceIn) -> VsOut {
    return vs_out(vec4(v.pos, 1.0), v.normal, v.uv, inst);
}

@vertex
fn vs_skinned(v: VertexIn, s: SkinIn, inst: InstanceIn) -> VsOut {
    let skin = palette[s.joints.x] * s.weights.x
        + palette[s.joints.y] * s.weights.y
        + palette[s.joints.z] * s.weights.z
        + palette[s.joints.w] * s.weights.w;
    let normal = mat3x3(skin[0].xyz, skin[1].xyz, skin[2].xyz) * v.normal;
    return vs_out(skin * vec4(v.pos, 1.0), normal, v.uv, inst);
}

// Порог Байера 4×4: при `a = k/16` остаётся ровно k пикселей блока.
const BAYER = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv) * in.tint;
    let px = vec2<u32>(in.clip.xy) % 4u;
    if t.a <= (BAYER[px.y * 4u + px.x] + 0.5) / 16.0 {
        discard;
    }
    let n = normalize(in.normal);
    var light = scene.ambient.rgb + scene.sun_color.rgb * max(dot(n, -scene.sun_dir), 0.0);
    for (var i = 0u; i < scene.light_count; i++) {
        let l = scene.lights[i];
        let to = l.pos - in.world;
        let d = length(to);
        // Без max за `range` отрицательное основание в квадрате снова даёт свет.
        let falloff = max(1.0 - d / l.range, 0.0);
        light += l.color.rgb * max(dot(n, to / d), 0.0) * falloff * falloff;
    }
    // max: `start == end` (туман по умолчанию) не делит на ноль.
    let span = max(scene.fog_end - scene.fog_start, 1e-6);
    let fog = clamp((distance(in.world, scene.eye) - scene.fog_start) / span, 0.0, 1.0);
    return vec4(mix(t.rgb * light, scene.fog_color.rgb, fog), 1.0);
}
