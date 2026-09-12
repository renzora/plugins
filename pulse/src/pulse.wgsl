@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Must match the struct `#[post_process]` generates, field for field: the user
// fields in declaration order, then padding out to two vec4s, then `enabled`
// last. Nothing checks this at run time.
struct PulseSettings {
    strength: f32,
    speed: f32,
    time: f32,
    _padding1: f32,
    _padding2: f32,
    _padding3: f32,
    _padding4: f32,
    enabled: f32,
};
@group(0) @binding(2) var<uniform> settings: PulseSettings;

@fragment
fn fragment(@builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let c = textureSample(screen_texture, texture_sampler, uv);
    let d = distance(uv, vec2<f32>(0.5, 0.5));
    let wave = sin(settings.time) * 0.5 + 0.5;
    let vignette = 1.0 - d * settings.strength * wave * 2.0;
    return vec4<f32>(c.rgb * clamp(vignette, 0.0, 1.0), c.a);
}
