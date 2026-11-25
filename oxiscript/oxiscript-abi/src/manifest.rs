//! The `oxi.manifest` Wasm custom section: a small UTF-8 text file the host
//! reads *before* instantiating, to decide whether it can run the module and
//! which capabilities to grant (and in which order to pass the handles to
//! `oxi_init`).
//!
//! ```text
//! oxi-manifest 1
//! abi 1.0
//! edition 2026
//! profile default
//! needs host:log@1 Log
//! needs host:state@1 State
//! ```
//!
//! One directive per line; blank lines and `#` comments are ignored; unknown
//! directives are an error (so a newer module is refused rather than
//! misrun). `needs` order = the order of `oxi_init` parameters. The Wasm
//! import section must import exactly the functions of the interfaces listed
//! here and nothing else (the runtime rejects a module that imports more:
//! that is the "no ambient authority" check at link time).

use std::fmt::{self, Write};

pub const MANIFEST_SECTION: &str = "oxi.manifest";
pub const MANIFEST_FORMAT: u32 = 1;
/// Maximum number of `needs` lines (= parameters of `oxi_init`).
pub const MAX_NEEDS: usize = 32;
/// The profiles a manifest may declare.
pub const PROFILES: &[&str] = &["default", "realtime", "safety", "kernel"];

/// Canonical decimal `u32`: digits only, no sign, no leading zeros.
fn canon_u32(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0'))
    {
        return None;
    }
    s.parse().ok()
}

/// `[A-Za-z_][A-Za-z0-9_]*`
fn is_name(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Need {
    /// `host:state` (no version).
    pub module: String,
    pub version: u32,
    /// The interface type name (`State`).
    pub interface: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub abi_major: u16,
    pub abi_minor: u16,
    /// Language edition (year), see the spec's editions policy.
    pub edition: u32,
    /// Profile the module was compiled under: `default`, `realtime`, `safety`, `kernel`.
    pub profile: String,
    pub needs: Vec<Need>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestError(pub String);

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "manifest: {}", self.0)
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    pub fn new(edition: u32, profile: &str) -> Manifest {
        Manifest {
            abi_major: crate::layout::ABI_MAJOR,
            abi_minor: crate::layout::ABI_MINOR,
            edition,
            profile: profile.to_string(),
            needs: vec![],
        }
    }

    pub fn print(&self) -> String {
        let mut s = format!(
            "oxi-manifest {MANIFEST_FORMAT}\nabi {}.{}\nedition {}\nprofile {}\n",
            self.abi_major, self.abi_minor, self.edition, self.profile
        );
        for n in &self.needs {
            let _ = writeln!(s, "needs {}@{} {}", n.module, n.version, n.interface);
        }
        s
    }

    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let e = |m: String| Err(ManifestError(m));
        let mut lines = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'));
        match lines.next() {
            Some(l) if l == format!("oxi-manifest {MANIFEST_FORMAT}") => {}
            other => {
                return e(format!(
                    "expected `oxi-manifest {MANIFEST_FORMAT}`, found {other:?}"
                ))
            }
        }
        let (mut abi, mut edition, mut profile) = (None, None, None);
        let mut needs: Vec<Need> = Vec::new();
        for l in lines {
            let parts: Vec<&str> = l.split_whitespace().collect();
            match parts.as_slice() {
                ["abi", v] => {
                    let bad = || ManifestError(format!("bad abi `{v}`"));
                    let (a, b) = v.split_once('.').ok_or_else(bad)?;
                    let pa = u16::try_from(canon_u32(a).ok_or_else(bad)?).map_err(|_| bad())?;
                    let pb = u16::try_from(canon_u32(b).ok_or_else(bad)?).map_err(|_| bad())?;
                    if abi.replace((pa, pb)).is_some() {
                        return e("duplicate `abi`".into());
                    }
                }
                ["edition", v] => {
                    let n =
                        canon_u32(v).ok_or_else(|| ManifestError(format!("bad edition `{v}`")))?;
                    if edition.replace(n).is_some() {
                        return e("duplicate `edition`".into());
                    }
                }
                ["profile", v] => {
                    if !PROFILES.contains(v) {
                        return e(format!(
                            "unknown profile `{v}` (expected one of {PROFILES:?})"
                        ));
                    }
                    if profile.replace((*v).to_string()).is_some() {
                        return e("duplicate `profile`".into());
                    }
                }
                ["needs", m, i] => {
                    let (module, ver) = m
                        .rsplit_once('@')
                        .ok_or_else(|| ManifestError(format!("bad needs `{m}`")))?;
                    let name = module.strip_prefix(crate::exports::HOST_NAMESPACE_PREFIX);
                    if !name.is_some_and(is_name) {
                        return e(format!("needs module `{module}` must be `host:<name>` with a non-empty identifier name"));
                    }
                    let version = canon_u32(ver)
                        .filter(|v| *v >= 1)
                        .ok_or_else(|| ManifestError(format!("bad version in `{m}`")))?;
                    if !is_name(i) {
                        return e(format!("bad interface name `{i}`"));
                    }
                    if needs
                        .iter()
                        .any(|n| n.module == module || n.interface == *i)
                    {
                        return e(format!("duplicate need `{m} {i}` (one capability per module and per interface name)"));
                    }
                    if needs.len() >= MAX_NEEDS {
                        return e(format!("more than {MAX_NEEDS} capabilities needed"));
                    }
                    needs.push(Need {
                        module: module.to_string(),
                        version,
                        interface: (*i).to_string(),
                    });
                }
                _ => return e(format!("unknown or malformed directive `{l}`")),
            }
        }
        let (abi_major, abi_minor) = abi.ok_or_else(|| ManifestError("missing `abi`".into()))?;
        Ok(Manifest {
            abi_major,
            abi_minor,
            edition: edition.ok_or_else(|| ManifestError("missing `edition`".into()))?,
            profile: profile.ok_or_else(|| ManifestError("missing `profile`".into()))?,
            needs,
        })
    }

    /// Host-side compatibility rule: same major, module minor <= host minor.
    pub fn compatible_with_host(&self, host_major: u16, host_minor: u16) -> bool {
        self.abi_major == host_major && self.abi_minor <= host_minor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut m = Manifest::new(2026, "default");
        m.needs.push(Need {
            module: "host:log".into(),
            version: 1,
            interface: "Log".into(),
        });
        m.needs.push(Need {
            module: "host:state".into(),
            version: 1,
            interface: "State".into(),
        });
        let t = m.print();
        assert_eq!(Manifest::parse(&t).unwrap(), m);
        assert_eq!(Manifest::parse(&t).unwrap().print(), t);
    }

    #[test]
    fn rejects_unknown_and_incomplete() {
        assert!(Manifest::parse("oxi-manifest 2\n").is_err());
        assert!(Manifest::parse(
            "oxi-manifest 1\nabi 1.0\nedition 2026\nprofile default\nfrobnicate 1\n"
        )
        .is_err());
        assert!(Manifest::parse("oxi-manifest 1\nabi 1.0\nedition 2026\n").is_err());
        assert!(Manifest::parse(
            "oxi-manifest 1\nabi 1.0\nedition 2026\nprofile x\nneeds fs:x@1 X\n"
        )
        .is_err());
    }

    #[test]
    fn compat() {
        let m = Manifest::new(2026, "default");
        assert!(m.compatible_with_host(1, 0));
        assert!(!m.compatible_with_host(2, 0));
        let mut newer = m.clone();
        newer.abi_minor = 3;
        assert!(!newer.compatible_with_host(1, 2));
    }
}
