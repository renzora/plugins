@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Must match the struct `#[post_process]` generates, field for field: the user
// fields in declaration order, then padding out to two vec4s, then `enabled`
// last. Nothing checks this at run time.
struct FrostedGlassSettings {
    intensity: f32,
    scale: f32,
    _padding1: f32,
    _padding2: f32,
    _padding3: f32,
    _padding4: f32,
    _padding5: f32,
    enabled: f32,
};
@group(0) @binding(2) var<uniform> settings: FrostedGlassSettings;

// Hash-based pseudo-random value in [0, 1) for a vec2 seed.
fn hash2(p: vec2<f32>) -> vec2<f32> {
    var q = vec2<f32>(
        dot(p, vec2<f32>(127.1, 311.7)),
        dot(p, vec2<f32>(269.5, 183.3)),
    );
    return fract(sin(q) * 43758.5453123);
}

@fragment
fn fragment(@builtin(position) pos: vec4<f32>, @location(0) in_uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(screen_texture, texture_sampler, in_uv);

    // Generate a noise-based UV offset
    let noise_uv = in_uv * settings.scale;
    let noise = hash2(floor(noise_uv)) + hash2(floor(noise_uv) + vec2<f32>(1.0, 0.0));
    // Map noise from [0,2) to [-1, 1)
    let offset = (noise - 1.0) * settings.intensity;

    let displaced_uv = clamp(in_uv + offset, vec2<f32>(0.0), vec2<f32>(1.0));
    return textureSample(screen_texture, texture_sampler, displaced_uv);
}
