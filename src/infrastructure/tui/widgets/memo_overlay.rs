use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

/// Render the memo editing overlay centered within the given area.
///
/// - `area`: The area to center the overlay within (typically the main pane).
/// - `text`: Current memo text (may contain newlines).
/// - `cursor_row`: Current cursor row within the text.
/// - `cursor_col`: Current cursor column within the text.
pub fn render_memo_overlay(
    frame: &mut Frame,
    area: Rect,
    text: &str,
    cursor_row: usize,
    cursor_col: usize,
) {
    // 80% width, 60% height of the given area
    let width = (area.width as u32 * 80 / 100) as u16;
    let height = (area.height as u32 * 60 / 100).max(8) as u16;

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let overlay_area = Rect::new(x, y, width.min(area.width), height.min(area.height));

    frame.render_widget(Clear, overlay_area);

    let block = Block::default()
        .title(" Memo ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(overlay_area);
    frame.render_widget(block, overlay_area);

    // Reserve last line for hint
    let text_area_height = inner.height.saturating_sub(1);

    // Build text lines
    let text_lines: Vec<Line> = if text.is_empty() {
        vec![]
    } else {
        text.split('\n').map(|l| Line::from(l.to_string())).collect()
    };

    let paragraph = Paragraph::new(text_lines);
    let text_area = Rect::new(inner.x, inner.y, inner.width, text_area_height);
    frame.render_widget(paragraph, text_area);

    // Hint at bottom of inner area
    let hint_y = inner.y + inner.height.saturating_sub(1);
    let hint_area = Rect::new(inner.x, hint_y, inner.width, 1);
    let hint = Paragraph::new(Line::from(Span::styled(
        "Enter: save  Ctrl+J: newline  Esc: cancel",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(hint, hint_area);

    // Set cursor position — cursor_col is a char count, compute display width
    let current_line = text.split('\n').nth(cursor_row).unwrap_or("");
    let display_width: usize = current_line.chars().take(cursor_col)
        .collect::<String>()
        .width();
    let cursor_x = inner.x + display_width as u16;
    let cursor_y = inner.y + cursor_row as u16;
    if cursor_x < inner.x + inner.width && cursor_y < inner.y + text_area_height {
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn render_memo_overlay_shows_title_and_hint() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 80, 24);
                render_memo_overlay(frame, area, "hello memo", 0, 10);
            })
            .unwrap();

        let buf = terminal.backend().buffer();

        let mut found_title = false;
        let mut found_text = false;
        let mut found_hint = false;

        for y in 0..24u16 {
            let row: String = (0..80u16)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect();
            if row.contains("Memo") {
                found_title = true;
            }
            if row.contains("hello memo") {
                found_text = true;
            }
            if row.contains("Ctrl+J") {
                found_hint = true;
            }
        }

        assert!(found_title, "Expected overlay title 'Memo'");
        assert!(found_text, "Expected memo text 'hello memo'");
        assert!(found_hint, "Expected hint text containing 'Ctrl+J'");
    }

    #[test]
    fn render_memo_overlay_multiline_text() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 80, 24);
                render_memo_overlay(frame, area, "line 1\nline 2\nline 3", 2, 6);
            })
            .unwrap();

        let buf = terminal.backend().buffer();

        let mut found_line1 = false;
        let mut found_line2 = false;
        let mut found_line3 = false;

        for y in 0..24u16 {
            let row: String = (0..80u16)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect();
            if row.contains("line 1") {
                found_line1 = true;
            }
            if row.contains("line 2") {
                found_line2 = true;
            }
            if row.contains("line 3") {
                found_line3 = true;
            }
        }

        assert!(found_line1, "Expected 'line 1' in overlay");
        assert!(found_line2, "Expected 'line 2' in overlay");
        assert!(found_line3, "Expected 'line 3' in overlay");
    }

    #[test]
    fn render_memo_overlay_empty_text() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 80, 24);
                render_memo_overlay(frame, area, "", 0, 0);
            })
            .unwrap();

        let buf = terminal.backend().buffer();

        let mut found_title = false;
        let mut found_hint = false;

        for y in 0..24u16 {
            let row: String = (0..80u16)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect();
            if row.contains("Memo") {
                found_title = true;
            }
            if row.contains("Esc: cancel") {
                found_hint = true;
            }
        }

        assert!(found_title, "Expected overlay title even with empty text");
        assert!(found_hint, "Expected hint even with empty text");
    }
}
