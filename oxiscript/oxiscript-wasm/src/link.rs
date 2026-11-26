//! Static Core WebAssembly linker: merges a compiler-generated program module
//! (which imports runtime functions from `oxi:rt`) with the embedded
//! `oxiscript-rc` memory runtime into ONE self-contained Core Wasm module.
//!
//! Pure Rust on `wasmparser` + `wasm-encoder` (no external tools), so it runs
//! identically in the CLI (`oxi build`/`oxi run`) and in the browser playground
//! compiled to `wasm32-unknown-unknown`.
//!
//! # Layout invariant
//! * Runtime (`oxiscript-rc`) keeps its original low addresses `[0, rt_heap_base)`
//!   (shadow stack, `.rodata`/`.data`/`.bss`, `oxi_scratch_ptr`) and its original
//!   function-table slots `[0, rt_table_len)`.
//! * Compiler places the program's static data starting at `p_data_base =
//!   align16(rt_heap_base)` and its closure table indices starting at `rt_table_len`.
//! * `oxi_init` calls `oxi_rt_init(typedesc_base, typedesc_count, static_end)`
//!   so the runtime's allocator starts right after all static data.
//! * The runtime's single import `("oxi:rt", "oxi_set_trap")` is resolved to the
//!   compiler's internal helper that writes the single exported mutable global
//!   `oxi_trap_kind`. No `oxi:rt` imports remain in the merged module.

use crate::cx::RT_MODULE;
use crate::{Diagnostic, E_LIMIT, I_INTERNAL};
use oxiscript_abi::exports::{self, ExportKind as AbiKind, Provider};
use oxiscript_abi::layout::RESERVED_LOW_BYTES;
use oxiscript_abi::manifest::MANIFEST_SECTION;
use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::OnceLock;
use wasm_encoder::reencode::{self, Reencode};
use wasm_encoder::{
    CodeSection, ConstExpr, CustomSection, DataCountSection, DataSection, ElementSection,
    ElementSegment, Elements, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    GlobalSection, GlobalType, ImportSection, MemorySection, MemoryType, Module, RefType,
    TableSection, TableType, TypeSection, ValType,
};
use wasmparser::{
    CompositeInnerType, DataKind, ElementItems, ElementKind, ExternalKind, FunctionBody, Operator,
    Parser, Payload, TypeRef,
};

/// Embedded `oxiscript-rc` compiled to `wasm32-unknown-unknown` (`--release`)
/// with the exact ABI 1.0 Core Wasm feature set by `build.rs`.
pub(crate) const RUNTIME_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/oxiscript_rc.wasm"));

/// Temporary export emitted by the compiler in `RuntimeMode::Linked` so the
/// merger can wire the runtime's `("oxi:rt", "oxi_set_trap")` import to it;
/// stripped from the final merged export section.
pub(crate) const SET_TRAP_EXPORT: &str = "__oxi_set_trap";

/// Layout facts extracted once from [`RUNTIME_WASM`] so the compiler can place
/// its static data and closure table entries without overlap.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeLayout {
    /// 16-aligned base address for the program's static data (`>= __heap_base`).
    pub p_data_base: u32,
    /// Number of function table slots reserved for the runtime (`0..table_len`).
    pub table_len: u32,
    /// Address of the runtime's `oxi_scratch_ptr` static buffer.
    pub scratch_ptr: u32,
}

struct ParsedModule<'a> {
    types: Vec<(Vec<ValType>, Vec<ValType>)>,
    func_imports: Vec<(String, String, u32)>,
    def_func_types: Vec<u32>,
    bodies: Vec<FunctionBody<'a>>,
    table_len: u32,
    mem_min_pages: u64,
    globals: Vec<(GlobalType, wasmparser::ConstExpr<'a>)>,
    exports: Vec<(String, ExternalKind, u32)>,
    active_elems: Vec<(i32, Vec<u32>)>,
    data_segments: Vec<(Option<i32>, &'a [u8])>,
    has_data_count: bool,
    custom_sections: Vec<(&'a str, &'a [u8])>,
}

fn err(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(I_INTERNAL, msg, None)
}

fn conv_val_type(vt: wasmparser::ValType) -> Result<ValType, Diagnostic> {
    match vt {
        wasmparser::ValType::I32 => Ok(ValType::I32),
        wasmparser::ValType::I64 => Ok(ValType::I64),
        wasmparser::ValType::F32 => Ok(ValType::F32),
        wasmparser::ValType::F64 => Ok(ValType::F64),
        other => Err(err(format!("linker: unsupported value type {other:?}"))),
    }
}

fn eval_const_i32(expr: &wasmparser::ConstExpr<'_>) -> Result<i32, Diagnostic> {
    let mut r = expr.get_operators_reader();
    let op = r
        .read()
        .map_err(|e| err(format!("linker: bad const expr: {e}")))?;
    let val = match op {
        Operator::I32Const { value } => value,
        other => return Err(err(format!("linker: expected i32.const, got {other:?}"))),
    };
    let end = r
        .read()
        .map_err(|e| err(format!("linker: bad const expr end: {e}")))?;
    if !matches!(end, Operator::End) {
        return Err(err(format!("linker: expected end in const expr, got {end:?}")));
    }
    Ok(val)
}

fn parse_module(bytes: &[u8]) -> Result<ParsedModule<'_>, Diagnostic> {
    let mut m = ParsedModule {
        types: Vec::new(),
        func_imports: Vec::new(),
        def_func_types: Vec::new(),
        bodies: Vec::new(),
        table_len: 0,
        mem_min_pages: 0,
        globals: Vec::new(),
        exports: Vec::new(),
        active_elems: Vec::new(),
        data_segments: Vec::new(),
        has_data_count: false,
        custom_sections: Vec::new(),
    };
    let mut tables_seen = 0u32;
    let mut mems_seen = 0u32;

    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|e| err(format!("linker: wasm parse error: {e}")))?;
        match payload {
            Payload::Version { .. } | Payload::End(_) => {}
            Payload::TypeSection(r) => {
                for rec in r {
                    let rec = rec.map_err(|e| err(format!("linker: type section: {e}")))?;
                    for sub in rec.into_types() {
                        match sub.composite_type.inner {
                            CompositeInnerType::Func(ft) => {
                                let p: Vec<ValType> = ft
                                    .params()
                                    .iter()
                                    .map(|t| conv_val_type(*t))
                                    .collect::<Result<_, _>>()?;
                                let res: Vec<ValType> = ft
                                    .results()
                                    .iter()
                                    .map(|t| conv_val_type(*t))
                                    .collect::<Result<_, _>>()?;
                                m.types.push((p, res));
                            }
                            other => {
                                return Err(err(format!("linker: non-function type {other:?}")));
                            }
                        }
                    }
                }
            }
            Payload::ImportSection(r) => {
                for imp in r.into_imports() {
                    let imp = imp.map_err(|e| err(format!("linker: import section: {e}")))?;
                    match imp.ty {
                        TypeRef::Func(ti) => {
                            m.func_imports
                                .push((imp.module.to_string(), imp.name.to_string(), ti));
                        }
                        other => {
                            return Err(err(format!(
                                "linker: non-function import `{}`.`{}` ({other:?})",
                                imp.module, imp.name
                            )));
                        }
                    }
                }
            }
            Payload::FunctionSection(r) => {
                for f in r {
                    m.def_func_types
                        .push(f.map_err(|e| err(format!("linker: function section: {e}")))?);
                }
            }
            Payload::TableSection(r) => {
                for t in r {
                    let t = t.map_err(|e| err(format!("linker: table section: {e}")))?;
                    tables_seen += 1;
                    if tables_seen > 1 || t.ty.table64 || t.ty.shared {
                        return Err(err("linker: unsupported table configuration"));
                    }
                    m.table_len = u32::try_from(t.ty.initial)
                        .map_err(|_| err("linker: table size overflows u32"))?;
                }
            }
            Payload::MemorySection(r) => {
                for mem in r {
                    let mem = mem.map_err(|e| err(format!("linker: memory section: {e}")))?;
                    mems_seen += 1;
                    if mems_seen > 1 || mem.memory64 || mem.shared {
                        return Err(err("linker: unsupported memory configuration"));
                    }
                    m.mem_min_pages = mem.initial;
                }
            }
            Payload::GlobalSection(r) => {
                for g in r {
                    let g = g.map_err(|e| err(format!("linker: global section: {e}")))?;
                    let gt = GlobalType {
                        val_type: conv_val_type(g.ty.content_type)?,
                        mutable: g.ty.mutable,
                        shared: g.ty.shared,
                    };
                    m.globals.push((gt, g.init_expr));
                }
            }
            Payload::ExportSection(r) => {
                for e in r {
                    let e = e.map_err(|e| err(format!("linker: export section: {e}")))?;
                    m.exports.push((e.name.to_string(), e.kind, e.index));
                }
            }
            Payload::StartSection { .. } => {
                return Err(err("linker: start section is forbidden by ABI 1.0"));
            }
            Payload::ElementSection(r) => {
                for elem in r {
                    let elem = elem.map_err(|e| err(format!("linker: element section: {e}")))?;
                    let off = match elem.kind {
                        ElementKind::Active {
                            table_index,
                            offset_expr,
                        } => {
                            if table_index.unwrap_or(0) != 0 {
                                return Err(err("linker: element targets non-zero table"));
                            }
                            eval_const_i32(&offset_expr)?
                        }
                        _ => return Err(err("linker: non-active element segment unsupported")),
                    };
                    let mut funcs = Vec::new();
                    match elem.items {
                        ElementItems::Functions(fr) => {
                            for f in fr {
                                funcs.push(f.map_err(|e| err(format!("linker: elem func: {e}")))?);
                            }
                        }
                        ElementItems::Expressions(_, er) => {
                            for e in er {
                                let ce = e.map_err(|e| err(format!("linker: elem expr: {e}")))?;
                                let mut rdr = ce.get_operators_reader();
                                match rdr
                                    .read()
                                    .map_err(|e| err(format!("linker: elem expr op: {e}")))?
                                {
                                    Operator::RefFunc { function_index } => {
                                        funcs.push(function_index);
                                    }
                                    other => {
                                        return Err(err(format!(
                                            "linker: unsupported elem expr {other:?}"
                                        )));
                                    }
                                }
                            }
                        }
                    }
                    m.active_elems.push((off, funcs));
                }
            }
            Payload::DataCountSection { .. } => {
                m.has_data_count = true;
            }
            Payload::CodeSectionStart { .. } => {}
            Payload::CodeSectionEntry(body) => {
                m.bodies.push(body);
            }
            Payload::DataSection(r) => {
                for seg in r {
                    let seg = seg.map_err(|e| err(format!("linker: data section: {e}")))?;
                    let off = match seg.kind {
                        DataKind::Active {
                            memory_index,
                            offset_expr,
                        } => {
                            if memory_index != 0 {
                                return Err(err("linker: data segment targets non-zero memory"));
                            }
                            Some(eval_const_i32(&offset_expr)?)
                        }
                        DataKind::Passive => None,
                    };
                    m.data_segments.push((off, seg.data));
                }
            }
            Payload::CustomSection(r) => {
                m.custom_sections.push((r.name(), r.data()));
            }
            other => {
                return Err(err(format!("linker: unexpected Wasm section {other:?}")));
            }
        }
    }
    if m.def_func_types.len() != m.bodies.len() {
        return Err(err("linker: function/code section length mismatch"));
    }
    Ok(m)
}

fn inspect_runtime() -> Result<RuntimeLayout, String> {
    // `libcore` in modern Rust toolchains may emit reference-types-encoded
    // element/table metadata; the merger normalizes sections and `compile_inner`
    // validates the final merged module with strict `allowed_features()`.
    let rt_feats = crate::allowed_features() | wasmparser::WasmFeatures::REFERENCE_TYPES;
    wasmparser::Validator::new_with_features(rt_feats)
        .validate_all(RUNTIME_WASM)
        .map_err(|e| format!("embedded runtime failed validation: {e}"))?;
    let r = parse_module(RUNTIME_WASM).map_err(|d| d.message)?;
    if r.func_imports.len() != 1
        || r.func_imports[0].0 != RT_MODULE
        || r.func_imports[0].1 != "oxi_set_trap"
    {
        return Err(format!(
            "embedded runtime must import only (`{RT_MODULE}`, `oxi_set_trap`), got {:?}",
            r.func_imports
        ));
    }
    let trap_ty = &r.types[r.func_imports[0].2 as usize];
    if trap_ty.0 != [ValType::I32] || !trap_ty.1.is_empty() {
        return Err(format!("embedded runtime `oxi_set_trap` has unexpected type {trap_ty:?}"));
    }

    let global_i32 = |name: &str| -> Result<u32, String> {
        let (_, _, idx) = r
            .exports
            .iter()
            .find(|(n, k, _)| n == name && *k == ExternalKind::Global)
            .ok_or_else(|| format!("embedded runtime missing global export `{name}`"))?;
        let (_, expr) = r
            .globals
            .get(*idx as usize)
            .ok_or_else(|| format!("embedded runtime global `{name}` index out of range"))?;
        let v = eval_const_i32(expr).map_err(|d| d.message)?;
        u32::try_from(v).map_err(|_| format!("embedded runtime global `{name}` is negative: {v}"))
    };

    let rt_heap_base = global_i32("__heap_base")?;
    let scratch_ptr = global_i32(exports::SCRATCH_PTR)?;
    if scratch_ptr < RESERVED_LOW_BYTES
        || scratch_ptr % 8 != 0
        || u64::from(scratch_ptr) + u64::from(exports::SCRATCH_MIN_CAP) > u64::from(rt_heap_base)
    {
        return Err(format!(
            "embedded runtime scratch ({scratch_ptr:#x}) invalid for __heap_base ({rt_heap_base:#x})"
        ));
    }
    for (off, data) in &r.data_segments {
        if let Some(o) = off {
            let start = u32::try_from(*o).map_err(|_| "negative runtime data offset".to_string())?;
            let end = u64::from(start) + data.len() as u64;
            if end > u64::from(rt_heap_base) {
                return Err(format!(
                    "embedded runtime data [{start:#x}, {end:#x}) exceeds __heap_base {rt_heap_base:#x}"
                ));
            }
        }
    }
    for spec in exports::EXPORTS
        .iter()
        .filter(|e| e.provider == Provider::Runtime && e.kind == AbiKind::Func)
    {
        if !r
            .exports
            .iter()
            .any(|(n, k, _)| n == spec.name && *k == ExternalKind::Func)
        {
            return Err(format!("embedded runtime missing ABI export `{}`", spec.name));
        }
    }

    let p_data_base = ((rt_heap_base + 15) & !15).max(RESERVED_LOW_BYTES);
    Ok(RuntimeLayout {
        p_data_base,
        table_len: r.table_len,
        scratch_ptr,
    })
}

pub(crate) fn runtime_layout() -> Result<RuntimeLayout, Diagnostic> {
    static INFO: OnceLock<Result<RuntimeLayout, String>> = OnceLock::new();
    INFO.get_or_init(inspect_runtime)
        .clone()
        .map_err(err)
}

struct Remapper<'a> {
    types: &'a [u32],
    funcs: &'a [u32],
    globals: &'a [u32],
    datas: &'a [u32],
}

impl Reencode for Remapper<'_> {
    type Error = Infallible;

    fn type_index(&mut self, ty: u32) -> Result<u32, reencode::Error<Self::Error>> {
        Ok(self.types[ty as usize])
    }

    fn function_index(&mut self, func: u32) -> Result<u32, reencode::Error<Self::Error>> {
        Ok(self.funcs[func as usize])
    }

    fn global_index(&mut self, global: u32) -> Result<u32, reencode::Error<Self::Error>> {
        Ok(self.globals[global as usize])
    }

    fn data_index(&mut self, data: u32) -> Result<u32, reencode::Error<Self::Error>> {
        Ok(self.datas[data as usize])
    }
}

fn reencode_body(
    rm: &mut Remapper<'_>,
    body: &FunctionBody<'_>,
) -> Result<Function, Diagnostic> {
    let mut locals_vec: Vec<(u32, ValType)> = Vec::new();
    let lr = body
        .get_locals_reader()
        .map_err(|e| err(format!("linker: locals reader: {e}")))?;
    for l in lr {
        let (cnt, vt) = l.map_err(|e| err(format!("linker: local: {e}")))?;
        locals_vec.push((cnt, conv_val_type(vt)?));
    }
    let mut func = Function::new(locals_vec);
    let ops = body
        .get_operators_reader()
        .map_err(|e| err(format!("linker: operators reader: {e}")))?;
    for op in ops {
        let op = op.map_err(|e| err(format!("linker: operator: {e}")))?;
        let ins = rm
            .instruction(op)
            .map_err(|e| err(format!("linker: reencode instruction: {e}")))?;
        func.instruction(&ins);
    }
    Ok(func)
}

/// Merge a compiler-generated program module (compiled with `RuntimeMode::Linked`
/// offsets) and [`RUNTIME_WASM`] into one Core Wasm module.
pub(crate) fn merge(program_wasm: &[u8]) -> Result<(Vec<u8>, usize), Diagnostic> {
    let p = parse_module(program_wasm)?;
    let r = parse_module(RUNTIME_WASM)?;

    // 1. Deduplicated types.
    let mut merged_types: Vec<(Vec<ValType>, Vec<ValType>)> = Vec::new();
    let mut type_intern: HashMap<(Vec<ValType>, Vec<ValType>), u32> = HashMap::new();
    let mut intern = |sig: &(Vec<ValType>, Vec<ValType>)| -> u32 {
        if let Some(&i) = type_intern.get(sig) {
            return i;
        }
        let i = merged_types.len() as u32;
        merged_types.push(sig.clone());
        type_intern.insert(sig.clone(), i);
        i
    };
    let p_type_map: Vec<u32> = p.types.iter().map(&mut intern).collect();
    let r_type_map: Vec<u32> = r.types.iter().map(&mut intern).collect();

    // 2. Partition P's imports into host imports (kept) and `oxi:rt` imports (resolved).
    let mut host_imports: Vec<(String, String, u32)> = Vec::new();
    let mut p_imp_target: Vec<Option<u32>> = Vec::with_capacity(p.func_imports.len());
    let mut p_rt_imp_names: Vec<(usize, String)> = Vec::new();
    for (idx, (m, f, ti)) in p.func_imports.iter().enumerate() {
        if m == RT_MODULE {
            p_imp_target.push(None);
            p_rt_imp_names.push((idx, f.clone()));
        } else {
            let new_idx = host_imports.len() as u32;
            host_imports.push((m.clone(), f.clone(), p_type_map[*ti as usize]));
            p_imp_target.push(Some(new_idx));
        }
    }

    let m_n_imports = host_imports.len() as u32;
    let p_n_imports = p.func_imports.len() as u32;
    let r_n_imports = r.func_imports.len() as u32;
    let p_n_defs = p.def_func_types.len() as u32;
    let r_n_defs = r.def_func_types.len() as u32;
    let total_funcs = (m_n_imports + p_n_defs + r_n_defs) as usize;
    let total_globals = p.globals.len() + r.globals.len();
    if total_funcs > crate::LIMIT_FUNCS || total_globals > crate::LIMIT_GLOBALS {
        return Err(Diagnostic::new(
            E_LIMIT,
            "merged module exceeds host function or global limits",
            None,
        ));
    }

    let p_def_base = m_n_imports;
    let r_def_base = m_n_imports + p_n_defs;

    // 3. Function index maps for R and P.
    // Find P's `__oxi_set_trap` helper so R's import 0 resolves to it.
    let set_trap_p_idx = p
        .exports
        .iter()
        .find(|(n, k, _)| n == SET_TRAP_EXPORT && *k == ExternalKind::Func)
        .map(|(_, _, i)| *i)
        .ok_or_else(|| err("linker: program module missing `__oxi_set_trap` helper"))?;
    if set_trap_p_idx < p_n_imports {
        return Err(err("linker: `__oxi_set_trap` must be a defined function"));
    }
    let set_trap_m_idx = p_def_base + (set_trap_p_idx - p_n_imports);

    let mut r_func_map: Vec<u32> = Vec::with_capacity((r_n_imports + r_n_defs) as usize);
    for (i, (m, f, _)) in r.func_imports.iter().enumerate() {
        if m == RT_MODULE && f == "oxi_set_trap" {
            r_func_map.push(set_trap_m_idx);
        } else {
            return Err(err(format!(
                "linker: unexpected runtime import #{i} `{m}`.`{f}`"
            )));
        }
    }
    for i in 0..r_n_defs {
        r_func_map.push(r_def_base + i);
    }

    // Map R's exported function names -> merged function indices.
    let mut r_export_funcs: HashMap<&str, u32> = HashMap::new();
    for (name, kind, idx) in &r.exports {
        if *kind == ExternalKind::Func {
            r_export_funcs.insert(name.as_str(), r_func_map[*idx as usize]);
        }
    }

    let mut p_func_map: Vec<u32> = Vec::with_capacity((p_n_imports + p_n_defs) as usize);
    for (i, slot) in p_imp_target.iter().enumerate() {
        if let Some(new_idx) = slot {
            p_func_map.push(*new_idx);
        } else {
            let name = &p_rt_imp_names
                .iter()
                .find(|(idx, _)| *idx == i)
                .expect("rt import recorded")
                .1;
            let target = *r_export_funcs.get(name.as_str()).ok_or_else(|| {
                err(format!(
                    "linker: runtime does not export required function `{name}`"
                ))
            })?;
            p_func_map.push(target);
        }
    }
    for i in 0..p_n_defs {
        p_func_map.push(p_def_base + i);
    }

    // 4. Global and data segment index maps.
    let p_n_globals = p.globals.len() as u32;
    let p_global_map: Vec<u32> = (0..p_n_globals).collect();
    let r_global_map: Vec<u32> = (0..r.globals.len() as u32)
        .map(|i| p_n_globals + i)
        .collect();

    let r_n_data = r.data_segments.len() as u32;
    let r_data_map: Vec<u32> = (0..r_n_data).collect();
    let p_data_map: Vec<u32> = (0..p.data_segments.len() as u32)
        .map(|i| r_n_data + i)
        .collect();

    let mut p_rm = Remapper {
        types: &p_type_map,
        funcs: &p_func_map,
        globals: &p_global_map,
        datas: &p_data_map,
    };
    let mut r_rm = Remapper {
        types: &r_type_map,
        funcs: &r_func_map,
        globals: &r_global_map,
        datas: &r_data_map,
    };

    // 5. Emit the single merged Core Wasm module.
    let mut out = Module::new();

    // Type section
    let mut ts = TypeSection::new();
    for (params, results) in &merged_types {
        ts.ty()
            .function(params.iter().copied(), results.iter().copied());
    }
    out.section(&ts);

    // Import section (host imports only)
    if !host_imports.is_empty() {
        let mut is = ImportSection::new();
        for (m, f, ti) in &host_imports {
            is.import(m, f, EntityType::Function(*ti));
        }
        out.section(&is);
    }

    // Function section
    let mut fs = FunctionSection::new();
    for &ti in &p.def_func_types {
        fs.function(p_type_map[ti as usize]);
    }
    for &ti in &r.def_func_types {
        fs.function(r_type_map[ti as usize]);
    }
    out.section(&fs);

    // Table section (single funcref table covering R's slots + P's closure targets)
    let nt = u64::from(p.table_len.max(r.table_len));
    let mut tabs = TableSection::new();
    tabs.table(TableType {
        element_type: RefType::FUNCREF,
        table64: false,
        minimum: nt,
        maximum: Some(nt),
        shared: false,
    });
    out.section(&tabs);

    // Memory section (single 32-bit memory)
    let min_pages = p.mem_min_pages.max(r.mem_min_pages).max(1);
    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: min_pages,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    out.section(&mems);

    // Global section (P's globals, then R's globals)
    let mut gs = GlobalSection::new();
    for (gt, expr) in &p.globals {
        let ce = p_rm
            .const_expr(expr.clone())
            .map_err(|e| err(format!("linker: p global init: {e}")))?;
        gs.global(*gt, &ce);
    }
    for (gt, expr) in &r.globals {
        let ce = r_rm
            .const_expr(expr.clone())
            .map_err(|e| err(format!("linker: r global init: {e}")))?;
        gs.global(*gt, &ce);
    }
    out.section(&gs);

    // Export section: P's exports (minus internal `__oxi_set_trap`) + R's ABI func exports.
    let mut es = ExportSection::new();
    for (name, kind, idx) in &p.exports {
        if name == SET_TRAP_EXPORT {
            continue;
        }
        match kind {
            ExternalKind::Func => {
                es.export(name, ExportKind::Func, p_func_map[*idx as usize]);
            }
            ExternalKind::Global => {
                es.export(name, ExportKind::Global, p_global_map[*idx as usize]);
            }
            ExternalKind::Memory => {
                es.export(name, ExportKind::Memory, *idx);
            }
            ExternalKind::Table => {
                es.export(name, ExportKind::Table, *idx);
            }
            other => {
                return Err(err(format!("linker: unexpected export kind {other:?}")));
            }
        }
    }
    for spec in exports::EXPORTS
        .iter()
        .filter(|e| e.provider == Provider::Runtime && e.kind == AbiKind::Func)
    {
        let idx = *r_export_funcs
            .get(spec.name)
            .ok_or_else(|| err(format!("linker: missing runtime export `{}`", spec.name)))?;
        es.export(spec.name, ExportKind::Func, idx);
    }
    out.section(&es);

    // Element section: R's active elements, then P's active elements.
    if !r.active_elems.is_empty() || !p.active_elems.is_empty() {
        let mut elems = ElementSection::new();
        for (off, funcs) in &r.active_elems {
            let remapped: Vec<u32> = funcs.iter().map(|&f| r_func_map[f as usize]).collect();
            elems.segment(ElementSegment {
                mode: wasm_encoder::ElementMode::Active {
                    table: None,
                    offset: &ConstExpr::i32_const(*off),
                },
                elements: Elements::Functions(Cow::Owned(remapped)),
            });
        }
        for (off, funcs) in &p.active_elems {
            let remapped: Vec<u32> = funcs.iter().map(|&f| p_func_map[f as usize]).collect();
            elems.segment(ElementSegment {
                mode: wasm_encoder::ElementMode::Active {
                    table: None,
                    offset: &ConstExpr::i32_const(*off),
                },
                elements: Elements::Functions(Cow::Owned(remapped)),
            });
        }
        out.section(&elems);
    }

    // DataCount section if either module uses passive data segments / memory.init.
    let total_data = (r.data_segments.len() + p.data_segments.len()) as u32;
    if r.has_data_count || p.has_data_count {
        out.section(&DataCountSection { count: total_data });
    }

    // Code section: P's bodies, then R's bodies.
    let mut cs = CodeSection::new();
    for body in &p.bodies {
        cs.function(&reencode_body(&mut p_rm, body)?);
    }
    for body in &r.bodies {
        cs.function(&reencode_body(&mut r_rm, body)?);
    }
    out.section(&cs);

    // Data section: R's data segments, then P's data segments.
    let mut ds = DataSection::new();
    for (off, data) in &r.data_segments {
        match off {
            Some(o) => {
                ds.active(0, &ConstExpr::i32_const(*o), data.iter().copied());
            }
            None => {
                ds.passive(data.iter().copied());
            }
        }
    }
    for (off, data) in &p.data_segments {
        match off {
            Some(o) => {
                ds.active(0, &ConstExpr::i32_const(*o), data.iter().copied());
            }
            None => {
                ds.passive(data.iter().copied());
            }
        }
    }
    out.section(&ds);

    // Custom section: preserve `oxi.manifest` from P.
    for (name, data) in &p.custom_sections {
        if *name == MANIFEST_SECTION {
            out.section(&CustomSection {
                name: Cow::Borrowed(*name),
                data: Cow::Borrowed(*data),
            });
        }
    }

    let bytes = out.finish();
    if bytes.len() > (1 << 28) {
        return Err(Diagnostic::new(E_LIMIT, "output module too large", None));
    }
    Ok((bytes, (p_n_defs + r_n_defs) as usize))
}
