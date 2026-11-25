//! Linux framebuffer (`/dev/fb0`): query the mode with `FBIOGET_VSCREENINFO` /
//! `FBIOGET_FSCREENINFO`, convert the RGBA surface to the device's pixel format
//! ([`crate::pixel`]) and `pwrite` it row by row (no mmap, so no unsafe memory access to the
//! scan-out buffer). The console keeps drawing on the same buffer unless the VT is in graphics
//! mode; run from a spare VT. Not executed by tests.

use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;
use std::path::Path;

use crate::pixel::{self, BitField, PixelFormat};
use crate::Surface;

const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FbBitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

/// `struct fb_var_screeninfo` (160 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VarScreenInfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitfield,
    green: FbBitfield,
    blue: FbBitfield,
    transp: FbBitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
}

/// `struct fb_fix_screeninfo` (80 bytes on LP64).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FixScreenInfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    type_: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: libc::c_ulong,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

/// What [`present`] needs to know about the device, extracted from the two screeninfo structs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    /// Byte offset of the visible window (`yoffset * stride`).
    pub base: u64,
    pub format: PixelFormat,
}

impl Mode {
    pub fn from_info(v: &VarScreenInfo, f: &FixScreenInfo) -> Result<Mode, String> {
        let bf = |b: FbBitfield| BitField { offset: b.offset, length: b.length };
        let format = PixelFormat {
            bpp: v.bits_per_pixel,
            red: bf(v.red),
            green: bf(v.green),
            blue: bf(v.blue),
            transp: bf(v.transp),
        };
        if !format.is_supported() || v.nonstd != 0 || v.grayscale != 0 {
            return Err(format!("unsupported fbdev pixel format: {} bpp, nonstd {}, grayscale {}", v.bits_per_pixel, v.nonstd, v.grayscale));
        }
        Ok(Mode {
            width: v.xres as usize,
            height: v.yres as usize,
            stride: f.line_length as usize,
            base: u64::from(v.yoffset) * u64::from(f.line_length),
            format,
        })
    }
}

/// Opens `path` (e.g. `/dev/fb0`), reads its mode and writes `surface` into the top-left corner
/// (cropped to the screen). Needs the [`super::DeviceOptIn`] token that only [`super::require_opt_in`] hands out.
pub fn present(path: &Path, surface: &Surface, _opt_in: &super::DeviceOptIn) -> Result<(), String> {
    let file = OpenOptions::new().read(true).write(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut var = VarScreenInfo::default();
    let mut fix = FixScreenInfo::default();
    // SAFETY: both structs are repr(C) with the sizes of the kernel's `fb_var_screeninfo` /
    // `fb_fix_screeninfo`; the kernel fills them for these read-only queries.
    unsafe {
        if libc::ioctl(file.as_raw_fd(), FBIOGET_VSCREENINFO, &raw mut var) != 0 {
            return Err(format!("FBIOGET_VSCREENINFO: {}", std::io::Error::last_os_error()));
        }
        if libc::ioctl(file.as_raw_fd(), FBIOGET_FSCREENINFO, &raw mut fix) != 0 {
            return Err(format!("FBIOGET_FSCREENINFO: {}", std::io::Error::last_os_error()));
        }
    }
    let mode = Mode::from_info(&var, &fix)?;
    let bpp = mode.format.bytes_per_pixel();
    let (w, h) = (surface.width() as usize, surface.height() as usize);
    let (cw, ch) = (w.min(mode.width), h.min(mode.height));
    let mut row = vec![0u8; cw * bpp];
    for y in 0..ch {
        pixel::convert_row(&mode.format, &surface.rgba()[y * w * 4..(y * w + cw) * 4], &mut row);
        file.write_all_at(&row, mode.base + y as u64 * mode.stride as u64).map_err(|e| format!("write row {y}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_match_the_kernel_header() {
        assert_eq!(std::mem::size_of::<VarScreenInfo>(), 160);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(std::mem::size_of::<FixScreenInfo>(), 80);
    }

    #[test]
    fn mode_from_a_typical_xrgb_screen() {
        let v = VarScreenInfo {
            xres: 1920,
            yres: 1080,
            bits_per_pixel: 32,
            yoffset: 2,
            red: FbBitfield { offset: 16, length: 8, msb_right: 0 },
            green: FbBitfield { offset: 8, length: 8, msb_right: 0 },
            blue: FbBitfield { offset: 0, length: 8, msb_right: 0 },
            ..Default::default()
        };
        let f = FixScreenInfo { line_length: 7680, ..Default::default() };
        let m = Mode::from_info(&v, &f).unwrap();
        assert_eq!((m.width, m.height, m.stride, m.base), (1920, 1080, 7680, 15360));
        assert_eq!(m.format.pack(1, 2, 3), PixelFormat::XRGB8888.pack(1, 2, 3));
        let bad = VarScreenInfo { bits_per_pixel: 8, ..v };
        assert!(Mode::from_info(&bad, &f).is_err());
    }
}
