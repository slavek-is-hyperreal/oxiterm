//! Trap kinds. A trap is a Wasm `unreachable` executed after the runtime has
//! stored the kind code in the exported mutable global `oxi_trap_kind`, so the
//! host can report *why* the instance stopped (a bare Wasm trap carries no
//! reason).

/// Why an instance trapped. Codes are stable forever; 0 means "no trap".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TrapKind {
    /// Reached code the checker proved impossible (`unreachable`).
    Unreachable = 1,
    /// Integer overflow in a trapping (default) arithmetic or conversion op.
    Overflow = 2,
    DivByZero = 3,
    /// Array / string index out of bounds, shift amount out of range.
    Bounds = 4,
    /// Failed checked downcast.
    BadCast = 5,
    /// Null dereference that the type system could not exclude (host-boundary
    /// misuse only; checked code cannot reach it).
    NullDeref = 6,
    OutOfMemory = 7,
    /// Failed `assert`.
    Assert = 8,
    /// Invalid UTF-8, or a slice not on a character boundary.
    BadUtf8 = 9,
    /// Explicit `panic(...)`.
    Panic = 10,
}

impl TrapKind {
    pub const ALL: [TrapKind; 10] = [
        TrapKind::Unreachable,
        TrapKind::Overflow,
        TrapKind::DivByZero,
        TrapKind::Bounds,
        TrapKind::BadCast,
        TrapKind::NullDeref,
        TrapKind::OutOfMemory,
        TrapKind::Assert,
        TrapKind::BadUtf8,
        TrapKind::Panic,
    ];

    pub fn code(self) -> u32 {
        self as u32
    }

    pub fn from_code(code: u32) -> Option<TrapKind> {
        Self::ALL.iter().copied().find(|k| k.code() == code)
    }

    /// Text-form name (used by the IR text form and docs).
    pub fn name(self) -> &'static str {
        match self {
            TrapKind::Unreachable => "unreachable",
            TrapKind::Overflow => "overflow",
            TrapKind::DivByZero => "div_by_zero",
            TrapKind::Bounds => "bounds",
            TrapKind::BadCast => "bad_cast",
            TrapKind::NullDeref => "null_deref",
            TrapKind::OutOfMemory => "out_of_memory",
            TrapKind::Assert => "assert",
            TrapKind::BadUtf8 => "bad_utf8",
            TrapKind::Panic => "panic",
        }
    }

    pub fn from_name(name: &str) -> Option<TrapKind> {
        Self::ALL.iter().copied().find(|k| k.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_codes_round_trip() {
        for k in TrapKind::ALL {
            assert_eq!(TrapKind::from_name(k.name()), Some(k));
            assert_eq!(TrapKind::from_code(k.code()), Some(k));
        }
        assert_eq!(TrapKind::from_code(0), None);
    }
}
