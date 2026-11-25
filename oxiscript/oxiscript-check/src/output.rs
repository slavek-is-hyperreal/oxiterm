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
pub struct FieldSig {
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
    pub fields: Vec<FieldSig>,
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
    /// Exported function `main` of the entry module, if present.
    pub entry: Option<DefId>,
}

impl Checked {
    /// `true` when there are no error diagnostics: only then may 8.L use the tables.
    pub fn ok(&self) -> bool {
        !self.diagnostics.iter().any(Diagnostic::is_error)
    }
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }
}
