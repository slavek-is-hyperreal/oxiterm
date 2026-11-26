//! BAD ACTOR: manifest `target`/`features` parser properties and target table consistency.
#![allow(clippy::pedantic)]

use oxiscript_abi::manifest::Manifest;
use oxiscript_abi::target::{Feature, Target};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[(self.next() % v.len() as u64) as usize]
    }
}

#[test]
fn every_target_feature_subset_roundtrips_or_is_refused() {
    for t in Target::ALL {
        for mask in 0u32..(1 << Feature::ALL.len()) {
            let fs: Vec<Feature> = Feature::ALL.iter().enumerate().filter(|(i, _)| (mask >> i) & 1 == 1).map(|(_, f)| *f).collect();
            let mut m = Manifest::new(2026, "default");
            m.target = t;
            m.features = fs.clone();
            let text = m.print();
            let allowed = fs.iter().all(|f| t.allows(*f));
            match Manifest::parse(&text) {
                Ok(back) => {
                    assert!(allowed, "{t} {fs:?} parsed although a feature is not allowed:\n{text}");
                    assert_eq!(back.target, t);
                    assert_eq!(back.features, fs);
                    assert_eq!(back.print(), text, "print is not canonical");
                }
                Err(e) => assert!(!allowed, "{t} {fs:?} refused: {e}\n{text}"),
            }
        }
    }
}

#[test]
fn target_table_is_consistent() {
    for f in Feature::ALL {
        assert_eq!(Feature::parse(f.name()), Some(f));
        for t in Target::ALL {
            assert_eq!(t.allows(f), t >= f.min_target(), "{t} {f}");
            assert_eq!(t.features().contains(&f), t.allows(f));
        }
    }
    for w in Target::ALL.windows(2) {
        for f in w[0].features() {
            assert!(w[1].features().contains(&f), "{} lost {f}", w[1]);
        }
    }
    for t in Target::ALL {
        assert_eq!(Target::parse(t.name()), Some(t));
    }
    assert_eq!(Target::default(), Target::Device);
}

/// Token-soup fuzz: never panics; every accepted manifest satisfies the invariants and is idempotent under print/parse.
#[test]
fn token_soup_never_panics_and_accepted_manifests_are_sane() {
    let head = "oxi-manifest 1\nabi 1.0\nedition 2026\nprofile default\n";
    let words = [
        "target", "features", "web", "server", "device", "simd128", "exceptions", "tail-call", "extended-const", "memory64", "multi-memory", "needs", "host:log@1", "Log", "\n", "\n", " ", "\t", "\r\n", "\u{00A0}", "\u{3000}", "\u{2028}", "#", "", "Web", "tail_call", "-", "\0", "profile", "abi", "1.0", "gc", "threads", "relaxed-simd", "trunc_sat",
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut ok, mut total) = (0, 0);
    for _ in 0..200_000 {
        let mut s = String::from(head);
        for _ in 0..(rng.next() % 12) {
            s.push_str(rng.pick(&words));
            if !rng.next().is_multiple_of(3) {
                s.push(' ');
            }
        }
        total += 1;
        if let Ok(m) = Manifest::parse(&s) {
            ok += 1;
            assert!(m.features.iter().all(|f| m.target.allows(*f)), "accepted a feature above its target:\n{s:?}");
            let mut sorted = m.features.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted, m.features, "{s:?}");
            let p = m.print();
            let again = Manifest::parse(&p).unwrap_or_else(|e| panic!("print of an accepted manifest does not reparse: {e}\n{s:?}\n{p}"));
            assert_eq!(again.print(), p);
            assert_eq!(again.target, m.target);
            assert_eq!(again.features, m.features);
        }
    }
    println!("token soup: {ok}/{total} accepted");
    assert!(ok > 100, "fuzz vocabulary too weak: {ok}");
}
