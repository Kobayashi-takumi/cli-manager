use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::event::{EnableBracketedPaste, DisableBracketedPaste};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::Rect;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::domain::primitive::TerminalSize;
use crate::infrastructure::notification::MacOsNotifier;
use crate::infrastructure::tui::input::{InputHandler, InputMode};
use crate::infrastructure::tui::widgets::{dialog, layout, sidebar, terminal_view};
use crate::interface_adapter::controller::tui_controller::{AppAction, TuiController};
use crate::interface_adapter::port::{PtyPort, ScreenPort};

/// Which pane currently holds input focus.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusPane {
    Sidebar,
    Terminal,
}

/// Dialog state for overlay dialogs.
enum DialogState {
    None,
    CreateTerminal { input: String, cursor_pos: usize },
    ConfirmClose { terminal_name: String, is_running: bool },
}

/// Main TUI event loop.
///
/// Initializes crossterm raw mode + alternate screen, creates the ratatui Terminal,
/// runs the draw -> poll -> input loop, and cleans up on exit.
pub fn run<P: PtyPort, S: ScreenPort>(mut controller: TuiController<P, S>) -> anyhow::Result<()> {
    // === Initialization ===
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut input_handler = InputHandler::new();
    let mut should_quit = false;
    let mut dialog = DialogState::None;
    let mut focus = FocusPane::Terminal;
    let mut sidebar_scroll_offset: usize = 0;
    let mut notifier = MacOsNotifier::new();
    let mut in_scrollback = false;

    // === Main loop ===
    let result = main_loop(
        &mut terminal,
        &mut controller,
        &mut input_handler,
        &mut should_quit,
        &mut dialog,
        &mut focus,
        &mut sidebar_scroll_offset,
        &mut notifier,
        &mut in_scrollback,
    );

    // === Cleanup (always runs) ===
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableBracketedPaste);
    let _ = terminal.show_cursor();

    result
}

fn main_loop<P: PtyPort, S: ScreenPort>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    controller: &mut TuiController<P, S>,
    input_handler: &mut InputHandler,
    should_quit: &mut bool,
    dialog: &mut DialogState,
    focus: &mut FocusPane,
    sidebar_scroll_offset: &mut usize,
    notifier: &mut MacOsNotifier,
    in_scrollback: &mut bool,
) -> anyhow::Result<()> {
    while !*should_quit {
        // 1. Draw
        terminal.draw(|frame| {
            let areas = layout::compute_layout(frame.area());

            // Compute sidebar scroll offset before rendering
            let sidebar_inner_height = areas.sidebar.height.saturating_sub(2); // minus top/bottom border
            let content_height = sidebar_inner_height.saturating_sub(2); // minus help area (2 lines)
            *sidebar_scroll_offset = sidebar::compute_scroll_offset(
                controller.usecase().get_terminals().len(),
                controller.usecase().get_active_index(),
                content_height,
                *sidebar_scroll_offset,
            );

            // Collect dynamic cwds from OSC 7 for sidebar display
            let dynamic_cwds: Vec<Option<String>> = controller.usecase().get_terminals()
                .iter()
                .map(|t| controller.usecase().screen_port().get_cwd(t.id()).ok().flatten())
                .collect();

            // Sidebar
            sidebar::render(
                frame,
                areas.sidebar,
                controller.usecase().get_terminals(),
                controller.usecase().get_active_index(),
                *focus == FocusPane::Sidebar,
                *sidebar_scroll_offset,
                &dynamic_cwds,
            );

            // Terminal view - get active terminal info
            let (cells_opt, cursor_opt, cursor_visible, cwd_opt, scrollback_info) =
                match controller.usecase().get_active_terminal() {
                    Some(t) => {
                        let id = t.id();
                        let cwd = controller.usecase().screen_port().get_cwd(id)
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| t.cwd().display().to_string());
                        let cells = controller.usecase().screen_port().get_cells(id).ok();
                        let cursor = controller.usecase().screen_port().get_cursor(id).ok();
                        let visible = if *in_scrollback {
                            false // Hide cursor during scrollback
                        } else {
                            controller
                                .usecase()
                                .screen_port()
                                .get_cursor_visible(id)
                                .unwrap_or(true)
                        };
                        let sb_info = if *in_scrollback {
                            let offset = controller.usecase().screen_port().get_scrollback_offset(id).unwrap_or(0);
                            let max = controller.usecase().screen_port().get_max_scrollback(id).unwrap_or(0);
                            Some((offset, max))
                        } else {
                            None
                        };
                        (cells, cursor, visible, Some(cwd), sb_info)
                    }
                    None => (None, None, true, None, None),
                };
            terminal_view::render(
                frame,
                areas.main_pane,
                cells_opt,
                cursor_opt,
                cursor_visible,
                cwd_opt.as_deref(),
                *focus == FocusPane::Terminal,
                scrollback_info,
            );

            // Dialog overlay
            match dialog {
                DialogState::CreateTerminal { input, cursor_pos } => {
                    dialog::render_create_dialog(frame, input, *cursor_pos);
                }
                DialogState::ConfirmClose {
                    terminal_name,
                    is_running,
                } => {
                    dialog::render_confirm_close_dialog(frame, terminal_name, *is_running);
                }
                DialogState::None => {}
            }
        })?;

        // 2. Calculate terminal size from right pane
        let frame_size = terminal.size()?;
        let areas = layout::compute_layout(frame_size.into());
        let content_height = areas.main_pane.height.saturating_sub(1); // minus CWD bar
        let size = TerminalSize::new(areas.main_pane.width, content_height);

        // 3. Poll all ptys
        if let Err(e) = controller.dispatch(AppAction::PollAll, size)
            && !matches!(
                e,
                crate::shared::error::AppError::NoActiveTerminal
                    | crate::shared::error::AppError::TerminalNotFound(_)
                    | crate::shared::error::AppError::ScreenNotFound(_)
            )
        {
            return Err(e.into());
        }

        // 3.5. Drain pending notifications and send desktop notifications
        let pending = controller.usecase_mut().take_pending_notifications();
        if !pending.is_empty() {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
                .open("/tmp/cli_manager_notif_debug.log") {
                use std::io::Write;
                let _ = writeln!(f, "[app_runner] pending={} items: {:?}",
                    pending.len(), pending.iter().map(|(n, e)| format!("{}:{:?}", n, e)).collect::<Vec<_>>());
            }
        }
        for (terminal_name, event) in &pending {
            let sent = notifier.notify(terminal_name, event);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
                .open("/tmp/cli_manager_notif_debug.log") {
                use std::io::Write;
                let _ = writeln!(f, "[app_runner] notify({}, {:?}) => sent={}", terminal_name, event, sent);
            }
        }

        // 4. Check prefix timeout
        if let Some(action) = input_handler.check_timeout() {
            match controller.dispatch(action, size) {
                Ok(()) => {}
                Err(e) => {
                    if !matches!(e, crate::shared::error::AppError::NoActiveTerminal) {
                        return Err(e.into());
                    }
                }
            }
        }

        // 5. Poll for events (50ms timeout)
        if event::poll(Duration::from_millis(50))? {
            let ev = event::read()?;
            match ev {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_key_event(key, controller, input_handler, should_quit, dialog, focus, size, in_scrollback)?;
                }
                Event::Resize(cols, rows) => {
                    let new_full = Rect::new(0, 0, cols, rows);
                    let new_areas = layout::compute_layout(new_full);
                    let new_content_height = new_areas.main_pane.height.saturating_sub(1);
                    let pane_size =
                        TerminalSize::new(new_areas.main_pane.width, new_content_height);
                    controller.dispatch(AppAction::ResizeAll(pane_size), pane_size)?;
                }
                Event::Paste(text) => {
                    // Get bracketed paste mode flag from active terminal
                    let bracketed = controller.usecase().get_active_terminal()
                        .and_then(|t| controller.usecase().screen_port().get_bracketed_paste(t.id()).ok())
                        .unwrap_or(false);

                    let mut data = Vec::new();
                    if bracketed {
                        data.extend_from_slice(b"\x1b[200~");
                    }
                    data.extend_from_slice(text.as_bytes());
                    if bracketed {
                        data.extend_from_slice(b"\x1b[201~");
                    }

                    match controller.dispatch(AppAction::WriteToActive(data), size) {
                        Ok(()) => {}
                        Err(e) => {
                            if !matches!(e, crate::shared::error::AppError::NoActiveTerminal) {
                                return Err(e.into());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn handle_key_event<P: PtyPort, S: ScreenPort>(
    key: KeyEvent,
    controller: &mut TuiController<P, S>,
    input_handler: &mut InputHandler,
    should_quit: &mut bool,
    dialog: &mut DialogState,
    focus: &mut FocusPane,
    size: TerminalSize,
    in_scrollback: &mut bool,
) -> anyhow::Result<()> {
    // If a dialog is active, handle keys in the dialog
    if !matches!(dialog, DialogState::None) {
        handle_dialog_key(key, controller, input_handler, dialog, size)?;
        return Ok(());
    }

    // Sidebar focus: intercept ↑↓Enter before InputHandler
    if *focus == FocusPane::Sidebar {
        match key.code {
            KeyCode::Up if key.modifiers.is_empty() => {
                let _ = controller.dispatch(AppAction::SelectPrev, size);
                return Ok(());
            }
            KeyCode::Down if key.modifiers.is_empty() => {
                let _ = controller.dispatch(AppAction::SelectNext, size);
                return Ok(());
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                *focus = FocusPane::Terminal;
                return Ok(());
            }
            _ => {} // Fall through to InputHandler (Ctrl+b etc.)
        }
    }

    // Update application cursor keys flag from active terminal's DECCKM state
    let app_cursor = controller.usecase().get_active_terminal()
        .and_then(|t| controller.usecase().screen_port().get_application_cursor_keys(t.id()).ok())
        .unwrap_or(false);
    input_handler.set_application_cursor_keys(app_cursor);

    // Normal/PrefixWait/ScrollbackMode
    let Some(action) = input_handler.handle_key(key) else {
        return Ok(());
    };

    match action {
        AppAction::CreateTerminal { .. } => {
            exit_scrollback_if_active(controller, input_handler, in_scrollback);
            *dialog = DialogState::CreateTerminal {
                input: String::new(),
                cursor_pos: 0,
            };
            input_handler.set_mode(InputMode::DialogInput);
        }
        AppAction::CloseTerminal => {
            exit_scrollback_if_active(controller, input_handler, in_scrollback);
            // Check if the active terminal is running
            if let Some(terminal) = controller.usecase().get_active_terminal() {
                if terminal.status().is_running() {
                    *dialog = DialogState::ConfirmClose {
                        terminal_name: terminal.name().to_string(),
                        is_running: true,
                    };
                    input_handler.set_mode(InputMode::DialogInput);
                } else {
                    // Exited terminal: close immediately without confirmation
                    controller.dispatch(AppAction::CloseTerminal, size)?;
                }
            }
        }
        AppAction::Quit => {
            *should_quit = true;
        }
        AppAction::ToggleFocus => {
            exit_scrollback_if_active(controller, input_handler, in_scrollback);
            *focus = match *focus {
                FocusPane::Sidebar => FocusPane::Terminal,
                FocusPane::Terminal => FocusPane::Sidebar,
            };
        }
        AppAction::SelectNext | AppAction::SelectPrev | AppAction::SelectByIndex(_) => {
            exit_scrollback_if_active(controller, input_handler, in_scrollback);
            match controller.dispatch(action, size) {
                Ok(()) => {}
                Err(e) => {
                    if !matches!(e, crate::shared::error::AppError::NoActiveTerminal) {
                        return Err(e.into());
                    }
                }
            }
        }
        AppAction::EnterScrollback => {
            // Check if we can enter scrollback (not in alternate screen, has history)
            if let Some(t) = controller.usecase().get_active_terminal() {
                let id = t.id();
                let is_alt = controller.usecase().screen_port().is_alternate_screen(id).unwrap_or(false);
                let max = controller.usecase().screen_port().get_max_scrollback(id).unwrap_or(0);
                if !is_alt && max > 0 {
                    *in_scrollback = true;
                    input_handler.set_mode(InputMode::ScrollbackMode);
                }
            }
        }
        AppAction::ExitScrollback => {
            exit_scrollback_if_active(controller, input_handler, in_scrollback);
        }
        AppAction::ScrollbackUp(n) => {
            if let Some(t) = controller.usecase().get_active_terminal() {
                let id = t.id();
                let current = controller.usecase().screen_port().get_scrollback_offset(id).unwrap_or(0);
                let max = controller.usecase().screen_port().get_max_scrollback(id).unwrap_or(0);
                let new_offset = (current + n).min(max);
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, new_offset);
            }
        }
        AppAction::ScrollbackDown(n) => {
            if let Some(t) = controller.usecase().get_active_terminal() {
                let id = t.id();
                let current = controller.usecase().screen_port().get_scrollback_offset(id).unwrap_or(0);
                let new_offset = current.saturating_sub(n);
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, new_offset);
                if new_offset == 0 {
                    // Auto-exit scrollback when reaching bottom
                    *in_scrollback = false;
                    input_handler.set_mode(InputMode::Normal);
                }
            }
        }
        AppAction::ScrollbackPageUp => {
            if let Some(t) = controller.usecase().get_active_terminal() {
                let id = t.id();
                let current = controller.usecase().screen_port().get_scrollback_offset(id).unwrap_or(0);
                let max = controller.usecase().screen_port().get_max_scrollback(id).unwrap_or(0);
                let page = (size.rows as usize) / 2;
                let new_offset = (current + page).min(max);
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, new_offset);
            }
        }
        AppAction::ScrollbackPageDown => {
            if let Some(t) = controller.usecase().get_active_terminal() {
                let id = t.id();
                let current = controller.usecase().screen_port().get_scrollback_offset(id).unwrap_or(0);
                let page = (size.rows as usize) / 2;
                let new_offset = current.saturating_sub(page);
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, new_offset);
                if new_offset == 0 {
                    *in_scrollback = false;
                    input_handler.set_mode(InputMode::Normal);
                }
            }
        }
        AppAction::ScrollbackTop => {
            if let Some(t) = controller.usecase().get_active_terminal() {
                let id = t.id();
                let max = controller.usecase().screen_port().get_max_scrollback(id).unwrap_or(0);
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, max);
            }
        }
        AppAction::ScrollbackBottom => {
            if let Some(t) = controller.usecase().get_active_terminal() {
                let id = t.id();
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, 0);
            }
            *in_scrollback = false;
            input_handler.set_mode(InputMode::Normal);
        }
        other => {
            match controller.dispatch(other, size) {
                Ok(()) => {}
                Err(e) => {
                    // Silently ignore NoActiveTerminal for input forwarding
                    if !matches!(e, crate::shared::error::AppError::NoActiveTerminal) {
                        return Err(e.into());
                    }
                }
            }
        }
    }

    Ok(())
}

/// Exit scrollback mode and reset offset to 0 if currently in scrollback.
fn exit_scrollback_if_active<P: PtyPort, S: ScreenPort>(
    controller: &mut TuiController<P, S>,
    input_handler: &mut InputHandler,
    in_scrollback: &mut bool,
) {
    if *in_scrollback {
        if let Some(t) = controller.usecase().get_active_terminal() {
            let id = t.id();
            let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, 0);
        }
        *in_scrollback = false;
        input_handler.set_mode(InputMode::Normal);
    }
}

fn handle_dialog_key<P: PtyPort, S: ScreenPort>(
    key: KeyEvent,
    controller: &mut TuiController<P, S>,
    input_handler: &mut InputHandler,
    dialog: &mut DialogState,
    size: TerminalSize,
) -> anyhow::Result<()> {
    match dialog {
        DialogState::CreateTerminal { input, cursor_pos } => match key.code {
            KeyCode::Char(c) => {
                input.insert(*cursor_pos, c);
                *cursor_pos += 1;
            }
            KeyCode::Backspace => {
                if *cursor_pos > 0 {
                    input.remove(*cursor_pos - 1);
                    *cursor_pos -= 1;
                }
            }
            KeyCode::Enter => {
                let name = if input.is_empty() {
                    None
                } else {
                    Some(input.clone())
                };
                controller.dispatch(AppAction::CreateTerminal { name }, size)?;
                *dialog = DialogState::None;
                input_handler.set_mode(InputMode::Normal);
            }
            KeyCode::Esc => {
                *dialog = DialogState::None;
                input_handler.set_mode(InputMode::Normal);
            }
            _ => {}
        },
        DialogState::ConfirmClose { .. } => match key.code {
            KeyCode::Char('y') => {
                controller.dispatch(AppAction::CloseTerminal, size)?;
                *dialog = DialogState::None;
                input_handler.set_mode(InputMode::Normal);
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                *dialog = DialogState::None;
                input_handler.set_mode(InputMode::Normal);
            }
            _ => {}
        },
        DialogState::None => {}
    }
    Ok(())
}
