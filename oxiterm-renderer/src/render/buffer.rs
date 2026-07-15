//! Double-buffered screen character grid management.
//!
//! Defines character cells, layouts, buffer swaps, and flat-index conversions
//! used to represent character layout frames before drawing them to the terminal.

use oxiterm_proto::style::AnsiColor;

/// A single cell on the terminal screen representing a character and its styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// The decoded character in the cell.
    pub ch: char,
    /// Foreground text color.
    pub fg: AnsiColor,
    /// Background cell color.
    pub bg: AnsiColor,
    /// Render character in bold font.
    pub bold: bool,
    /// Draw underline under the character.
    pub underline: bool,
    /// Render character in italic font.
    pub italic: bool,
    /// Mark to skip rendering this cell (used for the second half of wide Unicode characters).
    pub skip: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: AnsiColor::Reset,
            bg: AnsiColor::Reset,
            bold: false,
            underline: false,
            italic: false,
            skip: false,
        }
    }
}

/// A linear grid buffer representing the full terminal screen layout.
pub struct CellBuffer {
    /// Flat vector representing the 2D grid of character cells.
    pub cells: Vec<Cell>,
    /// Screen width in characters.
    pub width: u16,
    /// Screen height in characters.
    pub height: u16,
    /// Embedded raw graphical payloads (e.g. Kitty Graphic Escape blocks).
    pub graphics: Vec<Vec<u8>>,
}

impl CellBuffer {
    /// Creates a new cell buffer with the given dimensions.
    pub fn new(width: u16, height: u16) -> Self {
        let size = width as usize * height as usize;
        Self {
            cells: vec![Cell::default(); size],
            width,
            height,
            graphics: Vec::new(),
        }
    }

    /// Converts 2D coordinates to a 1D flat index inside the cells vector.
    pub fn flat_idx(&self, x: u16, y: u16) -> Option<usize> {
        if x < self.width && y < self.height {
            Some(y as usize * self.width as usize + x as usize)
        } else {
            None
        }
    }

    /// Writes a cell value at the specified coordinate.
    pub fn set(&mut self, x: u16, y: u16, cell: Cell) {
        if let Some(idx) = self.flat_idx(x, y) {
            self.cells[idx] = cell;
        }
    }

    /// Resets the buffer to the default empty state, clearing graphics.
    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
        self.graphics.clear();
    }

    /// Sets all cells in the buffer to a distinct, unmatching state to force a full redraw.
    pub fn force_dirty(&mut self) {
        for cell in &mut self.cells {
            cell.ch = '\0';
            cell.fg = AnsiColor::Reset;
            cell.bg = AnsiColor::Reset;
            cell.bold = false;
            cell.underline = false;
            cell.italic = false;
            cell.skip = false;
        }
        self.graphics.clear();
    }
}

/// A double-buffering controller managing swap frames to minimize terminal redrawing flickers.
pub struct DoubleBuffer {
    /// Front buffer currently visible on the screen.
    pub front: CellBuffer,
    /// Back buffer where the next frame is written.
    pub back: CellBuffer,
}

impl DoubleBuffer {
    /// Creates a new double buffer.
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            front: CellBuffer::new(width, height),
            back: CellBuffer::new(width, height),
        }
    }

    /// Swaps the front and back buffers.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.front, &mut self.back);
    }

    /// Forces a full repaint by dirtying the front buffer.
    pub fn force_dirty(&mut self) {
        self.front.force_dirty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_creation() {
        let buf = CellBuffer::new(80, 24);
        assert_eq!(buf.width, 80);
        assert_eq!(buf.height, 24);
        assert_eq!(buf.cells.len(), 80 * 24);
    }

    #[test]
    fn test_buffer_clear() {
        let mut buf = CellBuffer::new(10, 10);
        buf.cells[0].ch = 'X';
        buf.clear();
        assert_eq!(buf.cells[0].ch, ' ');
    }
}
