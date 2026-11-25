//! The import trampoline: the only code that touches guest memory on behalf
//! of a capability. Capabilities never see `Caller`, memory or the instance.
//!
//! Per call: charge fuel, validate the capability handle (a handle that is
//! stale, forged, revoked or belongs to another interface is *denied*), copy
//! and validate the arguments out of guest memory (`ptr < 1024` with
//! `len > 0`, `ptr + len` beyond memory, `len` beyond the payload cap, bad
//! UTF-8 and non-canonical narrow integers are traps), call the capability
//! with host-owned values, then write results: scalars as Wasm results, and
//! strings/bytes through the out-buffer protocol (nothing is written unless
//! the whole value fits, and nothing outside `[out_ptr, out_ptr + len)`).

use crate::capability::{Capability, HostError, Value};
use crate::error::TrapCause;
use crate::handle::HandleTable;
use crate::limits::Limits;
use oxiscript_abi::idl::{
    IdlFn, IdlRet, IdlTy, Interface, STATUS_BUFFER_TOO_SMALL, STATUS_DENIED,
    STATUS_HOST_ERROR_BASE, STATUS_OK,
};
use oxiscript_abi::layout::{Prim, RESERVED_LOW_BYTES};
use oxiscript_abi::WasmTy;
use std::fmt;
use std::sync::Arc;
use wasmi::errors::HostError as WasmiHostError;
use wasmi::{Caller, Error as WasmiError, Memory, StoreLimits, TrapCode, Val};

/// A trap raised by the host side of an import (carried through the engine as
/// a host error and recovered in `Runtime::invoke`).
#[derive(Debug)]
pub(crate) struct HostTrap(pub TrapCause);

impl fmt::Display for HostTrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "host trap: {:?}", self.0)
    }
}

impl WasmiHostError for HostTrap {}

fn trap(cause: TrapCause) -> WasmiError {
    WasmiError::host(HostTrap(cause))
}

fn bad(func: &str, why: impl fmt::Display) -> WasmiError {
    trap(TrapCause::HostBadArgument(format!("{func}: {why}")))
}

pub(crate) struct CapSlot {
    pub interface: Arc<Interface>,
    /// `None` once revoked (the object is dropped, which unregisters it).
    pub imp: Option<Box<dyn Capability>>,
}

/// Per-instance store data.
pub(crate) struct Data {
    pub limits: StoreLimits,
    pub cfg: Arc<Limits>,
    pub caps: Vec<CapSlot>,
    /// Capability handle -> index into `caps`.
    pub handles: HandleTable<usize>,
    pub memory: Option<Memory>,
    pub host_calls: u64,
}

fn charge(caller: &mut Caller<'_, Data>, amount: u64) -> Result<(), WasmiError> {
    let fuel = caller.get_fuel()?;
    if fuel < amount {
        caller.set_fuel(0)?;
        return Err(TrapCode::OutOfFuel.into());
    }
    caller.set_fuel(fuel - amount)?;
    Ok(())
}

fn canonical(p: Prim, v: i32) -> bool {
    match p {
        Prim::Bool => v == 0 || v == 1,
        Prim::I8 => i8::try_from(v).is_ok(),
        Prim::U8 => u8::try_from(v).is_ok(),
        Prim::I16 => i16::try_from(v).is_ok(),
        Prim::U16 => u16::try_from(v).is_ok(),
        _ => true,
    }
}

/// Validate `[ptr, ptr+len)` against the memory size. An empty region is
/// always fine (the pointer is ignored).
fn region(ptr: u32, len: u32, mem_len: usize) -> bool {
    if len == 0 {
        return true;
    }
    if ptr < RESERVED_LOW_BYTES {
        return false;
    }
    match (ptr as usize).checked_add(len as usize) {
        Some(end) => end <= mem_len,
        None => false,
    }
}

fn zero(t: WasmTy) -> Val {
    match t {
        WasmTy::I64 => Val::I64(0),
        WasmTy::F32 => Val::F32(0.0f32.into()),
        WasmTy::F64 => Val::F64(0.0f64.into()),
        _ => Val::I32(0),
    }
}

/// Fill `results` with zeros of the right type, then the status.
fn failed_results(f: &IdlFn, itf: &Interface, status: i32, results: &mut [Val]) {
    let imp = itf
        .wasm_imports()
        .into_iter()
        .find(|w| w.name == f.name)
        .expect("function of the interface");
    for (r, t) in results.iter_mut().zip(&imp.results) {
        *r = zero(*t);
    }
    if let Some(r) = results.first_mut() {
        *r = Val::I32(status);
    }
}

fn value_to_val(v: Value, ty: &IdlTy) -> Option<Val> {
    match (ty, v) {
        (IdlTy::Prim(p), Value::I32(x)) if matches!(p.wasm(), WasmTy::I32) && canonical(*p, x) => {
            Some(Val::I32(x))
        }
        (IdlTy::Prim(p), Value::I64(x)) if matches!(p.wasm(), WasmTy::I64) => {
            let _ = p;
            Some(Val::I64(x))
        }
        (IdlTy::Prim(p), Value::F32(x)) if matches!(p.wasm(), WasmTy::F32) => {
            Some(Val::F32(x.into()))
        }
        (IdlTy::Prim(p), Value::F64(x)) if matches!(p.wasm(), WasmTy::F64) => {
            Some(Val::F64(x.into()))
        }
        (IdlTy::Handle(_), Value::I32(x)) => Some(Val::I32(x)),
        _ => None,
    }
}

/// Build the closure that implements import `fi` of interface `itf`, bound
/// to capability slot `ci`.
pub(crate) fn make_import(
    ci: usize,
    itf: Arc<Interface>,
    fi: usize,
) -> impl Fn(Caller<'_, Data>, &[Val], &mut [Val]) -> Result<(), WasmiError> + Send + Sync + 'static
{
    move |mut caller: Caller<'_, Data>, params: &[Val], results: &mut [Val]| {
        let f = &itf.fns[fi];
        let cfg = Arc::clone(&caller.data().cfg);
        charge(&mut caller, cfg.host_call_fuel)?;
        caller.data_mut().host_calls += 1;
        let has_status = matches!(f.ret, IdlRet::Result(_));

        let deny = |results: &mut [Val]| -> Result<(), WasmiError> {
            if has_status {
                failed_results(f, &itf, STATUS_DENIED, results);
                Ok(())
            } else {
                Err(trap(TrapCause::HostDenied {
                    interface: itf.import_module(),
                    func: f.name.clone(),
                }))
            }
        };

        // 1. capability handle: must be live, ours, and not revoked.
        let Val::I32(h) = params[0] else {
            return Err(bad(&f.name, "capability handle is not an i32"));
        };
        {
            let d = caller.data();
            match d.handles.get(h as u32) {
                Some(&idx) if idx == ci && d.caps[ci].imp.is_some() => {}
                _ => return deny(results),
            }
        }
        let mem = caller
            .data()
            .memory
            .ok_or_else(|| bad(&f.name, "no memory"))?;

        // 2. arguments.
        let mut args: Vec<Value> = Vec::with_capacity(f.params.len());
        let mut cursor = 1usize;
        for p in &f.params {
            match &p.ty {
                IdlTy::Prim(pr) => {
                    match params[cursor] {
                        Val::I32(x) => {
                            if !canonical(*pr, x) {
                                return Err(bad(
                                    &f.name,
                                    format!("argument `{}` is not a canonical {pr:?}: {x}", p.name),
                                ));
                            }
                            args.push(Value::I32(x));
                        }
                        Val::I64(x) => args.push(Value::I64(x)),
                        Val::F32(x) => args.push(Value::F32(x.into())),
                        Val::F64(x) => args.push(Value::F64(x.into())),
                        _ => return Err(bad(&f.name, "unsupported argument type")),
                    }
                    cursor += 1;
                }
                IdlTy::Handle(_) | IdlTy::Callback(_) => {
                    let Val::I32(x) = params[cursor] else {
                        return Err(bad(&f.name, "handle argument is not an i32"));
                    };
                    args.push(Value::I32(x));
                    cursor += 1;
                }
                IdlTy::Str | IdlTy::Bytes => {
                    let (Val::I32(ptr), Val::I32(len)) = (&params[cursor], &params[cursor + 1])
                    else {
                        return Err(bad(&f.name, "(ptr, len) are not i32"));
                    };
                    cursor += 2;
                    let (ptr, len) = (*ptr as u32, *len as u32);
                    if len > cfg.max_host_payload {
                        return Err(bad(
                            &f.name,
                            format!(
                                "`{}` is {len} bytes, the limit is {}",
                                p.name, cfg.max_host_payload
                            ),
                        ));
                    }
                    charge(&mut caller, u64::from(len) / cfg.bytes_per_fuel.max(1))?;
                    let data = mem.data(&caller);
                    if !region(ptr, len, data.len()) {
                        return Err(bad(
                            &f.name,
                            format!("`{}` region ({ptr}, {len}) is outside guest memory", p.name),
                        ));
                    }
                    let bytes = if len == 0 {
                        Vec::new()
                    } else {
                        data[ptr as usize..ptr as usize + len as usize].to_vec()
                    };
                    if matches!(p.ty, IdlTy::Str) {
                        match String::from_utf8(bytes) {
                            Ok(s) => args.push(Value::Str(s)),
                            Err(_) => {
                                return Err(bad(
                                    &f.name,
                                    format!("`{}` is not valid UTF-8", p.name),
                                ))
                            }
                        }
                    } else {
                        args.push(Value::Bytes(bytes));
                    }
                }
            }
        }

        // Out-buffer, validated before the call so a bad one never reaches
        // the capability.
        let out_buf = if matches!(f.ret, IdlRet::Result(Some(IdlTy::Str | IdlTy::Bytes))) {
            let (Val::I32(p), Val::I32(c)) = (&params[cursor], &params[cursor + 1]) else {
                return Err(bad(&f.name, "out-buffer is not (i32, i32)"));
            };
            let (p, c) = (*p as u32, *c as u32);
            if !region(p, c, mem.data(&caller).len()) {
                return Err(bad(
                    &f.name,
                    format!("out-buffer ({p}, {c}) is outside guest memory"),
                ));
            }
            Some((p, c))
        } else {
            None
        };

        // 3. the capability call (no Caller, no memory, no instance).
        let outcome = {
            let slot = &mut caller.data_mut().caps[ci];
            match slot.imp.as_mut() {
                Some(imp) => imp.call(f, &args),
                None => Err(HostError::Denied),
            }
        };

        let extra = caller.data_mut().caps[ci]
            .imp
            .as_mut()
            .map_or(0, |c| c.take_fuel_cost());
        if extra > 0 {
            charge(&mut caller, extra)?;
        }

        // 4. results.
        let value = match outcome {
            Ok(v) => v,
            Err(HostError::Denied) => return deny(results),
            Err(HostError::Status(s)) => {
                if has_status && s >= STATUS_HOST_ERROR_BASE {
                    failed_results(f, &itf, s, results);
                    return Ok(());
                }
                return Err(trap(TrapCause::HostFailure(format!(
                    "{}: status {s} is not allowed here",
                    f.name
                ))));
            }
            Err(HostError::Trap(m)) => {
                return Err(trap(TrapCause::HostFailure(format!("{}: {m}", f.name))))
            }
            Err(HostError::QueueFull(m)) => {
                return Err(trap(TrapCause::QueueFull(format!("{}: {m}", f.name))))
            }
        };

        let wrong = || {
            trap(TrapCause::HostFailure(format!(
                "{}: capability returned a value of the wrong type",
                f.name
            )))
        };
        match &f.ret {
            IdlRet::Void => {
                if value.is_some() {
                    return Err(wrong());
                }
            }
            IdlRet::Value(t) => {
                results[0] = value_to_val(value.ok_or_else(wrong)?, t).ok_or_else(wrong)?;
            }
            IdlRet::Result(None) => {
                if value.is_some() {
                    return Err(wrong());
                }
                results[0] = Val::I32(STATUS_OK);
            }
            IdlRet::Result(Some(IdlTy::Str | IdlTy::Bytes)) => {
                let bytes = match (&f.ret, value.ok_or_else(wrong)?) {
                    (IdlRet::Result(Some(IdlTy::Str)), Value::Str(s)) => s.into_bytes(),
                    (IdlRet::Result(Some(IdlTy::Bytes)), Value::Bytes(b)) => b,
                    _ => return Err(wrong()),
                };
                let (op, oc) = out_buf.expect("out-buffer computed above");
                let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
                if len > cfg.max_host_payload {
                    return Err(trap(TrapCause::HostFailure(format!(
                        "{}: result of {len} bytes exceeds the payload limit",
                        f.name
                    ))));
                }
                if len > oc {
                    results[0] = Val::I32(STATUS_BUFFER_TOO_SMALL);
                    results[1] = Val::I32(len as i32);
                } else {
                    charge(&mut caller, u64::from(len) / cfg.bytes_per_fuel.max(1))?;
                    if len > 0 {
                        let data = mem.data_mut(&mut caller);
                        // `region` was checked against this memory before the
                        // call and memory only grows.
                        data[op as usize..op as usize + len as usize].copy_from_slice(&bytes);
                    }
                    results[0] = Val::I32(STATUS_OK);
                    results[1] = Val::I32(len as i32);
                }
            }
            IdlRet::Result(Some(t)) => {
                results[0] = Val::I32(STATUS_OK);
                results[1] = value_to_val(value.ok_or_else(wrong)?, t).ok_or_else(wrong)?;
            }
        }
        Ok(())
    }
}
