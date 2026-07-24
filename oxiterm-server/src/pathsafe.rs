use std::path::{Path, PathBuf};

pub use oxiterm_proto::pathsafe::is_within_base;

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

    #[test]
    fn test_7_media_same_directory_allowed() {
        let temp = std::env::temp_dir().join("media_test_7");
        std::fs::create_dir_all(&temp).unwrap();
        let asset = temp.join("image.png");
        std::fs::write(&asset, b"img").unwrap();

        assert!(is_within_base(&temp, &asset));

        let _ = std::fs::remove_file(asset);
        let _ = std::fs::remove_dir(temp);
    }

    #[test]
    fn test_8_media_relative_within_app_base_allowed() {
        let app_base = std::env::temp_dir().join("media_test_8");
        let assets = app_base.join("assets");
        let demos = app_base.join("demos");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(&demos).unwrap();

        let asset = assets.join("mascot.svg");
        std::fs::write(&asset, b"mascot").unwrap();

        let rel_path = demos.join("../assets/mascot.svg");
        assert!(is_within_base(&app_base, &rel_path));

        let _ = std::fs::remove_file(asset);
        let _ = std::fs::remove_dir_all(app_base);
    }

    #[test]
    fn test_9_media_relative_escaping_app_base_blocked() {
        let app_base = std::env::temp_dir().join("media_test_9");
        let demos = app_base.join("demos");
        std::fs::create_dir_all(&demos).unwrap();

        let outside_dir = std::env::temp_dir().join("media_test_9_outside");
        std::fs::create_dir_all(&outside_dir).unwrap();
        let secret = outside_dir.join("secret.txt");
        std::fs::write(&secret, b"secret").unwrap();

        let rel_path = demos.join("../../media_test_9_outside/secret.txt");
        assert!(!is_within_base(&app_base, &rel_path));

        let _ = std::fs::remove_file(secret);
        let _ = std::fs::remove_dir(outside_dir);
        let _ = std::fs::remove_dir_all(app_base);
    }

    #[test]
    fn test_10_same_filename_resolves_locally() {
        let app_base = std::env::temp_dir().join("media_test_10");
        let demos = app_base.join("demos");
        std::fs::create_dir_all(&demos).unwrap();

        let base_file = app_base.join("icon.png");
        let demo_file = demos.join("icon.png");
        std::fs::write(&base_file, b"base_icon").unwrap();
        std::fs::write(&demo_file, b"demo_icon").unwrap();

        let resolved_demo = demos.join("icon.png");
        let resolved_base = app_base.join("icon.png");

        assert_eq!(resolved_demo.canonicalize().unwrap(), demo_file.canonicalize().unwrap());
        assert_ne!(resolved_demo.canonicalize().unwrap(), resolved_base.canonicalize().unwrap());

        let _ = std::fs::remove_dir_all(app_base);
    }

    #[test]
    fn test_11_symlink_escaping_app_base_blocked() {
        #[cfg(unix)]
        {
            let app_base = std::env::temp_dir().join("media_test_11");
            let demos = app_base.join("demos");
            std::fs::create_dir_all(&demos).unwrap();

            let outside_dir = std::env::temp_dir().join("media_test_11_outside");
            std::fs::create_dir_all(&outside_dir).unwrap();
            let secret = outside_dir.join("shadow");
            std::fs::write(&secret, b"shadow_data").unwrap();

            let symlink_path = demos.join("sym_shadow");
            let _ = std::os::unix::fs::symlink(&secret, &symlink_path);

            assert!(!is_within_base(&app_base, &symlink_path));

            let _ = std::fs::remove_file(symlink_path);
            let _ = std::fs::remove_file(secret);
            let _ = std::fs::remove_dir(outside_dir);
            let _ = std::fs::remove_dir_all(app_base);
        }
    }

    #[test]
    fn test_12_no_app_base_dir_denies() {
        let app_base: Option<&Path> = None;
        let target = std::env::temp_dir().join("some_file.txt");
        let is_safe = if let Some(base) = app_base {
            is_within_base(base, &target)
        } else {
            false
        };
        assert!(!is_safe);
    }

    #[test]
    fn test_13_absolute_path_outside_base_blocked() {
        let app_base = std::env::temp_dir().join("media_test_13");
        std::fs::create_dir_all(&app_base).unwrap();

        let abs_path = PathBuf::from("/etc/passwd");
        assert!(!is_within_base(&app_base, &abs_path));

        let _ = std::fs::remove_dir_all(app_base);
    }
}
