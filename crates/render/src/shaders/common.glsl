#version 450
// Shared by every GLSL visual: placed before the visual's code, which defines
// mainImage like on Shadertoy. Mirrors common.wgsl.

// Same layout as `Uniforms` in common.wgsl (std140).
layout(set = 0, binding = 0) uniform Uniforms {
    // 64 bands packed in vec4s.
    vec4 bands[16];
    vec2 resolution;
    // Seconds since start.
    float time;
    // 1.0 on a beat, then fading out.
    float beat;
    float bass;
    float mid;
    float treble;
    // Free parameters, e.g. set from sliders.
    vec4 params;
    // MIDI notes and controllers, packed like the bands.
    vec4 notes[32];
    vec4 controls[32];
} u;

const int BAND_COUNT = 64;
const float PI = 3.14159265;

// Shadertoy's names. iMouse isn't tracked: always 0.
#define iTime u.time
#define iResolution vec3(u.resolution, 1.0)
#define iMouse vec4(0.0)

// Loudness of band i, 0..1 (0 = lows, 63 = highs).
float band(int i) {
    int c = clamp(i, 0, BAND_COUNT - 1);
    return u.bands[c / 4][c % 4];
}

// Band value at a continuous position in 0..1 (low to high), interpolated.
float band_at(float t) {
    float x = clamp(t, 0.0, 1.0) * float(BAND_COUNT - 1);
    int i = int(x);
    return mix(band(i), band(min(i + 1, BAND_COUNT - 1)), fract(x));
}

// MIDI note n (60 = middle C): its velocity (0..1) while held, then fading
// out after release.
float note(int n) {
    int c = clamp(n, 0, 127);
    return u.notes[c / 4][c % 4];
}

// MIDI controller n (a knob or fader): its position, 0..1.
float cc(int n) {
    int c = clamp(n, 0, 127);
    return u.controls[c / 4][c % 4];
}

vec3 hue(float h) {
    vec3 k = vec3(0.0, 2.0 / 3.0, 1.0 / 3.0);
    return clamp(abs(fract(h + k) * 6.0 - 3.0) - 1.0, 0.0, 1.0);
}
