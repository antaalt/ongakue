// Placed after a GLSL visual's code: calls its mainImage for each pixel.

layout(location = 0) out vec4 ongakue_color;

void main() {
    // Like on Shadertoy, fragCoord starts at the bottom left.
    vec2 fragCoord = vec2(gl_FragCoord.x, u.resolution.y - gl_FragCoord.y);
    vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
    mainImage(color, fragCoord);
    ongakue_color = vec4(color.rgb, 1.0);
}
