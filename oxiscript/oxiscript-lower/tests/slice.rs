#![allow(clippy::pedantic)]
mod common;
use common::*;

#[test]
fn hello() {
    run_log(
        r#"
import { Log } from "host:log";
export function main(log: Log): void {
  log.write(2, "hi");
}
"#,
        &["hi"],
    );
}

#[test]
fn arithmetic_and_template() {
    run_log(
        r#"
import { Log } from "host:log";
function add(a: i32, b: i32): i32 { return a + b; }
export function main(log: Log): void {
  const x = add(40, 2);
  log.write(2, `x=${x}`);
}
"#,
        &["x=42"],
    );
}
