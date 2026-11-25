//! `oxifb`: render THTML pages with the OxiDE software sink.
//!
//! ```text
//! oxifb render <page.thtml> <out.png> [--cols N] [--rows N] [--scale S] [--examples DIR]
//! oxifb bench  <page.thtml> [--cols N] [--rows N] [--scale S] [--iters N]
//! oxifb fbdev  <page.thtml> --device /dev/fb0 --really-write-device [--cols N --rows N --scale S]   (feature `device`)
//! oxifb drm    <page.thtml> --device /dev/dri/card0 --really-write-device [--hold-secs N]          (feature `device`)
//! ```
//! `fbdev` and `drm` take over a real display. They exist only with `--features device`, refuse to
//! run without BOTH `--really-write-device` and the environment variable `OXIDE_ALLOW_DEVICE=1`,
//! and are never run by tests or CI.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use oxiterm_fb::{page, Rasterizer};

struct Args {
    cmd: String,
    pos: Vec<String>,
    cols: u16,
    rows: u16,
    scale: f64,
    iters: usize,
    examples: Option<PathBuf>,
    #[cfg_attr(not(feature = "device"), allow(dead_code))]
    device: Option<String>,
    #[cfg_attr(not(feature = "device"), allow(dead_code))]
    really: bool,
    #[cfg_attr(not(feature = "device"), allow(dead_code))]
    hold_secs: u64,
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or("missing command (render | bench | fbdev | drm)")?;
    let mut a = Args {
        cmd,
        pos: vec![],
        cols: 80,
        rows: 24,
        scale: 1.0,
        iters: 30,
        examples: None,
        device: None,
        really: false,
        hold_secs: 10,
    };
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--cols" => a.cols = val("--cols")?.parse().map_err(|e| format!("--cols: {e}"))?,
            "--rows" => a.rows = val("--rows")?.parse().map_err(|e| format!("--rows: {e}"))?,
            "--scale" => a.scale = val("--scale")?.parse().map_err(|e| format!("--scale: {e}"))?,
            "--iters" => a.iters = val("--iters")?.parse().map_err(|e| format!("--iters: {e}"))?,
            "--examples" => a.examples = Some(PathBuf::from(val("--examples")?)),
            "--device" => a.device = Some(val("--device")?),
            "--hold-secs" => a.hold_secs = val("--hold-secs")?.parse().map_err(|e| format!("--hold-secs: {e}"))?,
            "--really-write-device" => a.really = true,
            s if s.starts_with("--") => return Err(format!("unknown flag {s}")),
            _ => a.pos.push(arg),
        }
    }
    Ok(a)
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn run() -> Result<(), String> {
    let a = parse()?;
    let page_path = a.pos.first().ok_or("missing page path")?;
    let list = page::display_list_of_file(Path::new(page_path), a.examples.as_deref(), a.cols, a.rows)?;
    let mut r = Rasterizer::new(a.scale);
    let mut surface = r.surface_for(u32::from(a.cols), u32::from(a.rows)).ok_or("empty surface")?;
    match a.cmd.as_str() {
        "render" => {
            let out = a.pos.get(1).ok_or("missing output png path")?;
            r.render(&list, &mut surface);
            surface.save_png(Path::new(out))?;
            println!("{}x{} px, {} items -> {out}", surface.width(), surface.height(), list.items.len());
        }
        "bench" => {
            let mut cold = vec![];
            let mut warm = vec![];
            for i in 0..a.iters.max(2) {
                let mut r2 = Rasterizer::new(a.scale);
                let t = Instant::now();
                r2.render(&list, &mut surface);
                cold.push(t.elapsed().as_secs_f64() * 1e3);
                if i == 0 {
                    r.render(&list, &mut surface);
                }
                let t = Instant::now();
                r.render(&list, &mut surface);
                warm.push(t.elapsed().as_secs_f64() * 1e3);
            }
            println!(
                "{}x{} cells, scale {}: {}x{} px, {} items, median over {} runs: empty glyph cache {:.2} ms, warm cache {:.2} ms",
                a.cols,
                a.rows,
                a.scale,
                surface.width(),
                surface.height(),
                list.items.len(),
                cold.len(),
                median(&mut cold),
                median(&mut warm)
            );
        }
        #[cfg(feature = "device")]
        "fbdev" | "drm" => {
            use oxiterm_fb::device;
            let opt_in = device::require_opt_in(a.really)?;
            r.render(&list, &mut surface);
            let dev = a.device.as_deref().ok_or("--device is required")?;
            if a.cmd == "fbdev" {
                device::fbdev::present(Path::new(dev), &surface, &opt_in)?;
            } else {
                device::drm::present(Path::new(dev), &surface, std::time::Duration::from_secs(a.hold_secs), &opt_in)?;
            }
        }
        other => return Err(format!("unknown command {other:?} (device commands need `--features device`)")),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxifb: {e}");
            ExitCode::FAILURE
        }
    }
}
