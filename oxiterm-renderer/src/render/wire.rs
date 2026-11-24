//! Binary wire format for [`DisplayList`] (OxiUI phase 3).
//!
//! Compact, little-endian, dependency-free (builds for `wasm32`). The byte layout is
//! documented in `docs/ARCHITECTURE.md` ("Display-list protocol"). Summary:
//!
//! ```text
//! list   := version:u8(=2 or 3) count:u32 item*count
//! item   := tag:u8 body
//! colour := 0 | 1 r g b | 2 idx            (Reset | TrueColor | Color256)
//! rect   := x:f32 y:f32 w:f32 h:f32
//! Fill   := node:u32 rect radius:f32 fg bg
//! Border := node:u32 rect radius:f32 8*char:u32 fg bg
//! Text   := node:u32 origin:f32*2 clip:rect fg bg mods:u8 n:u32 (x y ch:u32 advance)*n
//! Run    := (v3 only, tag 9) the Text fields up to `mods`, then face:u8 size:f32 line_height:f32
//!           baseline:f32 n:u32 (x y ch:u32 advance)*n nf:u32 (idx:u32 face:u8)*nf
//!           nc:u32 (idx:u32 len:u32 utf8*len)*nc      (proportional text, see `ShapedRun`)
//! Path   := node:u32 n:u32 cmd*n fill_flag:u8 [colour] stroke_flag:u8 [colour width:f32]
//! cmd    := 0 x y | 1 x y | 2 cx cy x y | 3 c1x c1y c2x c2y x y | 4   (op:u8, f32 operands)
//! ```
//!
//! `decode` never panics on malformed input: it uses a bounds-checked cursor, validates
//! `char` code points, rejects non-finite `f32`s, caps every element count by the bytes that remain, and rejects
//! trailing bytes.

use anyhow::{anyhow, bail, ensure, Result};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{AnsiColor, BorderChars};

use crate::render::display_list::{DisplayItem, DisplayList, Glyph, GridCell, Modifiers, PathCmd, PointF, RectF, ShapedRun};

/// Baseline wire format version: what [`encode`] writes and every client understands.
pub const WIRE_VERSION: u8 = 2;
/// Version 3 adds the shaped text run (tag 9). It is written only by [`encode_for`], only for
/// clients that announced it (`0x12` caps bit 1), and only for a list that contains a shaped
/// run; every other list is byte-identical to version 2.
pub const WIRE_VERSION_3: u8 = 3;

const TAG_FILL: u8 = 1;
const TAG_BORDER: u8 = 2;
const TAG_TEXT: u8 = 3;
const TAG_LEGACY: u8 = 4;
const TAG_PUSH_CLIP: u8 = 5;
const TAG_POP_CLIP: u8 = 6;
const TAG_CELL_GRID: u8 = 7;
const TAG_PATH: u8 = 8;
const TAG_TEXT_RUN: u8 = 9;

const OP_MOVE: u8 = 0;
const OP_LINE: u8 = 1;
const OP_QUAD: u8 = 2;
const OP_CUBIC: u8 = 3;
const OP_CLOSE: u8 = 4;

const MOD_BOLD: u8 = 1;
const MOD_UNDERLINE: u8 = 2;
const MOD_ITALIC: u8 = 4;
const MOD_SKIP: u8 = 8;

/// Smallest encoded size of one glyph (x, y, ch, advance) and one grid cell (ch, fg, bg, mods).
const GLYPH_BYTES: usize = 16;
const CELL_MIN_BYTES: usize = 7;

/// Serializes a display list as version 2 (the baseline): a shaped text run is down-converted
/// by [`downgrade_v2`] on the way out.
pub fn encode(list: &DisplayList) -> Vec<u8> {
    encode_version(list, WIRE_VERSION)
}

/// Serializes `list` for a client that understands wire versions up to `max_version`: version 3
/// when `max_version >= 3` and the list holds a shaped run, version 2 otherwise (so a list
/// without proportional text is byte-identical for every client).
pub fn encode_for(list: &DisplayList, max_version: u8) -> Vec<u8> {
    let v3 = max_version >= WIRE_VERSION_3
        && list.items.iter().any(|i| matches!(i, DisplayItem::Text { shaped: Some(_), .. }));
    encode_version(list, if v3 { WIRE_VERSION_3 } else { WIRE_VERSION })
}

/// The glyphs a version-2 client gets for a shaped run: one glyph per char, a cluster's chars
/// all at the cluster's `x`/`y` (the first one keeps the advance, the others get `0.0`). Face,
/// size, baseline and line height are dropped; `y` stays the top of the line box.
fn expand_clusters(glyphs: &[Glyph], run: &ShapedRun) -> Vec<Glyph> {
    let mut out = Vec::with_capacity(glyphs.len());
    for (i, g) in glyphs.iter().enumerate() {
        match run.cluster_of(i) {
            Some(text) => {
                for (k, ch) in text.chars().enumerate() {
                    out.push(Glyph { ch, advance: if k == 0 { g.advance } else { 0.0 }, ..*g });
                }
            }
            None => out.push(*g),
        }
    }
    out
}

/// What a version-2 client sees of `list`: every shaped run replaced by its [`expand_clusters`]
/// cell-style run (`shaped: None`). `decode(&encode(list)) == downgrade_v2(list)`.
pub fn downgrade_v2(list: &DisplayList) -> DisplayList {
    let items = list
        .items
        .iter()
        .map(|item| match item {
            DisplayItem::Text { node, origin, clip, glyphs, fg, bg, mods, shaped: Some(run) } => DisplayItem::Text {
                node: *node,
                origin: *origin,
                clip: *clip,
                glyphs: expand_clusters(glyphs, run),
                fg: *fg,
                bg: *bg,
                mods: *mods,
                shaped: None,
            },
            other => other.clone(),
        })
        .collect();
    DisplayList { items }
}

fn encode_version(list: &DisplayList, version: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + list.items.len() * 32);
    out.push(version);
    put_u32(&mut out, list.items.len() as u32);
    for item in &list.items {
        match item {
            DisplayItem::Fill { node, rect, radius, fg, bg } => {
                out.push(TAG_FILL);
                put_u32(&mut out, node.0);
                put_rect(&mut out, *rect);
                put_f32(&mut out, *radius);
                put_color(&mut out, *fg);
                put_color(&mut out, *bg);
            }
            DisplayItem::Border { node, rect, radius, chars, fg, bg } => {
                out.push(TAG_BORDER);
                put_u32(&mut out, node.0);
                put_rect(&mut out, *rect);
                put_f32(&mut out, *radius);
                for c in [
                    chars.top_left, chars.top, chars.top_right, chars.left,
                    chars.right, chars.bot_left, chars.bot, chars.bot_right,
                ] {
                    put_u32(&mut out, c as u32);
                }
                put_color(&mut out, *fg);
                put_color(&mut out, *bg);
            }
            DisplayItem::Text { node, origin, clip, glyphs, fg, bg, mods, shaped } => {
                let run = shaped.as_deref().filter(|_| version >= WIRE_VERSION_3);
                out.push(if run.is_some() { TAG_TEXT_RUN } else { TAG_TEXT });
                put_u32(&mut out, node.0);
                put_f32(&mut out, origin.x);
                put_f32(&mut out, origin.y);
                put_rect(&mut out, *clip);
                put_color(&mut out, *fg);
                put_color(&mut out, *bg);
                out.push(mods_byte(*mods, false));
                if let Some(r) = run {
                    out.push(r.face);
                    put_f32(&mut out, r.size_px);
                    put_f32(&mut out, r.line_height);
                    put_f32(&mut out, r.baseline);
                }
                // A shaped run sent to a v2 client is flattened to one glyph per char.
                let flat;
                let glyphs: &[Glyph] = match (shaped.as_deref(), run) {
                    (Some(r), None) => {
                        flat = expand_clusters(glyphs, r);
                        &flat
                    }
                    _ => glyphs,
                };
                put_u32(&mut out, glyphs.len() as u32);
                for g in glyphs {
                    put_f32(&mut out, g.x);
                    put_f32(&mut out, g.y);
                    put_u32(&mut out, g.ch as u32);
                    put_f32(&mut out, g.advance);
                }
                if let Some(r) = run {
                    put_u32(&mut out, r.glyph_faces.len() as u32);
                    for (i, f) in &r.glyph_faces {
                        put_u32(&mut out, *i);
                        out.push(*f);
                    }
                    put_u32(&mut out, r.clusters.len() as u32);
                    for (i, t) in &r.clusters {
                        put_u32(&mut out, *i);
                        put_u32(&mut out, t.len() as u32);
                        out.extend_from_slice(t.as_bytes());
                    }
                }
            }
            DisplayItem::Legacy { node, content } => {
                out.push(TAG_LEGACY);
                put_u32(&mut out, node.0);
                put_rect(&mut out, *content);
            }
            DisplayItem::PushClip { rect } => {
                out.push(TAG_PUSH_CLIP);
                put_rect(&mut out, *rect);
            }
            DisplayItem::PopClip => out.push(TAG_POP_CLIP),
            DisplayItem::Path { node, cmds, fill, stroke } => {
                out.push(TAG_PATH);
                put_u32(&mut out, node.0);
                put_u32(&mut out, cmds.len() as u32);
                for c in cmds {
                    match c {
                        PathCmd::MoveTo(p) => {
                            out.push(OP_MOVE);
                            put_pt(&mut out, *p);
                        }
                        PathCmd::LineTo(p) => {
                            out.push(OP_LINE);
                            put_pt(&mut out, *p);
                        }
                        PathCmd::QuadTo(c1, p) => {
                            out.push(OP_QUAD);
                            put_pt(&mut out, *c1);
                            put_pt(&mut out, *p);
                        }
                        PathCmd::CubicTo(c1, c2, p) => {
                            out.push(OP_CUBIC);
                            put_pt(&mut out, *c1);
                            put_pt(&mut out, *c2);
                            put_pt(&mut out, *p);
                        }
                        PathCmd::Close => out.push(OP_CLOSE),
                    }
                }
                match fill {
                    Some(c) => {
                        out.push(1);
                        put_color(&mut out, *c);
                    }
                    None => out.push(0),
                }
                match stroke {
                    Some((c, w)) => {
                        out.push(1);
                        put_color(&mut out, *c);
                        put_f32(&mut out, *w);
                    }
                    None => out.push(0),
                }
            }
            DisplayItem::CellGrid { rect, cells } => {
                out.push(TAG_CELL_GRID);
                put_rect(&mut out, *rect);
                put_u32(&mut out, cells.len() as u32);
                for c in cells {
                    put_u32(&mut out, c.ch as u32);
                    put_color(&mut out, c.fg);
                    put_color(&mut out, c.bg);
                    out.push(mods_byte(c.mods, c.skip));
                }
            }
        }
    }
    out
}

/// Parses a display list (version 2 or 3). Errors (never panics) on truncated input, an unknown
/// version or tag, an invalid code point or colour tag, a malformed shaped run, or trailing bytes.
pub fn decode(bytes: &[u8]) -> Result<DisplayList> {
    let mut r = Reader { buf: bytes, pos: 0 };
    let version = r.u8()?;
    ensure!(version == WIRE_VERSION || version == WIRE_VERSION_3, "unsupported display list version {version}");
    let count = r.u32()? as usize;
    // Every item takes at least one byte, so a larger count cannot be valid.
    ensure!(count <= r.remaining(), "item count {count} exceeds payload");
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let tag = r.u8()?;
        let item = match tag {
            TAG_FILL => DisplayItem::Fill {
                node: NodeId(r.u32()?),
                rect: r.rect()?,
                radius: r.f32()?,
                fg: r.color()?,
                bg: r.color()?,
            },
            TAG_BORDER => {
                let node = NodeId(r.u32()?);
                let rect = r.rect()?;
                let radius = r.f32()?;
                let chars = BorderChars {
                    top_left: r.ch()?,
                    top: r.ch()?,
                    top_right: r.ch()?,
                    left: r.ch()?,
                    right: r.ch()?,
                    bot_left: r.ch()?,
                    bot: r.ch()?,
                    bot_right: r.ch()?,
                };
                DisplayItem::Border { node, rect, radius, chars, fg: r.color()?, bg: r.color()? }
            }
            TAG_TEXT => {
                let node = NodeId(r.u32()?);
                let origin = PointF { x: r.f32()?, y: r.f32()? };
                let clip = r.rect()?;
                let fg = r.color()?;
                let bg = r.color()?;
                let (mods, _) = parse_mods(r.u8()?);
                let n = r.u32()? as usize;
                ensure!(n <= r.remaining() / GLYPH_BYTES, "glyph count {n} exceeds payload");
                let mut glyphs = Vec::with_capacity(n);
                for _ in 0..n {
                    glyphs.push(Glyph { x: r.f32()?, y: r.f32()?, ch: r.ch()?, advance: r.f32()? });
                }
                DisplayItem::Text { node, origin, clip, glyphs, fg, bg, mods, shaped: None }
            }
            TAG_TEXT_RUN => {
                ensure!(version >= WIRE_VERSION_3, "shaped text run in a version {version} list");
                let node = NodeId(r.u32()?);
                let origin = PointF { x: r.f32()?, y: r.f32()? };
                let clip = r.rect()?;
                let fg = r.color()?;
                let bg = r.color()?;
                let (mods, _) = parse_mods(r.u8()?);
                let (face, size_px, line_height, baseline) = (r.u8()?, r.f32()?, r.f32()?, r.f32()?);
                let n = r.u32()? as usize;
                ensure!(n <= r.remaining() / GLYPH_BYTES, "glyph count {n} exceeds payload");
                let mut glyphs = Vec::with_capacity(n);
                for _ in 0..n {
                    glyphs.push(Glyph { x: r.f32()?, y: r.f32()?, ch: r.ch()?, advance: r.f32()? });
                }
                // Both tables are sorted by glyph index (strictly ascending, in range).
                let nf = r.u32()? as usize;
                ensure!(nf <= r.remaining() / 5, "face override count {nf} exceeds payload");
                let mut glyph_faces = Vec::with_capacity(nf);
                let mut prev: Option<u32> = None;
                for _ in 0..nf {
                    let (i, f) = (r.u32()?, r.u8()?);
                    ensure!((i as usize) < n && prev.map_or(true, |p| i > p), "bad face override index {i}");
                    prev = Some(i);
                    glyph_faces.push((i, f));
                }
                let nc = r.u32()? as usize;
                ensure!(nc <= r.remaining() / 8, "cluster count {nc} exceeds payload");
                let mut clusters = Vec::with_capacity(nc);
                let mut prev: Option<u32> = None;
                for _ in 0..nc {
                    let i = r.u32()?;
                    let len = r.u32()? as usize;
                    ensure!((i as usize) < n && prev.map_or(true, |p| i > p), "bad cluster index {i}");
                    prev = Some(i);
                    let text = std::str::from_utf8(r.take(len)?).map_err(|_| anyhow!("cluster text is not UTF-8"))?;
                    clusters.push((i, Box::<str>::from(text)));
                }
                let run = ShapedRun { face, size_px, line_height, baseline, glyph_faces, clusters };
                DisplayItem::Text { node, origin, clip, glyphs, fg, bg, mods, shaped: Some(Box::new(run)) }
            }
            TAG_LEGACY => DisplayItem::Legacy { node: NodeId(r.u32()?), content: r.rect()? },
            TAG_PUSH_CLIP => DisplayItem::PushClip { rect: r.rect()? },
            TAG_POP_CLIP => DisplayItem::PopClip,
            TAG_CELL_GRID => {
                let rect = r.rect()?;
                let n = r.u32()? as usize;
                ensure!(n <= r.remaining() / CELL_MIN_BYTES, "cell count {n} exceeds payload");
                let mut cells = Vec::with_capacity(n);
                for _ in 0..n {
                    let ch = r.ch()?;
                    let fg = r.color()?;
                    let bg = r.color()?;
                    let (mods, skip) = parse_mods(r.u8()?);
                    cells.push(GridCell { ch, fg, bg, mods, skip });
                }
                DisplayItem::CellGrid { rect, cells }
            }
            TAG_PATH => {
                let node = NodeId(r.u32()?);
                let n = r.u32()? as usize;
                // Smallest command (Close) is one byte.
                ensure!(n <= r.remaining(), "path command count {n} exceeds payload");
                let mut cmds = Vec::with_capacity(n);
                for _ in 0..n {
                    cmds.push(match r.u8()? {
                        OP_MOVE => PathCmd::MoveTo(r.pt()?),
                        OP_LINE => PathCmd::LineTo(r.pt()?),
                        OP_QUAD => PathCmd::QuadTo(r.pt()?, r.pt()?),
                        OP_CUBIC => PathCmd::CubicTo(r.pt()?, r.pt()?, r.pt()?),
                        OP_CLOSE => PathCmd::Close,
                        op => bail!("unknown path op {op}"),
                    });
                }
                let fill = match r.u8()? {
                    0 => None,
                    1 => Some(r.color()?),
                    f => bail!("invalid path fill flag {f}"),
                };
                let stroke = match r.u8()? {
                    0 => None,
                    1 => Some((r.color()?, r.f32()?)),
                    f => bail!("invalid path stroke flag {f}"),
                };
                DisplayItem::Path { node, cmds, fill, stroke }
            }
            other => bail!("unknown display item tag {other}"),
        };
        items.push(item);
    }
    ensure!(r.remaining() == 0, "{} trailing bytes after display list", r.remaining());
    Ok(DisplayList { items })
}

fn mods_byte(m: Modifiers, skip: bool) -> u8 {
    (if m.bold { MOD_BOLD } else { 0 })
        | (if m.underline { MOD_UNDERLINE } else { 0 })
        | (if m.italic { MOD_ITALIC } else { 0 })
        | (if skip { MOD_SKIP } else { 0 })
}

fn parse_mods(b: u8) -> (Modifiers, bool) {
    (
        Modifiers { bold: b & MOD_BOLD != 0, underline: b & MOD_UNDERLINE != 0, italic: b & MOD_ITALIC != 0 },
        b & MOD_SKIP != 0,
    )
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_pt(out: &mut Vec<u8>, p: PointF) {
    put_f32(out, p.x);
    put_f32(out, p.y);
}

fn put_rect(out: &mut Vec<u8>, r: RectF) {
    put_f32(out, r.x);
    put_f32(out, r.y);
    put_f32(out, r.w);
    put_f32(out, r.h);
}

fn put_color(out: &mut Vec<u8>, c: AnsiColor) {
    match c {
        AnsiColor::Reset => out.push(0),
        AnsiColor::TrueColor(r, g, b) => out.extend_from_slice(&[1, r, g, b]),
        AnsiColor::Color256(i) => out.extend_from_slice(&[2, i]),
    }
}

/// Bounds-checked little-endian cursor.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(anyhow!("truncated display list"));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Reads an `f32`, rejecting NaN and infinities.
    fn f32(&mut self) -> Result<f32> {
        let v = f32::from_bits(self.u32()?);
        ensure!(v.is_finite(), "non-finite f32 in display list");
        Ok(v)
    }

    fn pt(&mut self) -> Result<PointF> {
        Ok(PointF { x: self.f32()?, y: self.f32()? })
    }

    fn rect(&mut self) -> Result<RectF> {
        Ok(RectF { x: self.f32()?, y: self.f32()?, w: self.f32()?, h: self.f32()? })
    }

    fn ch(&mut self) -> Result<char> {
        let v = self.u32()?;
        char::from_u32(v).ok_or_else(|| anyhow!("invalid code point {v:#x}"))
    }

    fn color(&mut self) -> Result<AnsiColor> {
        match self.u8()? {
            0 => Ok(AnsiColor::Reset),
            1 => {
                let b = self.take(3)?;
                Ok(AnsiColor::TrueColor(b[0], b[1], b[2]))
            }
            2 => Ok(AnsiColor::Color256(self.u8()?)),
            t => bail!("unknown colour tag {t}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DisplayList {
        let mut l = DisplayList::new();
        l.push(DisplayItem::Fill {
            node: NodeId(0),
            rect: RectF::new(0.0, 0.0, 80.0, 24.0),
            radius: 0.0,
            fg: AnsiColor::Color256(15),
            bg: AnsiColor::TrueColor(1, 2, 3),
        });
        l.push(DisplayItem::Border {
            node: NodeId(7),
            rect: RectF::new(1.5, -2.0, 10.0, 3.25),
            radius: 0.75,
            chars: BorderChars::rounded(),
            fg: AnsiColor::Reset,
            bg: AnsiColor::Color256(0),
        });
        l.push(DisplayItem::Text {
            node: NodeId(9),
            origin: PointF { x: 2.0, y: 3.0 },
            clip: RectF::new(2.0, 3.0, 8.0, 1.0),
            glyphs: vec![
                Glyph { x: 0.0, y: 0.0, ch: 'h', advance: 1.0 },
                Glyph { x: 1.0, y: 0.0, ch: '漢', advance: 2.0 },
                Glyph { x: 3.0, y: 0.0, ch: '😀', advance: 2.0 },
            ],
            fg: AnsiColor::TrueColor(255, 0, 128),
            bg: AnsiColor::Reset,
            mods: Modifiers { bold: true, underline: false, italic: true },
            shaped: None,
        });
        l.push(DisplayItem::Legacy { node: NodeId(11), content: RectF::new(4.0, 5.0, 6.0, 7.0) });
        l.push(DisplayItem::PushClip { rect: RectF::new(0.0, 0.0, 5.0, 5.0) });
        l.push(DisplayItem::PopClip);
        let p = |x: f32, y: f32| PointF { x, y };
        let all_cmds = vec![
            PathCmd::MoveTo(p(0.5, 1.5)),
            PathCmd::LineTo(p(2.0, -3.0)),
            PathCmd::QuadTo(p(1.0, 1.0), p(2.0, 2.0)),
            PathCmd::CubicTo(p(3.0, 3.0), p(4.0, 4.0), p(5.5, 5.5)),
            PathCmd::Close,
        ];
        let st = Some((AnsiColor::TrueColor(1, 2, 3), 0.25));
        let fl = Some(AnsiColor::Color256(7));
        for (fill, stroke) in [(None, None), (fl, None), (None, st), (fl, st)] {
            l.push(DisplayItem::Path { node: NodeId(21), cmds: all_cmds.clone(), fill, stroke });
        }
        l.push(DisplayItem::Path { node: NodeId(22), cmds: vec![], fill: None, stroke: None });
        l.push(DisplayItem::CellGrid {
            rect: RectF::new(3.0, 4.0, 2.0, 2.0),
            cells: vec![
                GridCell { ch: 'a', fg: AnsiColor::Reset, bg: AnsiColor::Reset, mods: Modifiers::default(), skip: false },
                GridCell { ch: '漢', fg: AnsiColor::Color256(9), bg: AnsiColor::TrueColor(9, 8, 7), mods: Modifiers { bold: true, underline: true, italic: true }, skip: false },
                GridCell { ch: ' ', fg: AnsiColor::Color256(9), bg: AnsiColor::TrueColor(9, 8, 7), mods: Modifiers::default(), skip: true },
                GridCell { ch: '\u{10FFFF}', fg: AnsiColor::Reset, bg: AnsiColor::Reset, mods: Modifiers::default(), skip: false },
            ],
        });
        l
    }

    #[test]
    fn empty_list_round_trips() {
        let l = DisplayList::new();
        let bytes = encode(&l);
        assert_eq!(bytes, vec![2, 0, 0, 0, 0]);
        assert_eq!(decode(&bytes).unwrap(), l);
    }

    #[test]
    fn every_item_type_round_trips() {
        let l = sample();
        assert_eq!(decode(&encode(&l)).unwrap(), l);
    }

    #[test]
    fn each_item_round_trips_alone() {
        for item in sample().items {
            let l = DisplayList { items: vec![item] };
            assert_eq!(decode(&encode(&l)).unwrap(), l);
        }
    }

    #[test]
    fn colours_use_diff_engine_encoding() {
        let mut out = Vec::new();
        put_color(&mut out, AnsiColor::Reset);
        put_color(&mut out, AnsiColor::TrueColor(1, 2, 3));
        put_color(&mut out, AnsiColor::Color256(200));
        assert_eq!(out, vec![0, 1, 1, 2, 3, 2, 200]);
    }

    #[test]
    fn every_strict_prefix_is_rejected_without_panic() {
        let bytes = encode(&sample());
        for n in 0..bytes.len() {
            assert!(decode(&bytes[..n]).is_err(), "prefix of {n} bytes must not decode");
        }
    }

    #[test]
    fn unknown_version_is_rejected() {
        let mut bytes = encode(&sample());
        bytes[0] = 1;
        assert!(decode(&bytes).is_err());
        bytes[0] = 4;
        assert!(decode(&bytes).is_err());
        bytes[0] = 0;
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn trailing_bytes_and_bad_values_are_rejected() {
        let mut bytes = encode(&sample());
        bytes.push(0);
        assert!(decode(&bytes).is_err());
        // unknown item tag
        assert!(decode(&[2, 1, 0, 0, 0, 99]).is_err());
        // invalid code point in a one-cell grid (surrogate)
        let mut g = vec![2, 1, 0, 0, 0, TAG_CELL_GRID];
        g.extend_from_slice(&[0; 16]);
        g.extend_from_slice(&1u32.to_le_bytes());
        g.extend_from_slice(&0xD800u32.to_le_bytes());
        g.extend_from_slice(&[0, 0, 0]);
        assert!(decode(&g).is_err());
        // unknown colour tag
        let mut f = vec![2, 1, 0, 0, 0, TAG_FILL];
        f.extend_from_slice(&[0; 24]);
        f.extend_from_slice(&[9, 0]);
        assert!(decode(&f).is_err());
    }

    #[test]
    fn huge_counts_do_not_allocate_or_panic() {
        assert!(decode(&[2, 0xff, 0xff, 0xff, 0xff]).is_err());
        let mut g = vec![2, 1, 0, 0, 0, TAG_CELL_GRID];
        g.extend_from_slice(&[0; 16]);
        g.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&g).is_err());
        let mut t = vec![2, 1, 0, 0, 0, TAG_TEXT];
        t.extend_from_slice(&[0; 4 + 8 + 16]);
        t.extend_from_slice(&[0, 0, 0]);
        t.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&t).is_err());
    }

    fn path_header(n: u32) -> Vec<u8> {
        let mut v = vec![2, 1, 0, 0, 0, TAG_PATH, 0, 0, 0, 0];
        v.extend_from_slice(&n.to_le_bytes());
        v
    }

    #[test]
    fn pinned_v2_bytes() {
        let mut l = DisplayList::new();
        l.push(DisplayItem::Fill {
            node: NodeId(1),
            rect: RectF::new(1.0, 2.0, 3.0, 4.0),
            radius: 0.5,
            fg: AnsiColor::Reset,
            bg: AnsiColor::Color256(9),
        });
        let mut want = vec![2, 1, 0, 0, 0, 1, 1, 0, 0, 0];
        for v in [1.0f32, 2.0, 3.0, 4.0, 0.5] {
            want.extend_from_slice(&v.to_le_bytes());
        }
        want.extend_from_slice(&[0, 2, 9]);
        assert_eq!(encode(&l), want);

        let mut l = DisplayList::new();
        l.push(DisplayItem::Path {
            node: NodeId(3),
            cmds: vec![PathCmd::MoveTo(PointF { x: 1.0, y: 2.0 }), PathCmd::Close],
            fill: Some(AnsiColor::Color256(5)),
            stroke: Some((AnsiColor::Reset, 0.5)),
        });
        let mut want = vec![2, 1, 0, 0, 0, 8, 3, 0, 0, 0, 2, 0, 0, 0, 0];
        want.extend_from_slice(&1.0f32.to_le_bytes());
        want.extend_from_slice(&2.0f32.to_le_bytes());
        want.push(4);
        want.extend_from_slice(&[1, 2, 5, 1, 0]);
        want.extend_from_slice(&0.5f32.to_le_bytes());
        assert_eq!(encode(&l), want);
    }

    #[test]
    fn malformed_paths_are_rejected() {
        // unknown op
        let mut b = path_header(1);
        b.push(9);
        assert!(decode(&b).is_err());
        // count larger than payload
        assert!(decode(&path_header(u32::MAX)).is_err());
        // bad flags
        let mut b = path_header(0);
        b.extend_from_slice(&[2, 0]);
        assert!(decode(&b).is_err());
        let mut b = path_header(0);
        b.extend_from_slice(&[0, 7]);
        assert!(decode(&b).is_err());
        // NaN / infinity operands
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut b = path_header(1);
            b.push(OP_MOVE);
            b.extend_from_slice(&bad.to_le_bytes());
            b.extend_from_slice(&0.0f32.to_le_bytes());
            b.extend_from_slice(&[0, 0]);
            assert!(decode(&b).is_err());
        }
        // NaN radius on a Fill
        let mut f = vec![2, 1, 0, 0, 0, TAG_FILL];
        f.extend_from_slice(&[0; 20]);
        f.extend_from_slice(&f32::NAN.to_le_bytes());
        f.extend_from_slice(&[0, 0]);
        assert!(decode(&f).is_err());
    }


    fn shaped_sample() -> DisplayList {
        let mut l = sample();
        l.push(DisplayItem::Text {
            node: NodeId(30),
            origin: PointF { x: 1.5, y: 2.0 },
            clip: RectF::new(1.5, 2.0, 20.25, 3.0),
            glyphs: vec![
                Glyph { x: 0.0, y: 0.0, ch: 'o', advance: 1.5124999 },
                Glyph { x: 1.5124999, y: 0.0, ch: 'f', advance: 1.25 },
                Glyph { x: 2.7625, y: 0.0, ch: '\u{2500}', advance: 1.0 },
                Glyph { x: 0.0, y: 1.5, ch: 'e', advance: 1.36 },
            ],
            fg: AnsiColor::TrueColor(1, 2, 3),
            bg: AnsiColor::Color256(4),
            mods: Modifiers { bold: true, underline: true, italic: false },
            shaped: Some(Box::new(ShapedRun {
                face: 1,
                size_px: 24.0,
                line_height: 1.5,
                baseline: 1.2156,
                glyph_faces: vec![(2, 0)],
                clusters: vec![(1, "ffi".into()), (3, "e\u{301}".into())],
            })),
        });
        // a plain run next to it stays a tag-3 item
        l.push(DisplayItem::Text {
            node: NodeId(31),
            origin: PointF { x: 0.0, y: 0.0 },
            clip: RectF::new(0.0, 0.0, 4.0, 1.0),
            glyphs: vec![Glyph { x: 0.0, y: 0.0, ch: 'x', advance: 1.0 }],
            fg: AnsiColor::Reset,
            bg: AnsiColor::Reset,
            mods: Modifiers::default(),
            shaped: None,
        });
        l
    }

    #[test]
    fn v3_round_trips_shaped_runs() {
        let l = shaped_sample();
        let bytes = encode_for(&l, 3);
        assert_eq!(bytes[0], 3);
        assert_eq!(decode(&bytes).unwrap(), l);
        for item in l.items {
            let one = DisplayList { items: vec![item] };
            assert_eq!(decode(&encode_for(&one, 3)).unwrap(), one);
        }
    }

    #[test]
    fn pinned_v3_run_bytes() {
        let l = DisplayList {
            items: vec![DisplayItem::Text {
                node: NodeId(5),
                origin: PointF { x: 1.0, y: 2.0 },
                clip: RectF::new(0.0, 0.0, 3.0, 4.0),
                glyphs: vec![Glyph { x: 0.5, y: 0.0, ch: 'f', advance: 2.0 }],
                fg: AnsiColor::Reset,
                bg: AnsiColor::Color256(9),
                mods: Modifiers { bold: true, underline: false, italic: false },
                shaped: Some(Box::new(ShapedRun {
                    face: 1,
                    size_px: 24.0,
                    line_height: 1.5,
                    baseline: 1.0,
                    glyph_faces: vec![(0, 0)],
                    clusters: vec![(0, "fi".into())],
                })),
            }],
        };
        let mut want = vec![3, 1, 0, 0, 0, 9, 5, 0, 0, 0];
        for v in [1.0f32, 2.0, 0.0, 0.0, 3.0, 4.0] {
            want.extend_from_slice(&v.to_le_bytes());
        }
        want.extend_from_slice(&[0, 2, 9, 1]); // fg, bg, mods(bold)
        want.push(1); // face
        for v in [24.0f32, 1.5, 1.0] {
            want.extend_from_slice(&v.to_le_bytes());
        }
        want.extend_from_slice(&1u32.to_le_bytes());
        want.extend_from_slice(&0.5f32.to_le_bytes());
        want.extend_from_slice(&0.0f32.to_le_bytes());
        want.extend_from_slice(&('f' as u32).to_le_bytes());
        want.extend_from_slice(&2.0f32.to_le_bytes());
        want.extend_from_slice(&1u32.to_le_bytes()); // face overrides
        want.extend_from_slice(&0u32.to_le_bytes());
        want.push(0);
        want.extend_from_slice(&1u32.to_le_bytes()); // clusters
        want.extend_from_slice(&0u32.to_le_bytes());
        want.extend_from_slice(&2u32.to_le_bytes());
        want.extend_from_slice(b"fi");
        assert_eq!(encode_for(&l, 3), want);
    }

    #[test]
    fn v2_client_gets_the_deterministic_downgrade() {
        let l = shaped_sample();
        let v2 = encode_for(&l, 2);
        assert_eq!(v2, encode(&l));
        assert_eq!(v2[0], 2);
        let got = decode(&v2).unwrap();
        assert_eq!(got, downgrade_v2(&l));
        let DisplayItem::Text { glyphs, shaped, .. } = &got.items[got.items.len() - 2] else { panic!() };
        assert!(shaped.is_none());
        // 4 glyphs: "ffi" cluster at glyph 1 -> 3 chars, "e\u{301}" at glyph 3 -> 2 chars
        let chars: String = glyphs.iter().map(|g| g.ch).collect();
        assert_eq!(chars, "offi\u{2500}e\u{301}");
        // all chars of a cluster share its x/y; only the first keeps the advance
        assert_eq!((glyphs[1].x, glyphs[2].x, glyphs[3].x), (1.5124999, 1.5124999, 1.5124999));
        assert_eq!((glyphs[1].advance, glyphs[2].advance, glyphs[3].advance), (1.25, 0.0, 0.0));
        assert_eq!((glyphs[5].y, glyphs[6].y), (1.5, 1.5));
        // a v2 decoder (this one, told it is v2) refuses a run tag
        let mut bad = encode_for(&l, 3);
        bad[0] = 2;
        assert!(decode(&bad).is_err());
    }

    #[test]
    fn lists_without_shaped_runs_are_identical_for_every_client() {
        let l = sample();
        assert_eq!(encode_for(&l, 2), encode(&l));
        assert_eq!(encode_for(&l, 3), encode(&l));
        assert_eq!(encode_for(&l, 255), encode(&l));
        assert_eq!(encode_for(&DisplayList::new(), 3), vec![2, 0, 0, 0, 0]);
    }

    #[test]
    fn every_strict_prefix_of_a_v3_list_is_rejected() {
        let bytes = encode_for(&shaped_sample(), 3);
        for n in 0..bytes.len() {
            assert!(decode(&bytes[..n]).is_err(), "prefix of {n} bytes must not decode");
        }
    }

    #[test]
    fn malformed_shaped_runs_are_rejected() {
        let one = |run: ShapedRun| {
            encode_for(
                &DisplayList {
                    items: vec![DisplayItem::Text {
                        node: NodeId(1),
                        origin: PointF::default(),
                        clip: RectF::default(),
                        glyphs: vec![Glyph { x: 0.0, y: 0.0, ch: 'a', advance: 1.0 }; 2],
                        fg: AnsiColor::Reset,
                        bg: AnsiColor::Reset,
                        mods: Modifiers::default(),
                        shaped: Some(Box::new(run)),
                    }],
                },
                3,
            )
        };
        let base = ShapedRun { face: 1, size_px: 16.0, line_height: 1.0, baseline: 0.8, glyph_faces: vec![], clusters: vec![] };
        assert!(decode(&one(base.clone())).is_ok());
        // index out of range, not ascending, duplicated
        for faces in [vec![(2, 0)], vec![(1, 0), (0, 0)], vec![(1, 0), (1, 0)]] {
            assert!(decode(&one(ShapedRun { glyph_faces: faces, ..base.clone() })).is_err());
        }
        for cl in [vec![(2, "ab".into())], vec![(1, "ab".into()), (0, "cd".into())]] {
            assert!(decode(&one(ShapedRun { clusters: cl, ..base.clone() })).is_err());
        }
        // non-finite size, invalid UTF-8 in a cluster, huge counts
        assert!(decode(&one(ShapedRun { size_px: f32::NAN, ..base.clone() })).is_err());
        let mut b = one(ShapedRun { clusters: vec![(0, "ab".into())], ..base.clone() });
        let n = b.len();
        b[n - 2] = 0xff;
        b[n - 1] = 0xfe;
        assert!(decode(&b).is_err());
        let mut b = one(base);
        let n = b.len();
        b[n - 8..n - 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&b).is_err());
    }

    #[test]
    fn garbage_never_panics() {
        let mut seed = 0x1234_5678u32;
        for len in 0..200 {
            let mut buf = vec![2u8];
            for _ in 0..len {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                buf.push((seed >> 24) as u8 % 9);
            }
            let _ = decode(&buf);
            buf[0] = 3;
            let _ = decode(&buf);
        }
    }
}
