#![allow(clippy::pedantic, clippy::nursery)]
//! Closed programs: host capabilities, callbacks, out-buffers, several capabilities.
mod common;
use common::*;

fn drop_registrations(p: &mut Prog) {
    {
        let mut st = p.env.ticker.lock().unwrap();
        let mut a = std::mem::take(&mut st.tick_cbs);
        a.extend(std::mem::take(&mut st.done_cbs));
        st.releases.extend(a);
    }
    p.rt.pump().unwrap();
}

#[test]
fn p33_state_results() {
    run_log(
        r#"
import { Log } from "host:log";
import { State } from "host:state";
function setNum(st: State, key: string, v: i64): Result<i64, i32> {
  const slot = st.open(key)?;
  st.set_i64(slot, v)?;
  const back = st.get_i64(slot)?;
  return Ok(back * 2i64);
}
export function main(log: Log, st: State): void {
  const r = setNum(st, "answer", 21i64);
  switch (r.kind) {
    case "ok": log.write(2, `ok ${r.value}`); break;
    case "err": log.write(2, `err ${r.error}`); break;
  }
  const a = st.open("f");
  if (a.kind === "ok") {
    const s = a.value;
    const w = st.set_f64(s, 2.5);
    const g = st.get_f64(s);
    log.write(2, `${w.kind} ${g.kind === "ok" ? (g.value * 2.0) as i32 : -1}`);
  }
  if (a.kind === "ok") {
    const missing = st.read_str(a.value);
    log.write(2, missing.kind === "ok" ? "text" : "no text");
  }
}
"#,
        &["ok 42", "ok 5", "no text"],
    );
}

#[test]
fn p34_out_buffer_retry() {
    // 200 bytes do not fit the first 64-byte buffer: the generated loop retries
    run_log(
        r#"
import { Log } from "host:log";
import { State } from "host:state";
function roundtrip(st: State, text: string): Result<string, i32> {
  const slot = st.open("text")?;
  st.write_str(slot, text)?;
  return st.read_str(slot);
}
export function main(log: Log, st: State): void {
  let long = "";
  for (let i = 0; i < 25; i++) { long += "héllo,w"; }
  const r = roundtrip(st, long);
  if (r.kind === "ok") {
    log.write(2, `${r.value.byteLength} ${r.value === long} ${r.value.slice(0, 7)}`);
  } else {
    log.write(2, `err ${r.error}`);
  }
  const s = roundtrip(st, "short");
  log.write(2, s.kind === "ok" ? s.value : "err");
}
"#,
        &["200 true héllo,", "short"],
    );
}

#[test]
fn p35_callbacks_from_host() {
    let mut p = Prog::new(
        r#"
import { Log } from "host:log";
import { Ticker } from "host:ticker";
export function main(log: Log, t: Ticker): void {
  let total = 0;
  const seen: string[] = [];
  t.register((n, label) => {
    total += n;
    seen.push(label);
    log.write(2, `tick ${n} ${label} total=${total} seen=${seen.length}`);
  });
  t.register((n, label) => { log.write(2, `second ${n}`); });
  const subs = t.fire(10);
  log.write(2, `subs ${subs}`);
}
"#,
    );
    assert_eq!(p.log(), vec!["subs 2"]);
    p.rt.pump().unwrap();
    assert_eq!(
        p.log(),
        vec!["subs 2", "tick 10 tick10 total=10 seen=1", "second 11"]
    );
    drop_registrations(&mut p);
    p.finish();
}

#[test]
fn p36_state_watch_callbacks() {
    let mut p = Prog::new(
        r#"
import { Log } from "host:log";
import { State } from "host:state";
class Watcher {
  hits: i32 = 0;
  log: Log;
  constructor(log: Log) { this.log = log; }
  on(key: string): void { this.hits += 1; this.log.write(2, `changed ${key} #${this.hits}`); }
}
export function main(log: Log, st: State): void {
  const w = new Watcher(log);
  const r = st.open("k");
  if (r.kind === "ok") {
    st.watch(r.value, (slot, key) => { w.on(key); });
    st.set_i64(r.value, 1i64);
    st.set_i64(r.value, 2i64);
  }
}
"#,
    );
    p.rt.pump().unwrap();
    let log = p.log();
    // the host coalesces change notifications per subscriber: two writes, one event
    assert_eq!(log, vec!["changed k #1"]);
    // the host holds the registered closure (and what it captured) until it drops the registration
    let held = p.live_after_deinit();
    eprintln!("state watch: {held} objects held by the host registration");
    assert!(held > 0 && held < 10);
}

#[test]
fn p37_main_with_several_capabilities() {
    run_log(
        r#"
import { Log } from "host:log";
import { State } from "host:state";
import { Ticker } from "host:ticker";
struct Tools { log: Log; st: State; }
function use(t: Tools, msg: string): void { t.log.write(1, msg); }
export function main(log: Log, st: State, tk: Ticker): void {
  const tools: Tools = { log, st };
  use(tools, "tools ready");
  const slot = st.open("x");
  use(tools, slot.kind);
  const n = tk.fire(1);
  use(tools, `${n}`);
}
"#,
        &["tools ready", "ok", "0"],
    );
}

#[test]
fn p38_capability_in_closure_and_unused_capability() {
    // `state` is a needs line even though main never calls it
    run_log(
        r#"
import { Log } from "host:log";
import { State } from "host:state";
export function main(log: Log, state: State): void {
  const say = (s: string) => { log.write(2, s); };
  say("a");
  say("b");
}
"#,
        &["a", "b"],
    );
}

#[test]
fn p39_cap_in_other_function_gets_a_needs_line() {
    // `canvas` is only used by an exported function, not named by `main`
    let out = compile_full(
        r#"
import { Log } from "host:log";
import { Canvas } from "host:canvas";
@host export function draw(canvas: Canvas, x: i32): void { canvas.put(x, 0, "o"); }
export function main(log: Log): void { log.write(2, "ready"); }
"#,
    );
    assert_eq!(out.needs.len(), 2);
    assert_eq!(out.needs[0].interface, "Log");
    assert_eq!(out.needs[1].interface, "Canvas");
    let mut p = Prog::from_wasm(out.wasm);
    assert_eq!(p.log(), vec!["ready"]);
    p.finish();
}
