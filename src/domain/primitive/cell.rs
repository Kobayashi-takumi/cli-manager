#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub underline: bool,
    pub italic: bool,
    pub dim: bool,
    pub reverse: bool,
    pub strikethrough: bool,
    pub hidden: bool,
    pub width: u8, // 1 for normal, 2 for wide, 0 for wide-char continuation
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            underline: false,
            italic: false,
            dim: false,
            reverse: false,
            strikethrough: false,
            hidden: false,
            width: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CursorPos {
    pub row: u16,
    pub col: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_default_has_all_attributes_false() {
        let cell = Cell::default();
        assert_eq!(cell.ch, ' ');
        assert_eq!(cell.fg, Color::Default);
        assert_eq!(cell.bg, Color::Default);
        assert!(!cell.bold);
        assert!(!cell.underline);
        assert!(!cell.italic);
        assert!(!cell.dim);
        assert!(!cell.reverse);
        assert!(!cell.strikethrough);
        assert!(!cell.hidden);
    }

    #[test]
    fn cell_new_attributes_can_be_set() {
        let cell = Cell {
            ch: 'A',
            fg: Color::Indexed(1),
            bg: Color::Default,
            bold: true,
            underline: false,
            italic: true,
            dim: false,
            reverse: true,
            strikethrough: false,
            hidden: true,
            width: 1,
        };
        assert_eq!(cell.ch, 'A');
        assert!(cell.bold);
        assert!(cell.italic);
        assert!(cell.reverse);
        assert!(cell.hidden);
        assert!(!cell.underline);
        assert!(!cell.dim);
        assert!(!cell.strikethrough);
    }

    #[test]
    fn cell_default_width_is_one() {
        let cell = Cell::default();
        assert_eq!(cell.width, 1);
    }

    #[test]
    fn cell_width_can_be_set_to_zero_for_continuation() {
        let cell = Cell {
            width: 0,
            ..Cell::default()
        };
        assert_eq!(cell.width, 0);
    }

    #[test]
    fn cell_width_can_be_set_to_two_for_wide_char() {
        let cell = Cell {
            ch: '\u{3042}', // hiragana 'あ'
            width: 2,
            ..Cell::default()
        };
        assert_eq!(cell.ch, 'あ');
        assert_eq!(cell.width, 2);
    }
}
