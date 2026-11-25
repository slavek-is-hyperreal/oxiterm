#![allow(clippy::pedantic, clippy::nursery)]
mod common;
use common::*;
use oxiscript_runtime::Value;

#[test]
fn many_tiny_methods_module() {
    let mut src = String::from("import { Log } from \"host:log\";\n");
    for i in 0..1200 {
        src.push_str(&format!("export function g{i}(): i32 {{ return {i}; }}\n"));
    }
    src.push_str("export function main(log: Log): void { log.write(2, \"ok\"); }\n");
    let wasm = compile(&src);
    let env = env();
    match env.host.instantiate(&wasm, oxiscript_runtime::Grants::all()) {
        Ok(mut rt) => {
            assert_eq!(rt.call_export("g7", &[]).unwrap(), vec![Value::I32(7)]);
            eprintln!("tiny methods: loads");
        }
        Err(e) => eprintln!("tiny methods: load rejected: {e}"),
    }
}

fn lcg(s: &mut u64) -> i64 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*s >> 33) % 2001) as i64 - 1000
}

#[test]
fn differential_scalar_expressions() {
    let mut seed = 12345u64;
    let mut cases = Vec::new();
    for _ in 0..40 {
        cases.push((lcg(&mut seed), lcg(&mut seed), lcg(&mut seed)));
    }
    let src = "import { Log } from \"host:log\";\n\
export function f(a: i64, b: i64, c: i64): i64 { return (a * b + c) % 97 - (a ^ c) + ((b & 255) << 2) - (a >> 3); }\n\
export function g(a: i64, b: i64): i64 { return a *% 3i64 +% b -% (a | b); }\n\
export function main(log: Log): void { log.write(2, \"x\"); }\n";
    let mut p = Prog::new(src);
    for (a, b, c) in cases {
        let want_f = (a * b + c) % 97 - (a ^ c) + ((b & 255) << 2) - (a >> 3);
        let want_g = a.wrapping_mul(3).wrapping_add(b).wrapping_sub(a | b);
        assert_eq!(p.call("f", &[Value::I64(a), Value::I64(b), Value::I64(c)]), vec![Value::I64(want_f)]);
        assert_eq!(p.call("g", &[Value::I64(a), Value::I64(b)]), vec![Value::I64(want_g)]);
    }
}
