@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Must match the struct `#[post_process]` generates, field for field: the user
// fields in declaration order, then padding out to two vec4s, then `enabled`
// last. Nothing checks this at run time.
struct DreamSettings {
    intensity: f32,
    blur_radius: f32,
    threshold: f32,
    _padding1: f32,
    _padding2: f32,
    _padding3: f32,
    _padding4: f32,
    enabled: f32,
};
@group(0) @binding(2) var<uniform> settings: DreamSettings;

@fragment
fn fragment(@builtin(position) pos: vec4<f32>, @location(0) in_uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(screen_texture, texture_sampler, in_uv);

    let texel = vec2(1.0) / vec2<f32>(textureDimensions(screen_texture));
    let r = settings.blur_radius;

    // Simple box blur for glow
    var glow = vec3(0.0);
    var count = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let offset = vec2(f32(x), f32(y)) * texel * r;
            let s = textureSample(screen_texture, texture_sampler, in_uv + offset).rgb;
            let lum = dot(s, vec3(0.299, 0.587, 0.114));
            if lum > settings.threshold {
                glow += s;
            }
            count += 1.0;
        }
    }
    glow /= count;

    // Soft additive blend with desaturation
    let dreamy = color.rgb + glow * settings.intensity;
    let avg = dot(dreamy, vec3(0.333));
    let result = mix(dreamy, vec3(avg), settings.intensity * 0.3);
    return vec4(clamp(result, vec3(0.0), vec3(1.0)), color.a);
}
