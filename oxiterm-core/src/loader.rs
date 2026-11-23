//! Pure THTML string loading (no filesystem). The file loader lives in `oxiterm-server`.

use anyhow::Result;
use oxiterm_proto::style::RenderMode;
use oxiterm_renderer::document::THTMLDocument;
use oxiterm_renderer::parser::THTMLParser;
use tracing::warn;

/// Logs a warning for every clickable label containing ambiguous-width glyphs, which
/// render at different widths across terminals and make the label drift off its hit box.
/// `label` identifies the source (file path or "<inline>") in the message.
pub fn warn_ambiguous_clickables(doc: &THTMLDocument, label: &str) {
    for (node, text, symbols, letters) in doc.clickable_ambiguous_width() {
        if !symbols.is_empty() {
            warn!(
                "Clickable label {:?} (node {:?}) in {} contains ambiguous-width symbol(s) {:?}; \
                 clicks may miss on terminals that render them wide. Use unambiguous glyphs \
                 (e.g. '<' instead of '←').",
                text, node, label, symbols,
            );
        }
        if !letters.is_empty() {
            tracing::debug!(
                "Clickable label {:?} (node {:?}) in {} contains ambiguous-width letter(s) {:?}.",
                text, node, label, letters,
            );
        }
    }
}

/// Parses a THTML template direct string reference into a DOM document.
pub fn load_thtml_str(content: &str) -> Result<THTMLDocument> {
    let doc = THTMLParser::parse(content)
        .map_err(|e| anyhow::anyhow!("THTML Parsing Error: {}", e))?;
    warn_ambiguous_clickables(&doc, "<inline>");
    Ok(doc)
}

/// Parses `content` with `mode` (like the server's `load_thtml_file_mode` but from memory).
/// `label` identifies the source in warnings.
pub fn load_thtml_str_mode(content: &str, mode: RenderMode, label: &str) -> Result<THTMLDocument> {
    let doc = THTMLParser::parse_with_mode(content, mode)
        .map_err(|e| anyhow::anyhow!("THTML Parsing Error in {}: {}", label, e))?;
    warn_ambiguous_clickables(&doc, label);
    Ok(doc)
}
