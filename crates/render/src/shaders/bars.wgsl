const BAND_COUNT: u32 = 64u;

struct Uniforms {
    // 64 bands packed in vec4s: uniform arrays need a 16-byte element stride.
    bands: array<vec4<f32>, 16>,
    resolution: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

// A single triangle covering the whole screen.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn band(i: u32) -> f32 {
    return u.bands[i / 4u][i % 4u];
}

fn hue(h: f32) -> vec3<f32> {
    let k = vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0);
    return clamp(abs(fract(h + k) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = vec2<f32>(position.x / u.resolution.x, 1.0 - position.y / u.resolution.y);
    let background = vec4<f32>(0.02, 0.02, 0.04, 1.0);

    let x = uv.x * f32(BAND_COUNT);
    let i = min(u32(x), BAND_COUNT - 1u);
    let height = band(i);
    // Leave a gap between bars.
    if abs(fract(x) - 0.5) > 0.4 || uv.y > height {
        return background;
    }

    // Blue in the lows to red in the highs, brighter towards the top of the bar.
    let color = hue(0.66 * (1.0 - f32(i) / f32(BAND_COUNT - 1u)));
    let shade = 0.35 + 0.65 * uv.y / max(height, 0.001);
    return vec4<f32>(color * shade, 1.0);
}
