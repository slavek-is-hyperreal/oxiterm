//! The v0 workload set and the checksum rule (a mismatch between languages fails the run).

use std::path::PathBuf;

/// Which track a workload belongs to (methodology R01-R03).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    /// Identical algorithm in every language, no library shortcuts.
    Base,
    /// Each language's standard-library data structure (tagged `stdlib`).
    Stdlib,
}

impl Track {
    pub fn as_str(self) -> &'static str {
        match self {
            Track::Base => "base",
            Track::Stdlib => "stdlib",
        }
    }
}

/// Who chose the workload and where the non-OxiScript code comes from (verification A2/A3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Are We Fast Yet: the JavaScript twin is the UPSTREAM file, vendored verbatim and pinned.
    /// The Rust, C and `OxiScript` twins are our ports (no upstream exists). Headline group.
    Awfy,
    /// Our own workload, all twins written here. Reported separately, never a headline.
    Own,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Awfy => "awfy",
            Origin::Own => "own",
        }
    }
}

pub struct Workload {
    pub id: &'static str,
    pub origin: Origin,
    pub title: &'static str,
    pub track: Track,
    /// Problem size of the `server` profile (tuned so one wasmi iteration is roughly 50-200 ms).
    pub n: i32,
    /// Problem size used by the smoke tests and `--quick`.
    pub n_quick: i32,
    /// There is a C twin (the stdlib-track map has none: C has no standard hash map).
    pub has_c: bool,
    /// What `n` means.
    pub n_meaning: &'static str,
}

pub const WORKLOADS: &[Workload] = &[
    Workload { id: "awfy_mandelbrot", origin: Origin::Awfy, title: "AWFY Mandelbrot", track: Track::Base, n: 500, n_quick: 50, has_c: true, n_meaning: "image size (upstream verifies 500 -> 191)" },
    Workload { id: "awfy_sieve", origin: Origin::Awfy, title: "AWFY Sieve", track: Track::Base, n: 500, n_quick: 10, has_c: true, n_meaning: "sieves of 5000 (669 primes each)" },
    Workload { id: "sort", origin: Origin::Own, title: "merge sort", track: Track::Base, n: 60_000, n_quick: 500, has_c: true, n_meaning: "i32 keys" },
    Workload { id: "map_set", origin: Origin::Own, title: "Map/Set churn", track: Track::Stdlib, n: 20_000, n_quick: 200, has_c: false, n_meaning: "inserts, lookups, removals each" },
    Workload { id: "string_template", origin: Origin::Own, title: "string template", track: Track::Base, n: 12_000, n_quick: 100, has_c: true, n_meaning: "rows" },
    Workload { id: "json_tokenize", origin: Origin::Own, title: "JSON tokenizer", track: Track::Base, n: 6_000, n_quick: 50, has_c: true, n_meaning: "records (about 80 bytes each)" },
    Workload { id: "tree", origin: Origin::Own, title: "tree alloc/walk", track: Track::Base, n: 13, n_quick: 6, has_c: true, n_meaning: "tree depth (8 trees)" },
    Workload { id: "matmul", origin: Origin::Own, title: "matmul f64", track: Track::Base, n: 100, n_quick: 12, has_c: true, n_meaning: "matrix dimension" },
];

pub fn find(id: &str) -> Option<&'static Workload> {
    WORKLOADS.iter().find(|w| w.id == id)
}

/// `bench/oxiperf` of the repository this crate was built from.
pub fn bench_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bench/oxiperf")
}

pub fn suite_file(id: &str, name: &str) -> PathBuf {
    bench_root().join("suites").join(id).join(name)
}

pub fn oxs_source(id: &str) -> String {
    let p = suite_file(id, "main.oxs");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// A checksum disagreement between two executors of the same workload.
#[derive(Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub expected: u32,
    pub expected_from: String,
    pub got: u32,
    pub got_from: String,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "checksum mismatch: {} gave {} but {} gave {}", self.expected_from, self.expected, self.got_from, self.got)
    }
}

/// Parse and validate a child record's `checksum` field as an exact non-negative `u32` integer.
/// Returns `None` for missing, null, negative, fractional, string, boolean, or out-of-range values.
pub fn parse_u32_checksum(v: &serde_json::Value) -> Option<u32> {
    v.get("checksum")
        .and_then(serde_json::Value::as_u64)
        .and_then(|c| u32::try_from(c).ok())
}

/// All executors must agree. Returns the common checksum or the first disagreement.
pub fn verify_checksums(results: &[(String, u32)]) -> Result<u32, Mismatch> {
    let (first_from, first) = results.first().expect("at least one result").clone();
    for (from, c) in &results[1..] {
        if *c != first {
            return Err(Mismatch { expected: first, expected_from: first_from, got: *c, got_from: from.clone() });
        }
    }
    Ok(first)
}

/// One run's checksum outcome: either a validated `u32` or the reason the run/checksum was invalid.
pub type ChecksumEntry = (String, Result<u32, String>);

/// Verify that every run for a workload produced a valid `u32` checksum and that all agree.
pub fn verify_run_checksums(results: &[ChecksumEntry]) -> Result<u32, String> {
    if results.is_empty() {
        return Err("no checksums recorded".into());
    }
    let mut valid = Vec::with_capacity(results.len());
    for (from, r) in results {
        match r {
            Ok(c) => valid.push((from.clone(), *c)),
            Err(reason) => return Err(format!("{from}: {reason}")),
        }
    }
    verify_checksums(&valid).map_err(|m| m.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_checksums_pass() {
        let r = vec![("a".to_string(), 7), ("b".to_string(), 7), ("c".to_string(), 7)];
        assert_eq!(verify_checksums(&r), Ok(7));
    }

    #[test]
    fn mismatch_is_detected_and_named() {
        let r = vec![("rust".to_string(), 7), ("node".to_string(), 7), ("oxs".to_string(), 8)];
        let e = verify_checksums(&r).unwrap_err();
        assert_eq!(e.got_from, "oxs");
        assert_eq!(e.expected_from, "rust");
        assert!(e.to_string().contains("oxs gave 8"));
    }

    #[test]
    fn every_workload_has_all_sources() {
        for w in WORKLOADS {
            for f in ["main.oxs", "main.js", "main.rs"] {
                assert!(suite_file(w.id, f).exists(), "{} {}", w.id, f);
            }
            assert_eq!(suite_file(w.id, "main.c").exists(), w.has_c, "{}", w.id);
            if w.origin == Origin::Awfy {
                assert!(suite_file(w.id, "upstream/PIN").exists(), "{}: vendored upstream JS must carry its PIN file", w.id);
            }
        }
    }

    #[test]
    fn parse_u32_checksum_accepts_only_u32_integers() {
        assert_eq!(parse_u32_checksum(&serde_json::json!({"checksum": 0})), Some(0));
        assert_eq!(parse_u32_checksum(&serde_json::json!({"checksum": 42})), Some(42));
        assert_eq!(parse_u32_checksum(&serde_json::json!({"checksum": u32::MAX})), Some(u32::MAX));
        for bad in [
            serde_json::json!({}),
            serde_json::json!({"checksum": null}),
            serde_json::json!({"checksum": -7}),
            serde_json::json!({"checksum": 1.5}),
            serde_json::json!({"checksum": 1.0}),
            serde_json::json!({"checksum": "42"}),
            serde_json::json!({"checksum": true}),
            serde_json::json!({"checksum": u64::from(u32::MAX) + 1}),
        ] {
            assert_eq!(parse_u32_checksum(&bad), None, "must reject {bad}");
        }
    }

    #[test]
    fn verify_run_checksums_rejects_empty_and_invalid_entries() {
        assert!(verify_run_checksums(&[]).is_err());
        let bad = vec![("rust".to_string(), Ok(7)), ("node".to_string(), Err("missing checksum".into()))];
        let err = verify_run_checksums(&bad).unwrap_err();
        assert!(err.contains("node") && err.contains("missing checksum"));
        let ok = vec![("rust".to_string(), Ok(7)), ("node".to_string(), Ok(7))];
        assert_eq!(verify_run_checksums(&ok), Ok(7));
    }
}
