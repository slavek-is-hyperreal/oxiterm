use anyhow::Result;
use tracing::debug;
use russh::server::Session;
use russh::ChannelId;

pub use oxiterm_proto::style::{TerminalProfile, ColorDepth};

pub fn send_da1_query(channel: ChannelId, session: &mut Session) -> Result<()> {
    debug!("Sending DA1 query to channel {:?}", channel);
    session.data(channel, b"\x1b[c".to_vec().into());
    Ok(())
}

/// S6-01: Send test APC sequence to probe Kitty Graphics support
pub fn probe_kitty_graphics(channel: ChannelId, session: &mut Session) -> Result<()> {
    debug!("Sending Kitty Graphics probe to channel {:?}", channel);
    // Send a query check command: query support for graphics using a 1x1 dummy pixel
    session.data(channel, b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\".to_vec().into());
    Ok(())
}

/// S6-02: Recognize OK response from emulator for Kitty Graphics
pub fn parse_kitty_ack(buf: &[u8]) -> bool {
    let s = String::from_utf8_lossy(buf);
    s.starts_with("\x1b_G") && s.contains("OK")
}

/// S6-03: Check if terminal profile supports Sixel
pub fn probe_sixel_support(profile: &TerminalProfile) -> bool {
    profile.supports_sixel
}

pub fn enable_kitty_protocol(channel: ChannelId, session: &mut Session) -> Result<()> {
    debug!("Enabling Kitty Keyboard Protocol on channel {:?}", channel);
    session.data(channel, b"\x1b[=1u".to_vec().into());
    Ok(())
}

pub fn enable_sgr_mouse(channel: ChannelId, session: &mut Session) -> Result<()> {
    debug!("Enabling SGR Mouse Protocol on channel {:?}", channel);
    session.data(channel, b"\x1b[?1006h".to_vec().into());
    session.data(channel, b"\x1b[?1000h".to_vec().into());
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

pub fn send_unicode_version_osc(channel: ChannelId, session: &mut Session, version: u8) -> Result<()> {
    let osc = format!("\x1b]52;u;{version}\x1b\\");
    session.data(channel, osc.into_bytes().into());
    Ok(())
}

pub fn negotiate_capabilities(channel: ChannelId, session: &mut Session) -> Result<()> {
    send_da1_query(channel, session)?;
    probe_kitty_graphics(channel, session)?;
    enable_sgr_mouse(channel, session)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xterm_da1() {
        let mut profile = TerminalProfile::default();
        // Typical XTerm response
        profile.parse_da1_response(b"\x1b[?64;1;2;6;9;15;22c");
        assert!(profile.supports_sgr_mouse);
        assert_eq!(profile.color_depth, ColorDepth::TrueColor);
        assert!(!profile.supports_sixel);
        assert!(!profile.supports_kitty_gfx);
    }

    #[test]
    fn test_iterm2_da1() {
        let mut profile = TerminalProfile::default();
        // iTerm2 responds with Sixel support (contains ";4;")
        profile.parse_da1_response(b"\x1b[?63;1;2;4;10;15;22c");
        assert!(profile.supports_sgr_mouse);
        assert!(profile.supports_sixel);
        assert!(!profile.supports_kitty_gfx);
    }

    #[test]
    fn test_wezterm_da1() {
        let mut profile = TerminalProfile::default();
        // WezTerm typical DA1
        profile.parse_da1_response(b"\x1b[?65;1;9c");
        assert!(profile.supports_sgr_mouse);
        assert!(!profile.supports_sixel);
    }

    #[test]
    fn test_dec_vt340_da1() {
        let mut profile = TerminalProfile::default();
        profile.parse_da1_response(b"\x1b[?63;1;2;4c");
        assert!(profile.supports_sixel);
    }

    #[test]
    fn test_kitty_graphics_apc_ack() {
        let mut profile = TerminalProfile::default();
        profile.parse_da1_response(b"\x1b_Gi=31;OK\x1b\\");
        assert!(profile.supports_kitty_gfx);
        assert!(parse_kitty_ack(b"\x1b_Gi=31;OK\x1b\\"));
    }
}
