//! Exhaustiveness of `switch` by Maranget's usefulness algorithm
//! ("Warnings for pattern matching", JFP 2007), the algorithm rustc and OCaml use.
//!
//! The patterns that occur in OxiScript are *nullary constructor* patterns
//! (a union variant, an enum member, `null`, `true`/`false`) and wildcards
//! (`default`), so every constructor has arity 0 and the matrix has a single
//! column. The code below is the general algorithm (specialisation and default
//! matrices over a column signature) so nested patterns can be added without
//! changing the callers.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pat {
    Wild,
    /// Constructor number (dense from 0) with sub-patterns.
    Ctor(u32, Vec<Pat>),
}

/// The constructors a column can contain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColSig {
    /// Constructors `0..n` (union variants, enum members, `{null, non-null}`, `{false, true}`).
    Finite(u32),
    /// Unbounded (integers, strings): only a wildcard is exhaustive.
    Infinite,
}

type Row = Vec<Pat>;

fn specialize(rows: &[Row], c: u32) -> Vec<Row> {
    let mut out = Vec::new();
    for r in rows {
        match &r[0] {
            Pat::Ctor(k, sub) if *k == c => {
                let mut n = sub.clone();
                n.extend_from_slice(&r[1..]);
                out.push(n);
            }
            Pat::Wild => out.push(r[1..].to_vec()),
            Pat::Ctor(..) => {}
        }
    }
    out
}

fn default_matrix(rows: &[Row]) -> Vec<Row> {
    rows.iter()
        .filter(|r| r[0] == Pat::Wild)
        .map(|r| r[1..].to_vec())
        .collect()
}

/// Is vector `q` useful with respect to `rows` (does it match a value no row matches)?
/// `sigs[i]` is the signature of column `i`; constructors are nullary here, so the
/// signature list only ever shrinks from the front.
pub fn useful(rows: &[Row], q: &[Pat], sigs: &[ColSig]) -> bool {
    if q.is_empty() {
        return rows.is_empty();
    }
    match &q[0] {
        Pat::Ctor(c, sub) => {
            let mut nq = sub.clone();
            nq.extend_from_slice(&q[1..]);
            useful(&specialize(rows, *c), &nq, &sigs[1..])
        }
        Pat::Wild => {
            let used: Vec<u32> = rows
                .iter()
                .filter_map(|r| match &r[0] {
                    Pat::Ctor(k, _) => Some(*k),
                    Pat::Wild => None,
                })
                .collect();
            if let ColSig::Finite(n) = sigs[0] {
                let complete = (0..n).all(|c| used.contains(&c));
                if complete {
                    return (0..n).any(|c| {
                        let mut nq = vec![Pat::Wild; 0];
                        nq.extend_from_slice(&q[1..]);
                        useful(&specialize(rows, c), &nq, &sigs[1..])
                    });
                }
            }
            useful(&default_matrix(rows), &q[1..], &sigs[1..])
        }
    }
}

/// Constructors of a single column that no row covers; `None` entries are not
/// representable (infinite signature: report "other values").
pub fn missing(rows_single_col: &[Pat], sig: ColSig) -> Vec<u32> {
    let rows: Vec<Row> = rows_single_col.iter().map(|p| vec![p.clone()]).collect();
    match sig {
        ColSig::Finite(n) => (0..n)
            .filter(|c| useful(&rows, &[Pat::Ctor(*c, vec![])], &[sig]))
            .collect(),
        ColSig::Infinite => Vec::new(),
    }
}

/// Exhaustive iff the wildcard is not useful.
pub fn is_exhaustive(rows_single_col: &[Pat], sig: ColSig) -> bool {
    let rows: Vec<Row> = rows_single_col.iter().map(|p| vec![p.clone()]).collect();
    !useful(&rows, &[Pat::Wild], &[sig])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn c(n: u32) -> Pat {
        Pat::Ctor(n, vec![])
    }
    #[test]
    fn basics() {
        assert!(is_exhaustive(&[c(0), c(1)], ColSig::Finite(2)));
        assert!(!is_exhaustive(&[c(0)], ColSig::Finite(2)));
        assert_eq!(missing(&[c(0), c(2)], ColSig::Finite(4)), vec![1, 3]);
        assert!(is_exhaustive(&[c(0), Pat::Wild], ColSig::Finite(9)));
        assert!(!is_exhaustive(&[c(1)], ColSig::Infinite));
        assert!(is_exhaustive(&[Pat::Wild], ColSig::Infinite));
    }
    #[test]
    fn two_columns() {
        let rows = vec![vec![c(0), c(0)], vec![c(0), c(1)], vec![c(1), Pat::Wild]];
        assert!(!useful(&rows, &[Pat::Wild, Pat::Wild], &[ColSig::Finite(2), ColSig::Finite(2)]));
        let rows = vec![vec![c(0), c(0)], vec![c(1), Pat::Wild]];
        assert!(useful(&rows, &[Pat::Wild, Pat::Wild], &[ColSig::Finite(2), ColSig::Finite(2)]));
    }
}
