//! A tiny builder for producing IR programmatically (what slice 8.L's lowering
//! and test generators use). It assigns [`ValueId`]s in call order, so to get
//! canonical numbering (and `parse(print(f)) == f`) call it in textual order:
//! parameters first; then per block, block parameters before instructions.

use crate::types::*;

pub struct FuncBuilder {
    f: Func,
}

impl FuncBuilder {
    pub fn new(name: &str) -> FuncBuilder {
        FuncBuilder {
            f: Func {
                name: name.into(),
                export: false,
                attrs: vec![],
                params: vec![],
                rets: vec![],
                values: vec![],
                blocks: vec![],
            },
        }
    }

    #[must_use]
    pub fn export(mut self) -> Self {
        self.f.export = true;
        self
    }

    #[must_use]
    pub fn rets(mut self, rets: Vec<Ty>) -> Self {
        self.f.rets = rets;
        self
    }

    #[must_use]
    pub fn attr(mut self, name: &str, args: &[&str]) -> Self {
        self.f.attrs.push(Attr {
            name: name.into(),
            args: args.iter().map(ToString::to_string).collect(),
        });
        self
    }

    fn new_value(&mut self, name: &str, ty: Ty) -> ValueId {
        let id = ValueId(self.f.values.len() as u32);
        self.f.values.push(ValueInfo {
            name: name.into(),
            ty,
        });
        id
    }

    pub fn param(&mut self, name: &str, ty: Ty, own: bool) -> ValueId {
        let value = self.new_value(name, ty);
        self.f.params.push(Param { value, own });
        value
    }

    /// Add a block (terminator is a placeholder `trap unreachable` until [`Self::terminate`]).
    pub fn block(&mut self, label: &str) -> BlockId {
        self.f.blocks.push(Block {
            label: label.into(),
            params: vec![],
            insts: vec![],
            term: Terminator::Trap(TrapKind::Unreachable),
            term_loc: None,
        });
        BlockId(self.f.blocks.len() as u32 - 1)
    }

    pub fn block_param(&mut self, b: BlockId, name: &str, ty: Ty) -> ValueId {
        let v = self.new_value(name, ty);
        self.f.blocks[b.0 as usize].params.push(v);
        v
    }

    /// Append an instruction defining `results` (name, type) pairs.
    pub fn inst(&mut self, b: BlockId, op: Op, results: &[(&str, Ty)]) -> Vec<ValueId> {
        let ids: Vec<ValueId> = results
            .iter()
            .map(|(n, t)| self.new_value(n, t.clone()))
            .collect();
        self.f.blocks[b.0 as usize].insts.push(Inst {
            results: ids.clone(),
            op,
            loc: None,
        });
        ids
    }

    /// Single-result convenience.
    pub fn inst1(&mut self, b: BlockId, op: Op, name: &str, ty: Ty) -> ValueId {
        self.inst(b, op, &[(name, ty)])[0]
    }

    pub fn terminate(&mut self, b: BlockId, term: Terminator) {
        self.f.blocks[b.0 as usize].term = term;
    }

    pub fn finish(self) -> Func {
        self.f
    }
}

/// Branch target helper.
pub fn target(block: BlockId, args: &[ValueId]) -> Target {
    Target {
        block,
        args: args.to_vec(),
    }
}

/// Renumber a function's values into canonical textual order (parameters,
/// then per block its parameters and instruction results) and rewrite every
/// operand. **Verification requires canonical numbering**, so any pass that
/// inserts, removes or reorders value definitions or blocks must call this
/// afterwards. Value *names* are kept.
pub fn renumber(f: &mut Func) {
    let mut map = vec![ValueId(0); f.values.len()];
    let mut vals = Vec::with_capacity(f.values.len());
    let mut take = |old: ValueId, vals: &mut Vec<ValueInfo>, values: &[ValueInfo]| {
        map[old.0 as usize] = ValueId(vals.len() as u32);
        vals.push(values[old.0 as usize].clone());
    };
    let old_values = f.values.clone();
    for p in &f.params {
        take(p.value, &mut vals, &old_values);
    }
    for b in &f.blocks {
        for p in &b.params {
            take(*p, &mut vals, &old_values);
        }
        for i in &b.insts {
            for r in &i.results {
                take(*r, &mut vals, &old_values);
            }
        }
    }
    let m = |v: &mut ValueId| *v = map[v.0 as usize];
    for p in &mut f.params {
        m(&mut p.value);
    }
    for b in &mut f.blocks {
        b.params.iter_mut().for_each(m);
        for i in &mut b.insts {
            i.results.iter_mut().for_each(m);
            for o in i.op.operands_mut() {
                m(o);
            }
        }
        for o in b.term.operands_mut() {
            m(o);
        }
    }
    f.values = vals;
}
