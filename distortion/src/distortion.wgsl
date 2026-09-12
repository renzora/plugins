@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Must match the struct `#[post_process]` generates, field for field: the user
// fields in declaration order, then padding out to two vec4s, then `enabled`
// last. Nothing checks this at run time.
struct DistortionSettings {
    intensity: f32,
    speed: f32,
    scale: f32,
    time: f32,
    _padding1: f32,
    _padding2: f32,
    _padding3: f32,
    enabled: f32,
};
@group(0) @binding(2) var<uniform> settings: DistortionSettings;

@fragment
fn fragment(@builtin(position) pos: vec4<f32>, @location(0) in_uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(screen_texture, texture_sampler, in_uv);
    let t = settings.time * settings.speed;
    let offset = vec2(
        sin(in_uv.y * settings.scale + t) * settings.intensity,
        cos(in_uv.x * settings.scale + t * 0.7) * settings.intensity
    );
    let uv = clamp(in_uv + offset, vec2(0.0), vec2(1.0));
    return textureSample(screen_texture, texture_sampler, uv);
}
