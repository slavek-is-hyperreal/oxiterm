//! Hand-written WAT fixtures that follow abi.md (no compiler exists yet).
#![allow(dead_code, clippy::pedantic)]

use oxiscript_abi::exports::{self, ExportKind, EXPORTS};
use oxiscript_abi::WasmTy;
use oxiscript_runtime::caps::log::{LogLimits, LogProvider, LogSink};
use oxiscript_runtime::caps::state::{StateLimits, StateProvider, StateStore};
use oxiscript_runtime::{Host, Limits};
use std::sync::Arc;

pub const LOG: &str = "host:log@1 Log";
pub const STATE: &str = "host:state@1 State";

pub const IMPORT_LOG: &str =
    r#"(import "host:log@1" "write" (func $log_write (param i32 i32 i32 i32)))"#;
pub const IMPORT_STATE: &str = r#"
 (import "host:state@1" "open" (func $s_open (param i32 i32 i32) (result i32 i32)))
 (import "host:state@1" "get_i64" (func $s_get (param i32 i32) (result i32 i64)))
 (import "host:state@1" "set_i64" (func $s_set (param i32 i32 i64) (result i32)))
 (import "host:state@1" "read_str" (func $s_read (param i32 i32 i32 i32) (result i32 i32)))
 (import "host:state@1" "write_str" (func $s_write (param i32 i32 i32 i32) (result i32)))
 (import "host:state@1" "watch" (func $s_watch (param i32 i32 i32)))
"#;

fn wty(t: WasmTy) -> &'static str {
    t.name()
}

/// Builder for a conforming module.
#[derive(Clone)]
pub struct Fixture {
    pub needs: Vec<String>,
    pub imports: String,
    /// Extra top-level items (functions, data, globals).
    pub body: String,
    /// Statements run at the end of `oxi_init`.
    pub init: String,
    pub overrides: Vec<(String, String)>,
    pub skip_exports: Vec<String>,
    pub memory_pages: u32,
    pub scratch_ptr: u32,
    pub scratch_cap: u32,
    pub abi_version: u32,
    pub manifest: Option<String>,
    pub profile: &'static str,
    pub extra_custom: Vec<(String, Vec<u8>)>,
}

impl Fixture {
    pub fn new(needs: &[&str]) -> Fixture {
        Fixture {
            needs: needs.iter().map(|s| (*s).to_string()).collect(),
            imports: String::new(),
            body: String::new(),
            init: String::new(),
            overrides: vec![],
            skip_exports: vec![],
            memory_pages: 2,
            scratch_ptr: 2048,
            scratch_cap: 4096,
            abi_version: 1 << 16,
            manifest: None,
            profile: "default",
            extra_custom: vec![],
        }
    }

    pub fn imports(mut self, s: &str) -> Self {
        self.imports.push_str(s);
        self
    }
    pub fn body(mut self, s: &str) -> Self {
        self.body.push_str(s);
        self
    }
    pub fn init(mut self, s: &str) -> Self {
        self.init.push_str(s);
        self
    }
    pub fn over(mut self, name: &str, func_wat: &str) -> Self {
        self.overrides
            .push((name.to_string(), func_wat.to_string()));
        self
    }
    pub fn skip(mut self, name: &str) -> Self {
        self.skip_exports.push(name.to_string());
        self
    }

    pub fn manifest_text(&self) -> String {
        let mut m = format!(
            "oxi-manifest 1\nabi 1.0\nedition 2026\nprofile {}\n",
            self.profile
        );
        for n in &self.needs {
            m.push_str(&format!("needs {n}\n"));
        }
        m
    }

    pub fn wat(&self) -> String {
        let mut w = String::from("(module\n");
        w.push_str(&self.imports);
        w.push_str(&format!(
            "\n(memory (export \"memory\") {})\n",
            self.memory_pages
        ));
        w.push_str(&format!(
            r#"(global (export "oxi_abi_version") i32 (i32.const {}))
(global $tk (export "oxi_trap_kind") (mut i32) (i32.const 0))
(global (export "oxi_typedesc_base") i32 (i32.const 1024))
(global (export "oxi_typedesc_count") i32 (i32.const 16))
(global (export "oxi_scratch_ptr") i32 (i32.const {}))
(global (export "oxi_scratch_cap") i32 (i32.const {}))
(global $released (mut i32) (i32.const 0))
"#,
            self.abi_version, self.scratch_ptr, self.scratch_cap
        ));
        for i in 0..self.needs.len() {
            w.push_str(&format!("(global $cap{i} (mut i32) (i32.const 0))\n"));
        }
        // oxi_init
        let params: String = (0..self.needs.len())
            .map(|i| format!("(param $p{i} i32)"))
            .collect::<Vec<_>>()
            .join(" ");
        let stores: String = (0..self.needs.len())
            .map(|i| format!("(global.set $cap{i} (local.get $p{i}))"))
            .collect::<Vec<_>>()
            .join("\n");
        let ov = |n: &str| {
            self.overrides
                .iter()
                .find(|(k, _)| k == n)
                .map(|(_, v)| v.clone())
        };
        if !self.skip_exports.iter().any(|s| s == exports::INIT) {
            w.push_str(&ov(exports::INIT).unwrap_or_else(|| {
                format!(
                    "(func (export \"oxi_init\") {params}\n{stores}\n{}\n)\n",
                    self.init
                )
            }));
        }
        for spec in EXPORTS {
            if spec.kind != ExportKind::Func || spec.name == exports::INIT {
                continue;
            }
            if self.skip_exports.iter().any(|s| s == spec.name) {
                if spec.name == exports::TRAP {
                    w.push_str("(func $oxi_trap_fn (param i32) (global.set $tk (local.get 0)) unreachable)\n");
                }
                continue;
            }
            if let Some(v) = ov(spec.name) {
                w.push_str(&v);
                w.push('\n');
                continue;
            }
            let params: String = spec
                .params
                .iter()
                .map(|t| format!("(param {})", wty(*t)))
                .collect();
            let results: String = spec
                .results
                .iter()
                .map(|t| format!("(result {})", wty(*t)))
                .collect();
            let body = match spec.name {
                exports::TRAP => "(global.set $tk (local.get 0)) unreachable".to_string(),
                exports::RELEASE => "(global.set $released (local.get 0))".to_string(),
                exports::TICK => "(i64.const -1)".to_string(),
                _ => spec
                    .results
                    .iter()
                    .map(|t| format!("({}.const 0)", wty(*t)))
                    .collect::<Vec<_>>()
                    .join(" "),
            };
            let id = if spec.name == exports::TRAP {
                "$oxi_trap_fn"
            } else {
                ""
            };
            w.push_str(&format!(
                "(func {id} (export \"{}\") {params} {results} {body})\n",
                spec.name
            ));
        }
        w.push_str(&self.body);
        w.push_str("\n)");
        w
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut b = wat::parse_str(self.wat()).unwrap_or_else(|e| panic!("{e}\n{}", self.wat()));
        let m = self
            .manifest
            .clone()
            .unwrap_or_else(|| self.manifest_text());
        if !m.is_empty() {
            append_custom(&mut b, "oxi.manifest", m.as_bytes());
        }
        for (n, d) in &self.extra_custom {
            append_custom(&mut b, n, d);
        }
        b
    }
}

fn leb(mut v: usize, out: &mut Vec<u8>) {
    loop {
        let mut byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
}

pub fn append_custom(wasm: &mut Vec<u8>, name: &str, data: &[u8]) {
    let mut payload = Vec::new();
    leb(name.len(), &mut payload);
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(data);
    wasm.push(0);
    leb(payload.len(), wasm);
    wasm.extend_from_slice(&payload);
}

pub struct Env {
    pub host: Host,
    pub sink: Arc<LogSink>,
    pub store: StateStore,
}

pub fn env_with(limits: Limits) -> Env {
    let host = Host::new(limits);
    let sink = LogSink::new(256);
    let store = StateStore::new(StateLimits::default());
    host.register(Arc::new(LogProvider::new(
        Arc::clone(&sink),
        LogLimits::default(),
    )))
    .unwrap();
    host.register(Arc::new(StateProvider::new(store.clone())))
        .unwrap();
    Env { host, sink, store }
}

pub fn env() -> Env {
    env_with(Limits::default())
}

/// The standard "application": logs a greeting in init, opens the slot `k`,
/// exports `bump`, `get`, `local` and a few attack helpers.
pub fn app_no_cb() -> Fixture {
    Fixture::new(&[LOG, STATE])
        .imports(IMPORT_LOG)
        .imports(IMPORT_STATE)
        .body(
            r#"
(data (i32.const 3000) "hello")
(data (i32.const 3100) "k")
(data (i32.const 3200) "hello world")
(global $slot (mut i32) (i32.const 0))
(global $local (mut i32) (i32.const 0))
(func $open_k
  (local $st i32) (local $h i32)
  (call $s_open (global.get $cap1) (i32.const 3100) (i32.const 1))
  (local.set $h) (local.set $st)
  (global.set $slot (local.get $h)))
(func (export "bump") (result i64)
  (local $st i32) (local $v i64)
  (global.set $local (i32.add (global.get $local) (i32.const 1)))
  (call $s_get (global.get $cap1) (global.get $slot))
  (local.set $v) (local.set $st)
  (local.set $v (i64.add (local.get $v) (i64.const 1)))
  (drop (call $s_set (global.get $cap1) (global.get $slot) (local.get $v)))
  (local.get $v))
(func (export "get_status") (result i32)
  (local $v i64) (local $st i32)
  (call $s_get (global.get $cap1) (global.get $slot))
  (local.set $v) (local.set $st) (local.get $st))
(func (export "local") (result i32) (global.get $local))
(func (export "log_at") (param i32 i32)
  (call $log_write (global.get $cap0) (i32.const 2) (local.get 0) (local.get 1)))
(func (export "log_level") (param i32)
  (call $log_write (global.get $cap0) (local.get 0) (i32.const 3000) (i32.const 5)))
(func (export "log_with_cap") (param i32)
  (call $log_write (local.get 0) (i32.const 2) (i32.const 3000) (i32.const 5)))
(func (export "open_with_cap") (param i32) (result i32)
  (local $st i32) (local $h i32)
  (call $s_open (local.get 0) (i32.const 3100) (i32.const 1))
  (local.set $h) (local.set $st) (local.get $st))
(func (export "get_with_slot") (param i32) (result i32)
  (local $v i64) (local $st i32)
  (call $s_get (global.get $cap1) (local.get 0))
  (local.set $v) (local.set $st) (local.get $st))
(func (export "write_hw") (result i32)
  (call $s_write (global.get $cap1) (global.get $slot) (i32.const 3200) (i32.const 11)))
(func (export "read_into") (param $ptr i32) (param $cap i32) (result i32 i32)
  (call $s_read (global.get $cap1) (global.get $slot) (local.get $ptr) (local.get $cap)))
(func (export "spin") (loop $l (br $l)))
(func $rec (export "rec") (param i32) (result i32)
  (i32.add (call $rec (local.get 0)) (i32.const 1)))
(func (export "grow") (param i32) (result i32) (memory.grow (local.get 0)))
(func (export "boom") (call $oxi_trap_fn (i32.const 4)))
(func (export "boom_kind") (param i32) (call $oxi_trap_fn (local.get 0)))
(func (export "unreachable") unreachable)
(func (export "logs") (param i32)
  (local $i i32)
  (loop $l
    (call $log_write (global.get $cap0) (i32.const 2) (i32.const 3000) (i32.const 5))
    (local.set $i (i32.add (local.get $i) (i32.const 1)))
    (br_if $l (i32.lt_u (local.get $i) (local.get 0)))))
(func (export "echo_str") (param $ptr i32) (param $len i32) (result i32)
  (i32.add (local.get $len) (i32.load8_u (local.get $ptr))))
"#,
        )
        .init("(call $open_k)\n(call $log_write (global.get $cap0) (i32.const 2) (i32.const 3000) (i32.const 5))")
}

pub fn quick(f: &Fixture, e: &Env) -> oxiscript_runtime::Runtime {
    e.host
        .instantiate(&f.bytes(), oxiscript_runtime::Grants::all())
        .unwrap_or_else(|er| panic!("{er}\n{}", f.wat()))
}

pub fn app() -> Fixture {
    app_no_cb().body(
        r#"
(global $cb_closure (mut i32) (i32.const 0))
(global $cb_len (mut i32) (i32.const 0))
(global $cb_first (mut i32) (i32.const 0))
(global $cb_count (mut i32) (i32.const 0))
(func (export "cb:host:state@1.State.OnChange") (param $c i32) (param $slot i32) (param $ptr i32) (param $len i32)
  (global.set $cb_closure (local.get $c))
  (global.set $cb_len (local.get $len))
  (global.set $cb_first (i32.load8_u (local.get $ptr)))
  (global.set $cb_count (i32.add (global.get $cb_count) (i32.const 1))))
(func (export "cb_state") (result i32 i32 i32 i32)
  (global.get $cb_closure) (global.get $cb_len) (global.get $cb_first) (global.get $cb_count))
(func (export "released") (result i32) (global.get $released))
(func (export "rewatch") (param i32)
  (call $s_watch (global.get $cap1) (global.get $slot) (local.get 0)))
(func (export "watch_new") (param i32)
  (local $st i32) (local $h i32)
  (call $s_open (global.get $cap1) (i32.const 3100) (i32.const 1))
  (local.set $h) (local.set $st)
  (call $s_watch (global.get $cap1) (local.get $h) (local.get 0)))
(func (export "ping") (result i64)
  (drop (call $s_set (global.get $cap1) (global.get $slot) (i64.const 7)))
  (i64.const 0))
"#,
    )
}

pub fn app_cb() -> Fixture {
    app().init("(call $s_watch (global.get $cap1) (global.get $slot) (i32.const 4242))")
}
