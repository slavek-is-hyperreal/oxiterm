//! Statistics for OxiPerf (methodology R16): median, percentiles, hierarchical bootstrap
//! (resample invocations, then iterations inside each chosen invocation) with a seeded PRNG,
//! so the same raw data always gives the same interval.

/// Small deterministic PRNG (splitmix64). No external dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `0..n` (modulo bias is below 2^-40 for the sizes used here).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

fn sorted(v: &[f64]) -> Vec<f64> {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in measurements"));
    s
}

/// Median (mean of the two middle values for an even count). NaN for an empty slice.
pub fn median(v: &[f64]) -> f64 {
    percentile(v, 50.0)
}

/// Percentile `p` in 0..=100 with linear interpolation between ranks. NaN for an empty slice.
pub fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let s = sorted(v);
    let rank = (p / 100.0) * (s.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    s[lo] + (s[hi] - s[lo]) * (rank - lo as f64)
}

pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

/// Coefficient of variation (sample standard deviation / mean).
pub fn cv(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    let var = v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (v.len() - 1) as f64;
    var.sqrt() / m
}

pub fn geomean(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    (v.iter().map(|x| x.ln()).sum::<f64>() / v.len() as f64).exp()
}

/// One hierarchical resample: pick `groups.len()` invocations with replacement, then for each
/// picked invocation pick as many iterations as it has, with replacement. Pooled median.
fn resample_median(groups: &[Vec<f64>], rng: &mut Rng, scratch: &mut Vec<f64>) -> f64 {
    scratch.clear();
    for _ in 0..groups.len() {
        let g = &groups[rng.below(groups.len())];
        for _ in 0..g.len() {
            scratch.push(g[rng.below(g.len())]);
        }
    }
    median(scratch)
}

fn interval(mut samples: Vec<f64>) -> (f64, f64) {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let at = |p: f64| samples[((p * (samples.len() - 1) as f64).round()) as usize];
    (at(0.025), at(0.975))
}

/// 95% interval of the pooled median by hierarchical bootstrap. `groups[i]` = iteration times of
/// invocation `i`. Deterministic for a given `seed`.
pub fn bootstrap_median_ci(groups: &[Vec<f64>], resamples: usize, seed: u64) -> (f64, f64) {
    assert!(!groups.is_empty() && groups.iter().all(|g| !g.is_empty()));
    let mut rng = Rng::new(seed);
    let mut scratch = Vec::new();
    interval((0..resamples).map(|_| resample_median(groups, &mut rng, &mut scratch)).collect())
}

/// 95% interval of `median(a) / median(b)` (independent hierarchical resampling of both sides).
pub fn bootstrap_ratio_ci(a: &[Vec<f64>], b: &[Vec<f64>], resamples: usize, seed: u64) -> (f64, f64) {
    assert!(!a.is_empty() && !b.is_empty());
    let mut rng = Rng::new(seed);
    let mut scratch = Vec::new();
    interval(
        (0..resamples)
            .map(|_| {
                let x = resample_median(a, &mut rng, &mut scratch);
                let y = resample_median(b, &mut rng, &mut scratch);
                x / y
            })
            .collect(),
    )
}

/// Warm-up classification (cheap stand-in for the Barrett et al. changepoint test, which is TODO):
/// compares the median of the first and last third of the measured iterations of each invocation.
/// `steady` = within 5%, `slowdown`/`speedup` otherwise.
pub fn drift_status(iters: &[f64]) -> &'static str {
    if iters.len() < 9 {
        return "too_few_iterations";
    }
    let third = iters.len() / 3;
    let first = median(&iters[..third]);
    let last = median(&iters[iters.len() - third..]);
    let r = last / first;
    if r > 1.05 {
        "slowdown"
    } else if r < 0.95 {
        "speedup"
    } else {
        "steady"
    }
}

/// Variance components of a two-level experiment (Kalibera & Jones): between invocations and
/// within an invocation. `groups[i]` = iteration times of invocation `i` (all the same length `k`).
pub fn variance_components(groups: &[Vec<f64>]) -> (f64, f64) {
    let k = groups[0].len() as f64;
    let means: Vec<f64> = groups.iter().map(|g| mean(g)).collect();
    let within = groups.iter().map(|g| var(g)).sum::<f64>() / groups.len() as f64;
    let between = (var(&means) - within / k).max(0.0);
    (between, within)
}

fn var(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (v.len() - 1) as f64
}

/// Bounds of a dimensioning (they are part of the frozen plan and recorded in the manifest).
#[derive(Clone, Copy, Debug)]
pub struct PlanBounds {
    pub min_invocations: usize,
    pub max_invocations: usize,
    pub min_iters: usize,
    pub max_iters: usize,
    /// Wanted half-width of the 95% interval of the mean, relative to the mean.
    pub target_rel_halfwidth: f64,
}

/// Repetition counts derived from a measured pilot (Kalibera & Jones, ISMM 2013): the iterations
/// per invocation balance the cost of an invocation (`cost_invocation_ns`: process start, warm-up)
/// against the cost of an iteration, weighted by the variance ratio
/// (`n = sqrt(c_inv / c_iter * s_within^2 / s_between^2)`); the number of invocations then follows
/// from the wanted precision. Returns `(invocations, iterations_per_invocation)` clamped to `b`.
pub fn plan_counts(pilot: &[Vec<f64>], cost_invocation_ns: f64, b: PlanBounds) -> (usize, usize) {
    let (s_b, s_w) = variance_components(pilot);
    let all: Vec<f64> = pilot.iter().flatten().copied().collect();
    let m = mean(&all);
    let c_iter = m.max(1.0);
    let n_it = if s_b <= f64::EPSILON * m * m {
        b.max_iters
    } else {
        ((cost_invocation_ns.max(c_iter) / c_iter * s_w / s_b).sqrt().ceil() as usize).clamp(b.min_iters, b.max_iters)
    };
    let target = b.target_rel_halfwidth * m;
    let inv = if target <= 0.0 { b.max_invocations } else { ((1.96 * 1.96 * (s_b + s_w / n_it as f64) / (target * target)).ceil() as usize).clamp(b.min_invocations, b.max_invocations) };
    (inv, n_it)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_odd_even_empty() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert!(median(&[]).is_nan());
        assert_eq!(median(&[7.0]), 7.0);
    }

    #[test]
    fn percentiles_interpolate() {
        let v: Vec<f64> = (1..=5).map(f64::from).collect();
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 100.0), 5.0);
        assert_eq!(percentile(&v, 25.0), 2.0);
        assert!((percentile(&v, 95.0) - 4.8).abs() < 1e-12);
    }

    #[test]
    fn mean_cv_geomean() {
        assert_eq!(mean(&[1.0, 2.0, 3.0]), 2.0);
        assert!((cv(&[2.0, 4.0]) - (2.0f64.sqrt() / 3.0)).abs() < 1e-12);
        assert!((geomean(&[1.0, 4.0]) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn rng_is_deterministic() {
        let (mut a, mut b) = (Rng::new(42), Rng::new(42));
        for _ in 0..10 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }

    fn data() -> Vec<Vec<f64>> {
        vec![
            vec![10.0, 11.0, 10.5, 12.0, 10.2, 11.5],
            vec![10.1, 10.9, 10.4, 11.8, 10.3, 11.1],
            vec![9.9, 11.2, 10.6, 12.1, 10.0, 11.4],
        ]
    }

    #[test]
    fn bootstrap_is_deterministic_for_a_fixed_seed() {
        let d = data();
        let a = bootstrap_median_ci(&d, 2000, 7);
        let b = bootstrap_median_ci(&d, 2000, 7);
        assert_eq!(a, b);
        let pooled: Vec<f64> = d.iter().flatten().copied().collect();
        let m = median(&pooled);
        assert!(a.0 <= m && m <= a.1, "{a:?} must contain the median {m}");
        assert!(a.0 < a.1);
    }

    #[test]
    fn bootstrap_of_constant_data_is_a_point() {
        let d = vec![vec![5.0; 10], vec![5.0; 10]];
        assert_eq!(bootstrap_median_ci(&d, 500, 1), (5.0, 5.0));
        let e = vec![vec![10.0; 10], vec![10.0; 10]];
        assert_eq!(bootstrap_ratio_ci(&e, &d, 500, 1), (2.0, 2.0));
    }

    #[test]
    fn ratio_ci_separates_a_real_difference_from_noise() {
        let d = data();
        let slow: Vec<Vec<f64>> = d.iter().map(|g| g.iter().map(|x| x * 2.0).collect()).collect();
        let (lo, hi) = bootstrap_ratio_ci(&slow, &d, 2000, 3);
        assert!(lo > 1.5 && hi < 2.5, "{lo} {hi}");
        let (lo, hi) = bootstrap_ratio_ci(&d, &d, 2000, 3);
        assert!(lo < 1.0 && hi > 1.0, "same data must straddle 1.0: {lo} {hi}");
    }

    #[test]
    fn drift_classification() {
        assert_eq!(drift_status(&[1.0; 12]), "steady");
        let slowing: Vec<f64> = (0..12).map(|i| 1.0 + i as f64 * 0.1).collect();
        assert_eq!(drift_status(&slowing), "slowdown");
        let warming: Vec<f64> = (0..12).map(|i| 2.0 - i as f64 * 0.1).collect();
        assert_eq!(drift_status(&warming), "speedup");
        assert_eq!(drift_status(&[1.0, 2.0]), "too_few_iterations");
    }

    #[test]
    fn variance_components_separate_the_levels() {
        // identical invocations with noise inside: no between variance
        let g = vec![vec![1.0, 3.0, 1.0, 3.0], vec![1.0, 3.0, 1.0, 3.0]];
        let (b, w) = variance_components(&g);
        assert_eq!(b, 0.0);
        assert!(w > 1.0);
        // constant inside, different between: no within variance
        let g = vec![vec![1.0; 4], vec![3.0; 4], vec![2.0; 4]];
        let (b, w) = variance_components(&g);
        assert_eq!(w, 0.0);
        assert!(b > 0.5);
    }

    const B: PlanBounds = PlanBounds { min_invocations: 3, max_invocations: 10, min_iters: 10, max_iters: 50, target_rel_halfwidth: 0.02 };

    #[test]
    fn plan_uses_max_iterations_when_invocations_do_not_differ() {
        let g = vec![vec![10.0, 12.0, 11.0, 10.5], vec![10.0, 12.0, 11.0, 10.5]];
        assert_eq!(plan_counts(&g, 1e9, B).1, 50);
    }

    #[test]
    fn plan_uses_fewer_iterations_and_more_invocations_when_invocations_differ() {
        let noisy = vec![vec![10.0, 10.1, 10.0, 10.1], vec![14.0, 14.1, 14.0, 14.1], vec![8.0, 8.1, 8.0, 8.1]];
        let (inv, it) = plan_counts(&noisy, 20.0, B); // cheap invocations, big between variance
        assert_eq!(it, 10, "clamped to the minimum");
        assert_eq!(inv, 10, "clamped to the maximum: precision cannot be reached");
        // precise data needs only the minimum
        let quiet = vec![vec![10.0, 10.001, 10.0, 10.001]; 3];
        assert_eq!(plan_counts(&quiet, 20.0, B).0, 3);
    }

    #[test]
    fn plan_is_deterministic() {
        let g = vec![vec![10.0, 12.0, 11.0], vec![11.0, 13.0, 12.0]];
        assert_eq!(plan_counts(&g, 5e6, B), plan_counts(&g, 5e6, B));
    }
}
