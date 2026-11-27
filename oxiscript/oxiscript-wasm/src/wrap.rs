//! Everything around the user functions: runtime stub bodies, export
//! wrappers (host-facing flattening and canonicalisation), `cb:` trampolines,
//! `oxi_init`, `oxi_deinit`, `oxi_tick`.

use crate::body::{Body, Mem};
use crate::cx::{flat_ty, wasm_ty, Cx};
use crate::func::{gen_func, push_canon_float_body, vt};
use crate::modlayout::ty_key;
use crate::stubrt::{self, G_TRAP};
use crate::{Diagnostic, RuntimeMode, E_CONFIG, E_UNSUPPORTED};
use oxiscript_abi::exports::{self, HOST_PAYLOAD_MAX};
use oxiscript_abi::layout;
use oxiscript_abi::trap::TrapKind;
use oxiscript_ir::{Func, Ty};
use wasm_encoder::{BlockType, ExportKind, ValType};

fn canon_int(b: &mut Body, t: &Ty) {
    match t {
        Ty::Bool => {
            b.s().i32_eqz().i32_eqz();
        }
        Ty::I8 => {
            b.s().i32_extend8_s();
        }
        Ty::I16 => {
            b.s().i32_extend16_s();
        }
        Ty::U8 => {
            b.i32c(0xff);
            b.s().i32_and();
        }
        Ty::U16 => {
            b.i32c(0xffff);
            b.s().i32_and();
        }
        _ => {}
    }
}

/// Call finished: results are on the stack. Release `built`, then re-push the
/// results with floats canonicalised (ir.md section 6: no NaN payload reaches the host).
fn finish_results(cx: &Cx<'_>, b: &mut Body, rets: &[Ty], built: &[u32]) {
    let any_float = rets.iter().any(Ty::is_float);
    let locals: Vec<u32> = if any_float {
        let ls: Vec<u32> = rets.iter().map(|t| b.local(vt(t))).collect();
        for l in ls.iter().rev() {
            b.set(*l);
        }
        ls
    } else {
        Vec::new()
    };
    for l in built {
        b.get(*l);
        b.s().call(cx.rt("oxi_release"));
    }
    for (l, t) in locals.iter().zip(rets) {
        if t.is_float() {
            push_canon_float_body(b, *l, *t == Ty::F64);
        } else {
            b.get(*l);
        }
    }
}

fn needs_wrapper(f: &Func) -> bool {
    f.rets.iter().any(Ty::is_float)
        || f.params.iter().any(|p| {
        matches!(
            f.ty(p.value),
            Ty::Str | Ty::Bool | Ty::I8 | Ty::I16 | Ty::U8 | Ty::U16
        ) || matches!(f.ty(p.value), Ty::Array(e) if **e == Ty::U8)
    })
}

/// Push IR-typed arguments built from host-delivered Wasm parameters
/// starting at wasm local `first`. Returns the locals of built objects the
/// caller must release after the call.
fn push_host_params(
    cx: &Cx<'_>,
    b: &mut Body,
    params: &[(Ty, bool)],
    first: u32,
) -> Result<Vec<u32>, Diagnostic> {
    let mut w = first;
    let mut built = Vec::new();
    for (t, own) in params {
        match t {
            Ty::Str | Ty::Array(_) if matches!(t, Ty::Str) || matches!(t, Ty::Array(e) if **e == Ty::U8) => {
                let (ptr, len) = (w, w + 1);
                w += 2;
                b.get(len).i32c(HOST_PAYLOAD_MAX as i32);
                b.s().i32_gt_u().if_(BlockType::Empty);
                b.i32c(TrapKind::Bounds.code() as i32);
                b.s().global_set(G_TRAP).unreachable().end();
                let l = b.local(ValType::I32);
                if matches!(t, Ty::Str) {
                    b.get(ptr).get(len);
                    b.s().call(cx.rt("oxi_str_new"));
                    b.set(l);
                } else {
                    let aid = *cx.lay.array_ids.get(&ty_key(&Ty::U8)).ok_or_else(|| {
                        Diagnostic::new(E_CONFIG, "[u8] array type is not registered", None)
                    })?;
                    b.i32c(aid as i32).get(len);
                    b.s().call(cx.rt("oxi_array_new"));
                    b.set(l);
                    b.get(l).load(Mem::I32, layout::ARR_OFF_DATA);
                    b.get(ptr).get(len);
                    b.s().memory_copy(0, 0);
                }
                b.get(l);
                if !*own {
                    built.push(l);
                }
            }
            _ => {
                b.get(w);
                canon_int(b, t);
                w += 1;
            }
        }
    }
    Ok(built)
}

/// ABI 1.0 export boundary: scalars and `str`/`[u8]` parameters only; results
/// are scalars. Anything else would hand the host a raw guest pointer.
fn check_export_boundary(f: &Func) -> Result<(), Diagnostic> {
    let bad = |what: String| {
        Diagnostic::new(
            E_UNSUPPORTED,
            format!(
                "exported function with {what}; ABI 1.0 exports take scalars and `str`/`[u8]` parameters and return scalars"
            ),
            Some(f.name.clone()),
        )
    };
    for (i, p) in f.params.iter().enumerate() {
        let t = f.ty(p.value);
        let ok = t.is_numeric()
            || matches!(t, Ty::Bool | Ty::Str)
            || matches!(t, Ty::Array(e) if **e == Ty::U8);
        if !ok {
            return Err(bad(format!("a parameter {} of type `{}`", i + 1, oxiscript_ir::print::ty_text(t))));
        }
    }
    for (i, t) in f.rets.iter().enumerate() {
        if !(t.is_numeric() || matches!(t, Ty::Bool)) {
            return Err(bad(format!("a result {} of type `{}`", i + 1, oxiscript_ir::print::ty_text(t))));
        }
    }
    Ok(())
}

pub(crate) fn gen_all(cx: &mut Cx<'_>) -> Result<(), Diagnostic> {
    let m = cx.m;
    if !cx.opts.allow_raw_exports {
        for f in m.funcs.iter().filter(|f| f.export) {
            check_export_boundary(f)?;
        }
    }
    // 1. stub runtime bodies
    if cx.opts.runtime == RuntimeMode::Stub {
        let map = cx.rt_idx.clone();
        for n in stubrt::FUNCS {
            let f = stubrt::build(n, &|k: &str| map[k]);
            cx.set_body(map[*n], f);
        }
    }
    // 2. user functions
    for f in &m.funcs {
        let body = gen_func(cx, f)?;
        let idx = cx.fn_idx[&f.name];
        cx.set_body(idx, body);
    }
    // 3. lifecycle exports
    let n = cx.needs.len();
    let init = cx.declare(&vec![ValType::I32; n], &[]);
    {
        let mut b = Body::new(n);
        if matches!(cx.opts.runtime, RuntimeMode::Import | RuntimeMode::Linked) {
            b.s().global_get(2).global_get(3);
            b.i32c(cx.lay.static_end as i32);
            b.s().call(cx.rt(exports::RT_INIT));
        }
        if let Some(main) = m.func("main") {
            for i in 0..n {
                b.get(i as u32);
            }
            let _ = main;
            b.s().call(cx.fn_idx["main"]);
        }
        cx.set_body(init, b.finish());
    }
    cx.exports.push((exports::INIT.into(), ExportKind::Func, init));
    if cx.opts.runtime == RuntimeMode::Linked {
        let set_trap = cx.declare(&[ValType::I32], &[]);
        let mut b = Body::new(1);
        b.get(0);
        b.s().global_set(G_TRAP);
        cx.set_body(set_trap, b.finish());
        cx.exports
            .push((crate::link::SET_TRAP_EXPORT.into(), ExportKind::Func, set_trap));
    }
    let deinit = cx.declare(&[], &[]);
    {
        let mut b = Body::new(0);
        for g in &m.globals {
            if g.ty.is_managed() {
                let gi = cx.global_idx[&g.name];
                b.s().global_get(gi);
                b.s().call(cx.rt("oxi_release"));
                b.i32c(0);
                b.s().global_set(gi);
            }
        }
        cx.set_body(deinit, b.finish());
    }
    cx.exports
        .push((exports::DEINIT.into(), ExportKind::Func, deinit));
    let tick = cx.declare(&[ValType::I64], &[ValType::I64]);
    {
        let mut b = Body::new(1);
        let target = m.funcs.iter().find(|f| f.has_attr("tick"));
        if let Some(t) = target {
            let sig = t.sig();
            if sig.params != [Ty::I64] || sig.rets != [Ty::I64] {
                return Err(Diagnostic::new(
                    E_CONFIG,
                    "a `#tick` function must have the signature (i64) -> i64",
                    Some(t.name.clone()),
                ));
            }
            b.get(0);
            b.s().call(cx.fn_idx[&t.name]);
        } else {
            b.i64c(-1);
        }
        cx.set_body(tick, b.finish());
    }
    cx.exports.push((exports::TICK.into(), ExportKind::Func, tick));
    if cx.opts.runtime == RuntimeMode::Stub {
        for n in stubrt::FUNCS {
            if !stubrt::is_internal(n) {
                cx.exports.push(((*n).into(), ExportKind::Func, cx.rt_idx[*n]));
            }
        }
    }
    // 4. user exports
    for f in &m.funcs {
        if !f.export {
            continue;
        }
        let name = f
            .attrs
            .iter()
            .find(|a| a.name == "export_name")
            .and_then(|a| a.args.first().cloned())
            .unwrap_or_else(|| f.name.clone());
        let inner = cx.fn_idx[&f.name];
        let idx = if needs_wrapper(f) {
            let mut flat = Vec::new();
            let params: Vec<(Ty, bool)> = f
                .params
                .iter()
                .map(|p| (f.ty(p.value).clone(), p.own))
                .collect();
            params.iter().for_each(|(t, _)| flat_ty(t, &mut flat));
            let rets: Vec<ValType> = f.rets.iter().map(vt).collect();
            let w = cx.declare(&flat, &rets);
            let mut b = Body::new(flat.len());
            let built = push_host_params(cx, &mut b, &params, 0)?;
            b.s().call(inner);
            finish_results(cx, &mut b, &f.rets, &built);
            cx.set_body(w, b.finish());
            w
        } else {
            inner
        };
        cx.exports.push((name, ExportKind::Func, idx));
    }
    // 5. callback trampolines
    let needs = cx.needs.clone();
    for need in &needs {
        let key = format!("{}@{}", need.module, need.version);
        let Some(itf) = cx.interface_for(&key) else {
            return Err(Diagnostic::new(
                E_CONFIG,
                format!("needs `{key}`, which is not among the granted interfaces"),
                None,
            ));
        };
        let cbs = itf.wasm_callback_exports();
        for (c, wc) in itf.callbacks.iter().zip(cbs) {
            let used = m.imports.iter().any(|imp| {
                imp.module == key
                    && itf.find_fn(&imp.field).is_some_and(|f| {
                        f.params.iter().any(|p| {
                            matches!(&p.ty, oxiscript_abi::idl::IdlTy::Callback(n) if *n == c.name)
                        })
                    })
            });
            if !used {
                continue;
            }
            let sig = oxiscript_ir::host::callback_sig(itf, &c.name);
            let wp: Vec<ValType> = wc.params.iter().map(|t| wasm_ty(*t)).collect();
            let wr: Vec<ValType> = wc.results.iter().map(|t| wasm_ty(*t)).collect();
            let idx = cx.declare(&wp, &wr);
            let mut b = Body::new(wp.len());
            b.get(0);
            let params: Vec<(Ty, bool)> = sig.params.iter().map(|t| (t.clone(), false)).collect();
            let built = push_host_params(cx, &mut b, &params, 1)?;
            let mut cp = vec![ValType::I32];
            cp.extend(sig.params.iter().map(vt));
            let cr: Vec<ValType> = sig.rets.iter().map(vt).collect();
            let ti = cx.sig_type(&cp, &cr);
            b.get(0).load(Mem::I32, layout::CLOSURE_OFF_FN);
            b.s().call_indirect(0, ti);
            finish_results(cx, &mut b, &sig.rets, &built);
            cx.set_body(idx, b.finish());
            cx.exports.push((wc.name.clone(), ExportKind::Func, idx));
        }
    }
    let _ = flat_ty;
    Ok(())
}
