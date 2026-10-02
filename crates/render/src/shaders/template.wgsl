// A new visual. fs_main runs once per pixel and returns its color: here, the
// spectrum as a soft gradient, low frequencies on the left, flashing on beats.
// Open "Available inputs" below for everything the music provides.

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = position.xy / u.resolution;
    // A faint base, so the visual shows even without sound.
    let level = 0.15 + band_at(uv.x);
    // uv.y grows downwards: brightest at the bottom.
    let color = hue(uv.x + 0.05 * u.time) * level * uv.y;
    return vec4<f32>(color + 0.2 * u.beat, 1.0);
}
