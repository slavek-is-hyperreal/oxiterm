use crate::document::THTMLDocument;
use crate::layout::types::LayoutResult;
use crate::render::buffer::{CellBuffer, Cell};
use oxiterm_proto::dom::{NodeTag, NodeId};
use oxiterm_proto::style::TerminalProfile;
use std::path::Path;

pub struct Renderer;

impl Renderer {
    pub fn render_node(
        doc: &THTMLDocument,
        layout: &LayoutResult,
        buffer: &mut CellBuffer,
        profile: &TerminalProfile,
        base_dir: Option<&Path>,
    ) {
        // 1. Całkowite czyszczenie bufora do spacji (zapobiega duszkom jak "PROUALNA")
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
        
        // 2. Rekurencyjne rysowanie drzewa DOM z centrowaniem korzenia, jeśli ma sztywną mniejszą wielkość
        let (offset_x, offset_y) = layout.get_centering_offset(doc, buffer.width, buffer.height);

        Self::render_recursive(
            doc,
            layout,
            buffer,
            doc.root,
            offset_x,
            offset_y,
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
        parent_x: u16,
        parent_y: u16,
        inherited_fg: oxiterm_proto::style::AnsiColor,
        inherited_bg: oxiterm_proto::style::AnsiColor,
        profile: &TerminalProfile,
        base_dir: Option<&Path>,
    ) {
        if let Some(node) = doc.arena.get(node_id) {
            let rect = layout.nodes.get(&node_id).copied().unwrap_or_default();
            let abs_x = parent_x + rect.x;
            let abs_y = parent_y + rect.y;

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
                    buffer.set(abs_x + x, abs_y + y, Cell {
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
                    buffer.set(abs_x, abs_y, Cell {
                        ch: border.chars.top_left,
                        fg: border_fg,
                        bg: resolved_bg,
                        ..Default::default()
                    });
                    if rect.width > 1 {
                        buffer.set(abs_x + rect.width - 1, abs_y, Cell {
                            ch: border.chars.top_right,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                    }
                    if rect.height > 1 {
                        buffer.set(abs_x, abs_y + rect.height - 1, Cell {
                            ch: border.chars.bot_left,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                    }
                    if rect.width > 1 && rect.height > 1 {
                        buffer.set(abs_x + rect.width - 1, abs_y + rect.height - 1, Cell {
                            ch: border.chars.bot_right,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                    }

                    // Horizontal borders
                    for x in 1..rect.width.saturating_sub(1) {
                        buffer.set(abs_x + x, abs_y, Cell {
                            ch: border.chars.top,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                        if rect.height > 1 {
                            buffer.set(abs_x + x, abs_y + rect.height - 1, Cell {
                                ch: border.chars.bot,
                                fg: border_fg,
                                bg: resolved_bg,
                                ..Default::default()
                            });
                        }
                    }

                    // Vertical borders
                    for y in 1..rect.height.saturating_sub(1) {
                        buffer.set(abs_x, abs_y + y, Cell {
                            ch: border.chars.left,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                        if rect.width > 1 {
                            buffer.set(abs_x + rect.width - 1, abs_y + y, Cell {
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
                                    if cx < content_w && cy < content_h {
                                        buffer.set(content_x + cx, content_y + cy, Cell {
                                            ch,
                                            fg: resolved_fg,
                                            bg: resolved_bg,
                                            ..Default::default()
                                        });
                                        // Fill continuation cells with styled spaces
                                        for i in 1..char_w {
                                            if cx + i < content_w {
                                                buffer.set(content_x + cx + i, content_y + cy, Cell {
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
                        buffer.set(content_x + x, content_y, Cell {
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
        abs_x: u16,
        abs_y: u16,
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

                buf.set(abs_x + x, abs_y + y, Cell {
                    ch,
                    fg,
                    bg,
                    ..Default::default()
                });
            }
        }
    }

    fn pixmap_to_rgba_image(pixmap: resvg::tiny_skia::Pixmap) -> image::RgbaImage {
        let mut rgba_data = pixmap.data().to_vec();
        for pixel in rgba_data.chunks_exact_mut(4) {
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
        abs_x: u16,
        abs_y: u16,
        width: u16,
        height: u16,
        buffer: &mut CellBuffer,
        profile: &TerminalProfile,
        base_dir: Option<&Path>,
    ) {
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
            let is_rive = resolved_path.extension()
                .map(|ext| ext.to_string_lossy().to_lowercase() == "riv")
                .unwrap_or(false);

            // Fetch playback state if animation
            let mut frame_idx = None;
            if is_lottie {
                let playback = crate::render::cache::PlaybackRegistry::get().get_or_create(&resolved_path);
                let elapsed = playback.start_time.elapsed();
                frame_idx = Some(((elapsed.as_millis() * 15) / 1000) as usize % 30);
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
                let mut cmd = Vec::new();
                cmd.extend_from_slice(format!("\x1b[{};{}H", abs_y + 1, abs_x + 1).as_bytes());
                cmd.extend_from_slice(&bytes);
                buffer.graphics.push(cmd);
                
                for dy in 0..height {
                    for dx in 0..width {
                        if let Some(idx) = buffer.flat_idx(abs_x + dx, abs_y + dy) {
                            buffer.cells[idx].skip = true;
                        }
                    }
                }
                return;
            }

            // 2. Cache miss. Render from scratch
            let img_result = if is_svg {
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
            } else if is_lottie {
                // Procedural Lottie Loader animation
                (|| -> anyhow::Result<image::RgbaImage> {
                    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixel_w, pixel_h)
                        .ok_or_else(|| anyhow::anyhow!("Failed to create pixmap"))?;
                    
                    let center_x = pixel_w as f32 / 2.0;
                    let center_y = pixel_h as f32 / 2.0;
                    let radius = (pixel_w.min(pixel_h) as f32 / 2.0) - 10.0;
                    
                    let mut paint = resvg::tiny_skia::Paint::default();
                    paint.anti_alias = true;
                    
                    let grad = resvg::tiny_skia::LinearGradient::new(
                        resvg::tiny_skia::Point::from_xy(0.0, 0.0),
                        resvg::tiny_skia::Point::from_xy(pixel_w as f32, pixel_h as f32),
                        vec![
                            resvg::tiny_skia::GradientStop::new(0.0, resvg::tiny_skia::Color::from_rgba8(0, 240, 255, 255)),
                            resvg::tiny_skia::GradientStop::new(1.0, resvg::tiny_skia::Color::from_rgba8(0, 80, 255, 255)),
                        ],
                        resvg::tiny_skia::SpreadMode::Pad,
                        resvg::tiny_skia::Transform::identity(),
                    ).unwrap();
                    paint.shader = grad;
                    
                    let idx = frame_idx.unwrap_or(0);
                    let angle_offset = (idx as f32 / 30.0) * 2.0 * std::f32::consts::PI;
                    
                    for i in 0..8 {
                        let angle = angle_offset + (i as f32 / 8.0) * 2.0 * std::f32::consts::PI;
                        let size_factor = i as f32 / 8.0;
                        let dot_radius = 3.0 + size_factor * 6.0;
                        let dx = center_x + angle.cos() * radius;
                        let dy = center_y + angle.sin() * radius;
                        
                        let mut dp = resvg::tiny_skia::PathBuilder::new();
                        dp.push_circle(dx, dy, dot_radius);
                        if let Some(path) = dp.finish() {
                            pixmap.fill_path(&path, &paint, resvg::tiny_skia::FillRule::Winding, resvg::tiny_skia::Transform::identity(), None);
                        }
                    }
                    Ok(Self::pixmap_to_rgba_image(pixmap))
                })()
            } else if is_rive {
                // Procedural Rive Toggle Button widget
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
            } else {
                // Standard image rendering
                image::open(&resolved_path)
                    .map(|img| {
                        let rgba = img.to_rgba8();
                        image::imageops::resize(&rgba, pixel_w, pixel_h, image::imageops::FilterType::Triangle)
                    })
                    .map_err(|e| anyhow::anyhow!("{}", e))
            };

            match img_result {
                Ok(img) => {
                    if profile.supports_kitty_gfx {
                        let payload = super::kitty::KittyImageManager::transmit_image(pixel_w, pixel_h, &img);
                        
                        // Insert into cache
                        AssetCache::get().insert(
                            cache_key,
                            CacheValue {
                                format: GraphicFormat::Kitty(payload.clone()),
                            },
                        );

                        let mut cmd = Vec::new();
                        cmd.extend_from_slice(format!("\x1b[{};{}H", abs_y + 1, abs_x + 1).as_bytes());
                        cmd.extend_from_slice(&payload);
                        buffer.graphics.push(cmd);
                        
                        for dy in 0..height {
                            for dx in 0..width {
                                if let Some(idx) = buffer.flat_idx(abs_x + dx, abs_y + dy) {
                                    buffer.cells[idx].skip = true;
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

                        let mut cmd = Vec::new();
                        cmd.extend_from_slice(format!("\x1b[{};{}H", abs_y + 1, abs_x + 1).as_bytes());
                        cmd.extend_from_slice(&sixel_payload);
                        buffer.graphics.push(cmd);
                        
                        for dy in 0..height {
                            for dx in 0..width {
                                if let Some(idx) = buffer.flat_idx(abs_x + dx, abs_y + dy) {
                                    buffer.cells[idx].skip = true;
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
                            buffer.set(abs_x + dx, abs_y + dy, Cell {
                                ch,
                                fg: oxiterm_proto::style::AnsiColor::TrueColor(255, 0, 0),
                                ..Default::default()
                            });
                        }
                    }
                    let name = resolved_path.file_name().and_then(|n| n.to_str()).unwrap_or("Image");
                    let len = name.chars().count() as u16;
                    if width > len + 2 && height > 2 {
                        let start_x = abs_x + (width - len) / 2;
                        let start_y = abs_y + height / 2;
                        for (i, c) in name.chars().enumerate() {
                            buffer.set(start_x + i as u16, start_y, Cell {
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
        let layout = engine.compute(&mut doc, 5, 3).unwrap();

        let mut buffer = CellBuffer::new(5, 3);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None);

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
        let layout = engine.compute(&mut doc, 5, 1).unwrap();

        let mut buffer = CellBuffer::new(5, 1);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None);

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
        let layout = engine.compute(&mut doc, 6, 4).unwrap();

        let mut buffer = CellBuffer::new(6, 4);
        Renderer::render_node(&doc, &layout, &mut buffer, &TerminalProfile::default(), None);

        // Since the file is non-existent, it should render the fallback border of '*'
        assert_eq!(buffer.cells[0].ch, '*');
        assert_eq!(buffer.cells[5].ch, '*');
    }
}

