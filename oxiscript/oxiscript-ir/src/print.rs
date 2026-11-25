//! The stable OxiIR text form: printer. Grammar: see `docs/oxiscript/ir.md`.
//! `parse(print(m)) == m` and `print(parse(t)) == t` for canonical text.

use crate::types::*;
use std::fmt::Write;

pub const TEXT_VERSION: u32 = 1;

pub fn print_module(m: &Module) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "oxiir {TEXT_VERSION}");
    let _ = write!(s, "module {}", quote(&m.name));
    print_attrs(&mut s, &m.attrs);
    s.push('\n');

    if !m.types.is_empty() {
        s.push('\n');
    }
    for t in &m.types {
        let _ = write!(s, "type {}", t.name);
        print_attrs(&mut s, &t.attrs);
        s.push_str(" = ");
        match &t.kind {
            TypeKind::Struct { fields } => {
                s.push_str("struct ");
                print_fields(&mut s, fields);
            }
            TypeKind::Closure { captures } => {
                s.push_str("closure ");
                print_fields(&mut s, captures);
            }
            TypeKind::Union { variants } => {
                s.push_str("union {");
                for (i, v) in variants.iter().enumerate() {
                    s.push_str(if i == 0 { " " } else { ", " });
                    s.push_str(&v.name);
                    if !v.fields.is_empty() {
                        s.push(' ');
                        print_fields(&mut s, &v.fields);
                    }
                }
                s.push_str(if variants.is_empty() { "}" } else { " }" });
            }
        }
        s.push('\n');
    }

    if !m.imports.is_empty() {
        s.push('\n');
    }
    for i in &m.imports {
        let _ = writeln!(
            s,
            "import @{} = {} {} {}",
            i.name,
            quote(&i.module),
            quote(&i.field),
            sig_text(&i.sig)
        );
    }

    if !m.data.is_empty() {
        s.push('\n');
    }
    for d in &m.data {
        let _ = writeln!(s, "data @{} = {}", d.name, quote(&d.text));
    }

    if !m.globals.is_empty() {
        s.push('\n');
    }
    for g in &m.globals {
        let _ = write!(s, "global @{}: {}", g.name, ty_text(&g.ty));
        if let Some(c) = &g.init {
            let _ = write!(s, " = {}", const_text(c));
        }
        s.push('\n');
    }

    for f in &m.funcs {
        s.push('\n');
        print_func(&mut s, f);
    }
    s
}

fn print_fields(s: &mut String, fields: &[Field]) {
    s.push('{');
    for (i, f) in fields.iter().enumerate() {
        s.push_str(if i == 0 { " " } else { ", " });
        let _ = write!(s, "{}: {}", f.name, ty_text(&f.ty));
    }
    s.push_str(if fields.is_empty() { "}" } else { " }" });
}

fn print_attrs(s: &mut String, attrs: &[Attr]) {
    for a in attrs {
        let _ = write!(s, " #{}", a.name);
        if !a.args.is_empty() {
            s.push('(');
            for (i, arg) in a.args.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                if is_bare_arg(arg) {
                    s.push_str(arg);
                } else {
                    s.push_str(&quote(arg));
                }
            }
            s.push(')');
        }
    }
}

fn is_bare_arg(a: &str) -> bool {
    if a.is_empty() {
        return false;
    }
    let first = a.chars().next().unwrap();
    if first.is_ascii_digit() {
        // lexes as an integer token: only canonical decimal integers survive
        return a.chars().all(|c| c.is_ascii_digit())
            && (a == "0" || !a.starts_with('0'))
            && a.len() < 30;
    }
    if a.contains(':') {
        // `host:log`: identifiers joined by single colons
        return a.split(':').all(|seg| {
            seg.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && seg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.'))
        });
    }
    (first.is_ascii_alphabetic() || first == '_')
        && a.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.'))
}

pub fn quote(t: &str) -> String {
    let mut s = String::from("\"");
    for c in t.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(s, "\\u{{{:x}}}", c as u32);
            }
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

pub fn ty_text(t: &Ty) -> String {
    match t {
        Ty::Bool => "bool".into(),
        Ty::I8 => "i8".into(),
        Ty::I16 => "i16".into(),
        Ty::I32 => "i32".into(),
        Ty::I64 => "i64".into(),
        Ty::U8 => "u8".into(),
        Ty::U16 => "u16".into(),
        Ty::U32 => "u32".into(),
        Ty::U64 => "u64".into(),
        Ty::F32 => "f32".into(),
        Ty::F64 => "f64".into(),
        Ty::Ptr => "ptr".into(),
        Ty::Str => "str".into(),
        Ty::Ref(n) => format!("ref {n}"),
        Ty::Array(e) => format!("[{}]", ty_text(e)),
        Ty::Fn(sig) => format!("fn{}", sig_text(sig)),
        Ty::Opt(inner) => match **inner {
            Ty::Fn(_) => format!("({})?", ty_text(inner)),
            _ => format!("{}?", ty_text(inner)),
        },
    }
}

/// `(i32, str) -> void` / `-> i32` / `-> (i32, i64)`.
pub fn sig_text(sig: &Sig) -> String {
    let ps: Vec<String> = sig.params.iter().map(ty_text).collect();
    format!("({}) -> {}", ps.join(", "), rets_text(&sig.rets))
}

fn rets_text(rets: &[Ty]) -> String {
    match rets {
        [] => "void".into(),
        [one] => ty_text(one),
        many => format!(
            "({})",
            many.iter().map(ty_text).collect::<Vec<_>>().join(", ")
        ),
    }
}

pub fn fmt_f64(bits: u64) -> String {
    let v = f64::from_bits(bits);
    if v.is_nan() {
        format!("nan:0x{bits:016x}")
    } else if v.is_infinite() {
        if v > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        }
    } else {
        let t = format!("{v:?}");
        debug_assert_eq!(t.parse::<f64>().map(f64::to_bits), Ok(bits));
        t
    }
}

pub fn fmt_f32(bits: u32) -> String {
    let v = f32::from_bits(bits);
    if v.is_nan() {
        format!("nan:0x{bits:08x}")
    } else if v.is_infinite() {
        if v > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        }
    } else {
        let t = format!("{v:?}");
        debug_assert_eq!(t.parse::<f32>().map(f32::to_bits), Ok(bits));
        t
    }
}

pub fn const_text(c: &Const) -> String {
    match c {
        Const::Bool(b) => b.to_string(),
        Const::Int(i) => i.to_string(),
        Const::F32(b) => fmt_f32(*b),
        Const::F64(b) => fmt_f64(*b),
    }
}

fn print_func(s: &mut String, f: &Func) {
    if f.export {
        s.push_str("export ");
    }
    let _ = write!(s, "func @{}(", f.name);
    for (i, p) in f.params.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        let v = f.value(p.value);
        let _ = write!(
            s,
            "%{}: {}{}",
            v.name,
            if p.own { "own " } else { "" },
            ty_text(&v.ty)
        );
    }
    let _ = write!(s, ") -> {}", rets_text(&f.rets));
    print_attrs(s, &f.attrs);
    s.push_str(" {\n");
    let name = |v: &ValueId| format!("%{}", f.value(*v).name);
    let names = |vs: &[ValueId]| vs.iter().map(&name).collect::<Vec<_>>().join(", ");
    let target = |t: &Target| format!("{}({})", f.block(t.block).label, names(&t.args));
    for b in &f.blocks {
        let _ = write!(s, "{}", b.label);
        if !b.params.is_empty() {
            s.push('(');
            for (i, p) in b.params.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                let v = f.value(*p);
                let _ = write!(s, "%{}: {}", v.name, ty_text(&v.ty));
            }
            s.push(')');
        }
        s.push_str(":\n");
        for inst in &b.insts {
            s.push_str("  ");
            if !inst.results.is_empty() {
                for (i, r) in inst.results.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    let v = f.value(*r);
                    let _ = write!(s, "%{}: {}", v.name, ty_text(&v.ty));
                }
                s.push_str(" = ");
            }
            s.push_str(&op_text(&inst.op, &name, &names));
            print_loc(s, inst.loc);
            s.push('\n');
        }
        s.push_str("  ");
        match &b.term {
            Terminator::Br(t) => {
                let _ = write!(s, "br {}", target(t));
            }
            Terminator::BrIf { cond, then, els } => {
                let _ = write!(s, "br_if {}, {}, {}", name(cond), target(then), target(els));
            }
            Terminator::Switch {
                value,
                cases,
                default,
            } => {
                let _ = write!(s, "switch {}, [", name(value));
                for (i, (k, t)) in cases.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    let _ = write!(s, "{k}: {}", target(t));
                }
                let _ = write!(s, "], default {}", target(default));
            }
            Terminator::Ret(vs) => {
                s.push_str("ret");
                if !vs.is_empty() {
                    let _ = write!(s, " {}", names(vs));
                }
            }
            Terminator::Trap(k) => {
                let _ = write!(s, "trap {}", k.name());
            }
        }
        print_loc(s, b.term_loc);
        s.push('\n');
    }
    s.push_str("}\n");
}

fn print_loc(s: &mut String, loc: Option<Loc>) {
    if let Some(l) = loc {
        let _ = write!(s, " !{}:{}-{}", l.file, l.lo, l.hi);
    }
}

fn ovf_suffix(o: Ovf) -> &'static str {
    match o {
        Ovf::Trap => "",
        Ovf::Wrap => ".wrap",
        Ovf::Sat => ".sat",
    }
}

pub fn binop_name(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "add",
        BinOp::Sub => "sub",
        BinOp::Mul => "mul",
        BinOp::Div => "div",
        BinOp::Rem => "rem",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::Xor => "xor",
        BinOp::Shl => "shl",
        BinOp::Shr => "shr",
        BinOp::Min => "min",
        BinOp::Max => "max",
    }
}

pub fn unop_name(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "neg",
        UnOp::Not => "not",
        UnOp::Abs => "abs",
        UnOp::Sqrt => "sqrt",
        UnOp::Floor => "floor",
        UnOp::Ceil => "ceil",
        UnOp::Trunc => "trunc",
        UnOp::Nearest => "nearest",
        UnOp::Clz => "clz",
        UnOp::Ctz => "ctz",
        UnOp::Popcnt => "popcnt",
    }
}

pub fn cmpop_name(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "eq",
        CmpOp::Ne => "ne",
        CmpOp::Lt => "lt",
        CmpOp::Le => "le",
        CmpOp::Gt => "gt",
        CmpOp::Ge => "ge",
    }
}

fn type_ref_text(t: &TypeRef) -> String {
    match &t.variant {
        Some(v) => format!("{}::{}", t.ty, v),
        None => t.ty.clone(),
    }
}

fn field_ref_text(f: &FieldRef) -> String {
    match &f.variant {
        Some(v) => format!("{}::{}.{}", f.ty, v, f.field),
        None => format!("{}.{}", f.ty, f.field),
    }
}

fn op_text(op: &Op, n: &dyn Fn(&ValueId) -> String, ns: &dyn Fn(&[ValueId]) -> String) -> String {
    match op {
        Op::Const(c) => format!("const {}", const_text(c)),
        Op::Null => "null".into(),
        Op::Bin { op, ovf, a, b } => {
            format!("{}{} {}, {}", binop_name(*op), ovf_suffix(*ovf), n(a), n(b))
        }
        Op::Un { op, ovf, a } => format!("{}{} {}", unop_name(*op), ovf_suffix(*ovf), n(a)),
        Op::Cmp { op, a, b } => format!("{} {}, {}", cmpop_name(*op), n(a), n(b)),
        Op::Conv { ovf, a } => format!("conv{} {}", ovf_suffix(*ovf), n(a)),
        Op::Bitcast { a } => format!("bitcast {}", n(a)),
        Op::Select { cond, a, b } => format!("select {}, {}, {}", n(cond), n(a), n(b)),
        Op::Alloc(t) => format!("alloc {}", type_ref_text(t)),
        Op::GetField { field, obj } => format!("getfield {} {}", field_ref_text(field), n(obj)),
        Op::SetField {
            field,
            obj,
            val,
            init,
        } => {
            format!(
                "setfield{} {} {}, {}",
                if *init { ".init" } else { "" },
                field_ref_text(field),
                n(obj),
                n(val)
            )
        }
        Op::Tag { obj } => format!("tag {}", n(obj)),
        Op::IsUnique { obj } => format!("is_unique {}", n(obj)),
        Op::Clone { obj } => format!("clone {}", n(obj)),
        Op::ArrayNew { len } => format!("array.new {}", n(len)),
        Op::ArrayLen { arr } => format!("array.len {}", n(arr)),
        Op::ArrayGet { arr, idx } => format!("array.get {}, {}", n(arr), n(idx)),
        Op::ArraySet { arr, idx, val } => format!("array.set {}, {}, {}", n(arr), n(idx), n(val)),
        Op::ArrayPush { arr, val } => format!("array.push {}, {}", n(arr), n(val)),
        Op::StrConst { data } => format!("str.const @{data}"),
        Op::StrLen { s } => format!("str.len {}", n(s)),
        Op::StrFromUtf8 { bytes, len } => format!("str.from_utf8 {}, {}", n(bytes), n(len)),
        Op::StrConcat { a, b } => format!("str.concat {}, {}", n(a), n(b)),
        Op::StrEq { a, b } => format!("str.eq {}, {}", n(a), n(b)),
        Op::StrCmp { a, b } => format!("str.cmp {}, {}", n(a), n(b)),
        Op::StrSlice { s, lo, hi } => format!("str.slice {}, {}, {}", n(s), n(lo), n(hi)),
        Op::StrByte { s, idx } => format!("str.byte {}, {}", n(s), n(idx)),
        Op::StrFromF64 { x } => format!("str.from_f64 {}", n(x)),
        Op::ArrayEmpty => "array.empty".to_string(),
        Op::ArrayPop { arr } => format!("array.pop {}", n(arr)),
        Op::IsNull { val } => format!("is_null {}", n(val)),
        Op::Opt { val } => format!("opt {}", n(val)),
        Op::Unopt { val } => format!("unopt {}", n(val)),
        Op::ClosureNew {
            env,
            func,
            captures,
        } => format!("closure.new {env} @{func}({})", ns(captures)),
        Op::Call { callee, args } => format!("call @{callee}({})", ns(args)),
        Op::CallClosure { callee, args } => format!("call_closure {}({})", n(callee), ns(args)),
        Op::Intrinsic { name, args } => format!("intrinsic {name}({})", ns(args)),
        Op::Retain { val } => format!("retain {}", n(val)),
        Op::Release { val } => format!("release {}", n(val)),
        Op::GlobalGet { name } => format!("global.get @{name}"),
        Op::GlobalSet { name, val } => format!("global.set @{name}, {}", n(val)),
        Op::Load { ptr, offset } => format!("load {}, {offset}", n(ptr)),
        Op::Store { ptr, offset, val } => format!("store {}, {offset}, {}", n(ptr), n(val)),
    }
}
