//! Stage 7.6: media must show up in every output, and never silently disappear again.
//!
//! The cell sink (`render_node`) is what SSH terminals and web cell clients get. These tests pin,
//! per output, what each media node produces: Kitty/Sixel escape sequences, half-block cells,
//! diagram labels, the Rive widget, and that every refused `src` (traversal, absolute, encoded,
//! symlink out of the base) draws nothing at all.
use std::path::{Path, PathBuf};

use oxiterm_proto::style::{AnsiColor, RenderMode, TerminalProfile};
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::render::renderer::{Renderer, VIRTUAL_FS};
use oxiterm_renderer::CellBuffer;

const FLOW: &str = include_str!("../../examples/assets/flow.mmd");

/// `<tmp>/oxi_media_parity_<name>/{app/{assets,demos},outside}`; `app` is the app base and
/// `app/demos` the directory of the page that holds the media.
struct Fx {
    root: PathBuf,
}

impl Fx {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("oxi_media_parity_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["app/assets", "app/demos", "outside"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("app/assets/flow.mmd"), FLOW).unwrap();
        std::fs::write(root.join("outside/secret.mmd"), FLOW).unwrap();
        std::fs::write(root.join("app/assets/toggle.riv"), [0u8; 12]).unwrap();
        // A real PNG (the cell sink decodes it) and an SVG.
        let mut img = image::RgbaImage::new(16, 16);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 16) as u8, (y * 16) as u8, 200, 255]);
        }
        img.save(root.join("app/assets/a.png")).unwrap();
        std::fs::write(
            root.join("app/assets/m.svg"),
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><rect width="40" height="40" fill="#e33"/><circle cx="20" cy="20" r="10" fill="#fff"/></svg>"##,
        )
        .unwrap();
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/assets/bell.json"), root.join("app/assets/bell.json")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("outside/secret.mmd"), root.join("app/assets/link.mmd")).unwrap();
        Self { root }
    }
    fn app(&self) -> PathBuf {
        self.root.join("app")
    }
    fn page_dir(&self) -> PathBuf {
        self.root.join("app/demos")
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn web() -> TerminalProfile {
    TerminalProfile { is_web: true, ..Default::default() }
}
fn kitty() -> TerminalProfile {
    TerminalProfile { supports_kitty_gfx: true, ..Default::default() }
}
fn sixel() -> TerminalProfile {
    TerminalProfile { supports_sixel: true, ..Default::default() }
}

/// Renders one media element of `w x h` cells and returns the buffer (`cols x rows` big).
fn render(tag: &str, src: &str, w: u16, h: u16, profile: &TerminalProfile, base_dir: Option<&Path>, app_base: Option<&Path>) -> CellBuffer {
    let (cols, rows) = (w + 4, h + 2);
    let page = format!(r#"<screen style="width: {cols}; height: {rows}; flex-direction: column;"><{tag} src="{src}" alt="x" style="width: {w}; height: {h};" /></screen>"#);
    let mut doc = THTMLParser::parse_with_mode(&page, RenderMode::Tui).unwrap();
    let mut engine = LayoutEngine::new();
    engine.set_viewport_rows(rows);
    let layout = engine.compute(&mut doc, cols, 0, None).unwrap();
    let mut buf = CellBuffer::new(cols, rows);
    Renderer::render_node(&doc, &layout, &mut buf, profile, base_dir, app_base, 0);
    buf
}

fn text(buf: &CellBuffer) -> String {
    (0..buf.height).map(|y| (0..buf.width).map(|x| buf.cells[buf.flat_idx(x, y).unwrap()].ch).collect::<String>()).collect::<Vec<_>>().join("\n")
}

fn blank(buf: &CellBuffer) -> bool {
    buf.cells.iter().all(|c| c.ch == ' ') && buf.graphics.is_empty()
}

fn block_cells(buf: &CellBuffer) -> usize {
    buf.cells.iter().filter(|c| matches!(c.ch, '▀' | '▄' | '█')).count()
}

// ---- diagram -------------------------------------------------------------------------------

#[test]
fn diagram_src_is_relative_to_the_page_and_draws_the_real_labels() {
    let fx = Fx::new("diagram");
    let buf = render("diagram", "../assets/flow.mmd", 100, 30, &web(), Some(&fx.page_dir()), Some(&fx.app()));
    let t = text(&buf);
    // Real content, not a placeholder: node labels from flow.mmd.
    for label in ["OxiTerm Engine", "Renderer THTML", "App Server Dispatcher"] {
        assert!(t.contains(label), "missing {label:?} in\n{t}");
    }
    assert!(!t.contains("brak grafiki"), "placeholder instead of the diagram:\n{t}");
}

#[test]
fn diagram_with_the_old_root_relative_src_says_the_file_is_missing() {
    // `examples/assets/flow.mmd` from `app/demos` is `app/demos/examples/assets/flow.mmd`.
    let fx = Fx::new("diagram_old");
    let buf = render("diagram", "examples/assets/flow.mmd", 60, 14, &web(), Some(&fx.page_dir()), Some(&fx.app()));
    assert!(text(&buf).contains("brak grafiki"), "a missing file must be visible, not an empty box");
}

#[test]
fn every_refused_src_draws_nothing_for_every_media_tag() {
    let fx = Fx::new("traversal");
    let mut bad = vec![
        "../../outside/secret.mmd".to_string(),                 // out of the base
        "../../../etc/passwd".to_string(),
        "/etc/passwd".to_string(),                               // absolute
        format!("{}", fx.root.join("outside/secret.mmd").display()),
        "..%2f..%2foutside/secret.mmd".to_string(),              // percent-encoded
        "%2e%2e/%2e%2e/outside/secret.mmd".to_string(),
        "..%5c..%5coutside/secret.mmd".to_string(),
        "file:///etc/passwd".to_string(),                        // scheme
        "http://example.com/a.png".to_string(),
        "..\\..\\outside\\secret.mmd".to_string(),               // backslashes
        "../".to_string(),                                       // directory
    ];
    #[cfg(unix)]
    bad.push("../assets/link.mmd".to_string()); // symlink inside the base pointing outside it
    for src in &bad {
        for tag in ["diagram", "img", "video"] {
            let buf = render(tag, src, 40, 10, &TerminalProfile::default(), Some(&fx.page_dir()), Some(&fx.app()));
            assert!(blank(&buf), "{tag} src={src:?} must draw nothing, got:\n{}", text(&buf));
        }
    }
    // No app base at all fails closed too.
    let buf = render("diagram", "../assets/flow.mmd", 60, 14, &web(), Some(&fx.page_dir()), None);
    assert!(blank(&buf));
}

#[test]
fn virtual_root_reads_diagrams_from_the_virtual_file_system() {
    // The browser's local mode: no file system, the empty app base, files in VIRTUAL_FS.
    VIRTUAL_FS.with(|fs| {
        fs.borrow_mut().insert(PathBuf::from("assets/vroot_flow.mmd"), FLOW.as_bytes().to_vec());
    });
    let base = Path::new("demos");
    let empty = Path::new("");
    let buf = render("diagram", "../assets/vroot_flow.mmd", 100, 30, &web(), Some(base), Some(empty));
    assert!(text(&buf).contains("OxiTerm Engine"), "{}", text(&buf));
    // Not mounted (yet): nothing, no placeholder (the page is still fetching it).
    let buf = render("diagram", "../assets/not_mounted.mmd", 40, 10, &web(), Some(base), Some(empty));
    assert!(blank(&buf));
    // Still contained lexically.
    let buf = render("diagram", "../../assets/vroot_flow.mmd", 100, 30, &web(), Some(base), Some(empty));
    assert!(blank(&buf));
}

// ---- Rive ----------------------------------------------------------------------------------

#[test]
fn rive_widget_is_drawn_as_cells_for_web_clients_and_follows_its_state() {
    let fx = Fx::new("rive");
    let buf = render("img", "../assets/toggle.riv", 14, 3, &web(), Some(&fx.page_dir()), Some(&fx.app()));
    assert!(block_cells(&buf) >= 10, "the toggle must be drawn, got:\n{}", text(&buf));
    assert!(buf.graphics.is_empty());
    let blue = AnsiColor::TrueColor(0, 120, 255);
    assert!(!buf.cells.iter().any(|c| c.fg == blue || c.bg == blue), "off state has no blue track");

    // Toggle it through the registry the click handler uses (same resolved key).
    let key = oxiterm_proto::pathsafe::resolve_media(Some(&fx.page_dir()), Some(&fx.app()), "../assets/toggle.riv").unwrap();
    let reg = oxiterm_renderer::render::cache::PlaybackRegistry::get();
    // (the playback scope is the session id; `web()` renders in scope 0, so the click is in scope 0 too)
    reg.set_click(0, &key, true, Some((1, 1)));
    reg.set_click(0, &key, false, None);
    let on = render("img", "../assets/toggle.riv", 14, 3, &web(), Some(&fx.page_dir()), Some(&fx.app()));
    assert!(on.cells.iter().any(|c| c.fg == blue || c.bg == blue), "toggled: blue track expected");
}

// ---- SSH terminals (cell sink): Kitty, Sixel, half blocks ------------------------------------

#[cfg(feature = "native-media")]
#[test]
fn ssh_terminals_get_kitty_sixel_or_half_blocks_for_png_and_svg() {
    let fx = Fx::new("ssh");
    for (file, w, h) in [("a.png", 11u16, 5u16), ("m.svg", 13, 6)] {
        let src = format!("../assets/{file}");
        let k = render("img", &src, w, h, &kitty(), Some(&fx.page_dir()), Some(&fx.app()));
        assert!(k.graphics.iter().any(|g| g.windows(3).any(|w| w == b"\x1b_G")), "{file}: kitty APC graphics expected");
        let s = render("img", &src, w + 1, h + 1, &sixel(), Some(&fx.page_dir()), Some(&fx.app()));
        assert!(s.graphics.iter().any(|g| g.windows(2).any(|w| w == b"\x1bP")), "{file}: sixel DCS expected");
        let plain = render("img", &src, w + 2, h + 2, &TerminalProfile::default(), Some(&fx.page_dir()), Some(&fx.app()));
        assert!(plain.graphics.is_empty());
        assert!(block_cells(&plain) > (w as usize), "{file}: half-block fallback expected:\n{}", text(&plain));
    }
}

#[cfg(feature = "native-media")]
#[test]
fn ssh_terminals_get_lottie_frames_and_the_rive_widget() {
    let fx = Fx::new("ssh_anim");
    let k = render("img", "../assets/bell.json", 14, 6, &kitty(), Some(&fx.page_dir()), Some(&fx.app()));
    assert!(k.graphics.iter().any(|g| g.windows(3).any(|w| w == b"\x1b_G")), "lottie over kitty");
    let plain = render("img", "../assets/bell.json", 15, 6, &TerminalProfile::default(), Some(&fx.page_dir()), Some(&fx.app()));
    assert!(block_cells(&plain) > 5, "lottie as half blocks:\n{}", text(&plain));
    let r = render("img", "../assets/toggle.riv", 14, 3, &kitty(), Some(&fx.page_dir()), Some(&fx.app()));
    assert!(r.graphics.iter().any(|g| g.windows(3).any(|w| w == b"\x1b_G")), "rive over kitty");
}

#[test]
fn web_clients_leave_raster_media_to_the_dom_overlay() {
    let fx = Fx::new("web_img");
    let buf = render("img", "../assets/a.png", 12, 5, &web(), Some(&fx.page_dir()), Some(&fx.app()));
    assert!(blank(&buf), "the overlay shows the image; the canvas stays empty under it");
    // A missing file is still reported in the canvas.
    let missing = render("img", "../assets/nope.png", 30, 5, &web(), Some(&fx.page_dir()), Some(&fx.app()));
    assert!(text(&missing).contains("brak grafiki"));
}
