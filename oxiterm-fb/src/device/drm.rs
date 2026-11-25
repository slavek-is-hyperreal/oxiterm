//! DRM/KMS dumb-buffer output with raw ioctls (no libdrm, no extra crate).
//!
//! Sequence: find a connected connector and its preferred mode, pick its CRTC, create a 32 bpp
//! dumb buffer, register it as a framebuffer, map and fill it (XRGB8888), then `SETCRTC`. The
//! previous CRTC state is saved and restored, and the buffer destroyed, when [`present`] returns
//! (after `hold`), including on SIGINT/SIGTERM during `hold` (a flag-setting handler wakes the
//! hold loop; SIGKILL and a crash in the middle of setup cannot be caught and leave the CRTC as
//! the last `SETCRTC` set it, so keep `hold` short). Needs DRM master: run from a spare VT with no compositor running, or as the
//! only display client. Not executed by tests.

use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::iowr;
use crate::pixel::{self, PixelFormat};
use crate::Surface;

#[repr(C)]
#[derive(Clone, Copy)]
struct ModeInfo {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,
    vrefresh: u32,
    flags: u32,
    type_: u32,
    name: [u8; 32],
}

impl Default for ModeInfo {
    fn default() -> Self {
        // SAFETY: plain integers and a byte array; all-zero is valid.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Default)]
struct CardRes {
    fb_id_ptr: u64,
    crtc_id_ptr: u64,
    connector_id_ptr: u64,
    encoder_id_ptr: u64,
    count_fbs: u32,
    count_crtcs: u32,
    count_connectors: u32,
    count_encoders: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
#[derive(Default)]
struct GetConnector {
    encoders_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    count_modes: u32,
    count_props: u32,
    count_encoders: u32,
    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Default)]
struct GetEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

#[repr(C)]
#[derive(Default)]
struct Crtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: ModeInfo,
}

#[repr(C)]
#[derive(Default)]
struct FbCmd {
    fb_id: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    depth: u32,
    handle: u32,
}

#[repr(C)]
#[derive(Default)]
struct CreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
#[derive(Default)]
struct MapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[repr(C)]
#[derive(Default)]
struct DestroyDumb {
    handle: u32,
}

const GETRESOURCES: libc::c_ulong = iowr(b'd', 0xA0, std::mem::size_of::<CardRes>());
const GETCRTC: libc::c_ulong = iowr(b'd', 0xA1, std::mem::size_of::<Crtc>());
const SETCRTC: libc::c_ulong = iowr(b'd', 0xA2, std::mem::size_of::<Crtc>());
const GETENCODER: libc::c_ulong = iowr(b'd', 0xA6, std::mem::size_of::<GetEncoder>());
const GETCONNECTOR: libc::c_ulong = iowr(b'd', 0xA7, std::mem::size_of::<GetConnector>());
const ADDFB: libc::c_ulong = iowr(b'd', 0xAE, std::mem::size_of::<FbCmd>());
const RMFB: libc::c_ulong = iowr(b'd', 0xAF, std::mem::size_of::<u32>());
const CREATE_DUMB: libc::c_ulong = iowr(b'd', 0xB2, std::mem::size_of::<CreateDumb>());
const MAP_DUMB: libc::c_ulong = iowr(b'd', 0xB3, std::mem::size_of::<MapDumb>());
const DESTROY_DUMB: libc::c_ulong = iowr(b'd', 0xB4, std::mem::size_of::<DestroyDumb>());

const MODE_TYPE_PREFERRED: u32 = 1 << 3;
const CONNECTOR_CONNECTED: u32 = 1;

fn ioctl<T>(fd: i32, req: libc::c_ulong, arg: &mut T, what: &str) -> Result<(), String> {
    // SAFETY: `arg` is a repr(C) struct matching the kernel struct for `req`; pointers it holds
    // (set by the callers below) point into Vecs that outlive the call.
    let r = unsafe { libc::ioctl(fd, req, &raw mut *arg) };
    if r == 0 {
        Ok(())
    } else {
        Err(format!("{what}: {}", std::io::Error::last_os_error()))
    }
}

/// Everything created on the device, undone in `Drop` (also on an error part-way).
struct Output {
    fd: i32,
    saved: Option<Crtc>,
    /// Connectors that were attached to the CRTC before we touched it.
    restore_connectors: Vec<u32>,
    fb_id: u32,
    handle: u32,
    map: *mut libc::c_void,
    map_len: usize,
}

impl Drop for Output {
    fn drop(&mut self) {
        if let Some(mut saved) = self.saved.take() {
            // Nothing was displayed before (mode_valid 0): disable the CRTC again.
            let had_mode = saved.mode_valid != 0 && !self.restore_connectors.is_empty();
            if !had_mode {
                saved.mode_valid = 0;
                saved.fb_id = 0;
            }
            saved.set_connectors_ptr = self.restore_connectors.as_ptr() as u64;
            saved.count_connectors = if had_mode { self.restore_connectors.len() as u32 } else { 0 };
            let _ = ioctl(self.fd, SETCRTC, &mut saved, "restore SETCRTC");
        }
        if !self.map.is_null() {
            // SAFETY: `map`/`map_len` come from a successful mmap of this buffer.
            unsafe { libc::munmap(self.map, self.map_len) };
        }
        if self.fb_id != 0 {
            let mut id = self.fb_id;
            let _ = ioctl(self.fd, RMFB, &mut id, "RMFB");
        }
        if self.handle != 0 {
            let mut d = DestroyDumb { handle: self.handle };
            let _ = ioctl(self.fd, DESTROY_DUMB, &mut d, "DESTROY_DUMB");
        }
    }
}

fn ids(fd: i32) -> Result<(Vec<u32>, Vec<u32>), String> {
    let mut res = CardRes::default();
    ioctl(fd, GETRESOURCES, &mut res, "GETRESOURCES")?;
    let mut crtcs = vec![0u32; res.count_crtcs as usize];
    let mut conns = vec![0u32; res.count_connectors as usize];
    let mut res2 = CardRes {
        crtc_id_ptr: crtcs.as_mut_ptr() as u64,
        connector_id_ptr: conns.as_mut_ptr() as u64,
        count_crtcs: res.count_crtcs,
        count_connectors: res.count_connectors,
        ..Default::default()
    };
    ioctl(fd, GETRESOURCES, &mut res2, "GETRESOURCES (ids)")?;
    Ok((crtcs, conns))
}

/// A connected connector: its id, preferred mode and the CRTC to drive it with.
fn pick(fd: i32, crtcs: &[u32], conns: &[u32]) -> Result<(u32, ModeInfo, u32), String> {
    for &cid in conns {
        let mut c = GetConnector { connector_id: cid, ..Default::default() };
        ioctl(fd, GETCONNECTOR, &mut c, "GETCONNECTOR (counts)")?;
        if c.connection != CONNECTOR_CONNECTED || c.count_modes == 0 {
            continue;
        }
        let mut modes = vec![ModeInfo::default(); c.count_modes as usize];
        let mut encs = vec![0u32; c.count_encoders as usize];
        let mut c2 = GetConnector {
            connector_id: cid,
            modes_ptr: modes.as_mut_ptr() as u64,
            count_modes: c.count_modes,
            encoders_ptr: encs.as_mut_ptr() as u64,
            count_encoders: c.count_encoders,
            ..Default::default()
        };
        ioctl(fd, GETCONNECTOR, &mut c2, "GETCONNECTOR")?;
        let mode = modes.iter().find(|m| m.type_ & MODE_TYPE_PREFERRED != 0).unwrap_or(&modes[0]);
        let enc_id = if c2.encoder_id != 0 { c2.encoder_id } else { *encs.first().ok_or("connector has no encoder")? };
        let mut e = GetEncoder { encoder_id: enc_id, ..Default::default() };
        ioctl(fd, GETENCODER, &mut e, "GETENCODER")?;
        let crtc = if e.crtc_id != 0 {
            e.crtc_id
        } else {
            let bit = e.possible_crtcs.trailing_zeros() as usize;
            *crtcs.get(bit).ok_or("encoder has no usable CRTC")?
        };
        return Ok((cid, *mode, crtc));
    }
    Err("no connected connector with a mode".into())
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst); // async-signal-safe
}

/// Installs the SIGINT/SIGTERM flag handler and puts the previous ones back on drop.
struct SignalGuard {
    old: [(libc::c_int, libc::sigaction); 2],
}

impl SignalGuard {
    fn install() -> Self {
        STOP.store(false, Ordering::SeqCst);
        let mut old = [(libc::SIGINT, unsafe { std::mem::zeroed() }), (libc::SIGTERM, unsafe { std::mem::zeroed() })];
        for (sig, prev) in &mut old {
            // SAFETY: a zeroed sigaction with a handler that only stores to an atomic.
            unsafe {
                let mut act: libc::sigaction = std::mem::zeroed();
                act.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
                libc::sigemptyset(&raw mut act.sa_mask);
                libc::sigaction(*sig, &raw const act, &raw mut *prev);
            }
        }
        Self { old }
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        for (sig, prev) in &self.old {
            // SAFETY: restoring the action `install` saved.
            unsafe { libc::sigaction(*sig, &raw const *prev, std::ptr::null_mut()) };
        }
    }
}

/// Connectors whose current encoder drives `crtc_id`.
fn attached_to(fd: i32, conns: &[u32], crtc_id: u32) -> Vec<u32> {
    let mut out = vec![];
    for &cid in conns {
        let mut c = GetConnector { connector_id: cid, ..Default::default() };
        if ioctl(fd, GETCONNECTOR, &mut c, "GETCONNECTOR").is_err() || c.encoder_id == 0 {
            continue;
        }
        let mut e = GetEncoder { encoder_id: c.encoder_id, ..Default::default() };
        if ioctl(fd, GETENCODER, &mut e, "GETENCODER").is_ok() && e.crtc_id == crtc_id {
            out.push(cid);
        }
    }
    out
}

/// Shows `surface` on the first connected output of the DRM device `path` for `hold`, then
/// restores the previous state. Needs the [`super::DeviceOptIn`] token that only [`super::require_opt_in`] hands out.
pub fn present(path: &Path, surface: &Surface, hold: Duration, _opt_in: &super::DeviceOptIn) -> Result<(), String> {
    let file = OpenOptions::new().read(true).write(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let fd = file.as_raw_fd();
    let (crtcs, conns) = ids(fd)?;
    let (connector_id, mode, crtc_id) = pick(fd, &crtcs, &conns)?;
    let (w, h) = (u32::from(mode.hdisplay), u32::from(mode.vdisplay));

    let _signals = SignalGuard::install(); // declared before `out`: dropped after the restore
    let mut out = Output { fd, saved: None, restore_connectors: attached_to(fd, &conns, crtc_id), fb_id: 0, handle: 0, map: std::ptr::null_mut(), map_len: 0 };
    let mut saved = Crtc { crtc_id, ..Default::default() };
    ioctl(fd, GETCRTC, &mut saved, "GETCRTC")?;
    out.saved = Some(saved);

    let mut dumb = CreateDumb { width: w, height: h, bpp: 32, ..Default::default() };
    ioctl(fd, CREATE_DUMB, &mut dumb, "CREATE_DUMB")?;
    out.handle = dumb.handle;
    let mut fb = FbCmd { width: w, height: h, pitch: dumb.pitch, bpp: 32, depth: 24, handle: dumb.handle, ..Default::default() };
    ioctl(fd, ADDFB, &mut fb, "ADDFB")?;
    out.fb_id = fb.fb_id;
    let mut map = MapDumb { handle: dumb.handle, ..Default::default() };
    ioctl(fd, MAP_DUMB, &mut map, "MAP_DUMB")?;
    // SAFETY: mapping `size` bytes of the dumb buffer at the offset the kernel returned.
    let ptr = unsafe {
        libc::mmap(std::ptr::null_mut(), dumb.size as usize, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, map.offset as libc::off_t)
    };
    if ptr == libc::MAP_FAILED {
        return Err(format!("mmap: {}", std::io::Error::last_os_error()));
    }
    out.map = ptr;
    out.map_len = dumb.size as usize;
    // SAFETY: `ptr` is a live shared mapping of `size` bytes, exclusively used here.
    let buf = unsafe { std::slice::from_raw_parts_mut(ptr.cast::<u8>(), dumb.size as usize) };
    buf.fill(0);
    pixel::convert_image(
        &PixelFormat::XRGB8888,
        surface.rgba(),
        surface.width() as usize,
        surface.height() as usize,
        buf,
        dumb.pitch as usize,
        w as usize,
        h as usize,
    );

    let mut conn = connector_id;
    let mut set = Crtc {
        set_connectors_ptr: (&raw mut conn) as u64,
        count_connectors: 1,
        crtc_id,
        fb_id: fb.fb_id,
        mode_valid: 1,
        mode,
        ..Default::default()
    };
    ioctl(fd, SETCRTC, &mut set, "SETCRTC (needs DRM master)")?;
    let end = Instant::now() + hold;
    while !STOP.load(Ordering::SeqCst) {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        std::thread::sleep(left.min(Duration::from_millis(50)));
    }
    drop(out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_and_ioctl_numbers_match_the_kernel_headers() {
        assert_eq!(std::mem::size_of::<ModeInfo>(), 68);
        assert_eq!(std::mem::size_of::<CardRes>(), 64);
        assert_eq!(std::mem::size_of::<GetConnector>(), 80);
        assert_eq!(std::mem::size_of::<GetEncoder>(), 20);
        assert_eq!(std::mem::size_of::<Crtc>(), 104);
        assert_eq!(std::mem::size_of::<FbCmd>(), 28);
        assert_eq!(std::mem::size_of::<CreateDumb>(), 32);
        assert_eq!(std::mem::size_of::<MapDumb>(), 16);
        // Known values on x86-64 / arm64 (DRM_IOCTL_MODE_CREATE_DUMB = 0xC02064B2).
        assert_eq!(CREATE_DUMB as u32, 0xC020_64B2);
        assert_eq!(GETRESOURCES as u32, 0xC040_64A0);
        assert_eq!(SETCRTC as u32, 0xC068_64A2);
    }
}
