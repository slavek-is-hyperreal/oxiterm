//! Instantiation hygiene: types of infinite size, polymorphic recursion (growing instantiation
//! cycles) and ill-formed instantiations (`void`/`never` as a field or element type).

use crate::ty::*;
use crate::*;

/// A module global or a static field (by index into the record's fields).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum GKey {
    Global(DefId),
    Static(DefId, u32),
}

/// A generic definition mentions another (or itself) with type arguments.
#[derive(Clone, Debug)]
pub(crate) struct InstEdge {
    pub from: DefId,
    pub to: DefId,
    /// Some argument is a compound type containing a type parameter (`T[]`, `Box<T>`): every trip
    /// around a cycle containing such an edge makes the argument one level deeper.
    pub growing: bool,
    pub span: Span,
}

/// Visit every type node reachable from `t` once.
fn walk(tys: &Interner, t: Ty, seen: &mut HashSet<Ty>, f: &mut dyn FnMut(&TyKind)) {
    if !seen.insert(t) || seen.len() > 4096 {
        return;
    }
    let k = tys.kind(t);
    f(k);
    match k {
        TyKind::Array(e) | TyKind::Nullable(e) => walk(tys, *e, seen, f),
        TyKind::Adt(_, a) | TyKind::Variants(_, a, _) => {
            for x in a {
                walk(tys, *x, seen, f);
            }
        }
        TyKind::Fn(ps, r) => {
            for x in ps {
                walk(tys, *x, seen, f);
            }
            walk(tys, *r, seen, f);
        }
        _ => {}
    }
}

/// Strongly connected components (iterative Tarjan); `adj[i]` lists successors of node `i`.
fn scc(adj: &[Vec<usize>]) -> Vec<usize> {
    let n = adj.len();
    let (mut index, mut low) = (vec![usize::MAX; n], vec![0usize; n]);
    let (mut on, mut comp) = (vec![false; n], vec![usize::MAX; n]);
    let (mut stack, mut next, mut ncomp) = (Vec::new(), 0usize, 0usize);
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on[root] = true;
        while let Some(&(v, i)) = call.last() {
            if i < adj[v].len() {
                if let Some(c) = call.last_mut() {
                    c.1 += 1;
                }
                let w = adj[v][i];
                if index[w] == usize::MAX {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on[w] = true;
                    call.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                call.pop();
                if let Some(&(p, _)) = call.last() {
                    low[p] = low[p].min(low[v]);
                }
                if low[v] == index[v] {
                    while let Some(w) = stack.pop() {
                        on[w] = false;
                        comp[w] = ncomp;
                        if w == v {
                            break;
                        }
                    }
                    ncomp += 1;
                }
            }
        }
    }
    comp
}

impl Checker<'_> {
    fn ty_has_param(&self, t: Ty) -> bool {
        let mut found = false;
        walk(&self.tys, t, &mut HashSet::new(), &mut |k| found |= matches!(k, TyKind::Param(_)));
        found
    }

    /// Record that the definition being checked instantiates `to` with `args`.
    pub(crate) fn note_inst(&mut self, to: DefId, args: &[Ty], span: Span) {
        if args.is_empty() {
            return;
        }
        let Some(from) = self.fnctx.iter().rev().find_map(|c| c.def) else { return };
        if !args.iter().any(|a| self.ty_has_param(*a)) {
            return;
        }
        let growing = args.iter().any(|a| !matches!(self.tys.kind(*a), TyKind::Param(_)) && self.ty_has_param(*a));
        if self.inst_edges.len() < 200_000 {
            self.inst_edges.push(InstEdge { from, to, growing, span });
        }
    }

    /// `void` and `never` cannot be an array element or a field of a struct/class, also not through
    /// a generic instantiation (only fields that *become* exactly `void`/`never` count) (`Box<void>`, `g<void>()` returning `T[]`). Union members (`Result<void, E>`) may hold `void`.
    pub(crate) fn check_wf(&mut self, span: Span, t: Ty) -> bool {
        let mut bad: Option<&'static str> = None;
        let mut recs: Vec<(DefId, Vec<Ty>)> = Vec::new();
        walk(&self.tys, t, &mut HashSet::new(), &mut |k| match k {
            TyKind::Array(e) if matches!(e.0, 1 | 2) => bad = Some("an array of `void` or `never`"),
            TyKind::Adt(d, a) if a.iter().any(|x| matches!(x.0, 1 | 2)) => recs.push((*d, a.to_vec())),
            _ => {}
        });
        for (d, args) in recs {
            if !matches!(self.kind_of(d), DefKind::Struct | DefKind::Class) {
                continue;
            }
            self.ensure_record(d);
            let Some(rec) = self.records.get(&d).cloned() else { continue };
            let tps = self.defs[d.0 as usize].info.tparams.clone();
            let map: Vec<(ParamId, Ty)> = tps.into_iter().zip(args).collect();
            for f in rec.fields.iter().filter(|f| !f.is_static) {
                let ft = self.tys.subst(f.ty, &map);
                if matches!(ft.0, 1 | 2) {
                    bad = Some("a struct or class field that becomes `void` or `never`");
                }
            }
        }
        if let Some(what) = bad {
            self.err("OXS-E0115", span, &[("detail", &format!("ill-formed type: {what}"))]);
            return false;
        }
        true
    }

    /// Post-pass: by-value recursion and growing instantiation cycles.
    pub(crate) fn check_instantiation_cycles(&mut self) {
        // record -> record edges from field types
        let recs: Vec<DefId> = {
            let mut v: Vec<DefId> = self.records.keys().copied().collect();
            v.sort();
            v
        };
        let mut edges = std::mem::take(&mut self.inst_edges);
        let mut byval: Vec<(DefId, DefId, Span)> = Vec::new();
        let by_param: HashMap<DefId, Vec<usize>> = recs
            .iter()
            .filter_map(|d| {
                let r = self.records.get(d)?;
                let tps = &self.defs[d.0 as usize].info.tparams;
                let v: Vec<usize> = r
                    .fields
                    .iter()
                    .filter(|f| !f.is_static)
                    .filter_map(|f| match self.tys.kind(f.ty) {
                        TyKind::Param(p) => tps.iter().position(|x| x == p),
                        _ => None,
                    })
                    .collect();
                Some((*d, v))
            })
            .collect();
        for d in &recs {
            if !matches!(self.kind_of(*d), DefKind::Struct | DefKind::Class) {
                continue;
            }
            let Some(r) = self.records.get(d) else { continue };
            for f in r.fields.iter().filter(|f| !f.is_static) {
                // growing instantiation edges: any mention of a generic with compound arguments
                let mut mentions: Vec<(DefId, Vec<Ty>)> = Vec::new();
                walk(&self.tys, f.ty, &mut HashSet::new(), &mut |k| {
                    if let TyKind::Adt(t, a) = k {
                        if !a.is_empty() {
                            mentions.push((*t, a.to_vec()));
                        }
                    }
                });
                for (t, a) in mentions {
                    let growing = a.iter().any(|x| !matches!(self.tys.kind(*x), TyKind::Param(_)) && self.ty_has_param(*x));
                    if growing && edges.len() < 200_000 {
                        edges.push(InstEdge { from: *d, to: t, growing, span: f.span });
                    }
                }
                // by-value (non-null, non-array) containment
                if let TyKind::Adt(t, a) = self.tys.kind(f.ty) {
                    if matches!(self.kind_of(*t), DefKind::Struct | DefKind::Class) {
                        byval.push((*d, *t, f.span));
                        if let Some(ps) = by_param.get(t) {
                            for i in ps {
                                if let Some(x) = a.get(*i) {
                                    if let TyKind::Adt(t2, _) = self.tys.kind(*x) {
                                        if matches!(self.kind_of(*t2), DefKind::Struct | DefKind::Class) {
                                            byval.push((*d, *t2, f.span));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // by-value cycles
        let mut ids: HashMap<DefId, usize> = HashMap::new();
        for (a, b, _) in &byval {
            let n = ids.len();
            ids.entry(*a).or_insert(n);
            let n = ids.len();
            ids.entry(*b).or_insert(n);
        }
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
        for (a, b, _) in &byval {
            adj[ids[a]].push(ids[b]);
        }
        let comp = scc(&adj);
        let mut reported: HashSet<usize> = HashSet::new();
        for (a, b, sp) in &byval {
            let (x, y) = (ids[a], ids[b]);
            if (comp[x] == comp[y]) && (x != y || adj[x].contains(&x)) && reported.insert(comp[x]) {
                let n = self.def_name(*a).to_string();
                self.err("OXS-E0115", *sp, &[("detail", &format!("`{n}` contains itself by value (directly or through other structs), so no value of it can be built; make a field nullable (`| null`) or use an array"))]);
            }
        }
        // growing instantiation cycles
        let mut ids: HashMap<DefId, usize> = HashMap::new();
        for e in &edges {
            let n = ids.len();
            ids.entry(e.from).or_insert(n);
            let n = ids.len();
            ids.entry(e.to).or_insert(n);
        }
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
        for e in &edges {
            adj[ids[&e.from]].push(ids[&e.to]);
        }
        let comp = scc(&adj);
        let mut reported: HashSet<usize> = HashSet::new();
        for e in &edges {
            let (x, y) = (ids[&e.from], ids[&e.to]);
            if e.growing && comp[x] == comp[y] && reported.insert(comp[x]) {
                let n = self.def_name(e.to).to_string();
                self.err("OXS-E0113", e.span, &[("detail", &format!("`{n}` is instantiated with ever larger type arguments around a cycle (polymorphic recursion): monomorphisation would never end"))]);
            }
        }
    }

    /// Module initialisation order: items run in program order (imported modules first, then
    /// source order; a class's statics in member order). An initialiser may only read values
    /// initialised before it, directly or through any chain of calls.
    pub(crate) fn check_init_order(&mut self, order: &[usize]) {
        let mut ord: HashMap<GKey, i64> = HashMap::new();
        for &k in order {
            for &d in &self.mods[k].out.items {
                match self.defs[d.0 as usize].info.kind {
                    DefKind::Global => {
                        let n = ord.len() as i64;
                        ord.insert(GKey::Global(d), n);
                    }
                    DefKind::Struct | DefKind::Class => {
                        if let Some(r) = self.records.get(&d) {
                            for (i, f) in r.fields.iter().enumerate() {
                                if f.is_static {
                                    let n = ord.len() as i64;
                                    ord.insert(GKey::Static(d, i as u32), n);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // highest initialisation ordinal each function can read, transitively through calls
        let mut maxr: HashMap<DefId, i64> = HashMap::new();
        let mut rev: HashMap<DefId, Vec<DefId>> = HashMap::new();
        for (d, r) in &self.eff {
            let m = r.reads.iter().filter_map(|k| ord.get(k)).copied().max().unwrap_or(-1);
            maxr.insert(*d, m);
            for (c, _) in &r.edges {
                rev.entry(*c).or_default().push(*d);
            }
        }
        let mut work: Vec<DefId> = maxr.iter().filter(|(_, m)| **m >= 0).map(|(d, _)| *d).collect();
        while let Some(d) = work.pop() {
            let m = maxr.get(&d).copied().unwrap_or(-1);
            for c in rev.get(&d).map(Vec::as_slice).unwrap_or(&[]) {
                let cm = maxr.entry(*c).or_insert(-1);
                if *cm < m {
                    *cm = m;
                    work.push(*c);
                }
            }
        }
        let mut keys: Vec<GKey> = self.init_recs.keys().copied().collect();
        keys.sort_by_key(|k| ord.get(k).copied().unwrap_or(-1));
        for k in keys {
            let Some(&o) = ord.get(&k) else { continue };
            let rec = self.init_recs.get(&k).cloned().unwrap_or_default();
            for (r, sp) in rec.reads {
                if ord.get(&r).is_some_and(|x| *x >= o) {
                    let n = self.gkey_name(r);
                    self.err("OXS-E0123", sp, &[("detail", &format!("`{n}` is read before it is initialised: module-level initialisers run in order and may only read earlier ones"))]);
                }
            }
            for (c, sp) in rec.calls {
                if maxr.get(&c).is_some_and(|x| *x >= o) {
                    let n = self.def_name(c).to_string();
                    self.err("OXS-E0123", sp, &[("detail", &format!("`{n}` reads a module-level value that is initialised after (or by) this initialiser; reorder the declarations"))]);
                }
            }
        }
    }

    fn gkey_name(&self, k: GKey) -> String {
        match k {
            GKey::Global(d) => self.def_name(d).to_string(),
            GKey::Static(d, i) => {
                let f = self.records.get(&d).and_then(|r| r.fields.get(i as usize)).map_or("?", |f| f.name.as_str());
                format!("{}.{f}", self.def_name(d))
            }
        }
    }
}
