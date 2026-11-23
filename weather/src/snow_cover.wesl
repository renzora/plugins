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

// A cell's random gradient, hashed from its integer coordinates (PCG).
// Integers rather than the usual `fract(sin(...))` on positions: that loses
// precision tens of kilometres from the origin, where a city's streets are,
// and the drifts there broke up into square blocks.
fn gradient(cell: vec2<i32>) -> vec2<f32> {
    var h = bitcast<u32>(cell.x) * 747796405u + bitcast<u32>(cell.y) * 2891336453u + 1013904223u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    let angle = f32(h) * (6.2831853 / 4294967296.0);
    return vec2(cos(angle), sin(angle));
}

// Gradient noise, -1..1. Smoother than value noise, whose bilinear blend
// between random corner values draws visibly square blobs.
fn gradient_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = p - cell;
    let c = vec2<i32>(cell);
    // Quintic fade: no crease at the cell edges, where the cubic one leaves a
    // faint grid in the shading.
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = dot(gradient(c), f);
    let b = dot(gradient(c + vec2(1, 0)), f - vec2(1.0, 0.0));
    let d = dot(gradient(c + vec2(0, 1)), f - vec2(0.0, 1.0));
    let e = dot(gradient(c + vec2(1, 1)), f - vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y) * 1.4;
}

// Each finer octave turned by about 37 degrees, so no layer lines up with
// the world axes or with the one under it.
const OCTAVE_TURN: mat2x2<f32> = mat2x2<f32>(0.8, 0.6, -0.6, 0.8);

// Drifts a few metres across with finer breakup inside them, 0..1, so a
// dusting gathers in patches rather than tinting everything evenly. The
// sample point is warped by a broad noise first, which bends the drift edges
// into organic shapes instead of round blobs on a lattice.
fn drift(world_xz: vec2<f32>) -> f32 {
    // Keep the coordinates small before scaling: the pattern repeats every
    // 4096 m, far beyond what is ever seen at once.
    let p0 = world_xz - floor(world_xz / 4096.0) * 4096.0;
    let warp = vec2(gradient_noise(p0 * 0.02), gradient_noise(p0 * 0.02 + vec2(17.3, -9.1))) * 8.0;
    // Drifts about 15 m across, with two gentler layers of detail on their
    // edges. Finer, stronger layers turned every edge into scratchy hair.
    var p = (p0 + warp) * 0.065;
    var sum = 0.0;
    var amplitude = 0.6;
    for (var i = 0; i < 3; i++) {
        sum += gradient_noise(p) * amplitude;
        p = OCTAVE_TURN * p * 2.2 + vec2(5.2, 1.3);
        amplitude *= 0.4;
    }
    return saturate(sum * 0.8 + 0.5);
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
    // The drift spreads flat ground across most of the cover range, so the
    // snow arrives and melts as growing and shrinking drifts rather than
    // flat ground going white all at once.
    let exposure = facing * (0.3 + 0.7 * drift(world.xz));
    let threshold = 1.0 - cover;
    let amount = smoothstep(threshold, threshold + 0.22, exposure) * step(0.001, cover);
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
