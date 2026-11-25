//! Shared by the wasm tests: build the guest, load it in wasmi, drive it through
//! the same `Guest` trait the native tests use.
#![allow(dead_code)]

use oxiscript_rc::testkit::Guest;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use wasmi::{Engine, Func, Instance, Linker, Memory, Module, Store, Val};

/// Build the wasm artefact (`release` or `dev`) into the test tmp dir and
/// return its path. Panics loudly if the wasm32 target is missing.
pub fn build_wasm(profile: &str) -> PathBuf {
    let target_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wasm-build");
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["build", "--manifest-path", manifest, "--target", "wasm32-unknown-unknown", "--lib"])
        .arg("--target-dir")
        .arg(&target_dir);
    if profile == "release" {
        cmd.arg("--release");
    }
    let out = cmd.output().expect("cargo not runnable");
    assert!(
        out.status.success(),
        "building the wasm guest failed (is the wasm32-unknown-unknown target installed?):\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let dir = if profile == "release" { "release" } else { "debug" };
    target_dir.join("wasm32-unknown-unknown").join(dir).join("oxiscript_rc.wasm")
}

pub struct WasmGuest {
    pub store: Store<()>,
    pub instance: Instance,
    pub memory: Memory,
    funcs: HashMap<String, Func>,
    pub module: Module,
}

impl WasmGuest {
    pub fn load(path: &PathBuf) -> WasmGuest {
        let bytes = std::fs::read(path).unwrap();
        let engine = Engine::default();
        let module = Module::new(&engine, &bytes[..]).expect("valid wasm");
        let mut store = Store::new(&engine, ());
        let linker = <Linker<()>>::new(&engine);
        let instance = linker.instantiate_and_start(&mut store, &module).expect("instantiate");
        let memory = instance.get_memory(&store, "memory").expect("memory export");
        WasmGuest { store, instance, memory, funcs: HashMap::new(), module }
    }

    pub fn standard(path: &PathBuf) -> WasmGuest {
        let mut g = WasmGuest::load(path);
        oxiscript_rc::testkit::install_types(&mut g, &oxiscript_rc::testkit::standard_types());
        g
    }

    fn func(&mut self, name: &str) -> Func {
        if let Some(f) = self.funcs.get(name) {
            return *f;
        }
        let f = self.instance.get_func(&self.store, name).unwrap_or_else(|| panic!("missing export {name}"));
        self.funcs.insert(name.to_string(), f);
        f
    }

    pub fn call_vals(&mut self, name: &str, args: &[Val]) -> Result<Vec<Val>, wasmi::Error> {
        let f = self.func(name);
        let n = f.ty(&self.store).results().len();
        let mut res = vec![Val::I32(0); n];
        f.call(&mut self.store, args, &mut res)?;
        Ok(res)
    }

    pub fn try_call(&mut self, name: &str, args: &[u32]) -> Result<u32, wasmi::Error> {
        let a: Vec<Val> = args.iter().map(|&x| Val::I32(x as i32)).collect();
        let r = self.call_vals(name, &a)?;
        Ok(r.first().map_or(0, |v| v.i32().unwrap() as u32))
    }

    pub fn call(&mut self, name: &str, args: &[u32]) -> u32 {
        self.try_call(name, args).unwrap_or_else(|e| panic!("call {name}{args:?} trapped: {e}"))
    }

    pub fn global_i32(&mut self, name: &str) -> u32 {
        let g = self.instance.get_global(&self.store, name).unwrap_or_else(|| panic!("missing global {name}"));
        g.get(&self.store).i32().unwrap() as u32
    }

    pub fn mem(&self) -> &[u8] {
        self.memory.data(&self.store)
    }
    pub fn mem_size(&self) -> usize {
        self.mem().len()
    }
}

impl Guest for WasmGuest {
    fn init(&mut self, base: u32, count: u32) {
        self.call("oxi_rt_init", &[base, count]);
    }
    fn alloc(&mut self, id: u32) -> u32 {
        self.call("oxi_alloc", &[id])
    }
    fn retain(&mut self, p: u32) {
        self.call("oxi_retain", &[p]);
    }
    fn release(&mut self, p: u32) {
        self.call("oxi_release", &[p]);
    }
    fn clone_obj(&mut self, p: u32) -> u32 {
        self.call("oxi_clone", &[p])
    }
    fn is_unique(&mut self, p: u32) -> bool {
        self.call("oxi_is_unique", &[p]) == 1
    }
    fn collect(&mut self) -> u32 {
        self.call("oxi_collect_cycles", &[])
    }
    fn gc_step(&mut self, budget: u32) -> u32 {
        self.call("oxi_gc_step", &[budget])
    }
    fn stat(&mut self, key: u32) -> u32 {
        self.call("oxi_gc_stat", &[key])
    }
    fn config(&mut self, key: u32, v: u32) {
        self.call("oxi_gc_config", &[key, v]);
    }
    fn live_objects(&mut self) -> u32 {
        self.call("oxi_live_objects", &[])
    }
    fn live_bytes(&mut self) -> u32 {
        self.call("oxi_live_bytes", &[])
    }
    fn rd(&mut self, a: u32) -> u32 {
        let m = self.mem();
        u32::from_le_bytes(m[a as usize..a as usize + 4].try_into().unwrap())
    }
    fn wr(&mut self, a: u32, v: u32) {
        let m = self.memory.data_mut(&mut self.store);
        m[a as usize..a as usize + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn alloc_bytes(&mut self, n: u32) -> u32 {
        self.call("oxi_alloc_bytes", &[n])
    }
    fn free_bytes(&mut self, p: u32, n: u32) {
        self.call("oxi_free_bytes", &[p, n]);
    }
    fn array_new(&mut self, id: u32, len: u32) -> u32 {
        self.call("oxi_array_new", &[id, len])
    }
    fn array_grow(&mut self, a: u32, cap: u32) {
        self.call("oxi_array_grow", &[a, cap]);
    }
    fn str_new(&mut self, s: &str) -> u32 {
        let tmp = self.call("oxi_alloc_bytes", &[s.len().max(1) as u32]);
        self.wr_bytes(tmp, s.as_bytes());
        let r = self.call("oxi_str_new", &[tmp, s.len() as u32]);
        self.call("oxi_free_bytes", &[tmp, s.len().max(1) as u32]);
        r
    }
    fn str_read(&mut self, p: u32) -> String {
        let len = self.rd(p);
        String::from_utf8(self.rd_bytes(p + 8, len)).unwrap()
    }
    fn rd_bytes(&mut self, a: u32, n: u32) -> Vec<u8> {
        self.mem()[a as usize..(a + n) as usize].to_vec()
    }
    fn wr_bytes(&mut self, a: u32, b: &[u8]) {
        let m = self.memory.data_mut(&mut self.store);
        m[a as usize..a as usize + b.len()].copy_from_slice(b);
    }
}
