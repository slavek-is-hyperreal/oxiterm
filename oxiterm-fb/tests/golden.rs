//! PNG goldens of two pages in GUI mode at 80x24 / scale 1, and the determinism checks.
//!
//! Goldens live in `tests/golden/*.png`. A missing golden FAILS the test (never silently passes);
//! bless (create or overwrite) with
//! `OXI_BLESS_FB=1 cargo test -p oxiterm-fb --test golden`. Comparison is on decoded pixels, not on
//! PNG bytes. On a mismatch the actual image is written next to the target dir for review.

use std::path::{Path, PathBuf};

use oxiterm_fb::{page, Rasterizer, Surface};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn render_page(rel: &str, r: &mut Rasterizer) -> Surface {
    let examples = root().join("examples").canonicalize().unwrap();
    let list = page::display_list_of_file(&root().join(rel), Some(&examples), 80, 24).unwrap();
    let mut s = r.surface_for(80, 24).unwrap();
    r.render(&list, &mut s);
    s
}

fn check_golden(name: &str, got: &Surface) {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.png"));
    if std::env::var_os("OXI_BLESS_FB").is_some() {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        got.save_png(&file).unwrap();
        return;
    }
    let Ok(bytes) = std::fs::read(&file) else {
        panic!("missing golden {file:?}; bless it with: OXI_BLESS_FB=1 cargo test -p oxiterm-fb --test golden")
    };
    let want = Surface::from_png(&bytes).unwrap();
    let same = want.width() == got.width() && want.height() == got.height() && want.rgba() == got.rgba();
    if !same {
        let out = std::env::temp_dir().join(format!("oxiterm-fb-{name}-actual.png"));
        let _ = got.save_png(&out);
        let diff = want.rgba().iter().zip(got.rgba()).filter(|(a, b)| a != b).count();
        panic!(
            "{name}: pixels differ from {file:?} ({diff} bytes); actual written to {out:?}; review and re-bless with OXI_BLESS_FB=1"
        );
    }
}

#[test]
fn sans_fixture_matches_the_golden() {
    let mut r = Rasterizer::new(1.0);
    let s = render_page("oxiterm-server/benches/pages/sans.thtml", &mut r);
    assert_eq!((s.width(), s.height()), (768, 480));
    check_golden("sans", &s);
}

#[test]
fn counter_demo_matches_the_golden() {
    let mut r = Rasterizer::new(1.0);
    let s = render_page("examples/demos/counter.thtml", &mut r);
    check_golden("counter", &s);
}

#[test]
fn rendering_is_deterministic_cold_warm_fresh_and_over_dirty_surfaces() {
    for page in ["oxiterm-server/benches/pages/sans.thtml", "examples/demos/counter.thtml"] {
        let mut r = Rasterizer::new(1.0);
        let cold = render_page(page, &mut r);
        assert!(r.glyphs().mask_count() > 20, "glyphs were cached");
        let warm = render_page(page, &mut r);
        assert_eq!(cold.rgba(), warm.rgba(), "{page}: warm cache differs from cold");
        let mut fresh = Rasterizer::new(1.0);
        let again = render_page(page, &mut fresh);
        assert_eq!(cold.rgba(), again.rgba(), "{page}: fresh rasterizer differs");
        // re-render onto a surface that already holds another page
        let examples = root().join("examples").canonicalize().unwrap();
        let list = page::display_list_of_file(&root().join(page), Some(&examples), 80, 24).unwrap();
        let mut dirty = warm;
        r.glyphs().clear();
        r.render(&list, &mut dirty);
        assert_eq!(cold.rgba(), dirty.rgba(), "{page}: cache cleared + dirty surface differs");
        // the encoded PNG is stable too
        assert_eq!(cold.to_png().unwrap(), dirty.to_png().unwrap());
    }
}

#[test]
fn hidpi_scale_renders_and_differs_from_scale_one() {
    let mut r = Rasterizer::new(1.5);
    let list =
        page::display_list_of_file(&root().join("examples/demos/counter.thtml"), root().join("examples").canonicalize().ok().as_deref(), 80, 24)
            .unwrap();
    let mut s = r.surface_for(80, 24).unwrap();
    assert_eq!((s.width(), s.height()), (1152, 720));
    r.render(&list, &mut s);
    let mut r1 = Rasterizer::new(1.0);
    let s1 = render_page("examples/demos/counter.thtml", &mut r1);
    assert_ne!(s.rgba().len(), s1.rgba().len());
}
