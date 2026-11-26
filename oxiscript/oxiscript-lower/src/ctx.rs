//! Global lowering state: the checked program, the re-parsed ASTs, the output module
//! under construction, caches (types, symbols, constants, imports) and the
//! monomorphisation worklist.

use crate::{Diagnostic, MAX_INSTANCES};
use oxiscript_abi::idl::{IdlFn, Interface};
use oxiscript_check::{Checked, DefId, DefKind, ParamId, Ty};
use oxiscript_ir::{self as ir, Ty as It};
use oxiscript_syntax::visit::{walk_item, walk_member, Visitor};
use oxiscript_syntax::{FieldDecl, FnDecl, Item, ItemKind, Member, MemberKind, Module, RecordDecl, Span};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Copy)]
pub(crate) enum DefAst<'a> {
    Fn(&'a FnDecl),
    Var,
    Record(&'a RecordDecl),
    #[allow(dead_code)]
    Field(&'a FieldDecl),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum GKey {
    Global(DefId),
    Static(DefId, u32),
}

#[derive(Clone, Debug)]
pub(crate) struct GlobalIr {
    pub sym: String,
    /// IR type of the value (as seen by the program).
    pub vty: It,
    /// Managed non-nullable values are stored as `T?` and `unopt`ed on read.
    pub wrapped: bool,
}

pub(crate) enum Job {
    Fn { def: DefId, targs: Vec<Ty>, sym: String },
}

pub(crate) struct Lw<'c, 'a> {
    pub ck: &'c mut Checked,
    pub asts: &'a [Module],
    pub out: ir::Module,
    pub diags: Vec<Diagnostic>,
    /// Warnings (OXS-W07xx): the program is fine, something was dropped.
    pub notes: Vec<Diagnostic>,
    pub defast: HashMap<DefId, DefAst<'a>>,
    pub interfaces: Vec<Interface>,
    pub repr_cache: HashMap<Ty, Option<It>>,
    pub struct_names: HashMap<(DefId, Vec<Ty>), String>,
    pub union_names: HashMap<(DefId, Vec<Ty>), String>,
    pub union_variants: HashMap<String, Vec<String>>,
    pub type_idx: HashMap<String, usize>,
    pub boxes: HashMap<String, String>,
    pub cells: HashMap<String, String>,
    pub data: HashMap<String, String>,
    pub imports: HashMap<(String, String), String>,
    pub fn_syms: HashMap<(DefId, Vec<Ty>), String>,
    pub thunks: HashMap<String, String>,
    pub jobs: VecDeque<Job>,
    pub globals: HashMap<GKey, GlobalIr>,
    pub counter: u32,
    pub instances: usize,
    pub max_instances: usize,
    pub helper_done: HashSet<String>,
    pub budget_hit: bool,
    pub lower_all: bool,
}

pub(crate) fn sanitize(s: &str) -> String {
    let mut o: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if o.is_empty() {
        o.push('x');
    }
    o
}

struct Indexer<'a, 'c> {
    ck: &'c Checked,
    m: usize,
    out: &'c mut HashMap<DefId, DefAst<'a>>,
}

impl<'a, 'c> Visitor<'a> for Indexer<'a, 'c> {
    fn visit_item(&mut self, i: &'a Item) {
        if let Some(&d) = self.ck.modules[self.m].node_def.get(&i.id) {
            match &i.kind {
                ItemKind::Fn(f) => {
                    self.out.insert(d, DefAst::Fn(f));
                }
                ItemKind::Var(_) => {
                    self.out.insert(d, DefAst::Var);
                }
                ItemKind::Record(r) => {
                    self.out.insert(d, DefAst::Record(r));
                }
                _ => {}
            }
        }
        walk_item(self, i);
    }
    fn visit_member(&mut self, mem: &'a Member) {
        if let Some(&d) = self.ck.modules[self.m].node_def.get(&mem.id) {
            match &mem.kind {
                MemberKind::Method(f) | MemberKind::Constructor(f) => {
                    self.out.insert(d, DefAst::Fn(f));
                }
                MemberKind::Field(f) => {
                    self.out.insert(d, DefAst::Field(f));
                }
            }
        }
        walk_member(self, mem);
    }
}

impl<'c, 'a> Lw<'c, 'a> {
    pub fn new(ck: &'c mut Checked, asts: &'a [Module], interfaces: Vec<Interface>, name: &str) -> Lw<'c, 'a> {
        let mut defast = HashMap::new();
        for (m, ast) in asts.iter().enumerate() {
            let mut ix = Indexer {
                ck: &*ck,
                m,
                out: &mut defast,
            };
            for it in &ast.items {
                ix.visit_item(it);
            }
        }
        Lw {
            ck,
            asts,
            out: ir::Module {
                name: name.to_string(),
                ..ir::Module::default()
            },
            diags: Vec::new(),
            notes: Vec::new(),
            defast,
            interfaces,
            repr_cache: HashMap::new(),
            struct_names: HashMap::new(),
            union_names: HashMap::new(),
            union_variants: HashMap::new(),
            type_idx: HashMap::new(),
            boxes: HashMap::new(),
            cells: HashMap::new(),
            data: HashMap::new(),
            imports: HashMap::new(),
            fn_syms: HashMap::new(),
            thunks: HashMap::new(),
            jobs: VecDeque::new(),
            globals: HashMap::new(),
            counter: 0,
            instances: 0,
            max_instances: MAX_INSTANCES,
            helper_done: HashSet::new(),
            budget_hit: false,
            lower_all: false,
        }
    }

    /// Add a type declaration, indexing it by name.
    pub fn push_type(&mut self, decl: ir::TypeDecl) -> usize {
        self.out.types.push(decl);
        let i = self.out.types.len() - 1;
        self.type_idx.insert(self.out.types[i].name.clone(), i);
        i
    }

    pub fn type_decl(&self, name: &str) -> Option<&ir::TypeDecl> {
        self.type_idx.get(name).map(|&i| &self.out.types[i])
    }

    pub fn next_id(&mut self) -> u32 {
        self.counter += 1;
        self.counter
    }

    pub fn err(&mut self, code: &str, msg: impl Into<String>, span: Option<Span>) {
        if self.diags.len() < 100 {
            self.diags.push(Diagnostic::new(code, msg, span));
        }
    }

    pub fn unsupported(&mut self, what: &str, span: Option<Span>) {
        self.err("OXS-E0700", what.to_string(), span);
    }

    pub fn def_name(&self, d: DefId) -> String {
        self.ck.defs[d.0 as usize].name.clone()
    }

    pub fn subst(&mut self, t: Ty, map: &[(ParamId, Ty)]) -> Ty {
        self.ck.tys.subst(t, map)
    }

    pub fn tparam_map(&self, d: DefId, args: &[Ty]) -> Vec<(ParamId, Ty)> {
        self.ck.defs[d.0 as usize]
            .tparams
            .iter()
            .copied()
            .zip(args.iter().copied())
            .collect()
    }

    /// Count a generic instantiation against the budget.
    pub fn count_instance(&mut self, what: &str) -> bool {
        self.instances += 1;
        if self.instances > self.max_instances {
            if !self.budget_hit {
                self.budget_hit = true;
                self.err(
                    "OXS-E0701",
                    format!(
                        "more than {} instantiations (last: {what})",
                        self.max_instances
                    ),
                    None,
                );
            }
            return false;
        }
        true
    }

    pub fn str_data(&mut self, s: &str) -> String {
        if let Some(n) = self.data.get(s) {
            return n.clone();
        }
        let name = format!("s{}", self.data.len());
        self.out.data.push(ir::DataDecl {
            name: name.clone(),
            text: s.to_string(),
        });
        self.data.insert(s.to_string(), name.clone());
        name
    }

    pub fn host_import(&mut self, iface: usize, f: &IdlFn) -> String {
        let itf = self.interfaces[iface].clone();
        let key = (itf.import_module(), f.name.clone());
        if let Some(n) = self.imports.get(&key) {
            return n.clone();
        }
        let name = format!("{}.{}", sanitize(&itf.import_module()), sanitize(&f.name));
        self.out.imports.push(ir::Import {
            name: name.clone(),
            module: itf.import_module(),
            field: f.name.clone(),
            sig: ir::host::import_sig(f, &itf),
        });
        self.imports.insert(key, name.clone());
        name
    }

    pub fn find_interface(&self, module: &str, version: u32) -> Option<usize> {
        self.interfaces
            .iter()
            .position(|i| i.module == module && i.version == version)
    }

    /// Symbol of the instance `def<targs>` (queues its lowering the first time).
    pub fn fn_sym(&mut self, def: DefId, targs: Vec<Ty>) -> String {
        let key = (def, targs);
        if let Some(s) = self.fn_syms.get(&key) {
            return s.clone();
        }
        let id = self.next_id();
        let name = sanitize(&self.def_name(def));
        let owner = self.ck.defs[def.0 as usize]
            .owner
            .map(|o| format!("{}_", sanitize(&self.def_name(o))))
            .unwrap_or_default();
        let sym = format!("{owner}{name}.{id}");
        if !key.1.is_empty() {
            self.count_instance(&sym);
        }
        self.fn_syms.insert(key.clone(), sym.clone());
        self.jobs.push_back(Job::Fn {
            def,
            targs: key.1,
            sym: sym.clone(),
        });
        sym
    }

    pub fn global_ir(&mut self, key: GKey, ty: Ty) -> GlobalIr {
        if let Some(g) = self.globals.get(&key) {
            return g.clone();
        }
        let vty = self.repr(ty).unwrap_or(It::I32);
        let wrapped = vty.is_managed() && !matches!(vty, It::Opt(_));
        let sty = if wrapped { It::Opt(Box::new(vty.clone())) } else { vty.clone() };
        let id = self.next_id();
        let base = match key {
            GKey::Global(d) => sanitize(&self.def_name(d)),
            GKey::Static(d, i) => {
                let f = self.ck.records[&d].fields[i as usize].name.clone();
                format!("{}_{}", sanitize(&self.def_name(d)), sanitize(&f))
            }
        };
        let sym = format!("g_{base}.{id}");
        self.out.globals.push(ir::Global {
            name: sym.clone(),
            ty: sty,
            init: None,
        });
        let g = GlobalIr { sym, vty, wrapped };
        self.globals.insert(key, g.clone());
        g
    }

    pub fn kind_of(&self, d: DefId) -> DefKind {
        self.ck.defs[d.0 as usize].kind
    }
}
