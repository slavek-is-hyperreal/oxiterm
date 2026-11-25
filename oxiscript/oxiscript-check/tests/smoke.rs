#![allow(clippy::format_push_string)]
mod common;
use common::*;

#[test]
fn smoke_basic() {
    assert_ok(
        r#"
import { Log } from "host:log";
let count: i32 = 0;
function render(log: Log): void { log.write(1, `count = ${count}`); }
export function main(log: Log): void { count = 0; render(log); }
"#,
    );
}

#[test]
fn smoke_union() {
    assert_ok(
        r#"
type Shape =
  | { kind: "circle"; r: f64 }
  | { kind: "rect"; w: f64; h: f64 };
export function area(s: Shape): f64 {
  switch (s.kind) {
    case "circle": return 3.14 * s.r * s.r;
    case "rect": return s.w * s.h;
  }
}
function f(s: Shape): f64 {
  if (s.kind === "circle") { return s.r; }
  return s.w;
}
"#,
    );
}

#[test]
fn smoke_errors() {
    let r = show(
        r"
function f(a: i32, b: i64): i32 { return a + b; }
function g(): i32 { let x: i32 | null = null; return x + 1; }
",
    );
    println!("{r}");
    assert!(r.contains("OXS-E0110"));
}
