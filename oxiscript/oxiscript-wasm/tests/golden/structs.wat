(module
  (type (;0;) (func (param i32) (result i32)))
  (type (;1;) (func (param i32 i32)))
  (type (;2;) (func (param i32)))
  (type (;3;) (func (result i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (type (;5;) (func (param i32 i32 i32) (result i32)))
  (type (;6;) (func (param i64) (result i32)))
  (type (;7;) (func (param f64) (result i32)))
  (type (;8;) (func (param f64 f64) (result i32)))
  (type (;9;) (func (param i32) (result f64)))
  (type (;10;) (func (param i32 f64) (result i32)))
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
  (global (;2;) i32 i32.const 1040)
  (global (;3;) i32 i32.const 21)
  (global (;4;) i32 i32.const 1376)
  (global (;5;) i32 i32.const 4096)
  (global (;6;) (mut i32) i32.const 5472)
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
  (export "make" (func 21))
  (export "norm2" (func 22))
  (export "move_x" (func 23))
  (export "area" (func 24))
  (export "new_rect" (func 25))
  (func (;21;) (type 8) (param f64 f64) (result i32)
    (local i32)
    i32.const 16
    call 0
    local.set 2
    local.get 2
    local.get 0
    f64.store
    local.get 2
    local.get 1
    f64.store offset=8
    local.get 2
    return
    unreachable
  )
  (func (;22;) (type 9) (param i32) (result f64)
    (local f64 f64 f64 f64 f64)
    local.get 0
    f64.load
    local.set 1
    local.get 0
    f64.load offset=8
    local.set 2
    local.get 1
    local.get 1
    f64.mul
    local.set 3
    local.get 2
    local.get 2
    f64.mul
    local.set 4
    local.get 3
    local.get 4
    f64.add
    local.set 5
    local.get 5
    return
    unreachable
  )
  (func (;23;) (type 10) (param i32 f64) (result i32)
    (local i32 i32 i32 f64 f64)
    block ;; label = @1
      local.get 0
      call 6
      local.set 2
      local.get 2
      if ;; label = @2
        local.get 0
        local.set 4
        br 1 (;@1;)
      end
      local.get 0
      call 5
      local.set 3
      local.get 0
      call 4
      local.get 3
      local.set 4
    end
    local.get 4
    f64.load
    local.set 5
    local.get 5
    local.get 1
    f64.add
    local.set 6
    local.get 4
    local.get 6
    f64.store
    local.get 4
    return
    unreachable
  )
  (func (;24;) (type 9) (param i32) (result f64)
    (local i32 f64 f64 f64 f64 f64 f64)
    block ;; label = @1
      block ;; label = @2
        block ;; label = @3
          block ;; label = @4
            local.get 0
            i32.load
            local.set 1
            local.get 1
            br_table 3 (;@1;) 2 (;@2;) 1 (;@3;) 0 (;@4;)
          end
          i32.const 1
          global.set 0
          unreachable
        end
        f64.const 0x0p+0 (;=0;)
        local.set 7
        local.get 7
        return
      end
      local.get 0
      i32.load
      i32.const 1
      i32.ne
      if ;; label = @2
        i32.const 5
        global.set 0
        unreachable
      end
      local.get 0
      f64.load offset=8
      local.set 4
      local.get 0
      i32.load
      i32.const 1
      i32.ne
      if ;; label = @2
        i32.const 5
        global.set 0
        unreachable
      end
      local.get 0
      f64.load offset=16
      local.set 5
      local.get 4
      local.get 5
      f64.mul
      local.set 6
      local.get 6
      return
    end
    local.get 0
    i32.load
    i32.const 0
    i32.ne
    if ;; label = @1
      i32.const 5
      global.set 0
      unreachable
    end
    local.get 0
    f64.load offset=8
    local.set 2
    local.get 2
    local.get 2
    f64.mul
    local.set 3
    local.get 3
    return
    unreachable
  )
  (func (;25;) (type 8) (param f64 f64) (result i32)
    (local i32)
    i32.const 18
    call 0
    local.set 2
    local.get 2
    i32.const 1
    i32.store
    local.get 2
    local.get 0
    f64.store offset=8
    local.get 2
    local.get 1
    f64.store offset=16
    local.get 2
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
  (data (;0;) (i32.const 1024) "\04\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\04\01\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\10\00\00\00\01\01\00\00\00\00\00\00\00\00\00\00\10\00\00\00\02\01\00\00\00\00\00\00\00\00\00\00\18\00\00\00\02\01\00\00\00\00\00\00\00\00\00\00\08\00\00\00\02\01\00\00\00\00\00\00\00\00\00\00\08\00\00\00\01\00\01\00\00\04\00\00\00\00\00\00")
  (@custom "oxi.manifest" (after data) "oxi-manifest 1\0aabi 1.0\0aedition 2026\0aprofile default\0a")
)
