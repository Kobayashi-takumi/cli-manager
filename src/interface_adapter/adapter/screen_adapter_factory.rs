use crate::infrastructure::screen::VteScreenAdapter;
use crate::infrastructure::screen::Vt100ScreenAdapter;

/// Creates a concrete ScreenPort implementation (VTE-based).
pub fn create_screen_adapter() -> VteScreenAdapter {
    VteScreenAdapter::new()
}

/// Creates a concrete ScreenPort implementation (vt100 crate-based).
pub fn create_vt100_screen_adapter() -> Vt100ScreenAdapter {
    Vt100ScreenAdapter::new()
}
