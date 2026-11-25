//! Module-wide compilation context and final module assembly.

use crate::helpers::{self, HKey};
use crate::modlayout::Layout;
use crate::stubrt;
use crate::{
    CompileOptions, Diagnostic, RuntimeMode, E_CONFIG, E_LIMIT,
};
use oxiscript_abi::exports::{self, ExportKind as AbiKind, Provider};
use oxiscript_abi::idl::Interface;
use oxiscript_abi::manifest::{Manifest, Need, MANIFEST_SECTION};
use oxiscript_abi::WasmTy;
use oxiscript_ir::{Const, Module, Ty};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use wasm_encoder::{
    CodeSection, ConstExpr, CustomSection, DataSection, ElementSection, ElementSegment, Elements,
    EntityType, ExportKind, ExportSection, Function, FunctionSection, GlobalSection, GlobalType,
    ImportSection, MemorySection, MemoryType, RefType, TableSection, TableType, TypeSection,
    ValType,
};

/// Import module the runtime functions come from in [`RuntimeMode::Import`].
pub const RT_MODULE: &str = "oxi:rt";

pub(crate) const USER_GLOBAL_BASE: u32 = 9;

pub(crate) struct Cx<'a> {
    pub m: &'a Module,
    pub opts: &'a CompileOptions,
    pub lay: Layout,
    pub safety: bool,
    pub types: Vec<(Vec<ValType>, Vec<ValType>)>,
    type_map: HashMap<(Vec<ValType>, Vec<ValType>), u32>,
    /// (module, field, type index) of every function import, in index order.
    pub imports: Vec<(String, String, u32)>,
    pub imp_idx: HashMap<String, u32>,
    pub rt_idx: HashMap<String, u32>,
    pub fn_idx: HashMap<String, u32>,
    pub global_idx: HashMap<String, u32>,
    /// Defined functions: (type index, body).
    pub defs: Vec<(u32, Option<Function>)>,
    pub n_imports: u32,
    helpers: BTreeMap<HKey, u32>,
    pub closure_targets: Vec<String>,
    closure_ix: HashMap<String, u32>,
    pub exports: Vec<(String, ExportKind, u32)>,
    pub needs: Vec<Need>,
}

pub(crate) fn wasm_ty(t: WasmTy) -> ValType {
    match t {
        WasmTy::I64 => ValType::I64,
        WasmTy::F32 => ValType::F32,
        WasmTy::F64 => ValType::F64,
        _ => ValType::I32,
    }
}

/// Wasm parameter types of an IR signature for *host-facing* flattening:
/// `str` and `[u8]` become `(ptr, len)`.
pub(crate) fn flat_ty(t: &Ty, out: &mut Vec<ValType>) {
    match t {
        Ty::Str => out.extend([ValType::I32, ValType::I32]),
        Ty::Array(e) if **e == Ty::U8 => out.extend([ValType::I32, ValType::I32]),
        _ => out.push(crate::func::vt(t)),
    }
}

impl<'a> Cx<'a> {
    pub(crate) fn sig_type(&mut self, p: &[ValType], r: &[ValType]) -> u32 {
        let key = (p.to_vec(), r.to_vec());
        if let Some(i) = self.type_map.get(&key) {
            return *i;
        }
        let i = self.types.len() as u32;
        self.types.push(key.clone());
        self.type_map.insert(key, i);
        i
    }

    pub(crate) fn rt(&self, name: &str) -> u32 {
        self.rt_idx[name]
    }

    /// Declare a defined function; returns its function index.
    pub(crate) fn declare(&mut self, p: &[ValType], r: &[ValType]) -> u32 {
        let t = self.sig_type(p, r);
        self.defs.push((t, None));
        self.n_imports + self.defs.len() as u32 - 1
    }

    pub(crate) fn set_body(&mut self, idx: u32, f: Function) {
        self.defs[(idx - self.n_imports) as usize].1 = Some(f);
    }

    pub(crate) fn helper(&mut self, k: HKey) -> u32 {
        if let Some(i) = self.helpers.get(&k) {
            return *i;
        }
        let (p, r) = helpers::sig(k);
        let idx = self.declare(&p, &r);
        self.set_body(idx, helpers::build(k));
        self.helpers.insert(k, idx);
        idx
    }

    pub(crate) fn closure_index(&mut self, func: &str) -> u32 {
        if let Some(i) = self.closure_ix.get(func) {
            return *i;
        }
        let i = self.closure_targets.len() as u32;
        self.closure_targets.push(func.to_string());
        self.closure_ix.insert(func.to_string(), i);
        i
    }

    pub(crate) fn interface_for(&self, module_at_version: &str) -> Option<&'a Interface> {
        let o: &'a CompileOptions = self.opts;
        o.interfaces
            .iter()
            .find(|i| i.import_module() == module_at_version)
    }

    pub(crate) fn new(
        m: &'a Module,
        opts: &'a CompileOptions,
        lay: Layout,
    ) -> Result<Cx<'a>, Diagnostic> {
        let safety = m
            .attrs
            .iter()
            .any(|a| a.name == "profile" && a.args.first().map(String::as_str) == Some("safety"));
        let mut cx = Cx {
            m,
            opts,
            lay,
            safety,
            types: Vec::new(),
            type_map: HashMap::new(),
            imports: Vec::new(),
            imp_idx: HashMap::new(),
            rt_idx: HashMap::new(),
            fn_idx: HashMap::new(),
            global_idx: HashMap::new(),
            defs: Vec::new(),
            n_imports: 0,
            helpers: BTreeMap::new(),
            closure_targets: Vec::new(),
            closure_ix: HashMap::new(),
            exports: Vec::new(),
            needs: Vec::new(),
        };
        // host imports, typed boundary flattened to Wasm
        for imp in &m.imports {
            let mut p = Vec::new();
            imp.sig.params.iter().for_each(|t| flat_ty(t, &mut p));
            let r: Vec<ValType> = imp.sig.rets.iter().map(crate::func::vt).collect();
            let t = cx.sig_type(&p, &r);
            cx.imp_idx.insert(imp.name.clone(), cx.imports.len() as u32);
            cx.imports.push((imp.module.clone(), imp.field.clone(), t));
        }
        // runtime functions
        if opts.runtime == RuntimeMode::Import {
            let mut names: Vec<&'static str> = exports::EXPORTS
                .iter()
                .filter(|e| e.provider == Provider::Runtime && e.kind == AbiKind::Func)
                .map(|e| e.name)
                .collect();
            names.push("oxi_rt_init");
            for n in names {
                let (p, r) = if n == "oxi_rt_init" {
                    (vec![ValType::I32, ValType::I32], vec![])
                } else {
                    let e = exports::find(n).expect("export row");
                    (
                        e.params.iter().map(|t| wasm_ty(*t)).collect::<Vec<_>>(),
                        e.results.iter().map(|t| wasm_ty(*t)).collect::<Vec<_>>(),
                    )
                };
                let t = cx.sig_type(&p, &r);
                cx.rt_idx.insert(n.to_string(), cx.imports.len() as u32);
                cx.imports.push((RT_MODULE.to_string(), n.to_string(), t));
            }
        }
        cx.n_imports = cx.imports.len() as u32;
        if opts.runtime == RuntimeMode::Stub {
            for n in stubrt::FUNCS {
                let (p, r) = stubrt::sig(n);
                let idx = cx.declare(&p, &r);
                cx.rt_idx.insert((*n).to_string(), idx);
            }
        }
        // user functions
        for f in &m.funcs {
            let p: Vec<ValType> = f.params.iter().map(|p| crate::func::vt(f.ty(p.value))).collect();
            let r: Vec<ValType> = f.rets.iter().map(crate::func::vt).collect();
            let idx = cx.declare(&p, &r);
            cx.fn_idx.insert(f.name.clone(), idx);
        }
        for (i, g) in m.globals.iter().enumerate() {
            cx.global_idx
                .insert(g.name.clone(), USER_GLOBAL_BASE + i as u32);
        }
        Ok(cx)
    }

    /// Determine the manifest `needs` (explicit option, else derived from the imports).
    pub(crate) fn resolve_needs(&mut self) -> Result<(), Diagnostic> {
        let needs = if let Some(n) = &self.opts.needs {
            n.clone()
        } else {
            let mut v: Vec<Need> = Vec::new();
            for imp in &self.m.imports {
                let Some(itf) = self.interface_for(&imp.module) else {
                    return Err(Diagnostic::new(
                        E_CONFIG,
                        format!("import @{} needs `{}`, which is not among the granted interfaces", imp.name, imp.module),
                        None,
                    ));
                };
                let n = Need {
                    module: itf.module.clone(),
                    version: itf.version,
                    interface: itf.name.clone(),
                };
                if !v.contains(&n) {
                    v.push(n);
                }
            }
            v
        };
        for imp in &self.m.imports {
            if !needs
                .iter()
                .any(|n| format!("{}@{}", n.module, n.version) == imp.module)
            {
                return Err(Diagnostic::new(
                    E_CONFIG,
                    format!("import @{} uses `{}`, which is not in the manifest needs", imp.name, imp.module),
                    None,
                ));
            }
        }
        match self.m.func("main") {
            Some(main) => {
                if main.params.len() != needs.len()
                    || main.params.iter().any(|p| *main.ty(p.value) != Ty::I32)
                    || !main.rets.is_empty()
                {
                    return Err(Diagnostic::new(
                        E_CONFIG,
                        format!(
                            "`main` must take exactly {} i32 capability handle(s) (one per needs line) and return nothing",
                            needs.len()
                        ),
                        Some("main".into()),
                    ));
                }
            }
            None if !needs.is_empty() => {
                return Err(Diagnostic::new(
                    E_CONFIG,
                    "the module needs capabilities but has no `main` to receive their handles",
                    None,
                ));
            }
            None => {}
        }
        self.needs = needs;
        Ok(())
    }

    pub(crate) fn assemble(self, static_end: u32) -> Result<Vec<u8>, Diagnostic> {
        let stub = self.opts.runtime == RuntimeMode::Stub;
        if let Some((p, r)) = self
            .types
            .iter()
            .find(|(p, r)| p.len() > crate::LIMIT_PARAMS || r.len() > crate::LIMIT_PARAMS)
        {
            return Err(Diagnostic::new(
                E_LIMIT,
                format!(
                    "a function has {} parameters and {} results; the host allows at most {} of each",
                    p.len(),
                    r.len(),
                    crate::LIMIT_PARAMS
                ),
                None,
            ));
        }
        if self.defs.len() + self.imports.len() > crate::LIMIT_FUNCS
            || self.m.globals.len() + USER_GLOBAL_BASE as usize > crate::LIMIT_GLOBALS
        {
            return Err(Diagnostic::new(
                E_LIMIT,
                "too many functions or globals for the host limits",
                None,
            ));
        }
        let mut module = wasm_encoder::Module::new();
        let mut ts = TypeSection::new();
        for (p, r) in &self.types {
            ts.ty().function(p.iter().copied(), r.iter().copied());
        }
        module.section(&ts);
        let mut is = ImportSection::new();
        for (m, f, t) in &self.imports {
            is.import(m, f, EntityType::Function(*t));
        }
        module.section(&is);
        let mut fs = FunctionSection::new();
        for (t, _) in &self.defs {
            fs.function(*t);
        }
        module.section(&fs);
        let mut tabs = TableSection::new();
        let nt = self.closure_targets.len() as u64;
        tabs.table(TableType {
            element_type: RefType::FUNCREF,
            table64: false,
            minimum: nt,
            maximum: Some(nt),
            shared: false,
        });
        module.section(&tabs);
        let pages = (u64::from(static_end) + 4096 + 65536).div_ceil(65536);
        let mut mems = MemorySection::new();
        mems.memory(MemoryType {
            minimum: pages,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        module.section(&mems);
        // globals: fixed 0..9, then user globals
        let mut gs = GlobalSection::new();
        let g = |mutable| GlobalType {
            val_type: ValType::I32,
            mutable,
            shared: false,
        };
        let heap = static_end + 4096;
        gs.global(g(true), &ConstExpr::i32_const(0)); // 0 trap kind
        gs.global(g(false), &ConstExpr::i32_const(oxiscript_abi::layout::ABI_VERSION as i32)); // 1
        gs.global(g(false), &ConstExpr::i32_const(self.lay.typedesc_base as i32)); // 2
        gs.global(g(false), &ConstExpr::i32_const(self.lay.typedesc_count as i32)); // 3
        gs.global(g(false), &ConstExpr::i32_const(static_end as i32)); // 4 scratch ptr
        gs.global(g(false), &ConstExpr::i32_const(4096)); // 5 scratch cap
        gs.global(g(true), &ConstExpr::i32_const(heap as i32)); // 6
        gs.global(g(true), &ConstExpr::i32_const(0)); // 7
        gs.global(g(true), &ConstExpr::i32_const(0)); // 8
        for gl in &self.m.globals {
            let vt = crate::func::vt(&gl.ty);
            let ce = match (&gl.init, vt) {
                (Some(Const::Bool(b)), _) => ConstExpr::i32_const(i32::from(*b)),
                (Some(Const::Int(v)), ValType::I64) => ConstExpr::i64_const(*v as i64),
                (Some(Const::Int(v)), _) => ConstExpr::i32_const(*v as i64 as i32),
                (Some(Const::F32(b)), _) => ConstExpr::f32_const(wasm_encoder::Ieee32::new(*b)),
                (Some(Const::F64(b)), _) => ConstExpr::f64_const(wasm_encoder::Ieee64::new(*b)),
                (None, ValType::I64) => ConstExpr::i64_const(0),
                (None, ValType::F32) => ConstExpr::f32_const(wasm_encoder::Ieee32::new(0)),
                (None, ValType::F64) => ConstExpr::f64_const(wasm_encoder::Ieee64::new(0)),
                (None, _) => ConstExpr::i32_const(0),
            };
            gs.global(
                GlobalType {
                    val_type: vt,
                    mutable: true,
                    shared: false,
                },
                &ce,
            );
        }
        module.section(&gs);
        let mut es = ExportSection::new();
        es.export(exports::MEMORY, ExportKind::Memory, 0);
        es.export(exports::ABI_VERSION, ExportKind::Global, 1);
        es.export(exports::TRAP_KIND, ExportKind::Global, 0);
        es.export(exports::TYPEDESC_BASE, ExportKind::Global, 2);
        es.export(exports::TYPEDESC_COUNT, ExportKind::Global, 3);
        if stub {
            es.export(exports::SCRATCH_PTR, ExportKind::Global, 4);
        }
        es.export(exports::SCRATCH_CAP, ExportKind::Global, 5);
        for (n, k, i) in &self.exports {
            es.export(n, *k, *i);
        }
        module.section(&es);
        if nt > 0 {
            let idxs: Vec<u32> = self
                .closure_targets
                .iter()
                .map(|n| self.fn_idx[n])
                .collect();
            let mut elems = ElementSection::new();
            elems.segment(ElementSegment {
                mode: wasm_encoder::ElementMode::Active {
                    table: None,
                    offset: &ConstExpr::i32_const(0),
                },
                elements: Elements::Functions(Cow::Owned(idxs)),
            });
            module.section(&elems);
        }
        let mut cs = CodeSection::new();
        for (i, (_, body)) in self.defs.iter().enumerate() {
            let Some(b) = body else {
                return Err(Diagnostic::new(
                    crate::I_INTERNAL,
                    format!("function {} has no body", i as u32 + self.n_imports),
                    None,
                ));
            };
            cs.function(b);
        }
        module.section(&cs);
        let mut ds = DataSection::new();
        ds.active(
            0,
            &ConstExpr::i32_const(oxiscript_abi::layout::RESERVED_LOW_BYTES as i32),
            self.lay.segment.iter().copied(),
        );
        module.section(&ds);
        let mut mf = Manifest::new(edition(self.m), profile(self.m));
        mf.needs = self.needs.clone();
        let text = mf.print();
        module.section(&CustomSection {
            name: Cow::Borrowed(MANIFEST_SECTION),
            data: Cow::Borrowed(text.as_bytes()),
        });
        let bytes = module.finish();
        if bytes.len() > (1 << 28) {
            return Err(Diagnostic::new(E_LIMIT, "output module too large", None));
        }
        Ok(bytes)
    }
}

pub(crate) fn edition(m: &Module) -> u32 {
    m.attrs
        .iter()
        .find(|a| a.name == "edition")
        .and_then(|a| a.args.first())
        .and_then(|s| s.parse().ok())
        .unwrap_or(2026)
}

pub(crate) fn profile(m: &Module) -> &str {
    m.attrs
        .iter()
        .find(|a| a.name == "profile")
        .and_then(|a| a.args.first())
        .map_or("default", String::as_str)
}

