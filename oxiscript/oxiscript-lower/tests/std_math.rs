//! End-to-end runtime tests for `std/math` intrinsics, constants, bit ops, and integer math.
#![allow(clippy::pedantic)]

mod common;

use common::Prog;
use oxiscript_runtime::Value;

#[test]
fn math_float_intrinsics_and_constants() {
    let src = r#"
import {
  abs, sqrt, floor, ceil, trunc, round, min, max, copysign,
  bitsOf, fromBits, isNaN, isFinite, isInfinite, clamp, sign,
  absF32, sqrtF32, floorF32, ceilF32, truncF32, roundF32, minF32, maxF32, copysignF32,
  bitsOfF32, fromBitsF32, isNaNF32, isFiniteF32, isInfiniteF32, clampF32,
  PI, E, LN2, SQRT2, INF, NEG_INF, NAN, F32_INF, F32_NAN
} from "std/math";

@host export function t_sqrt(x: f64): f64 { return sqrt(x); }
@host export function t_abs(x: f64): f64 { return abs(x); }
@host export function t_floor(x: f64): f64 { return floor(x); }
@host export function t_ceil(x: f64): f64 { return ceil(x); }
@host export function t_trunc(x: f64): f64 { return trunc(x); }
@host export function t_round(x: f64): f64 { return round(x); }
@host export function t_min(a: f64, b: f64): f64 { return min(a, b); }
@host export function t_max(a: f64, b: f64): f64 { return max(a, b); }
@host export function t_copysign(x: f64, y: f64): f64 { return copysign(x, y); }
@host export function t_bits_rt(x: f64): u64 { return bitsOf(x); }
@host export function t_from_bits(b: u64): f64 { return fromBits(b); }
@host export function t_clamp(x: f64, lo: f64, hi: f64): f64 { return clamp(x, lo, hi); }
@host export function t_sign(x: f64): f64 { return sign(x); }
@host export function t_preds(): i32 {
  let ok: i32 = 0;
  if (isNaN(NAN)) { ok += 1; }
  if (!isNaN(1.0)) { ok += 2; }
  if (isInfinite(INF) && isInfinite(NEG_INF)) { ok += 4; }
  if (isFinite(PI) && isFinite(E) && isFinite(LN2) && isFinite(SQRT2)) { ok += 8; }
  if (!isFinite(INF) && !isFinite(NAN)) { ok += 16; }
  if (isNaNF32(F32_NAN) && isInfiniteF32(F32_INF) && isFiniteF32(1.5f32)) { ok += 32; }
  return ok;
}
@host export function t_f32(): f32 {
  const a = sqrtF32(absF32(-16.0f32));
  const b = floorF32(2.7f32) + ceilF32(1.2f32) + truncF32(-1.9f32) + roundF32(2.5f32);
  const c = minF32(maxF32(clampF32(10.0f32, 0.0f32, 5.0f32), 3.0f32), 4.0f32);
  const d = copysignF32(1.0f32, -2.0f32);
  const e = fromBitsF32(bitsOfF32(0.5f32));
  return a + b + c + d + e;
}
@host export function t_first_class_sqrt(): f64 {
  const xs: f64[] = [4.0, 9.0, 16.0];
  const ys = xs.map(sqrt);
  return ys[0] + ys[1] + ys[2];
}
"#;
    let mut p = Prog::new(src);
    assert_eq!(p.call("t_sqrt", &[Value::F64(9.0)]), vec![Value::F64(3.0)]);
    assert_eq!(p.call("t_abs", &[Value::F64(-42.5)]), vec![Value::F64(42.5)]);
    assert_eq!(p.call("t_floor", &[Value::F64(-1.2)]), vec![Value::F64(-2.0)]);
    assert_eq!(p.call("t_ceil", &[Value::F64(1.2)]), vec![Value::F64(2.0)]);
    assert_eq!(p.call("t_trunc", &[Value::F64(-1.9)]), vec![Value::F64(-1.0)]);
    // round is ties-to-even (Wasm nearest)
    assert_eq!(p.call("t_round", &[Value::F64(2.5)]), vec![Value::F64(2.0)]);
    assert_eq!(p.call("t_round", &[Value::F64(3.5)]), vec![Value::F64(4.0)]);
    assert_eq!(p.call("t_min", &[Value::F64(3.0), Value::F64(-1.0)]), vec![Value::F64(-1.0)]);
    assert_eq!(p.call("t_max", &[Value::F64(3.0), Value::F64(-1.0)]), vec![Value::F64(3.0)]);
    assert_eq!(p.call("t_copysign", &[Value::F64(5.0), Value::F64(-0.0)]), vec![Value::F64(-5.0)]);
    assert_eq!(p.call("t_bits_rt", &[Value::F64(1.0)]), vec![Value::I64(1.0f64.to_bits() as i64)]);
    // NaN canonicalisation in bitsOf
    let weird_nan = f64::from_bits(0xfff8_0000_0000_1234);
    assert_eq!(
        p.call("t_bits_rt", &[Value::F64(weird_nan)]),
        vec![Value::I64(0x7ff8_0000_0000_0000u64 as i64)]
    );
    assert_eq!(p.call("t_from_bits", &[Value::I64(1.0f64.to_bits() as i64)]), vec![Value::F64(1.0)]);
    assert_eq!(p.call("t_clamp", &[Value::F64(10.0), Value::F64(0.0), Value::F64(5.0)]), vec![Value::F64(5.0)]);
    assert_eq!(p.call("t_sign", &[Value::F64(-3.0)]), vec![Value::F64(-1.0)]);
    assert_eq!(p.call("t_preds", &[]), vec![Value::I32(63)]);
    // a=4, b=2+2+(-1)+2=5, c=4, d=-1, e=0.5 => 12.5
    assert_eq!(p.call("t_f32", &[]), vec![Value::F32(12.5)]);
    assert_eq!(p.call("t_first_class_sqrt", &[]), vec![Value::F64(9.0)]);
    p.finish();
}

#[test]
fn math_integer_bit_ops_and_bounds() {
    let src = r#"
import {
  clz8, ctz8, popcnt8, rotl8, rotr8,
  clz16, ctz16, popcnt16, rotl16, rotr16,
  clz32, ctz32, popcnt32, rotl32, rotr32,
  clz64, ctz64, popcnt64, rotl64, rotr64,
  clzI32, ctzI32, popcntI32, clzI64, ctzI64, popcntI64,
  minI32, maxI32, absI32, clampI32,
  minI64, maxI64, absI64, clampI64,
  minU32, maxU32, clampU32, minU64, maxU64, clampU64,
  wrappingAddI32, wrappingSubI32, wrappingMulI32, wrappingAbsI32,
  saturatingAddI32, saturatingSubI32, saturatingMulI32, saturatingAbsI32,
  wrappingAddI64, wrappingAbsI64, saturatingAbsI64,
  I8_MIN, I8_MAX, U8_MAX, I16_MIN, I16_MAX, U16_MAX,
  I32_MIN, I32_MAX, U32_MAX, I64_MIN, I64_MAX, U64_MAX
} from "std/math";

@host export function test_all(): i32 {
  let ok: i32 = 0;
  if (clz8(0u8) === 8u8 && clz8(0x0fu8) === 4u8 && ctz8(0x10u8) === 4u8 && popcnt8(0xaau8) === 4u8) { ok += 1; }
  if (rotl8(0x81u8, 1u8) === 0x03u8 && rotr8(0x03u8, 1u8) === 0x81u8) { ok += 2; }
  if (clz16(0u16) === 16u16 && ctz16(0x0100u16) === 8u16 && popcnt16(0xffffu16) === 16u16) { ok += 4; }
  if (rotl16(0x8001u16, 1u16) === 0x0003u16 && rotr16(0x0003u16, 1u16) === 0x8001u16) { ok += 8; }
  if (clz32(1u32) === 31u32 && ctz32(8u32) === 3u32 && popcnt32(0x0f0f_0f0fu32) === 16u32) { ok += 16; }
  if (rotl32(0x8000_0001u32, 1u32) === 3u32 && rotr32(3u32, 1u32) === 0x8000_0001u32 && rotl32(7u32, 32u32) === 7u32) { ok += 32; }
  if (clz64(1u64) === 63u64 && ctz64(16u64) === 4u64 && popcnt64(U64_MAX) === 64u64) { ok += 64; }
  if (rotl64(0x8000_0000_0000_0001u64, 1u64) === 3u64 && rotr64(3u64, 1u64) === 0x8000_0000_0000_0001u64) { ok += 128; }
  if (clzI32(-1) === 0 && ctzI32(0) === 32 && popcntI32(7) === 3 && clzI64(0i64) === 64i64 && ctzI64(8i64) === 3i64 && popcntI64(7i64) === 3i64) { ok += 256; }
  if (minI32(-5, 3) === -5 && maxI32(-5, 3) === 3 && absI32(-9) === 9 && clampI32(10, 0, 5) === 5) { ok += 512; }
  if (minI64(-5i64, 3i64) === -5i64 && maxI64(-5i64, 3i64) === 3i64 && absI64(-9i64) === 9i64 && clampI64(-2i64, 0i64, 5i64) === 0i64) { ok += 1024; }
  if (minU32(2u32, 9u32) === 2u32 && maxU32(2u32, 9u32) === 9u32 && clampU32(99u32, 1u32, 10u32) === 10u32) { ok += 2048; }
  if (minU64(2u64, 9u64) === 2u64 && maxU64(2u64, 9u64) === 9u64 && clampU64(0u64, 1u64, 10u64) === 1u64) { ok += 4096; }
  if (wrappingAddI32(I32_MAX, 1) === I32_MIN && wrappingSubI32(I32_MIN, 1) === I32_MAX && wrappingMulI32(I32_MAX, 2) === -2) { ok += 8192; }
  if (saturatingAddI32(I32_MAX, 10) === I32_MAX && saturatingSubI32(I32_MIN, 10) === I32_MIN && saturatingMulI32(I32_MAX, 2) === I32_MAX) { ok += 16384; }
  if (wrappingAbsI32(I32_MIN) === I32_MIN && saturatingAbsI32(I32_MIN) === I32_MAX) { ok += 32768; }
  if (wrappingAddI64(I64_MAX, 1i64) === I64_MIN && wrappingAbsI64(I64_MIN) === I64_MIN && saturatingAbsI64(I64_MIN) === I64_MAX) { ok += 65536; }
  if (I8_MIN === -128i8 && I8_MAX === 127i8 && U8_MAX === 255u8 && I16_MIN === -32768i16 && I16_MAX === 32767i16 && U16_MAX === 65535u16 && U32_MAX === 4294967295u32) { ok += 131072; }
  return ok;
}
"#;
    let mut p = Prog::new(src);
    assert_eq!(p.call("test_all", &[]), vec![Value::I32((1 << 18) - 1)]);
    p.finish();
}
