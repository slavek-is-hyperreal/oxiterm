//! Shaped text layout (Phase 6, Stage 6.3): the one result that layout measures and, from 6.4
//! on, the display-list builder paints.
//!
//! # Pipeline
//! 1. The text is normalised (`\r` and a trailing `\n` dropped, like `str::lines`) and every
//!    `\n`-separated paragraph is shaped **once, without a width limit**, by the process-wide
//!    [`Shaper`] (the cosmic-text `FontSystem` is built lazily on the first non-default text and
//!    shared by all sessions; see [`with_shaper`]).
//! 2. A glyph the face lacks (id 0, `.notdef`) gets the deterministic advance
//!    `unicode_cell_width(ch) * 9.6 * size_px / 16` px, summed over the cluster, so layout never
//!    depends on which glyphs a face happens to have. Positions are rebased on the new advances.
//! 3. Lines are broken here, greedily, at whitespace only, on those advances. A word wider than
//!    the limit overflows on a line of its own and is never split: the same rule as the mono
//!    path (`word_wrapped_line_count`), and what cosmic-text's `Wrap::Word` does too. (It is also
//!    why wrapping is not delegated: the library would break CJK between ideographs and would
//!    break against the `.notdef` advance before it could be substituted.) Trailing spaces at a
//!    soft break hang outside the line width; the last line of a paragraph keeps them.
//!
//! The unwrapped layout (`max_width == None`) is the base every wrapped layout derives from; it
//! also carries the min-content width (widest word) and max-content width (widest paragraph).
//!
//! # Units
//! Everything in a [`TextLayout`] is in px. Layout converts with `/ CELL_W_PX` (widths) and
//! `/ CELL_H_PX` (heights), see [`TextLayout::max_content_cells`] and friends.
//!
//! # Cache
//! [`TextCache`] is per `LayoutEngine`. Key: exact bits of (normalised text, face, size, line
//! height, bold, italic, max width or `None`). It is bounded and cleared only between frames
//! ([`TextCache::begin_frame`]), and then only entries unused in the previous frame go, so the
//! working set of a page is never reshaped every frame. Layouts are `Arc`, so
//! whoever holds one (the per-node side map of the engine) is unaffected by an eviction.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use oxiterm_proto::style::FontFace;
use parking_lot::Mutex;

use super::bundle::{MONO_TTF, SANS_TTF};
use super::shaping::{RawParagraph, Shaper};
use crate::font::{ResolvedFont, CELL_H_PX, CELL_W_PX, DEFAULT_FONT_SIZE_PX};

/// A word still fits when it overshoots the limit by no more than this (px). Absorbs the f32
/// noise of px to cells to px round trips so the last word cannot wrap on rounding.
pub const WRAP_EPS_PX: f32 = 0.01;
/// A width within this many cells below the max-content width is treated as "wide enough".
pub const GUARD_EPS_CELLS: f32 = 1e-3;
/// Past this many layouts the entries unused in the previous frame are dropped at frame start...
pub const MAX_ENTRIES: usize = 512;
/// ...or more glyphs than this (about 2 MB).
pub const MAX_GLYPHS: usize = 40_000;

/// One positioned glyph. `x` is from the start of its line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedGlyph {
    pub x: f32,
    pub advance: f32,
    pub glyph_id: u16,
    /// The face `glyph_id` belongs to. Normally the layout's own face; another one when the
    /// requested face lacks the character and the shaper fell back to the other bundled face
    /// (box drawing in sans). A painter must draw the glyph in THIS face.
    pub face: FontFace,
    /// Byte range of the cluster in [`TextLayout::text`].
    pub start: u32,
    pub end: u32,
    /// The cluster is whitespace (a break opportunity).
    pub is_space: bool,
    /// The face has no glyph; `advance` is the substituted cell-based advance.
    pub missing: bool,
}

/// One visual line.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutLine {
    /// Byte range of the line in [`TextLayout::text`].
    pub start: u32,
    pub end: u32,
    /// Width in px (trailing spaces of a soft-wrapped line not counted).
    pub width: f32,
    /// Top of the line box, from the top of the text block.
    pub top: f32,
    pub baseline: f32,
    pub glyphs: Vec<PlacedGlyph>,
}

/// A shaped text block at one max width.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    /// Normalised text the glyph byte ranges refer to.
    pub text: Arc<str>,
    pub font: ResolvedFont,
    /// The width limit in px this layout was broken at (`None`: only hard line breaks).
    pub max_width: Option<f32>,
    /// At least one line (empty text is one empty line).
    pub lines: Vec<LayoutLine>,
    /// Widest line.
    pub width: f32,
    /// `lines.len() * line_height_px`.
    pub height_px: f32,
    /// Widest word (px): the narrowest the block can get without splitting a word.
    pub min_content: f32,
    /// Widest paragraph (px): the width with no wrapping.
    pub max_content: f32,
}

impl TextLayout {
    /// Source text of a glyph's cluster.
    pub fn cluster_text(&self, g: &PlacedGlyph) -> &str {
        self.text.get(g.start as usize..g.end as usize).unwrap_or("")
    }
    /// Min-content width in cells.
    pub fn min_content_cells(&self) -> f32 {
        self.min_content / CELL_W_PX
    }
    /// Max-content width in cells.
    pub fn max_content_cells(&self) -> f32 {
        self.max_content / CELL_W_PX
    }
    /// Number of lines (at least 1).
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
    /// Height in cells: `lines * line_height_px / CELL_H_PX`.
    pub fn height_cells(&self) -> f32 {
        self.height_px / CELL_H_PX
    }
}

static SHAPER: OnceLock<Option<Mutex<Shaper>>> = OnceLock::new();

/// Runs `f` on the process-wide shaper, building it on first use (the bundled mono face is font
/// 0, the sans face font 1). `None` if the bundled faces fail to parse. One `FontSystem` serves
/// every session: its state is only font tables and per-call buffers, and shaping is a pure
/// function of its arguments, so sharing cannot change a result.
pub fn with_shaper<R>(f: impl FnOnce(&mut Shaper) -> R) -> Option<R> {
    let m = SHAPER.get_or_init(|| Shaper::new(&[MONO_TTF, SANS_TTF]).map(Mutex::new)).as_ref()?;
    Some(f(&mut m.lock()))
}

/// `true` once the shared shaper has been built (a test and diagnostics hook).
pub fn shaper_built() -> bool {
    SHAPER.get().is_some()
}

fn font_index(face: FontFace) -> usize {
    match face {
        FontFace::Mono => 0,
        FontFace::Sans => 1,
    }
}

/// `str::lines` semantics (no `\r`, no trailing newline) without allocating when already normal.
fn normalize(text: &str) -> Cow<'_, str> {
    if text.contains('\r') || text.ends_with('\n') {
        Cow::Owned(text.lines().collect::<Vec<_>>().join("\n"))
    } else {
        Cow::Borrowed(text)
    }
}

fn is_space_cluster(s: &str) -> bool {
    !s.is_empty() && s.chars().all(char::is_whitespace)
}

/// Advance of a glyph the face lacks.
fn notdef_advance(cluster: &str, size_px: f32) -> f32 {
    let cells: u32 = cluster
        .chars()
        .map(|c| u32::from(crate::render::unicode::UnicodeWidthCache::get().width(c)))
        .sum();
    cells as f32 * CELL_W_PX * size_px / DEFAULT_FONT_SIZE_PX
}

/// Builds the unwrapped layout (one line per paragraph).
fn build_base(text: Arc<str>, font: ResolvedFont) -> Option<TextLayout> {
    let raws: Vec<Option<RawParagraph>> = with_shaper(|s| {
        s.shape_paragraphs(font_index(font.face), &text, font.size_px, font.line_height_px, font.bold)
    })?;
    let lh = font.line_height_px;
    // Paragraphs the library gave no line for (empty ones) borrow the baseline of any other.
    let base_off = raws.iter().flatten().map(|p| p.baseline_off).next().unwrap_or(lh * 0.8);

    let mut lines = Vec::new();
    let (mut min_content, mut max_content) = (0.0f32, 0.0f32);
    let mut off = 0usize;
    for (pi, ptext) in text.split('\n').enumerate() {
        let raw = raws.get(pi).and_then(|r| r.as_ref());
        let mut glyphs = Vec::with_capacity(raw.map_or(0, |r| r.glyphs.len()));
        let mut delta = 0.0f32;
        if let Some(r) = raw {
            for g in &r.glyphs {
                let cluster = ptext.get(g.start..g.end).unwrap_or("");
                let missing = g.glyph_id == 0;
                let advance = if missing { notdef_advance(cluster, font.size_px) } else { g.advance };
                glyphs.push(PlacedGlyph {
                    x: g.x + delta,
                    advance,
                    glyph_id: g.glyph_id,
                    face: match g.font {
                        0 => FontFace::Mono,
                        1 => FontFace::Sans,
                        _ => font.face,
                    },
                    start: (off + g.start) as u32,
                    end: (off + g.end) as u32,
                    is_space: is_space_cluster(cluster),
                    missing,
                });
                delta += advance - g.advance;
            }
        }
        let width = glyphs.last().map_or(0.0, |g| g.x + g.advance);
        // Words: maximal runs of non-space glyphs.
        let mut i = 0;
        while i < glyphs.len() {
            if glyphs[i].is_space {
                i += 1;
                continue;
            }
            let mut j = i;
            while j < glyphs.len() && !glyphs[j].is_space {
                j += 1;
            }
            min_content = min_content.max(glyphs[j - 1].x + glyphs[j - 1].advance - glyphs[i].x);
            i = j;
        }
        max_content = max_content.max(width);
        let top = pi as f32 * lh;
        lines.push(LayoutLine {
            start: off as u32,
            end: (off + ptext.len()) as u32,
            width,
            top,
            baseline: top + raw.map_or(base_off, |r| r.baseline_off),
            glyphs,
        });
        off += ptext.len() + 1;
    }
    let height_px = lines.len().max(1) as f32 * lh;
    Some(TextLayout { text, font, max_width: None, width: max_content, lines, height_px, min_content, max_content })
}

/// Breaks the paragraphs of `base` at `max_px`.
fn wrap(base: &TextLayout, max_px: f32) -> TextLayout {
    let lh = base.font.line_height_px;
    let limit = max_px + WRAP_EPS_PX;
    let mut lines: Vec<LayoutLine> = Vec::new();
    for para in &base.lines {
        let g = &para.glyphs;
        let n = g.len();
        let baseline_off = para.baseline - para.top;
        // One output line = glyph range [ls, le) of the paragraph; `last_end` is the right edge
        // of its last word.
        let emit = |ls: usize, le: usize, width_to: f32, lines: &mut Vec<LayoutLine>| {
            let top = lines.len() as f32 * lh;
            let x0 = g.get(ls).map_or(0.0, |f| f.x);
            let glyphs: Vec<PlacedGlyph> = g[ls..le].iter().map(|p| PlacedGlyph { x: p.x - x0, ..*p }).collect();
            let (start, end) = match (glyphs.first(), glyphs.last()) {
                (Some(a), Some(b)) => (a.start, b.end),
                _ => (para.start, para.end),
            };
            lines.push(LayoutLine { start, end, width: (width_to - x0).max(0.0), top, baseline: top + baseline_off, glyphs });
        };
        if n == 0 {
            emit(0, 0, 0.0, &mut lines);
            continue;
        }
        let mut ls = 0usize;
        let mut has_word = false;
        let mut last_end = 0.0f32;
        let mut i = 0usize;
        while i < n {
            while i < n && g[i].is_space {
                i += 1;
            }
            if i == n {
                break;
            }
            let mut k = i;
            while k < n && !g[k].is_space {
                k += 1;
            }
            let word_end = g[k - 1].x + g[k - 1].advance;
            let x0 = g[ls].x;
            if has_word && word_end - x0 > limit {
                // Break before the word; the spaces before it hang off the previous line.
                emit(ls, i, last_end, &mut lines);
                ls = i;
            }
            has_word = true;
            last_end = word_end;
            i = k;
        }
        // Last line of the paragraph keeps its trailing spaces in the width.
        let end_x = g[n - 1].x + g[n - 1].advance;
        emit(ls, n, end_x, &mut lines);
    }
    let width = lines.iter().map(|l| l.width).fold(0.0, f32::max);
    let height_px = lines.len().max(1) as f32 * lh;
    TextLayout {
        text: base.text.clone(),
        font: base.font,
        max_width: Some(max_px),
        lines,
        width,
        height_px,
        min_content: base.min_content,
        max_content: base.max_content,
    }
}

fn hash_key(text: &str, f: &ResolvedFont, w: Option<f32>) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    (font_index(f.face) as u8).hash(&mut h);
    f.size_px.to_bits().hash(&mut h);
    f.line_height_px.to_bits().hash(&mut h);
    f.bold.hash(&mut h);
    f.italic.hash(&mut h);
    w.map(f32::to_bits).hash(&mut h);
    h.finish()
}

fn same_key(l: &TextLayout, text: &str, f: &ResolvedFont, w: Option<f32>) -> bool {
    l.font.face == f.face
        && l.font.size_px.to_bits() == f.size_px.to_bits()
        && l.font.line_height_px.to_bits() == f.line_height_px.to_bits()
        && l.font.bold == f.bold
        && l.font.italic == f.italic
        && l.max_width.map(f32::to_bits) == w.map(f32::to_bits)
        && &*l.text == text
}

struct Entry {
    /// Frame (see [`TextCache::begin_frame`]) in which the entry was last looked up or inserted.
    stamp: u64,
    layout: Arc<TextLayout>,
}

/// Cache of shaped layouts, one per `LayoutEngine`. See the module docs.
///
/// Bound: when it holds more than [`MAX_ENTRIES`] layouts or [`MAX_GLYPHS`] glyphs at the start
/// of a frame, the entries NOT used in the previous frame are dropped (stale keys: text that
/// changed, animated or bound text producing a new key per frame). The live working set of the
/// document is never dropped, so a big page cannot make every frame reshape; the cache is then
/// as large as the page's text, no larger.
#[derive(Default)]
pub struct TextCache {
    map: HashMap<u64, Vec<Entry>>,
    entries: usize,
    glyphs: usize,
    frame: u64,
}

impl TextCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cached layouts.
    pub fn len(&self) -> usize {
        self.entries
    }

    /// Total glyphs held by the cached layouts.
    pub fn glyph_count(&self) -> usize {
        self.glyphs
    }

    pub fn is_empty(&self) -> bool {
        self.entries == 0
    }

    /// Call at the start of every layout pass. Never evicts mid-frame, so what the solve
    /// measured is still there for the lookups after it.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        if self.entries <= MAX_ENTRIES && self.glyphs <= MAX_GLYPHS {
            return;
        }
        let keep_from = self.frame.saturating_sub(1);
        let (mut entries, mut glyphs) = (0, 0);
        self.map.retain(|_, v| {
            v.retain(|e| e.stamp >= keep_from);
            for e in v.iter() {
                entries += 1;
                glyphs += e.layout.lines.iter().map(|x| x.glyphs.len()).sum::<usize>();
            }
            !v.is_empty()
        });
        self.entries = entries;
        self.glyphs = glyphs;
    }

    fn find(&mut self, h: u64, text: &str, f: &ResolvedFont, w: Option<f32>) -> Option<Arc<TextLayout>> {
        let frame = self.frame;
        let e = self.map.get_mut(&h)?.iter_mut().find(|e| same_key(&e.layout, text, f, w))?;
        e.stamp = frame;
        Some(e.layout.clone())
    }

    fn insert(&mut self, h: u64, l: Arc<TextLayout>) {
        self.entries += 1;
        self.glyphs += l.lines.iter().map(|x| x.glyphs.len()).sum::<usize>();
        self.map.entry(h).or_default().push(Entry { stamp: self.frame, layout: l });
    }

    /// The unwrapped layout of `text` in `font` (shaped on a miss). `None` only when the
    /// bundled faces do not load.
    pub fn base(&mut self, text: &str, font: &ResolvedFont) -> Option<Arc<TextLayout>> {
        let norm = normalize(text);
        let h = hash_key(&norm, font, None);
        if let Some(l) = self.find(h, &norm, font, None) {
            return Some(l);
        }
        let l = Arc::new(build_base(Arc::from(&*norm), *font)?);
        self.insert(h, l.clone());
        Some(l)
    }

    /// `base` broken for a content box `width_cells` wide. When the width reaches the base's
    /// max-content width (the float guard: compared in cells, against the very value measure
    /// returned for MaxContent) the base itself is returned, so the last word can never wrap on
    /// rounding.
    pub fn at_width(&mut self, base: &Arc<TextLayout>, width_cells: f32) -> Arc<TextLayout> {
        if width_cells.is_nan() || width_cells >= base.max_content_cells() - GUARD_EPS_CELLS {
            return base.clone();
        }
        let px = width_cells.max(0.0) * CELL_W_PX;
        let h = hash_key(&base.text, &base.font, Some(px));
        if let Some(l) = self.find(h, &base.text, &base.font, Some(px)) {
            return l;
        }
        let l = Arc::new(wrap(base, px));
        self.insert(h, l.clone());
        l
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sans(size: f32) -> ResolvedFont {
        ResolvedFont { face: FontFace::Sans, size_px: size, line_height_px: size * 1.25, bold: false, italic: false }
    }

    #[test]
    fn unwrapped_one_line_per_paragraph() {
        let mut c = TextCache::new();
        let b = c.base("Hello world\nsecond", &sans(16.0)).unwrap();
        assert_eq!(b.lines.len(), 2);
        assert_eq!(b.height_px, 40.0);
        assert!(b.max_content > b.min_content && b.min_content > 0.0);
        assert_eq!(b.lines[1].top, 20.0);
    }

    #[test]
    fn empty_text_is_one_empty_line() {
        let mut c = TextCache::new();
        for t in ["", "\n"] {
            let b = c.base(t, &sans(16.0)).unwrap();
            assert_eq!(b.lines.len(), 1, "{t:?}");
            assert_eq!(b.max_content, 0.0);
            assert_eq!(b.height_cells(), 1.0);
        }
    }

    #[test]
    fn wrapping_is_monotone_and_never_splits_a_word() {
        let mut c = TextCache::new();
        let t = "The quick brown fox jumps over the lazy dog and keeps running";
        let b = c.base(t, &sans(16.0)).unwrap();
        let mut prev = 0;
        for w in (1..60).rev() {
            let l = c.at_width(&b, w as f32);
            assert!(l.lines.len() >= prev, "wider never gives more lines");
            prev = l.lines.len();
            for line in &l.lines {
                // lines start on a word (the spaces of a break hang off the previous line)
                let s = &l.text[line.start as usize..line.end as usize];
                assert!(!s.starts_with(char::is_whitespace) || line.start == 0);
            }
            let words: Vec<&str> = l.lines.iter().flat_map(|x| l.text[x.start as usize..x.end as usize].split_whitespace()).collect();
            assert_eq!(words.join(" "), t);
        }
        assert_eq!(c.at_width(&b, b.max_content_cells()).lines.len(), 1);
    }

    #[test]
    fn float_guard_returns_the_base() {
        let mut c = TextCache::new();
        let b = c.base("alpha beta gamma", &sans(16.0)).unwrap();
        let w = b.max_content_cells();
        for w in [w, w - 1e-4, w + 3.0] {
            assert!(Arc::ptr_eq(&c.at_width(&b, w), &b));
        }
        assert_eq!(c.at_width(&b, w - 0.5).lines.len(), 2);
    }

    #[test]
    fn long_word_overflows_on_its_own_line() {
        let mut c = TextCache::new();
        let b = c.base("tiny enormouslylongword end", &sans(16.0)).unwrap();
        let l = c.at_width(&b, 2.0);
        assert_eq!(l.lines.len(), 3);
        let widest = l.lines.iter().map(|x| x.width).fold(0.0, f32::max);
        assert!((widest - b.min_content).abs() < 1e-3, "{widest} vs {}", b.min_content);
        assert!(widest > 2.0 * CELL_W_PX);
        // min-content width wraps to exactly one word per line
        assert_eq!(c.at_width(&b, b.min_content_cells()).lines.len(), 3);
    }

    #[test]
    fn missing_glyphs_use_cell_width_advance() {
        let mut c = TextCache::new();
        for (ch, cells) in [("日", 2.0f32), ("😀", 2.0)] {
            for size in [16.0f32, 24.0] {
                let b = c.base(&format!("a{ch}b"), &sans(size)).unwrap();
                let g = &b.lines[0].glyphs;
                assert_eq!(g.len(), 3, "{ch}");
                assert!(g[1].missing && !g[0].missing && !g[2].missing, "{ch}");
                let want = cells * CELL_W_PX * size / 16.0;
                assert_eq!(g[1].advance, want, "{ch} at {size}");
                // later glyphs are rebased on the substituted advance
                assert!((g[2].x - (g[1].x + want)).abs() < 1e-3);
                assert!((b.max_content - (g[2].x + g[2].advance)).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn fallback_glyphs_record_the_face_they_come_from() {
        let mut c = TextCache::new();
        let b = c.base("a\u{2500}b", &sans(16.0)).unwrap();
        let g = &b.lines[0].glyphs;
        assert_eq!(g.len(), 3);
        assert_eq!((g[0].face, g[2].face), (FontFace::Sans, FontFace::Sans));
        // Noto Sans has no box drawing; the glyph comes from the mono face, one cell wide
        assert_eq!(g[1].face, FontFace::Mono);
        assert!(!g[1].missing && (g[1].advance - CELL_W_PX).abs() < 1e-4);
        let m = c.base("a\u{2500}b", &ResolvedFont { face: FontFace::Mono, ..sans(16.0) }).unwrap();
        assert!(m.lines[0].glyphs.iter().all(|g| g.face == FontFace::Mono));
    }

    #[test]
    fn cjk_run_is_one_unbreakable_word_like_the_mono_path() {
        let mut c = TextCache::new();
        let b = c.base("日本語のテキスト", &sans(16.0)).unwrap();
        assert_eq!(c.at_width(&b, 3.0).lines.len(), 1);
        assert_eq!(b.min_content, b.max_content);
        assert!((b.max_content - 8.0 * 2.0 * CELL_W_PX).abs() < 1e-3);
    }

    #[test]
    fn cache_hits_share_and_is_bounded() {
        let mut c = TextCache::new();
        let a = c.base("same", &sans(16.0)).unwrap();
        assert!(Arc::ptr_eq(&a, &c.base("same", &sans(16.0)).unwrap()));
        assert!(!Arc::ptr_eq(&a, &c.base("same", &sans(17.0)).unwrap()), "size is part of the key");
        let mut bold = sans(16.0);
        bold.bold = true;
        assert!(!Arc::ptr_eq(&a, &c.base("same", &bold).unwrap()), "bold is part of the key");
        // Frame 0 inserts more than the bound.
        for i in 0..(MAX_ENTRIES + 10) {
            c.base(&format!("text number {i}"), &sans(16.0)).unwrap();
        }
        assert!(c.len() > MAX_ENTRIES);
        // Frame 1: everything was used in the previous frame, so the live set is kept whole.
        c.begin_frame();
        assert!(c.len() > MAX_ENTRIES, "the working set is never dropped");
        // Only 3 entries are used in frame 1; at frame 2 the rest is stale and goes.
        for i in 0..3 {
            c.base(&format!("text number {i}"), &sans(16.0)).unwrap();
        }
        c.begin_frame();
        assert_eq!(c.len(), 3);
        assert!(c.glyph_count() > 0);
        // a held layout survives the eviction
        assert_eq!(a.text.as_ref(), "same");
        // under the bound nothing is dropped, however stale
        c.begin_frame();
        c.begin_frame();
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn mono_at_24px_is_scaled_cell_advance() {
        let mut c = TextCache::new();
        let f = ResolvedFont { face: FontFace::Mono, size_px: 24.0, line_height_px: 30.0, bold: false, italic: false };
        let b = c.base("abcd", &f).unwrap();
        assert!((b.max_content - 4.0 * 14.4).abs() < 1e-3);
        assert_eq!(b.height_cells(), 1.5);
    }
}
