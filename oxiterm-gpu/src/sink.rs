//! Headless `wgpu` `DisplayList` sink (`OxiUI` Phase 5.2).
//!
//! Renders [`DisplayItem::Fill`], [`DisplayItem::Border`] (single, rounded, double, radius ring,
//! and glyph fallback), [`DisplayItem::Text`] (both default monospace runs and shaped proportional
//! runs with clusters/ligatures/bold/italic/underline), [`DisplayItem::CellGrid`], and
//! [`DisplayItem::PushClip`]/[`DisplayItem::PopClip`] using two GLES 3.0 / WebGL2-compatible
//! instanced pipelines (vertex + fragment only, no compute shaders).
//!
//! `DisplayItem::Path` and `DisplayItem::Legacy` are ignored in Stage 5.2 (`Path` arrives in 5.6).

use std::ops::Range;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use oxiterm_proto::style::{AnsiColor, BorderChars};
use oxiterm_renderer::font::{CELL_H_PX, CELL_W_PX};
use oxiterm_renderer::render::display_list::{
    DisplayItem, DisplayList, Glyph, GridCell, Modifiers, PointF, RectF, ShapedRun, FACE_MONO,
    FACE_SANS,
};
use oxiterm_renderer::render::unicode::UnicodeWidthCache;

use crate::atlas::{self, AtlasEntry, AtlasLookup, GlyphAtlas, MAX_CLUSTER_CHARS, SUBPIXEL_STEPS};
use crate::color::{self, Rgb, DEFAULT_BG};
use crate::context::{GpuContext, GpuError};
use crate::image::{self, FrameImage};

/// Target format for headless rendering. Straight sRGB byte values (`Rgba8Unorm`, not `*Srgb`) so
/// alpha blending matches `oxiterm-fb` and `Canvas2D` (`fg * a + bg * (1 - a)` in sRGB space).
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Default text size (px at scale 1) of the cell-wise font.
const CELL_FONT_PX: f32 = 16.0;
/// Coordinate clamp margin in surface pixels (matches `oxiterm-fb`, prevents float/int overflow).
const LIM: f64 = 1.0e5;
/// Soft upper bound on instances queued into a single pass (8K x 48 B = 384 KiB per buffer).
/// Always further clamped to `device.limits().max_buffer_size / stride`. Keeping each chunk to
/// 384 KiB keeps normal UI frames (hundreds of instances) in one pass while bounding peak GPU
/// vertex, staging, and software-rasterizer binning memory on multi-million-item hostile frames.
const MAX_INSTANCES_PER_PASS: usize = 8_192;
/// Minimum instance / batch buffer capacity retained when shrinking after a spike.
const MIN_RETAINED_CAP: usize = 1024;
/// Number of consecutive low-usage frames (< 1/4 capacity) before shrinking oversized buffers.
const SHRINK_AFTER_LOW_FRAMES: u32 = 4;

fn fin(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite())
}

fn mono_glyphs_non_overlapping(glyphs: &[Glyph]) -> bool {
    let mut prev: Option<(f32, f32)> = None;
    for g in glyphs {
        if !fin(&[g.x, g.y, g.advance]) || g.advance <= 0.0 {
            return false;
        }
        if let Some((py, px_end)) = prev {
            if g.y < py || (g.y.to_bits() == py.to_bits() && g.x < px_end) {
                return false;
            }
        }
        prev = Some((g.y, g.x + g.advance));
    }
    true
}

fn lim(v: f64) -> f64 {
    v.clamp(-LIM, LIM)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalsUniform {
    viewport: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SdfInstance {
    rect: [f32; 4],
    color: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlyphInstance {
    pos_size: [f32; 4],
    uv_rect: [f32; 4],
    color: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchKind {
    Sdf,
    Glyph,
}

#[derive(Debug, Clone)]
struct DrawBatch {
    kind: BatchKind,
    scissor: [u32; 4],
    range: Range<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IRect {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl IRect {
    fn intersect(self, other: IRect) -> IRect {
        let out = IRect {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        };
        if out.x1 < out.x0 || out.y1 < out.y0 {
            IRect { x0: out.x0, y0: out.y0, x1: out.x0, y1: out.y0 }
        } else {
            out
        }
    }

    fn to_scissor(self, surface_w: u32, surface_h: u32) -> Option<[u32; 4]> {
        let clamped = self.intersect(IRect {
            x0: 0,
            y0: 0,
            x1: i64::from(surface_w),
            y1: i64::from(surface_h),
        });
        if clamped.x1 <= clamped.x0 || clamped.y1 <= clamped.y0 {
            None
        } else {
            Some([
                clamped.x0 as u32,
                clamped.y0 as u32,
                (clamped.x1 - clamped.x0) as u32,
                (clamped.y1 - clamped.y0) as u32,
            ])
        }
    }
}

/// Per-frame target state threaded through `DisplayList` traversal.
struct FrameTarget<'a> {
    view: &'a wgpu::TextureView,
    width: u32,
    height: u32,
    clips: Vec<IRect>,
    cleared: bool,
}

impl FrameTarget<'_> {
    fn clip(&self) -> IRect {
        *self.clips.last().expect("clip stack is never empty")
    }

    fn push_clip(&mut self, rect: IRect) {
        let next = self.clip().intersect(rect);
        self.clips.push(next);
    }

    fn scissor(&self) -> Option<[u32; 4]> {
        self.clip().to_scissor(self.width, self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BorderStyle {
    Single,
    Rounded,
    Double,
    Glyphs,
}

fn border_style(chars: &BorderChars, width: f32, height: f32) -> BorderStyle {
    if width < 2.0 || height < 2.0 {
        return BorderStyle::Glyphs;
    }
    if [chars.top, chars.bot, chars.left, chars.right].contains(&' ') {
        return BorderStyle::Glyphs;
    }
    match chars.top_left {
        '╭' => BorderStyle::Rounded,
        '╔' => BorderStyle::Double,
        '┌' => BorderStyle::Single,
        _ => BorderStyle::Glyphs,
    }
}

/// Reusable offscreen render target + staging readback buffer for a fixed pixel size.
struct OffscreenTarget {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_bytes_per_row: u32,
}

impl OffscreenTarget {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Result<Self, GpuError> {
        let max_dim = device.limits().max_texture_dimension_2d.min(image::MAX_DIM);
        if width == 0
            || height == 0
            || width > max_dim
            || height > max_dim
            || !image::size_ok(width, height)
        {
            return Err(GpuError::InvalidSurfaceSize { width, height, max_dim });
        }

        let unpadded = width
            .checked_mul(4)
            .ok_or(GpuError::InvalidSurfaceSize { width, height, max_dim })?;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded.div_ceil(align) * align;
        let readback_size = u64::from(padded_bytes_per_row) * u64::from(height);
        if readback_size > device.limits().max_buffer_size {
            return Err(GpuError::InvalidSurfaceSize { width, height, max_dim });
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("oxiterm-gpu-offscreen-target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oxiterm-gpu-readback"),
            size: readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok(Self { width, height, texture, view, readback, padded_bytes_per_row })
    }
}

/// Statistics from the most recent frame rendered by [`GpuSink`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub sdf_instances: usize,
    pub glyph_instances: usize,
    pub draw_batches: usize,
    pub passes: usize,
    /// Number of glyphs/runs dropped because their scaled pixel size exceeded [`crate::atlas::MAX_GLYPH_PX`] (512 px).
    pub dropped_oversized_glyphs: usize,
}

/// Headless GPU `DisplayList` sink backed by `wgpu`.
///
/// Shares a [`GpuContext`] (`Arc<GpuContext>`) rather than owning the device privately, so other
/// GPU subsystems (`OxiScript` kernels, 3D rendering, window surfaces) can share the same device.
pub struct GpuSink {
    ctx: Arc<GpuContext>,
    scale: f64,
    cw: f64,
    ch: f64,
    init_error: Option<GpuError>,

    sdf_pipeline: wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    atlas_bind_group_layout: wgpu::BindGroupLayout,
    atlas: GlyphAtlas,

    max_sdf_per_pass: usize,
    max_glyph_per_pass: usize,
    sdf_buf: Option<(wgpu::Buffer, usize)>,
    glyph_buf: Option<(wgpu::Buffer, usize)>,
    offscreen: Option<OffscreenTarget>,

    sdf_instances: Vec<SdfInstance>,
    glyph_instances: Vec<GlyphInstance>,
    batches: Vec<DrawBatch>,
    frame_peak_sdf: usize,
    frame_peak_glyph: usize,
    frame_peak_batches: usize,
    sdf_low_frames: u32,
    glyph_low_frames: u32,
    batches_low_frames: u32,
    last_stats: FrameStats,
}

impl GpuSink {
    /// Creates a new [`GpuSink`] on the shared `ctx` at pixel `scale` (clamped to `0.25..=16.0`).
    ///
    /// Never panics even if the underlying `wgpu::Device` has been lost or destroyed; any device
    /// initialization failure is captured and returned cleanly from subsequent render calls.
    /// Prefer [`Self::try_new`] when you want construction-time error reporting.
    pub fn new(ctx: Arc<GpuContext>, scale: f64) -> Self {
        let scale = if scale.is_finite() { scale.clamp(0.25, 16.0) } else { 1.0 };
        // Match oxiterm-fb's decimal rounding of CELL_W_PX (9.6) and CELL_H_PX (20.0).
        let cw = (f64::from(CELL_W_PX) * 1e4).round() / 1e4 * scale;
        let ch = (f64::from(CELL_H_PX) * 1e4).round() / 1e4 * scale;

        let device = ctx.device();
        let max_buf = device.limits().max_buffer_size;
        let max_sdf_per_pass = ((max_buf / std::mem::size_of::<SdfInstance>() as u64) as usize)
            .clamp(1, MAX_INSTANCES_PER_PASS);
        let max_glyph_per_pass = ((max_buf / std::mem::size_of::<GlyphInstance>() as u64) as usize)
            .clamp(1, MAX_INSTANCES_PER_PASS);

        let (
            (
                globals_buffer,
                globals_bind_group,
                atlas_bind_group_layout,
                sdf_pipeline,
                glyph_pipeline,
                atlas,
            ),
            init_error,
        ) = ctx.capture_error_scopes(|| {
            let (globals_buffer, globals_bgl, globals_bind_group) =
                create_globals_resources(device);
            let atlas_bind_group_layout = create_atlas_bind_group_layout(device);
            let sdf_pipeline = create_sdf_pipeline(device, &globals_bgl);
            let glyph_pipeline =
                create_glyph_pipeline(device, &globals_bgl, &atlas_bind_group_layout);
            let atlas = GlyphAtlas::new(device, &atlas_bind_group_layout);
            (
                globals_buffer,
                globals_bind_group,
                atlas_bind_group_layout,
                sdf_pipeline,
                glyph_pipeline,
                atlas,
            )
        });

        Self {
            ctx,
            scale,
            cw,
            ch,
            init_error,
            sdf_pipeline,
            glyph_pipeline,
            globals_buffer,
            globals_bind_group,
            atlas_bind_group_layout,
            atlas,
            max_sdf_per_pass,
            max_glyph_per_pass,
            sdf_buf: None,
            glyph_buf: None,
            offscreen: None,
            sdf_instances: Vec::new(),
            glyph_instances: Vec::new(),
            batches: Vec::new(),
            frame_peak_sdf: 0,
            frame_peak_glyph: 0,
            frame_peak_batches: 0,
            sdf_low_frames: 0,
            glyph_low_frames: 0,
            batches_low_frames: 0,
            last_stats: FrameStats::default(),
        }
    }

    /// Fallible constructor equivalent to [`Self::new`], returning `Err(GpuError::DeviceLost(_))`
    /// or another [`GpuError`] immediately if the shared device is lost or pipeline creation fails.
    pub fn try_new(ctx: Arc<GpuContext>, scale: f64) -> Result<Self, GpuError> {
        ctx.check_device_lost()?;
        let sink = Self::new(ctx, scale);
        if let Some(err) = sink.init_error {
            return Err(err);
        }
        Ok(sink)
    }

    /// Returns the shared [`GpuContext`].
    pub fn context(&self) -> &Arc<GpuContext> {
        &self.ctx
    }

    /// Pixel scale factor.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Cell dimensions in surface pixels `(cw, ch)`.
    pub fn cell_px(&self) -> (f64, f64) {
        (self.cw, self.ch)
    }

    /// Pixel size of a `cols` x `rows` cell viewport (rounded up, matching `oxiterm-fb`).
    pub fn pixel_size(&self, cols: u32, rows: u32) -> (u32, u32) {
        let up = |v: f64| ((v * 1e6).round() / 1e6).ceil() as u32;
        (up(f64::from(cols) * self.cw), up(f64::from(rows) * self.ch))
    }

    /// Read-only access to the glyph atlas (metrics, tests).
    pub fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    /// Clears the glyph atlas (useful for cold-cache benchmarks and tests).
    pub fn clear_atlas(&mut self) {
        if self.ctx.is_device_lost() || self.init_error.is_some() {
            return;
        }
        let ctx = Arc::clone(&self.ctx);
        let ((), err) = ctx.capture_error_scopes(|| {
            self.atlas.reset(ctx.device(), &self.atlas_bind_group_layout);
        });
        if let Some(err) = err {
            if matches!(err, GpuError::DeviceLost(_)) {
                self.init_error = Some(err);
            }
        }
    }

    /// Statistics recorded for the last rendered frame.
    pub fn last_stats(&self) -> FrameStats {
        self.last_stats
    }

    fn check_ready(&self) -> Result<(), GpuError> {
        self.ctx.check_device_lost()?;
        if let Some(err) = &self.init_error {
            return Err(err.clone());
        }
        Ok(())
    }

    /// Renders `list` at `cols` x `rows` cells into a read-back RGBA8 [`FrameImage`].
    pub fn render_cells(
        &mut self,
        list: &DisplayList,
        cols: u32,
        rows: u32,
    ) -> Result<FrameImage, GpuError> {
        let (w, h) = self.pixel_size(cols, rows);
        self.render(list, w, h)
    }

    /// Renders `list` into a `width` x `height` pixel RGBA8 [`FrameImage`] (headless readback).
    pub fn render(
        &mut self,
        list: &DisplayList,
        width: u32,
        height: u32,
    ) -> Result<FrameImage, GpuError> {
        self.check_ready()?;
        self.ensure_offscreen(width, height)?;
        let target = self.offscreen.take().expect("offscreen target just ensured");
        let render_res = self.render_into_view(list, &target.view, width, height);
        if let Err(err) = render_res {
            self.offscreen = Some(target);
            return Err(err);
        }

        let readback_res = self.readback_target(&target);
        self.offscreen = Some(target);
        readback_res
    }

    /// Renders `list` directly into a caller-provided `Rgba8Unorm` texture view (e.g. for future
    /// window swapchains or offscreen composition without CPU readback).
    pub fn render_into_view(
        &mut self,
        list: &DisplayList,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<(), GpuError> {
        self.check_ready()?;

        let max_dim = self.ctx.device().limits().max_texture_dimension_2d.min(image::MAX_DIM);
        if width == 0
            || height == 0
            || width > max_dim
            || height > max_dim
            || !image::size_ok(width, height)
        {
            return Err(GpuError::InvalidSurfaceSize { width, height, max_dim });
        }

        validate_target_view(view, width, height)?;

        let ctx = Arc::clone(&self.ctx);
        ctx.with_error_scopes(|| {
            let globals = GlobalsUniform { viewport: [width as f32, height as f32, 0.0, 0.0] };
            self.ctx
                .queue()
                .write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

            self.sdf_instances.clear();
            self.glyph_instances.clear();
            self.batches.clear();
            self.frame_peak_sdf = 0;
            self.frame_peak_glyph = 0;
            self.frame_peak_batches = 0;
            self.last_stats = FrameStats::default();
            let dropped_before = self.atlas.dropped_oversized_glyphs();

            let mut frame = FrameTarget {
                view,
                width,
                height,
                clips: vec![IRect { x0: 0, y0: 0, x1: i64::from(width), y1: i64::from(height) }],
                cleared: false,
            };

            self.paint_items(list, &mut frame);

            // Always execute at least one pass so the background clear happens even for empty lists.
            self.flush_pass(frame.view, !frame.cleared, false);
            self.last_stats.dropped_oversized_glyphs =
                (self.atlas.dropped_oversized_glyphs().saturating_sub(dropped_before)) as usize;
            self.maybe_shrink_buffers();
            Ok(())
        })
    }

    fn ensure_offscreen(&mut self, width: u32, height: u32) -> Result<(), GpuError> {
        if let Some(target) = &self.offscreen {
            if target.width == width && target.height == height {
                return Ok(());
            }
        }
        let ctx = Arc::clone(&self.ctx);
        let target = ctx.with_error_scopes(|| OffscreenTarget::new(ctx.device(), width, height))?;
        if let Some(old) = self.offscreen.replace(target) {
            old.texture.destroy();
            old.readback.destroy();
        }
        Ok(())
    }

    fn readback_target(&self, target: &OffscreenTarget) -> Result<FrameImage, GpuError> {
        self.ctx.with_error_scopes(|| {
            let device = self.ctx.device();
            let queue = self.ctx.queue();

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("oxiterm-gpu-readback-encoder"),
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &target.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(target.padded_bytes_per_row),
                        rows_per_image: Some(target.height),
                    },
                },
                wgpu::Extent3d {
                    width: target.width,
                    height: target.height,
                    depth_or_array_layers: 1,
                },
            );
            let sub_idx = queue.submit(std::iter::once(encoder.finish()));

            let slice = target.readback.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |res| {
                let _ = tx.send(res);
            });
            // Ensure `unmap()` is always called on every exit path (including poll/recv/map errors)
            // so the staging buffer is never left pending-mapped.
            let unmap_guard = BufferUnmapGuard(&target.readback);

            device
                .poll(wgpu::PollType::Wait { submission_index: Some(sub_idx), timeout: None })
                .map_err(|e| GpuError::Readback(e.to_string()))?;
            rx.recv()
                .map_err(|e| GpuError::Readback(e.to_string()))?
                .map_err(|e| GpuError::Readback(e.to_string()))?;

            let mapped = slice
                .get_mapped_range()
                .map_err(|e| GpuError::Readback(e.to_string()))?;
            let unpadded = (target.width as usize) * 4;
            let padded = target.padded_bytes_per_row as usize;
            let mut rgba = vec![0u8; unpadded * (target.height as usize)];
            for row in 0..(target.height as usize) {
                let src_start = row * padded;
                let dst_start = row * unpadded;
                rgba[dst_start..dst_start + unpadded]
                    .copy_from_slice(&mapped[src_start..src_start + unpadded]);
            }
            drop(mapped);
            drop(unmap_guard);

            FrameImage::from_rgba(target.width, target.height, rgba)
        })
    }

    fn flush_pass(&mut self, view: &wgpu::TextureView, clear: bool, wait_to_reclaim: bool) {
        let device = self.ctx.device();
        let queue = self.ctx.queue();

        self.frame_peak_sdf = self.frame_peak_sdf.max(self.sdf_instances.len());
        self.frame_peak_glyph = self.frame_peak_glyph.max(self.glyph_instances.len());
        self.frame_peak_batches = self.frame_peak_batches.max(self.batches.len());

        if !self.sdf_instances.is_empty() {
            ensure_instance_buffer(
                device,
                &mut self.sdf_buf,
                self.sdf_instances.len(),
                std::mem::size_of::<SdfInstance>(),
                "oxiterm-gpu-sdf-instances",
            );
            let (buf, _) = self.sdf_buf.as_ref().expect("sdf buffer ensured");
            queue.write_buffer(buf, 0, bytemuck::cast_slice(&self.sdf_instances));
        }

        if !self.glyph_instances.is_empty() {
            ensure_instance_buffer(
                device,
                &mut self.glyph_buf,
                self.glyph_instances.len(),
                std::mem::size_of::<GlyphInstance>(),
                "oxiterm-gpu-glyph-instances",
            );
            let (buf, _) = self.glyph_buf.as_ref().expect("glyph buffer ensured");
            queue.write_buffer(buf, 0, bytemuck::cast_slice(&self.glyph_instances));
        }

        let load_op = if clear {
            wgpu::LoadOp::Clear(wgpu::Color {
                r: f64::from(DEFAULT_BG[0]) / 255.0,
                g: f64::from(DEFAULT_BG[1]) / 255.0,
                b: f64::from(DEFAULT_BG[2]) / 255.0,
                a: 1.0,
            })
        } else {
            wgpu::LoadOp::Load
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("oxiterm-gpu-render-encoder"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("oxiterm-gpu-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: load_op, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            let mut active_kind: Option<BatchKind> = None;
            let mut active_scissor: Option<[u32; 4]> = None;

            for batch in &self.batches {
                if batch.range.is_empty() {
                    continue;
                }
                if active_kind != Some(batch.kind) {
                    match batch.kind {
                        BatchKind::Sdf => {
                            pass.set_pipeline(&self.sdf_pipeline);
                            pass.set_bind_group(0, &self.globals_bind_group, &[]);
                            let (buf, _) = self.sdf_buf.as_ref().expect("sdf buf");
                            pass.set_vertex_buffer(0, buf.slice(..));
                        }
                        BatchKind::Glyph => {
                            pass.set_pipeline(&self.glyph_pipeline);
                            pass.set_bind_group(0, &self.globals_bind_group, &[]);
                            pass.set_bind_group(1, self.atlas.bind_group(), &[]);
                            let (buf, _) = self.glyph_buf.as_ref().expect("glyph buf");
                            pass.set_vertex_buffer(0, buf.slice(..));
                        }
                    }
                    active_kind = Some(batch.kind);
                }
                if active_scissor != Some(batch.scissor) {
                    let [sx, sy, sw, sh] = batch.scissor;
                    pass.set_scissor_rect(sx, sy, sw, sh);
                    active_scissor = Some(batch.scissor);
                }
                pass.draw(0..6, batch.range.clone());
            }
        }

        let sub_idx = queue.submit(std::iter::once(encoder.finish()));
        if wait_to_reclaim {
            let _ =
                device.poll(wgpu::PollType::Wait { submission_index: Some(sub_idx), timeout: None });
        }

        self.last_stats.sdf_instances += self.sdf_instances.len();
        self.last_stats.glyph_instances += self.glyph_instances.len();
        self.last_stats.draw_batches += self.batches.len();
        self.last_stats.passes += 1;

        self.sdf_instances.clear();
        self.glyph_instances.clear();
        self.batches.clear();
    }

    fn maybe_shrink_buffers(&mut self) {
        let mut shrunk_gpu = false;

        let sdf_cap =
            self.sdf_buf.as_ref().map_or(0, |(_, c)| *c).max(self.sdf_instances.capacity());
        if sdf_cap > MIN_RETAINED_CAP && self.frame_peak_sdf < sdf_cap / 4 {
            self.sdf_low_frames += 1;
            if self.sdf_low_frames >= SHRINK_AFTER_LOW_FRAMES {
                let target = self.frame_peak_sdf.next_power_of_two().max(MIN_RETAINED_CAP);
                if self.sdf_buf.as_ref().is_some_and(|(_, c)| *c > target) {
                    if let Some((buf, _)) = self.sdf_buf.take() {
                        buf.destroy();
                    }
                    shrunk_gpu = true;
                }
                if self.sdf_instances.capacity() > target {
                    self.sdf_instances = Vec::with_capacity(target);
                }
                self.sdf_low_frames = 0;
            }
        } else {
            self.sdf_low_frames = 0;
        }

        let glyph_cap =
            self.glyph_buf.as_ref().map_or(0, |(_, c)| *c).max(self.glyph_instances.capacity());
        if glyph_cap > MIN_RETAINED_CAP && self.frame_peak_glyph < glyph_cap / 4 {
            self.glyph_low_frames += 1;
            if self.glyph_low_frames >= SHRINK_AFTER_LOW_FRAMES {
                let target = self.frame_peak_glyph.next_power_of_two().max(MIN_RETAINED_CAP);
                if self.glyph_buf.as_ref().is_some_and(|(_, c)| *c > target) {
                    if let Some((buf, _)) = self.glyph_buf.take() {
                        buf.destroy();
                    }
                    shrunk_gpu = true;
                }
                if self.glyph_instances.capacity() > target {
                    self.glyph_instances = Vec::with_capacity(target);
                }
                self.glyph_low_frames = 0;
            }
        } else {
            self.glyph_low_frames = 0;
        }

        let batch_cap = self.batches.capacity();
        if batch_cap > MIN_RETAINED_CAP && self.frame_peak_batches < batch_cap / 4 {
            self.batches_low_frames += 1;
            if self.batches_low_frames >= SHRINK_AFTER_LOW_FRAMES {
                let target = self.frame_peak_batches.next_power_of_two().max(MIN_RETAINED_CAP);
                if self.batches.capacity() > target {
                    self.batches = Vec::with_capacity(target);
                }
                self.batches_low_frames = 0;
            }
        } else {
            self.batches_low_frames = 0;
        }

        if shrunk_gpu {
            let _ = self.ctx.device().poll(wgpu::PollType::Poll);
        }
    }

    // ---- DisplayList traversal --------------------------------------------------------------

    fn paint_items(&mut self, list: &DisplayList, frame: &mut FrameTarget<'_>) {
        let base_depth = frame.clips.len();
        for item in &list.items {
            match item {
                DisplayItem::Fill { rect, radius, bg, .. } => {
                    if !fin(&[rect.x, rect.y, rect.w, rect.h, *radius]) {
                        continue;
                    }
                    if *radius > 0.0 {
                        self.rounded_fill(*rect, *radius, *bg, frame);
                    } else {
                        self.push_fill(*rect, color::bg(bg), frame);
                    }
                }
                DisplayItem::Border { rect, radius, chars, fg, bg, .. } => {
                    if fin(&[rect.x, rect.y, rect.w, rect.h, *radius]) {
                        self.border(*rect, *radius, chars, *fg, *bg, frame);
                    }
                }
                DisplayItem::Text { origin, clip, glyphs, fg, bg, mods, shaped, .. } => {
                    if !fin(&[origin.x, origin.y, clip.x, clip.y, clip.w, clip.h]) {
                        continue;
                    }
                    let clip_rect = self.cell_irect(*clip);
                    frame.push_clip(clip_rect);
                    match shaped {
                        Some(run) => self.shaped(*origin, glyphs, run, *fg, *bg, *mods, frame),
                        None => self.mono_text(*origin, glyphs, *fg, *bg, *mods, frame),
                    }
                    frame.clips.pop();
                }
                DisplayItem::CellGrid { rect, cells } => {
                    if fin(&[rect.x, rect.y, rect.w, rect.h]) {
                        self.grid(*rect, cells, frame);
                    }
                }
                DisplayItem::PushClip { rect } => {
                    let clip_rect = if fin(&[rect.x, rect.y, rect.w, rect.h]) {
                        self.cell_irect(*rect)
                    } else {
                        // Non-finite clip fails closed (clips everything away).
                        IRect { x0: 0, y0: 0, x1: 0, y1: 0 }
                    };
                    frame.push_clip(clip_rect);
                }
                DisplayItem::PopClip => {
                    if frame.clips.len() > base_depth {
                        frame.clips.pop();
                    }
                }
                // Path arrives in Stage 5.6; Legacy is replaced by CellGrid in GUI mode.
                DisplayItem::Path { .. } | DisplayItem::Legacy { .. } => {}
            }
        }
        frame.clips.truncate(base_depth);
    }

    // ---- geometry & batching helpers --------------------------------------------------------

    fn cell_edges(&self, x: f32, y: f32, w: f32, h: f32) -> (f64, f64, f64, f64) {
        let (x0, y0) = (f64::from(x) * self.cw, f64::from(y) * self.ch);
        let (x1, y1) = (x0 + f64::from(w) * self.cw, y0 + f64::from(h) * self.ch);
        (lim(x0), lim(y0), lim(x1), lim(y1))
    }

    fn cell_irect(&self, rect: RectF) -> IRect {
        let (x0, y0, x1, y1) = self.cell_edges(rect.x, rect.y, rect.w, rect.h);
        IRect {
            x0: x0.round() as i64,
            y0: y0.round() as i64,
            x1: x1.round() as i64,
            y1: y1.round() as i64,
        }
    }

    fn visible(clip: IRect, x0: f64, y0: f64, x1: f64, y1: f64, pad: f64) -> bool {
        x1 + pad > clip.x0 as f64
            && x0 - pad < clip.x1 as f64
            && y1 + pad > clip.y0 as f64
            && y0 - pad < clip.y1 as f64
    }

    fn rounded_px(&self, rect: RectF, radius: f32) -> (f64, f64, f64, f64, f64) {
        let (x0, y0, x1, y1) = self.cell_edges(rect.x, rect.y, rect.w, rect.h);
        let (width, height) = (x1 - x0, y1 - y0);
        let rad = (f64::from(radius) * self.cw).max(0.0).min(width / 2.0).min(height / 2.0);
        (x0, y0, width, height, rad)
    }

    fn push_sdf(&mut self, inst: SdfInstance, frame: &mut FrameTarget<'_>) {
        let Some(scissor) = frame.scissor() else { return };
        if self.sdf_instances.len() >= self.max_sdf_per_pass {
            self.flush_pass(frame.view, !frame.cleared, true);
            frame.cleared = true;
        }
        let idx = self.sdf_instances.len() as u32;
        self.sdf_instances.push(inst);
        if let Some(last) = self.batches.last_mut() {
            if last.kind == BatchKind::Sdf && last.scissor == scissor && last.range.end == idx {
                last.range.end = idx + 1;
                return;
            }
        }
        self.batches.push(DrawBatch { kind: BatchKind::Sdf, scissor, range: idx..idx + 1 });
    }

    fn push_glyph(&mut self, inst: GlyphInstance, frame: &mut FrameTarget<'_>) {
        let Some(scissor) = frame.scissor() else { return };
        if self.glyph_instances.len() >= self.max_glyph_per_pass {
            self.flush_pass(frame.view, !frame.cleared, true);
            frame.cleared = true;
        }
        let idx = self.glyph_instances.len() as u32;
        self.glyph_instances.push(inst);
        if let Some(last) = self.batches.last_mut() {
            if last.kind == BatchKind::Glyph && last.scissor == scissor && last.range.end == idx {
                last.range.end = idx + 1;
                return;
            }
        }
        self.batches.push(DrawBatch { kind: BatchKind::Glyph, scissor, range: idx..idx + 1 });
    }

    // ---- fills ------------------------------------------------------------------------------

    fn fill_edges(
        &mut self,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        rgb: Rgb,
        frame: &mut FrameTarget<'_>,
    ) {
        let clip = frame.clip();
        let (x0, y0, x1, y1) = (lim(x0).round(), lim(y0).round(), lim(x1).round(), lim(y1).round());
        if x1 <= x0 || y1 <= y0 || !Self::visible(clip, x0, y0, x1, y1, 0.0) {
            return;
        }
        self.push_sdf(
            SdfInstance {
                rect: [x0 as f32, y0 as f32, x1 as f32, y1 as f32],
                color: color::to_rgba_f32(rgb),
                params: [0.0, 0.0, 0.0, 0.0],
            },
            frame,
        );
    }

    fn fill_frect(
        &mut self,
        px: f64,
        py: f64,
        pw: f64,
        ph: f64,
        rgb: Rgb,
        frame: &mut FrameTarget<'_>,
    ) {
        if px.is_finite() && py.is_finite() && pw.is_finite() && ph.is_finite() {
            self.fill_edges(px, py, px + pw, py + ph, rgb, frame);
        }
    }

    fn push_fill(&mut self, rect: RectF, rgb: Rgb, frame: &mut FrameTarget<'_>) {
        if !fin(&[rect.x, rect.y, rect.w, rect.h]) || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let (x0, y0, x1, y1) = self.cell_edges(rect.x, rect.y, rect.w, rect.h);
        self.fill_edges(x0, y0, x1, y1, rgb, frame);
    }

    fn rounded_fill(&mut self, rect: RectF, radius: f32, bg: AnsiColor, frame: &mut FrameTarget<'_>) {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let clip = frame.clip();
        let (px, py, pw, ph, rad) = self.rounded_px(rect, radius);
        if pw <= 0.0 || ph <= 0.0 || !Self::visible(clip, px, py, px + pw, py + ph, 2.0) {
            return;
        }
        self.push_sdf(
            SdfInstance {
                rect: [px as f32, py as f32, (px + pw) as f32, (py + ph) as f32],
                color: color::to_rgba_f32(color::bg(&bg)),
                params: [rad as f32, 0.0, 0.0, 0.0],
            },
            frame,
        );
    }

    // ---- borders ----------------------------------------------------------------------------

    fn border(
        &mut self,
        rect: RectF,
        radius: f32,
        chars: &BorderChars,
        fg: AnsiColor,
        bg: AnsiColor,
        frame: &mut FrameTarget<'_>,
    ) {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let (bx0, by0, bx1, by1) = self.cell_edges(rect.x, rect.y, rect.w, rect.h);
        if !Self::visible(frame.clip(), bx0, by0, bx1, by1, self.cw + self.ch + 4.0) {
            return;
        }
        let (rx, ry, rw, rh) = (rect.x, rect.y, rect.w, rect.h);
        let bgc = color::bg(&bg);
        let fgc = color::fg(&fg);
        let vector_round = radius > 0.0
            && rw >= 2.0
            && rh >= 2.0
            && ![chars.top, chars.bot, chars.left, chars.right].contains(&' ');

        if vector_round {
            let (px, py, pw, ph, rad) = self.rounded_px(rect, radius);
            self.push_sdf(
                SdfInstance {
                    rect: [px as f32, py as f32, (px + pw) as f32, (py + ph) as f32],
                    color: color::to_rgba_f32(bgc),
                    params: [rad as f32, 0.0, self.cw as f32, self.ch as f32],
                },
                frame,
            );
        } else {
            self.push_fill(RectF::new(rx, ry, rw, 1.0), bgc, frame);
            if rh > 1.0 {
                self.push_fill(RectF::new(rx, ry + rh - 1.0, rw, 1.0), bgc, frame);
            }
            if rh > 2.0 {
                self.push_fill(RectF::new(rx, ry + 1.0, 1.0, rh - 2.0), bgc, frame);
                if rw > 1.0 {
                    self.push_fill(RectF::new(rx + rw - 1.0, ry + 1.0, 1.0, rh - 2.0), bgc, frame);
                }
            }
        }

        let style = if vector_round { BorderStyle::Rounded } else { border_style(chars, rw, rh) };
        if style == BorderStyle::Glyphs {
            self.glyph_frame(rect, chars, fg, bg, frame);
            return;
        }

        // Vector lines through the centres of the border cells, snapped to the pixel grid.
        let (cw, ch) = (self.cw, self.ch);
        let lw_dev = self.scale.round().max(1.0);
        let off = if (lw_dev as i64) % 2 == 1 { 0.5 } else { 0.0 };
        let snap = |v: f64| lim(v).round() + off;
        let (xf, yf, wf, hf) = (f64::from(rx), f64::from(ry), f64::from(rw), f64::from(rh));
        let cx0 = (xf + 0.5) * cw;
        let cx1 = (xf + wf - 0.5) * cw;
        let cy0 = (yf + 0.5) * ch;
        let cy1 = (yf + hf - 0.5) * ch;

        let mut emit_stroke = |x0: f64, y0: f64, x1: f64, y1: f64, rad: f64| {
            let (sx0, sy0, sx1, sy1) = (snap(x0), snap(y0), snap(x1), snap(y1));
            if sx1 < sx0 || sy1 < sy0 {
                return;
            }
            let clamped_r = rad.min((sx1 - sx0) / 2.0).min((sy1 - sy0) / 2.0).max(0.0);
            self.push_sdf(
                SdfInstance {
                    rect: [sx0 as f32, sy0 as f32, sx1 as f32, sy1 as f32],
                    color: color::to_rgba_f32(fgc),
                    params: [clamped_r as f32, lw_dev as f32, 0.0, 0.0],
                },
                frame,
            );
        };

        match style {
            BorderStyle::Single => emit_stroke(cx0, cy0, cx1, cy1, 0.0),
            BorderStyle::Rounded => {
                let want = if vector_round { f64::from(radius) * cw } else { cw.min(ch) / 2.0 };
                let rad = want.min((cx1 - cx0) / 2.0).min((cy1 - cy0) / 2.0);
                emit_stroke(cx0, cy0, cx1, cy1, rad);
            }
            BorderStyle::Double => {
                let delta = cw.min(ch) / 6.0;
                emit_stroke(cx0 - delta, cy0 - delta, cx1 + delta, cy1 + delta, 0.0);
                emit_stroke(cx0 + delta, cy0 + delta, cx1 - delta, cy1 - delta, 0.0);
            }
            BorderStyle::Glyphs => unreachable!("handled above"),
        }
    }

    fn glyph_frame(
        &mut self,
        rect: RectF,
        chars: &BorderChars,
        fg: AnsiColor,
        bg: AnsiColor,
        frame: &mut FrameTarget<'_>,
    ) {
        let (rx, ry, rw, rh) = (rect.x, rect.y, rect.w, rect.h);
        let mods = Modifiers::default();
        let clip = frame.clip();
        let range = |lo_px: i64, hi_px: i64, cell: f64, origin: f32, n_max: i64| {
            let lo = ((lo_px as f64 / cell).floor() - 1.0 - f64::from(origin)).ceil() as i64;
            let hi = ((hi_px as f64 / cell).ceil() + 1.0 - f64::from(origin)).floor() as i64;
            (lo.max(1), hi.min(n_max))
        };
        let n_cols = (rw - 1.0).ceil() as i64 - 1;
        let n_rows = (rh - 1.0).ceil() as i64 - 1;
        let (i0, i1) = range(clip.x0, clip.x1, self.cw, rx, n_cols);
        let (j0, j1) = range(clip.y0, clip.y1, self.ch, ry, n_rows);

        // When border cells do not overlap (rw >= 2 && rh >= 2), batch all background fills first
        // and then all glyphs to avoid alternating SDF/Glyph pipelines on every border cell (F-08).
        if rw >= 2.0 && rh >= 2.0 {
            let bgc = color::bg(&bg);
            self.push_fill(RectF::new(rx, ry, 1.0, 1.0), bgc, frame);
            self.push_fill(RectF::new(rx + rw - 1.0, ry, 1.0, 1.0), bgc, frame);
            self.push_fill(RectF::new(rx, ry + rh - 1.0, 1.0, 1.0), bgc, frame);
            self.push_fill(RectF::new(rx + rw - 1.0, ry + rh - 1.0, 1.0, 1.0), bgc, frame);
            for col in i0..=i1 {
                let cx = rx + col as f32;
                self.push_fill(RectF::new(cx, ry, 1.0, 1.0), bgc, frame);
                self.push_fill(RectF::new(cx, ry + rh - 1.0, 1.0, 1.0), bgc, frame);
            }
            for row in j0..=j1 {
                let cy = ry + row as f32;
                self.push_fill(RectF::new(rx, cy, 1.0, 1.0), bgc, frame);
                self.push_fill(RectF::new(rx + rw - 1.0, cy, 1.0, 1.0), bgc, frame);
            }

            let fgc = color::fg(&fg);
            let px = CELL_FONT_PX * self.scale as f32;
            let baseline_off = f64::from(self.atlas.mono_baseline_px()) * self.scale;
            let mut draw_ch = |cx: f32, cy: f32, ch: char| {
                if ch == ' ' || ch == '\0' || !fin(&[cx, cy]) {
                    return;
                }
                let (x0, y0, _, _) = self.cell_edges(cx, cy, 1.0, 1.0);
                self.text_at(FACE_MONO, ch, None, px, x0, y0 + baseline_off, fgc, mods, frame);
            };
            draw_ch(rx, ry, chars.top_left);
            draw_ch(rx + rw - 1.0, ry, chars.top_right);
            draw_ch(rx, ry + rh - 1.0, chars.bot_left);
            draw_ch(rx + rw - 1.0, ry + rh - 1.0, chars.bot_right);
            for col in i0..=i1 {
                let cx = rx + col as f32;
                draw_ch(cx, ry, chars.top);
                draw_ch(cx, ry + rh - 1.0, chars.bot);
            }
            for row in j0..=j1 {
                let cy = ry + row as f32;
                draw_ch(rx, cy, chars.left);
                draw_ch(rx + rw - 1.0, cy, chars.right);
            }
            return;
        }

        self.glyph_cell(rx, ry, 1.0, chars.top_left, fg, bg, mods, frame);
        if rw > 1.0 {
            self.glyph_cell(rx + rw - 1.0, ry, 1.0, chars.top_right, fg, bg, mods, frame);
        }
        if rh > 1.0 {
            self.glyph_cell(rx, ry + rh - 1.0, 1.0, chars.bot_left, fg, bg, mods, frame);
        }
        if rw > 1.0 && rh > 1.0 {
            self.glyph_cell(rx + rw - 1.0, ry + rh - 1.0, 1.0, chars.bot_right, fg, bg, mods, frame);
        }
        for col in i0..=i1 {
            let cx = rx + col as f32;
            self.glyph_cell(cx, ry, 1.0, chars.top, fg, bg, mods, frame);
            if rh > 1.0 {
                self.glyph_cell(cx, ry + rh - 1.0, 1.0, chars.bot, fg, bg, mods, frame);
            }
        }
        for row in j0..=j1 {
            let cy = ry + row as f32;
            self.glyph_cell(rx, cy, 1.0, chars.left, fg, bg, mods, frame);
            if rw > 1.0 {
                self.glyph_cell(rx + rw - 1.0, cy, 1.0, chars.right, fg, bg, mods, frame);
            }
        }
    }

    // ---- text & cell grids ------------------------------------------------------------------

    /// Paints an unshaped monospace `Text` run. When glyphs are non-overlapping, non-italic, and
    /// non-bold, batches all background fills first and then all glyph quads to avoid alternating
    /// SDF/Glyph draw calls on every character. Bold and italic glyphs can overhang into the next
    /// cell and must draw in per-cell order to match `oxiterm-fb`.
    #[allow(clippy::too_many_arguments)]
    fn mono_text(
        &mut self,
        origin: PointF,
        glyphs: &[Glyph],
        fg: AnsiColor,
        bg: AnsiColor,
        mods: Modifiers,
        frame: &mut FrameTarget<'_>,
    ) {
        let can_batch = !mods.italic && !mods.bold && mono_glyphs_non_overlapping(glyphs);
        if !can_batch {
            for glyph in glyphs {
                if fin(&[glyph.x, glyph.y, glyph.advance]) {
                    self.glyph_cell(
                        origin.x + glyph.x,
                        origin.y + glyph.y,
                        glyph.advance,
                        glyph.ch,
                        fg,
                        bg,
                        mods,
                        frame,
                    );
                }
            }
            return;
        }

        let bgc = color::bg(&bg);
        for glyph in glyphs {
            if fin(&[glyph.x, glyph.y, glyph.advance]) {
                self.push_fill(
                    RectF::new(origin.x + glyph.x, origin.y + glyph.y, glyph.advance, 1.0),
                    bgc,
                    frame,
                );
            }
        }

        let fgc = color::fg(&fg);
        let px = CELL_FONT_PX * self.scale as f32;
        let baseline_off = f64::from(self.atlas.mono_baseline_px()) * self.scale;
        for glyph in glyphs {
            if !fin(&[glyph.x, glyph.y, glyph.advance]) || glyph.ch == ' ' || glyph.ch == '\0' {
                continue;
            }
            let (x0, y0, _, _) =
                self.cell_edges(origin.x + glyph.x, origin.y + glyph.y, glyph.advance, 1.0);
            self.text_at(FACE_MONO, glyph.ch, None, px, x0, y0 + baseline_off, fgc, mods, frame);
        }

        if mods.underline {
            let thickness = (2.0 * self.scale).round().max(1.0);
            for glyph in glyphs {
                if !fin(&[glyph.x, glyph.y, glyph.advance]) || glyph.ch == ' ' || glyph.ch == '\0' {
                    continue;
                }
                let (x0, y0, x1, _) =
                    self.cell_edges(origin.x + glyph.x, origin.y + glyph.y, glyph.advance, 1.0);
                self.fill_frect(x0, y0 + self.ch - thickness, x1 - x0, thickness, fgc, frame);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn glyph_cell(
        &mut self,
        cx: f32,
        cy: f32,
        advance: f32,
        ch: char,
        fg: AnsiColor,
        bg: AnsiColor,
        mods: Modifiers,
        frame: &mut FrameTarget<'_>,
    ) {
        if !fin(&[cx, cy, advance]) {
            return;
        }
        self.push_fill(RectF::new(cx, cy, advance, 1.0), color::bg(&bg), frame);
        if ch == ' ' || ch == '\0' {
            return;
        }
        let (x0, y0, x1, _) = self.cell_edges(cx, cy, advance, 1.0);
        let width_px = x1 - x0;
        let fgc = color::fg(&fg);
        let base_y = y0 + f64::from(self.atlas.mono_baseline_px()) * self.scale;
        let px = CELL_FONT_PX * self.scale as f32;
        self.text_at(FACE_MONO, ch, None, px, x0, base_y, fgc, mods, frame);
        if mods.underline {
            let thickness = (2.0 * self.scale).round().max(1.0);
            self.fill_frect(x0, y0 + self.ch - thickness, width_px, thickness, fgc, frame);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text_at(
        &mut self,
        face: u8,
        ch: char,
        cluster: Option<&str>,
        px: f32,
        x_px: f64,
        base_y: f64,
        rgb: Rgb,
        mods: Modifiers,
        frame: &mut FrameTarget<'_>,
    ) {
        match cluster {
            Some(text) if text.chars().nth(MAX_CLUSTER_CHARS).is_none() => {
                let shaped_glyphs = self.atlas.cluster_glyphs(face, text, px);
                for glyph in shaped_glyphs.iter() {
                    self.draw_glyph(
                        glyph.face,
                        glyph.gid,
                        px,
                        x_px + f64::from(glyph.dx),
                        base_y,
                        rgb,
                        mods,
                        frame,
                    );
                }
            }
            _ => {
                if let Some((resolved_face, gid)) = self.atlas.char_glyph(face, ch) {
                    self.draw_glyph(resolved_face, gid, px, x_px, base_y, rgb, mods, frame);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_glyph(
        &mut self,
        face: u8,
        gid: u16,
        px: f32,
        x_px: f64,
        base_y: f64,
        rgb: Rgb,
        mods: Modifiers,
        frame: &mut FrameTarget<'_>,
    ) {
        if !x_px.is_finite() || !base_y.is_finite() || frame.scissor().is_none() {
            return;
        }
        let (x_px, base_y) = (lim(x_px), lim(base_y));
        let steps = f64::from(SUBPIXEL_STEPS);
        let mut ix = x_px.floor() as i64;
        let mut sub = ((x_px - x_px.floor()) * steps).round() as u8;
        if sub >= SUBPIXEL_STEPS {
            ix += 1;
            sub = 0;
        }
        let by = base_y.round() as i64;

        let entry = match self.atlas.get_or_insert(
            self.ctx.queue(),
            face,
            gid,
            px,
            mods.bold,
            mods.italic,
            sub,
        ) {
            AtlasLookup::Hit(entry) => entry,
            AtlasLookup::Empty => return,
            AtlasLookup::Full => {
                // Flush pending draw calls that reference the current atlas texture before
                // growing or resetting the atlas into a new texture.
                self.flush_pass(frame.view, !frame.cleared, false);
                frame.cleared = true;
                let device = self.ctx.device();
                if !self.atlas.grow(device, &self.atlas_bind_group_layout) {
                    self.atlas.reset(device, &self.atlas_bind_group_layout);
                }
                match self.atlas.get_or_insert(
                    self.ctx.queue(),
                    face,
                    gid,
                    px,
                    mods.bold,
                    mods.italic,
                    sub,
                ) {
                    AtlasLookup::Hit(entry) => entry,
                    AtlasLookup::Empty | AtlasLookup::Full => return,
                }
            }
        };

        self.emit_glyph_quad(entry, ix, by, rgb, frame);
    }

    fn emit_glyph_quad(
        &mut self,
        entry: AtlasEntry,
        ix: i64,
        by: i64,
        rgb: Rgb,
        frame: &mut FrameTarget<'_>,
    ) {
        let dx = ix + i64::from(entry.left);
        let dy = by - i64::from(entry.top);
        let glyph_rect = IRect {
            x0: dx,
            y0: dy,
            x1: dx + i64::from(entry.width),
            y1: dy + i64::from(entry.height),
        };
        let visible = glyph_rect.intersect(frame.clip());
        if visible.x1 <= visible.x0 || visible.y1 <= visible.y0 {
            return;
        }
        let atlas_sz = self.atlas.size() as f32;
        let u0 = entry.atlas_x as f32 / atlas_sz;
        let v0 = entry.atlas_y as f32 / atlas_sz;
        let u1 = (entry.atlas_x + entry.width) as f32 / atlas_sz;
        let v1 = (entry.atlas_y + entry.height) as f32 / atlas_sz;

        self.push_glyph(
            GlyphInstance {
                pos_size: [dx as f32, dy as f32, entry.width as f32, entry.height as f32],
                uv_rect: [u0, v0, u1, v1],
                color: color::to_rgba_f32(rgb),
            },
            frame,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn shaped_line_glyphs(
        &mut self,
        origin: PointF,
        glyphs: &[Glyph],
        run: &ShapedRun,
        start: usize,
        end: usize,
        base_y: f64,
        px: f32,
        fgc: Rgb,
        mods: Modifiers,
        frame: &mut FrameTarget<'_>,
    ) {
        for (offset, glyph) in glyphs[start..end].iter().enumerate() {
            if !fin(&[glyph.x, glyph.advance]) {
                continue;
            }
            let cluster = run.cluster_of(start + offset);
            let text_blank = match cluster {
                Some(text) => text.chars().all(char::is_whitespace),
                None => glyph.ch.is_whitespace(),
            };
            if text_blank {
                continue;
            }
            let x_px = f64::from(origin.x + glyph.x) * self.cw;
            let face = run.face_of(start + offset).min(FACE_SANS);
            self.text_at(face, glyph.ch, cluster, px, x_px, base_y, fgc, mods, frame);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn shaped(
        &mut self,
        origin: PointF,
        glyphs: &[Glyph],
        run: &ShapedRun,
        fg: AnsiColor,
        bg: AnsiColor,
        mods: Modifiers,
        frame: &mut FrameTarget<'_>,
    ) {
        struct ShapedLine {
            start: usize,
            end: usize,
            top: f32,
            left: f32,
            right: f32,
        }

        let px = run.size_px * self.scale as f32;
        if !atlas::size_ok(px) || !fin(&[run.line_height, run.baseline]) {
            if px.is_finite() && px > atlas::MAX_GLYPH_PX {
                self.atlas.record_oversized_glyph(glyphs.len().max(1) as u64);
            }
            return;
        }
        let fgc = color::fg(&fg);
        let mut lines = Vec::new();
        let mut line_start = 0;
        while line_start < glyphs.len() {
            let mut line_end = line_start + 1;
            let first_y_bits = glyphs[line_start].y.to_bits();
            while line_end < glyphs.len() && glyphs[line_end].y.to_bits() == first_y_bits {
                line_end += 1;
            }
            let line = &glyphs[line_start..line_end];
            let top = origin.y + line[0].y;
            let left = origin.x + line[0].x;
            let right = line
                .iter()
                .filter(|g| fin(&[g.x, g.advance]))
                .map(|g| origin.x + g.x + g.advance)
                .fold(left, f32::max);
            if fin(&[top, left, right]) {
                lines.push(ShapedLine { start: line_start, end: line_end, top, left, right });
            }
            line_start = line_end;
        }

        let can_batch_lines = lines.len() > 1
            && run.line_height > 0.0
            && !mods.bold
            && !mods.italic
            && lines.windows(2).all(|w| w[1].top >= w[0].top + run.line_height);

        if can_batch_lines {
            if bg != AnsiColor::Reset {
                let bgc = color::bg(&bg);
                for ln in &lines {
                    self.push_fill(
                        RectF::new(ln.left, ln.top, ln.right - ln.left, run.line_height),
                        bgc,
                        frame,
                    );
                }
            }
            for ln in &lines {
                let base_y = f64::from(ln.top + run.baseline) * self.ch;
                self.shaped_line_glyphs(
                    origin, glyphs, run, ln.start, ln.end, base_y, px, fgc, mods, frame,
                );
            }
            if mods.underline {
                let pxf = f64::from(run.size_px) * self.scale;
                let thickness = (pxf * 0.06).max(1.0);
                for ln in &lines {
                    let base_y = f64::from(ln.top + run.baseline) * self.ch;
                    self.fill_frect(
                        f64::from(ln.left) * self.cw,
                        base_y + pxf * 0.12,
                        f64::from(ln.right - ln.left) * self.cw,
                        thickness,
                        fgc,
                        frame,
                    );
                }
            }
            return;
        }

        for ln in &lines {
            if bg != AnsiColor::Reset {
                self.push_fill(
                    RectF::new(ln.left, ln.top, ln.right - ln.left, run.line_height),
                    color::bg(&bg),
                    frame,
                );
            }
            let base_y = f64::from(ln.top + run.baseline) * self.ch;
            self.shaped_line_glyphs(
                origin, glyphs, run, ln.start, ln.end, base_y, px, fgc, mods, frame,
            );
            if mods.underline {
                let pxf = f64::from(run.size_px) * self.scale;
                let thickness = (pxf * 0.06).max(1.0);
                self.fill_frect(
                    f64::from(ln.left) * self.cw,
                    base_y + pxf * 0.12,
                    f64::from(ln.right - ln.left) * self.cw,
                    thickness,
                    fgc,
                    frame,
                );
            }
        }
    }

    fn grid_batched(
        &mut self,
        rect: RectF,
        cols: usize,
        slice: &[GridCell],
        has_underline: bool,
        frame: &mut FrameTarget<'_>,
    ) {
        let width_cache = UnicodeWidthCache::get();
        for (idx, cell) in slice.iter().enumerate() {
            if cell.skip {
                continue;
            }
            let raw_adv = width_cache.width(cell.ch);
            let adv = f32::from(if raw_adv == 0 { 1 } else { raw_adv });
            self.push_fill(
                RectF::new(rect.x + (idx % cols) as f32, rect.y + (idx / cols) as f32, adv, 1.0),
                color::bg(&cell.bg),
                frame,
            );
        }
        let px = CELL_FONT_PX * self.scale as f32;
        let baseline_off = f64::from(self.atlas.mono_baseline_px()) * self.scale;
        for (idx, cell) in slice.iter().enumerate() {
            if cell.skip || cell.ch == ' ' || cell.ch == '\0' {
                continue;
            }
            let raw_adv = width_cache.width(cell.ch);
            let adv = f32::from(if raw_adv == 0 { 1 } else { raw_adv });
            let cx = rect.x + (idx % cols) as f32;
            let cy = rect.y + (idx / cols) as f32;
            let (x0, y0, _, _) = self.cell_edges(cx, cy, adv, 1.0);
            self.text_at(
                FACE_MONO,
                cell.ch,
                None,
                px,
                x0,
                y0 + baseline_off,
                color::fg(&cell.fg),
                cell.mods,
                frame,
            );
        }
        if has_underline {
            let thickness = (2.0 * self.scale).round().max(1.0);
            for (idx, cell) in slice.iter().enumerate() {
                if cell.skip || !cell.mods.underline || cell.ch == ' ' || cell.ch == '\0' {
                    continue;
                }
                let raw_adv = width_cache.width(cell.ch);
                let adv = f32::from(if raw_adv == 0 { 1 } else { raw_adv });
                let cx = rect.x + (idx % cols) as f32;
                let cy = rect.y + (idx / cols) as f32;
                let (x0, y0, x1, _) = self.cell_edges(cx, cy, adv, 1.0);
                self.fill_frect(
                    x0,
                    y0 + self.ch - thickness,
                    x1 - x0,
                    thickness,
                    color::fg(&cell.fg),
                    frame,
                );
            }
        }
    }

    fn grid(&mut self, rect: RectF, cells: &[GridCell], frame: &mut FrameTarget<'_>) {
        let cols = rect.w.max(0.0) as usize;
        if cols == 0 {
            return;
        }
        let max_cells = cols.saturating_mul(rect.h.max(0.0) as usize);
        let slice = &cells[..cells.len().min(max_cells)];
        if slice.is_empty() {
            return;
        }

        // Batch backgrounds -> glyphs -> underlines when no cell is bold/italic and no wide cell
        // overlaps a subsequent non-skipped cell on the same row (F-08, preserves bold/italic
        // per-cell overhang parity with oxiterm-fb).
        let mut can_batch = true;
        let mut has_underline = false;
        let width_cache = UnicodeWidthCache::get();
        let mut cur_row = usize::MAX;
        let mut next_free_col = 0usize;
        for (idx, cell) in slice.iter().enumerate() {
            let row = idx / cols;
            let col = idx % cols;
            if row != cur_row {
                cur_row = row;
                next_free_col = 0;
            }
            if cell.skip {
                continue;
            }
            if cell.mods.bold || cell.mods.italic || col < next_free_col {
                can_batch = false;
                break;
            }
            if cell.mods.underline {
                has_underline = true;
            }
            let raw_adv = width_cache.width(cell.ch);
            let adv = usize::from(if raw_adv == 0 { 1 } else { raw_adv });
            next_free_col = col.saturating_add(adv);
        }

        if can_batch {
            self.grid_batched(rect, cols, slice, has_underline, frame);
            return;
        }

        for (idx, cell) in slice.iter().enumerate() {
            if cell.skip {
                continue;
            }
            let raw_adv = width_cache.width(cell.ch);
            let adv = f32::from(if raw_adv == 0 { 1 } else { raw_adv });
            self.glyph_cell(
                rect.x + (idx % cols) as f32,
                rect.y + (idx / cols) as f32,
                adv,
                cell.ch,
                cell.fg,
                cell.bg,
                cell.mods,
                frame,
            );
        }
    }
}

struct BufferUnmapGuard<'a>(&'a wgpu::Buffer);

impl Drop for BufferUnmapGuard<'_> {
    fn drop(&mut self) {
        self.0.unmap();
    }
}

fn validate_target_view(view: &wgpu::TextureView, width: u32, height: u32) -> Result<(), GpuError> {
    let tex = view.texture();
    if tex.format() != TARGET_FORMAT {
        return Err(GpuError::InvalidTarget(format!(
            "target texture format {:?} does not match expected {:?}",
            tex.format(),
            TARGET_FORMAT
        )));
    }
    if !tex.usage().contains(wgpu::TextureUsages::RENDER_ATTACHMENT) {
        return Err(GpuError::InvalidTarget(
            "target texture is missing RENDER_ATTACHMENT usage".into(),
        ));
    }
    if tex.sample_count() != 1 {
        return Err(GpuError::InvalidTarget(format!(
            "target texture sample_count {} is unsupported (expected 1)",
            tex.sample_count()
        )));
    }
    if tex.dimension() != wgpu::TextureDimension::D2 {
        return Err(GpuError::InvalidTarget(format!(
            "target texture dimension {:?} is unsupported (expected D2)",
            tex.dimension()
        )));
    }
    if tex.width() != width || tex.height() != height {
        return Err(GpuError::InvalidTarget(format!(
            "target texture dimensions {}x{} do not match requested render size {width}x{height}",
            tex.width(),
            tex.height()
        )));
    }
    Ok(())
}

fn alpha_blend_state() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn create_globals_resources(
    device: &wgpu::Device,
) -> (wgpu::Buffer, wgpu::BindGroupLayout, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("oxiterm-gpu-globals"),
        size: std::mem::size_of::<GlobalsUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("oxiterm-gpu-globals-bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("oxiterm-gpu-globals-bg"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
    });
    (buffer, layout, bind_group)
}

fn create_atlas_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("oxiterm-gpu-atlas-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn create_sdf_pipeline(
    device: &wgpu::Device,
    globals_bgl: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("oxiterm-gpu-sdf-shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/sdf.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("oxiterm-gpu-sdf-pl"),
        bind_group_layouts: &[Some(globals_bgl)],
        immediate_size: 0,
    });
    let attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("oxiterm-gpu-sdf-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<SdfInstance>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &attrs,
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: TARGET_FORMAT,
                blend: Some(alpha_blend_state()),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_glyph_pipeline(
    device: &wgpu::Device,
    globals_bgl: &wgpu::BindGroupLayout,
    atlas_bgl: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("oxiterm-gpu-glyph-shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/glyph.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("oxiterm-gpu-glyph-pl"),
        bind_group_layouts: &[Some(globals_bgl), Some(atlas_bgl)],
        immediate_size: 0,
    });
    let attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("oxiterm-gpu-glyph-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<GlyphInstance>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &attrs,
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: TARGET_FORMAT,
                blend: Some(alpha_blend_state()),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn ensure_instance_buffer(
    device: &wgpu::Device,
    slot: &mut Option<(wgpu::Buffer, usize)>,
    needed: usize,
    stride: usize,
    label: &str,
) {
    if let Some((_, cap)) = slot {
        if *cap >= needed {
            return;
        }
    }
    let max_cap = ((device.limits().max_buffer_size / stride as u64) as usize).max(1);
    let cap = needed.next_power_of_two().max(256).min(max_cap);
    let size = (cap * stride) as u64;
    if let Some((old, _)) = slot.take() {
        old.destroy();
    }
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    *slot = Some((buf, cap));
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxiterm_proto::dom::NodeId;

    fn test_ctx() -> Option<Arc<GpuContext>> {
        match GpuContext::new_headless() {
            Ok(c) => Some(Arc::new(c)),
            Err(GpuError::NoAdapter(msg)) => {
                eprintln!("SKIP sink unit test: {msg}");
                None
            }
            Err(other) => panic!("GPU context init failed unexpectedly: {other}"),
        }
    }

    #[test]
    fn gpu_sink_and_atlas_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<GpuContext>();
        assert_send::<GlyphAtlas>();
        assert_send::<GpuSink>();
    }

    #[test]
    fn multipass_instance_cap_split_matches_single_pass() {
        let Some(ctx) = test_ctx() else { return };
        let mut items = Vec::new();
        for i in 0..24u32 {
            items.push(DisplayItem::Fill {
                node: NodeId(i + 1),
                rect: RectF::new((i % 8) as f32 * 1.5, (i / 8) as f32 * 1.5, 1.2, 1.2),
                radius: 0.2,
                fg: AnsiColor::Reset,
                bg: AnsiColor::TrueColor((i * 10) as u8, 120, 220),
            });
        }
        items.push(DisplayItem::Text {
            node: NodeId(100),
            origin: PointF { x: 0.5, y: 0.5 },
            clip: RectF::new(0.0, 0.0, 14.0, 5.0),
            glyphs: "SplitPassTest!"
                .chars()
                .enumerate()
                .map(|(i, ch)| Glyph { x: i as f32, y: 0.0, ch, advance: 1.0 })
                .collect(),
            fg: AnsiColor::TrueColor(255, 255, 255),
            bg: AnsiColor::Reset,
            mods: Modifiers::default(),
            shaped: None,
        });
        let list = DisplayList { items };

        let mut single = GpuSink::new(Arc::clone(&ctx), 1.0);
        let ref_img = single.render_cells(&list, 14, 5).unwrap();
        assert_eq!(single.last_stats().passes, 1);

        let mut split = GpuSink::new(ctx, 1.0);
        split.max_sdf_per_pass = 5;
        split.max_glyph_per_pass = 4;
        let split_img = split.render_cells(&list, 14, 5).unwrap();
        assert!(split.last_stats().passes >= 5, "expected multi-pass split, got {:?}", split.last_stats());
        assert_eq!(split_img, ref_img, "multi-pass split must be byte-identical to single-pass");
    }

    #[test]
    fn oversized_instance_buffers_shrink_after_low_usage_frames() {
        let Some(ctx) = test_ctx() else { return };
        let mut sink = GpuSink::new(ctx, 1.0);
        let big: Vec<DisplayItem> = (0..10_000u32)
            .map(|i| DisplayItem::Fill {
                node: NodeId(1),
                rect: RectF::new((i % 50) as f32 * 0.2, (i / 50) as f32 * 0.02, 0.2, 0.2),
                radius: 0.0,
                fg: AnsiColor::Reset,
                bg: AnsiColor::TrueColor((i & 255) as u8, 40, 80),
            })
            .collect();
        let _ = sink.render_cells(&DisplayList { items: big }, 12, 6).unwrap();
        let peak_vec_cap = sink.sdf_instances.capacity();
        let peak_buf_cap = sink.sdf_buf.as_ref().map_or(0, |(_, c)| *c);
        assert!(peak_buf_cap >= MAX_INSTANCES_PER_PASS, "expected grown GPU buffer, got {peak_buf_cap}");
        assert!(peak_vec_cap >= MAX_INSTANCES_PER_PASS, "expected grown CPU Vec, got {peak_vec_cap}");

        let small = DisplayList {
            items: vec![DisplayItem::Fill {
                node: NodeId(1),
                rect: RectF::new(0.0, 0.0, 2.0, 1.0),
                radius: 0.0,
                fg: AnsiColor::Reset,
                bg: AnsiColor::TrueColor(10, 20, 30),
            }],
        };
        for _ in 0..SHRINK_AFTER_LOW_FRAMES {
            let _ = sink.render_cells(&small, 12, 6).unwrap();
        }
        assert!(
            sink.sdf_instances.capacity() <= MIN_RETAINED_CAP,
            "CPU Vec did not shrink: cap={}",
            sink.sdf_instances.capacity()
        );
        assert!(sink.sdf_buf.is_none(), "oversized GPU buffer should be released after low-usage frames");

        // Next small frame reallocates only the small capacity.
        let _ = sink.render_cells(&small, 12, 6).unwrap();
        let new_buf_cap = sink.sdf_buf.as_ref().map_or(0, |(_, c)| *c);
        assert!(new_buf_cap <= MIN_RETAINED_CAP, "reallocated GPU buffer too large: {new_buf_cap}");
    }
}
