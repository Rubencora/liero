// shader.wgsl — Paletted R8 framebuffer → RGBA display.
//
// Bindings:
//   0: frame_tex — R8Uint texture (504×350), one palette index per pixel.
//   1: lut_tex   — Rgba8Unorm 256×1 texture, one RGBA entry per palette index.
//   2: opts      — uniform: { scanlines: u32 } (bit 0 = CRT scanlines on/off).

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
};

struct Opts {
    scanlines: u32,
};

@group(0) @binding(0) var frame_tex: texture_2d<u32>;
@group(0) @binding(1) var lut_tex:   texture_2d<f32>;
@group(0) @binding(2) var<uniform>   opts: Opts;

// Fullscreen quad as triangle strip (TL, BL, TR, BR).
// NDC Y is up; UV Y is down (top-left = (0,0)).
var<private> POSITIONS: array<vec2<f32>, 4> = array<vec2<f32>, 4>(
    vec2(-1.0,  1.0),
    vec2(-1.0, -1.0),
    vec2( 1.0,  1.0),
    vec2( 1.0, -1.0),
);
var<private> UVS: array<vec2<f32>, 4> = array<vec2<f32>, 4>(
    vec2(0.0, 0.0),
    vec2(0.0, 1.0),
    vec2(1.0, 0.0),
    vec2(1.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var out: VertexOutput;
    out.clip_pos = vec4(POSITIONS[vi], 0.0, 1.0);
    out.uv       = UVS[vi];
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dim   = vec2<f32>(textureDimensions(frame_tex));
    let coord = vec2<i32>(in.uv * dim);
    let idx   = i32(textureLoad(frame_tex, coord, 0).r);
    var color = textureLoad(lut_tex, vec2<i32>(idx, 0), 0);

    // CRT scanlines: darken every even row slightly.
    if opts.scanlines != 0u && (coord.y & 1) == 0 {
        color = vec4(color.rgb * 0.6, color.a);
    }

    return color;
}
