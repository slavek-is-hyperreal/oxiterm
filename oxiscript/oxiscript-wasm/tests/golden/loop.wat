(module
  (type (;0;) (func (param i32) (result i32)))
  (type (;1;) (func (param i32 i32)))
  (type (;2;) (func (param i32)))
  (type (;3;) (func (result i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (type (;5;) (func (param i32 i32 i32) (result i32)))
  (type (;6;) (func (param i64) (result i32)))
  (type (;7;) (func (param f64) (result i32)))
  (type (;8;) (func (param i32) (result i64)))
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
  (export "oxi_init" (func 23))
  (export "oxi_deinit" (func 24))
  (export "oxi_tick" (func 25))
  (export "sum" (func 21))
  (export "classify" (func 22))
  (func (;21;) (type 8) (param i32) (result i64)
    (local i32 i32 i32 i32 i32 i64 i64 i64 i64 i64)
    i32.const 0
    local.set 1
    i64.const 0
    local.set 6
    local.get 1
    local.get 6
    local.set 7
    local.set 2
    loop ;; label = @1
      local.get 2
      local.get 0
      i32.ge_s
      local.set 3
      local.get 3
      if ;; label = @2
        local.get 7
        local.set 10
        local.get 10
        return
      end
      local.get 2
      i64.extend_i32_s
      local.set 8
      local.get 7
      local.get 8
      i64.add
      local.set 9
      local.get 7
      local.get 9
      i64.xor
      local.get 8
      local.get 9
      i64.xor
      i64.and
      i64.const 0
      i64.lt_s
      if ;; label = @2
        i32.const 2
        global.set 0
        unreachable
      end
      i32.const 1
      local.set 4
      local.get 2
      local.get 4
      i32.add
      local.set 5
      local.get 2
      local.get 5
      i32.xor
      local.get 4
      local.get 5
      i32.xor
      i32.and
      i32.const 0
      i32.lt_s
      if ;; label = @2
        i32.const 2
        global.set 0
        unreachable
      end
      local.get 5
      local.get 9
      local.set 7
      local.set 2
      br 0 (;@1;)
    end
    unreachable
  )
  (func (;22;) (type 0) (param i32) (result i32)
    (local i32 i32 i32 i32)
    block ;; label = @1
      block ;; label = @2
        block ;; label = @3
          block ;; label = @4
            local.get 0
            i32.const -1
            i32.sub
            br_table 1 (;@3;) 3 (;@1;) 2 (;@2;) 0 (;@4;)
          end
          i32.const 0
          local.set 4
          local.get 4
          return
        end
        i32.const 102
        local.set 3
        local.get 3
        return
      end
      i32.const 101
      local.set 2
      local.get 2
      return
    end
    i32.const 100
    local.set 1
    local.get 1
    return
    unreachable
  )
  (func (;23;) (type 9)
    global.get 2
    global.get 3
    call 20
  )
  (func (;24;) (type 9))
  (func (;25;) (type 10) (param i64) (result i64)
    i64.const -1
  )
  (data (;0;) (i32.const 1024) "\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\04\01\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00")
  (@custom "oxi.manifest" (after data) "oxi-manifest 1\0aabi 1.0\0aedition 2026\0aprofile default\0a")
)
