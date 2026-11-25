//! Glyph lookup and rasterization with a cache.
//!
//! The display list carries chars and cluster text, not glyph ids, so ids are recovered here:
//! - a single char goes through the face's cmap (the face the run says, then the other bundled
//!   face, then nothing: `.notdef` is never painted);
//! - a multi-char cluster (ligature `ffi`, base + combining mark) is shaped again with the
//!   renderer's own `Shaper` on the same two embedded faces, which yields the ligature glyph or
//!   the base + mark glyphs with their relative offsets.
//!
//! Coverage masks come from `swash` (zeno rasterizer: fixed-point, no threads, no SIMD paths
//! chosen at run time), keyed by (face, glyph, size, bold, italic, quarter-pixel x phase), and are
//! cached for the life of the [`GlyphCache`]. A cached mask is the exact bytes a fresh render of
//! the same key produces, so a warm cache cannot change the output.

use std::collections::HashMap;
use std::rc::Rc;

use oxiterm_renderer::render::display_list::{FACE_MONO, FACE_SANS};
use oxiterm_renderer::text::bundle::{FontBundle, MONO_ID, SANS_ID};
use oxiterm_renderer::text::shaping::Shaper;
use swash::scale::{image::Image, Render, ScaleContext, Source};
use swash::zeno::{Angle, Format, Transform, Vector};
use swash::FontRef;

/// Subpixel phases of the x origin (quarter pixels).
pub const SUBPIXEL_STEPS: u8 = 4;
/// Byte budget of the mask and cluster caches together. When it is exceeded the whole cache is
/// dropped (deterministic; output is unaffected, a miss re-renders the identical bytes).
pub const CACHE_BUDGET_BYTES: usize = 64 << 20;
/// Accounting overhead per cache entry (key, map slot, `Rc`), also charged to empty masks.
const ENTRY_OVERHEAD: usize = 96;
/// Smallest and largest glyph pixel size ever rasterized or shaped. Anything outside is skipped.
/// (A 512 px glyph mask is well under 1 MiB.)
pub const MIN_GLYPH_PX: f32 = 0.5;
pub const MAX_GLYPH_PX: f32 = 512.0;
/// Longest cluster text that is re-shaped; longer ones fall back to their first char.
pub const MAX_CLUSTER_CHARS: usize = 64;

/// `true` for a pixel size the cache will rasterize or shape.
pub fn size_ok(px: f32) -> bool {
    px.is_finite() && (MIN_GLYPH_PX..=MAX_GLYPH_PX).contains(&px)
}
/// Synthetic bold strength as a fraction of the pixel size (outline growth per side).
pub const BOLD_STRENGTH: f32 = 0.035;
/// Synthetic italic slant, degrees.
pub const ITALIC_DEGREES: f32 = 14.0;

/// A rasterized glyph: 8-bit coverage, positioned relative to the pen origin on the baseline
/// (`left` right of the origin, `top` above the baseline, both in whole pixels).
#[derive(Debug)]
pub struct GlyphImage {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClusterGlyph {
    pub face: u8,
    pub gid: u16,
    /// Offset from the cluster origin in px (at the size the cluster was shaped with).
    pub dx: f32,
}

#[derive(Hash, PartialEq, Eq)]
struct MaskKey {
    face: u8,
    gid: u16,
    size_bits: u32,
    bold: bool,
    italic: bool,
    sub: u8,
}

pub struct GlyphCache {
    fonts: [FontRef<'static>; 2],
    ctx: ScaleContext,
    shaper: Shaper,
    masks: HashMap<MaskKey, Option<Rc<GlyphImage>>>,
    clusters: HashMap<(u8, Box<str>, u32), Rc<[ClusterGlyph]>>,
    /// Baseline below the top of a 20 px line for the mono face at 16 px (what the shaper uses).
    mono_baseline_px: f32,
    bytes: usize,
    pub(crate) mask_hits: u64,
    pub(crate) mask_misses: u64,
}

fn load(id: &str) -> FontRef<'static> {
    let bytes = FontBundle::font_bytes(id).expect("bundled face");
    FontRef::from_index(bytes, 0).expect("bundled face parses")
}

impl GlyphCache {
    pub fn new() -> Self {
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
        Self {
            fonts: [load(MONO_ID), load(SANS_ID)],
            ctx: ScaleContext::new(),
            shaper,
            masks: HashMap::new(),
            clusters: HashMap::new(),
            mono_baseline_px,
            bytes: 0,
            mask_hits: 0,
            mask_misses: 0,
        }
    }

    /// Distance from the top of a cell to the baseline of default (16 px, 20 px line) text, in
    /// px at scale 1: the same number the shaper uses for a 16 px mono shaped run.
    pub fn mono_baseline_px(&self) -> f32 {
        self.mono_baseline_px
    }

    /// Number of cached masks (tests).
    pub fn mask_count(&self) -> usize {
        self.masks.len()
    }

    /// Drops every cached mask and cluster.
    pub fn clear(&mut self) {
        self.masks.clear();
        self.clusters.clear();
        self.bytes = 0;
    }

    /// Accounted bytes of both caches (tests).
    pub fn cached_bytes(&self) -> usize {
        self.bytes
    }

    fn charge(&mut self, n: usize) {
        if self.bytes.saturating_add(n) > CACHE_BUDGET_BYTES {
            self.clear();
        }
        self.bytes += n;
    }

    fn lookup(&self, face: u8, ch: char) -> u16 {
        self.fonts[(face as usize).min(1)].charmap().map(ch)
    }

    /// Glyph of one char: the requested face, else the other bundled face. `None` when neither
    /// has it (the glyph is not painted).
    pub fn char_glyph(&self, face: u8, ch: char) -> Option<(u8, u16)> {
        let face = face.min(FACE_SANS);
        let g = self.lookup(face, ch);
        if g != 0 {
            return Some((face, g));
        }
        let other = 1 - face;
        let g = self.lookup(other, ch);
        (g != 0).then_some((other, g))
    }

    /// Glyphs of a multi-char cluster at `size_px`, shaped in `face`.
    pub fn cluster_glyphs(&mut self, face: u8, text: &str, size_px: f32) -> Rc<[ClusterGlyph]> {
        let face = face.min(FACE_SANS);
        if !size_ok(size_px) || text.is_empty() || text.chars().nth(MAX_CLUSTER_CHARS).is_some() {
            return Rc::from([]);
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
        let rc: Rc<[ClusterGlyph]> = glyphs.into();
        self.charge(ENTRY_OVERHEAD + text.len() + rc.len() * std::mem::size_of::<ClusterGlyph>());
        self.clusters.insert(key, rc.clone());
        rc
    }

    /// Coverage mask of a glyph, `sub` quarter pixels right of the pixel origin.
    pub fn mask(&mut self, face: u8, gid: u16, size_px: f32, bold: bool, italic: bool, sub: u8) -> Option<Rc<GlyphImage>> {
        if !size_ok(size_px) {
            return None;
        }
        let face = face.min(FACE_SANS);
        let key = MaskKey { face, gid, size_bits: size_px.to_bits(), bold, italic, sub };
        if let Some(m) = self.masks.get(&key) {
            self.mask_hits += 1;
            return m.clone();
        }
        self.mask_misses += 1;
        let img = self.render(&key);
        self.charge(ENTRY_OVERHEAD + img.as_ref().map_or(0, |m| m.data.len()));
        self.masks.insert(key, img.clone());
        img
    }

    fn render(&mut self, key: &MaskKey) -> Option<Rc<GlyphImage>> {
        let font = self.fonts[(key.face as usize).min(1)];
        let size = f32::from_bits(key.size_bits);
        let mut scaler = self.ctx.builder(font).size(size).hint(false).build();
        let mut render = Render::new(&[Source::Outline]);
        render.format(Format::Alpha).offset(Vector::new(f32::from(key.sub) / f32::from(SUBPIXEL_STEPS), 0.0));
        if key.bold {
            render.embolden(size * BOLD_STRENGTH);
        }
        if key.italic {
            render.transform(Some(Transform::skew(Angle::from_degrees(ITALIC_DEGREES), Angle::from_degrees(0.0))));
        }
        let mut image = Image::new();
        if !render.render_into(&mut scaler, key.gid, &mut image) {
            return None;
        }
        let p = image.placement;
        if p.width == 0 || p.height == 0 {
            return None;
        }
        Some(Rc::new(GlyphImage { left: p.left, top: p.top, width: p.width, height: p.height, data: image.data }))
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new()
    }
}
