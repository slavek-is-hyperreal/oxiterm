//! THTML templates file loader utilities.
//!
//! Exposes helper functions to read and parse template documents from local files or strings.

use std::path::Path;
use anyhow::{Result, Context};
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_proto::style::RenderMode;
use oxiterm_renderer::document::THTMLDocument;
use tracing::info;

/// Reads a THTML template file from the filesystem and parses it into a DOM document.
pub fn load_thtml_file<P: AsRef<Path>>(path: P) -> Result<THTMLDocument> {
    load_thtml_file_mode(path, RenderMode::Tui)
}

/// Like [`load_thtml_file`], but resolves `@media` render-mode rules for `mode`
/// (`RenderMode::Gui` for display-list web clients).
pub fn load_thtml_file_mode<P: AsRef<Path>>(path: P, mode: RenderMode) -> Result<THTMLDocument> {
    let path = path.as_ref();
    info!("Loading THTML file: {:?}", path);

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read THTML file: {:?}", path))?;

    let doc = THTMLParser::parse_with_mode(&content, mode)
        .map_err(|e| anyhow::anyhow!("THTML Parsing Error in {:?}: {}", path, e))?;
    warn_ambiguous_clickables(&doc, &path.display().to_string());

    Ok(doc)
}

pub use oxiterm_core::loader::{load_thtml_str, load_thtml_str_mode, warn_ambiguous_clickables};
