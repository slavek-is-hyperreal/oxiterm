//! Declaration collection, imports, signature resolution (lazy, with cycle detection).

use crate::flow::{full_mask, Flow};
use crate::ty::*;
use crate::*;
use oxiscript_abi::idl::{IdlRet, IdlTy};
use oxiscript_abi::layout::Prim;

pub(crate) struct SavedEnv {
    cm: usize,
    scopes: Vec<HashMap<String, Binding>>,
    fnctx: Vec<FnCtx>,
    flow: Flow,
    depth: u32,
    swapped: bool,
}

const KNOWN_ATTRS: [&str; 7] = ["expect", "profile", "allow", "inline", "noinline", "export_name", "host"];

impl<'a> Checker<'a> {
    // ---- environments -----------------------------------------------------

    /// Run `f` in the environment of definition `d`: module-level scope for
    /// top-level definitions (so lazily resolving a signature never sees the
    /// caller's locals), the current scopes for nested ones; plus `d`'s type parameters.
    pub fn with_def_env<R>(&mut self, d: DefId, f: impl FnOnce(&mut Self) -> R) -> R {
        let info = &self.defs[d.0 as usize].info;
        let local = info.local;
        let module = info.module as usize;
        let tps = info.tparams.clone();
        let saved = if local {
            None
        } else {
            Some(SavedEnv {
                cm: self.cm,
                scopes: std::mem::take(&mut self.scopes),
                fnctx: std::mem::take(&mut self.fnctx),
                flow: std::mem::take(&mut self.flow),
                depth: self.depth,
                swapped: true,
            })
        };
        if !local {
            // the depth counter is deliberately NOT reset: nested on-demand resolutions share
            // one recursion budget, so no chain of declarations can multiply the native stack use
            self.cm = module;
        }
        let mut sc = HashMap::new();
        for p in &tps {
            sc.insert(self.params[p.0 as usize].name.clone(), Binding::TParam(*p));
        }
        self.scopes.push(sc);
        let outer_sig = if local {
            self.local_sig.push(d);
            None
        } else {
            Some(std::mem::take(&mut self.local_sig))
        };
        let r = f(self);
        match outer_sig {
            None => {
                self.local_sig.pop();
            }
            Some(v) => self.local_sig = v,
        }
        self.scopes.pop();
        if let Some(s) = saved {
            debug_assert!(s.swapped);
            self.cm = s.cm;
            self.scopes = s.scopes;
            self.fnctx = s.fnctx;
            self.flow = s.flow;
            self.depth = s.depth;
        }
        r
    }

    // ---- module declaration -----------------------------------------------

    pub fn declare_module(&mut self, k: usize) {
        let Some(ast) = self.mods[k].ast else { return };
        self.check_attrs(&ast.attrs, "module", ast.id);
        for item in &ast.items {
            self.declare_item(item);
        }
    }

    pub fn resolve_module(&mut self, k: usize) {
        let items = self.mods[k].out.items.clone();
        for d in items {
            self.ensure_def(d);
        }
        self.flush_bounds();
    }

    pub fn ensure_def(&mut self, d: DefId) {
        match self.kind_of(d) {
            DefKind::Struct | DefKind::Class | DefKind::Variant { .. } => self.ensure_record(d),
            DefKind::Union => self.ensure_union(d),
            DefKind::Enum => self.ensure_enum(d),
            DefKind::Interface => self.ensure_iface(d),
            DefKind::Alias => self.ensure_alias(d),
            DefKind::Fn | DefKind::Method | DefKind::Ctor => self.ensure_fn(d),
            DefKind::Global => self.ensure_global(d),
            _ => {}
        }
    }

    fn bind_item(&mut self, name: &Ident, d: DefId, exported: bool, local: bool) {
        let type_item = matches!(self.kind_of(d), DefKind::Struct | DefKind::Class | DefKind::Enum | DefKind::Interface | DefKind::Alias | DefKind::Union);
        if type_item && (matches!(name.name.as_str(), "bool" | "string" | "void" | "never" | "Array") || NumTy::from_name(&name.name).is_some()) {
            self.err("OXS-E0123", name.span, &[("detail", &format!("`{}` is the name of a builtin type and cannot be redeclared", name.name))]);
        }
        if local {
            self.bind_scope(&name.name, Binding::Def(d), name.span);
            return;
        }
        let m = &mut self.mods[self.cm];
        if m.top.contains_key(&name.name) {
            self.err("OXS-E0301", name.span, &[("name", &name.name)]);
            return;
        }
        m.top.insert(name.name.clone(), Binding::Def(d));
        if exported {
            m.exports.insert(name.name.clone(), d);
        }
        m.out.items.push(d);
    }

    pub fn new_tparams(&mut self, tps: &'a [TypeParam], owner: DefId, base: &[ParamId]) -> Vec<ParamId> {
        let mut out: Vec<ParamId> = base.to_vec();
        let mut seen: HashSet<String> = base.iter().map(|p| self.params[p.0 as usize].name.clone()).collect();
        for (i, tp) in tps.iter().enumerate() {
            if !seen.insert(tp.name.name.clone()) {
                self.err("OXS-E0301", tp.name.span, &[("name", &tp.name.name)]);
            }
            let p = self.new_param(&tp.name.name, Some(owner), (base.len() + i) as u32);
            out.push(p);
        }
        out
    }

    fn alias_kind(t: &TypeExpr) -> Option<bool> {
        // Some(false): object alias (variant), Some(true): union alias, None: plain alias
        match &t.kind {
            TypeExprKind::Paren(i) => Self::alias_kind(i),
            TypeExprKind::Object(_) => Some(false),
            TypeExprKind::Union(ms) => {
                if ms.iter().any(|m| matches!(m.kind, TypeExprKind::Null)) {
                    None
                } else {
                    Some(true)
                }
            }
            _ => None,
        }
    }

    pub fn declare_item(&mut self, item: &'a Item) -> Option<DefId> {
        let local = !self.fnctx.is_empty();
        let ex = item.export;
        if local && ex {
            self.err("OXS-E0123", item.span, &[("detail", "nested declarations cannot be exported")]);
        }
        self.check_attrs(&item.attrs, "item", item.id);
        self.check_host_placement(item, local);
        let id = item.id;
        let sp = item.span;
        let d = match &item.kind {
            ItemKind::Import(imp) => {
                if local {
                    self.err("OXS-E0123", item.span, &[("detail", "`import` is only allowed at module level")]);
                } else {
                    self.declare_import(imp);
                }
                return None;
            }
            ItemKind::Var(v) => {
                let PatternKind::Ident(name) = &v.pattern.kind else {
                    self.err("OXS-E0115", v.pattern.span, &[("detail", "destructuring is not supported at module level")]);
                    return None;
                };
                let d = self.new_def(&name.name, DefKind::Global, sp, id, ex, vec![], None, Ast::Item(item));
                self.bind_item(name, d, ex, local);
                d
            }
            ItemKind::Fn(f) => {
                let d = self.new_def(&f.name.name, DefKind::Fn, sp, id, ex, vec![], None, Ast::Item(item));
                let tps = self.new_tparams(&f.type_params, d, &[]);
                self.defs[d.0 as usize].info.tparams = tps;
                self.reserved_fn(f);
                self.bind_item(&f.name, d, ex, local);
                d
            }
            ItemKind::Record(r) => {
                let kind = if r.kind == RecordKind::Struct { DefKind::Struct } else { DefKind::Class };
                let d = self.new_def(&r.name.name, kind, sp, id, ex, vec![], None, Ast::Item(item));
                let tps = self.new_tparams(&r.type_params, d, &[]);
                self.defs[d.0 as usize].info.tparams = tps.clone();
                for m in &r.members {
                    self.check_attrs(&m.attrs, "member", m.id);
                    match &m.kind {
                        MemberKind::Method(f) | MemberKind::Constructor(f) => {
                            let ck = if matches!(m.kind, MemberKind::Method(_)) { DefKind::Method } else { DefKind::Ctor };
                            let md = self.new_def(&f.name.name, ck, m.span, m.id, false, vec![], Some(d), Ast::Member(m, r));
                            if ck == DefKind::Ctor && (!f.type_params.is_empty() || f.mods.is_static) || ck == DefKind::Method && f.name.name == "constructor" {
                                self.err("OXS-E0123", f.name.span, &[("detail", "a constructor cannot be generic or `static`, and `constructor` is not a method name")]);
                            }
                            let own = self.new_tparams(&f.type_params, md, &tps);
                            self.defs[md.0 as usize].info.tparams = own;
                            self.reserved_fn(f);
                        }
                        MemberKind::Field(_) => {}
                    }
                }
                self.bind_item(&r.name, d, ex, local);
                d
            }
            ItemKind::Interface(i) => {
                let d = self.new_def(&i.name.name, DefKind::Interface, sp, id, ex, vec![], None, Ast::Item(item));
                let tps = self.new_tparams(&i.type_params, d, &[]);
                self.defs[d.0 as usize].info.tparams = tps;
                self.bind_item(&i.name, d, ex, local);
                d
            }
            ItemKind::Enum(e) => {
                let d = self.new_def(&e.name.name, DefKind::Enum, sp, id, ex, vec![], None, Ast::Item(item));
                self.bind_item(&e.name, d, ex, local);
                d
            }
            ItemKind::TypeAlias(a) => {
                let kind = match Self::alias_kind(&a.ty) {
                    Some(false) => DefKind::Variant { union: None, index: 0 },
                    Some(true) => DefKind::Union,
                    None => DefKind::Alias,
                };
                let d = self.new_def(&a.name.name, kind, sp, id, ex, vec![], None, Ast::Item(item));
                let tps = self.new_tparams(&a.type_params, d, &[]);
                self.defs[d.0 as usize].info.tparams = tps;
                self.bind_item(&a.name, d, ex, local);
                d
            }
            ItemKind::Law(l) => {
                if local {
                    self.err("OXS-E0125", item.span, &[("detail", "laws are only allowed at module level")]);
                    return None;
                }
                let d = self.new_def(&l.name.name, DefKind::Law, sp, id, ex, vec![], None, Ast::Item(item));
                let tps = self.new_tparams(&l.type_params, d, &[]);
                self.defs[d.0 as usize].info.tparams = tps;
                self.bind_item(&l.name, d, ex, local);
                d
            }
        };
        Some(d)
    }

    fn reserved_fn(&mut self, f: &'a FnDecl) {
        let kw = if f.mods.kernel {
            Some("kernel")
        } else if f.mods.is_async {
            Some("async")
        } else if !f.contracts.is_empty() {
            Some("requires")
        } else if f.params.iter().any(|p| p.mode == ParamMode::Inout) {
            Some("inout")
        } else {
            None
        };
        if let Some(k) = kw {
            self.err("OXS-E0900", f.name.span, &[("keyword", k)]);
        }
    }

    // ---- attributes ---------------------------------------------------------

    /// Whether the function definition carries `@host` (recorded by `check_attrs`).
    pub fn is_host_fn(&self, d: DefId) -> bool {
        let info = &self.defs[d.0 as usize].info;
        self.mods
            .get(info.module as usize)
            .and_then(|m| m.out.attrs.get(&info.node))
            .is_some_and(|v| v.iter().any(|(n, _)| n == "host"))
    }

    /// `@host` is legal only on an exported, non-generic, module-level function of the entry module.
    fn check_host_placement(&mut self, item: &'a Item, local: bool) {
        let Some(a) = item.attrs.iter().find(|a| !a.inner && a.name.name == "host") else { return };
        let detail = match &item.kind {
            ItemKind::Fn(f) => {
                if local {
                    "`@host` cannot be applied to a nested function"
                } else if !item.export {
                    "`@host` needs `export`: `@host export function f(...)`"
                } else if self.cm != 0 {
                    "`@host` is only allowed in the entry module; other modules cannot expose Wasm exports"
                } else if !f.type_params.is_empty() {
                    "a `@host` function cannot be generic"
                } else if f.name.name == "main" {
                    "`main` is the entry point and needs no `@host`"
                } else {
                    return;
                }
            }
            _ => "`@host` applies only to an exported function of the entry module",
        };
        self.err("OXS-E0121", a.span, &[("detail", detail)]);
    }

    pub fn check_attrs(&mut self, attrs: &'a [Attribute], what: &str, node: NodeId) {
        let mut rec: Vec<(String, Vec<String>)> = Vec::new();
        for a in attrs {
            let n = a.name.name.as_str();
            if !KNOWN_ATTRS.contains(&n) {
                self.err("OXS-E0124", a.name.span, &[("name", n)]);
                continue;
            }
            if a.inner && n != "profile" {
                self.err(
                    "OXS-E0123",
                    a.span,
                    &[("detail", &format!("`@@{n}` cannot be applied to a whole module; only `@@profile(...)` can"))],
                );
            }
            if !a.inner && what == "module" {
                continue;
            }
            let args: Vec<String> = a
                .args
                .iter()
                .map(|x| match &x.value {
                    AttrValue::Lit(l) => match &l.kind {
                        LitKind::Str(s) => s.clone(),
                        LitKind::Int { value, .. } => value.to_string(),
                        LitKind::Bool(b) => b.to_string(),
                        _ => String::new(),
                    },
                    AttrValue::Path(p) => p.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join("."),
                })
                .collect();
            match n {
                "profile" => {
                    let ok = a.args.len() == 1
                        && matches!(args[0].as_str(), "default" | "realtime" | "safety");
                    if a.args.len() == 1 && args[0] == "kernel" {
                        self.err("OXS-E0900", a.span, &[("keyword", "kernel")]);
                    } else if !ok {
                        self.err("OXS-E0123", a.span, &[("detail", "`@profile` takes one of `default`, `realtime`, `safety`")]);
                    }
                }
                "allow" => {
                    let code = args.first().cloned().unwrap_or_default();
                    if a.args.is_empty() {
                        self.err("OXS-E0123", a.span, &[("detail", "`@allow` needs a code and a reason: `@allow(\"OXS-P0102\", \"why\")`")]);
                    } else if !crate::registry().contains_key(code.as_str()) {
                        self.err("OXS-E0123", a.span, &[("detail", &format!("`@allow` names `{code}`, which is not a registered reason code"))]);
                    } else if a.args.len() < 2 || args[1].is_empty() {
                        self.err("OXS-W0002", a.span, &[("code", &code)]);
                    }
                }
                "export_name" => {
                    if a.args.len() != 1 || !matches!(&a.args[0].value, AttrValue::Lit(l) if matches!(l.kind, LitKind::Str(_))) {
                        self.err("OXS-E0123", a.span, &[("detail", "`@export_name` takes one string: `@export_name(\"init\")`")]);
                    } else if args[0].is_empty() || !args[0].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                        self.err("OXS-E0123", a.span, &[("detail", "`@export_name` must be a non-empty name of ASCII letters, digits and `_`")]);
                    }
                }
                "host" => {
                    if !a.args.is_empty() {
                        self.err("OXS-E0123", a.span, &[("detail", "`@host` takes no arguments; the Wasm name comes from `@export_name(\"x\")`")]);
                    } else if what != "item" {
                        self.err("OXS-E0121", a.span, &[("detail", "`@host` applies only to an exported function of the entry module: `@host export function f(...)`")]);
                    }
                }
                "inline" | "noinline" => {
                    if !a.args.is_empty() {
                        self.err("OXS-E0123", a.span, &[("detail", &format!("`@{n}` takes no arguments"))]);
                    }
                }
                _ => {}
            }
            rec.push((n.to_string(), args));
        }
        if let Some(a) = attrs.iter().find(|a| !a.inner && a.name.name == "export_name") {
            if !attrs.iter().any(|b| !b.inner && b.name.name == "host") {
                self.err("OXS-E0123", a.span, &[("detail", "`@export_name` only applies to a host-callable function: add `@host`")]);
            }
        }
        if !rec.is_empty() {
            self.mods[self.cm].out.attrs.insert(node, rec);
        }
    }

    // ---- imports -----------------------------------------------------------

    fn declare_import(&mut self, imp: &'a ImportDecl) {
        let LitKind::Str(spec) = &imp.source.kind else { return };
        let is_host = spec.starts_with("host:");
        let target = if is_host { None } else { self.mods[self.cm].targets.get(spec).copied() };
        if !is_host && target.is_none() {
            // loader already reported E0302
            return;
        }
        match &imp.clause {
            ImportClause::Namespace(id) => {
                let Some(t) = target else {
                    self.err("OXS-E0115", id.span, &[("detail", "namespace imports of host modules are not supported; import the interface by name")]);
                    return;
                };
                let d = self.new_def(&id.name, DefKind::Namespace, id.span, NodeId(u32::MAX), false, vec![], None, Ast::None);
                self.defs[d.0 as usize].info.target_module = Some(t as u32);
                self.defs[d.0 as usize].sig = Sig::Done;
                self.bind_import(&id.name, d, id.span);
            }
            ImportClause::Named(specs) => {
                for s in specs {
                    let local_name = s.alias.as_ref().unwrap_or(&s.name);
                    let found = if is_host {
                        self.host_member(spec, &s.name)
                    } else {
                        let t = target.unwrap_or(0);
                        let r = self.mods[t].exports.get(&s.name.name).copied();
                        if r.is_none() {
                            self.err(
                                "OXS-E0302",
                                s.name.span,
                                &[("spec", spec), ("detail", &format!("module has no exported member `{}`", s.name.name))],
                            );
                        }
                        r
                    };
                    if let Some(d) = found {
                        self.bind_import(&local_name.name, d, local_name.span);
                    }
                }
            }
        }
    }

    fn bind_import(&mut self, name: &str, d: DefId, span: Span) {
        let m = &mut self.mods[self.cm];
        if m.top.contains_key(name) {
            self.err("OXS-E0301", span, &[("name", name)]);
            return;
        }
        m.top.insert(name.to_string(), Binding::Def(d));
    }

    fn result_def(&self) -> Option<DefId> {
        match self.mods[self.prelude].top.get("Result") {
            Some(Binding::Def(d)) => Some(*d),
            _ => None,
        }
    }

    fn idl_ty(&mut self, t: &IdlTy, host: &str) -> Ty {
        match t {
            IdlTy::Prim(p) => match p {
                Prim::Bool => BOOL,
                Prim::I8 => num(NumTy::I8),
                Prim::U8 => num(NumTy::U8),
                Prim::I16 => num(NumTy::I16),
                Prim::U16 => num(NumTy::U16),
                Prim::I32 => num(NumTy::I32),
                Prim::U32 => num(NumTy::U32),
                Prim::I64 => num(NumTy::I64),
                Prim::U64 => num(NumTy::U64),
                Prim::F32 => num(NumTy::F32),
                Prim::F64 => num(NumTy::F64),
                Prim::Ptr => ERROR,
            },
            IdlTy::Str => STR,
            IdlTy::Bytes => self.tys.array(U8),
            IdlTy::Handle(n) | IdlTy::Callback(n) => {
                let key = format!("{host}#{n}");
                match self.host_defs.get(&key).copied() {
                    Some(d) => {
                        if self.kind_of(d) == DefKind::Handle {
                            self.tys.adt(d, &[])
                        } else {
                            self.aliases.get(&d).copied().unwrap_or(ERROR)
                        }
                    }
                    None => ERROR,
                }
            }
        }
    }

    /// Resolve `import { name } from "host:..."`; builds the host's definitions on first use.
    fn host_member(&mut self, spec: &str, name: &Ident) -> Option<DefId> {
        let Some(idx) = self.host_idl.iter().position(|i| i.module == spec) else {
            self.err("OXS-E0400", name.span, &[("module", spec)]);
            return None;
        };
        let iface = &self.host_idl[idx];
        let key = format!("{}#{}", iface.module, name.name);
        if !self.host_defs.contains_key(&format!("{}#{}", iface.module, iface.name)) {
            self.build_host(idx);
        }
        if let Some(d) = self.host_defs.get(&key) { Some(*d) } else {
            self.err("OXS-E0401", name.span, &[("name", &name.name), ("module", spec)]);
            None
        }
    }

    fn build_host(&mut self, idx: usize) {
        let iface = &self.host_idl[idx];
        let module = iface.module.clone();
        let sp = Span::DUMMY;
        let nid = NodeId(u32::MAX);
        let hd = self.new_def(&iface.name, DefKind::Host, sp, nid, true, vec![], None, Ast::None);
        self.defs[hd.0 as usize].sig = Sig::Done;
        self.host_defs.insert(format!("{module}#{}", iface.name), hd);
        let mut handles = Vec::new();
        for h in &iface.handles {
            let d = self.new_def(h, DefKind::Handle, sp, nid, true, vec![], Some(hd), Ast::None);
            self.defs[d.0 as usize].sig = Sig::Done;
            self.host_defs.insert(format!("{module}#{h}"), d);
            handles.push((h.clone(), d));
        }
        // callbacks: alias defs whose target is a function type
        let mut callbacks = Vec::new();
        for c in &iface.callbacks {
            let d = self.new_def(&c.name, DefKind::Alias, sp, nid, true, vec![], Some(hd), Ast::None);
            self.defs[d.0 as usize].sig = Sig::Done;
            self.host_defs.insert(format!("{module}#{}", c.name), d);
            let ps: Vec<Ty> = c.params.iter().map(|p| self.idl_ty(&p.ty, &module)).collect();
            let r = match c.ret {
                None => VOID,
                Some(p) => self.idl_ty(&IdlTy::Prim(p), &module),
            };
            let ft = self.tys.func(&ps, r);
            self.aliases.insert(d, ft);
            callbacks.push((c.name.clone(), ft));
        }
        let result = self.result_def();
        let mut fns = Vec::new();
        let iface = &self.host_idl[idx];
        let fdecls: Vec<_> = iface.fns.clone();
        let version = iface.version;
        for f in &fdecls {
            let ps: Vec<Ty> = f.params.iter().map(|p| self.idl_ty(&p.ty, &module)).collect();
            let r = match &f.ret {
                IdlRet::Void => VOID,
                IdlRet::Value(t) => self.idl_ty(t, &module),
                IdlRet::Result(t) => {
                    let ok = t.as_ref().map_or(VOID, |t| self.idl_ty(t, &module));
                    match result {
                        Some(rd) => self.tys.adt(rd, &[ok, I32]),
                        None => ERROR,
                    }
                }
            };
            let ft = self.tys.func(&ps, r);
            fns.push((f.name.clone(), ft));
        }
        self.hosts.insert(hd, HostSig { module: module.clone(), version, fns, handles, callbacks });
    }

    // ---- aliases -------------------------------------------------------------

    /// Declarations are resolved on demand, so a chain of forward references recurses once per
    /// declaration; beyond [`MAX_SIG_DEPTH`] levels the definition is given up (`OXS-E0113`).
    fn sig_overflow(&mut self, d: DefId) {
        let info = &self.defs[d.0 as usize].info;
        let (sp, kind, tparams, module) = (info.span, info.kind, info.tparams.clone(), info.module);
        let _ = module;
        self.err("OXS-E0113", sp, &[("detail", "declarations depend on each other through too long a chain of forward references; declare them in dependency order")]);
        self.defs[d.0 as usize].sig = Sig::Done;
        // placeholder signatures so later lookups of this definition find something (never panic)
        match kind {
            DefKind::Alias => {
                self.aliases.insert(d, ERROR);
            }
            DefKind::Fn | DefKind::Method | DefKind::Ctor | DefKind::Law => {
                self.fns.insert(d, FnSig { params: vec![], ret: ERROR, tparams, mutating: false, is_static: false, private: false });
            }
            DefKind::Global => {
                self.globals.insert(d, GlobalSig { ty: ERROR, mutable: false });
            }
            DefKind::Interface => {
                self.ifaces.insert(d, IfaceSig { methods: vec![] });
            }
            DefKind::Struct | DefKind::Class | DefKind::Variant { .. } => {
                self.records.insert(d, RecordSig { is_class: kind == DefKind::Class, fields: vec![], methods: vec![], ctor: None, implements: vec![], discr: None });
            }
            DefKind::Union => {
                self.unions.insert(d, UnionSig { discr_field: "kind".into(), discr_ty: ERROR, variants: vec![] });
            }
            DefKind::Enum => {
                self.enums.insert(d, EnumSig { repr: NumTy::I32, members: vec![] });
            }
            _ => {}
        }
    }

    pub fn ensure_alias(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return self.ensure_alias_inner(d);
        }
        if self.sig_depth >= MAX_SIG_DEPTH || self.depth + SIG_COST > MAX_EXPR_DEPTH {
            return self.sig_overflow(d);
        }
        self.sig_depth += 1;
        self.depth += SIG_COST;
        self.ensure_alias_inner(d);
        self.depth -= SIG_COST;
        self.sig_depth -= 1;
    }

    pub fn ensure_iface(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return self.ensure_iface_inner(d);
        }
        if self.sig_depth >= MAX_SIG_DEPTH || self.depth + SIG_COST > MAX_EXPR_DEPTH {
            return self.sig_overflow(d);
        }
        self.sig_depth += 1;
        self.depth += SIG_COST;
        self.ensure_iface_inner(d);
        self.depth -= SIG_COST;
        self.sig_depth -= 1;
    }

    pub fn ensure_record(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return self.ensure_record_inner(d);
        }
        if self.sig_depth >= MAX_SIG_DEPTH || self.depth + SIG_COST > MAX_EXPR_DEPTH {
            return self.sig_overflow(d);
        }
        self.sig_depth += 1;
        self.depth += SIG_COST;
        self.ensure_record_inner(d);
        self.depth -= SIG_COST;
        self.sig_depth -= 1;
    }

    pub fn ensure_variant(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return self.ensure_variant_inner(d);
        }
        if self.sig_depth >= MAX_SIG_DEPTH || self.depth + SIG_COST > MAX_EXPR_DEPTH {
            return self.sig_overflow(d);
        }
        self.sig_depth += 1;
        self.depth += SIG_COST;
        self.ensure_variant_inner(d);
        self.depth -= SIG_COST;
        self.sig_depth -= 1;
    }

    pub fn ensure_union(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return self.ensure_union_inner(d);
        }
        if self.sig_depth >= MAX_SIG_DEPTH || self.depth + SIG_COST > MAX_EXPR_DEPTH {
            return self.sig_overflow(d);
        }
        self.sig_depth += 1;
        self.depth += SIG_COST;
        self.ensure_union_inner(d);
        self.depth -= SIG_COST;
        self.sig_depth -= 1;
    }

    pub fn ensure_fn(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return self.ensure_fn_inner(d);
        }
        if self.sig_depth >= MAX_SIG_DEPTH || self.depth + SIG_COST > MAX_EXPR_DEPTH {
            return self.sig_overflow(d);
        }
        self.sig_depth += 1;
        self.depth += SIG_COST;
        self.ensure_fn_inner(d);
        self.depth -= SIG_COST;
        self.sig_depth -= 1;
    }

    pub fn ensure_global(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return self.ensure_global_inner(d);
        }
        if self.sig_depth >= MAX_SIG_DEPTH || self.depth + SIG_COST > MAX_EXPR_DEPTH {
            return self.sig_overflow(d);
        }
        self.sig_depth += 1;
        self.depth += SIG_COST;
        self.ensure_global_inner(d);
        self.depth -= SIG_COST;
        self.sig_depth -= 1;
    }

    pub fn ensure_enum(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return self.ensure_enum_inner(d);
        }
        if self.sig_depth >= MAX_SIG_DEPTH || self.depth + SIG_COST > MAX_EXPR_DEPTH {
            return self.sig_overflow(d);
        }
        self.sig_depth += 1;
        self.depth += SIG_COST;
        self.ensure_enum_inner(d);
        self.depth -= SIG_COST;
        self.sig_depth -= 1;
    }

    fn ensure_alias_inner(&mut self, d: DefId) {
        match self.defs[d.0 as usize].sig {
            Sig::Done => return,
            Sig::InProgress => {
                let sp = self.defs[d.0 as usize].info.span;
                let n = self.def_name(d).to_string();
                self.err("OXS-E0123", sp, &[("detail", &format!("Type alias '{n}' circularly references itself"))]);
                self.aliases.insert(d, ERROR);
                self.defs[d.0 as usize].sig = Sig::Done;
                return;
            }
            Sig::Pending => {}
        }
        let Ast::Item(item) = self.defs[d.0 as usize].ast else {
            self.defs[d.0 as usize].sig = Sig::Done;
            return;
        };
        let ItemKind::TypeAlias(a) = &item.kind else { return };
        self.defs[d.0 as usize].sig = Sig::InProgress;
        let t = self.with_def_env(d, |s| {
            s.resolve_bounds(d, &a.type_params);
            s.lower_type(&a.ty)
        });
        if self.defs[d.0 as usize].sig == Sig::InProgress {
            self.aliases.insert(d, t);
            self.defs[d.0 as usize].sig = Sig::Done;
        }
    }

    pub fn resolve_bounds(&mut self, d: DefId, tps: &'a [TypeParam]) {
        let all = self.defs[d.0 as usize].info.tparams.clone();
        let base = all.len() - tps.len();
        for (i, tp) in tps.iter().enumerate() {
            if let Some(b) = &tp.bound {
                let bt = self.lower_bound_type(b);
                let ok = matches!(self.tys.kind(bt), TyKind::Adt(bd, _) if self.kind_of(*bd) == DefKind::Interface);
                if bt != ERROR && !ok {
                    let s = self.show(bt);
                    self.err("OXS-E0117", b.span, &[("detail", &format!("type parameter bound must be an interface, found '{s}'"))]);
                } else if ok {
                    self.params[all[base + i].0 as usize].bound = Some(bt);
                }
            }
        }
    }

    // ---- enums ---------------------------------------------------------------

    fn ensure_enum_inner(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return;
        }
        self.defs[d.0 as usize].sig = Sig::Done;
        let Ast::Item(item) = self.defs[d.0 as usize].ast else { return };
        let ItemKind::Enum(e) = &item.kind else { return };
        let repr = if let Some(r) = &e.repr {
            let t = self.with_def_env(d, |s| s.lower_type(r));
            match self.tys.kind(t) {
                TyKind::Num(n) if is_int(*n) => *n,
                TyKind::Error => NumTy::I32,
                _ => {
                    self.err("OXS-E0115", r.span, &[("detail", "an enum's underlying type must be an integer type")]);
                    NumTy::I32
                }
            }
        } else {
            NumTy::I32
        };
        let mut members: Vec<(String, i128)> = Vec::new();
        let mut next: i128 = 0;
        if e.variants.len() > 65_536 {
            self.err("OXS-E0113", item.span, &[("detail", "an enum can have at most 65536 members")]);
            self.enums.insert(d, EnumSig { repr, members });
            return;
        }
        let mut const_ids: Vec<NodeId> = Vec::new();
        for v in &e.variants {
            if let Some(x) = &v.value {
                struct Ids(Vec<NodeId>);
                impl<'b> Visitor<'b> for Ids {
                    fn visit_expr(&mut self, e: &'b Expr) {
                        self.0.push(e.id);
                        walk_expr(self, e);
                    }
                }
                let mut ids = Ids(Vec::new());
                ids.visit_expr(x);
                const_ids.extend(ids.0);
            }
        }
        let mut names_seen: HashSet<&str> = HashSet::new();
        let mut vals_seen: HashSet<i128> = HashSet::new();
        let mut by_name: HashMap<String, i128> = HashMap::new();
        for v in &e.variants {
            let val = match &v.value {
                Some(x) => if let Some(n) = self.const_int(x, &by_name) { n } else {
                    self.err("OXS-E0123", x.span, &[("detail", "enum member value must be a constant integer expression")]);
                    next
                },
                None => next,
            };
            let (lo, hi) = (-(int_max(repr, true) as i128), int_max(repr, false) as i128);
            if val < lo || val > hi {
                self.err("OXS-E0102", v.span, &[("value", &val.to_string()), ("ty", repr.name())]);
            }
            if !names_seen.insert(v.name.name.as_str()) {
                self.err("OXS-E0301", v.name.span, &[("name", &v.name.name)]);
            } else {
                if !vals_seen.insert(val) {
                    self.err("OXS-E0123", v.span, &[("detail", &format!("duplicate enum value {val}: members of a closed enum must have distinct values"))]);
                }
                by_name.insert(v.name.name.clone(), val);
                members.push((v.name.name.clone(), val));
            }
            next = val.wrapping_add(1);
        }
        self.enums.insert(d, EnumSig { repr, members });
        self.with_def_env(d, |s| {
            for id in const_ids {
                s.set_ty(id, num(repr));
            }
        });
    }

    #[allow(clippy::self_only_used_in_recursion)]
    fn const_int(&mut self, e: &'a Expr, prev: &HashMap<String, i128>) -> Option<i128> {
        match &e.kind {
            ExprKind::Lit(l) => match &l.kind {
                LitKind::Int { value, .. } => i128::try_from(*value).ok(),
                _ => None,
            },
            ExprKind::Paren(i) => self.const_int(i, prev),
            ExprKind::Unary { op: UnOp::Neg, expr } => self.const_int(expr, prev)?.checked_neg(),
            ExprKind::Unary { op: UnOp::BitNot, expr } => Some(!self.const_int(expr, prev)?),
            ExprKind::Ident(i) => prev.get(&i.name).copied(),
            ExprKind::Binary { op, lhs, rhs } => {
                let (a, b) = (self.const_int(lhs, prev)?, self.const_int(rhs, prev)?);
                match op {
                    BinOp::Add => a.checked_add(b),
                    BinOp::Sub => a.checked_sub(b),
                    BinOp::Mul => a.checked_mul(b),
                    BinOp::Shl => u32::try_from(b).ok().filter(|s| *s < 100).and_then(|s| a.checked_shl(s)),
                    BinOp::Shr => u32::try_from(b).ok().filter(|s| *s < 100).map(|s| a >> s),
                    BinOp::BitOr => Some(a | b),
                    BinOp::BitAnd => Some(a & b),
                    BinOp::BitXor => Some(a ^ b),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    // ---- interfaces ----------------------------------------------------------

    fn ensure_iface_inner(&mut self, d: DefId) {
        match self.defs[d.0 as usize].sig {
            Sig::Done => return,
            Sig::InProgress => {
                let sp = self.defs[d.0 as usize].info.span;
                let n = self.def_name(d).to_string();
                self.err("OXS-E0123", sp, &[("detail", &format!("Interface '{n}' circularly extends itself"))]);
                return;
            }
            Sig::Pending => {}
        }
        let Ast::Item(item) = self.defs[d.0 as usize].ast else { return };
        let ItemKind::Interface(i) = &item.kind else { return };
        self.defs[d.0 as usize].sig = Sig::InProgress;
        self.with_def_env(d, |s| {
            s.resolve_bounds(d, &i.type_params);
            let mut methods: Vec<IfaceMethod> = Vec::new();
            let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut too_many = false;
            for m in &i.methods {
                let own = s.new_tparams(&m.type_params, d, &[]);
                let mut sc = HashMap::new();
                for p in &own {
                    sc.insert(s.params[p.0 as usize].name.clone(), Binding::TParam(*p));
                }
                s.scopes.push(sc);
                let ps: Vec<Ty> = m
                    .params
                    .iter()
                    .map(|p| if let Some(t) = &p.ty { s.lower_type(t) } else {
                        s.err("OXS-E0103", p.span, &[("what", "parameter")]);
                        ERROR
                    })
                    .collect();
                let r = m.ret.as_ref().map_or(VOID, |t| s.lower_type(t));
                s.scopes.pop();
                if !names.insert(m.name.name.clone()) {
                    s.err("OXS-E0301", m.name.span, &[("name", &m.name.name)]);
                } else {
                    methods.push(IfaceMethod { name: m.name.name.clone(), params: ps, ret: r, tparams: own });
                }
            }
            for ex in &i.extends {
                let t = s.lower_bound_type(ex);
                if let TyKind::Adt(ed, eargs) = s.tys.kind(t).clone() {
                    if s.kind_of(ed) == DefKind::Interface {
                        s.ensure_iface(ed);
                        let Some(sig) = s.ifaces.get(&ed).cloned() else { continue };
                        let tps = s.defs[ed.0 as usize].info.tparams.clone();
                        let map: Vec<(ParamId, Ty)> = tps.into_iter().zip(eargs.iter().copied()).collect();
                        for m in sig.methods {
                            if names.contains(&m.name) {
                                continue;
                            }
                            if methods.len() >= MAX_IFACE_METHODS {
                                if !too_many {
                                    too_many = true;
                                    s.err("OXS-E0113", ex.span, &[("detail", "an interface can have at most 256 methods including inherited ones")]);
                                }
                                break;
                            }
                            names.insert(m.name.clone());
                            let ps: Vec<Ty> = m.params.iter().map(|p| s.tys.subst(*p, &map)).collect();
                            let r = s.tys.subst(m.ret, &map);
                            methods.push(IfaceMethod { name: m.name, params: ps, ret: r, tparams: m.tparams });
                        }
                        continue;
                    }
                }
                if t != ERROR {
                    s.err("OXS-E0117", ex.span, &[("detail", "an interface can only extend interfaces")]);
                }
            }
            s.ifaces.insert(d, IfaceSig { methods });
        });
        self.defs[d.0 as usize].sig = Sig::Done;
    }

    // ---- records and unions --------------------------------------------------

    fn ensure_record_inner(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return;
        }
        if matches!(self.kind_of(d), DefKind::Variant { .. }) {
            self.ensure_variant(d);
            return;
        }
        let Ast::Item(item) = self.defs[d.0 as usize].ast else { return };
        let ItemKind::Record(r) = &item.kind else { return };
        self.defs[d.0 as usize].sig = Sig::InProgress;
        let is_class = r.kind == RecordKind::Class;
        let sig = self.with_def_env(d, |s| {
            s.resolve_bounds(d, &r.type_params);
            let mut fields: Vec<FieldInfo> = Vec::new();
            let mut methods: Vec<(String, DefId)> = Vec::new();
            let mut ctor = None;
            let mut member_names: HashSet<&str> = HashSet::new();
            for m in &r.members {
                match &m.kind {
                    MemberKind::Field(f) => {
                        let ty = s.lower_type(&f.ty);
                        if !member_names.insert(f.name.name.as_str()) {
                            s.err("OXS-E0301", f.name.span, &[("name", &f.name.name)]);
                            continue;
                        }
                        if ty == VOID || ty == NEVER {
                            s.err("OXS-E0115", f.ty.span, &[("detail", "a field cannot have type `void` or `never`")]);
                        }
                        if !is_class && f.is_static {
                            s.err("OXS-E0123", f.name.span, &[("detail", "static fields are only allowed in classes")]);
                        }
                        fields.push(FieldInfo {
                            name: f.name.name.clone(),
                            ty,
                            readonly: f.readonly,
                            private: f.private,
                            is_static: f.is_static,
                            has_init: f.init.is_some(),
                            span: f.name.span,
                        });
                    }
                    MemberKind::Method(f) => {
                        let Some(&md) = s.mods[s.cm].out.node_def.get(&m.id) else { continue };
                        if !member_names.insert(f.name.name.as_str()) {
                            s.err("OXS-E0301", f.name.span, &[("name", &f.name.name)]);
                            continue;
                        }
                        if f.mods.mutating && is_class {
                            s.err("OXS-E0123", f.name.span, &[("detail", "`mutating` is only meaningful on struct methods; class methods may always modify `this`")]);
                        }
                        methods.push((f.name.name.clone(), md));
                    }
                    MemberKind::Constructor(f) => {
                        let Some(&md) = s.mods[s.cm].out.node_def.get(&m.id) else { continue };
                        if !is_class {
                            s.err("OXS-E0123", f.name.span, &[("detail", "structs have no constructor; create them with an object literal")]);
                        } else if ctor.is_some() {
                            s.err("OXS-E0301", f.name.span, &[("name", "constructor")]);
                        } else {
                            ctor = Some(md);
                        }
                    }
                }
            }
            let mut implements = Vec::new();
            for ty in &r.implements {
                let t = s.lower_bound_type(ty);
                let ok = matches!(s.tys.kind(t), TyKind::Adt(id, _) if s.kind_of(*id) == DefKind::Interface);
                if ok {
                    implements.push(t);
                } else if t != ERROR {
                    s.err("OXS-E0117", ty.span, &[("detail", "`implements` needs an interface")]);
                }
            }
            RecordSig { is_class, fields, methods, ctor, implements, discr: None }
        });
        self.records.insert(d, sig);
        self.defs[d.0 as usize].sig = Sig::Done;
    }

    fn lit_discr(&self, l: &Lit) -> Option<Discr> {
        match &l.kind {
            LitKind::Str(s) => Some(Discr::Str(s.clone())),
            LitKind::Int { value, .. } => i128::try_from(*value).ok().map(Discr::Int),
            LitKind::Bool(b) => Some(Discr::Bool(*b)),
            _ => None,
        }
    }

    fn variant_object(&self, d: DefId) -> Option<&'a [oxiscript_syntax::FieldSig]> {
        let strip = |mut t: &'a TypeExpr| -> Option<&'a [oxiscript_syntax::FieldSig]> {
            while let TypeExprKind::Paren(i) = &t.kind {
                t = i;
            }
            match &t.kind {
                TypeExprKind::Object(f) => Some(f.as_slice()),
                _ => None,
            }
        };
        match self.defs[d.0 as usize].ast {
            Ast::Obj(t) => strip(t),
            Ast::Item(item) => match &item.kind {
                ItemKind::TypeAlias(a) => strip(&a.ty),
                _ => None,
            },
            _ => None,
        }
    }

    fn ensure_variant_inner(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return;
        }
        self.defs[d.0 as usize].sig = Sig::InProgress;
        let Some(obj) = self.variant_object(d) else {
            self.defs[d.0 as usize].sig = Sig::Done;
            return;
        };
        let sig = self.with_def_env(d, |s| {
            let mut fields = Vec::new();
            let mut discr: Option<(String, Discr)> = None;
            let mut field_names: HashSet<String> = HashSet::new();
            for f in obj {
                let mut t = &f.ty;
                while let TypeExprKind::Paren(i) = &t.kind {
                    t = i;
                }
                if let TypeExprKind::Lit(l) = &t.kind {
                    if discr.is_none() {
                        match s.lit_discr(l) {
                            Some(v) => discr = Some((f.name.name.clone(), v)),
                            None => {
                                s.err("OXS-E0115", f.span, &[("detail", "a discriminant must be a string, integer or boolean literal")]);
                            }
                        }
                        continue;
                    }
                    s.err("OXS-E0115", f.span, &[("detail", "only one field of a union member may have a literal type (the discriminant)")]);
                    continue;
                }
                if field_names.contains(&f.name.name) || discr.as_ref().is_some_and(|(n, _)| *n == f.name.name) {
                    s.err("OXS-E0301", f.name.span, &[("name", &f.name.name)]);
                    continue;
                }
                let ty = s.lower_type(&f.ty);
                field_names.insert(f.name.name.clone());
                fields.push(FieldInfo {
                    name: f.name.name.clone(),
                    ty,
                    readonly: f.readonly,
                    private: false,
                    is_static: false,
                    has_init: false,
                    span: f.name.span,
                });
            }
            RecordSig { is_class: false, fields, methods: vec![], ctor: None, implements: vec![], discr }
        });
        self.records.insert(d, sig);
        self.defs[d.0 as usize].sig = Sig::Done;
    }

    fn ensure_union_inner(&mut self, d: DefId) {
        if self.defs[d.0 as usize].sig != Sig::Pending {
            return;
        }
        self.defs[d.0 as usize].sig = Sig::InProgress;
        let Ast::Item(item) = self.defs[d.0 as usize].ast else { return };
        let ItemKind::TypeAlias(a) = &item.kind else { return };
        let mut members: &'a [TypeExpr] = &[];
        let mut t = &a.ty;
        while let TypeExprKind::Paren(i) = &t.kind {
            t = i;
        }
        if let TypeExprKind::Union(ms) = &t.kind {
            members = ms.as_slice();
        }
        let utps = self.defs[d.0 as usize].info.tparams.clone();
        let uname = self.def_name(d).to_string();
        let mut vars: Vec<(DefId, Vec<Ty>)> = Vec::new();
        if members.len() > 64 {
            self.err("OXS-E0113", a.ty.span, &[("detail", "a union can have at most 64 variants")]);
            self.defs[d.0 as usize].sig = Sig::Done;
            return;
        }
        for (i, m) in members.iter().enumerate() {
            let mut mt = m;
            while let TypeExprKind::Paren(x) = &mt.kind {
                mt = x;
            }
            match &mt.kind {
                TypeExprKind::Object(_) => {
                    let vd = self.new_def(
                        &format!("v{i}"),
                        DefKind::Variant { union: Some(d), index: i as u32 },
                        mt.span,
                        NodeId(u32::MAX),
                        false,
                        utps.clone(),
                        None,
                        Ast::Obj(mt),
                    );
                    self.defs[vd.0 as usize].info.module = self.defs[d.0 as usize].info.module;
                    let targs: Vec<Ty> = utps.iter().map(|p| self.tys.mk(TyKind::Param(*p))).collect();
                    vars.push((vd, targs));
                }
                TypeExprKind::Path { .. } => {
                    let ty = self.with_def_env(d, |s| s.lower_type(mt));
                    match self.tys.kind(ty).clone() {
                        TyKind::Adt(vd, vargs) if matches!(self.kind_of(vd), DefKind::Variant { union: None, .. }) => {
                            let ok = if utps.is_empty() {
                                vargs.is_empty()
                            } else {
                                vargs.len() == utps.len()
                                    && vargs.iter().zip(&utps).all(|(a, p)| *self.tys.kind(*a) == TyKind::Param(*p))
                            };
                            if !ok {
                                self.err("OXS-E0115", mt.span, &[("detail", "a named union member must be non-generic (in a non-generic union) or take exactly the union's type parameters")]);
                                continue;
                            }
                            self.defs[vd.0 as usize].info.kind = DefKind::Variant { union: Some(d), index: i as u32 };
                            vars.push((vd, vargs.to_vec()));
                        }
                        TyKind::Error => {}
                        _ => {
                            self.err(
                                "OXS-E0115",
                                mt.span,
                                &[("detail", "union members must be object types (`{ kind: \"a\"; ... }`) or aliases of them; `string | number` style unions are not supported")],
                            );
                        }
                    }
                }
                _ => {
                    self.err(
                        "OXS-E0115",
                        mt.span,
                        &[("detail", "union members must be object types with a literal `kind` field; wrap other types, e.g. `{ kind: \"num\"; value: i32 }`")],
                    );
                }
            }
        }
        // resolve variants and check the discriminant
        let mut field: Option<String> = None;
        let mut dty = ERROR;
        let mut out: Vec<UnionVariant> = Vec::new();
        for (vd, targs) in vars {
            self.ensure_variant(vd);
            let Some((fname, val)) = self.records.get(&vd).and_then(|r| r.discr.clone()) else {
                let sp = self.defs[vd.0 as usize].info.span;
                self.err("OXS-E0116", sp, &[("detail", "union member has no discriminant: give it a field with a literal type such as `kind: \"circle\"`")]);
                continue;
            };
            let vty = match &val {
                Discr::Str(_) => STR,
                Discr::Int(_) => I32,
                Discr::Bool(_) => BOOL,
            };
            match &field {
                None => {
                    field = Some(fname.clone());
                    dty = vty;
                }
                Some(f) if *f != fname || dty != vty => {
                    let sp = self.defs[vd.0 as usize].info.span;
                    self.err(
                        "OXS-E0116",
                        sp,
                        &[("detail", &format!("all members of `{uname}` must use the same discriminant field `{f}` with literals of one kind"))],
                    );
                    continue;
                }
                _ => {}
            }
            if out.iter().any(|o| o.value == val) {
                let sp = self.defs[vd.0 as usize].info.span;
                self.err("OXS-E0116", sp, &[("detail", &format!("duplicate discriminant value in union `{uname}`"))]);
                continue;
            }
            if self.defs[vd.0 as usize].info.name.starts_with('v') && self.defs[vd.0 as usize].info.node == NodeId(u32::MAX) {
                self.defs[vd.0 as usize].info.name = match &val {
                    Discr::Str(s) => s.clone(),
                    Discr::Int(n) => format!("{n}"),
                    Discr::Bool(b) => format!("{b}"),
                };
            }
            out.push(UnionVariant { def: vd, args: targs, value: val });
        }
        // re-number variant indices after dropping bad ones
        for (i, v) in out.iter().enumerate() {
            if let DefKind::Variant { union, .. } = self.defs[v.def.0 as usize].info.kind {
                self.defs[v.def.0 as usize].info.kind = DefKind::Variant { union, index: i as u32 };
            }
        }
        let _ = full_mask(0);
        self.unions.insert(
            d,
            UnionSig { discr_field: field.unwrap_or_else(|| "kind".into()), discr_ty: dty, variants: out },
        );
        self.defs[d.0 as usize].sig = Sig::Done;
    }

    // ---- functions -----------------------------------------------------------

    fn ensure_fn_inner(&mut self, d: DefId) {
        match self.defs[d.0 as usize].sig {
            Sig::Done => return,
            Sig::InProgress => {
                if !self.fns.contains_key(&d) {
                    return;
                }
                // recursion through an un-annotated return type
                return;
            }
            Sig::Pending => {}
        }
        let (decl, member, record, item) = match self.defs[d.0 as usize].ast {
            Ast::Item(item) => match &item.kind {
                ItemKind::Fn(f) => (f, None, None, Some(item)),
                _ => return,
            },
            Ast::Member(m, r) => match &m.kind {
                MemberKind::Method(f) | MemberKind::Constructor(f) => (f, Some(m), Some(r), None),
                MemberKind::Field(_) => return,
            },
            _ => return,
        };
        let _ = (member, item);
        self.defs[d.0 as usize].sig = Sig::InProgress;
        let exported = self.defs[d.0 as usize].info.exported;
        let is_ctor = self.kind_of(d) == DefKind::Ctor;
        let owner = self.defs[d.0 as usize].info.owner;
        let sig = self.with_def_env(d, |s| {
            let all = s.defs[d.0 as usize].info.tparams.clone();
            let own_n = decl.type_params.len();
            s.resolve_bounds(d, &decl.type_params);
            let _ = own_n;
            let mut params = Vec::new();
            let mut pnames: HashSet<&str> = HashSet::new();
            for p in &decl.params {
                let ty = if let Some(t) = &p.ty { s.lower_type(t) } else {
                    s.err("OXS-E0103", p.span, &[("what", "parameter")]);
                    ERROR
                };
                if ty == VOID || ty == NEVER {
                    s.err("OXS-E0115", p.span, &[("detail", "a parameter cannot have type `void` or `never`")]);
                }
                if !pnames.insert(p.name.name.as_str()) {
                    s.err("OXS-E0301", p.name.span, &[("name", &p.name.name)]);
                }
                params.push(ParamSig { name: p.name.name.clone(), ty, has_default: p.default.is_some() });
            }
            // a parameter with a default may not be followed by one without
            let mut seen_default = false;
            for (p, ps) in decl.params.iter().zip(&params) {
                if seen_default && !ps.has_default {
                    s.err("OXS-E0123", p.span, &[("detail", "a required parameter cannot follow a parameter with a default value")]);
                }
                seen_default |= ps.has_default;
            }
            let ret = if is_ctor {
                let od = owner.unwrap_or(d);
                let args: Vec<Ty> = s.defs[od.0 as usize].info.tparams.iter().map(|p| s.tys.mk(TyKind::Param(*p))).collect();
                s.tys.adt(od, &args)
            } else if let Some(r) = &decl.ret {
                s.lower_type(r)
            } else if exported {
                s.err("OXS-E0103", decl.name.span, &[("what", "return type of an exported function")]);
                ERROR
            } else {
                ERROR
            };
            let _ = record;
            FnSig {
                params,
                ret,
                tparams: all,
                mutating: decl.mods.mutating,
                is_static: decl.mods.is_static,
                private: decl.mods.private,
            }
        });
        let infer = !is_ctor && decl.ret.is_none() && !exported && decl.body.is_some();
        self.fns.insert(d, sig);
        if infer {
            // infer the return type by checking the body now
            self.fns.get_mut(&d).map(|s| s.ret = ERROR);
            self.check_fn_body(d);
        }
        self.defs[d.0 as usize].sig = Sig::Done;
    }

    // ---- globals ---------------------------------------------------------------

    fn ensure_global_inner(&mut self, d: DefId) {
        match self.defs[d.0 as usize].sig {
            Sig::Done => return,
            Sig::InProgress => {
                let sp = self.defs[d.0 as usize].info.span;
                let n = self.def_name(d).to_string();
                self.err("OXS-E0103", sp, &[("what", &format!("`{n}`, whose initialiser refers to itself,"))]);
                self.globals.insert(d, GlobalSig { ty: ERROR, mutable: true });
                self.defs[d.0 as usize].sig = Sig::Done;
                return;
            }
            Sig::Pending => {}
        }
        self.defs[d.0 as usize].sig = Sig::InProgress;
        self.check_global(d);
        self.defs[d.0 as usize].sig = Sig::Done;
    }

    fn abi_scalar_ok(&self, t: Ty) -> bool {
        match self.tys.kind(t) {
            TyKind::Bool | TyKind::Num(_) | TyKind::Error => true,
            TyKind::Adt(d, _) => matches!(self.kind_of(*d), DefKind::Enum | DefKind::Handle | DefKind::Host),
            _ => false,
        }
    }

    fn abi_param_ok(&self, t: Ty) -> bool {
        t == STR || *self.tys.kind(t) == TyKind::Array(U8) || self.abi_scalar_ok(t)
    }

    fn abi_ret_ok(&self, t: Ty) -> bool {
        t == VOID || self.abi_scalar_ok(t)
    }

    /// Whether the exported function signature rules hold (convention 7 and `main`).
    pub fn check_export_sig(&mut self, d: DefId) {
        let Some(sig) = self.fns.get(&d).cloned() else { return };
        let info = self.defs[d.0 as usize].info.clone();
        let name = info.name.clone();
        let host = self.is_host_fn(d);
        let strs = sig
            .params
            .iter()
            .filter(|p| p.ty == STR || *self.tys.kind(p.ty) == TyKind::Array(U8))
            .count();
        if host && strs > 1 {
            self.err(
                "OXS-E0121",
                info.span,
                &[("detail", &format!("`@host` function `{name}` has more than one `string`/`u8[]` parameter; the host can deliver at most one string per call"))],
            );
        }
        if host && info.module == 0 && name != "main" && sig.tparams.is_empty() {
            // ABI 1.0 export boundary: scalar (number, bool, enum) or `string`/`u8[]` parameters, scalar or void result.
            for p in &sig.params {
                if !self.abi_param_ok(p.ty) {
                    let s = self.show(p.ty);
                    self.err(
                        "OXS-E0121",
                        info.span,
                        &[("detail", &format!("`@host` function `{name}`: parameter `{}: {s}` does not fit the ABI 1.0 export boundary (only numbers, `bool`, enums, `string` and `u8[]` parameters); drop `@host` or change the signature", p.name))],
                    );
                }
            }
            if !self.abi_ret_ok(sig.ret) {
                let s = self.show(sig.ret);
                self.err(
                    "OXS-E0121",
                    info.span,
                    &[("detail", &format!("`@host` function `{name}`: result type `{s}` does not fit the ABI 1.0 export boundary (only numbers, `bool`, enums or `void`); drop `@host` or change the signature"))],
                );
            }
        }
        if name == "main" {
            if !info.exported {
                self.err("OXS-E0121", info.span, &[("detail", "`main` must be exported: `export function main(...)`")]);
                return;
            }
            if sig.ret != VOID && sig.ret != ERROR {
                self.err("OXS-E0121", info.span, &[("detail", "`main` must return `void`")]);
            }
            for p in &sig.params {
                let host = matches!(self.tys.kind(p.ty), TyKind::Adt(h, _) if self.kind_of(*h) == DefKind::Host);
                if !host && p.ty != ERROR {
                    let s = self.show(p.ty);
                    self.err(
                        "OXS-E0121",
                        info.span,
                        &[("detail", &format!("every parameter of `main` must be a host capability type; `{}: {s}` is not", p.name))],
                    );
                }
            }
            if info.module as usize == 0 {
                self.entry = Some(d);
            }
            self.mods[info.module as usize].out.main = Some(d);
        }
    }
}
