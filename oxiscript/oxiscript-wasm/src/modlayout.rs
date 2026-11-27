//! Module-level memory layout: type ids, field offsets, the static data
//! segment (string constants, pointer maps, the type descriptor table).

use crate::{Diagnostic, E_LIMIT};
use oxiscript_abi::layout::{
    self, pack_meta, Colour, FieldKind, Prim, TypeDesc, TypeKind, FIRST_USER_TYPE_ID, HEADER_SIZE,
    RC_STICKY, RESERVED_LOW_BYTES, TYPE_DESC_SIZE, TYPE_FLAG_ACYCLIC, TYPE_ID_STRING,
};
use oxiscript_ir::{Module, Ty, TypeKind as IrKind};
use std::collections::{BTreeMap, HashMap};

/// Layout of one record-like object (struct, closure environment, union variant).
#[derive(Clone, Debug)]
pub(crate) struct Rec {
    pub id: u32,
    pub offsets: Vec<u32>,
    pub tys: Vec<Ty>,
    /// Field name -> index (first declaration wins).
    pub ix: HashMap<String, usize>,
}

#[derive(Clone, Debug)]
pub(crate) enum TLay {
    /// Struct or closure environment.
    Rec(Rec),
    /// One record per variant (the tag sits at offset 0).
    Union {
        variants: Vec<(String, Rec)>,
    },
}

pub(crate) struct Layout {
    pub types: Vec<TLay>,
    pub type_ix: HashMap<String, usize>,
    /// Array element type -> descriptor id, keyed by the element type's debug text.
    pub array_ids: BTreeMap<String, u32>,
    /// Payload address of each string constant (indexed like `Module::data`).
    pub str_addr: Vec<u32>,
    pub data_ix: HashMap<String, usize>,
    pub typedesc_base: u32,
    pub typedesc_count: u32,
    /// Start address of [`Self::segment`] in linear memory.
    pub data_base: u32,
    /// The static segment, placed at [`Self::data_base`].
    pub segment: Vec<u8>,
    /// First address after the static data (8-aligned).
    pub static_end: u32,
}

pub(crate) fn field_kind(t: &Ty) -> FieldKind {
    match t {
        Ty::Bool => FieldKind::Prim(Prim::Bool),
        Ty::I8 => FieldKind::Prim(Prim::I8),
        Ty::U8 => FieldKind::Prim(Prim::U8),
        Ty::I16 => FieldKind::Prim(Prim::I16),
        Ty::U16 => FieldKind::Prim(Prim::U16),
        Ty::I32 => FieldKind::Prim(Prim::I32),
        Ty::U32 => FieldKind::Prim(Prim::U32),
        Ty::I64 => FieldKind::Prim(Prim::I64),
        Ty::U64 => FieldKind::Prim(Prim::U64),
        Ty::F32 => FieldKind::Prim(Prim::F32),
        Ty::F64 => FieldKind::Prim(Prim::F64),
        Ty::Ptr => FieldKind::Prim(Prim::Ptr),
        _ => FieldKind::Ref,
    }
}

pub(crate) fn ty_key(t: &Ty) -> String {
    format!("{t:?}")
}

fn visit_ty(t: &Ty, out: &mut Vec<Ty>, seen: &mut BTreeMap<String, ()>) {
    match t {
        Ty::Array(e) => {
            let k = ty_key(e);
            if seen.insert(k, ()).is_none() {
                out.push((**e).clone());
            }
            visit_ty(e, out, seen);
        }
        Ty::Opt(i) => visit_ty(i, out, seen),
        Ty::Fn(sig) => {
            sig.params.iter().for_each(|p| visit_ty(p, out, seen));
            sig.rets.iter().for_each(|p| visit_ty(p, out, seen));
        }
        _ => {}
    }
}

fn align_to(v: &mut Vec<u8>, a: usize) {
    while v.len() % a != 0 {
        v.push(0);
    }
}

fn ptr_count_u16(n: usize, ctx: &str) -> Result<u16, Diagnostic> {
    u16::try_from(n).map_err(|_| {
        Diagnostic::new(
            E_LIMIT,
            format!("{ctx} has {n} managed fields; the type descriptor holds at most {}", u16::MAX),
            None,
        )
    })
}

fn seg_addr(base: u32, seg: &[u8]) -> Result<u32, Diagnostic> {
    u32::try_from(seg.len())
        .ok()
        .and_then(|n| n.checked_add(base))
        .ok_or_else(|| Diagnostic::new(E_LIMIT, "static data too large", None))
}

fn alloc_id(next_id: &mut u32, what: &str) -> Result<u32, Diagnostic> {
    let id = *next_id;
    if id > layout::MAX_TYPE_ID {
        return Err(Diagnostic::new(E_LIMIT, format!("too many types ({what})"), None));
    }
    *next_id += 1;
    Ok(id)
}

fn rec_layout(
    prefix: u32,
    fields: &[(String, Ty)],
    id: u32,
    ctx: &str,
) -> Result<(Rec, layout::RecordLayout), Diagnostic> {
    let kinds: Vec<FieldKind> = fields.iter().map(|(_, t)| field_kind(t)).collect();
    let l = layout::try_layout_fields(prefix, &kinds).map_err(|e| {
        Diagnostic::new(E_LIMIT, format!("layout of {ctx} overflows: {e:?}"), None)
    })?;
    Ok((
        Rec {
            id,
            offsets: l.offsets.clone(),
            tys: fields.iter().map(|(_, t)| t.clone()).collect(),
            ix: {
                let mut ix = HashMap::with_capacity(fields.len());
                for (i, (n, _)) in fields.iter().enumerate() {
                    ix.entry(n.clone()).or_insert(i);
                }
                ix
            },
        },
        l,
    ))
}

impl Layout {
    pub(crate) fn build(m: &Module) -> Result<Layout, Diagnostic> {
        Self::build_at(m, RESERVED_LOW_BYTES)
    }

    pub(crate) fn build_at(m: &Module, base: u32) -> Result<Layout, Diagnostic> {
        let mut seg: Vec<u8> = Vec::new();
        let mut descs: BTreeMap<u32, TypeDesc> = BTreeMap::new();
        let mut ptr_maps: Vec<(u32, Vec<u32>)> = Vec::new(); // (type id, slots)

        // 1. string constants (immortal, sticky rc, green)
        let mut str_addr = Vec::new();
        let mut data_ix = HashMap::new();
        for (i, d) in m.data.iter().enumerate() {
            align_to(&mut seg, 8);
            let len = u32::try_from(d.text.len())
                .map_err(|_| Diagnostic::new(E_LIMIT, "string constant too large", None))?;
            let payload = layout::checked_string_payload_size(len)
                .ok_or_else(|| Diagnostic::new(E_LIMIT, "string constant too large", None))?;
            seg.extend_from_slice(&RC_STICKY.to_le_bytes());
            seg.extend_from_slice(&pack_meta(TYPE_ID_STRING, Colour::Green, false).to_le_bytes());
            let at = seg_addr(base, &seg)?;
            str_addr.push(at);
            data_ix.insert(d.name.clone(), i);
            seg.extend_from_slice(&len.to_le_bytes());
            seg.extend_from_slice(&0u32.to_le_bytes());
            seg.extend_from_slice(d.text.as_bytes());
            seg.push(0);
            let used = layout::object_alloc_size(payload).unwrap_or(0) - HEADER_SIZE;
            while (seg_addr(base, &seg)? - at) < used {
                seg.push(0);
            }
            if seg_addr(base, &seg)? > (1 << 30) {
                return Err(Diagnostic::new(E_LIMIT, "static data too large", None));
            }
        }

        // 2. type ids and record layouts
        let mut next_id = FIRST_USER_TYPE_ID;
        let mut types = Vec::new();
        let mut type_ix = HashMap::new();
        for (ti, t) in m.types.iter().enumerate() {
            type_ix.insert(t.name.clone(), ti);
            let acyc_attr = t.attrs.iter().any(|a| a.name == "acyclic");
            match &t.kind {
                IrKind::Struct { fields } | IrKind::Closure { captures: fields } => {
                    let prefix = if matches!(t.kind, IrKind::Closure { .. }) {
                        layout::CLOSURE_OFF_CAPTURES
                    } else {
                        layout::RECORD_FIELDS_PREFIX
                    };
                    let fl: Vec<(String, Ty)> = fields
                        .iter()
                        .map(|f| (f.name.clone(), f.ty.clone()))
                        .collect();
                    let id = alloc_id(&mut next_id, &t.name)?;
                    let (rec, l) = rec_layout(prefix, &fl, id, &t.name)?;
                    let kind = if prefix == 0 {
                        TypeKind::Struct
                    } else {
                        TypeKind::Closure
                    };
                    let acyclic = acyc_attr || l.ptr_slots.is_empty();
                    descs.insert(
                        id,
                        TypeDesc {
                            size_or_elem: l.size.max(if prefix == 0 { 0 } else { prefix }),
                            kind,
                            flags: if acyclic { TYPE_FLAG_ACYCLIC } else { 0 },
                            ptr_count: ptr_count_u16(l.ptr_slots.len(), &t.name)?,
                            ptr_map_addr: 0,
                        },
                    );
                    ptr_maps.push((id, l.ptr_slots));
                    types.push(TLay::Rec(rec));
                }
                IrKind::Union { variants } => {
                    let mut vs = Vec::new();
                    for v in variants {
                        let fl: Vec<(String, Ty)> = v
                            .fields
                            .iter()
                            .map(|f| (f.name.clone(), f.ty.clone()))
                            .collect();
                        let id = alloc_id(&mut next_id, &t.name)?;
                        let (rec, l) = rec_layout(
                            layout::UNION_FIELDS_PREFIX,
                            &fl,
                            id,
                            &format!("{}::{}", t.name, v.name),
                        )?;
                        let acyclic = acyc_attr || l.ptr_slots.is_empty();
                        descs.insert(
                            id,
                            TypeDesc {
                                size_or_elem: l.size,
                                kind: TypeKind::UnionVariant,
                                flags: if acyclic { TYPE_FLAG_ACYCLIC } else { 0 },
                                ptr_count: ptr_count_u16(l.ptr_slots.len(), &format!("{}::{}", t.name, v.name))?,
                                ptr_map_addr: 0,
                            },
                        );
                        ptr_maps.push((id, l.ptr_slots));
                        vs.push((v.name.clone(), rec));
                    }
                    types.push(TLay::Union { variants: vs });
                }
            }
        }

        // 3. array element types (first-appearance order)
        let mut elems: Vec<Ty> = Vec::new();
        let mut seen = BTreeMap::new();
        for t in &m.types {
            match &t.kind {
                IrKind::Struct { fields } | IrKind::Closure { captures: fields } => {
                    fields.iter().for_each(|f| visit_ty(&f.ty, &mut elems, &mut seen));
                }
                IrKind::Union { variants } => variants
                    .iter()
                    .flat_map(|v| v.fields.iter())
                    .for_each(|f| visit_ty(&f.ty, &mut elems, &mut seen)),
            }
        }
        for i in &m.imports {
            i.sig.params.iter().for_each(|p| visit_ty(p, &mut elems, &mut seen));
            i.sig.rets.iter().for_each(|p| visit_ty(p, &mut elems, &mut seen));
        }
        m.globals.iter().for_each(|g| visit_ty(&g.ty, &mut elems, &mut seen));
        for f in &m.funcs {
            f.values.iter().for_each(|v| visit_ty(&v.ty, &mut elems, &mut seen));
            f.rets.iter().for_each(|v| visit_ty(v, &mut elems, &mut seen));
        }
        let mut array_ids = BTreeMap::new();
        for e in &elems {
            let id = alloc_id(&mut next_id, "array element types")?;
            let fk = field_kind(e);
            let slots: Vec<u32> = if fk == FieldKind::Ref { vec![0] } else { vec![] };
            descs.insert(
                id,
                TypeDesc {
                    size_or_elem: fk.size(),
                    kind: TypeKind::Array,
                    flags: if slots.is_empty() { TYPE_FLAG_ACYCLIC } else { 0 },
                    ptr_count: ptr_count_u16(slots.len(), "array element")?,
                    ptr_map_addr: 0,
                },
            );
            ptr_maps.push((id, slots));
            array_ids.insert(ty_key(e), id);
        }
        descs.insert(
            TYPE_ID_STRING,
            TypeDesc {
                size_or_elem: 0,
                kind: TypeKind::String,
                flags: TYPE_FLAG_ACYCLIC,
                ptr_count: 0,
                ptr_map_addr: 0,
            },
        );

        // 4. pointer maps
        align_to(&mut seg, 4);
        for (id, slots) in &ptr_maps {
            if slots.is_empty() {
                continue;
            }
            let at = seg_addr(base, &seg)?;
            for s in slots {
                seg.extend_from_slice(&s.to_le_bytes());
            }
            descs.get_mut(id).expect("desc").ptr_map_addr = at;
        }

        // 5. descriptor table
        align_to(&mut seg, 16);
        let typedesc_base = seg_addr(base, &seg)?;
        let typedesc_count = next_id.max(FIRST_USER_TYPE_ID);
        let table_len = typedesc_count
            .checked_mul(TYPE_DESC_SIZE)
            .ok_or_else(|| Diagnostic::new(E_LIMIT, "type descriptor table too large", None))?;
        let mut table = vec![0u8; table_len as usize];
        for (id, d) in &descs {
            let o = (*id as usize) * (TYPE_DESC_SIZE as usize);
            table[o..o + 16].copy_from_slice(&d.encode());
        }
        seg.extend_from_slice(&table);
        align_to(&mut seg, 8);
        let static_end = seg_addr(base, &seg)?;
        if static_end > (1 << 30) {
            return Err(Diagnostic::new(E_LIMIT, "static data too large", None));
        }
        Ok(Layout {
            types,
            type_ix,
            array_ids,
            str_addr,
            data_ix,
            typedesc_base,
            typedesc_count,
            data_base: base,
            segment: seg,
            static_end,
        })
    }

    /// Record layout and (for union variants) the variant index of a type reference.
    pub(crate) fn rec(&self, ty: &str, variant: Option<&str>) -> Option<(&Rec, Option<u32>)> {
        match (self.types.get(*self.type_ix.get(ty)?)?, variant) {
            (TLay::Rec(r), None) => Some((r, None)),
            (TLay::Union { variants }, Some(v)) => {
                let i = variants.iter().position(|(n, _)| n == v)?;
                Some((&variants[i].1, u32::try_from(i).ok()))
            }
            _ => None,
        }
    }
}
