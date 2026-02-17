use ratatui::layout::{Constraint, Layout, Rect};

pub struct LayoutAreas {
    pub sidebar: Rect,
    pub main_pane: Rect,
}

pub fn compute_layout(area: Rect) -> LayoutAreas {
    let chunks = Layout::horizontal([Constraint::Length(25), Constraint::Min(0)]).split(area);
    LayoutAreas {
        sidebar: chunks[0],
        main_pane: chunks[1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_layout_splits_area_with_fixed_sidebar() {
        let area = Rect::new(0, 0, 80, 24);
        let result = compute_layout(area);

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
        let result = compute_layout(area);

        // Sidebar takes all available width (capped at area width)
        assert_eq!(result.sidebar.width, 20);
        assert_eq!(result.main_pane.width, 0);
    }

    #[test]
    fn compute_layout_with_exact_sidebar_width() {
        let area = Rect::new(0, 0, 25, 10);
        let result = compute_layout(area);

        assert_eq!(result.sidebar.width, 25);
        assert_eq!(result.main_pane.width, 0);
    }

    #[test]
    fn compute_layout_preserves_origin() {
        let area = Rect::new(5, 3, 80, 24);
        let result = compute_layout(area);

        assert_eq!(result.sidebar.x, 5);
        assert_eq!(result.sidebar.y, 3);
        assert_eq!(result.main_pane.x, 30); // 5 + 25
        assert_eq!(result.main_pane.y, 3);
    }

    #[test]
    fn compute_layout_with_large_area() {
        let area = Rect::new(0, 0, 200, 50);
        let result = compute_layout(area);

        assert_eq!(result.sidebar.width, 25);
        assert_eq!(result.main_pane.width, 175);
        assert_eq!(result.sidebar.height, 50);
        assert_eq!(result.main_pane.height, 50);
    }

    #[test]
    fn compute_layout_with_zero_area() {
        let area = Rect::new(0, 0, 0, 0);
        let result = compute_layout(area);

        assert_eq!(result.sidebar.width, 0);
        assert_eq!(result.main_pane.width, 0);
    }
}
