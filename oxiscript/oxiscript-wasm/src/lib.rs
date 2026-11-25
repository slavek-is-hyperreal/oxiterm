//! OxiIR -> Core WebAssembly code generator (Phase 8, slice 8.G).
//!
//! [`compile`] verifies the IR (with the trusted [`VerifyOptions`] and the
//! typed host boundary against the granted IDL interfaces), lowers it to Core
//! Wasm restricted to exactly the ABI feature set ([`allowed_features`]),
//! and validates its own output with `wasmparser`.
//!
//! Control flow is structured with the "Beyond Relooper" algorithm (reverse
//! postorder + dominators; irreducible CFGs are rejected with
//! [`E_IRREDUCIBLE`]). Every SSA value lives in a Wasm local; block parameters
//! are locals assigned from the operand stack (a parallel move). Compiler
//! raised traps are `global.set $oxi_trap_kind; unreachable`.
//!
//! Spec: `docs/oxiscript/{ir,abi,contracts}.md`.
#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::wildcard_imports,
    clippy::match_same_arms,
    clippy::too_many_arguments,
    clippy::needless_pass_by_value
)]

mod body;
mod cfg;
mod cx;
mod func;
mod helpers;
mod modlayout;
mod stubrt;
mod wrap;

use oxiscript_abi::idl::Interface;
use oxiscript_abi::manifest::Need;
use oxiscript_ir::{Module, VerifyOptions};
use std::fmt;
use wasmparser::{Validator, WasmFeatures};

pub use cx::RT_MODULE;

/// Codegen diagnostic codes (appended to `oxiscript-abi/registry/codes.txt`).
pub const E_IRREDUCIBLE: &str = "OXS-E0600";
pub const E_UNSUPPORTED: &str = "OXS-E0601";
pub const E_VERIFY: &str = "OXS-E0602";
pub const E_LIMIT: &str = "OXS-E0603";
pub const E_CONFIG: &str = "OXS-E0604";
pub const I_INTERNAL: &str = "OXS-I0002";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub func: Option<String>,
}

impl Diagnostic {
    pub(crate) fn new(code: &'static str, message: impl Into<String>, func: Option<String>) -> Self {
        Diagnostic {
            code,
            message: message.into(),
            func,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.func {
            Some(n) => write!(f, "{}: @{n}: {}", self.code, self.message),
            None => write!(f, "{}: {}", self.code, self.message),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, d) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}

/// Where the runtime-support functions (`oxi_alloc`, `oxi_retain`, ...) come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Embed the **test stub** runtime (bump allocator, exact counts): the
    /// module is complete and loads in `oxiscript-runtime`.
    Stub,
    /// Import the runtime functions from the Wasm module [`RT_MODULE`]
    /// (`oxi:rt`), field = ABI export name (plus `oxi_rt_init(i32, i32)`).
    /// The result is meant to be statically linked with the real runtime
    /// (slice 8.M) and is not loadable on its own.
    Import,
}

#[derive(Clone, Debug)]
pub struct CompileOptions {
    /// Granted host interfaces (typed-boundary check and callback trampolines).
    pub interfaces: Vec<Interface>,
    /// Manifest `needs` in `oxi_init` parameter order. `None`: derived from the
    /// module's imports in order of first use.
    pub needs: Option<Vec<Need>>,
    /// Trusted verification options (only the toolchain sets these).
    pub verify: VerifyOptions,
    pub runtime: RuntimeMode,
    /// Validate the output with `wasmparser` and the exact ABI feature set.
    pub validate_output: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        CompileOptions {
            interfaces: Vec::new(),
            needs: None,
            verify: VerifyOptions::default(),
            runtime: RuntimeMode::Stub,
            validate_output: true,
        }
    }
}

/// Statistics of one compilation.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub functions: usize,
    pub bytes: usize,
    pub static_end: u32,
}

pub struct Output {
    pub wasm: Vec<u8>,
    pub stats: Stats,
}

/// The exact Core Wasm feature set of ABI 1.0: MVP, multi-value, bulk memory,
/// sign extension, mutable globals. Everything else (`trunc_sat`, reference
/// types, tail calls, SIMD, multi-memory, memory64, ...) is off.
pub fn allowed_features() -> WasmFeatures {
    WasmFeatures::WASM1
        | WasmFeatures::MULTI_VALUE
        | WasmFeatures::BULK_MEMORY
        | WasmFeatures::SIGN_EXTENSION
}

/// Validate `wasm` against [`allowed_features`].
pub fn validate(wasm: &[u8]) -> Result<(), String> {
    Validator::new_with_features(allowed_features())
        .validate_all(wasm)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Compile a module to Core Wasm bytes.
pub fn compile(module: &Module, opts: &CompileOptions) -> Result<Vec<u8>, Diagnostics> {
    compile_with_stats(module, opts).map(|o| o.wasm)
}

pub fn compile_with_stats(module: &Module, opts: &CompileOptions) -> Result<Output, Diagnostics> {
    compile_inner(module, opts).map_err(|d| Diagnostics(vec![d]))
}

fn compile_inner(m: &Module, opts: &CompileOptions) -> Result<Output, Diagnostic> {
    let errs = oxiscript_ir::host::verify_with_interfaces(m, opts.verify, &opts.interfaces);
    if !errs.is_empty() {
        // report every verifier error
        let d = errs
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(Diagnostic::new(E_VERIFY, d, None));
    }
    let lay = modlayout::Layout::build(m)?;
    let static_end = lay.static_end;
    let mut cx = cx::Cx::new(m, opts, lay)?;
    cx.resolve_needs()?;
    wrap::gen_all(&mut cx)?;
    let functions = cx.defs.len();
    let wasm = cx.assemble(static_end)?;
    if opts.validate_output {
        validate(&wasm).map_err(|e| {
            Diagnostic::new(I_INTERNAL, format!("generated module failed validation: {e}"), None)
        })?;
    }
    let bytes = wasm.len();
    Ok(Output {
        wasm,
        stats: Stats {
            functions,
            bytes,
            static_end,
        },
    })
}
