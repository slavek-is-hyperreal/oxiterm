use std::io::Write;
use anyhow::Result;
use tracing::debug;

#[derive(Debug, Clone, Default)]
pub struct TerminalProfile {
    pub supports_kitty_kbd: bool,
    pub supports_kitty_gfx: bool,
    pub supports_sgr_mouse: bool,
    pub color_depth: ColorDepth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorDepth {
    #[default]
    TrueColor,
    Color256,
    Color16,
}

impl TerminalProfile {
    pub fn parse_da1_response(&mut self, response: &[u8]) {
        let s = String::from_utf8_lossy(response);
        if s.contains("?64") || s.contains("?62") {
            // Level 4 or Level 2 terminal
            self.supports_sgr_mouse = true;
            self.color_depth = ColorDepth::TrueColor;
        }
        if s.contains(";4;") || s.contains(";4c") {
            self.supports_kitty_gfx = true; // Sixel/Graphics support
        }
    }
}

pub fn send_da1_query(writer: &mut impl Write) -> Result<()> {
    debug!("Sending DA1 query");
    writer.write_all(b"\x1b[c")?;
    writer.flush()?;
    Ok(())
}

pub fn enable_kitty_protocol(writer: &mut impl Write) -> Result<()> {
    debug!("Enabling Kitty Keyboard Protocol");
    // CSI = 1 u: Enable all features
    writer.write_all(b"\x1b[=1u")?;
    writer.flush()?;
    Ok(())
}

pub fn enable_sgr_mouse(writer: &mut impl Write) -> Result<()> {
    debug!("Enabling SGR Mouse Protocol");
    writer.write_all(b"\x1b[?1006h")?;
    writer.write_all(b"\x1b[?1000h")?; // Standard mouse tracking
    writer.flush()?;
    Ok(())
}

pub fn send_bsu(writer: &mut impl Write) -> Result<()> {
    writer.write_all(b"\x1b[?2026h")?;
    writer.flush()?;
    Ok(())
}

pub fn send_esu(writer: &mut impl Write) -> Result<()> {
    writer.write_all(b"\x1b[?2026l")?;
    writer.flush()?;
    Ok(())
}

pub fn negotiate_capabilities(writer: &mut impl Write, profile: &mut TerminalProfile) -> Result<()> {
    send_da1_query(writer)?;
    // In a real implementation, we would wait for the response and call parse_da1_response.
    // For now, we assume standard features if it's a modern terminal.
    enable_sgr_mouse(writer)?;
    profile.supports_sgr_mouse = true;
    Ok(())
}
