use std::path::Path;

/// Checks if `target` path is strictly within the `base` directory, preventing path traversal attacks.
///
/// Returns false if canonicalization fails (e.g. file does not exist) or if target is outside base.
/// Strictly fail-closed.
pub fn is_within_base(base: &Path, target: &Path) -> bool {
    let canonical_base = match base.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let canonical_target = match target.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    canonical_target.starts_with(canonical_base)
}
