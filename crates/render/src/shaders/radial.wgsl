// The spectrum as rays around a pulsing core, mirrored left and right: lows at
// the top, highs at the bottom.

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let scale = min(u.resolution.x, u.resolution.y);
    // Centered, y up, -0.5..0.5 on the short side.
    let p = (position.xy - 0.5 * u.resolution) / scale * vec2<f32>(1.0, -1.0);
    let r = length(p);
    let aa = 1.5 / scale;
    // 0 at the top, 1 at the bottom, on both sides.
    let t = abs(atan2(p.x, p.y)) / PI;

    // The core grows with the bass and kicks on beats.
    let core = 0.15 + 0.05 * u.bass + 0.04 * u.beat;
    let tint = hue(0.55 + 0.35 * t + 0.02 * u.time);

    // Background: faint waves radiating outwards, pushed along by the mids.
    let waves = 0.5 + 0.5 * sin(30.0 * r - 2.0 * u.time - 8.0 * u.mid);
    var color = tint * waves * 0.05 * smoothstep(1.0, 0.0, r);
    color += hue(0.6 + 0.02 * u.time) * u.beat * 0.2 * smoothstep(0.8, 0.0, r);

    // Rays: one per band, with a gap between them.
    let x = t * f32(BAND_COUNT);
    let i = min(u32(x), BAND_COUNT - 1u);
    let value = band(i);
    let outer = core + 0.01 + 0.3 * value;
    let across = abs(fract(x) - 0.5) * r * PI / f32(BAND_COUNT); // distance to the ray's axis
    let width = 0.3 * r * PI / f32(BAND_COUNT);
    let ray = smoothstep(width + aa, width, across)
        * smoothstep(core - aa, core + aa, r)
        * smoothstep(outer + aa, outer - aa, r);
    color += tint * ray * (0.4 + 0.6 * value);

    // Soft glow around the ray tips, smooth across bands.
    let smooth_value = band_at(t);
    let tip = core + 0.01 + 0.3 * smooth_value;
    color += tint * smooth_value * 0.35 * exp(-25.0 * abs(r - tip)) * step(core, r);

    // Core: dark disc with a bright rim that flashes on beats.
    let inside = smoothstep(core + aa, core - aa, r);
    color = mix(color, tint * 0.04 * (1.0 - r / core), inside);
    color += vec3<f32>(1.0) * exp(-abs(r - core) / (2.0 * aa)) * (0.3 + 0.7 * u.beat);

    // Treble adds a fine shimmering grain.
    color += (hash(position.xy + fract(u.time) * 97.0) - 0.5) * 0.1 * u.treble;

    return vec4<f32>(max(color, vec3<f32>(0.0)), 1.0);
}
