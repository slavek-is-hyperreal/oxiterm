//! Node layout rendering engine.
//!
//! Coordinates layout outputs, border styling, text formatting, and media decoding
//! to paint target DOM trees into screen cell buffers.

use crate::document::THTMLDocument;
use crate::layout::types::LayoutResult;
use crate::render::buffer::{CellBuffer, Cell};
use crate::render::build::{build_display_list, precompute_inherited_colors};
use crate::render::cell_sink::CellSink;
use crate::render::display_list::{DisplayItem, DisplayList, GridCell, RectF};
use oxiterm_proto::dom::{Node, NodeTag};
use oxiterm_proto::style::{RenderMode, TerminalProfile};
use std::path::{Path, PathBuf};
use std::collections::HashMap;

use std::sync::{OnceLock, RwLock};

static GLOBAL_VIRTUAL_FS: OnceLock<RwLock<HashMap<PathBuf, Vec<u8>>>> = OnceLock::new();

fn get_global_virtual_fs() -> &'static RwLock<HashMap<PathBuf, Vec<u8>>> {
    GLOBAL_VIRTUAL_FS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Thread-local cache proxy providing simulated file system storage for assets.
pub struct VirtualFsProxy;

impl VirtualFsProxy {
    /// The bytes of `path`, read straight from the global map (no whole-map clone).
    pub fn get(&self, path: &Path) -> Option<Vec<u8>> {
        get_global_virtual_fs().read().ok()?.get(path).cloned()
    }

    /// Is `path` mounted (no clone).
    pub fn contains(&self, path: &Path) -> bool {
        get_global_virtual_fs().read().is_ok_and(|g| g.contains_key(path))
    }

    /// Mounts `bytes` at `path` unless that would exceed `max_files` entries or `max_total` bytes
    /// in all (the replaced entry does not count). Returns whether it was mounted.
    pub fn insert_capped(&self, path: PathBuf, bytes: Vec<u8>, max_files: usize, max_total: usize) -> bool {
        let Ok(mut g) = get_global_virtual_fs().write() else { return false };
        let old = g.get(&path).map_or(0, Vec::len);
        let total: usize = g.values().map(Vec::len).sum::<usize>() - old + bytes.len();
        if (!g.contains_key(&path) && g.len() >= max_files) || total > max_total {
            return false;
        }
        g.insert(path, bytes);
        true
    }

    /// Executes a closure with access to the thread-local virtual file system.
    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&std::cell::RefCell<HashMap<PathBuf, Vec<u8>>>) -> R,
    {
        thread_local! {
            static LOCAL_FS: std::cell::RefCell<HashMap<PathBuf, Vec<u8>>> = std::cell::RefCell::new(HashMap::new());
        }

        LOCAL_FS.with(|local| {
            if let Ok(global) = get_global_virtual_fs().read() {
                *local.borrow_mut() = global.clone();
            }
        });

        let result = LOCAL_FS.with(f);

        LOCAL_FS.with(|local| {
            if let Ok(mut global) = get_global_virtual_fs().write() {
                *global = local.borrow().clone();
            }
        });

        result
    }
}

/// Global virtual file system proxy instance.
pub static VIRTUAL_FS: VirtualFsProxy = VirtualFsProxy;

/// Engine translating DOM trees and styling specs to screen characters and graphic payloads.
pub struct Renderer;

impl Renderer {
    pub fn read_asset(path: &Path) -> Result<Vec<u8>, anyhow::Error> {
        let path_str = path.to_string_lossy().into_owned();
        if let Some(bytes) = VIRTUAL_FS.get(path) {
            return Ok(bytes);
        }
        
        #[cfg(not(feature = "native-media"))]
        {
            Err(anyhow::anyhow!("AssetNotFound: {}", path_str))
        }
        #[cfg(feature = "native-media")]
        {
            // A regular file of sane size only: `std::fs::read` on a FIFO blocks forever (and
            // `SvgCache` holds its global lock across this call), a device or a huge file is no image.
            const MAX_READ: u64 = 32 * 1024 * 1024;
            match std::fs::metadata(path) {
                Ok(m) if m.is_file() && m.len() <= MAX_READ => {}
                Ok(_) => return Err(anyhow::anyhow!("AssetRefused: {}: not a regular file or too large", path_str)),
                Err(e) => return Err(anyhow::anyhow!("AssetNotFound: {}, system error: {}", path_str, e)),
            }
            std::fs::read(path).map_err(|e| anyhow::anyhow!("AssetNotFound: {}, system error: {}", path_str, e))
        }
    }

    /// Resolves a media `src` (page-relative, contained in the app base; see
    /// `oxiterm_proto::pathsafe::resolve_media`). A refused `src` is logged and draws nothing.
    pub(crate) fn resolve_src(src: &str, base_dir: Option<&Path>, app_base_dir: Option<&Path>) -> Option<PathBuf> {
        let resolved = oxiterm_proto::pathsafe::resolve_media(base_dir, app_base_dir, src);
        if resolved.is_none() {
            tracing::warn!("renderer: media src {:?} refused (outside the app base or not a plain relative path)", src);
        }
        resolved
    }

    /// The app base is the empty path: the browser's local mode, where files live in
    /// [`VIRTUAL_FS`] (or only in the page's DOM) and the real filesystem is never consulted.
    fn is_virtual_root(app_base_dir: Option<&Path>) -> bool {
        app_base_dir.is_some_and(|b| b.as_os_str().is_empty())
    }

    /// Does the asset exist: in the virtual file system, or (native media) on disk.
    pub(crate) fn asset_exists(path: &Path) -> bool {
        if VIRTUAL_FS.contains(path) {
            return true;
        }
        #[cfg(feature = "native-media")]
        {
            path.exists()
        }
        #[cfg(not(feature = "native-media"))]
        {
            false
        }
    }

    #[inline]
    fn safe_set(buffer: &mut CellBuffer, x: i32, y: i32, cell: Cell) {
        if x >= 0 && x < buffer.width as i32 && y >= 0 && y < buffer.height as i32 {
            buffer.set(x as u16, y as u16, cell);
        }
    }

    pub fn render_node(
        doc: &THTMLDocument,
        layout: &LayoutResult,
        buffer: &mut CellBuffer,
        profile: &TerminalProfile,
        base_dir: Option<&Path>,
        app_base_dir: Option<&Path>,
        scroll_offset: u16,
    ) {
        let _ = Self::render_frame(doc, layout, buffer, profile, base_dir, app_base_dir, scroll_offset);
    }

    /// Paints exactly like [`Renderer::render_node`] and returns the [`DisplayList`] that was
    /// painted, for display-list clients (OxiUI phase 3).
    ///
    /// The list is self-contained: it starts with a full-viewport `Fill` (the background that
    /// `render_node` clears to), and every `Legacy` item is replaced by a `CellGrid` holding
    /// the cells that item's legacy painter produced (its content rect, clamped to the buffer).
    /// Painting the returned list with `CellSink` onto an empty buffer of the same size
    /// reproduces `buffer`, except for `buffer.graphics`.
    pub fn render_frame(
        doc: &THTMLDocument,
        layout: &LayoutResult,
        buffer: &mut CellBuffer,
        profile: &TerminalProfile,
        base_dir: Option<&Path>,
        app_base_dir: Option<&Path>,
        scroll_offset: u16,
    ) -> DisplayList {
        Self::render_frame_mode(doc, layout, buffer, profile, base_dir, app_base_dir, scroll_offset, RenderMode::Tui)
    }

    /// [`Renderer::render_frame`] with an explicit builder mode. `RenderMode::Gui` keeps the
    /// layout's fractional geometry in the display list (the cell buffer is still painted by
    /// the cell sink, which truncates); `Tui` is the historical whole-cell behaviour.
    #[allow(clippy::too_many_arguments)]
    pub fn render_frame_mode(
        doc: &THTMLDocument,
        layout: &LayoutResult,
        buffer: &mut CellBuffer,
        profile: &TerminalProfile,
        base_dir: Option<&Path>,
        app_base_dir: Option<&Path>,
        scroll_offset: u16,
        mode: RenderMode,
    ) -> DisplayList {
        // 1. Build the display list, centering the root element if it has a smaller fixed size
        let (offset_x, offset_y) = layout.get_centering_offset(doc, buffer.width, buffer.height);
        let start_x = offset_x as i32;
        let start_y = (offset_y as i32) - (scroll_offset as i32);

        let colors = precompute_inherited_colors(doc);
        let fonts = crate::font::resolve_fonts(doc, mode);
        let built = build_display_list(doc, layout, &colors, &fonts, (start_x, start_y), (offset_x, offset_y), mode);

        // 2. Completely clear the buffer to spaces (prevents artifacts/ghosting from previous
        //    frames). Expressed as a leading Fill so the list is self-contained.
        let mut list = DisplayList::new();
        list.push(DisplayItem::Fill {
            node: doc.root,
            rect: RectF::new(0.0, 0.0, buffer.width as f32, buffer.height as f32),
            radius: 0.0,
            fg: oxiterm_proto::style::AnsiColor::Color256(15),
            bg: oxiterm_proto::style::AnsiColor::Color256(0),
        });
        list.items.extend(built.items);

        // 3. Rasterize it; tag content is still painted by the legacy per-tag cell code.
        //    Each legacy paint is captured as a CellGrid (in list order).
        let mut captured: Vec<DisplayItem> = Vec::new();
        CellSink::paint(&list, buffer, |node_id, content, buf| {
            if let Some(node) = doc.arena.get(node_id) {
                Self::paint_content(
                    node,
                    content.x as i32,
                    content.y as i32,
                    content.w as u16,
                    content.h as u16,
                    buf,
                    profile,
                    base_dir,
                    app_base_dir,
                );
            }
            captured.push(Self::capture_grid(buf, content));
        });

        let mut captured = captured.into_iter();
        for item in &mut list.items {
            if matches!(item, DisplayItem::Legacy { .. }) {
                if let Some(grid) = captured.next() {
                    *item = grid;
                }
            }
        }
        list
    }

    /// Copies `rect` (clamped to the buffer) out of `buf` as a `CellGrid`.
    fn capture_grid(buf: &CellBuffer, rect: RectF) -> DisplayItem {
        let x0 = (rect.x as i32).clamp(0, buf.width as i32);
        let y0 = (rect.y as i32).clamp(0, buf.height as i32);
        let x1 = (rect.x as i32).saturating_add(rect.w as i32).clamp(x0, buf.width as i32);
        let y1 = (rect.y as i32).saturating_add(rect.h as i32).clamp(y0, buf.height as i32);
        let mut cells = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
        for y in y0..y1 {
            for x in x0..x1 {
                let c = &buf.cells[y as usize * buf.width as usize + x as usize];
                cells.push(GridCell::from(*c));
            }
        }
        DisplayItem::CellGrid {
            rect: RectF::new(x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32),
            cells,
        }
    }

    /// Paints the legacy tag-specific content (img, video, diagram) of one node into
    /// its content rect (inside the border).
    #[allow(clippy::too_many_arguments)]
    fn paint_content(
        node: &Node,
        content_x: i32,
        content_y: i32,
        content_w: u16,
        content_h: u16,
        buffer: &mut CellBuffer,
        profile: &TerminalProfile,
        base_dir: Option<&Path>,
        app_base_dir: Option<&Path>,
    ) {
        match &node.tag {
            NodeTag::Img => {
                Self::render_img(
                    node,
                    content_x,
                    content_y,
                    content_w,
                    content_h,
                    buffer,
                    profile,
                    base_dir,
                    app_base_dir,
                );
            }
            NodeTag::Video => {
                Self::render_vid(
                    node,
                    content_x,
                    content_y,
                    content_w,
                    content_h,
                    buffer,
                    profile,
                    base_dir,
                    app_base_dir,
                );
            }
            NodeTag::Diagram => {
                Self::render_diagram(
                    node,
                    content_x,
                    content_y,
                    content_w,
                    content_h,
                    buffer,
                    base_dir,
                    app_base_dir,
                );
            }
            _ => {}
        }
    }

    fn unicode_block_fallback(
        img: &image::RgbaImage,
        abs_x: i32,
        abs_y: i32,
        width: u16,
        height: u16,
        buf: &mut CellBuffer,
    ) {
        let target_w = width as u32;
        let target_h = (height as u32) * 2;
        if target_w == 0 || target_h == 0 {
            return;
        }
        let resized = image::imageops::resize(
            img,
            target_w,
            target_h,
            image::imageops::FilterType::Nearest,
        );

        for y in 0..height {
            for x in 0..width {
                let top_px = resized.get_pixel(x as u32, 2 * y as u32);
                let bot_px = resized.get_pixel(x as u32, 2 * y as u32 + 1);

                let top_color = if top_px[3] >= 128 {
                    oxiterm_proto::style::AnsiColor::TrueColor(top_px[0], top_px[1], top_px[2])
                } else {
                    oxiterm_proto::style::AnsiColor::Reset
                };

                let bot_color = if bot_px[3] >= 128 {
                    oxiterm_proto::style::AnsiColor::TrueColor(bot_px[0], bot_px[1], bot_px[2])
                } else {
                    oxiterm_proto::style::AnsiColor::Reset
                };

                let (ch, fg, bg) = match (top_color, bot_color) {
                    (oxiterm_proto::style::AnsiColor::Reset, oxiterm_proto::style::AnsiColor::Reset) => {
                        (' ', oxiterm_proto::style::AnsiColor::Reset, oxiterm_proto::style::AnsiColor::Reset)
                    }
                    (top, oxiterm_proto::style::AnsiColor::Reset) => {
                        ('▀', top, oxiterm_proto::style::AnsiColor::Reset)
                    }
                    (oxiterm_proto::style::AnsiColor::Reset, bot) => {
                        ('▄', bot, oxiterm_proto::style::AnsiColor::Reset)
                    }
                    (top, bot) => {
                        if top == bot {
                            ('█', top, oxiterm_proto::style::AnsiColor::Reset)
                        } else {
                            ('▀', top, bot)
                        }
                    }
                };

                Self::safe_set(buf, abs_x + x as i32, abs_y + y as i32, Cell {
                    ch,
                    fg,
                    bg,
                    ..Default::default()
                });
            }
        }
    }

    #[cfg(feature = "native-media")]
    fn pixmap_to_rgba_image(pixmap: resvg::tiny_skia::Pixmap) -> image::RgbaImage {
        let mut rgba_data = pixmap.data().to_vec();
        for pixel in rgba_data.chunks_exact_mut(4) {
            // Swap blue (index 0) and red (index 2) channels to convert tiny-skia's BGRA format to standard RGBA.
            pixel.swap(0, 2);
            let alpha = pixel[3];
            if alpha > 0 && alpha < 255 {
                let a_factor = 255.0 / alpha as f32;
                pixel[0] = (pixel[0] as f32 * a_factor).min(255.0) as u8;
                pixel[1] = (pixel[1] as f32 * a_factor).min(255.0) as u8;
                pixel[2] = (pixel[2] as f32 * a_factor).min(255.0) as u8;
            }
        }
        image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(pixmap.width(), pixmap.height(), rgba_data).unwrap()
    }

    pub(crate) fn draw_missing_placeholder(
        abs_x: i32,
        abs_y: i32,
        width: u16,
        height: u16,
        label: &str,
        buffer: &mut CellBuffer,
    ) {
        if width == 0 || height == 0 {
            return;
        }

        for dy in 0..height {
            for dx in 0..width {
                let is_border = dy == 0 || dy == height - 1 || dx == 0 || dx == width - 1;
                let ch = if is_border { '*' } else { ' ' };
                Self::safe_set(buffer, abs_x + dx as i32, abs_y + dy as i32, Cell {
                    ch,
                    fg: oxiterm_proto::style::AnsiColor::TrueColor(255, 0, 0),
                    ..Default::default()
                });
            }
        }

        let full_text = format!("brak grafiki: {}", label);
        let full_len = full_text.chars().count() as u16;
        let short_len = label.chars().count() as u16;

        let (display_str, display_len) = if width >= full_len + 2 {
            (full_text.as_str(), full_len)
        } else if width >= short_len + 2 {
            (label, short_len)
        } else if width >= 3 {
            ("?", 1)
        } else {
            ("", 0)
        };

        if display_len > 0 && height >= 2 {
            let start_x = abs_x + (width as i32 - display_len as i32) / 2;
            let start_y = abs_y + height as i32 / 2;
            for (i, c) in display_str.chars().enumerate() {
                Self::safe_set(buffer, start_x + i as i32, start_y, Cell {
                    ch: c,
                    fg: oxiterm_proto::style::AnsiColor::TrueColor(255, 0, 0),
                    ..Default::default()
                });
            }
        }
    }

    fn render_img(
        node: &oxiterm_proto::dom::Node,
        abs_x: i32,
        abs_y: i32,
        width: u16,
        height: u16,
        buffer: &mut CellBuffer,
        profile: &TerminalProfile,
        base_dir: Option<&Path>,
        app_base_dir: Option<&Path>,
    ) {

        if let Some(ref src) = node.attrs.src {
            let Some(resolved_path) = Self::resolve_src(src, base_dir, app_base_dir) else {
                return;
            };
            
            let pixel_w = width as u32 * 10;
            let pixel_h = height as u32 * 20;
            if pixel_w == 0 || pixel_h == 0 {
                return;
            }

            let filename = resolved_path.file_name().and_then(|n| n.to_str()).unwrap_or("Image");
            // The browser's local mode has no file system to ask: its DOM overlay shows the file
            // (or nothing), so no placeholder is drawn there.
            if !Self::is_virtual_root(app_base_dir) && !Self::asset_exists(&resolved_path) {
                Self::draw_missing_placeholder(abs_x, abs_y, width, height, filename, buffer);
                return;
            }

            // Web clients show raster/SVG/Lottie as DOM overlays above the canvas (media layer),
            // fed by the 0x20/0x21 messages. The procedural Rive widget is the exception: it is
            // drawn here as cells (half blocks) so it works in every web mode and stays interactive.
            let is_rive_src = resolved_path.extension().is_some_and(|e| e.eq_ignore_ascii_case("riv"));
            if profile.is_web && !is_rive_src {
                return;
            }

            let is_svg = resolved_path.extension()
                .map(|ext| ext.to_string_lossy().to_lowercase() == "svg")
                .unwrap_or(false);
            let is_lottie = resolved_path.extension()
                .map(|ext| ext.to_string_lossy().to_lowercase() == "json")
                .unwrap_or(false);
            // Check for the `.riv` extension to run a CPU-rendered procedural toggle button simulation
            // rather than using a full Rive runtime, maintaining a low binary footprint.
            let is_rive = resolved_path.extension()
                .map(|ext| ext.to_string_lossy().to_lowercase() == "riv")
                .unwrap_or(false);

            // Fetch playback state if animation
            let mut frame_idx = None;
            if is_lottie {
                #[cfg(feature = "native-media")]
                {
                    let playback = crate::render::cache::PlaybackRegistry::get().get_or_create(profile.playback_scope, &resolved_path);
                    if let Some(safe_anim) = crate::render::cache::PlaybackRegistry::get().get_or_load_lottie(&resolved_path) {
                        let lock = safe_anim.lock().unwrap();
                        let total_frames = lock.anim.totalframe();
                        let fps = lock.anim.framerate();
                        if total_frames > 0 {
                            let fps = if fps > 0.0 { fps } else { 30.0 };
                            let elapsed_secs = playback.start_time.elapsed().as_secs_f64();
                            frame_idx = Some((elapsed_secs * fps) as usize % total_frames);
                        } else {
                            frame_idx = Some(0);
                        }
                    } else {
                        frame_idx = Some(0);
                    }
                }
                #[cfg(not(feature = "native-media"))]
                {
                    frame_idx = Some(0);
                }
            } else if is_rive {
                let playback = crate::render::cache::PlaybackRegistry::get().get_or_create(profile.playback_scope, &resolved_path);
                let mut bits = 0;
                if playback.hover { bits |= 1; }
                if playback.click_active { bits |= 2; }
                if playback.toggled { bits |= 4; }
                frame_idx = Some(bits);
            }

            // 1. Look up in AssetCache
            use crate::render::cache::{AssetCache, CacheKey, CacheValue, GraphicFormat};
            let cache_key = CacheKey {
                path: resolved_path.clone(),
                width_px: pixel_w,
                height_px: pixel_h,
                frame_idx,
            };

            let cached = AssetCache::get().lookup(&cache_key);

            let render_bytes = if let Some(cv) = cached {
                match cv.format {
                    GraphicFormat::Sixel(bytes) => {
                        if profile.supports_sixel {
                            Some(bytes)
                        } else {
                            None
                        }
                    }
                    GraphicFormat::Kitty(bytes) => {
                        if profile.supports_kitty_gfx {
                            Some(bytes)
                        } else {
                            None
                        }
                    }
                }
            } else {
                None
            };

            if let Some(bytes) = render_bytes {
                // Cache hit! Put sequence on screen
                if abs_x >= 0 && abs_y >= 0 {
                    let mut cmd = Vec::new();
                    cmd.extend_from_slice(format!("\x1b[{};{}H", abs_y + 1, abs_x + 1).as_bytes());
                    cmd.extend_from_slice(&bytes);
                    buffer.graphics.push(cmd);
                }
                
                for dy in 0..height {
                    for dx in 0..width {
                        let tx = abs_x + dx as i32;
                        let ty = abs_y + dy as i32;
                        if tx >= 0 && tx < buffer.width as i32 && ty >= 0 && ty < buffer.height as i32 {
                            if let Some(idx) = buffer.flat_idx(tx as u16, ty as u16) {
                                buffer.cells[idx].skip = true;
                            }
                        }
                    }
                }
                return;
            }

            // 2. Cache miss. Render from scratch
            let img_result = if is_svg {
                #[cfg(not(feature = "native-media"))]
                {
                    Err(anyhow::anyhow!("SVG rendering not supported on WASM"))
                }
                #[cfg(feature = "native-media")]
                {
                    // SVG rendering via resvg + tiny-skia
                    (|| -> anyhow::Result<image::RgbaImage> {
                        use crate::render::cache::SvgCache;
                        let tree = SvgCache::get().get_or_load(&resolved_path)?;
                        let mut pixmap = resvg::tiny_skia::Pixmap::new(pixel_w, pixel_h)
                            .ok_or_else(|| anyhow::anyhow!("Failed to create pixmap"))?;
                        
                        let size = tree.size();
                        let scale_x = pixel_w as f32 / size.width();
                        let scale_y = pixel_h as f32 / size.height();
                        let transform = resvg::tiny_skia::Transform::from_scale(scale_x, scale_y);
                        
                        resvg::render(&tree, transform, &mut pixmap.as_mut());
                        Ok(Self::pixmap_to_rgba_image(pixmap))
                    })()
                }
            } else if is_lottie {
                #[cfg(not(feature = "native-media"))]
                {
                    Err(anyhow::anyhow!("Lottie rendering not supported on WASM"))
                }
                #[cfg(feature = "native-media")]
                {
                    // Native Lottie Rendering using rlottie
                    (|| -> anyhow::Result<image::RgbaImage> {
                        if let Some(safe_anim) = crate::render::cache::PlaybackRegistry::get().get_or_load_lottie(&resolved_path) {
                            let mut lock = safe_anim.lock().unwrap();
                            let size = rlottie::Size {
                                width: pixel_w as usize,
                                height: pixel_h as usize,
                            };
                            let mut surface = rlottie::Surface::new(size);
                            
                            let total_frames = lock.anim.totalframe();
                            let frame = frame_idx.unwrap_or(0);
                            let frame_to_render = if total_frames > 0 { frame % total_frames } else { 0 };
                            
                            lock.anim.render(frame_to_render, &mut surface);
                            
                            Ok(unpremultiply_bgra_to_rgba(surface.data(), pixel_w, pixel_h))
                        } else {
                            Err(anyhow::anyhow!("Lottie animation not loaded"))
                        }
                    })()
                }
            } else if is_rive {
                // Procedural Rive toggle (pure Rust, see `render::rive`). Web clients get it as half
                // blocks, so it is rasterized at 1 x 2 pixels per cell and the fallback maps it 1:1.
                let bits = frame_idx.unwrap_or(0);
                if profile.is_web {
                    // Cached per (size, state) and capped (F-13): not re-rasterised on every frame.
                    Ok((*super::rive::toggle_half_block_image(u32::from(width), u32::from(height), bits)).clone())
                } else {
                    Ok(super::rive::toggle_image_capped(pixel_w, pixel_h, bits))
                }
            } else {
                // Standard image rendering
                Self::read_asset(&resolved_path)
                    .and_then(|bytes| {
                        image::ImageReader::new(std::io::Cursor::new(bytes))
                            .with_guessed_format()
                            .map_err(|e| anyhow::anyhow!("{}", e))
                    })
                    .and_then(|r| r.decode().map_err(|e| anyhow::anyhow!("{}", e)))
                    .map(|img| {
                        let rgba = img.to_rgba8();
                        image::imageops::resize(&rgba, pixel_w, pixel_h, image::imageops::FilterType::Triangle)
                    })
            };

            match img_result {
                Ok(img) => {
                    if profile.is_web {
                        Self::unicode_block_fallback(&img, abs_x, abs_y, width, height, buffer);
                    } else if profile.supports_kitty_gfx {
                        let payload = super::kitty::KittyImageManager::transmit_image(
                            pixel_w,
                            pixel_h,
                            width as u32,
                            height as u32,
                            &img,
                        );
                        
                        // Insert into cache
                        AssetCache::get().insert(
                            cache_key,
                            CacheValue {
                                format: GraphicFormat::Kitty(payload.clone()),
                            },
                        );

                        if abs_x >= 0 && abs_y >= 0 {
                            let mut cmd = Vec::new();
                            cmd.extend_from_slice(format!("\x1b[{};{}H", abs_y + 1, abs_x + 1).as_bytes());
                            cmd.extend_from_slice(&payload);
                            buffer.graphics.push(cmd);
                        }
                        
                        for dy in 0..height {
                            for dx in 0..width {
                                let tx = abs_x + dx as i32;
                                let ty = abs_y + dy as i32;
                                if tx >= 0 && tx < buffer.width as i32 && ty >= 0 && ty < buffer.height as i32 {
                                    if let Some(idx) = buffer.flat_idx(tx as u16, ty as u16) {
                                        buffer.cells[idx].skip = true;
                                    }
                                }
                            }
                        }
                    } else if profile.supports_sixel {
                        let sixel_payload = super::sixel::SixelCodec::encode_sixel_static(&img);
                        
                        // Insert into cache
                        AssetCache::get().insert(
                            cache_key,
                            CacheValue {
                                format: GraphicFormat::Sixel(sixel_payload.clone()),
                            },
                        );

                        if abs_x >= 0 && abs_y >= 0 {
                            let mut cmd = Vec::new();
                            cmd.extend_from_slice(format!("\x1b[{};{}H", abs_y + 1, abs_x + 1).as_bytes());
                            cmd.extend_from_slice(&sixel_payload);
                            buffer.graphics.push(cmd);
                        }
                        
                        for dy in 0..height {
                            for dx in 0..width {
                                let tx = abs_x + dx as i32;
                                let ty = abs_y + dy as i32;
                                if tx >= 0 && tx < buffer.width as i32 && ty >= 0 && ty < buffer.height as i32 {
                                    if let Some(idx) = buffer.flat_idx(tx as u16, ty as u16) {
                                        buffer.cells[idx].skip = true;
                                    }
                                }
                            }
                        }
                    } else {
                        Self::unicode_block_fallback(&img, abs_x, abs_y, width, height, buffer);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to render image from {:?}: {}", resolved_path, e);
                    let name = resolved_path.file_name().and_then(|n| n.to_str()).unwrap_or("Image");
                    Self::draw_missing_placeholder(abs_x, abs_y, width, height, name, buffer);
                }
            }
        }
    }

    fn render_vid(
        node: &oxiterm_proto::dom::Node,
        abs_x: i32,
        abs_y: i32,
        width: u16,
        height: u16,
        buffer: &mut CellBuffer,
        profile: &TerminalProfile,
        base_dir: Option<&Path>,
        app_base_dir: Option<&Path>,
    ) {
        if let Some(ref src) = node.attrs.src {
            let Some(resolved_path) = Self::resolve_src(src, base_dir, app_base_dir) else {
                return;
            };

            let pixel_w = width as u32 * 10;
            let pixel_h = height as u32 * 20;
            if pixel_w == 0 || pixel_h == 0 {
                return;
            }

            let filename = resolved_path.file_name().and_then(|n| n.to_str()).unwrap_or("Video");
            if !Self::is_virtual_root(app_base_dir) && !Self::asset_exists(&resolved_path) {
                Self::draw_missing_placeholder(abs_x, abs_y, width, height, filename, buffer);
                return;
            }

            if profile.is_web {
                return;
            }

            #[cfg(feature = "native-media")]
            let fps = if profile.supports_kitty_gfx {
                30
            } else if profile.supports_sixel {
                10
            } else {
                2
            };

            #[cfg(feature = "native-media")]
            let frame: Option<std::sync::Arc<Vec<u8>>> =
                crate::render::cache::VideoPlayerRegistry::get().get_frame(&resolved_path, pixel_w, pixel_h, fps);

            #[cfg(feature = "native-media")]
            if let Some(raw_rgba) = frame {
                if profile.supports_kitty_gfx {
                    // Zero-copy transmit path. Pass Arc slice reference directly to transmit_image to avoid allocation.
                    let payload = super::kitty::KittyImageManager::transmit_image(
                        pixel_w,
                        pixel_h,
                        width as u32,
                        height as u32,
                        &raw_rgba,
                    );
                    if abs_x >= 0 && abs_y >= 0 {
                        let mut cmd = Vec::new();
                        cmd.extend_from_slice(format!("\x1b[{};{}H", abs_y + 1, abs_x + 1).as_bytes());
                        cmd.extend_from_slice(&payload);
                        buffer.graphics.push(cmd);
                    }
                    
                    for dy in 0..height {
                        for dx in 0..width {
                            let tx = abs_x + dx as i32;
                            let ty = abs_y + dy as i32;
                            if tx >= 0 && tx < buffer.width as i32 && ty >= 0 && ty < buffer.height as i32 {
                                if let Some(idx) = buffer.flat_idx(tx as u16, ty as u16) {
                                    buffer.cells[idx].skip = true;
                                }
                            }
                        }
                    }
                    return;
                } else {
                    // For fallback formats, construct the ImageBuffer by cloning raw_rgba.
                    if let Some(img) = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(pixel_w, pixel_h, (*raw_rgba).clone()) {
                        if profile.supports_sixel {
                            let sixel_payload = super::sixel::SixelCodec::encode_sixel_static(&img);
                            if abs_x >= 0 && abs_y >= 0 {
                                let mut cmd = Vec::new();
                                cmd.extend_from_slice(format!("\x1b[{};{}H", abs_y + 1, abs_x + 1).as_bytes());
                                cmd.extend_from_slice(&sixel_payload);
                                buffer.graphics.push(cmd);
                            }
                            
                            for dy in 0..height {
                                for dx in 0..width {
                                    let tx = abs_x + dx as i32;
                                    let ty = abs_y + dy as i32;
                                    if tx >= 0 && tx < buffer.width as i32 && ty >= 0 && ty < buffer.height as i32 {
                                        if let Some(idx) = buffer.flat_idx(tx as u16, ty as u16) {
                                            buffer.cells[idx].skip = true;
                                        }
                                    }
                                }
                            }
                        } else {
                            Self::unicode_block_fallback(&img, abs_x, abs_y, width, height, buffer);
                        }
                        return;
                    }
                }
            }
            Self::draw_missing_placeholder(abs_x, abs_y, width, height, filename, buffer);
        }
    }

    fn render_diagram(
        node: &oxiterm_proto::dom::Node,
        abs_x: i32,
        abs_y: i32,
        width: u16,
        height: u16,
        buffer: &mut CellBuffer,
        base_dir: Option<&Path>,
        app_base_dir: Option<&Path>,
    ) {
        if width == 0 || height == 0 {
            return;
        }

        if let Some(ref src) = node.attrs.src {
            let Some(resolved_path) = Self::resolve_src(src, base_dir, app_base_dir) else {
                return;
            };

            let filename = resolved_path.file_name().and_then(|n| n.to_str()).unwrap_or("Diagram");
            if !Self::asset_exists(&resolved_path) {
                // Local mode: the page fetches the file after the first frame (see `LocalSession`),
                // so for now there is nothing to say; the next frame has it.
                if !Self::is_virtual_root(app_base_dir) {
                    Self::draw_missing_placeholder(abs_x, abs_y, width, height, filename, buffer);
                }
                return;
            }

            let content = match Self::read_asset(&resolved_path) {
                Ok(b) => String::from_utf8_lossy(&b).to_string(),
                Err(_) => return,
            };

            let graph = match crate::diagram::parse_mermaid(&content) {
                Ok(g) => g,
                Err(_) => return,
            };

            let layout = crate::diagram::compute_layout(&graph);
            let full_grid = crate::diagram::CellGrid::render_grid(&graph, &layout);

            let target_cols = width as usize;
            let target_rows = height as usize;
            let fits_in_box = full_grid.cols <= target_cols && full_grid.rows <= target_rows.saturating_sub(1);

            let alt = node.attrs.alt.as_deref().unwrap_or("Diagram");
            let header_str = if fits_in_box {
                format!("{} | {} nodes, {} edges | ({}x{})", alt, graph.nodes.len(), graph.edges.len(), full_grid.cols, full_grid.rows)
            } else {
                format!("{} | {} nodes, {} edges | ({}x{}) [Enter]", alt, graph.nodes.len(), graph.edges.len(), full_grid.cols, full_grid.rows)
            };

            let fg = oxiterm_proto::style::AnsiColor::Color256(15);
            for (i, ch) in header_str.chars().enumerate() {
                if (i as u16) < width {
                    Self::safe_set(buffer, abs_x + i as i32, abs_y, Cell { ch, fg, ..Default::default() });
                }
            }

            let body_rows = target_rows.saturating_sub(1);
            if body_rows == 0 {
                return;
            }

            let preview_mode = node.attrs.preview.as_deref().unwrap_or("minimap");
            let body_grid = if fits_in_box {
                full_grid
            } else {
                match preview_mode {
                    "crop" => full_grid.crop(target_cols, body_rows, node.attrs.preview_anchor.as_deref(), &graph, &layout),
                    _ => full_grid.minimap(target_cols, body_rows),
                }
            };

            for r in 0..body_grid.rows {
                for c in 0..body_grid.cols {
                    let ch = body_grid.get(c, r);
                    Self::safe_set(buffer, abs_x + c as i32, abs_y + 1 + r as i32, Cell { ch, fg, ..Default::default() });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::engine::LayoutEngine;
    use oxiterm_proto::dom::{Node, NodeTag};
    use oxiterm_proto::style::{AnsiColor, BorderStyle, BorderChars};

    #[test]
    fn test_t22_diagram_src_outside_base_dir_draws_nothing() {
        let mut buffer = CellBuffer::new(40, 20);
        let mut node = Node::new(NodeTag::Diagram);
        node.attrs.src = Some("../secret.mmd".to_string());
        node.attrs.alt = Some("Secret".to_string());

        let base_dir = Path::new("/app/examples");
        let app_base_dir = Path::new("/app/examples");

        Renderer::render_diagram(&node, 0, 0, 40, 20, &mut buffer, Some(base_dir), Some(app_base_dir));

        let non_empty = buffer.cells.iter().filter(|c| c.ch != ' ').count();
        assert_eq!(non_empty, 0, "Diagram with path traversal outside app_base_dir must draw nothing");
    }

    #[test]
    fn test_border_and_transparency_rendering() {
        let mut doc = THTMLDocument::new();
        
        let mut parent = Node::new(NodeTag::Box);
        parent.style.width = Some(5.0);
        parent.style.height = Some(3.0);
        parent.style.bg = AnsiColor::TrueColor(10, 20, 30);
        parent.style.border = Some(BorderStyle {
            fg: AnsiColor::TrueColor(100, 100, 100),
            chars: BorderChars::default(),
        });
        
        let mut child = Node::new(NodeTag::Text);
        child.text = Some("A".to_string());
        child.style.fg = AnsiColor::Reset;
        child.style.width = Some(1.0);
        child.style.height = Some(1.0);
        
        let parent_id = doc.arena.alloc(parent);
        let child_id = doc.arena.alloc(child);
        doc.append_child(parent_id, child_id).unwrap();
        doc.append_child(doc.root, parent_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 5, 3, None).unwrap();

        let mut buffer = CellBuffer::new(5, 3);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None, None, 0);

        // Check top-left corner border character '┌'
        let tl_idx = buffer.flat_idx(0, 0).unwrap();
        let tl_cell = &buffer.cells[tl_idx];
        assert_eq!(tl_cell.ch, '┌');
        assert_eq!(tl_cell.fg, AnsiColor::TrueColor(100, 100, 100));
        assert_eq!(tl_cell.bg, AnsiColor::TrueColor(10, 20, 30));

        // Check offset child text character 'A' at (1, 1) inside border box
        let content_idx = buffer.flat_idx(1, 1).unwrap();
        let content_cell = &buffer.cells[content_idx];
        assert_eq!(content_cell.ch, 'A');
        assert_eq!(content_cell.bg, AnsiColor::TrueColor(10, 20, 30)); // Inherited bg
    }

    #[test]
    fn test_wide_character_rendering() {
        let mut doc = THTMLDocument::new();
        
        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("🚀A".to_string()); // Rocket (width 2) + A (width 1)
        text_node.style.width = Some(5.0);
        text_node.style.height = Some(1.0);
        text_node.style.bg = AnsiColor::TrueColor(5, 5, 5);
        
        let node_id = doc.arena.alloc(text_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 5, 1, None).unwrap();

        let mut buffer = CellBuffer::new(5, 1);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None, None, 0);

        // Check index 0: contains '🚀'
        assert_eq!(buffer.cells[0].ch, '🚀');
        assert_eq!(buffer.cells[0].bg, AnsiColor::TrueColor(5, 5, 5));

        // Check index 1: contains ' ' (continuation cell filled with styled space)
        assert_eq!(buffer.cells[1].ch, ' ');
        assert_eq!(buffer.cells[1].bg, AnsiColor::TrueColor(5, 5, 5));

        // Check index 2: contains 'A' (advanced correctly by 2 cells)
        assert_eq!(buffer.cells[2].ch, 'A');
        assert_eq!(buffer.cells[2].bg, AnsiColor::TrueColor(5, 5, 5));
    }

    #[test]
    fn test_image_fallback_rendering() {
        let mut doc = THTMLDocument::new();
        
        let mut img_node = Node::new(NodeTag::Img);
        img_node.attrs.src = Some("nonexistent_test_image.png".to_string());
        img_node.style.width = Some(6.0);
        img_node.style.height = Some(4.0);
        
        let node_id = doc.arena.alloc(img_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 6, 4, None).unwrap();

        let mut buffer = CellBuffer::new(6, 4);
        let base = std::path::Path::new(".");
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None, Some(base), 0);

        // Since the file is non-existent, it should render the fallback border of '*'
        assert_eq!(buffer.cells[0].ch, '*');
        assert_eq!(buffer.cells[5].ch, '*');
    }

    #[test]
    fn test_video_fallback_rendering() {
        let mut doc = THTMLDocument::new();
        
        let mut vid_node = Node::new(NodeTag::Video);
        vid_node.attrs.src = Some("nonexistent_video.mp4".to_string());
        vid_node.style.width = Some(15.0);
        vid_node.style.height = Some(5.0);
        
        let node_id = doc.arena.alloc(vid_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 15, 5, None).unwrap();

        let mut buffer = CellBuffer::new(15, 5);
        let base = std::path::Path::new(".");
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None, Some(base), 0);

        // Falling back should draw border of '*'
        assert_eq!(buffer.cells[0].ch, '*');
        assert_eq!(buffer.cells[14].ch, '*');
    }

    #[test]
    fn test_contract_9_placeholder_full_label() {
        let mut buffer = CellBuffer::new(30, 5);
        Renderer::draw_missing_placeholder(0, 0, 30, 5, "mascot.svg", &mut buffer);
        let row2: String = (0..30).map(|x| buffer.cells[2 * 30 + x].ch).collect();
        assert!(row2.contains("brak grafiki: mascot.svg"));
    }

    #[test]
    fn test_contract_10_placeholder_question_mark_for_small_width() {
        let mut buffer = CellBuffer::new(6, 4);
        Renderer::draw_missing_placeholder(0, 0, 6, 4, "mascot.svg", &mut buffer);
        let row2: String = (0..6).map(|x| buffer.cells[2 * 6 + x].ch).collect();
        assert!(row2.contains('?'));
        assert!(!row2.contains("mascot.svg"));
    }

    #[test]
    fn test_contract_11_placeholder_border_only_for_tiny_width() {
        let mut buffer = CellBuffer::new(2, 2);
        Renderer::draw_missing_placeholder(0, 0, 2, 2, "mascot.svg", &mut buffer);
        assert_eq!(buffer.cells[0].ch, '*');
        assert_eq!(buffer.cells[1].ch, '*');
        assert_eq!(buffer.cells[2].ch, '*');
        assert_eq!(buffer.cells[3].ch, '*');
    }

    #[test]
    fn test_contract_12_video_placeholder_adaptive_label() {
        let mut buffer = CellBuffer::new(20, 5);
        Renderer::draw_missing_placeholder(0, 0, 20, 5, "clip.mp4", &mut buffer);
        let row2: String = (0..20).map(|x| buffer.cells[2 * 20 + x].ch).collect();
        assert!(row2.contains("clip.mp4"));
    }

    #[test]
    fn test_contract_13_web_mode_missing_file_renders_placeholder() {
        let mut doc = THTMLDocument::new();
        let mut img_node = Node::new(NodeTag::Img);
        img_node.attrs.src = Some("missing_web_image.png".to_string());
        img_node.style.width = Some(40.0);
        img_node.style.height = Some(5.0);
        let node_id = doc.arena.alloc(img_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 40, 5, None).unwrap();
        let mut buffer = CellBuffer::new(40, 5);
        let mut profile = TerminalProfile::default();
        profile.is_web = true;
        let base = std::path::Path::new(".");
        Renderer::render_node(&doc, &layout, &mut buffer, &profile, None, Some(base), 0);

        assert_eq!(buffer.cells[0].ch, '*');
        let row2: String = (0..40).map(|x| buffer.cells[2 * 40 + x].ch).collect();
        assert!(row2.contains("brak grafiki: missing_web_image.png"));
    }

    #[test]
    fn test_contract_14_web_mode_existing_file_skips_pixel_decoding() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_web_existing.png");
        std::fs::write(&test_file, b"FAKE_PNG_DATA").unwrap();

        let mut doc = THTMLDocument::new();
        let mut img_node = Node::new(NodeTag::Img);
        img_node.attrs.src = Some("test_web_existing.png".to_string());
        img_node.style.width = Some(30.0);
        img_node.style.height = Some(5.0);
        let node_id = doc.arena.alloc(img_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 30, 5, None).unwrap();
        let mut buffer = CellBuffer::new(30, 5);
        let mut profile = TerminalProfile::default();
        profile.is_web = true;
        Renderer::render_node(&doc, &layout, &mut buffer, &profile, Some(&temp_dir), Some(&temp_dir), 0);

        // Should not render red border '*' for existing file on web
        assert_ne!(buffer.cells[0].ch, '*');

        let _ = std::fs::remove_file(test_file);
    }

    #[test]
    fn test_29_wrap_word_renders_rows() {
        let mut doc = THTMLDocument::new();
        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("aa bb cc".to_string());
        text_node.style.wrap = oxiterm_proto::style::WrapMode::Word;
        text_node.style.width = Some(5.0);
        
        let node_id = doc.arena.alloc(text_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 5, 2, None).unwrap();

        let mut buffer = CellBuffer::new(5, 2);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None, None, 0);

        assert_eq!(buffer.cells[0].ch, 'a');
        assert_eq!(buffer.cells[1].ch, 'a');
        assert_eq!(buffer.cells[2].ch, ' ');
        assert_eq!(buffer.cells[3].ch, 'b');
        assert_eq!(buffer.cells[4].ch, 'b');

        assert_eq!(buffer.cells[5].ch, 'c');
        assert_eq!(buffer.cells[6].ch, 'c');
    }

    #[test]
    fn test_31_password_input_masks() {
        let mut doc = THTMLDocument::new();
        let mut input_node = Node::new(NodeTag::Input);
        input_node.attrs.input_type = Some("password".to_string());
        input_node.text = Some("secret".to_string());
        input_node.style.width = Some(10.0);
        input_node.style.height = Some(1.0);
        
        let node_id = doc.arena.alloc(input_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 10, 1, None).unwrap();

        let mut buffer = CellBuffer::new(10, 1);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None, None, 0);

        for i in 0..6 {
            assert_eq!(buffer.cells[i].ch, '*');
        }
        for i in 6..10 {
            assert_eq!(buffer.cells[i].ch, '_');
        }
    }

    #[test]
    fn test_button_text_rendering() {
        use oxiterm_proto::style::{AnsiColor, BorderChars, BorderStyle};
        let mut doc = THTMLDocument::new();
        let mut btn = Node::new(NodeTag::Button);
        btn.text = Some("Click Me".to_string());
        btn.style.border = Some(BorderStyle {
            fg: AnsiColor::Reset,
            chars: BorderChars::single(),
        });
        btn.style.padding.left = 1.0;
        btn.style.padding.right = 1.0;
        
        let node_id = doc.arena.alloc(btn);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        // text 8 + border 2 + padding 2 = 12 columns wide, 3 rows high
        let layout = engine.compute(&mut doc, 20, 5, None).unwrap();
        assert_eq!(layout.nodes[&node_id].width, 12.0);
        assert_eq!(layout.nodes[&node_id].height, 3.0);

        let mut buffer = CellBuffer::new(20, 5);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None, None, 0);

        let row2: String = (0..20).map(|x| buffer.cells[2 * 20 + x].ch).collect();
        assert!(row2.contains("Click Me"));
    }
}

#[cfg(feature = "native-media")]
pub fn unpremultiply_bgra_to_rgba(data: &[rlottie::Bgra], width: u32, height: u32) -> image::RgbaImage {
    let mut rgba_data = Vec::with_capacity(data.len() * 4);
    for pixel in data {
        let a = pixel.a;
        if a == 0 {
            rgba_data.push(0);
            rgba_data.push(0);
            rgba_data.push(0);
            rgba_data.push(0);
        } else {
            let r = ((pixel.r as u32 * 255 / a as u32).min(255)) as u8;
            let g = ((pixel.g as u32 * 255 / a as u32).min(255)) as u8;
            let b = ((pixel.b as u32 * 255 / a as u32).min(255)) as u8;
            rgba_data.push(r);
            rgba_data.push(g);
            rgba_data.push(b);
            rgba_data.push(a);
        }
    }
    image::ImageBuffer::from_raw(width, height, rgba_data).unwrap()
}

