//! THTML templates file loader utilities.
//!
//! Exposes helper functions to read and parse template documents from local files or strings.

use std::path::Path;
use anyhow::{Result, Context};
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::document::THTMLDocument;
use tracing::info;

/// Reads a THTML template file from the filesystem and parses it into a DOM document.
pub fn load_thtml_file<P: AsRef<Path>>(path: P) -> Result<THTMLDocument> {
    let path = path.as_ref();
    info!("Loading THTML file: {:?}", path);
    
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read THTML file: {:?}", path))?;
    
    let doc = THTMLParser::parse(&content)
        .map_err(|e| anyhow::anyhow!("THTML Parsing Error in {:?}: {}", path, e))?;
    
    Ok(doc)
}

/// Parses a THTML template direct string reference into a DOM document.
pub fn load_thtml_str(content: &str) -> Result<THTMLDocument> {
    let doc = THTMLParser::parse(content)
        .map_err(|e| anyhow::anyhow!("THTML Parsing Error: {}", e))?;
    Ok(doc)
}
