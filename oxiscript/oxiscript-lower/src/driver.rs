//! The driver: entry points, the instantiation worklist, function/constructor/initialiser
//! lowering, the `main` wrapper and the `.oxs` -> Wasm pipeline.

use crate::ctx::{DefAst, GKey, Job, Lw};
use crate::fb::Fb;
use crate::fx::{CtorState, Fx, FnKind, Slot, THIS_FIELD};
use crate::{Diagnostic, Diagnostics};
use oxiscript_abi::idl::Interface;
use oxiscript_abi::manifest::Need;
use oxiscript_check::{CheckOptions, Checked, DefId, DefKind, ModuleLoader, NoLoader, ParamId, Ty, TyKind};
use oxiscript_ir::{self as ir, FieldRef, Op, Terminator, Ty as It, TypeRef, ValueId};
use oxiscript_syntax::visit::Visitor;
use oxiscript_syntax::{FileId, ItemKind, MemberKind, Module, PatternKind};

/// Options of the pipeline.
#[derive(Clone, Debug)]
pub struct Options {
    pub runtime: oxiscript_wasm::RuntimeMode,
    /// Validate the generated Wasm.
    pub validate_output: bool,
    /// Lower every non-generic function, method and constructor, not only what `main` and the
    /// exports reach (tests and tools that want all code verified).
    pub lower_all: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            runtime: oxiscript_wasm::RuntimeMode::Stub,
            validate_output: true,
            lower_all: false,
        }
    }
}

/// The result of the whole pipeline.
pub struct CompileOutput {
    pub wasm: Vec<u8>,
    pub module: ir::Module,
    pub needs: Vec<Need>,
    /// Warnings of the lowering (`OXS-W0700`).
    pub notes: Vec<Diagnostic>,
}

/// A lowered program: the module and the capabilities (`needs`, in `oxi_init` order) it uses.
pub struct Lowered {
    pub module: ir::Module,
    pub needs: Vec<Need>,
    /// Warnings (`OXS-W0700`: a function that cannot be exported under ABI 1.0 stays internal).
    pub notes: Vec<Diagnostic>,
}

fn check_diags(ck: &Checked) -> Diagnostics {
    Diagnostics(
        ck.errors()
            .map(|d| Diagnostic::new(d.code, d.message.clone(), Some(d.span)))
            .collect(),
    )
}

/// Lower a checked program to OxiIR. The result is verified by the caller (`compile_*` do).
pub fn lower(ck: &mut Checked, interfaces: &[Interface]) -> Result<ir::Module, Diagnostics> {
    lower_full(ck, interfaces).map(|l| l.module)
}

/// Like [`lower`], also returning the manifest `needs`.
pub fn lower_full(ck: &mut Checked, interfaces: &[Interface]) -> Result<Lowered, Diagnostics> {
    lower_with(ck, interfaces, false)
}

/// [`lower_full`] with the choice of lowering all code (`all`) or only the reachable part.
pub fn lower_with(ck: &mut Checked, interfaces: &[Interface], all: bool) -> Result<Lowered, Diagnostics> {
    if !ck.ok() {
        return Err(check_diags(ck));
    }
    let srcs = ck.sources.clone();
    let asts: Vec<Module> = srcs
        .iter()
        .enumerate()
        .map(|(i, s)| oxiscript_syntax::parse(s, FileId(i as u32)).module)
        .collect();
    let mut lw = Lw::new(ck, &asts, interfaces.to_vec(), "program");
    lw.lower_all = all;
    lw.run()
}

impl<'c, 'a> Lw<'c, 'a> {
    fn run(&mut self) -> Result<Lowered, Diagnostics> {
        // module initialisers first: they queue the functions they call
        let init = self.lower_init();
        self.out.funcs.push(init);
        // exported functions and `main`
        let entry = self.ck.entry;
        let exports: Vec<DefId> = self.ck.modules[0]
            .items
            .iter()
            .copied()
            .filter(|d| {
                let i = &self.ck.defs[d.0 as usize];
                i.kind == DefKind::Fn && i.exported && !i.local && i.tparams.is_empty() && Some(*d) != entry
            })
            .collect();
        for d in exports {
            self.fn_sym(d, Vec::new());
        }
        let user_main = entry.map(|d| self.fn_sym(d, Vec::new()));
        if self.lower_all {
            self.queue_all_roots();
        }
        // statics and classes without explicit constructors are lowered on demand; run the worklist
        while let Some(job) = self.jobs.pop_front() {
            if self.diags.len() >= 100 || self.budget_hit {
                break;
            }
            match job {
                Job::Fn { def, targs, sym } => {
                    if let Some(f) = self.lower_fn_job(def, targs, &sym) {
                        self.out.funcs.push(f);
                    }
                }
            }
        }
        if !self.diags.is_empty() {
            return Err(Diagnostics(std::mem::take(&mut self.diags)));
        }
        let needs = self.compute_needs();
        let main = self.build_main(&needs, user_main);
        self.out.funcs.push(main);
        let module = std::mem::take(&mut self.out);
        let notes = std::mem::take(&mut self.notes);
        Ok(Lowered { module, needs, notes })
    }

    /// Queue every non-generic function, method and constructor of every module.
    fn queue_all_roots(&mut self) {
        let n = self.ck.defs.len();
        for i in 0..n {
            let d = DefId(i as u32);
            let info = &self.ck.defs[i];
            if !info.tparams.is_empty() || info.local {
                continue;
            }
            if let Some(o) = info.owner {
                if !self.ck.defs[o.0 as usize].tparams.is_empty() || self.ck.defs[o.0 as usize].local {
                    continue;
                }
            }
            let prelude = self.ck.prelude == Some(info.module);
            if prelude {
                continue;
            }
            match info.kind {
                DefKind::Fn | DefKind::Method | DefKind::Ctor if self.defast.contains_key(&d) => {
                    self.fn_sym(d, Vec::new());
                }
                _ => {}
            }
        }
    }

    fn compute_needs(&mut self) -> Vec<Need> {
        let mut needs: Vec<Need> = Vec::new();
        if let Some(entry) = self.ck.entry {
            let sig = self.ck.fns[&entry].clone();
            for p in &sig.params {
                if let TyKind::Adt(d, _) = self.ck.tys.kind(p.ty).clone() {
                    if let Some(h) = self.ck.hosts.get(&d) {
                        let n = Need {
                            module: h.module.clone(),
                            version: h.version,
                            interface: self.ck.defs[d.0 as usize].name.clone(),
                        };
                        if !needs.contains(&n) {
                            needs.push(n);
                        }
                    }
                }
            }
        }
        for imp in &self.out.imports {
            if let Some(itf) = self.interfaces.iter().find(|i| i.import_module() == imp.module) {
                let n = Need {
                    module: itf.module.clone(),
                    version: itf.version,
                    interface: itf.name.clone(),
                };
                if !needs.contains(&n) {
                    needs.push(n);
                }
            }
        }
        needs
    }

    /// The IR `main(caps...)`: module initialisers, then the program's `main` with its capabilities.
    fn build_main(&mut self, needs: &[Need], user_main: Option<String>) -> ir::Func {
        let mut fb = Fb::new("main");
        let params: Vec<ValueId> = needs.iter().map(|_| fb.param("cap", It::I32, false)).collect();
        fb.emit0(Op::Call {
            callee: "init.globals".into(),
            args: vec![],
        });
        if let (Some(um), Some(entry)) = (user_main, self.ck.entry) {
            // the user's main receives the first capabilities, in its own parameter order
            let sig = self.ck.fns[&entry].clone();
            let mut args = Vec::new();
            for p in &sig.params {
                if let TyKind::Adt(d, _) = self.ck.tys.kind(p.ty).clone() {
                    if let Some(h) = self.ck.hosts.get(&d) {
                        let n = Need {
                            module: h.module.clone(),
                            version: h.version,
                            interface: self.ck.defs[d.0 as usize].name.clone(),
                        };
                        if let Some(i) = needs.iter().position(|x| *x == n) {
                            args.push(params[i]);
                        }
                    }
                }
            }
            fb.emit0(Op::Call { callee: um, args });
        }
        fb.term(Terminator::Ret(vec![]));
        fb.finish()
    }

    // ------------------------------------------------------------------ initialisers

    fn lower_init(&mut self) -> ir::Func {
        let order: Vec<u32> = self.ck.order.clone();
        let mut fx = Fx::new(self, "init.globals", 0, Vec::new());
        for m in order {
            let m = m as usize;
            if m >= fx.lw.asts.len() {
                continue;
            }
            let ast: &Module = &fx.lw.asts[m];
            for item in &ast.items {
                match &item.kind {
                    ItemKind::Var(v) => {
                        let PatternKind::Ident(id) = &v.pattern.kind else { continue };
                        let Some(init) = &v.init else { continue };
                        let Some(&d) = fx.lw.ck.modules[m].node_def.get(&item.id) else { continue };
                        let Some(g) = fx.lw.ck.globals.get(&d).cloned() else { continue };
                        let _ = id;
                        fx.with_ctx(m, Vec::new(), |fx| {
                            let o = fx.expr(init);
                            if let Some(v) = fx.own_of(o) {
                                fx.store_global(GKey::Global(d), g.ty, v);
                            }
                        });
                    }
                    ItemKind::Record(r) => {
                        let Some(&d) = fx.lw.ck.modules[m].node_def.get(&item.id) else { continue };
                        let rec = fx.lw.ck.records.get(&d).cloned();
                        let Some(rec) = rec else { continue };
                        // statics of generic classes are not supported (no instantiation context)
                        for mem in &r.members {
                            if let MemberKind::Field(f) = &mem.kind {
                                if f.is_static {
                                    let Some(init) = &f.init else { continue };
                                    let Some(idx) = rec.fields.iter().position(|x| x.is_static && x.name == f.name.name) else {
                                        continue;
                                    };
                                    let fty = rec.fields[idx].ty;
                                    fx.with_ctx(m, Vec::new(), |fx| {
                                        let o = fx.expr(init);
                                        if let Some(v) = fx.own_of(o) {
                                            fx.store_global(GKey::Static(d, idx as u32), fty, v);
                                        }
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        fx.emit_return(None);
        fx.finish()
    }

    // ------------------------------------------------------------------ functions

    fn lower_fn_job(&mut self, def: DefId, targs: Vec<Ty>, sym: &str) -> Option<ir::Func> {
        match self.ck.defs[def.0 as usize].kind {
            DefKind::Fn | DefKind::Method => self.lower_plain(def, targs, sym),
            DefKind::Ctor => self.lower_ctor(def, targs, sym, true),
            DefKind::Class | DefKind::Struct => self.lower_ctor(def, targs, sym, false),
            _ => {
                self.unsupported("this definition", None);
                None
            }
        }
    }

    fn lower_plain(&mut self, def: DefId, targs: Vec<Ty>, sym: &str) -> Option<ir::Func> {
        let info = self.ck.defs[def.0 as usize].clone();
        let sig = self.ck.fns.get(&def)?.clone();
        let map: Vec<(ParamId, Ty)> = sig.tparams.iter().copied().zip(targs.iter().copied()).collect();
        let Some(DefAst::Fn(decl)) = self.defast.get(&def).copied() else {
            self.unsupported("a function without a body", Some(info.span));
            return None;
        };
        let Some(body) = decl.body.as_ref() else {
            self.unsupported("a function without a body", Some(info.span));
            return None;
        };
        let ret = self.subst(sig.ret, &map);
        let m = info.module as usize;
        let mut is_export = info.exported && m == 0 && info.kind == DefKind::Fn && !info.local && Some(def) != self.ck.entry;
        let name = info.name.clone();
        if is_export && !self.abi_exportable(&sig, &map) {
            is_export = false;
            self.notes.push(Diagnostic::new(
                "OXS-W0700",
                format!("function `{name}` is not exported: ABI 1.0 exports take scalars and `str`/`[u8]` parameters and return scalars"),
                Some(info.span),
            ));
        }
        let owner_args: Vec<Ty> = match info.owner {
            Some(o) => {
                let n = self.ck.defs[o.0 as usize].tparams.len();
                targs.iter().take(n).copied().collect()
            }
            None => Vec::new(),
        };
        let mut fx = Fx::new(self, sym, m, map.clone());
        fx.fn_def = Some(def);
        fx.ret_ty = ret;
        fx.kind = if sig.mutating { FnKind::Mutating } else { FnKind::Plain };
        fx.scan_assigned_body(|sc| sc.visit_block(body));
        // receiver
        if let (Some(owner), false) = (info.owner, sig.is_static) {
            let this_ty = {
                let t = fx.lw.ck.tys.adt(owner, &owner_args);
                t
            };
            if let (Some(irt), Some(&tl)) = (fx.lw.repr(this_ty), fx.lw.ck.modules[m].this_locals.get(&def)) {
                let own = sig.mutating;
                let v = fx.fb.param("this", irt.clone(), own);
                fx.this_key = Some(tl.0);
                fx.declare(tl.0, Slot::Ssa { v, owned: own });
                if own {
                    fx.fb.f.rets.push(irt);
                }
            }
        }
        for (i, p) in decl.params.iter().enumerate() {
            let pty = fx.lw.subst(sig.params[i].ty, &map);
            let Some(&l) = fx.lw.ck.modules[m].binding_local.get(&p.name.id) else { continue };
            if let Some(irt) = fx.lw.repr(pty) {
                let v = fx.fb.param("p", irt, false);
                fx.bind_param(l, v);
            }
        }
        if let Some(r) = fx.lw.repr(ret) {
            fx.fb.f.rets.push(r);
        }
        if is_export {
            fx.fb.f.export = true;
            fx.fb.attr("export_name", &[&name]);
        }
        fx.block_stmts(body);
        Some(fx.finish())
    }

    /// ABI 1.0 export boundary: scalars, capability handles and `str`/`[u8]` parameters, scalar results.
    fn abi_exportable(&mut self, sig: &oxiscript_check::FnSig, map: &[(ParamId, Ty)]) -> bool {
        let ok_param = |t: &It| t.is_scalar() || matches!(t, It::Str) || matches!(t, It::Array(e) if **e == It::U8);
        for p in &sig.params {
            let t = self.subst(p.ty, map);
            if let Some(r) = self.repr(t) {
                if !ok_param(&r) {
                    return false;
                }
            }
        }
        let rt = self.subst(sig.ret, map);
        match self.repr(rt) {
            Some(r) => r.is_scalar(),
            None => true,
        }
    }

    fn lower_ctor(&mut self, def: DefId, targs: Vec<Ty>, sym: &str, explicit: bool) -> Option<ir::Func> {
        let info = self.ck.defs[def.0 as usize].clone();
        let (class, ctor_sig) = if explicit {
            (info.owner?, self.ck.fns.get(&def).cloned())
        } else {
            (def, None)
        };
        let m = self.ck.defs[class.0 as usize].module as usize;
        let cmap = self.tparam_map(class, &targs);
        let class_ty = self.ck.tys.adt(class, &targs);
        let ir_ty = self.struct_name(class, &targs);
        let map: Vec<(ParamId, Ty)> = match &ctor_sig {
            Some(s) => s.tparams.iter().copied().zip(targs.iter().copied()).collect(),
            None => cmap.clone(),
        };
        let mut fx = Fx::new(self, sym, m, map.clone());
        fx.fn_def = Some(def);
        fx.ret_ty = class_ty;
        fx.fb.f.rets = vec![It::Ref(ir_ty.clone())];
        fx.ctor = Some(CtorState {
            class,
            ir_ty,
            args: targs,
            materialised: None,
        });
        let decl = match fx.lw.defast.get(&def).copied() {
            Some(DefAst::Fn(d)) if explicit => Some(d),
            _ => None,
        };
        if let Some(d) = decl {
            if let Some(b) = &d.body {
                fx.scan_assigned_body(|sc| sc.visit_block(b));
            }
        }
        if let Some(&tl) = fx.lw.ck.modules[m].this_locals.get(&def) {
            fx.this_key = Some(tl.0);
        }
        if let (Some(d), Some(sig)) = (decl, &ctor_sig) {
            for (i, p) in d.params.iter().enumerate() {
                let pty = fx.lw.subst(sig.params[i].ty, &map);
                let Some(&l) = fx.lw.ck.modules[m].binding_local.get(&p.name.id) else { continue };
                if let Some(irt) = fx.lw.repr(pty) {
                    let v = fx.fb.param("p", irt, false);
                    fx.bind_param(l, v);
                }
            }
        }
        // field initialisers run first, in declaration order
        fx.init_ctor_fields(class, &cmap);
        fx.ctor_base_scopes = fx.scopes.len();
        if let Some(d) = decl {
            if let Some(b) = &d.body {
                fx.push_scope();
                fx.ctor_base_scopes = fx.scopes.len();
                for s in &b.stmts {
                    fx.stmt(s);
                }
                fx.pop_scope();
            }
        }
        fx.ctor_finish();
        Some(fx.finish())
    }
}

impl<'l, 'c, 'a> Fx<'l, 'c, 'a> {
    pub fn store_global(&mut self, key: GKey, ty: Ty, v: ValueId) {
        let ty = self.lw_ty(ty);
        let g = self.lw.global_ir(key, ty);
        let val = if g.wrapped {
            self.fb.emit1(Op::Opt { val: v }, It::Opt(Box::new(g.vty.clone())))
        } else {
            v
        };
        self.fb.emit0(Op::GlobalSet { name: g.sym.clone(), val });
    }

    fn lw_ty(&mut self, t: Ty) -> Ty {
        self.cty(t)
    }

    /// Evaluate the initialisers of the instance fields into virtual field variables.
    fn init_ctor_fields(&mut self, class: DefId, cmap: &[(ParamId, Ty)]) {
        let rec = self.lw.ck.records[&class].clone();
        let module = self.lw.ck.defs[class.0 as usize].module as usize;
        for (i, f) in rec.fields.iter().enumerate() {
            if f.is_static {
                continue;
            }
            let key = THIS_FIELD | i as u32;
            let fty = self.lw.subst(f.ty, cmap);
            let Some(irt) = self.lw.repr(fty) else { continue };
            if f.has_init {
                if let Some(init) = self.field_init_expr(class, &f.name) {
                    let o = self.with_ctx(module, cmap.to_vec(), |fx| fx.expr(init));
                    if let Some(v) = self.own_of(o) {
                        self.env.insert(key, Slot::Ssa { v, owned: true });
                        continue;
                    }
                }
            }
            // scalars and nullable references default to zero / null; other fields must be assigned
            let d = match &irt {
                It::Opt(_) => Some(self.fb.emit1(Op::Null, irt.clone())),
                t if t.is_scalar() => Some(self.dummy(t)),
                It::Str => {
                    let v = self.str_const("");
                    self.retain(v);
                    Some(v)
                }
                It::Array(_) => Some(self.fb.emit1(Op::ArrayEmpty, irt.clone())),
                _ => None,
            };
            if let Some(v) = d {
                self.env.insert(key, Slot::Ssa { v, owned: true });
            }
        }
    }

    /// Allocate the object of the constructor under lowering from the virtual field variables.
    pub fn materialise_ctor_this(&mut self) -> Option<ValueId> {
        let (class, ir_ty, args, done) = {
            let c = self.ctor.as_ref()?;
            (c.class, c.ir_ty.clone(), c.args.clone(), c.materialised)
        };
        if done.is_some() {
            return done;
        }
        if self.scopes.len() > self.ctor_base_scopes.max(1) || !self.loops.is_empty() {
            self.lw.unsupported(
                "using `this` as a value inside nested control flow of a constructor before it is complete",
                None,
            );
        }
        let rec = self.lw.ck.records[&class].clone();
        let obj = self.fb.emit1(
            Op::Alloc(TypeRef {
                ty: ir_ty.clone(),
                variant: None,
            }),
            It::Ref(ir_ty.clone()),
        );
        let cmap = self.lw.tparam_map(class, &args);
        for (i, f) in rec.fields.iter().enumerate() {
            if f.is_static {
                continue;
            }
            let key = THIS_FIELD | i as u32;
            let fty = self.lw.subst(f.ty, &cmap);
            if self.lw.repr(fty).is_none() {
                continue;
            }
            if let Some(Slot::Ssa { v, .. }) = self.env.remove(&key) {
                self.fb.emit0(Op::SetField {
                    field: FieldRef {
                        ty: ir_ty.clone(),
                        variant: None,
                        field: f.name.clone(),
                    },
                    obj,
                    val: v,
                    init: true,
                });
            } else {
                self.lw.unsupported("a constructor path that leaves a field unassigned", None);
            }
        }
        if let Some(c) = self.ctor.as_mut() {
            c.materialised = Some(obj);
        }
        if let Some(k) = self.this_key {
            self.declare(k, Slot::Ssa { v: obj, owned: true });
        }
        Some(obj)
    }

    /// End of a constructor body (or a `return;` inside it): build the object and return it.
    pub fn ctor_finish(&mut self) {
        if self.dead {
            return;
        }
        let Some(obj) = self.materialise_ctor_this() else { return };
        // the returned object is not released with the scopes
        if let Some(k) = self.this_key {
            self.env.insert(k, Slot::Ssa { v: obj, owned: false });
        }
        self.emit_return(Some(obj));
    }

}

// ------------------------------------------------------------------------------ pipeline

fn convert_wasm(d: &oxiscript_wasm::Diagnostics) -> Diagnostics {
    Diagnostics(
        d.0.iter()
            .map(|x| Diagnostic::new(x.code, x.to_string(), None))
            .collect(),
    )
}

/// Check, lower, verify and compile a program with imports.
pub fn compile_program(
    entry_name: &str,
    src: &str,
    loader: &mut dyn ModuleLoader,
    interfaces: &[Interface],
    opts: &Options,
) -> Result<CompileOutput, Diagnostics> {
    let mut ck = oxiscript_check::check_program(
        entry_name,
        src,
        loader,
        &CheckOptions {
            hosts: interfaces.to_vec(),
        },
    );
    if !ck.ok() {
        return Err(check_diags(&ck));
    }
    let Lowered { module, needs, notes } = lower_with(&mut ck, interfaces, opts.lower_all)?;
    let errs = ir::host::verify_with_interfaces(&module, ir::VerifyOptions::default(), interfaces);
    if !errs.is_empty() {
        return Err(Diagnostics(
            errs.iter()
                .take(20)
                .map(|e| Diagnostic::new("OXS-E0702", e.to_string(), None))
                .collect(),
        ));
    }
    let copts = oxiscript_wasm::CompileOptions {
        interfaces: interfaces.to_vec(),
        needs: Some(needs.clone()),
        runtime: opts.runtime,
        validate_output: opts.validate_output,
        ..oxiscript_wasm::CompileOptions::default()
    };
    let wasm = oxiscript_wasm::compile(&module, &copts).map_err(|d| convert_wasm(&d))?;
    Ok(CompileOutput { wasm, module, needs, notes })
}

/// Check, lower, verify and compile a single source file to Wasm bytes.
pub fn compile_source(src: &str, interfaces: &[Interface], opts: &Options) -> Result<Vec<u8>, Diagnostics> {
    compile_program("", src, &mut NoLoader, interfaces, opts).map(|o| o.wasm)
}
