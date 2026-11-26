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
}
