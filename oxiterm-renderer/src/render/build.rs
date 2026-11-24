//! DOM + layout to [`DisplayList`] builder.
//!
//! Walks the document in historical paint order (`layout.paint_order` when present, otherwise
//! a pre-order walk from the root) and emits, per node, a [`DisplayItem::Fill`], an optional
//! [`DisplayItem::Border`], then a [`DisplayItem::Text`] for Text/Button/Input content or a
//! [`DisplayItem::Legacy`] for Img/Video/Diagram.
//!
//! `RenderMode::Tui` snaps every rect and padding to whole cells (the historical output,
//! guarded by the golden tests). `RenderMode::Gui` keeps the layout's fractional rects and
//! origins; text still wraps against `floor(inner width)` columns and glyph offsets stay in
//! whole cells relative to the (possibly fractional) origin.
//!
//! **Shaped text (Phase 6.4).** A Text/Button node in GUI mode that has a stored layout in
//! `LayoutResult::shaped` (its font is not the root default, see `LayoutEngine::shaped_text`) is
//! emitted from that layout: one `Text` item per node with `shaped: Some(..)`, never re-shaped or
//! re-wrapped. Glyph `x` = `px / 9.6`, glyph `y` = the top of its line box (`line.top px / 20`;
//! the baseline is `y + ShapedRun::baseline`), `advance` = `px / 9.6`. Only glyphs whose origin
//! lies inside the item's `clip` are kept (the same rule as the cell path, on the exact
//! fractional rect instead of whole columns). Inputs never have a shaped layout (they stay on the
//! cell path, so password masking is untouched).

use crate::document::THTMLDocument;
use crate::font::ResolvedFonts;
use crate::text::layout::TextLayout;
use crate::layout::types::{to_cells, LayoutResult};
use crate::render::display_list::{DisplayItem, DisplayList, Glyph, Modifiers, PointF, RectF};
use crate::render::unicode::UnicodeWidthCache;
use oxiterm_proto::dom::{Node, NodeId, NodeTag};
use oxiterm_proto::style::{AnsiColor, Position, RenderMode, WrapMode};
use std::collections::HashMap;

/// Resolved `(fg, bg)` colours per node after inheritance.
pub(crate) type ResolvedColors = HashMap<NodeId, (AnsiColor, AnsiColor)>;

/// Resolves inherited foreground/background colours for every node reachable from the root.
pub(crate) fn precompute_inherited_colors(doc: &THTMLDocument) -> ResolvedColors {
    let mut map = HashMap::new();
    resolve_colors_recursive(doc, doc.root, AnsiColor::Color256(15), AnsiColor::Color256(0), &mut map);
    map
}

fn resolve_colors_recursive(
    doc: &THTMLDocument,
    node_id: NodeId,
    inherited_fg: AnsiColor,
    inherited_bg: AnsiColor,
    map: &mut ResolvedColors,
) {
    if let Some(node) = doc.arena.get(node_id) {
        let fg = match node.style.fg {
            AnsiColor::Reset => inherited_fg,
            c => c,
        };
        let bg = match node.style.bg {
            AnsiColor::Reset => inherited_bg,
            c => c,
        };
        map.insert(node_id, (fg, bg));
        for &child_id in &node.children {
            resolve_colors_recursive(doc, child_id, fg, bg, map);
        }
    }
}

/// Builds the display list for one frame.
///
/// `(start_x, start_y)` is the origin for normally positioned nodes (centering and scroll
/// applied); `(offset_x, offset_y)` is the origin for `Position::Fixed` nodes.
pub(crate) fn build_display_list(
    doc: &THTMLDocument,
    layout: &LayoutResult,
    colors: &ResolvedColors,
    fonts: &ResolvedFonts,
    start: (i32, i32),
    offset: (u16, u16),
    mode: RenderMode,
) -> DisplayList {
    let mut list = DisplayList::new();
    if !layout.paint_order.is_empty() {
        for &node_id in &layout.paint_order {
            if let Some(&(fg, bg)) = colors.get(&node_id) {
                emit_node(&mut list, doc, layout, fonts, node_id, start, offset, mode, fg, bg);
            }
        }
    } else {
        walk(&mut list, doc, layout, colors, fonts, doc.root, start, offset, mode);
    }
    list
}

fn walk(
    list: &mut DisplayList,
    doc: &THTMLDocument,
    layout: &LayoutResult,
    colors: &ResolvedColors,
    fonts: &ResolvedFonts,
    node_id: NodeId,
    start: (i32, i32),
    offset: (u16, u16),
    mode: RenderMode,
) {
    let Some(node) = doc.arena.get(node_id) else { return };
    if let Some(&(fg, bg)) = colors.get(&node_id) {
        emit_node(list, doc, layout, fonts, node_id, start, offset, mode, fg, bg);
    }
    for &child_id in &node.children {
        walk(list, doc, layout, colors, fonts, child_id, start, offset, mode);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_node(
    list: &mut DisplayList,
    doc: &THTMLDocument,
    layout: &LayoutResult,
    fonts: &ResolvedFonts,
    node_id: NodeId,
    start: (i32, i32),
    offset: (u16, u16),
    mode: RenderMode,
    fg: AnsiColor,
    bg: AnsiColor,
) {
    let Some(node) = doc.arena.get(node_id) else { return };
    let rect = layout.nodes.get(&node_id).copied().unwrap_or_default();
    let fixed = node.style.position == Position::Fixed;
    let (origin_x, origin_y) = if fixed { (offset.0 as i32, offset.1 as i32) } else { start };
    // TUI: layout rects snap to whole cells with the shared rule. GUI: fractional as laid out.
    let (abs_x, abs_y, rw, rh) = match mode {
        RenderMode::Tui => (
            (origin_x + to_cells(rect.x) as i32) as f32,
            (origin_y + to_cells(rect.y) as i32) as f32,
            to_cells(rect.width) as f32,
            to_cells(rect.height) as f32,
        ),
        RenderMode::Gui => (
            origin_x as f32 + rect.x,
            origin_y as f32 + rect.y,
            rect.width.max(0.0),
            rect.height.max(0.0),
        ),
    };
    let radius = node.style.border_radius.unwrap_or(0.0);

    let border_box = RectF::new(abs_x, abs_y, rw, rh);
    list.push(DisplayItem::Fill { node: node_id, rect: border_box, radius, fg, bg });

    let has_border = node.style.border.is_some();
    if let Some(border) = &node.style.border {
        let border_fg = match border.fg {
            AnsiColor::Reset => fg,
            c => c,
        };
        list.push(DisplayItem::Border {
            node: node_id,
            rect: border_box,
            radius,
            chars: border.chars.clone(),
            fg: border_fg,
            bg,
        });
    }

    let (cx, cy) = if has_border { (abs_x + 1.0, abs_y + 1.0) } else { (abs_x, abs_y) };
    let (cw, ch) = if has_border { ((rw - 2.0).max(0.0), (rh - 2.0).max(0.0)) } else { (rw, rh) };
    match node.tag {
        NodeTag::Text | NodeTag::Button => {
            let font = fonts.get(node_id);
            let mods = Modifiers { bold: font.bold, underline: false, italic: font.italic };
            let shaped = if mode == RenderMode::Gui { layout.shaped.get(&node_id) } else { None };
            let item = match shaped {
                Some(l) => shaped_text_item(node, node_id, l, cx, cy, cw, ch, mode, fg, bg, mods),
                None => text_item(node, node_id, cx, cy, cw, ch, mode, fg, bg, mods),
            };
            if let Some(item) = item {
                list.push(item);
            }
        }
        NodeTag::Input => list.push(input_item(node, node_id, cx, cy, cw, fg, bg)),
        NodeTag::Img | NodeTag::Video | NodeTag::Diagram => {
            list.push(DisplayItem::Legacy { node: node_id, content: RectF::new(cx, cy, cw, ch) });
        }
        _ => {}
    }
}

/// Lays out Text/Button content (padding, optional word wrap, wide-glyph wrapping) into one
/// positioned-glyph run. Returns `None` when the node has no text.
#[allow(clippy::too_many_arguments)]
fn text_item(
    node: &Node,
    node_id: NodeId,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    content_h: f32,
    mode: RenderMode,
    fg: AnsiColor,
    bg: AnsiColor,
    mods: Modifiers,
) -> Option<DisplayItem> {
    let text = node.text.as_ref()?;
    // Padding: whole cells in TUI, as styled in GUI. (Integer-valued f32 maths is exact.)
    let pad = |v: f32| match mode {
        RenderMode::Tui => to_cells(v) as f32,
        RenderMode::Gui => v.max(0.0),
    };
    let pad_x = if node.tag == NodeTag::Button { pad(node.style.padding.left) } else { 0.0 };
    let pad_y = if node.tag == NodeTag::Button { pad(node.style.padding.top) } else { 0.0 };
    let draw_x = content_x + pad_x;
    let draw_y = content_y + pad_y;
    let inner_wf = (content_w - pad_x).max(0.0);
    let inner_hf = (content_h - pad_y).max(0.0);
    // Wrapping and the glyph loop work on whole columns/rows.
    let inner_w = inner_wf.floor().min(f32::from(u16::MAX)) as u16;
    let inner_h = inner_hf.floor().min(f32::from(u16::MAX)) as u16;

    let lines_to_draw: Vec<String> = if node.style.wrap == WrapMode::Word && inner_w > 0 {
        let mut result = Vec::new();
        for line in text.lines() {
            if line.is_empty() {
                result.push(String::new());
                continue;
            }
            let mut current_line = String::new();
            let mut current_line_width = 0;
            let mut line_has_words = false;

            let words = line.split_whitespace();
            for word in words {
                let word_w = word
                    .chars()
                    .map(|c| UnicodeWidthCache::get().width(c) as u16)
                    .sum::<u16>();

                if !line_has_words {
                    current_line = word.to_string();
                    current_line_width = word_w;
                    line_has_words = true;
                } else {
                    let space_w = 1;
                    if current_line_width + space_w + word_w <= inner_w {
                        current_line.push(' ');
                        current_line.push_str(word);
                        current_line_width += space_w + word_w;
                    } else {
                        result.push(current_line);
                        current_line = word.to_string();
                        current_line_width = word_w;
                    }
                }
            }
            if line_has_words {
                result.push(current_line);
            } else {
                result.push(String::new());
            }
        }
        result
    } else {
        text.lines().map(|s| s.to_string()).collect()
    };

    let mut glyphs = Vec::new();
    let mut cy = 0;
    for line in lines_to_draw {
        let mut cx = 0;
        for ch in line.chars() {
            let char_w = UnicodeWidthCache::get().width(ch) as u16;
            if char_w > 0 {
                if char_w > 1 && cx + char_w > inner_w && char_w <= inner_w {
                    cx = 0;
                    cy += 1;
                }
                if cx < inner_w && cy < inner_h {
                    glyphs.push(Glyph { x: cx as f32, y: cy as f32, ch, advance: char_w as f32 });
                }
                cx += char_w;
            }
        }
        cy += 1;
    }

    Some(DisplayItem::Text {
        node: node_id,
        origin: PointF { x: draw_x, y: draw_y },
        clip: RectF::new(draw_x, draw_y, inner_wf, inner_hf),
        glyphs,
        fg,
        bg,
        mods,
        shaped: None,
    })
}

/// Without `text-shaping` no layout is ever stored, so this is never reached.
#[cfg(not(feature = "text-shaping"))]
#[allow(clippy::too_many_arguments)]
fn shaped_text_item(
    _: &Node,
    _: NodeId,
    _: &TextLayout,
    _: f32,
    _: f32,
    _: f32,
    _: f32,
    _: RenderMode,
    _: AnsiColor,
    _: AnsiColor,
    _: Modifiers,
) -> Option<DisplayItem> {
    None
}

/// Emits a Text/Button node from its stored shaped layout (see the module docs). The padding and
/// the content box are computed exactly as `text_item` does, so the origin and clip agree.
#[cfg(feature = "text-shaping")]
#[allow(clippy::too_many_arguments)]
fn shaped_text_item(
    node: &Node,
    node_id: NodeId,
    layout: &TextLayout,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    content_h: f32,
    mode: RenderMode,
    fg: AnsiColor,
    bg: AnsiColor,
    mods: Modifiers,
) -> Option<DisplayItem> {
    use crate::font::{CELL_H_PX, CELL_W_PX};
    use crate::render::display_list::{ShapedRun, FACE_MONO, FACE_SANS};
    use oxiterm_proto::style::FontFace;
    node.text.as_ref()?;
    let pad = |v: f32| match mode {
        RenderMode::Tui => to_cells(v) as f32,
        RenderMode::Gui => v.max(0.0),
    };
    let pad_x = if node.tag == NodeTag::Button { pad(node.style.padding.left) } else { 0.0 };
    let pad_y = if node.tag == NodeTag::Button { pad(node.style.padding.top) } else { 0.0 };
    let (draw_x, draw_y) = (content_x + pad_x, content_y + pad_y);
    let (inner_wf, inner_hf) = ((content_w - pad_x).max(0.0), (content_h - pad_y).max(0.0));

    let face_id = |f: FontFace| match f {
        FontFace::Mono => FACE_MONO,
        FontFace::Sans => FACE_SANS,
    };
    let mut glyphs = Vec::new();
    let mut glyph_faces = Vec::new();
    let mut clusters: Vec<(u32, Box<str>)> = Vec::new();
    for line in &layout.lines {
        let y = line.top / CELL_H_PX;
        for g in &line.glyphs {
            let x = g.x / CELL_W_PX;
            // Same rule as the cell path: the glyph origin must lie inside the clip.
            if x >= inner_wf || y >= inner_hf {
                continue;
            }
            let text = layout.cluster_text(g);
            let idx = glyphs.len() as u32;
            glyphs.push(Glyph { x, y, ch: text.chars().next().unwrap_or(' '), advance: g.advance / CELL_W_PX });
            if text.chars().nth(1).is_some() {
                clusters.push((idx, text.into()));
            }
            if g.face != layout.font.face {
                glyph_faces.push((idx, face_id(g.face)));
            }
        }
    }
    let baseline = layout.lines.first().map_or(0.0, |l| (l.baseline - l.top) / CELL_H_PX);
    Some(DisplayItem::Text {
        node: node_id,
        origin: PointF { x: draw_x, y: draw_y },
        clip: RectF::new(draw_x, draw_y, inner_wf, inner_hf),
        glyphs,
        fg,
        bg,
        mods,
        shaped: Some(Box::new(ShapedRun {
            face: face_id(layout.font.face),
            size_px: layout.font.size_px,
            line_height: layout.font.line_height_px / CELL_H_PX,
            baseline,
            glyph_faces,
            clusters,
        })),
    })
}

/// Lays out a single-row input field: text (masked for passwords) padded with `_`.
/// Every char occupies exactly one cell, even wide ones.
fn input_item(
    node: &Node,
    node_id: NodeId,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    fg: AnsiColor,
    bg: AnsiColor,
) -> DisplayItem {
    let text_str = node.text.as_deref().unwrap_or("");
    let is_password = node.attrs.input_type.as_deref().map(|t| t == "password").unwrap_or(false);
    let mut chars = text_str.chars();
    // One glyph per whole column (content_w is integral in TUI, possibly fractional in GUI).
    let cols = content_w.floor().clamp(0.0, f32::from(u16::MAX)) as u16;
    let glyphs = (0..cols)
        .map(|x| {
            let ch = match chars.next() {
                Some(c) => if is_password { '*' } else { c },
                None => '_',
            };
            Glyph { x: x as f32, y: 0.0, ch, advance: 1.0 }
        })
        .collect();
    DisplayItem::Text {
        node: node_id,
        origin: PointF { x: content_x, y: content_y },
        clip: RectF::new(content_x, content_y, content_w, 1.0),
        glyphs,
        fg,
        bg,
        mods: Modifiers::default(),
        shaped: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph_str(item: &DisplayItem) -> Vec<(i32, i32, char)> {
        match item {
            DisplayItem::Text { glyphs, .. } => {
                glyphs.iter().map(|g| (g.x as i32, g.y as i32, g.ch)).collect()
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    fn text_node(tag: NodeTag, text: &str) -> Node {
        let mut n = Node::new(tag);
        n.text = Some(text.to_string());
        n
    }

    #[test]
    fn word_wrap_produces_glyph_rows() {
        let mut n = text_node(NodeTag::Text, "ab cd ef");
        n.style.wrap = WrapMode::Word;
        let item = text_item(&n, NodeId(0), 0.0, 0.0, 5.0, 3.0, RenderMode::Tui, AnsiColor::Reset, AnsiColor::Reset, Modifiers::default()).unwrap();
        let g = glyph_str(&item);
        assert_eq!(
            g,
            vec![(0, 0, 'a'), (1, 0, 'b'), (2, 0, ' '), (3, 0, 'c'), (4, 0, 'd'), (0, 1, 'e'), (1, 1, 'f')]
        );
    }

    #[test]
    fn wide_glyph_wraps_at_edge() {
        let n = text_node(NodeTag::Text, "a\u{4e16}");
        let item = text_item(&n, NodeId(0), 2.0, 1.0, 2.0, 2.0, RenderMode::Tui, AnsiColor::Reset, AnsiColor::Reset, Modifiers::default()).unwrap();
        let DisplayItem::Text { glyphs, origin, .. } = &item else { unreachable!() };
        assert_eq!((origin.x, origin.y), (2.0, 1.0));
        assert_eq!(glyphs.len(), 2);
        assert_eq!((glyphs[1].x, glyphs[1].y, glyphs[1].advance), (0.0, 1.0, 2.0));
    }

    #[test]
    fn password_input_is_masked_and_padded() {
        let mut n = text_node(NodeTag::Input, "abc");
        n.attrs.input_type = Some("password".to_string());
        let item = input_item(&n, NodeId(0), 1.0, 2.0, 5.0, AnsiColor::Reset, AnsiColor::Reset);
        let s: String = glyph_str(&item).iter().map(|g| g.2).collect();
        assert_eq!(s, "***__");
    }

    #[test]
    fn button_padding_offsets_origin_and_clip() {
        let mut n = text_node(NodeTag::Button, "ok");
        n.style.padding.left = 2.0;
        n.style.padding.top = 1.0;
        let item = text_item(&n, NodeId(0), 3.0, 4.0, 10.0, 5.0, RenderMode::Tui, AnsiColor::Reset, AnsiColor::Reset, Modifiers::default()).unwrap();
        let DisplayItem::Text { origin, clip, .. } = &item else { unreachable!() };
        assert_eq!((origin.x, origin.y), (5.0, 5.0));
        assert_eq!(*clip, RectF::new(5.0, 5.0, 8.0, 4.0));
    }

    fn fill_rects(list: &DisplayList) -> Vec<(f32, f32)> {
        list.items
            .iter()
            .filter_map(|i| match i {
                DisplayItem::Fill { rect, .. } => Some((rect.x, rect.w)),
                _ => None,
            })
            .collect()
    }

    fn build_doc(src: &str, mode: RenderMode) -> DisplayList {
        use crate::layout::engine::LayoutEngine;
        let mut doc = crate::parser::thtml::THTMLParser::parse_with_mode(src, mode).unwrap();
        let layout = LayoutEngine::with_mode(mode).compute(&mut doc, 21, 0, None).unwrap();
        let colors = precompute_inherited_colors(&doc);
        let fonts = crate::font::resolve_fonts(&doc, mode);
        build_display_list(&doc, &layout, &colors, &fonts, (0, 0), (0, 0), mode)
    }

    #[test]
    fn gui_mode_keeps_fractional_geometry_tui_stays_integral() {
        let src = r#"<screen style="width: 21; height: 3; flex-direction: row;"><box style="flex: 1; border-radius: 4.8px;"><text>a</text></box><box style="flex: 1;"><text>b</text></box></screen>"#;
        let gui = build_doc(src, RenderMode::Gui);
        let xs = fill_rects(&gui);
        assert!(xs.iter().any(|&(x, w)| x == 10.5 && w == 10.5), "gui: {xs:?}");
        assert!(xs.iter().any(|&(x, w)| x == 0.0 && w == 10.5), "gui: {xs:?}");
        let tui = build_doc(src, RenderMode::Tui);
        for (x, w) in fill_rects(&tui) {
            assert_eq!((x.fract(), w.fract()), (0.0, 0.0), "tui must stay integral");
        }
        // border-radius reaches Fill (0.5 cell = 4.8px / 9.6px) and is square elsewhere.
        let radii: Vec<f32> = gui
            .items
            .iter()
            .filter_map(|i| if let DisplayItem::Fill { radius, .. } = i { Some(*radius) } else { None })
            .collect();
        assert!(radii.iter().any(|r| (r - 0.5).abs() < 1e-5) && radii.contains(&0.0), "{radii:?}");
    }

    #[test]
    fn border_radius_lands_in_border_item() {
        let src = r#"<screen style="width: 21; height: 3;"><box style="border-style: single; border-radius: 2;">x</box></screen>"#;
        let l = build_doc(src, RenderMode::Gui);
        assert!(l.items.iter().any(|i| matches!(i, DisplayItem::Border { radius, .. } if *radius == 2.0)));
    }

    #[test]
    fn gui_text_wraps_on_floor_of_fractional_width() {
        let mut n = text_node(NodeTag::Text, "abcdef");
        n.style.wrap = WrapMode::Word;
        let item = text_item(&n, NodeId(0), 0.5, 0.0, 3.7, 3.0, RenderMode::Gui, AnsiColor::Reset, AnsiColor::Reset, Modifiers::default()).unwrap();
        let DisplayItem::Text { origin, glyphs, .. } = &item else { unreachable!() };
        assert_eq!(origin.x, 0.5);
        assert_eq!(glyphs.len(), 3, "floor(3.7) = 3 columns");
    }
}
