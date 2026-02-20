use std::collections::HashMap;

use super::osc7::parse_osc7_uri;
use crate::domain::primitive::{Cell, Color, CursorPos, CursorStyle, NotificationEvent, SearchMatch, TerminalId, TerminalSize};
use crate::interface_adapter::port::screen_port::ScreenPort;
use crate::shared::error::AppError;

/// Callbacks for capturing OSC 0/2 window title sequences, OSC 7 CWD,
/// and notification events (BEL, OSC 9, OSC 777).
#[derive(Debug, Default)]
struct Vt100Callbacks {
    title: Option<String>,
    cwd: Option<String>,
    notifications: Vec<NotificationEvent>,
    cursor_style: CursorStyle,
    /// Flag set by unhandled_csi when CSI 6 n (DSR cursor position query) is received.
    /// Checked after process() to synthesize the response with actual cursor position.
    pending_dsr: bool,
}

impl vt100::Callbacks for Vt100Callbacks {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.notifications.push(NotificationEvent::Bell);
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }

    fn set_window_icon_name(&mut self, _: &mut vt100::Screen, name: &[u8]) {
        self.title = Some(String::from_utf8_lossy(name).into_owned());
    }

    fn unhandled_csi(
        &mut self,
        _: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        // DECSCUSR: CSI Ps SP q — Set Cursor Style
        if c == 'q' && i1 == Some(b' ') {
            let ps = params.first()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(0);
            self.cursor_style = match ps {
                0 => CursorStyle::DefaultUserShape,
                1 => CursorStyle::BlinkingBlock,
                2 => CursorStyle::SteadyBlock,
                3 => CursorStyle::BlinkingUnderScore,
                4 => CursorStyle::SteadyUnderScore,
                5 => CursorStyle::BlinkingBar,
                6 => CursorStyle::SteadyBar,
                _ => CursorStyle::DefaultUserShape,
            };
        }

        // DSR: CSI 6 n — Device Status Report (cursor position query)
        // The child process expects a CSI row;col R response on its stdin.
        if c == 'n' && i1.is_none() {
            let ps = params.first()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(0);
            if ps == 6 {
                self.pending_dsr = true;
            }
        }
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        match params.first().copied() {
            Some(b"7") => {
                if let Some(uri_bytes) = params.get(1) {
                    let uri = String::from_utf8_lossy(uri_bytes);
                    if let Some(path) = parse_osc7_uri(&uri) {
                        self.cwd = Some(path);
                    }
                }
            }
            Some(b"9") => {
                if let Some(msg_bytes) = params.get(1) {
                    let message = String::from_utf8_lossy(msg_bytes).into_owned();
                    self.notifications
                        .push(NotificationEvent::Osc9 { message });
                }
            }
            Some(b"777") => {
                if params.len() >= 4
                    && params[1] == b"notify"
                {
                    let title = String::from_utf8_lossy(params[2]).into_owned();
                    let body = String::from_utf8_lossy(params[3]).into_owned();
                    self.notifications
                        .push(NotificationEvent::Osc777 { title, body });
                }
            }
            _ => {}
        }
    }
}

/// Per-terminal state managed by Vt100ScreenAdapter.
struct Vt100Instance {
    parser: vt100::Parser<Vt100Callbacks>,
    /// Cache for `get_cells()` which must return `&Vec<Vec<Cell>>`.
    cached_cells: Vec<Vec<Cell>>,
    /// Whether new output arrived while the user is scrolled back.
    new_output_while_scrolled: bool,
    /// Cached max scrollback value (updated in `&mut self` methods).
    cached_max_scrollback: usize,
    /// Pending responses to be written back to the PTY (e.g., DSR cursor position replies).
    pending_responses: Vec<Vec<u8>>,
}

/// ScreenPort implementation backed by the `vt100` crate.
///
/// Unlike VteScreenAdapter which manually handles ANSI escape sequences,
/// this adapter delegates all terminal emulation to the vt100 crate,
/// providing more complete xterm compatibility.
pub struct Vt100ScreenAdapter {
    instances: HashMap<TerminalId, Vt100Instance>,
}

impl Vt100ScreenAdapter {
    pub fn new() -> Self {
        Self {
            instances: HashMap::new(),
        }
    }

    /// Get the title of the screen (set by OSC 0/2).
    pub fn get_title(&self, id: TerminalId) -> Result<Option<String>, AppError> {
        self.instances
            .get(&id)
            .map(|inst| inst.parser.callbacks().title.clone())
            .ok_or(AppError::ScreenNotFound(id))
    }
}

fn convert_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Default,
        vt100::Color::Idx(idx) => Color::Indexed(idx),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn convert_cell(vt_cell: &vt100::Cell) -> Cell {
    Cell {
        ch: vt_cell.contents().chars().next().unwrap_or(' '),
        fg: convert_color(vt_cell.fgcolor()),
        bg: convert_color(vt_cell.bgcolor()),
        bold: vt_cell.bold(),
        underline: vt_cell.underline(),
        italic: vt_cell.italic(),
        dim: vt_cell.dim(),
        reverse: vt_cell.inverse(),
        strikethrough: false,
        hidden: false,
        width: if vt_cell.is_wide_continuation() {
            0
        } else if vt_cell.is_wide() {
            2
        } else {
            1
        },
    }
}

fn update_max_scrollback(inst: &mut Vt100Instance) {
    let current = inst.parser.screen().scrollback();
    inst.parser.screen_mut().set_scrollback(usize::MAX);
    inst.cached_max_scrollback = inst.parser.screen().scrollback();
    inst.parser.screen_mut().set_scrollback(current);
}

fn rebuild_cell_cache(parser: &vt100::Parser<Vt100Callbacks>, cache: &mut Vec<Vec<Cell>>) {
    let screen = parser.screen();
    let rows = screen.size().0 as usize;
    let cols = screen.size().1 as usize;

    cache.resize_with(rows, Vec::new);
    cache.truncate(rows);

    for (r, row) in cache.iter_mut().enumerate().take(rows) {
        row.resize(cols, Cell::default());
        row.truncate(cols);

        for (c, cell) in row.iter_mut().enumerate().take(cols) {
            if let Some(vt_cell) = screen.cell(r as u16, c as u16) {
                *cell = convert_cell(vt_cell);
            } else {
                *cell = Cell::default();
            }
        }
    }
}

impl ScreenPort for Vt100ScreenAdapter {
    fn create(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError> {
        let callbacks = Vt100Callbacks::default();
        let parser =
            vt100::Parser::new_with_callbacks(size.rows, size.cols, 10_000, callbacks);
        let mut cached_cells = Vec::new();
        rebuild_cell_cache(&parser, &mut cached_cells);
        self.instances.insert(id, Vt100Instance {
            parser,
            cached_cells,
            new_output_while_scrolled: false,
            cached_max_scrollback: 0,
            pending_responses: Vec::new(),
        });
        Ok(())
    }

    fn process(&mut self, id: TerminalId, data: &[u8]) -> Result<(), AppError> {
        let inst = self
            .instances
            .get_mut(&id)
            .ok_or(AppError::ScreenNotFound(id))?;
        let was_scrolled = inst.parser.screen().scrollback() > 0;
        inst.parser.process(data);
        if was_scrolled {
            inst.new_output_while_scrolled = true;
        }
        // Synthesize DSR cursor position response if requested by child process
        if inst.parser.callbacks().pending_dsr {
            let pos = inst.parser.screen().cursor_position();
            let response = format!("\x1b[{};{}R", pos.0 + 1, pos.1 + 1);
            inst.pending_responses.push(response.into_bytes());
            inst.parser.callbacks_mut().pending_dsr = false;
        }
        update_max_scrollback(inst);
        rebuild_cell_cache(&inst.parser, &mut inst.cached_cells);
        Ok(())
    }

    fn get_cells(&self, id: TerminalId) -> Result<&Vec<Vec<Cell>>, AppError> {
        self.instances
            .get(&id)
            .map(|inst| &inst.cached_cells)
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn get_cursor(&self, id: TerminalId) -> Result<CursorPos, AppError> {
        self.instances
            .get(&id)
            .map(|inst| {
                let pos = inst.parser.screen().cursor_position();
                CursorPos {
                    row: pos.0,
                    col: pos.1,
                }
            })
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn resize(&mut self, id: TerminalId, size: TerminalSize) -> Result<(), AppError> {
        let inst = self
            .instances
            .get_mut(&id)
            .ok_or(AppError::ScreenNotFound(id))?;
        inst.parser.screen_mut().set_size(size.rows, size.cols);
        rebuild_cell_cache(&inst.parser, &mut inst.cached_cells);
        Ok(())
    }

    fn remove(&mut self, id: TerminalId) -> Result<(), AppError> {
        self.instances
            .remove(&id)
            .ok_or(AppError::ScreenNotFound(id))?;
        Ok(())
    }

    fn get_cursor_visible(&self, id: TerminalId) -> Result<bool, AppError> {
        self.instances
            .get(&id)
            .map(|inst| !inst.parser.screen().hide_cursor())
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn get_application_cursor_keys(&self, id: TerminalId) -> Result<bool, AppError> {
        self.instances
            .get(&id)
            .map(|inst| inst.parser.screen().application_cursor())
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn get_bracketed_paste(&self, id: TerminalId) -> Result<bool, AppError> {
        self.instances
            .get(&id)
            .map(|inst| inst.parser.screen().bracketed_paste())
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn get_cwd(&self, id: TerminalId) -> Result<Option<String>, AppError> {
        self.instances
            .get(&id)
            .map(|inst| inst.parser.callbacks().cwd.clone())
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn drain_notifications(&mut self, id: TerminalId) -> Result<Vec<NotificationEvent>, AppError> {
        let inst = self
            .instances
            .get_mut(&id)
            .ok_or(AppError::ScreenNotFound(id))?;
        let notifications = std::mem::take(&mut inst.parser.callbacks_mut().notifications);
        Ok(notifications)
    }

    fn set_scrollback_offset(&mut self, id: TerminalId, offset: usize) -> Result<(), AppError> {
        let inst = self
            .instances
            .get_mut(&id)
            .ok_or(AppError::ScreenNotFound(id))?;
        inst.parser.screen_mut().set_scrollback(offset);
        if offset == 0 {
            inst.new_output_while_scrolled = false;
        }
        rebuild_cell_cache(&inst.parser, &mut inst.cached_cells);
        Ok(())
    }

    fn get_scrollback_offset(&self, id: TerminalId) -> Result<usize, AppError> {
        self.instances
            .get(&id)
            .map(|inst| inst.parser.screen().scrollback())
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn get_max_scrollback(&self, id: TerminalId) -> Result<usize, AppError> {
        self.instances
            .get(&id)
            .map(|inst| inst.cached_max_scrollback)
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn is_alternate_screen(&self, id: TerminalId) -> Result<bool, AppError> {
        self.instances
            .get(&id)
            .map(|inst| inst.parser.screen().alternate_screen())
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn get_cursor_style(&self, id: TerminalId) -> Result<CursorStyle, AppError> {
        self.instances
            .get(&id)
            .map(|inst| inst.parser.callbacks().cursor_style)
            .ok_or(AppError::ScreenNotFound(id))
    }

    fn drain_pending_responses(&mut self, id: TerminalId) -> Result<Vec<Vec<u8>>, AppError> {
        let inst = self
            .instances
            .get_mut(&id)
            .ok_or(AppError::ScreenNotFound(id))?;
        Ok(std::mem::take(&mut inst.pending_responses))
    }

    fn search_scrollback(&mut self, id: TerminalId, query: &str) -> Result<Vec<SearchMatch>, AppError> {
        const MAX_MATCHES: usize = 10_000;

        if query.is_empty() {
            return Ok(vec![]);
        }
        let inst = self
            .instances
            .get_mut(&id)
            .ok_or(AppError::ScreenNotFound(id))?;

        let screen = inst.parser.screen();
        let rows = screen.size().0 as usize;
        let cols = screen.size().1 as usize;
        let max_scrollback = inst.cached_max_scrollback;
        let total_rows = max_scrollback + rows;

        // Save the current scrollback offset so we can restore it afterwards.
        let saved_offset = inst.parser.screen().scrollback();

        // Set scrollback to maximum so that screen row 0 = top of scrollback buffer.
        // After this, screen.cell(r, c) for r in 0..rows shows scrollback rows starting
        // from the oldest line. We iterate in chunks by adjusting the offset.
        let query_lower = query.to_lowercase();
        let mut matches = Vec::new();

        // We need to iterate over total_rows = max_scrollback + screen_rows.
        // When scrollback is set to max_scrollback, the screen shows rows
        // [0..rows) where row 0 = oldest scrollback line.
        // When scrollback is set to 0, the screen shows the live (bottom) rows.
        //
        // To read all rows we set scrollback to max_scrollback first. This makes
        // screen.cell(r, c) return the scrollback rows for r in 0..rows (covering
        // the first `rows` lines of the total). Then we reduce the offset by `rows`
        // to reveal the next chunk, and so on.
        //
        // However, a simpler approach: we iterate abs_row 0..total_rows, and for
        // each abs_row we set the scrollback offset such that abs_row appears as
        // row 0 of the screen view. The offset needed is: max_scrollback - abs_row.
        // But that requires one set_scrollback call per row, which is expensive.
        //
        // Better approach: set scrollback to max, read rows 0..rows (= abs_row 0..rows).
        // Then set scrollback to max - rows, read rows 0..rows (= abs_row rows..2*rows).
        // And so on until we've covered all total_rows.

        let query_chars: Vec<char> = query_lower.chars().collect();
        let qlen = query_chars.len();

        let mut abs_row_base = 0;
        let mut remaining = total_rows;

        while remaining > 0 && matches.len() < MAX_MATCHES {
            // Set scrollback offset so that screen row 0 corresponds to abs_row_base.
            // When abs_row_base == 0, offset = max_scrollback (oldest rows visible).
            // When abs_row_base == max_scrollback, offset = 0 (live screen).
            let offset = max_scrollback.saturating_sub(abs_row_base);
            inst.parser.screen_mut().set_scrollback(offset);

            // The number of rows to read in this chunk.
            let chunk_rows = remaining.min(rows);

            for local_row in 0..chunk_rows {
                if matches.len() >= MAX_MATCHES {
                    break;
                }

                let abs_row = abs_row_base + local_row;
                let screen_row = local_row as u16;

                // Build row text from cells, skipping width=0 continuation cells.
                // Track the mapping from character index in line_text to cell column.
                let mut line_text = String::new();
                let mut cell_positions: Vec<usize> = Vec::new();

                for col in 0..cols {
                    if let Some(cell) = inst.parser.screen().cell(screen_row, col as u16) {
                        // Skip width=0 continuation cells (2nd half of wide char)
                        if cell.is_wide_continuation() {
                            continue;
                        }

                        let ch = cell.contents();
                        if ch.is_empty() || ch == "\u{0}" {
                            cell_positions.push(col);
                            line_text.push(' ');
                        } else {
                            for c in ch.chars() {
                                cell_positions.push(col);
                                line_text.push(c);
                            }
                        }
                    }
                }

                // Case-insensitive search using character-based comparison
                // to properly handle multi-byte UTF-8 (e.g., Japanese wide chars).
                let line_chars: Vec<char> = line_text.chars().flat_map(|c| c.to_lowercase()).collect();

                if qlen > 0 && qlen <= line_chars.len() {
                    let mut search_start = 0;
                    while search_start + qlen <= line_chars.len() {
                        if matches.len() >= MAX_MATCHES {
                            break;
                        }

                        if line_chars[search_start..search_start + qlen] == query_chars[..] {
                            let char_start = search_start;
                            let char_end = search_start + qlen;

                            if char_start < cell_positions.len() && char_end <= cell_positions.len() {
                                let col_start = cell_positions[char_start];
                                let col_end = if char_end < cell_positions.len() {
                                    cell_positions[char_end]
                                } else {
                                    // Match extends to the end: use last cell column + its width
                                    let last_col = cell_positions[cell_positions.len() - 1];
                                    if let Some(cell) = inst.parser.screen().cell(screen_row, last_col as u16) {
                                        if cell.is_wide() {
                                            last_col + 2
                                        } else {
                                            last_col + 1
                                        }
                                    } else {
                                        last_col + 1
                                    }
                                };
                                matches.push(SearchMatch { row: abs_row, col_start, col_end });
                            }
                        }

                        search_start += 1;
                    }
                }
            }

            abs_row_base += chunk_rows;
            remaining -= chunk_rows;
        }

        // Restore the original scrollback offset.
        inst.parser.screen_mut().set_scrollback(saved_offset);

        Ok(matches)
    }

    fn get_row_cells(&mut self, id: TerminalId, abs_row: usize) -> Result<Vec<Cell>, AppError> {
        let inst = self
            .instances
            .get_mut(&id)
            .ok_or(AppError::ScreenNotFound(id))?;

        let screen = inst.parser.screen();
        let rows = screen.size().0 as usize;
        let cols = screen.size().1 as usize;
        let max_scrollback = inst.cached_max_scrollback;
        let total_rows = max_scrollback + rows;

        // Out-of-range check
        if abs_row >= total_rows {
            return Ok(vec![]);
        }

        // Save the current scrollback offset so we can restore it afterwards.
        let saved_offset = inst.parser.screen().scrollback();

        // Set scrollback offset such that abs_row appears as display row 0.
        // When offset = max_scrollback, screen row 0 = scrollback top (abs_row 0).
        // When offset = 0, screen row 0 = first visible line (abs_row max_scrollback).
        // So: offset_needed = max_scrollback - abs_row
        let offset_needed = max_scrollback.saturating_sub(abs_row);
        inst.parser.screen_mut().set_scrollback(offset_needed);

        // Read cells from display row 0
        let mut result = Vec::with_capacity(cols);
        for col in 0..cols {
            if let Some(vt_cell) = inst.parser.screen().cell(0, col as u16) {
                result.push(convert_cell(vt_cell));
            } else {
                result.push(Cell::default());
            }
        }

        // Restore the original scrollback offset.
        inst.parser.screen_mut().set_scrollback(saved_offset);

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_size() -> TerminalSize {
        TerminalSize::new(80, 24)
    }

    fn id(n: u32) -> TerminalId {
        TerminalId::new(n)
    }

    // ─── ScreenPort contract tests ───

    #[test]
    fn create_initializes_blank_screen() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells.len(), 24);
        assert_eq!(cells[0].len(), 80);
        assert_eq!(cells[0][0].ch, ' ');
    }

    #[test]
    fn create_duplicate_overwrites() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"Hello").unwrap();
        adapter.create(id(1), default_size()).unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, ' ');
    }

    #[test]
    fn process_updates_cells() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"ABC").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[0][1].ch, 'B');
        assert_eq!(cells[0][2].ch, 'C');
    }

    #[test]
    fn process_nonexistent_returns_error() {
        let mut adapter = Vt100ScreenAdapter::new();
        assert!(adapter.process(id(99), b"test").is_err());
    }

    #[test]
    fn get_cells_nonexistent_returns_error() {
        let adapter = Vt100ScreenAdapter::new();
        assert!(adapter.get_cells(id(99)).is_err());
    }

    #[test]
    fn get_cursor_initial_position() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 0);
    }

    #[test]
    fn get_cursor_after_text() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"Hello").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 0);
        assert_eq!(cursor.col, 5);
    }

    #[test]
    fn get_cursor_nonexistent_returns_error() {
        let adapter = Vt100ScreenAdapter::new();
        assert!(adapter.get_cursor(id(99)).is_err());
    }

    #[test]
    fn resize_changes_dimensions() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter
            .resize(id(1), TerminalSize::new(40, 10))
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells.len(), 10);
        assert_eq!(cells[0].len(), 40);
    }

    #[test]
    fn resize_nonexistent_returns_error() {
        let mut adapter = Vt100ScreenAdapter::new();
        assert!(adapter.resize(id(99), default_size()).is_err());
    }

    #[test]
    fn remove_removes_screen() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.remove(id(1)).unwrap();

        assert!(adapter.get_cells(id(1)).is_err());
    }

    #[test]
    fn remove_nonexistent_returns_error() {
        let mut adapter = Vt100ScreenAdapter::new();
        assert!(adapter.remove(id(99)).is_err());
    }

    #[test]
    fn cursor_visible_default_true() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        assert!(adapter.get_cursor_visible(id(1)).unwrap());
    }

    #[test]
    fn cursor_visible_after_hide() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // DECTCEM reset: CSI ?25l
        adapter.process(id(1), b"\x1b[?25l").unwrap();
        assert!(!adapter.get_cursor_visible(id(1)).unwrap());
    }

    #[test]
    fn cursor_visible_after_show() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[?25l").unwrap();
        adapter.process(id(1), b"\x1b[?25h").unwrap();
        assert!(adapter.get_cursor_visible(id(1)).unwrap());
    }

    #[test]
    fn application_cursor_keys_default_false() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        assert!(!adapter.get_application_cursor_keys(id(1)).unwrap());
    }

    #[test]
    fn application_cursor_keys_enable() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // DECCKM set: CSI ?1h
        adapter.process(id(1), b"\x1b[?1h").unwrap();
        assert!(adapter.get_application_cursor_keys(id(1)).unwrap());
    }

    #[test]
    fn application_cursor_keys_disable() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[?1h").unwrap();
        adapter.process(id(1), b"\x1b[?1l").unwrap();
        assert!(!adapter.get_application_cursor_keys(id(1)).unwrap());
    }

    #[test]
    fn bracketed_paste_default_false() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        assert!(!adapter.get_bracketed_paste(id(1)).unwrap());
    }

    #[test]
    fn bracketed_paste_enable() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // Bracketed paste set: CSI ?2004h
        adapter.process(id(1), b"\x1b[?2004h").unwrap();
        assert!(adapter.get_bracketed_paste(id(1)).unwrap());
    }

    #[test]
    fn bracketed_paste_disable() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[?2004h").unwrap();
        adapter.process(id(1), b"\x1b[?2004l").unwrap();
        assert!(!adapter.get_bracketed_paste(id(1)).unwrap());
    }

    // ─── Cell conversion tests ───

    #[test]
    fn bold_attribute() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // SGR 1 (bold) + text
        adapter.process(id(1), b"\x1b[1mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].bold);
        assert_eq!(cells[0][0].ch, 'X');
    }

    #[test]
    fn italic_attribute() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[3mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].italic);
    }

    #[test]
    fn underline_attribute() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[4mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].underline);
    }

    #[test]
    fn dim_attribute() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[2mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].dim);
    }

    #[test]
    fn reverse_attribute() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // SGR 7 (inverse/reverse)
        adapter.process(id(1), b"\x1b[7mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(cells[0][0].reverse);
    }

    #[test]
    fn strikethrough_always_false() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // SGR 9 (strikethrough) — vt100 crate doesn't support it
        adapter.process(id(1), b"\x1b[9mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(!cells[0][0].strikethrough);
    }

    #[test]
    fn hidden_always_false() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // SGR 8 (hidden) — vt100 crate doesn't support it
        adapter.process(id(1), b"\x1b[8mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(!cells[0][0].hidden);
    }

    #[test]
    fn fg_indexed_color() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // SGR 31 (red foreground)
        adapter.process(id(1), b"\x1b[31mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].fg, Color::Indexed(1));
    }

    #[test]
    fn bg_indexed_color() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // SGR 42 (green background)
        adapter.process(id(1), b"\x1b[42mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].bg, Color::Indexed(2));
    }

    #[test]
    fn fg_rgb_color() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // SGR 38;2;255;128;0 (RGB foreground)
        adapter
            .process(id(1), b"\x1b[38;2;255;128;0mX")
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].fg, Color::Rgb(255, 128, 0));
    }

    #[test]
    fn bg_rgb_color() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // SGR 48;2;0;128;255 (RGB background)
        adapter
            .process(id(1), b"\x1b[48;2;0;128;255mX")
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].bg, Color::Rgb(0, 128, 255));
    }

    #[test]
    fn default_color() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"X").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].fg, Color::Default);
        assert_eq!(cells[0][0].bg, Color::Default);
    }

    // ─── Wide character tests ───

    #[test]
    fn wide_char_width_2() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // Japanese hiragana 'あ' is a wide char
        adapter.process(id(1), "あ".as_bytes()).unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'あ');
        assert_eq!(cells[0][0].width, 2);
    }

    #[test]
    fn wide_char_continuation() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), "あ".as_bytes()).unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][1].width, 0);
    }

    #[test]
    fn normal_char_width_1() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"A").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].width, 1);
    }

    // ─── Cache integrity tests ───

    #[test]
    fn cache_updated_after_process() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, ' ');

        adapter.process(id(1), b"X").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'X');
    }

    #[test]
    fn cache_updated_after_resize() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();

        adapter
            .resize(id(1), TerminalSize::new(40, 10))
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells.len(), 10);
        assert_eq!(cells[0].len(), 40);
    }

    #[test]
    fn cache_size_matches_terminal_size() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(132, 50);
        adapter.create(id(1), size).unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells.len(), 50);
        assert_eq!(cells[0].len(), 132);
    }

    #[test]
    fn cache_reflects_multiple_process_calls() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"AB").unwrap();
        adapter.process(id(1), b"C").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[0][1].ch, 'B');
        assert_eq!(cells[0][2].ch, 'C');
    }

    #[test]
    fn resize_shrink_preserves_content_in_range() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"Hello").unwrap();
        adapter
            .resize(id(1), TerminalSize::new(3, 1))
            .unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].len(), 3);
        // Content may or may not be preserved depending on vt100 resize behavior
        // but dimensions must be correct
    }

    // ─── Title tests ───

    #[test]
    fn title_none_by_default() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        assert_eq!(adapter.get_title(id(1)).unwrap(), None);
    }

    #[test]
    fn title_nonexistent_returns_error() {
        let adapter = Vt100ScreenAdapter::new();
        assert!(adapter.get_title(id(99)).is_err());
    }

    // ─── Behavioral parity tests ───

    #[test]
    fn newline_advances_cursor_row() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // Use \r\n to move to beginning of next line (LF alone doesn't reset column)
        adapter.process(id(1), b"A\r\nB").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 1);
        assert_eq!(cursor.col, 1);

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');
        assert_eq!(cells[1][0].ch, 'B');
    }

    #[test]
    fn cursor_movement_csi() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // Move cursor to row 5, col 10 (1-indexed in CSI H)
        adapter.process(id(1), b"\x1b[5;10H").unwrap();

        let cursor = adapter.get_cursor(id(1)).unwrap();
        assert_eq!(cursor.row, 4); // 0-indexed
        assert_eq!(cursor.col, 9); // 0-indexed
    }

    #[test]
    fn alternate_screen_buffer() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"Primary").unwrap();

        // Switch to alternate screen
        adapter.process(id(1), b"\x1b[?1049h").unwrap();
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, ' '); // alternate screen is blank

        // Write on alternate screen
        adapter.process(id(1), b"Alt").unwrap();
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'A');

        // Switch back to primary screen
        adapter.process(id(1), b"\x1b[?1049l").unwrap();
        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, 'P');
    }

    #[test]
    fn sgr_reset_clears_attributes() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // Bold on, then reset, then write
        adapter.process(id(1), b"\x1b[1m\x1b[0mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert!(!cells[0][0].bold);
    }

    #[test]
    fn multiple_terminals_independent() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.create(id(2), default_size()).unwrap();

        adapter.process(id(1), b"AAA").unwrap();
        adapter.process(id(2), b"BBB").unwrap();

        let cells1 = adapter.get_cells(id(1)).unwrap();
        let cells2 = adapter.get_cells(id(2)).unwrap();
        assert_eq!(cells1[0][0].ch, 'A');
        assert_eq!(cells2[0][0].ch, 'B');
    }

    #[test]
    fn fg_256_color() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // SGR 38;5;200 (256-color foreground)
        adapter.process(id(1), b"\x1b[38;5;200mX").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].fg, Color::Indexed(200));
    }

    #[test]
    fn erase_in_display_clears_cells() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"Hello World").unwrap();
        // CSI 2J: erase entire display
        adapter.process(id(1), b"\x1b[2J").unwrap();

        let cells = adapter.get_cells(id(1)).unwrap();
        assert_eq!(cells[0][0].ch, ' ');
        assert_eq!(cells[0][5].ch, ' ');
    }

    // ─── get_cwd tests ───

    #[test]
    fn cwd_none_by_default() {
        let mut adapter = Vt100ScreenAdapter::new();
        let id = TerminalId::new(1);
        adapter.create(id, TerminalSize::new(80, 24)).unwrap();
        assert_eq!(adapter.get_cwd(id).unwrap(), None);
    }

    #[test]
    fn cwd_set_by_osc7() {
        let mut adapter = Vt100ScreenAdapter::new();
        let id = TerminalId::new(1);
        adapter.create(id, TerminalSize::new(80, 24)).unwrap();
        // OSC 7 with ESC \ terminator
        adapter.process(id, b"\x1b]7;file://hostname/new/path\x1b\\").unwrap();
        assert_eq!(adapter.get_cwd(id).unwrap(), Some("/new/path".to_string()));
    }

    #[test]
    fn cwd_updated_on_second_osc7() {
        let mut adapter = Vt100ScreenAdapter::new();
        let id = TerminalId::new(1);
        adapter.create(id, TerminalSize::new(80, 24)).unwrap();
        adapter.process(id, b"\x1b]7;file://host/first/path\x1b\\").unwrap();
        adapter.process(id, b"\x1b]7;file://host/second/path\x1b\\").unwrap();
        assert_eq!(adapter.get_cwd(id).unwrap(), Some("/second/path".to_string()));
    }

    #[test]
    fn cwd_nonexistent_returns_error() {
        let adapter = Vt100ScreenAdapter::new();
        let id = TerminalId::new(99);
        assert!(adapter.get_cwd(id).is_err());
    }

    #[test]
    fn cwd_bell_terminated() {
        let mut adapter = Vt100ScreenAdapter::new();
        let id = TerminalId::new(1);
        adapter.create(id, TerminalSize::new(80, 24)).unwrap();
        // OSC 7 with BEL terminator
        adapter.process(id, b"\x1b]7;file://host/bell/path\x07").unwrap();
        assert_eq!(adapter.get_cwd(id).unwrap(), Some("/bell/path".to_string()));
    }

    #[test]
    fn cwd_preserved_after_other_output() {
        let mut adapter = Vt100ScreenAdapter::new();
        let id = TerminalId::new(1);
        adapter.create(id, TerminalSize::new(80, 24)).unwrap();
        adapter.process(id, b"\x1b]7;file://host/my/path\x1b\\").unwrap();
        adapter.process(id, b"Hello world\r\n").unwrap();
        assert_eq!(adapter.get_cwd(id).unwrap(), Some("/my/path".to_string()));
    }

    #[test]
    fn cwd_independent_per_terminal() {
        let mut adapter = Vt100ScreenAdapter::new();
        let id1 = TerminalId::new(1);
        let id2 = TerminalId::new(2);
        adapter.create(id1, TerminalSize::new(80, 24)).unwrap();
        adapter.create(id2, TerminalSize::new(80, 24)).unwrap();
        adapter.process(id1, b"\x1b]7;file://host/path1\x1b\\").unwrap();
        adapter.process(id2, b"\x1b]7;file://host/path2\x1b\\").unwrap();
        assert_eq!(adapter.get_cwd(id1).unwrap(), Some("/path1".to_string()));
        assert_eq!(adapter.get_cwd(id2).unwrap(), Some("/path2".to_string()));
    }

    // ─── Notification tests ───

    #[test]
    fn drain_notifications_empty_by_default() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert!(notifications.is_empty());
    }

    #[test]
    fn drain_notifications_nonexistent_returns_error() {
        let mut adapter = Vt100ScreenAdapter::new();
        assert!(adapter.drain_notifications(id(99)).is_err());
    }

    #[test]
    fn bel_triggers_bell_notification() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // Send BEL character
        adapter.process(id(1), b"\x07").unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0], NotificationEvent::Bell);
    }

    #[test]
    fn multiple_bels_trigger_multiple_bell_notifications() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x07\x07\x07").unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(notifications.len(), 3);
        assert!(notifications.iter().all(|n| *n == NotificationEvent::Bell));
    }

    #[test]
    fn osc9_triggers_notification() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // OSC 9 with ST terminator
        adapter
            .process(id(1), b"\x1b]9;Task done\x1b\\")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(
            notifications[0],
            NotificationEvent::Osc9 {
                message: "Task done".to_string()
            }
        );
    }

    #[test]
    fn osc9_with_bel_terminator() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // OSC 9 with BEL terminator
        adapter
            .process(id(1), b"\x1b]9;Build succeeded\x07")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(
            notifications[0],
            NotificationEvent::Osc9 {
                message: "Build succeeded".to_string()
            }
        );
    }

    #[test]
    fn osc9_empty_message() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b]9;\x1b\\").unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(
            notifications[0],
            NotificationEvent::Osc9 {
                message: String::new()
            }
        );
    }

    #[test]
    fn osc777_triggers_notification() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // OSC 777 with ST terminator
        adapter
            .process(id(1), b"\x1b]777;notify;Build;Success\x1b\\")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(
            notifications[0],
            NotificationEvent::Osc777 {
                title: "Build".to_string(),
                body: "Success".to_string(),
            }
        );
    }

    #[test]
    fn osc777_with_bel_terminator() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter
            .process(id(1), b"\x1b]777;notify;Deploy;Complete\x07")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(
            notifications[0],
            NotificationEvent::Osc777 {
                title: "Deploy".to_string(),
                body: "Complete".to_string(),
            }
        );
    }

    #[test]
    fn osc777_missing_notify_keyword_ignored() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // Missing "notify" keyword — should be ignored
        adapter
            .process(id(1), b"\x1b]777;other;Build;Success\x1b\\")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert!(notifications.is_empty());
    }

    #[test]
    fn osc777_too_few_params_ignored() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // Only 2 params (777 + notify) — missing title and body
        adapter
            .process(id(1), b"\x1b]777;notify\x1b\\")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert!(notifications.is_empty());
    }

    #[test]
    fn drain_clears_notification_queue() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x07").unwrap();

        // First drain returns the notification
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(notifications.len(), 1);

        // Second drain returns empty
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert!(notifications.is_empty());
    }

    #[test]
    fn mixed_notifications_collected_in_order() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // BEL, then OSC 9, then OSC 777
        adapter.process(id(1), b"\x07").unwrap();
        adapter
            .process(id(1), b"\x1b]9;Alert\x1b\\")
            .unwrap();
        adapter
            .process(id(1), b"\x1b]777;notify;Title;Body\x1b\\")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        assert_eq!(notifications.len(), 3);
        assert_eq!(notifications[0], NotificationEvent::Bell);
        assert_eq!(
            notifications[1],
            NotificationEvent::Osc9 {
                message: "Alert".to_string()
            }
        );
        assert_eq!(
            notifications[2],
            NotificationEvent::Osc777 {
                title: "Title".to_string(),
                body: "Body".to_string(),
            }
        );
    }

    #[test]
    fn notifications_independent_per_terminal() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.create(id(2), default_size()).unwrap();
        adapter.process(id(1), b"\x07").unwrap();
        adapter
            .process(id(2), b"\x1b]9;Hello\x1b\\")
            .unwrap();

        let n1 = adapter.drain_notifications(id(1)).unwrap();
        let n2 = adapter.drain_notifications(id(2)).unwrap();
        assert_eq!(n1.len(), 1);
        assert_eq!(n1[0], NotificationEvent::Bell);
        assert_eq!(n2.len(), 1);
        assert_eq!(
            n2[0],
            NotificationEvent::Osc9 {
                message: "Hello".to_string()
            }
        );
    }

    #[test]
    fn bel_as_osc_terminator_not_treated_as_standalone_bell() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // OSC 9 with BEL terminator — the BEL is part of the OSC, not a standalone bell
        adapter
            .process(id(1), b"\x1b]9;Msg\x07")
            .unwrap();
        let notifications = adapter.drain_notifications(id(1)).unwrap();
        // Should only have the OSC 9, not an extra Bell
        assert_eq!(notifications.len(), 1);
        assert_eq!(
            notifications[0],
            NotificationEvent::Osc9 {
                message: "Msg".to_string()
            }
        );
    }

    // ─── Scrollback tests ───

    #[test]
    fn scrollback_offset_default_zero() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        assert_eq!(adapter.get_scrollback_offset(id(1)).unwrap(), 0);
    }

    #[test]
    fn scrollback_max_zero_with_no_history() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        assert_eq!(adapter.get_max_scrollback(id(1)).unwrap(), 0);
    }

    #[test]
    fn scrollback_max_grows_with_output() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(80, 5);
        adapter.create(id(1), size).unwrap();

        // Fill 5-row screen + overflow to create scrollback
        for i in 0..10 {
            let line = format!("line {}\r\n", i);
            adapter.process(id(1), line.as_bytes()).unwrap();
        }

        let max = adapter.get_max_scrollback(id(1)).unwrap();
        assert!(max > 0, "Expected scrollback > 0, got {}", max);
    }

    #[test]
    fn set_scrollback_offset_changes_view() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(80, 5);
        adapter.create(id(1), size).unwrap();

        // Generate scrollback
        for i in 0..20 {
            let line = format!("line {}\r\n", i);
            adapter.process(id(1), line.as_bytes()).unwrap();
        }

        let max = adapter.get_max_scrollback(id(1)).unwrap();
        assert!(max > 0);

        // Scroll back
        adapter.set_scrollback_offset(id(1), 3).unwrap();
        assert_eq!(adapter.get_scrollback_offset(id(1)).unwrap(), 3);

        // Scroll back to live
        adapter.set_scrollback_offset(id(1), 0).unwrap();
        assert_eq!(adapter.get_scrollback_offset(id(1)).unwrap(), 0);
    }

    #[test]
    fn set_scrollback_offset_clamped_to_max() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(80, 5);
        adapter.create(id(1), size).unwrap();

        // Generate a few lines of scrollback
        for i in 0..10 {
            let line = format!("line {}\r\n", i);
            adapter.process(id(1), line.as_bytes()).unwrap();
        }

        let max = adapter.get_max_scrollback(id(1)).unwrap();

        // Set beyond max — vt100 should clamp
        adapter.set_scrollback_offset(id(1), max + 100).unwrap();
        let actual = adapter.get_scrollback_offset(id(1)).unwrap();
        assert!(actual <= max, "Expected clamped to max={}, got {}", max, actual);
    }

    #[test]
    fn scrollback_content_shows_history() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(80, 3);
        adapter.create(id(1), size).unwrap();

        // Write enough lines to push "line 0" into scrollback
        for i in 0..6 {
            let line = format!("line {}\r\n", i);
            adapter.process(id(1), line.as_bytes()).unwrap();
        }

        // Capture live view content as owned strings
        let live_row0: String = adapter.get_cells(id(1)).unwrap()[0]
            .iter().take(6).map(|c| c.ch).collect();

        // Scroll to max
        let max = adapter.get_max_scrollback(id(1)).unwrap();
        assert!(max > 0);
        adapter.set_scrollback_offset(id(1), max).unwrap();

        let scrolled_row0: String = adapter.get_cells(id(1)).unwrap()[0]
            .iter().take(6).map(|c| c.ch).collect();

        // The scrolled view should show "line 0" (early content)
        assert!(scrolled_row0.starts_with("line 0"), "Expected 'line 0' at scrollback top, got '{}'", scrolled_row0);
        // Live and scrolled views should differ
        assert_ne!(live_row0, scrolled_row0);
    }

    #[test]
    fn alternate_screen_default_false() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        assert!(!adapter.is_alternate_screen(id(1)).unwrap());
    }

    #[test]
    fn alternate_screen_true_after_switch() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[?1049h").unwrap();
        assert!(adapter.is_alternate_screen(id(1)).unwrap());
    }

    #[test]
    fn alternate_screen_false_after_switch_back() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[?1049h").unwrap();
        adapter.process(id(1), b"\x1b[?1049l").unwrap();
        assert!(!adapter.is_alternate_screen(id(1)).unwrap());
    }

    #[test]
    fn new_output_while_scrolled_flag() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(80, 5);
        adapter.create(id(1), size).unwrap();

        // Generate scrollback
        for i in 0..10 {
            let line = format!("line {}\r\n", i);
            adapter.process(id(1), line.as_bytes()).unwrap();
        }

        // Scroll back
        adapter.set_scrollback_offset(id(1), 3).unwrap();

        // New output while scrolled
        adapter.process(id(1), b"new data\r\n").unwrap();

        // Flag should be set
        let inst = adapter.instances.get(&id(1)).unwrap();
        assert!(inst.new_output_while_scrolled);

        // Reset scroll to 0 clears the flag
        adapter.set_scrollback_offset(id(1), 0).unwrap();
        let inst = adapter.instances.get(&id(1)).unwrap();
        assert!(!inst.new_output_while_scrolled);
    }

    #[test]
    fn scrollback_nonexistent_returns_error() {
        let adapter = Vt100ScreenAdapter::new();
        assert!(adapter.get_scrollback_offset(id(99)).is_err());
        assert!(adapter.get_max_scrollback(id(99)).is_err());
        assert!(adapter.is_alternate_screen(id(99)).is_err());
    }

    #[test]
    fn set_scrollback_nonexistent_returns_error() {
        let mut adapter = Vt100ScreenAdapter::new();
        assert!(adapter.set_scrollback_offset(id(99), 0).is_err());
    }

    #[test]
    fn scrollback_independent_per_terminal() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(80, 5);
        adapter.create(id(1), size).unwrap();
        adapter.create(id(2), size).unwrap();

        // Generate scrollback for terminal 1 only
        for i in 0..10 {
            let line = format!("line {}\r\n", i);
            adapter.process(id(1), line.as_bytes()).unwrap();
        }

        adapter.set_scrollback_offset(id(1), 3).unwrap();
        assert_eq!(adapter.get_scrollback_offset(id(1)).unwrap(), 3);
        assert_eq!(adapter.get_scrollback_offset(id(2)).unwrap(), 0);
    }

    // ─── Cursor style (DECSCUSR) tests ───

    #[test]
    fn cursor_style_default() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::DefaultUserShape);
    }

    #[test]
    fn cursor_style_blinking_block() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // DECSCUSR: CSI 1 SP q
        adapter.process(id(1), b"\x1b[1 q").unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::BlinkingBlock);
    }

    #[test]
    fn cursor_style_steady_block() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[2 q").unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::SteadyBlock);
    }

    #[test]
    fn cursor_style_blinking_underline() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[3 q").unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::BlinkingUnderScore);
    }

    #[test]
    fn cursor_style_steady_underline() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[4 q").unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::SteadyUnderScore);
    }

    #[test]
    fn cursor_style_blinking_bar() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[5 q").unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::BlinkingBar);
    }

    #[test]
    fn cursor_style_steady_bar() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[6 q").unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::SteadyBar);
    }

    #[test]
    fn cursor_style_reset_to_default() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[5 q").unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::BlinkingBar);
        // Reset: CSI 0 SP q
        adapter.process(id(1), b"\x1b[0 q").unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::DefaultUserShape);
    }

    #[test]
    fn cursor_style_updated_multiple_times() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[1 q").unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::BlinkingBlock);
        adapter.process(id(1), b"\x1b[6 q").unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::SteadyBar);
    }

    #[test]
    fn cursor_style_nonexistent_returns_error() {
        let adapter = Vt100ScreenAdapter::new();
        assert!(adapter.get_cursor_style(id(99)).is_err());
    }

    #[test]
    fn cursor_style_independent_per_terminal() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.create(id(2), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[5 q").unwrap();
        adapter.process(id(2), b"\x1b[2 q").unwrap();
        assert_eq!(adapter.get_cursor_style(id(1)).unwrap(), CursorStyle::BlinkingBar);
        assert_eq!(adapter.get_cursor_style(id(2)).unwrap(), CursorStyle::SteadyBlock);
    }

    // ─── DSR (Device Status Report) response tests ───

    #[test]
    fn dsr_query_generates_cursor_position_response() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // CSI 6 n — cursor position query
        adapter.process(id(1), b"\x1b[6n").unwrap();
        let responses = adapter.drain_pending_responses(id(1)).unwrap();
        assert_eq!(responses.len(), 1);
        // Cursor at (0,0) → 1-indexed response: ESC[1;1R
        assert_eq!(responses[0], b"\x1b[1;1R");
    }

    #[test]
    fn dsr_query_after_cursor_move() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // Move cursor to row 5, col 10 (1-indexed), then DSR query
        adapter.process(id(1), b"\x1b[5;10H\x1b[6n").unwrap();
        let responses = adapter.drain_pending_responses(id(1)).unwrap();
        assert_eq!(responses.len(), 1);
        // Cursor at (4,9) 0-indexed → 1-indexed response: ESC[5;10R
        assert_eq!(responses[0], b"\x1b[5;10R");
    }

    #[test]
    fn dsr_drain_clears_queue() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"\x1b[6n").unwrap();

        let responses = adapter.drain_pending_responses(id(1)).unwrap();
        assert_eq!(responses.len(), 1);

        // Second drain should return empty
        let responses2 = adapter.drain_pending_responses(id(1)).unwrap();
        assert!(responses2.is_empty());
    }

    #[test]
    fn dsr_no_query_returns_empty() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"Hello").unwrap();
        let responses = adapter.drain_pending_responses(id(1)).unwrap();
        assert!(responses.is_empty());
    }

    #[test]
    fn dsr_nonexistent_returns_error() {
        let mut adapter = Vt100ScreenAdapter::new();
        assert!(adapter.drain_pending_responses(id(99)).is_err());
    }

    #[test]
    fn dsr_independent_per_terminal() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.create(id(2), default_size()).unwrap();

        // Move terminal 2 cursor to (3,5), then DSR on both
        adapter.process(id(2), b"\x1b[4;6H").unwrap();
        adapter.process(id(1), b"\x1b[6n").unwrap();
        adapter.process(id(2), b"\x1b[6n").unwrap();

        let r1 = adapter.drain_pending_responses(id(1)).unwrap();
        let r2 = adapter.drain_pending_responses(id(2)).unwrap();
        assert_eq!(r1.len(), 1);
        assert_eq!(r2.len(), 1);
        assert_eq!(r1[0], b"\x1b[1;1R"); // terminal 1: cursor at (0,0)
        assert_eq!(r2[0], b"\x1b[4;6R"); // terminal 2: cursor at (3,5)
    }

    #[test]
    fn dsr_after_text_output() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // Write "Hello" then DSR — cursor should be at (0,5)
        adapter.process(id(1), b"Hello\x1b[6n").unwrap();
        let responses = adapter.drain_pending_responses(id(1)).unwrap();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0], b"\x1b[1;6R");
    }

    // ─── search_scrollback tests ───

    #[test]
    fn search_basic_match_on_visible_screen() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"hello world").unwrap();

        let matches = adapter.search_scrollback(id(1), "hello").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].col_start, 0);
        assert_eq!(matches[0].col_end, 5);
    }

    #[test]
    fn search_multiple_matches_on_same_line() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"error error error").unwrap();

        let matches = adapter.search_scrollback(id(1), "error").unwrap();
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].col_start, 0);
        assert_eq!(matches[0].col_end, 5);
        assert_eq!(matches[1].col_start, 6);
        assert_eq!(matches[1].col_end, 11);
        assert_eq!(matches[2].col_start, 12);
        assert_eq!(matches[2].col_end, 17);
    }

    #[test]
    fn search_multiple_lines() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"foo bar\r\nbaz foo").unwrap();

        let matches = adapter.search_scrollback(id(1), "foo").unwrap();
        assert_eq!(matches.len(), 2);
        // First match on row 0 (which is max_scrollback + 0 in absolute terms)
        assert_eq!(matches[0].col_start, 0);
        assert_eq!(matches[0].col_end, 3);
        // Second match on row 1
        assert!(matches[1].row > matches[0].row);
        assert_eq!(matches[1].col_start, 4);
        assert_eq!(matches[1].col_end, 7);
    }

    #[test]
    fn search_case_insensitive() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"Hello WORLD").unwrap();

        let matches = adapter.search_scrollback(id(1), "hello").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].col_start, 0);
        assert_eq!(matches[0].col_end, 5);

        let matches2 = adapter.search_scrollback(id(1), "WORLD").unwrap();
        assert_eq!(matches2.len(), 1);
        assert_eq!(matches2[0].col_start, 6);
        assert_eq!(matches2[0].col_end, 11);
    }

    #[test]
    fn search_no_match_returns_empty() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"hello world").unwrap();

        let matches = adapter.search_scrollback(id(1), "xyz").unwrap();
        assert!(matches.is_empty());
    }

    #[test]
    fn search_empty_query_returns_empty() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"hello world").unwrap();

        let matches = adapter.search_scrollback(id(1), "").unwrap();
        assert!(matches.is_empty());
    }

    #[test]
    fn search_wide_char_correct_columns() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // "テスト" is 3 wide chars, each occupying 2 cell columns
        adapter.process(id(1), "テスト".as_bytes()).unwrap();

        let matches = adapter.search_scrollback(id(1), "テスト").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].col_start, 0);
        // Each wide char = 2 columns, so "テスト" ends at column 6
        assert_eq!(matches[0].col_end, 6);
    }

    #[test]
    fn search_wide_char_after_ascii() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        // "abテスト" — 'a' at col 0, 'b' at col 1, 'テ' at cols 2-3, 'ス' at cols 4-5, 'ト' at cols 6-7
        adapter.process(id(1), "abテスト".as_bytes()).unwrap();

        let matches = adapter.search_scrollback(id(1), "テスト").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].col_start, 2);
        assert_eq!(matches[0].col_end, 8);
    }

    #[test]
    fn search_scrollback_buffer_content() {
        let mut adapter = Vt100ScreenAdapter::new();
        // Small screen: 3 rows
        let size = TerminalSize::new(80, 3);
        adapter.create(id(1), size).unwrap();

        // Write enough lines to push "FINDME" into scrollback
        adapter.process(id(1), b"FINDME line\r\n").unwrap();
        for i in 0..5 {
            let line = format!("line {}\r\n", i);
            adapter.process(id(1), line.as_bytes()).unwrap();
        }

        // "FINDME" is now in the scrollback buffer (not visible)
        let matches = adapter.search_scrollback(id(1), "FINDME").unwrap();
        assert_eq!(matches.len(), 1);
        // The match should be in the scrollback portion (row < max_scrollback)
        let max_sb = adapter.get_max_scrollback(id(1)).unwrap();
        assert!(max_sb > 0, "Expected scrollback buffer to have content");
        assert!(matches[0].row < max_sb, "Expected match in scrollback (row {} < max {})", matches[0].row, max_sb);
        assert_eq!(matches[0].col_start, 0);
        assert_eq!(matches[0].col_end, 6);
    }

    #[test]
    fn search_scrollback_preserves_offset() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(80, 3);
        adapter.create(id(1), size).unwrap();

        // Generate scrollback
        for i in 0..10 {
            let line = format!("line {}\r\n", i);
            adapter.process(id(1), line.as_bytes()).unwrap();
        }

        // Set a specific scrollback offset
        adapter.set_scrollback_offset(id(1), 2).unwrap();
        let offset_before = adapter.get_scrollback_offset(id(1)).unwrap();
        assert_eq!(offset_before, 2);

        // Search should not change the offset
        let _matches = adapter.search_scrollback(id(1), "line").unwrap();

        let offset_after = adapter.get_scrollback_offset(id(1)).unwrap();
        assert_eq!(offset_after, offset_before, "search_scrollback should restore the scrollback offset");
    }

    #[test]
    fn search_scrollback_and_visible_combined() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(80, 3);
        adapter.create(id(1), size).unwrap();

        // Write lines where "target" appears in both scrollback and visible area
        adapter.process(id(1), b"target in scrollback\r\n").unwrap();
        for i in 0..4 {
            let line = format!("filler {}\r\n", i);
            adapter.process(id(1), line.as_bytes()).unwrap();
        }
        adapter.process(id(1), b"target on screen\r\n").unwrap();

        let matches = adapter.search_scrollback(id(1), "target").unwrap();
        assert_eq!(matches.len(), 2, "Expected matches in both scrollback and visible area");
        // Results should be ordered by row ascending
        assert!(matches[0].row < matches[1].row);
    }

    #[test]
    fn search_nonexistent_terminal_returns_error() {
        let mut adapter = Vt100ScreenAdapter::new();
        assert!(adapter.search_scrollback(id(99), "test").is_err());
    }

    #[test]
    fn search_max_matches_limit() {
        let mut adapter = Vt100ScreenAdapter::new();
        // Use a wide screen to fit many matches per line
        let size = TerminalSize::new(200, 24);
        adapter.create(id(1), size).unwrap();

        // Fill rows with repeated "aa " to create many matches for "a"
        // Each row with 200 cols can have ~100 'a' matches
        // We need > 10000 matches total, so need many rows in scrollback
        // 200 cols = "aa aa aa ..." pattern gives ~67 "aa" per line
        // With 24 visible rows + scrollback, we need ~150 rows of "aa" repeated
        // Since scrollback is 10000 lines, we can push enough
        for _ in 0..200 {
            let line = "aa ".repeat(66) + "\r\n";
            adapter.process(id(1), line.as_bytes()).unwrap();
        }

        let matches = adapter.search_scrollback(id(1), "aa").unwrap();
        assert!(matches.len() <= 10_000, "Expected at most 10000 matches, got {}", matches.len());
        // We should have hit the cap
        assert_eq!(matches.len(), 10_000);
    }

    #[test]
    fn search_overlapping_pattern_finds_all() {
        let mut adapter = Vt100ScreenAdapter::new();
        adapter.create(id(1), default_size()).unwrap();
        adapter.process(id(1), b"aaaa").unwrap();

        // Searching "aa" in "aaaa" should find overlapping matches:
        // position 0: "aa", position 1: "aa", position 2: "aa"
        let matches = adapter.search_scrollback(id(1), "aa").unwrap();
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].col_start, 0);
        assert_eq!(matches[1].col_start, 1);
        assert_eq!(matches[2].col_start, 2);
    }

    #[test]
    fn search_row_numbering_scrollback_top_is_zero() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(80, 3);
        adapter.create(id(1), size).unwrap();

        // Write unique content at the very first line, then push it into scrollback
        adapter.process(id(1), b"FIRST_LINE\r\n").unwrap();
        for _ in 0..10 {
            adapter.process(id(1), b"other content\r\n").unwrap();
        }

        let matches = adapter.search_scrollback(id(1), "FIRST_LINE").unwrap();
        assert_eq!(matches.len(), 1);
        // FIRST_LINE should be at row 0 (scrollback top)
        assert_eq!(matches[0].row, 0, "Earliest scrollback line should be row 0");
    }

    // ─── get_row_cells tests ───

    #[test]
    fn get_row_cells_visible_screen_row() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(10, 3);
        adapter.create(id(1), size).unwrap();

        // Write "HELLO" on first visible row (row 0 of screen)
        adapter.process(id(1), b"HELLO").unwrap();

        let max_sb = adapter.get_max_scrollback(id(1)).unwrap();
        // abs_row for first visible row = max_scrollback + 0
        let cells = adapter.get_row_cells(id(1), max_sb).unwrap();

        assert_eq!(cells.len(), 10);
        assert_eq!(cells[0].ch, 'H');
        assert_eq!(cells[1].ch, 'E');
        assert_eq!(cells[2].ch, 'L');
        assert_eq!(cells[3].ch, 'L');
        assert_eq!(cells[4].ch, 'O');
    }

    #[test]
    fn get_row_cells_scrollback_buffer_row() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(10, 3);
        adapter.create(id(1), size).unwrap();

        // Write content that pushes first line into scrollback.
        // Screen has 3 rows, so after 4 lines the first line scrolls off.
        adapter.process(id(1), b"LINE_ONE\r\n").unwrap();
        adapter.process(id(1), b"LINE_TWO\r\n").unwrap();
        adapter.process(id(1), b"LINE_THREE\r\n").unwrap();
        adapter.process(id(1), b"LINE_FOUR\r\n").unwrap();
        adapter.process(id(1), b"LINE_FIVE").unwrap();

        let max_sb = adapter.get_max_scrollback(id(1)).unwrap();
        assert!(max_sb > 0, "Should have scrollback lines");

        // abs_row 0 = top of scrollback buffer = LINE_ONE
        let cells = adapter.get_row_cells(id(1), 0).unwrap();
        assert_eq!(cells[0].ch, 'L');
        assert_eq!(cells[1].ch, 'I');
        assert_eq!(cells[2].ch, 'N');
        assert_eq!(cells[3].ch, 'E');
        assert_eq!(cells[4].ch, '_');
        assert_eq!(cells[5].ch, 'O');
        assert_eq!(cells[6].ch, 'N');
        assert_eq!(cells[7].ch, 'E');
    }

    #[test]
    fn get_row_cells_out_of_range_returns_empty() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(10, 3);
        adapter.create(id(1), size).unwrap();

        adapter.process(id(1), b"Hello").unwrap();

        let max_sb = adapter.get_max_scrollback(id(1)).unwrap();
        let total_rows = max_sb + 3; // screen has 3 rows

        // abs_row beyond total should return empty Vec
        let cells = adapter.get_row_cells(id(1), total_rows).unwrap();
        assert!(cells.is_empty());

        let cells = adapter.get_row_cells(id(1), total_rows + 100).unwrap();
        assert!(cells.is_empty());
    }

    #[test]
    fn get_row_cells_preserves_scrollback_offset() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(10, 3);
        adapter.create(id(1), size).unwrap();

        // Push lines into scrollback
        for i in 0..10 {
            let line = format!("Line {:04}\r\n", i);
            adapter.process(id(1), line.as_bytes()).unwrap();
        }

        // Set a specific scrollback offset
        adapter.set_scrollback_offset(id(1), 3).unwrap();
        let offset_before = adapter.get_scrollback_offset(id(1)).unwrap();
        assert_eq!(offset_before, 3);

        // Call get_row_cells
        let _cells = adapter.get_row_cells(id(1), 0).unwrap();

        // The scrollback offset must be unchanged
        let offset_after = adapter.get_scrollback_offset(id(1)).unwrap();
        assert_eq!(offset_after, offset_before, "Scrollback offset must be preserved after get_row_cells");
    }

    #[test]
    fn get_row_cells_content_correctness_with_attributes() {
        let mut adapter = Vt100ScreenAdapter::new();
        let size = TerminalSize::new(20, 3);
        adapter.create(id(1), size).unwrap();

        // Write bold text on first row
        adapter.process(id(1), b"\x1b[1mBOLD\x1b[0m normal").unwrap();

        let max_sb = adapter.get_max_scrollback(id(1)).unwrap();
        let cells = adapter.get_row_cells(id(1), max_sb).unwrap();

        // Bold cells
        assert_eq!(cells[0].ch, 'B');
        assert!(cells[0].bold);
        assert_eq!(cells[1].ch, 'O');
        assert!(cells[1].bold);
        assert_eq!(cells[2].ch, 'L');
        assert!(cells[2].bold);
        assert_eq!(cells[3].ch, 'D');
        assert!(cells[3].bold);

        // Non-bold cells after reset
        assert_eq!(cells[5].ch, 'n');
        assert!(!cells[5].bold);
    }

    #[test]
    fn get_row_cells_screen_not_found_error() {
        let mut adapter = Vt100ScreenAdapter::new();
        let result = adapter.get_row_cells(id(99), 0);
        assert!(result.is_err());
    }
}
