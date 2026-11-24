//! The bundled fonts and the GUI reference cell (Phase 6, Stage 6.1).
//!
//! One unit, set by the server: the GUI cell is defined by the bundled mono face, not by
//! whatever monospace font a client happens to have. Display-list clients fetch
//! [`FontBundle::metrics`] (as JSON) before their first `0x10` dims message and use it for
//! cols/rows, painting and pointer mapping.

use std::sync::OnceLock;

use serde::Serialize;

use super::shaping::{Shaper, LINE_HEIGHT_FACTOR};

/// Face id (and CSS family name the client registers) of the monospace face.
pub const MONO_ID: &str = "oxi-mono";
/// Face id (and CSS family name the client registers) of the proportional face.
pub const SANS_ID: &str = "oxi-sans";
/// Default GUI font size in px.
pub const DEFAULT_FONT_SIZE_PX: f32 = 16.0;

pub static MONO_TTF: &[u8] = include_bytes!("../../assets/fonts/NotoSansMono-Regular.ttf");
pub static SANS_TTF: &[u8] = include_bytes!("../../assets/fonts/NotoSans-Regular.ttf");
/// OFL 1.1 texts; they must be shipped with the fonts.
pub static MONO_LICENSE: &[u8] = include_bytes!("../../assets/fonts/NotoSansMono-OFL.txt");
pub static SANS_LICENSE: &[u8] = include_bytes!("../../assets/fonts/NotoSans-OFL.txt");

/// One face as the client sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FaceInfo {
    /// Stable id; also the CSS family name the client registers the face under.
    pub id: &'static str,
    /// The font's own family name (informational).
    pub family_name: &'static str,
    /// Server path of the font file.
    pub url_path: String,
    /// Server path of the licence text.
    pub license_path: String,
}

/// Metrics document served as `/oxiui/fonts.json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FontMetrics {
    /// GUI reference cell width in CSS px: advance of the mono face at `font_size_px`.
    pub cell_w_px: f32,
    /// GUI reference cell height in CSS px: `font_size_px * 1.25` (the line height used by the
    /// shaper and by the pre-6.1 client, 20 px at 16 px).
    pub cell_h_px: f32,
    pub font_size_px: f32,
    pub faces: Vec<FaceInfo>,
}

pub struct FontBundle;

impl FontBundle {
    /// Bytes of the file served for `id`, or `None` for an unknown id.
    pub fn font_bytes(id: &str) -> Option<&'static [u8]> {
        match id {
            MONO_ID => Some(MONO_TTF),
            SANS_ID => Some(SANS_TTF),
            _ => None,
        }
    }

    /// Bytes of the OFL text for `id`.
    pub fn license_bytes(id: &str) -> Option<&'static [u8]> {
        match id {
            MONO_ID => Some(MONO_LICENSE),
            SANS_ID => Some(SANS_LICENSE),
            _ => None,
        }
    }

    /// The reference cell and the faces. The width is read from the mono face (shaped "M" at the
    /// default size), computed once.
    pub fn metrics() -> &'static FontMetrics {
        static M: OnceLock<FontMetrics> = OnceLock::new();
        M.get_or_init(|| {
            let size = DEFAULT_FONT_SIZE_PX;
            let mut shaper = Shaper::new(&[MONO_TTF]).expect("bundled mono face parses");
            let cell_w_px = shaper
                .shape("M", size, None, false)
                .first()
                .and_then(|l| l.glyphs.first())
                .map(|g| g.advance)
                .expect("bundled mono face shapes M");
            let face = |id: &'static str, family_name: &'static str| FaceInfo {
                id,
                family_name,
                url_path: format!("/oxiui/fonts/{id}.ttf"),
                license_path: format!("/oxiui/fonts/LICENSE-{id}.txt"),
            };
            FontMetrics {
                cell_w_px,
                cell_h_px: size * LINE_HEIGHT_FACTOR,
                font_size_px: size,
                faces: vec![face(MONO_ID, "Noto Sans Mono"), face(SANS_ID, "Noto Sans")],
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_is_mono_advance_at_default_size() {
        let m = FontBundle::metrics();
        assert_eq!(m.font_size_px, 16.0);
        // Noto Sans Mono: 600 / 1000 em, so 9.6 px at 16 px.
        assert!((m.cell_w_px - 9.6).abs() < 1e-4, "{}", m.cell_w_px);
        assert_eq!(m.cell_h_px, 20.0);
    }

    #[test]
    fn every_mono_glyph_has_the_cell_advance() {
        let mut s = Shaper::new(&[MONO_TTF]).unwrap();
        let w = FontBundle::metrics().cell_w_px;
        for ch in ["i", "W", "m", ".", "0"] {
            let g = &s.shape(ch, 16.0, None, false)[0].glyphs[0];
            assert!((g.advance - w).abs() < 1e-4, "{ch}");
        }
    }

    #[test]
    fn faces_load_in_the_shaper_and_have_distinct_ids() {
        for id in [MONO_ID, SANS_ID] {
            let bytes = FontBundle::font_bytes(id).unwrap();
            let mut s = Shaper::new(&[bytes]).expect("face parses");
            assert_eq!(s.shape("Hi", 16.0, None, false)[0].glyphs.len(), 2);
            assert!(FontBundle::license_bytes(id).unwrap().starts_with(b"Copyright"));
        }
        assert!(FontBundle::font_bytes("nope").is_none());
        let m = FontBundle::metrics();
        assert_eq!(m.faces.len(), 2);
        assert_ne!(m.faces[0].id, m.faces[1].id);
    }
}
