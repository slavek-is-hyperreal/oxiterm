//! BAD ACTOR 7.1 (client glue). Failing tests = findings.
#![cfg(feature = "local")]
use oxiterm_web::local::{LocalConfig, LocalSession};

const PAGE: &str = "<screen><text>hi</text><button event-htmx=\"inc:c\">go</button></screen>";

fn sess(actions: bool) -> LocalSession {
    let cfg = LocalConfig { start_page: "index.thtml".into(), cols: 80, rows: 24, mobile: false, actions, app_token: String::new() };
    LocalSession::new(&cfg, vec![("index.thtml".into(), PAGE.into())], 0.0).unwrap()
}

/// The centre of the first cell whose click parks an action: found by scanning the grid with real
/// press+release pointer events (the page's button is where the layout put it, not at a fixed cell).
fn find_button(s: &mut LocalSession, t: &mut f64) -> (f32, f32) {
    s.pump(*t);
    for row in 0..24 {
        for col in 0..80 {
            let (x, y) = (col as f32 + 0.5, row as f32 + 0.5);
            for action in [0u8, 1] {
                let mut p = vec![0x03];
                p.extend_from_slice(&x.to_le_bytes());
                p.extend_from_slice(&y.to_le_bytes());
                p.extend_from_slice(&[0, action, 0]);
                s.input(&p, *t);
            }
            *t += 400.0;
            s.pump(*t);
            if !s.take_actions().is_empty() {
                return (x, y);
            }
        }
    }
    panic!("no cell of the page parks an action");
}

fn click(s: &mut LocalSession, (x, y): (f32, f32), t: f64) {
    for action in [0u8, 1] {
        let mut p = vec![0x03];
        p.extend_from_slice(&x.to_le_bytes());
        p.extend_from_slice(&y.to_le_bytes());
        p.extend_from_slice(&[0, action, 0]);
        s.input(&p, t);
    }
}

fn rng(x: &mut u64) -> u64 { *x ^= *x << 13; *x ^= *x >> 7; *x ^= *x << 17; *x }

#[test]
fn held_input_fuzz_no_panic() {
    let mut s = sess(true);
    let mut x = 0x1234_5678_9abc_def1u64;
    for i in 0..30_000 {
        let n = (rng(&mut x) % 20) as usize;
        let mut b: Vec<u8> = (0..n).map(|_| rng(&mut x) as u8).collect();
        if !b.is_empty() && rng(&mut x) % 4 != 0 { b[0] = [1, 2, 3, 0x11, 0x31, 0x10][(rng(&mut x) % 6) as usize]; }
        // keep resize small here: see f1
        if b.first() == Some(&0x10) { b.truncate(1); }
        let t = i as f64;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { s.input(&b, t); if i % 50 == 0 { s.pump(t); s.take_actions(); s.take_messages(); s.take_frames(); } }))
            .unwrap_or_else(|_| panic!("panic on {b:?}"));
    }
}

/// F8 (was F1; crash/OOM): 0x10 resize was not clamped. 65535x65535 cells are now clamped to
/// MAX_COLS x MAX_ROWS (500x200) by the decoder, the debouncer and the loop.
#[test]
fn f1_huge_resize() {
    let mut s = sess(false);
    s.input(&[0x10, 0xff, 0xff, 0xff, 0xff], 1.0);
    for t in [500.0, 1000.0, 2000.0, 5000.0] { s.pump(t); s.take_frames(); }
    let d = *s.event_loop().session.dims.read();
    assert_eq!((d.cols, d.rows), (oxiterm_proto::input::wire::MAX_COLS, oxiterm_proto::input::wire::MAX_ROWS));
    // a session *constructed* huge is clamped too
    let cfg = LocalConfig { start_page: "index.thtml".into(), cols: 65535, rows: 65535, mobile: false, actions: false, app_token: String::new() };
    let s2 = LocalSession::new(&cfg, vec![("index.thtml".into(), PAGE.into())], 0.0).unwrap();
    let d = *s2.event_loop().session.dims.read();
    assert_eq!((d.cols, d.rows), (500, 200));
}

/// Corrected: the old assertion expected a 1000x1000 grid to be accepted as is, i.e. that nothing
/// is clamped. That was the bug's symptom: the grid is now bounded at 500x200, and a request for
/// more is cut to it, still quickly.
#[test]
fn f1b_big_but_finite_resize_is_bounded() {
    let mut s = sess(false);
    let t = std::time::Instant::now();
    s.input(&[0x10, 0xe8, 0x03, 0xe8, 0x03], 1.0); // asks for 1000x1000
    for t in [500.0, 1000.0, 2000.0] { s.pump(t); s.take_frames(); }
    let d = *s.event_loop().session.dims.read(); assert_eq!((d.cols, d.rows), (500, 200));
    assert!(t.elapsed().as_millis() < 2000, "resize took {:?}", t.elapsed());
}

#[test]
fn held_zero_resize_and_reply_ids() {
    let mut s = sess(true);
    s.input(&[0x10, 0, 0, 0, 0], 1.0);
    s.pump(2.0);
    for id in [0u32, 1, 1, u32::MAX, 77] { s.action_reply(id, serde_json::json!({"c": "x"})); s.action_failed(id); }
    for v in [serde_json::json!([1, 2]), serde_json::json!("s"), serde_json::json!(null), serde_json::json!({"open_url": 5, "set_app_token": [1], "c": {"a": 1}})] {
        s.action_reply(1, v);
        s.pump(3.0);
    }
    s.take_messages();
}

#[test]
fn held_pending_replies_capped() {
    let mut s = sess(true);
    let mut t = 1.0;
    let spot = find_button(&mut s, &mut t);
    let mut parked = 0;
    for _ in 0..500 {
        t += 400.0;
        click(&mut s, spot, t);
        t += 1.0;
        s.pump(t);
        parked += s.take_actions().len();
    }
    // the click really parks actions, nothing ever answers, and the table stays bounded
    assert!(parked > 64, "only {parked} actions parked: the test is not hitting the button");
    assert!(s.pending_replies() <= 64, "{} replies pending", s.pending_replies());
}

#[test]
fn held_extreme_pointer_and_navigate() {
    let mut s = sess(true);
    for v in [f32::MAX, f32::MIN, 1e30, -1e30, 0.0, -0.0, f32::MIN_POSITIVE] {
        let mut p = vec![0x03];
        p.extend_from_slice(&v.to_le_bytes());
        p.extend_from_slice(&v.to_le_bytes());
        p.extend_from_slice(&[0, 0, 0]);
        s.input(&p, 1.0);
        p[10] = 1;
        s.input(&p, 1.0);
        s.pump(2.0);
    }
    let mut nav = vec![0x31];
    nav.extend(std::iter::repeat(b'a').take(1 << 20));
    s.input(&nav, 3.0);
    let mut nav = vec![0x31];
    nav.extend_from_slice(b"../../etc/passwd");
    s.input(&nav, 4.0);
    s.pump(5.0);
    assert_eq!(s.current_page().as_deref(), Some("index.thtml"));
}

#[test]
fn held_pending_replies_really_parked() {
    let mut s = sess(true);
    let mut t = 1.0;
    let spot = find_button(&mut s, &mut t);
    let mut parked = 0;
    for _ in 0..300 {
        t += 400.0;
        click(&mut s, spot, t);
        t += 1.0;
        s.pump(t);
        parked += s.take_actions().len();
    }
    eprintln!("parked {parked}, pending {}", s.pending_replies());
    assert!(parked > 64, "test did not hit the button: parked {parked}");
    assert!(s.pending_replies() <= 64);
    // the oldest replies were dropped: answering them is a no-op, the newest still works
    s.action_reply(1, serde_json::json!({"c": "late"}));
    s.pump(t + 1.0);
}
