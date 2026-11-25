#![allow(clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.L: seeded differential generator. Eval-while-generate: an AST of a small subset is both
//! emitted as OxiScript and executed by a reference model in Rust. Oracles: log equals the model's, plain
//! and metamorphically-wrapped variants agree, zero live objects after deinit, no trap.
mod common;
use common::*;
use std::collections::HashMap;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 { self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); self.0 >> 33 }
    fn below(&mut self, n: u64) -> u64 { self.next() % n }
    fn chance(&mut self, pct: u64) -> bool { self.below(100) < pct }
}

#[derive(Clone, Debug)]
enum I {
    Lit(i64), Var(String), Add(Box<I>, Box<I>), Sub(Box<I>, Box<I>), MulMod(Box<I>, i64),
    ArrLen(String), ArrGet(String, Box<I>), StrLen(String), StA(String), ClN(String), ClStA(String),
    Call(String, Box<I>), Wrap(u8, Box<I>),
}
#[derive(Clone, Debug)]
enum S { Lit(String), Var(String), Cat(Box<S>, Box<S>), OfInt(Box<I>), Wrap(u8, Box<S>), StS(String), ClItem(String, Box<I>) }
#[derive(Clone, Debug)]
enum St {
    DeclI(String, I), SetI(String, I), AddI(String, I),
    DeclS(String, S), SetS(String, S), AppS(String, S),
    DeclA(String, Vec<i64>), Push(String, I), SetElem(String, I, I), PopIf(String),
    DeclSt(String, I, S), CopySt(String, String), StSetA(String, I), StAddA(String, I), StPush(String, I), StSetS(String, S), StBump(String, I),
    DeclCl(String, I), AliasCl(String, String), ClAdd(String, I), ClPushItem(String, S), ClStAdd(String, I), SnapSt(String, String), ClSetSt(String, String),
    DeclF(String, String), // closure over int var
    CallF(String, I),
    Obs(S),
    If(I, i64, Vec<St>, Vec<St>),
    For(String, i64, Vec<St>),
    BreakIf(I, i64, i64), ContIf(I, i64, i64), RetIf(I, i64, i64),
}

#[derive(Clone, Debug)]
struct Stv { a: i64, xs: usize, s: String }
#[derive(Default, Clone)]
struct Env {
    ints: HashMap<String, i64>, strs: HashMap<String, String>, arrs: Vec<Vec<i64>>, arrv: HashMap<String, usize>,
    sts: HashMap<String, Stv>, cls: HashMap<String, usize>, clh: Vec<(i64, Vec<String>, Stv)>, fs: HashMap<String, String>,
    out: Vec<String>,
}
#[derive(PartialEq)]
enum Flow { Next, Break, Cont, Ret }

fn wrapi(v: u8, e: &str) -> String {
    match v % 5 { 0 => format!("idt<i32>({e})"), 1 => format!("(() => {e})()"), 2 => format!("[{e}][0]"), 3 => format!("mk({e}).v"), _ => format!("new Wr({e}).v") }
}
fn wraps(v: u8, e: &str) -> String {
    match v % 4 { 0 => format!("idt<string>({e})"), 1 => format!("(() => {e})()"), 2 => format!("[{e}][0]"), _ => format!("new WrS({e}).v") }
}

fn ei(e: &I, meta: bool) -> String {
    let m = |x: &I| ei(x, meta);
    match e {
        I::Lit(n) => if *n < 0 { format!("({n})") } else { n.to_string() },
        I::Var(v) => v.clone(),
        I::Add(a, b) => format!("({} + {})", m(a), m(b)),
        I::Sub(a, b) => format!("({} - {})", m(a), m(b)),
        I::MulMod(a, k) => format!("(({} * {k}) % 997)", m(a)),
        I::ArrLen(a) => format!("{a}.length"),
        I::ArrGet(a, i) => format!("{a}[(({} % {a}.length) + {a}.length) % {a}.length]", m(i)),
        I::StrLen(s) => format!("{s}.byteLength"),
        I::StA(s) => format!("{s}.a"),
        I::ClN(c) => format!("{c}.n"),
        I::ClStA(c) => format!("{c}.st.a"),
        I::Call(f, a) => format!("{f}({})", m(a)),
        I::Wrap(v, x) => if meta { wrapi(*v, &m(x)) } else { m(x) },
    }
}
fn es(e: &S, meta: bool) -> String {
    let m = |x: &S| es(x, meta);
    match e {
        S::Lit(s) => format!("\"{s}\""),
        S::Var(v) => v.clone(),
        S::Cat(a, b) => format!("({} + {})", m(a), m(b)),
        S::OfInt(i) => format!("`${{{}}}`", ei(i, meta)),
        S::Wrap(v, x) => if meta { wraps(*v, &m(x)) } else { m(x) },
        S::StS(s) => format!("{s}.s"),
        S::ClItem(c, i) => format!("{c}.items[(({} % {c}.items.length) + {c}.items.length) % {c}.items.length]", ei(i, meta)),
    }
}

fn mi(e: &I, env: &mut Env) -> i64 {
    match e {
        I::Lit(n) => *n,
        I::Var(v) => env.ints[v],
        I::Add(a, b) => { let x = mi(a, env); let y = mi(b, env); x + y }
        I::Sub(a, b) => { let x = mi(a, env); let y = mi(b, env); x - y }
        I::MulMod(a, k) => (mi(a, env) * k) % 997,
        I::ArrLen(a) => env.arrs[env.arrv[a]].len() as i64,
        I::ArrGet(a, i) => { let a_idx = env.arrv[a]; let n = env.arrs[a_idx].len() as i64; let ix = mi(i, env); let j = ((ix % n) + n) % n; env.arrs[a_idx][j as usize] }
        I::StrLen(s) => env.strs[s].len() as i64,
        I::StA(s) => env.sts[s].a,
        I::ClN(c) => env.clh[env.cls[c]].0,
        I::ClStA(c) => env.clh[env.cls[c]].2.a,
        I::Call(f, a) => { let d = mi(a, env); let v = env.fs[f].clone(); let x = env.ints[&v] + d; env.ints.insert(v, x); x }
        I::Wrap(_, x) => mi(x, env),
    }
}
fn ms(e: &S, env: &mut Env) -> String {
    match e {
        S::Lit(s) => s.clone(),
        S::Var(v) => env.strs[v].clone(),
        S::Cat(a, b) => { let x = ms(a, env); let y = ms(b, env); x + &y }
        S::OfInt(i) => mi(i, env).to_string(),
        S::Wrap(_, x) => ms(x, env),
        S::StS(s) => env.sts[s].s.clone(),
        S::ClItem(c, i) => { let ix = mi(i, env); let h = &env.clh[env.cls[c]].1; let n = h.len() as i64; h[(((ix % n) + n) % n) as usize].clone() }
    }
}

fn indent(n: usize) -> String { "  ".repeat(n) }
fn emit(st: &[St], meta: bool, d: usize, out: &mut String) {
    for s in st {
        let p = indent(d);
        let line = match s {
            St::DeclI(n, e) => format!("let {n} = {};", ei(e, meta)),
            St::SetI(n, e) => format!("{n} = {};", ei(e, meta)),
            St::AddI(n, e) => format!("{n} += {};", ei(e, meta)),
            St::DeclS(n, e) => format!("let {n} = {};", es(e, meta)),
            St::SetS(n, e) => format!("{n} = {};", es(e, meta)),
            St::AppS(n, e) => format!("{n} += {};", es(e, meta)),
            St::DeclA(n, v) => format!("const {n}: i32[] = [{}];", v.iter().map(|x| if *x < 0 { format!("({x})") } else { x.to_string() }).collect::<Vec<_>>().join(", ")),
            St::Push(n, e) => format!("{n}.push({});", ei(e, meta)),
            St::SetElem(n, i, e) => format!("{n}[(({} % {n}.length) + {n}.length) % {n}.length] = {};", ei(i, meta), ei(e, meta)),
            St::PopIf(n) => format!("if ({n}.length > 1) {{ {n}.pop(); }}"),
            St::DeclSt(n, a, s) => format!("let {n}: Sx = {{ a: {}, xs: [1, 2], s: {} }};", ei(a, meta), es(s, meta)),
            St::CopySt(n, src) => format!("let {n}: Sx = {src};"),
            St::StSetA(n, e) => format!("{n}.a = {};", ei(e, meta)),
            St::StAddA(n, e) => format!("{n}.a += {};", ei(e, meta)),
            St::StPush(n, e) => format!("{n}.xs.push({});", ei(e, meta)),
            St::StSetS(n, e) => format!("{n}.s = {};", es(e, meta)),
            St::StBump(n, e) => format!("{n}.bump({});", ei(e, meta)),
            St::DeclCl(n, e) => format!("const {n} = new Cx({});", ei(e, meta)),
            St::AliasCl(n, src) => format!("const {n} = {src};"),
            St::ClAdd(n, e) => format!("{n}.n += {};", ei(e, meta)),
            St::ClPushItem(n, e) => format!("{n}.items.push({});", es(e, meta)),
            St::ClStAdd(n, e) => format!("{n}.st.a += {};", ei(e, meta)),
            St::SnapSt(n, c) => format!("let {n}: Sx = {c}.st;"),
            St::ClSetSt(c, s) => format!("{c}.st = {s};"),
            St::DeclF(f, v) => format!("const {f} = (d: i32): i32 => {{ {v} += d; return {v}; }};"),
            St::CallF(f, e) => format!("{f}({});", ei(e, meta)),
            St::Obs(e) => format!("log.write(2, {});", es(e, meta)),
            St::If(c, k, t, e) => { let mut o = format!("if (({} % 7) === {k}) {{\n", ei(c, meta)); emit(t, meta, d + 1, &mut o); o.push_str(&format!("{p}}} else {{\n")); emit(e, meta, d + 1, &mut o); o.push_str(&format!("{p}}}")); o }
            St::For(v, n, b) => { let mut o = format!("for (let {v} = 0; {v} < {n}; {v}++) {{\n"); emit(b, meta, d + 1, &mut o); o.push_str(&format!("{p}}}")); o }
            St::BreakIf(e, m, r) => format!("if ((({} % {m}) + {m}) % {m} === {r}) {{ break; }}", ei(e, meta)),
            St::ContIf(e, m, r) => format!("if ((({} % {m}) + {m}) % {m} === {r}) {{ continue; }}", ei(e, meta)),
            St::RetIf(e, m, r) => format!("if ((({} % {m}) + {m}) % {m} === {r}) {{ return; }}", ei(e, meta)),
        };
        out.push_str(&p); out.push_str(&line); out.push('\n');
    }
}

fn exec(st: &[St], env: &mut Env) -> Flow {
    for s in st {
        match s {
            St::DeclI(n, e) | St::SetI(n, e) => { let v = mi(e, env); env.ints.insert(n.clone(), v); }
            St::AddI(n, e) => { let old = env.ints[n]; let v = mi(e, env); env.ints.insert(n.clone(), old + v); }
            St::DeclS(n, e) | St::SetS(n, e) => { let v = ms(e, env); env.strs.insert(n.clone(), v); }
            St::AppS(n, e) => { let old = env.strs[n].clone(); let v = ms(e, env); env.strs.insert(n.clone(), old + &v); }
            St::DeclA(n, v) => { env.arrs.push(v.clone()); env.arrv.insert(n.clone(), env.arrs.len() - 1); }
            St::Push(n, e) => { let v = mi(e, env); let i = env.arrv[n]; env.arrs[i].push(v); }
            St::SetElem(n, i, e) => { let ix = mi(i, env); let v = mi(e, env); let a = env.arrv[n]; let l = env.arrs[a].len() as i64; env.arrs[a][(((ix % l) + l) % l) as usize] = v; }
            St::PopIf(n) => { let i = env.arrv[n]; if env.arrs[i].len() > 1 { env.arrs[i].pop(); } }
            St::DeclSt(n, a, s) => { let a = mi(a, env); let s = ms(s, env); env.arrs.push(vec![1, 2]); let xs = env.arrs.len() - 1; env.sts.insert(n.clone(), Stv { a, xs, s }); }
            St::CopySt(n, src) => { let v = env.sts[src].clone(); env.sts.insert(n.clone(), v); }
            St::StSetA(n, e) => { let v = mi(e, env); env.sts.get_mut(n).unwrap().a = v; }
            St::StAddA(n, e) => { let old = env.sts[n].a; let v = mi(e, env); env.sts.get_mut(n).unwrap().a = old + v; }
            St::StPush(n, e) => { let v = mi(e, env); let x = env.sts[n].xs; env.arrs[x].push(v); }
            St::StSetS(n, e) => { let v = ms(e, env); env.sts.get_mut(n).unwrap().s = v; }
            St::StBump(n, e) => { let old = env.sts[n].a; let v = mi(e, env); env.sts.get_mut(n).unwrap().a = old + v; }
            St::DeclCl(n, e) => { let v = mi(e, env); env.arrs.push(vec![1, 2]); let xs = env.arrs.len() - 1; env.clh.push((v, vec!["i".to_string()], Stv { a: 100, xs, s: "cs".into() })); env.cls.insert(n.clone(), env.clh.len() - 1); }
            St::AliasCl(n, src) => { let h = env.cls[src]; env.cls.insert(n.clone(), h); }
            St::ClAdd(n, e) => { let h = env.cls[n]; let old = env.clh[h].0; let v = mi(e, env); env.clh[h].0 = old + v; }
            St::ClPushItem(n, e) => { let h = env.cls[n]; let v = ms(e, env); env.clh[h].1.push(v); }
            St::ClStAdd(n, e) => { let h = env.cls[n]; let old = env.clh[h].2.a; let v = mi(e, env); env.clh[h].2.a = old + v; }
            St::SnapSt(n, c) => { let v = env.clh[env.cls[c]].2.clone(); env.sts.insert(n.clone(), v); }
            St::ClSetSt(c, s) => { let v = env.sts[s].clone(); let h = env.cls[c]; env.clh[h].2 = v; }
            St::DeclF(f, v) => { env.fs.insert(f.clone(), v.clone()); }
            St::CallF(f, e) => { let d = mi(e, env); let v = env.fs[f].clone(); let x = env.ints[&v] + d; env.ints.insert(v, x); }
            St::Obs(e) => { let v = ms(e, env); env.out.push(v); }
            St::If(c, k, t, e) => { let v = mi(c, env); let f = if v % 7 == *k { exec(t, env) } else { exec(e, env) }; if f != Flow::Next { return f; } }
            St::For(v, n, b) => { let _ = v; for _ in 0..*n { match exec(b, env) { Flow::Break => break, Flow::Ret => return Flow::Ret, _ => {} } } }
            St::BreakIf(e, m, r) => { let v = mi(e, env); if ((v % m) + m) % m == *r { return Flow::Break; } }
            St::ContIf(e, m, r) => { let v = mi(e, env); if ((v % m) + m) % m == *r { return Flow::Cont; } }
            St::RetIf(e, m, r) => { let v = mi(e, env); if ((v % m) + m) % m == *r { return Flow::Ret; } }
        }
    }
    Flow::Next
}

/// scope tracking while generating
#[derive(Default, Clone)]
struct Scope { ro: Vec<String>, ints: Vec<String>, strs: Vec<String>, arrs: Vec<String>, sts: Vec<String>, cls: Vec<String>, fs: Vec<String> }

struct Gen { r: Rng, uid: u32, wrap_pct: u64 }
impl Gen {
    fn name(&mut self, p: &str) -> String { self.uid += 1; format!("{p}{}", self.uid) }
    fn pick(&mut self, v: &[String]) -> String { v[self.r.below(v.len() as u64) as usize].clone() }
    fn wrap_i(&mut self, e: I) -> I { if self.r.chance(self.wrap_pct) { I::Wrap(self.r.below(5) as u8, Box::new(e)) } else { e } }
    fn wrap_s(&mut self, e: S) -> S { if self.r.chance(self.wrap_pct) { S::Wrap(self.r.below(4) as u8, Box::new(e)) } else { e } }
    fn iexp(&mut self, sc: &Scope, d: u32) -> I {
        let k = if d == 0 { self.r.below(4) } else { self.r.below(14) };
        let e = match k {
            0 => I::Lit(self.r.below(50) as i64 - 10),
            1 | 2 if !sc.ints.is_empty() || !sc.ro.is_empty() => { let all: Vec<String> = sc.ints.iter().chain(sc.ro.iter()).cloned().collect(); I::Var(self.pick(&all)) }
            3 if !sc.strs.is_empty() => I::StrLen(self.pick(&sc.strs)),
            4 | 5 => { let a = self.iexp(sc, d - 1); let b = self.iexp(sc, d - 1); I::Add(Box::new(a), Box::new(b)) }
            6 => { let a = self.iexp(sc, d - 1); let b = self.iexp(sc, d - 1); I::Sub(Box::new(a), Box::new(b)) }
            7 => { let a = self.iexp(sc, d - 1); I::MulMod(Box::new(a), self.r.below(9) as i64 + 1) }
            8 if !sc.arrs.is_empty() => I::ArrLen(self.pick(&sc.arrs)),
            9 if !sc.arrs.is_empty() => { let a = self.pick(&sc.arrs); let i = self.iexp(sc, d - 1); I::ArrGet(a, Box::new(i)) }
            10 if !sc.sts.is_empty() => I::StA(self.pick(&sc.sts)),
            11 if !sc.cls.is_empty() => { let c = self.pick(&sc.cls); if self.r.chance(50) { I::ClN(c) } else { I::ClStA(c) } }
            12 if !sc.fs.is_empty() => { let f = self.pick(&sc.fs); let a = self.iexp(sc, d - 1); I::Call(f, Box::new(a)) }
            _ => I::Lit(self.r.below(9) as i64),
        };
        self.wrap_i(e)
    }
    fn sexp(&mut self, sc: &Scope, d: u32) -> S {
        let k = if d == 0 { self.r.below(3) } else { self.r.below(8) };
        let e = match k {
            0 => S::Lit(["a", "bc", "é", "x1", "😀"][self.r.below(5) as usize].to_string()),
            1 if !sc.strs.is_empty() => S::Var(self.pick(&sc.strs)),
            2 | 3 => { let i = self.iexp(sc, d.saturating_sub(1)); S::OfInt(Box::new(i)) }
            4 | 5 => { let a = self.sexp(sc, d - 1); let b = self.sexp(sc, d - 1); S::Cat(Box::new(a), Box::new(b)) }
            6 if !sc.sts.is_empty() => S::StS(self.pick(&sc.sts)),
            7 if !sc.cls.is_empty() => { let c = self.pick(&sc.cls); let i = self.iexp(sc, 1); S::ClItem(c, Box::new(i)) }
            _ => S::Lit("q".into()),
        };
        self.wrap_s(e)
    }
    fn block(&mut self, sc: &mut Scope, n: usize, depth: u32, in_loop: bool) -> Vec<St> {
        let mut v = Vec::new();
        for _ in 0..n { self.stmt(sc, depth, in_loop, &mut v); }
        v
    }
    fn stmt(&mut self, sc: &mut Scope, depth: u32, in_loop: bool, out: &mut Vec<St>) {
        let k = self.r.below(34);
        match k {
            0 | 1 => { let n = self.name("i"); let e = self.iexp(sc, 2); out.push(St::DeclI(n.clone(), e)); sc.ints.push(n); }
            2 | 3 if !sc.ints.is_empty() => { let n = self.pick(&sc.ints); let e = self.iexp(sc, 2); out.push(St::AddI(n.clone(), e)); out.push(St::SetI(n.clone(), I::MulMod(Box::new(I::Var(n)), 1))); }
            4 | 5 => { let n = self.name("s"); let e = self.sexp(sc, 2); out.push(St::DeclS(n.clone(), e)); sc.strs.push(n); }
            6 if !sc.strs.is_empty() => { let n = self.pick(&sc.strs); let e = self.sexp(sc, 2); out.push(St::AppS(n, e)); }
            7 if !sc.strs.is_empty() => { let n = self.pick(&sc.strs); let e = self.sexp(sc, 1); out.push(St::SetS(n, e)); }
            8 => { let n = self.name("a"); let l = self.r.below(3) as usize + 1; let v = (0..l).map(|_| self.r.below(20) as i64).collect(); out.push(St::DeclA(n.clone(), v)); sc.arrs.push(n); }
            9 | 10 if !sc.arrs.is_empty() => { let n = self.pick(&sc.arrs); let e = self.iexp(sc, 2); out.push(St::Push(n, e)); }
            11 if !sc.arrs.is_empty() => { let n = self.pick(&sc.arrs); let i = self.iexp(sc, 1); let e = self.iexp(sc, 1); out.push(St::SetElem(n, i, e)); }
            12 if !sc.arrs.is_empty() => { let n = self.pick(&sc.arrs); out.push(St::PopIf(n)); }
            13 => { let n = self.name("t"); let a = self.iexp(sc, 1); let s = self.sexp(sc, 1); out.push(St::DeclSt(n.clone(), a, s)); sc.sts.push(n); }
            14 if !sc.sts.is_empty() => { let src = self.pick(&sc.sts); let n = self.name("t"); out.push(St::CopySt(n.clone(), src)); sc.sts.push(n); }
            15 if !sc.sts.is_empty() => { let n = self.pick(&sc.sts); let e = self.iexp(sc, 1); out.push(if self.r.chance(50) { St::StSetA(n, e) } else { St::StAddA(n, e) }); }
            16 if !sc.sts.is_empty() => { let n = self.pick(&sc.sts); let e = self.iexp(sc, 1); out.push(St::StPush(n, e)); }
            17 if !sc.sts.is_empty() => { let n = self.pick(&sc.sts); let e = self.sexp(sc, 1); out.push(St::StSetS(n, e)); }
            18 if !sc.sts.is_empty() => { let n = self.pick(&sc.sts); let e = self.iexp(sc, 1); out.push(St::StBump(n, e)); }
            19 => { let n = self.name("c"); let e = self.iexp(sc, 1); out.push(St::DeclCl(n.clone(), e)); sc.cls.push(n); }
            20 if !sc.cls.is_empty() => { let src = self.pick(&sc.cls); let n = self.name("c"); out.push(St::AliasCl(n.clone(), src)); sc.cls.push(n); }
            21 if !sc.cls.is_empty() => { let n = self.pick(&sc.cls); let e = self.iexp(sc, 1); out.push(St::ClAdd(n, e)); }
            22 if !sc.cls.is_empty() => { let n = self.pick(&sc.cls); let e = self.sexp(sc, 1); out.push(St::ClPushItem(n, e)); }
            23 if !sc.cls.is_empty() => { let n = self.pick(&sc.cls); let e = self.iexp(sc, 1); out.push(St::ClStAdd(n, e)); }
            24 if !sc.cls.is_empty() => { let c = self.pick(&sc.cls); let n = self.name("t"); out.push(St::SnapSt(n.clone(), c)); sc.sts.push(n); }
            25 if !sc.cls.is_empty() && !sc.sts.is_empty() => { let c = self.pick(&sc.cls); let s = self.pick(&sc.sts); out.push(St::ClSetSt(c, s)); }
            26 if !sc.ints.is_empty() => { let v = self.pick(&sc.ints); let f = self.name("f"); out.push(St::DeclF(f.clone(), v)); sc.fs.push(f); }
            27 if !sc.fs.is_empty() => { let f = self.pick(&sc.fs); let e = self.iexp(sc, 1); out.push(St::CallF(f, e)); }
            28 | 29 => { let e = self.sexp(sc, 2); out.push(St::Obs(e)); }
            30 if depth > 0 => {
                let c = self.iexp(sc, 1); let k = self.r.below(7) as i64;
                let mut s1 = sc.clone(); let t = self.block(&mut s1, 3, depth - 1, in_loop);
                let mut s2 = sc.clone(); let e = self.block(&mut s2, 2, depth - 1, in_loop);
                out.push(St::If(c, k, t, e));
            }
            31 | 32 if depth > 0 => {
                let v = self.name("k"); let n = self.r.below(4) as i64 + 1;
                let mut s1 = sc.clone(); s1.ro.push(v.clone());
                let mut b = self.block(&mut s1, 4, depth - 1, true);
                // loop var must be known to the model: DeclI is emitted by For (handled by exec through env.ints)
                let mut s0 = sc.clone(); s0.ro.push(v.clone()); let e = self.iexp(&s0, 1); let m = self.r.below(3) as i64 + 2;
                let pos = self.r.below(b.len() as u64 + 1) as usize;
                b.insert(pos, if self.r.chance(50) { St::BreakIf(e, m, 0) } else { St::ContIf(e, m, 1) });
                out.push(St::For(v, n, b));
            }
            33 if depth == 3 || !in_loop => { let e = self.iexp(sc, 1); out.push(St::RetIf(e, 9, 0)); }
            _ => {}
        }
    }
}

// The loop variable is declared by the emitted `for`; make the model aware: exec sets env.ints[v] per iteration.
fn fix_loops(st: &mut [St]) {
    for s in st.iter_mut() {
        match s {
            St::If(_, _, t, e) => { fix_loops(t); fix_loops(e); }
            St::For(v, _, b) => { b.insert(0, St::DeclI(format!("__loopvar_{v}"), I::Lit(0))); fix_loops(b); }
            _ => {}
        }
    }
}

const PRELUDE: &str = r#"import { Log } from "host:log";
struct Sx { a: i32; xs: i32[]; s: string; mutating bump(d: i32): void { this.a += d; } }
class Cx { n: i32; items: string[] = ["i"]; st: Sx = { a: 100, xs: [1, 2], s: "cs" }; constructor(n: i32) { this.n = n; } }
struct W1 { v: i32; }
class Wr { v: i32; constructor(v: i32) { this.v = v; } }
class WrS { v: string; constructor(v: string) { this.v = v; } }
function mk(x: i32): W1 { return { v: x }; }
function idt<T>(x: T): T { return x; }
"#;

/// Final observation of all live variables (so every path is compared and every var stays live to the end).
fn summary(sc: &Scope) -> S {
    let mut e = S::Lit("=".into());
    for i in &sc.ints { e = S::Cat(Box::new(e), Box::new(S::Cat(Box::new(S::Lit(" ".into())), Box::new(S::OfInt(Box::new(I::Var(i.clone()))))))); }
    for s in &sc.strs { e = S::Cat(Box::new(e), Box::new(S::Cat(Box::new(S::Lit(" ".into())), Box::new(S::Var(s.clone()))))); }
    for a in &sc.arrs { e = S::Cat(Box::new(e), Box::new(S::OfInt(Box::new(I::Add(Box::new(I::ArrLen(a.clone())), Box::new(I::ArrGet(a.clone(), Box::new(I::Lit(1))))))))); }
    for t in &sc.sts { e = S::Cat(Box::new(e), Box::new(S::Cat(Box::new(S::OfInt(Box::new(I::StA(t.clone())))), Box::new(S::StS(t.clone()))))); }
    for c in &sc.cls { e = S::Cat(Box::new(e), Box::new(S::Cat(Box::new(S::OfInt(Box::new(I::Add(Box::new(I::ClN(c.clone())), Box::new(I::ClStA(c.clone())))))), Box::new(S::ClItem(c.clone(), Box::new(I::Lit(3))))))); }
    e
}

pub struct Prog8 { pub src_plain: String, pub src_meta: String, pub want: Vec<String> }

fn make(seed: u64, nfuncs: usize) -> Prog8 {
    let mut want = Vec::new();
    let mut plain = String::from(PRELUDE);
    let mut meta = String::from(PRELUDE);
    let mut calls = String::new();
    for f in 0..nfuncs {
        let mut g = Gen { r: Rng(seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(f as u64 * 7919 + 1)), uid: 0, wrap_pct: 25 };
        let mut sc = Scope::default();
        // seed variables so every kind exists
        let mut body = vec![St::DeclI("i0".into(), I::Lit(3)), St::DeclS("s0".into(), S::Lit("a".into())), St::DeclA("a0".into(), vec![4, 5])];
        sc.ints.push("i0".into()); sc.strs.push("s0".into()); sc.arrs.push("a0".into());
        let mut rest = g.block(&mut sc, 12, 3, false);
        body.append(&mut rest);
        let mut tail = Vec::new();
        // loop vars: declare in model. We emit the summary only over variables declared at function level (sc after block holds them).
        tail.push(St::Obs(summary(&sc)));
        fix_loops(&mut body);
        let mut env = Env::default();
        // loop variables are defined per iteration by the model through DeclI("__loopvar_*") which is never read: harmless.
        // the real loop variable is read via I::Var(k): give it a value that the emitted code also has (the iteration index).
        let fl = exec2(&body, &mut env);
        if fl != Flow::Ret { let _ = exec(&tail, &mut env); }
        want.extend(env.out.iter().cloned());
        for (m, buf) in [(false, &mut plain), (true, &mut meta)] {
            buf.push_str(&format!("function sc{f}(log: Log): void {{\n"));
            let mut o = String::new();
            emit(&body, m, 1, &mut o);
            buf.push_str(&o);
            let mut t = String::new();
            emit(&tail, m, 1, &mut t);
            buf.push_str(&t);
            buf.push_str("}\n");
        }
        calls.push_str(&format!("  sc{f}(log);\n"));
    }
    for buf in [&mut plain, &mut meta] { buf.push_str(&format!("export function main(log: Log): void {{\n{calls}}}\n")); }
    Prog8 { src_plain: plain, src_meta: meta, want }
}

// For: the model must give the loop variable its iteration value; implemented by a dedicated executor wrapper.
fn run_model_with_loopvars(body: &[St], env: &mut Env) { exec2(body, env); }
fn exec2(st: &[St], env: &mut Env) -> Flow {
    for s in st {
        match s {
            St::For(v, n, b) => {
                for it in 0..*n {
                    env.ints.insert(v.clone(), it);
                    match exec2(b, env) { Flow::Break => break, Flow::Ret => return Flow::Ret, _ => {} }
                }
            }
            St::If(c, k, t, e) => { let x = mi(c, env); let f = if x % 7 == *k { exec2(t, env) } else { exec2(e, env) }; if f != Flow::Next { return f; } }
            other => { let f = exec(std::slice::from_ref(other), env); if f != Flow::Next { return f; } }
        }
    }
    Flow::Next
}

fn run_prog(src: &str) -> Result<Vec<String>, String> {
    match super_run(src) { Ok((log, live)) => if live != 0 { Err(format!("LEAK {live}")) } else { Ok(log) }, Err(e) => Err(e) }
}
fn super_run(src: &str) -> Result<(Vec<String>, i32), String> {
    let r = std::panic::catch_unwind(|| {
        let wasm = oxiscript_lower::compile_source(src, &interfaces(), &oxiscript_lower::Options::default()).map_err(|e| format!("REJECT {e}"))?;
        let env = env();
        let mut rt = env.host.instantiate(&wasm, oxiscript_runtime::Grants::all()).map_err(|e| format!("TRAP {e}"))?;
        let log: Vec<String> = env.sink.lines().into_iter().map(|l| l.msg).collect();
        let live = rt.deinit().map_err(|e| format!("deinit {e}"))?;
        Ok::<_, String>((log, live))
    });
    match r { Ok(x) => x, Err(_) => Err("PANIC in compile/run".into()) }
}

fn campaign(seeds: std::ops::Range<u64>, nfuncs: usize) -> (usize, Vec<String>) {
    let mut fails = Vec::new();
    let mut total = 0;
    for seed in seeds {
        let p = make(seed, nfuncs);
        total += nfuncs;
        for (kind, src) in [("plain", &p.src_plain), ("meta", &p.src_meta)] {
            match run_prog(src) {
                Ok(log) => if log != p.want && !truncated_equal(&log, &p.want) {
                    let i = (0..log.len().max(p.want.len())).find(|&i| log.get(i) != p.want.get(i)).unwrap_or(0);
                    let c = |o: Option<&String>| o.map(|s| format!("len {} {:?}", s.len(), s.chars().take(60).collect::<String>())).unwrap_or("<none>".into());
                    fails.push(format!("seed {seed} {kind}: LOG mismatch at line {i}: got {} want {} (log lines {} vs {})\n{}", c(log.get(i)), c(p.want.get(i)), log.len(), p.want.len(), src));
                },
                Err(e) => fails.push(format!("seed {seed} {kind}: {}\n{}", e.chars().take(300).collect::<String>(), src)),
            }
        }
    }
    (total, fails)
}

#[test]
fn fuzz_short_seeded() {
    let (n, fails) = campaign(0..200, 1);
    eprintln!("generated functions: {n} (x2 variants), failures: {}", fails.len());
    for f in fails.iter().take(2) { eprintln!("{f}"); }
    assert!(fails.is_empty(), "{} failing programs (first shown above)", fails.len());
}

#[test]
#[ignore = "long differential campaign (~minutes); run with --release -- --ignored"]
fn fuzz_long_seeded() {
    let (n, fails) = campaign(1000..1800, 12);
    eprintln!("generated functions: {n} (x2 variants), failures: {}", fails.len());
    for f in fails.iter() { eprintln!("{}", f.lines().next().unwrap_or("")); }
    if let Some(f) = fails.first() { std::fs::write("/tmp/claude-1000/-vectorlegis-ssd-pool-oxiterm-oxiterm/6830e8c3-c411-43eb-ad3c-c468305978e8/scratchpad/badactor-8l/first_fail.txt", f).ok(); }
    assert!(fails.is_empty(), "{} failing programs", fails.len());
}

/// The host log caps a line at 1024 bytes (char boundary): treat a capped line as equal to its model.
fn truncated_equal(got: &[String], want: &[String]) -> bool {
    got.len() == want.len() && got.iter().zip(want).all(|(g, w)| g == w || (g.len() >= 1000 && g.len() <= 1024 && w.starts_with(g.as_str())))
}
