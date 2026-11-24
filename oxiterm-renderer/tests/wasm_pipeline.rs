//! Stage 6.6: the whole text pipeline is the same on every target, not only raw shaping (that is
//! `text_shaping.rs`): THTML parse, TCSS fonts, shaped measuring (the 6.3 line breaker and
//! `.notdef` substitution), px to cell conversion and the display-list build.
//!
//! Everything is embedded (`include_str!`), so the test runs unchanged natively and under wasmtime:
//! `CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime --dir=." cargo test -p oxiterm-renderer
//! --no-default-features --features text-shaping --test wasm_pipeline --target wasm32-wasip1`.
#![cfg(feature = "text-shaping")]

use oxiterm_proto::style::RenderMode;

#[path = "support/dl_dump.rs"]
mod dl_dump;
use dl_dump::{dump, render};

const SANS_PAGE: &str = include_str!("../../oxiterm-server/benches/pages/sans.thtml");
/// The committed golden of the sans benchmark page (also read by `shaped_display_list.rs`).
const SANS_GOLDEN: &str = include_str!("golden_dl/sans@80x24.txt");

/// The proportional benchmark page at 80x24 gives, byte for byte, the committed golden: glyph
/// x/y/advance bits, baselines, line breaks and the px to cell conversion are identical on
/// x86_64 and wasm32.
#[test]
fn sans_page_matches_the_committed_golden_on_this_target() {
    let (list, _, _) = render(SANS_PAGE, RenderMode::Gui, 80, 24);
    let got = dump(&list);
    assert!(got.contains("run face="), "the fixture must produce shaped runs");
    if got != SANS_GOLDEN {
        let first = got.lines().zip(SANS_GOLDEN.lines()).enumerate().find(|(_, (a, b))| a != b);
        panic!("sans page differs from golden_dl/sans@80x24.txt on {}; first diff: {first:?}", std::env::consts::ARCH);
    }
}

/// Paths the sans golden does not take: characters Noto Sans lacks (CJK, `.notdef` substitution,
/// a box-drawing fallback to the mono face), a combining mark, a ligature, an explicit newline,
/// a long unbreakable word that must be split, narrow widths, a larger font size and wrap none.
const EDGE_PAGE: &str = "<style>\
.s { font-family: sans-serif; wrap: word; width: 12; fg: #ffffff; bg: #102030 }\
.n { font-family: sans-serif; wrap: none; font-size: 18px; line-height: 24px }\
.b { font-family: sans-serif; font-size: 22px; padding-left: 1; border-style: single }\
</style>\
<screen style=\"width: 40; height: 20; flex-direction: column;\">\
<text class=\"s\">ab \u{65e5}\u{672c}\u{8a9e} cd \u{2500}\u{2502} office fl e\u{301}x</text>\
<text class=\"s\">Supercalifragilisticexpialidocious_and_more\nsecond   line here</text>\
<text class=\"n\">AVAVA To Ty \u{65e5}\u{672c} wrap none line that is long</text>\
<button class=\"b\" event-htmx=\"x:y\">Go \u{65e5}\u{672c} office</button>\
</screen>";

/// FNV-1a 64 of `dump()` of the edge page. Not a golden file: the number was produced on
/// x86_64 and must come out the same under wasmtime. If the pipeline changes on purpose, the
/// failure message prints the new value to paste here (after the same check on wasm32).
const EDGE_FNV: u64 = 0xf33f_b687_9919_7215;

fn fnv1a(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[test]
fn edge_cases_hash_is_the_same_on_every_target() {
    let (list, _, _) = render(EDGE_PAGE, RenderMode::Gui, 40, 20);
    let got = dump(&list);
    // not vacuous: shaped runs exist, a glyph the face lacks was substituted (CJK is one glyph
    // per char at the font's own .notdef advance) and a fallback face or a cluster was recorded
    assert!(got.matches("run face=").count() >= 4, "{got}");
    assert!(got.contains("'\u{65e5}'"), "{got}");
    assert!(got.contains("faces=[(") || got.contains("clusters=[("), "{got}");
    if std::env::var_os("OXI_PRINT_EDGE_DUMP").is_some() {
        println!("{got}");
    }
    assert_eq!(fnv1a(&got), EDGE_FNV, "edge-page dump changed (arch {}); new value: {:#018x}", std::env::consts::ARCH, fnv1a(&got));
}
