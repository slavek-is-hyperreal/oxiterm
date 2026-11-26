use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let rc_dir = manifest_dir.join("../oxiscript-rc");
    println!("cargo:rerun-if-changed=build.rs");
    for rel in [
        "Cargo.toml",
        "build.rs",
        "src/lib.rs",
        "src/heap.rs",
        "src/rt.rs",
        "src/wasm.rs",
    ] {
        println!("cargo:rerun-if-changed={}", rc_dir.join(rel).display());
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let target_dir = out_dir.join("rc-target");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());

    let mut cmd = Command::new(cargo);
    cmd.args([
        "build",
        "--manifest-path",
        rc_dir.join("Cargo.toml").to_str().unwrap(),
        "--target",
        "wasm32-unknown-unknown",
        "--release",
        "--lib",
        "--target-dir",
        target_dir.to_str().unwrap(),
    ]);
    // Avoid leaking outer clippy/target/workspace flags into the guest wasm32 build.
    for var in [
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CLIPPY_ARGS",
        "CARGO_ENCODED_RUSTFLAGS",
    ] {
        cmd.env_remove(var);
    }
    // Pin the exact Core Wasm ABI 1.0 feature set (no trunc_sat, reference-types, simd, tail-call).
    cmd.env(
        "RUSTFLAGS",
        "-C target-feature=-nontrapping-fptoint,-reference-types,-simd128,-tail-call,+bulk-memory,+sign-ext,+multivalue",
    );

    let status = cmd
        .status()
        .expect("failed to invoke cargo to build oxiscript-rc for wasm32-unknown-unknown");
    assert!(
        status.success(),
        "building oxiscript-rc for wasm32-unknown-unknown failed (is the target installed?)"
    );

    let built = target_dir
        .join("wasm32-unknown-unknown")
        .join("release")
        .join("oxiscript_rc.wasm");
    let dest = out_dir.join("oxiscript_rc.wasm");
    std::fs::copy(&built, &dest).unwrap_or_else(|e| {
        panic!("failed to copy {} to {}: {e}", built.display(), dest.display())
    });
}
