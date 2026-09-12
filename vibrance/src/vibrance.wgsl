@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Must match the struct `#[post_process]` generates, field for field: the user
// fields in declaration order, then padding out to two vec4s, then `enabled`
// last. Nothing checks this at run time.
struct VibranceSettings {
    intensity: f32,
    _padding1: f32,
    _padding2: f32,
    _padding3: f32,
    _padding4: f32,
    _padding5: f32,
    _padding6: f32,
    enabled: f32,
};
@group(0) @binding(2) var<uniform> settings: VibranceSettings;

@fragment
fn fragment(@builtin(position) pos: vec4<f32>, @location(0) in_uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(screen_texture, texture_sampler, in_uv);

    let mx = max(color.r, max(color.g, color.b));
    let avg = (color.r + color.g + color.b) / 3.0;
    // Saturation is low when max ~= avg
    let sat = mx - avg;
    // Boost less-saturated pixels more (smart vibrance)
    let boost = settings.intensity * (1.0 - sat) * (mx - avg);
    let result = color.rgb + (color.rgb - vec3(avg)) * boost;
    return vec4(clamp(result, vec3(0.0), vec3(1.0)), color.a);
}
