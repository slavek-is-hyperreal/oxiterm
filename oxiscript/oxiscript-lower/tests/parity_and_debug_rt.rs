#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! BL04, BL09, BL10 verification suite in `oxiscript-lower`:
//! - BL04: scan compiled OxiScript programs (release and debug Linked) for builder paths / rustc hash
//! - BL09: f32 shortest round-trip text edge cases
//! - BL10: Stub-vs-Linked-vs-LinkedDebug parity matrix (verifying compiler output under
//!   `debug_runtime: true` with `stale_object` trap 65 and 0xDD freed-memory poisoning).
mod common;
use common::*;
use oxiscript_runtime::{Grants, Value};
use oxiscript_wasm::RuntimeMode;
use std::path::PathBuf;

const HDR: &str = "import { Log } from \"host:log\";\n";

fn assert_no_builder_paths(label: &str, wasm: &[u8]) {
    for needle in [b"/home".as_slice(), b"/Users", b".cargo/registry", b"rustc/"] {
        if let Some(pos) = wasm.windows(needle.len()).position(|w| w == needle) {
            let start = pos.saturating_sub(16);
            let end = (pos + needle.len() + 64).min(wasm.len());
            let snippet = String::from_utf8_lossy(&wasm[start..end]);
            panic!(
                "BL04: {label} embeds {:?} at {pos}: {snippet:?}",
                String::from_utf8_lossy(needle)
            );
        }
    }
}

#[test]
fn compiled_oxs_modules_have_no_builder_paths_or_rustc_hash() {
    let examples_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sites/oxiscript/examples");
    for name in ["hello", "counter", "errors", "laws", "shapes", "structs"] {
        let src = std::fs::read_to_string(examples_dir.join(format!("{name}.oxs"))).unwrap();
        let rel = compile_with_mode(&src, RuntimeMode::Linked, false);
        assert_no_builder_paths(&format!("{name}.oxs (linked release)"), &rel);
        let dbg = compile_with_mode(&src, RuntimeMode::Linked, true);
        assert_no_builder_paths(&format!("{name}.oxs (linked debug)"), &dbg);
    }
}

#[test]
fn f32_shortest_round_trip_formatting_edges() {
    let mut p = Prog::new(&format!(
        "{HDR}{}",
        r#"
export function main(log: Log): void {
  const z: f32 = 0.0f32;
  const nz: f32 = -0.0f32;
  const one: f32 = 1.0f32;
  const tenth: f32 = 0.1f32;
  const maxExactInt: f32 = 16777216.0f32;
  const maxVal: f32 = 3.4028235e38f32;
  const minPos: f32 = 1.4e-45f32;
  const nanVal: f32 = z / z;
  const posInf: f32 = one / z;
  const negInf: f32 = -one / z;
  log.write(2, `${z}|${nz}|${one}|${tenth}`);
  log.write(2, `${maxExactInt}|${maxVal}|${minPos}`);
  log.write(2, `${nanVal}|${posInf}|${negInf}`);
  log.write(2, tenth.toString() + "," + nz.toString() + "," + maxVal.toString());
}
"#
    ));
    assert_eq!(
        p.log(),
        vec![
            "0|-0|1|0.1",
            "16777216|3.4028235e38|1e-45",
            "NaN|Infinity|-Infinity",
            "0.1,-0,3.4028235e38",
        ]
    );
    p.finish();
}

/// BL10: run a broad matrix of OxiScript programs in ALL THREE modes:
/// 1. `RuntimeMode::Stub`
/// 2. `RuntimeMode::Linked` (release `oxiscript-rc`)
/// 3. `RuntimeMode::Linked` + `debug_runtime: true` (debug `oxiscript-rc` with
///    `stale_object` trap 65 + 0xDD freed-memory poisoning).
#[test]
fn parity_matrix_stub_linked_and_debug_runtime() {
    let cases: &[(&str, &str, bool)] = &[
        (
            "structs_and_copy",
            r#"
struct Pt { x: i32; y: i32; }
function shift(p: Pt, dx: i32): Pt { return { x: p.x + dx, y: p.y }; }
export function main(log: Log): void {
  const a: Pt = { x: 3, y: 4 };
  const b = shift(a, 10);
  log.write(2, `${a.x},${a.y}|${b.x},${b.y}`);
}
"#,
            false,
        ),
        (
            "classes_and_arrays",
            r#"
class Counter { n: i32; constructor(n: i32) { this.n = n; } bump(): i32 { this.n += 1; return this.n; } }
export function main(log: Log): void {
  const cs: Counter[] = [new Counter(0), new Counter(10)];
  let sum = 0;
  for (let i = 0; i < 20; i++) {
    sum += cs[i % 2].bump();
  }
  log.write(2, `${sum}`);
}
"#,
            false,
        ),
        (
            "unions_and_results",
            r#"
type Shape = { kind: "circle"; r: i32 } | { kind: "rect"; w: i32; h: i32 };
function score(s: Shape): Result<i32, string> {
  switch (s.kind) {
    case "circle": return Ok(s.r * s.r);
    case "rect": return s.w < 0 ? Err("neg") : Ok(s.w * s.h);
  }
}
export function main(log: Log): void {
  const xs: Shape[] = [{ kind: "circle", r: 5 }, { kind: "rect", w: 3, h: 4 }, { kind: "rect", w: -1, h: 2 }];
  for (const s of xs) {
    const r = score(s);
    switch (r.kind) {
      case "ok": log.write(2, `ok:${r.value}`);
      case "err": log.write(2, `err:${r.error}`);
    }
  }
}
"#,
            false,
        ),
        (
            "closures_and_hof",
            r#"
export function main(log: Log): void {
  const xs: i32[] = [1, 2, 3, 4, 5];
  const factor = 3;
  const ys = xs.map((x: i32) => x * factor).filter((x: i32) => x > 6);
  const total = ys.reduce((acc: i32, x: i32) => acc + x, 0);
  log.write(2, `${ys.length}:${total}`);
}
"#,
            false,
        ),
        (
            "strings_and_utf8",
            r#"
export function main(log: Log): void {
  const s = "héllo" + " " + "wörld";
  const cut = s.slice(0, 6);
  log.write(2, `${cut}|${s.byteLength}|${s.includes("wör")}|${s.startsWith("hé")}`);
}
"#,
            false,
        ),
        (
            "cycles_in_debug_runtime",
            r#"
class Node { other: Node | null; tag: string; constructor(t: string) { this.other = null; this.tag = t; } }
export function main(log: Log): void {
  for (let i = 0; i < 50; i++) {
    const a = new Node(`a${i}`);
    const b = new Node(`b${i}`);
    a.other = b;
    b.other = a;
  }
  log.write(2, "cycles-ok");
}
"#,
            true,
        ),
    ];

    for (name, body, has_cycles) in cases {
        let src = format!("{HDR}{body}");
        let w_stub = compile_with_mode(&src, RuntimeMode::Stub, false);
        let w_linked = compile_with_mode(&src, RuntimeMode::Linked, false);
        let w_debug = compile_with_mode(&src, RuntimeMode::Linked, true);

        let mut p_stub = Prog::from_wasm(w_stub);
        let mut p_linked = Prog::from_wasm(w_linked);
        let mut p_debug = Prog::from_wasm(w_debug);

        assert_eq!(p_stub.log(), p_linked.log(), "{name}: Stub vs Linked log mismatch");
        assert_eq!(p_linked.log(), p_debug.log(), "{name}: Linked vs LinkedDebug log mismatch");

        let live_linked = p_linked.live_after_deinit();
        let live_debug = p_debug.live_after_deinit();
        let live_stub = p_stub.live_after_deinit();
        assert_eq!(live_linked, 0, "{name}: Linked leaked {live_linked}");
        assert_eq!(live_debug, 0, "{name}: LinkedDebug leaked {live_debug}");
        if !has_cycles {
            assert_eq!(live_stub, 0, "{name}: Stub leaked {live_stub}");
        }
    }

    // Also run a stateful host-export + callback program through all three modes.
    let host_src = format!(
        "{HDR}{}",
        r#"
import { Ticker } from "host:ticker";
let hits: i32 = 0;
export function main(log: Log, t: Ticker): void {
  t.register((n: i32, label: string) => {
    hits += n;
    log.write(2, `${label}:${hits}`);
  });
}
@host export function trigger(t: i32): i32 { return hits + t; }
"#
    );
    let mut logs = Vec::new();
    for (mode, dbg) in [
        (RuntimeMode::Stub, false),
        (RuntimeMode::Linked, false),
        (RuntimeMode::Linked, true),
    ] {
        let w = compile_with_mode(&host_src, mode, dbg);
        let env = env();
        let mut rt = env.host.instantiate(&w, Grants::all()).unwrap();
        let cbs = env.ticker.lock().unwrap().tick_cbs.clone();
        assert_eq!(cbs.len(), 1);
        env.ticker.lock().unwrap().queue.push(
            oxiscript_runtime::CallbackEvent {
                callback: "OnTick".into(),
                closure: cbs[0],
                args: vec![Value::I32(5), Value::Str("ev".into())],
            },
        );
        let st = rt.pump().unwrap();
        assert_eq!(st.delivered, 1);
        let r = rt.call_export("trigger", &[Value::I32(7)]).unwrap();
        assert_eq!(r, vec![Value::I32(12)]);
        // Drop host registration and pump release so live objects drop to 0.
        let rel = std::mem::take(&mut env.ticker.lock().unwrap().tick_cbs);
        env.ticker.lock().unwrap().releases.extend(rel);
        let st2 = rt.pump().unwrap();
        assert_eq!(st2.released, 1);
        assert_eq!(rt.deinit().unwrap(), 0, "leak in ({mode:?}, dbg={dbg})");
        logs.push(env.sink.lines().into_iter().map(|l| l.msg).collect::<Vec<_>>());
    }
    assert_eq!(logs[0], logs[1], "host callback Stub vs Linked mismatch");
    assert_eq!(logs[1], logs[2], "host callback Linked vs LinkedDebug mismatch");
}
