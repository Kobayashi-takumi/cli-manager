use ratatui::layout::{Constraint, Layout, Rect};

use crate::infrastructure::tui::app_runner::MINI_TERMINAL_HEIGHT;

pub struct LayoutAreas {
    pub sidebar: Rect,
    pub main_pane: Rect,
    pub mini_terminal: Option<Rect>,
}

pub fn compute_layout(area: Rect, mini_terminal_visible: bool) -> LayoutAreas {
    let chunks = Layout::horizontal([Constraint::Length(25), Constraint::Min(0)]).split(area);
    let sidebar = chunks[0];
    let main_pane = chunks[1];

    if mini_terminal_visible {
        let main_chunks = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(MINI_TERMINAL_HEIGHT),
        ]).split(main_pane);
        LayoutAreas {
            sidebar,
            main_pane: main_chunks[0],
            mini_terminal: Some(main_chunks[1]),
        }
    } else {
        LayoutAreas {
            sidebar,
            main_pane,
            mini_terminal: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Existing tests (updated with mini_terminal_visible: false) ===

    #[test]
    fn compute_layout_splits_area_with_fixed_sidebar() {
        let area = Rect::new(0, 0, 80, 24);
        let result = compute_layout(area, false);

        assert_eq!(result.sidebar.x, 0);
        assert_eq!(result.sidebar.y, 0);
        assert_eq!(result.sidebar.width, 25);
        assert_eq!(result.sidebar.height, 24);

        assert_eq!(result.main_pane.x, 25);
        assert_eq!(result.main_pane.y, 0);
        assert_eq!(result.main_pane.width, 55);
        assert_eq!(result.main_pane.height, 24);
    }

    #[test]
    fn compute_layout_with_narrow_area() {
        // When the area is narrower than the sidebar width
        let area = Rect::new(0, 0, 20, 10);
        let result = compute_layout(area, false);

        // Sidebar takes all available width (capped at area width)
        assert_eq!(result.sidebar.width, 20);
        assert_eq!(result.main_pane.width, 0);
    }

    #[test]
    fn compute_layout_with_exact_sidebar_width() {
        let area = Rect::new(0, 0, 25, 10);
        let result = compute_layout(area, false);

        assert_eq!(result.sidebar.width, 25);
        assert_eq!(result.main_pane.width, 0);
    }

    #[test]
    fn compute_layout_preserves_origin() {
        let area = Rect::new(5, 3, 80, 24);
        let result = compute_layout(area, false);

        assert_eq!(result.sidebar.x, 5);
        assert_eq!(result.sidebar.y, 3);
        assert_eq!(result.main_pane.x, 30); // 5 + 25
        assert_eq!(result.main_pane.y, 3);
    }

    #[test]
    fn compute_layout_with_large_area() {
        let area = Rect::new(0, 0, 200, 50);
        let result = compute_layout(area, false);

        assert_eq!(result.sidebar.width, 25);
        assert_eq!(result.main_pane.width, 175);
        assert_eq!(result.sidebar.height, 50);
        assert_eq!(result.main_pane.height, 50);
    }

    #[test]
    fn compute_layout_with_zero_area() {
        let area = Rect::new(0, 0, 0, 0);
        let result = compute_layout(area, false);

        assert_eq!(result.sidebar.width, 0);
        assert_eq!(result.main_pane.width, 0);
    }

    // === New tests for mini_terminal support ===

    #[test]
    fn compute_layout_mini_terminal_not_visible_matches_original() {
        // With mini_terminal_visible: false, output should be same as before
        let area = Rect::new(0, 0, 80, 24);
        let result = compute_layout(area, false);

        assert_eq!(result.sidebar, Rect::new(0, 0, 25, 24));
        assert_eq!(result.main_pane, Rect::new(25, 0, 55, 24));
        assert!(result.mini_terminal.is_none());
    }

    #[test]
    fn compute_layout_mini_terminal_visible_splits_main_pane() {
        // With visible=true on 80x24 area:
        // sidebar: 25 wide, 24 tall
        // main_pane height: 24 - MINI_TERMINAL_HEIGHT(10) = 14
        // mini_terminal: height 10, same width as original main_pane
        let area = Rect::new(0, 0, 80, 24);
        let result = compute_layout(area, true);

        // Sidebar unchanged
        assert_eq!(result.sidebar, Rect::new(0, 0, 25, 24));

        // Main pane shrinks vertically
        assert_eq!(result.main_pane.x, 25);
        assert_eq!(result.main_pane.y, 0);
        assert_eq!(result.main_pane.width, 55);
        assert_eq!(result.main_pane.height, 14); // 24 - 10

        // Mini terminal occupies the bottom
        let mini = result.mini_terminal.expect("mini_terminal should be Some");
        assert_eq!(mini.x, 25);
        assert_eq!(mini.y, 14); // 0 + 14
        assert_eq!(mini.width, 55);
        assert_eq!(mini.height, 10); // MINI_TERMINAL_HEIGHT
    }

    #[test]
    fn compute_layout_mini_terminal_visible_with_small_area() {
        // Very short area (80x12) should not panic
        let area = Rect::new(0, 0, 80, 12);
        let result = compute_layout(area, true);

        // Sidebar remains the full height
        assert_eq!(result.sidebar.height, 12);

        // Main pane + mini terminal should sum to original main_pane height (12)
        let mini = result.mini_terminal.expect("mini_terminal should be Some");
        assert_eq!(result.main_pane.height + mini.height, 12);

        // Mini terminal gets its requested height (10), main_pane gets the rest (2)
        assert_eq!(mini.height, 10);
        assert_eq!(result.main_pane.height, 2);
    }

    #[test]
    fn compute_layout_mini_terminal_rect_position() {
        // Verify the mini_terminal Rect has correct x, y, width, height
        // with a non-zero origin area
        let area = Rect::new(0, 0, 100, 30);
        let result = compute_layout(area, true);

        let mini = result.mini_terminal.expect("mini_terminal should be Some");

        // x should match main_pane x (sidebar width = 25)
        assert_eq!(mini.x, 25);
        // y = main_pane.y + main_pane.height
        assert_eq!(mini.y, result.main_pane.y + result.main_pane.height);
        // width should match main_pane width
        assert_eq!(mini.width, result.main_pane.width);
        // height should be MINI_TERMINAL_HEIGHT
        assert_eq!(mini.height, MINI_TERMINAL_HEIGHT);
    }
}
