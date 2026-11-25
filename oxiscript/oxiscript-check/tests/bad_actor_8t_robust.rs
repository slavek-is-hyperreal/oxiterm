#![allow(clippy::format_push_string)]
//! BAD ACTOR 8.T: determinism, span sanity, registry codes, side-table invariants over a pool of
//! hand-written, valid-corpus, generated and mutated programs; everything on a 1 MB stack.
mod common;
#[path = "bad_actor_8t_probe.rs"]
mod p1;
#[path = "bad_actor_8t_probe2.rs"]
mod p2;
use common::*;
use oxiscript_abi::codes;
use oxiscript_check::ty::{TyKind, ERROR};
use oxiscript_check::*;
use oxiscript_syntax::*;

fn scale(full: usize) -> usize {
    if !cfg!(debug_assertions) || std::env::var_os("OXI_FULL_FUZZ").is_some() { full } else { (full / 20).max(30) }
}

pub fn pool() -> Vec<String> {
    let mut v: Vec<String> = p1::C.iter().chain(p2::C.iter()).map(|(_, s)| (*s).to_string()).collect();
    let dir = format!("{}/../oxiscript-syntax/tests/corpus/valid", env!("CARGO_MANIFEST_DIR"));
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "oxs") {
                v.push(std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    v.extend(valid_programs());
    v
}

/// Programs the checker should accept (used as mutation seeds and invariant subjects).
pub fn valid_programs() -> Vec<String> {
    [
"type Shape = { kind: \"c\"; r: f64 } | { kind: \"s\"; w: f64; h: f64 };\nfunction area(s: Shape): f64 { switch (s.kind) { case \"c\": return 3.14 * s.r * s.r; case \"s\": return s.w * s.h; } }\nfunction f(xs: Shape[]): f64 { let t = 0.0; for (const x of xs) { t += area(x); } return t; }",
"struct Node { value: i32; next: Node | null; }\nfunction sum(n: Node | null): i32 { let t = 0; let c = n; while (c !== null) { t += c.value; c = c.next; } return t; }",
"function id<T>(x: T): T { return x; }\nstruct Box<T> { v: T; get(): T { return this.v; } }\nfunction f(): i32 { const b: Box<i32> = { v: 3 }; return id(b).get() + id<i32>(2); }",
"function g(): Result<i32, string> { return Ok(1); }\nfunction f(): Result<i32, string> { const a = g()?; const b = g()?; return Ok(a + b); }",
"enum C: u8 { R, G = 5, B }\nfunction f(c: C): string { return switch (c) { case C.R: \"r\", case C.G: \"g\", case C.B: \"b\", }; }",
"class Ctr { n: i32 = 0; constructor() {} inc(): i32 { this.n += 1; return this.n; } }\nfunction f(): i32 { const c = new Ctr(); const g = () => c.inc(); g(); return g(); }",
"function f(a: i32 | null, b: i32 | null): i32 { if (a !== null && b !== null) { return a + b; } else if (a !== null) { return a; } return b ?? 0; }",
"interface Sh { area(): f64; }\nstruct Sq { s: f64; area(): f64 { return this.s * this.s; } }\nfunction tot<T extends Sh>(xs: T[]): f64 { let t = 0.0; for (const x of xs) { t += x.area(); } return t; }\nfunction f(): f64 { const a: Sq[] = [{ s: 1.0 }]; return tot(a); }",
"import { Log } from \"host:log\";\nexport function main(log: Log): void { log.write(1, \"hi\"); }",
"function f(n: u8): u16 { let x = n as u16; x = x * 2; return x << 1; }\nfunction g(a: f64): i32 { return a as i32; }",
"function map<T, U>(a: T[], f: (x: T) => U): U[] { const r: U[] = []; for (const x of a) { r.push(f(x)); } return r; }\nfunction f(a: i32[]): string[] { return map(a, (x) => `${x}`); }",
"struct P { x: i32; y: i32; mutating mv(dx: i32): void { this.x += dx; } }\nfunction f(): i32 { let p: P = { x: 1, y: 2 }; p.mv(3); const { x, y } = p; return x + y; }",
"law comm(a: i32, b: i32) when a > -1000 && b > -1000 { a + b === b + a }",
"function f(s: string | null): i32 { const t = s; if (t === null) { return 0; } const g = (): string => t; return 1; }",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn on_small_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    std::thread::Builder::new().stack_size(1 << 20).spawn(f).unwrap().join().expect("panic or stack overflow")
}

fn fingerprint(c: &Checked) -> String {
    let mut s = String::new();
    for d in &c.diagnostics {
        s.push_str(&format!("{} {:?} {} {:?}\n", d.code, d.span, d.message, d.fixes));
    }
    s.push_str(&format!("tys {} instances {:?}\n", c.tys.len(), c.instances));
    let mut ef: Vec<_> = c.effects.iter().collect();
    ef.sort_by_key(|(k, _)| **k);
    s.push_str(&format!("{ef:?}\n"));
    for m in &c.modules {
        s.push_str(&format!("{:?}\n{:?}\n", m.expr_ty, m.locals));
        let mut r: Vec<_> = m.res.iter().collect();
        r.sort_by_key(|(k, _)| **k);
        s.push_str(&format!("{r:?}\n"));
        let mut k: Vec<_> = m.coercions.iter().collect();
        k.sort_by_key(|(k, _)| **k);
        s.push_str(&format!("{k:?}\n"));
        let mut k: Vec<_> = m.calls.iter().collect();
        k.sort_by_key(|(k, _)| **k);
        s.push_str(&format!("{k:?}\n"));
        let mut k: Vec<_> = m.closures.iter().collect();
        k.sort_by_key(|(k, _)| **k);
        s.push_str(&format!("{k:?}\n"));
        let mut k: Vec<_> = m.switches.iter().collect();
        k.sort_by_key(|(k, _)| **k);
        s.push_str(&format!("{k:?}\n"));
    }
    s
}

#[test]
fn determinism_same_input_same_output() {
    let pool = pool();
    let bad = on_small_stack(move || {
        let h = hosts();
        let mut bad = Vec::new();
        for src in &pool {
            let a = fingerprint(&check_source(src, &h));
            for _ in 0..4 {
                let b = fingerprint(&check_source(src, &h));
                if a != b {
                    bad.push(src.clone());
                    break;
                }
            }
        }
        bad
    });
    assert!(bad.is_empty(), "nondeterministic on {} programs, first:\n{}", bad.len(), bad.first().unwrap());
}

struct Inv<'c> {
    c: &'c Checked,
    bad: Vec<String>,
}

impl<'a> Visitor<'a> for Inv<'_> {
    fn visit_expr(&mut self, e: &'a Expr) {
        let m = &self.c.modules[0];
        match m.expr_ty.get(e.id.0 as usize) {
            None => self.bad.push(format!("no type for expr at {}", e.span.lo)),
            Some(&t) => {
                if t == ERROR || self.c.tys.has_error(t) {
                    self.bad.push(format!("ERROR type at {}", e.span.lo));
                }
                self.check_ty(t, e.span.lo);
            }
        }
        if let Some(r) = m.res.get(&e.id) {
            self.check_res(*r, e.span.lo);
        } else if matches!(e.kind, ExprKind::Ident(_)) {
            self.bad.push(format!("ident without res at {}", e.span.lo));
        }
        if let Some(ci) = m.calls.get(&e.id) {
            if let Some(i) = ci.instance {
                match self.c.instances.get(i as usize) {
                    None => self.bad.push(format!("instance index out of range at {}", e.span.lo)),
                    Some(inst) => {
                        if inst.args != ci.type_args {
                            self.bad.push(format!("instance args != call type_args at {}", e.span.lo));
                        }
                    }
                }
            }
        }
        if let Some(co) = m.coercions.get(&e.id) {
            if co.from == co.to {
                self.bad.push(format!("identity coercion at {}", e.span.lo));
            }
        }
        if let Some(sw) = m.switches.get(&e.id) {
            if sw.kind == SwitchKind::Union && sw.union.is_none() {
                self.bad.push(format!("union switch without union at {}", e.span.lo));
            }
        }
        walk_expr(self, e);
    }
}

impl Inv<'_> {
    fn check_ty(&mut self, t: Ty, at: u32) {
        match self.c.tys.kind(t).clone() {
            TyKind::Nullable(i) => {
                if matches!(self.c.tys.kind(i), TyKind::Null | TyKind::Nullable(_) | TyKind::Never | TyKind::Void | TyKind::Error) {
                    self.bad.push(format!("malformed Nullable type at {at}"));
                }
                self.check_ty(i, at);
            }
            TyKind::Array(i) => {
                if matches!(self.c.tys.kind(i), TyKind::Void | TyKind::Never | TyKind::Null) {
                    self.bad.push(format!("array of void/never/null at {at}"));
                }
                self.check_ty(i, at);
            }
            TyKind::Fn(ps, r) => {
                for p in ps.iter() {
                    self.check_ty(*p, at);
                }
                self.check_ty(r, at);
            }
            TyKind::Adt(d, args) => {
                if d.0 as usize >= self.c.defs.len() {
                    self.bad.push(format!("Adt def out of range at {at}"));
                }
                for a in args.iter() {
                    if matches!(self.c.tys.kind(*a), TyKind::Never) {
                        self.bad.push(format!("never type argument at {at}"));
                    }
                    self.check_ty(*a, at);
                }
            }
            TyKind::Variants(d, args, mask) => {
                match self.c.unions.get(&d) {
                    Some(u) => {
                        if mask == 0 || u.variants.len() < 64 && mask >> u.variants.len() != 0 {
                            self.bad.push(format!("bad variant mask at {at}"));
                        }
                    }
                    None => self.bad.push(format!("Variants of a non-union at {at}")),
                }
                for a in args.iter() {
                    self.check_ty(*a, at);
                }
            }
            _ => {}
        }
    }
    fn check_res(&mut self, r: Res, at: u32) {
        let c = self.c;
        let nd = c.defs.len() as u32;
        let ok = match r {
            Res::Local(l) | Res::This(l) => (l.0 as usize) < c.modules[0].locals.len(),
            Res::Def(d) | Res::Method(d) => d.0 < nd,
            Res::Field { owner, index } | Res::StaticField { owner, index } => {
                c.records.get(&owner).is_some_and(|r| (index as usize) < r.fields.len())
                    || c.unions.get(&owner).is_some()
                    || owner.0 < nd && c.records.get(&owner).is_none()
            }
            Res::IfaceMethod { iface, index } => c.ifaces.get(&iface).is_some_and(|i| (index as usize) < i.methods.len()),
            Res::HostFn { host, index } => c.hosts.get(&host).is_some_and(|h| (index as usize) < h.fns.len()),
            Res::EnumMember { def, index } => c.enums.get(&def).is_some_and(|e| (index as usize) < e.members.len()),
            Res::Discriminant { union } => c.unions.contains_key(&union),
            Res::Builtin(_) => true,
        };
        if !ok {
            self.bad.push(format!("dangling res {r:?} at {at}"));
        }
    }
}

/// Invariants of an accepted program; returns violations.
/// Violations that are already reported as findings F08 (void/never through generics) are filtered.
fn known(b: &str) -> bool {
    b.starts_with("never type argument") || b.starts_with("array of void")
}

pub fn invariants(src: &str, c: &Checked) -> Vec<String> {
    let mut inv = Inv { c, bad: Vec::new() };
    let parsed = parse(src, FileId(0));
    inv.visit_module(&parsed.module);
    inv.bad.into_iter().filter(|b| !known(b)).collect()
}

pub fn diag_sanity(src: &str, c: &Checked) -> Vec<String> {
    let reg = codes::seed();
    let mut bad = Vec::new();
    for d in &c.diagnostics {
        if d.code == "OXS-E0990" {
            bad.push(format!("internal poison diagnostic E0990: {}", d.message));
        }
        if d.span.lo > d.span.hi || d.span.hi as usize > src.len() {
            bad.push(format!("span out of range {:?} (len {}) {}", d.span, src.len(), d.code));
        } else if !src.is_char_boundary(d.span.lo as usize) || !src.is_char_boundary(d.span.hi as usize) {
            bad.push(format!("span not on char boundary {:?} {}", d.span, d.code));
        }
        match reg.iter().find(|r| r.code == d.code) {
            None => bad.push(format!("code {} not in registry", d.code)),
            Some(r) => {
                if !matches!(r.phase.as_str(), "check" | "parse" | "link" | "lex") {
                    bad.push(format!("code {} has phase {}", d.code, r.phase));
                }
            }
        }
        for f in &d.fixes {
            for e in &f.edits {
                if e.span.lo > e.span.hi || e.span.hi as usize > src.len() {
                    bad.push(format!("fix edit span out of range {:?} in {}", e.span, d.code));
                }
            }
        }
    }
    bad
}

#[test]
fn invariants_over_pool() {
    let pool = pool();
    let (n_ok, bad) = on_small_stack(move || {
        let h = hosts();
        let mut bad = Vec::new();
        let mut n_ok = 0;
        for src in &pool {
            let c = check_source(src, &h);
            for b in diag_sanity(src, &c) {
                bad.push(format!("{b}\n--- {src}"));
            }
            if c.ok() {
                n_ok += 1;
                for b in invariants(src, &c) {
                    bad.push(format!("{b}\n--- {src}"));
                }
            }
        }
        (n_ok, bad)
    });
    eprintln!("accepted {n_ok}");
    assert!(bad.is_empty(), "{} violations:\n{}", bad.len(), bad.iter().take(10).cloned().collect::<Vec<_>>().join("\n\n"));
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const SNIPS: &[&str] = &[
    "i32", "i64", "u8", "f64", "string", "bool", "null", " | null", "[]", "!== null", "=== null", "?", "?.", "??", "as i64", "as u8",
    "this.", "return ", "const ", "let ", "=> ", "<T>", "<i32>", "{ kind: \"c\", r: 1.0 }", "[1, 2]", "(", ")", "{", "}", ";", "mutating ", "private ",
    "readonly ", "static ", "switch (", "case ", "default: ", "while (true) ", "break;", "continue;", "~", "-", "<<", "+%", "/ 0", "never", "void", "Result<i32, string>", "Ok(1)", "Err(\"e\")",
];

/// Token-ish mutations: delete/duplicate/swap spans of lines or words, splice snippets, replace type names.
fn mutate(rng: &mut Rng, src: &str) -> String {
    let words: Vec<&str> = src.split_inclusive([' ', '\n', ';', '(', ')', '{', '}']).collect();
    if words.is_empty() {
        return String::new();
    }
    let mut w: Vec<String> = words.iter().map(|s| s.to_string()).collect();
    for _ in 0..1 + rng.below(3) {
        let i = rng.below(w.len());
        match rng.below(6) {
            0 => {
                w.remove(i);
                if w.is_empty() {
                    break;
                }
            }
            1 => {
                let x = w[i].clone();
                w.insert(i, x);
            }
            2 => {
                let j = rng.below(w.len());
                w.swap(i, j);
            }
            3 => w.insert(i, SNIPS[rng.below(SNIPS.len())].to_string()),
            4 => w[i] = SNIPS[rng.below(SNIPS.len())].to_string(),
            _ => {
                let j = rng.below(w.len());
                let x = w[j].clone();
                w[i] = x;
            }
        }
    }
    w.concat()
}

#[test]
fn mutation_fuzz_invariants() {
    let n = scale(60_000);
    let seeds = {
        let mut s = valid_programs();
        s.extend(pool());
        s
    };
    let (total, acc, bad) = on_small_stack(move || {
        let h = hosts();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut acc, mut bad) = (0usize, Vec::new());
        for _ in 0..n {
            let base = &seeds[rng.below(seeds.len())];
            let src = mutate(&mut rng, base);
            let c = check_source(&src, &h);
            for b in diag_sanity(&src, &c) {
                bad.push(format!("{b}\n--- {src}"));
            }
            if c.ok() {
                acc += 1;
                for b in invariants(&src, &c) {
                    bad.push(format!("{b}\n--- {src}"));
                }
            }
            if bad.len() > 20 {
                break;
            }
        }
        (n, acc, bad)
    });
    eprintln!("mutations {total}, accepted {acc}");
    assert!(bad.is_empty(), "{} violations:\n{}", bad.len(), bad.iter().take(8).cloned().collect::<Vec<_>>().join("\n\n"));
}

/// Replace identifiers/literals with other plausible ones, keeping the program mostly well-formed
/// (much higher acceptance rate than token soup, so the invariants walker sees many more tables).
fn swap_mutate(rng: &mut Rng, src: &str) -> String {
    const TY: &[&str] = &["i32", "i64", "u8", "u16", "f64", "f32", "string", "bool"];
    const LIT: &[&str] = &["0", "1", "255", "300", "1.5", "\"s\"", "true", "null"];
    const OP: &[&str] = &["+", "-", "*", "===", "!==", "<", "&&", "||", "??", "<<"];
    let mut out = String::new();
    let mut hits = 0;
    let target = 1 + rng.below(3);
    for tok in src.split_inclusive([' ', '\n']) {
        let t = tok.trim();
        let rep = if TY.contains(&t) && rng.below(12) == 0 {
            Some(TY[rng.below(TY.len())])
        } else if t.parse::<f64>().is_ok() && rng.below(10) == 0 {
            Some(LIT[rng.below(LIT.len())])
        } else if OP.contains(&t) && rng.below(10) == 0 {
            Some(OP[rng.below(OP.len())])
        } else if (t == "const" || t == "let") && rng.below(15) == 0 {
            Some(if t == "const" { "let" } else { "const" })
        } else {
            None
        };
        match rep {
            Some(r) if hits < target + 2 => {
                hits += 1;
                out.push_str(r);
                out.push(' ');
            }
            _ => out.push_str(tok),
        }
    }
    out
}

#[test]
fn swap_fuzz_invariants() {
    let n = scale(150_000);
    let seeds = valid_programs();
    let (acc, bad) = on_small_stack(move || {
        let h = hosts();
        let mut seen = std::collections::HashSet::new();
        let mut rng = Rng(0xDEAD_BEEF_1234_5678);
        let (mut acc, mut bad) = (0usize, Vec::new());
        for _ in 0..n {
            let base = &seeds[rng.below(seeds.len())];
            let src = swap_mutate(&mut rng, base);
            if !seen.insert(src.clone()) {
                continue;
            }
            let c = check_source(&src, &h);
            for b in diag_sanity(&src, &c) {
                bad.push(format!("{b}\n--- {src}"));
            }
            if c.ok() {
                acc += 1;
                for b in invariants(&src, &c) {
                    bad.push(format!("{b}\n--- {src}"));
                }
            }
            if bad.len() > 20 {
                break;
            }
        }
        eprintln!("distinct swap mutants {}", seen.len());
        (acc, bad)
    });
    eprintln!("swap mutations {n}, accepted distinct {acc}");
    assert!(bad.is_empty(), "{} violations:\n{}", bad.len(), bad.iter().take(8).cloned().collect::<Vec<_>>().join("\n\n"));
}
