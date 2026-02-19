use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

/// An item in the quick switcher list.
pub struct QuickSwitchItem {
    /// Original terminal index in the terminal list.
    pub terminal_index: usize,
    /// Text to display for this item (e.g., "1: Claude Code  ~/proj/cli").
    pub display_text: String,
    /// Character indices where the query matched (for highlighting).
    pub match_positions: Vec<usize>,
}

/// Calculate a centered rectangle within the given area.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

/// Build a vector of styled spans from text with highlighted match positions.
///
/// Matched characters are rendered in Cyan+Bold; non-matched in White.
/// When `is_selected` is true, all spans get a DarkGray background.
fn build_highlighted_spans(
    text: &str,
    positions: &[usize],
    is_selected: bool,
) -> Vec<Span<'static>> {
    let base_style = if is_selected {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };
    let highlight_style = if is_selected {
        Style::default()
            .fg(Color::Cyan)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    };

    let pos_set: HashSet<usize> = positions.iter().copied().collect();
    let chars: Vec<char> = text.chars().collect();

    let mut spans = Vec::new();
    let mut current_text = String::new();
    let mut current_highlight = false;

    for (i, ch) in chars.iter().enumerate() {
        let is_match = pos_set.contains(&i);
        if is_match != current_highlight {
            if !current_text.is_empty() {
                spans.push(Span::styled(
                    current_text.clone(),
                    if current_highlight {
                        highlight_style
                    } else {
                        base_style
                    },
                ));
                current_text.clear();
            }
            current_highlight = is_match;
        }
        current_text.push(*ch);
    }
    if !current_text.is_empty() {
        spans.push(Span::styled(
            current_text,
            if current_highlight {
                highlight_style
            } else {
                base_style
            },
        ));
    }

    spans
}

/// Render the Quick Switcher overlay.
///
/// Displays a centered dialog with a search input, separator, and a filtered
/// list of terminals. The selected item is highlighted with a `\u{25B8}` marker.
///
/// - `frame`: The ratatui frame to render into.
/// - `area`: The area within which to center the overlay.
/// - `query`: Current search query text.
/// - `cursor_pos`: Cursor position (char index) within the query.
/// - `items`: Filtered list of items to display.
/// - `selected_index`: Index into `items` that is currently selected.
pub fn render_quick_switcher(
    frame: &mut Frame,
    area: Rect,
    query: &str,
    cursor_pos: usize,
    items: &[QuickSwitchItem],
    selected_index: usize,
) {
    // Calculate dialog dimensions
    // Width: 50% of area.width, clamped to min 40, max 60
    let dialog_width = (area.width / 2).clamp(40, 60).min(area.width);
    // Height: min(items.len() + 5, 15)
    // +5 accounts for border(2) + query line(1) + separator(1) + footer(1)
    let dialog_height = ((items.len() + 5) as u16).min(15).min(area.height);

    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    // Clear background
    frame.render_widget(Clear, dialog_area);

    // Block with title, footer, rounded borders
    let block = Block::default()
        .title(
            Line::from(Span::styled(
                " Quick Switch ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ))
            .centered(),
        )
        .title_bottom(
            Line::from(Span::styled(
                " \u{2191}\u{2193} select  Enter confirm  Esc cancel ",
                Style::default().fg(Color::DarkGray),
            ))
            .centered(),
        )
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    // Guard against too-small terminal
    if inner.height < 3 || inner.width < 4 {
        return;
    }

    // Line 0: Query input "> {query}"
    let query_line = Line::from(vec![
        Span::styled(
            "> ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(query.to_string(), Style::default().fg(Color::White)),
    ]);
    let query_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(Paragraph::new(query_line), query_area);

    // Line 1: Separator
    let separator = "\u{2500}".repeat(inner.width as usize);
    let separator_line = Line::from(Span::styled(
        separator,
        Style::default().fg(Color::DarkGray),
    ));
    let sep_area = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    frame.render_widget(Paragraph::new(separator_line), sep_area);

    // Lines 2+: Item list
    let list_y = inner.y + 2;
    let list_height = inner.height.saturating_sub(2) as usize;

    if items.is_empty() {
        // Show "No matches" centered
        let no_match_text = "No matches";
        let pad = (inner.width as usize).saturating_sub(no_match_text.len()) / 2;
        let padded = format!("{}{}", " ".repeat(pad), no_match_text);
        let no_match_line = Line::from(Span::styled(
            padded,
            Style::default().fg(Color::DarkGray),
        ));
        if list_height > 0 {
            let no_match_area = Rect::new(inner.x, list_y, inner.width, 1);
            frame.render_widget(Paragraph::new(no_match_line), no_match_area);
        }
    } else {
        // Calculate scroll offset to keep selected_index visible
        let scroll_offset = if selected_index >= list_height {
            selected_index - list_height + 1
        } else {
            0
        };

        for (i, item) in items.iter().enumerate().skip(scroll_offset) {
            let row_index = i - scroll_offset;
            if row_index >= list_height {
                break;
            }

            let is_selected = i == selected_index;
            let prefix = if is_selected { "\u{25B8} " } else { "  " };

            let mut spans = vec![Span::styled(
                prefix.to_string(),
                if is_selected {
                    Style::default()
                        .fg(Color::Cyan)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                },
            )];

            let mut highlighted =
                build_highlighted_spans(&item.display_text, &item.match_positions, is_selected);
            spans.append(&mut highlighted);

            // If selected, fill the rest of the line with DarkGray background
            if is_selected {
                let used_width = prefix.width() + item.display_text.width();
                let remaining = (inner.width as usize).saturating_sub(used_width);
                if remaining > 0 {
                    spans.push(Span::styled(
                        " ".repeat(remaining),
                        Style::default().bg(Color::DarkGray),
                    ));
                }
            }

            let line = Line::from(spans);
            let line_area = Rect::new(inner.x, list_y + row_index as u16, inner.width, 1);
            frame.render_widget(Paragraph::new(line), line_area);
        }
    }

    // Cursor position at query input
    let display_width: usize = query.chars().take(cursor_pos).collect::<String>().width();
    let cursor_x = inner.x + 2 + display_width as u16; // 2 for "> "
    let cursor_y = inner.y;
    if cursor_x < inner.x + inner.width {
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn render_quick_switch(
        width: u16,
        height: u16,
        query: &str,
        cursor_pos: usize,
        items: &[QuickSwitchItem],
        selected_index: usize,
    ) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_quick_switcher(
                    frame,
                    frame.area(),
                    query,
                    cursor_pos,
                    items,
                    selected_index,
                );
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_to_string(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    // =========================================================================
    // Tests: QuickSwitchItem struct
    // =========================================================================

    #[test]
    fn quick_switch_item_fields_are_accessible() {
        let item = QuickSwitchItem {
            terminal_index: 2,
            display_text: "2: my-term  /home".to_string(),
            match_positions: vec![0, 3],
        };
        assert_eq!(item.terminal_index, 2);
        assert_eq!(item.display_text, "2: my-term  /home");
        assert_eq!(item.match_positions, vec![0, 3]);
    }

    #[test]
    fn quick_switch_item_with_empty_match_positions() {
        let item = QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: test".to_string(),
            match_positions: Vec::new(),
        };
        assert!(item.match_positions.is_empty());
    }

    // =========================================================================
    // Tests: centered_rect
    // =========================================================================

    #[test]
    fn centered_rect_centers_horizontally() {
        let area = Rect::new(0, 0, 100, 50);
        let result = centered_rect(40, 10, area);
        assert_eq!(result.x, 30);
        assert_eq!(result.width, 40);
    }

    #[test]
    fn centered_rect_centers_vertically() {
        let area = Rect::new(0, 0, 100, 50);
        let result = centered_rect(40, 10, area);
        assert_eq!(result.y, 20);
        assert_eq!(result.height, 10);
    }

    #[test]
    fn centered_rect_clamps_to_area_width() {
        let area = Rect::new(0, 0, 20, 50);
        let result = centered_rect(40, 10, area);
        assert_eq!(result.width, 20);
    }

    #[test]
    fn centered_rect_clamps_to_area_height() {
        let area = Rect::new(0, 0, 100, 5);
        let result = centered_rect(40, 10, area);
        assert_eq!(result.height, 5);
    }

    #[test]
    fn centered_rect_with_offset_area() {
        let area = Rect::new(10, 5, 80, 40);
        let result = centered_rect(40, 10, area);
        assert_eq!(result.x, 30); // 10 + (80-40)/2
        assert_eq!(result.y, 20); // 5 + (40-10)/2
    }

    #[test]
    fn centered_rect_when_width_equals_area() {
        let area = Rect::new(0, 0, 40, 50);
        let result = centered_rect(40, 10, area);
        assert_eq!(result.x, 0);
        assert_eq!(result.width, 40);
    }

    // =========================================================================
    // Tests: build_highlighted_spans
    // =========================================================================

    #[test]
    fn build_highlighted_spans_no_matches() {
        let spans = build_highlighted_spans("hello world", &[], false);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "hello world");
        assert_eq!(spans[0].style.fg, Some(Color::White));
    }

    #[test]
    fn build_highlighted_spans_all_matched() {
        let spans = build_highlighted_spans("abc", &[0, 1, 2], false);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "abc");
        assert_eq!(spans[0].style.fg, Some(Color::Cyan));
    }

    #[test]
    fn build_highlighted_spans_partial_match() {
        // "hello" with positions 0, 2 matched: "h" matched, "e" not, "l" matched, "lo" not
        let spans = build_highlighted_spans("hello", &[0, 2], false);
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].content.as_ref(), "h");
        assert_eq!(spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(spans[1].content.as_ref(), "e");
        assert_eq!(spans[1].style.fg, Some(Color::White));
        assert_eq!(spans[2].content.as_ref(), "l");
        assert_eq!(spans[2].style.fg, Some(Color::Cyan));
        assert_eq!(spans[3].content.as_ref(), "lo");
        assert_eq!(spans[3].style.fg, Some(Color::White));
    }

    #[test]
    fn build_highlighted_spans_selected_has_dark_gray_bg() {
        let spans = build_highlighted_spans("test", &[0], true);
        // First span "t" should have DarkGray background + Cyan fg
        assert_eq!(spans[0].style.bg, Some(Color::DarkGray));
        assert_eq!(spans[0].style.fg, Some(Color::Cyan));
        // Second span "est" should also have DarkGray background + White fg
        assert_eq!(spans[1].style.bg, Some(Color::DarkGray));
        assert_eq!(spans[1].style.fg, Some(Color::White));
    }

    #[test]
    fn build_highlighted_spans_empty_text() {
        let spans = build_highlighted_spans("", &[], false);
        assert!(spans.is_empty());
    }

    #[test]
    fn build_highlighted_spans_consecutive_matches() {
        let spans = build_highlighted_spans("abcd", &[1, 2], false);
        // "a" (white), "bc" (cyan), "d" (white)
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "a");
        assert_eq!(spans[1].content.as_ref(), "bc");
        assert_eq!(spans[2].content.as_ref(), "d");
    }

    #[test]
    fn build_highlighted_spans_first_char_highlighted() {
        let spans = build_highlighted_spans("abc", &[0], false);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), "a");
        assert_eq!(spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(spans[1].content.as_ref(), "bc");
        assert_eq!(spans[1].style.fg, Some(Color::White));
    }

    #[test]
    fn build_highlighted_spans_last_char_highlighted() {
        let spans = build_highlighted_spans("abc", &[2], false);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), "ab");
        assert_eq!(spans[0].style.fg, Some(Color::White));
        assert_eq!(spans[1].content.as_ref(), "c");
        assert_eq!(spans[1].style.fg, Some(Color::Cyan));
    }

    #[test]
    fn build_highlighted_spans_selected_unmatched_has_white_fg() {
        let spans = build_highlighted_spans("test", &[], true);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].style.fg, Some(Color::White));
        assert_eq!(spans[0].style.bg, Some(Color::DarkGray));
    }

    #[test]
    fn build_highlighted_spans_selected_matched_has_bold() {
        let spans = build_highlighted_spans("ab", &[0], true);
        assert!(
            spans[0].style.add_modifier.contains(Modifier::BOLD),
            "Highlighted span in selected item should be bold"
        );
    }

    // =========================================================================
    // Tests: render_quick_switcher
    // =========================================================================

    #[test]
    fn quick_switcher_renders_title() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        let buf = render_quick_switch(80, 24, "", 0, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Quick Switch"),
            "Expected 'Quick Switch' title in overlay. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_renders_query() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        let buf = render_quick_switch(80, 24, "test", 4, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("test"),
            "Expected query text 'test' in overlay. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_renders_query_prompt() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        let buf = render_quick_switch(80, 24, "", 0, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains('>'),
            "Expected '>' query prompt. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_renders_selected_marker() {
        let items = vec![
            QuickSwitchItem {
                terminal_index: 0,
                display_text: "1: shell".to_string(),
                match_positions: vec![],
            },
            QuickSwitchItem {
                terminal_index: 1,
                display_text: "2: vim".to_string(),
                match_positions: vec![],
            },
        ];
        let buf = render_quick_switch(80, 24, "", 0, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains('\u{25B8}'),
            "Expected selected marker '\u{25B8}' in overlay. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_renders_no_matches() {
        let buf = render_quick_switch(80, 24, "xyz", 3, &[], 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("No matches"),
            "Expected 'No matches' when items is empty. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_small_terminal_no_crash() {
        // Should not panic on a 40x12 terminal
        let _buf = render_quick_switch(40, 12, "q", 1, &[], 0);
    }

    #[test]
    fn quick_switcher_very_small_terminal_no_crash() {
        // Very small terminal - should not panic
        let _buf = render_quick_switch(10, 5, "", 0, &[], 0);
    }

    #[test]
    fn quick_switcher_zero_size_no_crash() {
        let _buf = render_quick_switch(0, 0, "", 0, &[], 0);
    }

    #[test]
    fn quick_switcher_renders_footer() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        let buf = render_quick_switch(80, 24, "", 0, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("select") && content.contains("Enter"),
            "Expected footer with 'select' and 'Enter'. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_footer_contains_esc_cancel() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        let buf = render_quick_switch(80, 24, "", 0, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Esc cancel"),
            "Expected 'Esc cancel' in footer. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_renders_separator() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        let buf = render_quick_switch(80, 24, "", 0, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains('\u{2500}'),
            "Expected separator line with '\u{2500}'. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_renders_item_text() {
        let items = vec![
            QuickSwitchItem {
                terminal_index: 0,
                display_text: "1: Claude Code".to_string(),
                match_positions: vec![],
            },
            QuickSwitchItem {
                terminal_index: 1,
                display_text: "2: vim editor".to_string(),
                match_positions: vec![],
            },
        ];
        let buf = render_quick_switch(80, 24, "", 0, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Claude Code"),
            "Expected item text 'Claude Code'. Got:\n{}",
            content,
        );
        assert!(
            content.contains("vim editor"),
            "Expected item text 'vim editor'. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_second_item_selected() {
        let items = vec![
            QuickSwitchItem {
                terminal_index: 0,
                display_text: "1: shell".to_string(),
                match_positions: vec![],
            },
            QuickSwitchItem {
                terminal_index: 1,
                display_text: "2: vim".to_string(),
                match_positions: vec![],
            },
        ];
        let buf = render_quick_switch(80, 24, "", 0, &items, 1);
        // The marker should appear on a line with "2: vim"
        let mut found = false;
        for y in 0..buf.area.height {
            let row: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if row.contains('\u{25B8}') && row.contains("vim") {
                found = true;
            }
        }
        assert!(
            found,
            "Expected marker on line with 'vim'.",
        );
    }

    #[test]
    fn quick_switcher_has_rounded_border() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        let buf = render_quick_switch(80, 24, "", 0, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains('\u{256D}'),
            "Expected rounded top-left corner. Got:\n{}",
            content,
        );
        assert!(
            content.contains('\u{256E}'),
            "Expected rounded top-right corner. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_scrolling_many_items() {
        // Create more items than can fit in the dialog
        let items: Vec<QuickSwitchItem> = (0..20)
            .map(|i| QuickSwitchItem {
                terminal_index: i,
                display_text: format!("{}: terminal {}", i + 1, i + 1),
                match_positions: vec![],
            })
            .collect();
        // Select the last item; dialog should scroll without panic
        let buf = render_quick_switch(80, 24, "", 0, &items, 19);
        let content = buffer_to_string(&buf);
        // Last item should be visible
        assert!(
            content.contains("terminal 20"),
            "Expected last item 'terminal 20' visible after scrolling. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_query_with_cursor_at_middle() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        // cursor_pos=2 in "hello" should not panic and should render correctly
        let _buf = render_quick_switch(80, 24, "hello", 2, &items, 0);
    }

    #[test]
    fn quick_switcher_empty_query_renders() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        let buf = render_quick_switch(80, 24, "", 0, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains('>'),
            "Expected '>' query prompt. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_selected_beyond_items_no_crash() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        // selected_index out of bounds should not crash
        let _buf = render_quick_switch(80, 24, "", 0, &items, 5);
    }

    #[test]
    fn quick_switcher_match_positions_highlight() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: Claude Code".to_string(),
            match_positions: vec![3, 4, 5], // "Cla" highlighted
        }];
        let buf = render_quick_switch(80, 24, "cla", 3, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("Claude Code"),
            "Expected full item text displayed. Got:\n{}",
            content,
        );
    }

    #[test]
    fn quick_switcher_width_clamped_min() {
        // With area width 50, dialog width = 50/2 = 25, but clamped to min 40
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        let _buf = render_quick_switch(50, 24, "", 0, &items, 0);
        // No panic is the test
    }

    #[test]
    fn quick_switcher_width_clamped_max() {
        // With area width 200, dialog width = 200/2 = 100, but clamped to max 60
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: shell".to_string(),
            match_positions: vec![],
        }];
        let _buf = render_quick_switch(200, 24, "", 0, &items, 0);
        // No panic is the test
    }

    #[test]
    fn quick_switcher_single_item_no_scroll() {
        let items = vec![QuickSwitchItem {
            terminal_index: 0,
            display_text: "1: only".to_string(),
            match_positions: vec![],
        }];
        let buf = render_quick_switch(80, 24, "", 0, &items, 0);
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("only"),
            "Expected single item 'only'. Got:\n{}",
            content,
        );
    }
}
