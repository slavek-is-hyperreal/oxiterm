//! Pre-instantiation validation of a module against the ABI.
//!
//! Order (each step has its own reason code, see `registry/codes.txt`):
//! 1. size (`E0409`), 2. engine validation/translation (`E0408`),
//! 0. target: a manifest naming a target or Wasm features beyond the host's
//!    `max_target` is refused first (`E0411`), before the engine would fail on
//!    an unsupported instruction with a less helpful `E0408`,
//! 3. manifest: exactly one well-formed section (`E0402`),
//! 4. ABI/profile compatibility (`E0403`),
//! 5. `needs` against the registered and granted capabilities (`E0400`/`E0401`),
//! 6. imports: functions only, each from a needed interface, each with the
//!    exact signature derived from the IDL (`E0405`/`E0400`/`E0401`/`E0404`),
//! 7. exports: the whole ABI table with exact kinds and signatures,
//!    no stray `oxi_*`/`cb:*` names, callbacks present where needed
//!    (`E0406`/`E0407`).
//!
//! This is the wasm-level equivalent of `oxiscript_ir::host::verify_imports`
//! (contracts.md item 11): at the Wasm level everything is an integer, so the
//! guarantee that a module cannot forge addresses comes from the per-call
//! bounds checks in `host.rs`; what is checked here is that nothing but the
//! IDL-shaped functions can be imported.

use crate::capability::{CapabilityProvider, Grants};
use crate::error::{Error, Result};
use crate::limits::Limits;
use oxiscript_abi::exports::{self, ExportKind};
use oxiscript_abi::idl::{IdlTy, Interface};
use oxiscript_abi::layout::{ABI_MAJOR, ABI_MINOR};
use oxiscript_abi::manifest::{Manifest, MANIFEST_SECTION};
use oxiscript_abi::target::{Feature, Target};
use oxiscript_abi::WasmTy;
use std::collections::HashMap;
use std::sync::Arc;
use wasmparser::WasmFeatures;
use wasmi::{ExternType, FuncType, Module, Mutability, ValType};

/// Result of a successful check: what to instantiate and with what.
pub(crate) struct Plan {
    pub manifest: Manifest,
    /// One provider per `needs` line, in manifest order.
    pub providers: Vec<Arc<dyn CapabilityProvider>>,
}

pub(crate) fn val_ty(t: WasmTy) -> ValType {
    match t {
        WasmTy::I32 => ValType::I32,
        WasmTy::I64 => ValType::I64,
        WasmTy::F32 => ValType::F32,
        WasmTy::F64 => ValType::F64,
        // WasmTy is non-exhaustive; V128 and friends are not part of ABI 1.0.
        _ => ValType::V128,
    }
}

fn tys(v: &[WasmTy]) -> Vec<ValType> {
    v.iter().map(|t| val_ty(*t)).collect()
}

fn sig_text(ft: &FuncType) -> String {
    format!("({:?}) -> ({:?})", ft.params(), ft.results())
}

fn expect_sig(ft: &FuncType, params: &[ValType], results: &[ValType]) -> bool {
    ft.params() == params && ft.results() == results
}

/// Structural module caps (matching `wasmi::EnforcedLimits::strict()` minus the
/// lazy-compilation-only 40-avg-bytes/function heuristic, plus a total declared
/// locals cap to prevent tiny-body eager-compile amplification; see `runtime.rs`).
const MAX_PARAMS: usize = 32;
const MAX_RESULTS: usize = 32;
const MAX_FUNCTIONS: u32 = 10_000;
const MAX_TOTAL_LOCALS: u64 = 1_000_000;
const MAX_GLOBALS: u32 = 1_000;
const MAX_TABLES: u32 = 100;
const MAX_ELEMENT_SEGMENTS: u32 = 1_000;
const MAX_MEMORIES: u32 = 1;
const MAX_DATA_SEGMENTS: u32 = 1_000;

/// Reject any export that is a re-export of an imported function and enforce
/// structural module limits (independent of wasmi's lazy-compilation heuristic).
pub(crate) fn reject_reexports(wasm: &[u8]) -> Result<()> {
    use wasmparser::{CompositeInnerType, ExternalKind, Parser, Payload, TypeRef};
    let bad = |e: wasmparser::BinaryReaderError| Error::reject("OXS-E0408", e.to_string());
    let mut imported_funcs = 0u32;
    let mut defined_funcs = 0u32;
    let mut total_locals = 0u64;
    let mut globals = 0u32;
    let mut tables = 0u32;
    let mut memories = 0u32;
    let mut elem_segs = 0u32;
    let mut data_segs = 0u32;
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.map_err(bad)? {
            Payload::TypeSection(r) => {
                for rg in r {
                    let rg = rg.map_err(bad)?;
                    for st in rg.into_types() {
                        if let CompositeInnerType::Func(ft) = st.composite_type.inner {
                            if ft.params().len() > MAX_PARAMS {
                                return Err(Error::reject(
                                    "OXS-E0408",
                                    format!(
                                        "function type has {} parameters (limit {MAX_PARAMS})",
                                        ft.params().len()
                                    ),
                                ));
                            }
                            if ft.results().len() > MAX_RESULTS {
                                return Err(Error::reject(
                                    "OXS-E0408",
                                    format!(
                                        "function type has {} results (limit {MAX_RESULTS})",
                                        ft.results().len()
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
            Payload::ImportSection(r) => {
                for imp in r {
                    let imp = imp.map_err(bad)?;
                    if matches!(imp.ty, TypeRef::Func(_)) {
                        imported_funcs = imported_funcs.saturating_add(1);
                    } else {
                        // ABI 1.0 imports functions only (no memory, table, global or tag),
                        // refused here independently of what the engine would accept.
                        return Err(Error::reject(
                            "OXS-E0405",
                            format!(
                                "import `{}`.`{}` is not a function",
                                imp.module, imp.name
                            ),
                        ));
                    }
                }
            }
            Payload::FunctionSection(r) => {
                defined_funcs = defined_funcs.saturating_add(r.count());
            }
            Payload::GlobalSection(r) => {
                globals = globals.saturating_add(r.count());
            }
            // table64 is the table twin of memory64 (same ABI 1.0 rule).
            Payload::TableSection(r) => {
                tables = tables.saturating_add(r.count());
                for t in r {
                    let t = t.map_err(bad)?;
                    if t.ty.table64 || t.ty.shared {
                        return Err(Error::reject(
                            "OXS-E0408",
                            "table64 and shared tables are not allowed (ABI 1.0)",
                        ));
                    }
                }
            }
            // ABI 1.0 allows one 32-bit, unshared memory. Enforced here, independently of
            // which wasmi features Cargo feature unification happened to enable.
            Payload::MemorySection(r) => {
                memories = memories.saturating_add(r.count());
                for m in r {
                    let m = m.map_err(bad)?;
                    if m.memory64 || m.shared {
                        return Err(Error::reject(
                            "OXS-E0408",
                            "memory64 and shared memories are not allowed (ABI 1.0)",
                        ));
                    }
                }
            }
            Payload::ElementSection(r) => {
                elem_segs = elem_segs.saturating_add(r.count());
            }
            Payload::DataSection(r) => {
                data_segs = data_segs.saturating_add(r.count());
            }
            Payload::ExportSection(r) => {
                for e in r {
                    let e = e.map_err(bad)?;
                    if e.kind == ExternalKind::Func && e.index < imported_funcs {
                        return Err(Error::reject(
                            "OXS-E0407",
                            format!("export `{}` re-exports an imported host function", e.name),
                        ));
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                let locals = body.get_locals_reader().map_err(bad)?;
                for entry in locals {
                    let (count, _) = entry.map_err(bad)?;
                    total_locals = total_locals.saturating_add(u64::from(count));
                    if total_locals > MAX_TOTAL_LOCALS {
                        return Err(Error::reject(
                            "OXS-E0408",
                            format!(
                                "module declares more than {MAX_TOTAL_LOCALS} total locals across its functions"
                            ),
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    let total_funcs = imported_funcs.saturating_add(defined_funcs);
    if total_funcs > MAX_FUNCTIONS {
        return Err(Error::reject(
            "OXS-E0408",
            format!("module defines {total_funcs} functions (limit {MAX_FUNCTIONS})"),
        ));
    }
    if globals > MAX_GLOBALS {
        return Err(Error::reject(
            "OXS-E0408",
            format!("module defines {globals} globals (limit {MAX_GLOBALS})"),
        ));
    }
    if tables > MAX_TABLES {
        return Err(Error::reject(
            "OXS-E0408",
            format!("module defines {tables} tables (limit {MAX_TABLES})"),
        ));
    }
    if memories > MAX_MEMORIES {
        return Err(Error::reject(
            "OXS-E0408",
            format!("module defines {memories} memories (limit {MAX_MEMORIES})"),
        ));
    }
    if elem_segs > MAX_ELEMENT_SEGMENTS {
        return Err(Error::reject(
            "OXS-E0408",
            format!("module defines {elem_segs} element segments (limit {MAX_ELEMENT_SEGMENTS})"),
        ));
    }
    if data_segs > MAX_DATA_SEGMENTS {
        return Err(Error::reject(
            "OXS-E0408",
            format!("module defines {data_segs} data segments (limit {MAX_DATA_SEGMENTS})"),
        ));
    }
    Ok(())
}

/// Wasm features beyond the Device base set that this build of the engine can
/// execute (docs/oxiscript/abi.md 3.6). A module may declare a feature only if
/// its target allows it *and* the engine has it. SIMD, exception handling,
/// memory64 and multi-memory are deliberately absent: the engine is configured
/// with them off (`Host::with_options`), and ABI 1.0 has one 32-bit memory.
pub(crate) const ENGINE_FEATURES: &[Feature] = &[Feature::TailCall, Feature::ExtendedConst];

fn feature_flags(f: Feature) -> WasmFeatures {
    match f {
        Feature::Simd128 => WasmFeatures::SIMD,
        Feature::Exceptions => WasmFeatures::EXCEPTIONS,
        Feature::TailCall => WasmFeatures::TAIL_CALL,
        Feature::ExtendedConst => WasmFeatures::EXTENDED_CONST,
        Feature::Memory64 => WasmFeatures::MEMORY64,
    }
}

fn device_base() -> WasmFeatures {
    WasmFeatures::WASM1
        | WasmFeatures::MULTI_VALUE
        | WasmFeatures::BULK_MEMORY
        | WasmFeatures::SIGN_EXTENSION
}

/// Target policy, FAIL-CLOSED (abi.md 3.6): the module's bytes must validate against the Device
/// base plus the features its manifest declares AND this host provides; anything else is refused
/// here, never left to "the engine will refuse it". A missing or malformed manifest is judged
/// against the Device base only (`check` then reports it as `E0402`).
pub(crate) fn check_target(wasm: &[u8], max_target: Target, limits: &Limits) -> Result<()> {
    use wasmparser::{Parser, Payload};
    let mut manifest: Option<Manifest> = None;
    for payload in Parser::new(0).parse_all(wasm) {
        let Ok(Payload::CustomSection(c)) = payload else {
            continue;
        };
        if c.name() != MANIFEST_SECTION {
            continue;
        }
        // Size cap before any parsing (a huge section is E0402 in `check` anyway).
        if c.data().len() > limits.max_manifest_bytes {
            return Err(Error::reject(
                "OXS-E0402",
                format!("manifest is larger than {} bytes", limits.max_manifest_bytes),
            ));
        }
        // A manifest we cannot read is `E0402` (same as `check`), not a reason to guess a target.
        let text = std::str::from_utf8(c.data())
            .map_err(|_| Error::reject("OXS-E0402", "manifest is not valid UTF-8"))?;
        manifest = Some(Manifest::parse(text).map_err(|e| Error::reject("OXS-E0402", e.to_string()))?);
        break;
    }
    let mut allowed = device_base();
    let mut declared = Vec::new();
    if let Some(m) = &manifest {
        if m.target > max_target {
            return Err(Error::reject(
                "OXS-E0411",
                format!(
                    "module targets `{}`, the host allows at most `{max_target}`",
                    m.target
                ),
            ));
        }
        for f in &m.features {
            if !max_target.allows(*f) || !ENGINE_FEATURES.contains(f) {
                return Err(Error::reject(
                    "OXS-E0411",
                    format!(
                        "module targets `{}` and needs feature `{f}`, which this host (max target `{max_target}`) does not provide",
                        m.target
                    ),
                ));
            }
            allowed |= feature_flags(*f);
        }
        // exnref needs the reference-types machinery; granted only with a declared `exceptions`
        // (which has already passed `max_target` and the engine check above).
        if m.features.contains(&Feature::Exceptions) {
            allowed |= WasmFeatures::REFERENCE_TYPES;
        }
        declared = m.features.iter().map(|f| f.name()).collect::<Vec<_>>();
    }
    // The claim must be true: the code may use nothing beyond the Device base set plus the
    // *declared* features (a `device` module with `return_call` is refused even by a Web host,
    // whose engine would run it).
    if wasmparser::Validator::new_with_features(allowed)
        .validate_all(wasm)
        .is_ok()
    {
        return Ok(());
    }
    // It needs more than it declares. If this host could run it with its full set the manifest
    // lies (E0411); otherwise it is outside what the host provides at all (E0408), or just
    // malformed (the engine would say the same).
    let mut host_set = device_base();
    for f in ENGINE_FEATURES.iter().filter(|f| max_target.allows(**f)) {
        host_set |= feature_flags(*f);
    }
    if declared_is_lie(wasm, host_set) {
        return Err(Error::reject(
            "OXS-E0411",
            format!(
                "module declares features [{}] but its code uses more (undeclared features are refused)",
                declared.join(" ")
            ),
        ));
    }
    Err(Error::reject(
        "OXS-E0408",
        "module is malformed or uses Wasm features this host does not provide (Device base set + declared target features only)",
    ))
}

fn declared_is_lie(wasm: &[u8], host_set: WasmFeatures) -> bool {
    wasmparser::Validator::new_with_features(host_set)
        .validate_all(wasm)
        .is_ok()
}

pub(crate) fn check(
    module: &Module,
    limits: &Limits,
    providers: &[Arc<dyn CapabilityProvider>],
    grants: &Grants,
) -> Result<Plan> {
    // 3. manifest: exactly one section, bounded, UTF-8.
    let mut sections = module
        .custom_sections()
        .filter(|s| s.name() == MANIFEST_SECTION);
    let Some(section) = sections.next() else {
        return Err(Error::reject("OXS-E0402", "no `oxi.manifest` section"));
    };
    if sections.next().is_some() {
        return Err(Error::reject(
            "OXS-E0402",
            "more than one `oxi.manifest` section",
        ));
    }
    if section.data().len() > limits.max_manifest_bytes {
        return Err(Error::reject(
            "OXS-E0402",
            format!(
                "manifest is larger than {} bytes",
                limits.max_manifest_bytes
            ),
        ));
    }
    let text = std::str::from_utf8(section.data())
        .map_err(|_| Error::reject("OXS-E0402", "manifest is not valid UTF-8"))?;
    let manifest = Manifest::parse(text).map_err(|e| Error::reject("OXS-E0402", e.to_string()))?;

    // 4. ABI and profile.
    if !manifest.compatible_with_host(ABI_MAJOR, ABI_MINOR) {
        return Err(Error::reject(
            "OXS-E0403",
            format!(
                "module ABI {}.{} is not compatible with host ABI {ABI_MAJOR}.{ABI_MINOR}",
                manifest.abi_major, manifest.abi_minor
            ),
        ));
    }
    if manifest.profile == "kernel" {
        return Err(Error::reject(
            "OXS-E0403",
            "profile `kernel` cannot run on the Wasm host",
        ));
    }

    // 5. needs -> providers.
    let mut chosen: Vec<Arc<dyn CapabilityProvider>> = Vec::new();
    for need in &manifest.needs {
        let key = format!("{}@{}", need.module, need.version);
        let Some(p) = providers
            .iter()
            .find(|p| p.interface().import_module() == key)
        else {
            return Err(Error::reject(
                "OXS-E0400",
                format!("module needs `{key}`, which the host does not provide"),
            ));
        };
        if !grants.allows(&key) {
            return Err(Error::reject(
                "OXS-E0400",
                format!("module needs `{key}`, which was not granted to it"),
            ));
        }
        if p.interface().name != need.interface {
            return Err(Error::reject(
                "OXS-E0401",
                format!(
                    "no interface `{}` in `{key}` (the host calls it `{}`)",
                    need.interface,
                    p.interface().name
                ),
            ));
        }
        chosen.push(Arc::clone(p));
    }
    let interfaces: Vec<&Interface> = chosen.iter().map(|p| p.interface()).collect();

    // 6. imports.
    let mut callbacks_needed: Vec<String> = Vec::new();
    for imp in module.imports() {
        let ExternType::Func(ft) = imp.ty() else {
            return Err(Error::reject(
                "OXS-E0405",
                format!(
                    "import `{}`.`{}` is not a function",
                    imp.module(),
                    imp.name()
                ),
            ));
        };
        let Some(itf) = interfaces
            .iter()
            .find(|i| i.import_module() == imp.module())
        else {
            return Err(Error::reject(
                "OXS-E0400",
                format!(
                    "module imports `{}`, which its manifest does not list as needed",
                    imp.module()
                ),
            ));
        };
        let Some(f) = itf.find_fn(imp.name()) else {
            return Err(Error::reject(
                "OXS-E0401",
                format!(
                    "no function `{}` in interface {} ({})",
                    imp.name(),
                    itf.name,
                    imp.module()
                ),
            ));
        };
        let want = itf
            .wasm_imports()
            .into_iter()
            .find(|w| w.name == f.name)
            .expect("wasm_imports covers every fn");
        if !expect_sig(ft, &tys(&want.params), &tys(&want.results)) {
            return Err(Error::reject(
                "OXS-E0404",
                format!(
                    "import `{}`.`{}` has type {}, the interface requires ({:?}) -> ({:?})",
                    imp.module(),
                    imp.name(),
                    sig_text(ft),
                    tys(&want.params),
                    tys(&want.results)
                ),
            ));
        }
        for p in &f.params {
            if let IdlTy::Callback(c) = &p.ty {
                let name = exports::callback_export_name(&itf.module, itf.version, &itf.name, c);
                if !callbacks_needed.contains(&name) {
                    callbacks_needed.push(name);
                }
            }
        }
    }

    // 7. exports.
    let mut by_name: HashMap<String, ExternType> = HashMap::new();
    for e in module.exports() {
        by_name.insert(e.name().to_string(), e.ty().clone());
    }
    for spec in exports::EXPORTS {
        let bad = |why: String| {
            Err(Error::reject(
                "OXS-E0406",
                format!("export `{}`: {why}", spec.name),
            ))
        };
        let Some(ty) = by_name.get(spec.name) else {
            return bad("missing".into());
        };
        match (spec.kind, ty) {
            (ExportKind::Func, ExternType::Func(ft)) => {
                let params: Vec<ValType> = if spec.name == exports::INIT {
                    vec![ValType::I32; manifest.needs.len()]
                } else {
                    tys(spec.params)
                };
                if !expect_sig(ft, &params, &tys(spec.results)) {
                    return bad(format!(
                        "has type {}, expected ({params:?}) -> ({:?})",
                        sig_text(ft),
                        tys(spec.results)
                    ));
                }
            }
            (ExportKind::Global | ExportKind::MutGlobal, ExternType::Global(g)) => {
                let want_mut = if spec.kind == ExportKind::MutGlobal {
                    Mutability::Var
                } else {
                    Mutability::Const
                };
                if g.content() != tys(spec.results)[0] || g.mutability() != want_mut {
                    return bad(format!(
                        "is a {:?} {:?} global, expected {:?} {:?}",
                        g.mutability(),
                        g.content(),
                        want_mut,
                        tys(spec.results)[0]
                    ));
                }
            }
            (ExportKind::Memory, ExternType::Memory(m)) => {
                if m.is_64() {
                    return bad("must be a 32-bit memory".into());
                }
                if m.minimum() > u64::from(limits.max_memory_pages) {
                    return Err(Error::reject(
                        "OXS-E0409",
                        format!(
                            "memory needs {} pages, the host allows {}",
                            m.minimum(),
                            limits.max_memory_pages
                        ),
                    ));
                }
            }
            _ => return bad("has the wrong kind".into()),
        }
    }
    let expected_cbs: Vec<(String, Vec<ValType>, Vec<ValType>)> = interfaces
        .iter()
        .flat_map(|i| i.wasm_callback_exports())
        .map(|c| (c.name, tys(&c.params), tys(&c.results)))
        .collect();
    for (name, ty) in &by_name {
        if exports::find(name.as_str()).is_some() {
            continue;
        }
        if name.starts_with(exports::CALLBACK_PREFIX) {
            let Some((_, p, r)) = expected_cbs.iter().find(|(n, _, _)| n == name) else {
                return Err(Error::reject(
                    "OXS-E0407",
                    format!("export `{name}` is not a callback of a needed interface"),
                ));
            };
            match ty {
                ExternType::Func(ft) if expect_sig(ft, p, r) => {}
                _ => {
                    return Err(Error::reject(
                        "OXS-E0407",
                        format!(
                        "callback export `{name}` has the wrong type, expected ({p:?}) -> ({r:?})"
                    ),
                    ))
                }
            }
        } else if name == exports::COLLECT_STEP || name == exports::COLLECT_STAT {
            // optional runtime exports: (i32) -> i32
            match ty {
                ExternType::Func(ft) if expect_sig(ft, &[ValType::I32], &[ValType::I32]) => {}
                _ => {
                    return Err(Error::reject(
                        "OXS-E0407",
                        format!("optional runtime export `{name}` must have type (i32) -> i32"),
                    ))
                }
            }
        } else if exports::is_reserved_export_name(name.as_str()) {
            return Err(Error::reject(
                "OXS-E0407",
                format!("export `{name}` uses a reserved name that is not part of the ABI table"),
            ));
        } else if !matches!(ty, ExternType::Func(_)) {
            return Err(Error::reject(
                "OXS-E0407",
                format!("export `{name}` is not a function (user exports are functions)"),
            ));
        }
    }
    for cb in callbacks_needed {
        if !by_name.contains_key(cb.as_str()) {
            return Err(Error::reject(
                "OXS-E0406",
                format!("export `{cb}`: missing (an imported function takes this callback)"),
            ));
        }
    }

    Ok(Plan {
        manifest,
        providers: chosen,
    })
}
