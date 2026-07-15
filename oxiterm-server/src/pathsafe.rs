use std::path::{Path, PathBuf};

/// Checks if `target` path is strictly within the `base` directory, preventing path traversal attacks.
///
/// Return false if canonicalization fails (e.g. file does not exist) or if target is outside base.
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

/// Resolves mobile variant of the source path if it exists on disk, else returns the original path.
pub fn resolve_variant(source_path: &Path, is_mobile: bool) -> PathBuf {
    if is_mobile {
        if let Some(stem) = source_path.file_stem().and_then(|s| s.to_str()) {
            if let Some(ext) = source_path.extension().and_then(|e| e.to_str()) {
                let mobile_name = format!("{}_mobile.{}", stem, ext);
                if let Some(parent) = source_path.parent() {
                    let mobile_path = parent.join(mobile_name);
                    if mobile_path.exists() {
                        return mobile_path;
                    }
                }
            }
        }
    }
    source_path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_1_target_inside_base() {
        let temp = std::env::temp_dir();
        let base = temp.join("pathsafe_test_base");
        std::fs::create_dir_all(&base).unwrap();
        
        let target = base.join("valid.thtml");
        std::fs::write(&target, b"test").unwrap();
        
        assert!(is_within_base(&base, &target));
        
        let _ = std::fs::remove_file(target);
        let _ = std::fs::remove_dir(base);
    }

    #[test]
    fn test_2_target_escaping_base() {
        let temp = std::env::temp_dir();
        let base = temp.join("pathsafe_test_base_esc");
        std::fs::create_dir_all(&base).unwrap();
        
        let target = temp.join("escape.thtml");
        std::fs::write(&target, b"test").unwrap();
        
        // target is outside base, even with relative components
        let rel_target = base.join("../escape.thtml");
        assert!(!is_within_base(&base, &rel_target));
        
        let _ = std::fs::remove_file(target);
        let _ = std::fs::remove_dir(base);
    }

    #[test]
    fn test_3_non_existent_target() {
        let temp = std::env::temp_dir();
        let base = temp.join("pathsafe_test_base_non");
        std::fs::create_dir_all(&base).unwrap();
        
        let target = base.join("nonexistent.thtml");
        assert!(!is_within_base(&base, &target));
        
        let _ = std::fs::remove_dir(base);
    }

    #[test]
    fn test_4_resolve_variant_mobile_exists() {
        let temp = std::env::temp_dir();
        let app_file = temp.join("app.thtml");
        let mobile_file = temp.join("app_mobile.thtml");
        std::fs::write(&app_file, b"app").unwrap();
        std::fs::write(&mobile_file, b"mobile").unwrap();

        let resolved = resolve_variant(&app_file, true);
        assert_eq!(resolved, mobile_file);

        let _ = std::fs::remove_file(app_file);
        let _ = std::fs::remove_file(mobile_file);
    }

    #[test]
    fn test_5_resolve_variant_mobile_not_exists() {
        let temp = std::env::temp_dir();
        let app_file = temp.join("app_no_mobile.thtml");
        std::fs::write(&app_file, b"app").unwrap();

        let resolved = resolve_variant(&app_file, true);
        assert_eq!(resolved, app_file);

        let _ = std::fs::remove_file(app_file);
    }

    #[test]
    fn test_6_resolve_variant_not_mobile() {
        let temp = std::env::temp_dir();
        let app_file = temp.join("app_test_false.thtml");
        let mobile_file = temp.join("app_test_false_mobile.thtml");
        std::fs::write(&app_file, b"app").unwrap();
        std::fs::write(&mobile_file, b"mobile").unwrap();

        let resolved = resolve_variant(&app_file, false);
        assert_eq!(resolved, app_file);

        let _ = std::fs::remove_file(app_file);
        let _ = std::fs::remove_file(mobile_file);
    }
}
