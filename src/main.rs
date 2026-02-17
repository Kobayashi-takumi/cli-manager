mod domain;
mod usecase;
mod interface_adapter;
mod infrastructure;
mod shared;

use crate::interface_adapter::adapter::pty_adapter_factory;
use crate::interface_adapter::adapter::screen_adapter_factory;
use crate::usecase::terminal_usecase::TerminalUsecase;
use crate::interface_adapter::controller::tui_controller::TuiController;
use crate::infrastructure::tui::app_runner;

fn main() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;

    // Infrastructure concrete adapters
    let pty_adapter = pty_adapter_factory::create_pty_adapter();
    let screen_adapter = screen_adapter_factory::create_vt100_screen_adapter();

    // Usecase (depends on port traits via generics)
    let usecase = TerminalUsecase::new(cwd, pty_adapter, screen_adapter);

    // Controller
    let controller = TuiController::new(usecase);

    // Run TUI
    app_runner::run(controller)?;

    Ok(())
}
