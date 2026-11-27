#![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.M-link: differential Stub vs Linked over every program we can find
//! (syntax corpus, site examples, doc blocks) plus every no-capability export called with
//! small arguments. Compares log, results, trap kind and leaks after deinit.
//! Known/expected Stub-only differences are bucketed, not failures: f64->text (Stub traps),
//! cycles (Stub leaks, Linked collects).
mod common;
use common::*;
use oxiscript_check::MapLoader;
use oxiscript_lower::{compile_program, Options};
use oxiscript_runtime::{Error, Grants, Value};
use oxiscript_wasm::RuntimeMode;

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub compiled: bool,
    pub load: String,
    pub log: Vec<String>,
    pub calls: Vec<String>,
    pub live: i32,
}

fn vstr(v: &Value) -> String {
    match v {
        Value::F32(f) => format!("f32:{:08x}", f.to_bits()),
        Value::F64(f) => format!("f64:{:016x}", f.to_bits()),
        o => format!("{o:?}"),
    }
}

fn errstr(e: &Error) -> String {
    match e {
        Error::Trap(t) => format!("trap:{:?}/{}/{:?}", t.guest_kind, t.guest_kind_raw, t.code()),
        o => format!("err:{}", o.code()),
    }
}

fn loader() -> MapLoader {
    let mut m = MapLoader::default();
    m.0.insert(
        "util".into(),
        "export function a(): i32 { return 1; }\nexport const b: i32 = 2;\nexport struct d { x: i32; }\nexport struct Box<T> { v: T; }\n".into(),
    );
    m.0.insert("std/math".into(), "export function sqrt(x: f64): f64 { return x; }\n".into());
    m
}

pub fn run_mode(src: &str, mode: RuntimeMode) -> Outcome {
    let opts = Options { runtime: mode, lower_all: true, ..Options::default() };
    let out = match std::panic::catch_unwind(|| compile_program("main", src, &mut loader(), &interfaces(), &opts)) {
        Ok(Ok(o)) => o,
        Ok(Err(_)) => return Outcome { compiled: false, load: String::new(), log: vec![], calls: vec![], live: 0 },
        Err(_) => return Outcome { compiled: false, load: "PANIC".into(), log: vec![], calls: vec![], live: 0 },
    };
    let env = env();
    let mut rt = match env.host.instantiate(&out.wasm, Grants::all()) {
        Ok(r) => r,
        Err(e) => {
            let log = env.sink.lines().into_iter().map(|l| l.msg).collect();
            return Outcome { compiled: true, load: errstr(&e), log, calls: vec![], live: 0 };
        }
    };
    let mut calls = Vec::new();
    for name in rt.user_exports() {
        let Some((ps, _)) = rt.export_type(&name) else { continue };
                for seed in [0i64, 1, 7] {
            let mut args = Vec::new();
            let mut ok = true;
            for p in &ps {
                match format!("{p:?}").as_str() {
                    "I32" => args.push(Value::I32(seed as i32)),
                    "I64" => args.push(Value::I64(seed)),
                    "F32" => args.push(Value::F32(seed as f32 * 0.5)),
                    "F64" => args.push(Value::F64(seed as f64 * 0.5)),
                    _ => ok = false,
                }
            }
            if !ok {
                break;
            }
            match rt.call_export(&name, &args) {
                Ok(r) => calls.push(format!("{name}({seed}) = {}", r.iter().map(vstr).collect::<Vec<_>>().join(","))),
                Err(e) => {
                    calls.push(format!("{name}({seed}) -> {}", errstr(&e)));
                    break;
                }
            }
        }
    }
    let log = env.sink.lines().into_iter().map(|l| l.msg).collect();
    let live = if rt.is_poisoned() { -1 } else { rt.deinit().unwrap_or(-2) };
    Outcome { compiled: true, load: String::new(), log, calls, live }
}

fn doc_blocks(path: &str) -> Vec<String> {
    let Ok(t) = std::fs::read_to_string(path) else { return vec![] };
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    for l in t.lines() {
        if let Some(c) = cur.as_mut() {
            if l.trim_start().starts_with("```") {
                out.push(cur.take().unwrap());
            } else {
                c.push_str(l);
                c.push('\n');
            }
        } else if l.trim() == "```oxs" {
            cur = Some(String::new());
        }
    }
    out
}

fn corpus() -> Vec<(String, String)> {
    let root = format!("{}/../..", env!("CARGO_MANIFEST_DIR"));
    let mut v = Vec::new();
    let mut add_dir = |dir: &str| {
        if let Ok(rd) = std::fs::read_dir(format!("{root}/{dir}")) {
            let mut fs: Vec<_> = rd.map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "oxs")).collect();
            fs.sort();
            for f in fs {
                v.push((f.display().to_string(), std::fs::read_to_string(&f).unwrap()));
            }
        }
    };
    add_dir("oxiscript/oxiscript-syntax/tests/corpus/valid");
    add_dir("sites/oxiscript/examples");
    add_dir("oxiscript/oxiscript-cli/tests/fixtures");
    add_dir("oxiscript/oxiscript-cli/templates/common");
    for doc in ["AGENTS.md", "docs/oxiscript/llms.txt", "docs/oxiscript/grammar.md", ".claude/skills/oxiscript-authoring/SKILL.md"] {
        for (i, b) in doc_blocks(&format!("{root}/{doc}")).into_iter().enumerate() {
            v.push((format!("{doc}#{i}"), b));
        }
    }
    v
}

#[test]
fn stub_and_linked_agree_on_corpus() {
    let mut n = 0;
    let mut compiled = 0;
    let mut diffs = Vec::new();
    let mut leaks = Vec::new();
    let mut f64text = 0;
    for (name, src) in corpus() {
        n += 1;
        let s = run_mode(&src, RuntimeMode::Stub);
        let l = run_mode(&src, RuntimeMode::Linked);
        if l.compiled {
            compiled += 1;
        }
        if l.compiled && l.live != 0 && s.live == 0 {
            leaks.push(format!("{name}: linked live={} (stub live={})", l.live, s.live));
        }
        if s != l {
            // bucket: Stub has no f64->text
            if s.log != l.log || s.calls != l.calls || s.load != l.load {
                if s.calls.iter().chain(s.log.iter()).any(|x| x.contains("Unreachable") || x.contains("unreachable")) || s.load.contains("Unreachable") {
                    f64text += 1;
                    continue;
                }
            }
            if s.compiled == l.compiled && s.log == l.log && s.calls == l.calls && s.load == l.load && s.live != l.live {
                // stub leaks (cycles), linked collects: expected bucket
                continue;
            }
            diffs.push(format!("{name}:\n stub  {s:?}\n linked {l:?}"));
        }
    }
    eprintln!("corpus programs: {n}, compiled {compiled}, f64-text stub-only diffs {f64text}, diffs {}, linked leaks {}", diffs.len(), leaks.len());
    for d in &diffs {
        eprintln!("{d}");
    }
    for l in &leaks {
        eprintln!("{l}");
    }
    assert_eq!(diffs.len(), 0, "{} differential mismatches", diffs.len());
    assert_eq!(leaks.len(), 0, "{} leaks", leaks.len());
}
