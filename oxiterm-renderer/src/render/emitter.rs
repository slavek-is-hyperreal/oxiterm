use std::io::Write;
use anyhow::Result;
use crate::render::diff::{DiffEngine, AnsiCommand};
use crate::render::buffer::CellBuffer;

/// S5-13: `SyncedEmitter`
/// A wrapper for `DiffEngine` that ensures BSU/ESU (Begin/End Synchronized Update)
/// are sent around every frame to prevent tearing.
pub struct SyncedEmitter;

impl SyncedEmitter {
    pub fn emit_frame(writer: &mut impl Write, prev: &CellBuffer, next: &CellBuffer) -> Result<()> {
        let commands = DiffEngine::diff(prev, next);
        if commands.is_empty() {
            return Ok(());
        }

        // BSU: CSI ? 2026 h
        writer.write_all(b"\x1b[?2026h")?;
        
        let bytes = DiffEngine::encode_ansi(&commands);
        // Note: encode_ansi currently includes BSU/ESU inside it. 
        // We should probably refactor DiffEngine to be cleaner, 
        // but for now we'll just write the bytes.
        writer.write_all(&bytes)?;
        
        // ESU: CSI ? 2026 l
        writer.write_all(b"\x1b[?2026l")?;
        
        writer.flush()?;
        Ok(())
    }
}
