#![allow(clippy::pedantic, clippy::nursery, clippy::format_push_string)]
use oxiscript_lower::{compile_source, Options};
use std::time::Instant;

fn main() {
    let itf = oxiscript_abi::idl::parse(oxiscript_abi::idl::EXAMPLE_LOG_IDL).unwrap();
    for n in [500usize, 2000, 8000] {
        let mut s = String::from("import { Log } from \"host:log\";\nstruct P { x: i32; y: string; }\n");
        for i in 0..n {
            s.push_str(&format!(
                "function f{i}(a: i32, p: P): i32 {{\n  let t = a;\n  for (let i = 0; i < 3; i++) {{ t += i * a; }}\n  const q = p;\n  if (t > {i}) {{ return t + q.x; }}\n  return t;\n}}\n"
            ));
        }
        s.push_str("export function main(log: Log): void {\n  const p: P = { x: 1, y: \"s\" };\n  let t = 0;\n");
        for i in 0..n.min(400) {
            s.push_str(&format!("  t += f{i}({i}, p);\n"));
        }
        s.push_str("  log.write(2, `${t}`);\n}\n");
        let lines = s.lines().count();
        let t0 = Instant::now();
        let w = compile_source(&s, &itf, &Options { lower_all: true, ..Options::default() }).unwrap_or_else(|e| panic!("{e}"));
        let dt = t0.elapsed().as_secs_f64();
        println!("{lines} lines -> {} bytes wasm in {:.3}s = {:.0} lines/s", w.len(), dt, lines as f64 / dt);
    }
}
