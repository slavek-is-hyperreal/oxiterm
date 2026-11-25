#![allow(clippy::pedantic)]
mod common;

use arbitrary::{Arbitrary, Unstructured};
use common::*;
use oxiscript_runtime::{Grants, Limits};

/// Deterministic pseudo-random bytes (no clock, no RNG crate).
fn bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 24) as u8
        })
        .collect()
}

/// Random *valid* modules (wasm-smith) with the manifest appended: loading
/// must never panic or hang; they lack the ABI exports, so they must all be
/// rejected with a reason code, never accepted.
#[test]
fn random_valid_modules_are_rejected_cleanly() {
    let e = env_with(Limits::default());
    let mut rejected = 0;
    for seed in 0..300u64 {
        let raw = bytes(seed, 2048);
        let mut u = Unstructured::new(&raw);
        let Ok(m) = wasm_smith::Module::arbitrary(&mut u) else {
            continue;
        };
        let mut wasm = m.to_bytes();
        append_custom(
            &mut wasm,
            "oxi.manifest",
            Fixture::new(&[]).manifest_text().as_bytes(),
        );
        // wasmparser (independent of wasmi's copy) must not crash either
        let _ = wasmparser::Validator::new().validate_all(&wasm);
        match e.host.instantiate(&wasm, Grants::all()) {
            Ok(_) => panic!("random module accepted (seed {seed})"),
            Err(err) => {
                assert!(err.code().starts_with("OXS-"), "{err}");
                rejected += 1;
            }
        }
    }
    assert!(rejected > 100, "only {rejected} modules were generated");
}

/// Mutated copies of a conforming module: every outcome is Ok or a coded
/// error; Ok instances are driven under a tight fuel budget without hanging.
#[test]
fn mutated_conforming_module_never_panics() {
    let e = env_with(Limits {
        fuel_per_call: 100_000,
        ..Limits::default()
    });
    let good = app().bytes();
    let (mut ok, mut bad) = (0, 0);
    for seed in 0..1500u64 {
        let r = bytes(seed, 12);
        let mut w = good.clone();
        let n = 1 + (r[0] % 3) as usize;
        for k in 0..n {
            let pos = (u32::from_le_bytes([r[1 + k], r[4 + k], r[7 + k], 0]) as usize) % w.len();
            w[pos] ^= 1 << (r[10] % 8);
        }
        match e.host.instantiate(&w, Grants::all()) {
            Ok(mut rt) => {
                ok += 1;
                for name in rt.user_exports() {
                    if let Some((p, _)) = rt.export_type(&name) {
                        if p.is_empty() {
                            let _ = rt.call_export(&name, &[]);
                        }
                    }
                }
            }
            Err(err) => {
                assert!(err.code().starts_with("OXS-"));
                bad += 1;
            }
        }
    }
    assert!(ok + bad == 1500 && bad > 0);
}
