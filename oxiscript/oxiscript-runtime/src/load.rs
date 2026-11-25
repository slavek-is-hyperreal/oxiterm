//! Pre-instantiation validation of a module against the ABI.
//!
//! Order (each step has its own reason code, see `registry/codes.txt`):
//! 1. size (`E0409`), 2. engine validation/translation (`E0408`),
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
use oxiscript_abi::WasmTy;
use std::collections::HashMap;
use std::sync::Arc;
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

/// Reject any export that is a re-export of an imported function: user
/// exports, `cb:*` trampolines and ABI rows must be *defined* in the module,
/// otherwise the embedder could be handed a raw host import as an entry point.
pub(crate) fn reject_reexports(wasm: &[u8]) -> Result<()> {
    use wasmparser::{ExternalKind, Parser, Payload, TypeRef};
    let bad = |e: wasmparser::BinaryReaderError| Error::reject("OXS-E0408", e.to_string());
    let mut imported_funcs = 0u32;
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.map_err(bad)? {
            Payload::ImportSection(r) => {
                for imp in r {
                    if matches!(imp.map_err(bad)?.ty, TypeRef::Func(_)) {
                        imported_funcs += 1;
                    }
                }
            }
            // ABI 1.0 allows one 32-bit, unshared memory. Enforced here, independently of
            // which wasmi features Cargo feature unification happened to enable.
            Payload::MemorySection(r) => {
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
            _ => {}
        }
    }
    Ok(())
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
