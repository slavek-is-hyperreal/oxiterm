//! THTML templates file loader utilities.
//!
//! Exposes helper functions to read and parse template documents from local files or strings.

use std::path::Path;
use anyhow::{Result, Context};
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::document::THTMLDocument;
use tracing::{info, warn};

/// Logs a warning for every clickable label containing ambiguous-width glyphs, which
/// render at different widths across terminals and make the label drift off its hit box.
/// `label` identifies the source (file path or "<inline>") in the message.
fn warn_ambiguous_clickables(doc: &THTMLDocument, label: &str) {
    for (node, text, bad) in doc.clickable_ambiguous_width() {
        warn!(
            "Clickable label {:?} (node {:?}) in {} contains ambiguous-width char(s) {:?}; \
             clicks may miss on terminals that render them wide. Use unambiguous glyphs \
             (e.g. '<' instead of '←').",
            text, node, label, bad,
        );
    }
}

/// Reads a THTML template file from the filesystem and parses it into a DOM document.
pub fn load_thtml_file<P: AsRef<Path>>(path: P) -> Result<THTMLDocument> {
    let path = path.as_ref();
    info!("Loading THTML file: {:?}", path);

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read THTML file: {:?}", path))?;

    let doc = THTMLParser::parse(&content)
        .map_err(|e| anyhow::anyhow!("THTML Parsing Error in {:?}: {}", path, e))?;
    warn_ambiguous_clickables(&doc, &path.display().to_string());

    Ok(doc)
}

/// Parses a THTML template direct string reference into a DOM document.
pub fn load_thtml_str(content: &str) -> Result<THTMLDocument> {
    let doc = THTMLParser::parse(content)
        .map_err(|e| anyhow::anyhow!("THTML Parsing Error: {}", e))?;
    warn_ambiguous_clickables(&doc, "<inline>");
    Ok(doc)
}
