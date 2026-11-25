#![allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::match_same_arms,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::wildcard_imports,
    clippy::struct_excessive_bools,
    clippy::large_enum_variant,
    clippy::option_option,
    clippy::ref_option,
    clippy::trivially_copy_pass_by_ref,
    clippy::redundant_closure_for_method_calls,
    clippy::single_match_else,
    clippy::format_push_string,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::self_only_used_in_recursion,
    clippy::unused_self,
    clippy::return_self_not_must_use,
    clippy::doc_markdown,
    clippy::needless_lifetimes,
    clippy::manual_let_else,
    clippy::unnecessary_wraps,
    clippy::if_not_else,
    clippy::map_unwrap_or,
    clippy::cognitive_complexity,
    clippy::collapsible_else_if,
    clippy::while_let_loop,
    clippy::naive_bytecount
)]
//! Registry consistency, keyword lists, lexer details and formatter style.
mod common;
use oxiscript_syntax::lexer::{is_reserved_word, keyword, Tk};
use oxiscript_syntax::*;

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn every_code_used_by_the_syntax_crate_is_in_the_registry() {
    let registry = read("oxiscript/oxiscript-abi/registry/codes.txt");
    let mut found = 0;
    for dir in ["src", "src/parser", "src/fmt"] {
        for e in std::fs::read_dir(format!("{}/{dir}", env!("CARGO_MANIFEST_DIR"))).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            let mut i = 0;
            while let Some(at) = text[i..].find("\"OXS-") {
                let s = i + at + 1;
                let code = &text[s..(s + 9).min(text.len())];
                assert!(
                    registry.contains(&format!("code {code}\n")),
                    "{} uses unregistered {code}",
                    p.display()
                );
                found += 1;
                i = s + 4;
            }
        }
    }
    assert!(found > 20);
}

#[test]
fn lexer_word_lists_match_keywords_rs() {
    for w in keywords::KEYWORDS {
        assert!(keyword(w).is_some(), "{w}");
    }
    for w in keywords::RESERVED {
        assert!(is_reserved_word(w), "{w}");
        assert!(keyword(w).is_none());
    }
    for w in keywords::CONTEXTUAL.iter().chain(keywords::BUILTIN_TYPES) {
        assert!(keyword(w).is_none() && !is_reserved_word(w), "{w}");
    }
}

fn kinds(src: &str) -> Vec<Tk> {
    lex(src, FileId(0)).tokens.iter().map(|t| t.kind).collect()
}

#[test]
fn lexer_numbers() {
    use Tk::*;
    assert_eq!(
        kinds("1 1.5 .5 1e3 1.5e-3 0xff 0b1 0o7 1u8 0xffu8 1.0f32 .5f32 1e3f64"),
        [Int, Float, Float, Float, Float, Int, Int, Int, Int, Int, Float, Float, Float, Eof]
    );
    assert_eq!(kinds("0x1f32"), [Int, Eof]);
    assert_eq!(kinds("t.0"), [Ident, Float, Eof]);
    assert_eq!(kinds("1..2"), [Int, Dot, Float, Eof]);
    assert_eq!(kinds("a?.5:1"), [Ident, Question, Float, Colon, Int, Eof]);
    assert_eq!(kinds("a?.b"), [Ident, QDot, Ident, Eof]);
    let lx = lex("1f32", FileId(0));
    assert_eq!(lx.diags.len(), 1);
    assert_eq!(lex("1_000 1__0 1_", FileId(0)).diags.len(), 2);
    assert_eq!(
        lex("340282366920938463463374607431768211455", FileId(0))
            .diags
            .len(),
        0
    );
    assert_eq!(
        lex("340282366920938463463374607431768211456", FileId(0))
            .diags
            .len(),
        1
    );
}

#[test]
fn lexer_operators_and_templates() {
    use Tk::*;
    assert_eq!(
        kinds("+% -% *% +| -| *| +%= *|= >>= >= >> > <<= <= === !== == != => ++ --"),
        [
            PlusWrap, MinusWrap, StarWrap, PlusSat, MinusSat, StarSat, PlusWrapEq, StarSatEq,
            ShrEq, Ge, Shr, Gt, ShlEq, Le, EqEqEq, NotEqEq, EqEq, NotEq, FatArrow, PlusPlus,
            MinusMinus, Eof
        ]
    );
    assert_eq!(
        kinds("`a${x}b${y}c` `plain`"),
        [
            TemplateHead,
            Ident,
            TemplateMiddle,
            Ident,
            TemplateTail,
            TemplateFull,
            Eof
        ]
    );
    assert_eq!(
        kinds("`${ {a: 1} }`"),
        [
            TemplateHead,
            LBrace,
            Ident,
            Colon,
            Int,
            RBrace,
            TemplateTail,
            Eof
        ]
    );
    assert_eq!(
        kinds("`a${`b${c}`}`"),
        [
            TemplateHead,
            TemplateHead,
            Ident,
            TemplateTail,
            TemplateTail,
            Eof
        ]
    );
}

#[test]
fn lexer_trivia_and_odd_input() {
    let lx = lex("\u{feff}a // c\n/* b */ b /// d\n//// e\n", FileId(0));
    assert_eq!(lx.comments.len(), 4);
    use oxiscript_syntax::CommentKind::*;
    assert_eq!(
        lx.comments.iter().map(|c| c.kind).collect::<Vec<_>>(),
        [Line, Block, Doc, Line]
    );
    assert_eq!(
        lx.tokens[0].lo, 3,
        "BOM is skipped, spans stay byte offsets of the original"
    );
    assert!(lx.tokens[1].nl && lx.tokens[1].ws);
    // CRLF doc comments do not keep the CR
    let lx = lex("/// d\r\nx", FileId(0));
    assert_eq!(lx.comments[0].hi, 5);
    for s in [
        "\0",
        "\u{7f}",
        "é",
        "\u{1F600}x",
        "a\u{200b}b",
        "\"\u{0}\"",
        "'\\u{10FFFF}'",
        "\"\\u{110000}\"",
    ] {
        let _ = parse_module(s);
    }
    assert_eq!(parse_module("const s = \"\\u{1F600}\\x41\";").1.len(), 0);
    assert_eq!(parse_module("const s = \"\\x80\";").1[0].code, "OXS-E0013");
    assert_eq!(
        parse_module("const s = \"\\u{d800}\";").1[0].code,
        "OXS-E0013"
    );
    assert_eq!(parse_module("const s = \"a\nb\";").1[0].code, "OXS-E0002");
}

#[test]
fn formatter_canonical_style() {
    let f = |s: &str| format(s).unwrap();
    assert_eq!(f("let   x=1 ;"), "let x = 1;\n");
    assert_eq!(
        f("function f(){if(a)b();else c();}"),
        "function f() {\n  if (a) {\n    b();\n  } else {\n    c();\n  }\n}\n"
    );
    assert_eq!(f("const s='it\\'s';"), "const s = \"it's\";\n");
    assert!(format("const y = f()  ?;").is_err());
    assert_eq!(f("const y = f()?;"), "const y = f()?;\n");
    assert_eq!(
        f("let a = 1;\n\n\n\nlet b = 2;"),
        "let a = 1;\n\nlet b = 2;\n"
    );
    assert_eq!(f(""), "");
    assert_eq!(f("// only a comment"), "// only a comment\n");
    assert_eq!(
        f("\u{feff}let a=1;\r\nlet b=2;\r\n"),
        "let a = 1;\nlet b = 2;\n"
    );
    assert_eq!(f("let x=- -y;"), "let x = - -y;\n");
    assert_eq!(f("let x=a as (u8|i8);"), "let x = a as (u8 | i8);\n");
    assert_eq!(
        f("function g(){for(let i=0;i<3;i++)f(i);}"),
        "function g() {\n  for (let i = 0; i < 3; i++) {\n    f(i);\n  }\n}\n"
    );
    // width 100
    let long = format!(
        "const x = f({});",
        (0..40)
            .map(|i| format!("arg{i}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let out = f(&long);
    assert!(out.lines().all(|l| l.chars().count() <= 100), "{out}");
    assert!(out.lines().count() > 3);
    // 2-space indentation
    assert!(f("function f(){if(a){b();}}").contains("\n    b();"));
}

#[test]
fn formatter_keeps_every_comment() {
    let src = "/* a */ function /* b */ f /* c */ ( /* d */ x: /* e */ i32 /* f */ ) /* g */ : /* h */ i32 /* i */ { // j\n return /* k */ x /* l */ ; /* m */ } // n\n/* o */\n";
    let out = format(src).unwrap();
    for c in "abcdefghijklmno".chars() {
        let needle = format!(" {c} ");
        let alt = format!("// {c}");
        assert!(
            out.contains(&needle) || out.contains(&alt),
            "comment {c} lost in:\n{out}"
        );
    }
    assert_eq!(format(&out).unwrap(), out);
}

#[test]
fn formatter_errors_on_invalid_input() {
    let e = format("let = ;").unwrap_err();
    assert!(!e.diagnostics.is_empty());
    assert!(e.to_string().contains("OXS-E0001"));
}

#[test]
fn deep_but_legal_input_formats_on_a_small_stack() {
    // nesting within the default limit formats on a 2 MB stack (threads are only an
    // optimisation; if the platform cannot create one the test has nothing to check)
    let n = 40;
    let src = format!("const a = {}1{};", "f(".repeat(n), ")".repeat(n));
    let o = FormatOptions {
        parse: ParseOptions {
            use_thread: false,
            ..ParseOptions::default()
        },
        ..FormatOptions::default()
    };
    let src2 = src.clone();
    let r = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || (format(&src2).unwrap(), format_with(&src2, &o).unwrap()))
        .map(|h| h.join());
    if let Ok(joined) = r {
        let (a, b) = joined.unwrap();
        assert!(a.starts_with("const a = f("));
        assert_eq!(a, b, "the helper thread must not change the result");
    }
}

/// Every fenced `ts` block of grammar.md is a program and parses.
#[test]
fn grammar_md_examples_parse() {
    let g = read("docs/oxiscript/grammar.md");
    let mut n = 0;
    let mut rest = g.as_str();
    while let Some(a) = rest.find("```ts\n") {
        let body = &rest[a + 6..];
        let end = body.find("```").unwrap();
        let code = &body[..end];
        let (_, d) = parse_module(code);
        assert!(d.is_empty(), "grammar.md example failed:\n{code}\n{d:#?}");
        let f = format(code).unwrap();
        assert_eq!(format(&f).unwrap(), f);
        n += 1;
        rest = &body[end + 3..];
    }
    assert!(n >= 8, "found only {n} examples");
}
