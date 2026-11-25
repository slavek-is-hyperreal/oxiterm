//! A minimal IR function builder: values and blocks are created in any order
//! (`build::renumber` makes the numbering canonical at the end), instructions
//! carry a source location, and the current insertion block is part of the state.

use oxiscript_ir::build::renumber;
use oxiscript_ir::{Attr, Block, BlockId, Func, Inst, Loc, Op, Param, Target, Terminator, Ty as It, ValueId, ValueInfo};

pub(crate) struct Fb {
    pub f: Func,
    pub cur: BlockId,
    pub loc: Option<Loc>,
}

impl Fb {
    pub fn new(name: &str) -> Fb {
        let mut fb = Fb {
            f: Func {
                name: name.to_string(),
                export: false,
                attrs: Vec::new(),
                params: Vec::new(),
                rets: Vec::new(),
                values: Vec::new(),
                blocks: Vec::new(),
            },
            cur: BlockId(0),
            loc: None,
        };
        fb.cur = fb.new_block("entry");
        fb
    }

    fn value(&mut self, hint: &str, ty: It) -> ValueId {
        let id = ValueId(self.f.values.len() as u32);
        self.f.values.push(ValueInfo {
            name: format!("{hint}{}", id.0),
            ty,
        });
        id
    }

    pub fn param(&mut self, hint: &str, ty: It, own: bool) -> ValueId {
        let value = self.value(hint, ty);
        self.f.params.push(Param { value, own });
        value
    }

    pub fn new_block(&mut self, hint: &str) -> BlockId {
        let id = BlockId(self.f.blocks.len() as u32);
        self.f.blocks.push(Block {
            label: format!("{hint}{}", id.0),
            params: Vec::new(),
            insts: Vec::new(),
            term: Terminator::Trap(oxiscript_ir::TrapKind::Unreachable),
            term_loc: None,
        });
        id
    }

    pub fn block_param(&mut self, b: BlockId, hint: &str, ty: It) -> ValueId {
        let v = self.value(hint, ty);
        self.f.blocks[b.0 as usize].params.push(v);
        v
    }

    pub fn emit(&mut self, op: Op, tys: &[It]) -> Vec<ValueId> {
        let results: Vec<ValueId> = tys.iter().map(|t| self.value("v", t.clone())).collect();
        let loc = self.loc;
        self.f.blocks[self.cur.0 as usize].insts.push(Inst {
            results: results.clone(),
            op,
            loc,
        });
        results
    }

    pub fn emit1(&mut self, op: Op, ty: It) -> ValueId {
        self.emit(op, &[ty])[0]
    }

    pub fn emit0(&mut self, op: Op) {
        self.emit(op, &[]);
    }

    pub fn term(&mut self, t: Terminator) {
        let b = &mut self.f.blocks[self.cur.0 as usize];
        b.term = t;
        b.term_loc = self.loc;
    }

    pub fn ty(&self, v: ValueId) -> &It {
        &self.f.values[v.0 as usize].ty
    }

    pub fn is_managed(&self, v: ValueId) -> bool {
        self.ty(v).is_managed()
    }

    pub fn attr(&mut self, name: &str, args: &[&str]) {
        self.f.attrs.push(Attr {
            name: name.to_string(),
            args: args.iter().map(ToString::to_string).collect(),
        });
    }

    pub fn finish(mut self) -> Func {
        renumber(&mut self.f);
        self.f
    }
}

pub(crate) fn tgt(block: BlockId, args: &[ValueId]) -> Target {
    Target {
        block,
        args: args.to_vec(),
    }
}
