//! Bundled OxiScript standard library (`std/...`) sources, embedded at build time
//! so the CLI, tests, lowering, and wasm32 browser playground all resolve them
//! identically without touching the filesystem.

/// Known standard library modules: `(canonical_name, source)`.
pub const STD_MODULES: &[(&str, &str)] = &[
    ("std/math", include_str!("../../std/math.oxs")),
    ("std/string", include_str!("../../std/string.oxs")),
    ("std/array", include_str!("../../std/array.oxs")),
    ("std/result", include_str!("../../std/result.oxs")),
    ("std/collections", include_str!("../../std/collections.oxs")),
];

/// Comma-separated list of available std modules for diagnostics.
pub const AVAILABLE_LIST: &str = "std/math, std/string, std/array, std/result, std/collections";

/// Whether `spec` addresses the `std` namespace (`"std"` or `"std/..."`).
pub fn is_std_spec(spec: &str) -> bool {
    spec == "std" || spec.starts_with("std/")
}

/// Look up the bundled source of a `std/...` module.
pub fn source(spec: &str) -> Option<&'static str> {
    STD_MODULES
        .iter()
        .find(|(name, _)| *name == spec)
        .map(|(_, src)| *src)
}

/// Known compiler intrinsic names allowed in `@intrinsic("...")` inside bundled std modules.
pub const KNOWN_INTRINSICS: &[&str] = &[
    "abs",
    "sqrt",
    "floor",
    "ceil",
    "trunc",
    "nearest",
    "min",
    "max",
    "clz",
    "ctz",
    "popcnt",
    "bitcast",
    "str_byte",
    "str_cmp",
    "str_from_utf8",
];

/// Whether `name` is a known compiler intrinsic.
pub fn is_known_intrinsic(name: &str) -> bool {
    KNOWN_INTRINSICS.contains(&name)
}
