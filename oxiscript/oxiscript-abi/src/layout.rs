//! Value and memory layout (wasm32, little endian, one linear memory).
//!
//! Every managed object is `[header (8 bytes)][payload]`. A *pointer to an
//! object points at the payload*; the header lives at `ptr - 8`. Objects are
//! 8-aligned and their total size is a multiple of 8. Address 0 is null and
//! the first [`RESERVED_LOW_BYTES`] bytes are never allocated.

use crate::WasmTy;

/// ABI version: `major << 16 | minor`. The module exports it as the immutable
/// global `oxi_abi_version`. A host accepts the same major and a minor not
/// greater than its own.
pub const ABI_MAJOR: u16 = 1;
pub const ABI_MINOR: u16 = 0;
pub const ABI_VERSION: u32 = ((ABI_MAJOR as u32) << 16) | (ABI_MINOR as u32);

/// Null pointer value.
pub const NULL: u32 = 0;
/// Bytes at the bottom of linear memory that are never allocated or mapped to
/// data (so small-integer-looking pointers fault in debug tooling).
pub const RESERVED_LOW_BYTES: u32 = 1024;
/// Allocation alignment (and object alignment).
pub const OBJECT_ALIGN: u32 = 8;

// ---- header -------------------------------------------------------------

pub const HEADER_SIZE: u32 = 8;
/// Offset of the reference count relative to the object pointer (negative).
pub const OFF_RC: i32 = -8;
/// Offset of the meta word relative to the object pointer (negative).
pub const OFF_META: i32 = -4;

/// Reference counts at or above this value are *sticky*: `retain` and
/// `release` leave them unchanged and the object is never freed (static data,
/// string constants, and objects whose count saturated). The count is a
/// non-atomic `u32` (one isolate = one thread).
pub const RC_STICKY: u32 = 0x8000_0000;

/// Meta word: bits 0..=23 type id, bits 24..=26 colour, bit 27 buffered
/// (in the cycle collector's candidate-roots buffer), bits 28..=31 reserved
/// (must be zero; future: weak/finalizer/pinned flags).
pub const META_TYPE_MASK: u32 = 0x00FF_FFFF;
pub const META_COLOUR_SHIFT: u32 = 24;
pub const META_COLOUR_MASK: u32 = 0b111 << META_COLOUR_SHIFT;
pub const META_BUFFERED: u32 = 1 << 27;

/// Cycle-collector colours (Bacon-Rajan synchronous; Nim ORC uses the same
/// scheme). `Green` objects can never be part of a cycle and are never
/// buffered or traced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Colour {
    Black = 0,
    Gray = 1,
    White = 2,
    Purple = 3,
    Green = 4,
}

impl Colour {
    pub fn from_bits(bits: u32) -> Option<Colour> {
        Some(match bits {
            0 => Colour::Black,
            1 => Colour::Gray,
            2 => Colour::White,
            3 => Colour::Purple,
            4 => Colour::Green,
            _ => return None,
        })
    }
}

/// Pack a meta word; `None` if `type_id` does not fit 24 bits.
pub fn try_pack_meta(type_id: u32, colour: Colour, buffered: bool) -> Option<u32> {
    if type_id > MAX_TYPE_ID {
        return None;
    }
    Some(
        type_id | ((colour as u32) << META_COLOUR_SHIFT) | if buffered { META_BUFFERED } else { 0 },
    )
}

/// Pack a meta word. **Panics in every build** if `type_id` does not fit 24 bits
/// (an aliased type id would free or trace the wrong object).
pub fn pack_meta(type_id: u32, colour: Colour, buffered: bool) -> u32 {
    try_pack_meta(type_id, colour, buffered).expect("type id exceeds 24 bits")
}

/// Unpack a meta word into `(type_id, colour, buffered)`. `None` if a
/// reserved bit is set or the colour is invalid.
pub fn unpack_meta(meta: u32) -> Option<(u32, Colour, bool)> {
    if meta >> 28 != 0 {
        return None;
    }
    let colour = Colour::from_bits((meta & META_COLOUR_MASK) >> META_COLOUR_SHIFT)?;
    Some((meta & META_TYPE_MASK, colour, meta & META_BUFFERED != 0))
}

// ---- type ids -----------------------------------------------------------

pub const TYPE_ID_INVALID: u32 = 0;
/// The one built-in type id: `string`.
pub const TYPE_ID_STRING: u32 = 1;
/// Ids 2..16 are reserved for built-ins added by minor ABI versions.
pub const FIRST_USER_TYPE_ID: u32 = 16;
pub const MAX_TYPE_ID: u32 = META_TYPE_MASK;

// ---- payload layouts ----------------------------------------------------

/// String payload: `u32 byte_len`, `u32 reserved (0; hash cache later)`, the
/// UTF-8 bytes, then one `0` byte that is *not* counted in `byte_len`
/// (C-interop convenience). Strings are immutable. The pointer to a string
/// plus [`STR_OFF_BYTES`] is the data pointer used when flattening to
/// `(ptr, len)` at the host boundary.
pub const STR_OFF_LEN: u32 = 0;
pub const STR_OFF_RESERVED: u32 = 4;
pub const STR_OFF_BYTES: u32 = 8;
/// Payload size of a string of `len` bytes (before 8-rounding of the whole
/// object); `None` if it (or the whole object) does not fit in `u32`.
pub fn checked_string_payload_size(len: u32) -> Option<u32> {
    let p = STR_OFF_BYTES.checked_add(len)?.checked_add(1)?;
    object_alloc_size(p)?;
    Some(p)
}

/// As [`checked_string_payload_size`]; **panics in every build** on overflow.
pub fn string_payload_size(len: u32) -> u32 {
    checked_string_payload_size(len).expect("string length overflows the object size")
}

/// Bytes of an array data block (`cap * elem_size`); `None` on overflow.
pub fn array_data_bytes(cap: u32, elem_size: u32) -> Option<u32> {
    cap.checked_mul(elem_size)
}

/// Array payload: `u32 len`, `u32 cap`, `u32 data` where `data` is a pointer
/// to a separate *header-less* block of `cap * elem_size` bytes obtained from
/// `oxi_alloc_bytes` (8-aligned, owned by the array, freed with it).
/// Elements are stored back to back at their natural size; an element is
/// either a scalar, a managed reference (4 bytes), or - reserved for value
/// aggregates and device buffers - an inline record whose pointer slots are
/// listed in the array's type descriptor pointer map (see [`TypeDesc`]).
pub const ARR_OFF_LEN: u32 = 0;
pub const ARR_OFF_CAP: u32 = 4;
pub const ARR_OFF_DATA: u32 = 8;
pub const ARR_PAYLOAD_SIZE: u32 = 12;
/// Largest element count (`i32::MAX`): lengths and indices are `i32`.
pub const ARR_MAX_LEN: u32 = 0x7FFF_FFFF;

/// Closure payload: `u32 table_index` (index into the module's function table;
/// the function takes the closure itself as first parameter), `u32 reserved`,
/// then the captured values laid out like record fields starting at
/// [`CLOSURE_OFF_CAPTURES`]. A plain function used as a value is a closure
/// with no captures.
pub const CLOSURE_OFF_FN: u32 = 0;
pub const CLOSURE_OFF_RESERVED: u32 = 4;
pub const CLOSURE_OFF_CAPTURES: u32 = 8;

/// Union (sum type) payload: `u32 tag` at offset 0, variant fields follow
/// (laid out with prefix [`UNION_FIELDS_PREFIX`]). The header `type_id` names
/// the *variant* (each variant is its own record type with the tag as hidden
/// first field), so release/trace need no extra dispatch. `tag` is the
/// variant's declaration index. `Result<T,E>` is the union `Ok{value}` (tag 0),
/// `Err{error}` (tag 1).
pub const UNION_OFF_TAG: u32 = 0;
pub const UNION_FIELDS_PREFIX: u32 = 4;

/// Struct and class payload: fields in declaration order, natural alignment,
/// no reordering ([`layout_fields`] with prefix 0).
pub const RECORD_FIELDS_PREFIX: u32 = 0;

// ---- primitive and field layout ----------------------------------------

/// Storage primitives. Narrow integers are kept sign/zero-extended in `i32`
/// registers (values always canonical for their declared width) and stored at
/// their natural size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Prim {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    /// Raw address (wasm32): 4 bytes, not managed.
    Ptr,
}

impl Prim {
    pub fn size(self) -> u32 {
        match self {
            Prim::Bool | Prim::I8 | Prim::U8 => 1,
            Prim::I16 | Prim::U16 => 2,
            Prim::I32 | Prim::U32 | Prim::F32 | Prim::Ptr => 4,
            Prim::I64 | Prim::U64 | Prim::F64 => 8,
        }
    }

    pub fn align(self) -> u32 {
        self.size()
    }

    /// The Wasm value type a value of this primitive occupies.
    pub fn wasm(self) -> WasmTy {
        match self {
            Prim::I64 | Prim::U64 => WasmTy::I64,
            Prim::F32 => WasmTy::F32,
            Prim::F64 => WasmTy::F64,
            _ => WasmTy::I32,
        }
    }
}

/// What occupies a field slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldKind {
    Prim(Prim),
    /// A managed reference (string, array, record, closure, union, nullable
    /// of those): 4 bytes, 4-aligned, listed in the pointer map.
    Ref,
    /// An inline record of the given size and alignment, whose own pointer
    /// slots (offsets relative to the record start) are given. Reserved for
    /// value aggregates (arrays of inline structs, SIMD vectors, device
    /// buffer elements); the MVP lowering never produces it but the layout
    /// function and the descriptors already support it.
    Inline {
        size: u32,
        align: u32,
        ptr_slots: &'static [u32],
    },
}

impl FieldKind {
    pub fn size(&self) -> u32 {
        match self {
            FieldKind::Prim(p) => p.size(),
            FieldKind::Ref => 4,
            FieldKind::Inline { size, .. } => *size,
        }
    }

    pub fn align(&self) -> u32 {
        match self {
            FieldKind::Prim(p) => p.align(),
            FieldKind::Ref => 4,
            FieldKind::Inline { align, .. } => *align,
        }
    }
}

/// Result of laying out a record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordLayout {
    /// Payload offset of each field, in declaration order.
    pub offsets: Vec<u32>,
    /// Payload size, rounded up to [`OBJECT_ALIGN`] (so header + payload keeps
    /// objects 8-aligned back to back).
    pub size: u32,
    /// Payload offsets of every managed-reference slot (the pointer map),
    /// ascending.
    pub ptr_slots: Vec<u32>,
}

/// Round `x` up to a multiple of `align`; `None` if `align` is not a power of
/// two (>= 1) or the result overflows.
pub fn align_up(x: u32, align: u32) -> Option<u32> {
    if !align.is_power_of_two() {
        return None;
    }
    x.checked_add(align - 1).map(|v| v & !(align - 1))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// An offset or size does not fit in `u32`.
    Overflow,
    /// A field alignment is not a power of two >= 1.
    BadAlign,
    /// Inline pointer slots are unsorted, duplicated or outside the field.
    BadPtrSlots,
}

/// The single field-layout function (compiler and runtime must both use it):
/// start at `prefix` bytes (0 for records, 8 for closures, 4 for union
/// variants), place each field at the next offset aligned to its alignment,
/// round the total up to 8. Every step is checked.
pub fn try_layout_fields(prefix: u32, fields: &[FieldKind]) -> Result<RecordLayout, LayoutError> {
    let mut off = prefix;
    let mut offsets = Vec::with_capacity(fields.len());
    let mut ptr_slots = Vec::new();
    for f in fields {
        off = align_up(off, f.align()).ok_or_else(|| {
            if f.align().is_power_of_two() {
                LayoutError::Overflow
            } else {
                LayoutError::BadAlign
            }
        })?;
        offsets.push(off);
        match f {
            FieldKind::Ref => ptr_slots.push(off),
            FieldKind::Inline {
                size,
                ptr_slots: ps,
                ..
            } => {
                let sorted = ps.windows(2).all(|w| w[0] < w[1]);
                let inside = ps
                    .iter()
                    .all(|p| p.checked_add(4).is_some_and(|e| e <= *size));
                if !sorted || !inside {
                    return Err(LayoutError::BadPtrSlots);
                }
                for p in *ps {
                    ptr_slots.push(off.checked_add(*p).ok_or(LayoutError::Overflow)?);
                }
            }
            FieldKind::Prim(_) => {}
        }
        off = off.checked_add(f.size()).ok_or(LayoutError::Overflow)?;
    }
    let size = align_up(off, OBJECT_ALIGN).ok_or(LayoutError::Overflow)?;
    object_alloc_size(size).ok_or(LayoutError::Overflow)?;
    Ok(RecordLayout {
        offsets,
        size,
        ptr_slots,
    })
}

/// As [`try_layout_fields`]; **panics in every build** on invalid input.
pub fn layout_fields(prefix: u32, fields: &[FieldKind]) -> RecordLayout {
    try_layout_fields(prefix, fields).expect("invalid record layout")
}

/// Total allocation size (header + payload) of an object with this payload
/// size; `None` on overflow.
pub fn object_alloc_size(payload: u32) -> Option<u32> {
    align_up(HEADER_SIZE.checked_add(payload)?, OBJECT_ALIGN)
}

// ---- type descriptors ---------------------------------------------------

/// Size of one descriptor in the descriptor table.
pub const TYPE_DESC_SIZE: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TypeKind {
    Struct = 1,
    UnionVariant = 2,
    Closure = 3,
    String = 4,
    /// Array; `size_or_elem` of the descriptor is the element size and the
    /// pointer map describes pointer slots *within one element*.
    Array = 5,
}

impl TypeKind {
    pub fn from_u8(v: u8) -> Option<TypeKind> {
        Some(match v {
            1 => TypeKind::Struct,
            2 => TypeKind::UnionVariant,
            3 => TypeKind::Closure,
            4 => TypeKind::String,
            5 => TypeKind::Array,
            _ => return None,
        })
    }
}

/// `flags` bit: instances can never be part of a reference cycle; allocate
/// them `Green` and never buffer or trace them. The compiler sets it when the
/// type's reachable type graph is acyclic (strings, scalars-only records,
/// arrays of such, closures capturing only such, ...).
pub const TYPE_FLAG_ACYCLIC: u8 = 1;

/// One entry of the descriptor table (16 bytes, 4 little-endian words):
///
/// ```text
/// word0: size_or_elem   record/closure/union-variant: payload size (8-rounded);
///                       array: element size; string: 0
/// word1: kind(8) | flags(8) << 8 | ptr_count(16) << 16
/// word2: ptr_map_addr   address of `ptr_count` u32 payload offsets (record)
///                       or element-relative offsets (array); 0 if none
/// word3: reserved (0)   future: drop hook / debug-name index
/// ```
///
/// The table starts at the address in the exported immutable global
/// `oxi_typedesc_base` and has `oxi_typedesc_count` entries, indexed directly
/// by type id (entry 0 is all zeros).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeDesc {
    pub size_or_elem: u32,
    pub kind: TypeKind,
    pub flags: u8,
    pub ptr_count: u16,
    pub ptr_map_addr: u32,
}

impl TypeDesc {
    pub fn encode(&self) -> [u8; 16] {
        let w1 = u32::from(self.kind as u8)
            | u32::from(self.flags) << 8
            | u32::from(self.ptr_count) << 16;
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&self.size_or_elem.to_le_bytes());
        out[4..8].copy_from_slice(&w1.to_le_bytes());
        out[8..12].copy_from_slice(&self.ptr_map_addr.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8; 16]) -> Option<TypeDesc> {
        let w = |i: usize| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        let w1 = w(1);
        if w(3) != 0 {
            return None;
        }
        Some(TypeDesc {
            size_or_elem: w(0),
            kind: TypeKind::from_u8((w1 & 0xFF) as u8)?,
            flags: ((w1 >> 8) & 0xFF) as u8,
            ptr_count: (w1 >> 16) as u16,
            ptr_map_addr: w(2),
        })
    }

    /// Colour a fresh instance gets.
    pub fn initial_colour(&self) -> Colour {
        if self.flags & TYPE_FLAG_ACYCLIC != 0 {
            Colour::Green
        } else {
            Colour::Black
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_round_trip() {
        let m = pack_meta(0x00AB_CDEF, Colour::Purple, true);
        assert_eq!(unpack_meta(m), Some((0x00AB_CDEF, Colour::Purple, true)));
        assert_eq!(unpack_meta(1 << 31), None);
        assert_eq!(unpack_meta(7 << 24), None);
    }

    #[test]
    fn struct_layout_natural_alignment() {
        // struct { a: u8, b: f64, c: i32, d: ref }
        let l = layout_fields(
            RECORD_FIELDS_PREFIX,
            &[
                FieldKind::Prim(Prim::U8),
                FieldKind::Prim(Prim::F64),
                FieldKind::Prim(Prim::I32),
                FieldKind::Ref,
            ],
        );
        assert_eq!(l.offsets, vec![0, 8, 16, 20]);
        assert_eq!(l.size, 24);
        assert_eq!(l.ptr_slots, vec![20]);
    }

    #[test]
    fn empty_record_is_zero_sized_payload() {
        let l = layout_fields(0, &[]);
        assert_eq!(l.size, 0);
        assert_eq!(object_alloc_size(l.size), Some(8));
    }

    #[test]
    fn union_variant_and_closure_prefixes() {
        // Circle { r: f64 }: tag at 0, r aligned to 8.
        let l = layout_fields(UNION_FIELDS_PREFIX, &[FieldKind::Prim(Prim::F64)]);
        assert_eq!(l.offsets, vec![8]);
        assert_eq!(l.size, 16);
        // Ok { value: i32 }
        let l = layout_fields(UNION_FIELDS_PREFIX, &[FieldKind::Prim(Prim::I32)]);
        assert_eq!(l.offsets, vec![4]);
        assert_eq!(l.size, 8);
        // closure capturing (ref, i32)
        let l = layout_fields(
            CLOSURE_OFF_CAPTURES,
            &[FieldKind::Ref, FieldKind::Prim(Prim::I32)],
        );
        assert_eq!(l.offsets, vec![8, 12]);
        assert_eq!(l.ptr_slots, vec![8]);
        assert_eq!(l.size, 16);
    }

    #[test]
    fn inline_record_slots_are_rebased() {
        // Reserved path: a struct holding an inline {i32, ref} record.
        static SLOTS: [u32; 1] = [4];
        let l = layout_fields(
            0,
            &[
                FieldKind::Prim(Prim::U8),
                FieldKind::Inline {
                    size: 8,
                    align: 4,
                    ptr_slots: &SLOTS,
                },
            ],
        );
        assert_eq!(l.offsets, vec![0, 4]);
        assert_eq!(l.ptr_slots, vec![8]);
    }

    #[test]
    fn type_desc_round_trip() {
        let d = TypeDesc {
            size_or_elem: 24,
            kind: TypeKind::Struct,
            flags: TYPE_FLAG_ACYCLIC,
            ptr_count: 3,
            ptr_map_addr: 4096,
        };
        assert_eq!(TypeDesc::decode(&d.encode()), Some(d));
        assert_eq!(d.initial_colour(), Colour::Green);
        let mut bad = d.encode();
        bad[12] = 1;
        assert_eq!(TypeDesc::decode(&bad), None);
    }

    #[test]
    fn string_sizes() {
        assert_eq!(string_payload_size(0), 9);
        assert_eq!(object_alloc_size(string_payload_size(0)), Some(24));
        assert_eq!(object_alloc_size(ARR_PAYLOAD_SIZE), Some(24));
        assert_eq!(checked_string_payload_size(u32::MAX - 4), None);
        assert_eq!(array_data_bytes(0x8000_0000, 4), None);
        assert_eq!(array_data_bytes(10, 8), Some(80));
    }

    #[test]
    fn layout_rejects_invalid_input() {
        let big = FieldKind::Inline {
            size: u32::MAX - 7,
            align: 8,
            ptr_slots: &[],
        };
        assert_eq!(
            try_layout_fields(0, &[big, big]),
            Err(LayoutError::Overflow)
        );
        let a0 = FieldKind::Inline {
            size: 4,
            align: 0,
            ptr_slots: &[],
        };
        assert_eq!(try_layout_fields(0, &[a0]), Err(LayoutError::BadAlign));
        let a3 = FieldKind::Inline {
            size: 4,
            align: 3,
            ptr_slots: &[],
        };
        assert_eq!(try_layout_fields(0, &[a3]), Err(LayoutError::BadAlign));
        for slots in [&[4u32, 0][..], &[0, 0], &[8]] {
            let f = FieldKind::Inline {
                size: 8,
                align: 4,
                ptr_slots: slots,
            };
            assert_eq!(
                try_layout_fields(0, &[f]),
                Err(LayoutError::BadPtrSlots),
                "{slots:?}"
            );
        }
        assert_eq!(align_up(u32::MAX, 8), None);
        assert_eq!(try_pack_meta(0x0100_0000, Colour::Black, false), None);
    }

    #[test]
    fn abi_version_packs() {
        assert_eq!(ABI_VERSION >> 16, u32::from(ABI_MAJOR));
        assert_eq!(ABI_VERSION & 0xFFFF, u32::from(ABI_MINOR));
    }
}
