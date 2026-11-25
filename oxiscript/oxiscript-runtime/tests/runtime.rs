#![allow(clippy::pedantic)]
mod common;

use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_runtime::caps::state::StateValue;
use oxiscript_runtime::{Error, Grants, Limits, TrapCause, Value};

fn trap_of(e: Error) -> oxiscript_runtime::TrapInfo {
    match e {
        Error::Trap(t) => t,
        other => panic!("expected trap, got {other:?}"),
    }
}

fn code_of(r: Result<oxiscript_runtime::Runtime, Error>) -> String {
    match r {
        Ok(_) => panic!("module was accepted"),
        Err(e) => e.code().to_string(),
    }
}

fn load(e: &Env, f: &Fixture) -> Result<oxiscript_runtime::Runtime, Error> {
    e.host.instantiate(&f.bytes(), Grants::all())
}

// ---- happy path -------------------------------------------------------

#[test]
fn valid_module_calls_log_and_state() {
    let e = env();
    let mut rt = quick(&app(), &e);
    let lines = e.sink.lines();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].msg, "hello");
    assert_eq!(lines[0].level, 2);
    // state: bump from the guest, read from the host
    assert_eq!(rt.call_export("bump", &[]).unwrap(), vec![Value::I64(1)]);
    assert_eq!(rt.call_export("bump", &[]).unwrap(), vec![Value::I64(2)]);
    assert_eq!(
        e.store.get_in(rt.namespace(), "k"),
        Some(StateValue::I64(2))
    );
    // host writes, guest sees
    e.store
        .set_in(rt.namespace(), "k", StateValue::I64(40))
        .unwrap();
    assert_eq!(rt.call_export("bump", &[]).unwrap(), vec![Value::I64(41)]);
    assert!(rt.last_fuel_used() > 0);
    assert!(rt.host_calls() >= 4);
}

#[test]
fn str_payload_goes_through_scratch() {
    let e = env();
    let mut rt = quick(&app(), &e);
    // The export takes (ptr, len): the host supplies both from one Str.
    let r = rt
        .call_export("echo_str", &[Value::Str("abc".into())])
        .unwrap();
    assert_eq!(r, vec![Value::I32(3 + 97)]);
    // two payloads are refused, without poisoning
    let err = rt
        .call_export(
            "echo_str",
            &[Value::Str("a".into()), Value::Str("b".into())],
        )
        .unwrap_err();
    assert!(matches!(err, Error::BadCall(_)));
    assert!(!rt.is_poisoned());
    // payload above HOST_PAYLOAD_MAX
    let big = "x".repeat(5000);
    assert!(matches!(
        rt.call_export("echo_str", &[Value::Str(big)]),
        Err(Error::BadCall(_))
    ));
    // wrong arity / types / unknown / reserved
    assert!(matches!(
        rt.call_export("bump", &[Value::I32(1)]),
        Err(Error::BadCall(_))
    ));
    assert!(matches!(
        rt.call_export("nope", &[]),
        Err(Error::BadCall(_))
    ));
    assert!(matches!(
        rt.call_export("oxi_init", &[]),
        Err(Error::BadCall(_))
    ));
    assert!(matches!(
        rt.call_export("memory", &[]),
        Err(Error::BadCall(_))
    ));
    assert!(!rt.is_poisoned());
    assert!(rt.user_exports().contains(&"bump".to_string()));
}

// ---- load-time rejection ---------------------------------------------

#[test]
fn rejects_missing_and_mistyped_exports() {
    let e = env();
    for n in [
        "oxi_tick",
        "oxi_alloc",
        "oxi_release",
        "oxi_init",
        "oxi_deinit",
        "oxi_trap",
        "oxi_str_new",
    ] {
        let f = app().skip(n);
        assert_eq!(code_of(load(&e, &f)), "OXS-E0406", "missing {n}");
    }
    // wrong signature
    let f = app().over(
        "oxi_tick",
        r#"(func (export "oxi_tick") (param i32) (result i64) (i64.const 0))"#,
    );
    assert_eq!(code_of(load(&e, &f)), "OXS-E0406");
    // oxi_init arity must equal the number of needs
    let f = app().over("oxi_init", r#"(func (export "oxi_init"))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0406");
    // mutable scratch pointer
    let mut f = app();
    let wat = f.wat().replace(
        r#"(global (export "oxi_scratch_ptr") i32"#,
        r#"(global (export "oxi_scratch_ptr") (mut i32)"#,
    );
    let mut bytes = wat::parse_str(&wat).unwrap();
    append_custom(&mut bytes, "oxi.manifest", f.manifest_text().as_bytes());
    f.manifest = None;
    assert_eq!(
        code_of(e.host.instantiate(&bytes, Grants::all())),
        "OXS-E0406"
    );
    // trap_kind must be mutable
    let wat = app().wat().replace(
        r#"(global $tk (export "oxi_trap_kind") (mut i32)"#,
        r#"(global $tk (export "oxi_trap_kind") i32"#,
    );
    let wat = wat.replace("(global.set $tk (local.get 0)) unreachable", "unreachable");
    let mut bytes = wat::parse_str(&wat).unwrap();
    append_custom(&mut bytes, "oxi.manifest", app().manifest_text().as_bytes());
    assert_eq!(
        code_of(e.host.instantiate(&bytes, Grants::all())),
        "OXS-E0406"
    );
}

#[test]
fn rejects_stray_exports() {
    let e = env();
    // unknown reserved name
    let f = app().body(r#"(func (export "oxi_backdoor") nop)"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0407");
    // callback export that no needed interface declares
    let f = app_no_cb().body(r#"(func (export "cb:host:state@1.State.Nope") (param i32) nop)"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0407");
    // callback with the wrong signature
    let f = app_no_cb().body(r#"(func (export "cb:host:state@1.State.OnChange") (param i32) nop)"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0407");
    // user export that is not a function
    let f = app().body(r#"(global (export "g") i32 (i32.const 1))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0407");
    let f = app().body(r#"(table (export "t") 1 funcref)"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0407");
    // a needed callback must be exported when an import takes it
    let f = app_no_cb();
    assert_eq!(code_of(load(&e, &f)), "OXS-E0406"); // `watch` is imported, no cb export
}

#[test]
fn rejects_bad_imports() {
    let e = env();
    // extra import not in the manifest
    let f = Fixture::new(&[LOG])
        .imports(r#"(import "host:state@1" "open" (func (param i32 i32 i32) (result i32 i32)))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0400");
    // import of a version the host does not provide, with a matching need
    let f = Fixture::new(&["host:log@2 Log"])
        .imports(r#"(import "host:log@2" "write" (func (param i32 i32 i32 i32)))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0400");
    // WASI / env
    let f = Fixture::new(&[LOG])
        .imports(r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0400");
    // unknown function in a granted interface
    let f = Fixture::new(&[LOG]).imports(r#"(import "host:log@1" "flush" (func (param i32)))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0401");
    // forged shape: log.write with the typed pair replaced by i64s
    let f =
        Fixture::new(&[LOG]).imports(r#"(import "host:log@1" "write" (func (param i32 i32 i64)))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0404");
    // missing implicit capability handle parameter
    let f =
        Fixture::new(&[LOG]).imports(r#"(import "host:log@1" "write" (func (param i32 i32 i32)))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0404");
    // wrong result shape on a result<> function
    let f = Fixture::new(&[STATE])
        .imports(r#"(import "host:state@1" "get_i64" (func (param i32 i32) (result i64)))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0404");
    // non-function imports
    for imp in [
        r#"(import "env" "g" (global i32))"#,
        r#"(import "env" "t" (table 1 funcref))"#,
    ] {
        let f = Fixture::new(&[LOG]).imports(imp);
        assert_eq!(code_of(load(&e, &f)), "OXS-E0405", "{imp}");
    }
}

#[test]
fn rejects_imported_memory() {
    let e = env();
    // own + imported memory: refused by the engine (one memory only)
    let f = Fixture::new(&[LOG]).imports(r#"(import "env" "mem" (memory 1))"#);
    assert!(load(&e, &f).is_err());
}

#[test]
fn rejects_ungranted_and_unknown_needs() {
    let e = env();
    let f = Fixture::new(&[LOG, STATE])
        .imports(IMPORT_LOG)
        .imports(IMPORT_STATE)
        .body(r#"(func (export "cb:host:state@1.State.OnChange") (param i32 i32 i32 i32) nop)"#);
    // only log granted
    let r = e.host.instantiate(&f.bytes(), Grants::only(["host:log@1"]));
    assert_eq!(code_of(r), "OXS-E0400");
    let r = e.host.instantiate(&f.bytes(), Grants::none());
    assert_eq!(code_of(r), "OXS-E0400");
    // need for a capability nobody registered
    let f = Fixture::new(&["host:fs@1 Fs"]);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0400");
    // right module, wrong interface name
    let f = Fixture::new(&["host:log@1 Logger"]);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0401");
}

#[test]
fn rejects_bad_manifests_and_abi() {
    let e = env();
    let mut f = Fixture::new(&[]);
    f.manifest = Some(String::new());
    assert_eq!(code_of(load(&e, &f)), "OXS-E0402");
    let mut f = Fixture::new(&[]);
    f.manifest =
        Some("oxi-manifest 1\nabi 1.0\nedition 2026\nprofile default\nfrobnicate\n".into());
    assert_eq!(code_of(load(&e, &f)), "OXS-E0402");
    // duplicate section
    let mut f = Fixture::new(&[]);
    f.extra_custom
        .push(("oxi.manifest".into(), f.manifest_text().into_bytes()));
    assert_eq!(code_of(load(&e, &f)), "OXS-E0402");
    // not UTF-8
    let mut f = Fixture::new(&[]);
    f.manifest = Some(String::new());
    f.extra_custom
        .push(("oxi.manifest".into(), vec![0xFF, 0xFE, 0xFD]));
    assert_eq!(code_of(load(&e, &f)), "OXS-E0402");
    // oversized
    let mut f = Fixture::new(&[]);
    f.manifest = Some(format!(
        "{}# {}\n",
        Fixture::new(&[]).manifest_text(),
        "x".repeat(5000)
    ));
    assert_eq!(code_of(load(&e, &f)), "OXS-E0402");
    // newer ABI major / minor
    let mut f = Fixture::new(&[]);
    f.manifest = Some("oxi-manifest 1\nabi 2.0\nedition 2026\nprofile default\n".into());
    assert_eq!(code_of(load(&e, &f)), "OXS-E0403");
    let mut f = Fixture::new(&[]);
    f.manifest = Some("oxi-manifest 1\nabi 1.5\nedition 2026\nprofile default\n".into());
    assert_eq!(code_of(load(&e, &f)), "OXS-E0403");
    // kernel profile
    let mut f = Fixture::new(&[]);
    f.profile = "kernel";
    assert_eq!(code_of(load(&e, &f)), "OXS-E0403");
    // exported version global disagrees with the manifest
    let mut f = Fixture::new(&[]);
    f.abi_version = 2 << 16;
    assert_eq!(code_of(load(&e, &f)), "OXS-E0403");
}

#[test]
fn rejects_what_the_engine_must_not_run() {
    let e = env();
    // garbage and truncation
    for bytes in [vec![], vec![1, 2, 3], b"\0asm\x01\0\0\0\xff\xff".to_vec()] {
        assert_eq!(
            code_of(e.host.instantiate(&bytes, Grants::all())),
            "OXS-E0408"
        );
    }
    let good = Fixture::new(&[]).bytes();
    assert!(e.host.instantiate(&good, Grants::all()).is_ok());
    for cut in [8, 20, good.len() / 2, good.len() - 3] {
        let r = e.host.instantiate(&good[..cut], Grants::all());
        assert!(r.is_err(), "cut at {cut}");
    }
    // start function
    let f = Fixture::new(&[]).body("(func $s nop) (start $s)");
    assert_eq!(code_of(load(&e, &f)), "OXS-E0408");
    // proposals the ABI does not allow
    let f =
        Fixture::new(&[]).body(r#"(func (export "v") (result v128) (v128.const i32x4 0 0 0 0))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0408");
    let f = Fixture::new(&[])
        .body(r#"(func (export "r") (result i32) (i32.trunc_sat_f32_s (f32.const 1)))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0408");
    let f = Fixture::new(&[]).body(r#"(func (export "t") (return_call 0))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0408");
    let f = Fixture::new(&[]).body(r#"(func (export "r") (result externref) (ref.null extern))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0408");
    let f = Fixture::new(&[]).body(r#"(memory 1)"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0408");
    // sign-extension, bulk memory and multi-value are allowed
    let f = Fixture::new(&[]).body(
        r#"(func (export "ok") (result i32 i32)
             (memory.fill (i32.const 4000) (i32.const 1) (i32.const 4))
             (i32.extend8_s (i32.const 255)) (i32.const 2))"#,
    );
    let mut rt = load(&e, &f).unwrap();
    assert_eq!(
        rt.call_export("ok", &[]).unwrap(),
        vec![Value::I32(-1), Value::I32(2)]
    );
}

#[test]
fn rejects_scratch_and_limit_violations() {
    let e = env();
    for (ptr, cap) in [
        (0, 4096),
        (512, 4096),
        (2052, 4096),
        (2048, 100),
        (131_000, 4096),
        (2048, 200_000),
    ] {
        let mut f = Fixture::new(&[]);
        f.scratch_ptr = ptr;
        f.scratch_cap = cap;
        assert_eq!(code_of(load(&e, &f)), "OXS-E0410", "({ptr}, {cap})");
    }
    // module size
    let e2 = env_with(Limits {
        max_module_bytes: 100,
        ..Limits::default()
    });
    assert_eq!(code_of(load(&e2, &Fixture::new(&[]))), "OXS-E0409");
    // initial memory above the ceiling
    let e3 = env_with(Limits {
        max_memory_pages: 4,
        ..Limits::default()
    });
    let mut f = Fixture::new(&[]);
    f.memory_pages = 10;
    assert_eq!(code_of(load(&e3, &f)), "OXS-E0409");
    // table above the ceiling
    let e4 = env_with(Limits {
        max_table_elements: 10,
        ..Limits::default()
    });
    let f = Fixture::new(&[]).body("(table 1000 funcref)");
    assert!(load(&e4, &f).is_err());
    // a module whose init is too slow never becomes visible
    let e5 = env_with(Limits {
        fuel_per_call: 1000,
        ..Limits::default()
    });
    let f = Fixture::new(&[]).init("(loop $l (br $l))");
    let err = load(&e5, &f).err().unwrap();
    assert_eq!(trap_of(err).cause, TrapCause::OutOfFuel);
    assert_eq!(e5.host.live_instances(), 0);
}

// ---- capabilities: forged handles, revocation --------------------------

#[test]
fn forged_and_cross_interface_handles() {
    let e = env();
    let mut rt = quick(&app(), &e);
    // status-bearing import: denial is a status the guest can see
    for bogus in [0, 1, 12345, -1, 0x7FFF_FFFF] {
        let r = rt
            .call_export("open_with_cap", &[Value::I32(bogus)])
            .unwrap();
        assert_eq!(r, vec![Value::I32(2)], "forged handle {bogus}");
    }
    // the log capability's handle used on the state interface is denied too
    // (capability 0 is `log`; find its value through a legal call path: the
    // handles are seeded per instance, so just try a window of small values)
    // void import: there is no status channel, so the instance traps
    let t = trap_of(
        rt.call_export("log_with_cap", &[Value::I32(777)])
            .unwrap_err(),
    );
    assert!(matches!(t.cause, TrapCause::HostDenied { .. }), "{t:?}");
    assert_eq!(t.code(), "OXS-E0424");
    assert!(rt.is_poisoned());
    // forged slot handle: denied status, no trap
    let mut rt2 = quick(&app(), &e);
    for bogus in [0, 1, 99999, -5] {
        assert_eq!(
            rt2.call_export("get_with_slot", &[Value::I32(bogus)])
                .unwrap(),
            vec![Value::I32(2)]
        );
    }
    assert!(!rt2.is_poisoned());
}

#[test]
fn capability_handle_from_another_interface_is_denied() {
    // `init` leaks both capability handles into exported-visible state by
    // re-exporting them through functions.
    let e = env();
    let f = app().body(
        r#"(func (export "cap0") (result i32) (global.get $cap0))
           (func (export "cap1") (result i32) (global.get $cap1))"#,
    );
    let mut rt = quick(&f, &e);
    let log_cap = match rt.call_export("cap0", &[]).unwrap()[0] {
        Value::I32(x) => x,
        _ => unreachable!(),
    };
    let state_cap = match rt.call_export("cap1", &[]).unwrap()[0] {
        Value::I32(x) => x,
        _ => unreachable!(),
    };
    assert_ne!(log_cap, state_cap);
    // log's handle on state.open -> denied; state's handle on state.open -> ok
    assert_eq!(
        rt.call_export("open_with_cap", &[Value::I32(log_cap)])
            .unwrap(),
        vec![Value::I32(2)]
    );
    assert_eq!(
        rt.call_export("open_with_cap", &[Value::I32(state_cap)])
            .unwrap(),
        vec![Value::I32(0)]
    );
    // state's handle on log.write (void) -> trap
    let t = trap_of(
        rt.call_export("log_with_cap", &[Value::I32(state_cap)])
            .unwrap_err(),
    );
    assert!(matches!(t.cause, TrapCause::HostDenied { .. }));
    // (cross-instance handles: see `handles_differ_per_instance_with_entropy_and_are_stable_without`)
}

#[test]
fn revoked_capability_is_denied() {
    let e = env();
    e.store.set("k", StateValue::I64(1)).unwrap();
    let mut rt = quick(&app(), &e);
    assert_eq!(
        rt.call_export("get_status", &[]).unwrap(),
        vec![Value::I32(0)]
    );
    assert!(rt.revoke("host:state@1"));
    assert!(!rt.revoke("host:nope@1"));
    // status-bearing functions report denial...
    assert_eq!(
        rt.call_export("get_status", &[]).unwrap(),
        vec![Value::I32(2)]
    );
    assert!(!rt.is_poisoned());
    // ...functions without a status channel trap
    let mut rt = quick(&app(), &e);
    assert!(rt.revoke("host:log@1"));
    let t = trap_of(rt.call_export("log_level", &[Value::I32(2)]).unwrap_err());
    assert!(matches!(t.cause, TrapCause::HostDenied { .. }));
}

// ---- host <-> guest data: bounds attacks -------------------------------

#[test]
fn scratch_and_pointer_attacks_trap_cleanly() {
    let e = env();
    let attacks: &[(u32, u32)] = &[
        (0, 4),              // null region
        (1023, 2),           // straddles the reserved low bytes
        (0xFFFF_FFF0, 0x20), // ptr + len wraps u32
        (0xFFFF_FFFF, 2),    // wraps
        (131_072, 1),        // one past the end (2 pages)
        (131_000, 100),      // crosses the end
        (2048, 0xFFFF_FFFF), // huge len
        (3000, 100_000),     // len above the payload cap
        (3000, 70_000),      // just above max_host_payload
    ];
    for (ptr, len) in attacks {
        let mut rt = quick(&app(), &e);
        let t = trap_of(
            rt.call_export(
                "log_at",
                &[Value::I32(*ptr as i32), Value::I32(*len as i32)],
            )
            .unwrap_err(),
        );
        assert!(
            matches!(t.cause, TrapCause::HostBadArgument(_)),
            "({ptr},{len}) {t:?}"
        );
        assert_eq!(t.code(), "OXS-E0423");
        assert!(rt.is_poisoned());
    }
    // len 0 never reads memory, whatever the pointer
    let mut rt = quick(&app(), &e);
    rt.call_export("log_at", &[Value::I32(-1), Value::I32(0)])
        .unwrap();
    // a valid region at the very end of memory is fine (it is only text)
    let mut rt = quick(&app(), &e);
    rt.call_export("log_at", &[Value::I32(131_072 - 4), Value::I32(4)])
        .unwrap();
    // regions overlapping the scratch area are legal reads
    rt.call_export("log_at", &[Value::I32(2048), Value::I32(4096)])
        .unwrap();
}

#[test]
fn bad_utf8_and_noncanonical_values_trap() {
    let e = env();
    let f = app().body(
        r#"(data (i32.const 3300) "\ff\fe")
           (func (export "log_bad") (call $log_write (global.get $cap0) (i32.const 2) (i32.const 3300) (i32.const 2)))"#,
    );
    let mut rt = quick(&f, &e);
    let t = trap_of(rt.call_export("log_bad", &[]).unwrap_err());
    assert!(
        matches!(&t.cause, TrapCause::HostBadArgument(m) if m.contains("UTF-8")),
        "{t:?}"
    );
    // level is u8: 300 and -1 are not canonical
    for lvl in [300, -1, 256] {
        let mut rt = quick(&app(), &e);
        let t = trap_of(rt.call_export("log_level", &[Value::I32(lvl)]).unwrap_err());
        assert!(matches!(t.cause, TrapCause::HostBadArgument(_)), "{lvl}");
    }
    let mut rt = quick(&app(), &e);
    rt.call_export("log_level", &[Value::I32(255)]).unwrap();
}

#[test]
fn out_buffer_retry_protocol() {
    let e = env();
    let mut rt = quick(&app(), &e);
    assert_eq!(
        rt.call_export("write_hw", &[]).unwrap(),
        vec![Value::I32(0)]
    );
    // too small: status 1 and the required length, nothing written
    let sentinel = rt.read_memory(8192, 16).unwrap();
    let r = rt
        .call_export("read_into", &[Value::I32(8192), Value::I32(4)])
        .unwrap();
    assert_eq!(r, vec![Value::I32(1), Value::I32(11)]);
    assert_eq!(rt.read_memory(8192, 16).unwrap(), sentinel);
    // retry with a big enough buffer
    let r = rt
        .call_export("read_into", &[Value::I32(8192), Value::I32(64)])
        .unwrap();
    assert_eq!(r, vec![Value::I32(0), Value::I32(11)]);
    assert_eq!(rt.read_memory(8192, 11).unwrap(), b"hello world");
    // exactly fitting
    let r = rt
        .call_export("read_into", &[Value::I32(9000), Value::I32(11)])
        .unwrap();
    assert_eq!(r, vec![Value::I32(0), Value::I32(11)]);
    assert_eq!(
        rt.read_memory(9000, 12).unwrap()[11],
        0,
        "no write past len"
    );
    // zero-capacity buffer: pointer is ignored, required length reported
    let r = rt
        .call_export("read_into", &[Value::I32(-1), Value::I32(0)])
        .unwrap();
    assert_eq!(r, vec![Value::I32(1), Value::I32(11)]);
    // an out-buffer outside memory is a trap
    for (p, c) in [(131_000, 1000), (0, 16), (0xFFFF_FFFFu32 as i32, 8)] {
        let mut rt = quick(&app(), &e);
        rt.call_export("write_hw", &[]).unwrap();
        let t = trap_of(
            rt.call_export("read_into", &[Value::I32(p), Value::I32(c)])
                .unwrap_err(),
        );
        assert!(
            matches!(t.cause, TrapCause::HostBadArgument(_)),
            "({p},{c})"
        );
    }
    // host status from the capability: reading an unset key
    let f = app().body(
        r#"(func (export "unset") (result i32)
             (local $st i32) (local $h i32)
             (call $s_open (global.get $cap1) (i32.const 3000) (i32.const 5))
             (local.set $h) (local.set $st)
             (call $s_get (global.get $cap1) (local.get $h)) (drop) (local.set $st) (local.get $st))"#,
    );
    let mut rt = quick(&f, &e);
    assert_eq!(rt.call_export("unset", &[]).unwrap(), vec![Value::I32(16)]);
    // type mismatch status
    e.store.set("k", StateValue::Str("x".into())).unwrap();
    assert_eq!(
        rt.call_export("get_status", &[]).unwrap(),
        vec![Value::I32(17)]
    );
}

// ---- resources ---------------------------------------------------------

#[test]
fn fuel_stops_infinite_loops_and_trap_then_recover() {
    let e = env_with(Limits {
        fuel_per_call: 100_000,
        ..Limits::default()
    });
    let mut rt = quick(&app(), &e);
    let t = trap_of(rt.call_export("spin", &[]).unwrap_err());
    assert_eq!(t.cause, TrapCause::OutOfFuel);
    assert_eq!(t.code(), "OXS-E0420");
    assert!(
        t.fuel_used >= 99_990 && t.fuel_used <= 100_000,
        "{}",
        t.fuel_used
    );
    assert!(rt.is_poisoned());
    assert!(matches!(rt.call_export("bump", &[]), Err(Error::Poisoned)));
    assert!(matches!(rt.tick(0), Err(Error::Poisoned)));
    assert!(matches!(rt.pump(), Err(Error::Poisoned)));
    // recovery: restart rebuilds the module; host state survived
    e.store.set("k", StateValue::I64(5)).unwrap();
    rt.restart().unwrap();
    assert!(!rt.is_poisoned());
    assert_eq!(rt.call_export("bump", &[]).unwrap(), vec![Value::I64(6)]);
    assert_eq!(rt.call_export("local", &[]).unwrap(), vec![Value::I32(1)]);
    // fuel is per call, not cumulative
    for _ in 0..50 {
        rt.call_export("bump", &[]).unwrap();
    }
}

#[test]
fn host_calls_cost_fuel() {
    let e = env_with(Limits {
        fuel_per_call: 5_000,
        host_call_fuel: 1_000,
        ..Limits::default()
    });
    let mut rt = quick(&app(), &e);
    let t = trap_of(rt.call_export("logs", &[Value::I32(100)]).unwrap_err());
    assert_eq!(t.cause, TrapCause::OutOfFuel);
}

#[test]
fn memory_grow_beyond_limit_fails_cleanly() {
    let e = env_with(Limits {
        max_memory_pages: 4,
        ..Limits::default()
    });
    let mut rt = quick(&app(), &e);
    assert_eq!(
        rt.call_export("grow", &[Value::I32(1)]).unwrap(),
        vec![Value::I32(2)]
    );
    assert_eq!(
        rt.call_export("grow", &[Value::I32(1)]).unwrap(),
        vec![Value::I32(3)]
    );
    assert_eq!(
        rt.call_export("grow", &[Value::I32(1)]).unwrap(),
        vec![Value::I32(-1)]
    );
    assert_eq!(
        rt.call_export("grow", &[Value::I32(100_000)]).unwrap(),
        vec![Value::I32(-1)]
    );
    assert_eq!(rt.memory_pages(), 4);
    assert!(!rt.is_poisoned());
}

#[test]
fn deep_recursion_is_stopped() {
    let e = env();
    let mut rt = quick(&app(), &e);
    let t = trap_of(rt.call_export("rec", &[Value::I32(0)]).unwrap_err());
    assert_eq!(t.cause, TrapCause::StackOverflow);
    assert_eq!(t.code(), "OXS-E0421");
    assert!(rt.is_poisoned());
}

#[test]
fn log_abuse_is_dropped_not_trapped() {
    let e = env();
    let mut rt = quick(&app(), &e);
    rt.call_export("logs", &[Value::I32(1000)]).unwrap();
    // 1 greeting + at most 64 lines for the turn
    assert_eq!(e.sink.lines().len(), 1 + 64);
    assert!(e.sink.dropped() >= 900);
    assert!(!rt.is_poisoned());
    // a new turn gets a new budget
    rt.call_export("logs", &[Value::I32(3)]).unwrap();
    assert_eq!(e.sink.lines().len(), 1 + 64 + 3);
}

#[test]
fn instance_limit_counts_the_transient_reload_instance() {
    let e = env_with(Limits {
        max_instances: 2,
        ..Limits::default()
    });
    let mut a = quick(&app(), &e);
    let b = quick(&app(), &e);
    assert_eq!(e.host.live_instances(), 2);
    let third = e.host.instantiate(&app().bytes(), Grants::all());
    assert!(matches!(third, Err(Error::Limit(_))));
    // reload needs a second slot while the old instance is alive
    assert!(matches!(a.reload(&app().bytes()), Err(Error::Limit(_))));
    assert_eq!(a.call_export("bump", &[]).unwrap(), vec![Value::I64(1)]);
    drop(b);
    assert_eq!(e.host.live_instances(), 1);
    a.reload(&app().bytes()).unwrap();
    assert_eq!(e.host.live_instances(), 1);
    drop(a);
    assert_eq!(e.host.live_instances(), 0);
}

// ---- traps -------------------------------------------------------------

#[test]
fn guest_trap_kinds_are_structured() {
    let e = env();
    let mut rt = quick(&app(), &e);
    let t = trap_of(rt.call_export("boom", &[]).unwrap_err());
    assert_eq!(t.cause, TrapCause::Unreachable);
    assert_eq!(t.guest_kind, Some(TrapKind::Bounds));
    assert_eq!(t.guest_kind_raw, 4);
    assert!(rt.is_poisoned());
    assert!(matches!(rt.call_export("local", &[]), Err(Error::Poisoned)));
    rt.restart().unwrap();
    assert_eq!(rt.call_export("local", &[]).unwrap(), vec![Value::I32(0)]);
    // every documented kind maps
    for k in oxiscript_abi::trap::TrapKind::ALL {
        let mut rt = quick(&app(), &e);
        let t = trap_of(
            rt.call_export("boom_kind", &[Value::I32(k.code() as i32)])
                .unwrap_err(),
        );
        assert_eq!(t.guest_kind, Some(k));
    }
    // unknown code is kept raw
    let mut rt = quick(&app(), &e);
    let t = trap_of(rt.call_export("boom_kind", &[Value::I32(99)]).unwrap_err());
    assert_eq!(t.guest_kind, None);
    assert_eq!(t.guest_kind_raw, 99);
    assert!(format!("{}", Error::Trap(t)).contains("unknown(99)"));
    // plain unreachable: engine trap, trap kind stays 0
    let mut rt = quick(&app(), &e);
    let t = trap_of(rt.call_export("unreachable", &[]).unwrap_err());
    assert_eq!(t.guest_kind_raw, 0);
    assert_eq!(t.cause, TrapCause::Unreachable);
}

// ---- reload ------------------------------------------------------------

#[test]
fn reload_swaps_code_and_keeps_host_state() {
    let e = env();
    let mut rt = quick(&app(), &e);
    rt.call_export("bump", &[]).unwrap();
    rt.call_export("bump", &[]).unwrap();
    assert_eq!(rt.call_export("local", &[]).unwrap(), vec![Value::I32(2)]);
    let v2 = app().body(r#"(func (export "v2") (result i32) (i32.const 222))"#);
    let st = rt.reload(&v2.bytes()).unwrap();
    assert!(st.build.as_secs() < 5);
    // module-local state reset, host-held state survived
    assert_eq!(rt.call_export("local", &[]).unwrap(), vec![Value::I32(0)]);
    assert_eq!(
        e.store.get_in(rt.namespace(), "k"),
        Some(StateValue::I64(2))
    );
    assert_eq!(rt.call_export("bump", &[]).unwrap(), vec![Value::I64(3)]);
    assert_eq!(rt.call_export("v2", &[]).unwrap(), vec![Value::I32(222)]);
    assert_eq!(e.host.live_instances(), 1);
    // v2 init logged again
    assert_eq!(e.sink.lines().len(), 2);
}

#[test]
fn failed_reload_keeps_the_old_instance_running() {
    let e = env();
    let mut rt = quick(&app(), &e);
    rt.call_export("bump", &[]).unwrap();
    // 1: garbage
    assert!(rt.reload(b"not wasm").is_err());
    // 2: valid wasm that violates the ABI
    assert!(rt.reload(&app().skip("oxi_tick").bytes()).is_err());
    // 3: init traps
    let bad = app().init("unreachable");
    let t = trap_of(rt.reload(&bad.bytes()).unwrap_err());
    assert_eq!(t.cause, TrapCause::Unreachable);
    // 4: init exhausts fuel
    let e2 = env_with(Limits {
        fuel_per_call: 50_000,
        ..Limits::default()
    });
    let mut rt2 = quick(&app(), &e2);
    let t = trap_of(
        rt2.reload(&app().init("(loop $l (br $l))").bytes())
            .unwrap_err(),
    );
    assert_eq!(t.cause, TrapCause::OutOfFuel);
    assert_eq!(rt2.call_export("bump", &[]).unwrap(), vec![Value::I64(1)]);
    // 5: new module wants a capability that was not granted
    let mut rt3 = e
        .host
        .instantiate(&app().bytes(), Grants::only(["host:log@1", "host:state@1"]))
        .unwrap();
    let wants_more = Fixture::new(&[LOG, STATE, "host:fs@1 Fs"]);
    assert!(rt3.reload(&wants_more.bytes()).is_err());
    // the old one still works and still counts as the only instance of rt
    assert_eq!(rt.call_export("bump", &[]).unwrap(), vec![Value::I64(2)]);
    assert_eq!(rt.call_export("local", &[]).unwrap(), vec![Value::I32(2)]);
    assert_eq!(e.host.live_instances(), 2); // rt and rt3
}

#[test]
fn reload_recovers_a_poisoned_runtime_and_failed_reload_keeps_it_poisoned() {
    let e = env();
    let mut rt = quick(&app(), &e);
    let _ = rt.call_export("unreachable", &[]);
    assert!(rt.is_poisoned());
    assert!(rt.reload(b"junk").is_err());
    assert!(
        rt.is_poisoned(),
        "the failed reload must not touch the old instance"
    );
    rt.reload(&app().bytes()).unwrap();
    assert!(!rt.is_poisoned());
    assert_eq!(rt.call_export("bump", &[]).unwrap(), vec![Value::I64(1)]);
}

#[test]
fn failed_reload_drops_the_new_instances_watchers() {
    let e = env();
    let mut rt = quick(&app_cb(), &e);
    // new module registers a watcher in init, then traps
    let bad = app_cb().init("unreachable");
    assert!(rt.reload(&bad.bytes()).is_err());
    // only the old instance's watcher is live: exactly one event
    e.store.set("k", StateValue::I64(9)).unwrap();
    let s = rt.pump().unwrap();
    assert_eq!(s.delivered, 1);
}

// ---- callbacks ---------------------------------------------------------

#[test]
fn callbacks_arrive_between_calls_with_scratch_payload() {
    let e = env();
    let mut rt = quick(&app_cb(), &e);
    // nothing pending yet
    assert_eq!(rt.pump().unwrap().delivered, 0);
    e.store.set("k", StateValue::I64(1)).unwrap();
    let s = rt.pump().unwrap();
    assert_eq!((s.delivered, s.dropped), (1, 0));
    let r = rt.call_export("cb_state", &[]).unwrap();
    // closure 4242, key length 1, first byte 'k', one delivery
    assert_eq!(
        r,
        vec![
            Value::I32(4242),
            Value::I32(1),
            Value::I32(b'k' as i32),
            Value::I32(1)
        ]
    );
    // coalesced: two writes before one pump -> one event
    e.store.set("k", StateValue::I64(2)).unwrap();
    e.store.set("k", StateValue::I64(3)).unwrap();
    assert_eq!(rt.pump().unwrap().delivered, 1);
    // a write by the guest itself reaches its own watcher at the next pump
    rt.call_export("ping", &[]).unwrap();
    assert_eq!(rt.pump().unwrap().delivered, 1);
}

#[test]
fn pump_is_bounded_and_rewatch_releases_the_old_closure() {
    let e = env_with(Limits {
        max_events_per_pump: 2,
        ..Limits::default()
    });
    let mut rt = quick(&app_cb(), &e);
    for c in [1111, 2222, 3333, 4444] {
        rt.call_export("watch_new", &[Value::I32(c)]).unwrap();
    }
    e.store.set("k", StateValue::I64(1)).unwrap();
    // 5 watchers, 2 per pump
    let mut total = 0;
    for _ in 0..3 {
        let d = rt.pump().unwrap().delivered;
        assert!(d <= 2);
        total += d;
    }
    assert_eq!(total, 5);
    assert_eq!(rt.pump().unwrap().delivered, 0);
    // re-watching the same slot queues the old closure for release
    rt.call_export("rewatch", &[Value::I32(9999)]).unwrap();
    let s = rt.pump().unwrap();
    assert_eq!(s.released, 1);
    assert_eq!(
        rt.call_export("released", &[]).unwrap(),
        vec![Value::I32(4242)]
    );
}

#[test]
fn watcher_limit_traps_instead_of_growing() {
    let e = env();
    let mut rt = quick(&app_cb(), &e);
    let mut trapped = false;
    for i in 0..100 {
        match rt.call_export("watch_new", &[Value::I32(100 + i)]) {
            Ok(_) => {}
            Err(err) => {
                assert!(matches!(trap_of(err).cause, TrapCause::HostFailure(_)));
                trapped = true;
                break;
            }
        }
    }
    assert!(trapped);
}

#[test]
fn a_callback_that_loops_is_stopped_by_fuel() {
    let e = env_with(Limits {
        fuel_per_call: 20_000,
        ..Limits::default()
    });
    let f = app_no_cb().body(
        r#"(func (export "cb:host:state@1.State.OnChange") (param i32 i32 i32 i32) (loop $l (br $l)))"#,
    )
    .init("(call $s_watch (global.get $cap1) (global.get $slot) (i32.const 1))");
    let mut rt = quick(&f, &e);
    e.store.set("k", StateValue::I64(1)).unwrap();
    let t = trap_of(rt.pump().unwrap_err());
    assert_eq!(t.cause, TrapCause::OutOfFuel);
    assert!(rt.is_poisoned());
}

// ---- tick and lifecycle -----------------------------------------------

#[test]
fn tick_deinit_and_leak_hooks() {
    let e = env();
    let f = app()
        .over("oxi_tick", r#"(func (export "oxi_tick") (param i64) (result i64) (i64.add (local.get 0) (i64.const 16)))"#)
        .over("oxi_live_objects", r#"(func (export "oxi_live_objects") (result i32) (i32.const 0))"#);
    let mut rt = quick(&f, &e);
    assert_eq!(rt.tick(1000).unwrap(), 1016);
    assert_eq!(rt.live_objects().unwrap(), 0);
    assert_eq!(rt.collect_cycles().unwrap(), 0);
    assert_eq!(rt.deinit().unwrap(), 0);
}

/// Regression for wasmi's engine keeping compiled code of dropped modules:
/// one engine per instance means repeated reloads must not grow the process.
/// Ignored by default (RSS is noisy next to parallel tests); the `measure`
/// example reports the same number.
#[test]
#[ignore]
fn reload_does_not_leak() {
    fn rss_kib() -> i64 {
        let s = std::fs::read_to_string("/proc/self/statm").unwrap();
        s.split_whitespace().nth(1).unwrap().parse::<i64>().unwrap() * 4
    }
    let e = env();
    let bytes = app().bytes();
    let mut rt = e.host.instantiate(&bytes, Grants::all()).unwrap();
    for _ in 0..200 {
        rt.reload(&bytes).unwrap();
    }
    let before = rss_kib();
    for _ in 0..3000 {
        rt.reload(&bytes).unwrap();
    }
    assert!(
        rss_kib() - before < 4096,
        "RSS grew by {} KiB",
        rss_kib() - before
    );
}

#[test]
fn forbidden_proposals_are_rejected_before_wasmi_can_panic() {
    let e = env();
    // exception-handling tag, imported and defined
    let f = Fixture::new(&[]).imports(r#"(import "env" "t" (tag (param i32)))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0408");
    let f = Fixture::new(&[]).body("(tag (param i32))");
    assert_eq!(code_of(load(&e, &f)), "OXS-E0408");
    // threads: shared memory
    let f = Fixture::new(&[]).body(r#"(memory (export "shm") 1 1 shared)"#);
    assert!(load(&e, &f).is_err());
    // memory64
    let mut f = Fixture::new(&[]);
    let wat = f.wat().replace(
        r#"(memory (export "memory") 2)"#,
        r#"(memory (export "memory") i64 2)"#,
    );
    let mut bytes = wat::parse_str(&wat).unwrap();
    append_custom(&mut bytes, "oxi.manifest", f.manifest_text().as_bytes());
    f.manifest = None;
    assert_eq!(
        code_of(e.host.instantiate(&bytes, Grants::all())),
        "OXS-E0408"
    );
}

#[test]
fn guest_written_trap_kind_is_not_believed_for_engine_traps() {
    let e = env_with(Limits {
        fuel_per_call: 50_000,
        ..Limits::default()
    });
    let f = app().body(
        r#"(func (export "lie_then_spin") (global.set $tk (i32.const 7)) (loop $l (br $l)))"#,
    );
    let mut rt = quick(&f, &e);
    let t = trap_of(rt.call_export("lie_then_spin", &[]).unwrap_err());
    assert_eq!(t.cause, TrapCause::OutOfFuel);
    assert_eq!(t.guest_kind, None);
    assert_eq!(t.guest_kind_raw, 0);
}

#[test]
fn successful_reload_drops_the_old_instances_callbacks() {
    let e = env();
    let mut rt = quick(&app_cb(), &e);
    rt.reload(&app_cb().bytes()).unwrap();
    e.store.set("k", StateValue::I64(9)).unwrap();
    // only the new instance's watcher exists
    assert_eq!(rt.pump().unwrap().delivered, 1);
    assert_eq!(rt.pump().unwrap().delivered, 0);
}

#[test]
fn queued_releases_are_bounded_per_pump() {
    let e = env_with(Limits {
        max_events_per_pump: 3,
        ..Limits::default()
    });
    let f = app().body(
        r#"(func (export "rewatch_many") (param $n i32)
             (local $i i32)
             (loop $l
               (call $s_watch (global.get $cap1) (global.get $slot) (i32.add (i32.const 5000) (local.get $i)))
               (local.set $i (i32.add (local.get $i) (i32.const 1)))
               (br_if $l (i32.lt_u (local.get $i) (local.get $n)))))"#,
    );
    let mut rt = quick(&f, &e);
    rt.call_export("rewatch_many", &[Value::I32(10)]).unwrap(); // 9 releases queued
    let mut total = 0;
    for _ in 0..3 {
        let s = rt.pump().unwrap();
        assert!(s.released <= 3);
        total += s.released;
    }
    assert_eq!(total, 9);
}

#[test]
fn handles_differ_per_instance_with_entropy_and_are_stable_without() {
    use oxiscript_runtime::caps::log::{LogLimits, LogProvider, LogSink};
    use oxiscript_runtime::caps::state::{StateLimits, StateProvider, StateStore};
    let f = app().body(r#"(func (export "cap0") (result i32) (global.get $cap0))"#);
    let mk = |entropy: Option<[u8; 16]>| {
        let host = match entropy {
            Some(e) => oxiscript_runtime::Host::with_entropy(Limits::default(), e),
            None => oxiscript_runtime::Host::new(Limits::default()),
        };
        host.register(std::sync::Arc::new(LogProvider::new(
            LogSink::new(8),
            LogLimits::default(),
        )))
        .unwrap();
        host.register(std::sync::Arc::new(StateProvider::new(StateStore::new(
            StateLimits::default(),
        ))))
        .unwrap();
        host
    };
    let cap = |h: &oxiscript_runtime::Host| {
        let mut rt = h.instantiate(&f.bytes(), Grants::all()).unwrap();
        match rt.call_export("cap0", &[]).unwrap()[0] {
            Value::I32(x) => (x, rt),
            _ => unreachable!(),
        }
    };
    // deterministic mode: same module, same value, whatever the host did before
    let h = mk(None);
    let (a, ra) = cap(&h);
    let (b, _rb) = cap(&h);
    assert_eq!(a, b);
    drop(ra);
    // entropy mode: differs per instance and per secret; a handle of another
    // instance is denied
    let h = mk(Some([7; 16]));
    let (a, _ra) = cap(&h);
    let (b, mut rb) = cap(&h);
    assert_ne!(a, b);
    let (c, _) = cap(&mk(Some([8; 16])));
    assert_ne!(a, c);
    let t = trap_of(
        rb.call_export("log_with_cap", &[Value::I32(a)])
            .unwrap_err(),
    );
    assert!(matches!(t.cause, TrapCause::HostDenied { .. }));
}

#[test]
fn state_namespaces_isolate_runtimes_and_survive_restart() {
    let e = env();
    let mut a = quick(&app(), &e);
    let mut b = quick(&app(), &e);
    assert_ne!(a.namespace(), b.namespace());
    a.call_export("bump", &[]).unwrap();
    a.restart().unwrap();
    assert_eq!(a.call_export("bump", &[]).unwrap(), vec![Value::I64(2)]);
    assert_eq!(b.call_export("bump", &[]).unwrap(), vec![Value::I64(1)]);
    // host namespace is a read-only fallback: shadowed once the guest wrote
    e.store.set("k", StateValue::I64(100)).unwrap();
    assert_eq!(a.call_export("bump", &[]).unwrap(), vec![Value::I64(3)]);
    // anonymous namespaces are freed with their runtime
    let ns = b.namespace().to_string();
    assert!(e.store.get_in(&ns, "k").is_some());
    drop(b);
    assert!(e.store.get_in(&ns, "k").is_none());
    // named namespaces persist until the embedder clears them
    let r = e
        .host
        .instantiate_in(&app().bytes(), Grants::all(), "app:one")
        .unwrap();
    drop(r);
    e.store.set_in("app:one", "x", StateValue::I64(1)).unwrap();
    assert_eq!(e.store.get_in("app:one", "x"), Some(StateValue::I64(1)));
    assert!(e.store.delete_in("app:one", "x"));
    e.store.clear_ns("app:one");
    assert_eq!(e.store.bytes_in("app:one"), 0);
}

#[test]
fn host_write_does_not_notify_a_shadowing_namespace() {
    let e = env();
    let mut rt = quick(&app_cb(), &e);
    rt.call_export("bump", &[]).unwrap(); // guest now owns `k`
    rt.pump().unwrap();
    e.store.set("k", StateValue::I64(5)).unwrap();
    assert_eq!(rt.pump().unwrap().delivered, 0);
}

#[test]
fn reexported_imports_are_rejected() {
    let e = env();
    let f = app().body(r#"(export "relog" (func $log_write))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0407");
    let f = app_no_cb().body(r#"(export "cb:host:state@1.State.OnChange" (func $s_watch))"#);
    assert_eq!(code_of(load(&e, &f)), "OXS-E0407");
}

#[test]
fn revocation_is_sticky_until_regrant() {
    let e = env();
    e.store.set("k", StateValue::I64(1)).unwrap();
    let mut rt = quick(&app(), &e);
    assert!(rt.revoke("host:state@1"));
    let _ = rt.call_export("unreachable", &[]);
    rt.restart().unwrap();
    assert_eq!(
        rt.call_export("get_status", &[]).unwrap(),
        vec![Value::I32(2)]
    );
    rt.reload(&app().bytes()).unwrap();
    assert_eq!(
        rt.call_export("get_status", &[]).unwrap(),
        vec![Value::I32(2)]
    );
    assert!(rt.regrant("host:state@1"));
    rt.restart().unwrap();
    assert_eq!(
        rt.call_export("get_status", &[]).unwrap(),
        vec![Value::I32(0)]
    );
}

#[test]
fn release_queue_is_bounded_with_a_dedicated_code() {
    let e = env();
    let f = app().body(
        r#"(func (export "rewatch_many") (param $n i32)
             (local $i i32)
             (loop $l
               (call $s_watch (global.get $cap1) (global.get $slot) (i32.add (i32.const 5000) (local.get $i)))
               (local.set $i (i32.add (local.get $i) (i32.const 1)))
               (br_if $l (i32.lt_u (local.get $i) (local.get 0)))))"#,
    );
    let mut rt = quick(&f, &e);
    let t = trap_of(
        rt.call_export("rewatch_many", &[Value::I32(100_000)])
            .unwrap_err(),
    );
    assert!(matches!(t.cause, TrapCause::QueueFull(_)));
    assert_eq!(t.code(), "OXS-E0429");
    assert!(rt.host_calls() <= 1024 + 8, "{}", rt.host_calls());
}

#[test]
fn log_lines_are_sanitised() {
    use oxiscript_runtime::caps::log::sanitize;
    assert_eq!(sanitize("a\nb\x1b[2Jc\0", 100), "a\\x0ab\\x1b[2Jc\\x00");
    // never cut inside an escape
    assert_eq!(sanitize("ab\n", 4), "ab");
    assert!(!sanitize("\u{85}\u{7f}x", 100).chars().any(char::is_control));
}
