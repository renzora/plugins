@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Must match the struct `#[post_process]` generates, field for field: the user
// fields in declaration order, then padding out to two vec4s, then `enabled`
// last. Nothing checks this at run time.
struct GrayscaleSettings {
    intensity: f32,
    luminance_r: f32,
    luminance_g: f32,
    luminance_b: f32,
    _padding1: f32,
    _padding2: f32,
    _padding3: f32,
    enabled: f32,
};
@group(0) @binding(2) var<uniform> settings: GrayscaleSettings;

@fragment
fn fragment(@builtin(position) pos: vec4<f32>, @location(0) in_uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(screen_texture, texture_sampler, in_uv);

    let luma = dot(color.rgb, vec3(settings.luminance_r, settings.luminance_g, settings.luminance_b));
    let gray = vec3(luma);
    return vec4(mix(color.rgb, gray, settings.intensity), color.a);
}
