// Instanced glyph-atlas quad shader (GLES 3.0 / WebGL2 compatible).
//
// Each instance draws one CPU-rasterized glyph mask (`R8Unorm` coverage texture packed by
// `etagere`) at an exact integer pixel rect on the render target, tinted by `color`.
// Point/nearest sampling maps each mask texel 1:1 to its target pixel, and hardware alpha
// blending (`SrcAlpha / OneMinusSrcAlpha` on an `Rgba8Unorm` target) reproduces the sRGB-space
// coverage blend of `oxiterm-fb`.

struct Globals {
    // xy = viewport size in pixels, zw = unused padding.
    viewport: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> globals: Globals;

@group(1) @binding(0)
var atlas_tex: texture_2d<f32>;

@group(1) @binding(1)
var atlas_sampler: sampler;

struct InstanceInput {
    // (x_px, y_px, width_px, height_px) on the render target (exact integer glyph placement).
    @location(0) pos_size: vec4<f32>,
    // (u0, v0, u1, v1) normalised UV coordinates into the R8Unorm glyph atlas.
    @location(1) uv_rect: vec4<f32>,
    // Straight sRGB foreground colour (r, g, b, a) in [0, 1].
    @location(2) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    inst: InstanceInput,
) -> VertexOutput {
    var corner = vec2<f32>(0.0, 0.0);
    switch vid {
        case 0u: { corner = vec2<f32>(0.0, 0.0); }
        case 1u: { corner = vec2<f32>(1.0, 0.0); }
        case 2u: { corner = vec2<f32>(0.0, 1.0); }
        case 3u: { corner = vec2<f32>(1.0, 0.0); }
        case 4u: { corner = vec2<f32>(1.0, 1.0); }
        default: { corner = vec2<f32>(0.0, 1.0); }
    }

    let px = inst.pos_size.xy + corner * inst.pos_size.zw;
    let ndc_x = (px.x / globals.viewport.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (px.y / globals.viewport.y) * 2.0;

    var out: VertexOutput;
    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.uv = mix(inst.uv_rect.xy, inst.uv_rect.zw, corner);
    out.color = inst.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let cov = textureSample(atlas_tex, atlas_sampler, in.uv).r;
    let alpha = in.color.a * cov;
    if alpha <= 0.0 {
        discard;
    }
    return vec4<f32>(in.color.rgb, alpha);
}
