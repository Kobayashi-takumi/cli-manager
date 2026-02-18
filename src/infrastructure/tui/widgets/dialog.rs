use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

/// Calculate a centered rectangle within the given area
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

pub fn render_create_dialog(frame: &mut Frame, input: &str, cursor_pos: usize) {
    let area = frame.area();
    let dialog_area = centered_rect(30, 7, area);

    // Clear background
    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .title(" New Terminal ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Name: "),
            Span::styled(input, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  [Enter] Create  [Esc] Cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);

    // Set cursor position within the input field
    // cursor_pos is a char count; compute display width of text before cursor
    let display_width: usize = input.chars().take(cursor_pos)
        .collect::<String>()
        .width();
    let cursor_x = inner.x + 8 + display_width as u16; // "  Name: " = 8 chars
    let cursor_y = inner.y + 1; // Line 2 (0-indexed line 1)
    frame.set_cursor_position((cursor_x, cursor_y));
}

pub fn render_rename_dialog(frame: &mut Frame, input: &str, cursor_pos: usize) {
    let area = frame.area();
    let dialog_area = centered_rect(30, 7, area);

    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .title(" Rename Terminal ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Name: "),
            Span::styled(input, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  [Enter] Confirm  [Esc] Cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);

    // cursor_pos is a char count; compute display width of text before cursor
    let display_width: usize = input.chars().take(cursor_pos)
        .collect::<String>()
        .width();
    let cursor_x = inner.x + 8 + display_width as u16;
    let cursor_y = inner.y + 1;
    frame.set_cursor_position((cursor_x, cursor_y));
}

pub fn render_confirm_close_dialog(frame: &mut Frame, terminal_name: &str, is_running: bool) {
    if !is_running {
        return; // No dialog needed for exited terminals
    }

    let area = frame.area();
    let dialog_area = centered_rect(30, 8, area);

    // Clear background
    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .title(" Close Terminal? ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    let lines = vec![
        Line::from(""),
        Line::from(format!("  \"{}\" is running.", terminal_name)),
        Line::from("  Close anyway?"),
        Line::from(""),
        Line::from(Span::styled(
            "  [y] Yes    [n] No",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    // --- centered_rect tests ---

    #[test]
    fn centered_rect_centers_in_area() {
        let area = Rect::new(0, 0, 80, 24);
        let result = centered_rect(30, 7, area);

        assert_eq!(result.x, 25); // (80 - 30) / 2
        assert_eq!(result.y, 8); // (24 - 7) / 2 = 8 (integer division)
        assert_eq!(result.width, 30);
        assert_eq!(result.height, 7);
    }

    #[test]
    fn centered_rect_with_offset_origin() {
        let area = Rect::new(10, 5, 80, 24);
        let result = centered_rect(30, 7, area);

        assert_eq!(result.x, 35); // 10 + (80 - 30) / 2
        assert_eq!(result.y, 13); // 5 + (24 - 7) / 2
        assert_eq!(result.width, 30);
        assert_eq!(result.height, 7);
    }

    #[test]
    fn centered_rect_clamped_when_larger_than_area() {
        let area = Rect::new(0, 0, 20, 5);
        let result = centered_rect(30, 7, area);

        // Width and height should be clamped to area dimensions
        assert_eq!(result.width, 20);
        assert_eq!(result.height, 5);
        assert_eq!(result.x, 0); // saturating_sub(30) from 20 = 0, /2 = 0
        assert_eq!(result.y, 0);
    }

    #[test]
    fn centered_rect_exact_fit() {
        let area = Rect::new(0, 0, 30, 7);
        let result = centered_rect(30, 7, area);

        assert_eq!(result.x, 0);
        assert_eq!(result.y, 0);
        assert_eq!(result.width, 30);
        assert_eq!(result.height, 7);
    }

    #[test]
    fn centered_rect_with_odd_dimensions() {
        let area = Rect::new(0, 0, 81, 25);
        let result = centered_rect(30, 7, area);

        // (81 - 30) / 2 = 25 (integer division)
        assert_eq!(result.x, 25);
        // (25 - 7) / 2 = 9
        assert_eq!(result.y, 9);
    }

    #[test]
    fn centered_rect_zero_area() {
        let area = Rect::new(0, 0, 0, 0);
        let result = centered_rect(30, 7, area);

        assert_eq!(result.width, 0);
        assert_eq!(result.height, 0);
    }

    // --- render_create_dialog tests ---

    #[test]
    fn render_create_dialog_shows_title_and_input() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_create_dialog(frame, "my-term", 7);
            })
            .unwrap();

        let buf = terminal.backend().buffer();

        // Find the dialog content in the buffer
        let mut found_title = false;
        let mut found_name = false;
        let mut found_help = false;

        for y in 0..24u16 {
            let row: String = (0..80u16)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect();
            if row.contains("New Terminal") {
                found_title = true;
            }
            if row.contains("Name:") && row.contains("my-term") {
                found_name = true;
            }
            if row.contains("[Enter] Create") {
                found_help = true;
            }
        }

        assert!(found_title, "Expected dialog title 'New Terminal'");
        assert!(found_name, "Expected 'Name:' with input 'my-term'");
        assert!(found_help, "Expected help text '[Enter] Create'");
    }

    #[test]
    fn render_create_dialog_with_empty_input() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_create_dialog(frame, "", 0);
            })
            .unwrap();

        let buf = terminal.backend().buffer();

        let mut found_name_label = false;
        for y in 0..24u16 {
            let row: String = (0..80u16)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect();
            if row.contains("Name:") {
                found_name_label = true;
            }
        }
        assert!(found_name_label, "Expected 'Name:' label even with empty input");
    }

    // --- render_confirm_close_dialog tests ---

    #[test]
    fn render_confirm_close_dialog_shows_for_running() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_confirm_close_dialog(frame, "my-shell", true);
            })
            .unwrap();

        let buf = terminal.backend().buffer();

        let mut found_title = false;
        let mut found_name = false;
        let mut found_prompt = false;

        for y in 0..24u16 {
            let row: String = (0..80u16)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect();
            if row.contains("Close Terminal?") {
                found_title = true;
            }
            if row.contains("my-shell") && row.contains("running") {
                found_name = true;
            }
            if row.contains("[y] Yes") && row.contains("[n] No") {
                found_prompt = true;
            }
        }

        assert!(found_title, "Expected dialog title 'Close Terminal?'");
        assert!(found_name, "Expected terminal name in message");
        assert!(found_prompt, "Expected '[y] Yes [n] No' prompt");
    }

    #[test]
    fn render_confirm_close_dialog_skips_for_exited() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_confirm_close_dialog(frame, "my-shell", false);
            })
            .unwrap();

        let buf = terminal.backend().buffer();

        // Should NOT render anything (all spaces)
        let mut found_close = false;
        for y in 0..24u16 {
            let row: String = (0..80u16)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect();
            if row.contains("Close Terminal?") {
                found_close = true;
            }
        }
        assert!(!found_close, "Should not render dialog for exited terminal");
    }

    // --- render_rename_dialog tests ---

    #[test]
    fn render_rename_dialog_shows_title_and_input() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_rename_dialog(frame, "old-name", 8);
            })
            .unwrap();

        let buf = terminal.backend().buffer();

        let mut found_title = false;
        let mut found_name = false;
        let mut found_help = false;

        for y in 0..24u16 {
            let row: String = (0..80u16)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect();
            if row.contains("Rename Terminal") {
                found_title = true;
            }
            if row.contains("Name:") && row.contains("old-name") {
                found_name = true;
            }
            if row.contains("[Enter] Confirm") {
                found_help = true;
            }
        }

        assert!(found_title, "Expected dialog title 'Rename Terminal'");
        assert!(found_name, "Expected 'Name:' with input 'old-name'");
        assert!(found_help, "Expected help text '[Enter] Confirm'");
    }
}
