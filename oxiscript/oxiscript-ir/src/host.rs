//! The typed host boundary: the IR signature every host import must have for
//! a given IDL function, and the check that enforces it.
//!
//! Plain [`crate::verify`] only checks that import parameter *kinds* are legal
//! (scalars, `str`, `[u8]`, consumed `fn`); it cannot know which interface an
//! import belongs to. Without the IDL an untrusted module could declare the
//! *flattened* shape with raw integers (`(i32, u8, i32, i32)` for `log.write`)
//! and pass forged addresses or closure pointers. Therefore **the runtime
//! (8.R) and the code generator (8.G) must call [`verify_imports`] (or
//! [`verify_with_interfaces`]) before linking**: it requires every import to
//! name a granted interface function and to have exactly the typed signature
//! derived here.

use crate::types::{Module, Sig, Ty};
use crate::verify::{verify_with, VerifyError, VerifyOptions};
use oxiscript_abi::idl::{IdlFn, IdlRet, IdlTy, Interface};
use oxiscript_abi::layout::Prim;
use oxiscript_abi::WasmTy;

fn prim_ty(p: Prim) -> Ty {
    match p {
        Prim::Bool => Ty::Bool,
        Prim::I8 => Ty::I8,
        Prim::U8 => Ty::U8,
        Prim::I16 => Ty::I16,
        Prim::U16 => Ty::U16,
        Prim::I32 => Ty::I32,
        Prim::U32 => Ty::U32,
        Prim::I64 => Ty::I64,
        Prim::U64 => Ty::U64,
        Prim::F32 => Ty::F32,
        Prim::F64 => Ty::F64,
        Prim::Ptr => Ty::Ptr,
    }
}

/// IR type of an IDL parameter (callbacks become closure types).
fn idl_ty(t: &IdlTy, itf: &Interface) -> Ty {
    match t {
        IdlTy::Prim(p) => prim_ty(*p),
        IdlTy::Str => Ty::Str,
        IdlTy::Bytes => Ty::Array(Box::new(Ty::U8)),
        IdlTy::Handle(_) => Ty::I32,
        IdlTy::Callback(name) => Ty::Fn(Box::new(callback_sig(itf, name))),
    }
}

/// The closure type a callback parameter has in the IR: the callback's own
/// parameters (the closure receives them from the `cb:` trampoline) and result.
pub fn callback_sig(itf: &Interface, name: &str) -> Sig {
    let Some(c) = itf.callbacks.iter().find(|c| c.name == name) else {
        return Sig {
            params: vec![],
            rets: vec![],
        };
    };
    Sig {
        params: c.params.iter().map(|p| idl_ty(&p.ty, itf)).collect(),
        rets: c.ret.iter().map(|p| prim_ty(*p)).collect(),
    }
}

/// The typed IR signature of an interface function: the capability handle
/// first (`i32`), then the parameters, then (for `result<str|bytes>`) the
/// out-buffer as `[u8]`; results are `(i32 status[, value])` for `result<..>`.
pub fn import_sig(f: &IdlFn, itf: &Interface) -> Sig {
    let mut params = vec![Ty::I32];
    params.extend(f.params.iter().map(|p| idl_ty(&p.ty, itf)));
    let rets = match &f.ret {
        IdlRet::Void => vec![],
        IdlRet::Value(t) => vec![idl_ty(t, itf)],
        IdlRet::Result(None) => vec![Ty::I32],
        IdlRet::Result(Some(IdlTy::Str | IdlTy::Bytes)) => {
            params.push(Ty::Array(Box::new(Ty::U8)));
            vec![Ty::I32, Ty::I32]
        }
        IdlRet::Result(Some(t)) => vec![Ty::I32, idl_ty(t, itf)],
    };
    Sig { params, rets }
}

/// Wasm flattening of an IR boundary type: `str` and `[u8]` become
/// `(ptr, len)`, everything else one value. Codegen (8.G) uses the same rule;
/// the tests pin it against `Interface::wasm_imports()`.
pub fn flatten(t: &Ty, out: &mut Vec<WasmTy>) {
    match t {
        Ty::Str | Ty::Array(_) => {
            out.push(WasmTy::I32);
            out.push(WasmTy::I32);
        }
        Ty::I64 | Ty::U64 => out.push(WasmTy::I64),
        Ty::F32 => out.push(WasmTy::F32),
        Ty::F64 => out.push(WasmTy::F64),
        _ => out.push(WasmTy::I32),
    }
}

/// Check every import of `m` against the granted interfaces.
pub fn verify_imports(m: &Module, interfaces: &[Interface]) -> Vec<VerifyError> {
    let mut errs = Vec::new();
    let mut e = |msg: String| errs.push(VerifyError { func: None, msg });
    for imp in &m.imports {
        let Some(itf) = interfaces.iter().find(|i| i.import_module() == imp.module) else {
            e(format!(
                "OXS-E0400: import @{} needs `{}`, which the host did not grant",
                imp.name, imp.module
            ));
            continue;
        };
        let Some(f) = itf.find_fn(&imp.field) else {
            e(format!(
                "OXS-E0401: no function `{}` in interface {} ({})",
                imp.field, itf.name, imp.module
            ));
            continue;
        };
        let want = import_sig(f, itf);
        if imp.sig != want {
            e(format!(
                "import @{}: signature {} does not match the interface (expected {}); raw addresses and closure pointers are never part of an import",
                imp.name,
                crate::print::sig_text(&imp.sig),
                crate::print::sig_text(&want)
            ));
        }
    }
    errs
}

/// [`verify_with`] plus [`verify_imports`]: the check the runtime and code
/// generator run before linking a module.
pub fn verify_with_interfaces(
    m: &Module,
    opts: VerifyOptions,
    interfaces: &[Interface],
) -> Vec<VerifyError> {
    let mut errs = verify_with(m, opts);
    errs.extend(verify_imports(m, interfaces));
    errs
}
