(module
  (type (;0;) (func (param i32) (result i32)))
  (type (;1;) (func (param i32 i32)))
  (type (;2;) (func (param i32)))
  (type (;3;) (func (result i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (type (;5;) (func (param i32 i32 i32) (result i32)))
  (type (;6;) (func (param i64) (result i32)))
  (type (;7;) (func (param f64) (result i32)))
  (type (;8;) (func (param i32 i32 i32)))
  (type (;9;) (func))
  (type (;10;) (func (param i64) (result i64)))
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
  (import "oxi:rt" "oxi_rt_init" (func (;20;) (type 8) (param i32 i32 i32)))
  (table (;0;) 2 2 funcref)
  (memory (;0;) 2)
  (global (;0;) (mut i32) i32.const 0)
  (global (;1;) i32 i32.const 65536)
  (global (;2;) i32 i32.const 1040)
  (global (;3;) i32 i32.const 18)
  (global (;4;) i32 i32.const 1328)
  (global (;5;) i32 i32.const 4096)
  (global (;6;) (mut i32) i32.const 5424)
  (global (;7;) (mut i32) i32.const 0)
  (global (;8;) (mut i32) i32.const 0)
  (export "memory" (memory 0))
  (export "oxi_abi_version" (global 1))
  (export "oxi_trap_kind" (global 0))
  (export "oxi_typedesc_base" (global 2))
  (export "oxi_typedesc_count" (global 3))
  (export "oxi_scratch_cap" (global 5))
  (export "oxi_init" (func 27))
  (export "oxi_deinit" (func 28))
  (export "oxi_tick" (func 29))
  (export "make_adder" (func 22))
  (export "apply" (func 23))
  (export "compose" (func 25))
  (export "maybe_call" (func 26))
  (elem (;0;) (i32.const 0) func 21 24)
  (func (;21;) (type 4) (param i32 i32) (result i32)
    (local i32 i32)
    local.get 0
    i32.load offset=8
    local.set 2
    local.get 1
    local.get 2
    i32.add
    local.set 3
    local.get 1
    local.get 3
    i32.xor
    local.get 2
    local.get 3
    i32.xor
    i32.and
    i32.const 0
    i32.lt_s
    if ;; label = @1
      i32.const 2
      global.set 0
      unreachable
    end
    local.get 3
    return
    unreachable
  )
  (func (;22;) (type 0) (param i32) (result i32)
    (local i32)
    i32.const 16
    call 0
    local.set 1
    local.get 1
    i32.const 0
    i32.store
    local.get 1
    local.get 0
    i32.store offset=8
    local.get 1
    return
    unreachable
  )
  (func (;23;) (type 4) (param i32 i32) (result i32)
    (local i32)
    local.get 0
    local.get 1
    local.get 0
    i32.load
    call_indirect (type 4)
    local.set 2
    local.get 2
    return
    unreachable
  )
  (func (;24;) (type 4) (param i32 i32) (result i32)
    (local i32 i32 i32 i32)
    local.get 0
    i32.load offset=8
    local.set 2
    local.get 0
    i32.load offset=12
    local.set 3
    local.get 3
    local.get 1
    local.get 3
    i32.load
    call_indirect (type 4)
    local.set 4
    local.get 2
    local.get 4
    local.get 2
    i32.load
    call_indirect (type 4)
    local.set 5
    local.get 5
    return
    unreachable
  )
  (func (;25;) (type 4) (param i32 i32) (result i32)
    (local i32)
    i32.const 17
    call 0
    local.set 2
    local.get 2
    i32.const 1
    i32.store
    local.get 2
    local.get 0
    i32.store offset=8
    local.get 2
    local.get 1
    i32.store offset=12
    local.get 2
    return
    unreachable
  )
  (func (;26;) (type 4) (param i32 i32) (result i32)
    (local i32 i32 i32)
    local.get 0
    i32.eqz
    local.set 2
    local.get 2
    if ;; label = @1
      local.get 1
      return
    end
    local.get 0
    i32.eqz
    if ;; label = @1
      i32.const 6
      global.set 0
      unreachable
    end
    local.get 0
    local.set 3
    local.get 3
    local.get 1
    local.get 3
    i32.load
    call_indirect (type 4)
    local.set 4
    local.get 4
    return
    unreachable
  )
  (func (;27;) (type 9)
    global.get 2
    global.get 3
    i32.const 1328
    call 20
  )
  (func (;28;) (type 9))
  (func (;29;) (type 10) (param i64) (result i64)
    i64.const -1
  )
  (data (;0;) (i32.const 1024) "\08\00\00\00\0c\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\04\01\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\10\00\00\00\03\01\00\00\00\00\00\00\00\00\00\00\10\00\00\00\03\00\02\00\00\04\00\00\00\00\00\00")
  (@custom "oxi.manifest" (after data) "oxi-manifest 1\0aabi 1.0\0aedition 2026\0aprofile default\0a")
)
