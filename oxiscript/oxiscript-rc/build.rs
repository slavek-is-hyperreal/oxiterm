// The runtime is iterative and uses almost no shadow stack, so the wasm
// artefact asks for a 64 KiB one (Rust's default is 1 MiB, which would make
// the initial linear memory ~17 pages for nothing).
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-link-arg-cdylib=-zstack-size=65536");
        println!("cargo:rustc-link-arg-cdylib=--export=__heap_base");
    }
}
