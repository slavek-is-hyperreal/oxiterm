#![allow(clippy::doc_markdown, clippy::format_push_string, clippy::range_plus_one, clippy::similar_names, clippy::unnecessary_map_or, clippy::unnecessary_min_or_max, clippy::unreadable_literal, clippy::too_many_lines, clippy::cast_possible_truncation)]
//! BAD ACTOR (run stage). Reproducers and a corpus dump for the browser differential.
//!   cargo test -p oxiscript-playground --test bad_actor_run -- --nocapture
//! `BAD_ACTOR_CORPUS=<file>` also writes {src, json} cases (examples + 60 generated programs) that
//! `sites/test/bad_actor_run_parity.mjs` replays through the REAL runner bundle in headless Chromium.
use oxiscript_playground::run::{compile, run_json};
use serde_json::{json, Value};

const H: &str = "import { Log } from \"host:log\";\nimport { State } from \"host:state\";\n";

fn outcome(src: &str) -> String {
    let v: Value = serde_json::from_str(&run_json(src, 2_000_000)).unwrap();
    if v["ok"] == true {
        format!("compiled; lines={:?} stop={}", v["lines"].as_array().unwrap().iter().map(|l| l["msg"].as_str().unwrap().to_string()).collect::<Vec<_>>(), v["stop"])
    } else {
        format!("REJECTED stage={} {:?}", v["stage"], v["diagnostics"].as_array().unwrap().iter().map(|d| format!("{} {}", d["code"].as_str().unwrap_or(""), d["message"].as_str().unwrap_or(""))).collect::<Vec<_>>())
    }
}

#[test]
fn isolation_reserved_names_and_imports() {
    let cases: Vec<(&str, String)> = vec![
        ("host export named oxi_init", format!("{H}@host export function oxi_init(): void {{ }}\nexport function main(log: Log): void {{ log.write(2, \"m\"); }}")),
        ("export_name oxi_init", format!("{H}@host @export_name(\"oxi_init\") export function step(): void {{ }}\nexport function main(log: Log): void {{ log.write(2, \"m\"); }}")),
        ("export_name memory", format!("{H}@host @export_name(\"memory\") export function step(): void {{ }}\nexport function main(log: Log): void {{ log.write(2, \"m\"); }}")),
        ("export_name main", format!("{H}@host @export_name(\"main\") export function step(): void {{ }}\nexport function main(log: Log): void {{ log.write(2, \"m\"); }}")),
        ("export_name oxi_manifest/abi", format!("{H}@host @export_name(\"oxi_abi\") export function step(): void {{ }}\nexport function main(log: Log): void {{ log.write(2, \"m\"); }}")),
        ("export_name __heap_base", format!("{H}@host @export_name(\"__heap_base\") export function step(): void {{ }}\nexport function main(log: Log): void {{ log.write(2, \"m\"); }}")),
        ("two @host same export_name", format!("{H}@host @export_name(\"s\") export function a(): void {{ }}\n@host @export_name(\"s\") export function b(): void {{ }}\nexport function main(log: Log): void {{ log.write(2, \"m\"); }}")),
        ("import host:clock", "import { Clock } from \"host:clock\";\nimport { Log } from \"host:log\";\nexport function main(log: Log, c: Clock): void { log.write(2, \"x\"); }".to_string()),
        ("import host:fs", "import { Fs } from \"host:fs\";\nexport function main(f: Fs): void { }".to_string()),
        ("import rust:std", "import { x } from \"rust:std\";\nexport function main(): void { }".to_string()),
        ("import ./other", "import { x } from \"./other\";\nexport function main(): void { }".to_string()),
        ("import ../../etc/passwd", "import { x } from \"../../etc/passwd\";\nexport function main(): void { }".to_string()),
        ("import /etc/passwd", "import { x } from \"/etc/passwd\";\nexport function main(): void { }".to_string()),
        ("import std/math", "import { x } from \"std/math\";\nexport function main(): void { }".to_string()),
        ("import host:log@2", "import { Log } from \"host:log@2\";\nexport function main(log: Log): void { }".to_string()),
        ("forge Log from int", format!("{H}export function main(log: Log): void {{ const l: Log = 1 as Log; l.write(2, \"x\"); }}")),
        ("forge via struct literal", format!("{H}export function main(log: Log): void {{ const l: Log = {{ h: 1 }}; l.write(2, \"x\"); }}")),
        ("new Log", format!("{H}export function main(log: Log): void {{ const l = new Log(); l.write(2, \"x\"); }}")),
        ("main takes no cap, uses none", "export function main(): void { }".to_string()),
        ("two mains/duplicate", format!("{H}export function main(log: Log): void {{ }}\nexport function main(log: Log): void {{ }}")),
        ("log level 300", format!("{H}export function main(log: Log): void {{ log.write(300, \"x\"); }}")),
        ("log level -1", format!("{H}export function main(log: Log): void {{ log.write(-1, \"x\"); }}")),
        ("state slot forged", format!("{H}export function main(log: Log, st: State): void {{ const r = st.get_i64(123456); log.write(2, \"ok\"); }}")),
        ("state key 1 MiB", format!("{H}export function main(log: Log, st: State): void {{ let k = \"k\"; for (const i of range(0, 20)) {{ k = k + k; }} const r = st.open(k); log.write(2, \"opened\"); }}")),
        ("module-level let + init order", format!("{H}let g: i32 = 1;\nexport function main(log: Log): void {{ log.write(2, `${{g}}`); }}")),
        ("null byte / BOM source", format!("\u{feff}{H}export function main(log: Log): void {{ log.write(2, \"a\\0b\"); }}")),
        ("lone surrogate-ish / 4-byte chars", format!("{H}export function main(log: Log): void {{ log.write(2, \"\u{1F980}\u{200B}\u{202E}x\"); }}")),
        ("log message exactly 1025 bytes", format!("{H}export function main(log: Log): void {{ log.write(2, \"{}\"); }}", "a".repeat(1025))),
        ("log message 70000 bytes", format!("{H}export function main(log: Log): void {{ log.write(2, \"{}\"); }}", "a".repeat(70000))),
    ];
    for (name, src) in &cases {
        let r = std::panic::catch_unwind(|| outcome(src));
        println!("ISO {:<36} {}", name, r.unwrap_or_else(|_| "PANIC".to_string()));
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 { self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); self.0 >> 33 }
    fn pick<'a>(&mut self, xs: &'a [&'a str]) -> &'a str { xs[(self.next() as usize) % xs.len()] }
}

fn gen_program(rng: &mut Lcg) -> String {
    let i32s = ["0", "1", "-1", "2", "7", "31", "32", "100", "2147483647", "-2147483648", "65535"];
    let i64s = ["0", "1", "-1", "9223372036854775807", "-9223372036854775808", "4294967296"];
    let u8s = ["0u8", "1u8", "200u8", "255u8"];
    let f64s = ["0.0", "1.5", "-2.25", "1e10", "3.0"];
    let mut s = String::from(H);
    s.push_str("export function main(log: Log): void {\n");
    s.push_str(&format!("  const a: i32 = {};\n  const b: i32 = {};\n  const c: i64 = {};\n  const u: u8 = {};\n  const f: f64 = {};\n  const arr: i32[] = [1, 2, 3];\n  const t = \"h\u{e9}llo \u{1F980}\";\n", rng.pick(&i32s), rng.pick(&i32s), rng.pick(&i64s), rng.pick(&u8s), rng.pick(&f64s)));
    let i32e = ["(a + b)", "(a - b)", "(a * b)", "(a / b)", "(a % b)", "(a +% b)", "(a +| b)", "(a -% b)", "(a *| b)", "(a & b)", "(a | b)", "(a ^ b)", "(a << 3)", "(a >> 1)", "(c as i32)", "(f as i32)", "(u as i32)", "arr[a]", "arr[(b % 4)]", "(t.byteLength)", "(arr.length)", "(a.wrapping<u8>() as i32)", "(a.saturating<u8>() as i32)"];
    let i64e = ["(c + 1)", "(c * c)", "(c / (a as i64))", "((a as i64) * (b as i64))", "(c +% 1)", "(c - 1)"];
    let u8e = ["(u + 1u8)", "(u * 2u8)", "(u - 1u8)", "(a as u8)", "(u +% 100u8)", "(u +| 100u8)"];
    let stre = ["t.slice(0, a)", "t.slice(1, 3)", "t.slice(b, 2)", "`${a}-${b}`", "`${c}`", "`${u}`", "`${f}`", "`${(f as i32)}`", "(t.includes(\"l\") ? \"y\" : \"n\")", "`${t.indexOf(\"o\")}`", "`${arr.pop()}`", "`${(f * 2.0)}`"];
    for _ in 0..7 {
        let e = match rng.next() % 4 {
            0 => format!("`${{{}}}`", rng.pick(&i32e)),
            1 => format!("`${{{}}}`", rng.pick(&i64e)),
            2 => format!("`${{{}}}`", rng.pick(&u8e)),
            _ => rng.pick(&stre).to_string(),
        };
        s.push_str(&format!("  log.write(2, {e});\n"));
    }
    s.push_str("}\n");
    s
}

#[test]
fn corpus_and_native_stability() {
    let mut cases: Vec<Value> = Vec::new();
    let ex = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sites/oxiscript/examples");
    let mut srcs: Vec<(String, String)> = std::fs::read_dir(ex).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().map_or(false, |x| x == "oxs")).map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap())).collect();
    srcs.sort();
    let mut rng = Lcg(0xBAD_AC70);
    for i in 0..60 { srcs.push((format!("gen{i}"), gen_program(&mut rng))); }
    let (mut ok, mut rejected, mut stops, mut clean) = (0, 0, 0, 0);
    for (name, src) in &srcs {
        let j = run_json(src, 0.max(10_000_000));
        let v: Value = serde_json::from_str(&j).unwrap();
        if v["ok"] == true { ok += 1; if v["stop"].is_null() { clean += 1 } else { stops += 1 } } else { rejected += 1; if name.starts_with("gen") { println!("GEN REJECTED {name}: {}", v["diagnostics"][0]["message"]); } }
        cases.push(json!({ "name": name, "src": src, "json": v }));
    }
    println!("CORPUS programs={} compiled={ok} (clean={clean} trapped={stops}) rejected={rejected}", srcs.len());
    if let Ok(path) = std::env::var("BAD_ACTOR_CORPUS") { std::fs::write(path, serde_json::to_string(&cases).unwrap()).unwrap(); }
}

/// Cheap mutation fuzz over the examples: no panic may escape compile or run (a panic aborts the wasm instance).
#[test]
#[ignore = "20 s in debug; run with --ignored"]
fn mutation_fuzz_no_panic() {
    let ex = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sites/oxiscript/examples");
    let seeds: Vec<String> = std::fs::read_dir(ex).unwrap().map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap()).collect();
    let toks = ["(", ")", "{", "}", "[", "]", ";", "?", "!", "as", "i32", "u8", "f64", "null", "-", "*", "/", "%", "<<", "\"", "`", "${", "=>", "@host", "while (true) {", "export", "0", "9999999999999", "1e308", ".", ",", "main", "switch", "case"];
    let mut rng = Lcg(42);
    let (mut n, mut panics) = (0, 0);
    let t0 = std::time::Instant::now();
    for it in 0..4000 {
        let base = &seeds[(rng.next() as usize) % seeds.len()];
        let mut chars: Vec<char> = base.chars().collect();
        for _ in 0..(1 + rng.next() % 4) {
            let at = (rng.next() as usize) % (chars.len() + 1);
            match rng.next() % 3 {
                0 if at < chars.len() => { chars.remove(at); }
                1 => { for (k, c) in rng.pick(&toks).chars().enumerate() { chars.insert((at + k).min(chars.len()), c); } }
                _ if at + 5 < chars.len() => { let seg: Vec<char> = chars[at..at + 5].to_vec(); let to = (rng.next() as usize) % chars.len(); for (k, c) in seg.into_iter().enumerate() { chars.insert((to + k).min(chars.len()), c); } }
                _ => {}
            }
        }
        let src: String = chars.into_iter().collect();
        n += 1;
        let r = std::panic::catch_unwind(|| run_json(&src, 200_000));
        if r.is_err() { panics += 1; let f = std::env::temp_dir().join(format!("oxs_panic_{it}.oxs")); let _ = std::fs::write(&f, &src); let f = f.display(); println!("PANIC iteration {it}: saved {f}"); }
    }
    println!("FUZZ mutated programs={n} panics={panics} in {:?}", t0.elapsed());
    assert_eq!(panics, 0);
}

#[test]
fn compile_fn_standalone() {
    // compile() of a rejected program returns Err JSON, never panics
    assert!(compile("export function main(): void { while }").is_err());
}
