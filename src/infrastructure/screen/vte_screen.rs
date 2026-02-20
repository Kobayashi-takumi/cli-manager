use std::collections::HashMap;

use crate::domain::primitive::{Cell, Color, CursorPos, CursorStyle, NotificationEvent, SearchMatch, TerminalId, TerminalSize};
use crate::interface_adapter::port::screen_port::ScreenPort;
use crate::shared::error::AppError;

use super::osc7::parse_osc7_uri;

/// Internal state for a single screen buffer.
struct ScreenInstance {
    cells: Vec<Vec<Cell>>,
    cursor: CursorPos,
    size: TerminalSize,
    // SGR state
    current_fg: Color,
    current_bg: Color,
    current_bold: bool,
    current_underline: bool,
    current_italic: bool,
    current_dim: bool,
    current_reverse: bool,
    current_strikethrough: bool,
    current_hidden: bool,
    // Scroll region (DECSTBM)
    scroll_top: u16,    // 0-indexed top row of scroll region
    scroll_bottom: u16, // 0-indexed bottom row of scroll region
    // Alternate screen buffer state
    saved_primary_cells: Option<Vec<Vec<Cell>>>,
    saved_primary_cursor: Option<CursorPos>,
    is_alternate_screen: bool,
    // Saved cursor position (SCP/RCP)
    saved_cursor: Option<CursorPos>,
    // Saved cursor position + SGR state (DECSC/DECRC via ESC 7/8)
    saved_cursor_dec: Option<SavedCursorState>,
    // DEC Private Mode flags
    cursor_visible: bool,         // DECTCEM: default true
    autowrap: bool,               // DECAWM: default true
    application_cursor_keys: bool, // DECCKM: default false
    bracketed_paste: bool,        // default false
    // Saved primary screen scroll region for alternate screen switching
    saved_primary_scroll_top: Option<u16>,
    saved_primary_scroll_bottom: Option<u16>,
    // Window title set by OSC 0/2
    title: Option<String>,
    // Current working directory set by OSC 7
    cwd: Option<String>,
    // Notification event queue (BEL, OSC 9, OSC 777)
    notifications: Vec<NotificationEvent>,
}

/// Saved cursor state for DECSC/DECRC (ESC 7/8).
struct SavedCursorState {
    cursor: CursorPos,
    fg: Color,
    bg: Color,
    bold: bool,
    underline: bool,
    italic: bool,
    dim: bool,
    reverse: bool,
    strikethrough: bool,
    hidden: bool,
}

impl ScreenInstance {
    fn new(size: TerminalSize) -> Self {
        let cells = vec![vec![Cell::default(); size.cols as usize]; size.rows as usize];
        Self {
            cells,
            cursor: CursorPos::default(),
            size,
            current_fg: Color::Default,
            current_bg: Color::Default,
            current_bold: false,
            current_underline: false,
            current_italic: false,
            current_dim: false,
            current_reverse: false,
            current_strikethrough: false,
            current_hidden: false,
            scroll_top: 0,
            scroll_bottom: size.rows - 1,
            saved_primary_cells: None,
            saved_primary_cursor: None,
            is_alternate_screen: false,
            saved_cursor: None,
            saved_cursor_dec: None,
            cursor_visible: true,
            autowrap: true,
            application_cursor_keys: false,
            bracketed_paste: false,
            saved_primary_scroll_top: None,
            saved_primary_scroll_bottom: None,
            title: None,
            cwd: None,
            notifications: Vec::new(),
        }
    }

    /// Scroll up within the scroll region: remove the top row of the region
    /// and insert a blank row at the bottom of the region.
    fn scroll_up(&mut self) {
        let top = self.scroll_top as usize;
        let bottom = self.scroll_bottom as usize;
        if top < bottom && bottom < self.cells.len() {
            self.cells.remove(top);
            self.cells
                .insert(bottom, vec![Cell::default(); self.size.cols as usize]);
        }
    }

    /// Scroll down within the scroll region: remove the bottom row of the region
    /// and insert a blank row at the top of the region.
    fn scroll_down(&mut self) {
        let top = self.scroll_top as usize;
        let bottom = self.scroll_bottom as usize;
        if top < bottom && bottom < self.cells.len() {
            self.cells.remove(bottom);
            self.cells
                .insert(top, vec![Cell::default(); self.size.cols as usize]);
        }
    }

    /// Erase a cell at the given position (set to default Cell).
    fn erase_cell(&mut self, row: u16, col: u16) {
        if (row as usize) < self.cells.len()
            && (col as usize) < self.cells[row as usize].len()
        {
            self.cells[row as usize][col as usize] = Cell::default();
        }
    }

    /// Switch to the alternate screen buffer.
    ///
    /// Saves the current (primary) cells and cursor, then clears the buffer
    /// and resets the cursor to the origin.
    fn enter_alternate_screen(&mut self) {
        // Save primary state (only if not already in alternate screen,
        // to prevent overwriting the original primary with alternate content)
        if !self.is_alternate_screen {
            self.saved_primary_cells = Some(std::mem::take(&mut self.cells));
            self.saved_primary_cursor = Some(self.cursor);
            self.saved_primary_scroll_top = Some(self.scroll_top);
            self.saved_primary_scroll_bottom = Some(self.scroll_bottom);
        }

        // Reset to clean state for alternate screen
        self.cells = vec![vec![Cell::default(); self.size.cols as usize]; self.size.rows as usize];
        self.cursor = CursorPos::default();
        self.scroll_top = 0;
        self.scroll_bottom = self.size.rows - 1;

        self.is_alternate_screen = true;
    }

    /// Switch back from the alternate screen buffer to the primary buffer.
    ///
    /// Restores the previously saved primary cells and cursor. If no saved
    /// state exists, this is a no-op (except clearing the flag).
    fn leave_alternate_screen(&mut self) {
        if let Some(cells) = self.saved_primary_cells.take() {
            self.cells = cells;
        }
        if let Some(cursor) = self.saved_primary_cursor.take() {
            self.cursor = cursor;
        }
        if let Some(top) = self.saved_primary_scroll_top.take() {
            self.scroll_top = top;
        }
        if let Some(bottom) = self.saved_primary_scroll_bottom.take() {
            self.scroll_bottom = bottom;
        }
        self.is_alternate_screen = false;
    }
}

impl vte::Perform for ScreenInstance {
    fn print(&mut self, c: char) {
        use unicode_width::UnicodeWidthChar;

        let char_width = c.width().unwrap_or(1);

        let row = self.cursor.row as usize;
        let col = self.cursor.col as usize;
        let cols = self.size.cols as usize;

        if char_width == 2 && col + 1 >= cols {
            // Wide char does not fit at end of line: pad last cell and wrap
            if col < cols && row < self.cells.len() {
                self.cells[row][col] = Cell::default();
            }
            self.cursor.col = 0;
            self.cursor.row += 1;
            if self.cursor.row > self.scroll_bottom {
                self.scroll_up();
                self.cursor.row = self.scroll_bottom;
            }
            // Recalculate position after wrap
            let row = self.cursor.row as usize;
            let col = self.cursor.col as usize;

            if row < self.cells.len() && col + 1 < cols {
                // Left half of wide char
                self.cells[row][col] = Cell {
                    ch: c,
                    fg: self.current_fg,
                    bg: self.current_bg,
                    bold: self.current_bold,
                    underline: self.current_underline,
                    italic: self.current_italic,
                    dim: self.current_dim,
                    reverse: self.current_reverse,
                    strikethrough: self.current_strikethrough,
                    hidden: self.current_hidden,
                    width: 2,
                };
                // Right half placeholder
                self.cells[row][col + 1] = Cell {
                    ch: ' ',
                    width: 0,
                    ..Cell::default()
                };
                self.cursor.col += 2;
            }
        } else if char_width == 2 {
            // Wide char fits within current line
            if row < self.cells.len() && col + 1 < cols {
                self.cells[row][col] = Cell {
                    ch: c,
                    fg: self.current_fg,
                    bg: self.current_bg,
                    bold: self.current_bold,
                    underline: self.current_underline,
                    italic: self.current_italic,
                    dim: self.current_dim,
                    reverse: self.current_reverse,
                    strikethrough: self.current_strikethrough,
                    hidden: self.current_hidden,
                    width: 2,
                };
                self.cells[row][col + 1] = Cell {
                    ch: ' ',
                    width: 0,
                    ..Cell::default()
                };
                self.cursor.col += 2;
            }
            // Autowrap check after advancing
            if self.cursor.col >= self.size.cols {
                self.cursor.col = 0;
                self.cursor.row += 1;
                if self.cursor.row > self.scroll_bottom {
                    self.scroll_up();
                    self.cursor.row = self.scroll_bottom;
                }
            }
        } else {
            // Normal width-1 character (existing logic)
            if row < self.cells.len() && col < self.cells[row].len() {
                self.cells[row][col] = Cell {
                    ch: c,
                    fg: self.current_fg,
                    bg: self.current_bg,
                    bold: self.current_bold,
                    underline: self.current_underline,
                    italic: self.current_italic,
                    dim: self.current_dim,
                    reverse: self.current_reverse,
                    strikethrough: self.current_strikethrough,
                    hidden: self.current_hidden,
                    width: 1,
                };
            }
            self.cursor.col += 1;
            if self.cursor.col >= self.size.cols {
                self.cursor.col = 0;
                self.cursor.row += 1;
                if self.cursor.row > self.scroll_bottom {
                    self.scroll_up();
                    self.cursor.row = self.scroll_bottom;
                }
            }
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // LF (Line Feed)
            0x0A => {
                if self.cursor.row == self.scroll_bottom {
                    // Cursor is at the bottom of the scroll region — scroll up
                    self.scroll_up();
                } else if self.cursor.row < self.size.rows - 1 {
                    // Otherwise just move cursor down
                    self.cursor.row += 1;
                }
            }
            // CR (Carriage Return)
            0x0D => {
                self.cursor.col = 0;
            }
            // BS (Backspace)
            0x08 => {
                if self.cursor.col > 0 {
                    self.cursor.col -= 1;
                }
            }
            // TAB
            0x09 => {
                self.cursor.col = ((self.cursor.col / 8) + 1) * 8;
                if self.cursor.col >= self.size.cols {
                    self.cursor.col = self.size.cols - 1;
                }
            }
            // BEL (Bell notification)
            0x07 => {
                self.notifications.push(NotificationEvent::Bell);
            }
            // Others: ignore
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        // Collect params into a flat Vec<u16> for index-based access.
        // Each param group's first element is the primary value.
        let params_list: Vec<u16> = params.iter().map(|p| p[0]).collect();

        // DEC Private Mode (CSI ? ... h/l)
        if intermediates == [b'?'] {
            match action {
                'h' => {
                    // Set Mode
                    for &p in &params_list {
                        match p {
                            1049 | 1047 | 47 => self.enter_alternate_screen(),
                            25 => self.cursor_visible = true,
                            7 => self.autowrap = true,
                            1 => self.application_cursor_keys = true,
                            2004 => self.bracketed_paste = true,
                            _ => {}
                        }
                    }
                }
                'l' => {
                    // Reset Mode
                    for &p in &params_list {
                        match p {
                            1049 | 1047 | 47 => self.leave_alternate_screen(),
                            25 => self.cursor_visible = false,
                            7 => self.autowrap = false,
                            1 => self.application_cursor_keys = false,
                            2004 => self.bracketed_paste = false,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        // Standard CSI sequences
        match action {
            // SGR (Select Graphic Rendition)
            'm' => {
                if params_list.is_empty() {
                    // Treat as reset
                    self.current_fg = Color::Default;
                    self.current_bg = Color::Default;
                    self.current_bold = false;
                    self.current_underline = false;
                    self.current_italic = false;
                    self.current_dim = false;
                    self.current_reverse = false;
                    self.current_strikethrough = false;
                    self.current_hidden = false;
                    return;
                }

                let mut i = 0;
                while i < params_list.len() {
                    match params_list[i] {
                        0 => {
                            self.current_fg = Color::Default;
                            self.current_bg = Color::Default;
                            self.current_bold = false;
                            self.current_underline = false;
                            self.current_italic = false;
                            self.current_dim = false;
                            self.current_reverse = false;
                            self.current_strikethrough = false;
                            self.current_hidden = false;
                        }
                        1 => self.current_bold = true,
                        2 => self.current_dim = true,
                        3 => self.current_italic = true,
                        4 => self.current_underline = true,
                        7 => self.current_reverse = true,
                        8 => self.current_hidden = true,
                        9 => self.current_strikethrough = true,
                        22 => {
                            self.current_bold = false;
                            self.current_dim = false;
                        }
                        23 => self.current_italic = false,
                        24 => self.current_underline = false,
                        27 => self.current_reverse = false,
                        28 => self.current_hidden = false,
                        29 => self.current_strikethrough = false,
                        // Standard foreground colors (30-37)
                        n @ 30..=37 => self.current_fg = Color::Indexed((n - 30) as u8),
                        // Extended foreground color
                        38 => {
                            if i + 1 < params_list.len() {
                                match params_list[i + 1] {
                                    5 => {
                                        // 256-color: 38;5;n
                                        if i + 2 < params_list.len() {
                                            self.current_fg =
                                                Color::Indexed(params_list[i + 2] as u8);
                                            i += 2;
                                        }
                                    }
                                    2 => {
                                        // RGB: 38;2;r;g;b
                                        if i + 4 < params_list.len() {
                                            self.current_fg = Color::Rgb(
                                                params_list[i + 2] as u8,
                                                params_list[i + 3] as u8,
                                                params_list[i + 4] as u8,
                                            );
                                            i += 4;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        // Default foreground
                        39 => self.current_fg = Color::Default,
                        // Standard background colors (40-47)
                        n @ 40..=47 => self.current_bg = Color::Indexed((n - 40) as u8),
                        // Extended background color
                        48 => {
                            if i + 1 < params_list.len() {
                                match params_list[i + 1] {
                                    5 => {
                                        // 256-color: 48;5;n
                                        if i + 2 < params_list.len() {
                                            self.current_bg =
                                                Color::Indexed(params_list[i + 2] as u8);
                                            i += 2;
                                        }
                                    }
                                    2 => {
                                        // RGB: 48;2;r;g;b
                                        if i + 4 < params_list.len() {
                                            self.current_bg = Color::Rgb(
                                                params_list[i + 2] as u8,
                                                params_list[i + 3] as u8,
                                                params_list[i + 4] as u8,
                                            );
                                            i += 4;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        // Default background
                        49 => self.current_bg = Color::Default,
                        // Bright foreground colors (90-97)
                        n @ 90..=97 => self.current_fg = Color::Indexed((n - 90 + 8) as u8),
                        // Bright background colors (100-107)
                        n @ 100..=107 => self.current_bg = Color::Indexed((n - 100 + 8) as u8),
                        _ => {}
                    }
                    i += 1;
                }
            }

            // CUP (Cursor Position) — 'H' or 'f'
            'H' | 'f' => {
                let row = if !params_list.is_empty() && params_list[0] > 0 {
                    params_list[0] - 1
                } else {
                    0
                };
                let col = if params_list.len() > 1 && params_list[1] > 0 {
                    params_list[1] - 1
                } else {
                    0
                };
                self.cursor.row = row.min(self.size.rows.saturating_sub(1));
                self.cursor.col = col.min(self.size.cols.saturating_sub(1));
            }

            // ED (Erase in Display)
            'J' => {
                let mode = if params_list.is_empty() {
                    0
                } else {
                    params_list[0]
                };
                match mode {
                    // Erase from cursor to end of screen
                    0 => {
                        // Clear rest of current line
                        for c in self.cursor.col..self.size.cols {
                            self.erase_cell(self.cursor.row, c);
                        }
                        // Clear all following lines
                        for r in (self.cursor.row + 1)..self.size.rows {
                            for c in 0..self.size.cols {
                                self.erase_cell(r, c);
                            }
                        }
                    }
                    // Erase from start of screen to cursor
                    1 => {
                        // Clear all preceding lines
                        for r in 0..self.cursor.row {
                            for c in 0..self.size.cols {
                                self.erase_cell(r, c);
                            }
                        }
                        // Clear current line up to and including cursor
                        for c in 0..=self.cursor.col {
                            self.erase_cell(self.cursor.row, c);
                        }
                    }
                    // Erase entire screen
                    2 => {
                        for r in 0..self.size.rows {
                            for c in 0..self.size.cols {
                                self.erase_cell(r, c);
                            }
                        }
                    }
                    _ => {}
                }
            }

            // EL (Erase in Line)
            'K' => {
                let mode = if params_list.is_empty() {
                    0
                } else {
                    params_list[0]
                };
                match mode {
                    // Erase from cursor to end of line
                    0 => {
                        for c in self.cursor.col..self.size.cols {
                            self.erase_cell(self.cursor.row, c);
                        }
                    }
                    // Erase from start of line to cursor
                    1 => {
                        for c in 0..=self.cursor.col {
                            self.erase_cell(self.cursor.row, c);
                        }
                    }
                    // Erase entire line
                    2 => {
                        for c in 0..self.size.cols {
                            self.erase_cell(self.cursor.row, c);
                        }
                    }
                    _ => {}
                }
            }

            // CUU (Cursor Up)
            'A' => {
                let n = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0]
                };
                self.cursor.row = self.cursor.row.saturating_sub(n);
            }

            // CUD (Cursor Down)
            'B' => {
                let n = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0]
                };
                self.cursor.row = (self.cursor.row + n).min(self.size.rows.saturating_sub(1));
            }

            // CUF (Cursor Forward / Right)
            'C' => {
                let n = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0]
                };
                self.cursor.col = (self.cursor.col + n).min(self.size.cols.saturating_sub(1));
            }

            // CUB (Cursor Backward / Left)
            'D' => {
                let n = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0]
                };
                self.cursor.col = self.cursor.col.saturating_sub(n);
            }

            // DECSTBM (Set Top and Bottom Margins)
            'r' => {
                let top = if !params_list.is_empty() && params_list[0] > 0 {
                    params_list[0] - 1 // 1-indexed to 0-indexed
                } else {
                    0
                };
                let bottom = if params_list.len() > 1 && params_list[1] > 0 {
                    (params_list[1] - 1).min(self.size.rows - 1) // 1-indexed to 0-indexed, clamp
                } else {
                    self.size.rows - 1
                };
                if top < bottom {
                    self.scroll_top = top;
                    self.scroll_bottom = bottom;
                }
                // DECSTBM always resets cursor to home position
                self.cursor.row = 0;
                self.cursor.col = 0;
            }

            // IL (Insert Lines)
            'L' => {
                let n = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0] as usize
                };
                let row = self.cursor.row as usize;
                let bottom = self.scroll_bottom as usize;
                // Only operate if cursor is within scroll region
                if row >= self.scroll_top as usize && row <= bottom {
                    for _ in 0..n {
                        if bottom < self.cells.len() {
                            self.cells.remove(bottom);
                            self.cells
                                .insert(row, vec![Cell::default(); self.size.cols as usize]);
                        }
                    }
                }
            }

            // DL (Delete Lines)
            'M' => {
                let n = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0] as usize
                };
                let row = self.cursor.row as usize;
                let bottom = self.scroll_bottom as usize;
                if row >= self.scroll_top as usize && row <= bottom {
                    for _ in 0..n {
                        if row < self.cells.len() && bottom < self.cells.len() {
                            self.cells.remove(row);
                            self.cells
                                .insert(bottom, vec![Cell::default(); self.size.cols as usize]);
                        }
                    }
                }
            }

            // SU (Scroll Up)
            'S' => {
                let n = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0]
                };
                for _ in 0..n {
                    self.scroll_up();
                }
            }

            // SD (Scroll Down)
            'T' => {
                let n = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0]
                };
                for _ in 0..n {
                    self.scroll_down();
                }
            }

            // ICH (Insert Characters)
            '@' => {
                let n = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0] as usize
                };
                let row = self.cursor.row as usize;
                let col = self.cursor.col as usize;
                let cols = self.size.cols as usize;
                if row < self.cells.len() {
                    for _ in 0..n {
                        if col < cols {
                            self.cells[row].pop(); // remove last character
                            self.cells[row].insert(col, Cell::default()); // insert blank at cursor
                        }
                    }
                }
            }

            // DCH (Delete Characters)
            'P' => {
                let n = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0] as usize
                };
                let row = self.cursor.row as usize;
                let col = self.cursor.col as usize;
                if row < self.cells.len() {
                    for _ in 0..n {
                        if col < self.cells[row].len() {
                            self.cells[row].remove(col); // remove char at cursor
                            self.cells[row].push(Cell::default()); // append blank at end
                        }
                    }
                }
            }

            // ECH (Erase Characters)
            'X' => {
                let n = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0] as usize
                };
                let row = self.cursor.row as usize;
                let col = self.cursor.col as usize;
                if row < self.cells.len() {
                    for i in 0..n {
                        let c = col + i;
                        if c < self.cells[row].len() {
                            self.cells[row][c] = Cell::default();
                        }
                    }
                }
                // Cursor does not move
            }

            // CHA (Cursor Horizontal Absolute)
            'G' => {
                let col = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0]
                };
                self.cursor.col = (col - 1).min(self.size.cols.saturating_sub(1));
            }

            // VPA (Cursor Vertical Absolute)
            'd' => {
                let row = if params_list.is_empty() || params_list[0] == 0 {
                    1
                } else {
                    params_list[0]
                };
                self.cursor.row = (row - 1).min(self.size.rows.saturating_sub(1));
            }

            // SCP (Save Cursor Position)
            's' => {
                self.saved_cursor = Some(self.cursor);
            }

            // RCP (Restore Cursor Position)
            'u' => {
                if let Some(saved) = self.saved_cursor {
                    self.cursor = saved;
                }
            }

            // Unknown CSI: ignore
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        // Only handle VT100-compatible ESC sequences (no intermediates)
        if !intermediates.is_empty() {
            return;
        }
        match byte {
            // ESC 7 (DECSC) -- Save Cursor Position + SGR state
            b'7' => {
                self.saved_cursor_dec = Some(SavedCursorState {
                    cursor: self.cursor,
                    fg: self.current_fg,
                    bg: self.current_bg,
                    bold: self.current_bold,
                    underline: self.current_underline,
                    italic: self.current_italic,
                    dim: self.current_dim,
                    reverse: self.current_reverse,
                    strikethrough: self.current_strikethrough,
                    hidden: self.current_hidden,
                });
            }
            // ESC 8 (DECRC) -- Restore Cursor Position + SGR state
            b'8' => {
                if let Some(saved) = &self.saved_cursor_dec {
                    self.cursor = saved.cursor;
                    self.current_fg = saved.fg;
                    self.current_bg = saved.bg;
                    self.current_bold = saved.bold;
                    self.current_underline = saved.underline;
                    self.current_italic = saved.italic;
                    self.current_dim = saved.dim;
                    self.current_reverse = saved.reverse;
                    self.current_strikethrough = saved.strikethrough;
                    self.current_hidden = saved.hidden;
                }
            }
            // ESC M (Reverse Index / RI) -- cursor up 1, if at scroll_top then scroll down
            b'M' => {
                if self.cursor.row == self.scroll_top {
                    self.scroll_down();
                } else if self.cursor.row > 0 {
                    self.cursor.row -= 1;
                }
            }
            // ESC D (Index / IND) -- cursor down 1, if at scroll_bottom then scroll up
            b'D' => {
                if self.cursor.row == self.scroll_bottom {
                    self.scroll_up();
                } else if self.cursor.row < self.size.rows - 1 {
                    self.cursor.row += 1;
                }
            }
            // ESC E (Next Line / NEL) -- cursor to next line start
            b'E' => {
                self.cursor.col = 0;
                if self.cursor.row == self.scroll_bottom {
                    self.scroll_up();
                } else if self.cursor.row < self.size.rows - 1 {
                    self.cursor.row += 1;
                }
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() {
            return;
        }
        // OSC 0 (Set Icon Name and Window Title) or OSC 2 (Set Window Title)
        match params[0] {
            b"0" | b"2" => {
                if params.len() > 1 {
                    self.title = Some(String::from_utf8_lossy(params[1]).to_string());
                }
            }
            // OSC 7 (Set Current Working Directory)
            b"7" => {
                if params.len() > 1 {
                    let uri = String::from_utf8_lossy(params[1]);
                    if let Some(path) = parse_osc7_uri(&uri) {
                        self.cwd = Some(path);
                    }
                }
            }
            // OSC 9 (iTerm2-compatible notification)
            b"9" => {
                if params.len() > 1 {
                    let message = String::from_utf8_lossy(params[1]).to_string();
                    self.notifications.push(NotificationEvent::Osc9 { message });
                }
            }
            // OSC 777 (rxvt-compatible notification)
            b"777" => {
                if params.len() >= 4 && params[1] == b"notify" {
                    let title = String::from_utf8_lossy(params[2]).to_string();
                    let body = String::from_utf8_lossy(params[3]).to_string();
                    self.notifications
                        .push(NotificationEvent::Osc777 { title, body });
                }
            }
            // Other OSC sequences: ignore silently
            _ => {}
        }
    }
}

/// Concrete implementation of `ScreenPort` using the `vte` crate for ANSI parsing.
///
/// Manages multiple screen instances indexed by `TerminalId`.
pub struct VteScreenAdapter {
    screens: HashMap<TerminalId, ScreenInstance>,
    /// Parser stored separately to avoid borrow conflicts during `process()`.
    /// The parser itself is stateless between calls per-screen, but we store one
    /// per screen to correctly handle partial sequences across `process()` calls.
    parsers: HashMap<TerminalId, vte::Parser>,
}

impl VteScreenAdapter {
    pub fn new() -> Self {
        Self {
            screens: HashMap::new(),
            parsers: HashMap::new(),
        }
    }

    /// Get the title of the screen (set by OSC 0/2).
    /// This is for testing and future use (e.g., showing tab titles).
    pub fn get_title(&self, id: TerminalId) -> Result<Option<String>, AppError> {
        self.screens
            .get(&id)
            .map(|s| s.title.clone())
            .ok_or(AppError::ScreenNotFound(id))
    }
}

impl ScreenPort for VteScreenAdapter {
    fn create(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError> {
        let screen = ScreenInstance::new(size);
        self.screens.insert(id, screen);
        self.parsers.insert(id, vte::Parser::new());
        Ok(())
    }

    fn process(&mut self, id: TerminalId, data: &[u8]) -> Result<(), AppError> {
        let screen = self
            .screens
            .get_mut(&id)
            .ok_or(AppError::ScreenNotFound(id))?;
        let parser = self
            .parsers
            .get_mut(&id)
            .ok_or(AppError::ScreenNotFound(id))?;

        parser.advance(screen, data);
        Ok(())
    }

    fn get_cells(&self, id: TerminalId) -> Result<&Vec<Vec<Cell>>, AppError> {
        self.screens
            .get(&id)
            .map(|s| &s.cells)
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn get_cursor(&self, id: TerminalId) -> Result<CursorPos, AppError> {
        self.screens
            .get(&id)
            .map(|s| s.cursor)
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn resize(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError> {
        let screen = self
            .screens
            .get_mut(&id)
            .ok_or(AppError::ScreenNotFound(id))?;

        let new_rows = size.rows as usize;
        let new_cols = size.cols as usize;

        // Adjust rows
        if new_rows > screen.cells.len() {
            // Add blank rows
            for _ in screen.cells.len()..new_rows {
                screen.cells.push(vec![Cell::default(); new_cols]);
            }
        } else {
            screen.cells.truncate(new_rows);
        }

        // Adjust columns for all rows
        for row in &mut screen.cells {
            if new_cols > row.len() {
                row.resize(new_cols, Cell::default());
            } else {
                row.truncate(new_cols);
            }
        }

        screen.size = size;

        // Reset scroll region to full screen
        screen.scroll_top = 0;
        screen.scroll_bottom = size.rows - 1;

        // Clamp cursor position
        if screen.cursor.row >= size.rows {
            screen.cursor.row = size.rows.saturating_sub(1);
        }
        if screen.cursor.col >= size.cols {
            screen.cursor.col = size.cols.saturating_sub(1);
        }

        Ok(())
    }

    fn remove(&mut self, id: TerminalId) -> Result<(), AppError> {
        self.screens
            .remove(&id)
            .ok_or(AppError::ScreenNotFound(id))?;
        self.parsers.remove(&id);
        Ok(())
    }

    fn get_cursor_visible(&self, id: TerminalId) -> Result<bool, AppError> {
        self.screens
            .get(&id)
            .map(|s| s.cursor_visible)
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn get_application_cursor_keys(&self, id: TerminalId) -> Result<bool, AppError> {
        self.screens
            .get(&id)
            .map(|s| s.application_cursor_keys)
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn get_bracketed_paste(&self, id: TerminalId) -> Result<bool, AppError> {
        self.screens
            .get(&id)
            .map(|s| s.bracketed_paste)
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn get_cwd(&self, id: TerminalId) -> Result<Option<String>, AppError> {
        self.screens
            .get(&id)
            .map(|inst| inst.cwd.clone())
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn drain_notifications(&mut self, id: TerminalId) -> Result<Vec<NotificationEvent>, AppError> {
        let screen = self
            .screens
            .get_mut(&id)
            .ok_or(AppError::ScreenNotFound(id))?;
        Ok(std::mem::take(&mut screen.notifications))
    }

    fn set_scrollback_offset(&mut self, _id: TerminalId, _offset: usize) -> Result<(), AppError> {
        // VteScreenAdapter does not support scrollback
        Ok(())
    }

    fn get_scrollback_offset(&self, _id: TerminalId) -> Result<usize, AppError> {
        Ok(0)
    }

    fn get_max_scrollback(&self, _id: TerminalId) -> Result<usize, AppError> {
        Ok(0)
    }

    fn is_alternate_screen(&self, id: TerminalId) -> Result<bool, AppError> {
        self.screens
            .get(&id)
            .map(|s| s.is_alternate_screen)
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn get_cursor_style(&self, _id: TerminalId) -> Result<CursorStyle, AppError> {
        // VteScreenAdapter does not track cursor style
        Ok(CursorStyle::DefaultUserShape)
    }

    fn drain_pending_responses(&mut self, _id: TerminalId) -> Result<Vec<Vec<u8>>, AppError> {
        // VteScreenAdapter does not handle DSR queries
        Ok(vec![])
    }

    fn search_scrollback(&mut self, _id: TerminalId, _query: &str) -> Result<Vec<SearchMatch>, AppError> {
        // VteScreenAdapter does not support scrollback search
        Ok(vec![])
    }

    fn get_row_cells(&mut self, _id: TerminalId, _abs_row: usize) -> Result<Vec<Cell>, AppError> {
        // VteScreenAdapter does not support scrollback row access
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_size() -> TerminalSize {
        TerminalSize::new(80, 24)
    }

    fn small_size() -> TerminalSize {
        TerminalSize::new(10, 5)
    }

    fn id(n: u32) -> TerminalId {
        TerminalId::new(n)
    }

    /// Helper to collect characters from a row as a String.
    fn row_text(cells: &Vec<Vec<Cell>>, row: usize) -> String {
        cells[row].iter().map(|c| c.ch).collect::<String>()
    }

    // =========================================================================
    // Tests: create()
    // =========================================================================

    #[test]
    fn create_initializes_correct_size_grid() {
        let mut adapter = VteScreenAdapter::new();
        let size = TerminalSize::new(80, 24);

        adapter.create(id(1), size).unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells.len(), 24);
        assert_eq!(cells[0].len(), 80);
    }

    #[test]
    fn create_initializes_all_cells_to_default() {
        let mut adapter = VteScreenAdapter::new();
        let size = small_size();

        adapter.create(id(1), size).unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        for row in cells {
            for cell in row {
                assert_eq!(cell.ch, ' ');
                assert_eq!(cell.fg, Color::Default);
                assert_eq!(cell.bg, Color::Default);
                assert!(!cell.bold);
                assert!(!cell.underline);
                assert!(!cell.italic);
                assert!(!cell.dim);
                assert!(!cell.reverse);
                assert!(!cell.strikethrough);
                assert!(!cell.hidden);
            }
        }
    }

    #[test]
    fn create_initializes_cursor_at_origin() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 0);
    }

    #[test]
    fn create_multiple_screens() {
        let mut adapter = VteScreenAdapter::new();
        let size = small_size();

        adapter.create(id(1), size).unwrap();
        adapter.create(id(2), size).unwrap();

        assert!(adapter.get_cells(id(1)).is_ok());
        assert!(adapter.get_cells(id(2)).is_ok());
    }

    // =========================================================================
    // Tests: process() — simple text printing
    // =========================================================================

    #[test]
    fn process_prints_characters_at_correct_positions() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"Hello").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'H');
        assert_eq!(cells[0][1].ch, 'e');
        assert_eq!(cells[0][2].ch, 'l');
        assert_eq!(cells[0][3].ch, 'l');
        assert_eq!(cells[0][4].ch, 'o');
    }

    #[test]
    fn process_advances_cursor_after_print() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"AB").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 2);
    }

    #[test]
    fn process_on_nonexistent_screen_returns_error() {
        let mut adapter = VteScreenAdapter::new();

        let result = adapter.process(id(99), b"Hello");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ScreenNotFound(_)));
    }

    // =========================================================================
    // Tests: process() — line feed and carriage return
    // =========================================================================

    #[test]
    fn process_linefeed_moves_cursor_down() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"A\nB").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 1);
        assert_eq!(cursor.col, 2);

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[1][1].ch, 'B');
    }

    #[test]
    fn process_carriage_return_moves_cursor_to_col_zero() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"Hello\rW").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // 'W' overwrites 'H' at col 0
        assert_eq!(cells[0][0].ch, 'W');
        assert_eq!(cells[0][1].ch, 'e');

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 1);
    }

    #[test]
    fn process_crlf_moves_to_start_of_next_line() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"AB\r\nCD").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[0][1].ch, 'B');
        assert_eq!(cells[1][0].ch, 'C');
        assert_eq!(cells[1][1].ch, 'D');

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 1);
        assert_eq!(cursor.col, 2);
    }

    // =========================================================================
    // Tests: process() — backspace
    // =========================================================================

    #[test]
    fn process_backspace_moves_cursor_left() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Write "AB", backspace, write "C" -> "AC"
        adapter.process(id(1), b"AB\x08C").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[0][1].ch, 'C');

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.col, 2);
    }

    #[test]
    fn process_backspace_at_col_zero_stays_at_zero() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"\x08").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.col, 0);
    }

    // =========================================================================
    // Tests: process() — tab
    // =========================================================================

    #[test]
    fn process_tab_advances_to_next_tab_stop() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(40, 5)).unwrap();

        adapter.process(id(1), b"AB\t").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        // cursor was at col 2, next tab stop = ((2/8)+1)*8 = 8
        assert_eq!(cursor.col, 8);
    }

    #[test]
    fn process_tab_clamps_to_last_col() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap(); // 10 cols

        // Move cursor to col 9 by printing 9 chars, then tab
        adapter.process(id(1), b"123456789\t").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        // Cursor was at col 9, next tab = ((9/8)+1)*8 = 16, clamped to 9 (cols-1)
        assert_eq!(cursor.col, 9);
    }

    // =========================================================================
    // Tests: process() — auto-wrap at end of line
    // =========================================================================

    #[test]
    fn process_auto_wraps_at_end_of_line() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        adapter.process(id(1), b"ABCDEF").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // First row: ABCDE
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[0][4].ch, 'E');
        // Second row: F
        assert_eq!(cells[1][0].ch, 'F');

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 1);
        assert_eq!(cursor.col, 1);
    }

    // =========================================================================
    // Tests: process() — scroll when reaching bottom
    // =========================================================================

    #[test]
    fn process_scrolls_when_cursor_exceeds_bottom() {
        let mut adapter = VteScreenAdapter::new();
        // 5 cols, 3 rows
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        // Fill all 3 rows and overflow: 5 + 5 + 5 = 15 chars fills all, +1 triggers scroll
        adapter.process(id(1), b"AAAAABBBBBCCCCCX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // After scroll, first row should be former row 1 (BBBBB)
        assert_eq!(row_text(cells, 0).trim_end(), "BBBBB");
        assert_eq!(row_text(cells, 1).trim_end(), "CCCCC");
        // Row 2 should have 'X' at col 0 and spaces after
        assert_eq!(cells[2][0].ch, 'X');

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 2);
        assert_eq!(cursor.col, 1);
    }

    #[test]
    fn process_scroll_via_linefeed_at_bottom() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(10, 3)).unwrap();

        // Use CR+LF to start each line at col 0
        adapter.process(id(1), b"ROW0\r\n").unwrap(); // row 0: "ROW0", cursor now at (1,0)
        adapter.process(id(1), b"ROW1\r\n").unwrap(); // row 1: "ROW1", cursor now at (2,0)
        adapter.process(id(1), b"ROW2\r\n").unwrap(); // row 2: "ROW2", LF scrolls up, cursor at (2,0)

        let cells = adapter.get_cells(id(1)).unwrap();
        // After scroll: original row 0 gone, row 1->0, row 2->1, blank->2
        assert_eq!(cells[0][0].ch, 'R');
        assert_eq!(cells[0][1].ch, 'O');
        assert_eq!(cells[0][2].ch, 'W');
        assert_eq!(cells[0][3].ch, '1');
        assert_eq!(cells[1][0].ch, 'R');
        assert_eq!(cells[1][1].ch, 'O');
        assert_eq!(cells[1][2].ch, 'W');
        assert_eq!(cells[1][3].ch, '2');
        // Row 2 should be blank
        assert_eq!(cells[2][0].ch, ' ');

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 2);
        assert_eq!(cursor.col, 0);
    }

    // =========================================================================
    // Tests: process() — CSI cursor movement
    // =========================================================================

    #[test]
    fn process_csi_cursor_position() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // ESC[5;10H — move cursor to row 5, col 10 (1-indexed)
        adapter.process(id(1), b"\x1b[5;10H").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 4); // 0-indexed
        assert_eq!(cursor.col, 9); // 0-indexed
    }

    #[test]
    fn process_csi_cursor_position_defaults() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // Move somewhere first
        adapter.process(id(1), b"\x1b[10;20H").unwrap();
        // ESC[H — move to home (1,1)
        adapter.process(id(1), b"\x1b[H").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 0);
    }

    #[test]
    fn process_csi_cursor_position_clamps_to_screen_bounds() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap(); // 10x5

        // ESC[100;200H — should clamp to (4, 9)
        adapter.process(id(1), b"\x1b[100;200H").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 4); // rows - 1
        assert_eq!(cursor.col, 9); // cols - 1
    }

    #[test]
    fn process_csi_cursor_up() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        adapter.process(id(1), b"\x1b[10;1H").unwrap(); // row 9
        adapter.process(id(1), b"\x1b[3A").unwrap(); // up 3

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 6);
    }

    #[test]
    fn process_csi_cursor_up_clamps_at_zero() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        adapter.process(id(1), b"\x1b[3;1H").unwrap(); // row 2
        adapter.process(id(1), b"\x1b[100A").unwrap(); // up 100

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
    }

    #[test]
    fn process_csi_cursor_down() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        adapter.process(id(1), b"\x1b[5B").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 5);
    }

    #[test]
    fn process_csi_cursor_down_clamps_at_bottom() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap(); // 5 rows

        adapter.process(id(1), b"\x1b[100B").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 4); // rows - 1
    }

    #[test]
    fn process_csi_cursor_forward() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        adapter.process(id(1), b"\x1b[10C").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.col, 10);
    }

    #[test]
    fn process_csi_cursor_backward() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        adapter.process(id(1), b"\x1b[1;20H").unwrap(); // col 19
        adapter.process(id(1), b"\x1b[5D").unwrap(); // left 5

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.col, 14);
    }

    #[test]
    fn process_csi_cursor_backward_clamps_at_zero() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        adapter.process(id(1), b"\x1b[1;5H").unwrap(); // col 4
        adapter.process(id(1), b"\x1b[100D").unwrap(); // left 100

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.col, 0);
    }

    #[test]
    fn process_csi_cursor_movement_default_param_is_one() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        adapter.process(id(1), b"\x1b[5;5H").unwrap(); // row=4, col=4
        adapter.process(id(1), b"\x1b[A").unwrap(); // up 1 (default)
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 3);

        adapter.process(id(1), b"\x1b[B").unwrap(); // down 1
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 4);

        adapter.process(id(1), b"\x1b[C").unwrap(); // right 1
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.col, 5);

        adapter.process(id(1), b"\x1b[D").unwrap(); // left 1
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.col, 4);
    }

    // =========================================================================
    // Tests: process() — CSI SGR (colors and attributes)
    // =========================================================================

    #[test]
    fn process_sgr_bold_and_underline() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[1;4m — bold + underline, then print
        adapter.process(id(1), b"\x1b[1;4mA").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].bold);
        assert!(cells[0][0].underline);
        assert_eq!(cells[0][0].ch, 'A');
    }

    #[test]
    fn process_sgr_reset() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set bold, then reset, then print
        adapter.process(id(1), b"\x1b[1mA\x1b[0mB").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].bold); // 'A' is bold
        assert!(!cells[0][1].bold); // 'B' is not bold
    }

    #[test]
    fn process_sgr_empty_params_resets() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set bold, then ESC[m (no params = reset), then print
        adapter.process(id(1), b"\x1b[1mA\x1b[mB").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].bold); // 'A' is bold
        assert!(!cells[0][1].bold); // 'B' is not bold
    }

    #[test]
    fn process_sgr_standard_foreground_colors() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[31m — red foreground
        adapter.process(id(1), b"\x1b[31mR").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].fg, Color::Indexed(1)); // red = 31 - 30 = 1
    }

    #[test]
    fn process_sgr_standard_background_colors() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[42m — green background
        adapter.process(id(1), b"\x1b[42mG").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].bg, Color::Indexed(2)); // green = 42 - 40 = 2
    }

    #[test]
    fn process_sgr_256_color_foreground() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[38;5;196m — 256-color fg = 196
        adapter.process(id(1), b"\x1b[38;5;196mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].fg, Color::Indexed(196));
    }

    #[test]
    fn process_sgr_256_color_background() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[48;5;82m — 256-color bg = 82
        adapter.process(id(1), b"\x1b[48;5;82mY").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].bg, Color::Indexed(82));
    }

    #[test]
    fn process_sgr_rgb_foreground() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[38;2;255;128;0m — RGB fg
        adapter
            .process(id(1), b"\x1b[38;2;255;128;0mR")
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].fg, Color::Rgb(255, 128, 0));
    }

    #[test]
    fn process_sgr_rgb_background() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[48;2;10;20;30m — RGB bg
        adapter
            .process(id(1), b"\x1b[48;2;10;20;30mB")
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].bg, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn process_sgr_bright_foreground_colors() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[91m — bright red foreground
        adapter.process(id(1), b"\x1b[91mR").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].fg, Color::Indexed(9)); // 91 - 90 + 8 = 9
    }

    #[test]
    fn process_sgr_bright_background_colors() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[102m — bright green background
        adapter.process(id(1), b"\x1b[102mG").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].bg, Color::Indexed(10)); // 102 - 100 + 8 = 10
    }

    #[test]
    fn process_sgr_default_foreground() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set fg to red, then reset fg to default
        adapter.process(id(1), b"\x1b[31mR\x1b[39mD").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].fg, Color::Indexed(1)); // red
        assert_eq!(cells[0][1].fg, Color::Default); // default
    }

    #[test]
    fn process_sgr_default_background() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set bg to green, then reset bg to default
        adapter.process(id(1), b"\x1b[42mG\x1b[49mD").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].bg, Color::Indexed(2)); // green
        assert_eq!(cells[0][1].bg, Color::Default); // default
    }

    #[test]
    fn process_sgr_disable_bold_and_underline() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Bold+underline on, print A, then disable them, print B
        adapter
            .process(id(1), b"\x1b[1;4mA\x1b[22;24mB")
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].bold);
        assert!(cells[0][0].underline);
        assert!(!cells[0][1].bold);
        assert!(!cells[0][1].underline);
    }

    // =========================================================================
    // Tests: process() — CSI erase operations
    // =========================================================================

    #[test]
    fn process_erase_display_from_cursor() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        // Fill screen with 'X'
        adapter.process(id(1), b"XXXXX").unwrap();
        adapter.process(id(1), b"\x1b[2;1H").unwrap(); // row 1, col 0
        adapter.process(id(1), b"XXXXX").unwrap();
        adapter.process(id(1), b"\x1b[3;1H").unwrap(); // row 2, col 0
        adapter.process(id(1), b"XXXXX").unwrap();

        // Move to row 1, col 2 and erase from cursor to end
        adapter.process(id(1), b"\x1b[2;3H").unwrap();
        adapter.process(id(1), b"\x1b[0J").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0 should be untouched
        assert_eq!(cells[0][0].ch, 'X');
        assert_eq!(cells[0][4].ch, 'X');
        // Row 1: col 0,1 untouched, col 2,3,4 erased
        assert_eq!(cells[1][0].ch, 'X');
        assert_eq!(cells[1][1].ch, 'X');
        assert_eq!(cells[1][2].ch, ' ');
        assert_eq!(cells[1][3].ch, ' ');
        assert_eq!(cells[1][4].ch, ' ');
        // Row 2 should be fully erased
        assert_eq!(cells[2][0].ch, ' ');
    }

    #[test]
    fn process_erase_display_to_cursor() {
        let mut adapter = VteScreenAdapter::new();
        // Use 6-col screen so printing 5 X's per row doesn't trigger wrap
        adapter.create(id(1), TerminalSize::new(6, 3)).unwrap();

        // Fill 5 cells per row (leaving last col blank to avoid wrap on last row)
        adapter.process(id(1), b"\x1b[1;1HXXXXX").unwrap();
        adapter.process(id(1), b"\x1b[2;1HXXXXX").unwrap();
        adapter.process(id(1), b"\x1b[3;1HXXXXX").unwrap();

        // Verify filled
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[2][4].ch, 'X');

        // Move to row 1, col 2 and erase from start to cursor
        adapter.process(id(1), b"\x1b[2;3H").unwrap();
        adapter.process(id(1), b"\x1b[1J").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0 should be fully erased
        assert_eq!(cells[0][0].ch, ' ');
        assert_eq!(cells[0][4].ch, ' ');
        // Row 1: col 0,1,2 erased, col 3,4 untouched
        assert_eq!(cells[1][0].ch, ' ');
        assert_eq!(cells[1][1].ch, ' ');
        assert_eq!(cells[1][2].ch, ' ');
        assert_eq!(cells[1][3].ch, 'X');
        assert_eq!(cells[1][4].ch, 'X');
        // Row 2 should be untouched
        assert_eq!(cells[2][0].ch, 'X');
    }

    #[test]
    fn process_erase_entire_display() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        // Fill and erase all
        adapter.process(id(1), b"XXXXX").unwrap();
        adapter.process(id(1), b"\x1b[2;1HXXXXX").unwrap();
        adapter.process(id(1), b"\x1b[3;1HXXXXX").unwrap();
        adapter.process(id(1), b"\x1b[2J").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        for r in 0..3 {
            for c in 0..5 {
                assert_eq!(cells[r][c].ch, ' ', "cell [{r}][{c}] should be blank");
            }
        }
    }

    #[test]
    fn process_erase_line_from_cursor() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        adapter.process(id(1), b"XXXXX").unwrap();
        adapter.process(id(1), b"\x1b[1;3H").unwrap(); // col 2
        adapter.process(id(1), b"\x1b[0K").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'X');
        assert_eq!(cells[0][1].ch, 'X');
        assert_eq!(cells[0][2].ch, ' ');
        assert_eq!(cells[0][3].ch, ' ');
        assert_eq!(cells[0][4].ch, ' ');
    }

    #[test]
    fn process_erase_line_to_cursor() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        adapter.process(id(1), b"XXXXX").unwrap();
        adapter.process(id(1), b"\x1b[1;3H").unwrap(); // col 2
        adapter.process(id(1), b"\x1b[1K").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, ' ');
        assert_eq!(cells[0][1].ch, ' ');
        assert_eq!(cells[0][2].ch, ' ');
        assert_eq!(cells[0][3].ch, 'X');
        assert_eq!(cells[0][4].ch, 'X');
    }

    #[test]
    fn process_erase_entire_line() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        adapter.process(id(1), b"XXXXX").unwrap();
        adapter.process(id(1), b"\x1b[1;3H").unwrap(); // col 2
        adapter.process(id(1), b"\x1b[2K").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        for c in 0..5 {
            assert_eq!(cells[0][c].ch, ' ');
        }
    }

    // =========================================================================
    // Tests: resize()
    // =========================================================================

    #[test]
    fn resize_grows_rows_and_cols() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        adapter.process(id(1), b"AB").unwrap();

        adapter
            .resize(id(1), TerminalSize::new(10, 5))
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells.len(), 5);
        assert_eq!(cells[0].len(), 10);
        // Existing content preserved
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[0][1].ch, 'B');
        // New cells are default
        assert_eq!(cells[0][5].ch, ' ');
        assert_eq!(cells[3][0].ch, ' ');
    }

    #[test]
    fn resize_shrinks_rows_and_cols() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(10, 5)).unwrap();

        adapter.process(id(1), b"ABCDEFGHIJ").unwrap();

        adapter
            .resize(id(1), TerminalSize::new(3, 2))
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].len(), 3);
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[0][1].ch, 'B');
        assert_eq!(cells[0][2].ch, 'C');
    }

    #[test]
    fn resize_clamps_cursor() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(10, 10)).unwrap();

        // Move cursor to row 8, col 8
        adapter.process(id(1), b"\x1b[9;9H").unwrap();
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 8);
        assert_eq!(cursor.col, 8);

        // Shrink to 5x5
        adapter.resize(id(1), TerminalSize::new(5, 5)).unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 4); // clamped to rows-1
        assert_eq!(cursor.col, 4); // clamped to cols-1
    }

    #[test]
    fn resize_nonexistent_screen_returns_error() {
        let mut adapter = VteScreenAdapter::new();

        let result = adapter.resize(id(99), TerminalSize::new(10, 10));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ScreenNotFound(_)));
    }

    // =========================================================================
    // Tests: remove()
    // =========================================================================

    #[test]
    fn remove_removes_screen() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.remove(id(1)).unwrap();

        let result = adapter.get_cells(id(1));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ScreenNotFound(_)));
    }

    #[test]
    fn remove_nonexistent_screen_returns_error() {
        let mut adapter = VteScreenAdapter::new();

        let result = adapter.remove(id(99));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ScreenNotFound(_)));
    }

    #[test]
    fn remove_does_not_affect_other_screens() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();
        adapter.create(id(2), small_size()).unwrap();

        adapter.remove(id(1)).unwrap();

        assert!(adapter.get_cells(id(1)).is_err());
        assert!(adapter.get_cells(id(2)).is_ok());
    }

    // =========================================================================
    // Tests: get_cells() and get_cursor() error paths
    // =========================================================================

    #[test]
    fn get_cells_nonexistent_returns_error() {
        let adapter = VteScreenAdapter::new();

        let result = adapter.get_cells(id(42));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ScreenNotFound(_)));
    }

    #[test]
    fn get_cursor_nonexistent_returns_error() {
        let adapter = VteScreenAdapter::new();

        let result = adapter.get_cursor(id(42));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ScreenNotFound(_)));
    }

    // =========================================================================
    // Tests: process() — multiple process calls (incremental)
    // =========================================================================

    #[test]
    fn process_incremental_calls_preserve_state() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"AB").unwrap();
        adapter.process(id(1), b"CD").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[0][1].ch, 'B');
        assert_eq!(cells[0][2].ch, 'C');
        assert_eq!(cells[0][3].ch, 'D');

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.col, 4);
    }

    #[test]
    fn process_sgr_state_persists_across_calls() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set bold in first call
        adapter.process(id(1), b"\x1b[1m").unwrap();
        // Print in second call — should still be bold
        adapter.process(id(1), b"A").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].bold);
    }

    // =========================================================================
    // Tests: process() — CUP with 'f' (alternate cursor position)
    // =========================================================================

    #[test]
    fn process_csi_cursor_position_with_f() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // ESC[3;7f — move cursor to row 3, col 7 (1-indexed)
        adapter.process(id(1), b"\x1b[3;7f").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 2);
        assert_eq!(cursor.col, 6);
    }

    // =========================================================================
    // Tests: process() — combined sequences (integration-style)
    // =========================================================================

    #[test]
    fn process_combined_text_and_control_sequences() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(20, 5)).unwrap();

        // Print "Hello", move to next line, set red color, print "World"
        adapter
            .process(id(1), b"Hello\r\n\x1b[31mWorld")
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();

        // First row
        assert_eq!(cells[0][0].ch, 'H');
        assert_eq!(cells[0][0].fg, Color::Default);

        // Second row — red text
        assert_eq!(cells[1][0].ch, 'W');
        assert_eq!(cells[1][0].fg, Color::Indexed(1));
        assert_eq!(cells[1][4].ch, 'd');
        assert_eq!(cells[1][4].fg, Color::Indexed(1));
    }

    #[test]
    fn process_erase_default_param() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        adapter.process(id(1), b"XXXXX").unwrap();
        adapter.process(id(1), b"\x1b[1;3H").unwrap(); // col 2

        // ESC[J (no params) = erase from cursor to end (same as ESC[0J)
        adapter.process(id(1), b"\x1b[J").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'X');
        assert_eq!(cells[0][1].ch, 'X');
        assert_eq!(cells[0][2].ch, ' ');
    }

    #[test]
    fn process_erase_line_default_param() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        adapter.process(id(1), b"XXXXX").unwrap();
        adapter.process(id(1), b"\x1b[1;3H").unwrap(); // col 2

        // ESC[K (no params) = erase from cursor to end of line
        adapter.process(id(1), b"\x1b[K").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'X');
        assert_eq!(cells[0][1].ch, 'X');
        assert_eq!(cells[0][2].ch, ' ');
        assert_eq!(cells[0][3].ch, ' ');
        assert_eq!(cells[0][4].ch, ' ');
    }

    // =========================================================================
    // Tests: process() — SGR new attributes (italic, dim, reverse, strikethrough, hidden)
    // =========================================================================

    #[test]
    fn process_sgr_italic() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[3m — italic
        adapter.process(id(1), b"\x1b[3mA").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].italic);
        assert_eq!(cells[0][0].ch, 'A');
    }

    #[test]
    fn process_sgr_dim() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[2m — dim
        adapter.process(id(1), b"\x1b[2mA").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].dim);
        assert_eq!(cells[0][0].ch, 'A');
    }

    #[test]
    fn process_sgr_reverse() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[7m — reverse
        adapter.process(id(1), b"\x1b[7mA").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].reverse);
        assert_eq!(cells[0][0].ch, 'A');
    }

    #[test]
    fn process_sgr_hidden() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[8m — hidden
        adapter.process(id(1), b"\x1b[8mA").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].hidden);
        assert_eq!(cells[0][0].ch, 'A');
    }

    #[test]
    fn process_sgr_strikethrough() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[9m — strikethrough
        adapter.process(id(1), b"\x1b[9mA").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].strikethrough);
        assert_eq!(cells[0][0].ch, 'A');
    }

    #[test]
    fn process_sgr_not_italic() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set italic, print A, then disable italic, print B
        adapter.process(id(1), b"\x1b[3mA\x1b[23mB").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].italic);
        assert!(!cells[0][1].italic);
    }

    #[test]
    fn process_sgr_22_disables_bold_and_dim() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set bold + dim, print A, then SGR 22 (normal intensity), print B
        adapter
            .process(id(1), b"\x1b[1;2mA\x1b[22mB")
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].bold);
        assert!(cells[0][0].dim);
        assert!(!cells[0][1].bold);
        assert!(!cells[0][1].dim);
    }

    #[test]
    fn process_sgr_not_reverse() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set reverse, print A, then disable reverse (SGR 27), print B
        adapter.process(id(1), b"\x1b[7mA\x1b[27mB").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].reverse);
        assert!(!cells[0][1].reverse);
    }

    #[test]
    fn process_sgr_not_hidden() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set hidden, print A, then disable hidden (SGR 28), print B
        adapter.process(id(1), b"\x1b[8mA\x1b[28mB").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].hidden);
        assert!(!cells[0][1].hidden);
    }

    #[test]
    fn process_sgr_not_strikethrough() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set strikethrough, print A, then disable strikethrough (SGR 29), print B
        adapter.process(id(1), b"\x1b[9mA\x1b[29mB").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].strikethrough);
        assert!(!cells[0][1].strikethrough);
    }

    #[test]
    fn process_sgr_reset_clears_all_new_attributes() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set all new attributes, print A, then reset (SGR 0), print B
        adapter
            .process(id(1), b"\x1b[1;2;3;4;7;8;9mA\x1b[0mB")
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // A should have all attributes
        assert!(cells[0][0].bold);
        assert!(cells[0][0].dim);
        assert!(cells[0][0].italic);
        assert!(cells[0][0].underline);
        assert!(cells[0][0].reverse);
        assert!(cells[0][0].hidden);
        assert!(cells[0][0].strikethrough);

        // B should have all attributes cleared
        assert!(!cells[0][1].bold);
        assert!(!cells[0][1].dim);
        assert!(!cells[0][1].italic);
        assert!(!cells[0][1].underline);
        assert!(!cells[0][1].reverse);
        assert!(!cells[0][1].hidden);
        assert!(!cells[0][1].strikethrough);
        assert_eq!(cells[0][1].fg, Color::Default);
        assert_eq!(cells[0][1].bg, Color::Default);
    }

    #[test]
    fn process_sgr_empty_params_resets_all_new_attributes() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set all attributes, print A, then ESC[m (empty = reset), print B
        adapter
            .process(id(1), b"\x1b[2;3;7;8;9mA\x1b[mB")
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].dim);
        assert!(cells[0][0].italic);
        assert!(cells[0][0].reverse);
        assert!(cells[0][0].hidden);
        assert!(cells[0][0].strikethrough);

        assert!(!cells[0][1].dim);
        assert!(!cells[0][1].italic);
        assert!(!cells[0][1].reverse);
        assert!(!cells[0][1].hidden);
        assert!(!cells[0][1].strikethrough);
    }

    #[test]
    fn process_sgr_multiple_new_attributes_combined() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // ESC[2;3;9m — dim + italic + strikethrough in one sequence
        adapter.process(id(1), b"\x1b[2;3;9mA").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].dim);
        assert!(cells[0][0].italic);
        assert!(cells[0][0].strikethrough);
        assert!(!cells[0][0].bold);
        assert!(!cells[0][0].reverse);
        assert!(!cells[0][0].hidden);
    }

    #[test]
    fn process_sgr_new_attributes_persist_across_calls() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Set italic in first call
        adapter.process(id(1), b"\x1b[3m").unwrap();
        // Print in second call — should still be italic
        adapter.process(id(1), b"A").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].italic);
    }

    // =========================================================================
    // Tests: Alternate screen buffer (Task #16)
    // =========================================================================

    #[test]
    fn enter_alternate_screen_saves_primary_and_clears_buffer() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap(); // 10x5

        // Write some content to primary buffer
        adapter.process(id(1), b"Hello").unwrap();
        // Move cursor to a known position
        adapter.process(id(1), b"\x1b[3;5H").unwrap(); // row=2, col=4

        let cursor_before = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor_before.row, 2);
        assert_eq!(cursor_before.col, 4);

        // Enter alternate screen via CSI ?1049h
        adapter.process(id(1), b"\x1b[?1049h").unwrap();

        // Buffer should be cleared (all default cells)
        let cells = adapter.get_cells(id(1)).unwrap();
        for r in 0..5 {
            for c in 0..10 {
                assert_eq!(
                    cells[r][c].ch, ' ',
                    "cell [{r}][{c}] should be blank after entering alternate screen"
                );
            }
        }

        // Cursor should be reset to (0, 0)
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 0);
    }

    #[test]
    fn leave_alternate_screen_restores_primary() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap(); // 10x5

        // Write content to primary buffer
        adapter.process(id(1), b"Hello").unwrap();
        adapter.process(id(1), b"\x1b[2;3H").unwrap(); // cursor row=1, col=2

        let primary_cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(primary_cursor.row, 1);
        assert_eq!(primary_cursor.col, 2);

        // Enter alternate screen
        adapter.process(id(1), b"\x1b[?1049h").unwrap();

        // Write something on alternate screen
        adapter.process(id(1), b"ALT").unwrap();

        // Leave alternate screen
        adapter.process(id(1), b"\x1b[?1049l").unwrap();

        // Primary content should be restored
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'H');
        assert_eq!(cells[0][1].ch, 'e');
        assert_eq!(cells[0][2].ch, 'l');
        assert_eq!(cells[0][3].ch, 'l');
        assert_eq!(cells[0][4].ch, 'o');

        // Primary cursor should be restored
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 1);
        assert_eq!(cursor.col, 2);
    }

    #[test]
    fn alternate_screen_writes_do_not_affect_primary() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap(); // 10x5

        // Write content to primary
        adapter.process(id(1), b"Primary").unwrap();

        // Enter alternate screen
        adapter.process(id(1), b"\x1b[?1049h").unwrap();

        // Write lots of content on alternate screen
        adapter.process(id(1), b"ALTERNATE!").unwrap();
        adapter.process(id(1), b"\x1b[2;1H").unwrap();
        adapter.process(id(1), b"LINE 2").unwrap();

        // Verify alternate screen has the alternate content
        let alt_cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(alt_cells[0][0].ch, 'A');
        assert_eq!(alt_cells[0][1].ch, 'L');
        assert_eq!(alt_cells[1][0].ch, 'L');

        // Leave alternate screen
        adapter.process(id(1), b"\x1b[?1049l").unwrap();

        // Primary content should be intact
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'P');
        assert_eq!(cells[0][1].ch, 'r');
        assert_eq!(cells[0][2].ch, 'i');
        assert_eq!(cells[0][3].ch, 'm');
        assert_eq!(cells[0][4].ch, 'a');
        assert_eq!(cells[0][5].ch, 'r');
        assert_eq!(cells[0][6].ch, 'y');
    }

    #[test]
    fn remove_screen_while_in_alternate_does_not_panic() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Enter alternate screen
        adapter.process(id(1), b"\x1b[?1049h").unwrap();

        // Remove the screen while still in alternate mode — should not panic
        adapter.remove(id(1)).unwrap();

        // Verify screen is gone
        assert!(adapter.get_cells(id(1)).is_err());
    }

    #[test]
    fn csi_1049h_and_1049l_switches_alternate_screen() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"ABC").unwrap();

        // Enter via ?1049h
        adapter.process(id(1), b"\x1b[?1049h").unwrap();

        // Screen should be clear
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, ' ');

        // Write on alternate
        adapter.process(id(1), b"XYZ").unwrap();

        // Leave via ?1049l
        adapter.process(id(1), b"\x1b[?1049l").unwrap();

        // Original content restored
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[0][1].ch, 'B');
        assert_eq!(cells[0][2].ch, 'C');
    }

    #[test]
    fn csi_1047h_and_1047l_switches_alternate_screen() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"DEF").unwrap();

        // Enter via ?1047h
        adapter.process(id(1), b"\x1b[?1047h").unwrap();

        // Screen should be clear
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, ' ');

        // Write on alternate
        adapter.process(id(1), b"UVW").unwrap();

        // Leave via ?1047l
        adapter.process(id(1), b"\x1b[?1047l").unwrap();

        // Original content restored
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'D');
        assert_eq!(cells[0][1].ch, 'E');
        assert_eq!(cells[0][2].ch, 'F');
    }

    #[test]
    fn csi_47h_and_47l_switches_alternate_screen() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"GHI").unwrap();

        // Enter via ?47h
        adapter.process(id(1), b"\x1b[?47h").unwrap();

        // Screen should be clear
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, ' ');

        // Write on alternate
        adapter.process(id(1), b"RST").unwrap();

        // Leave via ?47l
        adapter.process(id(1), b"\x1b[?47l").unwrap();

        // Original content restored
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'G');
        assert_eq!(cells[0][1].ch, 'H');
        assert_eq!(cells[0][2].ch, 'I');
    }

    #[test]
    fn alternate_screen_cursor_reset_to_origin() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Move cursor to non-zero position
        adapter.process(id(1), b"\x1b[4;8H").unwrap(); // row=3, col=7

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 3);
        assert_eq!(cursor.col, 7);

        // Enter alternate screen
        adapter.process(id(1), b"\x1b[?1049h").unwrap();

        // Cursor should be at (0, 0)
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 0);

        // Move cursor in alternate screen
        adapter.process(id(1), b"\x1b[3;5H").unwrap(); // row=2, col=4

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 2);
        assert_eq!(cursor.col, 4);

        // Leave alternate screen
        adapter.process(id(1), b"\x1b[?1049l").unwrap();

        // Primary cursor should be restored (row=3, col=7)
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 3);
        assert_eq!(cursor.col, 7);
    }

    // =========================================================================
    // Tests: DECSTBM (Set Top and Bottom Margins) — CSI r
    // =========================================================================

    #[test]
    fn decstbm_sets_scroll_region() {
        // Test 1: DECSTBM sets scroll_top/scroll_bottom correctly
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(10, 10)).unwrap();

        // CSI 3;7r — set scroll region rows 3..7 (1-indexed) = rows 2..6 (0-indexed)
        adapter.process(id(1), b"\x1b[3;7r").unwrap();

        // After DECSTBM, cursor goes to home (0,0)
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 0);

        // Write identifying chars on rows 0-9
        for r in 0..10u16 {
            adapter
                .process(id(1), format!("\x1b[{};1H{}", r + 1, r).as_bytes())
                .unwrap();
        }

        // Now move cursor to scroll_bottom (row 6) and do LF to trigger scroll
        adapter.process(id(1), b"\x1b[7;1H").unwrap(); // row 6 (0-indexed)
        adapter.process(id(1), b"\n").unwrap(); // should scroll within region

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0-1 should be unchanged (outside region above)
        assert_eq!(cells[0][0].ch, '0');
        assert_eq!(cells[1][0].ch, '1');
        // Row 2 should now have what was row 3 ('3')
        assert_eq!(cells[2][0].ch, '3');
        // Row 3 should now have what was row 4 ('4')
        assert_eq!(cells[3][0].ch, '4');
        // Row 5 should now have what was row 6 ('6')
        assert_eq!(cells[5][0].ch, '6');
        // Row 6 should be blank (new line inserted at bottom of region)
        assert_eq!(cells[6][0].ch, ' ');
        // Rows 7-9 should be unchanged (outside region below)
        assert_eq!(cells[7][0].ch, '7');
        assert_eq!(cells[8][0].ch, '8');
        assert_eq!(cells[9][0].ch, '9');
    }

    #[test]
    fn decstbm_moves_cursor_to_home() {
        // Test 2: DECSTBM resets cursor to (0,0)
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(10, 10)).unwrap();

        // Move cursor somewhere
        adapter.process(id(1), b"\x1b[5;8H").unwrap();
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 4);
        assert_eq!(cursor.col, 7);

        // Set scroll region — cursor should reset to home
        adapter.process(id(1), b"\x1b[2;9r").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 0);
    }

    #[test]
    fn decstbm_scroll_only_within_region() {
        // Test 3: LF at scroll_bottom scrolls only within region
        let mut adapter = VteScreenAdapter::new();
        // 5 cols, 6 rows
        adapter.create(id(1), TerminalSize::new(5, 6)).unwrap();

        // Set scroll region to rows 2-4 (1-indexed) = rows 1-3 (0-indexed)
        adapter.process(id(1), b"\x1b[2;4r").unwrap();

        // Write content on each row
        for r in 0..6u16 {
            let ch = (b'A' + r as u8) as char;
            adapter
                .process(id(1), format!("\x1b[{};1H{}", r + 1, ch).as_bytes())
                .unwrap();
        }

        // Move cursor to scroll_bottom (row 3, 0-indexed) and LF
        adapter.process(id(1), b"\x1b[4;1H").unwrap(); // row 3
        adapter.process(id(1), b"\n").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'A' (unchanged, outside region)
        assert_eq!(cells[0][0].ch, 'A');
        // Row 1: was 'B' but row 2 ('C') should have scrolled up here
        assert_eq!(cells[1][0].ch, 'C');
        // Row 2: was 'C' but row 3 ('D') should have scrolled up here
        assert_eq!(cells[2][0].ch, 'D');
        // Row 3: should be blank (new line inserted at bottom of region)
        assert_eq!(cells[3][0].ch, ' ');
        // Row 4: 'E' (unchanged, outside region)
        assert_eq!(cells[4][0].ch, 'E');
        // Row 5: 'F' (unchanged, outside region)
        assert_eq!(cells[5][0].ch, 'F');
    }

    #[test]
    fn decstbm_outside_rows_unaffected() {
        // Test 4: Rows outside scroll region are not affected by scrolling
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 5)).unwrap();

        // Set scroll region to rows 2-4 (1-indexed) = rows 1-3 (0-indexed)
        adapter.process(id(1), b"\x1b[2;4r").unwrap();

        // Write content on rows 0 and 4 (outside region)
        adapter.process(id(1), b"\x1b[1;1HTOP").unwrap();
        adapter.process(id(1), b"\x1b[5;1HBOT").unwrap();

        // Write in scroll region
        adapter.process(id(1), b"\x1b[2;1Haaa").unwrap();
        adapter.process(id(1), b"\x1b[3;1Hbbb").unwrap();
        adapter.process(id(1), b"\x1b[4;1Hccc").unwrap();

        // Scroll multiple times by LF at scroll_bottom
        adapter.process(id(1), b"\x1b[4;1H").unwrap();
        adapter.process(id(1), b"\n").unwrap(); // scroll 1
        adapter.process(id(1), b"\n").unwrap(); // scroll 2 (cursor stays at scroll_bottom)
        adapter.process(id(1), b"\n").unwrap(); // scroll 3

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0 unchanged
        assert_eq!(cells[0][0].ch, 'T');
        assert_eq!(cells[0][1].ch, 'O');
        assert_eq!(cells[0][2].ch, 'P');
        // Row 4 unchanged
        assert_eq!(cells[4][0].ch, 'B');
        assert_eq!(cells[4][1].ch, 'O');
        assert_eq!(cells[4][2].ch, 'T');
    }

    #[test]
    fn decstbm_reset_with_no_params() {
        // Test 5: CSI r with no params resets scroll region to full screen
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 5)).unwrap();

        // Set a restricted region
        adapter.process(id(1), b"\x1b[2;4r").unwrap();

        // Reset with no params
        adapter.process(id(1), b"\x1b[r").unwrap();

        // Write content on all rows
        for r in 0..5u16 {
            let ch = (b'A' + r as u8) as char;
            adapter
                .process(id(1), format!("\x1b[{};1H{}", r + 1, ch).as_bytes())
                .unwrap();
        }

        // Move cursor to last row and LF — should scroll entire screen
        adapter.process(id(1), b"\x1b[5;1H").unwrap(); // row 4 (0-indexed)
        adapter.process(id(1), b"\n").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0 should now have what was row 1 ('B')
        assert_eq!(cells[0][0].ch, 'B');
        // Row 3 should now have what was row 4 ('E')
        assert_eq!(cells[3][0].ch, 'E');
        // Row 4 should be blank
        assert_eq!(cells[4][0].ch, ' ');
    }

    #[test]
    fn scroll_down_works_correctly() {
        // Test 6: scroll_down inserts blank line at top of region, removes bottom
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 5)).unwrap();

        // Set scroll region to rows 2-4 (1-indexed) = rows 1-3 (0-indexed)
        adapter.process(id(1), b"\x1b[2;4r").unwrap();

        // Write content
        adapter.process(id(1), b"\x1b[1;1HTOP").unwrap();
        adapter.process(id(1), b"\x1b[2;1Haaa").unwrap();
        adapter.process(id(1), b"\x1b[3;1Hbbb").unwrap();
        adapter.process(id(1), b"\x1b[4;1Hccc").unwrap();
        adapter.process(id(1), b"\x1b[5;1HBOT").unwrap();

        // Access the ScreenInstance directly to call scroll_down
        let screen = adapter.screens.get_mut(&id(1)).unwrap();
        screen.scroll_down();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'TOP' unchanged
        assert_eq!(cells[0][0].ch, 'T');
        // Row 1: should be blank (scroll_down inserts at top of region)
        assert_eq!(cells[1][0].ch, ' ');
        assert_eq!(cells[1][1].ch, ' ');
        // Row 2: was 'aaa' (shifted down from row 1)
        assert_eq!(cells[2][0].ch, 'a');
        // Row 3: was 'bbb' (shifted down from row 2) — 'ccc' was removed
        assert_eq!(cells[3][0].ch, 'b');
        // Row 4: 'BOT' unchanged
        assert_eq!(cells[4][0].ch, 'B');
    }

    #[test]
    fn auto_wrap_scroll_respects_scroll_region() {
        // Test 7: Auto-wrap scrolling respects scroll region
        let mut adapter = VteScreenAdapter::new();
        // 3 cols, 5 rows
        adapter.create(id(1), TerminalSize::new(3, 5)).unwrap();

        // Set scroll region to rows 2-4 (1-indexed) = rows 1-3 (0-indexed)
        adapter.process(id(1), b"\x1b[2;4r").unwrap();

        // Write outside region
        adapter.process(id(1), b"\x1b[1;1HTOP").unwrap();
        adapter.process(id(1), b"\x1b[5;1HBOT").unwrap();

        // Position cursor at scroll_bottom (row 3), col 0
        adapter.process(id(1), b"\x1b[4;1H").unwrap();
        // Write 4 chars to trigger auto-wrap + scroll within region
        adapter.process(id(1), b"XXXY").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'TOP' unchanged (outside region)
        assert_eq!(cells[0][0].ch, 'T');
        assert_eq!(cells[0][1].ch, 'O');
        assert_eq!(cells[0][2].ch, 'P');
        // Row 3 had 'XXX', after wrap+scroll it moves up
        // Row 3 (scroll_bottom) is the new line with 'Y' at col 0
        assert_eq!(cells[3][0].ch, 'Y');
        // Row 4: 'BOT' unchanged (outside region)
        assert_eq!(cells[4][0].ch, 'B');
        assert_eq!(cells[4][1].ch, 'O');
        assert_eq!(cells[4][2].ch, 'T');
    }

    #[test]
    fn resize_resets_scroll_region() {
        // Test 8: resize() resets scroll region to full screen
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 10)).unwrap();

        // Set restricted scroll region
        adapter.process(id(1), b"\x1b[3;7r").unwrap();

        // Resize
        adapter.resize(id(1), TerminalSize::new(5, 8)).unwrap();

        // Write content on all rows
        for r in 0..8u16 {
            let ch = (b'A' + r as u8) as char;
            adapter
                .process(id(1), format!("\x1b[{};1H{}", r + 1, ch).as_bytes())
                .unwrap();
        }

        // Move to last row and LF — should scroll the full screen
        adapter.process(id(1), b"\x1b[8;1H").unwrap();
        adapter.process(id(1), b"\n").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0 should now have what was row 1 ('B') — full screen scroll
        assert_eq!(cells[0][0].ch, 'B');
        // Last row should be blank
        assert_eq!(cells[7][0].ch, ' ');
    }

    #[test]
    fn decstbm_invalid_params_ignored() {
        // Test 9: Invalid params (top >= bottom) are ignored
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 5)).unwrap();

        // Write content
        for r in 0..5u16 {
            let ch = (b'A' + r as u8) as char;
            adapter
                .process(id(1), format!("\x1b[{};1H{}", r + 1, ch).as_bytes())
                .unwrap();
        }

        // Try setting top >= bottom: CSI 5;3r (top=4, bottom=2, invalid)
        adapter.process(id(1), b"\x1b[5;3r").unwrap();

        // Cursor goes to home regardless (DECSTBM always resets cursor)
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 0);

        // Scroll region should still be full screen (default)
        // LF at bottom row should scroll entire screen
        adapter.process(id(1), b"\x1b[5;1H").unwrap();
        adapter.process(id(1), b"\n").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0 should now have what was row 1 ('B')
        assert_eq!(cells[0][0].ch, 'B');
        // Row 4 should be blank
        assert_eq!(cells[4][0].ch, ' ');
    }

    // =========================================================================
    // Tests: CSI L (Insert Lines / IL) — Task #18
    // =========================================================================

    #[test]
    fn csi_il_inserts_line_and_removes_bottom_of_scroll_region() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 6)).unwrap();

        // Set scroll region to rows 2-5 (1-indexed) = rows 1-4 (0-indexed)
        adapter.process(id(1), b"\x1b[2;5r").unwrap();

        // Write content on each row
        for r in 0..6u16 {
            let ch = (b'A' + r as u8) as char;
            adapter
                .process(id(1), format!("\x1b[{};1H{}", r + 1, ch).as_bytes())
                .unwrap();
        }

        // Move cursor to row 3 (1-indexed) = row 2 (0-indexed), within scroll region
        adapter.process(id(1), b"\x1b[3;1H").unwrap();

        // Insert 1 line: CSI L
        adapter.process(id(1), b"\x1b[L").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'A' unchanged (outside scroll region above)
        assert_eq!(cells[0][0].ch, 'A');
        // Row 1: 'B' unchanged (top of scroll region, above cursor)
        assert_eq!(cells[1][0].ch, 'B');
        // Row 2: blank (newly inserted line at cursor position)
        assert_eq!(cells[2][0].ch, ' ');
        // Row 3: 'C' (shifted down from row 2)
        assert_eq!(cells[3][0].ch, 'C');
        // Row 4: 'D' (shifted down from row 3; 'E' was pushed off scroll_bottom)
        assert_eq!(cells[4][0].ch, 'D');
        // Row 5: 'F' unchanged (outside scroll region below)
        assert_eq!(cells[5][0].ch, 'F');
    }

    #[test]
    fn csi_il_default_param_is_one() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 4)).unwrap();

        // Fill rows
        adapter.process(id(1), b"\x1b[1;1HA").unwrap();
        adapter.process(id(1), b"\x1b[2;1HB").unwrap();
        adapter.process(id(1), b"\x1b[3;1HC").unwrap();
        adapter.process(id(1), b"\x1b[4;1HD").unwrap();

        // Move to row 2 (0-indexed: row 1)
        adapter.process(id(1), b"\x1b[2;1H").unwrap();

        // CSI L with no params (default n=1)
        adapter.process(id(1), b"\x1b[L").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[1][0].ch, ' '); // inserted blank line
        assert_eq!(cells[2][0].ch, 'B'); // shifted down
        assert_eq!(cells[3][0].ch, 'C'); // shifted down, 'D' pushed off
    }

    // =========================================================================
    // Tests: CSI M (Delete Lines / DL) — Task #18
    // =========================================================================

    #[test]
    fn csi_dl_deletes_line_and_inserts_blank_at_bottom_of_scroll_region() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 6)).unwrap();

        // Set scroll region to rows 2-5 (1-indexed) = rows 1-4 (0-indexed)
        adapter.process(id(1), b"\x1b[2;5r").unwrap();

        // Write content on each row
        for r in 0..6u16 {
            let ch = (b'A' + r as u8) as char;
            adapter
                .process(id(1), format!("\x1b[{};1H{}", r + 1, ch).as_bytes())
                .unwrap();
        }

        // Move cursor to row 3 (1-indexed) = row 2 (0-indexed), within scroll region
        adapter.process(id(1), b"\x1b[3;1H").unwrap();

        // Delete 1 line: CSI M
        adapter.process(id(1), b"\x1b[M").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'A' unchanged (outside scroll region)
        assert_eq!(cells[0][0].ch, 'A');
        // Row 1: 'B' unchanged (top of region, above cursor)
        assert_eq!(cells[1][0].ch, 'B');
        // Row 2: 'D' (was row 3, shifted up after 'C' deleted)
        assert_eq!(cells[2][0].ch, 'D');
        // Row 3: 'E' (was row 4, shifted up)
        assert_eq!(cells[3][0].ch, 'E');
        // Row 4: blank (inserted at scroll_bottom)
        assert_eq!(cells[4][0].ch, ' ');
        // Row 5: 'F' unchanged (outside scroll region)
        assert_eq!(cells[5][0].ch, 'F');
    }

    #[test]
    fn csi_dl_default_param_is_one() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 4)).unwrap();

        // Fill rows
        adapter.process(id(1), b"\x1b[1;1HA").unwrap();
        adapter.process(id(1), b"\x1b[2;1HB").unwrap();
        adapter.process(id(1), b"\x1b[3;1HC").unwrap();
        adapter.process(id(1), b"\x1b[4;1HD").unwrap();

        // Move to row 2 (0-indexed: row 1)
        adapter.process(id(1), b"\x1b[2;1H").unwrap();

        // CSI M with no params (default n=1)
        adapter.process(id(1), b"\x1b[M").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[1][0].ch, 'C'); // row 2 ('C') shifted up
        assert_eq!(cells[2][0].ch, 'D'); // row 3 ('D') shifted up
        assert_eq!(cells[3][0].ch, ' '); // blank at bottom
    }

    // =========================================================================
    // Tests: CSI @ (Insert Characters / ICH) — Task #18
    // =========================================================================

    #[test]
    fn csi_ich_inserts_characters_pushing_end_off_line() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(6, 3)).unwrap();

        // Write "ABCDEF" on row 0
        adapter.process(id(1), b"\x1b[1;1HABCDEF").unwrap();

        // Move cursor to col 2 (0-indexed: col 1)
        adapter.process(id(1), b"\x1b[1;2H").unwrap();

        // Insert 2 characters: CSI 2 @
        adapter.process(id(1), b"\x1b[2@").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // 'A' remains at col 0
        assert_eq!(cells[0][0].ch, 'A');
        // Cols 1-2: blank (inserted)
        assert_eq!(cells[0][1].ch, ' ');
        assert_eq!(cells[0][2].ch, ' ');
        // 'B','C','D' shifted right; 'E','F' pushed off
        assert_eq!(cells[0][3].ch, 'B');
        assert_eq!(cells[0][4].ch, 'C');
        assert_eq!(cells[0][5].ch, 'D');
    }

    // =========================================================================
    // Tests: CSI P (Delete Characters / DCH) — Task #18
    // =========================================================================

    #[test]
    fn csi_dch_deletes_characters_shifting_left() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(6, 3)).unwrap();

        // Write "ABCDEF" on row 0
        adapter.process(id(1), b"\x1b[1;1HABCDEF").unwrap();

        // Move cursor to col 2 (0-indexed: col 1)
        adapter.process(id(1), b"\x1b[1;2H").unwrap();

        // Delete 2 characters: CSI 2 P
        adapter.process(id(1), b"\x1b[2P").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // 'A' stays at col 0
        assert_eq!(cells[0][0].ch, 'A');
        // 'D','E','F' shifted left to cols 1,2,3
        assert_eq!(cells[0][1].ch, 'D');
        assert_eq!(cells[0][2].ch, 'E');
        assert_eq!(cells[0][3].ch, 'F');
        // Cols 4,5: blank (filled from right)
        assert_eq!(cells[0][4].ch, ' ');
        assert_eq!(cells[0][5].ch, ' ');
    }

    // =========================================================================
    // Tests: CSI X (Erase Characters / ECH) — Task #18
    // =========================================================================

    #[test]
    fn csi_ech_erases_characters_without_moving_cursor() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(6, 3)).unwrap();

        // Write "ABCDEF" on row 0
        adapter.process(id(1), b"\x1b[1;1HABCDEF").unwrap();

        // Move cursor to col 3 (0-indexed: col 2)
        adapter.process(id(1), b"\x1b[1;3H").unwrap();

        // Erase 2 characters: CSI 2 X
        adapter.process(id(1), b"\x1b[2X").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[0][1].ch, 'B');
        // Cols 2,3 erased
        assert_eq!(cells[0][2].ch, ' ');
        assert_eq!(cells[0][3].ch, ' ');
        // Cols 4,5 remain intact
        assert_eq!(cells[0][4].ch, 'E');
        assert_eq!(cells[0][5].ch, 'F');

        // Cursor should NOT have moved
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 2);
    }

    #[test]
    fn csi_ech_cursor_does_not_move_explicit() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(10, 3)).unwrap();

        // Write content
        adapter.process(id(1), b"ABCDEFGHIJ").unwrap();

        // Move cursor to col 5 (0-indexed: col 4)
        adapter.process(id(1), b"\x1b[1;5H").unwrap();
        let cursor_before = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor_before.row, 0);
        assert_eq!(cursor_before.col, 4);

        // Erase 3 characters
        adapter.process(id(1), b"\x1b[3X").unwrap();

        // Cursor position should be unchanged
        let cursor_after = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor_after.row, cursor_before.row);
        assert_eq!(cursor_after.col, cursor_before.col);
    }

    // =========================================================================
    // Tests: CSI S (Scroll Up / SU) — Task #18
    // =========================================================================

    #[test]
    fn csi_su_scrolls_up_within_scroll_region() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 6)).unwrap();

        // Set scroll region to rows 2-5 (1-indexed) = rows 1-4 (0-indexed)
        adapter.process(id(1), b"\x1b[2;5r").unwrap();

        // Write content on each row
        for r in 0..6u16 {
            let ch = (b'A' + r as u8) as char;
            adapter
                .process(id(1), format!("\x1b[{};1H{}", r + 1, ch).as_bytes())
                .unwrap();
        }

        // CSI S (scroll up 1 line)
        adapter.process(id(1), b"\x1b[S").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'A' unchanged (outside region)
        assert_eq!(cells[0][0].ch, 'A');
        // Row 1: 'C' (was row 2, shifted up)
        assert_eq!(cells[1][0].ch, 'C');
        // Row 2: 'D' (was row 3, shifted up)
        assert_eq!(cells[2][0].ch, 'D');
        // Row 3: 'E' (was row 4, shifted up)
        assert_eq!(cells[3][0].ch, 'E');
        // Row 4: blank (new line at bottom of scroll region)
        assert_eq!(cells[4][0].ch, ' ');
        // Row 5: 'F' unchanged (outside region)
        assert_eq!(cells[5][0].ch, 'F');
    }

    // =========================================================================
    // Tests: CSI T (Scroll Down / SD) — Task #18
    // =========================================================================

    #[test]
    fn csi_sd_scrolls_down_within_scroll_region() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 6)).unwrap();

        // Set scroll region to rows 2-5 (1-indexed) = rows 1-4 (0-indexed)
        adapter.process(id(1), b"\x1b[2;5r").unwrap();

        // Write content on each row
        for r in 0..6u16 {
            let ch = (b'A' + r as u8) as char;
            adapter
                .process(id(1), format!("\x1b[{};1H{}", r + 1, ch).as_bytes())
                .unwrap();
        }

        // CSI T (scroll down 1 line)
        adapter.process(id(1), b"\x1b[T").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'A' unchanged (outside region)
        assert_eq!(cells[0][0].ch, 'A');
        // Row 1: blank (new line at top of scroll region)
        assert_eq!(cells[1][0].ch, ' ');
        // Row 2: 'B' (was row 1, shifted down)
        assert_eq!(cells[2][0].ch, 'B');
        // Row 3: 'C' (was row 2, shifted down)
        assert_eq!(cells[3][0].ch, 'C');
        // Row 4: 'D' (was row 3, shifted down; 'E' pushed off scroll_bottom)
        assert_eq!(cells[4][0].ch, 'D');
        // Row 5: 'F' unchanged (outside region)
        assert_eq!(cells[5][0].ch, 'F');
    }

    // =========================================================================
    // Tests: CSI G (Cursor Horizontal Absolute / CHA) — Task #18
    // =========================================================================

    #[test]
    fn csi_cha_moves_cursor_to_absolute_column() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(20, 5)).unwrap();

        // Move cursor to row 3, col 10 first
        adapter.process(id(1), b"\x1b[3;10H").unwrap();

        // CSI 5 G — move to column 5 (1-indexed), row stays at 2 (0-indexed)
        adapter.process(id(1), b"\x1b[5G").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 2); // row unchanged
        assert_eq!(cursor.col, 4); // 5-1 = 4 (0-indexed)
    }

    // =========================================================================
    // Tests: CSI d (Cursor Vertical Absolute / VPA) — Task #18
    // =========================================================================

    #[test]
    fn csi_vpa_moves_cursor_to_absolute_row() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(20, 10)).unwrap();

        // Move cursor to row 3, col 10 first
        adapter.process(id(1), b"\x1b[3;10H").unwrap();

        // CSI 7 d — move to row 7 (1-indexed), col stays at 9 (0-indexed)
        adapter.process(id(1), b"\x1b[7d").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 6); // 7-1 = 6 (0-indexed)
        assert_eq!(cursor.col, 9); // col unchanged
    }

    // =========================================================================
    // Tests: CSI s / CSI u (Save/Restore Cursor Position) — Task #18
    // =========================================================================

    #[test]
    fn csi_scp_rcp_saves_and_restores_cursor() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(20, 10)).unwrap();

        // Move cursor to (5, 12)
        adapter.process(id(1), b"\x1b[6;13H").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 5);
        assert_eq!(cursor.col, 12);

        // Save cursor: CSI s
        adapter.process(id(1), b"\x1b[s").unwrap();

        // Move cursor somewhere else
        adapter.process(id(1), b"\x1b[1;1H").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 0);

        // Restore cursor: CSI u
        adapter.process(id(1), b"\x1b[u").unwrap();

        // Cursor should be back at (5, 12)
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 5);
        assert_eq!(cursor.col, 12);
    }

    #[test]
    fn csi_rcp_without_save_does_nothing() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(20, 10)).unwrap();

        // Move cursor to (3, 7)
        adapter.process(id(1), b"\x1b[4;8H").unwrap();

        let cursor_before = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor_before.row, 3);
        assert_eq!(cursor_before.col, 7);

        // Restore without prior save: CSI u
        adapter.process(id(1), b"\x1b[u").unwrap();

        // Cursor should be unchanged
        let cursor_after = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor_after.row, 3);
        assert_eq!(cursor_after.col, 7);
    }

    #[test]
    fn double_enter_alternate_screen_is_stable() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Write primary content
        adapter.process(id(1), b"First").unwrap();

        // Enter alternate screen first time
        adapter.process(id(1), b"\x1b[?1049h").unwrap();

        // Write on alternate
        adapter.process(id(1), b"Alt1").unwrap();

        // Enter alternate screen again (double enter)
        adapter.process(id(1), b"\x1b[?1049h").unwrap();

        // Screen should be clear again (fresh alternate)
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, ' ');

        // Cursor should be reset
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 0);

        // Leave alternate screen once — should still restore primary
        adapter.process(id(1), b"\x1b[?1049l").unwrap();

        // Primary content should be restored
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'F');
        assert_eq!(cells[0][1].ch, 'i');
        assert_eq!(cells[0][2].ch, 'r');
        assert_eq!(cells[0][3].ch, 's');
        assert_eq!(cells[0][4].ch, 't');
    }

    // =========================================================================
    // Tests: OSC dispatch — title setting
    // =========================================================================

    #[test]
    fn osc_0_sets_title() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // OSC 0 (Set Icon Name and Window Title): \x1b]0;My Title\x07
        adapter.process(id(1), b"\x1b]0;My Title\x07").unwrap();

        let title = adapter.get_title(id(1)).unwrap();
        assert_eq!(title, Some("My Title".to_string()));
    }

    #[test]
    fn osc_2_sets_title() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // OSC 2 (Set Window Title): \x1b]2;Window Title\x07
        adapter.process(id(1), b"\x1b]2;Window Title\x07").unwrap();

        let title = adapter.get_title(id(1)).unwrap();
        assert_eq!(title, Some("Window Title".to_string()));
    }

    #[test]
    fn osc_title_overwritten_by_subsequent_set() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // Set title once
        adapter.process(id(1), b"\x1b]0;First Title\x07").unwrap();
        assert_eq!(
            adapter.get_title(id(1)).unwrap(),
            Some("First Title".to_string())
        );

        // Set title again — should overwrite
        adapter
            .process(id(1), b"\x1b]2;Second Title\x07")
            .unwrap();
        assert_eq!(
            adapter.get_title(id(1)).unwrap(),
            Some("Second Title".to_string())
        );
    }

    #[test]
    fn osc_unknown_command_ignored() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // Set a known title first
        adapter.process(id(1), b"\x1b]0;Known Title\x07").unwrap();

        // Unknown OSC command 999 — should be silently ignored
        adapter.process(id(1), b"\x1b]999;data\x07").unwrap();

        // Title should remain unchanged
        let title = adapter.get_title(id(1)).unwrap();
        assert_eq!(title, Some("Known Title".to_string()));
    }

    #[test]
    fn osc_title_initially_none() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // Before any OSC sequence, title should be None
        let title = adapter.get_title(id(1)).unwrap();
        assert_eq!(title, None);
    }

    #[test]
    fn get_title_returns_error_for_unknown_screen() {
        let adapter = VteScreenAdapter::new();

        let result = adapter.get_title(id(99));
        assert!(result.is_err());
    }

    // Note: TERM environment variable (set in PortablePtyAdapter::spawn) is tested
    // at integration level since it requires actual pty/process spawning via FFI.
    // The change itself is a single `cmd.env("TERM", "xterm-256color")` call in
    // portable_pty_adapter.rs and does not lend itself to isolated unit testing.

    // =========================================================================
    // Tests: DEC Private Mode — cursor_visible (CSI ?25h/l) — Task #19
    // =========================================================================

    #[test]
    fn dec_private_mode_cursor_visible_default_true() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // cursor_visible should be true by default
        let visible = adapter.get_cursor_visible(id(1)).unwrap();
        assert!(visible);
    }

    #[test]
    fn dec_private_mode_cursor_visible_hide() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // CSI ?25l — hide cursor
        adapter.process(id(1), b"\x1b[?25l").unwrap();

        let visible = adapter.get_cursor_visible(id(1)).unwrap();
        assert!(!visible);
    }

    #[test]
    fn dec_private_mode_cursor_visible_show() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Hide cursor first
        adapter.process(id(1), b"\x1b[?25l").unwrap();
        assert!(!adapter.get_cursor_visible(id(1)).unwrap());

        // CSI ?25h — show cursor
        adapter.process(id(1), b"\x1b[?25h").unwrap();

        let visible = adapter.get_cursor_visible(id(1)).unwrap();
        assert!(visible);
    }

    // =========================================================================
    // Tests: DEC Private Mode — autowrap (CSI ?7h/l) — Task #19
    // =========================================================================

    #[test]
    fn dec_private_mode_autowrap_default_true() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        let screen = adapter.screens.get(&id(1)).unwrap();
        assert!(screen.autowrap);
    }

    #[test]
    fn dec_private_mode_autowrap_disable() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // CSI ?7l — disable autowrap
        adapter.process(id(1), b"\x1b[?7l").unwrap();

        let screen = adapter.screens.get(&id(1)).unwrap();
        assert!(!screen.autowrap);
    }

    #[test]
    fn dec_private_mode_autowrap_enable() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Disable first
        adapter.process(id(1), b"\x1b[?7l").unwrap();
        assert!(!adapter.screens.get(&id(1)).unwrap().autowrap);

        // CSI ?7h — enable autowrap
        adapter.process(id(1), b"\x1b[?7h").unwrap();

        let screen = adapter.screens.get(&id(1)).unwrap();
        assert!(screen.autowrap);
    }

    // =========================================================================
    // Tests: DEC Private Mode — application_cursor_keys (CSI ?1h/l) — Task #19
    // =========================================================================

    #[test]
    fn dec_private_mode_application_cursor_keys_default_false() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        let screen = adapter.screens.get(&id(1)).unwrap();
        assert!(!screen.application_cursor_keys);
    }

    #[test]
    fn dec_private_mode_application_cursor_keys_enable() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // CSI ?1h — enable application cursor keys (DECCKM)
        adapter.process(id(1), b"\x1b[?1h").unwrap();

        let screen = adapter.screens.get(&id(1)).unwrap();
        assert!(screen.application_cursor_keys);
    }

    #[test]
    fn dec_private_mode_application_cursor_keys_disable() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Enable first
        adapter.process(id(1), b"\x1b[?1h").unwrap();
        assert!(adapter.screens.get(&id(1)).unwrap().application_cursor_keys);

        // CSI ?1l — disable
        adapter.process(id(1), b"\x1b[?1l").unwrap();

        let screen = adapter.screens.get(&id(1)).unwrap();
        assert!(!screen.application_cursor_keys);
    }

    // =========================================================================
    // Tests: DEC Private Mode — bracketed_paste (CSI ?2004h/l) — Task #19
    // =========================================================================

    #[test]
    fn dec_private_mode_bracketed_paste_default_false() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        let screen = adapter.screens.get(&id(1)).unwrap();
        assert!(!screen.bracketed_paste);
    }

    #[test]
    fn dec_private_mode_bracketed_paste_enable() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // CSI ?2004h — enable bracketed paste
        adapter.process(id(1), b"\x1b[?2004h").unwrap();

        let screen = adapter.screens.get(&id(1)).unwrap();
        assert!(screen.bracketed_paste);
    }

    #[test]
    fn dec_private_mode_bracketed_paste_disable() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Enable first
        adapter.process(id(1), b"\x1b[?2004h").unwrap();
        assert!(adapter.screens.get(&id(1)).unwrap().bracketed_paste);

        // CSI ?2004l — disable
        adapter.process(id(1), b"\x1b[?2004l").unwrap();

        let screen = adapter.screens.get(&id(1)).unwrap();
        assert!(!screen.bracketed_paste);
    }

    // =========================================================================
    // Tests: ESC 7 / ESC 8 (DECSC/DECRC) — Save/Restore Cursor + SGR — Task #19
    // =========================================================================

    #[test]
    fn esc_decsc_decrc_saves_and_restores_cursor_and_sgr() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // Move cursor to (5, 12) and set SGR attributes
        adapter.process(id(1), b"\x1b[6;13H").unwrap(); // row=5, col=12
        adapter.process(id(1), b"\x1b[1;3;31;42m").unwrap(); // bold, italic, fg=red, bg=green

        // ESC 7 — save cursor + SGR
        adapter.process(id(1), b"\x1b7").unwrap();

        // Move cursor elsewhere and reset SGR
        adapter.process(id(1), b"\x1b[1;1H\x1b[0m").unwrap();

        // Verify cursor has moved and SGR is reset
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 0);

        // ESC 8 — restore cursor + SGR
        adapter.process(id(1), b"\x1b8").unwrap();

        // Cursor should be restored
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 5);
        assert_eq!(cursor.col, 12);

        // SGR attributes should be restored — print a char and check
        adapter.process(id(1), b"X").unwrap();
        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[5][12].bold);
        assert!(cells[5][12].italic);
        assert_eq!(cells[5][12].fg, Color::Indexed(1)); // red
        assert_eq!(cells[5][12].bg, Color::Indexed(2)); // green
    }

    #[test]
    fn esc_decrc_without_prior_save_does_nothing() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // Move cursor to (3, 7)
        adapter.process(id(1), b"\x1b[4;8H").unwrap();

        // ESC 8 without prior ESC 7
        adapter.process(id(1), b"\x1b8").unwrap();

        // Cursor should be unchanged
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 3);
        assert_eq!(cursor.col, 7);
    }

    #[test]
    fn esc_decsc_decrc_restores_all_sgr_attributes() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // Set all SGR attributes
        adapter.process(id(1), b"\x1b[1;2;3;4;7;8;9m").unwrap();
        // bold, dim, italic, underline, reverse, hidden, strikethrough
        adapter.process(id(1), b"\x1b[38;2;100;200;50m").unwrap(); // RGB fg
        adapter.process(id(1), b"\x1b[48;5;130m").unwrap(); // 256-color bg

        // Move cursor
        adapter.process(id(1), b"\x1b[10;20H").unwrap();

        // ESC 7 — save
        adapter.process(id(1), b"\x1b7").unwrap();

        // Reset all
        adapter.process(id(1), b"\x1b[0m\x1b[1;1H").unwrap();

        // ESC 8 — restore
        adapter.process(id(1), b"\x1b8").unwrap();

        // Print and verify
        adapter.process(id(1), b"Z").unwrap();
        let cells = adapter.get_cells(id(1)).unwrap();
        let cell = &cells[9][19]; // row 9, col 19
        assert!(cell.bold);
        assert!(cell.dim);
        assert!(cell.italic);
        assert!(cell.underline);
        assert!(cell.reverse);
        assert!(cell.hidden);
        assert!(cell.strikethrough);
        assert_eq!(cell.fg, Color::Rgb(100, 200, 50));
        assert_eq!(cell.bg, Color::Indexed(130));
    }

    // =========================================================================
    // Tests: ESC M (Reverse Index / RI) — Task #19
    // =========================================================================

    #[test]
    fn esc_ri_moves_cursor_up_one_line() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(10, 5)).unwrap();

        // Move cursor to row 3
        adapter.process(id(1), b"\x1b[4;1H").unwrap();

        // ESC M — reverse index (cursor up)
        adapter.process(id(1), b"\x1bM").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 2);
    }

    #[test]
    fn esc_ri_at_scroll_top_triggers_scroll_down() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 5)).unwrap();

        // Set scroll region rows 2-4 (1-indexed) = rows 1-3 (0-indexed)
        adapter.process(id(1), b"\x1b[2;4r").unwrap();

        // Write content in scroll region
        adapter.process(id(1), b"\x1b[2;1Haaa").unwrap();
        adapter.process(id(1), b"\x1b[3;1Hbbb").unwrap();
        adapter.process(id(1), b"\x1b[4;1Hccc").unwrap();

        // Write outside region
        adapter.process(id(1), b"\x1b[1;1HTOP").unwrap();
        adapter.process(id(1), b"\x1b[5;1HBOT").unwrap();

        // Move cursor to scroll_top (row 1, 0-indexed)
        adapter.process(id(1), b"\x1b[2;1H").unwrap();

        // ESC M — reverse index at scroll_top should scroll down
        adapter.process(id(1), b"\x1bM").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'TOP' unchanged (outside region)
        assert_eq!(cells[0][0].ch, 'T');
        // Row 1: blank (new row inserted by scroll_down at top of region)
        assert_eq!(cells[1][0].ch, ' ');
        // Row 2: 'aaa' (shifted down)
        assert_eq!(cells[2][0].ch, 'a');
        // Row 3: 'bbb' (shifted down; 'ccc' pushed off)
        assert_eq!(cells[3][0].ch, 'b');
        // Row 4: 'BOT' unchanged (outside region)
        assert_eq!(cells[4][0].ch, 'B');

        // Cursor should remain at scroll_top
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 1);
    }

    #[test]
    fn esc_ri_at_row_zero_stays() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(10, 5)).unwrap();

        // Cursor at row 0 (scroll_top is also 0 by default)
        // ESC M at scroll_top triggers scroll_down
        adapter.process(id(1), b"\x1b[1;1HA").unwrap();
        adapter.process(id(1), b"\x1b[2;1HB").unwrap();
        adapter.process(id(1), b"\x1b[1;1H").unwrap(); // row 0

        adapter.process(id(1), b"\x1bM").unwrap();

        // scroll_down should have happened: row 0 is blank, row 1 is 'A'
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, ' ');
        assert_eq!(cells[1][0].ch, 'A');

        // Cursor stays at row 0
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
    }

    // =========================================================================
    // Tests: ESC D (Index / IND) — Task #19
    // =========================================================================

    #[test]
    fn esc_ind_moves_cursor_down_one_line() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(10, 5)).unwrap();

        // Cursor at row 0
        adapter.process(id(1), b"\x1bD").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 1);
    }

    #[test]
    fn esc_ind_at_scroll_bottom_triggers_scroll_up() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 5)).unwrap();

        // Set scroll region rows 2-4 (1-indexed) = rows 1-3 (0-indexed)
        adapter.process(id(1), b"\x1b[2;4r").unwrap();

        // Write content
        adapter.process(id(1), b"\x1b[1;1HTOP").unwrap();
        adapter.process(id(1), b"\x1b[2;1Haaa").unwrap();
        adapter.process(id(1), b"\x1b[3;1Hbbb").unwrap();
        adapter.process(id(1), b"\x1b[4;1Hccc").unwrap();
        adapter.process(id(1), b"\x1b[5;1HBOT").unwrap();

        // Move cursor to scroll_bottom (row 3, 0-indexed)
        adapter.process(id(1), b"\x1b[4;1H").unwrap();

        // ESC D — index at scroll_bottom should scroll up
        adapter.process(id(1), b"\x1bD").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'TOP' unchanged
        assert_eq!(cells[0][0].ch, 'T');
        // Row 1: 'bbb' (shifted up from row 2)
        assert_eq!(cells[1][0].ch, 'b');
        // Row 2: 'ccc' (shifted up from row 3)
        assert_eq!(cells[2][0].ch, 'c');
        // Row 3: blank (new line at bottom of region)
        assert_eq!(cells[3][0].ch, ' ');
        // Row 4: 'BOT' unchanged
        assert_eq!(cells[4][0].ch, 'B');

        // Cursor stays at scroll_bottom
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 3);
    }

    #[test]
    fn esc_ind_at_last_row_triggers_scroll_up() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        // Fill rows
        adapter.process(id(1), b"\x1b[1;1HA").unwrap();
        adapter.process(id(1), b"\x1b[2;1HB").unwrap();
        adapter.process(id(1), b"\x1b[3;1HC").unwrap();

        // Move to last row (row 2, 0-indexed = scroll_bottom)
        adapter.process(id(1), b"\x1b[3;1H").unwrap();

        // ESC D
        adapter.process(id(1), b"\x1bD").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'B' (shifted up)
        assert_eq!(cells[0][0].ch, 'B');
        // Row 1: 'C' (shifted up)
        assert_eq!(cells[1][0].ch, 'C');
        // Row 2: blank
        assert_eq!(cells[2][0].ch, ' ');
    }

    // =========================================================================
    // Tests: ESC E (Next Line / NEL) — Task #19
    // =========================================================================

    #[test]
    fn esc_nel_moves_to_start_of_next_line() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(10, 5)).unwrap();

        // Move cursor to row 1, col 5
        adapter.process(id(1), b"\x1b[2;6H").unwrap();

        // ESC E — next line
        adapter.process(id(1), b"\x1bE").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 2);
        assert_eq!(cursor.col, 0);
    }

    #[test]
    fn esc_nel_at_scroll_bottom_scrolls_up() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 3)).unwrap();

        adapter.process(id(1), b"\x1b[1;1HA").unwrap();
        adapter.process(id(1), b"\x1b[2;1HB").unwrap();
        adapter.process(id(1), b"\x1b[3;3HC").unwrap(); // row 2, col 2

        // ESC E at scroll_bottom (row 2)
        adapter.process(id(1), b"\x1bE").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'B' (shifted up)
        assert_eq!(cells[0][0].ch, 'B');
        // Cursor should be at (2, 0)
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 2);
        assert_eq!(cursor.col, 0);
    }

    // =========================================================================
    // Tests: get_cursor_visible (ScreenPort trait) — Task #19
    // =========================================================================

    #[test]
    fn get_cursor_visible_returns_true_by_default() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        assert!(adapter.get_cursor_visible(id(1)).unwrap());
    }

    #[test]
    fn get_cursor_visible_returns_false_after_hide() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"\x1b[?25l").unwrap();
        assert!(!adapter.get_cursor_visible(id(1)).unwrap());
    }

    #[test]
    fn get_cursor_visible_nonexistent_returns_error() {
        let adapter = VteScreenAdapter::new();

        let result = adapter.get_cursor_visible(id(99));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ScreenNotFound(_)));
    }

    // =========================================================================
    // Tests: Alternate screen scroll region save/restore — Task #19 QA fix
    // =========================================================================

    #[test]
    fn alternate_screen_saves_and_restores_scroll_region() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 6)).unwrap();

        // Set scroll region to rows 2-5 (1-indexed) = rows 1-4 (0-indexed)
        adapter.process(id(1), b"\x1b[2;5r").unwrap();

        // Verify scroll region is set
        let screen = adapter.screens.get(&id(1)).unwrap();
        assert_eq!(screen.scroll_top, 1);
        assert_eq!(screen.scroll_bottom, 4);

        // Enter alternate screen
        adapter.process(id(1), b"\x1b[?1049h").unwrap();

        // Scroll region should be reset in alternate screen
        let screen = adapter.screens.get(&id(1)).unwrap();
        assert_eq!(screen.scroll_top, 0);
        assert_eq!(screen.scroll_bottom, 5);

        // Leave alternate screen
        adapter.process(id(1), b"\x1b[?1049l").unwrap();

        // Scroll region should be restored
        let screen = adapter.screens.get(&id(1)).unwrap();
        assert_eq!(screen.scroll_top, 1);
        assert_eq!(screen.scroll_bottom, 4);
    }

    #[test]
    fn alternate_screen_scroll_region_reset_allows_full_screen_scroll() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), TerminalSize::new(5, 5)).unwrap();

        // Set restricted scroll region on primary
        adapter.process(id(1), b"\x1b[2;4r").unwrap();

        // Enter alternate screen
        adapter.process(id(1), b"\x1b[?1049h").unwrap();

        // Write content on all rows
        for r in 0..5u16 {
            let ch = (b'A' + r as u8) as char;
            adapter
                .process(id(1), format!("\x1b[{};1H{}", r + 1, ch).as_bytes())
                .unwrap();
        }

        // Move to last row and LF — should scroll full screen (not restricted region)
        adapter.process(id(1), b"\x1b[5;1H").unwrap();
        adapter.process(id(1), b"\n").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0 should have 'B' (full screen scroll)
        assert_eq!(cells[0][0].ch, 'B');
        // Row 4 should be blank
        assert_eq!(cells[4][0].ch, ' ');
    }

    // =========================================================================
    // Tests: Wide character (CJK / emoji) support
    // =========================================================================

    #[test]
    fn wide_char_consumes_two_cells() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Print 'あ' (U+3042, hiragana, width 2)
        adapter
            .process(id(1), "あ".as_bytes())
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'あ');
        assert_eq!(cells[0][0].width, 2);
        assert_eq!(cells[0][1].width, 0);
        assert_eq!(cells[0][1].ch, ' ');

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.col, 2);
    }

    #[test]
    fn wide_char_continuation_cell_has_width_zero() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter
            .process(id(1), "漢".as_bytes())
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Right half (continuation) must have width 0
        assert_eq!(cells[0][1].width, 0);
        assert_eq!(cells[0][1].ch, ' ');
    }

    #[test]
    fn mixed_narrow_and_wide_chars() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // "aあb" -> 'a'(w1) + 'あ'(w2) + placeholder(w0) + 'b'(w1) = 4 columns
        adapter
            .process(id(1), "aあb".as_bytes())
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'a');
        assert_eq!(cells[0][0].width, 1);
        assert_eq!(cells[0][1].ch, 'あ');
        assert_eq!(cells[0][1].width, 2);
        assert_eq!(cells[0][2].ch, ' ');
        assert_eq!(cells[0][2].width, 0);
        assert_eq!(cells[0][3].ch, 'b');
        assert_eq!(cells[0][3].width, 1);

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.col, 4);
    }

    #[test]
    fn wide_char_wraps_when_at_last_column() {
        let mut adapter = VteScreenAdapter::new();
        // Width 5, so col 4 is the last column
        let size = TerminalSize::new(5, 3);
        adapter.create(id(1), size).unwrap();

        // Fill 4 columns with 'abcd', then try wide char at col 4
        adapter
            .process(id(1), "abcdあ".as_bytes())
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // Row 0: 'a','b','c','d',' ' (last col padded with space)
        assert_eq!(cells[0][0].ch, 'a');
        assert_eq!(cells[0][1].ch, 'b');
        assert_eq!(cells[0][2].ch, 'c');
        assert_eq!(cells[0][3].ch, 'd');
        assert_eq!(cells[0][4].ch, ' '); // padding

        // Row 1: 'あ' + continuation
        assert_eq!(cells[1][0].ch, 'あ');
        assert_eq!(cells[1][0].width, 2);
        assert_eq!(cells[1][1].ch, ' ');
        assert_eq!(cells[1][1].width, 0);

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 1);
        assert_eq!(cursor.col, 2);
    }

    #[test]
    fn normal_char_has_width_one_in_cell() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        adapter.process(id(1), b"A").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[0][0].width, 1);
    }

    #[test]
    fn multiple_wide_chars_in_sequence() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // "あい" -> each is 2 columns = 4 columns total
        adapter
            .process(id(1), "あい".as_bytes())
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'あ');
        assert_eq!(cells[0][0].width, 2);
        assert_eq!(cells[0][1].width, 0);
        assert_eq!(cells[0][2].ch, 'い');
        assert_eq!(cells[0][2].width, 2);
        assert_eq!(cells[0][3].width, 0);

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.col, 4);
    }

    #[test]
    fn wide_char_at_exact_line_end_wraps_correctly() {
        let mut adapter = VteScreenAdapter::new();
        // Width 4 means cols 0-3
        let size = TerminalSize::new(4, 3);
        adapter.create(id(1), size).unwrap();

        // "ab" fills cols 0,1. Wide char at col 2 fits (takes cols 2-3)
        adapter
            .process(id(1), "abあ".as_bytes())
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'a');
        assert_eq!(cells[0][1].ch, 'b');
        assert_eq!(cells[0][2].ch, 'あ');
        assert_eq!(cells[0][2].width, 2);
        assert_eq!(cells[0][3].width, 0);

        // After filling row 0 completely, cursor wraps to next row
        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 1);
        assert_eq!(cursor.col, 0);
    }

    #[test]
    fn cell_default_in_grid_has_width_one() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        // All default cells should have width 1
        for row in cells {
            for cell in row {
                assert_eq!(cell.width, 1);
            }
        }
    }

    // =========================================================================
    // Tests: get_application_cursor_keys (ScreenPort trait) — Task #23
    // =========================================================================

    #[test]
    fn get_application_cursor_keys_returns_false_by_default() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        assert!(!adapter.get_application_cursor_keys(id(1)).unwrap());
    }

    #[test]
    fn get_application_cursor_keys_returns_true_after_decckm_set() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // CSI ?1h enables application cursor keys (DECCKM set)
        adapter.process(id(1), b"\x1b[?1h").unwrap();
        assert!(adapter.get_application_cursor_keys(id(1)).unwrap());
    }

    #[test]
    fn get_application_cursor_keys_returns_false_after_decckm_reset() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Enable then disable
        adapter.process(id(1), b"\x1b[?1h").unwrap();
        adapter.process(id(1), b"\x1b[?1l").unwrap();
        assert!(!adapter.get_application_cursor_keys(id(1)).unwrap());
    }

    #[test]
    fn get_application_cursor_keys_nonexistent_returns_error() {
        let adapter = VteScreenAdapter::new();

        let result = adapter.get_application_cursor_keys(id(99));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ScreenNotFound(_)));
    }

    // =========================================================================
    // Tests: get_bracketed_paste (ScreenPort trait) — Task #23
    // =========================================================================

    #[test]
    fn get_bracketed_paste_returns_false_by_default() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        assert!(!adapter.get_bracketed_paste(id(1)).unwrap());
    }

    #[test]
    fn get_bracketed_paste_returns_true_after_enable() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // CSI ?2004h enables bracketed paste mode
        adapter.process(id(1), b"\x1b[?2004h").unwrap();
        assert!(adapter.get_bracketed_paste(id(1)).unwrap());
    }

    #[test]
    fn get_bracketed_paste_returns_false_after_disable() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), small_size()).unwrap();

        // Enable then disable
        adapter.process(id(1), b"\x1b[?2004h").unwrap();
        adapter.process(id(1), b"\x1b[?2004l").unwrap();
        assert!(!adapter.get_bracketed_paste(id(1)).unwrap());
    }

    #[test]
    fn get_bracketed_paste_nonexistent_returns_error() {
        let adapter = VteScreenAdapter::new();

        let result = adapter.get_bracketed_paste(id(99));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ScreenNotFound(_)));
    }

    // =========================================================================
    // Tests: get_cwd + OSC 7
    // =========================================================================

    #[test]
    fn cwd_none_by_default() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        assert_eq!(adapter.get_cwd(id(1)).unwrap(), None);
    }

    #[test]
    fn cwd_set_by_osc7() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // OSC 7 with ESC \ terminator
        adapter
            .process(id(1), b"\x1b]7;file://hostname/new/path\x1b\\")
            .unwrap();
        assert_eq!(
            adapter.get_cwd(id(1)).unwrap(),
            Some("/new/path".to_string())
        );
    }

    #[test]
    fn cwd_updated_on_second_osc7() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter
            .process(id(1), b"\x1b]7;file://host/first/path\x1b\\")
            .unwrap();
        adapter
            .process(id(1), b"\x1b]7;file://host/second/path\x1b\\")
            .unwrap();
        assert_eq!(
            adapter.get_cwd(id(1)).unwrap(),
            Some("/second/path".to_string())
        );
    }

    #[test]
    fn cwd_nonexistent_returns_error() {
        let adapter = VteScreenAdapter::new();
        let result = adapter.get_cwd(id(99));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ScreenNotFound(_)));
    }

    #[test]
    fn cwd_bell_terminated() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // OSC 7 with BEL terminator
        adapter
            .process(id(1), b"\x1b]7;file://host/bell/path\x07")
            .unwrap();
        assert_eq!(
            adapter.get_cwd(id(1)).unwrap(),
            Some("/bell/path".to_string())
        );
    }

    #[test]
    fn cwd_preserved_after_other_output() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter
            .process(id(1), b"\x1b]7;file://host/my/path\x1b\\")
            .unwrap();
        adapter.process(id(1), b"Hello world\r\n").unwrap();
        assert_eq!(
            adapter.get_cwd(id(1)).unwrap(),
            Some("/my/path".to_string())
        );
    }

    #[test]
    fn cwd_independent_per_terminal() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.create(id(2), default_size()).unwrap();
        adapter
            .process(id(1), b"\x1b]7;file://host/path1\x1b\\")
            .unwrap();
        adapter
            .process(id(2), b"\x1b]7;file://host/path2\x1b\\")
            .unwrap();
        assert_eq!(
            adapter.get_cwd(id(1)).unwrap(),
            Some("/path1".to_string())
        );
        assert_eq!(
            adapter.get_cwd(id(2)).unwrap(),
            Some("/path2".to_string())
        );
    }

    // =========================================================================
    // Tests: BEL / OSC 9 / OSC 777 notification detection
    // =========================================================================

    #[test]
    fn bel_detected_as_bell_notification() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x07").unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(notifications, vec![NotificationEvent::Bell]);
    }

    #[test]
    fn two_consecutive_bels_queued() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x07\x07").unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(
            notifications,
            vec![NotificationEvent::Bell, NotificationEvent::Bell]
        );
    }

    #[test]
    fn osc9_notification_detected() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // OSC 9 ; Task done ST
        adapter
            .process(id(1), b"\x1b]9;Task done\x1b\\")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(
            notifications,
            vec![NotificationEvent::Osc9 {
                message: "Task done".to_string()
            }]
        );
    }

    #[test]
    fn osc777_notification_detected() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // OSC 777 ; notify ; Build ; Success ST
        adapter
            .process(id(1), b"\x1b]777;notify;Build;Success\x1b\\")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(
            notifications,
            vec![NotificationEvent::Osc777 {
                title: "Build".to_string(),
                body: "Success".to_string(),
            }]
        );
    }

    #[test]
    fn drain_notifications_clears_queue() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x07").unwrap();

        let first_drain = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(first_drain, vec![NotificationEvent::Bell]);

        let second_drain = adapter.drain_notifications(id(1)).unwrap();
        assert!(second_drain.is_empty());
    }

    #[test]
    fn osc9_without_message_is_ignored() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // OSC 9 with no message parameter (only the command number)
        adapter.process(id(1), b"\x1b]9\x1b\\").unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert!(notifications.is_empty());
    }

    #[test]
    fn osc777_with_insufficient_params_is_ignored() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        // Only 2 params (missing title and body): 777;notify
        adapter
            .process(id(1), b"\x1b]777;notify\x1b\\")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert!(
            notifications.is_empty(),
            "OSC 777 with only 2 params should be ignored"
        );

        // params[1] is not "notify"
        adapter
            .process(id(1), b"\x1b]777;other;Title;Body\x1b\\")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert!(
            notifications.is_empty(),
            "OSC 777 with non-notify second param should be ignored"
        );
    }

    // ─── Scrollback stub tests ───

    #[test]
    fn scrollback_offset_always_zero() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        assert_eq!(adapter.get_scrollback_offset(id(1)).unwrap(), 0);
    }

    #[test]
    fn scrollback_max_always_zero() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        assert_eq!(adapter.get_max_scrollback(id(1)).unwrap(), 0);
    }

    #[test]
    fn set_scrollback_offset_noop() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // Should not error, just no-op
        adapter.set_scrollback_offset(id(1), 42).unwrap();
        assert_eq!(adapter.get_scrollback_offset(id(1)).unwrap(), 0);
    }

    #[test]
    fn is_alternate_screen_reflects_state() {
        let mut adapter = VteScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        assert!(!adapter.is_alternate_screen(id(1)).unwrap());

        // Switch to alternate screen
        adapter.process(id(1), b"\x1b[?1049h").unwrap();
        assert!(adapter.is_alternate_screen(id(1)).unwrap());

        // Switch back
        adapter.process(id(1), b"\x1b[?1049l").unwrap();
        assert!(!adapter.is_alternate_screen(id(1)).unwrap());
    }
}
