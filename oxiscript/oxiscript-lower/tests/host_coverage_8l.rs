#![allow(clippy::pedantic, clippy::nursery, dead_code)]
//! Coverage the BAD ACTOR 8.L attack did not reach: out-buffer retry with changing result lengths,
//! callbacks fired after state changes, `needs` ordering against `main` parameters.
mod common;
use common::*;
use oxiscript_abi::idl::{self, IdlFn, Interface};
use oxiscript_runtime::{Capability, CapabilityProvider, Grants, HostError, Limits, Value};
use std::sync::{Arc, Mutex};

const BLOB_IDL: &str = r#"
interface Blob "host:blob" @1 {
    fn text() -> result<str>;
    fn data() -> result<bytes>;
}
"#;

/// Scripted provider: every `text`/`data` call answers with the next length of the script (the last one repeats).
struct BlobProvider {
    itf: Interface,
    script: Arc<Mutex<Vec<usize>>>,
    calls: Arc<Mutex<usize>>,
}
struct BlobCap {
    script: Arc<Mutex<Vec<usize>>>,
    calls: Arc<Mutex<usize>>,
}
impl CapabilityProvider for BlobProvider {
    fn interface(&self) -> &Interface {
        &self.itf
    }
    fn instantiate(&self, _c: &oxiscript_runtime::capability::InstanceCtx) -> Box<dyn Capability> {
        Box::new(BlobCap { script: Arc::clone(&self.script), calls: Arc::clone(&self.calls) })
    }
}
impl Capability for BlobCap {
    fn call(&mut self, f: &IdlFn, _a: &[Value]) -> Result<Option<Value>, HostError> {
        let n = {
            let mut s = self.script.lock().unwrap();
            if s.len() > 1 { s.remove(0) } else { s[0] }
        };
        *self.calls.lock().unwrap() += 1;
        // all 'x', last byte 'y' (so a truncated or stale result is visible)
        let mut v = vec![b'x'; n];
        if n > 0 {
            v[n - 1] = b'y';
        }
        match f.name.as_str() {
            "text" => Ok(Some(Value::Str(String::from_utf8(v).unwrap()))),
            "data" => Ok(Some(Value::Bytes(v))),
            o => Err(HostError::Trap(format!("blob: {o}"))),
        }
    }
}

fn blob_run(script: Vec<usize>, which: &str) -> (Vec<String>, usize) {
    let mut ifs = interfaces();
    ifs.extend(idl::parse(BLOB_IDL).unwrap());
    let body = if which == "text" {
        r#"const r = b.text(); if (r.kind === "ok") { log.write(2, `ok ${r.value.byteLength} ${r.value.slice(r.value.byteLength - 1, r.value.byteLength)}`); } else { log.write(2, `err ${r.error}`); }"#
    } else {
        r#"const r = b.data(); if (r.kind === "ok") { log.write(2, `ok ${r.value.length} ${r.value[r.value.length - 1]}`); } else { log.write(2, `err ${r.error}`); }"#
    };
    let src = format!(
        "import {{ Log }} from \"host:log\";\nimport {{ Blob }} from \"host:blob\";\nexport function main(log: Log, b: Blob): void {{ {body} }}\n"
    );
    let wasm = oxiscript_lower::compile_source(&src, &ifs, &oxiscript_lower::Options::default()).unwrap_or_else(|e| panic!("{e}\n{src}"));
    let e = env();
    let calls = Arc::new(Mutex::new(0usize));
    let itf = ifs.iter().find(|i| i.name == "Blob").unwrap().clone();
    e.host
        .register(Arc::new(BlobProvider { itf, script: Arc::new(Mutex::new(script)), calls: Arc::clone(&calls) }))
        .unwrap();
    let mut rt = e.host.instantiate(&wasm, Grants::all()).unwrap_or_else(|er| panic!("instantiate: {er}"));
    let log: Vec<String> = e.sink.lines().into_iter().map(|l| l.msg).collect();
    let live = rt.deinit().unwrap();
    assert_eq!(live, 0, "leak after {which} {:?}", log);
    let n = *calls.lock().unwrap();
    (log, n)
}

#[test]
fn out_buffer_boundary_lengths_text() {
    for n in [1usize, 63, 64, 65, 127, 128, 129, 4096, 4097, 20000] {
        let (log, calls) = blob_run(vec![n], "text");
        assert_eq!(log, vec![format!("ok {n} y")], "n={n}");
        // one call when it fits the first 64-byte buffer, exactly two otherwise (host reports the size)
        assert_eq!(calls, if n <= 64 { 1 } else { 2 }, "n={n}");
    }
}

#[test]
fn out_buffer_boundary_lengths_bytes() {
    for n in [1usize, 63, 64, 65, 4096, 4097] {
        let (log, calls) = blob_run(vec![n], "bytes");
        assert_eq!(log, vec![format!("ok {n} 121")], "n={n}");
        assert_eq!(calls, if n <= 64 { 1 } else { 2 }, "n={n}");
    }
}

#[test]
fn out_buffer_length_changes_between_retries() {
    // grows: 10 would fit, but the first answer is 100, then 200, then 300 (each retry sees a bigger result)
    let (log, calls) = blob_run(vec![100, 200, 300], "text");
    assert_eq!(log, vec!["ok 300 y"]);
    assert_eq!(calls, 4); // 64 -> 100 -> 200 -> 300 -> fits
    // shrinks after the retry: 100 (too big for 64) then 30
    let (log, calls) = blob_run(vec![100, 30], "text");
    assert_eq!(log, vec!["ok 30 y"]);
    assert_eq!(calls, 2);
    // shrinks and grows again
    let (log, calls) = blob_run(vec![65, 10, 10], "bytes");
    assert_eq!(log, vec!["ok 10 121"]);
    assert_eq!(calls, 2);
    // the guest gives up after a bounded number of retries when the host keeps growing: an error status, no trap, no leak
    let (log, calls) = blob_run((1..=20).map(|i| 100 * i).collect(), "text");
    assert_eq!(log, vec!["err 1"]);
    assert!(calls <= 6, "calls={calls}");
}

#[test]
fn out_buffer_above_the_host_payload_limit_traps_in_the_host() {
    // abi: results above `Limits::max_host_payload` (64 KiB default) are a host failure, not a status
    let mut ifs = interfaces();
    ifs.extend(idl::parse(BLOB_IDL).unwrap());
    let src = "import { Log } from \"host:log\";\nimport { Blob } from \"host:blob\";\nexport function main(log: Log, b: Blob): void { const r = b.text(); log.write(2, r.kind); }\n";
    let wasm = oxiscript_lower::compile_source(src, &ifs, &oxiscript_lower::Options::default()).unwrap();
    let e = env_with_limits(Limits::default());
    let itf = ifs.iter().find(|i| i.name == "Blob").unwrap().clone();
    e.host
        .register(Arc::new(BlobProvider { itf, script: Arc::new(Mutex::new(vec![70_000])), calls: Arc::new(Mutex::new(0)) }))
        .unwrap();
    let r = e.host.instantiate(&wasm, Grants::all());
    assert!(r.is_err(), "a result over the payload limit must be a clean host failure");
}

// ---- callbacks fired after state changes ----
#[test]
fn callbacks_see_state_changed_after_registration() {
    let mut p = Prog::new(
        r#"
import { Log } from "host:log";
import { Ticker } from "host:ticker";
class Box { v: string; constructor(v: string) { this.v = v; } }
let g: string | null = null;
export function main(log: Log, t: Ticker): void {
  let counter = 0;
  const box = new Box("a" + "1");
  g = "g" + "1";
  t.register((n, label) => {
    counter += n;
    log.write(2, `${label} counter=${counter} box=${box.v} g=${g ?? "-"}`);
  });
  counter = 100;
  box.v = "b" + "2";
  g = "g" + "2";
  t.fire(5);
  counter = 1000;
  box.v = "c" + "3";
  g = null;
  t.fire(7);
}
"#,
    );
    p.rt.pump().unwrap();
    // both events are delivered after main finished: they see the final state
    assert_eq!(p.log(), vec!["tick5 counter=1005 box=c3 g=-", "tick7 counter=1012 box=c3 g=-"]);
    {
        let mut st = p.env.ticker.lock().unwrap();
        let mut a = std::mem::take(&mut st.tick_cbs);
        a.extend(std::mem::take(&mut st.done_cbs));
        st.releases.extend(a);
    }
    p.rt.pump().unwrap();
    p.finish();
}

// ---- needs order vs main parameters ----
#[test]
fn needs_follow_main_parameter_order() {
    let out = compile_full(
        r#"
import { Log } from "host:log";
import { State } from "host:state";
import { Ticker } from "host:ticker";
export function main(tk: Ticker, log: Log, st: State): void {
  log.write(2, "hello");
  const s = st.open("k");
  if (s.kind === "ok") { st.set_i64(s.value, 7i64); const g = st.get_i64(s.value); log.write(2, g.kind === "ok" ? `${g.value}` : "bad"); }
  log.write(2, `${tk.fire(3)}`);
}
"#,
    );
    let names: Vec<&str> = out.needs.iter().map(|n| n.interface.as_str()).collect();
    assert_eq!(names, ["Ticker", "Log", "State"]);
    let e = env();
    let mut p = Prog { rt: e.host.instantiate(&out.wasm, Grants::all()).unwrap(), env: e, wasm: out.wasm.clone() };
    // each handle reached its own provider
    assert_eq!(p.log(), vec!["hello", "7", "0"]);
    assert!(p.env.store.namespaces() >= 1, "the state write must land in the store");
    p.finish();
}
