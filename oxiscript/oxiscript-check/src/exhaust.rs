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

#[cfg(test)]
type Row = Vec<Pat>;

#[cfg(test)]
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

#[cfg(test)]
fn default_matrix(rows: &[Row]) -> Vec<Row> {
    rows.iter()
        .filter(|r| r[0] == Pat::Wild)
        .map(|r| r[1..].to_vec())
        .collect()
}

/// Is vector `q` useful with respect to `rows` (does it match a value no row matches)?
/// `sigs[i]` is the signature of column `i`; constructors are nullary here, so the
/// signature list only ever shrinks from the front.
#[cfg(test)]
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
            let used: std::collections::HashSet<u32> = rows
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
    // Single nullary column: `useful(rows, [Ctor c])` holds iff no row is `c` or a wildcard.
    // This linear form is the same algorithm specialised to one column (checked against
    // the general `useful` in the tests), so a 20 000-member enum stays linear.
    match sig {
        ColSig::Finite(n) => {
            if rows_single_col.contains(&Pat::Wild) {
                return Vec::new();
            }
            let covered: std::collections::HashSet<u32> = rows_single_col
                .iter()
                .filter_map(|p| if let Pat::Ctor(k, _) = p { Some(*k) } else { None })
                .collect();
            (0..n).filter(|c| !covered.contains(c)).collect()
        }
        ColSig::Infinite => Vec::new(),
    }
}

/// Exhaustive iff the wildcard is not useful.
pub fn is_exhaustive(rows_single_col: &[Pat], sig: ColSig) -> bool {
    if rows_single_col.contains(&Pat::Wild) {
        return true;
    }
    match sig {
        ColSig::Finite(_) => missing(rows_single_col, sig).is_empty(),
        ColSig::Infinite => false,
    }
}

/// The general algorithm applied to the same question (used by tests as the reference).
#[cfg(test)]
pub fn is_exhaustive_general(rows_single_col: &[Pat], sig: ColSig) -> bool {
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
    fn linear_form_agrees_with_the_general_algorithm() {
        let mut seed = 12345u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        for _ in 0..500 {
            let n = next() % 6 + 1;
            let k = next() % 8;
            let rows: Vec<Pat> = (0..k)
                .map(|_| if next() % 7 == 0 { Pat::Wild } else { c(next() % (n + 1)) })
                .collect();
            let sig = if next() % 5 == 0 { ColSig::Infinite } else { ColSig::Finite(n) };
            let rows: Vec<Pat> = rows
                .into_iter()
                .filter(|p| match (p, sig) {
                    (Pat::Ctor(x, _), ColSig::Finite(n)) => *x < n,
                    (Pat::Ctor(..), ColSig::Infinite) => false,
                    _ => true,
                })
                .collect();
            assert_eq!(is_exhaustive(&rows, sig), is_exhaustive_general(&rows, sig), "{rows:?} {sig:?}");
            if let ColSig::Finite(_) = sig {
                let miss = missing(&rows, sig);
                for ctor in 0..n {
                    let r2: Vec<Vec<Pat>> = rows.iter().map(|p| vec![p.clone()]).collect();
                    assert_eq!(miss.contains(&ctor), useful(&r2, &[c(ctor)], &[sig]));
                }
            }
        }
    }
    #[test]
    fn two_columns() {
        let rows = vec![vec![c(0), c(0)], vec![c(0), c(1)], vec![c(1), Pat::Wild]];
        assert!(!useful(&rows, &[Pat::Wild, Pat::Wild], &[ColSig::Finite(2), ColSig::Finite(2)]));
        let rows = vec![vec![c(0), c(0)], vec![c(1), Pat::Wild]];
        assert!(useful(&rows, &[Pat::Wild, Pat::Wild], &[ColSig::Finite(2), ColSig::Finite(2)]));
    }
}
