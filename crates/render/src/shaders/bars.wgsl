// Classic spectrum bars, low frequencies on the left.

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
