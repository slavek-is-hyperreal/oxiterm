//! The checked program: the contract 8.L (lowering) builds on.
//!
//! Everything is keyed by [`DefId`], [`ParamId`], [`LocalId`] or the syntax
//! crate's per-module `NodeId`; nothing borrows the AST, so a `Checked` can
//! outlive the parse trees (8.L re-walks the same `Module`s and looks nodes up
//! in the side tables of the module with the same index).

use crate::ty::{DefId, Interner, ParamId, Ty};
use oxiscript_syntax::{Diagnostic, FileId, NodeId, NumTy, Span};
use std::collections::HashMap;

/// A local variable, parameter or pattern binding; unique within its module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefKind {
    Struct,
    Class,
    /// Discriminated union (sum type); its variants are `Variant` defs.
    Union,
    /// A union member (`Shape#circle`). `union` is set once the owning union is resolved.
    Variant {
        union: Option<DefId>,
        index: u32,
    },
    /// C-like enumeration.
    Enum,
    /// Interface (generic bound only).
    Interface,
    /// `type A = T;` (expanded on use; never appears in a `Ty`).
    Alias,
    /// Host capability type (an IDL interface).
    Host,
    /// Host handle type declared inside an IDL interface.
    Handle,
    /// Free function (including nested functions and `Ok`/`Err` of the prelude).
    Fn,
    /// Method of a struct or class (`owner` in [`DefInfo`]).
    Method,
    /// Class constructor.
    Ctor,
    /// Module-level `let`/`const`.
    Global,
    Law,
    /// `import * as ns`; `module` index in `DefInfo::target_module`.
    Namespace,
}

#[derive(Clone, Debug)]
pub struct DefInfo {
    pub name: String,
    pub kind: DefKind,
    /// Index into [`Checked::modules`].
    pub module: u32,
    pub span: Span,
    /// The syntax node of the declaring item or member (`Item::id` / `Member::id`).
    pub node: NodeId,
    pub exported: bool,
    /// Type parameters in scope for this definition. For methods and
    /// constructors the owner's parameters come first.
    pub tparams: Vec<ParamId>,
    /// Struct/class for methods and constructors; interface/host for IDL members.
    pub owner: Option<DefId>,
    /// For `Namespace`: the imported module.
    pub target_module: Option<u32>,
    /// Declared in a function body (nested item) rather than at module level.
    pub local: bool,
}

#[derive(Clone, Debug)]
pub struct TypeParamInfo {
    pub name: String,
    /// Interface bound (`T extends Shape`) as an `Adt` type.
    pub bound: Option<Ty>,
    pub owner: Option<DefId>,
    pub index: u32,
}

#[derive(Clone, Debug)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Ty,
    pub readonly: bool,
    pub private: bool,
    pub is_static: bool,
    pub has_init: bool,
    pub span: Span,
}

/// The value of a union discriminant (`kind: "circle"`).
#[derive(Clone, Debug, PartialEq)]
pub enum Discr {
    Str(String),
    Int(i128),
    Bool(bool),
}

#[derive(Clone, Debug)]
pub struct RecordSig {
    pub is_class: bool,
    /// Instance and static fields in declaration order. For a union variant the
    /// discriminant field is **not** listed (it is derived from the tag).
    pub fields: Vec<FieldInfo>,
    pub methods: Vec<(String, DefId)>,
    pub ctor: Option<DefId>,
    pub implements: Vec<Ty>,
    /// For union variants: the discriminant field name and value.
    pub discr: Option<(String, Discr)>,
}

#[derive(Clone, Debug)]
pub struct UnionVariant {
    pub def: DefId,
    /// The variant's type arguments written in terms of the union's own type
    /// parameters (identity for anonymous variants, empty for named non-generic ones).
    pub args: Vec<Ty>,
    pub value: Discr,
}

#[derive(Clone, Debug)]
pub struct UnionSig {
    pub discr_field: String,
    /// `string`, `i32` or `bool`: the type of reading `u.kind`.
    pub discr_ty: Ty,
    pub variants: Vec<UnionVariant>,
}

#[derive(Clone, Debug)]
pub struct EnumSig {
    pub repr: NumTy,
    pub members: Vec<(String, i128)>,
}

#[derive(Clone, Debug)]
pub struct ParamSig {
    pub name: String,
    pub ty: Ty,
    pub has_default: bool,
}

#[derive(Clone, Debug)]
pub struct FnSig {
    pub params: Vec<ParamSig>,
    pub ret: Ty,
    /// All type parameters (owner's first).
    pub tparams: Vec<ParamId>,
    pub mutating: bool,
    pub is_static: bool,
    pub private: bool,
}

#[derive(Clone, Debug)]
pub struct IfaceMethod {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub tparams: Vec<ParamId>,
}

#[derive(Clone, Debug)]
pub struct IfaceSig {
    /// Own methods plus those of `extends` interfaces (flattened, own first).
    pub methods: Vec<IfaceMethod>,
}

#[derive(Clone, Debug)]
pub struct HostSig {
    /// Import module (`host:log`) and version.
    pub module: String,
    pub version: u32,
    /// Methods as function types (`fn` of the IDL, without the capability handle).
    pub fns: Vec<(String, Ty)>,
    pub handles: Vec<(String, DefId)>,
    /// Callback types (importable by name).
    pub callbacks: Vec<(String, Ty)>,
}

#[derive(Clone, Debug)]
pub struct GlobalSig {
    pub ty: Ty,
    pub mutable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    ArrayLength,
    ArrayPush,
    ArrayPop,
    ArrayMap,
    ArrayFilter,
    ArrayReduce,
    ArrayForEach,
    ArraySlice,
    ArraySome,
    ArrayEvery,
    ArrayConcat,
    StrByteLength,
    StrBytes,
    StrChars,
    StrSlice,
    StrStartsWith,
    StrEndsWith,
    StrIncludes,
    StrIndexOf,
    NumToString,
    BoolToString,
    /// `x.wrapping<T>()`
    NumWrapping,
    /// `x.saturating<T>()`
    NumSaturating,
    /// `u.downcast<Variant>()`
    Downcast,
    /// `range(a, b)`
    Range,
    /// Deterministic 64-bit hash on primitives and `string` (satisfies `Hash`).
    PrimHash,
    /// Value equality on primitives and `string` (satisfies `Eq<Self>`).
    PrimEquals,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Res {
    Local(LocalId),
    /// The receiver `this` (a local in method bodies; this is its id).
    This(LocalId),
    /// A function, global, class (static access), enum or namespace by name.
    Def(DefId),
    /// Instance field `index` of `owner` (struct, class or union variant).
    Field { owner: DefId, index: u32 },
    StaticField { owner: DefId, index: u32 },
    /// A user method (struct/class) selected statically.
    Method(DefId),
    /// A method of a type parameter's interface bound; resolved after monomorphisation.
    IfaceMethod { iface: DefId, index: u32 },
    /// A method of a host capability (`index` into `HostSig::fns`).
    HostFn { host: DefId, index: u32 },
    EnumMember { def: DefId, index: u32 },
    /// `u.kind` of a discriminated union (read-only, derived from the tag).
    Discriminant { union: DefId },
    Builtin(Builtin),
}

/// An implicit conversion at an expression whose type differs from the type its context demands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coercion {
    pub from: Ty,
    pub to: Ty,
}

#[derive(Clone, Debug, Default)]
pub struct CallInfo {
    /// Explicit or inferred type arguments, in `FnSig::tparams` order (owner's first for methods).
    pub type_args: Vec<Ty>,
    /// Index into [`Checked::instances`] when the callee is generic.
    pub instance: Option<u32>,
    /// Trailing parameters filled by their default expression.
    pub defaults_used: u32,
    /// Trailing `void` parameters (`Ok()` for `Result<void, E>`) the call omitted.
    pub void_elided: u32,
}

/// A monomorphisation request: `def` instantiated with `args` (in terms of the
/// type parameters of the *calling* definition when it is generic itself).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Instance {
    pub def: DefId,
    pub args: Vec<Ty>,
}

#[derive(Clone, Debug)]
pub struct Capture {
    pub local: LocalId,
    /// The variable is assigned after initialisation somewhere (a shared cell is needed).
    pub mutated: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ClosureInfo {
    pub captures: Vec<Capture>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    Const,
    Let,
    Param,
    This,
    /// Loop variable of `for ... of`.
    ForOf,
}

#[derive(Clone, Debug)]
pub struct LocalInfo {
    pub name: String,
    /// Declared (or inferred) type; narrowed types are in `expr_ty`.
    pub ty: Ty,
    pub kind: LocalKind,
    pub span: Span,
    pub captured: bool,
    /// Assigned after its initialisation (anywhere, including inside closures).
    pub mutated: bool,
    pub used: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchKind {
    Enum,
    Union,
    Nullable,
    Bool,
    Int,
    Str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaseLabel {
    /// Variant index of the union.
    Variant(u32),
    /// Enum member index.
    Member(u32),
    Null,
    /// A constant label of an `Int`/`Str`/`Bool` switch (the expression's own table entry).
    Value,
    Default,
}

#[derive(Clone, Debug)]
pub struct SwitchInfo {
    pub kind: SwitchKind,
    /// The scrutinee for a discriminated union is `x.kind`; this is `x`'s union type.
    pub union: Option<Ty>,
    /// Labels of each clause/arm, in source order.
    pub labels: Vec<Vec<CaseLabel>>,
    pub exhaustive: bool,
    pub has_default: bool,
}

#[derive(Clone, Debug)]
pub struct ModuleInfo {
    /// Canonical name (`""` for a single-file check, a path or `std/..` for imports).
    pub name: String,
    pub file: FileId,
    /// Whether this module was loaded from the bundled standard library (`std/...` or prelude).
    pub is_std: bool,
    /// Top-level definitions in declaration order.
    pub items: Vec<DefId>,
    /// Type of every expression node (after narrowing), indexed by `NodeId`; `ERROR`-free when there are no errors.
    pub expr_ty: Vec<Ty>,
    pub res: HashMap<NodeId, Res>,
    pub locals: Vec<LocalInfo>,
    /// Binding identifier / pattern identifier node -> local.
    pub binding_local: HashMap<NodeId, LocalId>,
    /// `Item::id` / `Member::id` -> definition.
    pub node_def: HashMap<NodeId, DefId>,
    pub coercions: HashMap<NodeId, Coercion>,
    pub calls: HashMap<NodeId, CallInfo>,
    pub closures: HashMap<NodeId, ClosureInfo>,
    pub switches: HashMap<NodeId, SwitchInfo>,
    /// Object literal -> the struct or variant it constructs.
    pub object_lits: HashMap<NodeId, DefId>,
    /// `x as T` -> (from, to); also records `?` -> the `Result` type it unwraps.
    pub casts: HashMap<NodeId, Coercion>,
    /// `main` of this module, if any.
    pub main: Option<DefId>,
    /// Recognised attributes by attached node: name and rendered arguments.
    pub attrs: HashMap<NodeId, Vec<(String, Vec<String>)>>,
    /// The `this` local of each method/constructor body.
    pub this_locals: HashMap<DefId, LocalId>,
}

/// What running a function may do, transitively through the functions it calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectSummary {
    /// Uses a host capability value (and so may call the host).
    pub uses_host: bool,
    /// Reads module-level `let` state or a static field.
    pub reads_globals: bool,
    /// Writes module-level `let` state or a static field.
    pub writes_globals: bool,
    /// Writes an array element, pushes/pops an array, or assigns a class field: visible to every alias.
    pub mutates_heap: bool,
    /// Calls a method through an interface bound (`T extends I`): the callee, and so its effects, are unknown.
    pub calls_unknown: bool,
}

impl EffectSummary {
    pub fn is_pure(&self) -> bool {
        !self.uses_host && !self.reads_globals && !self.writes_globals && !self.mutates_heap && !self.calls_unknown
    }
}

/// Result of checking a program (entry module = module 0).
#[derive(Debug)]
pub struct Checked {
    pub diagnostics: Vec<Diagnostic>,
    pub tys: Interner,
    pub defs: Vec<DefInfo>,
    pub params: Vec<TypeParamInfo>,
    pub modules: Vec<ModuleInfo>,
    pub records: HashMap<DefId, RecordSig>,
    pub unions: HashMap<DefId, UnionSig>,
    pub enums: HashMap<DefId, EnumSig>,
    pub ifaces: HashMap<DefId, IfaceSig>,
    pub fns: HashMap<DefId, FnSig>,
    pub globals: HashMap<DefId, GlobalSig>,
    pub aliases: HashMap<DefId, Ty>,
    pub hosts: HashMap<DefId, HostSig>,
    pub instances: Vec<Instance>,
    /// Transitive effect summary of every function, method, constructor and law (feeds `#effects`).
    pub effects: HashMap<DefId, EffectSummary>,
    /// Exported function `main` of the entry module, if present.
    pub entry: Option<DefId>,
    /// Index of the implicit prelude module (`Result`, `Ok`, `Err`), always the last module.
    pub prelude: Option<u32>,
    /// Source text of every module, indexed like [`Checked::modules`] (the prelude last): 8.L
    /// re-parses them (node ids are deterministic) because `Checked` does not keep the trees.
    pub sources: Vec<String>,
    /// Module check/initialisation order: the prelude first, then imports before importers.
    pub order: Vec<u32>,
}

impl Checked {
    /// Whether the function carries `@host` (it becomes a Wasm export of the entry module).
    pub fn is_host_fn(&self, d: DefId) -> bool {
        let info = &self.defs[d.0 as usize];
        self.modules
            .get(info.module as usize)
            .and_then(|m| m.attrs.get(&info.node))
            .is_some_and(|v| v.iter().any(|(n, _)| n == "host"))
    }

    /// If `d` is a bundled standard-library `@intrinsic("...")` function, returns the intrinsic name.
    pub fn intrinsic_of(&self, d: DefId) -> Option<&str> {
        let info = &self.defs[d.0 as usize];
        let m = self.modules.get(info.module as usize)?;
        if !m.is_std {
            return None;
        }
        m.attrs
            .get(&info.node)?
            .iter()
            .find(|(n, _)| n == "intrinsic")
            .and_then(|(_, a)| a.first().map(String::as_str))
    }

    /// Name of the first side table that holds the poison type, if any. Poison is only legal in
    /// a program that already has errors; an error-free program must be free of it by construction.
    pub(crate) fn poisoned_table(&self) -> Option<&'static str> {
        use crate::ty::ERROR;
        let e = |t: Ty| t == ERROR || self.tys.has_error(t);
        for m in &self.modules {
            if m.locals.iter().any(|l| e(l.ty)) {
                return Some("local variable types");
            }
            if m.coercions.values().chain(m.casts.values()).any(|c| e(c.from) || e(c.to)) {
                return Some("coercions or casts");
            }
            if m.calls.values().any(|c| c.type_args.iter().any(|t| e(*t))) {
                return Some("call type arguments");
            }
            if m.switches.values().any(|s| s.union.is_some_and(e)) {
                return Some("switch information");
            }
        }
        if self.instances.iter().any(|i| i.args.iter().any(|t| e(*t))) {
            return Some("monomorphisation instances");
        }
        if self.fns.values().any(|f| e(f.ret) || f.params.iter().any(|p| e(p.ty))) {
            return Some("function signatures");
        }
        if self.records.values().any(|r| r.fields.iter().any(|f| e(f.ty))) {
            return Some("struct fields");
        }
        if self.globals.values().any(|g| e(g.ty)) {
            return Some("global types");
        }
        if self.aliases.values().any(|t| e(*t)) {
            return Some("type aliases");
        }
        if self.unions.values().any(|u| e(u.discr_ty)) {
            return Some("union signatures");
        }
        if self.hosts.values().any(|h| h.fns.iter().any(|(_, t)| e(*t)) || h.callbacks.iter().any(|(_, t)| e(*t))) {
            return Some("host signatures");
        }
        None
    }

    /// `true` when there are no error diagnostics: only then may 8.L use the tables.
    pub fn ok(&self) -> bool {
        !self.diagnostics.iter().any(Diagnostic::is_error)
    }
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }
}
