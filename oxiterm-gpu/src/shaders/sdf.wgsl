// Instanced SDF rounded-rectangle, ring cutout, and border-stroke shader (GLES 3.0 / WebGL2 compatible).
//
// Each instance expands a 6-vertex unit quad in surface pixel coordinates around the primitive's
// bounding box, plus an anti-aliasing margin. The fragment shader evaluates an exact signed
// distance field at the pixel centre (`@builtin(position).xy`) and converts distance to a 1 px
// analytic coverage alpha: `clamp(0.5 - d, 0.0, 1.0)`.
//
// Supported modes via `params = (radius_px, stroke_width_px, inner_inset_x_px, inner_inset_y_px)`:
// 1. Stroked border (`stroke_width_px > 0`):
//    `rect` is the stroke centerline box (already pixel-snapped on the CPU).
//    - `radius_px <= 0`: sharp miter box distance (`max(q.x, q.y)`) so corners stay crisp at any width.
//    - `radius_px > 0`: circular rounded-box SDF for smooth rounded corners.
//    Distance to the stroke is `abs(d_box) - 0.5 * stroke_width_px`.
// 2. Rounded border background ring (`stroke_width_px == 0` and `inner_inset > 0`):
//    `rect` is the outer rounded box; the interior inset by `(inner_inset_x, inner_inset_y)` is
//    subtracted via CSG (`max(d_outer, -d_inner)`), leaving only the rounded frame ring.
// 3. Solid or rounded fill (`stroke_width_px == 0` and `inner_inset == 0`):
//    `rect` is the fill box. For integer-snapped square fills (`radius_px == 0`), `0.5 - d` is
//    1.0 everywhere inside and 0.0 everywhere outside (exact crisp pixel coverage).

struct Globals {
    // xy = viewport size in pixels, zw = unused padding (16-byte uniform alignment).
    viewport: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> globals: Globals;

struct InstanceInput {
    // (x0, y0, x1, y1) in surface pixel coordinates.
    @location(0) rect: vec4<f32>,
    // Straight sRGB colour (r, g, b, a) in [0, 1].
    @location(1) color: vec4<f32>,
    // (radius_px, stroke_width_px, inner_inset_x_px, inner_inset_y_px).
    @location(2) params: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Box centre in surface pixel coordinates.
    @location(1) center: vec2<f32>,
    // Box half-extents in surface pixel coordinates.
    @location(2) half_size: vec2<f32>,
    // Pass-through primitive parameters.
    @location(3) params: vec4<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    inst: InstanceInput,
) -> VertexOutput {
    // Two triangles covering [0, 1]^2: (0,0), (1,0), (0,1), (1,0), (1,1), (0,1).
    var corner = vec2<f32>(0.0, 0.0);
    switch vid {
        case 0u: { corner = vec2<f32>(0.0, 0.0); }
        case 1u: { corner = vec2<f32>(1.0, 0.0); }
        case 2u: { corner = vec2<f32>(0.0, 1.0); }
        case 3u: { corner = vec2<f32>(1.0, 0.0); }
        case 4u: { corner = vec2<f32>(1.0, 1.0); }
        default: { corner = vec2<f32>(0.0, 1.0); }
    }

    // Expand the rasterized quad by half the stroke width plus a 1 px AA margin so antialiased
    // fringes and strokes centred on the rect edges are never clipped by the quad triangles.
    let margin = 0.5 * inst.params.y + 1.0;
    let min_p = inst.rect.xy - vec2<f32>(margin, margin);
    let max_p = inst.rect.zw + vec2<f32>(margin, margin);
    let px = mix(min_p, max_p, corner);

    // Convert surface pixel coordinates (top-left origin, +Y down) to NDC ([-1, 1], +Y up).
    let ndc_x = (px.x / globals.viewport.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (px.y / globals.viewport.y) * 2.0;

    var out: VertexOutput;
    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.color = inst.color;
    out.center = 0.5 * (inst.rect.xy + inst.rect.zw);
    out.half_size = 0.5 * (inst.rect.zw - inst.rect.xy);
    out.params = inst.params;
    return out;
}

// Exact Euclidean signed distance to a rounded box centred at the origin.
// Negative inside, zero on the boundary, positive outside.
fn sd_rounded_box(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let r = clamp(radius, 0.0, min(half_size.x, half_size.y));
    let q = abs(p) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// L-infinity box distance used for sharp miter border strokes (`radius == 0`):
// isolines are exact rectangles with 90-degree corners rather than rounded bevels.
fn sd_box_miter(p: vec2<f32>, half_size: vec2<f32>) -> f32 {
    let q = abs(p) - half_size;
    return max(q.x, q.y);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // `@builtin(position).xy` is the pixel centre (ix + 0.5, iy + 0.5) in framebuffer pixels.
    let rel = in.clip_pos.xy - in.center;
    let radius = in.params.x;
    let stroke_w = in.params.y;
    let inset = in.params.zw;

    var d: f32;
    if stroke_w > 0.0 {
        // Stroked border around the centerline box.
        let d_box = select(
            sd_box_miter(rel, in.half_size),
            sd_rounded_box(rel, in.half_size, radius),
            radius > 0.0,
        );
        d = abs(d_box) - 0.5 * stroke_w;
    } else if inset.x > 0.0 || inset.y > 0.0 {
        // Rounded border ring background: outer rounded silhouette minus sharp inner box.
        let d_outer = sd_rounded_box(rel, in.half_size, radius);
        let inner_half = in.half_size - inset;
        if inner_half.x > 0.0 && inner_half.y > 0.0 {
            let d_inner = sd_rounded_box(rel, inner_half, 0.0);
            d = max(d_outer, -d_inner);
        } else {
            d = d_outer;
        }
    } else {
        // Solid or rounded filled rectangle.
        d = sd_rounded_box(rel, in.half_size, radius);
    }

    // Analytic 1 px edge coverage centred on d = 0.
    let cov = clamp(0.5 - d, 0.0, 1.0);
    let alpha = in.color.a * cov;
    if alpha <= 0.0 {
        discard;
    }
    return vec4<f32>(in.color.rgb, alpha);
}
