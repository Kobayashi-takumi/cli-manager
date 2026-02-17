/// Terminal notification events received from child processes.
///
/// These events represent various notification mechanisms that terminal
/// applications use to signal the user (e.g., bell, OSC 9, OSC 777).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationEvent {
    /// BEL character (\x07) received.
    Bell,
    /// OSC 9 notification (iTerm2-compatible).
    Osc9 { message: String },
    /// OSC 777 notification (rxvt-compatible).
    Osc777 { title: String, body: String },
}

impl NotificationEvent {
    /// Return a short summary string for the notification.
    pub fn summary(&self) -> &str {
        match self {
            Self::Bell => "Bell",
            Self::Osc9 { message } => message.as_str(),
            Self::Osc777 { title, .. } => title.as_str(),
        }
    }

    /// Return (title, body) pair suitable for desktop notification display.
    pub fn to_notification_parts(&self) -> (&str, &str) {
        match self {
            Self::Bell => ("CLI Manager", "Task completed (bell)"),
            Self::Osc9 { message } => ("CLI Manager", message.as_str()),
            Self::Osc777 { title, body } => (title.as_str(), body.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Tests: Bell variant
    // =========================================================================

    #[test]
    fn bell_summary_returns_bell() {
        let event = NotificationEvent::Bell;
        assert_eq!(event.summary(), "Bell");
    }

    #[test]
    fn bell_notification_parts_returns_cli_manager_and_task_completed() {
        let event = NotificationEvent::Bell;
        let (title, body) = event.to_notification_parts();
        assert_eq!(title, "CLI Manager");
        assert_eq!(body, "Task completed (bell)");
    }

    // =========================================================================
    // Tests: Osc9 variant
    // =========================================================================

    #[test]
    fn osc9_summary_returns_message() {
        let event = NotificationEvent::Osc9 {
            message: "Build succeeded".to_string(),
        };
        assert_eq!(event.summary(), "Build succeeded");
    }

    #[test]
    fn osc9_notification_parts_returns_cli_manager_and_message() {
        let event = NotificationEvent::Osc9 {
            message: "Build succeeded".to_string(),
        };
        let (title, body) = event.to_notification_parts();
        assert_eq!(title, "CLI Manager");
        assert_eq!(body, "Build succeeded");
    }

    #[test]
    fn osc9_empty_message_summary_returns_empty_str() {
        let event = NotificationEvent::Osc9 {
            message: String::new(),
        };
        assert_eq!(event.summary(), "");
    }

    #[test]
    fn osc9_empty_message_notification_parts_returns_empty_body() {
        let event = NotificationEvent::Osc9 {
            message: String::new(),
        };
        let (title, body) = event.to_notification_parts();
        assert_eq!(title, "CLI Manager");
        assert_eq!(body, "");
    }

    // =========================================================================
    // Tests: Osc777 variant
    // =========================================================================

    #[test]
    fn osc777_summary_returns_title() {
        let event = NotificationEvent::Osc777 {
            title: "Cargo".to_string(),
            body: "Build complete".to_string(),
        };
        assert_eq!(event.summary(), "Cargo");
    }

    #[test]
    fn osc777_notification_parts_returns_title_and_body() {
        let event = NotificationEvent::Osc777 {
            title: "Cargo".to_string(),
            body: "Build complete".to_string(),
        };
        let (title, body) = event.to_notification_parts();
        assert_eq!(title, "Cargo");
        assert_eq!(body, "Build complete");
    }

    #[test]
    fn osc777_empty_title_and_body() {
        let event = NotificationEvent::Osc777 {
            title: String::new(),
            body: String::new(),
        };
        assert_eq!(event.summary(), "");
        let (title, body) = event.to_notification_parts();
        assert_eq!(title, "");
        assert_eq!(body, "");
    }

    // =========================================================================
    // Tests: Clone and PartialEq
    // =========================================================================

    #[test]
    fn notification_event_clone_equals_original() {
        let original = NotificationEvent::Osc9 {
            message: "test".to_string(),
        };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn different_variants_are_not_equal() {
        let bell = NotificationEvent::Bell;
        let osc9 = NotificationEvent::Osc9 {
            message: "Bell".to_string(),
        };
        assert_ne!(bell, osc9);
    }

    #[test]
    fn osc9_different_messages_are_not_equal() {
        let a = NotificationEvent::Osc9 {
            message: "hello".to_string(),
        };
        let b = NotificationEvent::Osc9 {
            message: "world".to_string(),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn osc777_different_titles_are_not_equal() {
        let a = NotificationEvent::Osc777 {
            title: "A".to_string(),
            body: "same".to_string(),
        };
        let b = NotificationEvent::Osc777 {
            title: "B".to_string(),
            body: "same".to_string(),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn osc777_different_bodies_are_not_equal() {
        let a = NotificationEvent::Osc777 {
            title: "same".to_string(),
            body: "X".to_string(),
        };
        let b = NotificationEvent::Osc777 {
            title: "same".to_string(),
            body: "Y".to_string(),
        };
        assert_ne!(a, b);
    }

    // =========================================================================
    // Tests: Debug
    // =========================================================================

    #[test]
    fn debug_format_includes_variant_name() {
        let bell = NotificationEvent::Bell;
        let debug = format!("{:?}", bell);
        assert!(debug.contains("Bell"));

        let osc9 = NotificationEvent::Osc9 {
            message: "msg".to_string(),
        };
        let debug = format!("{:?}", osc9);
        assert!(debug.contains("Osc9"));
        assert!(debug.contains("msg"));
    }
}
