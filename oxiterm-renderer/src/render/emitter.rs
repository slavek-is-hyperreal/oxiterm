//! Frame emission and screen update traits.
//!
//! Defines the [`FrameSink`] trait used to write computed visual cell buffers
//! and accessibility documents to various output terminals.

use anyhow::Result;
use crate::render::buffer::CellBuffer;

/// A sink representing an output channel for rendering frames (e.g. SSH session, raw terminal).
pub trait FrameSink: Send {
    /// Compares front and back buffers, sends the diff update to the output channel.
    ///
    /// Returns `Ok(true)` if frame drawing commands were actually transmitted.
    fn send_frame(&mut self, front: &CellBuffer, back: &CellBuffer) -> Result<bool>;

    /// Sends a display list instead of a cell diff (OxiUI phase 3). Sinks that do not
    /// support display lists return `Ok(false)` and the caller keeps using `send_frame`.
    ///
    /// Returns `Ok(true)` if a frame was actually transmitted.
    fn send_display_list(&mut self, _list: &crate::render::display_list::DisplayList) -> Result<bool> { Ok(false) }

    /// Sends the ARIA-mirror JSON document (OxiUI phase 3c, wire tag `0x37`) that accompanies
    /// a display list. Sinks without such a channel ignore it and return `Ok(false)`.
    ///
    /// Returns `Ok(true)` if a message was actually transmitted (it is skipped when unchanged).
    fn send_a11y(&mut self, _json: &[u8]) -> Result<bool> { Ok(false) }

    /// Updates the accessibility tree or client document representation.
    fn update_document(&mut self, _doc: &crate::document::THTMLDocument) -> Result<()> { Ok(()) }

    /// Initializes and configures the output terminal environment.
    fn setup(&mut self) -> Result<()> { Ok(()) }

    /// Sends a clear screen sequence to the output terminal.
    fn clear_screen(&mut self) -> Result<()> { Ok(()) }

    /// Sends commands to clear existing graphics layout objects.
    fn clear_graphics(&mut self) -> Result<()> { Ok(()) }
}
