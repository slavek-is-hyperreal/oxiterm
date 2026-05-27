//! Node layout rendering engine.
//!
//! Coordinates layout outputs, border styling, text formatting, and media decoding
//! to paint target DOM trees into screen cell buffers.

use crate::document::THTMLDocument;
use crate::layout::types::LayoutResult;
use crate::render::buffer::{CellBuffer, Cell};
use oxiterm_proto::dom::{NodeTag, NodeId};
use oxiterm_proto::style::TerminalProfile;
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
        let local_bytes = VIRTUAL_FS.with(|fs| {
            fs.borrow().get(path).cloned()
        });
        if let Some(bytes) = local_bytes {
            return Ok(bytes);
        }
        
        #[cfg(target_arch = "wasm32")]
        {
            Err(anyhow::anyhow!("AssetNotFound: {}", path_str))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::fs::read(path).map_err(|e| anyhow::anyhow!("AssetNotFound: {}, system error: {}", path_str, e))
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
        scroll_offset: u16,
    ) {
        // 1. Completely clear the buffer to spaces (prevents artifacts/ghosting from previous frames)
        for y in 0..buffer.height {
            for x in 0..buffer.width {
                buffer.set(x, y, Cell {
                    ch: ' ',
                    fg: oxiterm_proto::style::AnsiColor::Color256(15),
                    bg: oxiterm_proto::style::AnsiColor::Color256(0),
                    ..Default::default()
                });
            }
        }
        
        // 2. Recursively draw the DOM tree, centering the root element if it has a smaller fixed size
        let (offset_x, offset_y) = layout.get_centering_offset(doc, buffer.width, buffer.height);
        let start_x = offset_x as i32;
        let start_y = (offset_y as i32) - (scroll_offset as i32);

        Self::render_recursive(
            doc,
            layout,
            buffer,
            doc.root,
            start_x,
            start_y,
            oxiterm_proto::style::AnsiColor::Color256(15),
            oxiterm_proto::style::AnsiColor::Color256(0),
            profile,
            base_dir,
        );
    }

    fn render_recursive(
        doc: &THTMLDocument,
        layout: &LayoutResult,
        buffer: &mut CellBuffer,
        node_id: NodeId,
        parent_x: i32,
        parent_y: i32,
        inherited_fg: oxiterm_proto::style::AnsiColor,
        inherited_bg: oxiterm_proto::style::AnsiColor,
        profile: &TerminalProfile,
        base_dir: Option<&Path>,
    ) {
        if let Some(node) = doc.arena.get(node_id) {
            let rect = layout.nodes.get(&node_id).copied().unwrap_or_default();
            let abs_x = parent_x + rect.x as i32;
            let abs_y = parent_y + rect.y as i32;

            let resolved_fg = match node.style.fg {
                oxiterm_proto::style::AnsiColor::Reset => inherited_fg,
                c => c,
            };
            let resolved_bg = match node.style.bg {
                oxiterm_proto::style::AnsiColor::Reset => inherited_bg,
                c => c,
            };

            // Draw background
            for y in 0..rect.height {
                for x in 0..rect.width {
                    Self::safe_set(buffer, abs_x + x as i32, abs_y + y as i32, Cell {
                        ch: ' ',
                        fg: resolved_fg,
                        bg: resolved_bg,
                        ..Default::default()
                    });
                }
            }

            // Draw border if defined
            if let Some(border) = &node.style.border {
                let border_fg = match border.fg {
                    oxiterm_proto::style::AnsiColor::Reset => resolved_fg,
                    c => c,
                };
                
                if rect.width > 0 && rect.height > 0 {
                    // Corners
                    Self::safe_set(buffer, abs_x, abs_y, Cell {
                        ch: border.chars.top_left,
                        fg: border_fg,
                        bg: resolved_bg,
                        ..Default::default()
                    });
                    if rect.width > 1 {
                        Self::safe_set(buffer, abs_x + rect.width as i32 - 1, abs_y, Cell {
                            ch: border.chars.top_right,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                    }
                    if rect.height > 1 {
                        Self::safe_set(buffer, abs_x, abs_y + rect.height as i32 - 1, Cell {
                            ch: border.chars.bot_left,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                    }
                    if rect.width > 1 && rect.height > 1 {
                        Self::safe_set(buffer, abs_x + rect.width as i32 - 1, abs_y + rect.height as i32 - 1, Cell {
                            ch: border.chars.bot_right,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                    }

                    // Horizontal borders
                    for x in 1..rect.width.saturating_sub(1) {
                        Self::safe_set(buffer, abs_x + x as i32, abs_y, Cell {
                            ch: border.chars.top,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                        if rect.height > 1 {
                            Self::safe_set(buffer, abs_x + x as i32, abs_y + rect.height as i32 - 1, Cell {
                                ch: border.chars.bot,
                                fg: border_fg,
                                bg: resolved_bg,
                                ..Default::default()
                            });
                        }
                    }

                    // Vertical borders
                    for y in 1..rect.height.saturating_sub(1) {
                        Self::safe_set(buffer, abs_x, abs_y + y as i32, Cell {
                            ch: border.chars.left,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                        if rect.width > 1 {
                            Self::safe_set(buffer, abs_x + rect.width as i32 - 1, abs_y + y as i32, Cell {
                                ch: border.chars.right,
                                fg: border_fg,
                                bg: resolved_bg,
                                ..Default::default()
                            });
                        }
                    }
                }
            }

            let has_border = node.style.border.is_some();
            let content_x = if has_border { abs_x + 1 } else { abs_x };
            let content_y = if has_border { abs_y + 1 } else { abs_y };
            let content_w = if has_border { rect.width.saturating_sub(2) } else { rect.width };
            let content_h = if has_border { rect.height.saturating_sub(2) } else { rect.height };

            match &node.tag {
                NodeTag::Text => {
                    if let Some(text) = &node.text {
                        let mut cx = 0;
                        let mut cy = 0;
                        for ch in text.chars() {
                            if ch == '\n' {
                                cx = 0;
                                cy += 1;
                            } else {
                                let char_w = crate::render::unicode::UnicodeWidthCache::get().width(ch) as u16;
                                if char_w > 0 {
                                    if char_w > 1 && cx + char_w > content_w && char_w <= content_w {
                                        cx = 0;
                                        cy += 1;
                                    }
                                    if cx < content_w && cy < content_h {
                                        Self::safe_set(buffer, content_x + cx as i32, content_y + cy as i32, Cell {
                                            ch,
                                            fg: resolved_fg,
                                            bg: resolved_bg,
                                            ..Default::default()
                                        });
                                        // Fill continuation cells with styled spaces
                                        for i in 1..char_w {
                                            if cx + i < content_w {
                                                Self::safe_set(buffer, content_x + (cx + i) as i32, content_y + cy as i32, Cell {
                                                    ch: ' ',
                                                    fg: resolved_fg,
                                                    bg: resolved_bg,
                                                    ..Default::default()
                                                });
                                            }
                                        }
                                    }
                                    cx += char_w;
                                }
                            }
                        }
                    }
                }
                NodeTag::Input => {
                    for x in 0..content_w {
                        Self::safe_set(buffer, content_x + x as i32, content_y, Cell {
                            ch: '_',
                            fg: resolved_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                    }
                }
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
                    );
                }
                _ => {}
            }

            for &child_id in &node.children {
                Self::render_recursive(
                    doc,
                    layout,
                    buffer,
                    child_id,
                    parent_x,
                    parent_y,
                    resolved_fg,
                    resolved_bg,
                    profile,
                    base_dir,
                );
            }
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

    #[cfg(not(target_arch = "wasm32"))]
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

    fn render_img(
        node: &oxiterm_proto::dom::Node,
        abs_x: i32,
        abs_y: i32,
        width: u16,
        height: u16,
        buffer: &mut CellBuffer,
        profile: &TerminalProfile,
        base_dir: Option<&Path>,
    ) {
        if profile.is_web {
            return;
        }

        if let Some(ref src) = node.attrs.src {
            let resolved_path = if let Some(base) = base_dir {
                base.join(src)
            } else {
                std::path::PathBuf::from(src)
            };
            
            let pixel_w = width as u32 * 10;
            let pixel_h = height as u32 * 20;
            if pixel_w == 0 || pixel_h == 0 {
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
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let playback = crate::render::cache::PlaybackRegistry::get().get_or_create(&resolved_path);
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
                #[cfg(target_arch = "wasm32")]
                {
                    frame_idx = Some(0);
                }
            } else if is_rive {
                let playback = crate::render::cache::PlaybackRegistry::get().get_or_create(&resolved_path);
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
                #[cfg(target_arch = "wasm32")]
                {
                    Err(anyhow::anyhow!("SVG rendering not supported on WASM"))
                }
                #[cfg(not(target_arch = "wasm32"))]
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
                #[cfg(target_arch = "wasm32")]
                {
                    Err(anyhow::anyhow!("Lottie rendering not supported on WASM"))
                }
                #[cfg(not(target_arch = "wasm32"))]
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
                            
                            let mut rgba_data = Vec::with_capacity(surface.data().len() * 4);
                            for pixel in surface.data() {
                                rgba_data.push(pixel.r);
                                rgba_data.push(pixel.g);
                                rgba_data.push(pixel.b);
                                rgba_data.push(pixel.a);
                            }
                            
                            image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(pixel_w, pixel_h, rgba_data)
                                .ok_or_else(|| anyhow::anyhow!("Failed to construct RgbaImage from lottie surface"))
                        } else {
                            Err(anyhow::anyhow!("Lottie animation not loaded"))
                        }
                    })()
                }
            } else if is_rive {
                #[cfg(target_arch = "wasm32")]
                {
                    Err(anyhow::anyhow!("Rive rendering not supported on WASM"))
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    // Procedural Rive Toggle Button widget simulation.
                    // We run interactive state updates on the CPU to avoid heavyweight web/GPU dependencies.
                    (|| -> anyhow::Result<image::RgbaImage> {
                        let mut pixmap = resvg::tiny_skia::Pixmap::new(pixel_w, pixel_h)
                            .ok_or_else(|| anyhow::anyhow!("Failed to create pixmap"))?;
                        
                        let bits = frame_idx.unwrap_or(0);
                        let hover = (bits & 1) != 0;
                        let click_active = (bits & 2) != 0;
                        let toggled = (bits & 4) != 0;
                        
                        let pad = 6.0;
                        let rect_w = pixel_w as f32 - pad * 2.0;
                        let rect_h = pixel_h as f32 - pad * 2.0;
                        
                        let rect = resvg::tiny_skia::Rect::from_xywh(pad, pad, rect_w, rect_h)
                            .ok_or_else(|| anyhow::anyhow!("Failed to create Rect"))?;
                        let mut pb = resvg::tiny_skia::PathBuilder::new();
                        pb.push_rect(rect);
                        let bg_path = pb.finish().ok_or_else(|| anyhow::anyhow!("Failed to finish path"))?;
                        
                        // Track background
                        let mut bg_paint = resvg::tiny_skia::Paint::default();
                        bg_paint.anti_alias = true;
                        
                        let bg_color = if toggled {
                            resvg::tiny_skia::Color::from_rgba8(0, 120, 255, 255)
                        } else if hover {
                            resvg::tiny_skia::Color::from_rgba8(70, 70, 70, 255)
                        } else {
                            resvg::tiny_skia::Color::from_rgba8(35, 35, 35, 255)
                        };
                        bg_paint.set_color(bg_color);
                        
                        pixmap.fill_path(&bg_path, &bg_paint, resvg::tiny_skia::FillRule::Winding, resvg::tiny_skia::Transform::identity(), None);
                        
                        // Outline border
                        let mut border_paint = resvg::tiny_skia::Paint::default();
                        border_paint.anti_alias = true;
                        
                        let border_color = if click_active {
                            resvg::tiny_skia::Color::from_rgba8(0, 255, 255, 255)
                        } else if hover {
                            resvg::tiny_skia::Color::from_rgba8(220, 220, 220, 255)
                        } else {
                            resvg::tiny_skia::Color::from_rgba8(90, 90, 90, 255)
                        };
                        border_paint.set_color(border_color);
                        
                        let mut stroke = resvg::tiny_skia::Stroke::default();
                        stroke.width = 3.0;
                        
                        pixmap.stroke_path(&bg_path, &border_paint, &stroke, resvg::tiny_skia::Transform::identity(), None);

                        // Sliding knob
                        let mut knob_paint = resvg::tiny_skia::Paint::default();
                        knob_paint.anti_alias = true;
                        
                        let knob_color = if click_active {
                            resvg::tiny_skia::Color::from_rgba8(0, 255, 255, 255)
                        } else {
                            resvg::tiny_skia::Color::from_rgba8(255, 255, 255, 255)
                        };
                        knob_paint.set_color(knob_color);
                        
                        let knob_radius = rect_h / 2.0 - 4.0;
                        let knob_x = if toggled {
                            pixel_w as f32 - pad - knob_radius - 4.0
                        } else {
                            pad + knob_radius + 4.0
                        };
                        let knob_y = pixel_h as f32 / 2.0;
                        
                        let mut knob_builder = resvg::tiny_skia::PathBuilder::new();
                        knob_builder.push_circle(knob_x, knob_y, knob_radius);
                        if let Some(knob_path) = knob_builder.finish() {
                            pixmap.fill_path(&knob_path, &knob_paint, resvg::tiny_skia::FillRule::Winding, resvg::tiny_skia::Transform::identity(), None);
                        }
                        
                        Ok(Self::pixmap_to_rgba_image(pixmap))
                    })()
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
                    if profile.supports_kitty_gfx {
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
                    for dy in 0..height {
                        for dx in 0..width {
                            let ch = if dy == 0 || dy == height - 1 || dx == 0 || dx == width - 1 {
                                '*'
                            } else {
                                ' '
                            };
                            Self::safe_set(buffer, abs_x + dx as i32, abs_y + dy as i32, Cell {
                                ch,
                                fg: oxiterm_proto::style::AnsiColor::TrueColor(255, 0, 0),
                                ..Default::default()
                            });
                        }
                    }
                    let name = resolved_path.file_name().and_then(|n| n.to_str()).unwrap_or("Image");
                    let len = name.chars().count() as u16;
                    if width > len + 2 && height > 2 {
                        let start_x = abs_x + (width - len) as i32 / 2;
                        let start_y = abs_y + height as i32 / 2;
                        for (i, c) in name.chars().enumerate() {
                            Self::safe_set(buffer, start_x + i as i32, start_y, Cell {
                                ch: c,
                                fg: oxiterm_proto::style::AnsiColor::TrueColor(255, 0, 0),
                                ..Default::default()
                            });
                        }
                    }
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
    ) {
        if profile.is_web {
            return;
        }

        let draw_fallback = |resolved_path: &Path, buffer: &mut CellBuffer| {
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
            let name = resolved_path.file_name().and_then(|n| n.to_str()).unwrap_or("Video");
            let is_missing = {
                #[cfg(target_arch = "wasm32")]
                {
                    true
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    !crate::render::cache::VideoPlayerRegistry::is_ffmpeg_available()
                }
            };
            let display_name = if is_missing {
                format!("[Video Error: ffmpeg missing! {}]", name)
            } else {
                format!("[Video: {}]", name)
            };
            let len = display_name.chars().count() as u16;
            if width > len + 2 && height > 2 {
                let start_x = abs_x + (width - len) as i32 / 2;
                let start_y = abs_y + height as i32 / 2;
                for (i, c) in display_name.chars().enumerate() {
                    Self::safe_set(buffer, start_x + i as i32, start_y, Cell {
                        ch: c,
                        fg: oxiterm_proto::style::AnsiColor::TrueColor(255, 0, 0),
                        ..Default::default()
                    });
                }
            }
        };

        if let Some(ref src) = node.attrs.src {
            let resolved_path = if let Some(base) = base_dir {
                base.join(src)
            } else {
                std::path::PathBuf::from(src)
            };

            let pixel_w = width as u32 * 10;
            let pixel_h = height as u32 * 20;
            if pixel_w == 0 || pixel_h == 0 {
                return;
            }

            let fps = if profile.supports_kitty_gfx {
                30
            } else if profile.supports_sixel {
                10
            } else {
                2
            };

            let frame: Option<std::sync::Arc<Vec<u8>>> = {
                #[cfg(target_arch = "wasm32")]
                {
                    None
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    crate::render::cache::VideoPlayerRegistry::get().get_frame(&resolved_path, pixel_w, pixel_h, fps)
                }
            };

            #[cfg(not(target_arch = "wasm32"))]
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
            draw_fallback(&resolved_path, buffer);
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
    fn test_border_and_transparency_rendering() {
        let mut doc = THTMLDocument::new();
        
        let mut parent = Node::new(NodeTag::Box);
        parent.style.width = Some(5);
        parent.style.height = Some(3);
        parent.style.bg = AnsiColor::TrueColor(10, 20, 30);
        parent.style.border = Some(BorderStyle {
            fg: AnsiColor::TrueColor(100, 100, 100),
            chars: BorderChars::default(),
        });
        
        let mut child = Node::new(NodeTag::Text);
        child.text = Some("A".to_string());
        child.style.fg = AnsiColor::Reset;
        child.style.width = Some(1);
        child.style.height = Some(1);
        
        let parent_id = doc.arena.alloc(parent);
        let child_id = doc.arena.alloc(child);
        doc.append_child(parent_id, child_id).unwrap();
        doc.append_child(doc.root, parent_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 5, 3, None).unwrap();

        let mut buffer = CellBuffer::new(5, 3);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None, 0);

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
        text_node.style.width = Some(5);
        text_node.style.height = Some(1);
        text_node.style.bg = AnsiColor::TrueColor(5, 5, 5);
        
        let node_id = doc.arena.alloc(text_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 5, 1, None).unwrap();

        let mut buffer = CellBuffer::new(5, 1);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None, 0);

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
        img_node.style.width = Some(6);
        img_node.style.height = Some(4);
        
        let node_id = doc.arena.alloc(img_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 6, 4, None).unwrap();

        let mut buffer = CellBuffer::new(6, 4);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None, 0);

        // Since the file is non-existent, it should render the fallback border of '*'
        assert_eq!(buffer.cells[0].ch, '*');
        assert_eq!(buffer.cells[5].ch, '*');
    }

    #[test]
    fn test_video_fallback_rendering() {
        let mut doc = THTMLDocument::new();
        
        let mut vid_node = Node::new(NodeTag::Video);
        vid_node.attrs.src = Some("nonexistent_video.mp4".to_string());
        vid_node.style.width = Some(15);
        vid_node.style.height = Some(5);
        
        let node_id = doc.arena.alloc(vid_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 15, 5, None).unwrap();

        let mut buffer = CellBuffer::new(15, 5);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None, 0);

        // Falling back should draw border of '*'
        assert_eq!(buffer.cells[0].ch, '*');
        assert_eq!(buffer.cells[14].ch, '*');
    }
}

