use oxiterm_proto::style::AnsiColor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: AnsiColor,
    pub bg: AnsiColor,
    pub bold: bool,
    pub underline: bool,
    pub italic: bool,
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

pub struct CellBuffer {
    pub cells: Vec<Cell>,
    pub width: u16,
    pub height: u16,
    pub graphics: Vec<Vec<u8>>,
}

impl CellBuffer {
    pub fn new(width: u16, height: u16) -> Self {
        let size = width as usize * height as usize;
        Self {
            cells: vec![Cell::default(); size],
            width,
            height,
            graphics: Vec::new(),
        }
    }

    pub fn flat_idx(&self, x: u16, y: u16) -> Option<usize> {
        if x < self.width && y < self.height {
            Some(y as usize * self.width as usize + x as usize)
        } else {
            None
        }
    }

    pub fn set(&mut self, x: u16, y: u16, cell: Cell) {
        if let Some(idx) = self.flat_idx(x, y) {
            self.cells[idx] = cell;
        }
    }

    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
        self.graphics.clear();
    }
}


pub struct DoubleBuffer {
    pub front: CellBuffer,
    pub back: CellBuffer,
}

impl DoubleBuffer {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            front: CellBuffer::new(width, height),
            back: CellBuffer::new(width, height),
        }
    }

    pub fn swap(&mut self) {
        std::mem::swap(&mut self.front, &mut self.back);
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
