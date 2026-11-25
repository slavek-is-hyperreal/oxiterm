//! Examples for `oxi explain`: `(code, failing module, fixed module)`.
//! A unit test checks that every failing module emits its code and every
//! fixed module does not.

pub const EXAMPLES: &[(&str, &str, &str)] = &[
    ("OXS-E0001", "const a = (1;\n", "const a = (1);\n"),
    ("OXS-E0002", "const s = \"abc;\n", "const s = \"abc\";\n"),
    ("OXS-E0003", "const a = 1f32;\n", "const a = 1.0f32;\n"),
    ("OXS-E0004", "const a: i32 = 1\n", "const a: i32 = 1;\n"),
    (
        "OXS-E0005",
        "export function same(a: i32, b: i32): bool {\n  return a == b;\n}\n",
        "export function same(a: i32, b: i32): bool {\n  return a === b;\n}\n",
    ),
    (
        "OXS-E0006",
        "function g(): Result<i32, string> { return Ok(1); }\nexport function f(): Result<i32, string> {\n  const a = g() ?;\n  return Ok(a);\n}\n",
        "function g(): Result<i32, string> { return Ok(1); }\nexport function f(): Result<i32, string> {\n  const a = g()?;\n  return Ok(a);\n}\n",
    ),
    (
        "OXS-E0007",
        "export function f(a: i32, b: i32, c: i32): bool { return a & b === c; }\n",
        "export function f(a: i32, b: i32, c: i32): bool { return (a & b) === c; }\n",
    ),
    (
        "OXS-E0008",
        "export function f(a: u32, b: u32): u32 { return a << b + 1u32; }\n",
        "export function f(a: u32, b: u32): u32 { return a << (b + 1u32); }\n",
    ),
    (
        "OXS-E0009",
        "export function f(a: bool | null, b: bool): bool { return a ?? b || false; }\n",
        "export function f(a: bool | null, b: bool): bool { return (a ?? b) || false; }\n",
    ),
    (
        "OXS-E0010",
        "export function f(x: u16): u8 { return x as u8 | 1; }\n",
        "export function f(x: u16): u8 { return (x as u8) | 1; }\n",
    ),
    ("OXS-E0013", "const s = \"\\q\";\n", "const s = \"\\\\q\";\n"),
    ("OXS-E0014", "const a = 1 # 2;\n", "const a = 1;\n"),
    (
        "OXS-E0015",
        "export function f(): void {\n  1 = 2;\n}\n",
        "export function f(): void {\n  let a: i32 = 1;\n  a = 2;\n}\n",
    ),
    (
        "OXS-E0016",
        "export function f(): i32 {\n  let i: i32 = 0;\n  const j = i++;\n  return j;\n}\n",
        "export function f(): i32 {\n  let i: i32 = 0;\n  i++;\n  return i;\n}\n",
    ),
    (
        "OXS-E0100",
        "export function widen(n: i32): f64 {\n  return n;\n}\n",
        "export function widen(n: i32): f64 {\n  return n as f64;\n}\n",
    ),
    (
        "OXS-E0101",
        "type S = { kind: \"a\" } | { kind: \"b\" };\nexport function f(s: S): void {\n  switch (s.kind) {\n    case \"a\":\n      return;\n  }\n}\n",
        "type S = { kind: \"a\" } | { kind: \"b\" };\nexport function f(s: S): void {\n  switch (s.kind) {\n    case \"a\":\n      return;\n    case \"b\":\n      return;\n  }\n}\n",
    ),
    ("OXS-E0102", "const a: u8 = 300;\n", "const a: u8 = 255;\n"),
    (
        "OXS-E0103",
        "const p = { x: 1.0, y: 2.0 };\n",
        "struct Point {\n  x: f64;\n  y: f64;\n}\n\nconst p: Point = { x: 1.0, y: 2.0 };\n",
    ),
    (
        "OXS-E0104",
        "function g(): Result<i32, string> { return Ok(1); }\nexport function f(): void {\n  const a = g()?;\n}\n",
        "function g(): Result<i32, string> { return Ok(1); }\nexport function f(): Result<i32, string> {\n  const a = g()?;\n  return Ok(a);\n}\n",
    ),
    (
        "OXS-E0105",
        "export function f(s: string | null): i32 {\n  return s.byteLength;\n}\n",
        "export function f(s: string | null): i32 {\n  if (s === null) {\n    return 0;\n  }\n  return s.byteLength;\n}\n",
    ),
    (
        "OXS-E0106",
        "function g(): Result<string, string> { return Ok(\"a\"); }\nexport function f(): Result<i32, string> {\n  const n = g()?.byteLength;\n  return Ok(n);\n}\n",
        "function g(): Result<string, string> { return Ok(\"a\"); }\nexport function f(): Result<i32, string> {\n  const n = (g()?).byteLength;\n  return Ok(n);\n}\n",
    ),
    (
        "OXS-E0107",
        "function g(a: i32): i32 { return a; }\nexport function f(): i32 {\n  return g(1, 2);\n}\n",
        "function g(a: i32): i32 { return a; }\nexport function f(): i32 {\n  return g(1);\n}\n",
    ),
    (
        "OXS-E0108",
        "export function size(s: string): i32 {\n  return s.length;\n}\n",
        "export function size(s: string): i32 {\n  return s.byteLength;\n}\n",
    ),
    (
        "OXS-E0109",
        "const a: i32 = 1;\nexport function f(): void {\n  a = 2;\n}\n",
        "let a: i32 = 1;\nexport function f(): void {\n  a = 2;\n}\n",
    ),
    (
        "OXS-E0110",
        "export function f(a: bool): i32 {\n  return a + 1;\n}\n",
        "export function f(a: i32): i32 {\n  return a + 1;\n}\n",
    ),
    (
        "OXS-E0111",
        "export function f(): i32 {\n  return \"a\" as i32;\n}\n",
        "export function f(): i32 {\n  return 97;\n}\n",
    ),
    (
        "OXS-E0112",
        "export function f(): void {\n  const a = 1;\n  a();\n}\n",
        "function a(): void {}\nexport function f(): void {\n  a();\n}\n",
    ),
    (
        "OXS-E0114",
        "export function f(a: i32): i32 {\n  if (a > 0) {\n    return 1;\n  }\n}\n",
        "export function f(a: i32): i32 {\n  if (a > 0) {\n    return 1;\n  }\n  return 0;\n}\n",
    ),
    (
        "OXS-E0115",
        "export function f(a: string | i32): void {}\n",
        "export function f(a: string | null): void {}\n",
    ),
    (
        "OXS-E0117",
        "export function f(): void {\n  const a: Array<i32, i32> = [];\n}\n",
        "export function f(): void {\n  const a: Array<i32> = [];\n}\n",
    ),
    (
        "OXS-E0118",
        "export function f(): i32 {\n  let a: i32;\n  return a;\n}\n",
        "export function f(): i32 {\n  let a: i32 = 0;\n  return a;\n}\n",
    ),
    (
        "OXS-E0119",
        "class C {\n  private n: i32 = 0;\n}\nexport function f(c: C): i32 {\n  return c.n;\n}\n",
        "class C {\n  n: i32 = 0;\n}\nexport function f(c: C): i32 {\n  return c.n;\n}\n",
    ),
    (
        "OXS-E0120",
        "struct P {\n  x: i32;\n}\nexport function f(): P {\n  return { x: 1, y: 2 };\n}\n",
        "struct P {\n  x: i32;\n}\nexport function f(): P {\n  return { x: 1 };\n}\n",
    ),
    (
        "OXS-E0121",
        "struct P {\n  x: i32;\n}\n@host export function f(p: P): i32 {\n  return p.x;\n}\n",
        "struct P {\n  x: i32;\n}\nexport function f(p: P): i32 {\n  return p.x;\n}\n",
    ),
    (
        "OXS-E0122",
        "export function f(): void {\n  break;\n}\n",
        "export function f(): void {\n  while (true) {\n    break;\n  }\n}\n",
    ),
    (
        "OXS-E0124",
        "@foo\nexport function f(): void {}\n",
        "@inline\nexport function f(): void {}\n",
    ),
    (
        "OXS-E0127",
        "export function f(a: i32): i32 {\n  return a ?? 0;\n}\n",
        "export function f(a: i32 | null): i32 {\n  return a ?? 0;\n}\n",
    ),
    (
        "OXS-E0128",
        "export function f(a: bool): i32 {\n  return -a;\n}\n",
        "export function f(a: i32): i32 {\n  return -a;\n}\n",
    ),
    (
        "OXS-E0300",
        "export function half(x: number): number {\n  return x / 2;\n}\n",
        "export function half(x: f64): f64 {\n  return x / 2.0;\n}\n",
    ),
    (
        "OXS-E0301",
        "export function f(): void {}\nexport function f(): void {}\n",
        "export function f(): void {}\nexport function g(): void {}\n",
    ),
    (
        "OXS-E0302",
        "import { a } from \"./nope\";\n",
        "// create nope.oxs next to this file (or fix the path), then import from it\n",
    ),
    (
        "OXS-E0303",
        "struct P {\n  x: i32;\n}\nexport function f(): void {\n  const a = P;\n}\n",
        "struct P {\n  x: i32;\n}\nexport function f(): void {\n  const a: P = { x: 1 };\n}\n",
    ),
    (
        "OXS-E0400",
        "import { Beep } from \"host:beep\";\n",
        "import { Log } from \"host:log\";\n",
    ),
    (
        "OXS-E0401",
        "import { Nope } from \"host:log\";\n",
        "import { Log } from \"host:log\";\n",
    ),
    (
        "OXS-E0900",
        "class A {}\nclass B extends A {}\n",
        "interface Named {\n  name(): string;\n}\nclass A {}\nclass B {\n  a: A = new A();\n}\n",
    ),
    (
        "OXS-W0002",
        "@allow(\"OXS-P0102\")\nexport function f(): void {}\n",
        "@allow(\"OXS-P0102\", \"scalar loop is fine here\")\nexport function f(): void {}\n",
    ),
    (
        "OXS-W0004",
        "export function f(): void {\n  1 + 2;\n}\n",
        "export function f(): i32 {\n  return 1 + 2;\n}\n",
    ),
];

#[cfg(test)]
mod tests {
    use super::EXAMPLES;
    use oxiscript_check::{check_source, CheckOptions};

    fn codes(src: &str) -> Vec<String> {
        let opts = CheckOptions {
            hosts: crate::project::builtin_hosts(),
        };
        check_source(src, &opts)
            .diagnostics
            .iter()
            .map(|d| d.code.to_string())
            .collect()
    }

    #[test]
    fn every_example_fails_with_its_code_and_the_fix_removes_it() {
        let mut seen = std::collections::HashSet::new();
        for (code, bad, good) in EXAMPLES {
            assert!(seen.insert(*code), "duplicate example for {code}");
            assert!(oxiscript_abi::codes::lookup(code).is_some(), "{code} is not in the registry");
            let c = codes(bad);
            assert!(c.iter().any(|x| x == code), "{code}: failing example emits {c:?}\n{bad}");
            let c = codes(good);
            assert!(!c.iter().any(|x| x == code), "{code}: fixed example still emits it: {c:?}\n{good}");
            // the fixed form must not be wrong in other ways, except the deliberate placeholders
            if !matches!(*code, "OXS-E0302") {
                assert!(c.is_empty(), "{code}: fixed example has diagnostics {c:?}\n{good}");
            }
        }
    }
}
