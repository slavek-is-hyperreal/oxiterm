//! BAD ACTOR: manifest abuse for `target`/`features` (full ABI modules, so success = "ok").
#![allow(clippy::pedantic, dead_code)]
mod common;

use common::*;
use oxiscript_abi::target::Target;
use oxiscript_runtime::{Grants, Host, HostOptions, Limits};

fn host_for(max: Target) -> Host {
    Host::with_options(Limits::default(), HostOptions { max_target: max, ..HostOptions::default() })
}
fn base() -> String {
    "oxi-manifest 1\nabi 1.0\nedition 2026\nprofile default\n".to_string()
}
fn run(wasm: &[u8], h: Target) -> String {
    match host_for(h).instantiate(wasm, Grants::all()) {
        Ok(_) => "ok".into(),
        Err(e) => e.code().trim_start_matches("OXS-").to_string(),
    }
}
fn with_manifest(m: &str) -> Vec<u8> {
    let mut f = Fixture::new(&[]);
    f.manifest = Some(m.to_string());
    f.bytes()
}
/// manifest bytes (may be non UTF-8) appended as custom section
fn with_manifest_bytes(m: &[u8]) -> Vec<u8> {
    let mut f = Fixture::new(&[]);
    f.manifest = Some(String::new());
    let mut b = f.bytes();
    append_custom(&mut b, "oxi.manifest", m);
    b
}
/// Move the custom section(s) named oxi.manifest to just after the 8 byte header.
fn manifest_first(wasm: &[u8]) -> Vec<u8> {
    use wasmparser::{Parser, Payload};
    let mut sec = vec![];
    let mut rest = vec![];
    let mut last = 8usize;
    for p in Parser::new(0).parse_all(wasm) {
        if let Payload::CustomSection(c) = p.unwrap() {
            if c.name() == "oxi.manifest" {
                // section = id(1) + leb(size) + payload ; find the start by scanning back from range
                let r = c.range();
                let mut start = r.start - 1;
                // walk back over the size LEB and id byte
                while wasm[start - 1] & 0x80 != 0 {
                    start -= 1;
                }
                start -= 1; // id byte
                sec.extend_from_slice(&wasm[start..r.end]);
                rest.extend_from_slice(&wasm[last..start]);
                last = r.end;
            }
        }
    }
    rest.extend_from_slice(&wasm[last..]);
    let mut out = wasm[..8].to_vec();
    out.extend_from_slice(&sec);
    out.extend_from_slice(&rest[..]);
    out
}

#[test]
fn manifest_variants_matrix() {
    let b = base();
    // (label, manifest, expected on Device, Web, Server)
    let cases: Vec<(&str, String, [&str; 3])> = vec![
        ("plain", b.clone(), ["ok", "ok", "ok"]),
        ("target web", format!("{b}target web\n"), ["E0411", "ok", "ok"]),
        ("target server", format!("{b}target server\n"), ["E0411", "E0411", "ok"]),
        ("target device explicit", format!("{b}target device\n"), ["ok", "ok", "ok"]),
        // F6 fixed: only ASCII space/tab separate manifest tokens (was: any Unicode whitespace).
        ("nbsp separator", format!("{b}target\u{00A0}web\n"), ["E0402", "E0402", "E0402"]),
        ("ideographic space", format!("{b}target\u{3000}web\n"), ["E0402", "E0402", "E0402"]),
        ("tab", format!("{b}target\tweb\n"), ["E0411", "ok", "ok"]),
        ("Web", format!("{b}target Web\n"), ["E0402", "E0402", "E0402"]),
        ("WEB", format!("{b}target WEB\n"), ["E0402", "E0402", "E0402"]),
        ("web NUL", format!("{b}target web\0\n"), ["E0402", "E0402", "E0402"]),
        ("fullwidth web", format!("{b}target \u{FF57}\u{FF45}\u{FF42}\n"), ["E0402", "E0402", "E0402"]),
        ("cyrillic e", format!("{b}target w\u{0435}b\n"), ["E0402", "E0402", "E0402"]),
        ("trailing comment", format!("{b}target web # hi\n"), ["E0402", "E0402", "E0402"]),
        ("dup target same", format!("{b}target web\ntarget web\n"), ["E0402", "E0402", "E0402"]),
        ("dup target diff", format!("{b}target device\ntarget server\n"), ["E0402", "E0402", "E0402"]),
        ("dup features", format!("{b}target web\nfeatures tail-call\nfeatures tail-call\n"), ["E0402", "E0402", "E0402"]),
        ("features before target", format!("{b}features tail-call\ntarget web\n"), ["E0411", "ok", "ok"]),
        ("features no target", format!("{b}features tail-call\n"), ["E0402", "E0402", "E0402"]),
        ("feature above target", format!("{b}target web\nfeatures multi-memory\n"), ["E0402", "E0402", "E0402"]),
        ("feature uppercase", format!("{b}target web\nfeatures Tail-Call\n"), ["E0402", "E0402", "E0402"]),
        ("feature empty", format!("{b}target web\nfeatures\n"), ["E0402", "E0402", "E0402"]),
        ("feature dup in line", format!("{b}target web\nfeatures tail-call tail-call\n"), ["E0402", "E0402", "E0402"]),
        ("target missing value", format!("{b}target\n"), ["E0402", "E0402", "E0402"]),
        ("target two values", format!("{b}target web server\n"), ["E0402", "E0402", "E0402"]),
        ("simd declared (engine lacks)", format!("{b}target web\nfeatures simd128\n"), ["E0411", "E0411", "E0411"]),
        ("exceptions declared (engine lacks)", format!("{b}target web\nfeatures exceptions\n"), ["E0411", "E0411", "E0411"]),
        ("memory64 declared server", format!("{b}target server\nfeatures memory64\n"), ["E0411", "E0411", "E0411"]),
        ("tail-call honest web", format!("{b}target web\nfeatures tail-call\n"), ["E0411", "ok", "ok"]),
        // F2 fixed: `multi-memory` is no longer a feature (ABI 1.0: one memory), unknown feature = E0402.
        ("multi-memory honest server (code has 1 memory)", format!("{b}target server\nfeatures multi-memory\n"), ["E0402", "E0402", "E0402"]),
        ("CRLF", b.replace('\n', "\r\n") + "target web\r\n", ["E0411", "ok", "ok"]),
        ("lone CR target", format!("{b}target device\rtarget web\n"), ["E0402", "E0402", "E0402"]),
        ("BOM", format!("\u{FEFF}{b}target web\n"), ["E0402", "E0402", "E0402"]),
        ("U+2028 sep", format!("{b}target device\u{2028}target web\n"), ["E0402", "E0402", "E0402"]),
        ("target in comment only", format!("{b}# target server\n"), ["ok", "ok", "ok"]),
        ("indented", format!("{b}   target web   \n"), ["E0411", "ok", "ok"]),
        ("big comment pad > cap", format!("{b}target web\n#{}\n", "x".repeat(5000)), ["E0402", "E0402", "E0402"]),
    ];
    let mut bad = vec![];
    for (label, m, want) in &cases {
        let w = with_manifest(m);
        let got: Vec<String> = Target::ALL.iter().map(|h| run(&w, *h)).collect();
        println!("{:<48} {:?}", label, got);
        // also when the manifest section comes first
        let w2 = manifest_first(&w);
        let got2: Vec<String> = Target::ALL.iter().map(|h| run(&w2, *h)).collect();
        if got2 != got {
            bad.push(format!("{label}: placement changes result {got:?} vs first {got2:?}"));
        }
        for i in 0..3 {
            // "E0411" vs "E0402" as first refusal are both fine when both apply (policy), but ok-vs-refuse is not.
            let is_ok = got[i] == "ok";
            if is_ok != (want[i] == "ok") {
                bad.push(format!("{label} on {}: got {} want {}", Target::ALL[i], got[i], want[i]));
            }
        }
    }
    assert_eq!(bad.len(), 0, "{}", bad.join("\n"));
}

#[test]
fn two_manifest_sections_in_any_order_are_refused() {
    let b = base();
    for (a, c) in [(b.clone(), format!("{b}target web\n")), (format!("{b}target web\n"), b.clone()), (b.clone(), b.clone()), (format!("{b}target web\n"), "garbage".to_string())] {
        let w = with_manifest_bytes(c.as_bytes());
        // with_manifest_bytes appended an empty-first manifest; rebuild: first = a, second = c
        let mut f = Fixture::new(&[]);
        f.manifest = Some(a.clone());
        let mut x = f.bytes();
        append_custom(&mut x, "oxi.manifest", c.as_bytes());
        for h in Target::ALL {
            let r = run(&x, h);
            assert!(r != "ok", "two sections loaded ok on {h}: first={a:?} second={c:?}");
        }
        let _ = w;
    }
}

#[test]
fn non_utf8_and_empty_manifest_refused() {
    for m in [&b"\xff\xfe"[..], b"", b"oxi-manifest 1\nabi 1.0\nedition 2026\nprofile default\ntarget web\n\xff\n"] {
        let w = with_manifest_bytes(m);
        for h in Target::ALL {
            let r = run(&w, h);
            assert!(r != "ok", "{m:?} on {h} loaded");
        }
    }
}

/// A valid module followed by a second module / garbage: nothing may run.
#[test]
fn trailing_bytes_and_concatenated_modules() {
    let ok = with_manifest(&base());
    let web_simd = {
        let mut b = wat::parse_str("(module (func (result i32) v128.const i64x2 0 0 i32x4.extract_lane 0))").unwrap();
        append_custom(&mut b, "oxi.manifest", format!("{}target web\n", base()).as_bytes());
        b
    };
    for tail in [web_simd.clone(), vec![0u8; 3], b"\0asm".to_vec(), vec![0, 1, 2, 3, 4, 5, 6, 7, 8]] {
        let mut x = ok.clone();
        x.extend_from_slice(&tail);
        for h in Target::ALL {
            let r = run(&x, h);
            assert!(r != "ok", "module + trailing {} bytes loaded ok on {h}", tail.len());
        }
    }
}

/// reload/restart go through the same check: a Device host must refuse a hot-reload to a web module.
#[test]
fn reload_to_higher_target_refused() {
    let dev = with_manifest(&base());
    let web = with_manifest(&format!("{}target web\n", base()));
    let host = host_for(Target::Device);
    let mut rt = host.instantiate(&dev, Grants::all()).unwrap();
    let e = rt.reload(&web).unwrap_err();
    assert_eq!(e.code(), "OXS-E0411");
    // the old instance still works after the refused reload
    rt.restart().unwrap();
}

/// Huge manifest (just under the module cap) is judged by check_target before the 4096 byte cap is applied.
#[test]
fn huge_manifest_is_cheap() {
    use std::time::Instant;
    for (name, body) in [
        ("comments", "#xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n".repeat(70_000)),
        ("needs", "needs host:log@1 Log\n".repeat(150_000)),
        ("features", format!("target web\nfeatures {}\n", "tail-call ".repeat(300_000))),
        ("blank", "\n".repeat(3_500_000)),
    ] {
        let w = with_manifest(&format!("{}{body}", base()));
        assert!(w.len() < 4 << 20, "{name} {}", w.len());
        let t = Instant::now();
        let r = run(&w, Target::Server);
        let ms = t.elapsed().as_millis();
        println!("huge manifest {name}: {r} in {ms} ms");
        assert!(r != "ok", "{name}");
        assert!(ms < 1500, "{name}: {ms} ms");
    }
}

/// Every prefix of a valid web module and every single-bit flip inside its manifest: no panic, no hang,
/// and nothing loads that the intact module would not (a flip may only turn ok into a refusal or keep a valid manifest).
#[test]
fn truncation_and_bitflip_never_panic() {
    let m = format!("{}target web\nfeatures tail-call\n", base());
    let w = with_manifest(&m);
    for h in Target::ALL {
        for cut in 0..w.len() {
            let r = std::panic::catch_unwind(|| run(&w[..cut], h));
            assert!(r.is_ok(), "panic at cut {cut} on {h}");
            assert_ne!(r.unwrap(), "ok", "truncated module (cut {cut}) loaded on {h}");
        }
    }
    // flip every bit of the manifest text
    let pos = w.windows(m.len()).position(|x| x == m.as_bytes()).unwrap();
    let mut loaded_web = 0;
    for i in 0..m.len() * 8 {
        let mut x = w.clone();
        x[pos + i / 8] ^= 1 << (i % 8);
        for h in Target::ALL {
            let r = std::panic::catch_unwind(|| run(&x, h));
            assert!(r.is_ok(), "panic on manifest bit {i} host {h}");
            if h == Target::Device {
                // a device host may only load it if the flip produced a device-level manifest
                if r.unwrap() == "ok" {
                    let t = String::from_utf8_lossy(&x[pos..pos + m.len()]).to_string();
                    assert!(!t.contains("target web") && !t.contains("target server") || t.contains('#'), "device host loaded web manifest after flip {i}: {t:?}");
                }
            } else if r.unwrap() == "ok" && h == Target::Web {
                loaded_web += 1;
            }
        }
    }
    println!("web host loaded {loaded_web} bit-flipped manifests");
}

/// Cost of the three validator passes for a 3.9 MB module near the size cap.
#[test]
fn big_module_validation_cost() {
    use std::time::Instant;
    // one function body of ~3.9M `nop`s
    let mut body = vec![0u8]; // 0 locals
    body.extend(std::iter::repeat_n(0x01u8, 3_900_000));
    body.push(0x0b);
    let mut leb = vec![];
    let mut n = body.len();
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 { leb.push(b | 0x80) } else { leb.push(b); break }
    }
    let mut code_sec = vec![1u8];
    code_sec.extend(&leb);
    code_sec.extend(&body);
    let mut sec_len = vec![];
    let mut n = code_sec.len();
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 { sec_len.push(b | 0x80) } else { sec_len.push(b); break }
    }
    let mut w = b"\0asm\x01\0\0\0".to_vec();
    w.extend([1, 4, 1, 0x60, 0, 0]); // type () -> ()
    w.extend([3, 2, 1, 0]); // func section
    w.push(10);
    w.extend(&sec_len);
    w.extend(&code_sec);
    for (label, m) in [("device", base()), ("web+tail", format!("{}target web\nfeatures tail-call\n", base())), ("server", format!("{}target server\n", base()))] {
        let mut x = w.clone();
        append_custom(&mut x, "oxi.manifest", m.as_bytes());
        assert!(x.len() < 4 << 20, "{}", x.len());
        let t = Instant::now();
        let r = run(&x, Target::Server);
        println!("3.9MB module, manifest {label}: {r} in {} ms", t.elapsed().as_millis());
    }
}
