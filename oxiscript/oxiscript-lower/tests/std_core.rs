//! End-to-end runtime tests for `std/string`, `std/array`, and `std/result` (zero leaks).
#![allow(clippy::pedantic)]

mod common;

use common::{run_log, Prog};
use oxiscript_runtime::Value;

#[test]
fn std_string_all_functions() {
    let src = r#"
import { Log } from "host:log";
import {
  isEmpty, charCount, byteAt, compare, fromUtf8, fromCodePoint,
  trim, trimStart, trimEnd, split, replace, replaceAll, repeat,
  padStart, padEnd, toUpperAscii, toLowerAscii, lastIndexOf,
  parseInt, parseInt32, parseFloat, toRadix, toRadixU64, toRadixI32, toFixed
} from "std/string";
import { isNaN, isInfinite, abs, I64_MIN } from "std/math";
import { unwrapOr, isErr } from "std/result";

export function main(log: Log): void {
  // basics & UTF-8
  if (isEmpty("") && !isEmpty("a")) { log.write(2, "empty:ok"); }
  log.write(2, `cc=${charCount("zażółć")}`);
  log.write(2, `b0=${byteAt("ABC", 0)}`);
  log.write(2, `cmp=${compare("abc", "abd")},${compare("abc", "abc")},${compare("abd", "abc")}`);

  const goodUtf8 = fromUtf8([104u8, 105u8]);
  const badUtf8 = fromUtf8([0xffu8, 0x80u8]);
  log.write(2, `utf8=${unwrapOr(goodUtf8, "?")},err=${isErr(badUtf8)}`);

  const cpOk = fromCodePoint(0x1f600u32);
  const cpBad = fromCodePoint(0xd800u32);
  log.write(2, `cp=${charCount(unwrapOr(cpOk, ""))},err=${isErr(cpBad)}`);

  // trim
  log.write(2, `t=[${trim("  \t\nhi \r ")}]|[${trimStart("  hi ")}]|[${trimEnd(" hi  ")}]`);

  // split
  const parts = split("a,b,,c", ",");
  log.write(2, `sp=${parts.length}:${parts[0]}|${parts[1]}|${parts[2]}|${parts[3]}`);
  const chars = split("aé!", "");
  log.write(2, `sp0=${chars.length}:${chars[0]}|${chars[1]}|${chars[2]}`);

  // replace / replaceAll / repeat / pad / case / lastIndexOf
  log.write(2, `rep=${replace("foo bar foo", "foo", "baz")}`);
  log.write(2, `repa=${replaceAll("foo bar foo", "foo", "baz")}`);
  log.write(2, `rpt=${repeat("ab", 3)}|${repeat("ab", 0)}`);
  log.write(2, `pad=[${padStart("7", 3, "0")}]|[${padEnd("hi", 5, ".-")}]`);
  log.write(2, `case=${toUpperAscii("Abc123é")}|${toLowerAscii("AbC123é")}`);
  log.write(2, `lio=${lastIndexOf("ababa", "aba")},${lastIndexOf("abc", "z")}`);

  // parseInt / parseInt32
  log.write(2, `pi=${unwrapOr(parseInt(" -42 "), 0i64)},${unwrapOr(parseInt("ff", 16), 0i64)},${unwrapOr(parseInt("-9223372036854775808"), 0i64) === I64_MIN}`);
  log.write(2, `pi_err=${isErr(parseInt(""))},${isErr(parseInt("12x"))},${isErr(parseInt("99999999999999999999999"))},${isErr(parseInt("10", 1))}`);
  log.write(2, `pi32=${unwrapOr(parseInt32("-2147483648"), 0)},err=${isErr(parseInt32("2147483648"))}`);

  // parseFloat
  const f1 = unwrapOr(parseFloat("  -12.345e2 "), 0.0);
  const f2 = unwrapOr(parseFloat("NaN"), 0.0);
  const f3 = unwrapOr(parseFloat("-Infinity"), 0.0);
  const f4 = unwrapOr(parseFloat("0.00125"), 0.0);
  log.write(2, `pf=${abs(f1 - (-1234.5)) < 1e-12},${isNaN(f2)},${isInfinite(f3) && f3 < 0.0},${abs(f4 - 0.00125) < 1e-15},err=${isErr(parseFloat("abc"))}`);

  // toRadix / toFixed
  log.write(2, `rad=${toRadix(255i64, 16)},${toRadix(-255i64, 16)},${toRadixU64(42u64, 2)},${toRadixI32(-2147483648, 16)}`);
  log.write(2, `fix=${toFixed(3.14159, 2)},${toFixed(-0.004, 2)},${toFixed(9.996, 2)},${toFixed(12345.0, 0)},${toFixed(f2, 2)},${toFixed(f3, 1)}`);
}
"#;
    run_log(
        src,
        &[
            "empty:ok",
            "cc=6",
            "b0=65",
            "cmp=-1,0,1",
            "utf8=hi,err=true",
            "cp=1,err=true",
            "t=[hi]|[hi ]|[ hi]",
            "sp=4:a|b||c",
            "sp0=3:a|é|!",
            "rep=baz bar foo",
            "repa=baz bar baz",
            "rpt=ababab|",
            "pad=[007]|[hi.-.]",
            "case=ABC123é|abc123é",
            "lio=2,-1",
            "pi=-42,255,true",
            "pi_err=true,true,true,true",
            "pi32=-2147483648,err=true",
            "pf=true,true,true,true,err=true",
            "rad=ff,-ff,101010,-80000000",
            "fix=3.14,0.00,10.00,12345,NaN,-Infinity",
        ],
    );
}

#[test]
fn std_array_all_functions_and_merge_sort_stability() {
    let src = r#"
import { Log } from "host:log";
import {
  isEmpty, first, last, sortBy, sortI32, sortI64, sortU32, sortU64, sortF64, sortString,
  isSortedBy, reverse, reverseInPlace, indexOf, includes, find, findIndex, join, fill, flatMap
} from "std/array";
import { NAN, bitsOf } from "std/math";

struct Pair {
  key: i32;
  seq: i32;
}

export function main(log: Log): void {
  const empty: i32[] = [];
  log.write(2, `empty=${isEmpty(empty)},${first(empty) === null},${last(empty) === null}`);
  const nums = [5, 2, 9, 1, 5, 6];
  log.write(2, `fl=${first(nums) ?? -1},${last(nums) ?? -1}`);

  // sortI32 does not mutate original
  const s32 = sortI32(nums);
  log.write(2, `s32=${s32[0]},${s32[1]},${s32[2]},${s32[3]},${s32[4]},${s32[5]}|orig0=${nums[0]}`);
  log.write(2, `sorted=${isSortedBy(s32, (a: i32, b: i32): i32 => a - b)},${isSortedBy(nums, (a: i32, b: i32): i32 => a - b)}`);

  // stability of sortBy: equal keys keep original seq order
  const pairs: Pair[] = [
    { key: 2, seq: 1 },
    { key: 1, seq: 2 },
    { key: 2, seq: 3 },
    { key: 1, seq: 4 },
    { key: 2, seq: 5 },
    { key: 0, seq: 6 },
    { key: 1, seq: 7 },
  ];
  const sp = sortBy(pairs, (a: Pair, b: Pair): i32 => a.key - b.key);
  let seqs = "";
  for (const p of sp) {
    seqs = `${seqs}${p.seq}`;
  }
  log.write(2, `stable=${seqs}`);

  // sortI64 / sortU32 / sortU64 / sortF64 (with -0.0, +0.0, NaN) / sortString
  const s64 = sortI64([3i64, -1i64, 2i64]);
  const su32 = sortU32([10u32, 2u32, 30u32]);
  const su64 = sortU64([10u64, 2u64, 30u64]);
  const sf = sortF64([1.0, NAN, 0.0, -0.0, -2.0]);
  const ss = sortString(["pear", "apple", "banana", "apple"]);
  log.write(2, `types=${s64[0]},${su32[0]},${su64[2]},${bitsOf(sf[1]) === bitsOf(-0.0)},${bitsOf(sf[2]) === bitsOf(0.0)},${sf[4] !== sf[4]}`);
  log.write(2, `sstr=${join(ss, ",")}`);

  // reverse & reverseInPlace
  const rev = reverse([1, 2, 3]);
  const mutArr = [10, 20, 30, 40];
  reverseInPlace(mutArr);
  log.write(2, `rev=${rev[0]},${rev[2]}|mut=${mutArr[0]},${mutArr[3]}`);

  // indexOf / includes (over Eq<T> for primitives and string!)
  log.write(2, `idx=${indexOf([10, 20, 30], 20)},${indexOf([10, 20], 99)},${includes(["a", "b", "c"], "b")},${includes(["a", "b"], "z")}`);

  // find / findIndex
  const found = find([3, 7, 12, 15], (x: i32): bool => x > 10);
  const missing = find([1, 2, 3], (x: i32): bool => x > 10);
  log.write(2, `find=${found ?? -1},${missing === null},${findIndex([3, 7, 12], (x: i32): bool => x > 10)}`);

  // fill / flatMap / join
  const f = fill("x", 3);
  const fm = flatMap([1, 2, 3], (x: i32): i32[] => [x, x * 10]);
  log.write(2, `fill=${join(f, "-")}|fm=${fm.length}:${fm[0]},${fm[1]},${fm[4]},${fm[5]}`);
}
"#;
    run_log(
        src,
        &[
            "empty=true,true,true",
            "fl=5,6",
            "s32=1,2,5,5,6,9|orig0=5",
            "sorted=true,false",
            "stable=6247135",
            "types=-1,2,30,true,true,true",
            "sstr=apple,apple,banana,pear",
            "rev=3,1|mut=40,10",
            "idx=1,-1,true,false",
            "find=12,true,2",
            "fill=x-x-x|fm=6:1,10,3,30",
        ],
    );
}

#[test]
fn std_result_all_functions() {
    let src = r#"
import { Log } from "host:log";
import {
  isOk, isErr, unwrapOr, unwrapOrElse, unwrapErrOr, okValue, errValue,
  map, mapErr, mapOr, andThen, orElse, flatten, all
} from "std/result";

export function main(log: Log): void {
  const a: Result<i32, string> = Ok(21);
  const b: Result<i32, string> = Err("boom");
  log.write(2, `pred=${isOk(a)},${isErr(a)},${isOk(b)},${isErr(b)}`);
  log.write(2, `unw=${unwrapOr(a, 0)},${unwrapOr(b, 99)},${unwrapOrElse(b, (e: string): i32 => e.byteLength)},${unwrapErrOr(a, "none")},${unwrapErrOr(b, "none")}`);
  log.write(2, `opt=${okValue(a) ?? -1},${okValue(b) === null},${errValue(a) === null},${errValue(b) ?? ""}`);
  const m1 = map(a, (x: i32): i32 => x * 2);
  const m2 = mapErr(b, (e: string): i32 => e.byteLength);
  log.write(2, `map=${unwrapOr(m1, 0)},${unwrapErrOr(m2, 0)},${mapOr(a, 0, (x: i32): i32 => x + 1)},${mapOr(b, 7, (x: i32): i32 => x + 1)}`);
  const at = andThen(a, (x: i32): Result<i32, string> => x > 0 ? Ok(x + 10) : Err("neg"));
  const oe = orElse(b, (e: string): Result<i32, i32> => Ok(e.byteLength));
  log.write(2, `chain=${unwrapOr(at, 0)},${unwrapOr(oe, 0)}`);
  const nested: Result<Result<i32, string>, string> = Ok(Ok(77));
  const nestedErr: Result<Result<i32, string>, string> = Ok(Err("inner"));
  log.write(2, `flat=${unwrapOr(flatten(nested), 0)},${unwrapErrOr(flatten(nestedErr), "")}`);
  const rsOk: Result<i32, string>[] = [Ok(1), Ok(2), Ok(3)];
  const rsErr: Result<i32, string>[] = [Ok(1), Err("fail"), Ok(3)];
  const all1 = all(rsOk);
  const all2 = all(rsErr);
  log.write(2, `all=${isOk(all1)},${unwrapOr(all1, []).length},${unwrapErrOr(all2, "")}`);
}
"#;
    run_log(
        src,
        &[
            "pred=true,false,false,true",
            "unw=21,99,4,none,boom",
            "opt=21,true,true,boom",
            "map=42,4,22,7",
            "chain=31,4",
            "flat=77,inner",
            "all=true,3,fail",
        ],
    );
}

#[test]
fn sort_comparison_sequence_is_fixed_and_deterministic() {
    let src = r#"
import { sortBy } from "std/array";

@host export function compare_trace(): i32 {
  let hash: i32 = 0;
  const xs = [4, 1, 3, 2, 1, 5, 0, 3];
  const sorted = sortBy(xs, (a: i32, b: i32): i32 => {
    hash = hash *% 31 +% (a *% 7 +% b);
    return a - b;
  });
  return sorted[0] === 0 && sorted[7] === 5 ? hash : -1;
}
"#;
    let mut p1 = Prog::new(src);
    let mut p2 = Prog::new(src);
    let r1 = p1.call("compare_trace", &[]);
    let r2 = p2.call("compare_trace", &[]);
    assert_eq!(r1, r2);
    assert_ne!(r1, vec![Value::I32(-1)]);
    p1.finish();
    p2.finish();
}
