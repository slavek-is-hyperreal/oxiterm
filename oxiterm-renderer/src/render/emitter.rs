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

    /// Updates the accessibility tree or client document representation.
    fn update_document(&mut self, _doc: &crate::document::THTMLDocument) -> Result<()> { Ok(()) }

    /// Initializes and configures the output terminal environment.
    fn setup(&mut self) -> Result<()> { Ok(()) }

    /// Sends a clear screen sequence to the output terminal.
    fn clear_screen(&mut self) -> Result<()> { Ok(()) }

    /// Sends commands to clear existing graphics layout objects.
    fn clear_graphics(&mut self) -> Result<()> { Ok(()) }
}
