#![allow(dead_code)]
use oxiscript_syntax::*;

/// Debug rendering of a module without spans and node ids.
pub fn shape(m: &Module) -> String {
    unwrap_paren_try(&(strip(&format!("{:?}", m.items)) + &strip(&format!("{:?}", m.attrs))))
}

/// The formatter spells `f()?.y` as `(f()?).y` (grammar 1.5: without the
/// parentheses it would lex as optional chaining), which adds a `Paren` node.
/// Shapes are compared with `Paren(Try(..))` replaced by `Try(..)`.
pub fn unwrap_paren_try(s: &str) -> String {
    const PAT: &str = "Paren(Expr { id: , span: , kind: Try(";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find(PAT) {
        out.push_str(&rest[..p]);
        let inner = &rest[p + "Paren(".len()..];
        // find the `)` closing `Paren(`, skipping string literals
        let (mut depth, mut in_str, mut esc) = (1usize, false, false);
        let mut end = inner.len();
        for (i, c) in inner.char_indices() {
            if in_str {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push_str(&inner[..end]);
        rest = inner.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

pub fn strip(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("Span { file: FileId(") {
            if let Some(e) = r.find('}') {
                rest = &r[e + 1..];
                continue;
            }
        }
        if let Some(r) = rest.strip_prefix("NodeId(") {
            if let Some(e) = r.find(')') {
                rest = &r[e + 1..];
                continue;
            }
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

pub fn corpus(dir: &str) -> Vec<(String, String)> {
    let d = format!("{}/tests/corpus/{dir}", env!("CARGO_MANIFEST_DIR"));
    let mut v: Vec<(String, String)> = std::fs::read_dir(d)
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "oxs") {
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                Some((name, std::fs::read_to_string(&p).unwrap()))
            } else {
                None
            }
        })
        .collect();
    v.sort();
    v
}

/// Compact s-expression of an expression (enough to assert parse shapes).
pub fn sx(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Lit(l) => match &l.kind {
            LitKind::Int { value, suffix } => format!("{value}{}", suffix.map_or("", NumTy::name)),
            LitKind::Float { text, suffix } => format!("{text}{}", suffix.map_or("", NumTy::name)),
            LitKind::Str(s) => format!("{s:?}"),
            LitKind::Bool(b) => b.to_string(),
            LitKind::Null => "null".into(),
        },
        ExprKind::Ident(i) => i.name.clone(),
        ExprKind::This => "this".into(),
        ExprKind::Paren(x) => format!("(paren {})", sx(x)),
        ExprKind::Binary { op, lhs, rhs } => {
            format!(
                "({} {} {})",
                oxiscript_syntax::parser::op_text(*op),
                sx(lhs),
                sx(rhs)
            )
        }
        ExprKind::Unary { op, expr } => format!("({op:?} {})", sx(expr)),
        ExprKind::Cond { cond, then, els } => format!("(? {} {} {})", sx(cond), sx(then), sx(els)),
        ExprKind::Call {
            callee,
            type_args,
            args,
            optional,
        } => {
            let a: Vec<String> = args.iter().map(sx).collect();
            format!(
                "(call{} {}{} {})",
                if *optional { "?" } else { "" },
                sx(callee),
                if type_args.is_empty() {
                    String::new()
                } else {
                    format!("<{}>", type_args.len())
                },
                a.join(" ")
            )
        }
        ExprKind::Member {
            obj,
            name,
            optional,
        } => format!(
            "({} {} {})",
            if *optional { "?." } else { "." },
            sx(obj),
            name.name
        ),
        ExprKind::Index {
            obj,
            index,
            optional,
        } => format!(
            "({} {} {})",
            if *optional { "?[]" } else { "[]" },
            sx(obj),
            sx(index)
        ),
        ExprKind::Try(x) => format!("(try {})", sx(x)),
        ExprKind::Cast { expr, .. } => format!("(as {})", sx(expr)),
        ExprKind::Arrow(a) => {
            let ps: Vec<&str> = a.params.iter().map(|p| p.name.name.as_str()).collect();
            let body = match &a.body {
                ArrowBody::Expr(x) => sx(x),
                ArrowBody::Block(_) => "{}".into(),
            };
            format!(
                "(arrow [{}]{} {})",
                ps.join(","),
                if a.ret.is_some() { ":T" } else { "" },
                body
            )
        }
        ExprKind::Assign { target, value, .. } => format!("(= {} {})", sx(target), sx(value)),
        ExprKind::Switch(s) => format!("(switch {} arms={})", sx(&s.scrutinee), s.arms.len()),
        ExprKind::Update { target, prefix, .. } => format!(
            "(upd{} {})",
            if *prefix { "pre" } else { "post" },
            sx(target)
        ),
        ExprKind::Array(v) => format!("(array {})", v.iter().map(sx).collect::<Vec<_>>().join(" ")),
        ExprKind::Object(v) => format!("(object {})", v.len()),
        ExprKind::New { args, .. } => format!("(new {})", args.len()),
        ExprKind::Template(p) => format!("(template {})", p.len()),
    }
}

/// The initialiser of `const x = SRC;`, as an s-expression; panics if it has diagnostics.
pub fn init_sx(src: &str) -> String {
    let text = format!("const x = {src};");
    let (m, d) = parse_module(&text);
    assert!(d.is_empty(), "{src}: {d:#?}");
    match &m.items[0].kind {
        ItemKind::Var(v) => sx(v.init.as_ref().unwrap()),
        _ => panic!(),
    }
}

pub fn codes(src: &str) -> Vec<&'static str> {
    parse_module(src).1.iter().map(|d| d.code).collect()
}
