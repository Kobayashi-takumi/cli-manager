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
use crate::infrastructure::ipc::UnixSocketServer;
use crate::infrastructure::ipc::socket_discovery;
use crate::interface_adapter::port::IpcPort;

fn main() -> anyhow::Result<()> {
    // Check for subcommands first
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "ctl" {
        crate::infrastructure::ipc::cli_client::run(&args);
    }
    if args.len() >= 2 && args[1] == "mcp-server" {
        crate::infrastructure::mcp::mcp_server::run();
    }

    let cwd = std::env::current_dir()?;

    // Infrastructure concrete adapters
    let pty_adapter = pty_adapter_factory::create_pty_adapter();
    let screen_adapter = screen_adapter_factory::create_vt100_screen_adapter();

    // IPC server
    let socket_path = format!("/tmp/cli-manager-{}.sock", std::process::id());
    let ipc_server = UnixSocketServer::new(&socket_path)?;

    // Set CLI_MANAGER_SOCK env var for child processes
    // SAFETY: This is called before any threads are spawned. The env var is set
    // once at startup for child processes to discover the IPC socket path.
    unsafe {
        std::env::set_var("CLI_MANAGER_SOCK", ipc_server.socket_path());
    }

    // Write socket path to discovery file (~/.cli-manager/socket)
    // so external tools can find the IPC socket without env var
    let _ = socket_discovery::write_socket_path(ipc_server.socket_path());

    // Usecase (depends on port traits via generics)
    let usecase = TerminalUsecase::new(cwd, pty_adapter, screen_adapter);

    // Controller
    let controller = TuiController::new(usecase);

    // Run TUI with IPC
    app_runner::run(controller, Some(Box::new(ipc_server)))?;

    // Clean up discovery file on exit
    socket_discovery::remove_socket_path();

    Ok(())
}
