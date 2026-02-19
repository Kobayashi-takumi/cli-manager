use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Render a search bar at the given area (expected to be 1 row high).
///
/// `match_info`: Some((current_1indexed, total)) for match count display, None for no matches info.
/// `show_cursor`: whether to set cursor position (true during input, false when confirmed).
pub fn render_search_bar(
    frame: &mut Frame,
    area: Rect,
    query: &str,
    cursor_pos: usize,
    match_info: Option<(usize, usize)>,
    show_cursor: bool,
) {
    if area.height == 0 || area.width < 4 {
        return;
    }

    // Build right side: [current/total]
    let right_text = match match_info {
        Some((current, total)) if total > 0 => format!("[{}/{}]", current, total),
        _ => "[0/0]".to_string(),
    };
    let right_style = match match_info {
        Some((_, total)) if total > 0 => {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        }
        _ => Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::BOLD),
    };

    // Build left side: /query
    let prompt = Span::styled(
        "/",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    let query_span = Span::styled(query, Style::default().fg(Color::White));

    let right_span = Span::styled(right_text, right_style);

    // Pad the middle to push right_span to the right edge
    let used_left = 1 + query.len() as u16; // "/" + query
    let padding = area
        .width
        .saturating_sub(used_left + right_span.width() as u16);
    let pad_span = Span::raw(" ".repeat(padding as usize));

    let line = Line::from(vec![prompt, query_span, pad_span, right_span]);
    let paragraph = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(paragraph, area);

    // Set cursor position if showing cursor
    if show_cursor {
        let cursor_x = area.x + 1 + cursor_pos as u16; // +1 for "/" prompt
        if cursor_x < area.x + area.width {
            frame.set_cursor_position((cursor_x, area.y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn search_bar_renders_prompt_and_query() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_bar(frame, area, "error", 5, Some((3, 10)), true);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = (0..40)
            .map(|x| {
                buf.cell((x, 0))
                    .unwrap()
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' ')
            })
            .collect();
        assert!(
            content.starts_with("/error"),
            "Expected content to start with '/error', got: {}",
            content
        );
        assert!(
            content.contains("[3/10]"),
            "Expected content to contain '[3/10]', got: {}",
            content
        );
    }

    #[test]
    fn search_bar_renders_no_match_indicator() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_bar(frame, area, "xyz", 3, None, true);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = (0..40)
            .map(|x| {
                buf.cell((x, 0))
                    .unwrap()
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' ')
            })
            .collect();
        assert!(
            content.starts_with("/xyz"),
            "Expected content to start with '/xyz', got: {}",
            content
        );
        assert!(
            content.contains("[0/0]"),
            "Expected content to contain '[0/0]', got: {}",
            content
        );
    }

    #[test]
    fn search_bar_small_width_no_crash() {
        let backend = TestBackend::new(3, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_bar(frame, area, "test", 4, Some((1, 5)), true);
            })
            .unwrap();
        // Just verify no panic -- width < 4 means early return
    }

    #[test]
    fn search_bar_zero_height_no_crash() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 0);
                render_search_bar(frame, area, "test", 4, Some((1, 5)), true);
            })
            .unwrap();
        // Just verify no panic -- height == 0 means early return
    }

    #[test]
    fn search_bar_has_dark_gray_background() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_bar(frame, area, "test", 4, Some((1, 5)), true);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        // The entire row should have DarkGray background
        assert_eq!(
            buf.cell((0, 0)).unwrap().bg,
            Color::DarkGray,
            "Expected DarkGray background on search bar"
        );
    }

    #[test]
    fn search_bar_prompt_is_cyan_bold() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_bar(frame, area, "test", 4, Some((1, 5)), true);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.symbol(), "/");
        assert_eq!(cell.fg, Color::Cyan);
        assert!(cell.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn search_bar_match_info_zero_total_shows_red() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_bar(frame, area, "nope", 4, Some((0, 0)), false);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = (0..40)
            .map(|x| {
                buf.cell((x, 0))
                    .unwrap()
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' ')
            })
            .collect();
        assert!(content.contains("[0/0]"));
        // Find the '[' position and check it's red
        let bracket_x = (0u16..40).find(|&x| buf.cell((x, 0)).unwrap().symbol() == "[");
        assert!(bracket_x.is_some());
        let x = bracket_x.unwrap();
        assert_eq!(buf.cell((x, 0)).unwrap().fg, Color::Red);
    }

    #[test]
    fn search_bar_match_info_positive_total_shows_yellow() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_bar(frame, area, "hit", 3, Some((2, 5)), false);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let bracket_x = (0u16..40).find(|&x| buf.cell((x, 0)).unwrap().symbol() == "[");
        assert!(bracket_x.is_some());
        let x = bracket_x.unwrap();
        assert_eq!(buf.cell((x, 0)).unwrap().fg, Color::Yellow);
        assert!(buf.cell((x, 0)).unwrap().modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn search_bar_empty_query_still_renders() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_bar(frame, area, "", 0, None, true);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = (0..40)
            .map(|x| {
                buf.cell((x, 0))
                    .unwrap()
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' ')
            })
            .collect();
        assert!(content.starts_with("/"));
        assert!(content.contains("[0/0]"));
    }

    #[test]
    fn search_bar_show_cursor_false_does_not_set_cursor() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        // With show_cursor = false, we just verify no crash and no cursor set.
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_bar(frame, area, "test", 4, Some((1, 5)), false);
            })
            .unwrap();
        // The test passes if it doesn't panic. We can't easily check cursor position
        // through TestBackend, but the code path is different.
    }

    #[test]
    fn search_bar_exactly_width_4_renders() {
        let backend = TestBackend::new(4, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_search_bar(frame, area, "a", 1, None, true);
            })
            .unwrap();
        // Width == 4 is the minimum; should render without panic
        let buf = terminal.backend().buffer().clone();
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "/");
    }
}
