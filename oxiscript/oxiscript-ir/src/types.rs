//! OxiIR data types. See `docs/oxiscript/ir.md` for the semantics (ownership,
//! traps, determinism); this file is the shape.
//!
//! Naming: symbols (types, functions, imports, data, globals) are identified
//! by their *name string*; values and blocks by dense ids local to a function.
//! Canonical numbering: [`ValueId`]s are assigned in textual order (function
//! parameters, then for each block its parameters followed by its
//! instruction results), so `parse(print(m)) == m` holds for any module whose
//! ids follow that order (the parser and the [`crate::build`] helpers do).

pub use oxiscript_abi::trap::TrapKind;

/// Maximum nesting depth of a [`Ty`] accepted by the parser and the verifier
/// (`[[[...]]]`, `fn() -> fn() -> ...`). Bounds recursion everywhere.
pub const MAX_TYPE_DEPTH: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ValueId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

/// Source location carried through lowering so reports and traps can point at
/// source (`file` is a `FileId` of `oxiscript-syntax`; offsets are bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Loc {
    pub file: u32,
    pub lo: u32,
    pub hi: u32,
}

/// Generic attribute: `#name` or `#name(arg, "arg two")`. Used for function
/// effect summaries, profiles, inline hints, `@expect` assertions, ... Names
/// and meanings are registered in `docs/oxiscript/ir.md`; unknown attributes
/// must be preserved and ignored by passes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub args: Vec<String>,
}

// ---- types --------------------------------------------------------------

/// Function signature (also the type of a closure value, where `rets.len() <= 1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sig {
    pub params: Vec<Ty>,
    /// Empty = void. More than one result is Core Wasm multi-value; only
    /// imports and plain functions may have several (closures: at most one).
    pub rets: Vec<Ty>,
}

/// Value types. Scalars are unmanaged; `Ref`, `Str`, `Array`, `Fn` are
/// reference-counted managed pointers (never null) and `Opt(..)` of those is
/// the nullable form (null = address 0).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ty {
    Bool,
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
    /// Raw wasm32 address (unmanaged). Only runtime-support modules use it.
    Ptr,
    /// Immutable UTF-8 string object.
    Str,
    /// Managed reference to an object of a declared type (struct, closure env
    /// or union).
    Ref(String),
    /// Managed array object.
    Array(Box<Ty>),
    /// Closure value (managed object holding a function index and captures).
    Fn(Box<Sig>),
    /// Nullable managed reference; the inner type is `Ref`, `Str`, `Array` or `Fn`.
    Opt(Box<Ty>),
}

impl Ty {
    pub fn is_int(&self) -> bool {
        matches!(
            self,
            Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64 | Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64
        )
    }
    pub fn is_signed_int(&self) -> bool {
        matches!(self, Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64)
    }
    pub fn is_float(&self) -> bool {
        matches!(self, Ty::F32 | Ty::F64)
    }
    pub fn is_numeric(&self) -> bool {
        self.is_int() || self.is_float()
    }
    /// Bit width of an integer or float type.
    pub fn bits(&self) -> Option<u32> {
        Some(match self {
            Ty::I8 | Ty::U8 => 8,
            Ty::I16 | Ty::U16 => 16,
            Ty::I32 | Ty::U32 | Ty::F32 | Ty::Ptr => 32,
            Ty::I64 | Ty::U64 | Ty::F64 => 64,
            _ => return None,
        })
    }
    /// Managed (reference-counted) type, nullable or not.
    pub fn is_managed(&self) -> bool {
        matches!(
            self,
            Ty::Str | Ty::Ref(_) | Ty::Array(_) | Ty::Fn(_) | Ty::Opt(_)
        )
    }
    /// Plain scalar (no RC).
    pub fn is_scalar(&self) -> bool {
        self.is_numeric() || matches!(self, Ty::Bool | Ty::Ptr)
    }
}

// ---- module -------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Module {
    pub name: String,
    /// Module attributes, e.g. `#profile(realtime)`, `#edition(2026)`,
    /// `#raw_memory` (permits `load`/`store`/`intrinsic`: runtime-support modules only).
    pub attrs: Vec<Attr>,
    pub types: Vec<TypeDecl>,
    pub imports: Vec<Import>,
    pub data: Vec<DataDecl>,
    pub globals: Vec<Global>,
    pub funcs: Vec<Func>,
}

impl Module {
    pub fn type_decl(&self, name: &str) -> Option<&TypeDecl> {
        self.types.iter().find(|t| t.name == name)
    }
    pub fn func(&self, name: &str) -> Option<&Func> {
        self.funcs.iter().find(|f| f.name == name)
    }
    pub fn import(&self, name: &str) -> Option<&Import> {
        self.imports.iter().find(|i| i.name == name)
    }
    pub fn global(&self, name: &str) -> Option<&Global> {
        self.globals.iter().find(|g| g.name == name)
    }
    pub fn has_attr(&self, name: &str) -> bool {
        self.attrs.iter().any(|a| a.name == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: Ty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeKind {
    /// Struct or class: a heap object, fields in declaration order.
    Struct { fields: Vec<Field> },
    /// Closure environment: captured values; laid out after the closure header words.
    Closure { captures: Vec<Field> },
    /// Tagged union; the tag of a variant is its index.
    Union { variants: Vec<Variant> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeDecl {
    pub name: String,
    pub attrs: Vec<Attr>,
    pub kind: TypeKind,
}

/// A host import: a raw Wasm-level function (flat types, multi-value
/// results). Lowering from the language builds the ABI mapping of
/// `oxiscript-abi::idl` into these signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub name: String,
    /// Wasm import module, e.g. `host:log@1`.
    pub module: String,
    /// Wasm import field name.
    pub field: String,
    pub sig: Sig,
}

/// A UTF-8 string constant placed in the data segment as an immortal string object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataDecl {
    pub name: String,
    pub text: String,
}

/// Module-level mutable state (module-local: it resets on reload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Global {
    pub name: String,
    pub ty: Ty,
    /// Initial value: a scalar constant, or `None` meaning zero / null.
    pub init: Option<Const>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueInfo {
    pub name: String,
    pub ty: Ty,
}

/// Function parameter. `own` = the callee consumes one reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub value: ValueId,
    pub own: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Func {
    pub name: String,
    pub export: bool,
    pub attrs: Vec<Attr>,
    pub params: Vec<Param>,
    pub rets: Vec<Ty>,
    /// Indexed by [`ValueId`].
    pub values: Vec<ValueInfo>,
    /// `blocks[0]` is the entry block (no parameters).
    pub blocks: Vec<Block>,
}

impl Func {
    pub fn value(&self, v: ValueId) -> &ValueInfo {
        &self.values[v.0 as usize]
    }
    pub fn ty(&self, v: ValueId) -> &Ty {
        &self.values[v.0 as usize].ty
    }
    pub fn block(&self, b: BlockId) -> &Block {
        &self.blocks[b.0 as usize]
    }
    pub fn sig(&self) -> Sig {
        Sig {
            params: self
                .params
                .iter()
                .map(|p| self.ty(p.value).clone())
                .collect(),
            rets: self.rets.clone(),
        }
    }
    pub fn has_attr(&self, name: &str) -> bool {
        self.attrs.iter().any(|a| a.name == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub label: String,
    /// Block parameters (SSA phi replacement). Arguments passed to them are
    /// *owned* (consumed) when the type is managed.
    pub params: Vec<ValueId>,
    pub insts: Vec<Inst>,
    pub term: Terminator,
    pub term_loc: Option<Loc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inst {
    pub results: Vec<ValueId>,
    pub op: Op,
    pub loc: Option<Loc>,
}

/// A branch target with its block arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub block: BlockId,
    pub args: Vec<ValueId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Terminator {
    Br(Target),
    BrIf {
        cond: ValueId,
        then: Target,
        els: Target,
    },
    /// Multi-way branch on an integer value.
    Switch {
        value: ValueId,
        cases: Vec<(i64, Target)>,
        default: Target,
    },
    /// Consumes (moves out) its operands.
    Ret(Vec<ValueId>),
    Trap(TrapKind),
}

// ---- operations ---------------------------------------------------------

/// Constants. Floats are stored as bit patterns so equality is total and NaN
/// payloads round-trip through the text form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Const {
    Bool(bool),
    Int(i128),
    F32(u32),
    F64(u64),
}

/// How an integer operation treats overflow (and out-of-range shifts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ovf {
    /// Default: trap (`overflow`, or `bounds` for an out-of-range shift amount).
    Trap,
    /// Two's complement wrap-around (shift amount masked like Wasm).
    Wrap,
    /// Saturate to the type's min/max (add, sub, mul and conversions only).
    Sat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    /// Integer: traps `div_by_zero` (and `overflow` for MIN/-1 in `Trap` mode). Float: IEEE.
    Div,
    /// Integer remainder (sign of the dividend); traps like `Div`. Float: not allowed.
    Rem,
    And,
    Or,
    Xor,
    Shl,
    /// Arithmetic on signed types, logical on unsigned.
    Shr,
    /// Float only; any NaN operand gives canonical NaN.
    Min,
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    /// Arithmetic negation (integer: traps on MIN in `Trap` mode; the mode is
    /// stored in the instruction).
    Neg,
    /// Bitwise not (ints) / logical not (bool).
    Not,
    // float only
    Abs,
    Sqrt,
    Floor,
    Ceil,
    Trunc,
    Nearest,
    // int only
    Clz,
    Ctz,
    Popcnt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A reference to a declared type, optionally to one variant of a union.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeRef {
    pub ty: String,
    pub variant: Option<String>,
}

/// A reference to a field of a struct, closure env, or union variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldRef {
    pub ty: String,
    pub variant: Option<String>,
    pub field: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Op {
    Const(Const),
    /// The null value of an `Opt` result type.
    Null,
    Bin {
        op: BinOp,
        ovf: Ovf,
        a: ValueId,
        b: ValueId,
    },
    Un {
        op: UnOp,
        ovf: Ovf,
        a: ValueId,
    },
    Cmp {
        op: CmpOp,
        a: ValueId,
        b: ValueId,
    },
    /// Numeric conversion (source type = type of `a`, target = result type).
    /// `Trap` mode traps `overflow` on out-of-range values and on NaN;
    /// `Wrap` (int to int) truncates; `Sat` (float to int) saturates, NaN -> 0.
    Conv {
        ovf: Ovf,
        a: ValueId,
    },
    /// Reinterpret bits between same-width int and float (NaN is canonicalised first).
    Bitcast {
        a: ValueId,
    },
    /// Scalar select (no managed values: ownership would be ambiguous).
    Select {
        cond: ValueId,
        a: ValueId,
        b: ValueId,
    },

    /// Allocate a zeroed object (rc = 1, owned). Variant allocation fixes the tag.
    Alloc(TypeRef),
    GetField {
        field: FieldRef,
        obj: ValueId,
    },
    /// Store into a field. Consumes `val` (ownership moves into the object)
    /// and, for a managed field, releases the previous contents. With
    /// `init == true` (`setfield.init`) the previous contents are known to be
    /// zero/null (a freshly allocated object) and no release happens.
    SetField {
        field: FieldRef,
        obj: ValueId,
        val: ValueId,
        init: bool,
    },
    /// Variant tag of a union value (u32).
    Tag {
        obj: ValueId,
    },
    /// rc == 1 (copy-on-write test). Operand borrowed.
    IsUnique {
        obj: ValueId,
    },
    /// Shallow copy of a struct/union/array object: new owned object, every
    /// child reference retained.
    Clone {
        obj: ValueId,
    },

    /// New zero-initialised array of `len` elements; result `[T]`.
    ArrayNew {
        len: ValueId,
    },
    ArrayLen {
        arr: ValueId,
    },
    /// Bounds-checked (traps `bounds`). Result borrowed from the array.
    ArrayGet {
        arr: ValueId,
        idx: ValueId,
    },
    /// Bounds-checked. Consumes `val` and releases the previous element.
    ArraySet {
        arr: ValueId,
        idx: ValueId,
        val: ValueId,
    },
    /// Append (amortised growth). Consumes `val`.
    ArrayPush {
        arr: ValueId,
        val: ValueId,
    },

    /// The immortal string object for a data declaration (borrowed, sticky rc).
    StrConst {
        data: String,
    },
    /// Byte length of a string (i32).
    StrLen {
        s: ValueId,
    },
    /// Build a new owned string from the first `len` bytes of a `[u8]` array
    /// (traps `bounds` if `len` is negative or exceeds the array length,
    /// `bad_utf8` if the bytes are not valid UTF-8). Used to turn the content
    /// a host wrote into an out-buffer into a `str`.
    StrFromUtf8 {
        bytes: ValueId,
        len: ValueId,
    },

    /// New owned string `a ++ b` (both borrowed). Traps `out_of_memory`.
    StrConcat {
        a: ValueId,
        b: ValueId,
    },
    /// Bytewise string equality (both borrowed) -> bool.
    StrEq {
        a: ValueId,
        b: ValueId,
    },
    /// Bytewise (= code point) order of two strings (both borrowed) -> i32 in `-1, 0, 1`.
    StrCmp {
        a: ValueId,
        b: ValueId,
    },
    /// New owned string of the byte range `lo..hi` of `s` (traps `bounds` when
    /// out of range or `lo > hi`, `bad_utf8` when a boundary is inside a code point).
    StrSlice {
        s: ValueId,
        lo: ValueId,
        hi: ValueId,
    },
    /// Byte `idx` of a string (bounds-checked) -> u8.
    StrByte {
        s: ValueId,
        idx: ValueId,
    },
    /// Shortest round-trip decimal text of a float (`NaN`, `Infinity`, `-Infinity`); f64 operand.
    StrFromF64 {
        x: ValueId,
    },
    /// New empty array (`len 0`, owned) of any element type, including non-null references.
    ArrayEmpty,
    /// Remove and return the last element (traps `bounds` when empty); the result is
    /// **owned** (the array's reference moves out).
    ArrayPop {
        arr: ValueId,
    },
    IsNull {
        val: ValueId,
    },
    /// `T` -> `T?` (no runtime effect; ownership unchanged).
    Opt {
        val: ValueId,
    },
    /// `T?` -> `T`, traps `null_deref` if null (the checker emits it after narrowing).
    Unopt {
        val: ValueId,
    },

    /// Create a closure of declared env type `env` calling `func` (whose first
    /// parameter must be `ref env`); consumes `captures`, in field order.
    ClosureNew {
        env: String,
        func: String,
        captures: Vec<ValueId>,
    },
    /// Call a function or import by symbol. Callee borrowed args unless the
    /// parameter is `own`; results are owned.
    Call {
        callee: String,
        args: Vec<ValueId>,
    },
    /// Call a closure value (borrowed); the closure is passed as the callee's first argument.
    CallClosure {
        callee: ValueId,
        args: Vec<ValueId>,
    },
    /// Call a runtime-support function by its ABI export name (`oxi_*`), raw
    /// signature. Only modules with `#raw_memory` may use it.
    Intrinsic {
        name: String,
        args: Vec<ValueId>,
    },

    Retain {
        val: ValueId,
    },
    Release {
        val: ValueId,
    },

    GlobalGet {
        name: String,
    },
    /// Consumes `val` if managed and releases the previous value.
    GlobalSet {
        name: String,
        val: ValueId,
    },

    /// Raw memory access (`#raw_memory` modules only). Result type gives the access type.
    Load {
        ptr: ValueId,
        offset: u32,
    },
    Store {
        ptr: ValueId,
        offset: u32,
        val: ValueId,
    },
}

impl Op {
    /// Does executing this op possibly trap (so it cannot be freely reordered
    /// or removed without care)? Conservative.
    pub fn may_trap(&self) -> bool {
        match self {
            Op::Bin { op, ovf, .. } => match op {
                BinOp::Div | BinOp::Rem => true,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Shl | BinOp::Shr => *ovf == Ovf::Trap,
                _ => false,
            },
            Op::Un { op, ovf, .. } => *op == UnOp::Neg && *ovf == Ovf::Trap,
            Op::Conv { ovf, .. } => *ovf == Ovf::Trap,
            Op::Alloc(_)
            | Op::ArrayNew { .. }
            | Op::ArrayGet { .. }
            | Op::ArraySet { .. }
            | Op::ArrayPush { .. }
            | Op::ClosureNew { .. }
            | Op::StrFromUtf8 { .. }
            | Op::StrConcat { .. }
            | Op::StrSlice { .. }
            | Op::StrByte { .. }
            | Op::ArrayPop { .. }
            | Op::ArrayEmpty => true,
            Op::Unopt { .. } | Op::Load { .. } | Op::Store { .. } => true,
            Op::Call { .. } | Op::CallClosure { .. } | Op::Intrinsic { .. } | Op::Clone { .. } => {
                true
            }
            _ => false,
        }
    }

    /// Does this op allocate managed memory (effect summary `alloc`)?
    pub fn allocates(&self) -> bool {
        matches!(
            self,
            Op::Alloc(_)
                | Op::ArrayNew { .. }
                | Op::ArrayPush { .. }
                | Op::ClosureNew { .. }
                | Op::Clone { .. }
                | Op::StrFromUtf8 { .. }
                | Op::StrConcat { .. }
                | Op::StrSlice { .. }
                | Op::StrFromF64 { .. }
                | Op::ArrayEmpty
        )
    }

    /// Mutable access to every value operand (same order as [`Op::operands`]);
    /// for passes and for mutation tests.
    pub fn operands_mut(&mut self) -> Vec<&mut ValueId> {
        match self {
            Op::Const(_)
            | Op::Null
            | Op::Alloc(_)
            | Op::StrConst { .. }
            | Op::GlobalGet { .. }
            | Op::ArrayEmpty => {
                vec![]
            }
            Op::StrConcat { a, b } | Op::StrEq { a, b } | Op::StrCmp { a, b } => vec![a, b],
            Op::StrSlice { s, lo, hi } => vec![s, lo, hi],
            Op::StrByte { s, idx } => vec![s, idx],
            Op::StrFromF64 { x } => vec![x],
            Op::ArrayPop { arr } => vec![arr],
            Op::Bin { a, b, .. } | Op::Cmp { a, b, .. } => vec![a, b],
            Op::Un { a, .. } | Op::Conv { a, .. } | Op::Bitcast { a } => vec![a],
            Op::Select { cond, a, b } => vec![cond, a, b],
            Op::GetField { obj, .. }
            | Op::Tag { obj }
            | Op::IsUnique { obj }
            | Op::Clone { obj } => vec![obj],
            Op::SetField { obj, val, .. } => vec![obj, val],
            Op::ArrayNew { len } => vec![len],
            Op::ArrayLen { arr } => vec![arr],
            Op::ArrayGet { arr, idx } => vec![arr, idx],
            Op::ArraySet { arr, idx, val } => vec![arr, idx, val],
            Op::ArrayPush { arr, val } => vec![arr, val],
            Op::StrLen { s } => vec![s],
            Op::StrFromUtf8 { bytes, len } => vec![bytes, len],
            Op::IsNull { val }
            | Op::Opt { val }
            | Op::Unopt { val }
            | Op::Retain { val }
            | Op::Release { val } => vec![val],
            Op::ClosureNew { captures, .. } => captures.iter_mut().collect(),
            Op::Call { args, .. } | Op::Intrinsic { args, .. } => args.iter_mut().collect(),
            Op::CallClosure { callee, args } => {
                std::iter::once(callee).chain(args.iter_mut()).collect()
            }
            Op::GlobalSet { val, .. } => vec![val],
            Op::Load { ptr, .. } => vec![ptr],
            Op::Store { ptr, val, .. } => vec![ptr, val],
        }
    }

    /// All value operands, in textual order.
    pub fn operands(&self) -> Vec<ValueId> {
        match self {
            Op::Const(_)
            | Op::Null
            | Op::Alloc(_)
            | Op::StrConst { .. }
            | Op::GlobalGet { .. }
            | Op::ArrayEmpty => {
                vec![]
            }
            Op::StrConcat { a, b } | Op::StrEq { a, b } | Op::StrCmp { a, b } => vec![*a, *b],
            Op::StrSlice { s, lo, hi } => vec![*s, *lo, *hi],
            Op::StrByte { s, idx } => vec![*s, *idx],
            Op::StrFromF64 { x } => vec![*x],
            Op::ArrayPop { arr } => vec![*arr],
            Op::Bin { a, b, .. } | Op::Cmp { a, b, .. } => vec![*a, *b],
            Op::Un { a, .. } | Op::Conv { a, .. } | Op::Bitcast { a } => vec![*a],
            Op::Select { cond, a, b } => vec![*cond, *a, *b],
            Op::GetField { obj, .. }
            | Op::Tag { obj }
            | Op::IsUnique { obj }
            | Op::Clone { obj } => vec![*obj],
            Op::SetField { obj, val, .. } => vec![*obj, *val],
            Op::ArrayNew { len } => vec![*len],
            Op::ArrayLen { arr } => vec![*arr],
            Op::ArrayGet { arr, idx } => vec![*arr, *idx],
            Op::ArraySet { arr, idx, val } => vec![*arr, *idx, *val],
            Op::ArrayPush { arr, val } => vec![*arr, *val],
            Op::StrLen { s } => vec![*s],
            Op::StrFromUtf8 { bytes, len } => vec![*bytes, *len],
            Op::IsNull { val }
            | Op::Opt { val }
            | Op::Unopt { val }
            | Op::Retain { val }
            | Op::Release { val } => vec![*val],
            Op::ClosureNew { captures, .. } => captures.clone(),
            Op::Call { args, .. } | Op::Intrinsic { args, .. } => args.clone(),
            Op::CallClosure { callee, args } => std::iter::once(*callee)
                .chain(args.iter().copied())
                .collect(),
            Op::GlobalSet { val, .. } => vec![*val],
            Op::Load { ptr, .. } => vec![*ptr],
            Op::Store { ptr, val, .. } => vec![*ptr, *val],
        }
    }
}

impl Terminator {
    /// Mutable access to every value operand and block target (for mutation tests and passes).
    pub fn operands_mut(&mut self) -> Vec<&mut ValueId> {
        match self {
            Terminator::Br(t) => t.args.iter_mut().collect(),
            Terminator::BrIf { cond, then, els } => std::iter::once(cond)
                .chain(then.args.iter_mut())
                .chain(els.args.iter_mut())
                .collect(),
            Terminator::Switch {
                value,
                cases,
                default,
            } => std::iter::once(value)
                .chain(cases.iter_mut().flat_map(|(_, t)| t.args.iter_mut()))
                .chain(default.args.iter_mut())
                .collect(),
            Terminator::Ret(vs) => vs.iter_mut().collect(),
            Terminator::Trap(_) => vec![],
        }
    }

    /// All value operands (branch arguments, conditions, return values).
    pub fn operands(&self) -> Vec<ValueId> {
        match self {
            Terminator::Br(t) => t.args.clone(),
            Terminator::BrIf { cond, then, els } => std::iter::once(*cond)
                .chain(then.args.iter().copied())
                .chain(els.args.iter().copied())
                .collect(),
            Terminator::Switch {
                value,
                cases,
                default,
            } => std::iter::once(*value)
                .chain(cases.iter().flat_map(|(_, t)| t.args.iter().copied()))
                .chain(default.args.iter().copied())
                .collect(),
            Terminator::Ret(vs) => vs.clone(),
            Terminator::Trap(_) => vec![],
        }
    }

    pub fn targets(&self) -> Vec<&Target> {
        match self {
            Terminator::Br(t) => vec![t],
            Terminator::BrIf { then, els, .. } => vec![then, els],
            Terminator::Switch { cases, default, .. } => cases
                .iter()
                .map(|(_, t)| t)
                .chain(std::iter::once(default))
                .collect(),
            _ => vec![],
        }
    }
}
