/// Represents a single search match position in the scrollback buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    /// Row number (scrollback buffer top = 0).
    pub row: usize,
    /// Match start column (inclusive, cell-based).
    pub col_start: usize,
    /// Match end column (exclusive, cell-based).
    pub col_end: usize,
}
