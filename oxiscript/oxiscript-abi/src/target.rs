//! Compilation targets (`oxi build --target device|web|server`).
//!
//! A target is **not** a profile. Profiles (`realtime`, `safety`, `kernel`)
//! restrict what a *program* may do and are checked on the source. A target
//! chooses which Wasm features the *code generator* may use and the *host*
//! must provide. The language and its semantics are identical on every
//! target; only the lowering differs. A feature that would change observable
//! behaviour (GC, relaxed-simd, threads) belongs to no target.
//!
//! Targets are totally ordered: `Device < Web < Server`; each one's feature
//! set contains the previous one's. Spec: `docs/oxiscript/abi.md` 3.4.

use std::fmt;

/// A Wasm feature beyond the Device (Core Wasm) set that a module may need.
/// The Device base set (MVP, multi-value, bulk-memory, sign-extension,
/// mutable globals) is implied everywhere and never listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Feature {
    Simd128,
    /// Standard exception handling with `exnref` (Wasm 3.0).
    Exceptions,
    TailCall,
    ExtendedConst,
    Memory64,
    MultiMemory,
}

impl Feature {
    pub const ALL: [Feature; 6] = [
        Feature::Simd128,
        Feature::Exceptions,
        Feature::TailCall,
        Feature::ExtendedConst,
        Feature::Memory64,
        Feature::MultiMemory,
    ];

    /// The name used in the manifest `features` line.
    pub fn name(self) -> &'static str {
        match self {
            Feature::Simd128 => "simd128",
            Feature::Exceptions => "exceptions",
            Feature::TailCall => "tail-call",
            Feature::ExtendedConst => "extended-const",
            Feature::Memory64 => "memory64",
            Feature::MultiMemory => "multi-memory",
        }
    }

    pub fn parse(s: &str) -> Option<Feature> {
        Feature::ALL.into_iter().find(|f| f.name() == s)
    }

    /// The least target that provides this feature.
    pub fn min_target(self) -> Target {
        match self {
            Feature::Simd128 | Feature::Exceptions | Feature::TailCall | Feature::ExtendedConst => {
                Target::Web
            }
            Feature::Memory64 | Feature::MultiMemory => Target::Server,
        }
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    /// Core Wasm for wasmi on ESP32 and small devices. The default.
    #[default]
    Device,
    /// What every evergreen browser ships.
    Web,
    /// wasmtime / a server-side wasmi.
    Server,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::Device, Target::Web, Target::Server];

    pub fn name(self) -> &'static str {
        match self {
            Target::Device => "device",
            Target::Web => "web",
            Target::Server => "server",
        }
    }

    pub fn parse(s: &str) -> Option<Target> {
        Target::ALL.into_iter().find(|t| t.name() == s)
    }

    /// The extra features (beyond the Device base set) this target allows.
    pub fn features(self) -> Vec<Feature> {
        Feature::ALL
            .into_iter()
            .filter(|f| f.min_target() <= self)
            .collect()
    }

    pub fn allows(self, f: Feature) -> bool {
        f.min_target() <= self
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_are_nested_and_named() {
        assert!(Target::Device.features().is_empty());
        assert_eq!(Target::Web.features().len(), 4);
        assert_eq!(Target::Server.features().len(), 6);
        for t in Target::ALL {
            assert_eq!(Target::parse(t.name()), Some(t));
        }
        for f in Feature::ALL {
            assert_eq!(Feature::parse(f.name()), Some(f));
        }
        assert!(Target::Device < Target::Web && Target::Web < Target::Server);
        assert!(!Target::Web.allows(Feature::Memory64));
        assert_eq!(Target::default(), Target::Device);
    }
}
