//! OxiScript type checker (slice 8.T).
//!
//! # Mechanisms reused and written
//!
//! * **Bidirectional local type inference** in the style of Pierce & Turner
//!   ("Local Type Inference", 2000) and Dunfield & Krishnaswami ("Bidirectional
//!   Typing", 2021): every expression is checked in one of two modes, *synthesis*
//!   (`expr(e, None)`: the type comes from the expression) or *checking*
//!   (`expr(e, Some(expected))`: the expected type flows into literals, object
//!   literals, arrays, lambdas, `switch` arms and conditionals). Generic calls
//!   bind type parameters by one-way matching of parameter types against
//!   argument types, context-sensitive arguments (lambdas without parameter
//!   types) last, as TypeScript does. There are **no inference variables**, so
//!   no unification table is needed and `ena` is not used: the only type
//!   information that flows "backwards" is the expected type of an expression,
//!   which a parameter in the matcher already carries. If global inference or
//!   `let x = []` style variables are ever added, `ena` is the table to adopt.
//! * **Flow-sensitive narrowing** follows TypeScript's control-flow model: a
//!   `Flow` state (a flow node) holds facts about *reference paths* (a local,
//!   `this`, and field chains of value types), refined by conditions
//!   (`=== null`, `s.kind === "x"`, `switch`), killed by assignments, joined at
//!   merges. Unlike TypeScript it is restricted to cases that are sound for our
//!   types (see `flow.rs`).
//! * **Exhaustiveness** of `switch` is Maranget's usefulness algorithm
//!   ("Warnings for pattern matching", 2007) over constructor rows (`exhaust.rs`).
//! * Diagnostics mirror TypeScript's wording where the meaning is the same and
//!   always carry registry codes (`oxiscript-abi/registry/codes.txt`).
//!
//! # Entry points
//!
//! [`check_source`] checks one file, [`check_program`] a program with imports
//! through a [`ModuleLoader`]. The result is a [`Checked`] (`output.rs`): types
//! per expression, resolutions, narrowing, coercions, closure captures,
//! monomorphisation instances. A program with syntax errors is **not checked**:
//! only its parse diagnostics are returned.
//!
//! # Limits (bounded work)
//!
//! Types nest at most [`ty::MAX_TY_DEPTH`] levels and the type table holds at
//! most [`ty::MAX_TY_COUNT`] types; expression recursion is capped at
//! [`MAX_EXPR_DEPTH`]; long left-deep operator chains and member/call chains
//! are walked iteratively; unions have at most 64 variants; at most
//! [`MAX_ERRORS`] diagnostics are kept. Work is linear in the program size
//! except flow joins, which are linear in the number of live facts per branch.
#![forbid(unsafe_code)]
#![allow(
    clippy::assigning_clones,
    clippy::option_map_unit_fn,
    clippy::nonminimal_bool,
    clippy::while_let_loop,
    clippy::format_collect,
    clippy::format_push_string,
    clippy::unreadable_literal,
    clippy::needless_lifetimes,
    clippy::single_match,
    clippy::too_many_arguments,
    clippy::collapsible_match,
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::match_same_arms,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::wildcard_imports,
    clippy::struct_excessive_bools,
    clippy::large_enum_variant,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::manual_let_else,
    clippy::cognitive_complexity,
    clippy::type_complexity,
    clippy::unnecessary_wraps,
    clippy::if_not_else,
    clippy::map_unwrap_or,
    clippy::only_used_in_recursion,
    clippy::unused_self
)]

mod builtins;
mod calls;
mod decl;
mod exhaust;
mod expr;
mod flow;
mod inst;
mod stmt;
mod typeexpr;

pub mod output;
pub mod std_lib;
pub mod ty;

use oxiscript_abi::idl::Interface;
use oxiscript_syntax::*;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

pub use output::*;
pub use ty::{DefId, Interner, ParamId, Ty, TyKind};

/// Expression/statement recursion cap (`OXS-E0113` beyond it).
pub const MAX_EXPR_DEPTH: u32 = 120;
/// Nested on-demand declaration resolutions (`OXS-E0113` beyond it).
pub const MAX_SIG_DEPTH: u32 = 64;
/// Recursion budget one on-demand declaration resolution consumes (shared with expression depth).
pub(crate) const SIG_COST: u32 = 3;
/// Methods of one interface, inherited ones included (`OXS-E0113` beyond it).
pub const MAX_IFACE_METHODS: usize = 256;
/// Diagnostics kept before the checker stops recording.
pub const MAX_ERRORS: usize = 100;

pub const PRELUDE_SRC: &str = "\
type Result<T, E> = { kind: \"ok\"; value: T } | { kind: \"err\"; error: E };
function Ok<T, E>(value: T): Result<T, E> { return { kind: \"ok\", value }; }
function Err<T, E>(error: E): Result<T, E> { return { kind: \"err\", error }; }
";

/// Options for a check.
#[derive(Clone, Debug, Default)]
pub struct CheckOptions {
    /// Host capability interfaces (parsed IDL) that `import ... from "host:name"` may name.
    pub hosts: Vec<Interface>,
}

/// Supplies the source of imported modules.
pub trait ModuleLoader {
    /// `importer` is the canonical name of the importing module, `spec` the import string.
    /// Returns the imported module's canonical name and its source.
    fn load(&mut self, importer: &str, spec: &str) -> Option<(String, String)>;
    /// Called whenever `check_program` loads a bundled `std/...` module into a new `FileId` slot,
    /// so loaders that mirror `FileId`s (e.g. CLI source maps) stay in sync.
    fn on_std_loaded(&mut self, _name: &str, _src: &str) {}
}

/// A loader with no modules.
pub struct NoLoader;
impl ModuleLoader for NoLoader {
    fn load(&mut self, _: &str, _: &str) -> Option<(String, String)> {
        None
    }
}

/// An in-memory loader: canonical names map to sources. Relative specs
/// (`./x`, `../x`) are resolved against the importer's directory, with an
/// implied `.oxs` extension that is not part of the canonical name.
#[derive(Default)]
pub struct MapLoader(pub HashMap<String, String>);
impl ModuleLoader for MapLoader {
    fn load(&mut self, importer: &str, spec: &str) -> Option<(String, String)> {
        let name = if spec.starts_with("./") || spec.starts_with("../") {
            let mut parts: Vec<&str> = importer.split('/').collect();
            parts.pop();
            for seg in spec.split('/') {
                match seg {
                    "." | "" => {}
                    ".." => {
                        parts.pop();
                    }
                    s => parts.push(s),
                }
            }
            parts.join("/")
        } else {
            spec.to_string()
        };
        self.0.get(&name).map(|s| (name, s.clone()))
    }
}

/// Check one source file (no imports other than `host:`).
pub fn check_source(src: &str, opts: &CheckOptions) -> Checked {
    check_program("", src, &mut NoLoader, opts)
}

/// Check a program: the entry module `entry_name` and everything it imports.
pub fn check_program(
    entry_name: &str,
    entry_src: &str,
    loader: &mut dyn ModuleLoader,
    opts: &CheckOptions,
) -> Checked {
    let mut names: Vec<String> = vec![entry_name.to_string()];
    let mut srcs: Vec<String> = vec![entry_src.to_string()];
    let mut is_std: Vec<bool> = vec![false];
    let mut parsed: Vec<Parsed> = Vec::new();
    let mut targets: Vec<HashMap<String, usize>> = Vec::new();
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut by_name: HashMap<String, usize> = HashMap::new();
    by_name.insert(entry_name.to_string(), 0);
    let mut order: Vec<usize> = Vec::new();
    // iterative DFS over imports
    let mut i = 0;
    let mut state: Vec<u8> = vec![0];
    let mut stack: Vec<usize> = Vec::new();
    // modules are parsed in index order; discovery appends
    while i < names.len() {
        let p = parse(&srcs[i], FileId(i as u32));
        parsed.push(p);
        targets.push(HashMap::new());
        let specs: Vec<(String, Span)> = parsed[i]
            .module
            .items
            .iter()
            .filter_map(|it| match &it.kind {
                ItemKind::Import(im) => match &im.source.kind {
                    LitKind::Str(s) if !s.starts_with("host:") => Some((s.clone(), im.source.span)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        for (spec, span) in specs {
            if targets[i].contains_key(&spec) {
                continue;
            }
            if names.len() >= 256 {
                diags.push(Diagnostic::new(
                    "OXS-E0302",
                    span,
                    format!("cannot resolve import `{spec}`: too many modules"),
                ));
                continue;
            }
            if std_lib::is_std_spec(&spec) {
                match std_lib::source(&spec) {
                    Some(src) => {
                        let idx = if let Some(&k) = by_name.get(&spec) {
                            k
                        } else {
                            let k = names.len();
                            by_name.insert(spec.clone(), k);
                            names.push(spec.clone());
                            srcs.push(src.to_string());
                            is_std.push(true);
                            state.push(0);
                            loader.on_std_loaded(&spec, src);
                            k
                        };
                        targets[i].insert(spec, idx);
                    }
                    None => diags.push(Diagnostic::new(
                        "OXS-E0302",
                        span,
                        format!(
                            "cannot resolve import `{spec}`: unknown standard library module (available: {})",
                            std_lib::AVAILABLE_LIST
                        ),
                    )),
                }
                continue;
            }
            match loader.load(&names[i], &spec) {
                Some((canon, src)) => {
                    let idx = if let Some(&k) = by_name.get(&canon) {
                        k
                    } else {
                        let k = names.len();
                        by_name.insert(canon.clone(), k);
                        names.push(canon);
                        srcs.push(src);
                        is_std.push(false);
                        state.push(0);
                        k
                    };
                    targets[i].insert(spec, idx);
                }
                None => diags.push(Diagnostic::new(
                    "OXS-E0302",
                    span,
                    format!("cannot resolve import `{spec}`: module not found"),
                )),
            }
        }
        i += 1;
    }
    // post-order with cycle detection
    let n = names.len();
    let mut cyc: Vec<(usize, String, Span)> = Vec::new();
    for root in 0..n {
        if state[root] != 0 {
            continue;
        }
        stack.push(root);
        let mut iters: Vec<(usize, Vec<usize>)> = Vec::new();
        state[root] = 1;
        let mut deps0: Vec<usize> = targets[root].values().copied().collect();
        deps0.sort_unstable();
        iters.push((root, deps0));
        stack.clear();
        while let Some((m, deps)) = iters.last_mut() {
            if let Some(d) = deps.pop() {
                let m = *m;
                match state[d] {
                    0 => {
                        state[d] = 1;
                        let mut dd: Vec<usize> = targets[d].values().copied().collect();
                        dd.sort_unstable();
                        iters.push((d, dd));
                    }
                    1 => {
                        let spec = targets[m]
                            .iter()
                            .find(|(_, v)| **v == d)
                            .map(|(k, _)| k.clone())
                            .unwrap_or_default();
                        cyc.push((m, spec, Span::DUMMY));
                    }
                    _ => {}
                }
            } else {
                let m = *m;
                state[m] = 2;
                order.push(m);
                iters.pop();
            }
        }
    }
    for p in &parsed {
        diags.extend(p.diagnostics.iter().cloned());
    }
    for (m, spec, _) in &cyc {
        // locate the import for a span
        let mut span = Span::new(FileId(*m as u32), 0, 0);
        for it in &parsed[*m].module.items {
            if let ItemKind::Import(im) = &it.kind {
                if matches!(&im.source.kind, LitKind::Str(s) if s == spec) {
                    span = im.source.span;
                }
            }
        }
        diags.push(Diagnostic::new(
            "OXS-E0302",
            span,
            format!("cannot resolve import `{spec}`: cyclic import"),
        ));
    }
    if diags.iter().any(Diagnostic::is_error) {
        return Checked::syntax_only(diags);
    }
    let prelude_idx = n;
    let prelude = parse(PRELUDE_SRC, FileId(prelude_idx as u32));
    debug_assert_eq!(prelude.diagnostics.len(), 0, "prelude must parse cleanly");
    let mut ck = Checker::new(opts, &srcs, PRELUDE_SRC);
    let mut asts: Vec<&Module> = parsed.iter().map(|p| &p.module).collect();
    asts.push(&prelude.module);
    let mut all_names = names.clone();
    all_names.push("std/prelude".to_string());
    is_std.push(true);
    let mut check_order = vec![prelude_idx];
    check_order.extend(order.iter().copied());
    ck.run(&asts, &all_names, &is_std, &targets, &check_order);
    let mut checked = ck.finish(diags);
    checked.sources = srcs.clone();
    checked.sources.push(PRELUDE_SRC.to_string());
    checked.order = check_order.iter().map(|&k| k as u32).collect();
    checked
}

// ----------------------------------------------------------------------------
// internal state
// ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Binding {
    Def(DefId),
    Local(LocalId),
    TParam(ParamId),
    /// A builtin type name (`i32`, `string`, `void`, ...).
    Prim(Ty),
    /// `Array`
    ArrayCtor,
    /// Builtin value (`range`).
    BuiltinFn(Builtin),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Sig {
    Pending,
    InProgress,
    Done,
}

#[derive(Clone, Copy)]
pub(crate) enum Ast<'a> {
    None,
    Item(&'a Item),
    /// Member of a record; also holds the record declaration.
    Member(&'a Member, &'a RecordDecl),
    /// A top-level union alias member list element or object alias.
    Obj(&'a TypeExpr),
    /// Interface method signature.
    #[allow(dead_code)]
    IfaceMethod(&'a FnDecl),
}

pub(crate) struct DefData<'a> {
    pub info: DefInfo,
    pub ast: Ast<'a>,
    pub sig: Sig,
    /// Scope names of this def's type parameters (owner's first).
    pub body_done: bool,
}

pub(crate) struct ModData<'a> {
    pub ast: Option<&'a Module>,
    pub top: HashMap<String, Binding>,
    pub exports: HashMap<String, DefId>,
    pub targets: HashMap<String, usize>,
    pub out: ModuleInfo,
    /// Parallel to `out.locals`: index of the function context that declared it.
    pub local_fn: Vec<u32>,
    #[allow(dead_code)]
    pub is_prelude: bool,
    pub is_std: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct LoopCtx {
    pub label: Option<String>,
    pub is_switch: bool,
    pub is_loop: bool,
    pub breaks: Vec<flow::Flow>,
    pub brk: bool,
}

pub(crate) struct FnCtx {
    /// Declared return type, or the expected type for a closure; `None` while inferring.
    pub ret: Option<Ty>,
    pub inferred: Option<Ty>,
    pub is_closure: bool,
    pub is_ctor: bool,
    pub this: Option<(LocalId, Ty)>,
    pub this_is_value: bool,
    pub mutating: bool,
    pub owner: Option<DefId>,
    pub loops: Vec<LoopCtx>,
    pub captures: Vec<LocalId>,
    #[allow(dead_code)]
    pub name: String,
    pub name_span: Option<Span>,
    pub is_law: bool,
    pub tparams: Vec<ParamId>,
    /// Names assigned anywhere in the outermost function body / inside closures.
    pub assigned: Rc<HashSet<String>>,
    pub closure_assigned: Rc<HashSet<String>>,
    pub closure_recv: Rc<HashSet<String>>,
    pub has_ret_value: bool,
    pub exported: bool,
    /// The function whose effects this body contributes to (closures share their enclosing function).
    pub def: Option<DefId>,
}

pub(crate) struct Checker<'a> {
    pub tys: Interner,
    pub defs: Vec<DefData<'a>>,
    pub params: Vec<TypeParamInfo>,
    pub mods: Vec<ModData<'a>>,
    pub prelude: usize,
    pub records: HashMap<DefId, RecordSig>,
    pub unions: HashMap<DefId, UnionSig>,
    pub enums: HashMap<DefId, EnumSig>,
    pub ifaces: HashMap<DefId, IfaceSig>,
    pub fns: HashMap<DefId, FnSig>,
    pub globals: HashMap<DefId, GlobalSig>,
    pub aliases: HashMap<DefId, Ty>,
    pub hosts: HashMap<DefId, HostSig>,
    pub host_defs: HashMap<String, DefId>,
    pub host_idl: &'a [Interface],
    pub instances: Vec<Instance>,
    pub inst_map: HashMap<Instance, u32>,
    pub diags: Vec<Diagnostic>,
    pub srcs: Vec<&'a str>,
    pub too_many: bool,
    // ---- current context
    pub cm: usize,
    pub scopes: Vec<HashMap<String, Binding>>,
    pub fnctx: Vec<FnCtx>,
    /// Local definitions whose signatures are being resolved (innermost last).
    pub local_sig: Vec<DefId>,
    pub flow: flow::Flow,
    pub paths: flow::PathTable,
    pub depth: u32,
    pub sig_depth: u32,
    pub bparams: [ParamId; 3],
    pub entry: Option<DefId>,
    pub pending_bounds: Vec<(DefId, Vec<Ty>, Span, usize)>,
    pub this_member_ok: bool,
    pub assign_target: bool,
    pub skip_unassigned: bool,
    /// `fnctx.len()` while a default-parameter expression is being checked (`?` is rejected there).
    pub default_depth: Option<usize>,
    pub pending_label: Option<String>,
    pub eff: HashMap<DefId, EffRec>,
    pub(crate) inst_edges: Vec<inst::InstEdge>,
    /// The module global / static whose initialiser is being checked.
    pub(crate) cur_init: Option<inst::GKey>,
    pub(crate) init_recs: HashMap<inst::GKey, InitRec>,
    pub memo: HashMap<NodeId, Ty>,
    pub effects: HashMap<DefId, EffectSummary>,
}

#[derive(Default, Clone)]
pub(crate) struct EffRec {
    pub mask: u8,
    pub edges: Vec<(DefId, Span)>,
    /// Module globals and static fields this function reads directly (init-order analysis).
    pub reads: Vec<inst::GKey>,
}

/// What the initialiser of one module global / static field reads and calls.
#[derive(Default, Clone)]
pub(crate) struct InitRec {
    pub reads: Vec<(inst::GKey, Span)>,
    pub calls: Vec<(DefId, Span)>,
}

static REGISTRY: OnceLock<HashMap<String, (String, Option<String>)>> = OnceLock::new();

fn registry() -> &'static HashMap<String, (String, Option<String>)> {
    REGISTRY.get_or_init(|| {
        oxiscript_abi::codes::seed()
            .into_iter()
            .map(|c| (c.code, (c.message, c.fix)))
            .collect()
    })
}

impl Checked {
    fn syntax_only(diagnostics: Vec<Diagnostic>) -> Checked {
        Checked {
            diagnostics,
            tys: Interner::new(),
            defs: Vec::new(),
            params: Vec::new(),
            modules: Vec::new(),
            records: HashMap::new(),
            unions: HashMap::new(),
            enums: HashMap::new(),
            ifaces: HashMap::new(),
            fns: HashMap::new(),
            globals: HashMap::new(),
            aliases: HashMap::new(),
            hosts: HashMap::new(),
            instances: Vec::new(),
            effects: HashMap::new(),
            entry: None,
            prelude: None,
            sources: Vec::new(),
            order: Vec::new(),
        }
    }
}

impl<'a> Checker<'a> {
    fn new(opts: &'a CheckOptions, srcs: &'a [String], prelude_src: &'a str) -> Checker<'a> {
        let mut s: Vec<&'a str> = srcs.iter().map(String::as_str).collect();
        s.push(prelude_src);
        let mut ck = Checker {
            tys: Interner::new(),
            defs: Vec::new(),
            params: Vec::new(),
            mods: Vec::new(),
            prelude: 0,
            records: HashMap::new(),
            unions: HashMap::new(),
            enums: HashMap::new(),
            ifaces: HashMap::new(),
            fns: HashMap::new(),
            globals: HashMap::new(),
            aliases: HashMap::new(),
            hosts: HashMap::new(),
            host_defs: HashMap::new(),
            host_idl: &opts.hosts,
            instances: Vec::new(),
            inst_map: HashMap::new(),
            diags: Vec::new(),
            srcs: s,
            too_many: false,
            cm: 0,
            scopes: Vec::new(),
            fnctx: Vec::new(),
            local_sig: Vec::new(),
            flow: flow::Flow::default(),
            paths: flow::PathTable::default(),
            depth: 0,
            sig_depth: 0,
            bparams: [ParamId(0); 3],
            entry: None,
            pending_bounds: Vec::new(),
            this_member_ok: false,
            assign_target: false,
            skip_unassigned: false,
            default_depth: None,
            pending_label: None,
            eff: HashMap::new(),
            inst_edges: Vec::new(),
            cur_init: None,
            init_recs: HashMap::new(),
            memo: HashMap::new(),
            effects: HashMap::new(),
        };
        let mut ids = [ParamId(0); 3];
        for (k, nm) in ["U", "V", "W"].iter().enumerate() {
            ids[k] = ck.new_param(nm, None, 0);
        }
        ck.bparams = ids;
        ck
    }

    pub fn new_param(&mut self, name: &str, owner: Option<DefId>, index: u32) -> ParamId {
        self.params.push(TypeParamInfo {
            name: name.to_string(),
            bound: None,
            owner,
            index,
        });
        ParamId(u32::try_from(self.params.len() - 1).unwrap_or(u32::MAX))
    }

    // ---- diagnostics ------------------------------------------------------

    /// Report `code` at `span`; the message is the registry template filled with `args`.
    pub fn err(&mut self, code: &'static str, span: Span, args: &[(&str, &str)]) -> usize {
        self.err_fix(code, span, args, None)
    }

    pub fn err_fix(
        &mut self,
        code: &'static str,
        span: Span,
        args: &[(&str, &str)],
        fix: Option<Fix>,
    ) -> usize {
        if self.diags.len() >= MAX_ERRORS {
            if !self.too_many {
                self.too_many = true;
                self.diags.push(Diagnostic::new(
                    "OXS-E0113",
                    span,
                    format!("more than {MAX_ERRORS} diagnostics; giving up on this program"),
                ));
            }
            return self.diags.len() - 1;
        }
        let (msg, fixt) = registry()
            .get(code)
            .map(|(m, f)| (m.clone(), f.clone()))
            .unwrap_or_default();
        let mut d = Diagnostic::new(code, span, oxiscript_abi::codes::render(&msg, args));
        if let Some(f) = fix {
            d.fixes.push(f);
        } else if let Some(t) = fixt {
            d.fixes.push(Fix {
                title: oxiscript_abi::codes::render(&t, args),
                edits: Vec::new(),
            });
        }
        self.diags.push(d);
        self.diags.len() - 1
    }

    /// `OXS-E0100` mismatch with a TypeScript-style message.
    pub fn mismatch(&mut self, span: Span, expected: Ty, found: Ty) {
        if self.tys.has_error(expected) || self.tys.has_error(found) {
            return;
        }
        let (e, f) = (self.show(expected), self.show(found));
        self.err(
            "OXS-E0100",
            span,
            &[("expected", &e), ("found", &f)],
        );
    }

    pub fn span_text(&self, span: Span) -> &'a str {
        let s = self.srcs.get(span.file.0 as usize).copied().unwrap_or("");
        s.get(span.lo as usize..span.hi as usize).unwrap_or("")
    }

    pub fn clip_snippet(&self, span: Span) -> String {
        let raw = self.span_text(span).trim();
        let flat: String = raw
            .chars()
            .map(|c| if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c })
            .take(40)
            .collect();
        if raw.chars().count() > 40 {
            format!("{flat}...")
        } else {
            flat
        }
    }

    // ---- type display -----------------------------------------------------

    pub fn show(&self, t: Ty) -> String {
        let mut s = String::new();
        self.show_into(t, &mut s, 0);
        s
    }

    fn show_into(&self, t: Ty, out: &mut String, depth: u32) {
        if depth > 12 {
            out.push_str("...");
            return;
        }
        match self.tys.kind(t) {
            TyKind::Error => out.push_str("<error>"),
            TyKind::Never => out.push_str("never"),
            TyKind::Void => out.push_str("void"),
            TyKind::Bool => out.push_str("bool"),
            TyKind::Num(n) => out.push_str(n.name()),
            TyKind::Str => out.push_str("string"),
            TyKind::Null => out.push_str("null"),
            TyKind::Array(e) => {
                let paren = matches!(self.tys.kind(*e), TyKind::Fn(..) | TyKind::Nullable(_));
                if paren {
                    out.push('(');
                }
                self.show_into(*e, out, depth + 1);
                if paren {
                    out.push(')');
                }
                out.push_str("[]");
            }
            TyKind::Nullable(e) => {
                self.show_into(*e, out, depth + 1);
                out.push_str(" | null");
            }
            TyKind::Adt(d, a) => {
                let info = &self.defs[d.0 as usize].info;
                if let DefKind::Variant { union, .. } = info.kind {
                    if let Some(u) = union {
                        out.push_str(&self.defs[u.0 as usize].info.name);
                        out.push('#');
                    }
                    out.push_str(&info.name);
                    // variants print without arguments unless they have their own
                    if !a.is_empty() && union.is_none() {
                        self.show_args(a, out, depth);
                    }
                } else {
                    out.push_str(&info.name);
                    self.show_args(a, out, depth);
                }
            }
            TyKind::Variants(d, a, m) => {
                let mut first = true;
                let names: Vec<String> = self
                    .unions
                    .get(d)
                    .map(|u| {
                        u.variants
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| m & (1u64 << i) != 0)
                            .map(|(_, v)| self.defs[v.def.0 as usize].info.name.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                for n in names {
                    if !first {
                        out.push_str(" | ");
                    }
                    first = false;
                    out.push_str(&self.defs[d.0 as usize].info.name);
                    out.push('#');
                    out.push_str(&n);
                }
                let _ = a;
            }
            TyKind::Param(p) => out.push_str(&self.params[p.0 as usize].name),
            TyKind::Fn(p, r) => {
                out.push('(');
                for (i, x) in p.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&format!("p{i}: "));
                    self.show_into(*x, out, depth + 1);
                }
                out.push_str(") => ");
                self.show_into(*r, out, depth + 1);
            }
        }
    }

    fn show_args(&self, a: &[Ty], out: &mut String, depth: u32) {
        if a.is_empty() {
            return;
        }
        out.push('<');
        for (i, x) in a.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            self.show_into(*x, out, depth + 1);
        }
        out.push('>');
    }

    // ---- module tables ----------------------------------------------------

    pub fn set_ty(&mut self, id: NodeId, t: Ty) {
        let o = &mut self.mods[self.cm].out.expr_ty;
        let i = id.0 as usize;
        if o.len() <= i {
            o.resize(i + 1, ty::ERROR);
        }
        o[i] = t;
    }

    pub fn set_res(&mut self, id: NodeId, r: Res) {
        self.mods[self.cm].out.res.insert(id, r);
    }

    pub fn new_local(&mut self, name: &str, ty: Ty, kind: LocalKind, span: Span, node: NodeId) -> LocalId {
        let fnidx = u32::try_from(self.fnctx.len().saturating_sub(1)).unwrap_or(0);
        let m = &mut self.mods[self.cm];
        m.out.locals.push(LocalInfo {
            name: name.to_string(),
            ty,
            kind,
            span,
            captured: false,
            mutated: false,
            used: false,
        });
        m.local_fn.push(fnidx);
        let id = LocalId(u32::try_from(m.out.locals.len() - 1).unwrap_or(u32::MAX));
        if node.0 < u32::MAX - 1 {
            m.out.binding_local.insert(node, id);
        }
        id
    }

    pub fn def_name(&self, d: DefId) -> &str {
        &self.defs[d.0 as usize].info.name
    }

    pub fn new_def(
        &mut self,
        name: &str,
        kind: DefKind,
        span: Span,
        node: NodeId,
        exported: bool,
        tparams: Vec<ParamId>,
        owner: Option<DefId>,
        ast: Ast<'a>,
    ) -> DefId {
        let local = !self.fnctx.is_empty();
        self.defs.push(DefData {
            info: DefInfo {
                name: name.to_string(),
                kind,
                module: self.cm as u32,
                span,
                node,
                exported,
                tparams,
                owner,
                target_module: None,
                local,
            },
            ast,
            sig: Sig::Pending,
            body_done: false,
        });
        let id = DefId(u32::try_from(self.defs.len() - 1).unwrap_or(u32::MAX));
        if node != NodeId(u32::MAX) {
            self.mods[self.cm].out.node_def.insert(node, id);
        }
        id
    }

    pub fn kind_of(&self, d: DefId) -> DefKind {
        self.defs[d.0 as usize].info.kind
    }

    // ---- driver -----------------------------------------------------------

    fn run(
        &mut self,
        asts: &[&'a Module],
        names: &[String],
        is_std: &[bool],
        targets: &[HashMap<String, usize>],
        order: &[usize],
    ) {
        let total = asts.len();
        for (fid, ast) in asts.iter().enumerate() {
            let std_mod = is_std.get(fid).copied().unwrap_or(false);
            self.mods.push(ModData {
                ast: Some(ast),
                top: HashMap::new(),
                exports: HashMap::new(),
                targets: targets.get(fid).cloned().unwrap_or_default(),
                out: ModuleInfo {
                    name: names[fid].clone(),
                    file: FileId(fid as u32),
                    is_std: std_mod,
                    items: Vec::new(),
                    expr_ty: Vec::new(),
                    res: HashMap::new(),
                    locals: Vec::new(),
                    binding_local: HashMap::new(),
                    node_def: HashMap::new(),
                    coercions: HashMap::new(),
                    calls: HashMap::new(),
                    closures: HashMap::new(),
                    switches: HashMap::new(),
                    object_lits: HashMap::new(),
                    casts: HashMap::new(),
                    main: None,
                    attrs: HashMap::new(),
                    this_locals: HashMap::new(),
                },
                local_fn: Vec::new(),
                is_prelude: fid == total - 1,
                is_std: std_mod,
            });
        }
        self.prelude = total - 1;
        for &k in order {
            self.cm = k;
            self.declare_module(k);
        }
        for &k in order {
            self.cm = k;
            self.resolve_module(k);
        }
        for &k in order {
            self.cm = k;
            self.check_module_bodies(k);
        }
        self.check_instantiation_cycles();
        self.check_init_order(order);
        self.compute_effects();
        if !self.diags.iter().any(Diagnostic::is_error) {
            for &k in order {
                if self.mods[k].is_std || self.mods[k].is_prelude {
                    continue;
                }
                let unused: Vec<(Span, String)> = self.mods[k]
                    .out
                    .locals
                    .iter()
                    .filter(|l| {
                        matches!(l.kind, LocalKind::Let | LocalKind::Const | LocalKind::ForOf)
                            && !l.used
                            && !l.name.is_empty()
                            && !l.name.starts_with('_')
                    })
                    .map(|l| (l.span, l.name.clone()))
                    .collect();
                for (sp, name) in unused {
                    self.err("OXS-W0001", sp, &[("name", &name)]);
                }
            }
        }
        if self.tys.overflowed {
            self.err(
                "OXS-E0113",
                Span::DUMMY,
                &[("detail", "a type is nested too deeply or the program creates too many types")],
            );
        }
    }

    fn finish(mut self, mut diags: Vec<Diagnostic>) -> Checked {
        diags.append(&mut self.diags);
        let prelude = u32::try_from(self.prelude).ok();
        for m in &mut self.mods {
            let locals = &m.out.locals;
            for c in m.out.closures.values_mut() {
                for cap in &mut c.captures {
                    cap.mutated = locals[cap.local.0 as usize].mutated;
                }
            }
        }
        let had_errors = diags.iter().any(Diagnostic::is_error);
        let mut checked = Checked {
            effects: std::mem::take(&mut self.effects),
            diagnostics: diags,
            tys: self.tys,
            defs: self.defs.into_iter().map(|d| d.info).collect(),
            params: self.params,
            modules: self.mods.into_iter().map(|m| m.out).collect(),
            records: self.records,
            unions: self.unions,
            enums: self.enums,
            ifaces: self.ifaces,
            fns: self.fns,
            globals: self.globals,
            aliases: self.aliases,
            hosts: self.hosts,
            instances: self.instances,
            entry: self.entry,
            prelude,
            sources: Vec::new(),
            order: Vec::new(),
        };
        if !had_errors {
            if let Some(what) = checked.poisoned_table() {
                let msg = registry().get("OXS-E0990").map(|(m, _)| m.clone()).unwrap_or_default();
                checked.diagnostics.push(Diagnostic::new("OXS-E0990", Span::DUMMY, oxiscript_abi::codes::render(&msg, &[("detail", what)])));
            }
        }
        checked
    }
}
