//! Font properties after inheritance (Phase 6, Stage 6.2).
//!
//! The cascade (`parser::tcss`) stores what each node *declares* in `ComputedStyle::font`
//! (every field optional = inherit). [`resolve_fonts`] turns that into the *effective*
//! [`ResolvedFont`] of every node reachable from the root, inherited parent to child.
//!
//! Nothing here depends on the `text-shaping` feature, so `oxiterm-core` and the TUI path can
//! use it. The unit constants below duplicate the bundle's (`text::bundle`); a test behind
//! `text-shaping` asserts they agree.
//!
//! Stage 6.2 only resolves. Layout measuring and glyph positioning read it from 6.3 on.

use crate::document::THTMLDocument;
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{FontFace, FontSize, FontWeight, LineHeight, RenderMode};
use std::collections::HashMap;

/// Default font size in px (the bundle's `DEFAULT_FONT_SIZE_PX`).
pub const DEFAULT_FONT_SIZE_PX: f32 = 16.0;
/// Line height as a factor of the font size (the shaper's `LINE_HEIGHT_FACTOR`).
pub const LINE_HEIGHT_FACTOR: f32 = 1.25;
/// GUI reference cell width in px (advance of the mono face at the default size).
pub const CELL_W_PX: f32 = 9.6;
/// GUI reference cell height in px (`DEFAULT_FONT_SIZE_PX * LINE_HEIGHT_FACTOR`).
pub const CELL_H_PX: f32 = 20.0;
/// Resolved sizes are clamped to this range so nested `em` cannot run away.
const SIZE_RANGE: (f32, f32) = (1.0, 1024.0);

/// The effective font of one node. All fields are plain values, so 6.3 can compare it
/// with `==` to decide whether a node's layout inputs changed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedFont {
    /// Bundle face.
    pub face: FontFace,
    /// Font size in px.
    pub size_px: f32,
    /// Line height in px (already resolved against `size_px`).
    pub line_height_px: f32,
    /// Weight 600 or more.
    pub bold: bool,
    /// `italic` or `oblique`.
    pub italic: bool,
}

impl ResolvedFont {
    /// `true` when text in this font is measured in whole cells with no shaping: the root default
    /// (mono, 16 px, line height 20 px). Bold and italic alone do not leave this path: the mono
    /// face is fixed-advance and a synthetic bold or slant in a client does not change advances.
    /// The whole font is compared (face, size and line height), not just the face: mono at
    /// 24 px or with a taller line is not on the cell grid.
    #[inline]
    pub fn uses_cell_metrics(&self) -> bool {
        self.face == FontFace::Mono
            && self.size_px == DEFAULT_FONT_SIZE_PX
            && self.line_height_px == CELL_H_PX
    }
}

impl Default for ResolvedFont {
    /// The root default: mono, 16 px, line height = the reference cell height (20 px).
    fn default() -> Self {
        Self {
            face: FontFace::Mono,
            size_px: DEFAULT_FONT_SIZE_PX,
            line_height_px: CELL_H_PX,
            bold: false,
            italic: false,
        }
    }
}

/// What a child inherits: the resolved font plus the parts that must stay symbolic.
#[derive(Clone, Copy)]
struct Inherited {
    font: ResolvedFont,
    /// Numeric weight (bolder/lighter need it; `bold` alone loses it).
    weight: u16,
    /// The inherited line-height rule, so a unitless multiplier follows each child's own size.
    line_height: LineHeight,
}

impl Default for Inherited {
    fn default() -> Self {
        Self { font: ResolvedFont::default(), weight: 400, line_height: LineHeight::Multiplier(LINE_HEIGHT_FACTOR) }
    }
}

/// Effective fonts of a document. Pages that declare no font property pay nothing: no map
/// is built and every lookup returns the default.
#[derive(Debug, Default)]
pub struct ResolvedFonts {
    /// `None`: no node declares a font property, so every node has the default font.
    map: Option<HashMap<NodeId, ResolvedFont>>,
}

impl ResolvedFonts {
    /// The effective font of `id` (the default for unknown or unreachable nodes).
    #[inline]
    pub fn get(&self, id: NodeId) -> ResolvedFont {
        match &self.map {
            None => ResolvedFont::default(),
            Some(m) => m.get(&id).copied().unwrap_or_default(),
        }
    }

    /// `true` when no node declares a font property (every lookup is the default).
    #[inline]
    pub fn is_uniform(&self) -> bool {
        self.map.is_none()
    }
}

/// CSS `bolder` / `lighter` steps.
fn step_weight(inherited: u16, w: FontWeight) -> u16 {
    match w {
        FontWeight::Absolute(v) => v,
        FontWeight::Bolder => match inherited {
            0..=399 => 400,
            400..=599 => 700,
            _ => 900,
        },
        FontWeight::Lighter => match inherited {
            0..=599 => 100,
            600..=799 => 400,
            _ => 700,
        },
    }
}

fn resolve_one(parent: &Inherited, decl: &oxiterm_proto::style::FontDecl, mode: RenderMode) -> Inherited {
    let mut out = *parent;
    // GUI-only properties: TUI has one font and cell size, so they are skipped there.
    if mode == RenderMode::Gui {
        if let Some(f) = decl.family {
            out.font.face = f;
        }
        if let Some(s) = decl.size {
            let px = match s {
                FontSize::Px(v) => v,
                FontSize::Em(f) => parent.font.size_px * f,
            };
            out.font.size_px = px.clamp(SIZE_RANGE.0, SIZE_RANGE.1);
        }
        if let Some(lh) = decl.line_height {
            out.line_height = lh;
        }
        // The line height follows the node's own size when it is a multiplier (also when only
        // the size changed), and stays absolute when it is a length.
        out.font.line_height_px = match out.line_height {
            LineHeight::Multiplier(m) => out.font.size_px * m,
            LineHeight::Px(p) => p,
        };
    }
    if let Some(w) = decl.weight {
        out.weight = step_weight(parent.weight, w);
        out.font.bold = out.weight >= 600;
    }
    if let Some(i) = decl.italic {
        out.font.italic = i;
    }
    out
}

/// Resolves the effective font of every node reachable from the root.
///
/// In [`RenderMode::Tui`] only weight and style apply (bold/italic cell modifiers);
/// `font-family`, `font-size` and `line-height` are ignored. Returns the uniform (empty)
/// result without allocating when no node declares any font property.
pub fn resolve_fonts(doc: &THTMLDocument, mode: RenderMode) -> ResolvedFonts {
    if !doc.arena.iter().any(|(_, n)| !n.style.font.is_empty()) {
        return ResolvedFonts { map: None };
    }
    let mut map = HashMap::new();
    let mut stack = vec![(doc.root, Inherited::default())];
    while let Some((id, parent)) = stack.pop() {
        let Some(node) = doc.arena.get(id) else { continue };
        let cur = resolve_one(&parent, &node.style.font, mode);
        map.insert(id, cur.font);
        for &c in &node.children {
            stack.push((c, cur));
        }
    }
    ResolvedFonts { map: Some(map) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::THTMLParser;

    fn doc(src: &str, mode: RenderMode) -> THTMLDocument {
        THTMLParser::parse_with_mode(src, mode).unwrap()
    }

    /// Resolved font of the n-th node in pre-order below the root (0 = first child of root).
    fn nth(d: &THTMLDocument, f: &ResolvedFonts, n: usize) -> ResolvedFont {
        let mut order = Vec::new();
        let mut stack = vec![d.root];
        while let Some(id) = stack.pop() {
            order.push(id);
            for &c in d.arena.get(id).unwrap().children.iter().rev() {
                stack.push(c);
            }
        }
        f.get(order[n + 1])
    }

    #[test]
    fn root_defaults_and_uniform_fast_path() {
        let d = doc(r#"<box><text>a</text></box>"#, RenderMode::Gui);
        let f = resolve_fonts(&d, RenderMode::Gui);
        assert!(f.is_uniform());
        let r = nth(&d, &f, 1);
        assert_eq!(r, ResolvedFont::default());
        assert_eq!((r.face, r.size_px, r.line_height_px, r.bold, r.italic), (FontFace::Mono, 16.0, 20.0, false, false));
    }

    #[test]
    fn nested_override_and_inheritance() {
        let d = doc(
            r#"<style>.p { font-family: sans-serif; font-size: 20px; font-weight: bold }
                       .c { font-family: monospace; font-style: italic }</style>
               <box class="p"><text>a</text><box class="c"><text>b</text></box></box><text>after</text>"#,
            RenderMode::Gui,
        );
        let f = resolve_fonts(&d, RenderMode::Gui);
        assert!(!f.is_uniform());
        let p = nth(&d, &f, 0);
        assert_eq!((p.face, p.size_px, p.bold, p.italic), (FontFace::Sans, 20.0, true, false));
        assert_eq!(p.line_height_px, 25.0, "multiplier follows the node's own size");
        assert_eq!(nth(&d, &f, 1), p, "child inherits everything");
        let c = nth(&d, &f, 2);
        assert_eq!((c.face, c.size_px, c.bold, c.italic), (FontFace::Mono, 20.0, true, true));
        assert_eq!(nth(&d, &f, 3), c, "grandchild inherits the override");
        assert_eq!(nth(&d, &f, 4), ResolvedFont::default(), "sibling outside the subtree is untouched");
    }

    #[test]
    fn size_relative_to_parent() {
        let d = doc(
            r#"<style>.a { font-size: 20px } .b { font-size: 1.5em } .c { font-size: 50% }</style>
               <box class="a"><box class="b"><box class="c"><text>x</text></box></box></box>"#,
            RenderMode::Gui,
        );
        let f = resolve_fonts(&d, RenderMode::Gui);
        assert_eq!(nth(&d, &f, 0).size_px, 20.0);
        assert_eq!(nth(&d, &f, 1).size_px, 30.0);
        assert_eq!(nth(&d, &f, 2).size_px, 15.0);
        // A relative size on the root is relative to the 16 px default.
        let d = doc(r#"<style>.a { font-size: 2em }</style><box class="a">x</box>"#, RenderMode::Gui);
        assert_eq!(nth(&d, &resolve_fonts(&d, RenderMode::Gui), 0).size_px, 32.0);
    }

    #[test]
    fn line_height_multiplier_follows_size_and_length_stays() {
        let d = doc(
            r#"<style>.a { line-height: 2 } .b { font-size: 10px } .c { line-height: 30px } .d { font-size: 40px }</style>
               <box class="a"><box class="b"><box class="c"><box class="d">x</box></box></box></box>"#,
            RenderMode::Gui,
        );
        let f = resolve_fonts(&d, RenderMode::Gui);
        assert_eq!(nth(&d, &f, 0).line_height_px, 32.0); // 2 * 16
        assert_eq!(nth(&d, &f, 1).line_height_px, 20.0); // 2 * 10
        assert_eq!(nth(&d, &f, 2).line_height_px, 30.0);
        assert_eq!(nth(&d, &f, 3).line_height_px, 30.0, "px line height is inherited as px");
        // Only the size changed: the default 1.25 follows it.
        let d = doc(r#"<style>.a { font-size: 32px }</style><box class="a">x</box>"#, RenderMode::Gui);
        assert_eq!(nth(&d, &resolve_fonts(&d, RenderMode::Gui), 0).line_height_px, 40.0);
    }

    #[test]
    fn weight_steps_and_threshold() {
        let d = doc(
            r#"<style>.a { font-weight: 500 } .b { font-weight: bolder } .c { font-weight: lighter } .e { font-weight: 600 } .n { font-weight: normal }</style>
               <box class="a"><box class="b"><box class="c">x</box></box></box>
               <box class="e"><box class="n">y</box></box>"#,
            RenderMode::Gui,
        );
        let f = resolve_fonts(&d, RenderMode::Gui);
        assert!(!nth(&d, &f, 0).bold, "500 is not bold");
        assert!(nth(&d, &f, 1).bold, "bolder than 500 is 700");
        assert!(!nth(&d, &f, 2).bold, "lighter than 700 is 400");
        assert!(nth(&d, &f, 3).bold, "600 is bold");
        assert!(!nth(&d, &f, 4).bold);
    }

    #[test]
    fn tui_ignores_gui_only_properties_but_keeps_weight_and_style() {
        let src = r#"<style>.a { font-family: sans-serif; font-size: 30px; line-height: 3; font-weight: bold; font-style: italic }</style>
                     <box class="a"><text>x</text></box>"#;
        let d = doc(src, RenderMode::Tui);
        let r = nth(&d, &resolve_fonts(&d, RenderMode::Tui), 1);
        assert_eq!((r.face, r.size_px, r.line_height_px), (FontFace::Mono, 16.0, 20.0));
        assert!(r.bold && r.italic);
        let g = nth(&d, &resolve_fonts(&d, RenderMode::Gui), 1);
        assert_eq!((g.face, g.size_px, g.line_height_px), (FontFace::Sans, 30.0, 90.0));
    }

    #[test]
    fn unreachable_nodes_get_the_default() {
        let mut d = doc(r#"<style>.a { font-size: 30px }</style><box class="a">x</box>"#, RenderMode::Gui);
        let orphan = d.arena.alloc(oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Text));
        assert_eq!(resolve_fonts(&d, RenderMode::Gui).get(orphan), ResolvedFont::default());
    }
}

#[cfg(all(test, feature = "text-shaping"))]
mod bundle_agreement {
    use super::*;
    use crate::text::bundle::{FontBundle, DEFAULT_FONT_SIZE_PX as B_SIZE};

    #[test]
    fn constants_match_the_bundle() {
        let m = FontBundle::metrics();
        assert_eq!(DEFAULT_FONT_SIZE_PX, B_SIZE);
        assert!((CELL_W_PX - m.cell_w_px).abs() < 1e-4, "{} vs {}", CELL_W_PX, m.cell_w_px);
        assert_eq!(CELL_H_PX, m.cell_h_px);
        assert_eq!(LINE_HEIGHT_FACTOR, crate::text::shaping::LINE_HEIGHT_FACTOR);
    }
}
