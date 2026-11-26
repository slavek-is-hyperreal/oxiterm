//! CPU-rasterized glyph atlas packed with `etagere` into an `R8Unorm` GPU texture.
//!
//! Uses the exact same bundled fonts (`NotoSansMono-Regular.ttf`, `NotoSans-Regular.ttf`),
//! `swash` outline rasterizer (quarter-pixel horizontal subpixel phases, synthetic bold/italic),
//! and `oxiterm-renderer` cluster `Shaper` as `oxiterm-fb` so GPU text matches CPU reference
//! rendering pixel-for-pixel.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use etagere::{size2, AtlasAllocator};
use oxiterm_renderer::render::display_list::{FACE_MONO, FACE_SANS};
use oxiterm_renderer::text::bundle::{FontBundle, MONO_ID, SANS_ID};
use oxiterm_renderer::text::shaping::Shaper;
use swash::scale::{image::Image, Render, ScaleContext, Source};
use swash::zeno::{Angle, Format, Transform, Vector};
use swash::FontRef;

/// Subpixel phases of the x origin (quarter pixels), matching `oxiterm-fb`.
pub const SUBPIXEL_STEPS: u8 = 4;
/// Initial atlas texture side length in pixels (1024x1024 `R8Unorm` = 1 MiB).
pub const INITIAL_ATLAS_SIZE: u32 = 1024;
/// Maximum atlas texture side length (capped further by device `max_texture_dimension_2d`).
pub const MAX_ATLAS_SIZE: u32 = 2048;
/// Smallest glyph pixel size rasterized or shaped.
pub const MIN_GLYPH_PX: f32 = 0.5;
/// Largest glyph pixel size rasterized or shaped (bounds per-glyph atlas space).
pub const MAX_GLYPH_PX: f32 = 512.0;
/// Longest cluster text re-shaped; longer ones fall back to their first char.
pub const MAX_CLUSTER_CHARS: usize = 64;
/// Synthetic bold strength as a fraction of pixel size (matches `oxiterm-fb`).
pub const BOLD_STRENGTH: f32 = 0.035;
/// Synthetic italic slant in degrees (matches `oxiterm-fb`).
pub const ITALIC_DEGREES: f32 = 14.0;
/// Byte budget for the CPU cluster cache before clearing.
const CLUSTER_CACHE_BUDGET: usize = 8 << 20;
const CLUSTER_ENTRY_OVERHEAD: usize = 96;
/// Byte budget for the glyph entry map (mirrors `oxiterm-fb`'s bounded cache design).
pub const ENTRY_CACHE_BUDGET: usize = 16 << 20;
const MASK_ENTRY_OVERHEAD: usize = 96;
/// Hard upper bound on cached visible glyph entries before triggering an atlas flush + reset.
const MAX_ATLAS_ENTRIES: usize = 65_536;
/// Small bounded set of blank/outline-less glyph keys so repeated zero-width lookups stay fast
/// without unbounded growth across distinct sizes.
const MAX_EMPTY_ENTRIES: usize = 512;
/// 1 px zero border on each side of every packed glyph to prevent neighbour bleed.
const GLYPH_PAD: u32 = 1;

/// Returns `true` for a pixel size the atlas will rasterize or shape.
pub fn size_ok(px: f32) -> bool {
    px.is_finite() && (MIN_GLYPH_PX..=MAX_GLYPH_PX).contains(&px)
}

/// One shaped glyph within a multi-char cluster.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClusterGlyph {
    pub face: u8,
    pub gid: u16,
    /// Horizontal offset from the cluster origin in px.
    pub dx: f32,
}

/// Placement and atlas pixel coordinates of a cached glyph mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasEntry {
    /// Horizontal bearing from pen origin to left edge of mask (px).
    pub left: i32,
    /// Vertical bearing from baseline up to top edge of mask (px).
    pub top: i32,
    /// Mask width in pixels (excluding padding).
    pub width: u32,
    /// Mask height in pixels (excluding padding).
    pub height: u32,
    /// X coordinate of the mask's top-left pixel inside the atlas texture.
    pub atlas_x: u32,
    /// Y coordinate of the mask's top-left pixel inside the atlas texture.
    pub atlas_y: u32,
}

/// Outcome of requesting a glyph in the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtlasLookup {
    /// Glyph is resident in the atlas (newly uploaded or cache hit).
    Hit(AtlasEntry),
    /// Glyph has no visible outline (e.g. whitespace or empty glyph).
    Empty,
    /// The atlas texture has no space left for this glyph; caller should grow or flush+reset.
    Full,
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct MaskKey {
    face: u8,
    gid: u16,
    size_bits: u32,
    bold: bool,
    italic: bool,
    sub: u8,
}

fn load_font(id: &str) -> FontRef<'static> {
    let bytes = FontBundle::font_bytes(id).expect("bundled face exists");
    FontRef::from_index(bytes, 0).expect("bundled face parses")
}

/// Dynamic GPU glyph atlas backed by `etagere`, `swash`, and an `R8Unorm` `wgpu::Texture`.
pub struct GlyphAtlas {
    fonts: [FontRef<'static>; 2],
    scale_ctx: ScaleContext,
    shaper: Shaper,
    mono_baseline_px: f32,

    atlas_size: u32,
    max_atlas_size: u32,
    allocator: AtlasAllocator,
    texture: wgpu::Texture,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,

    entries: HashMap<MaskKey, AtlasEntry>,
    entry_bytes: usize,
    empty_glyphs: HashSet<MaskKey>,
    clusters: HashMap<(u8, Box<str>, u32), Arc<[ClusterGlyph]>>,
    cluster_bytes: usize,

    hits: u64,
    misses: u64,
    resets: u64,
    dropped_oversized_glyphs: u64,
}

impl GlyphAtlas {
    /// Creates a new glyph atlas texture and bind group on `device`.
    pub fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let mono = FontBundle::font_bytes(MONO_ID).expect("mono face");
        let sans = FontBundle::font_bytes(SANS_ID).expect("sans face");
        let mut shaper = Shaper::new(&[mono, sans]).expect("bundled faces shape");
        let mono_baseline_px = shaper
            .shape_paragraphs(FACE_MONO as usize, "M", 16.0, 20.0, false)
            .into_iter()
            .next()
            .flatten()
            .map(|p| p.baseline_off)
            .expect("mono baseline");

        let max_dim = device.limits().max_texture_dimension_2d;
        let max_atlas_size = MAX_ATLAS_SIZE.min(max_dim).max(256);
        let atlas_size = INITIAL_ATLAS_SIZE.min(max_atlas_size);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("oxiterm-glyph-atlas-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            // Nearest sampling keeps pixel-aligned swash masks crisp and 1:1 with oxiterm-fb.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let (texture, bind_group) = create_atlas_texture(device, layout, &sampler, atlas_size);
        let allocator = AtlasAllocator::new(size2(atlas_size.cast_signed(), atlas_size.cast_signed()));

        Self {
            fonts: [load_font(MONO_ID), load_font(SANS_ID)],
            scale_ctx: ScaleContext::new(),
            shaper,
            mono_baseline_px,
            atlas_size,
            max_atlas_size,
            allocator,
            texture,
            sampler,
            bind_group,
            entries: HashMap::new(),
            entry_bytes: 0,
            empty_glyphs: HashSet::new(),
            clusters: HashMap::new(),
            cluster_bytes: 0,
            hits: 0,
            misses: 0,
            resets: 0,
            dropped_oversized_glyphs: 0,
        }
    }

    /// Distance from the top of a cell to the baseline of default (16 px, 20 px line) mono text.
    pub fn mono_baseline_px(&self) -> f32 {
        self.mono_baseline_px
    }

    /// Current square side length of the atlas texture in pixels.
    pub fn size(&self) -> u32 {
        self.atlas_size
    }

    /// Current GPU bind group (`@binding(0)` texture view + `@binding(1)` sampler).
    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Number of glyph entries currently cached.
    pub fn glyph_count(&self) -> usize {
        self.entries.len()
    }

    /// Cache hit counter (for diagnostics and tests).
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Cache miss counter (for diagnostics and tests).
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Number of atlas clears/resizes performed.
    pub fn resets(&self) -> u64 {
        self.resets
    }

    /// Cumulative number of glyphs/runs dropped because their scaled pixel size exceeded
    /// [`MAX_GLYPH_PX`] (512 px).
    pub fn dropped_oversized_glyphs(&self) -> u64 {
        self.dropped_oversized_glyphs
    }

    pub(crate) fn record_oversized_glyph(&mut self, count: u64) {
        self.dropped_oversized_glyphs = self.dropped_oversized_glyphs.saturating_add(count);
    }

    /// Returns `true` if the atlas can still double in size before hitting `max_atlas_size`.
    pub fn can_grow(&self) -> bool {
        self.atlas_size < self.max_atlas_size
    }

    /// Doubles the atlas size (up to `max_atlas_size`), recreating a zero-initialised texture and
    /// clearing all entries. Returns `false` if already at maximum size.
    pub fn grow(&mut self, device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> bool {
        if !self.can_grow() {
            return false;
        }
        let next = (self.atlas_size.saturating_mul(2)).min(self.max_atlas_size);
        self.atlas_size = next;
        self.reset(device, layout);
        true
    }

    /// Clears all cached glyphs and allocates a fresh zero-initialised GPU atlas texture.
    pub fn reset(&mut self, device: &wgpu::Device, layout: &wgpu::BindGroupLayout) {
        let (texture, bind_group) =
            create_atlas_texture(device, layout, &self.sampler, self.atlas_size);
        self.texture.destroy();
        self.texture = texture;
        self.bind_group = bind_group;
        self.allocator =
            AtlasAllocator::new(size2(self.atlas_size.cast_signed(), self.atlas_size.cast_signed()));
        if self.entries.capacity() > 4096 {
            self.entries = HashMap::new();
        } else {
            self.entries.clear();
        }
        self.entry_bytes = 0;
        self.empty_glyphs.clear();
        self.resets += 1;
    }

    fn lookup_cmap(&self, face: u8, ch: char) -> u16 {
        self.fonts[(face as usize).min(1)].charmap().map(ch)
    }

    /// Maps one character to `(face, glyph_id)` using the requested face then the other bundled
    /// face. Returns `None` when neither face contains the character (`.notdef` is never drawn).
    pub fn char_glyph(&self, face: u8, ch: char) -> Option<(u8, u16)> {
        let face = face.min(FACE_SANS);
        let g = self.lookup_cmap(face, ch);
        if g != 0 {
            return Some((face, g));
        }
        let other = 1 - face;
        let g = self.lookup_cmap(other, ch);
        (g != 0).then_some((other, g))
    }

    /// Shapes a multi-character cluster (e.g. ligature or combining mark) at `size_px`.
    pub fn cluster_glyphs(&mut self, face: u8, text: &str, size_px: f32) -> Arc<[ClusterGlyph]> {
        let face = face.min(FACE_SANS);
        if !size_ok(size_px) || text.is_empty() || text.chars().nth(MAX_CLUSTER_CHARS).is_some() {
            return Arc::from([]);
        }
        let key = (face, Box::<str>::from(text), size_px.to_bits());
        if let Some(c) = self.clusters.get(&key) {
            return c.clone();
        }
        let paras = self.shaper.shape_paragraphs(face as usize, text, size_px, size_px * 1.25, false);
        let glyphs: Vec<ClusterGlyph> = paras
            .into_iter()
            .next()
            .flatten()
            .map(|p| {
                p.glyphs
                    .iter()
                    .filter(|g| g.glyph_id != 0)
                    .map(|g| ClusterGlyph {
                        face: if g.font <= 1 { g.font as u8 } else { face },
                        gid: g.glyph_id,
                        dx: g.x,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let arc: Arc<[ClusterGlyph]> = glyphs.into();
        let cost = CLUSTER_ENTRY_OVERHEAD + text.len() + arc.len() * std::mem::size_of::<ClusterGlyph>();
        if self.cluster_bytes.saturating_add(cost) > CLUSTER_CACHE_BUDGET {
            self.clusters.clear();
            self.cluster_bytes = 0;
        }
        self.cluster_bytes += cost;
        self.clusters.insert(key, arc.clone());
        arc
    }

    fn remember_empty(&mut self, key: MaskKey) {
        if self.empty_glyphs.len() >= MAX_EMPTY_ENTRIES {
            self.empty_glyphs.clear();
        }
        self.empty_glyphs.insert(key);
    }

    /// Looks up or rasterizes + uploads a glyph mask into the GPU atlas.
    #[allow(clippy::too_many_arguments)]
    pub fn get_or_insert(
        &mut self,
        queue: &wgpu::Queue,
        face: u8,
        gid: u16,
        size_px: f32,
        bold: bool,
        italic: bool,
        sub: u8,
    ) -> AtlasLookup {
        if !size_ok(size_px) {
            if size_px.is_finite() && size_px > MAX_GLYPH_PX {
                self.record_oversized_glyph(1);
            }
            return AtlasLookup::Empty;
        }
        let face = face.min(FACE_SANS);
        let key = MaskKey { face, gid, size_bits: size_px.to_bits(), bold, italic, sub };
        if let Some(entry) = self.entries.get(&key) {
            self.hits += 1;
            return AtlasLookup::Hit(*entry);
        }
        if self.empty_glyphs.contains(&key) {
            self.hits += 1;
            return AtlasLookup::Empty;
        }

        // Rasterize with swash identically to oxiterm-fb.
        let font = self.fonts[face as usize];
        let mut scaler = self.scale_ctx.builder(font).size(size_px).hint(false).build();
        let mut render = Render::new(&[Source::Outline]);
        render
            .format(Format::Alpha)
            .offset(Vector::new(f32::from(sub) / f32::from(SUBPIXEL_STEPS), 0.0));
        if bold {
            render.embolden(size_px * BOLD_STRENGTH);
        }
        if italic {
            render.transform(Some(Transform::skew(
                Angle::from_degrees(ITALIC_DEGREES),
                Angle::from_degrees(0.0),
            )));
        }
        let mut image = Image::new();
        if !render.render_into(&mut scaler, gid, &mut image) {
            self.misses += 1;
            self.remember_empty(key);
            return AtlasLookup::Empty;
        }
        let p = image.placement;
        if p.width == 0 || p.height == 0 {
            self.misses += 1;
            self.remember_empty(key);
            return AtlasLookup::Empty;
        }

        let padded_w = p.width.saturating_add(GLYPH_PAD * 2);
        let padded_h = p.height.saturating_add(GLYPH_PAD * 2);
        if padded_w > self.atlas_size || padded_h > self.atlas_size {
            // Single glyph exceeds current atlas dimension; caller may grow or skip.
            return if self.can_grow() && padded_w <= self.max_atlas_size && padded_h <= self.max_atlas_size {
                AtlasLookup::Full
            } else {
                self.misses += 1;
                self.remember_empty(key);
                AtlasLookup::Empty
            };
        }

        // Enforce entry-count and metadata-byte budgets so the map never grows without bound.
        if !self.entries.is_empty()
            && (self.entries.len() >= MAX_ATLAS_ENTRIES
                || self.entry_bytes.saturating_add(MASK_ENTRY_OVERHEAD) > ENTRY_CACHE_BUDGET)
        {
            return AtlasLookup::Full;
        }

        let Some(alloc) =
            self.allocator.allocate(size2(padded_w.cast_signed(), padded_h.cast_signed()))
        else {
            return AtlasLookup::Full;
        };

        let atlas_x = (alloc.rectangle.min.x as u32) + GLYPH_PAD;
        let atlas_y = (alloc.rectangle.min.y as u32) + GLYPH_PAD;

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: atlas_x, y: atlas_y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &image.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(p.width),
                rows_per_image: Some(p.height),
            },
            wgpu::Extent3d { width: p.width, height: p.height, depth_or_array_layers: 1 },
        );

        let entry = AtlasEntry {
            left: p.left,
            top: p.top,
            width: p.width,
            height: p.height,
            atlas_x,
            atlas_y,
        };
        self.misses += 1;
        self.entry_bytes = self.entry_bytes.saturating_add(MASK_ENTRY_OVERHEAD);
        self.entries.insert(key, entry);
        AtlasLookup::Hit(entry)
    }
}

fn create_atlas_texture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    size: u32,
) -> (wgpu::Texture, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("oxiterm-glyph-atlas"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("oxiterm-glyph-atlas-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    (texture, bind_group)
}
