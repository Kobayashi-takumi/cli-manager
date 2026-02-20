use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

pub fn render_help_overlay(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Help ")
        .title_alignment(Alignment::Center)
        .title_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .title_bottom(Line::from(" ? / Esc to close ").centered().style(
            Style::default().fg(Color::DarkGray),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Guard against too-small terminal
    if inner.height < 4 || inner.width < 10 {
        return;
    }

    // Split inner vertically: prefix line, blank, 3-column area, general section
    let sections = Layout::vertical([
        Constraint::Length(1), // "Prefix: Ctrl+b"
        Constraint::Length(1), // blank
        Constraint::Min(0),   // 3-column area
        Constraint::Length(3), // general section (header + content + blank)
    ])
    .split(inner);

    // Prefix line
    let prefix_line = Line::from(Span::styled(
        "  Prefix: Ctrl+b",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(prefix_line), sections[0]);

    // 3-column layout
    let columns = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(sections[2]);

    // TERMINAL column
    let terminal_lines = make_category(
        "TERMINAL",
        Color::Cyan,
        &[
            ("c", "New terminal"),
            ("d", "Close terminal"),
            ("r", "Rename"),
            ("m", "Memo"),
            ("`", "Mini Terminal"),
        ],
    );
    frame.render_widget(Paragraph::new(terminal_lines), columns[0]);

    // NAVIGATION column
    let nav_lines = make_category(
        "NAVIGATION",
        Color::Green,
        &[
            ("n/\u{2193}", "Next terminal"),
            ("p/\u{2191}", "Previous terminal"),
            ("1-9", "Jump to #N"),
            ("f", "Quick switch"),
            ("o", "Toggle pane"),
        ],
    );
    frame.render_widget(Paragraph::new(nav_lines), columns[1]);

    // SCROLLBACK column
    let scroll_lines = make_category(
        "SCROLLBACK",
        Color::Yellow,
        &[
            ("[", "Scrollback mode"),
            ("\u{2191}/k", "Scroll up"),
            ("\u{2193}/j", "Scroll down"),
            ("PgUp", "Page up"),
            ("PgDn", "Page down"),
            ("g", "Go to top"),
            ("G", "Go to bottom"),
            ("Esc/q", "Exit scrollback"),
            ("/", "Search"),
            ("n", "Next match"),
            ("N", "Prev match"),
            ("y", "Yank line"),
            ("Y", "Yank visible"),
            ("v", "Visual select"),
            ("V", "Visual line"),
        ],
    );
    frame.render_widget(Paragraph::new(scroll_lines), columns[2]);

    // GENERAL section (below columns)
    let general_header = Line::from(Span::styled(
        "  GENERAL",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    ));
    let general_content = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "q",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Quit    "),
        Span::styled(
            "^b",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Send Ctrl+b    "),
        Span::styled(
            "?",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" This help    "),
        Span::styled(
            "]",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Paste yank"),
    ]);
    let general = Paragraph::new(vec![general_header, general_content]);
    frame.render_widget(general, sections[3]);
}

fn make_category<'a>(
    title: &'a str,
    color: Color,
    bindings: &[(&'a str, &'a str)],
) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    // Category header
    lines.push(Line::from(Span::styled(
        format!("  {}", title),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )));
    // Keybinding lines
    for (key, desc) in bindings {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{:<5}", key),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {}", desc), Style::default().fg(Color::White)),
        ]));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn render_help(width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_help_overlay(frame, frame.area());
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_to_string(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let cell = &buf[(x, y)];
                s.push_str(cell.symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn help_overlay_renders_title() {
        let buf = render_help(80, 24);
        let content = buffer_to_string(&buf);
        assert!(content.contains("Help"), "Expected 'Help' title in overlay");
    }

    #[test]
    fn help_overlay_renders_category_headers() {
        let buf = render_help(80, 24);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("TERMINAL"),
            "Expected 'TERMINAL' category header"
        );
        assert!(
            content.contains("NAVIGATION"),
            "Expected 'NAVIGATION' category header"
        );
        assert!(
            content.contains("SCROLLBACK"),
            "Expected 'SCROLLBACK' category header"
        );
        assert!(
            content.contains("GENERAL"),
            "Expected 'GENERAL' category header"
        );
    }

    #[test]
    fn help_overlay_renders_keybindings() {
        let buf = render_help(80, 24);
        let content = buffer_to_string(&buf);
        // Terminal column
        assert!(
            content.contains("New terminal"),
            "Expected 'New terminal' keybinding"
        );
        assert!(
            content.contains("Close terminal"),
            "Expected 'Close terminal' keybinding"
        );
        assert!(content.contains("Rename"), "Expected 'Rename' keybinding");
        assert!(content.contains("Memo"), "Expected 'Memo' keybinding");
        // Navigation column
        assert!(
            content.contains("Next terminal"),
            "Expected 'Next terminal' keybinding"
        );
        assert!(
            content.contains("Toggle pane"),
            "Expected 'Toggle pane' keybinding"
        );
        // Scrollback column
        assert!(
            content.contains("Scrollback mode"),
            "Expected 'Scrollback mode' keybinding"
        );
        // General section
        assert!(content.contains("Quit"), "Expected 'Quit' keybinding");
    }

    #[test]
    fn help_overlay_renders_hint() {
        let buf = render_help(80, 24);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Esc to close"),
            "Expected 'Esc to close' hint at bottom"
        );
    }

    #[test]
    fn help_overlay_renders_prefix_info() {
        let buf = render_help(80, 24);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Prefix: Ctrl+b"),
            "Expected 'Prefix: Ctrl+b' at top"
        );
    }

    #[test]
    fn help_overlay_small_terminal_no_crash() {
        // Small terminal - should not panic
        let _buf = render_help(20, 6);
    }

    #[test]
    fn help_overlay_very_small_terminal_no_crash() {
        // Very small terminal - should not panic
        let _buf = render_help(10, 4);
    }

    #[test]
    fn help_overlay_renders_send_ctrl_b_hint() {
        let buf = render_help(80, 24);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Send Ctrl+b"),
            "Expected 'Send Ctrl+b' in GENERAL section"
        );
    }

    #[test]
    fn help_overlay_renders_this_help_hint() {
        let buf = render_help(80, 24);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("This help"),
            "Expected 'This help' in GENERAL section"
        );
    }

    #[test]
    fn help_overlay_has_rounded_border() {
        let buf = render_help(80, 24);
        // Rounded border uses '╭' (top-left) and '╮' (top-right)
        let content = buffer_to_string(&buf);
        assert!(
            content.contains('\u{256D}'),
            "Expected rounded top-left corner"
        );
        assert!(
            content.contains('\u{256E}'),
            "Expected rounded top-right corner"
        );
    }

    #[test]
    fn help_overlay_renders_navigation_keys() {
        let buf = render_help(80, 24);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Previous terminal"),
            "Expected 'Previous terminal' keybinding"
        );
        assert!(
            content.contains("Jump to #N"),
            "Expected 'Jump to #N' keybinding"
        );
    }

    #[test]
    fn help_overlay_renders_scrollback_keys() {
        let buf = render_help(80, 24);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Scroll up"),
            "Expected 'Scroll up' keybinding"
        );
        assert!(
            content.contains("Scroll down"),
            "Expected 'Scroll down' keybinding"
        );
        assert!(
            content.contains("Go to top"),
            "Expected 'Go to top' keybinding"
        );
        assert!(
            content.contains("Go to bottom"),
            "Expected 'Go to bottom' keybinding"
        );
        assert!(
            content.contains("Exit scrollback"),
            "Expected 'Exit scrollback' keybinding"
        );
    }

    #[test]
    fn help_overlay_minimum_viable_size() {
        // Just large enough to render content (no panic)
        let _buf = render_help(40, 16);
    }

    #[test]
    fn help_overlay_zero_size_no_crash() {
        let _buf = render_help(0, 0);
    }

    #[test]
    fn help_overlay_renders_mini_terminal_keybinding() {
        let buf = render_help(80, 24);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Mini Terminal"),
            "Expected 'Mini Terminal' keybinding in TERMINAL category"
        );
    }

    #[test]
    fn help_overlay_renders_quick_switch_keybinding() {
        let buf = render_help(80, 24);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Quick switch"),
            "Expected 'Quick switch' keybinding in NAVIGATION category"
        );
    }

    #[test]
    fn help_overlay_renders_search_keybindings() {
        let buf = render_help(80, 28);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Search"),
            "Expected 'Search' keybinding in SCROLLBACK category"
        );
        assert!(
            content.contains("Next match"),
            "Expected 'Next match' keybinding in SCROLLBACK category"
        );
        assert!(
            content.contains("Prev match"),
            "Expected 'Prev match' keybinding in SCROLLBACK category"
        );
    }

    #[test]
    fn help_overlay_renders_yank_keybindings() {
        let buf = render_help(80, 28);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Yank line"),
            "Expected 'Yank line' keybinding in SCROLLBACK category"
        );
        assert!(
            content.contains("Yank visible"),
            "Expected 'Yank visible' keybinding in SCROLLBACK category"
        );
    }

    #[test]
    fn help_overlay_renders_visual_keybindings() {
        let buf = render_help(80, 28);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Visual select"),
            "Expected 'Visual select' keybinding in SCROLLBACK category"
        );
        assert!(
            content.contains("Visual line"),
            "Expected 'Visual line' keybinding in SCROLLBACK category"
        );
    }

    #[test]
    fn help_overlay_renders_paste_yank_keybinding() {
        let buf = render_help(80, 28);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Paste yank"),
            "Expected 'Paste yank' keybinding in GENERAL section"
        );
    }
}
