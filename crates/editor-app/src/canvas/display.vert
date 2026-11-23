uniform mat3x2 dst;
uniform vec4 uv_rect;
out vec2 uv;

void main() {
    vec2 corner = vec2(gl_VertexID & 1, gl_VertexID >> 1);
    gl_Position = vec4(dst * vec3(corner, 1.0), 0.0, 1.0);
    uv = uv_rect.xy + corner * uv_rect.zw;
}
