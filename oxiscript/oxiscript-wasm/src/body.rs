//! A function body under construction: instruction bytes plus the local
//! declarations (parameters first, then declared locals in allocation order).

use wasm_encoder::{Function, InstructionSink, MemArg, ValType};

/// Memory access flavours the ABI uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mem {
    U8,
    I8,
    U16,
    I16,
    I32,
    I64,
    F32,
    F64,
}

impl Mem {
    pub(crate) fn size(self) -> u32 {
        match self {
            Mem::U8 | Mem::I8 => 1,
            Mem::U16 | Mem::I16 => 2,
            Mem::I32 | Mem::F32 => 4,
            Mem::I64 | Mem::F64 => 8,
        }
    }
    pub(crate) fn align_log2(self) -> u32 {
        match self.size() {
            1 => 0,
            2 => 1,
            4 => 2,
            _ => 3,
        }
    }
    pub(crate) fn val(self) -> ValType {
        match self {
            Mem::I64 => ValType::I64,
            Mem::F32 => ValType::F32,
            Mem::F64 => ValType::F64,
            _ => ValType::I32,
        }
    }
}

pub(crate) struct Body {
    pub(crate) code: Vec<u8>,
    nparams: u32,
    locals: Vec<ValType>,
}

impl Body {
    pub(crate) fn new(nparams: usize) -> Body {
        Body {
            code: Vec::new(),
            nparams: nparams as u32,
            locals: Vec::new(),
        }
    }

    pub(crate) fn s(&mut self) -> InstructionSink<'_> {
        InstructionSink::new(&mut self.code)
    }

    /// Declare a fresh local; returns its index.
    pub(crate) fn local(&mut self, t: ValType) -> u32 {
        self.locals.push(t);
        self.nparams + self.locals.len() as u32 - 1
    }

    pub(crate) fn n_locals(&self) -> u32 {
        self.nparams + self.locals.len() as u32
    }

    pub(crate) fn finish(mut self) -> Function {
        self.s().end();
        let mut f = Function::new_with_locals_types(self.locals.iter().copied());
        f.raw(self.code.iter().copied());
        f
    }

    // ---- memory ----
    fn ma(offset: u32, m: Mem) -> MemArg {
        MemArg {
            offset: u64::from(offset),
            align: m.align_log2(),
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        }
    }

    pub(crate) fn load(&mut self, m: Mem, offset: u32) -> &mut Self {
        let a = Self::ma(offset, m);
        let mut s = self.s();
        match m {
            Mem::U8 => s.i32_load8_u(a),
            Mem::I8 => s.i32_load8_s(a),
            Mem::U16 => s.i32_load16_u(a),
            Mem::I16 => s.i32_load16_s(a),
            Mem::I32 => s.i32_load(a),
            Mem::I64 => s.i64_load(a),
            Mem::F32 => s.f32_load(a),
            Mem::F64 => s.f64_load(a),
        };
        self
    }

    pub(crate) fn store(&mut self, m: Mem, offset: u32) -> &mut Self {
        let a = Self::ma(offset, m);
        let mut s = self.s();
        match m {
            Mem::U8 | Mem::I8 => s.i32_store8(a),
            Mem::U16 | Mem::I16 => s.i32_store16(a),
            Mem::I32 => s.i32_store(a),
            Mem::I64 => s.i64_store(a),
            Mem::F32 => s.f32_store(a),
            Mem::F64 => s.f64_store(a),
        };
        self
    }

    // ---- tiny conveniences ----
    pub(crate) fn get(&mut self, l: u32) -> &mut Self {
        self.s().local_get(l);
        self
    }
    pub(crate) fn set(&mut self, l: u32) -> &mut Self {
        self.s().local_set(l);
        self
    }
    pub(crate) fn tee(&mut self, l: u32) -> &mut Self {
        self.s().local_tee(l);
        self
    }
    pub(crate) fn i32c(&mut self, v: i32) -> &mut Self {
        self.s().i32_const(v);
        self
    }
    pub(crate) fn i64c(&mut self, v: i64) -> &mut Self {
        self.s().i64_const(v);
        self
    }
}
