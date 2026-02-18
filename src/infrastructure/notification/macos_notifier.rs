use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::domain::primitive::NotificationEvent;

/// Desktop notification sender for macOS using `notify-rust`.
///
/// Features:
/// - Rate limiting: suppresses repeated notifications from the same terminal
///   within a configurable cooldown period (default 1 second).
/// - Enable/disable toggle: can be globally turned off.
/// - Failure-safe: errors from `Notification::show()` are silently ignored
///   so the TUI application never crashes due to notification delivery failure.
pub struct MacOsNotifier {
    enabled: bool,
    last_notify: HashMap<String, Instant>,
    cooldown: Duration,
}

impl MacOsNotifier {
    pub fn new() -> Self {
        Self {
            enabled: true,
            last_notify: HashMap::new(),
            cooldown: Duration::from_secs(1),
        }
    }

    /// Send a desktop notification for the given terminal and event.
    ///
    /// Returns `true` if a notification was actually attempted (i.e., not
    /// suppressed by the enabled flag or rate limiter). Returns `false` if
    /// the notification was skipped.
    ///
    /// Errors from `Notification::show()` are silently ignored to avoid
    /// crashing the application.
    pub fn notify(&mut self, terminal_name: &str, event: &NotificationEvent) -> bool {
        if !self.enabled {
            return false;
        }

        // Rate limiting: skip if within cooldown for same terminal
        let now = Instant::now();
        if let Some(last) = self.last_notify.get(terminal_name) {
            if now.duration_since(*last) < self.cooldown {
                return false;
            }
        }
        self.last_notify.insert(terminal_name.to_string(), now);

        let (title, body) = event.to_notification_parts();
        let summary = format!("{} - {}", title, terminal_name);
        Self::send_notification(&summary, body);

        true
    }

    /// Actually deliver the notification to the OS.
    ///
    /// Separated from `notify()` so that the rate-limiting and gating logic
    /// can be tested without triggering real desktop notifications.
    /// In test builds, this is a no-op.
    #[cfg(not(test))]
    fn send_notification(summary: &str, body: &str) {
        let _ = notify_rust::Notification::new()
            .summary(summary)
            .body(body)
            .show();
    }

    #[cfg(test)]
    fn send_notification(_summary: &str, _body: &str) {
        // No-op in tests to avoid blocking on macOS notification center
    }

    /// Enable or disable desktop notifications globally.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::primitive::NotificationEvent;

    // =========================================================================
    // Tests: new()
    // =========================================================================

    #[test]
    fn new_creates_enabled_notifier() {
        let notifier = MacOsNotifier::new();
        assert!(notifier.enabled);
    }

    #[test]
    fn new_creates_notifier_with_1s_cooldown() {
        let notifier = MacOsNotifier::new();
        assert_eq!(notifier.cooldown, Duration::from_secs(1));
    }

    #[test]
    fn new_creates_notifier_with_empty_last_notify_map() {
        let notifier = MacOsNotifier::new();
        assert!(notifier.last_notify.is_empty());
    }

    // =========================================================================
    // Tests: set_enabled()
    // =========================================================================

    #[test]
    fn set_enabled_false_disables_notifier() {
        let mut notifier = MacOsNotifier::new();
        notifier.set_enabled(false);
        assert!(!notifier.enabled);
    }

    #[test]
    fn set_enabled_true_re_enables_notifier() {
        let mut notifier = MacOsNotifier::new();
        notifier.set_enabled(false);
        notifier.set_enabled(true);
        assert!(notifier.enabled);
    }

    // =========================================================================
    // Tests: notify() with enabled=false
    // =========================================================================

    #[test]
    fn notify_returns_false_when_disabled() {
        let mut notifier = MacOsNotifier::new();
        notifier.set_enabled(false);

        let event = NotificationEvent::Bell;
        let result = notifier.notify("term-1", &event);
        assert!(!result);
    }

    #[test]
    fn notify_does_not_record_timestamp_when_disabled() {
        let mut notifier = MacOsNotifier::new();
        notifier.set_enabled(false);

        let event = NotificationEvent::Bell;
        notifier.notify("term-1", &event);
        assert!(notifier.last_notify.is_empty());
    }

    // =========================================================================
    // Tests: notify() with enabled=true (first call)
    // =========================================================================

    #[test]
    fn notify_returns_true_on_first_call_for_terminal() {
        let mut notifier = MacOsNotifier::new();
        let event = NotificationEvent::Bell;

        let result = notifier.notify("term-1", &event);
        assert!(result);
    }

    #[test]
    fn notify_records_timestamp_on_successful_attempt() {
        let mut notifier = MacOsNotifier::new();
        let event = NotificationEvent::Bell;

        notifier.notify("term-1", &event);
        assert!(notifier.last_notify.contains_key("term-1"));
    }

    // =========================================================================
    // Tests: rate limiting
    // =========================================================================

    #[test]
    fn notify_rate_limits_same_terminal_within_cooldown() {
        let mut notifier = MacOsNotifier::new();
        let event = NotificationEvent::Bell;

        // First call: should succeed
        let first = notifier.notify("term-1", &event);
        assert!(first);

        // Second call immediately: should be rate-limited
        let second = notifier.notify("term-1", &event);
        assert!(!second);
    }

    #[test]
    fn notify_allows_different_terminals_within_cooldown() {
        let mut notifier = MacOsNotifier::new();
        let event = NotificationEvent::Bell;

        let first = notifier.notify("term-1", &event);
        assert!(first);

        // Different terminal name: not rate-limited
        let second = notifier.notify("term-2", &event);
        assert!(second);
    }

    #[test]
    fn notify_rate_limit_does_not_update_timestamp() {
        let mut notifier = MacOsNotifier::new();
        let event = NotificationEvent::Bell;

        notifier.notify("term-1", &event);
        let first_time = *notifier.last_notify.get("term-1").unwrap();

        // Rate-limited call should NOT update timestamp
        notifier.notify("term-1", &event);
        let second_time = *notifier.last_notify.get("term-1").unwrap();

        assert_eq!(first_time, second_time);
    }

    #[test]
    fn notify_allows_after_cooldown_expires() {
        let mut notifier = MacOsNotifier::new();
        // Set a very short cooldown for testing
        notifier.cooldown = Duration::from_millis(1);
        let event = NotificationEvent::Bell;

        let first = notifier.notify("term-1", &event);
        assert!(first);

        // Sleep longer than cooldown
        std::thread::sleep(Duration::from_millis(5));

        let second = notifier.notify("term-1", &event);
        assert!(second);
    }

    // =========================================================================
    // Tests: different event types
    // =========================================================================

    #[test]
    fn notify_works_with_osc9_event() {
        let mut notifier = MacOsNotifier::new();
        let event = NotificationEvent::Osc9 {
            message: "Build complete".to_string(),
        };

        let result = notifier.notify("builder", &event);
        assert!(result);
    }

    #[test]
    fn notify_works_with_osc777_event() {
        let mut notifier = MacOsNotifier::new();
        let event = NotificationEvent::Osc777 {
            title: "Cargo".to_string(),
            body: "Build succeeded".to_string(),
        };

        let result = notifier.notify("cargo-term", &event);
        assert!(result);
    }

    // =========================================================================
    // Tests: edge cases
    // =========================================================================

    #[test]
    fn notify_with_empty_terminal_name() {
        let mut notifier = MacOsNotifier::new();
        let event = NotificationEvent::Bell;

        let result = notifier.notify("", &event);
        assert!(result);
        assert!(notifier.last_notify.contains_key(""));
    }

    #[test]
    fn notify_rate_limits_independently_per_terminal() {
        let mut notifier = MacOsNotifier::new();
        let event = NotificationEvent::Bell;

        // Notify three different terminals
        assert!(notifier.notify("a", &event));
        assert!(notifier.notify("b", &event));
        assert!(notifier.notify("c", &event));

        // All three are rate-limited on immediate retry
        assert!(!notifier.notify("a", &event));
        assert!(!notifier.notify("b", &event));
        assert!(!notifier.notify("c", &event));
    }
}
