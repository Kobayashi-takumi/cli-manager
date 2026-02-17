use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::domain::model::ManagedTerminal;

/// Lines per terminal entry: name + cwd + status + separator.
const LINES_PER_TERMINAL: usize = 4;
/// The last terminal omits the separator, so it uses 3 lines.
const LINES_LAST_TERMINAL: usize = 3;

/// Compute the total content height (in lines) for a terminal list.
fn total_content_lines(terminal_count: usize) -> usize {
    if terminal_count == 0 {
        return 0;
    }
    // All but the last have 4 lines; the last has 3 lines.
    (terminal_count - 1) * LINES_PER_TERMINAL + LINES_LAST_TERMINAL
}

/// Compute the scroll offset so the active terminal is always visible.
///
/// Returns the line offset to pass to `Paragraph::scroll()`.
pub fn compute_scroll_offset(
    terminal_count: usize,
    active_index: Option<usize>,
    visible_height: u16,
    current_offset: usize,
) -> usize {
    let Some(active) = active_index else {
        return 0;
    };
    if terminal_count == 0 {
        return 0;
    }

    let visible = visible_height as usize;
    let total = total_content_lines(terminal_count);

    // If everything fits, no scrolling needed
    if total <= visible {
        return 0;
    }

    // Start line of the active terminal
    let active_start = active * LINES_PER_TERMINAL;
    // End line (exclusive) of the active terminal
    let active_end = if active == terminal_count - 1 {
        active_start + LINES_LAST_TERMINAL
    } else {
        active_start + LINES_PER_TERMINAL
    };

    // Scroll up if active is above visible area
    if active_start < current_offset {
        return active_start;
    }

    // Scroll down if active is below visible area
    if active_end > current_offset + visible {
        return active_end.saturating_sub(visible);
    }

    // Otherwise keep current offset, but clamp to valid range
    let max_offset = total.saturating_sub(visible);
    current_offset.min(max_offset)
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    terminals: &[ManagedTerminal],
    active_index: Option<usize>,
    is_focused: bool,
    scroll_offset: usize,
    dynamic_cwds: &[Option<String>],
) {
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let block = Block::default()
        .title(format!("Terminals  {}", terminals.len()))
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split inner into content area (scrollable) and help area (fixed at bottom)
    let chunks = Layout::vertical([
        Constraint::Min(0),    // content area
        Constraint::Length(2), // help hints (always visible)
    ])
    .split(inner);
    let content_area = chunks[0];
    let help_area = chunks[1];

    let mut lines: Vec<Line> = Vec::new();

    for (i, terminal) in terminals.iter().enumerate() {
        let is_active = active_index == Some(i);
        let style = if is_active {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        // Line 1: icon + display name + notification mark
        let icon = terminal.status().icon();
        let notification_mark = if terminal.has_unread_notification() {
            " *"
        } else {
            ""
        };
        let name_style = if terminal.has_unread_notification() {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
                .bg(if is_active { Color::DarkGray } else { Color::Reset })
        } else {
            style
        };
        let line1 = Line::from(vec![Span::styled(
            format!("{} {}{}", icon, terminal.display_name(), notification_mark),
            name_style,
        )]);
        lines.push(line1);

        // Line 2: cwd (truncated to fit sidebar width)
        let max_width = inner.width.saturating_sub(2) as usize;
        let cwd_str = dynamic_cwds.get(i)
            .and_then(|c| c.as_deref())
            .map(|s| s.to_string())
            .unwrap_or_else(|| terminal.cwd().display().to_string());
        let cwd_display = if cwd_str.len() > max_width {
            format!("  ...{}", &cwd_str[cwd_str.len() - (max_width - 5)..])
        } else {
            format!("  {}", cwd_str)
        };
        lines.push(Line::from(Span::styled(cwd_display, style)));

        // Line 3: status text
        let status_text = format!("  {}", terminal.status().status_text());
        lines.push(Line::from(Span::styled(status_text, style)));

        // Separator line (except after last item)
        if i < terminals.len() - 1 {
            lines.push(Line::from("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"));
        }
    }

    // Render terminal list with scroll offset
    let total_lines = lines.len();
    let paragraph = Paragraph::new(lines).scroll((scroll_offset as u16, 0));
    frame.render_widget(paragraph, content_area);

    // Render scrollbar if content overflows
    let visible_height = content_area.height as usize;
    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(total_lines.saturating_sub(visible_height))
            .position(scroll_offset);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            content_area,
            &mut scrollbar_state,
        );
    }

    // Help hints (always visible in fixed area)
    let help_lines = vec![
        Line::from(Span::styled(
            "^t c:New d:Del o:Pane q:Quit",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "\u{2191}\u{2193}:Sel",
            Style::default().add_modifier(Modifier::DIM),
        )),
    ];
    let help_paragraph = Paragraph::new(help_lines);
    frame.render_widget(help_paragraph, help_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::domain::primitive::TerminalId;

    fn create_terminal(id: u32, name: &str) -> ManagedTerminal {
        ManagedTerminal::new(
            TerminalId::new(id),
            name.to_string(),
            PathBuf::from("/home/user"),
        )
    }

    fn create_exited_terminal(id: u32, name: &str, exit_code: i32) -> ManagedTerminal {
        let mut t = create_terminal(id, name);
        t.mark_exited(exit_code);
        t
    }

    #[test]
    fn render_empty_terminal_list() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals: Vec<ManagedTerminal> = vec![];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, None, false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        // Verify the block title contains the terminal count
        let buf = terminal.backend().buffer();
        let title_row: String = (0..30).map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' ')).collect();
        assert!(title_row.contains("Terminals"));
        assert!(title_row.contains("0"));
    }

    #[test]
    fn render_single_running_terminal() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![create_terminal(1, "test-shell")];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, Some(0), false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Check that the buffer contains terminal content (icon + name)
        let row1: String = (0..30).map(|x| buf[(x, 1)].symbol().chars().next().unwrap_or(' ')).collect();
        assert!(row1.contains("1: test-shell"), "Expected display_name in row1, got: {}", row1);
    }

    #[test]
    fn render_exited_terminal_shows_exit_code() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![create_exited_terminal(1, "done", 0)];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, Some(0), false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Status line should show "exited (0)"
        let row3: String = (0..30).map(|x| buf[(x, 3)].symbol().chars().next().unwrap_or(' ')).collect();
        assert!(row3.contains("exited (0)"), "Expected 'exited (0)' in row3, got: {}", row3);
    }

    #[test]
    fn render_multiple_terminals_with_separator() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![
            create_terminal(1, "first"),
            create_terminal(2, "second"),
        ];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, Some(0), false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // After 3 lines of first terminal, there should be a separator line
        let separator_row: String = (0..30).map(|x| buf[(x, 4)].symbol().chars().next().unwrap_or(' ')).collect();
        assert!(separator_row.contains("\u{2500}"), "Expected separator line with box-drawing char, got: {}", separator_row);
    }

    #[test]
    fn render_help_hints_at_bottom() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals: Vec<ManagedTerminal> = vec![];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, None, false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Help hints should be near the bottom of the area
        // Last two rows inside the block border (row 18 and 17, since row 19 is bottom border)
        let help_row1: String = (0..30).map(|x| buf[(x, 17)].symbol().chars().next().unwrap_or(' ')).collect();
        let help_row2: String = (0..30).map(|x| buf[(x, 18)].symbol().chars().next().unwrap_or(' ')).collect();
        assert!(help_row1.contains("^t c:New"), "Expected help hint in row17, got: {}", help_row1);
        assert!(help_row2.contains("Sel"), "Expected help hint in row18, got: {}", help_row2);
    }

    #[test]
    fn render_active_terminal_has_dark_gray_background() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![
            create_terminal(1, "first"),
            create_terminal(2, "second"),
        ];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, Some(0), false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // The active terminal (index 0) should have DarkGray background
        // Check cell at (1, 1) - inside the border, first char of first terminal line
        let cell = &buf[(1, 1)];
        assert_eq!(cell.bg, Color::DarkGray);
    }

    #[test]
    fn render_inactive_terminal_has_default_background() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![
            create_terminal(1, "first"),
            create_terminal(2, "second"),
        ];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, Some(0), false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // The second terminal (index 1) is inactive; its lines start at row 5 (after separator at row 4)
        let cell = &buf[(1, 5)];
        assert_eq!(cell.bg, Color::Reset);
    }

    #[test]
    fn render_focused_sidebar_has_cyan_border() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals: Vec<ManagedTerminal> = vec![];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, None, true, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Top-left corner border cell should have Cyan foreground
        let cell = &buf[(0, 0)];
        assert_eq!(cell.fg, Color::Cyan);
    }

    #[test]
    fn render_unfocused_sidebar_has_default_border() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals: Vec<ManagedTerminal> = vec![];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, None, false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Top-left corner border cell should have Reset (default) foreground
        let cell = &buf[(0, 0)];
        assert_eq!(cell.fg, Color::Reset);
    }

    // ===== compute_scroll_offset tests =====

    #[test]
    fn compute_scroll_offset_no_active_returns_zero() {
        assert_eq!(compute_scroll_offset(5, None, 20, 0), 0);
    }

    #[test]
    fn compute_scroll_offset_no_terminals_returns_zero() {
        assert_eq!(compute_scroll_offset(0, Some(0), 20, 0), 0);
    }

    #[test]
    fn compute_scroll_offset_all_fit_returns_zero() {
        // 2 terminals = 4 + 3 = 7 lines, visible = 20 → fits
        assert_eq!(compute_scroll_offset(2, Some(1), 20, 0), 0);
    }

    #[test]
    fn compute_scroll_offset_scrolls_down_to_active() {
        // 5 terminals: 4*4 + 3 = 19 lines total, visible = 10
        // Active = 4 (last), starts at line 16, ends at 19
        // current_offset = 0 → active_end (19) > 0 + 10 → scroll to 19 - 10 = 9
        assert_eq!(compute_scroll_offset(5, Some(4), 10, 0), 9);
    }

    #[test]
    fn compute_scroll_offset_scrolls_up_to_active() {
        // 5 terminals, visible = 10, current_offset = 12
        // Active = 1, starts at line 4
        // 4 < 12 → scroll up to 4
        assert_eq!(compute_scroll_offset(5, Some(1), 10, 12), 4);
    }

    #[test]
    fn compute_scroll_offset_preserves_offset_when_active_visible() {
        // 5 terminals: 19 lines total, visible = 10, current_offset = 4
        // Active = 2, starts at line 8, ends at 12
        // 8 >= 4 and 12 <= 4 + 10 = 14 → keep offset 4
        assert_eq!(compute_scroll_offset(5, Some(2), 10, 4), 4);
    }

    #[test]
    fn compute_scroll_offset_last_item_no_separator() {
        // 3 terminals: 4 + 4 + 3 = 11 lines, visible = 8
        // Active = 2 (last), starts at line 8, ends at 11
        // 11 > 0 + 8 → scroll to 11 - 8 = 3
        assert_eq!(compute_scroll_offset(3, Some(2), 8, 0), 3);
    }

    // ===== Scroll rendering tests =====

    #[test]
    fn render_with_scroll_offset_clips_top_items() {
        let backend = TestBackend::new(30, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![
            create_terminal(1, "first"),
            create_terminal(2, "second"),
            create_terminal(3, "third"),
        ];

        // Scroll offset = 4 should skip the first terminal (4 lines: name+cwd+status+separator)
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 12);
                render(frame, area, &terminals, Some(2), false, 4, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Row 1 (first content row inside border) should show "second", not "first"
        let row1: String = (0..30)
            .map(|x| buf[(x, 1)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            row1.contains("2: second"),
            "Expected '2: second' after scroll, got: {}",
            row1
        );
        // "first" should NOT appear in row 1
        assert!(!row1.contains("1: first"), "First terminal should be scrolled away");
    }

    #[test]
    fn render_help_hints_always_visible_when_scrolling() {
        let backend = TestBackend::new(30, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        // 3 terminals = 11 lines of content, area height = 12 (10 inner)
        let terminals = vec![
            create_terminal(1, "first"),
            create_terminal(2, "second"),
            create_terminal(3, "third"),
        ];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 12);
                render(frame, area, &terminals, Some(2), false, 4, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Help hints should be in the last 2 rows inside the border (rows 9 and 10, border at 11)
        let help_row1: String = (0..30)
            .map(|x| buf[(x, 9)].symbol().chars().next().unwrap_or(' '))
            .collect();
        let help_row2: String = (0..30)
            .map(|x| buf[(x, 10)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            help_row1.contains("^t c:New"),
            "Expected help hint when scrolling, got: {}",
            help_row1
        );
        assert!(
            help_row2.contains("Sel"),
            "Expected help hint when scrolling, got: {}",
            help_row2
        );
    }

    #[test]
    fn render_scrollbar_appears_when_content_overflows() {
        let backend = TestBackend::new(30, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        // 3 terminals = 11 lines, area inner height = 6 (8 - 2 border), content area = 4 (6 - 2 help)
        // 11 > 4, so scrollbar should appear
        let terminals = vec![
            create_terminal(1, "first"),
            create_terminal(2, "second"),
            create_terminal(3, "third"),
        ];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 8);
                render(frame, area, &terminals, Some(0), false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // The scrollbar renders on the right edge of the content area (col 28, inside border)
        // Check that at least one cell in the rightmost content column has a scrollbar character
        let right_col = 28; // inner right edge
        let has_scrollbar_char = (1..5).any(|y| {
            let sym = buf[(right_col, y)].symbol();
            // Scrollbar uses block chars like ▐, █, ▀, ▄, or similar
            sym != " " && sym != "│"
        });
        assert!(
            has_scrollbar_char,
            "Expected scrollbar characters on right edge when content overflows"
        );
    }

    #[test]
    fn render_no_scrollbar_when_all_fit() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        // 1 terminal = 3 lines, area inner height = 18, content area = 16 → fits
        let terminals = vec![create_terminal(1, "solo")];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, Some(0), false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // The right edge column inside border should have no scrollbar chars
        let right_col = 28;
        let has_scrollbar_char = (1..17).any(|y| {
            let sym = buf[(right_col, y)].symbol();
            sym != " " && sym != "│"
        });
        assert!(
            !has_scrollbar_char,
            "Expected no scrollbar when content fits within visible area"
        );
    }

    // ===== Dynamic CWD tests =====

    #[test]
    fn render_dynamic_cwd_overrides_static_cwd() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![create_terminal(1, "test-shell")];
        let dynamic_cwds = vec![Some("/new/path".to_string())];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, Some(0), false, 0, &dynamic_cwds);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Row 2 inside border should show dynamic cwd
        let row2: String = (0..30)
            .map(|x| buf[(x, 2)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(row2.contains("/new/path"), "Expected dynamic cwd in row2, got: {}", row2);
    }

    #[test]
    fn render_dynamic_cwd_none_falls_back_to_static() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![create_terminal(1, "test-shell")];
        let dynamic_cwds = vec![None];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, Some(0), false, 0, &dynamic_cwds);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Row 2 inside border should show static cwd from ManagedTerminal
        let row2: String = (0..30)
            .map(|x| buf[(x, 2)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(row2.contains("/home/user"), "Expected static cwd in row2, got: {}", row2);
    }

    #[test]
    fn render_mixed_dynamic_and_static_cwds() {
        let backend = TestBackend::new(30, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![
            create_terminal(1, "first"),
            create_terminal(2, "second"),
        ];
        let dynamic_cwds = vec![Some("/dynamic/path".to_string()), None];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 30, 20);
                render(frame, area, &terminals, Some(0), false, 0, &dynamic_cwds);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // First terminal (row 2) should show dynamic cwd
        let row2: String = (0..30)
            .map(|x| buf[(x, 2)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(row2.contains("/dynamic/path"), "Expected dynamic cwd for first terminal, got: {}", row2);

        // Second terminal (row 6, after separator at row 4-5) should show static cwd
        let row6: String = (0..30)
            .map(|x| buf[(x, 6)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(row6.contains("/home/user"), "Expected static cwd for second terminal, got: {}", row6);
    }

    // ===== Notification display tests =====

    fn create_notified_terminal(id: u32, name: &str) -> ManagedTerminal {
        let mut t = create_terminal(id, name);
        t.set_notification(crate::domain::primitive::NotificationEvent::Bell);
        t
    }

    #[test]
    fn render_notification_mark_shown_when_unread() {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![create_notified_terminal(1, "shell")];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 20);
                render(frame, area, &terminals, None, false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Row 1 (first content row inside border) should contain " *" notification mark
        let row1: String = (0..40)
            .map(|x| buf[(x, 1)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            row1.contains("*"),
            "Expected notification mark '*' in row1 for unread terminal, got: {}",
            row1
        );
    }

    #[test]
    fn render_no_notification_mark_when_no_unread() {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![create_terminal(1, "shell")];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 20);
                render(frame, area, &terminals, None, false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Row 1 should NOT contain "*"
        let row1: String = (0..40)
            .map(|x| buf[(x, 1)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            !row1.contains("*"),
            "Expected no notification mark '*' in row1 for terminal without notification, got: {}",
            row1
        );
    }

    #[test]
    fn render_notification_terminal_name_has_yellow_foreground() {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![create_notified_terminal(1, "shell")];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 20);
                render(frame, area, &terminals, None, false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Find the cell with '1' (start of display_name "1: shell") in row 1
        // The name span cells should have Yellow foreground
        // We check a cell that is part of the name text (e.g., the 's' in "shell")
        // Row 1 inside border: find position of name text
        let mut found_yellow = false;
        for x in 1..39 {
            let cell = &buf[(x, 1)];
            let ch = cell.symbol().chars().next().unwrap_or(' ');
            if ch == 's' || ch == 'h' || ch == 'e' || ch == 'l' {
                if cell.fg == Color::Yellow {
                    found_yellow = true;
                    break;
                }
            }
        }
        assert!(
            found_yellow,
            "Expected Yellow foreground on notification terminal name"
        );
    }

    #[test]
    fn render_no_notification_terminal_name_not_yellow() {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![create_terminal(1, "shell")];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 20);
                render(frame, area, &terminals, None, false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Name cells should NOT have Yellow foreground
        for x in 1..39 {
            let cell = &buf[(x, 1)];
            let ch = cell.symbol().chars().next().unwrap_or(' ');
            if ch == 's' || ch == 'h' || ch == 'e' || ch == 'l' {
                assert_ne!(
                    cell.fg,
                    Color::Yellow,
                    "Expected non-Yellow foreground on non-notification terminal name at x={}",
                    x
                );
            }
        }
    }

    #[test]
    fn render_mixed_notification_and_normal_terminals() {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![
            create_notified_terminal(1, "notified"),
            create_terminal(2, "normal"),
        ];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 20);
                render(frame, area, &terminals, Some(1), false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();

        // First terminal (row 1) should have "*" mark
        let row1: String = (0..40)
            .map(|x| buf[(x, 1)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            row1.contains("*"),
            "Expected '*' for notified terminal in row1, got: {}",
            row1
        );

        // Second terminal (row 5, after separator at row 4) should NOT have "*" mark
        let row5: String = (0..40)
            .map(|x| buf[(x, 5)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            !row5.contains("*"),
            "Expected no '*' for normal terminal in row5, got: {}",
            row5
        );
    }

    #[test]
    fn render_active_and_notified_terminal_has_dark_gray_background() {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let terminals = vec![create_notified_terminal(1, "shell")];

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 20);
                render(frame, area, &terminals, Some(0), false, 0, &vec![None; terminals.len()]);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        // Row 1: name row. Active+notified should have Yellow fg AND DarkGray bg
        let row1: String = (0..40)
            .map(|x| buf[(x, 1)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            row1.contains("*"),
            "Expected notification mark '*' for active+notified terminal, got: {}",
            row1
        );
        // Check that name cells have DarkGray background
        for x in 1..39 {
            let cell = &buf[(x, 1)];
            let ch = cell.symbol().chars().next().unwrap_or(' ');
            if ch != ' ' && ch != '│' {
                assert_eq!(
                    cell.bg,
                    Color::DarkGray,
                    "Expected DarkGray bg for active+notified terminal at x={}, ch='{}', got bg={:?}",
                    x, ch, cell.bg
                );
                assert_eq!(
                    cell.fg,
                    Color::Yellow,
                    "Expected Yellow fg for active+notified terminal at x={}, ch='{}', got fg={:?}",
                    x, ch, cell.fg
                );
                break;
            }
        }
    }
}
