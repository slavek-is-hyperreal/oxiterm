#![allow(clippy::cast_lossless, clippy::doc_markdown)]
//! BAD ACTOR tests for slice 8.C (ABI). Run with --release (wrap-around). Each FAILS today.
use oxiscript_abi::layout::*;
use oxiscript_abi::*;
use std::panic::catch_unwind;

fn doc(n: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../../docs/oxiscript/{n}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn ba01_string_payload_size_overflow() {
    let r = catch_unwind(|| string_payload_size(u32::MAX - 4));
    if let Ok(v) = r {
        assert!(
            v as u64 >= u32::MAX as u64 - 4,
            "wrapped to {v}: heap overflow when used as allocation size"
        );
    }
}
#[test]
fn ba02_layout_fields_overflow() {
    let big = FieldKind::Inline {
        size: u32::MAX - 7,
        align: 8,
        ptr_slots: &[],
    };
    let r = catch_unwind(|| layout_fields(0, &[big, big]));
    if let Ok(l) = r {
        assert!(
            l.offsets[1] >= l.offsets[0] + (u32::MAX - 7),
            "offsets wrapped: {:?} size {}",
            l.offsets,
            l.size
        );
    }
}
#[test]
fn ba03_layout_align_zero_overlaps() {
    let r = catch_unwind(|| {
        layout_fields(
            0,
            &[
                FieldKind::Prim(Prim::U8),
                FieldKind::Inline {
                    size: 4,
                    align: 0,
                    ptr_slots: &[],
                },
            ],
        )
    });
    if let Ok(l) = r {
        assert!(
            l.offsets[1] >= 1,
            "field overlaps previous: {:?}",
            l.offsets
        );
    }
}
#[test]
fn ba04_layout_non_pow2_align() {
    let r = catch_unwind(|| {
        layout_fields(
            0,
            &[
                FieldKind::Prim(Prim::U8),
                FieldKind::Inline {
                    size: 4,
                    align: 3,
                    ptr_slots: &[],
                },
            ],
        )
    });
    if let Ok(l) = r {
        assert_eq!(l.offsets[1] % 3, 0, "misaligned: {:?}", l.offsets);
    }
}
#[test]
fn ba05_pack_meta_truncates_type_id() {
    let r = catch_unwind(|| unpack_meta(pack_meta(0x0100_0001, Colour::Black, false)));
    if let Ok(Some((id, _, _))) = r {
        assert_eq!(id, 0x0100_0001, "type id silently aliased to {id}");
    }
}
#[test]
fn ba06_inline_ptr_slots_unvalidated() {
    // CORRECTED by the 8.C fix: layout_fields is now fallible (try_layout_fields returns Err;
    // layout_fields panics in every build), so invalid slots can never produce a layout.
    let bad = FieldKind::Inline {
        size: 8,
        align: 4,
        ptr_slots: &[4, 0, 100],
    };
    assert!(try_layout_fields(0, &[bad]).is_err());
    let ok = FieldKind::Inline {
        size: 8,
        align: 4,
        ptr_slots: &[0, 4],
    };
    let l = try_layout_fields(0, &[ok]).unwrap();
    assert!(l.ptr_slots.windows(2).all(|w| w[0] < w[1]) && l.ptr_slots.iter().all(|s| *s < l.size));
}
#[test]
fn ba07_cb_export_names_collide_across_versions() {
    let src = "interface State \"host:state\" @1 { callback C(a: i32); }\ninterface State \"host:state\" @2 { callback C(a: i32); }";
    let ifs = idl::parse(src).unwrap();
    let a = ifs[0].wasm_callback_exports()[0].name.clone();
    let b = ifs[1].wasm_callback_exports()[0].name.clone();
    assert_ne!(
        a, b,
        "two capability versions side by side export the same cb: name"
    );
}
#[test]
fn ba08_cb_export_names_collide_across_modules() {
    let src = "interface S \"host:a\" @1 { callback C(a: i32); }\ninterface S \"host:b\" @1 { callback C(a: i32); }";
    let ifs = idl::parse(src).unwrap();
    assert_ne!(
        ifs[0].wasm_callback_exports()[0].name,
        ifs[1].wasm_callback_exports()[0].name
    );
}
#[test]
fn ba09_two_str_callback_params_share_scratch() {
    assert!(
        idl::parse("interface S \"host:a\" @1 { callback C(a: str, b: str); }").is_err(),
        "ABI delivers str params at one scratch address: second param has no defined location"
    );
}
#[test]
fn ba10_handle_shadowed_by_primitive() {
    assert!(idl::parse("interface S \"host:a\" @1 { handle u8; fn f(h: u8); }").is_err());
}
#[test]
fn ba11_callback_duplicate_params() {
    assert!(idl::parse("interface S \"host:a\" @1 { callback C(a: i32, a: i32); }").is_err());
}
#[test]
fn ba12_manifest_duplicate_needs() {
    let t = "oxi-manifest 1\nabi 1.0\nedition 2026\nprofile default\nneeds host:log@1 Log\nneeds host:log@1 Log\n";
    assert!(manifest::Manifest::parse(t).is_err());
}
#[test]
fn ba13_manifest_unknown_profile() {
    let t = "oxi-manifest 1\nabi 1.0\nedition 2026\nprofile banana\n";
    assert!(
        manifest::Manifest::parse(t).is_err(),
        "abi.md: profile default|realtime|safety|kernel"
    );
}
#[test]
fn ba14_manifest_non_canonical_numbers() {
    let t = "oxi-manifest 1\nabi +1.+0\nedition +2026\nprofile default\n";
    assert!(manifest::Manifest::parse(t).is_err());
}
#[test]
fn ba15_manifest_empty_module_name() {
    let t = "oxi-manifest 1\nabi 1.0\nedition 2026\nprofile default\nneeds host:@1 X\n";
    assert!(manifest::Manifest::parse(t).is_err());
}
#[test]
fn ba16_registry_missing_blank_line_swallows_code() {
    let t = "code OXS-E0001\nseverity error\nphase parse\ntitle t\nmessage m\ncode OXS-E0002\nseverity error\nphase parse\ntitle t\nmessage m\n";
    assert!(
        codes::parse_registry(t).is_err(),
        "second code silently dropped (duplicate keys, first wins)"
    );
}
#[test]
fn ba17_registry_duplicate_key() {
    let t = "code OXS-E0001\nseverity error\nseverity warning\nphase parse\ntitle t\nmessage m\n";
    assert!(codes::parse_registry(t).is_err());
}
#[test]
fn ba18_render_reinterpolates_values() {
    assert_eq!(
        codes::render("{a} {b}", &[("a", "{b}"), ("b", "X")]),
        "{b} X"
    );
}
#[test]
fn ba19_trap_kind_provider_matches_abi_md() {
    assert_eq!(
        exports::find(exports::TRAP_KIND).unwrap().provider,
        exports::Provider::Compiler
    );
}
#[test]
fn ba20_abi_md_status_vs_buffer_len() {
    let d = doc("abi.md");
    let a = d.contains("on a non-zero status the value result is zero");
    let b = d.contains("returns status `1` and the required `len`");
    assert!(!(a && b), "5.1: value is zero on non-zero status; 5.2: status 1 returns required len; Result<T,i32> drops it");
}
#[test]
fn ba21_abi_md_scratch_only_write_place() {
    let d = doc("abi.md");
    assert!(
        !(d.contains("the only place the host may write")
            && d.contains("the host writes up to `out_cap` bytes"))
    );
}
#[test]
fn ba22_retain_marks_black_vs_green() {
    let s = exports::find(exports::RETAIN).unwrap().doc;
    assert!(
        !s.contains("marks the object Black") || s.contains("Green"),
        "oxi_retain would recolour Green objects"
    );
}
#[test]
fn ba23_ir_md_elision_vs_calls() {
    let d = doc("ir.md");
    assert!(
        !d.contains("calls that do not take/return `x`") || d.contains("any call may"),
        "elision across calls that can drop the last other reference via a global/shared structure = UAF"
    );
}
