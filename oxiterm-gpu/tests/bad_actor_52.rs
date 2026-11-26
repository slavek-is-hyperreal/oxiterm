//! BAD ACTOR probes for stage 5.2 (`oxiterm-gpu`). Report-only: tests that FAIL document a
//! finding (see docs/stages/5.2/bad-actor.md); tests that pass document what held up.
//!
//! Run: `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json WGPU_BACKEND=vulkan \
//!   cargo test --release -p oxiterm-gpu --test bad_actor_52 -- --test-threads=1 --nocapture`
//! Every test uses its OWN `GpuContext` (hostile tests may poison a device).

#![allow(clippy::all, clippy::pedantic)]

mod common;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxiterm_fb::Rasterizer;
use oxiterm_gpu::{GpuContext, GpuSink};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{AnsiColor, BorderChars};
use oxiterm_renderer::render::display_list::{
    DisplayItem, DisplayList, Glyph, GridCell, Modifiers, PointF, RectF, ShapedRun, FACE_MONO,
    FACE_SANS,
};

// ---------------------------------------------------------------- helpers

fn ctx() -> Option<Arc<GpuContext>> {
    common::ctx_or_skip("bad_actor_52", GpuContext::new_headless()).map(Arc::new)
}

fn rss_kb() -> u64 {
    let s = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    s.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) * 4
}

fn tc(r: u8, g: u8, b: u8) -> AnsiColor {
    AnsiColor::TrueColor(r, g, b)
}

fn fill(x: f32, y: f32, w: f32, h: f32, radius: f32, c: AnsiColor) -> DisplayItem {
    DisplayItem::Fill { node: NodeId(1), rect: RectF::new(x, y, w, h), radius, fg: AnsiColor::Reset, bg: c }
}

fn mono(x: f32, y: f32, s: &str, mods: Modifiers, fg: AnsiColor, bg: AnsiColor) -> DisplayItem {
    let glyphs: Vec<Glyph> =
        s.chars().enumerate().map(|(i, ch)| Glyph { x: i as f32, y: 0.0, ch, advance: 1.0 }).collect();
    DisplayItem::Text {
        node: NodeId(2),
        origin: PointF { x, y },
        clip: RectF::new(x - 1.0, y, glyphs.len() as f32 + 2.0, 1.0),
        glyphs,
        fg,
        bg,
        mods,
        shaped: None,
    }
}

fn shaped(
    x: f32,
    y: f32,
    s: &str,
    size_px: f32,
    face: u8,
    mods: Modifiers,
    clusters: Vec<(u32, Box<str>)>,
) -> DisplayItem {
    let mut gx = 0.0f32;
    let mut glyphs = Vec::new();
    for ch in s.chars() {
        let adv = size_px * 0.6 / 9.6;
        glyphs.push(Glyph { x: gx, y: 0.0, ch, advance: adv });
        gx += adv;
    }
    DisplayItem::Text {
        node: NodeId(3),
        origin: PointF { x, y },
        clip: RectF::new(x - 1.0, y - 1.0, gx + 2.0, 6.0),
        glyphs,
        fg: tc(240, 240, 240),
        bg: AnsiColor::Reset,
        mods,
        shaped: Some(Box::new(ShapedRun {
            face,
            size_px,
            line_height: size_px * 1.25 / 20.0,
            baseline: size_px * 0.95 / 20.0,
            glyph_faces: vec![],
            clusters,
        })),
    }
}

fn no_mods() -> Modifiers {
    Modifiers::default()
}

fn bold() -> Modifiers {
    Modifiers { bold: true, ..Default::default() }
}

fn lst(items: Vec<DisplayItem>) -> DisplayList {
    DisplayList { items }
}

/// Outcome of a call that may panic.
fn outcome<T>(f: impl FnOnce() -> Result<T, oxiterm_gpu::GpuError>) -> String {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(_)) => "OK".to_string(),
        Ok(Err(e)) => format!("Err({e})"),
        Err(p) => {
            let m = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_default();
            format!("PANIC({})", m.chars().take(160).collect::<String>().replace('\n', " "))
        }
    }
}

struct Xs(u64);
impl Xs {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn r(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f()
    }
    fn u(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn col(&mut self) -> AnsiColor {
        match self.u(4) {
            0 => AnsiColor::Reset,
            1 => AnsiColor::Color256(self.u(256) as u8),
            _ => tc(self.u(256) as u8, self.u(256) as u8, self.u(256) as u8),
        }
    }
}

/// Per-tile comparison. Returns (differing pixels aggregate, worst tile (tx,ty,count,tile_px)).
fn tile_diff(a: &[u8], b: &[u8], w: u32, h: u32, tol: u8, tile: u32) -> (u64, (u32, u32, u32, u32)) {
    let mut agg = 0u64;
    let tiles_x = w.div_ceil(tile);
    let tiles_y = h.div_ceil(tile);
    let mut worst = (0, 0, 0, 1);
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let (mut cnt, mut tot) = (0u32, 0u32);
            for y in ty * tile..((ty + 1) * tile).min(h) {
                for x in tx * tile..((tx + 1) * tile).min(w) {
                    let i = ((y * w + x) * 4) as usize;
                    tot += 1;
                    let d = (0..3).map(|c| a[i + c].abs_diff(b[i + c])).max().unwrap_or(0);
                    if d > tol {
                        cnt += 1;
                    }
                }
            }
            agg += u64::from(cnt);
            if cnt > worst.2 {
                worst = (tx, ty, cnt, tot);
            }
        }
    }
    (agg, worst)
}

/// Renders `list` on CPU and GPU, returns (aggregate differing, worst tile).
fn cpu_gpu_tiles(
    ctx: &Arc<GpuContext>,
    list: &DisplayList,
    cols: u32,
    rows: u32,
    scale: f64,
) -> (u32, u32, u64, (u32, u32, u32, u32)) {
    let mut cpu = Rasterizer::new(scale);
    let mut surf = cpu.surface_for(cols, rows).expect("surf");
    cpu.render(list, &mut surf);
    let mut gpu = GpuSink::new(ctx.clone(), scale);
    let img = gpu.render_cells(list, cols, rows).expect("gpu");
    assert_eq!((img.width(), img.height()), (surf.width(), surf.height()));
    let (agg, worst) = tile_diff(img.rgba(), surf.rgba(), img.width(), img.height(), 4, 16);
    let maxd = img.rgba().iter().zip(surf.rgba().iter()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
    LAST_MAXD.with(|c| c.set(maxd));
    (img.width(), img.height(), agg, worst)
}

thread_local! { static LAST_MAXD: std::cell::Cell<u8> = const { std::cell::Cell::new(0) }; }

// ---------------------------------------------------------------- 4. readback padding

/// Each pixel's colour encodes its own (x, y); exact check at awkward widths/heights.
#[test]
fn b_readback_row_padding_exact() {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx, 1.0);
    let cw = 9.6f32;
    let ch = 20.0f32;
    for &(w, h) in &[(1u32, 1u32), (1, 3), (3, 5), (63, 3), (64, 3), (65, 3), (257, 5), (4097, 3), (4096, 2)] {
        let mut items = Vec::new();
        for y in 0..h {
            for x in 0..w {
                // one 1x1-pixel fill per pixel, coded colour
                items.push(fill(x as f32 / cw, y as f32 / ch, 1.0 / cw, 1.0 / ch, 0.0, tc((x & 255) as u8, (x >> 8) as u8 * 16 + y as u8, 77)));
            }
        }
        let img = sink.render(&lst(items), w, h).expect("render");
        let mut bad = 0;
        for y in 0..h {
            for x in 0..w {
                let p = img.pixel(x, y);
                let want = [(x & 255) as u8, (x >> 8) as u8 * 16 + y as u8, 77];
                if p != want {
                    bad += 1;
                    if bad < 4 {
                        eprintln!("{w}x{h} px({x},{y}) got {p:?} want {want:?}");
                    }
                }
            }
        }
        assert_eq!(bad, 0, "{w}x{h}: {bad} wrong pixels");
        eprintln!("padding {w}x{h}: exact");
    }
}

// ---------------------------------------------------------------- 3. render_into_view (F-13)

fn mk_tex(ctx: &GpuContext, w: u32, h: u32, fmt: wgpu::TextureFormat, usage: wgpu::TextureUsages, samples: u32) -> wgpu::Texture {
    ctx.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("bad-actor"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format: fmt,
        usage,
        view_formats: &[],
    })
}

/// F-13 confirmation: each bad view is a panic (uncaptured error handler), not an `Err`.
#[test]
fn b_render_into_view_bad_views_panic_or_err() {
    let list = lst(vec![fill(1.0, 1.0, 3.0, 2.0, 0.0, tc(200, 10, 10))]);
    let ra = wgpu::TextureUsages::RENDER_ATTACHMENT;
    let cases: Vec<(&str, wgpu::TextureFormat, wgpu::TextureUsages, u32, u32, u32)> = vec![
        ("bgra8 view", wgpu::TextureFormat::Bgra8Unorm, ra, 1, 64, 64),
        ("rgba8-srgb view", wgpu::TextureFormat::Rgba8UnormSrgb, ra, 1, 64, 64),
        ("texture smaller than w/h", wgpu::TextureFormat::Rgba8Unorm, ra, 1, 32, 32),
        ("no RENDER_ATTACHMENT usage", wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING, 1, 64, 64),
        ("msaa x4 target", wgpu::TextureFormat::Rgba8Unorm, ra, 4, 64, 64),
    ];
    let mut panics = 0;
    for (label, fmt, usage, samples, tw, th) in cases {
        let Some(ctx) = ctx() else { return };
        let mut sink = GpuSink::new(ctx.clone(), 1.0);
        let tex = mk_tex(&ctx, tw, th, fmt, usage, samples);
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let r = outcome(|| sink.render_into_view(&list, &view, 64, 64));
        eprintln!("render_into_view [{label}] -> {r}");
        if r.starts_with("PANIC") {
            panics += 1;
        }
    }
    assert_eq!(panics, 0, "{panics} invalid-view cases panic instead of returning Err (F-13)");
}

/// A co-user's pushed-and-never-popped error scope swallows the sink's validation errors:
/// the sink returns `Ok(())` for a render that did nothing.
#[test]
fn b_foreign_error_scope_swallows_sink_errors() {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx.clone(), 1.0);
    let tex = mk_tex(&ctx, 64, 64, wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureUsages::RENDER_ATTACHMENT, 1);
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let scope = ctx.device().push_error_scope(wgpu::ErrorFilter::Validation);
    let list = lst(vec![fill(1.0, 1.0, 3.0, 2.0, 0.0, tc(200, 10, 10))]);
    let r = outcome(|| sink.render_into_view(&list, &view, 64, 64));
    let captured = pollster::block_on(scope.pop());
    eprintln!("with foreign scope: sink says {r}; scope captured: {}", captured.is_some());
    // The sink reported success although the frame was rejected by wgpu.
    assert!(r != "OK" || captured.is_none(), "sink returned Ok(()) while wgpu rejected the pass (error hidden in foreign scope)");
}

// ---------------------------------------------------------------- 3. device lost / destroyed

#[test]
fn b_render_after_device_destroy() {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx.clone(), 1.0);
    let list = lst(vec![fill(1.0, 1.0, 3.0, 2.0, 0.0, tc(200, 10, 10)), mono(1.0, 1.0, "hi", no_mods(), tc(255, 255, 255), AnsiColor::Reset)]);
    let ok = sink.render_cells(&list, 10, 4).is_ok();
    assert!(ok);
    ctx.device().destroy();
    let r = outcome(|| sink.render_cells(&list, 10, 4));
    eprintln!("render after device.destroy(): {r}");
    let r2 = outcome(|| sink.render_cells(&list, 11, 4)); // new offscreen target on dead device
    eprintln!("render (new size) after device.destroy(): {r2}");
    let r3 = outcome(|| {
        let mut s2 = GpuSink::new(ctx.clone(), 1.0);
        s2.render_cells(&list, 10, 4)
    });
    eprintln!("new sink after device.destroy(): {r3}");
    assert!(
        !r.starts_with("PANIC") && !r2.starts_with("PANIC") && !r3.starts_with("PANIC"),
        "render on a destroyed/lost device must return Err, not panic: {r} | {r2} | {r3}"
    );
}

// ---------------------------------------------------------------- 3. concurrency

#[test]
fn b_two_sinks_four_threads_one_context_deterministic() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<GpuContext>();
    let Some(ctx) = ctx() else { return };
    let scene = |k: u32| {
        lst(vec![
            fill(0.0, 0.0, 30.0, 8.0, 0.5, tc(10 + k as u8 * 7, 40, 90)),
            mono(1.0, 1.0, &format!("thread scene {k} Hello"), bold(), tc(250, 250, 250), AnsiColor::Reset),
            shaped(1.0, 3.0, "Proportional text WAV", 18.0 + k as f32, FACE_SANS, no_mods(), vec![]),
            DisplayItem::Border { node: NodeId(9), rect: RectF::new(0.5, 0.5, 28.0, 6.0), radius: 0.0, chars: BorderChars::double(), fg: tc(255, 200, 0), bg: AnsiColor::Reset },
        ])
    };
    // reference, single threaded
    let mut refs = Vec::new();
    for k in 0..4u32 {
        let mut s = GpuSink::new(ctx.clone(), 1.0);
        refs.push(s.render_cells(&scene(k), 30, 8).unwrap());
    }
    let t0 = Instant::now();
    let results: Vec<Vec<usize>> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..4u32)
            .map(|k| {
                let ctx = ctx.clone();
                let refs = &refs;
                sc.spawn(move || {
                    let mut s = GpuSink::new(ctx, 1.0);
                    let mut bad = Vec::new();
                    for i in 0..40 {
                        let img = s.render_cells(&scene(k), 30, 8).unwrap();
                        if img != refs[k as usize] {
                            bad.push(i);
                        }
                        if i % 10 == 5 {
                            s.clear_atlas();
                        }
                    }
                    bad
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    eprintln!("4 threads x 40 frames in {:?}; mismatching frames per thread: {results:?}", t0.elapsed());
    assert!(results.iter().all(Vec::is_empty), "concurrent sinks produced different pixels: {results:?}");
}

/// Interleaved sinks must not leak atlas/instance state: A, B thrashing, A again are byte-equal.
#[test]
fn b_determinism_across_sinks_and_thrash() {
    let Some(ctx) = ctx() else { return };
    let list = lst(vec![
        mono(1.0, 1.0, "Determinism 0123456789", bold(), tc(255, 255, 255), tc(0, 0, 80)),
        shaped(1.0, 3.0, "ffi fl Wave AVA", 21.0, FACE_SANS, Modifiers { italic: true, ..Default::default() }, vec![(0, "ffi".into())]),
    ]);
    let mut a = GpuSink::new(ctx.clone(), 1.5);
    let mut b = GpuSink::new(ctx.clone(), 1.5);
    let a1 = a.render_cells(&list, 40, 8).unwrap();
    // thrash B's atlas with many sizes
    let mut thrash = Vec::new();
    for i in 0..400 {
        thrash.push(shaped(0.0, (i % 20) as f32, "WMQ@gjy&", 40.0 + (i % 60) as f32, FACE_SANS, bold(), vec![]));
    }
    let _ = b.render_cells(&lst(thrash), 80, 24).unwrap();
    let b1 = b.render_cells(&list, 40, 8).unwrap();
    a.clear_atlas();
    let a2 = a.render_cells(&list, 40, 8).unwrap();
    assert_eq!(a1, b1, "sink A vs sink B");
    assert_eq!(a1, a2, "sink A before/after atlas clear");
    eprintln!("b atlas resets after thrash: {}", b.atlas().resets());
}

// ---------------------------------------------------------------- 2. atlas / resources

/// Atlas key: distinct sizes 12.0 vs 12.3 in one sink equal a fresh sink's output.
#[test]
fn b_atlas_key_no_collision_between_sizes() {
    let Some(ctx) = ctx() else { return };
    let l1 = lst(vec![shaped(1.0, 1.0, "Collision", 12.0, FACE_SANS, no_mods(), vec![])]);
    let l2 = lst(vec![shaped(1.0, 1.0, "Collision", 12.3, FACE_SANS, no_mods(), vec![])]);
    let l3 = lst(vec![shaped(1.0, 1.0, "Collision", 12.3, FACE_SANS, bold(), vec![])]);
    let mut s = GpuSink::new(ctx.clone(), 1.0);
    let _ = s.render_cells(&l1, 30, 4).unwrap();
    let a = s.render_cells(&l2, 30, 4).unwrap();
    let b = s.render_cells(&l3, 30, 4).unwrap();
    let mut fresh = GpuSink::new(ctx.clone(), 1.0);
    assert_eq!(a, fresh.render_cells(&l2, 30, 4).unwrap());
    let mut fresh = GpuSink::new(ctx.clone(), 1.0);
    assert_eq!(b, fresh.render_cells(&l3, 30, 4).unwrap());
}

/// Empty (no-outline) glyph entries are cached forever (`entries.insert(key, None)`) and
/// only cleared when the atlas fills, which they never do: unbounded growth per distinct size.
#[test]
fn b_atlas_empty_entries_grow_without_bound() {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx, 1.0);
    // find a char that is blank-but-mapped
    let cand = ['\u{200B}', '\u{200C}', '\u{200D}', '\u{FEFF}', '\u{2060}', '\u{00AD}', '\u{200E}'];
    let mut pick = None;
    for c in cand {
        let before = sink.atlas().glyph_count();
        let l = lst(vec![shaped(1.0, 1.0, &c.to_string(), 17.77, FACE_MONO, no_mods(), vec![])]);
        let _ = sink.render_cells(&l, 10, 3).unwrap();
        if sink.atlas().glyph_count() > before {
            pick = Some(c);
            break;
        }
    }
    let Some(c) = pick else {
        eprintln!("no blank-but-mapped char found; nothing cached");
        return;
    };
    let rss0 = rss_kb();
    let t = Instant::now();
    for frame in 0..10u32 {
        // 20k FRESH sizes per frame: the count must climb across frames if nothing is evicted
        let items: Vec<DisplayItem> = (0..20_000u32)
            .map(|i| shaped(1.0, 1.0, &c.to_string(), 0.5 + ((frame * 20_000 + i) as f32) * 0.0025, FACE_MONO, no_mods(), vec![]))
            .collect();
        let _ = sink.render_cells(&lst(items), 10, 3).unwrap();
        eprintln!("  frame {frame}: entries {}", sink.atlas().glyph_count());
    }
    let entries = sink.atlas().glyph_count();
    let rss1 = rss_kb();
    // a normal frame afterwards does not release them
    let _ = sink.render_cells(&lst(vec![mono(0.0, 0.0, "x", no_mods(), tc(1, 2, 3), AnsiColor::Reset)]), 10, 3).unwrap();
    let after = sink.atlas().glyph_count();
    eprintln!("blank char U+{:04X}: entries {entries} (after normal frame {after}), resets {}, rss +{} MB, {:?}", c as u32, sink.atlas().resets(), (rss1.saturating_sub(rss0)) / 1024, t.elapsed());
    assert!(after < 1000, "{after} blank-glyph entries retained across frames (no eviction, atlas never fills)");
}

/// 1000 normal frames: RSS should be flat.
#[test]
fn b_leak_1000_normal_frames() {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx, 1.0);
    let list = lst(vec![
        fill(0.0, 0.0, 40.0, 10.0, 0.5, tc(10, 40, 90)),
        mono(1.0, 1.0, "Leak check frame text", bold(), tc(250, 250, 250), AnsiColor::Reset),
        shaped(1.0, 3.0, "Proportional leak", 18.0, FACE_SANS, no_mods(), vec![]),
    ]);
    for _ in 0..50 {
        let _ = sink.render_cells(&list, 40, 10).unwrap();
    }
    let r0 = rss_kb();
    let t = Instant::now();
    for _ in 0..1000 {
        let _ = sink.render_cells(&list, 40, 10).unwrap();
    }
    let r1 = rss_kb();
    eprintln!("1000 frames {:?}; rss {} -> {} KB (+{} KB)", t.elapsed(), r0, r1, r1.saturating_sub(r0));
    assert!(r1.saturating_sub(r0) < 20 * 1024, "RSS grew {} KB over 1000 identical frames", r1 - r0);
}

/// Alternating surface sizes: each size change recreates texture + readback buffer.
#[test]
fn b_leak_size_flipping() {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx, 1.0);
    let list = lst(vec![fill(0.0, 0.0, 5.0, 2.0, 0.0, tc(9, 9, 9))]);
    for i in 0..30 {
        let _ = sink.render(&list, 200 + (i % 7), 100 + i % 5).unwrap();
    }
    let r0 = rss_kb();
    for i in 0..600u32 {
        let _ = sink.render(&list, 800 + (i % 7), 600 + i % 5).unwrap();
    }
    let r1 = rss_kb();
    eprintln!("600 size flips: rss {} -> {} KB", r0, r1);
    assert!(r1.saturating_sub(r0) < 64 * 1024, "RSS grew {} KB over 600 size flips", r1 - r0);
}

/// One large frame grows the instance buffer + CPU Vecs, which are never shrunk.
#[test]
fn b_instance_buffers_never_shrink() {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx, 1.0);
    let small = lst(vec![fill(0.0, 0.0, 5.0, 2.0, 0.0, tc(9, 9, 9))]);
    for _ in 0..5 {
        let _ = sink.render_cells(&small, 20, 5).unwrap();
    }
    let r0 = rss_kb();
    let big: Vec<DisplayItem> = (0..600_000u32)
        .map(|i| fill((i % 400) as f32 * 0.1, ((i / 400) % 200) as f32 * 0.1, 0.1, 0.1, 0.0, tc((i & 255) as u8, 1, 2)))
        .collect();
    let big = lst(big);
    let t = Instant::now();
    let _ = sink.render_cells(&big, 100, 50).unwrap();
    let dt = t.elapsed();
    drop(big);
    let r1 = rss_kb();
    for _ in 0..100 {
        let _ = sink.render_cells(&small, 20, 5).unwrap();
    }
    let r2 = rss_kb();
    eprintln!("600k fills {:?}; rss before {} KB, after big frame (list dropped) {} KB, after 100 small frames {} KB", dt, r0, r1, r2);
    drop(sink);
    let r3 = rss_kb();
    eprintln!("control: after dropping the sink rss {} KB (released {} KB)", r3, r2.saturating_sub(r3));
    assert!(r2.saturating_sub(r0) < 16 * 1024, "RSS stays +{} MB after one hostile frame (buffers never shrink)", (r2 - r0) / 1024);
}

/// Many sinks on one context: per-sink fixed cost (1 MiB atlas + pipelines).
#[test]
fn b_many_sinks_one_context() {
    let Some(ctx) = ctx() else { return };
    let r0 = rss_kb();
    let t = Instant::now();
    let mut sinks: Vec<GpuSink> = (0..64).map(|_| GpuSink::new(ctx.clone(), 1.0)).collect();
    let list = lst(vec![mono(0.0, 0.0, "many", no_mods(), tc(255, 255, 255), AnsiColor::Reset)]);
    for s in &mut sinks {
        let _ = s.render_cells(&list, 10, 2).unwrap();
    }
    let r1 = rss_kb();
    eprintln!("64 sinks: {:?}, rss +{} MB ({} KB/sink)", t.elapsed(), (r1 - r0) / 1024, (r1 - r0) / 64);
    drop(sinks);
}

/// Atlas thrash across frames: thousands of distinct sizes, bold/italic, then correctness vs CPU.
#[test]
fn b_atlas_thrash_then_correct() {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx.clone(), 1.0);
    let rss0 = rss_kb();
    let t = Instant::now();
    for f in 0..6u32 {
        let mut items = Vec::new();
        for i in 0..1500u32 {
            let sz = 20.0 + ((i * 7 + f * 13) % 300) as f32 * 0.37;
            let mods = Modifiers { bold: i % 3 == 0, italic: i % 5 == 0, underline: false };
            items.push(shaped((i % 30) as f32, (i % 20) as f32, "Wg@Q", sz, if i % 2 == 0 { FACE_SANS } else { FACE_MONO }, mods, vec![]));
        }
        let _ = sink.render_cells(&lst(items), 80, 24).unwrap();
    }
    eprintln!("thrash 6 frames x 1500 items: {:?}, resets {}, atlas {}, entries {}, rss +{} MB", t.elapsed(), sink.atlas().resets(), sink.atlas().size(), sink.atlas().glyph_count(), (rss_kb().saturating_sub(rss0)) / 1024);
    // after thrash a probe scene must equal CPU
    let probe = lst(vec![
        mono(1.0, 1.0, "After thrash", bold(), tc(255, 255, 255), tc(0, 0, 60)),
        shaped(1.0, 3.0, "Proportional Wave", 19.0, FACE_SANS, no_mods(), vec![]),
    ]);
    let mut cpu = Rasterizer::new(1.0);
    let mut surf = cpu.surface_for(40, 8).unwrap();
    cpu.render(&probe, &mut surf);
    let img = sink.render_cells(&probe, 40, 8).unwrap();
    let (agg, worst) = tile_diff(img.rgba(), surf.rgba(), img.width(), img.height(), 4, 16);
    eprintln!("post-thrash vs CPU: agg {agg}, worst tile {worst:?}");
    assert!(worst.2 <= 8, "post-thrash divergence worst tile {worst:?}");
}

// ---------------------------------------------------------------- 6. scale

#[test]
fn b_sink_new_scale_inputs() {
    let Some(ctx) = ctx() else { return };
    for sc in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e6, 1e-9, 16.0] {
        let r = outcome(|| {
            let mut s = GpuSink::new(ctx.clone(), sc);
            let (cw, _) = s.cell_px();
            let l = lst(vec![fill(0.0, 0.0, 2.0, 1.0, 0.0, tc(1, 2, 3)), mono(0.0, 0.0, "a", no_mods(), tc(9, 9, 9), AnsiColor::Reset)]);
            s.render_cells(&l, 4, 2).map(|_| cw)
        });
        eprintln!("scale {sc}: {r}");
        assert!(!r.starts_with("PANIC"), "scale {sc} -> {r}");
    }
}

// ---------------------------------------------------------------- 5. scissor / clip edges

#[test]
fn b_clip_edges_and_storms() {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx.clone(), 1.0);
    let (cols, rows) = (20u32, 6u32);
    let (w, _) = sink.pixel_size(cols, rows);
    let wc = w as f32 / 9.6;
    let base = vec![fill(0.0, 0.0, 20.0, 6.0, 0.0, tc(5, 5, 50))];
    let clips = [
        RectF::new(wc, 0.0, 0.0, 3.0),          // x == width, w == 0
        RectF::new(wc, 0.0, 5.0, 3.0),          // starts exactly at right edge
        RectF::new(-100.0, -100.0, 50.0, 50.0), // fully outside
        RectF::new(1e30, 1e30, 1e30, 1e30),
        RectF::new(-1e30, -1e30, 2e30, 2e30),
        RectF::new(3.0, 3.0, -2.0, -2.0),       // negative size
        RectF::new(f32::NAN, 0.0, 1.0, 1.0),
        RectF::new(0.0, 0.0, f32::INFINITY, f32::INFINITY),
    ];
    for c in clips {
        let mut items = base.clone();
        items.push(DisplayItem::PushClip { rect: c });
        items.push(fill(0.0, 0.0, 20.0, 6.0, 0.0, tc(250, 0, 0)));
        items.push(mono(0.0, 0.0, "clipped", bold(), tc(255, 255, 255), tc(0, 90, 0)));
        items.push(DisplayItem::PopClip);
        let r = outcome(|| sink.render_cells(&lst(items), cols, rows));
        assert_eq!(r, "OK", "clip {c:?} -> {r}");
    }
    // 100k nested clips then pops, and a pop storm
    let mut items = base.clone();
    for i in 0..100_000 {
        items.push(DisplayItem::PushClip { rect: RectF::new(0.0, 0.0, 20.0 - (i % 3) as f32 * 0.001, 6.0) });
        if i % 1000 == 0 {
            items.push(fill(0.0, 0.0, 1.0, 1.0, 0.0, tc(1, 2, 3)));
        }
    }
    for _ in 0..200_000 {
        items.push(DisplayItem::PopClip);
    }
    items.push(fill(0.0, 0.0, 3.0, 1.0, 0.0, tc(0, 255, 0)));
    let t = Instant::now();
    let img = sink.render_cells(&lst(items), cols, rows).expect("storm");
    eprintln!("100k nested clips + 200k pops: {:?}", t.elapsed());
    assert_eq!(img.pixel(1, 1), [0, 255, 0], "fill after pop storm");
}

// ---------------------------------------------------------------- 4. text edge cases

#[test]
fn b_text_edge_cases_no_panic_and_linear() {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx.clone(), 1.0);
    // zero-width, combining, RTL, controls, missing glyphs
    let weird = "a\u{0301}\u{0302}\u{0303} \u{200B}\u{200D}👨\u{200D}👩\u{200D}👧 שלום עולם مرحبا \t\u{0}\u{7}\u{FFFF}\u{10FFFF}\u{FE0F}日本語 \u{202E}rtl";
    let mut clusters: Vec<(u32, Box<str>)> = Vec::new();
    clusters.push((0, "a\u{0301}\u{0302}\u{0303}".into()));
    let mut items = vec![mono(0.0, 0.0, weird, bold(), tc(255, 255, 255), AnsiColor::Reset)];
    for (i, sz) in [0.1f32, 0.5, 0.51, 1.0, 5.0, 100.0, 500.0, 512.0, 513.0, 1e9, f32::MIN_POSITIVE, 1e-40, f32::NAN, f32::INFINITY].iter().enumerate() {
        items.push(shaped(0.0, (i % 5) as f32, weird, *sz, (i % 2) as u8, Modifiers { bold: i % 2 == 0, italic: i % 3 == 0, underline: i % 4 == 0 }, clusters.clone()));
    }
    // 10k combining marks on one base, as a cluster string
    let mut long = String::from("e");
    long.extend(std::iter::repeat_n('\u{0301}', 10_000));
    items.push(shaped(0.0, 1.0, "e", 20.0, FACE_SANS, no_mods(), vec![(0, long.clone().into())]));
    let mut c63 = String::from("e");
    c63.extend(std::iter::repeat_n('\u{0301}', 62));
    for i in 0..2000u32 {
        items.push(shaped(0.0, (i % 6) as f32, "e", 20.0 + (i % 50) as f32 * 0.5, FACE_SANS, no_mods(), vec![(0, c63.clone().into())]));
    }
    let t = Instant::now();
    let r = outcome(|| sink.render_cells(&lst(items), 40, 8));
    eprintln!("weird text: {r} in {:?}", t.elapsed());
    assert_eq!(r, "OK");

    // linear-time check on one run of N glyphs
    let mut times = Vec::new();
    for n in [100_000usize, 400_000] {
        let glyphs: Vec<Glyph> = (0..n).map(|i| Glyph { x: (i % 5000) as f32, y: (i / 5000) as f32, ch: (b'a' + (i % 26) as u8) as char, advance: 1.0 }).collect();
        let l = lst(vec![DisplayItem::Text { node: NodeId(1), origin: PointF { x: 0.0, y: 0.0 }, clip: RectF::new(0.0, 0.0, 100.0, 20.0), glyphs, fg: tc(255, 255, 255), bg: tc(0, 0, 0), mods: no_mods(), shaped: None }]);
        let t = Instant::now();
        let _ = sink.render_cells(&l, 80, 20).unwrap();
        times.push((n, t.elapsed()));
    }
    eprintln!("long mono run times: {times:?}");
    let ratio = times[1].1.as_secs_f64() / times[0].1.as_secs_f64().max(1e-6);
    assert!(ratio < 8.0, "4x glyphs took {ratio:.1}x time");

    // fractional accumulation: 10k items at x += 0.1 compared with CPU
    let mut acc = Vec::new();
    for i in 0..10_000u32 {
        acc.push(mono(i as f32 * 0.1 % 70.0, (i / 700) as f32 * 0.5, "ab", no_mods(), tc(255, 255, 255), AnsiColor::Reset));
    }
    let l = lst(acc);
    let (_, _, agg, worst) = cpu_gpu_tiles(&ctx, &l, 80, 8, 1.0);
    eprintln!("fractional accumulation vs CPU: agg {agg}, worst tile {worst:?}");
}

/// Single-pixel / giant text: a 500px-size run at scale 3 (px = 1500 > 512) silently vanishes.
#[test]
fn b_giant_text_is_silently_dropped_while_cpu_matches() {
    let Some(ctx) = ctx() else { return };
    let l = lst(vec![shaped(0.0, 0.0, "W", 170.0, FACE_SANS, bold(), vec![])]); // 170*3 = 510 px ok
    let l2 = lst(vec![shaped(0.0, 0.0, "W", 175.0, FACE_SANS, bold(), vec![])]); // 525 px: dropped
    let mut s = GpuSink::new(ctx.clone(), 3.0);
    let a = s.render_cells(&l, 20, 10).unwrap();
    let b = s.render_cells(&l2, 20, 10).unwrap();
    let bg = [15u8, 23, 42];
    let ink = |img: &oxiterm_gpu::FrameImage| img.rgba().chunks(4).filter(|p| p[..3] != bg[..] ).count();
    eprintln!("ink px: 510px glyph {}, 525px glyph {}", ink(&a), ink(&b));
    // CPU parity (same cutoff) - this is a documented limit, not a divergence
    let (_, _, agg, worst) = cpu_gpu_tiles(&ctx, &l2, 20, 10, 3.0);
    eprintln!("525px vs CPU: agg {agg}, worst {worst:?}");
}

// ---------------------------------------------------------------- 1. fuzz: hostile numbers

fn hostile_f32(r: &mut Xs) -> f32 {
    const V: [f32; 14] = [0.0, -0.0, 1.0, -1.0, 0.5, 1e-30, 1e30, -1e30, f32::MAX, f32::MIN, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 16777217.0];
    if r.u(3) == 0 { V[r.u(V.len() as u64) as usize] } else { r.r(-50.0, 120.0) }
}

fn hostile_scene(r: &mut Xs, n: usize) -> DisplayList {
    let mut items = Vec::new();
    for _ in 0..n {
        match r.u(8) {
            0 => items.push(fill(hostile_f32(r), hostile_f32(r), hostile_f32(r), hostile_f32(r), hostile_f32(r), r.col())),
            1 => {
                let chars = [BorderChars::single(), BorderChars::rounded(), BorderChars::double()][r.u(3) as usize].clone();
                items.push(DisplayItem::Border { node: NodeId(1), rect: RectF::new(hostile_f32(r), hostile_f32(r), hostile_f32(r), hostile_f32(r)), radius: hostile_f32(r), chars, fg: r.col(), bg: r.col() });
            }
            2 | 3 => {
                let k = r.u(30) as usize;
                let glyphs: Vec<Glyph> = (0..k).map(|_| Glyph { x: hostile_f32(r), y: hostile_f32(r), ch: char::from_u32(r.u(0x30000) as u32).unwrap_or('x'), advance: hostile_f32(r) }).collect();
                let sh = if r.u(2) == 0 { None } else {
                    Some(Box::new(ShapedRun { face: r.u(4) as u8, size_px: hostile_f32(r), line_height: hostile_f32(r), baseline: hostile_f32(r),
                        glyph_faces: vec![(r.u(5) as u32, r.u(3) as u8)], clusters: vec![(r.u(5) as u32, "e\u{301}\u{301}".into())] }))
                };
                items.push(DisplayItem::Text { node: NodeId(1), origin: PointF { x: hostile_f32(r), y: hostile_f32(r) }, clip: RectF::new(hostile_f32(r), hostile_f32(r), hostile_f32(r), hostile_f32(r)), glyphs, fg: r.col(), bg: r.col(), mods: Modifiers { bold: r.u(2) == 0, underline: r.u(2) == 0, italic: r.u(2) == 0 }, shaped: sh });
            }
            4 => {
                let (w, h) = (r.u(8) as f32, r.u(4) as f32);
                let cells: Vec<GridCell> = (0..r.u(40)).map(|_| GridCell { ch: char::from_u32(r.u(0x3000) as u32).unwrap_or('x'), fg: r.col(), bg: r.col(), mods: Modifiers { bold: r.u(2) == 0, underline: r.u(2) == 0, italic: r.u(2) == 0 }, skip: r.u(5) == 0 }).collect();
                items.push(DisplayItem::CellGrid { rect: RectF::new(hostile_f32(r), hostile_f32(r), if r.u(4) == 0 { hostile_f32(r) } else { w }, if r.u(4) == 0 { hostile_f32(r) } else { h }), cells });
            }
            5 => items.push(DisplayItem::PushClip { rect: RectF::new(hostile_f32(r), hostile_f32(r), hostile_f32(r), hostile_f32(r)) }),
            6 => items.push(DisplayItem::PopClip),
            _ => items.push(fill(r.r(0.0, 70.0), r.r(0.0, 20.0), r.r(0.0, 30.0), r.r(0.0, 10.0), r.r(0.0, 2.0), r.col())),
        }
    }
    lst(items)
}

fn run_hostile(seeds: u64, items: usize) {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx, 1.0);
    let rss0 = rss_kb();
    let t = Instant::now();
    let mut worst = Duration::ZERO;
    let mut bad = Vec::new();
    for seed in 1..=seeds {
        let mut r = Xs(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
        let l = hostile_scene(&mut r, items);
        let scale = [0.25, 1.0, 1.5, 3.0][(seed % 4) as usize];
        let mut s2;
        let s: &mut GpuSink = if seed % 4 == 1 { &mut sink } else { s2 = GpuSink::new(sink.context().clone(), scale); &mut s2 };
        let t0 = Instant::now();
        let res = outcome(|| s.render_cells(&l, 80, 24));
        worst = worst.max(t0.elapsed());
        if res != "OK" {
            bad.push((seed, res));
        }
    }
    eprintln!("hostile fuzz {seeds} seeds x {items} items: {:?}, worst frame {:?}, rss +{} MB, failures {}", t.elapsed(), worst, rss_kb().saturating_sub(rss0) / 1024, bad.len());
    for b in bad.iter().take(5) {
        eprintln!("  seed {} -> {}", b.0, b.1);
    }
    assert!(bad.is_empty(), "{} hostile scenes failed: {:?}", bad.len(), &bad[..bad.len().min(3)]);
}

#[test]
fn b_fuzz_hostile_small() {
    run_hostile(60, 80);
}

#[test]
#[ignore = "long fuzzer (minutes on llvmpipe); run with --ignored"]
fn b_fuzz_hostile_long() {
    run_hostile(1500, 300);
}

// ---------------------------------------------------------------- 1. differential, per tile

fn rand_scene(seed: u64, cols: u32, rows: u32) -> DisplayList {
    let mut r = Xs(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut items = Vec::new();
    let pool = ["Hello World", "fn main() { 0xFF }", "ffi fl AVA Wave", "Zażółć gęślą jaźń", "┌─┐│└┘ box", "e\u{301} a\u{308} n\u{303}", "日本語 wide", "The quick brown fox"];
    for _ in 0..(10 + r.u(30)) {
        let x = r.r(0.0, cols as f32 - 4.0).floor() + if r.u(3) == 0 { r.r(0.0, 1.0) } else { 0.0 };
        let y = r.r(0.0, rows as f32 - 2.0).floor() + if r.u(4) == 0 { r.r(0.0, 1.0) } else { 0.0 };
        let mods = Modifiers { bold: r.u(3) == 0, underline: r.u(4) == 0, italic: r.u(4) == 0 };
        match r.u(7) {
            0 => items.push(fill(x, y, r.r(1.0, 20.0), r.r(1.0, 6.0), if r.u(2) == 0 { 0.0 } else { r.r(0.1, 1.5) }, r.col())),
            1 => {
                let chars = [BorderChars::single(), BorderChars::rounded(), BorderChars::double()][r.u(3) as usize].clone();
                items.push(DisplayItem::Border { node: NodeId(1), rect: RectF::new(x, y, r.r(2.0, 20.0), r.r(2.0, 6.0)), radius: if r.u(3) == 0 { r.r(0.2, 1.2) } else { 0.0 }, chars, fg: r.col(), bg: r.col() });
            }
            2 | 3 => {
                let s = pool[r.u(pool.len() as u64) as usize];
                let cleaned: String = s.chars().filter(|c| !('\u{300}'..'\u{370}').contains(c) && (*c as u32) < 0x2000).collect();
                items.push(mono(x, y.floor(), &cleaned, mods, r.col(), r.col()));
            }
            4 | 5 => {
                let s = pool[r.u(4) as usize];
                let size = [10.0, 12.0, 14.0, 16.0, 20.0, 28.0, 40.0][r.u(7) as usize];
                let cl = if s.starts_with("ffi") { vec![(0u32, Box::<str>::from("ffi"))] } else { vec![] };
                items.push(shaped(x, y.floor(), s, size, r.u(2) as u8, mods, cl));
            }
            _ => {
                let (w, h) = (6u32, 2u32);
                let cells: Vec<GridCell> = (0..w * h).map(|i| GridCell { ch: if i % 5 == 1 { '日' } else { (b'a' + (i % 26) as u8) as char }, fg: r.col(), bg: r.col(), mods, skip: i % 5 == 2 }).collect();
                items.push(DisplayItem::CellGrid { rect: RectF::new(x.floor(), y.floor(), w as f32, h as f32), cells });
            }
        }
        if r.u(6) == 0 {
            items.insert(items.len() - 1, DisplayItem::PushClip { rect: RectF::new(r.r(0.0, 10.0), r.r(0.0, 5.0), r.r(5.0, 40.0), r.r(2.0, 10.0)) });
            items.push(DisplayItem::PopClip);
        }
    }
    lst(items)
}

fn run_diff(seeds: u64, limit_px: u32) -> Vec<(u64, f64, (u32, u32, u32, u32))> {
    let Some(ctx) = ctx() else { return vec![] };
    let mut rows_out = Vec::new();
    for seed in 1..=seeds {
        let scale = [1.0, 1.5, 2.0][(seed % 3) as usize];
        let l = rand_scene(seed, 70, 20);
        let (w, h, agg, worst) = cpu_gpu_tiles(&ctx, &l, 70, 20, scale);
        let pct = 100.0 * agg as f64 / (w as f64 * h as f64);
        rows_out.push((seed, pct, worst));
    }
    rows_out.sort_by(|a, b| b.2 .2.cmp(&a.2 .2));
    let max_agg = rows_out.iter().map(|r| r.1).fold(0.0, f64::max);
    eprintln!("differential: {} scenes, max aggregate {:.3}%", rows_out.len(), max_agg);
    for r in rows_out.iter().take(5) {
        eprintln!("  seed {} agg {:.3}% worst tile (tx,ty,diffpx,tilepx) {:?}", r.0, r.1, r.2);
    }
    let _ = limit_px;
    rows_out
}

#[test]
fn b_random_scene_differential_per_tile() {
    let rows = run_diff(40, 0);
    if rows.is_empty() { return; }
    let worst = rows[0].2;
    // 16x16 tile: 256 px. Aggregate gate in repo is 0.5% of the whole frame; a tile >25% wrong hides under it.
    assert!(worst.2 <= 64, "worst tile {worst:?} (seed {}) exceeds 25% of the tile", rows[0].0);
}

#[test]
#[ignore = "long differential fuzzer; run with --ignored"]
fn b_random_scene_differential_long() {
    let rows = run_diff(600, 0);
    assert!(rows.is_empty() || rows[0].2 .2 <= 64, "worst {:?}", rows[0]);
}

/// One scene per text feature, tight per-tile limit (reviewer F-02/F-12: the repo's 0.5% aggregate
/// cannot see these).
#[test]
fn b_feature_scenes_per_tile() {
    let Some(ctx) = ctx() else { return };
    let fg = tc(255, 255, 255);
    let bg = tc(20, 30, 90);
    let t = |m: Modifiers| lst(vec![fill(0.0, 0.0, 40.0, 4.0, 0.0, bg), mono(1.0, 1.0, "Hello GPU! Feature WWW", m, fg, bg)]);
    let s = |m: Modifiers| lst(vec![fill(0.0, 0.0, 40.0, 4.0, 0.0, bg), shaped(1.0, 1.0, "Hello GPU! Feature WWW", 18.0, FACE_SANS, m, vec![])]);
    let grid = |m: Modifiers| {
        let cells: Vec<GridCell> = "日本語ab語c".chars().enumerate().flat_map(|(i, c)| {
            let wide = (c as u32) > 0x2000;
            let mut v = vec![GridCell { ch: c, fg, bg, mods: m, skip: false }];
            if wide { v.push(GridCell { ch: ' ', fg, bg, mods: m, skip: true }); }
            let _ = i;
            v
        }).collect();
        lst(vec![DisplayItem::CellGrid { rect: RectF::new(1.0, 1.0, cells.len() as f32, 1.0), cells }])
    };
    let cases: Vec<(&str, DisplayList)> = vec![
        ("mono plain", t(no_mods())),
        ("mono bold", t(bold())),
        ("mono italic", t(Modifiers { italic: true, ..Default::default() })),
        ("mono underline", t(Modifiers { underline: true, ..Default::default() })),
        ("mono bold+italic+underline", t(Modifiers { bold: true, italic: true, underline: true })),
        ("sans bold", s(bold())),
        ("sans italic", s(Modifiers { italic: true, ..Default::default() })),
        ("sans underline", s(Modifiers { underline: true, ..Default::default() })),
        ("grid wide plain", grid(no_mods())),
        ("grid wide bold", grid(bold())),
        ("grid wide italic", grid(Modifiers { italic: true, ..Default::default() })),
    ];
    let mut fails = Vec::new();
    for (name, l) in &cases {
        for scale in [1.0, 2.0] {
            let (_, _, agg, worst) = cpu_gpu_tiles(&ctx, l, 40, 4, scale);
            eprintln!("{name:30} @{scale}: aggregate {agg:4} px, worst tile {:?}", worst);
            if worst.2 > 12 { fails.push(format!("{name}@{scale}: worst tile {} px of {}", worst.2, worst.3)); }
        }
    }
    assert!(fails.is_empty(), "per-tile divergences: {fails:#?}");
}

// ---------------------------------------------------------------- F-01 / buffer cap

/// F-01 via ONE `Text` item (cheap list): 4.3M glyphs -> >2^22 instances -> buffer > max_buffer_size.
/// Memory: 4.3M * 16 B glyphs (69 MB) + 2 x 4.3M * 48 B instance vecs (412 MB) + 2^23*48 B buffer attempt.
#[test]
#[ignore = "memory-heavy (~0.8 GB) and panics by design (confirms F-01 for the glyph path); run alone"]
fn b_f01_single_text_item_many_glyphs() {
    let Some(ctx) = ctx() else { return };
    let mut sink = GpuSink::new(ctx, 1.0);
    let n = (1usize << 22) + 100;
    let glyphs: Vec<Glyph> = (0..n).map(|i| Glyph { x: (i % 5000) as f32 * 0.01, y: 0.0, ch: 'a', advance: 0.01 }).collect();
    let l = lst(vec![DisplayItem::Text { node: NodeId(1), origin: PointF { x: 0.0, y: 0.0 }, clip: RectF::new(0.0, 0.0, 80.0, 1.0), glyphs, fg: tc(255, 255, 255), bg: tc(0, 0, 0), mods: no_mods(), shaped: None }]);
    let r = outcome(|| sink.render_cells(&l, 80, 24));
    eprintln!("F-01 glyph path: {r}");
    assert_eq!(r, "OK");
}

/// Attribution helper: for the worst random seeds, renders each drawing item alone (clip items kept)
/// and prints which item kinds diverge from the CPU reference and by how many pixels.
#[test]
#[ignore = "diagnostic: prints per-item divergence of random scenes (use --nocapture)"]
fn b_diff_attribution() {
    let Some(ctx) = ctx() else { return };
    let mut tot: std::collections::BTreeMap<String, (u32, u64, u32, u8)> = Default::default();
    for seed in 1..=60u64 {
        let scale = [1.0, 1.5, 2.0][(seed % 3) as usize];
        let l = rand_scene(seed, 70, 20);
        for (i, it) in l.items.iter().enumerate() {
            let kind = match it {
                DisplayItem::PushClip { .. } | DisplayItem::PopClip => continue,
                DisplayItem::Fill { radius, .. } => if *radius > 0.0 { "fill-round".to_string() } else { "fill".to_string() },
                DisplayItem::Border { radius, chars, .. } => format!("border-{}-r{}", chars.top_left, *radius > 0.0),
                DisplayItem::Text { shaped, mods, .. } => format!("text-{}-b{}i{}u{}", if shaped.is_some() { "sans" } else { "mono" }, mods.bold as u8, mods.italic as u8, mods.underline as u8),
                DisplayItem::CellGrid { cells, .. } => format!("grid-b{}", cells[0].mods.bold as u8),
                _ => "other".into(),
            };
            let only = DisplayList { items: l.items.iter().enumerate().filter(|(j, x)| *j == i || matches!(x, DisplayItem::PushClip { .. } | DisplayItem::PopClip)).map(|(_, x)| x.clone()).collect() };
            let (_, _, agg, worst) = cpu_gpu_tiles(&ctx, &only, 70, 20, scale);
            let e = tot.entry(kind).or_insert((0, 0, 0, 0));
            e.3 = e.3.max(LAST_MAXD.with(std::cell::Cell::get));
            e.0 += 1;
            e.1 += agg;
            e.2 = e.2.max(worst.2);
        }
    }
    for (k, (n, agg, worst, maxd)) in tot {
        eprintln!("{k:28} items {n:4} total diff px {agg:7} avg {:8.1} worst-tile {worst} max-chan-diff {maxd}", agg as f64 / n as f64);
    }
}

/// A co-user that replaces the device-wide uncaptured-error handler (legal on a shared device)
/// makes every sink validation failure invisible: `render_into_view` returns `Ok(())`.
#[test]
fn b_foreign_uncaptured_handler_hides_sink_errors() {
    let Some(ctx) = ctx() else { return };
    let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let s2 = seen.clone();
    ctx.device().on_uncaptured_error(Arc::new(move |_e: wgpu::Error| {
        s2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));
    let mut sink = GpuSink::new(ctx.clone(), 1.0);
    let tex = mk_tex(&ctx, 64, 64, wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureUsages::RENDER_ATTACHMENT, 1);
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let list = lst(vec![fill(1.0, 1.0, 3.0, 2.0, 0.0, tc(200, 10, 10))]);
    let r = outcome(|| sink.render_into_view(&list, &view, 64, 64));
    eprintln!("foreign handler: sink says {r}; handler saw {} errors", seen.load(std::sync::atomic::Ordering::SeqCst));
    assert!(r != "OK" || seen.load(std::sync::atomic::Ordering::SeqCst) == 0, "sink reports Ok(()) although wgpu rejected the frame");
}

/// Is the rounded-edge divergence AA noise or geometry? Prints diff histogram for single shapes.
#[test]
#[ignore = "diagnostic: histogram of channel diffs for one rounded shape per scale (use --nocapture)"]
fn b_rounded_diff_histogram() {
    let Some(ctx) = ctx() else { return };
    let shapes: Vec<(&str, DisplayItem)> = vec![
        ("fill r0.8 12x4", fill(1.0, 1.0, 12.0, 4.0, 0.8, tc(56, 189, 248))),
        ("fill r1.5 20x6", fill(1.0, 1.0, 20.0, 6.0, 1.5, tc(56, 189, 248))),
        ("border ╭ r0 12x4", DisplayItem::Border { node: NodeId(1), rect: RectF::new(1.0, 1.0, 12.0, 4.0), radius: 0.0, chars: BorderChars::rounded(), fg: tc(74, 222, 128), bg: AnsiColor::Reset }),
        ("border ┌ r0.9 12x4", DisplayItem::Border { node: NodeId(1), rect: RectF::new(1.0, 1.0, 12.0, 4.0), radius: 0.9, chars: BorderChars::single(), fg: tc(74, 222, 128), bg: AnsiColor::Reset }),
    ];
    for (name, it) in shapes {
        for scale in [1.0, 1.5, 2.0, 3.0] {
            let l = lst(vec![it.clone()]);
            let mut cpu = Rasterizer::new(scale);
            let mut surf = cpu.surface_for(16, 7).unwrap();
            cpu.render(&l, &mut surf);
            let mut gpu = GpuSink::new(ctx.clone(), scale);
            let img = gpu.render_cells(&l, 16, 7).unwrap();
            let mut h = [0u32; 5]; // >4, >8, >16, >32, >64
            for (a, b) in img.rgba().chunks(4).zip(surf.rgba().chunks(4)) {
                let d = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap();
                for (k, t) in [4u8, 8, 16, 32, 64].iter().enumerate() { if d > *t { h[k] += 1; } }
            }
            eprintln!("{name:20} @{scale}: px with maxchan diff >4:{} >8:{} >16:{} >32:{} >64:{}", h[0], h[1], h[2], h[3], h[4]);
        }
    }
}
