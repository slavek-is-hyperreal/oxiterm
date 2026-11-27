//! Library routines the lowering emits as OxiIR *text*, parsed on demand and appended to
//! the module (number formatting, string search, array iteration helpers). They are
//! ordinary verified IR functions using only the public ops, so they work with any
//! runtime and are subject to the same RC discipline as generated code.

use crate::ctx::Lw;
use crate::types::mangle;
use oxiscript_ir::{parse_module, print::ty_text, Ty as It};

const FMT_U: &str = r#"
func @lw.fmt_u(%m: u64, %neg: bool) -> str {
entry:
  %c0: u64 = const 0
  %ten: u64 = const 10
  %one: i32 = const 1
  %zero: i32 = const 0
  br cnt(%m, %one)
cnt(%t: u64, %k: i32):
  %big: bool = ge %t, %ten
  br_if %big, cnt_more(), cnt_done()
cnt_more:
  %t2: u64 = div %t, %ten
  %k2: i32 = add %k, %one
  br cnt(%t2, %k2)
cnt_done:
  %sgn: i32 = select %neg, %one, %zero
  %len: i32 = add %k, %sgn
  %buf: [u8] = array.new %len
  br fill(%m, %len)
fill(%t3: u64, %i: i32):
  %i2: i32 = sub %i, %one
  %d: u64 = rem %t3, %ten
  %d8: u8 = conv %d
  %c48: u8 = const 48
  %ch: u8 = add %d8, %c48
  array.set %buf, %i2, %ch
  %t4: u64 = div %t3, %ten
  %more: bool = ne %t4, %c0
  br_if %more, fill(%t4, %i2), fin()
fin:
  br_if %neg, putsign(), done()
putsign:
  %mc: u8 = const 45
  array.set %buf, %zero, %mc
  br done()
done:
  %s: str = str.from_utf8 %buf, %len
  release %buf
  ret %s
}
"#;

const ITOA: &str = r#"
func @lw.itoa(%x: i64) -> str {
entry:
  %z: i64 = const 0
  %neg: bool = lt %x, %z
  %nx: i64 = neg.wrap %x
  %ax: i64 = select %neg, %nx, %x
  %m: u64 = conv.wrap %ax
  %s: str = call @lw.fmt_u(%m, %neg)
  ret %s
}
"#;

const UTOA: &str = r#"
func @lw.utoa(%x: u64) -> str {
entry:
  %f: bool = const false
  %s: str = call @lw.fmt_u(%x, %f)
  ret %s
}
"#;

const BOOL_STR: &str = r#"
data @lw_true = "true"
data @lw_false = "false"
func @lw.bool_str(%b: bool) -> str {
entry:
  br_if %b, yes(), no()
yes:
  %t: str = str.const @lw_true
  retain %t
  ret %t
no:
  %f: str = str.const @lw_false
  retain %f
  ret %f
}
"#;

const STR_STARTS: &str = r#"
func @lw.str_starts(%s: str, %p: str) -> bool {
entry:
  %ls: i32 = str.len %s
  %lp: i32 = str.len %p
  %short: bool = lt %ls, %lp
  %zero: i32 = const 0
  %one: i32 = const 1
  br_if %short, no(), loop(%zero)
loop(%i: i32):
  %end: bool = ge %i, %lp
  br_if %end, yes(), cmp()
cmp:
  %a: u8 = str.byte %s, %i
  %b: u8 = str.byte %p, %i
  %eq: bool = eq %a, %b
  %i2: i32 = add %i, %one
  br_if %eq, loop(%i2), no()
yes:
  %t: bool = const true
  ret %t
no:
  %f: bool = const false
  ret %f
}
"#;

const STR_ENDS: &str = r#"
func @lw.str_ends(%s: str, %p: str) -> bool {
entry:
  %ls: i32 = str.len %s
  %lp: i32 = str.len %p
  %short: bool = lt %ls, %lp
  %zero: i32 = const 0
  %one: i32 = const 1
  %off: i32 = sub %ls, %lp
  br_if %short, no(), loop(%zero)
loop(%i: i32):
  %end: bool = ge %i, %lp
  br_if %end, yes(), cmp()
cmp:
  %j: i32 = add %off, %i
  %a: u8 = str.byte %s, %j
  %b: u8 = str.byte %p, %i
  %eq: bool = eq %a, %b
  %i2: i32 = add %i, %one
  br_if %eq, loop(%i2), no()
yes:
  %t: bool = const true
  ret %t
no:
  %f: bool = const false
  ret %f
}
"#;

const STR_INDEX_OF: &str = r#"
func @lw.str_index_of(%s: str, %p: str) -> i32 {
entry:
  %ls: i32 = str.len %s
  %lp: i32 = str.len %p
  %zero: i32 = const 0
  %one: i32 = const 1
  %neg1: i32 = const -1
  %lim: i32 = sub %ls, %lp
  %short: bool = lt %lim, %zero
  br_if %short, none(), outer(%zero)
outer(%i: i32):
  %past: bool = gt %i, %lim
  br_if %past, none(), inner_start()
inner_start:
  br inner(%zero)
inner(%j: i32):
  %jend: bool = ge %j, %lp
  br_if %jend, found(), cmp()
cmp:
  %k: i32 = add %i, %j
  %a: u8 = str.byte %s, %k
  %b: u8 = str.byte %p, %j
  %eq: bool = eq %a, %b
  %j2: i32 = add %j, %one
  br_if %eq, inner(%j2), next()
next:
  %i2: i32 = add %i, %one
  br outer(%i2)
found:
  ret %i
none:
  ret %neg1
}
"#;

const STR_BYTES: &str = r#"
func @lw.str_bytes(%s: str) -> [u8] {
entry:
  %n: i32 = str.len %s
  %zero: i32 = const 0
  %one: i32 = const 1
  %out: [u8] = array.empty
  br loop(%zero)
loop(%i: i32):
  %end: bool = ge %i, %n
  br_if %end, done(), body()
body:
  %b: u8 = str.byte %s, %i
  array.push %out, %b
  %i2: i32 = add %i, %one
  br loop(%i2)
done:
  ret %out
}
"#;

const STR_CHARS: &str = r#"
func @lw.str_chars(%s: str) -> [u32] {
entry:
  %n: i32 = str.len %s
  %zero: i32 = const 0
  %one: i32 = const 1
  %two: i32 = const 2
  %three: i32 = const 3
  %four: i32 = const 4
  %out: [u32] = array.empty
  br loop(%zero)
loop(%i: i32):
  %end: bool = ge %i, %n
  br_if %end, done(), body()
body:
  %b0: u8 = str.byte %s, %i
  %w0: u32 = conv %b0
  %c80: u32 = const 128
  %cE0: u32 = const 224
  %cF0: u32 = const 240
  %is1: bool = lt %w0, %c80
  br_if %is1, one_byte(), not1()
not1:
  %is2: bool = lt %w0, %cE0
  br_if %is2, two_bytes(), not2()
not2:
  %is3: bool = lt %w0, %cF0
  br_if %is3, three_bytes(), four_bytes()
one_byte:
  array.push %out, %w0
  %n1: i32 = add %i, %one
  br loop(%n1)
two_bytes:
  %m1f: u32 = const 31
  %m3f: u32 = const 63
  %s6: u32 = const 6
  %a2: u32 = and %w0, %m1f
  %a2s: u32 = shl %a2, %s6
  %i1: i32 = add %i, %one
  %b1: u8 = str.byte %s, %i1
  %w1: u32 = conv %b1
  %c1: u32 = and %w1, %m3f
  %cp2: u32 = or %a2s, %c1
  array.push %out, %cp2
  %n2: i32 = add %i, %two
  br loop(%n2)
three_bytes:
  %m0f: u32 = const 15
  %m3f3: u32 = const 63
  %s6b: u32 = const 6
  %s12: u32 = const 12
  %a3: u32 = and %w0, %m0f
  %a3s: u32 = shl %a3, %s12
  %j1: i32 = add %i, %one
  %j2: i32 = add %i, %two
  %d1: u8 = str.byte %s, %j1
  %d2: u8 = str.byte %s, %j2
  %e1: u32 = conv %d1
  %e2: u32 = conv %d2
  %f1: u32 = and %e1, %m3f3
  %f2: u32 = and %e2, %m3f3
  %f1s: u32 = shl %f1, %s6b
  %g: u32 = or %a3s, %f1s
  %cp3: u32 = or %g, %f2
  array.push %out, %cp3
  %n3: i32 = add %i, %three
  br loop(%n3)
four_bytes:
  %m07: u32 = const 7
  %m3f4: u32 = const 63
  %s6c: u32 = const 6
  %s12c: u32 = const 12
  %s18: u32 = const 18
  %a4: u32 = and %w0, %m07
  %a4s: u32 = shl %a4, %s18
  %k1: i32 = add %i, %one
  %k2: i32 = add %i, %two
  %k3: i32 = add %i, %three
  %h1: u8 = str.byte %s, %k1
  %h2: u8 = str.byte %s, %k2
  %h3: u8 = str.byte %s, %k3
  %l1: u32 = conv %h1
  %l2: u32 = conv %h2
  %l3: u32 = conv %h3
  %q1: u32 = and %l1, %m3f4
  %q2: u32 = and %l2, %m3f4
  %q3: u32 = and %l3, %m3f4
  %q1s: u32 = shl %q1, %s12c
  %q2s: u32 = shl %q2, %s6c
  %r1: u32 = or %a4s, %q1s
  %r2: u32 = or %r1, %q2s
  %cp4: u32 = or %r2, %q3
  array.push %out, %cp4
  %n4: i32 = add %i, %four
  br loop(%n4)
done:
  ret %out
}
"#;

const RANGE: &str = r#"
func @lw.range(%a: i32, %b: i32) -> [i32] {
entry:
  %zero: i32 = const 0
  %one: i32 = const 1
  %empty: bool = ge %a, %b
  br_if %empty, ret_empty(), alloc()
ret_empty:
  %e: [i32] = array.empty
  ret %e
alloc:
  %len: i32 = sub %b, %a
  %out: [i32] = array.new %len
  br loop(%zero)
loop(%idx: i32):
  %end: bool = ge %idx, %len
  br_if %end, done(), body()
body:
  %val: i32 = add %a, %idx
  array.set %out, %idx, %val
  %idx2: i32 = add.wrap %idx, %one
  br loop(%idx2)
done:
  ret %out
}
"#;

const HASH_U64: &str = r#"
func @lw.hash_u64(%x: u64) -> u64 {
entry:
  %c1: u64 = const 11400714819323198485
  %c2: u64 = const 13787848793156543929
  %c3: u64 = const 10723151780598845931
  %s30: u64 = const 30
  %s27: u64 = const 27
  %s31: u64 = const 31
  %z0: u64 = add.wrap %x, %c1
  %t1: u64 = shr %z0, %s30
  %z1: u64 = xor %z0, %t1
  %z2: u64 = mul.wrap %z1, %c2
  %t2: u64 = shr %z2, %s27
  %z3: u64 = xor %z2, %t2
  %z4: u64 = mul.wrap %z3, %c3
  %t3: u64 = shr %z4, %s31
  %z5: u64 = xor %z4, %t3
  ret %z5
}
"#;

const HASH_STR: &str = r#"
func @lw.hash_str(%s: str) -> u64 {
entry:
  %n: i32 = str.len %s
  %zero: i32 = const 0
  %one: i32 = const 1
  %basis: u64 = const 14695981039346656037
  %prime: u64 = const 1099511628211
  br loop(%zero, %basis)
loop(%i: i32, %h: u64):
  %end: bool = ge %i, %n
  br_if %end, done(), body()
body:
  %b: u8 = str.byte %s, %i
  %b64: u64 = conv %b
  %x: u64 = xor %h, %b64
  %h2: u64 = mul.wrap %x, %prime
  %i2: i32 = add %i, %one
  br loop(%i2, %h2)
done:
  ret %h
}
"#;

const FMOD_F64: &str = r#"
func @lw.fmod_f64(%x: f64, %y: f64) -> f64 {
entry:
  %ux0: u64 = bitcast %x
  %uy0: u64 = bitcast %y
  %sign_mask: u64 = const 9223372036854775808
  %abs_mask: u64 = const 9223372036854775807
  %sx: u64 = and %ux0, %sign_mask
  %ux: u64 = and %ux0, %abs_mask
  %uy: u64 = and %uy0, %abs_mask
  %c0: u64 = const 0
  %inf_bits: u64 = const 9218868437227405312
  %y_zero: bool = eq %uy, %c0
  %x_bad: bool = ge %ux, %inf_bits
  %y_nan: bool = gt %uy, %inf_bits
  %b1: bool = or %y_zero, %x_bad
  %is_nan: bool = or %b1, %y_nan
  br_if %is_nan, ret_nan(), chk_le()
ret_nan:
  %nan_bits: u64 = const 9221120237041090560
  %nan_f: f64 = bitcast %nan_bits
  ret %nan_f
chk_le:
  %le_xy: bool = le %ux, %uy
  br_if %le_xy, small(), norm_x()
small:
  %eq_xy: bool = eq %ux, %uy
  br_if %eq_xy, ret_zero(), ret_x()
ret_zero:
  %zf: f64 = bitcast %sx
  ret %zf
ret_x:
  ret %x
norm_x:
  %c52: u64 = const 52
  %mant_mask: u64 = const 4503599627370495
  %imp_bit: u64 = const 4503599627370496
  %ex_u: u64 = shr.wrap %ux, %c52
  %ex0: i32 = conv.wrap %ex_u
  %i0: i32 = const 0
  %i1: i32 = const 1
  %x_sub: bool = eq %ex0, %i0
  br_if %x_sub, norm_x_sub(), norm_x_norm()
norm_x_sub:
  %lz_x: u64 = clz %ux
  %c11: u64 = const 11
  %sh_x: u64 = sub.wrap %lz_x, %c11
  %mx_s: u64 = shl.wrap %ux, %sh_x
  %sh_xi: i32 = conv.wrap %sh_x
  %ex_s: i32 = sub.wrap %i1, %sh_xi
  br norm_y(%mx_s, %ex_s)
norm_x_norm:
  %mx_low: u64 = and %ux, %mant_mask
  %mx_n: u64 = or %mx_low, %imp_bit
  br norm_y(%mx_n, %ex0)
norm_y(%mx: u64, %ex: i32):
  %ey_u: u64 = shr.wrap %uy, %c52
  %ey0: i32 = conv.wrap %ey_u
  %y_sub: bool = eq %ey0, %i0
  br_if %y_sub, norm_y_sub(), norm_y_norm()
norm_y_sub:
  %lz_y: u64 = clz %uy
  %c11b: u64 = const 11
  %sh_y: u64 = sub.wrap %lz_y, %c11b
  %my_s: u64 = shl.wrap %uy, %sh_y
  %sh_yi: i32 = conv.wrap %sh_y
  %ey_s: i32 = sub.wrap %i1, %sh_yi
  br loop(%mx, %ex, %my_s, %ey_s)
norm_y_norm:
  %my_low: u64 = and %uy, %mant_mask
  %my_n: u64 = or %my_low, %imp_bit
  br loop(%mx, %ex, %my_n, %ey0)
loop(%rm: u64, %re: i32, %my: u64, %ey: i32):
  %more: bool = gt %re, %ey
  br_if %more, step(), after()
step:
  %ge_m: bool = ge %rm, %my
  %diff: u64 = sub.wrap %rm, %my
  %rm1: u64 = select %ge_m, %diff, %rm
  %is_z: bool = eq %rm1, %c0
  br_if %is_z, ret_zero(), step2(%rm1)
step2(%rm2: u64):
  %u1: u64 = const 1
  %rm3: u64 = shl.wrap %rm2, %u1
  %re2: i32 = sub.wrap %re, %i1
  br loop(%rm3, %re2, %my, %ey)
after:
  %ge_last: bool = ge %rm, %my
  %diff_last: u64 = sub.wrap %rm, %my
  %rem_m: u64 = select %ge_last, %diff_last, %rm
  %rem_z: bool = eq %rem_m, %c0
  br_if %rem_z, ret_zero(), pack(%rem_m)
pack(%pm: u64):
  %lz_r: u64 = clz %pm
  %c11c: u64 = const 11
  %sh_r: u64 = sub.wrap %lz_r, %c11c
  %pm2: u64 = shl.wrap %pm, %sh_r
  %sh_ri: i32 = conv.wrap %sh_r
  %pe: i32 = sub.wrap %ey, %sh_ri
  %is_norm: bool = gt %pe, %i0
  br_if %is_norm, pack_norm(%pm2, %pe), pack_sub(%pm2, %pe)
pack_norm(%nm: u64, %ne: i32):
  %ne_u: u64 = conv.wrap %ne
  %exp_bits: u64 = shl.wrap %ne_u, %c52
  %frac: u64 = and %nm, %mant_mask
  %b_a: u64 = or %sx, %exp_bits
  %b_out: u64 = or %b_a, %frac
  %f_out: f64 = bitcast %b_out
  ret %f_out
pack_sub(%sm: u64, %se: i32):
  %rsh_i: i32 = sub.wrap %i1, %se
  %rsh_u: u64 = conv.wrap %rsh_i
  %sub_m: u64 = shr.wrap %sm, %rsh_u
  %sb_out: u64 = or %sx, %sub_m
  %sf_out: f64 = bitcast %sb_out
  ret %sf_out
}
"#;

const FMOD_F32: &str = r#"
func @lw.fmod_f32(%x: f32, %y: f32) -> f32 {
entry:
  %ux0: u32 = bitcast %x
  %uy0: u32 = bitcast %y
  %sign_mask: u32 = const 2147483648
  %abs_mask: u32 = const 2147483647
  %sx: u32 = and %ux0, %sign_mask
  %ux: u32 = and %ux0, %abs_mask
  %uy: u32 = and %uy0, %abs_mask
  %c0: u32 = const 0
  %inf_bits: u32 = const 2139095040
  %y_zero: bool = eq %uy, %c0
  %x_bad: bool = ge %ux, %inf_bits
  %y_nan: bool = gt %uy, %inf_bits
  %b1: bool = or %y_zero, %x_bad
  %is_nan: bool = or %b1, %y_nan
  br_if %is_nan, ret_nan(), chk_le()
ret_nan:
  %nan_bits: u32 = const 2143289344
  %nan_f: f32 = bitcast %nan_bits
  ret %nan_f
chk_le:
  %le_xy: bool = le %ux, %uy
  br_if %le_xy, small(), norm_x()
small:
  %eq_xy: bool = eq %ux, %uy
  br_if %eq_xy, ret_zero(), ret_x()
ret_zero:
  %zf: f32 = bitcast %sx
  ret %zf
ret_x:
  ret %x
norm_x:
  %c23: u32 = const 23
  %mant_mask: u32 = const 8388607
  %imp_bit: u32 = const 8388608
  %ex_u: u32 = shr.wrap %ux, %c23
  %ex0: i32 = conv.wrap %ex_u
  %i0: i32 = const 0
  %i1: i32 = const 1
  %x_sub: bool = eq %ex0, %i0
  br_if %x_sub, norm_x_sub(), norm_x_norm()
norm_x_sub:
  %lz_x: u32 = clz %ux
  %c8: u32 = const 8
  %sh_x: u32 = sub.wrap %lz_x, %c8
  %mx_s: u32 = shl.wrap %ux, %sh_x
  %sh_xi: i32 = conv.wrap %sh_x
  %ex_s: i32 = sub.wrap %i1, %sh_xi
  br norm_y(%mx_s, %ex_s)
norm_x_norm:
  %mx_low: u32 = and %ux, %mant_mask
  %mx_n: u32 = or %mx_low, %imp_bit
  br norm_y(%mx_n, %ex0)
norm_y(%mx: u32, %ex: i32):
  %ey_u: u32 = shr.wrap %uy, %c23
  %ey0: i32 = conv.wrap %ey_u
  %y_sub: bool = eq %ey0, %i0
  br_if %y_sub, norm_y_sub(), norm_y_norm()
norm_y_sub:
  %lz_y: u32 = clz %uy
  %c8b: u32 = const 8
  %sh_y: u32 = sub.wrap %lz_y, %c8b
  %my_s: u32 = shl.wrap %uy, %sh_y
  %sh_yi: i32 = conv.wrap %sh_y
  %ey_s: i32 = sub.wrap %i1, %sh_yi
  br loop(%mx, %ex, %my_s, %ey_s)
norm_y_norm:
  %my_low: u32 = and %uy, %mant_mask
  %my_n: u32 = or %my_low, %imp_bit
  br loop(%mx, %ex, %my_n, %ey0)
loop(%rm: u32, %re: i32, %my: u32, %ey: i32):
  %more: bool = gt %re, %ey
  br_if %more, step(), after()
step:
  %ge_m: bool = ge %rm, %my
  %diff: u32 = sub.wrap %rm, %my
  %rm1: u32 = select %ge_m, %diff, %rm
  %is_z: bool = eq %rm1, %c0
  br_if %is_z, ret_zero(), step2(%rm1)
step2(%rm2: u32):
  %u1: u32 = const 1
  %rm3: u32 = shl.wrap %rm2, %u1
  %re2: i32 = sub.wrap %re, %i1
  br loop(%rm3, %re2, %my, %ey)
after:
  %ge_last: bool = ge %rm, %my
  %diff_last: u32 = sub.wrap %rm, %my
  %rem_m: u32 = select %ge_last, %diff_last, %rm
  %rem_z: bool = eq %rem_m, %c0
  br_if %rem_z, ret_zero(), pack(%rem_m)
pack(%pm: u32):
  %lz_r: u32 = clz %pm
  %c8c: u32 = const 8
  %sh_r: u32 = sub.wrap %lz_r, %c8c
  %pm2: u32 = shl.wrap %pm, %sh_r
  %sh_ri: i32 = conv.wrap %sh_r
  %pe: i32 = sub.wrap %ey, %sh_ri
  %is_norm: bool = gt %pe, %i0
  br_if %is_norm, pack_norm(%pm2, %pe), pack_sub(%pm2, %pe)
pack_norm(%nm: u32, %ne: i32):
  %ne_u: u32 = conv.wrap %ne
  %exp_bits: u32 = shl.wrap %ne_u, %c23
  %frac: u32 = and %nm, %mant_mask
  %b_a: u32 = or %sx, %exp_bits
  %b_out: u32 = or %b_a, %frac
  %f_out: f32 = bitcast %b_out
  ret %f_out
pack_sub(%sm: u32, %se: i32):
  %rsh_i: i32 = sub.wrap %i1, %se
  %rsh_u: u32 = conv.wrap %rsh_i
  %sub_m: u32 = shr.wrap %sm, %rsh_u
  %sb_out: u32 = or %sx, %sub_m
  %sf_out: f32 = bitcast %sb_out
  ret %sf_out
}
"#;

fn rc(t: &It, op: &str, v: &str) -> String {
    if t.is_managed() {
        format!("  {op} {v}\n")
    } else {
        String::new()
    }
}

impl<'c, 'a> Lw<'c, 'a> {
    /// Ensure the helper `name` (and what it needs) is part of the module; returns its symbol.
    pub fn helper(&mut self, name: &str) -> String {
        let (sym, src, deps): (&str, &str, &[&str]) = match name {
            "fmt_u" => ("lw.fmt_u", FMT_U, &[]),
            "itoa" => ("lw.itoa", ITOA, &["fmt_u"]),
            "utoa" => ("lw.utoa", UTOA, &["fmt_u"]),
            "bool_str" => ("lw.bool_str", BOOL_STR, &[]),
            "str_starts" => ("lw.str_starts", STR_STARTS, &[]),
            "str_ends" => ("lw.str_ends", STR_ENDS, &[]),
            "str_index_of" => ("lw.str_index_of", STR_INDEX_OF, &[]),
            "str_bytes" => ("lw.str_bytes", STR_BYTES, &[]),
            "str_chars" => ("lw.str_chars", STR_CHARS, &[]),
            "range" => ("lw.range", RANGE, &[]),
            "hash_u64" => ("lw.hash_u64", HASH_U64, &[]),
            "hash_str" => ("lw.hash_str", HASH_STR, &[]),
            "fmod_f64" => ("lw.fmod_f64", FMOD_F64, &[]),
            "fmod_f32" => ("lw.fmod_f32", FMOD_F32, &[]),
            other => panic!("unknown helper {other}"),
        };
        if self.helper_done.insert(sym.to_string()) {
            for d in deps {
                self.helper(d);
            }
            self.add_text(sym, src);
        }
        sym.to_string()
    }

    pub fn ensure_fmod_f64(&mut self) -> String {
        self.helper("fmod_f64")
    }

    pub fn ensure_fmod_f32(&mut self) -> String {
        self.helper("fmod_f32")
    }

    fn add_text(&mut self, what: &str, src: &str) {
        let text = format!("oxiir 1\nmodule \"helper\"\n{src}");
        match parse_module(&text) {
            Ok(m) => {
                self.out.data.extend(m.data);
                self.out.funcs.extend(m.funcs);
            }
            Err(e) => self.err("OXS-E0702", format!("helper {what}: {e}"), None),
        }
    }

    /// Array helper `kind` for element type `t` (and result element `u` for map/reduce).
    pub fn array_helper(&mut self, kind: &str, t: &It, u: Option<&It>) -> String {
        let tt = ty_text(t);
        let tm = mangle(t);
        let um = u.map(mangle).unwrap_or_default();
        let sym = format!("lw.arr_{kind}.{tm}.{um}");
        if !self.helper_done.insert(sym.clone()) {
            return sym;
        }
        let ut = u.map(ty_text).unwrap_or_default();
        let get_ret = rc(t, "retain", "%e");
        let rel_e = rc(t, "release", "%e");
        let get_ret2 = rc(t, "retain", "%e2");
        let src = match kind {
            "foreach" => format!(
                r#"
func @{sym}(%a: [{tt}], %f: fn({tt}) -> void) -> void {{
entry:
  %n: i32 = array.len %a
  %zero: i32 = const 0
  %one: i32 = const 1
  br loop(%zero)
loop(%i: i32):
  %end: bool = ge %i, %n
  br_if %end, done(), body()
body:
  %e: {tt} = array.get %a, %i
{get_ret}  call_closure %f(%e)
{rel_e}  %i2: i32 = add %i, %one
  br loop(%i2)
done:
  ret
}}
"#
            ),
            "map" => {
                let ur = u.cloned().unwrap_or(It::I32);
                let rel_r = rc(&ur, "retain", "%r");
                let _ = rel_r;
                format!(
                    r#"
func @{sym}(%a: [{tt}], %f: fn({tt}) -> {ut}) -> [{ut}] {{
entry:
  %n: i32 = array.len %a
  %zero: i32 = const 0
  %one: i32 = const 1
  %out: [{ut}] = array.empty
  br loop(%zero)
loop(%i: i32):
  %end: bool = ge %i, %n
  br_if %end, done(), body()
body:
  %e: {tt} = array.get %a, %i
{get_ret}  %r: {ut} = call_closure %f(%e)
{rel_e}  array.push %out, %r
  %i2: i32 = add %i, %one
  br loop(%i2)
done:
  ret %out
}}
"#
                )
            }
            "filter" => format!(
                r#"
func @{sym}(%a: [{tt}], %f: fn({tt}) -> bool) -> [{tt}] {{
entry:
  %n: i32 = array.len %a
  %zero: i32 = const 0
  %one: i32 = const 1
  %out: [{tt}] = array.empty
  br loop(%zero)
loop(%i: i32):
  %end: bool = ge %i, %n
  br_if %end, done(), body()
body:
  %e: {tt} = array.get %a, %i
{get_ret}  %k: bool = call_closure %f(%e)
  br_if %k, keep(), skip()
keep:
  array.push %out, %e
  %i2: i32 = add %i, %one
  br loop(%i2)
skip:
{rel_e}  %i3: i32 = add %i, %one
  br loop(%i3)
done:
  ret %out
}}
"#
            ),
            "reduce" => {
                let ur = u.cloned().unwrap_or(It::I32);
                let init_rc = rc(&ur, "retain", "%init");
                let rel_acc = rc(&ur, "release", "%acc");
                format!(
                    r#"
func @{sym}(%a: [{tt}], %f: fn({ut}, {tt}) -> {ut}, %init: {ut}) -> {ut} {{
entry:
  %n: i32 = array.len %a
  %zero: i32 = const 0
  %one: i32 = const 1
{init_rc}  br loop(%zero, %init)
loop(%i: i32, %acc: {ut}):
  %end: bool = ge %i, %n
  br_if %end, done(), body()
body:
  %e: {tt} = array.get %a, %i
{get_ret}  %acc2: {ut} = call_closure %f(%acc, %e)
{rel_e}{rel_acc}  %i2: i32 = add %i, %one
  br loop(%i2, %acc2)
done:
  ret %acc
}}
"#
                )
            }
            "some" | "every" => {
                let (stop_when, dflt) = if kind == "some" { ("true", "false") } else { ("false", "true") };
                format!(
                    r#"
func @{sym}(%a: [{tt}], %f: fn({tt}) -> bool) -> bool {{
entry:
  %n: i32 = array.len %a
  %zero: i32 = const 0
  %one: i32 = const 1
  br loop(%zero)
loop(%i: i32):
  %end: bool = ge %i, %n
  br_if %end, done(), body()
body:
  %e: {tt} = array.get %a, %i
{get_ret}  %k: bool = call_closure %f(%e)
{rel_e}  %stop: bool = const {stop_when}
  %hit: bool = eq %k, %stop
  %i2: i32 = add %i, %one
  br_if %hit, found(), loop(%i2)
found:
  %r: bool = const {stop_when}
  ret %r
done:
  %d: bool = const {dflt}
  ret %d
}}
"#
                )
            }
            "slice" => if !t.is_managed() {
                format!(
                    r#"
func @{sym}(%a: [{tt}], %lo: i32, %hi: i32) -> [{tt}] {{
entry:
  %n: i32 = array.len %a
  %zero: i32 = const 0
  %one: i32 = const 1
  %bad1: bool = lt %lo, %zero
  %bad2: bool = gt %hi, %n
  %bad3: bool = gt %lo, %hi
  %b12: bool = or %bad1, %bad2
  %bad: bool = or %b12, %bad3
  br_if %bad, oob(), start()
oob:
  trap bounds
start:
  %len: i32 = sub %hi, %lo
  %out: [{tt}] = array.new %len
  br loop(%zero)
loop(%k: i32):
  %end: bool = ge %k, %len
  br_if %end, done(), body()
body:
  %src_i: i32 = add.wrap %lo, %k
  %e: {tt} = array.get %a, %src_i
  array.set %out, %k, %e
  %k2: i32 = add.wrap %k, %one
  br loop(%k2)
done:
  ret %out
}}
"#
                )
            } else {
                format!(
                    r#"
func @{sym}(%a: [{tt}], %lo: i32, %hi: i32) -> [{tt}] {{
entry:
  %n: i32 = array.len %a
  %zero: i32 = const 0
  %one: i32 = const 1
  %bad1: bool = lt %lo, %zero
  %bad2: bool = gt %hi, %n
  %bad3: bool = gt %lo, %hi
  %b12: bool = or %bad1, %bad2
  %bad: bool = or %b12, %bad3
  br_if %bad, oob(), start()
oob:
  trap bounds
start:
  %out: [{tt}] = array.empty
  br loop(%lo)
loop(%i: i32):
  %end: bool = ge %i, %hi
  br_if %end, done(), body()
body:
  %e: {tt} = array.get %a, %i
{get_ret}  array.push %out, %e
  %i2: i32 = add %i, %one
  br loop(%i2)
done:
  ret %out
}}
"#
                )
            },
            "concat" => format!(
                r#"
func @{sym}(%a: [{tt}], %b: [{tt}]) -> [{tt}] {{
entry:
  %n: i32 = array.len %a
  %m: i32 = array.len %b
  %zero: i32 = const 0
  %one: i32 = const 1
  %out: [{tt}] = array.empty
  br l1(%zero)
l1(%i: i32):
  %end: bool = ge %i, %n
  br_if %end, mid(), b1()
b1:
  %e: {tt} = array.get %a, %i
{get_ret}  array.push %out, %e
  %i2: i32 = add %i, %one
  br l1(%i2)
mid:
  br l2(%zero)
l2(%j: i32):
  %end2: bool = ge %j, %m
  br_if %end2, done(), b2()
b2:
  %e2: {tt} = array.get %b, %j
{get_ret2}  array.push %out, %e2
  %j2: i32 = add %j, %one
  br l2(%j2)
done:
  ret %out
}}
"#
            ),
            _ => String::new(),
        };
        self.add_text(&sym, &src);
        sym
    }
}
