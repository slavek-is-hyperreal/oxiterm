//! The reason-code registry: stable identifiers for every diagnostic,
//! placement decision, and law outcome the toolchain can emit.
//!
//! Source of truth: `registry/codes.txt` (embedded as [`SEED`]); later slices
//! append entries to that file and never edit or reuse existing ones.
//!
//! Code shape: `OXS-<class><4 digits>`, class letter one of
//!
//! | class | meaning | severity |
//! |---|---|---|
//! | `E` | compile error (syntax, types, profiles, capabilities) | error |
//! | `W` | warning | warning |
//! | `P` | placement / performance report entry | note |
//! | `L` | law outcome | error or warning |
//! | `I` | internal compiler error | internal |
//!
//! Numbering ranges for `E`: 0001-0099 lexical and syntax, 0100-0299 types,
//! 0300-0399 names and modules, 0400-0499 capabilities and host, 0500-0599
//! profiles, 0900-0999 reserved syntax. `P`: 01xx loops/SIMD, 02xx allocation,
//! 03xx transfers/GPU.
//!
//! File format: records separated by a blank line, one `key value` per line:
//!
//! ```text
//! code OXS-E0001
//! severity error
//! phase parse
//! title Unexpected token
//! message expected {expected}, found {found}
//! fix Insert the expected token or remove the unexpected one.
//! ```
//!
//! `{name}` placeholders in `message`/`fix` are filled by the emitter.
//! `P` records also carry `assertable yes|no`: `yes` means a *semantic fact*
//! that `@expect` may assert and that only ever gets stronger within an
//! edition; `no` means a heuristic choice that is reported but never
//! assertable.

use std::collections::HashSet;
use std::fmt;

pub const SEED: &str = include_str!("../registry/codes.txt");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
    Internal,
}

impl Severity {
    fn parse(s: &str) -> Option<Severity> {
        Some(match s {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            "note" => Severity::Note,
            "internal" => Severity::Internal,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeDef {
    /// `OXS-E0001`
    pub code: String,
    pub severity: Severity,
    /// Compiler phase that owns it: `lex`, `parse`, `check`, `lower`, `place`, `law`, `link`, `runtime`.
    pub phase: String,
    pub title: String,
    pub message: String,
    pub fix: Option<String>,
    /// `P` codes only.
    pub assertable: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryError {
    pub line: usize,
    pub msg: String,
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "registry:{}: {}", self.line, self.msg)
    }
}

impl std::error::Error for RegistryError {}

/// Is `s` shaped like a reason code?
pub fn is_valid_code(s: &str) -> bool {
    let b = s.as_bytes();
    s.len() == 9
        && s.starts_with("OXS-")
        && matches!(b[4], b'E' | b'W' | b'P' | b'L' | b'I')
        && b[5..].iter().all(u8::is_ascii_digit)
}

/// Parse and validate a registry text (unique codes; class letter agrees with
/// severity; required fields present).
pub fn parse_registry(text: &str) -> Result<Vec<CodeDef>, RegistryError> {
    let mut out: Vec<CodeDef> = Vec::new();
    let mut seen = HashSet::new();
    let mut cur: Vec<(usize, String, String)> = Vec::new();
    let mut flush = |cur: &mut Vec<(usize, String, String)>,
                     out: &mut Vec<CodeDef>|
     -> Result<(), RegistryError> {
        if cur.is_empty() {
            return Ok(());
        }
        for (n, (ln, k, _)) in cur.iter().enumerate() {
            if cur[..n].iter().any(|(_, k2, _)| k2 == k) {
                return Err(RegistryError {
                    line: *ln,
                    msg: format!(
                        "duplicate key `{k}` in one record (missing blank line between records?)"
                    ),
                });
            }
        }
        let line0 = cur[0].0;
        let err = |msg: String| RegistryError { line: line0, msg };
        let get = |k: &str| {
            cur.iter()
                .find(|(_, key, _)| key == k)
                .map(|(_, _, v)| v.clone())
        };
        for (ln, k, _) in cur.iter() {
            if ![
                "code",
                "severity",
                "phase",
                "title",
                "message",
                "fix",
                "assertable",
            ]
            .contains(&k.as_str())
            {
                return Err(RegistryError {
                    line: *ln,
                    msg: format!("unknown key `{k}`"),
                });
            }
        }
        let code = get("code").ok_or_else(|| err("missing `code`".into()))?;
        if !is_valid_code(&code) {
            return Err(err(format!(
                "`{code}` is not a valid code (OXS-<E|W|P|L|I><4 digits>)"
            )));
        }
        if !seen.insert(code.clone()) {
            return Err(err(format!("duplicate code {code}")));
        }
        let severity = get("severity")
            .and_then(|s| Severity::parse(&s))
            .ok_or_else(|| err(format!("{code}: missing or invalid `severity`")))?;
        let class = code.as_bytes()[4];
        let ok = match class {
            b'E' => severity == Severity::Error,
            b'W' => severity == Severity::Warning,
            b'P' => severity == Severity::Note,
            b'L' => matches!(severity, Severity::Error | Severity::Warning),
            _ => severity == Severity::Internal,
        };
        if !ok {
            return Err(err(format!(
                "{code}: severity {severity:?} does not match class `{}`",
                class as char
            )));
        }
        let assertable = match get("assertable").as_deref() {
            None => None,
            Some("yes") => Some(true),
            Some("no") => Some(false),
            Some(o) => return Err(err(format!("{code}: assertable must be yes|no, got `{o}`"))),
        };
        if class == b'P' && assertable.is_none() {
            return Err(err(format!(
                "{code}: placement codes need `assertable yes|no`"
            )));
        }
        if class != b'P' && assertable.is_some() {
            return Err(err(format!("{code}: `assertable` is only for P codes")));
        }
        out.push(CodeDef {
            phase: get("phase").ok_or_else(|| err(format!("{code}: missing `phase`")))?,
            title: get("title").ok_or_else(|| err(format!("{code}: missing `title`")))?,
            message: get("message").ok_or_else(|| err(format!("{code}: missing `message`")))?,
            fix: get("fix"),
            code,
            severity,
            assertable,
        });
        cur.clear();
        Ok(())
    };
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_end();
        if line.starts_with('#') {
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut cur, &mut out)?;
            continue;
        }
        let (k, v) = line.split_once(' ').ok_or(RegistryError {
            line: i + 1,
            msg: "expected `key value`".into(),
        })?;
        cur.push((i + 1, k.to_string(), v.trim().to_string()));
    }
    flush(&mut cur, &mut out)?;
    Ok(out)
}

/// The seed registry, parsed. Panics only if the embedded file is invalid
/// (a unit test guards that).
pub fn seed() -> Vec<CodeDef> {
    parse_registry(SEED).expect("embedded registry is valid")
}

/// Look up a code in the seed registry.
pub fn lookup(code: &str) -> Option<CodeDef> {
    seed().into_iter().find(|c| c.code == code)
}

/// Fill `{name}` placeholders in a message template.
pub fn render(template: &str, args: &[(&str, &str)]) -> String {
    // single pass: substituted values are never re-scanned
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}').map(|c| (&after[..c], c)) {
            Some((name, close)) if args.iter().any(|(k, _)| *k == name) => {
                out.push_str(args.iter().find(|(k, _)| *k == name).map_or("", |(_, v)| v));
                rest = &after[close + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_valid_and_has_each_class() {
        let defs = parse_registry(SEED).unwrap();
        assert!(defs.len() >= 20);
        for class in ['E', 'W', 'P', 'L', 'I'] {
            assert!(
                defs.iter().any(|d| d.code.as_bytes()[4] == class as u8),
                "class {class}"
            );
        }
        // codes are listed in ascending order inside the file (review aid)
        let mut sorted: Vec<_> = defs.iter().map(|d| d.code.clone()).collect();
        sorted.sort();
        let orig: Vec<_> = defs.iter().map(|d| d.code.clone()).collect();
        assert_eq!(orig, sorted, "keep codes sorted");
    }

    #[test]
    fn lookup_and_render() {
        let d = lookup("OXS-E0001").unwrap();
        assert_eq!(
            render(&d.message, &[("expected", "`)`"), ("found", "`;`")]),
            "expected `)`, found `;`"
        );
        assert!(lookup("OXS-E9999").is_none());
    }

    #[test]
    fn validation_catches_mistakes() {
        let dup = "code OXS-E0001\nseverity error\nphase parse\ntitle t\nmessage m\n\ncode OXS-E0001\nseverity error\nphase parse\ntitle t\nmessage m\n";
        assert!(parse_registry(dup).unwrap_err().msg.contains("duplicate"));
        let bad_class = "code OXS-W0001\nseverity error\nphase parse\ntitle t\nmessage m\n";
        assert!(parse_registry(bad_class).is_err());
        let p_without = "code OXS-P0001\nseverity note\nphase place\ntitle t\nmessage m\n";
        assert!(parse_registry(p_without).is_err());
        assert!(!is_valid_code("OXS-E001"));
        assert!(!is_valid_code("OXS-X0001"));
        assert!(is_valid_code("OXS-P0102"));
    }
}
