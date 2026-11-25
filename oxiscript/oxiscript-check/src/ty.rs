//! Interned types. `Ty` is a `Copy` handle; equality is handle equality.

use oxiscript_syntax::NumTy;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ty(pub u32);

/// A declaration (struct, class, union, variant, enum, interface, alias, host type, function, global, law, namespace).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefId(pub u32);

/// A generic type parameter (global numbering across the program).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TyKind {
    /// Poison type: produced after a diagnostic, assignable to and from everything.
    Error,
    /// The type of expressions that never produce a value (`return`, a call that traps).
    Never,
    Void,
    Bool,
    Num(NumTy),
    Str,
    /// The type of the `null` literal; `T | null` is `Nullable(T)`.
    Null,
    Array(Ty),
    /// `T | null`; `T` is never `Null`, `Nullable`, `Never`, `Void` or `Error`.
    Nullable(Ty),
    /// A nominal type: struct, class, enum, union, union variant, interface,
    /// host capability or host handle, with its type arguments.
    Adt(DefId, Box<[Ty]>),
    /// A subset (at least two, not all) of the variants of a union, as a bit mask
    /// over variant indices. Produced by narrowing only.
    Variants(DefId, Box<[Ty]>, u64),
    Param(ParamId),
    Fn(Box<[Ty]>, Ty),
}

pub const ERROR: Ty = Ty(0);
pub const NEVER: Ty = Ty(1);
pub const VOID: Ty = Ty(2);
pub const BOOL: Ty = Ty(3);
pub const STR: Ty = Ty(4);
pub const NULL: Ty = Ty(5);
const NUM_BASE: u32 = 6;

pub const MAX_TY_DEPTH: u32 = 32;
pub const MAX_TY_COUNT: usize = 400_000;

pub fn num_index(n: NumTy) -> u32 {
    match n {
        NumTy::I8 => 0,
        NumTy::I16 => 1,
        NumTy::I32 => 2,
        NumTy::I64 => 3,
        NumTy::U8 => 4,
        NumTy::U16 => 5,
        NumTy::U32 => 6,
        NumTy::U64 => 7,
        NumTy::F32 => 8,
        NumTy::F64 => 9,
    }
}
pub const ALL_NUM: [NumTy; 10] = [
    NumTy::I8,
    NumTy::I16,
    NumTy::I32,
    NumTy::I64,
    NumTy::U8,
    NumTy::U16,
    NumTy::U32,
    NumTy::U64,
    NumTy::F32,
    NumTy::F64,
];

pub fn num(n: NumTy) -> Ty {
    Ty(NUM_BASE + num_index(n))
}
pub const I32: Ty = Ty(NUM_BASE + 2);
pub const F64: Ty = Ty(NUM_BASE + 9);
pub const U8: Ty = Ty(NUM_BASE + 4);

#[derive(Debug)]
pub struct Interner {
    kinds: Vec<TyKind>,
    depth: Vec<u8>,
    map: HashMap<TyKind, Ty>,
    /// Set when a type was refused for being too deep or the table too big.
    pub overflowed: bool,
}

impl Default for Interner {
    fn default() -> Self {
        Self::new()
    }
}

impl Interner {
    pub fn new() -> Interner {
        let mut i = Interner {
            kinds: Vec::new(),
            depth: Vec::new(),
            map: HashMap::new(),
            overflowed: false,
        };
        for k in [
            TyKind::Error,
            TyKind::Never,
            TyKind::Void,
            TyKind::Bool,
            TyKind::Str,
            TyKind::Null,
        ] {
            i.raw(k);
        }
        for n in ALL_NUM {
            i.raw(TyKind::Num(n));
        }
        i
    }
    fn raw(&mut self, k: TyKind) -> Ty {
        let d = match &k {
            TyKind::Array(t) | TyKind::Nullable(t) => u32::from(self.depth[t.0 as usize]) + 1,
            TyKind::Adt(_, a) | TyKind::Variants(_, a, _) => {
                if a.is_empty() {
                    0
                } else {
                    a.iter()
                        .map(|t| u32::from(self.depth[t.0 as usize]))
                        .max()
                        .unwrap_or(0)
                        + 1
                }
            }
            TyKind::Fn(p, r) => {
                p.iter()
                    .map(|t| u32::from(self.depth[t.0 as usize]))
                    .max()
                    .unwrap_or(0)
                    .max(u32::from(self.depth[r.0 as usize]))
                    + 1
            }
            _ => 0,
        };
        let t = Ty(u32::try_from(self.kinds.len()).unwrap_or(u32::MAX));
        self.depth.push(u8::try_from(d.min(255)).unwrap_or(255));
        self.kinds.push(k.clone());
        self.map.insert(k, t);
        t
    }
    pub fn mk(&mut self, k: TyKind) -> Ty {
        if let Some(&t) = self.map.get(&k) {
            return t;
        }
        if self.kinds.len() >= MAX_TY_COUNT {
            self.overflowed = true;
            return ERROR;
        }
        let t = self.raw(k);
        if u32::from(self.depth[t.0 as usize]) > MAX_TY_DEPTH {
            self.overflowed = true;
            return ERROR;
        }
        t
    }
    pub fn kind(&self, t: Ty) -> &TyKind {
        &self.kinds[t.0 as usize]
    }
    pub fn depth(&self, t: Ty) -> u32 {
        u32::from(self.depth[t.0 as usize])
    }
    pub fn len(&self) -> usize {
        self.kinds.len()
    }
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
    pub fn array(&mut self, t: Ty) -> Ty {
        if t == ERROR {
            return ERROR;
        }
        self.mk(TyKind::Array(t))
    }
    /// `t | null`, canonicalised (idempotent; `null | null` is `null`; poison stays poison).
    pub fn nullable(&mut self, t: Ty) -> Ty {
        match self.kind(t) {
            TyKind::Null | TyKind::Nullable(_) | TyKind::Error | TyKind::Never | TyKind::Void => t,
            _ => self.mk(TyKind::Nullable(t)),
        }
    }
    pub fn adt(&mut self, d: DefId, args: &[Ty]) -> Ty {
        if args.contains(&ERROR) {
            return ERROR;
        }
        self.mk(TyKind::Adt(d, args.into()))
    }
    pub fn func(&mut self, p: &[Ty], r: Ty) -> Ty {
        self.mk(TyKind::Fn(p.into(), r))
    }
    /// Replace type parameters. Linear in the size of `t`.
    pub fn subst(&mut self, t: Ty, map: &[(ParamId, Ty)]) -> Ty {
        if map.is_empty() {
            return t;
        }
        match self.kind(t).clone() {
            TyKind::Param(p) => map.iter().find(|(q, _)| *q == p).map_or(t, |(_, r)| *r),
            TyKind::Array(e) => {
                let e2 = self.subst(e, map);
                if e2 == e {
                    t
                } else {
                    self.array(e2)
                }
            }
            TyKind::Nullable(e) => {
                let e2 = self.subst(e, map);
                if e2 == e {
                    t
                } else {
                    self.nullable(e2)
                }
            }
            TyKind::Adt(d, a) => {
                let na: Vec<Ty> = a.iter().map(|x| self.subst(*x, map)).collect();
                if *na == *a {
                    t
                } else {
                    self.adt(d, &na)
                }
            }
            TyKind::Variants(d, a, m) => {
                let na: Vec<Ty> = a.iter().map(|x| self.subst(*x, map)).collect();
                if *na == *a || na.contains(&ERROR) {
                    t
                } else {
                    self.mk(TyKind::Variants(d, na.into(), m))
                }
            }
            TyKind::Fn(p, r) => {
                let np: Vec<Ty> = p.iter().map(|x| self.subst(*x, map)).collect();
                let nr = self.subst(r, map);
                if *np == *p && nr == r {
                    t
                } else {
                    self.func(&np, nr)
                }
            }
            _ => t,
        }
    }
    pub fn contains_param(&self, t: Ty, p: ParamId) -> bool {
        match self.kind(t) {
            TyKind::Param(q) => *q == p,
            TyKind::Array(e) | TyKind::Nullable(e) => self.contains_param(*e, p),
            TyKind::Adt(_, a) | TyKind::Variants(_, a, _) => {
                a.iter().any(|x| self.contains_param(*x, p))
            }
            TyKind::Fn(ps, r) => {
                ps.iter().any(|x| self.contains_param(*x, p)) || self.contains_param(*r, p)
            }
            _ => false,
        }
    }
    pub fn has_param(&self, t: Ty) -> bool {
        match self.kind(t) {
            TyKind::Param(_) => true,
            TyKind::Array(e) | TyKind::Nullable(e) => self.has_param(*e),
            TyKind::Adt(_, a) | TyKind::Variants(_, a, _) => a.iter().any(|x| self.has_param(*x)),
            TyKind::Fn(ps, r) => ps.iter().any(|x| self.has_param(*x)) || self.has_param(*r),
            _ => false,
        }
    }
    pub fn has_error(&self, t: Ty) -> bool {
        match self.kind(t) {
            TyKind::Error => true,
            TyKind::Array(e) | TyKind::Nullable(e) => self.has_error(*e),
            TyKind::Adt(_, a) | TyKind::Variants(_, a, _) => a.iter().any(|x| self.has_error(*x)),
            TyKind::Fn(ps, r) => ps.iter().any(|x| self.has_error(*x)) || self.has_error(*r),
            _ => false,
        }
    }
}

pub fn int_bits(n: NumTy) -> u32 {
    match n {
        NumTy::I8 | NumTy::U8 => 8,
        NumTy::I16 | NumTy::U16 => 16,
        NumTy::I32 | NumTy::U32 | NumTy::F32 => 32,
        NumTy::I64 | NumTy::U64 | NumTy::F64 => 64,
    }
}
pub fn is_signed(n: NumTy) -> bool {
    matches!(n, NumTy::I8 | NumTy::I16 | NumTy::I32 | NumTy::I64)
}
pub fn is_int(n: NumTy) -> bool {
    !n.is_float()
}
/// Largest magnitude representable as a non-negative value (`neg == false`) or
/// as a negated value (`neg == true`) in integer type `n`.
pub fn int_max(n: NumTy, neg: bool) -> u128 {
    let bits = int_bits(n);
    if is_signed(n) {
        let h = 1u128 << (bits - 1);
        if neg {
            h
        } else {
            h - 1
        }
    } else if neg {
        0
    } else {
        (1u128 << bits) - 1
    }
}
