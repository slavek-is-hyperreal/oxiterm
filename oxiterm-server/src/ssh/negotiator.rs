// Removed unused std::io::Write
use anyhow::Result;
use tracing::debug;
use russh::server::Session;
use russh::ChannelId;

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
        debug!("Parsing DA1 response: {}", s);
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

pub fn send_da1_query(channel: ChannelId, session: &mut Session) -> Result<()> {
    debug!("Sending DA1 query to channel {:?}", channel);
    session.data(channel, b"\x1b[c".to_vec().into());
    Ok(())
}

pub fn enable_kitty_protocol(channel: ChannelId, session: &mut Session) -> Result<()> {
    debug!("Enabling Kitty Keyboard Protocol on channel {:?}", channel);
    // CSI = 1 u: Enable all features
    session.data(channel, b"\x1b[=1u".to_vec().into());
    Ok(())
}

pub fn enable_sgr_mouse(channel: ChannelId, session: &mut Session) -> Result<()> {
    debug!("Enabling SGR Mouse Protocol on channel {:?}", channel);
    session.data(channel, b"\x1b[?1006h".to_vec().into());
    session.data(channel, b"\x1b[?1000h".to_vec().into()); // Standard mouse tracking
    Ok(())
}

pub fn send_bsu(channel: ChannelId, session: &mut Session) -> Result<()> {
    session.data(channel, b"\x1b[?2026h".to_vec().into());
    Ok(())
}

pub fn send_esu(channel: ChannelId, session: &mut Session) -> Result<()> {
    session.data(channel, b"\x1b[?2026l".to_vec().into());
    Ok(())
}

/// S5-26: Send Unicode version OSC (for VTM compliance).
pub fn send_unicode_version_osc(channel: ChannelId, session: &mut Session, version: u8) -> Result<()> {
    let osc = format!("\x1b]52;u;{version}\x1b\\");
    session.data(channel, osc.into_bytes().into());
    Ok(())
}

pub fn negotiate_capabilities(channel: ChannelId, session: &mut Session) -> Result<()> {
    send_da1_query(channel, session)?;
    // We don't wait here because this is async and event-driven.
    // The response will be caught by the EventLoop.
    // We can also pro-actively enable common features.
    enable_sgr_mouse(channel, session)?;
    Ok(())
}
