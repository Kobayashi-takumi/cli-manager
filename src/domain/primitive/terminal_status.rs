#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalStatus {
    Running,
    Exited(i32),
}

impl TerminalStatus {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    pub fn icon(&self) -> &str {
        match self {
            Self::Running => "●",
            Self::Exited(_) => "✗",
        }
    }

    pub fn status_text(&self) -> String {
        match self {
            Self::Running => "running".to_string(),
            Self::Exited(code) => format!("exited ({code})"),
        }
    }
}
