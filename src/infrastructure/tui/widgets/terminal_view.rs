use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color as RatColor, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::domain::primitive::{Cell, Color, CursorPos};

/// Search match highlight information for terminal_view rendering.
pub struct SearchHighlights {
    /// Matches as (row, col_start, col_end) in display coordinates (relative to visible area).
    pub matches: Vec<(usize, usize, usize)>,
    /// Index into `matches` of the current (focused) match, if any.
    pub current_match_index: Option<usize>,
}

/// Convert domain Color to ratatui Color
fn to_ratatui_color(color: Color) -> RatColor {
    match color {
        Color::Default => RatColor::Reset,
        Color::Indexed(n) => RatColor::Indexed(n),
        Color::Rgb(r, g, b) => RatColor::Rgb(r, g, b),
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    cells_opt: Option<&Vec<Vec<Cell>>>,
    cursor_opt: Option<CursorPos>,
    cursor_visible: bool,
    cwd_opt: Option<&str>,
    is_focused: bool,
    scrollback_info: Option<(usize, usize)>,
    in_scrollback: bool,
    search_highlights: Option<&SearchHighlights>,
) {
    if in_scrollback && area.width >= 4 && area.height >= 5 {
        render_scrollback_mode(frame, area, cells_opt, cwd_opt, is_focused, scrollback_info, search_highlights);
    } else {
        render_normal_mode(frame, area, cells_opt, cursor_opt, cursor_visible, cwd_opt, is_focused, scrollback_info, search_highlights);
    }
}

/// Build a progress bar string: `████░░░░░░` where filled proportion = offset/max.
fn build_progress_bar(width: usize, offset: usize, max: usize) -> String {
    if width == 0 || max == 0 {
        return String::new();
    }
    let filled = ((offset as f64 / max as f64) * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    let mut bar = String::with_capacity(width * 3); // UTF-8 chars may be multi-byte
    for _ in 0..filled {
        bar.push('█');
    }
    for _ in 0..empty {
        bar.push('░');
    }
    bar
}

fn render_scrollback_mode(
    frame: &mut Frame,
    area: Rect,
    cells_opt: Option<&Vec<Vec<Cell>>>,
    cwd_opt: Option<&str>,
    is_focused: bool,
    scrollback_info: Option<(usize, usize)>,
    search_highlights: Option<&SearchHighlights>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            " SCROLLBACK ",
            Style::default().fg(RatColor::LightCyan).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(RatColor::LightCyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Split inner into: CWD bar (1) | content (flexible) | status bar (1)
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);
    let cwd_area = chunks[0];
    let content_area = chunks[1];
    let status_area = chunks[2];

    // CWD bar
    let cwd_text = cwd_opt.unwrap_or("");
    let cwd_style = if is_focused {
        Style::default().bg(RatColor::DarkGray).fg(RatColor::White)
    } else {
        Style::default().fg(RatColor::DarkGray)
    };
    let cwd_line = Line::from(Span::styled(format!(" {} ", cwd_text), cwd_style));
    frame.render_widget(Paragraph::new(vec![cwd_line]), cwd_area);

    // Terminal content
    if let Some(cells) = cells_opt {
        let lines = cells_to_lines(cells, content_area.height as usize, content_area.width as usize, search_highlights);
        frame.render_widget(Paragraph::new(lines), content_area);
    }

    // Status bar
    let (offset, max) = scrollback_info.unwrap_or((0, 0));
    let offset_text = format!(" {}/{} ", offset, max);
    let hint_text = " ↑↓:line PgUp/Dn:page g/G:top/bottom q:exit ";
    let offset_len = offset_text.len();
    let hint_len = hint_text.len();
    let bar_width = (status_area.width as usize).saturating_sub(offset_len + hint_len);

    let status_style = Style::default().bg(RatColor::LightCyan).fg(RatColor::Black);
    let mut spans = vec![Span::styled(&offset_text, status_style)];
    if bar_width > 0 {
        let bar = build_progress_bar(bar_width, offset, max);
        spans.push(Span::styled(bar, status_style));
    }
    spans.push(Span::styled(hint_text, status_style));

    // Pad remaining width with background color
    let total_char_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let remaining = (status_area.width as usize).saturating_sub(total_char_len);
    if remaining > 0 {
        spans.push(Span::styled(" ".repeat(remaining), status_style));
    }

    frame.render_widget(Paragraph::new(vec![Line::from(spans)]), status_area);
}

fn render_normal_mode(
    frame: &mut Frame,
    area: Rect,
    cells_opt: Option<&Vec<Vec<Cell>>>,
    cursor_opt: Option<CursorPos>,
    cursor_visible: bool,
    cwd_opt: Option<&str>,
    is_focused: bool,
    scrollback_info: Option<(usize, usize)>,
    search_highlights: Option<&SearchHighlights>,
) {
    // Split into CWD bar (1 line) + terminal content
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let cwd_area = chunks[0];
    let content_area = chunks[1];

    // CWD bar — background changes based on focus
    let cwd_text = cwd_opt.unwrap_or("");
    let cwd_style = if is_focused {
        Style::default().bg(RatColor::DarkGray).fg(RatColor::White)
    } else {
        Style::default().fg(RatColor::DarkGray)
    };
    let cwd_line = Line::from(Span::styled(
        format!(" {} ", cwd_text),
        cwd_style,
    ));
    let cwd_paragraph = Paragraph::new(vec![cwd_line]);
    frame.render_widget(cwd_paragraph, cwd_area);

    // Terminal content
    match cells_opt {
        None => {
            // No terminal selected
            let msg = "No terminal. Press ^b c to create.";
            // Center the message
            let x = content_area.x + content_area.width.saturating_sub(msg.len() as u16) / 2;
            let y = content_area.y + content_area.height / 2;
            let line = Line::from(Span::styled(
                msg,
                Style::default().add_modifier(Modifier::DIM),
            ));
            let centered_area = Rect::new(x, y, msg.len() as u16, 1);
            frame.render_widget(Paragraph::new(vec![line]), centered_area);
        }
        Some(cells) => {
            let lines = cells_to_lines(cells, content_area.height as usize, content_area.width as usize, search_highlights);
            let paragraph = Paragraph::new(lines);
            frame.render_widget(paragraph, content_area);

            // Scrollback indicator (old style, only in normal mode)
            if let Some((offset, max)) = scrollback_info
                && offset > 0
            {
                let indicator = format!("[{}/{}]", offset, max);
                let indicator_len = indicator.len() as u16;
                if indicator_len < content_area.width {
                    let x = content_area.x + content_area.width - indicator_len;
                    let y = content_area.y;
                    let indicator_style = Style::default()
                        .bg(RatColor::Yellow)
                        .fg(RatColor::Black)
                        .add_modifier(Modifier::BOLD);
                    let indicator_span = Span::styled(indicator, indicator_style);
                    let indicator_area = Rect::new(x, y, indicator_len, 1);
                    frame.render_widget(Paragraph::new(vec![Line::from(indicator_span)]), indicator_area);
                }
            }

            // Cursor rendering
            if cursor_visible
                && let Some(cursor) = cursor_opt
            {
                let cursor_visual_col: u16 = cells
                    .get(cursor.row as usize)
                    .map(|row| {
                        row.iter()
                            .take(cursor.col as usize)
                            .filter(|c| c.width != 0)
                            .map(|c| if c.width == 2 { 2u16 } else { 1u16 })
                            .sum()
                    })
                    .unwrap_or(cursor.col);
                let cursor_x = content_area.x + cursor_visual_col;
                let cursor_y = content_area.y + cursor.row;
                if cursor_x < content_area.x + content_area.width
                    && cursor_y < content_area.y + content_area.height
                {
                    let cursor_ch = cells
                        .get(cursor.row as usize)
                        .and_then(|row| row.get(cursor.col as usize))
                        .map(|c| if c.width != 0 { c.ch } else { ' ' })
                        .unwrap_or(' ');
                    let cursor_cell = cells
                        .get(cursor.row as usize)
                        .and_then(|row| row.get(cursor.col as usize));
                    let cursor_style = if let Some(cell) = cursor_cell {
                        let fg = to_ratatui_color(cell.fg);
                        let bg = to_ratatui_color(cell.bg);
                        let (cursor_fg, cursor_bg) = if fg == RatColor::Reset && bg == RatColor::Reset {
                            (RatColor::Black, RatColor::White)
                        } else if cell.reverse {
                            (to_ratatui_color(cell.fg), to_ratatui_color(cell.bg))
                        } else {
                            (bg, fg)
                        };
                        Style::default().fg(cursor_fg).bg(cursor_bg)
                    } else {
                        Style::default().fg(RatColor::Black).bg(RatColor::White)
                    };
                    let cursor_span = Span::styled(cursor_ch.to_string(), cursor_style);
                    let cursor_area = Rect::new(cursor_x, cursor_y, 1, 1);
                    frame.render_widget(Paragraph::new(vec![Line::from(cursor_span)]), cursor_area);

                    frame.set_cursor_position((cursor_x, cursor_y));
                }
            }
        }
    }
}

/// Check if a cell at the given (display_row, cell_col) is within a search highlight range.
/// Returns `Some(true)` if it's the current match, `Some(false)` if a normal match, `None` if not highlighted.
fn check_search_highlight(
    highlights: Option<&SearchHighlights>,
    display_row: usize,
    cell_col: usize,
) -> Option<bool> {
    let hl = highlights?;
    for (i, &(row, col_start, col_end)) in hl.matches.iter().enumerate() {
        if row == display_row && cell_col >= col_start && cell_col < col_end {
            let is_current = hl.current_match_index == Some(i);
            return Some(is_current);
        }
    }
    None
}

/// Convert cell grid to ratatui Lines, applying visual width clipping and styling.
fn cells_to_lines<'a>(
    cells: &[Vec<Cell>],
    visible_rows: usize,
    visible_cols: usize,
    search_highlights: Option<&SearchHighlights>,
) -> Vec<Line<'a>> {
    cells
        .iter()
        .take(visible_rows)
        .enumerate()
        .map(|(row_idx, row)| {
            let spans: Vec<Span> = row
                .iter()
                .enumerate()
                .filter(|(_, cell)| cell.width != 0)
                .scan(0u16, |visual_col, (col_idx, cell)| {
                    let w = if cell.width == 2 { 2 } else { 1 };
                    *visual_col += w;
                    if *visual_col <= visible_cols as u16 {
                        Some((col_idx, cell))
                    } else {
                        None
                    }
                })
                .map(|(col_idx, cell)| {
                    let (fg, bg) = if cell.reverse {
                        let rfg = to_ratatui_color(cell.bg);
                        let rbg = to_ratatui_color(cell.fg);
                        if rfg == RatColor::Reset && rbg == RatColor::Reset {
                            (RatColor::Black, RatColor::White)
                        } else {
                            (rfg, rbg)
                        }
                    } else {
                        (to_ratatui_color(cell.fg), to_ratatui_color(cell.bg))
                    };
                    let (fg, bg) = if cell.hidden {
                        (bg, bg)
                    } else {
                        (fg, bg)
                    };
                    let mut style = Style::default().fg(fg).bg(bg);
                    if cell.bold {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if cell.underline {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    if cell.italic {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    if cell.dim {
                        style = style.add_modifier(Modifier::DIM);
                    }
                    if cell.strikethrough {
                        style = style.add_modifier(Modifier::CROSSED_OUT);
                    }

                    // Apply search highlight override
                    if let Some(is_current) = check_search_highlight(search_highlights, row_idx, col_idx) {
                        if is_current {
                            style = style
                                .fg(RatColor::Black)
                                .bg(RatColor::Rgb(255, 165, 0)); // Orange for current match
                        } else {
                            style = style
                                .fg(RatColor::Black)
                                .bg(RatColor::Yellow); // Yellow for normal matches
                        }
                    }

                    Span::styled(cell.ch.to_string(), style)
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn to_ratatui_color_converts_default() {
        assert_eq!(to_ratatui_color(Color::Default), RatColor::Reset);
    }

    #[test]
    fn to_ratatui_color_converts_indexed() {
        assert_eq!(to_ratatui_color(Color::Indexed(5)), RatColor::Indexed(5));
        assert_eq!(to_ratatui_color(Color::Indexed(0)), RatColor::Indexed(0));
        assert_eq!(to_ratatui_color(Color::Indexed(255)), RatColor::Indexed(255));
    }

    #[test]
    fn to_ratatui_color_converts_rgb() {
        assert_eq!(
            to_ratatui_color(Color::Rgb(255, 128, 0)),
            RatColor::Rgb(255, 128, 0)
        );
        assert_eq!(
            to_ratatui_color(Color::Rgb(0, 0, 0)),
            RatColor::Rgb(0, 0, 0)
        );
    }

    #[test]
    fn render_no_terminal_shows_placeholder() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 60, 20);
                render(frame, area, None, None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Check that the placeholder message appears somewhere in the buffer
        let mut found = false;
        for y in 0..20 {
            let row: String = (0..60)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect();
            if row.contains("No terminal") {
                found = true;
                break;
            }
        }
        assert!(found, "Expected placeholder message 'No terminal...'");
    }

    #[test]
    fn render_cwd_bar_shows_path() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 60, 20);
                render(frame, area, None, None, true, Some("/home/user/project"), true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // CWD bar is the first row
        let cwd_row: String = (0..60)
            .map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            cwd_row.contains("/home/user/project"),
            "Expected CWD in first row, got: {}",
            cwd_row
        );
    }

    #[test]
    fn render_cells_with_content() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![
            vec![
                Cell { ch: 'H', ..Cell::default() },
                Cell { ch: 'i', ..Cell::default() },
            ],
        ];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Content starts at row 1 (after CWD bar at row 0)
        assert_eq!(buf[(0, 1)].symbol(), "H");
        assert_eq!(buf[(1, 1)].symbol(), "i");
    }

    #[test]
    fn render_cells_with_bold_and_underline() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![
            Cell { ch: 'B', bold: true, ..Cell::default() },
            Cell { ch: 'U', underline: true, ..Cell::default() },
        ]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        assert!(buf[(0, 1)].modifier.contains(Modifier::BOLD));
        assert!(buf[(1, 1)].modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn render_cells_with_colors() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell {
            ch: 'C',
            fg: Color::Rgb(255, 0, 0),
            bg: Color::Indexed(4),
            ..Cell::default()
        }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        assert_eq!(buf[(0, 1)].fg, RatColor::Rgb(255, 0, 0));
        assert_eq!(buf[(0, 1)].bg, RatColor::Indexed(4));
    }

    #[test]
    fn render_clips_to_visible_area() {
        let backend = TestBackend::new(5, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        // Create cells larger than visible area
        let row = vec![
            Cell { ch: '1', ..Cell::default() },
            Cell { ch: '2', ..Cell::default() },
            Cell { ch: '3', ..Cell::default() },
            Cell { ch: '4', ..Cell::default() },
            Cell { ch: '5', ..Cell::default() },
            Cell { ch: '6', ..Cell::default() }, // beyond width of 5
            Cell { ch: '7', ..Cell::default() },
        ];
        let cells = vec![row.clone(), row.clone(), row.clone(), row.clone(), row]; // 5 rows, but only 3 visible (4 - 1 for CWD)

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 5, 4);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Content area is rows 1-3 (3 rows). Column 0-4 (5 cols).
        // Row 1 should have '1' through '5'
        assert_eq!(buf[(0, 1)].symbol(), "1");
        assert_eq!(buf[(4, 1)].symbol(), "5");
        // The 6th and 7th chars should NOT appear in the buffer
    }

    #[test]
    fn render_empty_cwd_bar_when_none() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                render(frame, area, None, None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // CWD bar should have dark gray background
        assert_eq!(buf[(0, 0)].bg, RatColor::DarkGray);
    }

    #[test]
    fn render_focused_cwd_bar_has_dark_gray_bg() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                render(frame, area, None, None, true, Some("/tmp"), true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        assert_eq!(buf[(0, 0)].bg, RatColor::DarkGray);
        assert_eq!(buf[(0, 0)].fg, RatColor::White);
    }

    #[test]
    fn render_unfocused_cwd_bar_has_no_bg() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                render(frame, area, None, None, true, Some("/tmp"), false, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Unfocused CWD bar should not have DarkGray background
        assert_eq!(buf[(0, 0)].bg, RatColor::Reset);
        assert_eq!(buf[(0, 0)].fg, RatColor::DarkGray);
    }

    #[test]
    fn render_cells_with_italic() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell {
            ch: 'I',
            italic: true,
            ..Cell::default()
        }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        assert!(buf[(0, 1)].modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn render_cells_with_dim() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell {
            ch: 'D',
            dim: true,
            ..Cell::default()
        }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        assert!(buf[(0, 1)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn render_cells_with_strikethrough() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell {
            ch: 'S',
            strikethrough: true,
            ..Cell::default()
        }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        assert!(buf[(0, 1)].modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn render_cells_with_reverse_swaps_fg_bg() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell {
            ch: 'R',
            fg: Color::Indexed(1),      // red
            bg: Color::Indexed(4),       // blue
            reverse: true,
            ..Cell::default()
        }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // reverse should swap fg and bg
        assert_eq!(buf[(0, 1)].fg, RatColor::Indexed(4)); // was bg
        assert_eq!(buf[(0, 1)].bg, RatColor::Indexed(1)); // was fg
    }

    #[test]
    fn render_cells_with_reverse_default_colors_uses_black_on_white() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        // reverse with both fg/bg = Default: should render as Black on White
        // (not Reset on Reset, which would be invisible)
        let cells = vec![vec![Cell {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            reverse: true,
            ..Cell::default()
        }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        assert_eq!(buf[(0, 1)].fg, RatColor::Black);
        assert_eq!(buf[(0, 1)].bg, RatColor::White);
    }

    #[test]
    fn render_cells_with_hidden_sets_fg_to_bg() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell {
            ch: 'H',
            fg: Color::Indexed(1),
            bg: Color::Indexed(4),
            hidden: true,
            ..Cell::default()
        }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // hidden: fg should equal bg
        assert_eq!(buf[(0, 1)].fg, RatColor::Indexed(4));
        assert_eq!(buf[(0, 1)].bg, RatColor::Indexed(4));
    }

    #[test]
    fn render_cells_with_reverse_and_hidden_combined() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        // reverse is applied first (swap fg/bg), then hidden (set fg=bg)
        let cells = vec![vec![Cell {
            ch: 'X',
            fg: Color::Indexed(1),
            bg: Color::Indexed(4),
            reverse: true,
            hidden: true,
            ..Cell::default()
        }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // After reverse: fg=4(blue), bg=1(red). After hidden: fg=bg=1(red)
        assert_eq!(buf[(0, 1)].fg, RatColor::Indexed(1));
        assert_eq!(buf[(0, 1)].bg, RatColor::Indexed(1));
    }

    #[test]
    fn render_cursor_position_ascii_only() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![
            Cell { ch: 'a', width: 1, ..Cell::default() },
            Cell { ch: 'b', width: 1, ..Cell::default() },
            Cell { ch: 'c', width: 1, ..Cell::default() },
        ]];

        let cursor = CursorPos { row: 0, col: 2 };

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                render(frame, area, Some(&cells), Some(cursor), true, None, true, None, false, None);
            })
            .unwrap();

        // Cursor should be at x=2 (content starts at x=0, cursor at col 2)
        // Content area starts at row 1 (after CWD bar)
        // For ASCII-only, visual col == logical col, so no adjustment needed
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(0, 1)].symbol(), "a");
        assert_eq!(buf[(1, 1)].symbol(), "b");
        assert_eq!(buf[(2, 1)].symbol(), "c");
    }

    #[test]
    fn render_cursor_position_with_wide_chars() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        // Row: 'あ'(width=2) + continuation(width=0) + 'b'(width=1)
        // Logical: [あ][cont][b]  at indices 0, 1, 2
        // Visual:  [あ][あ][b]    at visual cols 0-1, 2
        // cursor.col = 2 (pointing to 'b') -> visual column should be 2 (because 'あ' takes 2 visual cols)
        let cells = vec![vec![
            Cell { ch: 'あ', width: 2, ..Cell::default() },
            Cell { ch: ' ', width: 0, ..Cell::default() },
            Cell { ch: 'b', width: 1, ..Cell::default() },
        ]];

        let cursor = CursorPos { row: 0, col: 2 };

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                render(frame, area, Some(&cells), Some(cursor), true, None, true, None, false, None);
            })
            .unwrap();

        // This should not panic and should render correctly
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(0, 1)].symbol(), "あ");
        assert_eq!(buf[(2, 1)].symbol(), "b");
    }

    #[test]
    fn render_wide_char_clipped_at_boundary() {
        let backend = TestBackend::new(5, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        // Row with width 5: 'a'(1) + 'あ'(2) + cont(0) + 'b'(1) + 'い'(2) + cont(0)
        // Visual: a あ b い = 1+2+1+2 = 6 visual cols but only 5 visible
        // The 'い' should be clipped since it would start at visual col 4 and extend to col 5
        let cells = vec![vec![
            Cell { ch: 'a', width: 1, ..Cell::default() },
            Cell { ch: 'あ', width: 2, ..Cell::default() },
            Cell { ch: ' ', width: 0, ..Cell::default() },
            Cell { ch: 'b', width: 1, ..Cell::default() },
            Cell { ch: 'い', width: 2, ..Cell::default() },
            Cell { ch: ' ', width: 0, ..Cell::default() },
        ]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 5, 4);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Content area is rows 1-3 (3 rows), cols 0-4 (5 cols)
        assert_eq!(buf[(0, 1)].symbol(), "a");
        assert_eq!(buf[(1, 1)].symbol(), "あ");
        assert_eq!(buf[(3, 1)].symbol(), "b");
        // 'い' at visual col 4-5 should be clipped (doesn't fit in 5 cols)
    }

    #[test]
    fn render_cursor_visual_col_with_multiple_wide_chars() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        // Row: 'あ'(2) + cont(0) + 'い'(2) + cont(0) + 'c'(1)
        // Logical indices: 0, 1, 2, 3, 4
        // Visual cols:     0-1,  , 2-3,  , 4
        // cursor.col = 4 -> visual col should be 4
        let cells = vec![vec![
            Cell { ch: 'あ', width: 2, ..Cell::default() },
            Cell { ch: ' ', width: 0, ..Cell::default() },
            Cell { ch: 'い', width: 2, ..Cell::default() },
            Cell { ch: ' ', width: 0, ..Cell::default() },
            Cell { ch: 'c', width: 1, ..Cell::default() },
        ]];

        let cursor = CursorPos { row: 0, col: 4 };

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                render(frame, area, Some(&cells), Some(cursor), true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        assert_eq!(buf[(0, 1)].symbol(), "あ");
        assert_eq!(buf[(2, 1)].symbol(), "い");
        assert_eq!(buf[(4, 1)].symbol(), "c");
    }

    #[test]
    fn render_wide_chars_visual_width_clipping() {
        // Verifies that the scan-based visual width clipping works correctly.
        // With visible_cols=3 and row [あ(2), cont(0), い(2), cont(0)]:
        // After fix: filter gives [あ, い], scan stops い at visual col 4 > 3,
        // so only [あ] is rendered (2 visual cols).
        let backend = TestBackend::new(3, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![
            Cell { ch: 'あ', width: 2, ..Cell::default() },
            Cell { ch: ' ', width: 0, ..Cell::default() },
            Cell { ch: 'い', width: 2, ..Cell::default() },
            Cell { ch: ' ', width: 0, ..Cell::default() },
        ]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 3, 4);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Content area: rows 1-3, cols 0-2 (3 cols)
        // 'あ' takes 2 visual cols (fits in 3)
        assert_eq!(buf[(0, 1)].symbol(), "あ");
        // 'い' should not appear -- col 2 should be empty
        // ratatui may render it as " " or leave it empty
        let sym = buf[(2, 1)].symbol();
        assert_ne!(sym, "い", "Wide char 'い' should be clipped at 3-col boundary");
    }

    #[test]
    fn render_wide_char_skips_continuation_cell() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        // Simulate a wide character 'あ' (width=2) followed by its continuation (width=0), then 'b'
        let cells = vec![vec![
            Cell { ch: 'あ', width: 2, ..Cell::default() },
            Cell { ch: ' ', width: 0, ..Cell::default() },
            Cell { ch: 'b', width: 1, ..Cell::default() },
        ]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Row 1 (content starts after CWD bar at row 0)
        // ratatui handles wide char display width automatically.
        // 'あ' occupies 2 columns visually, then 'b' appears at column 2.
        assert_eq!(buf[(0, 1)].symbol(), "あ");
        assert_eq!(buf[(2, 1)].symbol(), "b");
    }

    #[test]
    fn render_mixed_narrow_and_wide_chars() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![
            Cell { ch: 'a', width: 1, ..Cell::default() },
            Cell { ch: 'あ', width: 2, ..Cell::default() },
            Cell { ch: ' ', width: 0, ..Cell::default() },
            Cell { ch: 'b', width: 1, ..Cell::default() },
        ]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 10, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Content row at y=1:
        // col 0: 'a' (width 1)
        // col 1-2: 'あ' (width 2, ratatui auto)
        // col 3: 'b'
        assert_eq!(buf[(0, 1)].symbol(), "a");
        assert_eq!(buf[(1, 1)].symbol(), "あ");
        assert_eq!(buf[(3, 1)].symbol(), "b");
    }

    #[test]
    fn render_scrollback_indicator_shows_offset_and_max() {
        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 5);
                render(frame, area, Some(&cells), None, true, None, true, Some((42, 1000)), false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Indicator "[42/1000]" should appear at the top-right of content_area (row 1)
        // content_area starts at row 1 (after CWD bar), width 30
        // "[42/1000]" is 9 chars, so it starts at col 30-9 = 21
        let indicator: String = (21..30)
            .map(|x| buf[(x, 1)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert_eq!(indicator, "[42/1000]");
        // Verify style: yellow bg, black fg, bold
        assert_eq!(buf[(21, 1)].bg, RatColor::Yellow);
        assert_eq!(buf[(21, 1)].fg, RatColor::Black);
        assert!(buf[(21, 1)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn render_scrollback_indicator_hidden_when_offset_zero() {
        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 5);
                render(frame, area, Some(&cells), None, true, None, true, Some((0, 1000)), false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Row 1 should NOT have yellow background anywhere
        let has_yellow = (0..30).any(|x| buf[(x, 1)].bg == RatColor::Yellow);
        assert!(!has_yellow, "Indicator should not appear when offset is 0");
    }

    #[test]
    fn render_scrollback_indicator_hidden_when_none() {
        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let has_yellow = (0..30).any(|x| buf[(x, 1)].bg == RatColor::Yellow);
        assert!(!has_yellow, "Indicator should not appear when scrollback_info is None");
    }

    #[test]
    fn render_scrollback_indicator_position_varies_with_digits() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                // "[5/50]" = 6 chars, starts at col 20-6 = 14
                render(frame, area, Some(&cells), None, true, None, true, Some((5, 50)), false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let indicator: String = (14..20)
            .map(|x| buf[(x, 1)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert_eq!(indicator, "[5/50]");
    }

    #[test]
    fn render_scrollback_indicator_not_shown_when_too_wide() {
        // Content area width = 3 (very narrow), indicator "[1/10]" = 6 chars > 3
        let backend = TestBackend::new(3, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 3, 4);
                render(frame, area, Some(&cells), None, true, None, true, Some((1, 10)), false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // No yellow background should appear since indicator doesn't fit
        let has_yellow = (0..3).any(|x| buf[(x, 1)].bg == RatColor::Yellow);
        assert!(!has_yellow, "Indicator should not appear when it doesn't fit");
    }

    // === Scrollback mode UI tests ===

    #[test]
    fn render_scrollback_mode_shows_border() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 60, 20);
                render(frame, area, Some(&cells), None, false, None, true, Some((10, 100)), true, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Top-left corner should be a rounded border character with LightCyan color
        assert_eq!(buf[(0, 0)].fg, RatColor::LightCyan);
        // Border chars: ╭ (top-left rounded)
        let corner = buf[(0, 0)].symbol();
        assert_eq!(corner, "╭", "Expected rounded border corner");
    }

    #[test]
    fn render_scrollback_mode_shows_scrollback_title() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 60, 20);
                render(frame, area, Some(&cells), None, false, None, true, Some((10, 100)), true, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let top_row: String = (0..60)
            .map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            top_row.contains("SCROLLBACK"),
            "Expected 'SCROLLBACK' in title, got: {}",
            top_row
        );
    }

    #[test]
    fn render_scrollback_mode_title_is_lightcyan_bold() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 60, 20);
                let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];
                render(frame, area, Some(&cells), None, false, None, true, Some((10, 100)), true, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Find 'S' of SCROLLBACK in top row
        let s_pos = (0..60).find(|&x| buf[(x, 0)].symbol() == "S");
        assert!(s_pos.is_some(), "Expected 'S' from SCROLLBACK in top row");
        let x = s_pos.unwrap();
        assert_eq!(buf[(x, 0)].fg, RatColor::LightCyan);
        assert!(buf[(x, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn render_scrollback_mode_shows_status_bar() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 80, 20);
                render(frame, area, Some(&cells), None, false, None, true, Some((50, 200)), true, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Status bar is at the bottom row inside the border (row 18 = area.y + area.height - 1 - 1)
        // Inner area: y=1..18, so status bar at y=18
        let status_row: String = (1..79)
            .map(|x| buf[(x, 18)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            status_row.contains("50/200"),
            "Expected '50/200' in status bar, got: {}",
            status_row
        );
        assert!(
            status_row.contains("q:exit"),
            "Expected 'q:exit' hint in status bar, got: {}",
            status_row
        );
    }

    #[test]
    fn render_scrollback_mode_status_bar_has_lightcyan_bg() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 60, 20);
                render(frame, area, Some(&cells), None, false, None, true, Some((10, 100)), true, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Status bar: inner y = 18 (area 20 - border 1 - border 1 = 18 inner rows, last inner row at y=18)
        // Inside border, first content col = 1
        assert_eq!(buf[(1, 18)].bg, RatColor::LightCyan);
        assert_eq!(buf[(1, 18)].fg, RatColor::Black);
    }

    #[test]
    fn render_scrollback_mode_offset_zero_still_shows_ui() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 60, 20);
                render(frame, area, Some(&cells), None, false, None, true, Some((0, 100)), true, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Should still show SCROLLBACK title and status bar even at offset 0
        let top_row: String = (0..60)
            .map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(top_row.contains("SCROLLBACK"));

        // Status bar should show 0/100
        let status_row: String = (1..59)
            .map(|x| buf[(x, 18)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(status_row.contains("0/100"), "Expected '0/100' in status bar, got: {}", status_row);
    }

    #[test]
    fn render_scrollback_mode_small_terminal_falls_back() {
        // area.width < 4 || area.height < 5 => fallback to normal mode (no border)
        let backend = TestBackend::new(3, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 3, 4);
                render(frame, area, Some(&cells), None, false, None, true, Some((5, 50)), true, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // In fallback mode, no LightCyan border
        assert_ne!(buf[(0, 0)].fg, RatColor::LightCyan, "Small terminal should fallback to normal mode");
    }

    #[test]
    fn render_scrollback_mode_no_old_indicator() {
        // In scrollback mode, the old yellow [offset/max] indicator should not appear
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 60, 20);
                render(frame, area, Some(&cells), None, false, None, true, Some((10, 100)), true, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // No cell should have Yellow background (old indicator style)
        let has_yellow_bg = (0..60).any(|x| {
            (0..20).any(|y| buf[(x, y)].bg == RatColor::Yellow)
        });
        assert!(!has_yellow_bg, "Scrollback mode should not show old yellow indicator");
    }

    #[test]
    fn render_scrollback_mode_progress_bar_present() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![Cell { ch: 'A', ..Cell::default() }]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 80, 20);
                render(frame, area, Some(&cells), None, false, None, true, Some((50, 100)), true, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Status bar at y=18 should contain progress bar characters
        let status_row: String = (1..79)
            .map(|x| buf[(x, 18)].symbol().chars().next().unwrap_or(' '))
            .collect();
        let has_filled = status_row.contains('█');
        let has_empty = status_row.contains('░');
        assert!(has_filled || has_empty, "Expected progress bar chars in status bar, got: {}", status_row);
    }

    // === build_progress_bar unit tests ===

    #[test]
    fn progress_bar_empty_when_width_zero() {
        assert_eq!(build_progress_bar(0, 50, 100), "");
    }

    #[test]
    fn progress_bar_empty_when_max_zero() {
        assert_eq!(build_progress_bar(10, 0, 0), "");
    }

    #[test]
    fn progress_bar_half_filled() {
        let bar = build_progress_bar(10, 50, 100);
        let filled = bar.chars().filter(|&c| c == '█').count();
        let empty = bar.chars().filter(|&c| c == '░').count();
        assert_eq!(filled, 5);
        assert_eq!(empty, 5);
    }

    #[test]
    fn progress_bar_fully_filled() {
        let bar = build_progress_bar(10, 100, 100);
        let filled = bar.chars().filter(|&c| c == '█').count();
        assert_eq!(filled, 10);
    }

    #[test]
    fn progress_bar_fully_empty() {
        let bar = build_progress_bar(10, 0, 100);
        let empty = bar.chars().filter(|&c| c == '░').count();
        assert_eq!(empty, 10);
    }

    // === check_search_highlight unit tests ===

    #[test]
    fn check_search_highlight_none_when_no_highlights() {
        let result = check_search_highlight(None, 0, 0);
        assert!(result.is_none());
    }

    #[test]
    fn check_search_highlight_none_when_not_in_range() {
        let hl = SearchHighlights {
            matches: vec![(0, 5, 10)],
            current_match_index: Some(0),
        };
        // col 3 is before the match range [5, 10)
        let result = check_search_highlight(Some(&hl), 0, 3);
        assert!(result.is_none());
    }

    #[test]
    fn check_search_highlight_returns_true_for_current_match() {
        let hl = SearchHighlights {
            matches: vec![(0, 5, 10)],
            current_match_index: Some(0),
        };
        let result = check_search_highlight(Some(&hl), 0, 5);
        assert_eq!(result, Some(true));
    }

    #[test]
    fn check_search_highlight_returns_false_for_non_current_match() {
        let hl = SearchHighlights {
            matches: vec![(0, 5, 10), (1, 0, 3)],
            current_match_index: Some(0),
        };
        // Match at index 1 is not current
        let result = check_search_highlight(Some(&hl), 1, 1);
        assert_eq!(result, Some(false));
    }

    #[test]
    fn check_search_highlight_exclusive_end() {
        let hl = SearchHighlights {
            matches: vec![(0, 5, 10)],
            current_match_index: Some(0),
        };
        // col 10 is at the exclusive end boundary -- should NOT match
        let result = check_search_highlight(Some(&hl), 0, 10);
        assert!(result.is_none());
        // col 9 is the last inclusive position -- should match
        let result = check_search_highlight(Some(&hl), 0, 9);
        assert_eq!(result, Some(true));
    }

    #[test]
    fn check_search_highlight_wrong_row_returns_none() {
        let hl = SearchHighlights {
            matches: vec![(0, 5, 10)],
            current_match_index: Some(0),
        };
        let result = check_search_highlight(Some(&hl), 1, 5);
        assert!(result.is_none());
    }

    #[test]
    fn check_search_highlight_no_current_match_index() {
        let hl = SearchHighlights {
            matches: vec![(0, 5, 10)],
            current_match_index: None,
        };
        // Match exists but no current index: returns false (not current)
        let result = check_search_highlight(Some(&hl), 0, 5);
        assert_eq!(result, Some(false));
    }

    // === search highlight rendering tests ===

    #[test]
    fn render_with_search_highlights_normal_match_has_yellow_bg() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![
            Cell { ch: 'a', ..Cell::default() },
            Cell { ch: 'b', ..Cell::default() },
            Cell { ch: 'c', ..Cell::default() },
        ]];

        let highlights = SearchHighlights {
            matches: vec![(0, 1, 2)], // highlight cell at col 1 ('b')
            current_match_index: None,
        };

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, Some(&highlights));
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Content at row 1 (after CWD bar), col 1 should have Yellow background
        assert_eq!(buf[(1, 1)].bg, RatColor::Yellow);
        assert_eq!(buf[(1, 1)].fg, RatColor::Black);
        // Col 0 ('a') should NOT have yellow bg
        assert_ne!(buf[(0, 1)].bg, RatColor::Yellow);
    }

    #[test]
    fn render_with_search_highlights_current_match_has_orange_bg() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![
            Cell { ch: 'x', ..Cell::default() },
            Cell { ch: 'y', ..Cell::default() },
            Cell { ch: 'z', ..Cell::default() },
        ]];

        let highlights = SearchHighlights {
            matches: vec![(0, 0, 2)], // highlight cols 0-1 ('x', 'y')
            current_match_index: Some(0),
        };

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, Some(&highlights));
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Current match should have orange bg (Rgb(255, 165, 0))
        assert_eq!(buf[(0, 1)].bg, RatColor::Rgb(255, 165, 0));
        assert_eq!(buf[(0, 1)].fg, RatColor::Black);
        assert_eq!(buf[(1, 1)].bg, RatColor::Rgb(255, 165, 0));
        // Col 2 ('z') should NOT have highlight bg
        assert_ne!(buf[(2, 1)].bg, RatColor::Rgb(255, 165, 0));
    }

    #[test]
    fn render_with_no_highlights_renders_normally() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![
            Cell { ch: 'a', ..Cell::default() },
        ]];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, None);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Without highlights, standard rendering - no yellow/orange bg
        assert_ne!(buf[(0, 1)].bg, RatColor::Yellow);
        assert_ne!(buf[(0, 1)].bg, RatColor::Rgb(255, 165, 0));
    }

    #[test]
    fn render_with_empty_highlights_renders_normally() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let cells = vec![vec![
            Cell { ch: 'a', ..Cell::default() },
        ]];

        let highlights = SearchHighlights {
            matches: vec![],
            current_match_index: None,
        };

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                render(frame, area, Some(&cells), None, true, None, true, None, false, Some(&highlights));
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        assert_ne!(buf[(0, 1)].bg, RatColor::Yellow);
    }
}
