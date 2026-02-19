use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color as RatColor, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::domain::primitive::{Cell, Color, CursorPos};

/// Convert domain Color to ratatui Color (same logic as terminal_view)
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
    is_focused: bool,
    scrollback_info: Option<(usize, usize)>,
    in_scrollback: bool,
) {
    // Guard against areas too small to render borders + content
    if area.width < 3 || area.height < 3 {
        return;
    }

    let border_color = if in_scrollback {
        RatColor::LightCyan
    } else if is_focused {
        RatColor::Yellow
    } else {
        RatColor::DarkGray
    };

    let block = if in_scrollback {
        let hint_style = Style::default()
            .fg(RatColor::LightCyan)
            .add_modifier(Modifier::DIM);

        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Span::styled(
                " SCROLLBACK ",
                Style::default().fg(RatColor::LightCyan).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Line::from(Span::styled(" ↑↓:scroll q:exit ", hint_style)).right_aligned())
            .border_style(Style::default().fg(border_color));

        if let Some((offset, max)) = scrollback_info {
            let indicator = format!("[{}/{}]", offset, max);
            block = block.title(
                Line::from(Span::styled(
                    indicator,
                    Style::default()
                        .fg(RatColor::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))
                .right_aligned(),
            );
        }
        block
    } else {
        let hint_style = Style::default()
            .fg(border_color)
            .add_modifier(Modifier::DIM);

        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Mini Terminal ")
            .title_bottom(Line::from(Span::styled(" ^b ` to close ", hint_style)).right_aligned())
            .border_style(Style::default().fg(border_color));

        if let Some((offset, max)) = scrollback_info {
            let indicator = format!("[{}/{}]", offset, max);
            block = block.title(
                Line::from(Span::styled(
                    indicator,
                    Style::default()
                        .fg(RatColor::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))
                .right_aligned(),
            );
        }
        block
    };

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    // Render cell grid content inside the border
    if let Some(cells) = cells_opt {
        let visible_rows = inner_area.height as usize;
        let visible_cols = inner_area.width as usize;

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
                        Span::styled(cell.ch.to_string(), style)
                    })
                    .collect();
                Line::from(spans)
            })
            .collect();

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, inner_area);

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
            let cursor_x = inner_area.x + cursor_visual_col;
            let cursor_y = inner_area.y + cursor.row;
            if cursor_x < inner_area.x + inner_area.width
                && cursor_y < inner_area.y + inner_area.height
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
                frame.render_widget(
                    Paragraph::new(vec![Line::from(cursor_span)]),
                    cursor_area,
                );

                frame.set_cursor_position((cursor_x, cursor_y));
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
    }

    #[test]
    fn to_ratatui_color_converts_rgb() {
        assert_eq!(
            to_ratatui_color(Color::Rgb(255, 128, 0)),
            RatColor::Rgb(255, 128, 0)
        );
    }

    #[test]
    fn render_shows_border_and_title() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(frame, area, None, None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Check for "Mini Terminal" in the top border row
        let top_row: String = (0..40)
            .map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            top_row.contains("Mini Terminal"),
            "Expected 'Mini Terminal' in title, got: {}",
            top_row
        );
    }

    #[test]
    fn render_shows_close_hint_in_bottom_border() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(frame, area, None, None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Check for close hint in the bottom border row
        let bottom_row: String = (0..40)
            .map(|x| {
                buf[(x, 9)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' ')
            })
            .collect();
        assert!(
            bottom_row.contains("to close"),
            "Expected close hint in bottom border, got: {}",
            bottom_row
        );
    }

    #[test]
    fn render_focused_has_yellow_border() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(frame, area, None, None, false, true, None, false); // is_focused = true
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Top-left corner should be yellow
        assert_eq!(buf[(0, 0)].fg, RatColor::Yellow);
    }

    #[test]
    fn render_unfocused_has_dark_gray_border() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(frame, area, None, None, false, false, None, false); // is_focused = false
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(0, 0)].fg, RatColor::DarkGray);
    }

    #[test]
    fn render_small_area_does_not_crash() {
        let backend = TestBackend::new(2, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 2, 2);
                render(frame, area, None, None, false, true, None, false);
            })
            .unwrap();
        // Should not panic -- the guard returns early for areas < 3x3
    }

    #[test]
    fn render_small_area_width_2_does_not_crash() {
        let backend = TestBackend::new(2, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 2, 5);
                render(frame, area, None, None, false, true, None, false);
            })
            .unwrap();
    }

    #[test]
    fn render_small_area_height_2_does_not_crash() {
        let backend = TestBackend::new(20, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 2);
                render(frame, area, None, None, false, true, None, false);
            })
            .unwrap();
    }

    #[test]
    fn render_cells_appear_inside_border() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![
            Cell {
                ch: 'X',
                ..Cell::default()
            },
            Cell {
                ch: 'Y',
                ..Cell::default()
            },
        ]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Content should appear at (1, 1) -- inside the border
        assert_eq!(buf[(1, 1)].symbol(), "X");
        assert_eq!(buf[(2, 1)].symbol(), "Y");
    }

    #[test]
    fn render_cells_with_bold_attribute() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![Cell {
            ch: 'B',
            bold: true,
            ..Cell::default()
        }]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert!(buf[(1, 1)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn render_cells_with_italic_attribute() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![Cell {
            ch: 'I',
            italic: true,
            ..Cell::default()
        }]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert!(buf[(1, 1)].modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn render_cells_with_underline_attribute() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![Cell {
            ch: 'U',
            underline: true,
            ..Cell::default()
        }]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert!(buf[(1, 1)].modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn render_cells_with_dim_attribute() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![Cell {
            ch: 'D',
            dim: true,
            ..Cell::default()
        }]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert!(buf[(1, 1)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn render_cells_with_strikethrough_attribute() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![Cell {
            ch: 'S',
            strikethrough: true,
            ..Cell::default()
        }]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert!(buf[(1, 1)].modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn render_cells_with_colors() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![Cell {
            ch: 'C',
            fg: Color::Rgb(255, 0, 0),
            bg: Color::Indexed(4),
            ..Cell::default()
        }]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(1, 1)].fg, RatColor::Rgb(255, 0, 0));
        assert_eq!(buf[(1, 1)].bg, RatColor::Indexed(4));
    }

    #[test]
    fn render_cells_with_reverse_swaps_fg_bg() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![Cell {
            ch: 'R',
            fg: Color::Indexed(1),
            bg: Color::Indexed(4),
            reverse: true,
            ..Cell::default()
        }]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(1, 1)].fg, RatColor::Indexed(4)); // was bg
        assert_eq!(buf[(1, 1)].bg, RatColor::Indexed(1)); // was fg
    }

    #[test]
    fn render_cells_with_reverse_default_uses_black_on_white() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![Cell {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            reverse: true,
            ..Cell::default()
        }]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(1, 1)].fg, RatColor::Black);
        assert_eq!(buf[(1, 1)].bg, RatColor::White);
    }

    #[test]
    fn render_cells_with_hidden_sets_fg_to_bg() {
        let backend = TestBackend::new(20, 6);
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
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(1, 1)].fg, RatColor::Indexed(4));
        assert_eq!(buf[(1, 1)].bg, RatColor::Indexed(4));
    }

    #[test]
    fn render_no_cells_renders_empty_bordered_box() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, None, None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Border should still be rendered
        let top_row: String = (0..20)
            .map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(top_row.contains("Mini Terminal"));
        // Inner area should be empty (spaces)
        assert_eq!(buf[(1, 1)].symbol(), " ");
    }

    #[test]
    fn render_cursor_visible_shows_reverse_video() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![
            Cell {
                ch: 'a',
                ..Cell::default()
            },
            Cell {
                ch: 'b',
                ..Cell::default()
            },
        ]];
        let cursor = CursorPos { row: 0, col: 1 };
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), Some(cursor), true, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Cursor at col 1, inside border => x=2, y=1
        // Default colors reversed: Black on White
        assert_eq!(buf[(2, 1)].fg, RatColor::Black);
        assert_eq!(buf[(2, 1)].bg, RatColor::White);
    }

    #[test]
    fn render_cursor_invisible_does_not_show_reverse() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![
            Cell {
                ch: 'a',
                ..Cell::default()
            },
            Cell {
                ch: 'b',
                ..Cell::default()
            },
        ]];
        let cursor = CursorPos { row: 0, col: 1 };
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), Some(cursor), false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Cursor is not visible, so 'b' at (2,1) should have default colors
        assert_eq!(buf[(2, 1)].fg, RatColor::Reset);
        assert_eq!(buf[(2, 1)].bg, RatColor::Reset);
    }

    #[test]
    fn render_wide_char_inside_border() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![
            Cell {
                ch: 'あ',
                width: 2,
                ..Cell::default()
            },
            Cell {
                ch: ' ',
                width: 0,
                ..Cell::default()
            },
            Cell {
                ch: 'b',
                width: 1,
                ..Cell::default()
            },
        ]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Inside border: content starts at (1, 1)
        assert_eq!(buf[(1, 1)].symbol(), "あ");
        assert_eq!(buf[(3, 1)].symbol(), "b");
    }

    #[test]
    fn render_multiple_rows_inside_border() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![
            vec![Cell {
                ch: 'A',
                ..Cell::default()
            }],
            vec![Cell {
                ch: 'B',
                ..Cell::default()
            }],
            vec![Cell {
                ch: 'C',
                ..Cell::default()
            }],
        ];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Rows inside border: y=1 (row0), y=2 (row1), y=3 (row2)
        assert_eq!(buf[(1, 1)].symbol(), "A");
        assert_eq!(buf[(1, 2)].symbol(), "B");
        assert_eq!(buf[(1, 3)].symbol(), "C");
    }

    #[test]
    fn render_clips_rows_to_inner_height() {
        // Area is 20x5 => border takes 2 rows, inner height = 3
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![
            vec![Cell { ch: '1', ..Cell::default() }],
            vec![Cell { ch: '2', ..Cell::default() }],
            vec![Cell { ch: '3', ..Cell::default() }],
            vec![Cell { ch: '4', ..Cell::default() }], // should be clipped
            vec![Cell { ch: '5', ..Cell::default() }], // should be clipped
        ];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 5);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Inner area: rows 1..3 (3 rows visible)
        assert_eq!(buf[(1, 1)].symbol(), "1");
        assert_eq!(buf[(1, 2)].symbol(), "2");
        assert_eq!(buf[(1, 3)].symbol(), "3");
        // Row 4 at y=4 is the bottom border, not content
    }

    #[test]
    fn render_clips_cols_to_inner_width() {
        // Area 6x5 => inner width = 4
        let backend = TestBackend::new(6, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![
            Cell { ch: 'a', ..Cell::default() },
            Cell { ch: 'b', ..Cell::default() },
            Cell { ch: 'c', ..Cell::default() },
            Cell { ch: 'd', ..Cell::default() },
            Cell { ch: 'e', ..Cell::default() }, // should be clipped
        ]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 6, 5);
                render(frame, area, Some(&cells), None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Inner cols: 1..4 (4 cols)
        assert_eq!(buf[(1, 1)].symbol(), "a");
        assert_eq!(buf[(2, 1)].symbol(), "b");
        assert_eq!(buf[(3, 1)].symbol(), "c");
        assert_eq!(buf[(4, 1)].symbol(), "d");
        // col 5 is the right border
    }

    #[test]
    fn render_cursor_with_wide_char_visual_col() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        // 'あ'(width=2) + cont(0) + 'b'(width=1)
        // cursor at col=2 -> visual col = 2 (あ takes 2 visual cols)
        let cells = vec![vec![
            Cell { ch: 'あ', width: 2, ..Cell::default() },
            Cell { ch: ' ', width: 0, ..Cell::default() },
            Cell { ch: 'b', width: 1, ..Cell::default() },
        ]];
        let cursor = CursorPos { row: 0, col: 2 };
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 6);
                render(frame, area, Some(&cells), Some(cursor), true, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Cursor should be at visual col 2, + border offset 1 = x=3
        // 'b' is at col 2, which has visual col 2, so cursor x = 1+2 = 3
        assert_eq!(buf[(3, 1)].fg, RatColor::Black);
        assert_eq!(buf[(3, 1)].bg, RatColor::White);
    }

    // === Scrollback indicator tests (Task #72) ===

    #[test]
    fn render_scrollback_indicator_visible() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let cells = vec![vec![Cell {
            ch: 'A',
            ..Cell::default()
        }]];
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(
                    frame,
                    area,
                    Some(&cells),
                    None,
                    false,
                    true,
                    Some((5, 100)),
                    false,
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let top_row: String = (0..40)
            .map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            top_row.contains("[5/100]"),
            "Expected '[5/100]' indicator, got: {}",
            top_row
        );
    }

    #[test]
    fn render_scrollback_indicator_not_visible_when_none() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(frame, area, None, None, false, true, None, false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let top_row: String = (0..40)
            .map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            !top_row.contains("["),
            "Expected no indicator, got: {}",
            top_row
        );
    }

    #[test]
    fn render_scrollback_indicator_style_is_yellow_bold() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(frame, area, None, None, false, true, Some((10, 50)), false);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Find the '[' character in the top row
        let bracket_x = (0..40).find(|&x| buf[(x, 0)].symbol() == "[");
        assert!(bracket_x.is_some(), "Expected '[' in top row");
        let x = bracket_x.unwrap();
        assert_eq!(buf[(x, 0)].fg, RatColor::Yellow);
        assert!(buf[(x, 0)].modifier.contains(Modifier::BOLD));
    }

    // === Scrollback mode UI tests ===

    #[test]
    fn render_scrollback_mode_has_lightcyan_border() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(frame, area, None, None, false, true, Some((5, 50)), true);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Top-left corner should be LightCyan
        assert_eq!(buf[(0, 0)].fg, RatColor::LightCyan);
    }

    #[test]
    fn render_scrollback_mode_shows_scrollback_title() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(frame, area, None, None, false, true, Some((5, 50)), true);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let top_row: String = (0..40)
            .map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            top_row.contains("SCROLLBACK"),
            "Expected 'SCROLLBACK' in title, got: {}",
            top_row
        );
    }

    #[test]
    fn render_scrollback_mode_shows_key_hints() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(frame, area, None, None, false, true, Some((5, 50)), true);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let bottom_row: String = (0..40)
            .map(|x| buf[(x, 9)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            bottom_row.contains("q:exit"),
            "Expected 'q:exit' in bottom border, got: {}",
            bottom_row
        );
    }

    #[test]
    fn render_scrollback_mode_not_focused_still_uses_lightcyan() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(frame, area, None, None, false, false, Some((5, 50)), true); // is_focused = false
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Border should still be LightCyan in scrollback mode regardless of focus
        assert_eq!(buf[(0, 0)].fg, RatColor::LightCyan);
    }

    #[test]
    fn render_normal_mode_still_shows_mini_terminal_title() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 10);
                render(frame, area, None, None, false, true, None, false); // in_scrollback = false
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let top_row: String = (0..40)
            .map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            top_row.contains("Mini Terminal"),
            "Expected 'Mini Terminal' in title when not in scrollback, got: {}",
            top_row
        );
    }
}
