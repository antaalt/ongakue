// Shared by every visual: prepended to each visual's shader source.

const BAND_COUNT: u32 = 64u;
const PI: f32 = 3.14159265;

struct Uniforms {
    // 64 bands packed in vec4s: uniform arrays need a 16-byte element stride.
    bands: array<vec4<f32>, 16>,
    resolution: vec2<f32>,
    // Seconds since start.
    time: f32,
    // 1.0 on a beat, then fading out.
    beat: f32,
    bass: f32,
    mid: f32,
    treble: f32,
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

// Band value at a continuous position in 0..1 (low to high), interpolated.
fn band_at(t: f32) -> f32 {
    let x = clamp(t, 0.0, 1.0) * f32(BAND_COUNT - 1u);
    let i = u32(x);
    return mix(band(i), band(min(i + 1u, BAND_COUNT - 1u)), fract(x));
}

fn hue(h: f32) -> vec3<f32> {
    let k = vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0);
    return clamp(abs(fract(h + k) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
}
