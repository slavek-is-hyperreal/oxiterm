//! Stage 6.0 determinism table for the shaping spike. The expected table is `f32::to_bits` of
//! every line width / glyph x / advance, baked in `text_shaping_expected.txt`. The same file must
//! pass on x86_64 and on wasm32 (wasmtime), which is what makes shaped layout portable.
//! Regenerate (only on a deliberate change): `cargo test -p oxiterm-renderer --no-default-features
//! --features text-shaping --test text_shaping dump_expected -- --ignored --nocapture`.
#![cfg(feature = "text-shaping")]
#![allow(clippy::pedantic)]

use oxiterm_renderer::text::shaping::Shaper;

static SANS: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
static MONO: &[u8] = include_bytes!("../assets/fonts/NotoSansMono-Regular.ttf");

const STRINGS: &[&str] = &[
    "Hello, world! 0123456789",
    "The quick brown fox jumps over the lazy dog",
    "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144} \u{104}\u{106}\u{118}\u{141}\u{143}\u{d3}\u{15a}\u{179}\u{17b}",
    "e\u{301} a\u{328}\u{301} o\u{308}",
    "office waffle fluffy affinity",
    "ab \u{65e5}\u{672c} cd",
    "AVAVA To Ty",
    "line one\nline two is longer",
];
const WIDTHS: &[Option<f32>] = &[None, Some(100.0), Some(37.5)];

fn render(shaper: &mut Shaper) -> String {
    let mut out = String::new();
    for (si, s) in STRINGS.iter().enumerate() {
        for w in WIDTHS {
            out.push_str(&format!("case {si} w={:?}\n", w.map(f32::to_bits)));
            for l in shaper.shape(s, 16.0, *w, false) {
                out.push_str(&format!(
                    " line w={:08x} top={:08x} base={:08x}\n",
                    l.width.to_bits(),
                    l.top.to_bits(),
                    l.baseline.to_bits()
                ));
                for g in &l.glyphs {
                    out.push_str(&format!(
                        "  g x={:08x} adv={:08x} id={} {:?}\n",
                        g.x.to_bits(),
                        g.advance.to_bits(),
                        g.glyph_id,
                        g.cluster
                    ));
                }
            }
        }
    }
    out
}

#[test]
fn shaping_bits_match_baked_table() {
    let mut sh = Shaper::new(&[SANS]).expect("font parses");
    let got = render(&mut sh);
    let want = include_str!("text_shaping_expected.txt");
    if got != want {
        let first = got.lines().zip(want.lines()).enumerate().find(|(_, (a, b))| a != b);
        panic!("shaping output differs from baked table; first diff: {first:?}");
    }
}

#[test]
fn shaping_is_repeatable_in_process() {
    let mut a = Shaper::new(&[SANS]).unwrap();
    let mut b = Shaper::new(&[SANS]).unwrap();
    assert_eq!(render(&mut a), render(&mut b));
    assert_eq!(render(&mut a), render(&mut a));
}

#[test]
fn wrapping_and_clusters() {
    let mut sh = Shaper::new(&[SANS]).unwrap();
    let wide = sh.shape("Hello, world! 0123456789", 16.0, None, false);
    assert_eq!(wide.len(), 1);
    let narrow = sh.shape("Hello, world! 0123456789", 16.0, Some(100.0), false);
    assert!(narrow.len() > 1);
    assert!(narrow.iter().all(|l| l.width <= 100.0));
    // Combining mark stays inside the cluster of its base.
    let c = sh.shape("e\u{301}", 16.0, None, false);
    assert!(c[0].glyphs.iter().any(|g| g.cluster == "e\u{301}"), "{:?}", c[0].glyphs);
}

#[test]
fn mono_font_has_equal_advances() {
    let mut sh = Shaper::new(&[MONO]).unwrap();
    let l = &sh.shape("iiiWWW000", 16.0, None, false)[0];
    let a = l.glyphs[0].advance;
    assert!(a > 0.0 && l.glyphs.iter().all(|g| g.advance.to_bits() == a.to_bits()));
}

#[test]
#[ignore = "regenerates the baked table"]
fn dump_expected() {
    let mut sh = Shaper::new(&[SANS]).unwrap();
    std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/text_shaping_expected.txt"), render(&mut sh))
        .unwrap();
}
