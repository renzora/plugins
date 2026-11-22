// Lying snow, laid into the deferred G-buffer.
//
// Two entry points over one fullscreen triangle:
//
// - `apply` reads the G-buffer and writes it, with snow on every surface that
//   faces the sky, into a scratch texture;
// - `copy` writes the scratch back over the G-buffer.
//
// Two passes because a pass cannot read the texture it draws into, and the
// G-buffer cannot be a copy source (Bevy creates it without that usage).
//
// The G-buffer layout is Bevy's (`bevy_pbr::deferred::functions`):
//   r: base colour (sRGB) and perceptual roughness, unorm4x8
//   g: emissive, rgb9e5 (left alone)
//   b: reflectance, metallic, occlusion, clearcoat, unorm4x8
//   a: 24-bit octahedral normal and 8 bits of flags
// It is mirrored here rather than imported so the plugin depends on nothing
// but the packing, which has been stable across Bevy releases.

struct FullscreenOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct SnowUniform {
    world_from_clip: mat4x4<f32>,
    // x: snow cover 0..1, yzw unused.
    params: vec4<f32>,
}

@group(0) @binding(0) var gbuffer: texture_2d<u32>;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var<uniform> snow: SnowUniform;
@group(0) @binding(3) var scratch: texture_2d<u32>;

const UNLIT_FLAG: u32 = 1u;
// Snow's albedo, linear. A little under white: fresh snow reflects about 85%
// and reads as overexposed paper above that.
const SNOW_ALBEDO: vec3<f32> = vec3<f32>(0.86, 0.88, 0.92);
const SNOW_ROUGHNESS: f32 = 0.6;

fn octahedral_decode(packed: vec2<f32>) -> vec3<f32> {
    let f = packed * 2.0 - 1.0;
    var n = vec3(f.xy, 1.0 - abs(f.x) - abs(f.y));
    let t = saturate(-n.z);
    let w = select(vec2(t), vec2(-t), n.xy >= vec2(0.0));
    n = vec3(n.xy + w, n.z);
    return normalize(n);
}

fn octahedral_encode(v: vec3<f32>) -> vec2<f32> {
    var n = v / (abs(v.x) + abs(v.y) + abs(v.z));
    let octahedral_wrap = (1.0 - abs(n.yx)) * select(vec2(-1.0), vec2(1.0), n.xy > vec2(0.0));
    let n_xy = select(octahedral_wrap, n.xy, n.z >= 0.0);
    return n_xy * 0.5 + 0.5;
}

fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2(1.0, 0.0));
    let c = hash(i + vec2(0.0, 1.0));
    let d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Patches a few metres across with finer breakup inside them, so a dusting
// gathers in drifts rather than tinting everything evenly.
fn drift(p: vec2<f32>) -> f32 {
    return value_noise(p * 0.18) * 0.55 + value_noise(p * 0.9) * 0.3 + value_noise(p * 4.0) * 0.15;
}

@fragment
fn apply(in: FullscreenOutput) -> @location(0) vec4<u32> {
    let pixel = vec2<i32>(in.position.xy);
    var g = textureLoad(gbuffer, pixel, 0);
    let d = textureLoad(depth, pixel, 0);
    // Reverse Z: the sky is at 0 and has no surface.
    if d <= 0.0 {
        return g;
    }
    let flags = (g.a >> 24u) & 0xFFu;
    if (flags & UNLIT_FLAG) != 0u {
        return g;
    }

    let packed_normal = vec2(f32(g.a & 0xFFFu), f32((g.a >> 12u) & 0xFFFu)) / 4095.0;
    let n = octahedral_decode(packed_normal);

    let ndc = vec3(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, d);
    let w = snow.world_from_clip * vec4(ndc, 1.0);
    let world = w.xyz / w.w;

    let cover = saturate(snow.params.x);
    // How much the surface faces the sky: none on walls, all on flat tops.
    let facing = smoothstep(0.35, 0.85, n.y);
    // Flat, exposed spots fill first; a full cover reaches the gentler slopes.
    let exposure = facing * (0.55 + 0.45 * drift(world.xz));
    let threshold = 1.0 - cover;
    let amount = smoothstep(threshold, threshold + 0.18, exposure) * step(0.001, cover);
    if amount <= 0.0 {
        return g;
    }

    var base = unpack4x8unorm(g.r);
    let snow_srgb = pow(SNOW_ALBEDO, vec3(1.0 / 2.2));
    base = vec4(mix(base.rgb, snow_srgb, amount), mix(base.a, SNOW_ROUGHNESS, amount));
    g.r = pack4x8unorm(base);

    // Snow is not metal, and it reflects like any dielectric.
    var props = unpack4x8unorm(g.b);
    props.r = mix(props.r, 0.5, amount);
    props.g = mix(props.g, 0.0, amount);
    g.b = pack4x8unorm(props);

    // Snow smooths what it lies on: bricks and tiles under it lose their
    // bumps toward the sky.
    let smoothed = normalize(mix(n, vec3(0.0, 1.0, 0.0), amount * 0.6));
    let encoded = vec2<u32>(saturate(octahedral_encode(smoothed)) * 4095.0 + 0.5);
    g.a = (encoded.x & 0xFFFu) | ((encoded.y & 0xFFFu) << 12u) | (flags << 24u);
    return g;
}

@fragment
fn copy(in: FullscreenOutput) -> @location(0) vec4<u32> {
    return textureLoad(scratch, vec2<i32>(in.position.xy), 0);
}
