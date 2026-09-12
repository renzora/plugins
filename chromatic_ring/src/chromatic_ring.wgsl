@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Must match the struct `#[post_process]` generates, field for field: the user
// fields in declaration order, then padding out to two vec4s, then `enabled`
// last. Nothing checks this at run time.
struct ChromaticRingSettings {
    intensity: f32,
    radius: f32,
    falloff: f32,
    _padding1: f32,
    _padding2: f32,
    _padding3: f32,
    _padding4: f32,
    enabled: f32,
};
@group(0) @binding(2) var<uniform> settings: ChromaticRingSettings;

@fragment
fn fragment(@builtin(position) pos: vec4<f32>, @location(0) in_uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(screen_texture, texture_sampler, in_uv);

    let center = vec2(0.5);
    let delta = in_uv - center;
    let dist = length(delta);

    // Chromatic split increases with distance from center, focused at radius
    let ring_factor = smoothstep(settings.radius - settings.falloff, settings.radius, dist);
    let offset = normalize(delta) * ring_factor * settings.intensity;

    let r = textureSample(screen_texture, texture_sampler, clamp(in_uv + offset, vec2(0.0), vec2(1.0))).r;
    let g = color.g;
    let b = textureSample(screen_texture, texture_sampler, clamp(in_uv - offset, vec2(0.0), vec2(1.0))).b;

    return vec4(r, g, b, color.a);
}
