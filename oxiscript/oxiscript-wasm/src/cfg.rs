//! Control-flow analysis for the structured-control-flow lowering
//! ("Beyond Relooper", Ramsey 2022): reverse postorder, dominators
//! (Cooper-Harvey-Kennedy), loop headers, merge nodes, reducibility.

use oxiscript_ir::{Func, Terminator};

pub(crate) const NONE: usize = usize::MAX;

pub(crate) struct Cfg {
    /// Reachable blocks in reverse postorder.
    pub rpo: Vec<usize>,
    /// RPO number per block (`NONE` = unreachable).
    pub num: Vec<usize>,
    #[allow(dead_code)]
    pub idom: Vec<usize>,
    /// Dominator-tree children (reachable only).
    pub children: Vec<Vec<usize>>,
    /// Number of forward in-edges (edges counted individually).
    pub fwd_in: Vec<u32>,
    pub is_header: Vec<bool>,
    /// Blocks that must be branch-table targets (forced labels).
    pub forced: Vec<bool>,
    /// Size of the dominator subtree (including the block).
    pub subtree: Vec<u32>,
}

impl Cfg {
    pub(crate) fn is_merge(&self, b: usize) -> bool {
        self.fwd_in[b] >= 2 || self.forced[b]
    }
}

fn succs(t: &Terminator) -> Vec<usize> {
    t.targets().iter().map(|t| t.block.0 as usize).collect()
}

/// Analyse `f`; `Err(block label)` if the reachable CFG is irreducible.
pub(crate) fn analyze(f: &Func) -> Result<Cfg, String> {
    let n = f.blocks.len();
    // iterative DFS for postorder
    let mut state = vec![0u8; n];
    let mut post: Vec<usize> = Vec::with_capacity(n);
    let mut stack: Vec<(usize, Vec<usize>, usize)> = Vec::new();
    state[0] = 1;
    stack.push((0, succs(&f.blocks[0].term), 0));
    while let Some((b, ss, i)) = stack.last_mut() {
        if *i < ss.len() {
            let s = ss[*i];
            *i += 1;
            if state[s] == 0 {
                state[s] = 1;
                let sv = succs(&f.blocks[s].term);
                stack.push((s, sv, 0));
            }
        } else {
            post.push(*b);
            stack.pop();
        }
    }
    let rpo: Vec<usize> = post.iter().rev().copied().collect();
    let mut num = vec![NONE; n];
    for (i, b) in rpo.iter().enumerate() {
        num[*b] = i;
    }
    // predecessors (reachable only)
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &b in &rpo {
        for s in succs(&f.blocks[b].term) {
            preds[s].push(b);
        }
    }
    // dominators (CHK) on RPO numbers
    let mut idom = vec![NONE; n];
    idom[0] = 0;
    let intersect = |idom: &Vec<usize>, mut a: usize, mut b: usize| -> usize {
        while a != b {
            while num[a] > num[b] {
                a = idom[a];
            }
            while num[b] > num[a] {
                b = idom[b];
            }
        }
        a
    };
    let mut changed = true;
    while changed {
        changed = false;
        for &b in rpo.iter().skip(1) {
            let mut new = NONE;
            for &p in &preds[b] {
                if idom[p] == NONE {
                    continue;
                }
                new = if new == NONE { p } else { intersect(&idom, p, new) };
            }
            if new != idom[b] {
                idom[b] = new;
                changed = true;
            }
        }
    }
    let mut children = vec![Vec::new(); n];
    for &b in rpo.iter().skip(1) {
        children[idom[b]].push(b);
    }
    // tin/tout over the dominator tree + subtree sizes (iterative)
    let mut tin = vec![0usize; n];
    let mut tout = vec![0usize; n];
    let mut subtree = vec![1u32; n];
    let mut clock = 0usize;
    let mut st: Vec<(usize, usize)> = vec![(0, 0)];
    tin[0] = 0;
    while let Some((b, i)) = st.last().copied() {
        if i == 0 {
            tin[b] = clock;
            clock += 1;
        }
        if i < children[b].len() {
            st.last_mut().expect("nonempty").1 += 1;
            st.push((children[b][i], 0));
        } else {
            tout[b] = clock;
            st.pop();
            if let Some(&(parent, _)) = st.last() {
                subtree[parent] += subtree[b];
            }
        }
    }
    let dominates = |a: usize, b: usize| tin[a] <= tin[b] && tout[b] <= tout[a];
    let mut fwd_in = vec![0u32; n];
    let mut is_header = vec![false; n];
    for &u in &rpo {
        for v in succs(&f.blocks[u].term) {
            if num[u] < num[v] {
                fwd_in[v] += 1;
            } else if dominates(v, u) {
                is_header[v] = true;
            } else {
                return Err(f.blocks[v].label.clone());
            }
        }
    }
    Ok(Cfg {
        rpo,
        num,
        idom,
        children,
        fwd_in,
        is_header,
        forced: vec![false; n],
        subtree,
    })
}
