//! The procedural `.riv` toggle widget (a stand-in for a Rive runtime), rasterized in pure Rust.
//!
//! No `resvg`/`tiny_skia`: the widget must draw in every build, including the browser's local
//! mode (`native-media` off), where it reaches the screen as half-block cells. The geometry is the
//! one the native build always drew: a track with a 3 px outline and a sliding knob, coloured by
//! the playback bits (`1` hover, `2` pressed, `4` toggled).

use image::RgbaImage;

const PAD: f32 = 6.0;

fn blend(px: &mut [f32; 4], rgb: [u8; 3], cov: f32) {
    let cov = cov.clamp(0.0, 1.0);
    if cov <= 0.0 {
        return;
    }
    let out_a = cov + px[3] * (1.0 - cov);
    for i in 0..3 {
        let c = f32::from(rgb[i]);
        px[i] = (c * cov + px[i] * px[3] * (1.0 - cov)) / out_a;
    }
    px[3] = out_a;
}

/// Signed distance to the rectangle `[x0,x1] x [y0,y1]` (negative inside).
fn rect_sd(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let (dx, dy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0)
}

/// Rasterizes the toggle into a `w x h` image (straight alpha; fully transparent outside the track).
pub fn toggle_image(w: u32, h: u32, bits: usize) -> RgbaImage {
    let hover = bits & 1 != 0;
    let pressed = bits & 2 != 0;
    let toggled = bits & 4 != 0;
    let (wf, hf) = (w as f32, h as f32);
    let rect_w = wf - PAD * 2.0;
    let rect_h = hf - PAD * 2.0;
    let mut img = RgbaImage::new(w, h);
    if rect_w <= 0.0 || rect_h <= 0.0 {
        return img;
    }
    let bg: [u8; 3] = if toggled { [0, 120, 255] } else if hover { [70, 70, 70] } else { [35, 35, 35] };
    let border: [u8; 3] = if pressed { [0, 255, 255] } else if hover { [220, 220, 220] } else { [90, 90, 90] };
    let knob_col: [u8; 3] = if pressed { [0, 255, 255] } else { [255, 255, 255] };
    let knob_r = rect_h / 2.0 - 4.0;
    let knob_x = if toggled { wf - PAD - knob_r - 4.0 } else { PAD + knob_r + 4.0 };
    let knob_y = hf / 2.0;
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut c = [0.0f32; 4];
            let sd = rect_sd(px, py, PAD, PAD, PAD + rect_w, PAD + rect_h);
            blend(&mut c, bg, 0.5 - sd);
            // 3 px outline centred on the track edge.
            blend(&mut c, border, 1.5 + 0.5 - sd.abs());
            if knob_r > 0.0 {
                let d = (px - knob_x).hypot(py - knob_y);
                blend(&mut c, knob_col, knob_r - d + 0.5);
            }
            img.put_pixel(x, y, image::Rgba([c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, (c[3] * 255.0).round() as u8]));
        }
    }
    img
}

/// Pixel budget of the web half-block raster (before the resize to `w x 2h` cells). The usual
/// widget (a few dozen cells) is far below it and keeps the full 8 x 16 oversampling; a
/// viewport-sized one (up to 500 x 200 cells, chosen by the visitor) is rasterised with less
/// oversampling instead of 12.8 Mpx per frame (BAD ACTOR 7.6 F-13).
pub const WEB_RASTER_BUDGET_PX: u64 = 1_500_000;
const WEB_CACHE_CAPACITY: usize = 64;

type WebKey = (u32, u32, usize);

/// The toggle as half-block pixels (`width` x `height * 2`), cached per `(size, state bits)`. The
/// key holds no path and no session, so sharing it between sessions leaks nothing.
pub fn toggle_half_block_image(width: u32, height: u32, bits: usize) -> std::sync::Arc<RgbaImage> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<WebKey, Arc<RgbaImage>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (width, height, bits);
    if let Ok(c) = cache.lock() {
        if let Some(img) = c.get(&key) {
            return Arc::clone(img);
        }
    }
    let (mut rw, mut rh) = (u64::from(width) * 8, u64::from(height) * 16);
    if rw * rh > WEB_RASTER_BUDGET_PX {
        // Same factor on both axes, never below one raster pixel per output pixel.
        let f = (WEB_RASTER_BUDGET_PX as f64 / (rw * rh) as f64).sqrt();
        rw = ((rw as f64 * f) as u64).max(u64::from(width));
        rh = ((rh as f64 * f) as u64).max(u64::from(height) * 2);
    }
    let big = toggle_image(rw as u32, rh as u32, bits);
    let img = Arc::new(image::imageops::resize(&big, width, height * 2, image::imageops::FilterType::Triangle));
    if let Ok(mut c) = cache.lock() {
        if c.len() >= WEB_CACHE_CAPACITY {
            c.clear();
        }
        c.insert(key, Arc::clone(&img));
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knob_moves_and_colours_follow_the_bits() {
        let off = toggle_image(140, 60, 0);
        let on = toggle_image(140, 60, 4);
        // Centre of the track near the left: the (white) knob when off, the blue track when on.
        assert_eq!(off.get_pixel(20, 30).0[..3], [255, 255, 255]);
        assert_eq!(on.get_pixel(20, 30).0[..3], [0, 120, 255]);
        assert_eq!(on.get_pixel(120, 30).0[..3], [255, 255, 255]);
        // Outside the track is transparent.
        assert_eq!(off.get_pixel(1, 1).0[3], 0);
        assert_eq!(toggle_image(8, 8, 0).get_pixel(4, 4).0[3], 0, "degenerate size draws nothing");
    }

    #[test]
    fn web_raster_is_capped_and_cached() {
        let a = toggle_half_block_image(496, 198, 0);
        assert_eq!((a.width(), a.height()), (496, 396));
        let b = toggle_half_block_image(496, 198, 0);
        assert!(std::sync::Arc::ptr_eq(&a, &b), "second render is a cache hit");
        // a small widget keeps the full oversampling
        let small = toggle_half_block_image(14, 3, 4);
        assert_eq!((small.width(), small.height()), (14, 6));
    }
}
