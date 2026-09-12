@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Must match the struct `#[post_process]` generates, field for field: the user
// fields in declaration order, then padding out to two vec4s, then `enabled`
// last. Nothing checks this at run time.
struct LightStreaksSettings {
    intensity: f32,
    threshold: f32,
    samples: f32,
    direction: f32,
    _padding1: f32,
    _padding2: f32,
    _padding3: f32,
    enabled: f32,
};
@group(0) @binding(2) var<uniform> settings: LightStreaksSettings;

@fragment
fn fragment(@builtin(position) pos: vec4<f32>, @location(0) in_uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(screen_texture, texture_sampler, in_uv);

    let dir = vec2(cos(settings.direction), sin(settings.direction));
    let texel = vec2(1.0) / vec2<f32>(textureDimensions(screen_texture));
    let num = max(i32(settings.samples), 1);

    var streak = vec3(0.0);
    for (var i = 0; i < num; i++) {
        let t = f32(i) / f32(num);
        let offset = dir * t * 0.1;
        let s = textureSample(screen_texture, texture_sampler, in_uv + offset).rgb;
        let lum = dot(s, vec3(0.299, 0.587, 0.114));
        if lum > settings.threshold {
            let weight = 1.0 - t;
            streak += s * weight;
        }
    }
    streak /= f32(num) * 0.5;

    return vec4(color.rgb + streak * settings.intensity, color.a);
}
