//! Seeded random DisplayList fuzzer (ignored by default). Run (release + overflow checks):
//!   CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true \
//!   cargo test -p oxiterm-fb --release --test bad_actor_fuzz --no-run
//!   (ulimit -v 4000000; FUZZ_CASES=200000 FUZZ_SEED=1 FUZZ_KNOWN=0 <test binary> --ignored --nocapture)
//! FUZZ_KNOWN=1 lets the already-reported classes through (huge border, NaN shaped y, huge size).
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use oxiterm_fb::Rasterizer;
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{AnsiColor, BorderChars};
use oxiterm_renderer::render::display_list::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 { self.next() % n.max(1) }
    fn coin(&mut self, p: u64) -> bool { self.below(100) < p }
    fn f(&mut self, known: bool) -> f32 {
        match self.below(14) {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => f32::NEG_INFINITY,
            3 => 0.0,
            4 => -0.0,
            5 => 16_777_216.0 + (self.below(5) as f32 - 2.0),
            6 => 1e20,
            7 => -1e20,
            8 => f32::MAX,
            9 => f32::MIN_POSITIVE,
            10 => (self.below(200) as f32 - 50.0) / 3.0,
            _ => (self.below(4000) as f32 - 1000.0) / 40.0,
        }
        .pipe(|v| v)
        .clamp_known(known)
    }
}
trait P { fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R where Self: Sized { f(self) } fn clamp_known(self, k: bool) -> Self; }
impl P for f32 { fn clamp_known(self, _k: bool) -> f32 { self } }

fn color(r: &mut Rng) -> AnsiColor {
    match r.below(3) { 0 => AnsiColor::Reset, 1 => AnsiColor::Color256(r.next() as u8), _ => AnsiColor::TrueColor(r.next() as u8, r.next() as u8, r.next() as u8) }
}
fn ch(r: &mut Rng) -> char {
    const P: [char; 14] = ['A', ' ', '\0', '中', '\u{301}', '😀', 'ﬃ', '─', '╭', '+', 'é', '\u{200d}', '\u{fffd}', '\n'];
    if r.coin(70) { P[r.below(14) as usize] } else { char::from_u32(r.below(0x11_0000) as u32).unwrap_or('x') }
}
fn rect(r: &mut Rng, k: bool) -> RectF {
    RectF::new(r.f(k), r.f(k), r.f(k), r.f(k))
}
fn small_rect(r: &mut Rng) -> RectF {
    RectF::new((r.below(40) as f32) - 5.0, (r.below(20) as f32) - 5.0, r.below(30) as f32 / 2.0, r.below(12) as f32 / 2.0)
}
fn pt(r: &mut Rng, k: bool) -> PointF { PointF { x: r.f(k), y: r.f(k) } }
fn pq(r: &mut Rng, k: bool) -> PointF { let mut p = pt(r, k); if !k { if !(p.x.abs() < 1e6) { p.x = 1.0 } if !(p.y.abs() < 1e6) { p.y = 1.0 } } p }
fn bchars(r: &mut Rng) -> BorderChars {
    BorderChars { top_left: ch(r), top_right: ch(r), bot_left: ch(r), bot_right: ch(r), top: ch(r), bot: ch(r), left: ch(r), right: ch(r) }
}

fn item(r: &mut Rng, known: bool) -> DisplayItem {
    let wild = r.coin(50);
    let rc = |r: &mut Rng| if wild { rect(r, known) } else { small_rect(r) };
    let node = NodeId(1);
    match r.below(9) {
        0 => DisplayItem::Fill { node, rect: rc(r), radius: r.f(known), fg: color(r), bg: color(r) },
        1 => {
            let mut rr = rc(r);
            if !known { if !(rr.x.abs() < 1e6) { rr.x = 0.5 } if !(rr.y.abs() < 1e6) { rr.y = 0.5 } }
            if !known { rr.w = rr.w.clamp(-1e5, 1e5); rr.h = rr.h.clamp(-1e5, 1e5); if rr.w.is_nan() { rr.w = 3.0 } if rr.h.is_nan() { rr.h = 3.0 } }
            DisplayItem::Border { node, rect: rr, radius: r.f(known), chars: bchars(r), fg: color(r), bg: color(r) }
        }
        2 | 3 => {
            let n = r.below(12) as usize;
            let shaped_on = r.coin(50);
            let glyphs: Vec<Glyph> = (0..n).map(|_| {
                let mut g = Glyph { x: r.f(known), y: r.f(known), ch: ch(r), advance: r.f(known) };
                if !known && shaped_on && g.y.is_nan() { g.y = 0.0 }
                g
            }).collect();
            let shaped = if shaped_on {
                let mut size = if r.coin(60) { 16.0 } else { r.f(known) };
                if !known && !(size <= 400.0 && size >= 0.01) { size = 12.0 }
                let nf = r.below(4) as usize;
                let nc = r.below(4) as usize;
                let mut gf: Vec<(u32, u8)> = (0..nf).map(|_| (r.below(20) as u32, r.next() as u8)).collect();
                gf.sort_by_key(|e| e.0);
                let mut cl: Vec<(u32, Box<str>)> = (0..nc).map(|_| (r.below(20) as u32, format!("{}{}{}", ch(r), ch(r), ch(r)).into())).collect();
                cl.sort_by_key(|e| e.0);
                Some(Box::new(ShapedRun { face: r.next() as u8, size_px: size, line_height: { let v = r.f(known); if known || v.abs() < 1e6 { v } else { 1.0 } }, baseline: { let v = r.f(known); if known || v.abs() < 1e6 { v } else { 0.8 } }, glyph_faces: gf, clusters: cl }))
            } else { None };
            let mut o = pt(r, known);
            if !known && !(o.x.abs() < 1e6) { o.x = 1.0 }
            if !known && !(o.y.abs() < 1e6) { o.y = 1.0 }
            let mut glyphs = glyphs;
            if !known { for g in &mut glyphs { if !(g.x.abs() < 1e6) { g.x = 0.0 } if !(g.y.abs() < 1e6) { g.y = 0.0 } } }
            DisplayItem::Text { node, origin: o, clip: rc(r), glyphs, fg: color(r), bg: color(r),
                mods: Modifiers { bold: r.coin(30), underline: r.coin(30), italic: r.coin(30) }, shaped }
        }
        4 => DisplayItem::Legacy { node, content: rc(r) },
        5 => {
            let mut rr = rc(r);
            if !known { if !(rr.x.abs() < 1e6) { rr.x = 0.5 } if !(rr.y.abs() < 1e6) { rr.y = 0.5 } }
            if !known { rr.w = rr.w.clamp(-100.0, 100.0); rr.h = rr.h.clamp(-100.0, 100.0); if rr.w.is_nan() { rr.w = 3.0 } if rr.h.is_nan() { rr.h = 3.0 } }
            let n = r.below(40) as usize;
            let cells = (0..n).map(|_| GridCell { ch: ch(r), fg: color(r), bg: color(r), mods: Modifiers { bold: r.coin(30), underline: r.coin(30), italic: r.coin(30) }, skip: r.coin(10) }).collect();
            DisplayItem::CellGrid { rect: rr, cells }
        }
        6 => DisplayItem::PushClip { rect: rc(r) },
        7 => DisplayItem::PopClip,
        _ => {
            let n = r.below(10) as usize;
            let cmds = (0..n).map(|_| match r.below(5) {
                0 => PathCmd::MoveTo(pq(r, known)),
                1 => PathCmd::LineTo(pq(r, known)),
                2 => PathCmd::QuadTo(pq(r, known), pq(r, known)),
                3 => PathCmd::CubicTo(pq(r, known), pq(r, known), pq(r, known)),
                _ => PathCmd::Close,
            }).collect();
            let fill = if r.coin(60) { Some(color(r)) } else { None };
            let stroke = if r.coin(60) { let mut w = r.f(known); if !known && !(w.abs() < 1e3) { w = 1.0 } Some((color(r), w)) } else { None };
            DisplayItem::Path { node, cmds, fill, stroke }
        }
    }
}

static CUR: AtomicU64 = AtomicU64::new(0);
static START_MS: AtomicU64 = AtomicU64::new(0);

#[test]
#[ignore]
fn fuzz() {
    let cases: u64 = std::env::var("FUZZ_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(2000);
    let seed0: u64 = std::env::var("FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let known = std::env::var("FUZZ_KNOWN").map(|v| v == "1").unwrap_or(false);
    let t0 = Instant::now();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(200));
        let s = START_MS.load(Ordering::Relaxed);
        let now = t0.elapsed().as_millis() as u64;
        if s != 0 && now.saturating_sub(s) > 8000 {
            eprintln!("HANG seed={}", CUR.load(Ordering::Relaxed));
            std::process::exit(99);
        }
    });
    std::panic::set_hook(Box::new(|i| { if std::env::var("FUZZ_LOC").is_ok() { eprintln!("  at {:?}", i.location()); } }));
    let mut warm = Rasterizer::new(1.0);
    let mut warm_surface = warm.surface_for(30, 10).unwrap();
    let mut bad = 0;
    let mut slow = 0;
    let start: u64 = std::env::var("FUZZ_START").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    for c in start..cases {
        let seed = seed0.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(c).wrapping_add(1) | 1;
        CUR.store(seed, Ordering::Relaxed);
        let mut r = Rng(seed);
        for _ in 0..4 { r.next(); }
        let n = r.below(30) as usize;
        let list = DisplayList { items: (0..n).map(|_| item(&mut r, known)).collect() };
        let scale = [1.0, 0.25, 2.0, 1.5, 16.0, f64::NAN][r.below(6) as usize];
        let t = Instant::now();
        START_MS.store((t0.elapsed().as_millis() as u64).max(1), Ordering::Relaxed);
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut fresh = Rasterizer::new(scale);
            let mut s1 = fresh.surface_for(30, 10).unwrap();
            fresh.render(&list, &mut s1);
            let mut s2 = fresh.surface_for(30, 10).unwrap();
            fresh.render(&list, &mut s2); // warm cache
            assert!(s1.rgba() == s2.rgba(), "NONDETERMINISM warm vs cold");
            if scale == 1.0 {
                warm.render(&list, &mut warm_surface);
                assert!(warm_surface.rgba() == s1.rgba(), "NONDETERMINISM long-lived cache");
            }
            assert!(s1.rgba().chunks_exact(4).all(|p| p[3] == 255), "alpha != 255");
        }));
        START_MS.store(0, Ordering::Relaxed);
        if t.elapsed() > Duration::from_millis(1500) { slow += 1; eprintln!("SLOW seed={seed} {:?}", t.elapsed()); }
        if let Err(e) = res {
            bad += 1;
            let m = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            eprintln!("CRASH seed={seed} scale={scale} {m}");
            let _ = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/claude-1000/-vectorlegis-ssd-pool-oxiterm-oxiterm/ea9c7764-805e-4a30-a817-028d63200a95/scratchpad/badactor-7.2/crash_seeds.txt").and_then(|mut f| writeln!(f, "{seed} {m}"));
            if bad > 40 { break; }
        }
    }
    eprintln!("done cases={cases} crashes={bad} slow={slow}");
    assert_eq!(bad + slow, 0);
}

#[test]
#[ignore]
fn dump_seed() {
    let seed: u64 = std::env::var("FUZZ_ONE").unwrap().parse().unwrap();
    let mut r = Rng(seed);
    for _ in 0..4 { r.next(); }
    let n = r.below(30) as usize;
    let list = DisplayList { items: (0..n).map(|_| item(&mut r, std::env::var("FUZZ_KNOWN").map(|v| v == "1").unwrap_or(false))).collect() };
    for i in &list.items { let one = DisplayList { items: vec![i.clone()] }; eprintln!("TRY {}", format!("{i:?}").chars().take(4000).collect::<String>()); let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { let sc: f64 = std::env::var("FUZZ_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0); let mut ra = Rasterizer::new(sc); let mut su = ra.surface_for(30, 10).unwrap(); let t = Instant::now(); ra.render(&one, &mut su); let e = t.elapsed(); if e > Duration::from_millis(15) { eprintln!("ITEM-SLOW {e:?}"); } })); if res.is_err() { eprintln!("CRASHER {i:?}"); } }
    { let sc: f64 = std::env::var("FUZZ_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0); let t = Instant::now(); let mut ra = Rasterizer::new(sc); eprintln!("NEW {:?}", t.elapsed()); let mut su = ra.surface_for(30, 10).unwrap(); let t = Instant::now(); ra.render(&list, &mut su); eprintln!("FULL cold {:?}", t.elapsed()); let t = Instant::now(); ra.render(&list, &mut su); eprintln!("FULL warm {:?}", t.elapsed()); for k in 1..=list.items.len() { let part = DisplayList { items: list.items[..k].to_vec() }; let t = Instant::now(); ra.render(&part, &mut su); let e = t.elapsed(); if e > Duration::from_millis(60) { eprintln!("PREFIX {k} {e:?} {:.200}", format!("{:?}", list.items[k-1])); } } }
    for i in &list.items { if let DisplayItem::Text { origin, glyphs, shaped, clip, .. } = i { eprintln!("TEXT origin={origin:?} clip={clip:?} g0={:?} shaped={:?}", glyphs.first(), shaped.as_ref().map(|s| (s.size_px, s.baseline, s.line_height))); } }
}
