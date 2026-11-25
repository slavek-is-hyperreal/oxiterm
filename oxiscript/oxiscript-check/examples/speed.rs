#![allow(clippy::format_push_string)]
//! Checking speed on a large generated program (structs, unions, generics, closures, switches).
//! `cargo run --release -p oxiscript-check --example speed -- [modules=2000]`
use oxiscript_check::{check_source, CheckOptions};
use std::time::Instant;

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let mut s = String::new();
    s.push_str("struct Box<T> { v: T; }\nfunction id<T>(x: T): T { return x; }\n");
    for i in 0..n {
        s.push_str(&format!(
            r#"struct P{i} {{ x: i32; y: i32; len(): i32 {{ return this.x + this.y; }} }}
type Shape{i} = {{ kind: "c"; r: f64 }} | {{ kind: "s"; w: f64; h: f64 }};
function area{i}(s: Shape{i}): f64 {{
  switch (s.kind) {{
    case "c": return 3.14 * s.r * s.r;
    case "s": return s.w * s.h;
  }}
}}
function work{i}(a: Array<i32>, p: P{i}, m: i32 | null): i32 {{
  let total = 0;
  for (const v of a) {{
    if (v > {i}) {{ total += v; }} else {{ total -= 1; }}
  }}
  const b = id<Box<i32>>({{ v: total }});
  const f = (k: i32) => k + p.len() + b.v;
  if (m !== null) {{ total += m; }}
  return f(total);
}}
"#
        ));
    }
    let lines = s.lines().count();
    let t = Instant::now();
    let c = check_source(&s, &CheckOptions::default());
    let el = t.elapsed();
    let errs: Vec<_> = c.errors().take(3).map(|d| d.code.to_string()).collect();
    println!("{} lines, {} KB, ok={} {:?} in {:?}: {:.0} lines/s", lines, s.len() / 1024, c.ok(), errs, el, lines as f64 / el.as_secs_f64());
}
