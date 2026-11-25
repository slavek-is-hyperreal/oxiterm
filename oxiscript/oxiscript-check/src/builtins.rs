//! Members of builtin types (arrays, strings, numbers, unions): the small
//! standard library the checker knows by name. They are lang items: 8.L lowers
//! each `Builtin` directly.

use crate::calls::{Callee, MemberOut};
use crate::ty::*;
use crate::*;

impl<'a> Checker<'a> {
    fn bp(&mut self, i: usize) -> Ty {
        let p = self.bparams[i];
        self.tys.mk(TyKind::Param(p))
    }

    fn bcallee(&mut self, b: Builtin, tparams: Vec<ParamId>, params: Vec<Ty>, ret: Ty) -> MemberOut {
        let n = params.len();
        MemberOut::Callable(
            Callee { tparams, params, defaults: vec![false; n], ret, def: None, owner_args: vec![], mutating: false, builtin: Some(b) },
            Res::Builtin(b),
        )
    }

    pub(crate) fn builtin_member(&mut self, base: Ty, name: &str) -> Option<MemberOut> {
        let kind = self.tys.kind(base).clone();
        match kind {
            TyKind::Array(t) => {
                let (u, v) = (self.bparams[0], self.bparams[1]);
                let (ut, _vt) = (self.bp(0), self.bp(1));
                let _ = v;
                let a = self.tys.array(t);
                Some(match name {
                    "length" => MemberOut::Value(I32, Res::Builtin(Builtin::ArrayLength)),
                    "push" => self.bcallee(Builtin::ArrayPush, vec![], vec![t], VOID),
                    "pop" => {
                        let r = self.tys.nullable(t);
                        self.bcallee(Builtin::ArrayPop, vec![], vec![], r)
                    }
                    "map" => {
                        let f = self.tys.func(&[t], ut);
                        let r = self.tys.array(ut);
                        self.bcallee(Builtin::ArrayMap, vec![u], vec![f], r)
                    }
                    "filter" => {
                        let f = self.tys.func(&[t], BOOL);
                        self.bcallee(Builtin::ArrayFilter, vec![], vec![f], a)
                    }
                    "reduce" => {
                        let f = self.tys.func(&[ut, t], ut);
                        self.bcallee(Builtin::ArrayReduce, vec![u], vec![f, ut], ut)
                    }
                    "forEach" => {
                        let f = self.tys.func(&[t], VOID);
                        self.bcallee(Builtin::ArrayForEach, vec![], vec![f], VOID)
                    }
                    "slice" => self.bcallee(Builtin::ArraySlice, vec![], vec![I32, I32], a),
                    "some" => {
                        let f = self.tys.func(&[t], BOOL);
                        self.bcallee(Builtin::ArraySome, vec![], vec![f], BOOL)
                    }
                    "every" => {
                        let f = self.tys.func(&[t], BOOL);
                        self.bcallee(Builtin::ArrayEvery, vec![], vec![f], BOOL)
                    }
                    "concat" => self.bcallee(Builtin::ArrayConcat, vec![], vec![a], a),
                    _ => return None,
                })
            }
            TyKind::Str => Some(match name {
                "byteLength" => MemberOut::Value(I32, Res::Builtin(Builtin::StrByteLength)),
                "bytes" => {
                    let r = self.tys.array(U8);
                    self.bcallee(Builtin::StrBytes, vec![], vec![], r)
                }
                "chars" => {
                    let r = self.tys.array(num(NumTy::U32));
                    self.bcallee(Builtin::StrChars, vec![], vec![], r)
                }
                "slice" => self.bcallee(Builtin::StrSlice, vec![], vec![I32, I32], STR),
                "startsWith" => self.bcallee(Builtin::StrStartsWith, vec![], vec![STR], BOOL),
                "endsWith" => self.bcallee(Builtin::StrEndsWith, vec![], vec![STR], BOOL),
                "includes" => self.bcallee(Builtin::StrIncludes, vec![], vec![STR], BOOL),
                "indexOf" => self.bcallee(Builtin::StrIndexOf, vec![], vec![STR], I32),
                _ => return None,
            }),
            TyKind::Num(_) => {
                let u = self.bparams[0];
                let ut = self.bp(0);
                Some(match name {
                    "toString" => self.bcallee(Builtin::NumToString, vec![], vec![], STR),
                    "wrapping" => self.bcallee(Builtin::NumWrapping, vec![u], vec![], ut),
                    "saturating" => self.bcallee(Builtin::NumSaturating, vec![u], vec![], ut),
                    _ => return None,
                })
            }
            TyKind::Bool => match name {
                "toString" => Some(self.bcallee(Builtin::BoolToString, vec![], vec![], STR)),
                _ => None,
            },
            _ => {
                if name == "downcast" && self.union_view(base).is_some() {
                    let u = self.bparams[0];
                    let ut = self.bp(0);
                    let r = self.tys.nullable(ut);
                    return Some(self.bcallee(Builtin::Downcast, vec![u], vec![], r));
                }
                None
            }
        }
    }

    /// Extra validation of the type arguments of `wrapping`/`saturating`/`downcast` once solved.
    pub(crate) fn check_builtin_targs(&mut self, b: Builtin, recv: Ty, targ: Ty, span: Span) {
        match b {
            Builtin::NumWrapping | Builtin::NumSaturating => {
                let ok = matches!((self.tys.kind(recv), self.tys.kind(targ)), (TyKind::Num(_), TyKind::Num(n)) if is_int(*n) || b == Builtin::NumSaturating);
                if !ok && targ != ERROR {
                    let (a, b2) = (self.show(recv), self.show(targ));
                    self.err("OXS-E0111", span, &[("from", &a), ("to", &b2)]);
                }
            }
            Builtin::Downcast => {
                let (Some(from), Some(to)) = (self.union_view(recv), self.union_view(targ)) else {
                    if targ != ERROR {
                        let (a, b2) = (self.show(recv), self.show(targ));
                        self.err("OXS-E0111", span, &[("from", &a), ("to", &b2)]);
                    }
                    return;
                };
                if from.0 != to.0 || from.1 != to.1 {
                    let (a, b2) = (self.show(recv), self.show(targ));
                    self.err("OXS-E0111", span, &[("from", &a), ("to", &b2)]);
                }
            }
            _ => {}
        }
    }
}
