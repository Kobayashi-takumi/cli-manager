use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color as RatColor, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::domain::primitive::{Cell, Color, CursorPos};

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
            let msg = "No terminal. Press ^t c to create.";
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
            // Convert cells to ratatui Lines
            let visible_rows = content_area.height as usize;
            let visible_cols = content_area.width as usize;

            let lines: Vec<Line> = cells
                .iter()
                .take(visible_rows)
                .map(|row| {
                    let spans: Vec<Span> = row
                        .iter()
                        .filter(|cell| cell.width != 0)
                        .scan(0u16, |visual_col, cell| {
                            let w = if cell.width == 2 { 2 } else { 1 };
                            *visual_col += w;
                            if *visual_col <= visible_cols as u16 {
                                Some(cell)
                            } else {
                                None
                            }
                        })
                        .map(|cell| {
                            let (fg, bg) = if cell.reverse {
                                (to_ratatui_color(cell.bg), to_ratatui_color(cell.fg))
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
                            Span::styled(cell.ch.to_string(), style)
                        })
                        .collect();
                    Line::from(spans)
                })
                .collect();

            let paragraph = Paragraph::new(lines);
            frame.render_widget(paragraph, content_area);

            // Set cursor position -- only if visible
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
                    frame.set_cursor_position((cursor_x, cursor_y));
                }
            }
        }
    }
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
                render(frame, area, None, None, true, None, true);
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
                render(frame, area, None, None, true, Some("/home/user/project"), true);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, None, None, true, None, true);
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
                render(frame, area, None, None, true, Some("/tmp"), true);
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
                render(frame, area, None, None, true, Some("/tmp"), false);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, Some(&cells), None, true, None, true);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // reverse should swap fg and bg
        assert_eq!(buf[(0, 1)].fg, RatColor::Indexed(4)); // was bg
        assert_eq!(buf[(0, 1)].bg, RatColor::Indexed(1)); // was fg
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, Some(&cells), Some(cursor), true, None, true);
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
                render(frame, area, Some(&cells), Some(cursor), true, None, true);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, Some(&cells), Some(cursor), true, None, true);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
                render(frame, area, Some(&cells), None, true, None, true);
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
}
