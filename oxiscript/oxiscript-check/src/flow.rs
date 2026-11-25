//! Flow-sensitive narrowing (TypeScript's control-flow model, restricted to what is sound).
//!
//! A [`Flow`] is the abstract state at a program point: facts `path -> narrowed type`,
//! the set of not-yet-assigned locals, and a `dead` flag (the point is unreachable).
//! Paths are a local variable (or `this` of a struct method) followed by at most
//! [`MAX_PATH`] field names of *value types* (structs, union variants). A path through
//! a class, an array element or a module-level `let` is never narrowed because any
//! call can change what it names. Facts are killed by assignment to the path or a
//! prefix of it, and by calling a `mutating` method on a prefix. Names assigned in a
//! closure are never narrowed anywhere.

use crate::ty::*;
use crate::*;

pub const MAX_PATH: usize = 6;

#[derive(Clone, Debug, Default)]
pub struct Flow {
    pub facts: Vec<(u32, Ty)>,
    pub unassigned: Vec<u32>,
    pub dead: bool,
}

impl Flow {
    pub fn get(&self, p: u32) -> Option<Ty> {
        self.facts.iter().rev().find(|(q, _)| *q == p).map(|(_, t)| *t)
    }
    pub fn set(&mut self, p: u32, t: Ty) {
        if let Some(f) = self.facts.iter_mut().find(|(q, _)| *q == p) {
            f.1 = t;
        } else {
            self.facts.push((p, t));
        }
    }
    /// `unassigned` is kept sorted; constructor field markers (`0x8000_0000 | index`) sort last.
    pub fn is_unassigned(&self, local: u32) -> bool {
        self.unassigned.binary_search(&local).is_ok()
    }
    pub fn assign(&mut self, local: u32) {
        if let Ok(i) = self.unassigned.binary_search(&local) {
            self.unassigned.remove(i);
        }
    }
    pub fn mark_unassigned(&mut self, local: u32) {
        if let Err(i) = self.unassigned.binary_search(&local) {
            self.unassigned.insert(i, local);
        }
    }
    /// Some constructor field is not assigned yet.
    pub fn has_unassigned_fields(&self) -> bool {
        self.unassigned.last().is_some_and(|u| *u & 0x8000_0000 != 0)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PathNode {
    pub parent: Option<u32>,
    pub root: u32,
    #[allow(dead_code)]
    pub field: u32,
    pub depth: u32,
}

#[derive(Default)]
pub struct PathTable {
    pub nodes: Vec<PathNode>,
    map: HashMap<(u32, u32, u32), u32>,
    syms: HashMap<String, u32>,
}

impl PathTable {
    pub fn root(&mut self, local: u32) -> u32 {
        if let Some(&p) = self.map.get(&(u32::MAX, local, 0)) {
            return p;
        }
        self.nodes.push(PathNode { parent: None, root: local, field: 0, depth: 0 });
        let id = (self.nodes.len() - 1) as u32;
        self.map.insert((u32::MAX, local, 0), id);
        id
    }
    pub fn field(&mut self, parent: u32, name: &str) -> Option<u32> {
        let n = self.nodes[parent as usize];
        if n.depth as usize >= MAX_PATH {
            return None;
        }
        let nsyms = self.syms.len() as u32;
        let sym = *self.syms.entry(name.to_string()).or_insert(nsyms);
        if let Some(&p) = self.map.get(&(parent, sym, 1)) {
            return Some(p);
        }
        self.nodes.push(PathNode { parent: Some(parent), root: n.root, field: sym, depth: n.depth + 1 });
        let id = (self.nodes.len() - 1) as u32;
        self.map.insert((parent, sym, 1), id);
        Some(id)
    }
    /// Is `anc` equal to `p` or a prefix of it?
    pub fn is_prefix(&self, anc: u32, mut p: u32) -> bool {
        loop {
            if p == anc {
                return true;
            }
            match self.nodes[p as usize].parent {
                Some(q) => p = q,
                None => return false,
            }
        }
    }
}

impl Checker<'_> {
    /// Remove facts about `p` and everything below it.
    pub fn kill_path(&mut self, p: u32) {
        let paths = &self.paths;
        self.flow.facts.retain(|(q, _)| !paths.is_prefix(p, *q));
    }

    // ---- union views ------------------------------------------------------

    /// The union a type belongs to with the set of variants it may be:
    /// `(union def, union args, mask)`.
    pub fn union_view(&self, t: Ty) -> Option<(DefId, Box<[Ty]>, u64)> {
        match self.tys.kind(t) {
            TyKind::Adt(d, a) => match self.defs[d.0 as usize].info.kind {
                DefKind::Union => {
                    let n = self.unions.get(d)?.variants.len();
                    Some((*d, a.clone(), full_mask(n)))
                }
                DefKind::Variant { union: Some(u), index } => {
                    let args: Box<[Ty]> = if self.defs[u.0 as usize].info.tparams.is_empty() {
                        Box::new([])
                    } else {
                        a.clone()
                    };
                    Some((u, args, 1u64 << index))
                }
                _ => None,
            },
            TyKind::Variants(d, a, m) => Some((*d, a.clone(), *m)),
            _ => None,
        }
    }

    /// Canonical type for a subset of a union's variants.
    pub fn mk_variants(&mut self, u: DefId, args: &[Ty], mask: u64) -> Ty {
        let Some(sig) = self.unions.get(&u) else { return ERROR };
        let n = sig.variants.len();
        if mask == 0 {
            return NEVER;
        }
        if mask == full_mask(n) {
            return self.tys.adt(u, args);
        }
        if mask.is_power_of_two() {
            let i = mask.trailing_zeros() as usize;
            let v = sig.variants[i].clone();
            let tp = self.defs[u.0 as usize].info.tparams.clone();
            let map: Vec<(ParamId, Ty)> = tp.iter().copied().zip(args.iter().copied()).collect();
            let vargs: Vec<Ty> = v.args.iter().map(|x| self.tys.subst(*x, &map)).collect();
            return self.tys.adt(v.def, &vargs);
        }
        self.tys.mk(TyKind::Variants(u, args.into(), mask))
    }

    /// Least type containing `a` and `b` that narrowing can express (joins of facts).
    pub fn join_ty(&mut self, a: Ty, b: Ty) -> Option<Ty> {
        if a == b {
            return Some(a);
        }
        if a == NEVER {
            return Some(b);
        }
        if b == NEVER {
            return Some(a);
        }
        if a == ERROR || b == ERROR {
            return Some(ERROR);
        }
        let split = |tys: &Interner, t: Ty| -> (bool, Option<Ty>) {
            match tys.kind(t) {
                TyKind::Null => (true, None),
                TyKind::Nullable(i) => (true, Some(*i)),
                _ => (false, Some(t)),
            }
        };
        let (an, ai) = split(&self.tys, a);
        let (bn, bi) = split(&self.tys, b);
        let inner = match (ai, bi) {
            (None, None) => None,
            (Some(x), None) | (None, Some(x)) => Some(x),
            (Some(x), Some(y)) => Some(self.join_nonnull(x, y)?),
        };
        match inner {
            None => Some(NULL),
            Some(i) => {
                if an || bn {
                    Some(self.tys.nullable(i))
                } else {
                    Some(i)
                }
            }
        }
    }

    fn join_nonnull(&mut self, x: Ty, y: Ty) -> Option<Ty> {
        if x == y {
            return Some(x);
        }
        let (ux, uy) = (self.union_view(x)?, self.union_view(y)?);
        if ux.0 == uy.0 && ux.1 == uy.1 {
            Some(self.mk_variants(ux.0, &ux.1, ux.2 | uy.2))
        } else {
            None
        }
    }

    /// Merge two control-flow paths.
    pub fn join_flows(&mut self, a: Flow, b: Flow) -> Flow {
        if a.dead {
            return b;
        }
        if b.dead {
            return a;
        }
        let mut facts = Vec::new();
        for (p, t) in &a.facts {
            if let Some(u) = b.get(*p) {
                if let Some(j) = self.join_ty(*t, u) {
                    facts.push((*p, j));
                }
            }
        }
        let (x, y) = (&a.unassigned, &b.unassigned);
        let mut un: Vec<u32> = Vec::with_capacity(x.len().max(y.len()));
        let (mut i, mut j) = (0, 0);
        while i < x.len() || j < y.len() {
            let v = match (x.get(i), y.get(j)) {
                (Some(p), Some(q)) if p == q => {
                    i += 1;
                    j += 1;
                    *p
                }
                (Some(p), Some(q)) if p < q => {
                    i += 1;
                    *p
                }
                (Some(_), Some(q)) => {
                    j += 1;
                    *q
                }
                (Some(p), None) => {
                    i += 1;
                    *p
                }
                (None, Some(q)) => {
                    j += 1;
                    *q
                }
                (None, None) => break,
            };
            un.push(v);
        }
        Flow { facts, unassigned: un, dead: false }
    }

    pub fn join_all(&mut self, fs: Vec<Flow>) -> Flow {
        let mut it = fs.into_iter();
        let mut acc = it.next().unwrap_or(Flow { facts: vec![], unassigned: vec![], dead: true });
        for f in it {
            acc = self.join_flows(acc, f);
        }
        acc
    }
}

pub fn full_mask(n: usize) -> u64 {
    if n >= 64 {
        u64::MAX
    } else {
        (1u64 << n) - 1
    }
}
