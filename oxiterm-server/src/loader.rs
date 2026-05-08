use std::path::Path;
use anyhow::{Result, Context};
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::document::THTMLDocument;
use tracing::info;

pub fn load_thtml_file<P: AsRef<Path>>(path: P) -> Result<THTMLDocument> {
    let path = path.as_ref();
    info!("Loading THTML file: {:?}", path);
    
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read THTML file: {:?}", path))?;
    
    let doc = THTMLParser::parse(&content)
        .map_err(|e| anyhow::anyhow!("THTML Parsing Error in {:?}: {}", path, e))?;
    
    Ok(doc)
}
