@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

// Must match the struct `#[post_process]` generates, field for field: the user
// fields in declaration order, then padding out to two vec4s, then `enabled`
// last. Nothing checks this at run time.
struct CrossHatchSettings {
    density: f32,
    thickness: f32,
    angle: f32,
    brightness: f32,
    _padding1: f32,
    _padding2: f32,
    _padding3: f32,
    enabled: f32,
};
@group(0) @binding(2) var<uniform> settings: CrossHatchSettings;

@fragment
fn fragment(@builtin(position) pos: vec4<f32>, @location(0) in_uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(screen_texture, texture_sampler, in_uv);

    let lum = dot(color.rgb, vec3(0.299, 0.587, 0.114));
    let dims = vec2<f32>(textureDimensions(screen_texture));
    let pixel = in_uv * dims;

    let s = sin(settings.angle);
    let c = cos(settings.angle);

    // Rotated coordinates for two hatch directions
    let p1 = pixel.x * c + pixel.y * s;
    let p2 = pixel.x * c - pixel.y * s;

    let line1 = abs(sin(p1 * settings.density * 0.01));
    let line2 = abs(sin(p2 * settings.density * 0.01));

    var hatch = settings.brightness;
    // Darker areas get more hatch lines
    if lum < 0.75 {
        hatch = min(hatch, smoothstep(0.0, settings.thickness, line1));
    }
    if lum < 0.5 {
        hatch = min(hatch, smoothstep(0.0, settings.thickness, line2));
    }
    if lum < 0.25 {
        hatch *= 0.5;
    }

    return vec4(vec3(hatch), color.a);
}
