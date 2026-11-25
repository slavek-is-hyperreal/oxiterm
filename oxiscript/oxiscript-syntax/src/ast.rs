//! The OxiScript AST for the MVP grammar (`docs/oxiscript/grammar.md`).
//!
//! Conventions:
//! * Every node carries a [`NodeId`] (unique within a parse, dense, assigned
//!   by the parser through [`NodeIdGen`]) and a [`Span`] (byte offsets).
//!   Side tables (types, resolutions, diagnostics, placement remarks) key on
//!   `NodeId`; nothing is stored in the tree that a later phase computes.
//! * Parenthesised expressions and doc comments are kept so the formatter can
//!   be lossless about structure; ordinary comments are trivia handled by the
//!   formatter slice (8.P) outside the AST.
//! * Syntax reserved for the future (kernel/parallel/async/inout/contracts)
//!   already has a slot (`FnModifiers::kernel`, `For::parallel`,
//!   `ParamMode::Inout`, `FnDecl::contracts`). The parser rejects it with
//!   `OXS-E0900` until the feature exists, so adding it later does not change
//!   the shape of the tree.
//! * Enums are deliberately **not** `#[non_exhaustive]`: when a node kind is
//!   added, every consumer in the workspace must be revisited.

/// Identifies a source file (an index into the host's file table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

/// A byte range `lo..hi` (UTF-8 byte offsets) in a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub const DUMMY: Span = Span {
        file: FileId(0),
        lo: 0,
        hi: 0,
    };

    pub fn new(file: FileId, lo: u32, hi: u32) -> Span {
        debug_assert!(lo <= hi);
        Span { file, lo, hi }
    }

    /// Smallest span covering both (same file assumed).
    #[must_use]
    pub fn to(self, other: Span) -> Span {
        Span {
            file: self.file,
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

/// Hands out dense, increasing [`NodeId`]s.
#[derive(Clone, Debug, Default)]
pub struct NodeIdGen(u32);

impl NodeIdGen {
    pub fn new() -> NodeIdGen {
        NodeIdGen(0)
    }
    pub fn next_id(&mut self) -> NodeId {
        let id = NodeId(self.0);
        self.0 += 1;
        id
    }
    /// Number of ids handed out so far (the size for dense side tables).
    pub fn count(&self) -> u32 {
        self.0
    }
}

// ---- leaves -------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub id: NodeId,
    pub span: Span,
    pub name: String,
}

/// Primitive numeric types usable as literal suffixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumTy {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
}

impl NumTy {
    pub fn name(self) -> &'static str {
        match self {
            NumTy::I8 => "i8",
            NumTy::I16 => "i16",
            NumTy::I32 => "i32",
            NumTy::I64 => "i64",
            NumTy::U8 => "u8",
            NumTy::U16 => "u16",
            NumTy::U32 => "u32",
            NumTy::U64 => "u64",
            NumTy::F32 => "f32",
            NumTy::F64 => "f64",
        }
    }
    pub fn from_name(s: &str) -> Option<NumTy> {
        Some(match s {
            "i8" => NumTy::I8,
            "i16" => NumTy::I16,
            "i32" => NumTy::I32,
            "i64" => NumTy::I64,
            "u8" => NumTy::U8,
            "u16" => NumTy::U16,
            "u32" => NumTy::U32,
            "u64" => NumTy::U64,
            "f32" => NumTy::F32,
            "f64" => NumTy::F64,
            _ => return None,
        })
    }
    pub fn is_float(self) -> bool {
        matches!(self, NumTy::F32 | NumTy::F64)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LitKind {
    /// Integer literal; the value is the magnitude (a leading `-` is a unary
    /// operator), range-checked against the final type by the checker.
    Int {
        value: u128,
        suffix: Option<NumTy>,
    },
    /// Float literal kept as source text (without separators/suffix) so the
    /// checker can convert exactly per target type.
    Float {
        text: String,
        suffix: Option<NumTy>,
    },
    /// String literal, escapes already decoded (valid UTF-8 by construction).
    Str(String),
    Bool(bool),
    Null,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lit {
    pub id: NodeId,
    pub span: Span,
    pub kind: LitKind,
}

// ---- attributes ---------------------------------------------------------

/// `@name`, `@name(args)` (outer) or `@@name(args)` (inner: applies to the
/// enclosing module).
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub id: NodeId,
    pub span: Span,
    pub inner: bool,
    pub name: Ident,
    pub args: Vec<AttrArg>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttrArg {
    pub id: NodeId,
    pub span: Span,
    /// `key: value` form.
    pub key: Option<Ident>,
    pub value: AttrValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    Lit(Lit),
    /// A bare name or dotted path, e.g. `simd`, `on: browser`.
    Path(Vec<Ident>),
}

// ---- module and items ---------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub id: NodeId,
    pub span: Span,
    /// Inner attributes (`@@profile(realtime);`).
    pub attrs: Vec<Attribute>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: NodeId,
    pub span: Span,
    /// `///` doc comment lines, text without the marker.
    pub doc: Vec<String>,
    pub attrs: Vec<Attribute>,
    pub export: bool,
    pub kind: ItemKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Import(ImportDecl),
    Var(VarDecl),
    Fn(FnDecl),
    /// `struct` (value semantics) or `class` (reference semantics).
    Record(RecordDecl),
    Interface(InterfaceDecl),
    Enum(EnumDecl),
    TypeAlias(TypeAliasDecl),
    Law(LawDecl),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportDecl {
    pub clause: ImportClause,
    /// `"./util"`, `"pkg"`, `"host:log"`.
    pub source: Lit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ImportClause {
    /// `{ a, b as c }`
    Named(Vec<ImportSpec>),
    /// `* as ns`
    Namespace(Ident),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportSpec {
    pub id: NodeId,
    pub span: Span,
    pub name: Ident,
    pub alias: Option<Ident>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    /// Mutable binding.
    Let,
    /// Immutable binding.
    Const,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VarDecl {
    pub kind: VarKind,
    pub pattern: Pattern,
    pub ty: Option<TypeExpr>,
    pub init: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeParam {
    pub id: NodeId,
    pub span: Span,
    pub name: Ident,
    /// `T extends Bound`
    pub bound: Option<TypeExpr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamMode {
    Value,
    /// Reserved: `inout` parameters (mutable value semantics).
    Inout,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub id: NodeId,
    pub span: Span,
    pub mode: ParamMode,
    pub name: Ident,
    /// Required on declarations; optional only for arrow-function parameters
    /// whose type comes from context.
    pub ty: Option<TypeExpr>,
    pub default: Option<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct FnModifiers {
    pub is_static: bool,
    /// Struct methods that assign to `this`.
    pub mutating: bool,
    pub private: bool,
    /// Reserved: `kernel function` (data-parallel kernel profile).
    pub kernel: bool,
    /// Reserved: `async function`.
    pub is_async: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContractKind {
    Requires,
    Ensures,
}

/// Reserved: `requires expr` / `ensures expr` after a signature.
#[derive(Clone, Debug, PartialEq)]
pub struct Contract {
    pub kind: ContractKind,
    pub expr: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    pub name: Ident,
    pub mods: FnModifiers,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub contracts: Vec<Contract>,
    /// `None` for interface method signatures.
    pub body: Option<Block>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    Struct,
    Class,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecordDecl {
    pub kind: RecordKind,
    pub name: Ident,
    pub type_params: Vec<TypeParam>,
    /// `implements A, B` (interfaces satisfied; checked structurally).
    pub implements: Vec<TypeExpr>,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub id: NodeId,
    pub span: Span,
    pub doc: Vec<String>,
    pub attrs: Vec<Attribute>,
    pub kind: MemberKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MemberKind {
    Field(FieldDecl),
    Method(FnDecl),
    /// `constructor(...) { ... }` (classes only); `FnDecl::name` is `constructor`.
    Constructor(FnDecl),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub init: Option<Expr>,
    pub readonly: bool,
    pub private: bool,
    pub is_static: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceDecl {
    pub name: Ident,
    pub type_params: Vec<TypeParam>,
    pub extends: Vec<TypeExpr>,
    /// Method signatures (`body == None`).
    pub methods: Vec<FnDecl>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnumDecl {
    pub name: Ident,
    /// `enum Color: u8 { ... }`
    pub repr: Option<TypeExpr>,
    pub variants: Vec<EnumVariant>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnumVariant {
    pub id: NodeId,
    pub span: Span,
    pub name: Ident,
    pub value: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeAliasDecl {
    pub name: Ident,
    pub type_params: Vec<TypeParam>,
    pub ty: TypeExpr,
}

/// `law name<T>(params) when precondition { body }`; the body is a block whose
/// tail expression (no `;`) is the property.
#[derive(Clone, Debug, PartialEq)]
pub struct LawDecl {
    pub name: Ident,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub when: Option<Expr>,
    pub body: Block,
}

// ---- types --------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct TypeExpr {
    pub id: NodeId,
    pub span: Span,
    pub kind: TypeExprKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeExprKind {
    /// `i32`, `string`, `Point`, `Result<T, E>`, `ns.Type<A>`. Builtins
    /// (`void`, `never`, primitives) are ordinary paths resolved by the checker.
    Path {
        segments: Vec<Ident>,
        args: Vec<TypeExpr>,
    },
    /// `T[]` and `Array<T>` both parse to this.
    Array(Box<TypeExpr>),
    /// `A | B | null`; flattened, at least two members.
    Union(Vec<TypeExpr>),
    /// `(a: i32, b: i32) => i32`
    Fn {
        params: Vec<FnTypeParam>,
        ret: Box<TypeExpr>,
    },
    /// A literal type: `"circle"`, `3`, `true`; only valid as a discriminant.
    Lit(Lit),
    /// Anonymous object type `{ kind: "circle"; r: f64 }`; only valid as a
    /// union variant or alias right-hand side (it becomes a named struct).
    Object(Vec<FieldSig>),
    /// `null` in type position.
    Null,
    Paren(Box<TypeExpr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnTypeParam {
    pub id: NodeId,
    pub span: Span,
    pub name: Ident,
    pub ty: TypeExpr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldSig {
    pub id: NodeId,
    pub span: Span,
    pub name: Ident,
    pub ty: TypeExpr,
    pub readonly: bool,
}

// ---- patterns -----------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub id: NodeId,
    pub span: Span,
    pub kind: PatternKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    Ident(Ident),
    /// `_`
    Wildcard,
    /// `{ a, b: renamed, c: { d } }` struct destructuring.
    Object(Vec<ObjectPatField>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectPatField {
    pub id: NodeId,
    pub span: Span,
    pub key: Ident,
    /// `None` = shorthand binding of `key`.
    pub pattern: Option<Pattern>,
}

// ---- statements ---------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub id: NodeId,
    pub span: Span,
    pub stmts: Vec<Stmt>,
    /// Final expression without `;`. The parser only produces it in `law`
    /// bodies; elsewhere a missing `;` is `OXS-E0004`.
    pub tail: Option<Box<Expr>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub id: NodeId,
    pub span: Span,
    /// Attributes on statements (`@expect(simd) for ...`).
    pub attrs: Vec<Attribute>,
    pub kind: StmtKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    Var(VarDecl),
    Expr(Expr),
    If {
        cond: Expr,
        then: Box<Stmt>,
        els: Option<Box<Stmt>>,
    },
    While {
        cond: Expr,
        body: Box<Stmt>,
    },
    DoWhile {
        body: Box<Stmt>,
        cond: Expr,
    },
    /// C-style `for (init; cond; update) body`. `parallel` is reserved.
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        update: Option<Expr>,
        body: Box<Stmt>,
        parallel: bool,
    },
    /// `for (const x of xs) body`. `parallel` is reserved.
    ForOf {
        kind: VarKind,
        binding: Pattern,
        iter: Expr,
        body: Box<Stmt>,
        parallel: bool,
    },
    Switch(SwitchStmt),
    Return(Option<Expr>),
    Break(Option<Ident>),
    Continue(Option<Ident>),
    Block(Block),
    Labeled {
        label: Ident,
        body: Box<Stmt>,
    },
    /// A nested declaration (function, struct, ...).
    Item(Box<Item>),
    Empty,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwitchStmt {
    pub scrutinee: Expr,
    pub cases: Vec<Case>,
}

/// `case a: case b: body` is one `Case` with two labels (consecutive empty
/// labels are grouping, never fallthrough). `default` may stand alone or
/// follow labels.
#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    pub id: NodeId,
    pub span: Span,
    pub labels: Vec<Expr>,
    pub is_default: bool,
    pub body: Vec<Stmt>,
}

// ---- expressions --------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub id: NodeId,
    pub span: Span,
    pub kind: ExprKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    /// `-x`
    Neg,
    /// `!x`
    Not,
    /// `~x`
    BitNot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    /// `+%`, `-%`, `*%`: wrapping.
    AddWrap,
    SubWrap,
    MulWrap,
    /// `+|`, `-|`, `*|`: saturating.
    AddSat,
    SubSat,
    MulSat,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Lt,
    Le,
    Gt,
    Ge,
    /// `===`
    Eq,
    /// `!==`
    Ne,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `??`
    Coalesce,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncDec {
    Inc,
    Dec,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Lit(Lit),
    /// `` `a ${b} c` ``: alternating string and expression parts.
    Template(Vec<TemplatePart>),
    Ident(Ident),
    This,
    /// `[a, b, c]`
    Array(Vec<Expr>),
    /// `{ a: 1, b }`; the type comes from context (bidirectional inference).
    Object(Vec<ObjectField>),
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// `a = b` (`op == None`) or `a += b` (`op == Some(Add)`).
    Assign {
        op: Option<BinOp>,
        target: Box<Expr>,
        value: Box<Expr>,
    },
    /// `i++` / `--i`; only valid as an expression statement or `for` update.
    Update {
        op: IncDec,
        prefix: bool,
        target: Box<Expr>,
    },
    /// `f(a)` or, with `optional`, `f?.(a)` (calls only if `f` is non-null;
    /// the result is then `R | null`).
    Call {
        callee: Box<Expr>,
        type_args: Vec<TypeExpr>,
        args: Vec<Expr>,
        optional: bool,
    },
    /// `new Counter<T>(args)`
    New {
        ty: TypeExpr,
        args: Vec<Expr>,
    },
    /// `a.b` or `a?.b`
    Member {
        obj: Box<Expr>,
        name: Ident,
        optional: bool,
    },
    /// `a[i]` or `a?.[i]`
    Index {
        obj: Box<Expr>,
        index: Box<Expr>,
        optional: bool,
    },
    /// Postfix `?` (Result propagation).
    Try(Box<Expr>),
    /// `x as T`: lossless numeric conversion, trapping if out of range. The
    /// type is a non-union type (`x as (A | B)` needs parentheses). Wrapping,
    /// saturating and downcast conversions are ordinary generic method calls
    /// (`x.wrapping<u8>()`, `x.saturating<u8>()`, `x.downcast<T>()`), not syntax.
    Cast {
        expr: Box<Expr>,
        ty: TypeExpr,
    },
    /// `c ? a : b`
    Cond {
        cond: Box<Expr>,
        then: Box<Expr>,
        els: Box<Expr>,
    },
    Arrow(ArrowFn),
    /// Expression-form `switch`: a `switch` in expression position; every arm is `case a, b: expr,`.
    Switch(SwitchExpr),
    Paren(Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TemplatePart {
    Str(String),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectField {
    pub id: NodeId,
    pub span: Span,
    pub name: Ident,
    /// `None` = shorthand `{ x }`.
    pub value: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArrowFn {
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: ArrowBody,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArrowBody {
    Expr(Box<Expr>),
    Block(Block),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwitchExpr {
    pub scrutinee: Box<Expr>,
    pub arms: Vec<Arm>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arm {
    pub id: NodeId,
    pub span: Span,
    pub labels: Vec<Expr>,
    pub is_default: bool,
    pub body: Expr,
}
