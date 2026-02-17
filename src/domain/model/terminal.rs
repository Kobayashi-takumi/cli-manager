use std::path::{Path, PathBuf};

use crate::domain::primitive::{NotificationEvent, TerminalId, TerminalStatus};

pub struct ManagedTerminal {
    id: TerminalId,
    name: String,
    cwd: PathBuf,
    status: TerminalStatus,
    last_notification: Option<NotificationEvent>,
    has_unread_notification: bool,
}

impl ManagedTerminal {
    pub fn new(id: TerminalId, name: String, cwd: PathBuf) -> Self {
        Self {
            id,
            name,
            cwd,
            status: TerminalStatus::Running,
            last_notification: None,
            has_unread_notification: false,
        }
    }

    pub fn id(&self) -> TerminalId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn status(&self) -> &TerminalStatus {
        &self.status
    }

    pub fn mark_exited(&mut self, exit_code: i32) {
        self.status = TerminalStatus::Exited(exit_code);
    }

    pub fn display_name(&self) -> String {
        format!("{}: {}", self.id.value(), self.name)
    }

    pub fn set_notification(&mut self, event: NotificationEvent) {
        self.last_notification = Some(event);
        self.has_unread_notification = true;
    }

    pub fn last_notification(&self) -> Option<&NotificationEvent> {
        self.last_notification.as_ref()
    }

    pub fn has_unread_notification(&self) -> bool {
        self.has_unread_notification
    }

    pub fn clear_notification(&mut self) {
        self.has_unread_notification = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::primitive::NotificationEvent;

    fn make_terminal() -> ManagedTerminal {
        ManagedTerminal::new(
            TerminalId::new(1),
            "test-term".to_string(),
            PathBuf::from("/tmp"),
        )
    }

    // =========================================================================
    // Tests: notification state initialization
    // =========================================================================

    #[test]
    fn new_terminal_has_no_unread_notification() {
        let terminal = make_terminal();
        assert!(!terminal.has_unread_notification());
    }

    #[test]
    fn new_terminal_has_no_last_notification() {
        let terminal = make_terminal();
        assert!(terminal.last_notification().is_none());
    }

    // =========================================================================
    // Tests: set_notification
    // =========================================================================

    #[test]
    fn set_notification_marks_unread_true() {
        let mut terminal = make_terminal();
        terminal.set_notification(NotificationEvent::Bell);
        assert!(terminal.has_unread_notification());
    }

    #[test]
    fn set_notification_stores_last_notification() {
        let mut terminal = make_terminal();
        terminal.set_notification(NotificationEvent::Bell);
        assert_eq!(terminal.last_notification(), Some(&NotificationEvent::Bell));
    }

    #[test]
    fn set_notification_overwrites_previous_notification() {
        let mut terminal = make_terminal();
        terminal.set_notification(NotificationEvent::Bell);

        let osc9 = NotificationEvent::Osc9 {
            message: "done".to_string(),
        };
        terminal.set_notification(osc9.clone());

        assert_eq!(terminal.last_notification(), Some(&osc9));
        assert!(terminal.has_unread_notification());
    }

    #[test]
    fn set_notification_with_osc777_stores_correctly() {
        let mut terminal = make_terminal();
        let event = NotificationEvent::Osc777 {
            title: "Build".to_string(),
            body: "Complete".to_string(),
        };
        terminal.set_notification(event.clone());
        assert_eq!(terminal.last_notification(), Some(&event));
    }

    // =========================================================================
    // Tests: clear_notification
    // =========================================================================

    #[test]
    fn clear_notification_marks_unread_false() {
        let mut terminal = make_terminal();
        terminal.set_notification(NotificationEvent::Bell);
        assert!(terminal.has_unread_notification());

        terminal.clear_notification();
        assert!(!terminal.has_unread_notification());
    }

    #[test]
    fn clear_notification_on_already_cleared_is_noop() {
        let mut terminal = make_terminal();
        terminal.clear_notification();
        assert!(!terminal.has_unread_notification());
    }

    #[test]
    fn clear_notification_does_not_remove_last_notification() {
        let mut terminal = make_terminal();
        terminal.set_notification(NotificationEvent::Bell);
        terminal.clear_notification();
        // last_notification is preserved even after clear (only has_unread is toggled)
        assert_eq!(terminal.last_notification(), Some(&NotificationEvent::Bell));
    }

    #[test]
    fn set_then_clear_then_set_again_works() {
        let mut terminal = make_terminal();

        terminal.set_notification(NotificationEvent::Bell);
        assert!(terminal.has_unread_notification());

        terminal.clear_notification();
        assert!(!terminal.has_unread_notification());

        let osc9 = NotificationEvent::Osc9 {
            message: "test".to_string(),
        };
        terminal.set_notification(osc9.clone());
        assert!(terminal.has_unread_notification());
        assert_eq!(terminal.last_notification(), Some(&osc9));
    }
}
