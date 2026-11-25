//! `oxi`: the OxiScript command-line tool (slice 8.D, DevKit v0).
//!
//! Exit codes (all commands): `0` success, `1` findings (errors reported,
//! `fmt --check` would change a file), `2` usage / I/O / configuration error,
//! `3` the command exists but needs a part of the toolchain that is not merged
//! yet (`run`, `build`).
#![allow(
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    clippy::many_single_char_names,
    clippy::format_push_string,
    clippy::doc_markdown,
    clippy::match_same_arms,
    clippy::single_match_else
)]

mod check;
mod examples;
mod explain;
mod fmtcmd;
mod hints;
mod newcmd;
mod project;
mod report;
mod runcmd;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "oxi", version, about = "The OxiScript toolchain: check, fmt, explain, new, run, build")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Type-check files or directories (default: the current directory).
    Check {
        paths: Vec<PathBuf>,
        /// Machine-readable output (schema in SCHEMA.md).
        #[arg(long)]
        json: bool,
        /// Read one source from stdin (name it with --stdin-path).
        #[arg(long)]
        stdin: bool,
        /// Path used for stdin input (project lookup, messages).
        #[arg(long, value_name = "PATH")]
        stdin_path: Option<PathBuf>,
        /// Apply machine-applicable fixes to the files, then check again.
        #[arg(long)]
        fix: bool,
    },
    /// Format files in the canonical style (no options).
    Fmt {
        paths: Vec<PathBuf>,
        /// Report files that would change; change nothing.
        #[arg(long)]
        check: bool,
        /// Format stdin to stdout.
        #[arg(long)]
        stdin: bool,
    },
    /// Explain a reason code, for example `OXS-E0102`.
    Explain { code: String },
    /// Create a project.
    New {
        name: String,
        /// blink | touch-button | sensor-dashboard | oxiui-page
        #[arg(long, default_value = "blink")]
        template: String,
    },
    /// Compile and run a program (needs 8.L lowering).
    Run { paths: Vec<PathBuf> },
    /// Compile a program to Wasm (needs 8.L lowering).
    Build { paths: Vec<PathBuf> },
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.cmd {
        Cmd::Check { paths, json, stdin, stdin_path, fix } => check::run(&check::Args { paths, json, stdin, stdin_path, fix }),
        Cmd::Fmt { paths, check, stdin } => fmtcmd::run(&fmtcmd::Args { paths, check, stdin }),
        Cmd::Explain { code } => explain::run(&code),
        Cmd::New { name, template } => newcmd::run(&name, &template),
        Cmd::Run { paths } => runcmd::run("run", &paths),
        Cmd::Build { paths } => runcmd::run("build", &paths),
    };
    std::process::exit(code);
}
