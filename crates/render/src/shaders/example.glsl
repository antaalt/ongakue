// A GLSL visual, written like on Shadertoy: mainImage gets the pixel's
// coordinates (from the bottom left) and returns its color. Shadertoy's iTime
// and iResolution work, along with the app's inputs: u.bass, u.beat, band(i),
// band_at(t), note(n), cc(n), hue(h)... See "Available inputs" below.

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float r = length(uv);
    float angle = atan(uv.y, uv.x);

    // Rings moving outwards, pushed along by the bass.
    float rings = 0.5 + 0.5 * sin(30.0 * r - 4.0 * iTime - 10.0 * u.bass);
    // The spectrum around the circle: lows on the right, highs on the left.
    float level = band_at(abs(angle) / PI);

    vec3 color = hue(0.6 + 0.3 * r + 0.05 * iTime) * rings * (0.15 + level);
    // A flash in the middle on each beat.
    color += vec3(u.beat * 0.3 * smoothstep(0.4, 0.0, r));
    fragColor = vec4(color, 1.0);
}
