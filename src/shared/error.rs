use crate::domain::primitive::TerminalId;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("Failed to spawn pty: {0}")]
    PtySpawn(#[source] std::io::Error),

    #[error("Pty I/O error for terminal {id}: {source}")]
    PtyIo {
        id: TerminalId,
        #[source]
        source: std::io::Error,
    },

    #[error("Terminal not found: {0}")]
    TerminalNotFound(TerminalId),

    #[error("Screen not found: {0}")]
    ScreenNotFound(TerminalId),

    #[error("No active terminal")]
    NoActiveTerminal,

    #[error("TUI error: {0}")]
    Tui(#[source] std::io::Error),
}
