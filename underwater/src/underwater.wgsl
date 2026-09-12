@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Must match the struct `#[post_process]` generates, field for field: the user
// fields in declaration order, then padding out to a multiple of four, then
// `enabled` last. Eight user fields plus `enabled` is nine, which rounds up to
// twelve. Nothing checks this at run time.
struct UnderwaterSettings {
    distortion: f32,
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
    tint_strength: f32,
    wave_speed: f32,
    wave_scale: f32,
    time: f32,
    _padding1: f32,
    _padding2: f32,
    _padding3: f32,
    enabled: f32,
};
@group(0) @binding(2) var<uniform> settings: UnderwaterSettings;

@fragment
fn fragment(@builtin(position) pos: vec4<f32>, @location(0) in_uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(screen_texture, texture_sampler, in_uv);
    let t = settings.time * settings.wave_speed;
    let offset = vec2(
        sin(in_uv.y * settings.wave_scale * 10.0 + t) * settings.distortion,
        cos(in_uv.x * settings.wave_scale * 8.0 + t * 1.3) * settings.distortion * 0.7
    );
    let uv = clamp(in_uv + offset, vec2(0.0), vec2(1.0));
    var result = textureSample(screen_texture, texture_sampler, uv);

    // Apply tint
    let tint = vec3(settings.tint_r, settings.tint_g, settings.tint_b);
    result = vec4(mix(result.rgb, result.rgb * tint, settings.tint_strength), result.a);

    return result;
}
