(module
  (type (;0;) (func (param i32) (result i32)))
  (type (;1;) (func (param i32 i32)))
  (type (;2;) (func (param i32)))
  (type (;3;) (func (result i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (type (;5;) (func (param i32 i32 i32) (result i32)))
  (type (;6;) (func (param i64) (result i32)))
  (type (;7;) (func (param f64) (result i32)))
  (type (;8;) (func (param i32 i32) (result i32 i32)))
  (type (;9;) (func (param i32 f32) (result i32)))
  (type (;10;) (func (result f64)))
  (type (;11;) (func))
  (type (;12;) (func (param i64) (result i64)))
  (import "oxi:rt" "oxi_alloc" (func (;0;) (type 0) (param i32) (result i32)))
  (import "oxi:rt" "oxi_alloc_bytes" (func (;1;) (type 0) (param i32) (result i32)))
  (import "oxi:rt" "oxi_free_bytes" (func (;2;) (type 1) (param i32 i32)))
  (import "oxi:rt" "oxi_retain" (func (;3;) (type 2) (param i32)))
  (import "oxi:rt" "oxi_release" (func (;4;) (type 2) (param i32)))
  (import "oxi:rt" "oxi_clone" (func (;5;) (type 0) (param i32) (result i32)))
  (import "oxi:rt" "oxi_is_unique" (func (;6;) (type 0) (param i32) (result i32)))
  (import "oxi:rt" "oxi_collect_cycles" (func (;7;) (type 3) (result i32)))
  (import "oxi:rt" "oxi_live_bytes" (func (;8;) (type 3) (result i32)))
  (import "oxi:rt" "oxi_live_objects" (func (;9;) (type 3) (result i32)))
  (import "oxi:rt" "oxi_str_new" (func (;10;) (type 4) (param i32 i32) (result i32)))
  (import "oxi:rt" "oxi_str_concat" (func (;11;) (type 4) (param i32 i32) (result i32)))
  (import "oxi:rt" "oxi_str_eq" (func (;12;) (type 4) (param i32 i32) (result i32)))
  (import "oxi:rt" "oxi_str_cmp" (func (;13;) (type 4) (param i32 i32) (result i32)))
  (import "oxi:rt" "oxi_str_slice" (func (;14;) (type 5) (param i32 i32 i32) (result i32)))
  (import "oxi:rt" "oxi_str_from_i64" (func (;15;) (type 6) (param i64) (result i32)))
  (import "oxi:rt" "oxi_str_from_f64" (func (;16;) (type 7) (param f64) (result i32)))
  (import "oxi:rt" "oxi_array_new" (func (;17;) (type 4) (param i32 i32) (result i32)))
  (import "oxi:rt" "oxi_array_grow" (func (;18;) (type 1) (param i32 i32)))
  (import "oxi:rt" "oxi_trap" (func (;19;) (type 2) (param i32)))
  (import "oxi:rt" "oxi_rt_init" (func (;20;) (type 1) (param i32 i32)))
  (table (;0;) 0 0 funcref)
  (memory (;0;) 2)
  (global (;0;) (mut i32) i32.const 0)
  (global (;1;) i32 i32.const 65536)
  (global (;2;) i32 i32.const 1024)
  (global (;3;) i32 i32.const 16)
  (global (;4;) i32 i32.const 1280)
  (global (;5;) i32 i32.const 4096)
  (global (;6;) (mut i32) i32.const 5376)
  (global (;7;) (mut i32) i32.const 0)
  (global (;8;) (mut i32) i32.const 0)
  (export "memory" (memory 0))
  (export "oxi_abi_version" (global 1))
  (export "oxi_trap_kind" (global 0))
  (export "oxi_typedesc_base" (global 2))
  (export "oxi_typedesc_count" (global 3))
  (export "oxi_scratch_cap" (global 5))
  (export "oxi_init" (func 26))
  (export "oxi_deinit" (func 27))
  (export "oxi_tick" (func 28))
  (export "add_wrap" (func 21))
  (export "checked" (func 22))
  (export "saturate" (func 23))
  (export "bits" (func 24))
  (export "floats" (func 29))
  (func (;21;) (type 4) (param i32 i32) (result i32)
    (local i32)
    local.get 0
    local.get 1
    i32.add
    local.set 2
    local.get 2
    return
    unreachable
  )
  (func (;22;) (type 8) (param i32 i32) (result i32 i32)
    (local i32 i32 i32 i32 i32 i32 i32 i64)
    local.get 0
    local.get 1
    i32.add
    local.set 2
    local.get 0
    local.get 2
    i32.xor
    local.get 1
    local.get 2
    i32.xor
    i32.and
    i32.const 0
    i32.lt_s
    if ;; label = @1
      i32.const 2
      global.set 0
      unreachable
    end
    local.get 2
    i64.extend_i32_s
    local.get 1
    i64.extend_i32_s
    i64.mul
    local.tee 9
    i32.wrap_i64
    local.set 3
    local.get 3
    i64.extend_i32_s
    local.get 9
    i64.ne
    if ;; label = @1
      i32.const 2
      global.set 0
      unreachable
    end
    i32.const 0
    local.set 4
    local.get 1
    local.get 4
    i32.eq
    local.set 5
    local.get 5
    if ;; label = @1
      i32.const 3
      global.set 0
      unreachable
    end
    local.get 3
    local.set 6
    local.get 1
    i32.eqz
    if ;; label = @1
      i32.const 3
      global.set 0
      unreachable
    end
    local.get 6
    i32.const -2147483648
    i32.eq
    local.get 1
    i32.const -1
    i32.eq
    i32.and
    if ;; label = @1
      i32.const 2
      global.set 0
      unreachable
    end
    local.get 6
    local.get 1
    i32.div_s
    local.set 7
    local.get 1
    i32.eqz
    if ;; label = @1
      i32.const 3
      global.set 0
      unreachable
    end
    local.get 7
    local.get 1
    i32.rem_s
    local.set 8
    local.get 7
    local.get 8
    return
    unreachable
  )
  (func (;23;) (type 6) (param i64) (result i32)
    (local i32 i64)
    local.get 0
    local.set 2
    local.get 2
    i64.const 0
    local.get 2
    i64.const 0
    i64.ge_s
    select
    local.set 2
    local.get 2
    i64.const 255
    local.get 2
    i64.const 255
    i64.le_s
    select
    local.set 2
    local.get 2
    i32.wrap_i64
    local.set 1
    local.get 1
    return
    unreachable
  )
  (func (;24;) (type 9) (param i32 f32) (result i32)
    (local i32 i32 i32 i32 i32 i32)
    local.get 0
    i32.popcnt
    local.set 2
    i32.const 3
    local.set 3
    local.get 0
    local.get 3
    i32.shl
    local.set 4
    local.get 2
    local.get 4
    i32.or
    local.set 5
    local.get 1
    f32.const nan (;=NaN;)
    local.get 1
    local.get 1
    f32.eq
    select
    i32.reinterpret_f32
    local.set 6
    local.get 5
    local.get 6
    i32.xor
    local.set 7
    local.get 7
    return
    unreachable
  )
  (func (;25;) (type 10) (result f64)
    (local f32 f32 f64 f64 f64 f64 f64 f64 f64 f64 f64 f64)
    f64.const -0x0p+0 (;=-0;)
    local.set 2
    f64.const nan:0x8000000000001 (;=NaN;)
    local.set 3
    f64.const inf (;=inf;)
    local.set 4
    f64.const -inf (;=-inf;)
    local.set 5
    f64.const 0x1.p-1074 (;=0.000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000005;)
    local.set 6
    f64.const 0x1.fffffffffffffp+1023 (;=179769313486231570000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000;)
    local.set 7
    f32.const 0x1.p-149 (;=0.000000000000000000000000000000000000000000001;)
    local.set 0
    f32.const -nan:0x400001 (;=NaN;)
    local.set 1
    f64.const 0x1.999999999999ap-4 (;=0.1;)
    local.set 8
    local.get 2
    local.get 6
    f64.add
    local.set 9
    local.get 9
    local.get 8
    f64.min
    local.set 10
    local.get 10
    f64.sqrt
    local.set 11
    local.get 11
    return
    unreachable
  )
  (func (;26;) (type 11)
    global.get 2
    global.get 3
    call 20
  )
  (func (;27;) (type 11))
  (func (;28;) (type 12) (param i64) (result i64)
    i64.const -1
  )
  (func (;29;) (type 10) (result f64)
    (local f64)
    call 25
    local.set 0
    local.get 0
    f64.const nan (;=NaN;)
    local.get 0
    local.get 0
    f64.eq
    select
  )
  (data (;0;) (i32.const 1024) "\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\04\01\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00")
  (@custom "oxi.manifest" (after data) "oxi-manifest 1\0aabi 1.0\0aedition 2026\0aprofile default\0a")
)
