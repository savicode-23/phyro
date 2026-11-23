uniform sampler2D artwork;
uniform vec4 tint;
in vec2 uv;
out vec4 color;

#if !HARDWARE_FILTER
vec4 sample_texel(ivec2 at, ivec2 size) {
    vec4 p = texelFetch(artwork, clamp(at, ivec2(0), size - 1), 0);
#if SRGB_SOURCE
    // Undo sRGB texture decoding *before* interpolating the premultiplied
    // texels. Each RGB component then varies linearly with its coverage.
    p.rgb = mix(1.055 * pow(p.rgb, vec3(1.0 / 2.4)) - 0.055,
                12.92 * p.rgb, lessThanEqual(p.rgb, vec3(0.0031308)));
#endif
    return p;
}
#endif

void main() {
#if HARDWARE_FILTER
    color = texture(artwork, uv) * tint;
#else
    ivec2 size = textureSize(artwork, 0);
    vec2 texel = uv * vec2(size) - 0.5;
    ivec2 at = ivec2(floor(texel));
    vec2 f = fract(texel);
    vec4 top = mix(sample_texel(at, size), sample_texel(at + ivec2(1, 0), size), f.x);
    vec4 bottom = mix(sample_texel(at + ivec2(0, 1), size), sample_texel(at + ivec2(1, 1), size), f.x);
    color = mix(top, bottom, f.y) * tint;
#endif
}
