uniform sampler2D artwork;
uniform vec4 tint;
uniform bool grayscale;
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
    vec4 pixel = texture(artwork, uv);
#else
    ivec2 size = textureSize(artwork, 0);
    vec2 texel = uv * vec2(size) - 0.5;
    ivec2 at = ivec2(floor(texel));
    vec2 f = fract(texel);
    vec4 top = mix(sample_texel(at, size), sample_texel(at + ivec2(1, 0), size), f.x);
    vec4 bottom = mix(sample_texel(at + ivec2(0, 1), size), sample_texel(at + ivec2(1, 1), size), f.x);
    vec4 pixel = mix(top, bottom, f.y);
#endif
    if (grayscale && pixel.a > 0.0) {
        // Relative luminance uses straight, linear RGB. Restore premultiplied
        // sRGB before blending so transparent artwork keeps its coverage.
        vec3 rgb = clamp(pixel.rgb / pixel.a, 0.0, 1.0);
        vec3 linear_rgb = mix(pow((rgb + 0.055) / 1.055, vec3(2.4)),
                              rgb / 12.92, lessThanEqual(rgb, vec3(0.04045)));
        float luminance = dot(linear_rgb, vec3(0.2126, 0.7152, 0.0722));
        float gray = luminance <= 0.0031308 ? 12.92 * luminance
            : 1.055 * pow(luminance, 1.0 / 2.4) - 0.055;
        pixel.rgb = vec3(gray * pixel.a);
    }
    // The onion-skin tint is an editing overlay applied after the preview.
    color = pixel * tint;
}
