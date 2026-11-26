use std::path::{Path, PathBuf};
use std::process::Command;

fn emit_rerun_dir(dir: &Path) {
    println!("cargo:rerun-if-changed={}", dir.display());
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                emit_rerun_dir(&p);
            } else {
                println!("cargo:rerun-if-changed={}", p.display());
            }
        }
    }
}

fn find_upward(start: &Path, filename: &str) -> Option<PathBuf> {
    let mut cur = Some(start);
    while let Some(d) = cur {
        let cand = d.join(filename);
        if cand.exists() {
            return Some(cand);
        }
        cur = d.parent();
    }
    None
}

fn rustc_info() -> (Option<String>, Option<String>) {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let commit = Command::new(&rustc)
        .arg("-vV")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("commit-hash: ").map(|h| h.trim().to_string()))
        });
    let sysroot = Command::new(&rustc)
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    (commit, sysroot)
}

fn encoded_rustflags(rc_dir: &Path, workspace_root: &Path) -> String {
    let mut flags: Vec<String> = vec![
        "-C".into(),
        "target-feature=-nontrapping-fptoint,-reference-types,-simd128,-tail-call,+bulk-memory,+sign-ext,+multivalue".into(),
        "-C".into(),
        "panic=abort".into(),
    ];

    // Remap all builder-specific paths (HOME, CARGO_HOME, registry, workspace/source dir,
    // rustc sysroot, and `/rustc/<commit-hash>`) so compiled `.wasm` artifacts are
    // reproducible and leak neither builder paths nor the rustc commit hash (BL04).
    // Note: rustc applies the LAST matching `--remap-path-prefix`, so list broad prefixes
    // first and more specific prefixes last.
    let mut add_remap = |from: &str, to: &str| {
        if !from.is_empty() {
            flags.push("--remap-path-prefix".into());
            flags.push(format!("{from}={to}"));
        }
    };

    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok());
    if let Some(ref h) = home {
        add_remap(h, "/h");
    }

    if let Ok(ch) = std::env::var("CARGO_HOME") {
        let ch_path = PathBuf::from(&ch);
        add_remap(&ch, "/c");
        add_remap(&ch_path.join("registry").join("src").to_string_lossy(), "/reg");
    } else if let Some(ref h) = home {
        let ch_path = PathBuf::from(h).join(".cargo");
        add_remap(&ch_path.to_string_lossy(), "/c");
        add_remap(&ch_path.join("registry").join("src").to_string_lossy(), "/reg");
    }

    add_remap(&workspace_root.to_string_lossy(), "/src");
    if let Ok(canon) = std::fs::canonicalize(workspace_root) {
        add_remap(&canon.to_string_lossy(), "/src");
    }

    add_remap(&rc_dir.to_string_lossy(), "/rc");
    if let Ok(canon) = std::fs::canonicalize(rc_dir) {
        add_remap(&canon.to_string_lossy(), "/rc");
    }

    let (commit, sysroot) = rustc_info();
    if let Some(sr) = sysroot {
        add_remap(&sr, "/rust");
    }
    if let Some(hash) = commit {
        add_remap(&format!("/rustc/{hash}"), "/rust");
    }

    flags.join("\x1f")
}

/// Scrub precompiled `libcore` `/rustc/<40-hex-hash>/` panic-location prefixes
/// (which `--remap-path-prefix` cannot reach in rustup's prebuilt `libcore.rlib`)
/// with a deterministic fixed-length replacement so no rustc commit hash or
/// `"rustc/"` path survives in `.wasm` artifacts. Because `oxiscript-rc`'s
/// `#[panic_handler]` ignores `PanicInfo`, these bytes are never read at runtime,
/// and keeping exact byte length preserves all Wasm section offsets and pointers.
fn scrub_precompiled_rustc_paths(wasm: &mut [u8]) {
    const PREFIX: &[u8] = b"/rustc/";
    const REPL: &[u8] = b"/rust/core/____________________________________/";
    assert_eq!(REPL.len(), PREFIX.len() + 40 + 1);
    let mut i = 0;
    while i + REPL.len() <= wasm.len() {
        if &wasm[i..i + PREFIX.len()] == PREFIX
            && wasm[i + PREFIX.len() + 40] == b'/'
            && wasm[i + PREFIX.len()..i + PREFIX.len() + 40]
                .iter()
                .all(u8::is_ascii_hexdigit)
        {
            wasm[i..i + REPL.len()].copy_from_slice(REPL);
            i += REPL.len();
        } else {
            i += 1;
        }
    }
}

fn build_rc(
    rc_dir: &Path,
    target_dir: &Path,
    out_dir: &Path,
    encoded_flags: &str,
    debug_profile: bool,
) {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut cmd = Command::new(cargo);
    cmd.args([
        "build",
        "--manifest-path",
        rc_dir.join("Cargo.toml").to_str().unwrap(),
        "--target",
        "wasm32-unknown-unknown",
        "--lib",
        "--locked",
        "--target-dir",
        target_dir.to_str().unwrap(),
    ]);
    if !debug_profile {
        cmd.arg("--release");
    }
    if let Ok(off) = std::env::var("CARGO_NET_OFFLINE") {
        if !off.is_empty() && off != "false" && off != "0" {
            cmd.arg("--offline");
        }
    }
    // Avoid leaking outer clippy/target/workspace flags into the guest wasm32 build.
    for var in [
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CLIPPY_ARGS",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("CARGO_ENCODED_RUSTFLAGS", encoded_flags);
    if debug_profile {
        // Keep `debug_assertions = true` (stale_object trap 65 + 0xDD poisoning)
        // while keeping the wasm compact and free of DWARF debug paths.
        cmd.env("CARGO_PROFILE_DEV_PANIC", "abort");
        cmd.env("CARGO_PROFILE_DEV_OPT_LEVEL", "1");
        cmd.env("CARGO_PROFILE_DEV_DEBUG_ASSERTIONS", "true");
        cmd.env("CARGO_PROFILE_DEV_DEBUG", "0");
        cmd.env("CARGO_PROFILE_DEV_STRIP", "symbols");
    } else {
        cmd.env("CARGO_PROFILE_RELEASE_PANIC", "abort");
        cmd.env("CARGO_PROFILE_RELEASE_OPT_LEVEL", "s");
        cmd.env("CARGO_PROFILE_RELEASE_LTO", "true");
        cmd.env("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "1");
        cmd.env("CARGO_PROFILE_RELEASE_STRIP", "symbols");
    }

    let status = cmd
        .status()
        .expect("failed to invoke cargo to build oxiscript-rc for wasm32-unknown-unknown");
    assert!(
        status.success(),
        "building oxiscript-rc for wasm32-unknown-unknown (debug={debug_profile}) failed (is the target installed?)"
    );

    let subdir = if debug_profile { "debug" } else { "release" };
    let built = target_dir
        .join("wasm32-unknown-unknown")
        .join(subdir)
        .join("oxiscript_rc.wasm");
    let dest_name = if debug_profile {
        "oxiscript_rc_debug.wasm"
    } else {
        "oxiscript_rc.wasm"
    };
    let dest = out_dir.join(dest_name);
    let mut bytes = std::fs::read(&built).unwrap_or_else(|e| {
        panic!("failed to read {}: {e}", built.display())
    });
    scrub_precompiled_rustc_paths(&mut bytes);
    std::fs::write(&dest, &bytes).unwrap_or_else(|e| {
        panic!("failed to write {}: {e}", dest.display())
    });
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let rc_dir = manifest_dir.join("../oxiscript-rc");
    let workspace_root = find_upward(&manifest_dir, "Cargo.lock")
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| manifest_dir.clone());

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", rc_dir.join("Cargo.toml").display());
    println!("cargo:rerun-if-changed={}", rc_dir.join("build.rs").display());
    emit_rerun_dir(&rc_dir.join("src"));
    if let Some(lock) = find_upward(&manifest_dir, "Cargo.lock") {
        println!("cargo:rerun-if-changed={}", lock.display());
    }
    for tc in ["rust-toolchain.toml", "rust-toolchain"] {
        if let Some(p) = find_upward(&manifest_dir, tc) {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }
    for env_var in [
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_NET_OFFLINE",
        "CARGO_HOME",
    ] {
        println!("cargo:rerun-if-env-changed={env_var}");
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let target_dir = out_dir.join("rc-target");
    let encoded_flags = encoded_rustflags(&rc_dir, &workspace_root);

    build_rc(&rc_dir, &target_dir, &out_dir, &encoded_flags, false);
    if std::env::var_os("CARGO_FEATURE_DEBUG_RUNTIME").is_some() {
        build_rc(&rc_dir, &target_dir, &out_dir, &encoded_flags, true);
    }
}
