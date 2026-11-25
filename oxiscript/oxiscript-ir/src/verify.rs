//! Structural and type verifier. Producers (slice 8.L, passes) must emit
//! verified IR; consumers (8.G, 8.M) may assume it. The verifier checks
//! typing, SSA dominance and symbol resolution; it does **not** check
//! reference-count balance (that is the RC-balance checker of slice 8.M,
//! specified against the ownership table in `docs/oxiscript/ir.md`).

use crate::types::*;
use oxiscript_abi::WasmTy;
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError {
    pub func: Option<String>,
    pub msg: String,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.func {
            Some(n) => write!(f, "@{n}: {}", self.msg),
            None => write!(f, "{}", self.msg),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Options chosen by the **trusted toolchain**, never by the module itself.
#[derive(Clone, Copy, Debug, Default)]
pub struct VerifyOptions {
    /// Permit `ptr`, `load`/`store`, `intrinsic`, ptr bitcasts and reserved
    /// export names: only for the runtime-support module that implements the
    /// ABI itself (slice 8.M). A module declaring `#raw_memory` is rejected
    /// unless this is set, and setting it does not need the attribute.
    pub allow_raw_memory: bool,
}

/// Verify with default (untrusted) options.
pub fn verify(m: &Module) -> Vec<VerifyError> {
    verify_with(m, VerifyOptions::default())
}

/// Verify with explicit options. Never panics, whatever the module contains.
pub fn verify_with(m: &Module, opts: VerifyOptions) -> Vec<VerifyError> {
    let mut v = Verifier {
        m,
        opts,
        errs: Vec::new(),
        func: None,
    };
    v.ids_prepass();
    if !v.errs.is_empty() {
        return v.errs; // later checks index by id
    }
    v.module();
    v.errs
}

/// Convenience: `Ok(())` or the first error (default options).
pub fn verify_ok(m: &Module) -> Result<(), VerifyError> {
    verify_ok_with(m, VerifyOptions::default())
}

pub fn verify_ok_with(m: &Module, opts: VerifyOptions) -> Result<(), VerifyError> {
    match verify_with(m, opts).into_iter().next() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// The Wasm value type a value of this IR type occupies (all managed
/// references and `ptr` are `i32`).
pub fn wasm_class(t: &Ty) -> WasmTy {
    match t {
        Ty::I64 | Ty::U64 => WasmTy::I64,
        Ty::F32 => WasmTy::F32,
        Ty::F64 => WasmTy::F64,
        _ => WasmTy::I32,
    }
}

/// Does the integer constant fit the type?
pub fn int_fits(t: &Ty, v: i128) -> bool {
    match t {
        Ty::I8 => (-128..=127).contains(&v),
        Ty::I16 => (-32768..=32767).contains(&v),
        Ty::I32 => (i32::MIN as i128..=i32::MAX as i128).contains(&v),
        Ty::I64 => (i64::MIN as i128..=i64::MAX as i128).contains(&v),
        Ty::U8 => (0..=255).contains(&v),
        Ty::U16 => (0..=65535).contains(&v),
        Ty::U32 | Ty::Ptr => (0..=u32::MAX as i128).contains(&v),
        Ty::U64 => (0..=u64::MAX as i128).contains(&v),
        _ => false,
    }
}

struct Verifier<'a> {
    m: &'a Module,
    opts: VerifyOptions,
    errs: Vec<VerifyError>,
    func: Option<String>,
}

impl Verifier<'_> {
    fn e(&mut self, msg: impl Into<String>) {
        self.errs.push(VerifyError {
            func: self.func.clone(),
            msg: msg.into(),
        });
    }

    fn unique<'b>(&mut self, what: &str, names: impl Iterator<Item = &'b str>) {
        let mut seen = HashSet::new();
        for n in names {
            if !seen.insert(n) {
                self.e(format!("duplicate {what} `{n}`"));
            }
        }
    }

    /// Module-wide structural pre-pass: every id and block target is in
    /// range and value numbering is canonical (textual definition order).
    /// Nothing else runs until this passes, so no later check can index out
    /// of range (including through callee signatures).
    fn ids_prepass(&mut self) {
        let m = self.m;
        // depth first: later checks recurse over types (compare, clone, print)
        let mut deep = false;
        let mut chk = |t: &Ty| deep |= too_deep(t, 1);
        for d in &m.types {
            match &d.kind {
                TypeKind::Struct { fields } => fields.iter().for_each(|f| chk(&f.ty)),
                TypeKind::Closure { captures } => captures.iter().for_each(|f| chk(&f.ty)),
                TypeKind::Union { variants } => variants
                    .iter()
                    .flat_map(|v| v.fields.iter())
                    .for_each(|f| chk(&f.ty)),
            }
        }
        for i in &m.imports {
            i.sig
                .params
                .iter()
                .chain(i.sig.rets.iter())
                .for_each(&mut chk);
        }
        m.globals.iter().for_each(|g| chk(&g.ty));
        for f in &m.funcs {
            f.values.iter().for_each(|v| chk(&v.ty));
            f.rets.iter().for_each(&mut chk);
        }
        if deep {
            self.e(format!("a type nests deeper than {MAX_TYPE_DEPTH} levels"));
            return;
        }
        for f in &m.funcs {
            self.func = Some(f.name.clone());
            let nvals = f.values.len();
            let nblocks = f.blocks.len();
            if nblocks == 0 {
                self.e("function has no blocks");
                continue;
            }
            let mut next = 0usize;
            let mut def = |this: &mut Self, v: ValueId| {
                if v.0 as usize != next {
                    this.e(format!(
                        "non-canonical value numbering: expected id {next}, found {} (ids follow textual definition order)",
                        v.0
                    ));
                }
                next += 1;
            };
            for p in &f.params {
                def(self, p.value);
            }
            for b in &f.blocks {
                for p in &b.params {
                    def(self, *p);
                }
                for i in &b.insts {
                    for r in &i.results {
                        def(self, *r);
                    }
                }
            }
            if next != nvals {
                self.e(format!("{nvals} value slots but {next} definitions"));
            }
            for b in &f.blocks {
                for i in &b.insts {
                    for o in i.op.operands() {
                        if o.0 as usize >= nvals {
                            self.e(format!(
                                "block `{}`: operand id {} out of range",
                                b.label, o.0
                            ));
                        }
                    }
                }
                for o in b.term.operands() {
                    if o.0 as usize >= nvals {
                        self.e(format!(
                            "block `{}`: operand id {} out of range",
                            b.label, o.0
                        ));
                    }
                }
                for t in b.term.targets() {
                    if t.block.0 as usize >= nblocks {
                        self.e(format!(
                            "block `{}`: branch to unknown block {}",
                            b.label, t.block.0
                        ));
                    }
                }
            }
        }
        self.func = None;
    }

    fn module(&mut self) {
        let m = self.m;
        if m.has_attr("raw_memory") && !self.opts.allow_raw_memory {
            self.e("#raw_memory modules may only be verified by the trusted toolchain (VerifyOptions::allow_raw_memory)");
        }
        for a in &m.attrs {
            self.attr_ok(a, "module");
        }
        self.unique("type", m.types.iter().map(|t| t.name.as_str()));
        self.unique(
            "symbol",
            m.imports
                .iter()
                .map(|i| i.name.as_str())
                .chain(m.funcs.iter().map(|f| f.name.as_str())),
        );
        self.unique("data", m.data.iter().map(|d| d.name.as_str()));
        self.unique("global", m.globals.iter().map(|g| g.name.as_str()));
        for n in m
            .imports
            .iter()
            .map(|i| &i.name)
            .chain(m.funcs.iter().map(|f| &f.name))
            .chain(m.data.iter().map(|d| &d.name))
            .chain(m.globals.iter().map(|g| &g.name))
        {
            if !valid_name(n) {
                self.e(format!("invalid symbol name `{n}` (use [A-Za-z0-9_.])"));
            }
        }

        for t in &m.types {
            if !valid_ident(&t.name) {
                self.e(format!("invalid type name `{}`", t.name));
            }
            for a in &t.attrs {
                self.attr_ok(a, &format!("type {}", t.name));
            }
            let groups: Vec<(&str, &Vec<Field>)> = match &t.kind {
                TypeKind::Struct { fields } => vec![("", fields)],
                TypeKind::Closure { captures } => vec![("", captures)],
                TypeKind::Union { variants } => {
                    self.unique("variant", variants.iter().map(|v| v.name.as_str()));
                    variants
                        .iter()
                        .map(|v| (v.name.as_str(), &v.fields))
                        .collect()
                }
            };
            for (vname, fields) in groups {
                if !vname.is_empty() && !valid_field_name(vname) {
                    self.e(format!("invalid variant name `{vname}`"));
                }
                self.unique("field", fields.iter().map(|f| f.name.as_str()));
                for f in fields {
                    if !valid_field_name(&f.name) {
                        self.e(format!("invalid field name `{}.{}`", t.name, f.name));
                    }
                    self.ty_valid(&f.ty, &format!("field `{}.{}`", t.name, f.name));
                }
            }
            self.check_colour_hints(t);
        }
        for i in &m.imports {
            self.import(i);
        }
        for g in &m.globals {
            self.ty_valid(&g.ty, &format!("global @{}", g.name));
            if g.ty.is_managed() && !matches!(g.ty, Ty::Opt(_)) {
                self.e(format!(
                    "global @{}: a managed global must be nullable (it starts null and has no initialiser)",
                    g.name
                ));
            }
            if let Some(c) = &g.init {
                if !g.ty.is_scalar() || !const_matches(c, &g.ty) {
                    self.e(format!(
                        "global @{}: initialiser does not match type",
                        g.name
                    ));
                }
            }
        }
        let mut exported: HashSet<String> = HashSet::new();
        for f in &m.funcs {
            self.func = Some(f.name.clone());
            self.function(f);
            if f.export {
                let name = f
                    .attrs
                    .iter()
                    .find(|a| a.name == "export_name")
                    .and_then(|a| a.args.first())
                    .map_or(f.name.as_str(), String::as_str);
                if !self.opts.allow_raw_memory
                    && oxiscript_abi::exports::is_reserved_export_name(name)
                {
                    self.e(format!("export name `{name}` is reserved by the ABI"));
                }
                if !exported.insert(name.to_string()) {
                    self.e(format!("duplicate export name `{name}`"));
                }
            }
            self.func = None;
        }
    }

    fn attr_ok(&mut self, a: &Attr, ctx: &str) {
        if !valid_name(&a.name) {
            self.e(format!("{ctx}: invalid attribute name `{}`", a.name));
        }
    }

    fn import(&mut self, i: &Import) {
        let ctx = format!("import @{}", i.name);
        let module_ok = i
            .module
            .strip_prefix("host:")
            .and_then(|r| r.rsplit_once('@'))
            .is_some_and(|(n, v)| {
                !n.is_empty()
                    && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !v.is_empty()
                    && v.bytes().all(|b| b.is_ascii_digit())
                    && !v.starts_with('0')
                    && v.parse::<u32>().is_ok()
            });
        if !module_ok {
            self.e(format!(
                "{ctx}: import module `{}` must be `host:<name>@<version>` (no env/WASI imports)",
                i.module
            ));
        }
        if i.field.is_empty() {
            self.e(format!("{ctx}: empty import field name"));
        }
        let raw = self.opts.allow_raw_memory;
        let flat = |t: &Ty| t.is_scalar() && (raw || *t != Ty::Ptr);
        for t in i.sig.params.iter().chain(i.sig.rets.iter()) {
            self.ty_valid(t, &ctx);
        }
        for t in &i.sig.params {
            // typed boundary: scalars, `str`, `[u8]` and consumed closures
            let ok = flat(t)
                || matches!(t, Ty::Str)
                || matches!(t, Ty::Array(e) if **e == Ty::U8)
                || matches!(t, Ty::Fn(sig) if sig.params.iter().all(|p| flat(p) || *p == Ty::Str || matches!(p, Ty::Array(e) if **e == Ty::U8)) && sig.rets.iter().all(&flat));
            if !ok {
                self.e(format!(
                    "{ctx}: parameter type `{}` is not allowed on a host import (scalars, str, [u8], fn over those)",
                    crate::print::ty_text(t)
                ));
            }
        }
        for t in &i.sig.rets {
            if !flat(t) {
                self.e(format!(
                    "{ctx}: host imports return flat scalars only, found `{}`",
                    crate::print::ty_text(t)
                ));
            }
        }
    }

    /// `#acyclic` must be true; `#acyclic` and `#cyclic` conflict.
    fn check_colour_hints(&mut self, t: &TypeDecl) {
        let acyc = t.attrs.iter().any(|a| a.name == "acyclic");
        let cyc = t.attrs.iter().any(|a| a.name == "cyclic");
        if acyc && cyc {
            self.e(format!("type {}: #acyclic and #cyclic conflict", t.name));
        }
        if acyc && self.can_reach_itself(&t.name) {
            self.e(format!(
                "type {}: #acyclic but its field types can form a cycle (self reference or closure capture)",
                t.name
            ));
        }
    }

    /// Conservative: true if `name` is reachable from its own fields through
    /// `ref` types, arrays and nullable forms, or any field is a closure type
    /// (a closure may capture the object).
    fn can_reach_itself(&self, name: &str) -> bool {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut stack: Vec<&str> = vec![name];
        let mut first = true;
        while let Some(cur) = stack.pop() {
            if !first && cur == name {
                return true;
            }
            if !first && !seen.insert(cur) {
                continue;
            }
            first = false;
            let Some(d) = self.m.type_decl(cur) else {
                continue;
            };
            let fields: Vec<&Field> = match &d.kind {
                TypeKind::Struct { fields } => fields.iter().collect(),
                TypeKind::Closure { captures } => captures.iter().collect(),
                TypeKind::Union { variants } => {
                    variants.iter().flat_map(|v| v.fields.iter()).collect()
                }
            };
            for f in fields {
                let mut t = &f.ty;
                loop {
                    match t {
                        Ty::Array(e) | Ty::Opt(e) => t = e,
                        Ty::Ref(n) => {
                            stack.push(n.as_str());
                            break;
                        }
                        Ty::Fn(_) => return true,
                        _ => break,
                    }
                }
            }
        }
        false
    }

    fn ty_valid(&mut self, t: &Ty, ctx: &str) {
        if too_deep(t, 1) {
            self.e(format!(
                "{ctx}: type nests deeper than {MAX_TYPE_DEPTH} levels"
            ));
            return;
        }
        self.ty_valid_rec(t, ctx);
    }

    fn ty_valid_rec(&mut self, t: &Ty, ctx: &str) {
        match t {
            Ty::Ptr if !self.opts.allow_raw_memory => {
                self.e(format!(
                    "{ctx}: `ptr` is only allowed in trusted raw-memory modules"
                ));
            }
            Ty::Ref(n) => {
                if self.m.type_decl(n).is_none() {
                    self.e(format!("{ctx}: unknown type `{n}`"));
                }
            }
            Ty::Array(e) => self.ty_valid_rec(e, ctx),
            Ty::Fn(sig) => {
                if sig.rets.len() > 1 {
                    self.e(format!("{ctx}: closure types have at most one result"));
                }
                for p in sig.params.iter().chain(sig.rets.iter()) {
                    self.ty_valid_rec(p, ctx);
                }
            }
            Ty::Opt(inner) => {
                if !matches!(**inner, Ty::Str | Ty::Ref(_) | Ty::Array(_) | Ty::Fn(_)) {
                    self.e(format!(
                        "{ctx}: only managed reference types can be nullable"
                    ));
                }
                self.ty_valid_rec(inner, ctx);
            }
            _ => {}
        }
    }

    fn function(&mut self, f: &Func) {
        let nvals = f.values.len();
        if !valid_name(&f.name) {
            self.e(format!("invalid function name `{}`", f.name));
        }
        for a in &f.attrs {
            self.attr_ok(a, "function");
        }
        for v in &f.values {
            if !valid_name(&v.name) {
                self.e(format!(
                    "invalid value name `{}` (use [A-Za-z0-9_.])",
                    v.name
                ));
            }
            self.ty_valid(&v.ty, &format!("value %{}", v.name));
        }
        for r in &f.rets {
            self.ty_valid(r, "result");
        }
        for p in &f.params {
            if p.own && !f.ty(p.value).is_managed() {
                self.e(format!(
                    "parameter %{}: `own` applies to managed (reference-counted) types only",
                    f.value(p.value).name
                ));
            }
        }
        self.unique("value name", f.values.iter().map(|v| v.name.as_str()));
        self.unique("block label", f.blocks.iter().map(|b| b.label.as_str()));
        for b in &f.blocks {
            if !valid_ident(&b.label) {
                self.e(format!("invalid block label `{}`", b.label));
            }
        }
        if !f.blocks[0].params.is_empty() {
            self.e("entry block must not have parameters");
        }

        // definition sites (ids are canonical, checked by the pre-pass)
        let mut def: Vec<(usize, i64)> = vec![(0, -1); nvals];
        let mut k = f.params.len();
        for (bi, b) in f.blocks.iter().enumerate() {
            for _ in &b.params {
                def[k] = (bi, -1);
                k += 1;
            }
            for (ii, inst) in b.insts.iter().enumerate() {
                for _ in &inst.results {
                    def[k] = (bi, ii as i64);
                    k += 1;
                }
            }
        }

        // dominators: Cooper-Harvey-Kennedy over reverse postorder (linear memory)
        let n = f.blocks.len();
        let succs: Vec<Vec<usize>> = f
            .blocks
            .iter()
            .map(|b| {
                b.term
                    .targets()
                    .iter()
                    .map(|t| t.block.0 as usize)
                    .collect()
            })
            .collect();
        let mut preds: Vec<Vec<usize>> = vec![vec![]; n];
        for (b, ss) in succs.iter().enumerate() {
            for s in ss {
                preds[*s].push(b);
            }
        }
        // iterative DFS postorder
        let mut po: Vec<usize> = vec![usize::MAX; n]; // postorder number
        let mut order: Vec<usize> = Vec::new();
        let mut visited = vec![false; n];
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        visited[0] = true;
        while let Some((b, i)) = stack.last_mut() {
            if *i < succs[*b].len() {
                let s = succs[*b][*i];
                *i += 1;
                if !visited[s] {
                    visited[s] = true;
                    stack.push((s, 0));
                }
            } else {
                po[*b] = order.len();
                order.push(*b);
                stack.pop();
            }
        }
        let reach = visited;
        let mut idom: Vec<usize> = vec![usize::MAX; n];
        idom[0] = 0;
        loop {
            let mut changed = false;
            for &b in order.iter().rev() {
                if b == 0 {
                    continue;
                }
                let mut new_idom = usize::MAX;
                for &p in &preds[b] {
                    if idom[p] == usize::MAX {
                        continue;
                    }
                    new_idom = if new_idom == usize::MAX {
                        p
                    } else {
                        let (mut x, mut y) = (p, new_idom);
                        while x != y {
                            while po[x] < po[y] {
                                x = idom[x];
                            }
                            while po[y] < po[x] {
                                y = idom[y];
                            }
                        }
                        x
                    };
                }
                if new_idom != usize::MAX && idom[b] != new_idom {
                    idom[b] = new_idom;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // dominator tree intervals for O(1) dominance queries
        let mut children: Vec<Vec<usize>> = vec![vec![]; n];
        for b in 1..n {
            if reach[b] && idom[b] != usize::MAX {
                children[idom[b]].push(b);
            }
        }
        let (mut tin, mut tout) = (vec![0usize; n], vec![0usize; n]);
        let mut clock = 0usize;
        let mut st: Vec<(usize, usize)> = vec![(0, 0)];
        tin[0] = clock;
        clock += 1;
        while let Some((b, i)) = st.last_mut() {
            if *i < children[*b].len() {
                let c = children[*b][*i];
                *i += 1;
                tin[c] = clock;
                clock += 1;
                st.push((c, 0));
            } else {
                tout[*b] = clock;
                clock += 1;
                st.pop();
            }
        }
        let dominates = |a: usize, b: usize| tin[a] <= tin[b] && tout[b] <= tout[a];

        for (bi, b) in f.blocks.iter().enumerate() {
            let check_use = |this: &mut Self, v: ValueId, at: i64| {
                let (db, di) = def[v.0 as usize];
                if !reach[bi] {
                    return;
                }
                let ok = if db == bi {
                    di < at
                } else {
                    reach[db] && dominates(db, bi)
                };
                if !ok {
                    this.e(format!(
                        "block `{}`: use of %{} is not dominated by its definition",
                        b.label, f.values[v.0 as usize].name
                    ));
                }
            };
            for (ii, inst) in b.insts.iter().enumerate() {
                for o in inst.op.operands() {
                    check_use(self, o, ii as i64);
                }
                self.inst(f, b, inst);
            }
            for o in b.term.operands() {
                check_use(self, o, b.insts.len() as i64);
            }
            self.terminator(f, b);
            self.check_init(f, b);
        }
    }

    /// Straight-line initialisation rule: a freshly allocated object whose
    /// type has non-nullable managed fields may be used *only* by
    /// `setfield.init` (in the same block) until every such field is set.
    /// This makes uninitialised (null) non-nullable references unobservable.
    fn check_init(&mut self, f: &Func, b: &Block) {
        let ctx = format!("block `{}`", b.label);
        let mut fresh: HashMap<ValueId, HashSet<String>> = HashMap::new();
        let mut allocated_here: HashSet<ValueId> = HashSet::new();
        for inst in &b.insts {
            if let Op::SetField {
                field,
                obj,
                val,
                init,
            } = &inst.op
            {
                if *init && !allocated_here.contains(obj) {
                    self.e(format!("{ctx}: setfield.init requires an object allocated earlier in the same block"));
                }
                if fresh.contains_key(val) {
                    self.e(format!(
                        "{ctx}: an incompletely initialised object escapes into a field"
                    ));
                }
                if let Some(need) = fresh.get_mut(obj) {
                    if *init {
                        need.remove(&field.field);
                    } else if need.contains(&field.field) {
                        self.e(format!(
                            "{ctx}: first store to managed field `{}` must be `setfield.init`",
                            field.field
                        ));
                    }
                    if need.is_empty() {
                        fresh.remove(obj);
                    }
                }
                continue;
            }
            for o in inst.op.operands() {
                if fresh.contains_key(&o) {
                    self.e(format!(
                        "{ctx}: %{} is used before all its non-nullable managed fields are initialised with setfield.init",
                        f.value(o).name
                    ));
                }
            }
            if let (Op::Alloc(tr), Some(r)) = (&inst.op, inst.results.first()) {
                allocated_here.insert(*r);
                if let Some(d) = self.m.type_decl(&tr.ty) {
                    let fields: &[Field] = match (&d.kind, &tr.variant) {
                        (TypeKind::Struct { fields }, None) => fields,
                        (TypeKind::Closure { captures }, None) => captures,
                        (TypeKind::Union { variants }, Some(v)) => variants
                            .iter()
                            .find(|x| &x.name == v)
                            .map_or(&[][..], |x| &x.fields),
                        _ => &[],
                    };
                    let need: HashSet<String> = fields
                        .iter()
                        .filter(|x| x.ty.is_managed() && !matches!(x.ty, Ty::Opt(_)))
                        .map(|x| x.name.clone())
                        .collect();
                    if !need.is_empty() {
                        fresh.insert(*r, need);
                    }
                }
            }
        }
        for o in b.term.operands() {
            if fresh.contains_key(&o) {
                self.e(format!(
                    "{ctx}: %{} escapes before it is fully initialised",
                    f.value(o).name
                ));
            }
        }
    }

    fn sig_of(&self, callee: &str) -> Option<(Sig, Vec<bool>)> {
        if let Some(f) = self.m.func(callee) {
            return Some((f.sig(), f.params.iter().map(|p| p.own).collect()));
        }
        self.m
            .import(callee)
            .map(|i| (i.sig.clone(), vec![false; i.sig.params.len()]))
    }

    fn check_args(&mut self, f: &Func, ctx: &str, params: &[Ty], args: &[ValueId]) {
        if params.len() != args.len() {
            self.e(format!(
                "{ctx}: expected {} arguments, found {}",
                params.len(),
                args.len()
            ));
            return;
        }
        for (i, (p, a)) in params.iter().zip(args).enumerate() {
            if f.ty(*a) != p {
                self.e(format!(
                    "{ctx}: argument {} has type `{}`, expected `{}`",
                    i + 1,
                    crate::print::ty_text(f.ty(*a)),
                    crate::print::ty_text(p)
                ));
            }
        }
    }

    fn check_results(&mut self, f: &Func, ctx: &str, rets: &[Ty], results: &[ValueId]) {
        let got: Vec<&Ty> = results.iter().map(|r| f.ty(*r)).collect();
        let want: Vec<&Ty> = rets.iter().collect();
        if got != want {
            self.e(format!(
                "{ctx}: result types do not match the callee ({} declared, {} bound)",
                want.len(),
                got.len()
            ));
        }
    }

    fn lookup_fields(&mut self, ty: &str, variant: Option<&str>) -> Option<Vec<Field>> {
        let Some(d) = self.m.type_decl(ty) else {
            self.e(format!("unknown type `{ty}`"));
            return None;
        };
        match (&d.kind, variant) {
            (TypeKind::Struct { fields }, None) => Some(fields.clone()),
            (TypeKind::Closure { captures }, None) => Some(captures.clone()),
            (TypeKind::Union { variants }, Some(v)) => {
                if let Some(x) = variants.iter().find(|x| x.name == v) {
                    Some(x.fields.clone())
                } else {
                    self.e(format!("`{ty}` has no variant `{v}`"));
                    None
                }
            }
            (TypeKind::Union { .. }, None) => {
                self.e(format!(
                    "`{ty}` is a union: name a variant (`{ty}::Variant`)"
                ));
                None
            }
            (_, Some(_)) => {
                self.e(format!("`{ty}` is not a union"));
                None
            }
        }
    }

    fn inst(&mut self, f: &Func, b: &Block, inst: &Inst) {
        let ctx = format!("block `{}`", b.label);
        let ty = |v: &ValueId| f.ty(*v).clone();
        let res = &inst.results;
        let tt = crate::print::ty_text;
        macro_rules! want_results {
            ($n:expr) => {
                if res.len() != $n {
                    self.e(format!(
                        "{ctx}: this instruction defines {} result(s), found {}",
                        $n,
                        res.len()
                    ));
                    return;
                }
            };
        }
        match &inst.op {
            Op::Const(c) => {
                want_results!(1);
                let rt = ty(&res[0]);
                if !rt.is_scalar() || !const_matches(c, &rt) {
                    self.e(format!("{ctx}: constant does not fit type `{}`", tt(&rt)));
                }
            }
            Op::Null => {
                want_results!(1);
                if !matches!(ty(&res[0]), Ty::Opt(_)) {
                    self.e(format!("{ctx}: `null` needs a nullable result type"));
                }
            }
            Op::Bin { op, ovf, a, b: bb } => {
                want_results!(1);
                let (ta, tb, tr) = (ty(a), ty(bb), ty(&res[0]));
                if ta != tb || ta != tr {
                    self.e(format!(
                        "{ctx}: {op:?} needs equal operand and result types"
                    ));
                    return;
                }
                let is_ptr = ta == Ty::Ptr;
                let ok = match op {
                    BinOp::Add | BinOp::Sub if is_ptr => true,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => ta.is_numeric(),
                    BinOp::Rem | BinOp::Shl | BinOp::Shr => ta.is_int(),
                    BinOp::And | BinOp::Or | BinOp::Xor => ta.is_int() || ta == Ty::Bool,
                    BinOp::Min | BinOp::Max => ta.is_float(),
                };
                if !ok {
                    self.e(format!("{ctx}: {:?} is not defined for `{}`", op, tt(&ta)));
                }
                let mode_ok = match ovf {
                    Ovf::Trap => true,
                    Ovf::Wrap => {
                        (ta.is_int() || is_ptr)
                            && !matches!(op, BinOp::And | BinOp::Or | BinOp::Xor)
                    }
                    Ovf::Sat => ta.is_int() && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul),
                };
                if !mode_ok {
                    self.e(format!(
                        "{ctx}: overflow mode {:?} is not allowed for {:?} on `{}`",
                        ovf,
                        op,
                        tt(&ta)
                    ));
                }
            }
            Op::Un { op, ovf, a } => {
                want_results!(1);
                let (ta, tr) = (ty(a), ty(&res[0]));
                if ta != tr {
                    self.e(format!(
                        "{ctx}: {op:?} needs equal operand and result types"
                    ));
                }
                let ok = match op {
                    UnOp::Neg => ta.is_numeric(),
                    UnOp::Not => ta.is_int() || ta == Ty::Bool,
                    UnOp::Abs
                    | UnOp::Sqrt
                    | UnOp::Floor
                    | UnOp::Ceil
                    | UnOp::Trunc
                    | UnOp::Nearest => ta.is_float(),
                    UnOp::Clz | UnOp::Ctz | UnOp::Popcnt => ta.is_int(),
                };
                if !ok {
                    self.e(format!("{ctx}: {:?} is not defined for `{}`", op, tt(&ta)));
                }
                if *ovf != Ovf::Trap && !(*op == UnOp::Neg && *ovf == Ovf::Wrap && ta.is_int()) {
                    self.e(format!(
                        "{ctx}: overflow mode {ovf:?} is not allowed for {op:?}"
                    ));
                }
            }
            Op::Cmp { op, a, b: bb } => {
                want_results!(1);
                let (ta, tb) = (ty(a), ty(bb));
                if ta != tb || ty(&res[0]) != Ty::Bool {
                    self.e(format!(
                        "{ctx}: comparison needs equal operand types and a bool result"
                    ));
                }
                let ordering = !matches!(op, CmpOp::Eq | CmpOp::Ne);
                if ordering && !(ta.is_numeric() || ta == Ty::Ptr) {
                    self.e(format!(
                        "{ctx}: ordering comparison on non-numeric `{}`",
                        tt(&ta)
                    ));
                }
            }
            Op::Conv { ovf, a } => {
                want_results!(1);
                let (from, to) = (ty(a), ty(&res[0]));
                if !from.is_numeric() || !to.is_numeric() || from == to {
                    self.e(format!(
                        "{ctx}: `conv` needs two different numeric types, found `{}` -> `{}`",
                        tt(&from),
                        tt(&to)
                    ));
                    return;
                }
                let ok = match ovf {
                    Ovf::Trap => true,
                    Ovf::Wrap => from.is_int() && to.is_int(),
                    Ovf::Sat => to.is_int(),
                };
                if !ok {
                    self.e(format!(
                        "{ctx}: conversion mode {:?} is not allowed from `{}` to `{}`",
                        ovf,
                        tt(&from),
                        tt(&to)
                    ));
                }
            }
            Op::Bitcast { a } => {
                want_results!(1);
                let (from, to) = (ty(a), ty(&res[0]));
                let int32 = |t: &Ty| matches!(t, Ty::I32 | Ty::U32);
                let ptr_cast = (from == Ty::Ptr && int32(&to)) || (to == Ty::Ptr && int32(&from));
                let num_cast = from.is_numeric()
                    && to.is_numeric()
                    && from.bits() == to.bits()
                    && from.is_float() != to.is_float();
                let ok = num_cast || (ptr_cast && self.opts.allow_raw_memory);
                if !ok {
                    self.e(format!(
                        "{ctx}: bitcast between `{}` and `{}` is not allowed",
                        tt(&from),
                        tt(&to)
                    ));
                }
            }
            Op::Select { cond, a, b: bb } => {
                want_results!(1);
                let t = ty(a);
                if ty(cond) != Ty::Bool || ty(bb) != t || ty(&res[0]) != t || !t.is_scalar() {
                    self.e(format!(
                        "{ctx}: select needs a bool condition and two equal scalar operands"
                    ));
                }
            }
            Op::Alloc(tr) => {
                want_results!(1);
                if self.lookup_fields(&tr.ty, tr.variant.as_deref()).is_some()
                    && ty(&res[0]) != Ty::Ref(tr.ty.clone())
                {
                    self.e(format!("{ctx}: alloc result must be `ref {}`", tr.ty));
                }
            }
            Op::GetField { field, obj } => {
                want_results!(1);
                if ty(obj) != Ty::Ref(field.ty.clone()) {
                    self.e(format!(
                        "{ctx}: getfield on a value of type `{}`, expected `ref {}`",
                        tt(&ty(obj)),
                        field.ty
                    ));
                }
                if let Some(fs) = self.lookup_fields(&field.ty, field.variant.as_deref()) {
                    match fs.iter().find(|x| x.name == field.field) {
                        Some(x) if x.ty == ty(&res[0]) => {}
                        Some(x) => self.e(format!(
                            "{ctx}: field `{}` has type `{}`, result is `{}`",
                            x.name,
                            tt(&x.ty),
                            tt(&ty(&res[0]))
                        )),
                        None => self.e(format!(
                            "{ctx}: no field `{}` in `{}`",
                            field.field, field.ty
                        )),
                    }
                }
            }
            Op::SetField {
                field, obj, val, ..
            } => {
                want_results!(0);
                if ty(obj) != Ty::Ref(field.ty.clone()) {
                    self.e(format!(
                        "{ctx}: setfield on a value of type `{}`, expected `ref {}`",
                        tt(&ty(obj)),
                        field.ty
                    ));
                }
                if let Some(fs) = self.lookup_fields(&field.ty, field.variant.as_deref()) {
                    match fs.iter().find(|x| x.name == field.field) {
                        Some(x) if x.ty == ty(val) => {}
                        Some(x) => self.e(format!(
                            "{ctx}: field `{}` has type `{}`, stored value is `{}`",
                            x.name,
                            tt(&x.ty),
                            tt(&ty(val))
                        )),
                        None => self.e(format!(
                            "{ctx}: no field `{}` in `{}`",
                            field.field, field.ty
                        )),
                    }
                }
            }
            Op::Tag { obj } => {
                want_results!(1);
                let is_union = matches!(ty(obj), Ty::Ref(ref n) if matches!(self.m.type_decl(n).map(|d| &d.kind), Some(TypeKind::Union { .. })));
                if !is_union || ty(&res[0]) != Ty::U32 {
                    self.e(format!(
                        "{ctx}: `tag` needs a union reference and yields u32"
                    ));
                }
            }
            Op::IsUnique { obj } => {
                want_results!(1);
                if !ty(obj).is_managed() || matches!(ty(obj), Ty::Opt(_)) || ty(&res[0]) != Ty::Bool
                {
                    self.e(format!(
                        "{ctx}: `is_unique` needs a non-null managed reference and yields bool"
                    ));
                }
            }
            Op::Clone { obj } => {
                want_results!(1);
                let t = ty(obj);
                if !matches!(t, Ty::Ref(_) | Ty::Array(_)) || ty(&res[0]) != t {
                    self.e(format!("{ctx}: `clone` needs a struct/union/array reference and yields the same type"));
                }
            }
            Op::ArrayNew { len } => {
                want_results!(1);
                match ty(&res[0]) {
                    Ty::Array(e) if ty(len) == Ty::I32 => {
                        if e.is_managed() && !matches!(*e, Ty::Opt(_)) {
                            self.e(format!(
                                "{ctx}: array.new zero-fills, so its element type must be a scalar or nullable (build arrays of non-null references with array.push)"
                            ));
                        }
                    }
                    _ => self.e(format!(
                        "{ctx}: array.new needs an i32 length and yields an array"
                    )),
                }
            }
            Op::ArrayLen { arr } => {
                want_results!(1);
                if !matches!(ty(arr), Ty::Array(_)) || ty(&res[0]) != Ty::I32 {
                    self.e(format!("{ctx}: array.len needs an array and yields i32"));
                }
            }
            Op::ArrayGet { arr, idx } => {
                want_results!(1);
                match ty(arr) {
                    Ty::Array(e) if *e == ty(&res[0]) && ty(idx) == Ty::I32 => {}
                    _ => self.e(format!(
                        "{ctx}: array.get needs an array, an i32 index and yields the element type"
                    )),
                }
            }
            Op::ArraySet { arr, idx, val } => {
                want_results!(0);
                match ty(arr) {
                    Ty::Array(e) if *e == ty(val) && ty(idx) == Ty::I32 => {}
                    _ => self.e(format!(
                        "{ctx}: array.set needs an array, an i32 index and an element value"
                    )),
                }
            }
            Op::ArrayPush { arr, val } => {
                want_results!(0);
                match ty(arr) {
                    Ty::Array(e) if *e == ty(val) => {}
                    _ => self.e(format!(
                        "{ctx}: array.push needs an array and an element value"
                    )),
                }
            }
            Op::StrConst { data } => {
                want_results!(1);
                if !self.m.data.iter().any(|d| &d.name == data) {
                    self.e(format!("{ctx}: unknown data @{data}"));
                }
                if ty(&res[0]) != Ty::Str {
                    self.e(format!("{ctx}: str.const yields str"));
                }
            }
            Op::StrLen { s } => {
                want_results!(1);
                if ty(s) != Ty::Str || ty(&res[0]) != Ty::I32 {
                    self.e(format!("{ctx}: str.len needs a str and yields i32"));
                }
            }
            Op::StrFromUtf8 { bytes, len } => {
                want_results!(1);
                if ty(bytes) != Ty::Array(Box::new(Ty::U8))
                    || ty(len) != Ty::I32
                    || ty(&res[0]) != Ty::Str
                {
                    self.e(format!(
                        "{ctx}: str.from_utf8 needs a [u8] and an i32 length and yields str"
                    ));
                }
            }
            Op::IsNull { val } => {
                want_results!(1);
                if !matches!(ty(val), Ty::Opt(_)) || ty(&res[0]) != Ty::Bool {
                    self.e(format!(
                        "{ctx}: is_null needs a nullable value and yields bool"
                    ));
                }
            }
            Op::Opt { val } => {
                want_results!(1);
                if ty(&res[0]) != Ty::Opt(Box::new(ty(val))) {
                    self.e(format!(
                        "{ctx}: `opt` result must be the nullable form of its operand"
                    ));
                }
            }
            Op::Unopt { val } => {
                want_results!(1);
                match ty(val) {
                    Ty::Opt(inner) if *inner == ty(&res[0]) => {}
                    _ => self.e(format!(
                        "{ctx}: `unopt` needs a nullable value and yields its inner type"
                    )),
                }
            }
            Op::ClosureNew {
                env,
                func,
                captures,
            } => {
                want_results!(1);
                let Some(d) = self.m.type_decl(env) else {
                    self.e(format!("{ctx}: unknown env type `{env}`"));
                    return;
                };
                let TypeKind::Closure { captures: caps } = &d.kind else {
                    self.e(format!("{ctx}: `{env}` is not a closure environment type"));
                    return;
                };
                let caps = caps.clone();
                let Some(target) = self.m.func(func) else {
                    self.e(format!("{ctx}: unknown function @{func}"));
                    return;
                };
                if target.params.iter().any(|p| p.own) {
                    self.e(format!("{ctx}: @{func} is used as a closure body and must not have `own` parameters"));
                }
                let tsig = target.sig();
                if tsig.rets.len() > 1 || tsig.params.first() != Some(&Ty::Ref(env.clone())) {
                    self.e(format!(
                        "{ctx}: @{func} must take `ref {env}` first and return at most one value"
                    ));
                    return;
                }
                let expected = Ty::Fn(Box::new(Sig {
                    params: tsig.params[1..].to_vec(),
                    rets: tsig.rets.clone(),
                }));
                if ty(&res[0]) != expected {
                    self.e(format!(
                        "{ctx}: closure.new result must be `{}`",
                        tt(&expected)
                    ));
                }
                let cap_tys: Vec<Ty> = caps.iter().map(|c| c.ty.clone()).collect();
                self.check_args(
                    f,
                    &format!("{ctx}: closure.new captures"),
                    &cap_tys,
                    captures,
                );
            }
            Op::Call { callee, args } => {
                let Some((sig, _)) = self.sig_of(callee) else {
                    self.e(format!("{ctx}: unknown callee @{callee}"));
                    return;
                };
                self.check_args(f, &format!("{ctx}: call @{callee}"), &sig.params, args);
                self.check_results(f, &format!("{ctx}: call @{callee}"), &sig.rets, res);
            }
            Op::CallClosure { callee, args } => match ty(callee) {
                Ty::Fn(sig) => {
                    self.check_args(f, &format!("{ctx}: call_closure"), &sig.params, args);
                    self.check_results(f, &format!("{ctx}: call_closure"), &sig.rets, res);
                }
                other => self.e(format!("{ctx}: call_closure on `{}`", tt(&other))),
            },
            Op::Intrinsic { name, args } => {
                if !self.opts.allow_raw_memory {
                    self.e(format!(
                        "{ctx}: `intrinsic` is only allowed in trusted raw-memory modules"
                    ));
                }
                match oxiscript_abi::exports::find(name) {
                    Some(spec) if matches!(spec.kind, oxiscript_abi::exports::ExportKind::Func) => {
                        let got: Vec<WasmTy> = args.iter().map(|a| wasm_class(f.ty(*a))).collect();
                        if got != spec.params {
                            self.e(format!("{ctx}: intrinsic {name}: operand types do not match the ABI signature"));
                        }
                        let got: Vec<WasmTy> = res.iter().map(|r| wasm_class(f.ty(*r))).collect();
                        if got != spec.results {
                            self.e(format!("{ctx}: intrinsic {name}: result types do not match the ABI signature"));
                        }
                    }
                    _ => self.e(format!(
                        "{ctx}: `{name}` is not a runtime function of the ABI"
                    )),
                }
            }
            Op::Retain { val } | Op::Release { val } => {
                want_results!(0);
                if !ty(val).is_managed() {
                    self.e(format!(
                        "{ctx}: retain/release need a managed reference, found `{}`",
                        tt(&ty(val))
                    ));
                }
            }
            Op::GlobalGet { name } => {
                want_results!(1);
                match self.m.global(name) {
                    Some(g) if g.ty == ty(&res[0]) => {}
                    Some(_) => self.e(format!("{ctx}: global.get @{name}: result type mismatch")),
                    None => self.e(format!("{ctx}: unknown global @{name}")),
                }
            }
            Op::GlobalSet { name, val } => {
                want_results!(0);
                match self.m.global(name) {
                    Some(g) if g.ty == ty(val) => {}
                    Some(_) => self.e(format!("{ctx}: global.set @{name}: value type mismatch")),
                    None => self.e(format!("{ctx}: unknown global @{name}")),
                }
            }
            Op::Load { ptr, .. } => {
                want_results!(1);
                if !self.opts.allow_raw_memory {
                    self.e(format!(
                        "{ctx}: `load` is only allowed in trusted raw-memory modules"
                    ));
                }
                if ty(ptr) != Ty::Ptr || !ty(&res[0]).is_scalar() {
                    self.e(format!("{ctx}: load needs a ptr and yields a scalar"));
                }
            }
            Op::Store { ptr, val, .. } => {
                want_results!(0);
                if !self.opts.allow_raw_memory {
                    self.e(format!(
                        "{ctx}: `store` is only allowed in trusted raw-memory modules"
                    ));
                }
                if ty(ptr) != Ty::Ptr || !ty(val).is_scalar() {
                    self.e(format!("{ctx}: store needs a ptr and a scalar value"));
                }
            }
        }
    }

    fn terminator(&mut self, f: &Func, b: &Block) {
        let ctx = format!("block `{}`", b.label);
        let n = f.blocks.len();
        for t in b.term.targets() {
            if (t.block.0 as usize) >= n {
                continue; // reported earlier
            }
            let dest = f.block(t.block);
            let want: Vec<Ty> = dest.params.iter().map(|p| f.ty(*p).clone()).collect();
            self.check_args(
                f,
                &format!("{ctx}: branch to `{}`", dest.label),
                &want,
                &t.args,
            );
        }
        match &b.term {
            Terminator::BrIf { cond, .. } => {
                if f.ty(*cond) != &Ty::Bool {
                    self.e(format!("{ctx}: br_if condition must be bool"));
                }
            }
            Terminator::Switch { value, cases, .. } => {
                if !f.ty(*value).is_int() {
                    self.e(format!("{ctx}: switch value must be an integer"));
                }
                let mut seen = HashSet::new();
                for (k, _) in cases {
                    if !seen.insert(*k) {
                        self.e(format!("{ctx}: duplicate switch case {k}"));
                    }
                    if !int_fits(f.ty(*value), i128::from(*k)) {
                        self.e(format!(
                            "{ctx}: switch case {k} does not fit the scrutinee type"
                        ));
                    }
                }
            }
            Terminator::Ret(vs) => {
                let got: Vec<&Ty> = vs.iter().map(|v| f.ty(*v)).collect();
                let want: Vec<&Ty> = f.rets.iter().collect();
                if got != want {
                    self.e(format!(
                        "{ctx}: ret does not match the function's result types"
                    ));
                }
            }
            _ => {}
        }
    }
}

fn const_matches(c: &Const, t: &Ty) -> bool {
    match (c, t) {
        (Const::Bool(_), Ty::Bool) => true,
        (Const::Int(v), t) if t.is_int() || *t == Ty::Ptr => int_fits(t, *v),
        (Const::F32(_), Ty::F32) | (Const::F64(_), Ty::F64) => true,
        _ => false,
    }
}

/// Bounded-recursion depth test (at most `MAX_TYPE_DEPTH + 1` frames).
fn too_deep(t: &Ty, d: u32) -> bool {
    if d > MAX_TYPE_DEPTH {
        return true;
    }
    match t {
        Ty::Array(e) | Ty::Opt(e) => too_deep(e, d + 1),
        Ty::Fn(s) => s
            .params
            .iter()
            .chain(s.rets.iter())
            .any(|x| too_deep(x, d + 1)),
        _ => false,
    }
}

/// `[A-Za-z0-9_.]+`: value, symbol and attribute names (what the text lexer reads after `%`, `@`, `#`).
fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// `[A-Za-z_][A-Za-z0-9_.]*`: type names and block labels.
fn valid_ident(s: &str) -> bool {
    valid_name(s) && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

/// Field and variant names: identifiers without dots (type names may contain dots, so the last dot splits `Type.field`).
fn valid_field_name(s: &str) -> bool {
    valid_ident(s) && !s.contains('.')
}
