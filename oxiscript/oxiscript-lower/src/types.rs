//! Language type -> IR type, and the generated type declarations (structs and classes,
//! unions, nullable-scalar boxes, closure cells).

use crate::ctx::{sanitize, Lw};
use oxiscript_check::{DefId, DefKind, Discr, ParamId, Ty, TyKind};
use oxiscript_ir::{self as ir, Field, Ty as It, TypeDecl, TypeKind};
use oxiscript_syntax::NumTy;

pub(crate) fn num_ir(n: NumTy) -> It {
    match n {
        NumTy::I8 => It::I8,
        NumTy::I16 => It::I16,
        NumTy::I32 => It::I32,
        NumTy::I64 => It::I64,
        NumTy::U8 => It::U8,
        NumTy::U16 => It::U16,
        NumTy::U32 => It::U32,
        NumTy::U64 => It::U64,
        NumTy::F32 => It::F32,
        NumTy::F64 => It::F64,
    }
}

/// Identifier-safe spelling of an IR type (for generated type names).
pub(crate) fn mangle(t: &It) -> String {
    match t {
        It::Bool => "bool".into(),
        It::I8 => "i8".into(),
        It::I16 => "i16".into(),
        It::I32 => "i32".into(),
        It::I64 => "i64".into(),
        It::U8 => "u8".into(),
        It::U16 => "u16".into(),
        It::U32 => "u32".into(),
        It::U64 => "u64".into(),
        It::F32 => "f32".into(),
        It::F64 => "f64".into(),
        It::Ptr => "ptr".into(),
        It::Str => "str".into(),
        It::Ref(n) => sanitize(n),
        It::Array(e) => format!("a_{}", mangle(e)),
        It::Fn(s) => {
            let ps: Vec<String> = s.params.iter().map(mangle).collect();
            let rs: Vec<String> = s.rets.iter().map(mangle).collect();
            format!("fn_{}_r_{}", ps.join("_"), rs.join("_"))
        }
        It::Opt(i) => format!("o_{}", mangle(i)),
        _ => "x".into(),
    }
}

impl<'c, 'a> Lw<'c, 'a> {
    /// IR type of a (concrete) language type; `None` for `void`/`never`.
    pub fn repr(&mut self, t: Ty) -> Option<It> {
        if let Some(r) = self.repr_cache.get(&t) {
            return r.clone();
        }
        let r = self.repr_uncached(t);
        self.repr_cache.insert(t, r.clone());
        r
    }

    fn repr_uncached(&mut self, t: Ty) -> Option<It> {
        match self.ck.tys.kind(t).clone() {
            TyKind::Error | TyKind::Never | TyKind::Void => None,
            TyKind::Null => {
                self.unsupported("a bare `null` value without a nullable context", None);
                Some(It::I32)
            }
            TyKind::Bool => Some(It::Bool),
            TyKind::Num(n) => Some(num_ir(n)),
            TyKind::Str => Some(It::Str),
            TyKind::Array(e) => {
                let er = self.repr(e).unwrap_or(It::I32);
                Some(It::Array(Box::new(er)))
            }
            TyKind::Nullable(inner) => {
                let ir = self.repr(inner).unwrap_or(It::I32);
                Some(self.nullable_of(ir))
            }
            TyKind::Adt(d, args) => self.adt_repr(d, &args),
            TyKind::Variants(u, args, _) => {
                let n = self.union_name(u, &args);
                Some(It::Ref(n))
            }
            TyKind::Param(_) => {
                self.unsupported("an unresolved type parameter reached the lowering", None);
                Some(It::I32)
            }
            TyKind::Fn(ps, r) => {
                let mut params = Vec::new();
                for p in ps.iter() {
                    if let Some(x) = self.repr(*p) {
                        params.push(x);
                    }
                }
                let rets = self.repr(r).into_iter().collect();
                Some(It::Fn(Box::new(ir::Sig { params, rets })))
            }
        }
    }

    /// `T?` for an IR type: references are nullable in place, scalars are boxed.
    pub fn nullable_of(&mut self, ir: It) -> It {
        match ir {
            It::Opt(_) => ir,
            It::Str | It::Ref(_) | It::Array(_) | It::Fn(_) => It::Opt(Box::new(ir)),
            scalar => {
                let nb = self.box_type(&scalar);
                It::Opt(Box::new(It::Ref(nb)))
            }
        }
    }

    fn adt_repr(&mut self, d: DefId, args: &[Ty]) -> Option<It> {
        match self.kind_of(d) {
            DefKind::Struct | DefKind::Class => Some(It::Ref(self.struct_name(d, args))),
            DefKind::Union => Some(It::Ref(self.union_name(d, args))),
            DefKind::Variant { union: Some(u), index } => {
                let uargs = self.variant_union_args(u, index, args);
                Some(It::Ref(self.union_name(u, &uargs)))
            }
            DefKind::Enum => {
                let repr = self.ck.enums.get(&d).map(|e| e.repr);
                Some(num_ir(repr.unwrap_or(NumTy::I32)))
            }
            DefKind::Host | DefKind::Handle => Some(It::I32),
            _ => {
                let n = self.def_name(d);
                self.unsupported(&format!("a value of type `{n}`"), None);
                Some(It::I32)
            }
        }
    }

    /// Union type arguments for a value typed as one of its variants.
    pub fn variant_union_args(&mut self, u: DefId, index: u32, vargs: &[Ty]) -> Vec<Ty> {
        let tps = self.ck.defs[u.0 as usize].tparams.clone();
        if tps.is_empty() {
            return Vec::new();
        }
        let Some(sig) = self.ck.unions.get(&u) else { return vargs.to_vec() };
        let pats = sig.variants[index as usize].args.clone();
        let identity = pats.len() == tps.len()
            && pats
                .iter()
                .zip(&tps)
                .all(|(p, q)| matches!(self.ck.tys.kind(*p), TyKind::Param(x) if x == q));
        if identity {
            return vargs.to_vec();
        }
        let mut binds: Vec<(ParamId, Ty)> = Vec::new();
        for (p, a) in pats.iter().zip(vargs) {
            self.bind_pattern(*p, *a, &mut binds);
        }
        tps.iter()
            .map(|q| binds.iter().find(|(x, _)| x == q).map(|(_, t)| *t).unwrap_or(crate::types::err_ty()))
            .collect()
    }

    fn bind_pattern(&mut self, pat: Ty, conc: Ty, binds: &mut Vec<(ParamId, Ty)>) {
        match (self.ck.tys.kind(pat).clone(), self.ck.tys.kind(conc).clone()) {
            (TyKind::Param(p), _) => {
                if !binds.iter().any(|(q, _)| *q == p) {
                    binds.push((p, conc));
                }
            }
            (TyKind::Array(a), TyKind::Array(b)) | (TyKind::Nullable(a), TyKind::Nullable(b)) => {
                self.bind_pattern(a, b, binds);
            }
            (TyKind::Adt(_, a), TyKind::Adt(_, b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    self.bind_pattern(*x, *y, binds);
                }
            }
            (TyKind::Fn(pa, ra), TyKind::Fn(pb, rb)) => {
                for (x, y) in pa.iter().zip(pb.iter()) {
                    self.bind_pattern(*x, *y, binds);
                }
                self.bind_pattern(ra, rb, binds);
            }
            _ => {}
        }
    }

    fn new_type(&mut self, base: &str, kind: TypeKind) -> (String, usize) {
        let id = self.next_id();
        let name = format!("{}_{id}", sanitize(base));
        let idx = self.push_type(TypeDecl {
            name: name.clone(),
            attrs: Vec::new(),
            kind,
        });
        (name, idx)
    }

    pub fn struct_name(&mut self, d: DefId, args: &[Ty]) -> String {
        let key = (d, args.to_vec());
        if let Some(n) = self.struct_names.get(&key) {
            return n.clone();
        }
        if !args.is_empty() {
            self.count_instance("struct");
        }
        let dn = self.def_name(d);
        let (name, idx) = self.new_type(&dn, TypeKind::Struct { fields: Vec::new() });
        self.struct_names.insert(key, name.clone());
        let map = self.tparam_map(d, args);
        let rec = self.ck.records.get(&d).cloned();
        let mut fields = Vec::new();
        if let Some(rec) = rec {
            for f in rec.fields.iter().filter(|f| !f.is_static) {
                let t = self.subst(f.ty, &map);
                if let Some(ft) = self.repr(t) {
                    fields.push(Field {
                        name: f.name.clone(),
                        ty: ft,
                    });
                }
            }
        }
        self.out.types[idx].kind = TypeKind::Struct { fields };
        name
    }

    /// IR name of a union variant: derived from its discriminant value when that is an identifier.
    fn variant_ir_names(&self, u: DefId) -> Vec<String> {
        let Some(sig) = self.ck.unions.get(&u) else { return Vec::new() };
        let mut names: Vec<String> = Vec::new();
        for (i, v) in sig.variants.iter().enumerate() {
            let base = match &v.value {
                Discr::Str(s) => sanitize(s),
                Discr::Int(n) => format!("n{}", n.to_string().replace('-', "m")),
                Discr::Bool(b) => format!("b{b}"),
            };
            let base = if base.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                format!("v{base}")
            } else {
                base
            };
            let mut n = base.clone();
            if names.contains(&n) {
                n = format!("{base}_{i}");
            }
            names.push(n);
        }
        names
    }

    pub fn union_name(&mut self, u: DefId, args: &[Ty]) -> String {
        let key = (u, args.to_vec());
        if let Some(n) = self.union_names.get(&key) {
            return n.clone();
        }
        if !args.is_empty() {
            self.count_instance("union");
        }
        let dn = self.def_name(u);
        let (name, idx) = self.new_type(&dn, TypeKind::Union { variants: Vec::new() });
        self.union_names.insert(key, name.clone());
        let vnames = self.variant_ir_names(u);
        self.union_variants.insert(name.clone(), vnames.clone());
        let umap = self.tparam_map(u, args);
        let sig = self.ck.unions.get(&u).cloned();
        let mut variants = Vec::new();
        if let Some(sig) = sig {
            for (i, v) in sig.variants.iter().enumerate() {
                let vtps = self.ck.defs[v.def.0 as usize].tparams.clone();
                let vargs: Vec<Ty> = v.args.iter().map(|a| self.subst(*a, &umap)).collect();
                let vmap: Vec<(ParamId, Ty)> = vtps.into_iter().zip(vargs).collect();
                let rec = self.ck.records.get(&v.def).cloned();
                let mut fields = Vec::new();
                if let Some(rec) = rec {
                    for f in rec.fields.iter().filter(|f| !f.is_static) {
                        let t = self.subst(f.ty, &vmap);
                        if let Some(ft) = self.repr(t) {
                            fields.push(Field {
                                name: f.name.clone(),
                                ty: ft,
                            });
                        }
                    }
                }
                variants.push(ir::Variant {
                    name: vnames[i].clone(),
                    fields,
                });
            }
        }
        self.out.types[idx].kind = TypeKind::Union { variants };
        name
    }

    /// IR variant name of variant definition `vdef` inside union IR type `uname`.
    pub fn variant_name(&self, uname: &str, index: u32) -> String {
        self.union_variants
            .get(uname)
            .and_then(|v| v.get(index as usize))
            .cloned()
            .unwrap_or_else(|| format!("v{index}"))
    }

    pub fn box_type(&mut self, scalar: &It) -> String {
        let key = mangle(scalar);
        if let Some(n) = self.boxes.get(&key) {
            return n.clone();
        }
        let name = format!("Nb_{key}");
        self.push_type(TypeDecl {
            name: name.clone(),
            attrs: Vec::new(),
            kind: TypeKind::Struct {
                fields: vec![Field {
                    name: "v".into(),
                    ty: scalar.clone(),
                }],
            },
        });
        self.boxes.insert(key, name.clone());
        name
    }

    pub fn cell_type(&mut self, ty: &It) -> String {
        let key = mangle(ty);
        if let Some(n) = self.cells.get(&key) {
            return n.clone();
        }
        let name = format!("Cell_{key}");
        self.push_type(TypeDecl {
            name: name.clone(),
            attrs: Vec::new(),
            kind: TypeKind::Struct {
                fields: vec![Field {
                    name: "v".into(),
                    ty: ty.clone(),
                }],
            },
        });
        self.cells.insert(key, name.clone());
        name
    }

    /// The shared environment type of closures without captures.
    pub fn empty_env(&mut self) -> String {
        if let Some(n) = self.thunks.get("#env") {
            return n.clone();
        }
        let n = self.closure_env_type(Vec::new());
        self.thunks.insert("#env".into(), n.clone());
        n
    }

    pub fn closure_env_type(&mut self, caps: Vec<Field>) -> String {
        let id = self.next_id();
        let name = format!("Env_{id}");
        self.push_type(TypeDecl {
            name: name.clone(),
            attrs: Vec::new(),
            kind: TypeKind::Closure { captures: caps },
        });
        name
    }
}

pub(crate) fn err_ty() -> Ty {
    oxiscript_check::ty::ERROR
}
