//! The runtime cannot depend on `oxiscript-abi` (that crate is std, the wasm
//! guest is `no_std`), so it carries its own copies of the layout constants.
//! This test is the tie: any drift from `layout.rs` / `trap.rs` fails here.

use oxiscript_abi::layout::{self as l, Colour, TypeKind};
use oxiscript_abi::trap::TrapKind;
use oxiscript_rc::rt;

#[test]
fn constants_match_the_abi_crate() {
    assert_eq!(rt::HEADER, l::HEADER_SIZE);
    assert_eq!(rt::STICKY, l::RC_STICKY);
    assert_eq!(rt::META_TYPE_MASK, l::META_TYPE_MASK);
    assert_eq!(rt::COLOUR_SHIFT, l::META_COLOUR_SHIFT);
    assert_eq!(rt::COLOUR_MASK, l::META_COLOUR_MASK);
    assert_eq!(rt::BUFFERED, l::META_BUFFERED);
    assert_eq!(rt::BLACK, Colour::Black as u32);
    assert_eq!(rt::GRAY, Colour::Gray as u32);
    assert_eq!(rt::WHITE, Colour::White as u32);
    assert_eq!(rt::PURPLE, Colour::Purple as u32);
    assert_eq!(rt::GREEN, Colour::Green as u32);
    assert!(Colour::from_bits(rt::PENDING).is_none() && Colour::from_bits(rt::ZOMBIE).is_none());
    assert_eq!(rt::TYPE_STRING, l::TYPE_ID_STRING);
    assert_eq!(rt::K_STRUCT, TypeKind::Struct as u32);
    assert_eq!(rt::K_VARIANT, TypeKind::UnionVariant as u32);
    assert_eq!(rt::K_CLOSURE, TypeKind::Closure as u32);
    assert_eq!(rt::K_STRING, TypeKind::String as u32);
    assert_eq!(rt::K_ARRAY, TypeKind::Array as u32);
    assert_eq!(rt::FLAG_ACYCLIC, u32::from(l::TYPE_FLAG_ACYCLIC));
    assert_eq!(rt::TD_SIZE, l::TYPE_DESC_SIZE);
    assert_eq!((rt::ARR_LEN, rt::ARR_CAP, rt::ARR_DATA), (l::ARR_OFF_LEN, l::ARR_OFF_CAP, l::ARR_OFF_DATA));
    assert_eq!(rt::ARR_OBJ_SIZE, l::object_alloc_size(l::ARR_PAYLOAD_SIZE).unwrap());
    assert_eq!(rt::ARR_MAX_LEN, l::ARR_MAX_LEN);
    assert_eq!(rt::T_UNREACHABLE, TrapKind::Unreachable.code());
    assert_eq!(rt::T_BOUNDS, TrapKind::Bounds.code());
    assert_eq!(rt::T_BAD_CAST, TrapKind::BadCast.code());
    assert_eq!(rt::T_NULL, TrapKind::NullDeref.code());
    assert_eq!(rt::T_OOM, TrapKind::OutOfMemory.code());
    assert_eq!(rt::T_BAD_UTF8, TrapKind::BadUtf8.code());
}

#[test]
fn string_layout_matches_the_abi() {
    // header 8 + (len, reserved, bytes, NUL) rounded to 8
    for len in [0u32, 1, 7, 8, 9, 100] {
        let payload = l::string_payload_size(len);
        assert_eq!(payload, 8 + len + 1);
        let total = l::object_alloc_size(payload).unwrap();
        assert_eq!(total, (8 + 8 + len + 1 + 7) & !7);
    }
}
