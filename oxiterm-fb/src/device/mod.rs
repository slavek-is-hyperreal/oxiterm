//! Scan-out backends: Linux fbdev (`/dev/fb0`) and a DRM/KMS dumb buffer.
//!
//! **These take over a real display.** They are compiled only with the `device` cargo feature,
//! refuse to run unless the caller passes the explicit opt-in (`oxifb ... --really-write-device`
//! AND the environment variable `OXIDE_ALLOW_DEVICE=1`, see [`require_opt_in`]), and no test or
//! CI job touches them. The tests cover only the pure parts (struct sizes, ioctl numbers, pixel
//! conversion in [`crate::pixel`]).
//!
//! Layout and ioctl numbers are taken from `linux/fb.h` and `drm/drm.h` / `drm/drm_mode.h`.
//! The `_IOC` encoding used here (`dir << 30 | size << 16 | type << 8 | nr`) is the one of
//! x86, arm, arm64 and riscv; the kernel's mips/powerpc/sparc encoding differs and is not
//! supported.

pub mod drm;
pub mod fbdev;

/// Proof that the caller passed the explicit opt-in. The only way to obtain one is
/// [`require_opt_in`]; the private field keeps library callers from constructing it, so
/// `fbdev::present` and `drm::present` cannot be reached without going through the guard.
#[derive(Debug)]
pub struct DeviceOptIn(());

/// The runtime guard of every device backend: needs the caller's explicit flag AND the
/// environment variable `OXIDE_ALLOW_DEVICE=1`.
pub fn require_opt_in(flag_given: bool) -> Result<DeviceOptIn, String> {
    check_opt_in(flag_given, std::env::var("OXIDE_ALLOW_DEVICE").ok().as_deref())
}

fn check_opt_in(flag_given: bool, env: Option<&str>) -> Result<DeviceOptIn, String> {
    if !flag_given {
        return Err("device output needs the explicit flag --really-write-device".into());
    }
    if env != Some("1") {
        return Err("device output needs the environment variable OXIDE_ALLOW_DEVICE=1".into());
    }
    Ok(DeviceOptIn(()))
}

/// `_IOWR(type, nr, size)` for the generic Linux ioctl encoding.
pub(crate) const fn iowr(ty: u8, nr: u8, size: usize) -> libc::c_ulong {
    ((3u32 << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr as u32) as libc::c_ulong
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_needs_flag_and_env() {
        assert!(check_opt_in(false, Some("1")).is_err());
        assert!(check_opt_in(true, None).is_err());
        assert!(check_opt_in(true, Some("0")).is_err());
        assert!(check_opt_in(true, Some("1")).is_ok());
    }
}
