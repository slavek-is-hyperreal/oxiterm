//! Stage 8.Sec regression tests for `oxiscript-syntax` (`[R3-201]`).

use oxiscript_syntax::{lex, parse_module, FileId};

const BIDI_CHARS: &[(char, &str)] = &[
    ('\u{061c}', "ALM"),
    ('\u{200e}', "LRM"),
    ('\u{200f}', "RLM"),
    ('\u{202a}', "LRE"),
    ('\u{202b}', "RLE"),
    ('\u{202c}', "PDF"),
    ('\u{202d}', "LRO"),
    ('\u{202e}', "RLO"),
    ('\u{2066}', "LRI"),
    ('\u{2067}', "RLI"),
    ('\u{2068}', "FSI"),
    ('\u{2069}', "PDI"),
    ('\u{200b}', "ZWSP"),
];

#[test]
fn bidi_and_invisible_warned_in_line_doc_and_block_comments() {
    for &(ch, name) in BIDI_CHARS {
        let cases = [
            ("line", format!("// before{ch}after\nexport function f(): i32 {{ return 1; }}\n")),
            ("doc", format!("/// doc{ch}comment\nexport function f(): i32 {{ return 1; }}\n")),
            ("block", format!("/* block{ch}comment */\nexport function f(): i32 {{ return 1; }}\n")),
            ("string", format!("export function f(): string {{ return \"hi{ch}there\"; }}\n")),
            ("template", format!("export function f(): string {{ return `hi{ch}there`; }}\n")),
        ];
        for (where_kind, src) in cases {
            let (_m, diags) = parse_module(&src);
            assert!(
                diags.iter().any(|d| d.code == "OXS-W0003"),
                "expected OXS-W0003 for {name} (U+{:04X}) in {where_kind} comment/literal, got {diags:?}",
                ch as u32,
            );
        }
    }
}

#[test]
fn escaped_bidi_in_string_is_allowed_and_normal_comment_whitespace_is_clean() {
    let src = "/* multi\n\tline\r\ncomment */\n/// doc\tcomment\n// normal line\nexport function f(): string {\n  return \"\\u{202e}\\u{2066}ok\\u{2069}\";\n}\n";
    let lx = lex(src, FileId(0));
    assert!(lx.diags.is_empty(), "unexpected diagnostics: {:?}", lx.diags);
}
