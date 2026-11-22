//! Cell sink: rasterizes a [`DisplayList`] into a [`CellBuffer`].
//!
//! Semantics are defined by the module documentation of [`crate::render::display_list`].
//! All coordinates are integer-valued in phase 1, so `f32 -> i32` conversion with `as` is
//! exact. Every write is clamped to the buffer bounds.

use crate::render::buffer::{Cell, CellBuffer};
use crate::render::display_list::{DisplayItem, DisplayList, Glyph, GridCell, Modifiers, RectF};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{AnsiColor, BorderChars};

/// Rasterizer painting display lists into character cells.
pub struct CellSink;

impl CellSink {
    /// Paints `list` into `buffer` in list order. `legacy` is invoked for every
    /// [`DisplayItem::Legacy`] item at its position in the list.
    pub fn paint(
        list: &DisplayList,
        buffer: &mut CellBuffer,
        mut legacy: impl FnMut(NodeId, RectF, &mut CellBuffer),
    ) {
        for item in &list.items {
            match item {
                DisplayItem::Fill { rect, fg, bg, .. } => Self::fill(buffer, *rect, *fg, *bg),
                DisplayItem::Border { rect, chars, fg, bg, .. } => {
                    Self::border(buffer, *rect, chars, *fg, *bg)
                }
                DisplayItem::Text { origin, clip, glyphs, fg, bg, mods, .. } => {
                    Self::text(buffer, origin.x as i32, origin.y as i32, *clip, glyphs, *fg, *bg, *mods)
                }
                DisplayItem::Legacy { node, content } => legacy(*node, *content, buffer),
                DisplayItem::CellGrid { rect, cells } => Self::grid(buffer, *rect, cells),
                // Vector paths are GUI-only; a terminal cannot draw them.
                DisplayItem::Path { .. } | DisplayItem::PushClip { .. } | DisplayItem::PopClip => {}
            }
        }
    }

    /// Returns one `CellGrid` per horizontal run of cells that differ between `base` and
    /// `fin` (compared in full, including `skip`). Painting the result over a buffer equal to
    /// `base` yields `fin`. Both buffers must have the same dimensions, otherwise the result
    /// is empty.
    pub fn diff_grids(base: &CellBuffer, fin: &CellBuffer) -> Vec<DisplayItem> {
        let mut out = Vec::new();
        if base.width != fin.width || base.height != fin.height {
            return out;
        }
        let w = fin.width as usize;
        for y in 0..fin.height as usize {
            let mut x = 0;
            while x < w {
                if base.cells[y * w + x] == fin.cells[y * w + x] {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < w && base.cells[y * w + x] != fin.cells[y * w + x] {
                    x += 1;
                }
                out.push(DisplayItem::CellGrid {
                    rect: RectF::new(start as f32, y as f32, (x - start) as f32, 1.0),
                    cells: fin.cells[y * w + start..y * w + x].iter().map(|c| GridCell::from(*c)).collect(),
                });
            }
        }
        out
    }

    #[inline]
    fn set(buffer: &mut CellBuffer, x: i32, y: i32, cell: Cell) {
        if x >= 0 && x < buffer.width as i32 && y >= 0 && y < buffer.height as i32 {
            buffer.set(x as u16, y as u16, cell);
        }
    }

    fn grid(buffer: &mut CellBuffer, rect: RectF, cells: &[GridCell]) {
        let (x0, y0, w, h) = (rect.x as i32, rect.y as i32, rect.w as i32, rect.h as i32);
        if w <= 0 || h <= 0 {
            return;
        }
        for y in 0..h {
            for x in 0..w {
                let Some(c) = cells.get((y as usize) * (w as usize) + x as usize) else {
                    return;
                };
                Self::set(
                    buffer,
                    x0 + x,
                    y0 + y,
                    Cell {
                        ch: c.ch,
                        fg: c.fg,
                        bg: c.bg,
                        bold: c.mods.bold,
                        underline: c.mods.underline,
                        italic: c.mods.italic,
                        skip: c.skip,
                    },
                );
            }
        }
    }

    fn fill(buffer: &mut CellBuffer, rect: RectF, fg: AnsiColor, bg: AnsiColor) {
        let (x0, y0, w, h) = (rect.x as i32, rect.y as i32, rect.w as i32, rect.h as i32);
        for y in 0..h {
            for x in 0..w {
                Self::set(buffer, x0 + x, y0 + y, Cell { ch: ' ', fg, bg, ..Default::default() });
            }
        }
    }

    fn border(buffer: &mut CellBuffer, rect: RectF, c: &BorderChars, fg: AnsiColor, bg: AnsiColor) {
        let (x0, y0, w, h) = (rect.x as i32, rect.y as i32, rect.w as i32, rect.h as i32);
        if w <= 0 || h <= 0 {
            return;
        }
        let mut put = |x: i32, y: i32, ch: char| {
            Self::set(buffer, x, y, Cell { ch, fg, bg, ..Default::default() });
        };

        // Corners
        put(x0, y0, c.top_left);
        if w > 1 {
            put(x0 + w - 1, y0, c.top_right);
        }
        if h > 1 {
            put(x0, y0 + h - 1, c.bot_left);
        }
        if w > 1 && h > 1 {
            put(x0 + w - 1, y0 + h - 1, c.bot_right);
        }

        // Horizontal edges
        for x in 1..(w - 1).max(0) {
            put(x0 + x, y0, c.top);
            if h > 1 {
                put(x0 + x, y0 + h - 1, c.bot);
            }
        }

        // Vertical edges
        for y in 1..(h - 1).max(0) {
            put(x0, y0 + y, c.left);
            if w > 1 {
                put(x0 + w - 1, y0 + y, c.right);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        buffer: &mut CellBuffer,
        ox: i32,
        oy: i32,
        clip: RectF,
        glyphs: &[Glyph],
        fg: AnsiColor,
        bg: AnsiColor,
        mods: Modifiers,
    ) {
        let clip_right = (clip.x + clip.w) as i32;
        let style = |ch: char| Cell {
            ch,
            fg,
            bg,
            bold: mods.bold,
            underline: mods.underline,
            italic: mods.italic,
            skip: false,
        };
        for g in glyphs {
            let x = ox + g.x as i32;
            let y = oy + g.y as i32;
            Self::set(buffer, x, y, style(g.ch));
            // Continuation cells of wide glyphs: styled spaces, clipped on the right edge.
            for i in 1..(g.advance as i32) {
                if x + i < clip_right {
                    Self::set(buffer, x + i, y, style(' '));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::display_list::PointF;

    #[test]
    fn diff_grids_reproduces_final_buffer() {
        let base = CellBuffer::new(10, 3);
        let mut fin = CellBuffer::new(10, 3);
        for (x, y, ch, skip) in [(1u16, 0u16, 'a', false), (2, 0, 'b', false), (5, 0, 'c', true), (9, 2, 'z', false)] {
            fin.set(x, y, Cell { ch, bold: true, skip, ..Default::default() });
        }
        let grids = CellSink::diff_grids(&base, &fin);
        assert_eq!(grids.len(), 3);
        let mut out = CellBuffer::new(10, 3);
        CellSink::paint(&DisplayList { items: grids }, &mut out, |_, _, _| {});
        assert_eq!(out.cells, fin.cells);
        assert!(CellSink::diff_grids(&fin, &fin).is_empty());
    }

    #[test]
    fn cell_grid_is_clamped_and_tolerates_bad_sizes() {
        let c = GridCell { ch: 'x', fg: AnsiColor::Reset, bg: AnsiColor::Reset, mods: Modifiers::default(), skip: false };
        let mut buf = CellBuffer::new(4, 2);
        let mut l = DisplayList::new();
        l.push(DisplayItem::CellGrid { rect: RectF::new(-1.0, -1.0, 3.0, 3.0), cells: vec![c; 9] });
        l.push(DisplayItem::CellGrid { rect: RectF::new(3.0, 1.0, 5.0, 5.0), cells: vec![c; 2] });
        l.push(DisplayItem::CellGrid { rect: RectF::new(0.0, 0.0, 0.0, 3.0), cells: vec![] });
        CellSink::paint(&l, &mut buf, |_, _, _| {});
        assert_eq!(buf.cells[0].ch, 'x');
        assert_eq!(buf.cells[3 + 4].ch, 'x');
    }

    fn nid() -> NodeId {
        NodeId(0)
    }

    fn chars() -> BorderChars {
        BorderChars::single()
    }

    fn border_list(rect: RectF) -> DisplayList {
        let mut l = DisplayList::new();
        l.push(DisplayItem::Border {
            node: nid(),
            rect,
            radius: 0.0,
            chars: chars(),
            fg: AnsiColor::Color256(1),
            bg: AnsiColor::Color256(2),
        });
        l
    }

    fn row(b: &CellBuffer, y: u16) -> String {
        (0..b.width).map(|x| b.cells[y as usize * b.width as usize + x as usize].ch).collect()
    }

    #[test]
    fn border_1x1_draws_only_top_left() {
        let mut b = CellBuffer::new(3, 3);
        CellSink::paint(&border_list(RectF::new(1.0, 1.0, 1.0, 1.0)), &mut b, |_, _, _| {});
        assert_eq!(row(&b, 0), "   ");
        assert_eq!(row(&b, 1), " ┌ ");
        assert_eq!(row(&b, 2), "   ");
    }

    #[test]
    fn border_2x1_draws_both_top_corners() {
        let mut b = CellBuffer::new(4, 2);
        CellSink::paint(&border_list(RectF::new(1.0, 0.0, 2.0, 1.0)), &mut b, |_, _, _| {});
        assert_eq!(row(&b, 0), " ┌┐ ");
        assert_eq!(row(&b, 1), "    ");
    }

    #[test]
    fn border_1x3_draws_left_corners_and_edge() {
        let mut b = CellBuffer::new(2, 4);
        CellSink::paint(&border_list(RectF::new(0.0, 0.0, 1.0, 3.0)), &mut b, |_, _, _| {});
        assert_eq!(row(&b, 0), "┌ ");
        assert_eq!(row(&b, 1), "│ ");
        assert_eq!(row(&b, 2), "└ ");
        assert_eq!(row(&b, 3), "  ");
    }

    #[test]
    fn border_is_clamped_to_buffer() {
        let mut b = CellBuffer::new(2, 2);
        CellSink::paint(&border_list(RectF::new(-1.0, -1.0, 5.0, 5.0)), &mut b, |_, _, _| {});
        assert_eq!(row(&b, 0), "  ");
    }

    #[test]
    fn wide_glyph_continuation_is_clipped() {
        let mut b = CellBuffer::new(6, 1);
        let mut l = DisplayList::new();
        l.push(DisplayItem::Text {
            node: nid(),
            origin: PointF { x: 1.0, y: 0.0 },
            clip: RectF::new(1.0, 0.0, 3.0, 1.0),
            glyphs: vec![
                Glyph { x: 0.0, y: 0.0, ch: 'a', advance: 1.0 },
                // Origin cell at x=3 is the last clip cell; continuation at x=4 is outside.
                Glyph { x: 2.0, y: 0.0, ch: '世', advance: 2.0 },
            ],
            fg: AnsiColor::Color256(3),
            bg: AnsiColor::Color256(4),
            mods: Modifiers::default(),
        });
        // Pre-mark the cell right of the clip to prove it is not touched.
        b.set(4, 0, Cell { ch: '#', ..Default::default() });
        CellSink::paint(&l, &mut b, |_, _, _| {});
        assert_eq!(row(&b, 0), " a 世# ");
        assert_eq!(b.cells[3].fg, AnsiColor::Color256(3));
    }

    #[test]
    fn wide_glyph_continuation_inside_clip_is_styled_space() {
        let mut b = CellBuffer::new(4, 1);
        let mut l = DisplayList::new();
        l.push(DisplayItem::Text {
            node: nid(),
            origin: PointF { x: 0.0, y: 0.0 },
            clip: RectF::new(0.0, 0.0, 4.0, 1.0),
            glyphs: vec![Glyph { x: 0.0, y: 0.0, ch: '世', advance: 2.0 }],
            fg: AnsiColor::Color256(3),
            bg: AnsiColor::Color256(4),
            mods: Modifiers::default(),
        });
        b.set(1, 0, Cell { ch: '#', ..Default::default() });
        CellSink::paint(&l, &mut b, |_, _, _| {});
        assert_eq!(row(&b, 0), "世   ");
        assert_eq!(b.cells[1].bg, AnsiColor::Color256(4));
    }

    #[test]
    fn legacy_invokes_callback_in_order() {
        let mut b = CellBuffer::new(2, 1);
        let mut l = DisplayList::new();
        l.push(DisplayItem::Fill {
            node: nid(),
            rect: RectF::new(0.0, 0.0, 2.0, 1.0),
            radius: 0.0,
            fg: AnsiColor::Reset,
            bg: AnsiColor::Reset,
        });
        l.push(DisplayItem::Legacy { node: nid(), content: RectF::new(0.0, 0.0, 1.0, 1.0) });
        let mut seen = None;
        CellSink::paint(&l, &mut b, |_, r, buf| {
            seen = Some(r);
            buf.set(0, 0, Cell { ch: 'X', ..Default::default() });
        });
        assert_eq!(seen, Some(RectF::new(0.0, 0.0, 1.0, 1.0)));
        assert_eq!(row(&b, 0), "X ");
    }
}
