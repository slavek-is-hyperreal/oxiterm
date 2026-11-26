//! Example CLI: render a `.thtml` page to PNG through the headless `wgpu` [`GpuSink`].
//!
//! Usage:
//! ```text
//! cargo run -p oxiterm-gpu --example render -- <page.thtml> <out.png> [--cols N] [--rows N] [--scale S] [--bench N]
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use oxiterm_gpu::{page, GpuContext, GpuSink};

struct Args {
    page: PathBuf,
    out: PathBuf,
    cols: u16,
    rows: u16,
    scale: f64,
    bench_iters: usize,
    examples: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut pos = Vec::new();
    let mut cols = 80u16;
    let mut rows = 24u16;
    let mut scale = 1.0f64;
    let mut bench_iters = 0usize;
    let mut examples = None;

    while let Some(arg) = it.next() {
        let mut next_val = |flag: &str| it.next().ok_or_else(|| format!("{flag} requires a value"));
        match arg.as_str() {
            "--cols" => cols = next_val("--cols")?.parse().map_err(|e| format!("--cols: {e}"))?,
            "--rows" => rows = next_val("--rows")?.parse().map_err(|e| format!("--rows: {e}"))?,
            "--scale" => scale = next_val("--scale")?.parse().map_err(|e| format!("--scale: {e}"))?,
            "--bench" => {
                bench_iters = next_val("--bench")?.parse().map_err(|e| format!("--bench: {e}"))?;
            }
            "--examples" => examples = Some(PathBuf::from(next_val("--examples")?)),
            "-h" | "--help" => {
                return Err(
                    "usage: render <page.thtml> <out.png> [--cols N] [--rows N] [--scale S] [--bench N] [--examples DIR]"
                        .into(),
                );
            }
            s if s.starts_with("--") => return Err(format!("unknown flag {s}")),
            _ => pos.push(arg),
        }
    }

    if pos.len() < 2 {
        return Err(
            "usage: render <page.thtml> <out.png> [--cols N] [--rows N] [--scale S] [--bench N]"
                .into(),
        );
    }

    Ok(Args {
        page: PathBuf::from(&pos[0]),
        out: PathBuf::from(&pos[1]),
        cols,
        rows,
        scale,
        bench_iters,
        examples,
    })
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let ctx = Arc::new(GpuContext::new_headless().map_err(|e| e.to_string())?);
    let info = ctx.adapter_info().clone();

    let list = page::display_list_of_file(
        &args.page,
        args.examples.as_deref(),
        args.cols,
        args.rows,
    )?;

    let mut sink = GpuSink::new(ctx, args.scale);

    let t0 = Instant::now();
    let frame = sink
        .render_cells(&list, u32::from(args.cols), u32::from(args.rows))
        .map_err(|e| e.to_string())?;
    let cold_ms = t0.elapsed().as_secs_f64() * 1e3;

    let stats = sink.last_stats();
    if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("{}: {e}", parent.display()))?;
        }
    }
    frame.save_png(Path::new(&args.out)).map_err(|e| e.to_string())?;

    println!(
        "oxiterm-gpu: {}x{} px ({}x{} cells @ {:.2}x) on {} ({:?}/{:?}) | {} items -> {} sdf, {} glyphs, {} batches | cold {:.2} ms -> {}",
        frame.width(),
        frame.height(),
        args.cols,
        args.rows,
        args.scale,
        info.name,
        info.backend,
        info.device_type,
        list.items.len(),
        stats.sdf_instances,
        stats.glyph_instances,
        stats.draw_batches,
        cold_ms,
        args.out.display()
    );

    if args.bench_iters > 0 {
        let mut times = Vec::with_capacity(args.bench_iters);
        for _ in 0..args.bench_iters {
            let t = Instant::now();
            let _ = sink
                .render_cells(&list, u32::from(args.cols), u32::from(args.rows))
                .map_err(|e| e.to_string())?;
            times.push(t.elapsed().as_secs_f64() * 1e3);
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = times[times.len() / 2];
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        println!(
            "warm ({} iters, incl. GPU readback): median {:.2} ms, mean {:.2} ms, min {:.2} ms, max {:.2} ms",
            times.len(),
            median,
            mean,
            times[0],
            times[times.len() - 1]
        );
    }

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("render: {e}");
            ExitCode::FAILURE
        }
    }
}
