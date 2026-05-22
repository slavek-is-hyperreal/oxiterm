use anyhow::Result;
use crate::render::buffer::CellBuffer;

pub trait FrameSink: Send {
    /// Compares front and back buffers, sends any frame update to the sink,
    /// and returns Ok(true) if a frame was actually transmitted (i.e. diff was non-empty).
    fn send_frame(&mut self, front: &CellBuffer, back: &CellBuffer) -> Result<bool>;

    /// Perform any initial environment configuration/setup for the sink.
    fn setup(&mut self) -> Result<()> { Ok(()) }

    /// Clears the physical display area.
    fn clear_screen(&mut self) -> Result<()> { Ok(()) }
}

