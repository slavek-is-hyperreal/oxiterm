//! Target parameters. ABI 1.0 is wasm32: addresses are `i32` and 4 bytes wide.
//! Lowering code refers to these instead of hard-coding 32 bits where a
//! constant suffices, so a later memory64 target is a contained change
//! (the object layout itself lives in `oxiscript-abi::layout`).

use wasm_encoder::ValType;

/// Wasm value type of an address / managed reference.
pub(crate) const PTR_VT: ValType = ValType::I32;
/// Size in bytes of an address stored in memory.
#[allow(dead_code)]
pub(crate) const PTR_BYTES: u32 = 4;
