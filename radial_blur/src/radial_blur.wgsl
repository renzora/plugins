@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Must match the struct `#[post_process]` generates, field for field: the user
// fields in declaration order, then padding out to two vec4s, then `enabled`
// last. Nothing checks this at run time.
struct RadialBlurSettings {
    intensity: f32,
    center_x: f32,
    center_y: f32,
    samples: f32,
    _padding1: f32,
    _padding2: f32,
    _padding3: f32,
    enabled: f32,
};
@group(0) @binding(2) var<uniform> settings: RadialBlurSettings;

@fragment
fn fragment(@builtin(position) pos: vec4<f32>, @location(0) in_uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(screen_texture, texture_sampler, in_uv);

    let center = vec2<f32>(settings.center_x, settings.center_y);
    let dir = in_uv - center;
    let num_samples = max(1.0, settings.samples);

    var acc = vec4<f32>(0.0);
    for (var i = 0.0; i < num_samples; i = i + 1.0) {
        let t = i / (num_samples - 1.0);
        let sample_uv = in_uv - dir * settings.intensity * t;
        let sample_uv_c = clamp(sample_uv, vec2<f32>(0.0), vec2<f32>(1.0));
        acc = acc + textureSample(screen_texture, texture_sampler, sample_uv_c);
    }

    return acc / num_samples;
}
