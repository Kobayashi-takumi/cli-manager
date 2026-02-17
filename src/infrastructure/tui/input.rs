use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::interface_adapter::controller::tui_controller::AppAction;

/// Represents the current mode of the input handler state machine.
///
/// - `Normal`: all key presses are forwarded to the active terminal.
/// - `PrefixWait(Instant)`: Ctrl+t was pressed; the handler waits for a
///   command key. The `Instant` records when we entered prefix mode so we
///   can detect a 1-second timeout.
/// - `DialogInput`: a dialog is active; the input handler yields `None` and
///   lets the dialog layer consume the keys.
pub enum InputMode {
    Normal,
    PrefixWait(Instant),
    DialogInput,
}

/// Converts crossterm `KeyEvent`s into `AppAction`s using a prefix-key state
/// machine (similar to tmux's Ctrl+b).
///
/// The prefix key is **Ctrl+t**. Pressing it transitions from `Normal` to
/// `PrefixWait`. A subsequent command key (e.g. `c`, `d`, `n`, `p`, `1`-`9`)
/// produces the corresponding `AppAction` and transitions back to `Normal`.
/// If no command key arrives within 1 second, or an unrecognised key is
/// pressed, the handler cancels and returns to `Normal`.
pub struct InputHandler {
    mode: InputMode,
    application_cursor_keys: bool,
}

impl InputHandler {
    pub fn new() -> Self {
        Self {
            mode: InputMode::Normal,
            application_cursor_keys: false,
        }
    }

    /// Read-only accessor for the current input mode.
    pub fn mode(&self) -> &InputMode {
        &self.mode
    }

    /// Replace the current input mode. Used by external layers (e.g. dialog
    /// open/close) to switch to/from `DialogInput`.
    pub fn set_mode(&mut self, mode: InputMode) {
        self.mode = mode;
    }

    /// Set whether application cursor keys mode (DECCKM) is active.
    ///
    /// When enabled, arrow keys send `ESC O A/B/C/D` instead of `ESC [ A/B/C/D`,
    /// and Home/End send `ESC O H`/`ESC O F` instead of `ESC [ H`/`ESC [ F`.
    pub fn set_application_cursor_keys(&mut self, enabled: bool) {
        self.application_cursor_keys = enabled;
    }

    /// Main entry point: translate a `KeyEvent` into an optional `AppAction`.
    ///
    /// Returns `None` when the key should be silently consumed (e.g. entering
    /// prefix mode, cancelling an unrecognised prefix command, or while a
    /// dialog is active).
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        match &self.mode {
            InputMode::Normal => self.handle_normal(key),
            InputMode::PrefixWait(_) => self.handle_prefix(key),
            InputMode::DialogInput => None,
        }
    }

    /// Called on every tick to detect prefix-mode timeout.
    ///
    /// If the handler has been in `PrefixWait` for >= 1 second, it transitions
    /// back to `Normal` and sends the literal Ctrl+t byte (`0x14`) to the
    /// active terminal (so the user's delayed Ctrl+t is not silently lost).
    pub fn check_timeout(&mut self) -> Option<AppAction> {
        if let InputMode::PrefixWait(since) = &self.mode
            && since.elapsed().as_millis() > 1000
        {
            self.mode = InputMode::Normal;
            return Some(AppAction::WriteToActive(vec![0x14]));
        }
        None
    }

    // =========================================================================
    // Private helpers
    // =========================================================================

    fn handle_normal(&mut self, key: KeyEvent) -> Option<AppAction> {
        // Ctrl+t -> enter prefix mode
        if key.code == KeyCode::Char('t') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.mode = InputMode::PrefixWait(Instant::now());
            return None;
        }

        // All other keys -> forward to active terminal as raw bytes
        let bytes = key_to_bytes(key, self.application_cursor_keys);
        if bytes.is_empty() {
            None
        } else {
            Some(AppAction::WriteToActive(bytes))
        }
    }

    fn handle_prefix(&mut self, key: KeyEvent) -> Option<AppAction> {
        self.mode = InputMode::Normal; // Always return to Normal

        match key.code {
            KeyCode::Char('c') if key.modifiers.is_empty() => {
                Some(AppAction::CreateTerminal { name: None })
            }
            KeyCode::Char('d') if key.modifiers.is_empty() => Some(AppAction::CloseTerminal),
            KeyCode::Char('n') if key.modifiers.is_empty() => Some(AppAction::SelectNext),
            KeyCode::Char('p') if key.modifiers.is_empty() => Some(AppAction::SelectPrev),
            KeyCode::Char(c @ '1'..='9') if key.modifiers.is_empty() => {
                Some(AppAction::SelectByIndex((c as u8 - b'1') as usize))
            }
            KeyCode::Char('q') if key.modifiers.is_empty() => Some(AppAction::Quit),
            KeyCode::Char('o') if key.modifiers.is_empty() => Some(AppAction::ToggleFocus),
            // Ctrl+t again -> send literal Ctrl+t to child process
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(AppAction::WriteToActive(vec![0x14]))
            }
            _ => None, // Cancel - unrecognised prefix command
        }
    }
}

/// Convert a `KeyEvent` to the bytes that should be sent to the pty.
///
/// When `application_cursor_keys` is true (DECCKM enabled), arrow keys send
/// `ESC O A/B/C/D` instead of `ESC [ A/B/C/D`, and Home/End send
/// `ESC O H`/`ESC O F` instead of `ESC [ H`/`ESC [ F`.
///
/// Returns an empty `Vec` for key codes that have no meaningful byte
/// representation (e.g. modifier-only presses).
fn key_to_bytes(key: KeyEvent, application_cursor_keys: bool) -> Vec<u8> {
    match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Control codes: Ctrl+a = 0x01, Ctrl+b = 0x02, ...
            let lower = c.to_ascii_lowercase();
            vec![lower as u8 - b'a' + 1]
        }
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            s.as_bytes().to_vec()
        }
        KeyCode::Enter => vec![0x0D],
        KeyCode::Backspace => vec![0x7F],
        KeyCode::Tab => vec![0x09],
        KeyCode::Esc => vec![0x1B],
        KeyCode::Up => {
            if application_cursor_keys {
                vec![0x1B, b'O', b'A']
            } else {
                vec![0x1B, b'[', b'A']
            }
        }
        KeyCode::Down => {
            if application_cursor_keys {
                vec![0x1B, b'O', b'B']
            } else {
                vec![0x1B, b'[', b'B']
            }
        }
        KeyCode::Right => {
            if application_cursor_keys {
                vec![0x1B, b'O', b'C']
            } else {
                vec![0x1B, b'[', b'C']
            }
        }
        KeyCode::Left => {
            if application_cursor_keys {
                vec![0x1B, b'O', b'D']
            } else {
                vec![0x1B, b'[', b'D']
            }
        }
        KeyCode::Home => {
            if application_cursor_keys {
                vec![0x1B, b'O', b'H']
            } else {
                vec![0x1B, b'[', b'H']
            }
        }
        KeyCode::End => {
            if application_cursor_keys {
                vec![0x1B, b'O', b'F']
            } else {
                vec![0x1B, b'[', b'F']
            }
        }
        KeyCode::Delete => vec![0x1B, b'[', b'3', b'~'],
        _ => Vec::new(),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Helpers
    // =========================================================================

    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    /// Assert that the handler is in Normal mode.
    fn assert_normal(handler: &InputHandler) {
        assert!(
            matches!(handler.mode(), InputMode::Normal),
            "expected InputMode::Normal"
        );
    }

    /// Assert that the handler is in PrefixWait mode.
    fn assert_prefix_wait(handler: &InputHandler) {
        assert!(
            matches!(handler.mode(), InputMode::PrefixWait(_)),
            "expected InputMode::PrefixWait"
        );
    }

    // =========================================================================
    // Tests: new()
    // =========================================================================

    #[test]
    fn new_starts_in_normal_mode() {
        let handler = InputHandler::new();
        assert_normal(&handler);
    }

    // =========================================================================
    // Tests: Normal mode
    // =========================================================================

    #[test]
    fn normal_regular_char_returns_write_to_active() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Char('a'), KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[b'a']));
        assert_normal(&handler);
    }

    #[test]
    fn normal_uppercase_char_returns_correct_bytes() {
        let mut handler = InputHandler::new();
        // 'A' is a single-byte character
        let key = make_key(KeyCode::Char('A'), KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[b'A']));
        assert_normal(&handler);
    }

    #[test]
    fn normal_multibyte_utf8_char_returns_correct_bytes() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Char('\u{3042}'), KeyModifiers::NONE); // 'あ' = U+3042

        let action = handler.handle_key(key);

        // 'あ' in UTF-8 is 0xE3, 0x81, 0x82
        let expected = "\u{3042}".as_bytes().to_vec();
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if *b == expected));
        assert_normal(&handler);
    }

    #[test]
    fn normal_ctrl_t_enters_prefix_wait() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Char('t'), KeyModifiers::CONTROL);

        let action = handler.handle_key(key);

        assert!(action.is_none());
        assert_prefix_wait(&handler);
    }

    #[test]
    fn normal_enter_returns_carriage_return() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Enter, KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x0D]));
    }

    #[test]
    fn normal_backspace_returns_del() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Backspace, KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x7F]));
    }

    #[test]
    fn normal_tab_returns_ht() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Tab, KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x09]));
    }

    #[test]
    fn normal_esc_returns_escape_byte() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Esc, KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B]));
    }

    #[test]
    fn normal_arrow_up_returns_escape_sequence() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Up, KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(
            matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'[', b'A'])
        );
    }

    #[test]
    fn normal_arrow_down_returns_escape_sequence() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Down, KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(
            matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'[', b'B'])
        );
    }

    #[test]
    fn normal_arrow_right_returns_escape_sequence() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Right, KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(
            matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'[', b'C'])
        );
    }

    #[test]
    fn normal_arrow_left_returns_escape_sequence() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Left, KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(
            matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'[', b'D'])
        );
    }

    #[test]
    fn normal_home_returns_escape_sequence() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Home, KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(
            matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'[', b'H'])
        );
    }

    #[test]
    fn normal_end_returns_escape_sequence() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::End, KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(
            matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'[', b'F'])
        );
    }

    #[test]
    fn normal_delete_returns_escape_sequence() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Delete, KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(
            matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'[', b'3', b'~'])
        );
    }

    #[test]
    fn normal_ctrl_c_returns_etx() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Char('c'), KeyModifiers::CONTROL);

        let action = handler.handle_key(key);

        // Ctrl+c = 0x03
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x03]));
        assert_normal(&handler);
    }

    #[test]
    fn normal_ctrl_a_returns_soh() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Char('a'), KeyModifiers::CONTROL);

        let action = handler.handle_key(key);

        // Ctrl+a = 0x01
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x01]));
    }

    #[test]
    fn normal_ctrl_z_returns_sub() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Char('z'), KeyModifiers::CONTROL);

        let action = handler.handle_key(key);

        // Ctrl+z = 0x1A
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1A]));
    }

    #[test]
    fn normal_unhandled_key_returns_none() {
        let mut handler = InputHandler::new();
        // F1 key is not mapped
        let key = make_key(KeyCode::F(1), KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(action.is_none());
        assert_normal(&handler);
    }

    // =========================================================================
    // Tests: PrefixWait mode
    // =========================================================================

    /// Helper: put the handler into PrefixWait mode
    fn enter_prefix(handler: &mut InputHandler) {
        let key = make_key(KeyCode::Char('t'), KeyModifiers::CONTROL);
        let result = handler.handle_key(key);
        assert!(result.is_none());
        assert_prefix_wait(handler);
    }

    #[test]
    fn prefix_c_creates_terminal() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('c'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(matches!(
            action,
            Some(AppAction::CreateTerminal { name: None })
        ));
        assert_normal(&handler);
    }

    #[test]
    fn prefix_d_closes_terminal() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('d'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::CloseTerminal)));
        assert_normal(&handler);
    }

    #[test]
    fn prefix_n_selects_next() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('n'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::SelectNext)));
        assert_normal(&handler);
    }

    #[test]
    fn prefix_p_selects_prev() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('p'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::SelectPrev)));
        assert_normal(&handler);
    }

    #[test]
    fn prefix_1_selects_index_0() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('1'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::SelectByIndex(0))));
        assert_normal(&handler);
    }

    #[test]
    fn prefix_5_selects_index_4() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('5'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::SelectByIndex(4))));
        assert_normal(&handler);
    }

    #[test]
    fn prefix_9_selects_index_8() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('9'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::SelectByIndex(8))));
        assert_normal(&handler);
    }

    #[test]
    fn prefix_ctrl_t_sends_literal_ctrl_t() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('t'), KeyModifiers::CONTROL);
        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x14]));
        assert_normal(&handler);
    }

    #[test]
    fn prefix_unknown_key_cancels_and_returns_none() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('z'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(action.is_none());
        assert_normal(&handler);
    }

    #[test]
    fn prefix_enter_cancels_and_returns_none() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Enter, KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(action.is_none());
        assert_normal(&handler);
    }

    #[test]
    fn prefix_esc_cancels_and_returns_none() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Esc, KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(action.is_none());
        assert_normal(&handler);
    }

    #[test]
    fn prefix_c_with_ctrl_modifier_cancels() {
        // Ctrl+c in prefix mode is NOT the 'c' command (command keys must have
        // no modifiers). It should cancel the prefix.
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let action = handler.handle_key(key);

        assert!(action.is_none());
        assert_normal(&handler);
    }

    #[test]
    fn prefix_0_cancels() {
        // '0' is out of '1'..='9' range; should cancel
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('0'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(action.is_none());
        assert_normal(&handler);
    }

    // =========================================================================
    // Tests: DialogInput mode
    // =========================================================================

    #[test]
    fn dialog_input_returns_none_for_any_key() {
        let mut handler = InputHandler::new();
        handler.set_mode(InputMode::DialogInput);

        let key = make_key(KeyCode::Char('a'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(action.is_none());
    }

    #[test]
    fn dialog_input_stays_in_dialog_mode() {
        let mut handler = InputHandler::new();
        handler.set_mode(InputMode::DialogInput);

        handler.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE));

        assert!(matches!(handler.mode(), InputMode::DialogInput));
    }

    #[test]
    fn dialog_input_returns_none_for_ctrl_t() {
        let mut handler = InputHandler::new();
        handler.set_mode(InputMode::DialogInput);

        let key = make_key(KeyCode::Char('t'), KeyModifiers::CONTROL);
        let action = handler.handle_key(key);

        assert!(action.is_none());
        assert!(matches!(handler.mode(), InputMode::DialogInput));
    }

    #[test]
    fn dialog_input_returns_none_for_enter() {
        let mut handler = InputHandler::new();
        handler.set_mode(InputMode::DialogInput);

        let key = make_key(KeyCode::Enter, KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(action.is_none());
    }

    // =========================================================================
    // Tests: set_mode / mode accessors
    // =========================================================================

    #[test]
    fn set_mode_changes_mode_to_dialog() {
        let mut handler = InputHandler::new();
        handler.set_mode(InputMode::DialogInput);
        assert!(matches!(handler.mode(), InputMode::DialogInput));
    }

    #[test]
    fn set_mode_changes_back_to_normal() {
        let mut handler = InputHandler::new();
        handler.set_mode(InputMode::DialogInput);
        handler.set_mode(InputMode::Normal);
        assert_normal(&handler);
    }

    // =========================================================================
    // Tests: check_timeout
    // =========================================================================

    #[test]
    fn check_timeout_in_normal_mode_returns_none() {
        let mut handler = InputHandler::new();

        let action = handler.check_timeout();

        assert!(action.is_none());
        assert_normal(&handler);
    }

    #[test]
    fn check_timeout_in_dialog_mode_returns_none() {
        let mut handler = InputHandler::new();
        handler.set_mode(InputMode::DialogInput);

        let action = handler.check_timeout();

        assert!(action.is_none());
    }

    #[test]
    fn check_timeout_in_prefix_wait_within_threshold_returns_none() {
        let mut handler = InputHandler::new();
        // Enter prefix mode (Instant::now() is set)
        enter_prefix(&mut handler);

        // Immediately check -- should still be within the 1-second window
        let action = handler.check_timeout();

        assert!(action.is_none());
        // Should still be in PrefixWait
        assert_prefix_wait(&handler);
    }

    // NOTE: Testing actual timeout expiry (>= 1 second) is difficult without
    // mocking `Instant`. We document this limitation here. The timeout logic
    // is straightforward (`since.elapsed().as_secs() >= 1`) and is covered
    // by integration/manual testing. A future refactor could inject a clock
    // trait to enable deterministic timeout testing.

    // =========================================================================
    // Tests: key_to_bytes (exercised via handle_key in Normal mode)
    // =========================================================================

    #[test]
    fn key_to_bytes_space() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Char(' '), KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[b' ']));
    }

    #[test]
    fn key_to_bytes_digit() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Char('7'), KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[b'7']));
    }

    #[test]
    fn key_to_bytes_special_char() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Char('/'), KeyModifiers::NONE);

        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[b'/']));
    }

    #[test]
    fn key_to_bytes_ctrl_d() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Char('d'), KeyModifiers::CONTROL);

        let action = handler.handle_key(key);

        // Ctrl+d = 0x04
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x04]));
    }

    #[test]
    fn key_to_bytes_ctrl_uppercase_c_returns_same_as_lowercase() {
        let mut handler = InputHandler::new();
        // Ctrl+C (uppercase) should produce same byte as Ctrl+c (0x03)
        let key = make_key(KeyCode::Char('C'), KeyModifiers::CONTROL);

        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x03]));
    }

    #[test]
    fn key_to_bytes_ctrl_uppercase_a_returns_same_as_lowercase() {
        let mut handler = InputHandler::new();
        // Ctrl+A (uppercase) should produce same byte as Ctrl+a (0x01)
        let key = make_key(KeyCode::Char('A'), KeyModifiers::CONTROL);

        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x01]));
    }

    #[test]
    fn key_to_bytes_ctrl_l() {
        let mut handler = InputHandler::new();
        let key = make_key(KeyCode::Char('l'), KeyModifiers::CONTROL);

        let action = handler.handle_key(key);

        // Ctrl+l = 0x0C
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x0C]));
    }

    // =========================================================================
    // Tests: Full flow (Normal -> PrefixWait -> Normal)
    // =========================================================================

    #[test]
    fn full_flow_prefix_create_then_normal_typing() {
        let mut handler = InputHandler::new();

        // Type 'h' in normal mode
        let action = handler.handle_key(make_key(KeyCode::Char('h'), KeyModifiers::NONE));
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[b'h']));
        assert_normal(&handler);

        // Ctrl+t enters prefix mode
        let action = handler.handle_key(make_key(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert!(action.is_none());
        assert_prefix_wait(&handler);

        // 'c' creates a terminal and returns to normal
        let action = handler.handle_key(make_key(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(matches!(
            action,
            Some(AppAction::CreateTerminal { name: None })
        ));
        assert_normal(&handler);

        // Back in normal mode, typing works again
        let action = handler.handle_key(make_key(KeyCode::Char('e'), KeyModifiers::NONE));
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[b'e']));
        assert_normal(&handler);
    }

    #[test]
    fn full_flow_prefix_cancel_then_resume_normal() {
        let mut handler = InputHandler::new();

        // Enter prefix
        enter_prefix(&mut handler);

        // Unknown key cancels
        let action = handler.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(action.is_none());
        assert_normal(&handler);

        // Normal typing resumes
        let action = handler.handle_key(make_key(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[b'a']));
    }

    #[test]
    fn full_flow_double_ctrl_t_sends_literal() {
        let mut handler = InputHandler::new();

        // First Ctrl+t enters prefix
        enter_prefix(&mut handler);

        // Second Ctrl+t sends literal 0x14
        let action = handler.handle_key(make_key(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x14]));
        assert_normal(&handler);
    }

    #[test]
    fn full_flow_dialog_mode_ignores_keys_then_resumes() {
        let mut handler = InputHandler::new();

        // Switch to dialog
        handler.set_mode(InputMode::DialogInput);

        // Keys are ignored
        assert!(handler
            .handle_key(make_key(KeyCode::Char('a'), KeyModifiers::NONE))
            .is_none());
        assert!(handler
            .handle_key(make_key(KeyCode::Char('t'), KeyModifiers::CONTROL))
            .is_none());

        // Switch back to normal
        handler.set_mode(InputMode::Normal);

        // Normal mode works again
        let action = handler.handle_key(make_key(KeyCode::Char('b'), KeyModifiers::NONE));
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[b'b']));
    }

    // =========================================================================
    // Tests: All digit keys in prefix mode (1-9)
    // =========================================================================

    #[test]
    fn prefix_q_produces_quit() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('q'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::Quit)));
        assert_normal(&handler);
    }

    #[test]
    fn prefix_o_produces_toggle_focus() {
        let mut handler = InputHandler::new();
        enter_prefix(&mut handler);

        let key = make_key(KeyCode::Char('o'), KeyModifiers::NONE);
        let action = handler.handle_key(key);

        assert!(matches!(action, Some(AppAction::ToggleFocus)));
        assert_normal(&handler);
    }

    #[test]
    fn prefix_digits_2_through_8() {
        for (digit, expected_index) in [
            ('2', 1usize),
            ('3', 2),
            ('4', 3),
            ('5', 4),
            ('6', 5),
            ('7', 6),
            ('8', 7),
        ] {
            let mut handler = InputHandler::new();
            enter_prefix(&mut handler);

            let key = make_key(KeyCode::Char(digit), KeyModifiers::NONE);
            let action = handler.handle_key(key);

            assert!(
                matches!(action, Some(AppAction::SelectByIndex(idx)) if idx == expected_index),
                "digit '{}' should map to SelectByIndex({})",
                digit,
                expected_index
            );
            assert_normal(&handler);
        }
    }

    // =========================================================================
    // Tests: Application cursor keys mode
    // =========================================================================

    #[test]
    fn normal_arrow_up_application_mode_returns_esc_o_a() {
        let mut handler = InputHandler::new();
        handler.set_application_cursor_keys(true);
        let key = make_key(KeyCode::Up, KeyModifiers::NONE);
        let action = handler.handle_key(key);
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'O', b'A']));
    }

    #[test]
    fn normal_arrow_down_application_mode_returns_esc_o_b() {
        let mut handler = InputHandler::new();
        handler.set_application_cursor_keys(true);
        let key = make_key(KeyCode::Down, KeyModifiers::NONE);
        let action = handler.handle_key(key);
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'O', b'B']));
    }

    #[test]
    fn normal_arrow_right_application_mode_returns_esc_o_c() {
        let mut handler = InputHandler::new();
        handler.set_application_cursor_keys(true);
        let key = make_key(KeyCode::Right, KeyModifiers::NONE);
        let action = handler.handle_key(key);
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'O', b'C']));
    }

    #[test]
    fn normal_arrow_left_application_mode_returns_esc_o_d() {
        let mut handler = InputHandler::new();
        handler.set_application_cursor_keys(true);
        let key = make_key(KeyCode::Left, KeyModifiers::NONE);
        let action = handler.handle_key(key);
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'O', b'D']));
    }

    #[test]
    fn normal_home_application_mode_returns_esc_o_h() {
        let mut handler = InputHandler::new();
        handler.set_application_cursor_keys(true);
        let key = make_key(KeyCode::Home, KeyModifiers::NONE);
        let action = handler.handle_key(key);
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'O', b'H']));
    }

    #[test]
    fn normal_end_application_mode_returns_esc_o_f() {
        let mut handler = InputHandler::new();
        handler.set_application_cursor_keys(true);
        let key = make_key(KeyCode::End, KeyModifiers::NONE);
        let action = handler.handle_key(key);
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'O', b'F']));
    }

    #[test]
    fn normal_arrow_up_normal_mode_still_returns_esc_bracket_a() {
        let mut handler = InputHandler::new();
        // application_cursor_keys is false by default
        let key = make_key(KeyCode::Up, KeyModifiers::NONE);
        let action = handler.handle_key(key);
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'[', b'A']));
    }

    #[test]
    fn application_cursor_keys_flag_toggle() {
        let mut handler = InputHandler::new();

        // Start in normal mode
        let key = make_key(KeyCode::Up, KeyModifiers::NONE);
        let action = handler.handle_key(key);
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'[', b'A']));

        // Enable application mode
        handler.set_application_cursor_keys(true);
        let action = handler.handle_key(key);
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'O', b'A']));

        // Disable application mode
        handler.set_application_cursor_keys(false);
        let action = handler.handle_key(key);
        assert!(matches!(action, Some(AppAction::WriteToActive(ref b)) if b == &[0x1B, b'[', b'A']));
    }
}
