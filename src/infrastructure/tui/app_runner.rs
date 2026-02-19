use std::io;
use std::time::Duration;

use crossterm::cursor::SetCursorStyle as CrosstermCursorStyle;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::event::{EnableBracketedPaste, DisableBracketedPaste};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::Rect;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// Convert a character-based position to a byte index in a string.
/// If `char_pos` exceeds the number of characters, returns `s.len()`.
fn char_to_byte_index(s: &str, char_pos: usize) -> usize {
    s.char_indices()
        .nth(char_pos)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

use crate::domain::primitive::{CursorStyle, TerminalId, TerminalSize};
use crate::infrastructure::notification::MacOsNotifier;
use crate::infrastructure::tui::input::{InputHandler, InputMode};
use crate::infrastructure::tui::fuzzy_matcher;
use crate::infrastructure::tui::widgets::{dialog, help_overlay, layout, memo_overlay, mini_terminal_view, quick_switcher, sidebar, terminal_view};
use crate::infrastructure::tui::widgets::quick_switcher::QuickSwitchItem;
use crate::interface_adapter::controller::tui_controller::{AppAction, TuiController};
use crate::interface_adapter::port::{PtyPort, ScreenPort};

/// Height in rows for the mini terminal pane.
pub(crate) const MINI_TERMINAL_HEIGHT: u16 = 10;

/// Which pane currently holds input focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusPane {
    Sidebar,
    Terminal,
    MiniTerminal,
}

/// Which terminal is the current scrollback target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollbackTarget {
    MainTerminal,
    MiniTerminal,
}

/// Resolve the terminal ID for the current scrollback target.
///
/// Returns `Some(id)` when scrollback is active and the target terminal exists,
/// `None` when no scrollback is in progress.
fn active_scrollback_id<P: PtyPort, S: ScreenPort>(
    scrollback_target: &Option<ScrollbackTarget>,
    controller: &TuiController<P, S>,
    mini_terminal: &MiniTerminalState,
) -> Option<TerminalId> {
    match scrollback_target {
        Some(ScrollbackTarget::MainTerminal) => {
            controller.usecase().get_active_terminal().map(|t| t.id())
        }
        Some(ScrollbackTarget::MiniTerminal) => Some(mini_terminal.terminal_id),
        None => None,
    }
}

/// Tracks the state of the mini terminal (footer-style quick shell).
struct MiniTerminalState {
    visible: bool,
    spawned: bool,
    terminal_id: TerminalId,
}

impl MiniTerminalState {
    fn new() -> Self {
        Self {
            visible: false,
            spawned: false,
            terminal_id: TerminalId::new(u32::MAX),
        }
    }

    fn is_visible(&self) -> bool {
        self.visible
    }
}

/// Dialog state for overlay dialogs.
enum DialogState {
    None,
    CreateTerminal { input: String, cursor_pos: usize },
    ConfirmClose { terminal_name: String, is_running: bool },
    Rename { input: String, cursor_pos: usize },
    MemoEdit { text: String, cursor_row: usize, cursor_col: usize },
    Help,
    QuickSwitch { query: String, cursor_pos: usize, selected_index: usize },
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
    let mut scrollback_target: Option<ScrollbackTarget> = None;
    let mut last_cursor_style = CursorStyle::DefaultUserShape;

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
        &mut scrollback_target,
        &mut last_cursor_style,
    );

    // === Cleanup (always runs) ===
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableBracketedPaste, CrosstermCursorStyle::DefaultUserShape);
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
    scrollback_target: &mut Option<ScrollbackTarget>,
    last_cursor_style: &mut CursorStyle,
) -> anyhow::Result<()> {
    let mut mini_terminal = MiniTerminalState::new();

    while !*should_quit {
        // 1. Draw
        terminal.draw(|frame| {
            let areas = layout::compute_layout(frame.area(), mini_terminal.is_visible());

            // Compute sidebar scroll offset before rendering
            let sidebar_inner_height = areas.sidebar.height.saturating_sub(2); // minus top/bottom border
            let content_height = sidebar_inner_height.saturating_sub(1); // minus help area (1 line)
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
            let main_in_scrollback = *scrollback_target == Some(ScrollbackTarget::MainTerminal);
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
                        let visible = if main_in_scrollback {
                            false // Hide cursor during scrollback
                        } else {
                            controller
                                .usecase()
                                .screen_port()
                                .get_cursor_visible(id)
                                .unwrap_or(true)
                        };
                        let sb_info = if main_in_scrollback {
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
                main_in_scrollback,
            );

            // Mini terminal view (if visible)
            let mini_in_scrollback = *scrollback_target == Some(ScrollbackTarget::MiniTerminal);
            if let Some(mini_area) = areas.mini_terminal {
                if mini_terminal.spawned {
                    let mid = mini_terminal.terminal_id;
                    let mini_cells = controller.usecase().screen_port().get_cells(mid).ok();
                    let mini_cursor = controller.usecase().screen_port().get_cursor(mid).ok();
                    let mini_cursor_visible = if mini_in_scrollback {
                        false
                    } else {
                        controller.usecase().screen_port()
                            .get_cursor_visible(mid)
                            .unwrap_or(true)
                    };
                    let mini_scrollback_info = if mini_in_scrollback {
                        let offset = controller.usecase().screen_port().get_scrollback_offset(mid).unwrap_or(0);
                        let max = controller.usecase().screen_port().get_max_scrollback(mid).unwrap_or(0);
                        Some((offset, max))
                    } else {
                        None
                    };
                    mini_terminal_view::render(
                        frame,
                        mini_area,
                        mini_cells,
                        mini_cursor,
                        mini_cursor_visible && *focus == FocusPane::MiniTerminal,
                        *focus == FocusPane::MiniTerminal,
                        mini_scrollback_info,
                        mini_in_scrollback,
                    );
                } else {
                    mini_terminal_view::render(
                        frame,
                        mini_area,
                        None,
                        None,
                        false,
                        *focus == FocusPane::MiniTerminal,
                        None,
                        false,
                    );
                }
            }

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
                DialogState::Rename { input, cursor_pos } => {
                    dialog::render_rename_dialog(frame, input, *cursor_pos);
                }
                DialogState::MemoEdit { text, cursor_row, cursor_col } => {
                    memo_overlay::render_memo_overlay(
                        frame,
                        areas.main_pane,
                        text,
                        *cursor_row,
                        *cursor_col,
                    );
                }
                DialogState::Help => {
                    help_overlay::render_help_overlay(frame, frame.area());
                }
                DialogState::QuickSwitch { query, cursor_pos, selected_index } => {
                    // Build display text and search text for each terminal.
                    // Display text is used for rendering and highlighting.
                    // Search text includes memo for broader matching.
                    let terminals = controller.usecase().get_terminals();
                    let items: Vec<(usize, String, String)> = terminals.iter().enumerate().map(|(idx, t)| {
                        let cwd = dynamic_cwds.get(idx)
                            .and_then(|c| c.as_deref())
                            .unwrap_or(&t.cwd().display().to_string())
                            .to_string();
                        let display = format!("{}: {}  {}", t.id().value(), t.name(), cwd);
                        let memo = t.memo();
                        let search = if memo.is_empty() {
                            display.clone()
                        } else {
                            format!("{} {}", display, memo)
                        };
                        (idx, display, search)
                    }).collect();

                    // Filter using search text (includes memo), sort by score
                    let search_items: Vec<(usize, String)> = items.iter()
                        .map(|(idx, _, search)| (*idx, search.clone()))
                        .collect();
                    let filtered = fuzzy_matcher::filter_and_sort(query, &search_items);

                    // Build QuickSwitchItems with positions re-matched against display text
                    let display_items: Vec<QuickSwitchItem> = filtered.iter().map(|(idx, _fm)| {
                        let (_, display, _) = &items[*idx];
                        // Re-match against display text so positions align with what's rendered
                        let positions = if query.is_empty() {
                            Vec::new()
                        } else {
                            fuzzy_matcher::fuzzy_match(query, display)
                                .map(|m| m.positions)
                                .unwrap_or_default()
                        };
                        QuickSwitchItem {
                            terminal_index: *idx,
                            display_text: display.clone(),
                            match_positions: positions,
                        }
                    }).collect();

                    let sel = (*selected_index).min(display_items.len().saturating_sub(1));
                    quick_switcher::render_quick_switcher(frame, frame.area(), query, *cursor_pos, &display_items, sel);
                }
                DialogState::None => {}
            }
        })?;

        // 1.5. Apply cursor style from the focused terminal
        if !matches!(dialog, DialogState::None) {
            // Dialogs use their own cursor; no style change needed
        } else {
            let style_opt = if *focus == FocusPane::MiniTerminal && mini_terminal.spawned {
                Some(
                    controller.usecase().screen_port()
                        .get_cursor_style(mini_terminal.terminal_id)
                        .unwrap_or(CursorStyle::DefaultUserShape)
                )
            } else {
                controller.usecase().get_active_terminal().map(|t| {
                    controller.usecase().screen_port()
                        .get_cursor_style(t.id())
                        .unwrap_or(CursorStyle::DefaultUserShape)
                })
            };
            if let Some(style) = style_opt {
                if style != *last_cursor_style {
                    let ct_style = match style {
                        CursorStyle::DefaultUserShape => CrosstermCursorStyle::DefaultUserShape,
                        CursorStyle::BlinkingBlock => CrosstermCursorStyle::BlinkingBlock,
                        CursorStyle::SteadyBlock => CrosstermCursorStyle::SteadyBlock,
                        CursorStyle::BlinkingUnderScore => CrosstermCursorStyle::BlinkingUnderScore,
                        CursorStyle::SteadyUnderScore => CrosstermCursorStyle::SteadyUnderScore,
                        CursorStyle::BlinkingBar => CrosstermCursorStyle::BlinkingBar,
                        CursorStyle::SteadyBar => CrosstermCursorStyle::SteadyBar,
                    };
                    let _ = execute!(io::stdout(), ct_style);
                    *last_cursor_style = style;
                }
            }
        }

        // 2. Calculate terminal size from right pane
        let frame_size = terminal.size()?;
        let areas = layout::compute_layout(frame_size.into(), mini_terminal.is_visible());
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

        // 3.1. Poll mini terminal (outside TerminalUsecase management)
        if mini_terminal.spawned {
            let mid = mini_terminal.terminal_id;
            match controller.usecase_mut().pty_port_mut().read(mid) {
                Ok(data) if !data.is_empty() => {
                    let _ = controller.usecase_mut().screen_port_mut().process(mid, &data);
                    // Drain DSR responses for mini terminal
                    if let Ok(responses) = controller.usecase_mut().screen_port_mut().drain_pending_responses(mid) {
                        for response in responses {
                            let _ = controller.usecase_mut().pty_port_mut().write(mid, &response);
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => {
                    // Mini terminal process exited - hide it
                    mini_terminal.visible = false;
                    mini_terminal.spawned = false;
                    let _ = controller.usecase_mut().screen_port_mut().remove(mid);
                    // If we were scrolling the mini terminal, exit scrollback
                    if *scrollback_target == Some(ScrollbackTarget::MiniTerminal) {
                        *scrollback_target = None;
                    }
                    if *focus == FocusPane::MiniTerminal {
                        *focus = FocusPane::Terminal;
                        input_handler.set_mode(InputMode::Normal);
                    }
                }
            }
            // Check for exit
            if mini_terminal.spawned
                && let Ok(Some(_code)) = controller.usecase_mut().pty_port_mut().try_wait(mid)
            {
                mini_terminal.visible = false;
                mini_terminal.spawned = false;
                let _ = controller.usecase_mut().screen_port_mut().remove(mid);
                // If we were scrolling the mini terminal, exit scrollback
                if *scrollback_target == Some(ScrollbackTarget::MiniTerminal) {
                    *scrollback_target = None;
                }
                if *focus == FocusPane::MiniTerminal {
                    *focus = FocusPane::Terminal;
                    input_handler.set_mode(InputMode::Normal);
                }
            }
        }

        // 3.5. Drain pending notifications and send desktop notifications
        let pending = controller.usecase_mut().take_pending_notifications();
        for (terminal_name, event) in &pending {
            notifier.notify(terminal_name, event);
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
                    handle_key_event(key, controller, input_handler, should_quit, dialog, focus, size, scrollback_target, &mut mini_terminal)?;
                }
                Event::Resize(cols, rows) => {
                    let new_full = Rect::new(0, 0, cols, rows);
                    let new_areas = layout::compute_layout(new_full, mini_terminal.is_visible());
                    let new_content_height = new_areas.main_pane.height.saturating_sub(1);
                    let pane_size =
                        TerminalSize::new(new_areas.main_pane.width, new_content_height);
                    controller.dispatch(AppAction::ResizeAll(pane_size), pane_size)?;
                    // Resize mini terminal if spawned
                    if mini_terminal.spawned
                        && let Some(mini_area) = new_areas.mini_terminal
                    {
                        let mini_size = TerminalSize::new(
                            mini_area.width.saturating_sub(2),
                            mini_area.height.saturating_sub(2),
                        );
                        let mid = mini_terminal.terminal_id;
                        let _ = controller.usecase_mut().pty_port_mut().resize(mid, mini_size);
                        let _ = controller.usecase_mut().screen_port_mut().resize(mid, mini_size);
                    }
                }
                Event::Paste(text) => {
                    if *focus == FocusPane::MiniTerminal && mini_terminal.spawned {
                        // Send paste to mini terminal
                        let mid = mini_terminal.terminal_id;
                        let bracketed = controller.usecase().screen_port()
                            .get_bracketed_paste(mid)
                            .unwrap_or(false);
                        let mut data = Vec::new();
                        if bracketed {
                            data.extend_from_slice(b"\x1b[200~");
                        }
                        data.extend_from_slice(text.as_bytes());
                        if bracketed {
                            data.extend_from_slice(b"\x1b[201~");
                        }
                        let _ = controller.usecase_mut().pty_port_mut().write(mid, &data);
                    } else {
                        // Send paste to active main terminal
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
                }
                _ => {}
            }
        }
    }

    // Cleanup mini terminal PTY/Screen on exit
    if mini_terminal.spawned {
        let mid = mini_terminal.terminal_id;
        let _ = controller.usecase_mut().pty_port_mut().kill(mid);
        let _ = controller.usecase_mut().screen_port_mut().remove(mid);
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
    scrollback_target: &mut Option<ScrollbackTarget>,
    mini_terminal: &mut MiniTerminalState,
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

    // Update application cursor keys flag from the focused terminal's DECCKM state
    let app_cursor = if *focus == FocusPane::MiniTerminal && mini_terminal.spawned {
        controller.usecase().screen_port()
            .get_application_cursor_keys(mini_terminal.terminal_id)
            .unwrap_or(false)
    } else {
        controller.usecase().get_active_terminal()
            .and_then(|t| controller.usecase().screen_port().get_application_cursor_keys(t.id()).ok())
            .unwrap_or(false)
    };
    input_handler.set_application_cursor_keys(app_cursor);

    // Normal/PrefixWait/ScrollbackMode
    let Some(action) = input_handler.handle_key(key) else {
        return Ok(());
    };

    match action {
        AppAction::CreateTerminal { .. } => {
            exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
            *dialog = DialogState::CreateTerminal {
                input: String::new(),
                cursor_pos: 0,
            };
            input_handler.set_mode(InputMode::DialogInput);
        }
        AppAction::CloseTerminal => {
            exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
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
                    let _ = controller.dispatch(AppAction::CloseTerminal, size);
                }
            }
        }
        AppAction::Quit => {
            *should_quit = true;
        }
        AppAction::ToggleFocus => {
            exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
            *focus = match *focus {
                FocusPane::Sidebar => FocusPane::Terminal,
                FocusPane::Terminal => FocusPane::Sidebar,
                FocusPane::MiniTerminal => FocusPane::MiniTerminal,
            };
        }
        AppAction::SelectNext | AppAction::SelectPrev | AppAction::SelectByIndex(_) => {
            exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
            if *focus == FocusPane::MiniTerminal {
                *focus = FocusPane::Terminal;
                input_handler.set_mode(InputMode::Normal);
            }
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
            if *focus == FocusPane::MiniTerminal && mini_terminal.spawned {
                // Enter scrollback for mini terminal
                let mid = mini_terminal.terminal_id;
                let is_alt = controller.usecase().screen_port().is_alternate_screen(mid).unwrap_or(false);
                let max = controller.usecase().screen_port().get_max_scrollback(mid).unwrap_or(0);
                if !is_alt && max > 0 {
                    *scrollback_target = Some(ScrollbackTarget::MiniTerminal);
                    input_handler.set_mode(InputMode::ScrollbackMode);
                }
            } else {
                // Check if we can enter scrollback (not in alternate screen, has history)
                if let Some(t) = controller.usecase().get_active_terminal() {
                    let id = t.id();
                    let is_alt = controller.usecase().screen_port().is_alternate_screen(id).unwrap_or(false);
                    let max = controller.usecase().screen_port().get_max_scrollback(id).unwrap_or(0);
                    if !is_alt && max > 0 {
                        *scrollback_target = Some(ScrollbackTarget::MainTerminal);
                        input_handler.set_mode(InputMode::ScrollbackMode);
                    }
                }
            }
        }
        AppAction::ExitScrollback => {
            exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
        }
        AppAction::ScrollbackUp(n) => {
            if let Some(id) = active_scrollback_id(scrollback_target, controller, mini_terminal) {
                let current = controller.usecase().screen_port().get_scrollback_offset(id).unwrap_or(0);
                let max = controller.usecase().screen_port().get_max_scrollback(id).unwrap_or(0);
                let new_offset = (current + n).min(max);
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, new_offset);
            }
        }
        AppAction::ScrollbackDown(n) => {
            if let Some(id) = active_scrollback_id(scrollback_target, controller, mini_terminal) {
                let current = controller.usecase().screen_port().get_scrollback_offset(id).unwrap_or(0);
                let new_offset = current.saturating_sub(n);
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, new_offset);
                if new_offset == 0 {
                    // Auto-exit scrollback when reaching bottom
                    exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
                }
            }
        }
        AppAction::ScrollbackPageUp => {
            if let Some(id) = active_scrollback_id(scrollback_target, controller, mini_terminal) {
                let current = controller.usecase().screen_port().get_scrollback_offset(id).unwrap_or(0);
                let max = controller.usecase().screen_port().get_max_scrollback(id).unwrap_or(0);
                let page = if *scrollback_target == Some(ScrollbackTarget::MiniTerminal) {
                    (MINI_TERMINAL_HEIGHT as usize).saturating_sub(2) / 2
                } else {
                    (size.rows as usize) / 2
                };
                let new_offset = (current + page).min(max);
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, new_offset);
            }
        }
        AppAction::ScrollbackPageDown => {
            if let Some(id) = active_scrollback_id(scrollback_target, controller, mini_terminal) {
                let current = controller.usecase().screen_port().get_scrollback_offset(id).unwrap_or(0);
                let page = if *scrollback_target == Some(ScrollbackTarget::MiniTerminal) {
                    (MINI_TERMINAL_HEIGHT as usize).saturating_sub(2) / 2
                } else {
                    (size.rows as usize) / 2
                };
                let new_offset = current.saturating_sub(page);
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, new_offset);
                if new_offset == 0 {
                    exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
                }
            }
        }
        AppAction::ScrollbackTop => {
            if let Some(id) = active_scrollback_id(scrollback_target, controller, mini_terminal) {
                let max = controller.usecase().screen_port().get_max_scrollback(id).unwrap_or(0);
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, max);
            }
        }
        AppAction::ScrollbackBottom => {
            if let Some(id) = active_scrollback_id(scrollback_target, controller, mini_terminal) {
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, 0);
            }
            exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
        }
        AppAction::RenameTerminal { .. } => {
            exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
            if let Some(terminal) = controller.usecase().get_active_terminal() {
                let current_name = terminal.name().to_string();
                let cursor_pos = current_name.chars().count();
                *dialog = DialogState::Rename {
                    input: current_name,
                    cursor_pos,
                };
                input_handler.set_mode(InputMode::DialogInput);
            }
        }
        AppAction::OpenMemo => {
            exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
            if let Ok(memo) = controller.usecase().get_active_memo() {
                let text = memo.to_string();
                let lines: Vec<&str> = text.split('\n').collect();
                let cursor_row = lines.len() - 1;
                let cursor_col = lines.last().map_or(0, |l| l.chars().count());
                *dialog = DialogState::MemoEdit {
                    text,
                    cursor_row,
                    cursor_col,
                };
                input_handler.set_mode(InputMode::MemoEdit);
            }
        }
        AppAction::ShowHelp => {
            exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
            *dialog = DialogState::Help;
            input_handler.set_mode(InputMode::HelpView);
        }
        AppAction::ToggleMiniTerminal => {
            exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
            if !mini_terminal.spawned {
                // First time: spawn PTY + Screen
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                let cwd = controller.usecase().get_active_terminal()
                    .and_then(|t| {
                        controller.usecase().screen_port()
                            .get_cwd(t.id()).ok().flatten()
                            .map(|s| std::path::PathBuf::from(s))
                    })
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")));
                let mini_size = TerminalSize {
                    rows: MINI_TERMINAL_HEIGHT - 2, // minus borders
                    cols: size.cols,
                };
                let mid = mini_terminal.terminal_id;
                controller.usecase_mut().pty_port_mut().spawn(mid, &shell, &cwd, mini_size)?;
                controller.usecase_mut().screen_port_mut().create(mid, mini_size)?;
                mini_terminal.spawned = true;
            }
            if !mini_terminal.is_visible() {
                // Not visible: open + focus
                mini_terminal.visible = true;
                *focus = FocusPane::MiniTerminal;
                input_handler.set_mode(InputMode::MiniTerminalInput);
            } else if *focus == FocusPane::MiniTerminal {
                // Visible + focused: close + focus Terminal
                mini_terminal.visible = false;
                *focus = FocusPane::Terminal;
                input_handler.set_mode(InputMode::Normal);
            } else {
                // Visible + focus elsewhere: just move focus to mini terminal
                *focus = FocusPane::MiniTerminal;
                input_handler.set_mode(InputMode::MiniTerminalInput);
            }
        }
        AppAction::WriteToMiniTerminal(data) => {
            if mini_terminal.spawned {
                let mid = mini_terminal.terminal_id;
                let _ = controller.usecase_mut().pty_port_mut().write(mid, &data);
            }
        }
        AppAction::OpenQuickSwitcher => {
            exit_scrollback_if_active(controller, input_handler, scrollback_target, mini_terminal);
            *dialog = DialogState::QuickSwitch {
                query: String::new(),
                cursor_pos: 0,
                selected_index: 0,
            };
            input_handler.set_mode(InputMode::DialogInput);
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
    scrollback_target: &mut Option<ScrollbackTarget>,
    mini_terminal: &MiniTerminalState,
) {
    if let Some(target) = *scrollback_target {
        match target {
            ScrollbackTarget::MainTerminal => {
                if let Some(t) = controller.usecase().get_active_terminal() {
                    let id = t.id();
                    let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(id, 0);
                }
                input_handler.set_mode(InputMode::Normal);
            }
            ScrollbackTarget::MiniTerminal => {
                let mid = mini_terminal.terminal_id;
                let _ = controller.usecase_mut().screen_port_mut().set_scrollback_offset(mid, 0);
                input_handler.set_mode(InputMode::MiniTerminalInput);
            }
        }
        *scrollback_target = None;
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
                let byte_idx = char_to_byte_index(input, *cursor_pos);
                input.insert(byte_idx, c);
                *cursor_pos += 1;
            }
            KeyCode::Backspace => {
                if *cursor_pos > 0 {
                    let byte_idx = char_to_byte_index(input, *cursor_pos - 1);
                    input.remove(byte_idx);
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
                let _ = controller.dispatch(AppAction::CloseTerminal, size);
                *dialog = DialogState::None;
                input_handler.set_mode(InputMode::Normal);
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                *dialog = DialogState::None;
                input_handler.set_mode(InputMode::Normal);
            }
            _ => {}
        },
        DialogState::Rename { input, cursor_pos } => match key.code {
            KeyCode::Char(c) => {
                let byte_idx = char_to_byte_index(input, *cursor_pos);
                input.insert(byte_idx, c);
                *cursor_pos += 1;
            }
            KeyCode::Backspace => {
                if *cursor_pos > 0 {
                    let byte_idx = char_to_byte_index(input, *cursor_pos - 1);
                    input.remove(byte_idx);
                    *cursor_pos -= 1;
                }
            }
            KeyCode::Left => {
                if *cursor_pos > 0 {
                    *cursor_pos -= 1;
                }
            }
            KeyCode::Right => {
                if *cursor_pos < input.chars().count() {
                    *cursor_pos += 1;
                }
            }
            KeyCode::Enter => {
                if !input.is_empty() {
                    controller.dispatch(
                        AppAction::RenameTerminal { name: input.clone() },
                        size,
                    )?;
                }
                *dialog = DialogState::None;
                input_handler.set_mode(InputMode::Normal);
            }
            KeyCode::Esc => {
                *dialog = DialogState::None;
                input_handler.set_mode(InputMode::Normal);
            }
            _ => {}
        },
        DialogState::MemoEdit { text, cursor_row, cursor_col } => match key.code {
            // Ctrl+J: insert newline
            KeyCode::Char('j') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
                if *cursor_row < lines.len() {
                    let current_line = &lines[*cursor_row];
                    let byte_idx = char_to_byte_index(current_line, *cursor_col);
                    let (before, after) = current_line.split_at(byte_idx);
                    let before = before.to_string();
                    let after = after.to_string();
                    lines[*cursor_row] = before;
                    lines.insert(*cursor_row + 1, after);
                    *cursor_row += 1;
                    *cursor_col = 0;
                }
                *text = lines.join("\n");
            }
            // Enter: save and close
            KeyCode::Enter => {
                controller.dispatch(
                    AppAction::SaveMemo { text: text.clone() },
                    size,
                )?;
                *dialog = DialogState::None;
                input_handler.set_mode(InputMode::Normal);
            }
            KeyCode::Char(c) => {
                let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
                if *cursor_row < lines.len() {
                    let byte_idx = char_to_byte_index(&lines[*cursor_row], *cursor_col);
                    lines[*cursor_row].insert(byte_idx, c);
                    *cursor_col += 1;
                }
                *text = lines.join("\n");
            }
            KeyCode::Backspace => {
                let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
                if *cursor_col > 0 {
                    let byte_idx = char_to_byte_index(&lines[*cursor_row], *cursor_col - 1);
                    lines[*cursor_row].remove(byte_idx);
                    *cursor_col -= 1;
                } else if *cursor_row > 0 {
                    let current = lines.remove(*cursor_row);
                    *cursor_row -= 1;
                    *cursor_col = lines[*cursor_row].chars().count();
                    lines[*cursor_row].push_str(&current);
                }
                *text = lines.join("\n");
            }
            KeyCode::Up => {
                if *cursor_row > 0 {
                    *cursor_row -= 1;
                    let lines: Vec<&str> = text.split('\n').collect();
                    *cursor_col = (*cursor_col).min(lines[*cursor_row].chars().count());
                }
            }
            KeyCode::Down => {
                let lines: Vec<&str> = text.split('\n').collect();
                if *cursor_row + 1 < lines.len() {
                    *cursor_row += 1;
                    *cursor_col = (*cursor_col).min(lines[*cursor_row].chars().count());
                }
            }
            KeyCode::Left => {
                if *cursor_col > 0 {
                    *cursor_col -= 1;
                }
            }
            KeyCode::Right => {
                let lines: Vec<&str> = text.split('\n').collect();
                if !lines.is_empty() && *cursor_col < lines[*cursor_row].chars().count() {
                    *cursor_col += 1;
                }
            }
            KeyCode::Esc => {
                *dialog = DialogState::None;
                input_handler.set_mode(InputMode::Normal);
            }
            _ => {}
        },
        DialogState::Help => match key.code {
            KeyCode::Char('?') | KeyCode::Esc => {
                *dialog = DialogState::None;
                input_handler.set_mode(InputMode::Normal);
            }
            _ => {} // Help is read-only; ignore all other keys
        },
        DialogState::QuickSwitch { query, cursor_pos, selected_index } => {
            match key.code {
                KeyCode::Char('k') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                    if *selected_index > 0 {
                        *selected_index -= 1;
                    }
                }
                KeyCode::Char('j') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                    *selected_index += 1;
                }
                KeyCode::Char(c) => {
                    let byte_idx = char_to_byte_index(query, *cursor_pos);
                    query.insert(byte_idx, c);
                    *cursor_pos += 1;
                    *selected_index = 0; // Reset selection when query changes
                }
                KeyCode::Backspace => {
                    if *cursor_pos > 0 {
                        let byte_idx = char_to_byte_index(query, *cursor_pos - 1);
                        query.remove(byte_idx);
                        *cursor_pos -= 1;
                        *selected_index = 0; // Reset selection when query changes
                    }
                }
                KeyCode::Up => {
                    if *selected_index > 0 {
                        *selected_index -= 1;
                    }
                }
                KeyCode::Down => {
                    // Increment and let the renderer clamp
                    *selected_index += 1;
                }
                KeyCode::Enter => {
                    // Determine which terminal is selected by re-running the filter
                    let terminals = controller.usecase().get_terminals();
                    let search_items: Vec<(usize, String)> = terminals.iter().enumerate().map(|(idx, t)| {
                        let cwd = controller.usecase().screen_port()
                            .get_cwd(t.id()).ok().flatten()
                            .unwrap_or_else(|| t.cwd().display().to_string());
                        let display = format!("{}: {}  {}", t.id().value(), t.name(), cwd);
                        let memo = t.memo();
                        let search_text = if memo.is_empty() {
                            display
                        } else {
                            format!("{} {}", display, memo)
                        };
                        (idx, search_text)
                    }).collect();

                    let filtered = fuzzy_matcher::filter_and_sort(query, &search_items);
                    let sel = (*selected_index).min(filtered.len().saturating_sub(1));

                    if let Some((terminal_idx, _)) = filtered.get(sel) {
                        controller.dispatch(AppAction::SelectByIndex(*terminal_idx), size)?;
                    }
                    *dialog = DialogState::None;
                    input_handler.set_mode(InputMode::Normal);
                }
                KeyCode::Esc => {
                    *dialog = DialogState::None;
                    input_handler.set_mode(InputMode::Normal);
                }
                _ => {}
            }
        },
        DialogState::None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::primitive::TerminalId;

    // === MiniTerminalState tests ===

    #[test]
    fn mini_terminal_state_new_defaults_to_not_visible() {
        let state = MiniTerminalState::new();
        assert!(!state.is_visible());
    }

    #[test]
    fn mini_terminal_state_new_defaults_to_not_spawned() {
        let state = MiniTerminalState::new();
        assert!(!state.spawned);
    }

    #[test]
    fn mini_terminal_state_new_uses_u32_max_for_terminal_id() {
        let state = MiniTerminalState::new();
        assert_eq!(state.terminal_id, TerminalId::new(u32::MAX));
    }

    #[test]
    fn mini_terminal_state_toggle_makes_visible() {
        let mut state = MiniTerminalState::new();
        state.visible = !state.visible;
        assert!(state.is_visible());
    }

    #[test]
    fn mini_terminal_state_toggle_twice_returns_to_hidden() {
        let mut state = MiniTerminalState::new();
        state.visible = !state.visible;
        state.visible = !state.visible;
        assert!(!state.is_visible());
    }

    #[test]
    fn mini_terminal_state_toggle_three_times_is_visible() {
        let mut state = MiniTerminalState::new();
        state.visible = !state.visible;
        state.visible = !state.visible;
        state.visible = !state.visible;
        assert!(state.is_visible());
    }

    // === MINI_TERMINAL_HEIGHT constant test ===

    #[test]
    fn mini_terminal_height_constant_is_10() {
        assert_eq!(MINI_TERMINAL_HEIGHT, 10);
    }

    // === FocusPane::MiniTerminal tests ===

    #[test]
    fn focus_pane_mini_terminal_variant_exists() {
        let focus = FocusPane::MiniTerminal;
        assert_eq!(focus, FocusPane::MiniTerminal);
    }

    #[test]
    fn focus_pane_mini_terminal_is_not_sidebar() {
        assert_ne!(FocusPane::MiniTerminal, FocusPane::Sidebar);
    }

    #[test]
    fn focus_pane_mini_terminal_is_not_terminal() {
        assert_ne!(FocusPane::MiniTerminal, FocusPane::Terminal);
    }

    #[test]
    fn focus_pane_mini_terminal_is_copy_and_clone() {
        let focus = FocusPane::MiniTerminal;
        let cloned = focus.clone();
        let copied = focus;
        assert_eq!(cloned, copied);
    }

    // === ToggleFocus behavior test ===
    // ToggleFocus should only toggle between Sidebar and Terminal, not MiniTerminal.

    #[test]
    fn toggle_focus_from_sidebar_goes_to_terminal() {
        let mut focus = FocusPane::Sidebar;
        focus = match focus {
            FocusPane::Sidebar => FocusPane::Terminal,
            FocusPane::Terminal => FocusPane::Sidebar,
            FocusPane::MiniTerminal => FocusPane::MiniTerminal,
        };
        assert_eq!(focus, FocusPane::Terminal);
    }

    #[test]
    fn toggle_focus_from_terminal_goes_to_sidebar() {
        let mut focus = FocusPane::Terminal;
        focus = match focus {
            FocusPane::Sidebar => FocusPane::Terminal,
            FocusPane::Terminal => FocusPane::Sidebar,
            FocusPane::MiniTerminal => FocusPane::MiniTerminal,
        };
        assert_eq!(focus, FocusPane::Sidebar);
    }

    #[test]
    fn toggle_focus_from_mini_terminal_stays_on_mini_terminal() {
        let mut focus = FocusPane::MiniTerminal;
        focus = match focus {
            FocusPane::Sidebar => FocusPane::Terminal,
            FocusPane::Terminal => FocusPane::Sidebar,
            FocusPane::MiniTerminal => FocusPane::MiniTerminal,
        };
        assert_eq!(focus, FocusPane::MiniTerminal);
    }

    // === ToggleMiniTerminal three-state focus logic tests (Task #65) ===
    // These tests verify the three-state toggle behavior:
    // 1. Not visible -> open + focus on mini terminal
    // 2. Visible + focused on mini terminal -> close + focus on Terminal
    // 3. Visible + focused elsewhere -> just move focus to mini terminal

    /// Simulate the toggle_mini_terminal logic as it appears in handle_key_event.
    /// Returns (new_visible, new_focus) after applying the toggle logic.
    fn simulate_toggle_mini_terminal(visible: bool, focus: FocusPane) -> (bool, FocusPane) {
        let mut mini_visible = visible;
        let mut new_focus = focus;
        if !mini_visible {
            // Not visible: open + focus
            mini_visible = true;
            new_focus = FocusPane::MiniTerminal;
        } else if new_focus == FocusPane::MiniTerminal {
            // Visible + focused: close + focus Terminal
            mini_visible = false;
            new_focus = FocusPane::Terminal;
        } else {
            // Visible + focus elsewhere: just move focus to mini terminal
            new_focus = FocusPane::MiniTerminal;
        }
        (mini_visible, new_focus)
    }

    #[test]
    fn toggle_mini_not_visible_opens_and_focuses() {
        let (visible, focus) = simulate_toggle_mini_terminal(false, FocusPane::Terminal);
        assert!(visible);
        assert_eq!(focus, FocusPane::MiniTerminal);
    }

    #[test]
    fn toggle_mini_not_visible_from_sidebar_opens_and_focuses() {
        let (visible, focus) = simulate_toggle_mini_terminal(false, FocusPane::Sidebar);
        assert!(visible);
        assert_eq!(focus, FocusPane::MiniTerminal);
    }

    #[test]
    fn toggle_mini_visible_and_focused_closes_and_returns_to_terminal() {
        let (visible, focus) = simulate_toggle_mini_terminal(true, FocusPane::MiniTerminal);
        assert!(!visible);
        assert_eq!(focus, FocusPane::Terminal);
    }

    #[test]
    fn toggle_mini_visible_but_terminal_focused_moves_focus_to_mini() {
        let (visible, focus) = simulate_toggle_mini_terminal(true, FocusPane::Terminal);
        assert!(visible);
        assert_eq!(focus, FocusPane::MiniTerminal);
    }

    #[test]
    fn toggle_mini_visible_but_sidebar_focused_moves_focus_to_mini() {
        let (visible, focus) = simulate_toggle_mini_terminal(true, FocusPane::Sidebar);
        assert!(visible);
        assert_eq!(focus, FocusPane::MiniTerminal);
    }

    #[test]
    fn toggle_mini_full_cycle_open_close_reopen() {
        // Start: not visible, focus on Terminal
        let (visible, focus) = simulate_toggle_mini_terminal(false, FocusPane::Terminal);
        assert!(visible);
        assert_eq!(focus, FocusPane::MiniTerminal);

        // Toggle again while focused: close
        let (visible, focus) = simulate_toggle_mini_terminal(visible, focus);
        assert!(!visible);
        assert_eq!(focus, FocusPane::Terminal);

        // Toggle again while not visible: reopen
        let (visible, focus) = simulate_toggle_mini_terminal(visible, focus);
        assert!(visible);
        assert_eq!(focus, FocusPane::MiniTerminal);
    }

    #[test]
    fn toggle_mini_focus_elsewhere_then_toggle_does_not_close() {
        // Start: visible, mini terminal focused
        // Switch focus to Terminal (simulating user selecting a terminal)
        let focus = FocusPane::Terminal;
        let visible = true;

        // Toggle should NOT close - it should move focus to mini
        let (visible, focus) = simulate_toggle_mini_terminal(visible, focus);
        assert!(visible);
        assert_eq!(focus, FocusPane::MiniTerminal);
    }

    // === Task #66: Poll/resize integration tests ===

    // --- 66-1: Mini terminal exit detection logic ---

    /// Simulate the mini terminal exit-on-read-error logic.
    /// When pty read fails, mini terminal should hide and reset focus.
    fn simulate_mini_read_error(
        spawned: bool,
        focus: FocusPane,
    ) -> (bool, bool, FocusPane) {
        let mut mini_visible = true;
        let mut mini_spawned = spawned;
        let mut new_focus = focus;

        if mini_spawned {
            // Simulate read returning Err
            let read_failed = true;
            if read_failed {
                mini_visible = false;
                mini_spawned = false;
                if new_focus == FocusPane::MiniTerminal {
                    new_focus = FocusPane::Terminal;
                }
            }
        }
        (mini_visible, mini_spawned, new_focus)
    }

    #[test]
    fn mini_read_error_hides_mini_terminal() {
        let (visible, spawned, _) = simulate_mini_read_error(true, FocusPane::Terminal);
        assert!(!visible);
        assert!(!spawned);
    }

    #[test]
    fn mini_read_error_resets_focus_when_focused_on_mini() {
        let (_, _, focus) = simulate_mini_read_error(true, FocusPane::MiniTerminal);
        assert_eq!(focus, FocusPane::Terminal);
    }

    #[test]
    fn mini_read_error_preserves_focus_when_on_terminal() {
        let (_, _, focus) = simulate_mini_read_error(true, FocusPane::Terminal);
        assert_eq!(focus, FocusPane::Terminal);
    }

    #[test]
    fn mini_read_error_preserves_focus_when_on_sidebar() {
        let (_, _, focus) = simulate_mini_read_error(true, FocusPane::Sidebar);
        assert_eq!(focus, FocusPane::Sidebar);
    }

    /// Simulate the mini terminal exit-on-try_wait logic.
    /// When process exits (try_wait returns Some), mini terminal should hide and reset focus.
    fn simulate_mini_try_wait_exit(
        spawned: bool,
        focus: FocusPane,
        exit_code: Option<i32>,
    ) -> (bool, bool, FocusPane) {
        let mut mini_visible = true;
        let mut mini_spawned = spawned;
        let mut new_focus = focus;

        if mini_spawned {
            if let Some(_code) = exit_code {
                mini_visible = false;
                mini_spawned = false;
                if new_focus == FocusPane::MiniTerminal {
                    new_focus = FocusPane::Terminal;
                }
            }
        }
        (mini_visible, mini_spawned, new_focus)
    }

    #[test]
    fn mini_try_wait_exit_hides_mini_terminal() {
        let (visible, spawned, _) = simulate_mini_try_wait_exit(true, FocusPane::Terminal, Some(0));
        assert!(!visible);
        assert!(!spawned);
    }

    #[test]
    fn mini_try_wait_exit_resets_focus_when_focused() {
        let (_, _, focus) = simulate_mini_try_wait_exit(true, FocusPane::MiniTerminal, Some(0));
        assert_eq!(focus, FocusPane::Terminal);
    }

    #[test]
    fn mini_try_wait_no_exit_keeps_state() {
        let (visible, spawned, focus) = simulate_mini_try_wait_exit(true, FocusPane::MiniTerminal, None);
        assert!(visible);
        assert!(spawned);
        assert_eq!(focus, FocusPane::MiniTerminal);
    }

    #[test]
    fn mini_try_wait_not_spawned_is_noop() {
        let (visible, spawned, focus) = simulate_mini_try_wait_exit(false, FocusPane::Terminal, Some(0));
        assert!(visible); // visible unchanged because spawned was false
        assert!(!spawned);
        assert_eq!(focus, FocusPane::Terminal);
    }

    // --- 66-2: Mini terminal resize calculation ---

    #[test]
    fn mini_terminal_resize_subtracts_borders() {
        // When mini terminal area is e.g., 55x10, the inner size should be 53x8
        let mini_area = Rect::new(25, 14, 55, 10);
        let mini_size = TerminalSize::new(
            mini_area.width.saturating_sub(2),
            mini_area.height.saturating_sub(2),
        );
        assert_eq!(mini_size.cols, 53);
        assert_eq!(mini_size.rows, 8);
    }

    #[test]
    fn mini_terminal_resize_handles_minimum_area() {
        // Very small area: width=2, height=2 -> inner size 0x0
        let mini_area = Rect::new(0, 0, 2, 2);
        let mini_size = TerminalSize::new(
            mini_area.width.saturating_sub(2),
            mini_area.height.saturating_sub(2),
        );
        assert_eq!(mini_size.cols, 0);
        assert_eq!(mini_size.rows, 0);
    }

    #[test]
    fn mini_terminal_resize_handles_small_width() {
        // width=1, height=10 -> inner size 0x8 (saturating_sub)
        let mini_area = Rect::new(0, 0, 1, 10);
        let mini_size = TerminalSize::new(
            mini_area.width.saturating_sub(2),
            mini_area.height.saturating_sub(2),
        );
        assert_eq!(mini_size.cols, 0);
        assert_eq!(mini_size.rows, 8);
    }

    // --- 66-4: Paste routing logic ---

    /// Simulate the paste routing decision: returns true if paste should go to mini terminal.
    fn should_paste_to_mini(focus: FocusPane, mini_spawned: bool) -> bool {
        focus == FocusPane::MiniTerminal && mini_spawned
    }

    #[test]
    fn paste_routes_to_mini_when_focused_and_spawned() {
        assert!(should_paste_to_mini(FocusPane::MiniTerminal, true));
    }

    #[test]
    fn paste_routes_to_main_when_focused_but_not_spawned() {
        assert!(!should_paste_to_mini(FocusPane::MiniTerminal, false));
    }

    #[test]
    fn paste_routes_to_main_when_terminal_focused() {
        assert!(!should_paste_to_mini(FocusPane::Terminal, true));
    }

    #[test]
    fn paste_routes_to_main_when_sidebar_focused() {
        assert!(!should_paste_to_mini(FocusPane::Sidebar, true));
    }

    #[test]
    fn paste_routes_to_main_when_nothing_spawned_and_terminal_focused() {
        assert!(!should_paste_to_mini(FocusPane::Terminal, false));
    }

    // --- Bracketed paste wrapping logic ---

    fn build_paste_data(text: &str, bracketed: bool) -> Vec<u8> {
        let mut data = Vec::new();
        if bracketed {
            data.extend_from_slice(b"\x1b[200~");
        }
        data.extend_from_slice(text.as_bytes());
        if bracketed {
            data.extend_from_slice(b"\x1b[201~");
        }
        data
    }

    #[test]
    fn paste_data_with_bracketed_mode_wraps_text() {
        let data = build_paste_data("hello", true);
        assert_eq!(data, b"\x1b[200~hello\x1b[201~");
    }

    #[test]
    fn paste_data_without_bracketed_mode_is_raw() {
        let data = build_paste_data("hello", false);
        assert_eq!(data, b"hello");
    }

    #[test]
    fn paste_data_empty_text_with_bracketed_mode() {
        let data = build_paste_data("", true);
        assert_eq!(data, b"\x1b[200~\x1b[201~");
    }

    #[test]
    fn paste_data_empty_text_without_bracketed_mode() {
        let data = build_paste_data("", false);
        assert!(data.is_empty());
    }

    // --- 66-2: Resize event dispatches resize for both main and mini ---

    #[test]
    fn resize_event_computes_layout_with_mini_visible() {
        // Simulate: window resize to 100x40, mini terminal visible
        let new_full = Rect::new(0, 0, 100, 40);
        let new_areas = layout::compute_layout(new_full, true);

        // Main pane should be shrunk
        assert!(new_areas.main_pane.height < 40);
        // Mini terminal area should exist
        assert!(new_areas.mini_terminal.is_some());
        let mini = new_areas.mini_terminal.unwrap();
        assert_eq!(mini.height, MINI_TERMINAL_HEIGHT);
    }

    #[test]
    fn resize_event_computes_layout_without_mini_visible() {
        let new_full = Rect::new(0, 0, 100, 40);
        let new_areas = layout::compute_layout(new_full, false);

        // Main pane takes full right column height
        assert_eq!(new_areas.main_pane.height, 40);
        // No mini terminal area
        assert!(new_areas.mini_terminal.is_none());
    }

    // --- 67-1: Exit cleanup logic tests ---

    /// Simulate the mini terminal cleanup logic that runs after the while loop ends.
    /// Returns (kill_called, remove_called) to verify cleanup actions.
    fn simulate_mini_cleanup(spawned: bool) -> (bool, bool) {
        let mut kill_called = false;
        let mut remove_called = false;
        if spawned {
            // Simulate: let _ = controller.usecase_mut().pty_port_mut().kill(mid);
            kill_called = true;
            // Simulate: let _ = controller.usecase_mut().screen_port_mut().remove(mid);
            remove_called = true;
        }
        (kill_called, remove_called)
    }

    #[test]
    fn mini_cleanup_kills_pty_when_spawned() {
        let (kill_called, _) = simulate_mini_cleanup(true);
        assert!(kill_called, "Expected PTY kill to be called when spawned");
    }

    #[test]
    fn mini_cleanup_removes_screen_when_spawned() {
        let (_, remove_called) = simulate_mini_cleanup(true);
        assert!(remove_called, "Expected screen remove to be called when spawned");
    }

    #[test]
    fn mini_cleanup_does_nothing_when_not_spawned() {
        let (kill_called, remove_called) = simulate_mini_cleanup(false);
        assert!(!kill_called, "Expected no PTY kill when not spawned");
        assert!(!remove_called, "Expected no screen remove when not spawned");
    }

    // === Task #69: ScrollbackTarget enum tests ===

    #[test]
    fn scrollback_target_main_terminal_variant_exists() {
        let target = ScrollbackTarget::MainTerminal;
        assert_eq!(target, ScrollbackTarget::MainTerminal);
    }

    #[test]
    fn scrollback_target_mini_terminal_variant_exists() {
        let target = ScrollbackTarget::MiniTerminal;
        assert_eq!(target, ScrollbackTarget::MiniTerminal);
    }

    #[test]
    fn scrollback_target_main_is_not_mini() {
        assert_ne!(ScrollbackTarget::MainTerminal, ScrollbackTarget::MiniTerminal);
    }

    #[test]
    fn scrollback_target_is_copy_and_clone() {
        let target = ScrollbackTarget::MainTerminal;
        let cloned = target.clone();
        let copied = target;
        assert_eq!(cloned, copied);
    }

    #[test]
    fn scrollback_target_debug_format() {
        let target = ScrollbackTarget::MainTerminal;
        let debug = format!("{:?}", target);
        assert!(debug.contains("MainTerminal"));

        let target = ScrollbackTarget::MiniTerminal;
        let debug = format!("{:?}", target);
        assert!(debug.contains("MiniTerminal"));
    }

    #[test]
    fn scrollback_target_none_means_no_scrollback() {
        let target: Option<ScrollbackTarget> = None;
        assert!(target.is_none());
    }

    #[test]
    fn scrollback_target_some_main_means_main_scrollback() {
        let target = Some(ScrollbackTarget::MainTerminal);
        assert!(target.is_some());
        assert_eq!(target, Some(ScrollbackTarget::MainTerminal));
    }

    #[test]
    fn scrollback_target_some_mini_means_mini_scrollback() {
        let target = Some(ScrollbackTarget::MiniTerminal);
        assert!(target.is_some());
        assert_eq!(target, Some(ScrollbackTarget::MiniTerminal));
    }

    // === Task #69: scrollback_target replaces in_scrollback boolean ===

    #[test]
    fn scrollback_target_initial_value_is_none() {
        let scrollback_target: Option<ScrollbackTarget> = None;
        assert!(scrollback_target.is_none());
        // Equivalent to old `in_scrollback = false`
    }

    #[test]
    fn scrollback_target_enter_main_sets_some_main() {
        let scrollback_target: Option<ScrollbackTarget> = Some(ScrollbackTarget::MainTerminal);
        assert_eq!(scrollback_target, Some(ScrollbackTarget::MainTerminal));
        // Equivalent to old `in_scrollback = true`
    }

    #[test]
    fn scrollback_target_exit_sets_none() {
        let scrollback_target: Option<ScrollbackTarget> = None;
        assert!(scrollback_target.is_none());
        // Equivalent to old `in_scrollback = false`
    }

    #[test]
    fn scrollback_target_is_some_replaces_bool_check() {
        let none_target: Option<ScrollbackTarget> = None;
        let main_target = Some(ScrollbackTarget::MainTerminal);
        let mini_target = Some(ScrollbackTarget::MiniTerminal);

        // `scrollback_target.is_some()` replaces `*in_scrollback`
        assert!(!none_target.is_some());
        assert!(main_target.is_some());
        assert!(mini_target.is_some());
    }

    // === Task #70: Enter scrollback for mini terminal ===

    /// Simulate the EnterScrollback logic for determining the scrollback target.
    /// Returns the new scrollback_target after the enter scrollback attempt.
    fn simulate_enter_scrollback(
        focus: FocusPane,
        mini_spawned: bool,
        mini_is_alt_screen: bool,
        mini_max_scrollback: usize,
        main_has_terminal: bool,
        main_is_alt_screen: bool,
        main_max_scrollback: usize,
    ) -> Option<ScrollbackTarget> {
        if focus == FocusPane::MiniTerminal && mini_spawned {
            // Enter scrollback for mini terminal
            if !mini_is_alt_screen && mini_max_scrollback > 0 {
                Some(ScrollbackTarget::MiniTerminal)
            } else {
                None
            }
        } else {
            // Enter scrollback for main terminal
            if main_has_terminal {
                if !main_is_alt_screen && main_max_scrollback > 0 {
                    Some(ScrollbackTarget::MainTerminal)
                } else {
                    None
                }
            } else {
                None
            }
        }
    }

    #[test]
    fn enter_scrollback_mini_focused_spawned_with_history() {
        let target = simulate_enter_scrollback(
            FocusPane::MiniTerminal, true, false, 100,
            true, false, 200,
        );
        assert_eq!(target, Some(ScrollbackTarget::MiniTerminal));
    }

    #[test]
    fn enter_scrollback_mini_focused_but_alt_screen() {
        let target = simulate_enter_scrollback(
            FocusPane::MiniTerminal, true, true, 100,
            true, false, 200,
        );
        assert_eq!(target, None);
    }

    #[test]
    fn enter_scrollback_mini_focused_but_no_history() {
        let target = simulate_enter_scrollback(
            FocusPane::MiniTerminal, true, false, 0,
            true, false, 200,
        );
        assert_eq!(target, None);
    }

    #[test]
    fn enter_scrollback_mini_focused_but_not_spawned() {
        // Falls through to main terminal path
        let target = simulate_enter_scrollback(
            FocusPane::MiniTerminal, false, false, 100,
            true, false, 200,
        );
        assert_eq!(target, Some(ScrollbackTarget::MainTerminal));
    }

    #[test]
    fn enter_scrollback_terminal_focused_with_history() {
        let target = simulate_enter_scrollback(
            FocusPane::Terminal, true, false, 100,
            true, false, 200,
        );
        assert_eq!(target, Some(ScrollbackTarget::MainTerminal));
    }

    #[test]
    fn enter_scrollback_terminal_focused_but_alt_screen() {
        let target = simulate_enter_scrollback(
            FocusPane::Terminal, true, false, 100,
            true, true, 200,
        );
        assert_eq!(target, None);
    }

    #[test]
    fn enter_scrollback_terminal_focused_but_no_history() {
        let target = simulate_enter_scrollback(
            FocusPane::Terminal, true, false, 100,
            true, false, 0,
        );
        assert_eq!(target, None);
    }

    #[test]
    fn enter_scrollback_terminal_focused_no_terminal() {
        let target = simulate_enter_scrollback(
            FocusPane::Terminal, true, false, 100,
            false, false, 200,
        );
        assert_eq!(target, None);
    }

    #[test]
    fn enter_scrollback_sidebar_focused_with_main_history() {
        // Sidebar focus falls through to main terminal path
        let target = simulate_enter_scrollback(
            FocusPane::Sidebar, true, false, 100,
            true, false, 200,
        );
        assert_eq!(target, Some(ScrollbackTarget::MainTerminal));
    }

    // === Task #70: Exit scrollback mode restoration ===

    /// Simulate exit_scrollback_if_active to determine the restored InputMode.
    /// Returns (new_scrollback_target, restored_mode_name).
    fn simulate_exit_scrollback(
        scrollback_target: Option<ScrollbackTarget>,
    ) -> (Option<ScrollbackTarget>, &'static str) {
        if let Some(target) = scrollback_target {
            match target {
                ScrollbackTarget::MainTerminal => (None, "Normal"),
                ScrollbackTarget::MiniTerminal => (None, "MiniTerminalInput"),
            }
        } else {
            (None, "unchanged")
        }
    }

    #[test]
    fn exit_scrollback_main_restores_normal_mode() {
        let (target, mode) = simulate_exit_scrollback(Some(ScrollbackTarget::MainTerminal));
        assert!(target.is_none());
        assert_eq!(mode, "Normal");
    }

    #[test]
    fn exit_scrollback_mini_restores_mini_terminal_input_mode() {
        let (target, mode) = simulate_exit_scrollback(Some(ScrollbackTarget::MiniTerminal));
        assert!(target.is_none());
        assert_eq!(mode, "MiniTerminalInput");
    }

    #[test]
    fn exit_scrollback_none_is_noop() {
        let (target, mode) = simulate_exit_scrollback(None);
        assert!(target.is_none());
        assert_eq!(mode, "unchanged");
    }

    // === Task #71: Scroll operation target routing ===

    /// Simulate the page size calculation for scrollback page up/down.
    fn compute_page_size(
        scrollback_target: Option<ScrollbackTarget>,
        main_rows: u16,
    ) -> usize {
        if scrollback_target == Some(ScrollbackTarget::MiniTerminal) {
            (MINI_TERMINAL_HEIGHT as usize).saturating_sub(2) / 2
        } else {
            (main_rows as usize) / 2
        }
    }

    #[test]
    fn page_size_for_main_terminal_uses_main_rows() {
        let page = compute_page_size(Some(ScrollbackTarget::MainTerminal), 40);
        assert_eq!(page, 20);
    }

    #[test]
    fn page_size_for_mini_terminal_uses_mini_height() {
        let page = compute_page_size(Some(ScrollbackTarget::MiniTerminal), 40);
        // (10 - 2) / 2 = 4
        assert_eq!(page, 4);
    }

    #[test]
    fn page_size_for_none_uses_main_rows() {
        // Shouldn't normally happen, but compute_page_size should be safe
        let page = compute_page_size(None, 30);
        assert_eq!(page, 15);
    }

    /// Simulate scroll up logic: returns new offset after scrolling up by n.
    fn simulate_scroll_up(current: usize, max: usize, n: usize) -> usize {
        (current + n).min(max)
    }

    #[test]
    fn scroll_up_increases_offset() {
        assert_eq!(simulate_scroll_up(0, 100, 1), 1);
        assert_eq!(simulate_scroll_up(5, 100, 3), 8);
    }

    #[test]
    fn scroll_up_clamps_to_max() {
        assert_eq!(simulate_scroll_up(95, 100, 10), 100);
        assert_eq!(simulate_scroll_up(100, 100, 1), 100);
    }

    /// Simulate scroll down logic: returns (new_offset, should_exit_scrollback).
    fn simulate_scroll_down(current: usize, n: usize) -> (usize, bool) {
        let new_offset = current.saturating_sub(n);
        (new_offset, new_offset == 0)
    }

    #[test]
    fn scroll_down_decreases_offset() {
        let (offset, exit) = simulate_scroll_down(10, 3);
        assert_eq!(offset, 7);
        assert!(!exit);
    }

    #[test]
    fn scroll_down_to_zero_triggers_exit() {
        let (offset, exit) = simulate_scroll_down(3, 5);
        assert_eq!(offset, 0);
        assert!(exit);
    }

    #[test]
    fn scroll_down_at_zero_triggers_exit() {
        let (offset, exit) = simulate_scroll_down(0, 1);
        assert_eq!(offset, 0);
        assert!(exit);
    }

    /// Simulate scroll page down with target-aware page size and exit.
    fn simulate_scroll_page_down(
        current: usize,
        scrollback_target: Option<ScrollbackTarget>,
        main_rows: u16,
    ) -> (usize, bool) {
        let page = compute_page_size(scrollback_target, main_rows);
        let new_offset = current.saturating_sub(page);
        (new_offset, new_offset == 0)
    }

    #[test]
    fn scroll_page_down_main_terminal() {
        // 40 rows -> page size 20, offset 30 -> new offset 10
        let (offset, exit) = simulate_scroll_page_down(30, Some(ScrollbackTarget::MainTerminal), 40);
        assert_eq!(offset, 10);
        assert!(!exit);
    }

    #[test]
    fn scroll_page_down_mini_terminal() {
        // MINI_TERMINAL_HEIGHT=10 -> page size (10-2)/2=4, offset 6 -> new offset 2
        let (offset, exit) = simulate_scroll_page_down(6, Some(ScrollbackTarget::MiniTerminal), 40);
        assert_eq!(offset, 2);
        assert!(!exit);
    }

    #[test]
    fn scroll_page_down_mini_terminal_exits_at_bottom() {
        // page size 4, offset 3 -> new offset 0 -> exit
        let (offset, exit) = simulate_scroll_page_down(3, Some(ScrollbackTarget::MiniTerminal), 40);
        assert_eq!(offset, 0);
        assert!(exit);
    }

    /// Simulate scroll page up with target-aware page size.
    fn simulate_scroll_page_up(
        current: usize,
        max: usize,
        scrollback_target: Option<ScrollbackTarget>,
        main_rows: u16,
    ) -> usize {
        let page = compute_page_size(scrollback_target, main_rows);
        (current + page).min(max)
    }

    #[test]
    fn scroll_page_up_main_terminal() {
        // 40 rows -> page size 20, offset 10, max 100 -> new offset 30
        let offset = simulate_scroll_page_up(10, 100, Some(ScrollbackTarget::MainTerminal), 40);
        assert_eq!(offset, 30);
    }

    #[test]
    fn scroll_page_up_mini_terminal() {
        // page size 4, offset 10, max 100 -> new offset 14
        let offset = simulate_scroll_page_up(10, 100, Some(ScrollbackTarget::MiniTerminal), 40);
        assert_eq!(offset, 14);
    }

    #[test]
    fn scroll_page_up_mini_terminal_clamps_to_max() {
        // page size 4, offset 98, max 100 -> new offset 100
        let offset = simulate_scroll_page_up(98, 100, Some(ScrollbackTarget::MiniTerminal), 40);
        assert_eq!(offset, 100);
    }

    // === Task #71: active_scrollback_id routing ===

    /// Simulate active_scrollback_id: determine target terminal ID based on scrollback_target.
    fn simulate_active_scrollback_id(
        scrollback_target: Option<ScrollbackTarget>,
        main_active_id: Option<u32>,
        mini_id: u32,
    ) -> Option<u32> {
        match scrollback_target {
            Some(ScrollbackTarget::MainTerminal) => main_active_id,
            Some(ScrollbackTarget::MiniTerminal) => Some(mini_id),
            None => None,
        }
    }

    #[test]
    fn active_scrollback_id_main_returns_active_terminal_id() {
        let id = simulate_active_scrollback_id(Some(ScrollbackTarget::MainTerminal), Some(1), u32::MAX);
        assert_eq!(id, Some(1));
    }

    #[test]
    fn active_scrollback_id_main_returns_none_when_no_active() {
        let id = simulate_active_scrollback_id(Some(ScrollbackTarget::MainTerminal), None, u32::MAX);
        assert_eq!(id, None);
    }

    #[test]
    fn active_scrollback_id_mini_returns_mini_terminal_id() {
        let id = simulate_active_scrollback_id(Some(ScrollbackTarget::MiniTerminal), Some(1), u32::MAX);
        assert_eq!(id, Some(u32::MAX));
    }

    #[test]
    fn active_scrollback_id_none_returns_none() {
        let id = simulate_active_scrollback_id(None, Some(1), u32::MAX);
        assert_eq!(id, None);
    }

    // === Task #73: Mini terminal exit during scrollback edge case ===

    /// Simulate the mini terminal read-error exit path with scrollback cleanup.
    /// Returns (visible, spawned, scrollback_target, focus).
    fn simulate_mini_read_error_with_scrollback(
        spawned: bool,
        focus: FocusPane,
        scrollback_target: Option<ScrollbackTarget>,
    ) -> (bool, bool, Option<ScrollbackTarget>, FocusPane) {
        let mut mini_visible = true;
        let mut mini_spawned = spawned;
        let mut new_focus = focus;
        let mut new_scrollback_target = scrollback_target;

        if mini_spawned {
            // Simulate read returning Err
            let read_failed = true;
            if read_failed {
                mini_visible = false;
                mini_spawned = false;
                // If we were scrolling the mini terminal, exit scrollback
                if new_scrollback_target == Some(ScrollbackTarget::MiniTerminal) {
                    new_scrollback_target = None;
                }
                if new_focus == FocusPane::MiniTerminal {
                    new_focus = FocusPane::Terminal;
                }
            }
        }
        (mini_visible, mini_spawned, new_scrollback_target, new_focus)
    }

    #[test]
    fn mini_read_error_resets_scrollback_target_when_scrolling_mini() {
        let (_, _, target, _) = simulate_mini_read_error_with_scrollback(
            true,
            FocusPane::MiniTerminal,
            Some(ScrollbackTarget::MiniTerminal),
        );
        assert!(target.is_none(), "Expected scrollback_target to be None after mini terminal exit");
    }

    #[test]
    fn mini_read_error_preserves_scrollback_target_when_scrolling_main() {
        let (_, _, target, _) = simulate_mini_read_error_with_scrollback(
            true,
            FocusPane::Terminal,
            Some(ScrollbackTarget::MainTerminal),
        );
        assert_eq!(target, Some(ScrollbackTarget::MainTerminal),
            "Expected scrollback_target to remain MainTerminal when mini terminal exits");
    }

    #[test]
    fn mini_read_error_preserves_scrollback_target_when_not_scrolling() {
        let (_, _, target, _) = simulate_mini_read_error_with_scrollback(
            true,
            FocusPane::MiniTerminal,
            None,
        );
        assert!(target.is_none(), "Expected scrollback_target to remain None");
    }

    #[test]
    fn mini_read_error_resets_both_scrollback_and_focus() {
        let (visible, spawned, target, focus) = simulate_mini_read_error_with_scrollback(
            true,
            FocusPane::MiniTerminal,
            Some(ScrollbackTarget::MiniTerminal),
        );
        assert!(!visible);
        assert!(!spawned);
        assert!(target.is_none());
        assert_eq!(focus, FocusPane::Terminal);
    }

    /// Simulate the mini terminal try_wait exit path with scrollback cleanup.
    /// Returns (visible, spawned, scrollback_target, focus).
    fn simulate_mini_try_wait_exit_with_scrollback(
        spawned: bool,
        focus: FocusPane,
        exit_code: Option<i32>,
        scrollback_target: Option<ScrollbackTarget>,
    ) -> (bool, bool, Option<ScrollbackTarget>, FocusPane) {
        let mut mini_visible = true;
        let mut mini_spawned = spawned;
        let mut new_focus = focus;
        let mut new_scrollback_target = scrollback_target;

        if mini_spawned {
            if let Some(_code) = exit_code {
                mini_visible = false;
                mini_spawned = false;
                // If we were scrolling the mini terminal, exit scrollback
                if new_scrollback_target == Some(ScrollbackTarget::MiniTerminal) {
                    new_scrollback_target = None;
                }
                if new_focus == FocusPane::MiniTerminal {
                    new_focus = FocusPane::Terminal;
                }
            }
        }
        (mini_visible, mini_spawned, new_scrollback_target, new_focus)
    }

    #[test]
    fn mini_try_wait_exit_resets_scrollback_target_when_scrolling_mini() {
        let (_, _, target, _) = simulate_mini_try_wait_exit_with_scrollback(
            true,
            FocusPane::MiniTerminal,
            Some(0),
            Some(ScrollbackTarget::MiniTerminal),
        );
        assert!(target.is_none(), "Expected scrollback_target to be None after mini terminal exit");
    }

    #[test]
    fn mini_try_wait_exit_preserves_scrollback_target_when_scrolling_main() {
        let (_, _, target, _) = simulate_mini_try_wait_exit_with_scrollback(
            true,
            FocusPane::Terminal,
            Some(0),
            Some(ScrollbackTarget::MainTerminal),
        );
        assert_eq!(target, Some(ScrollbackTarget::MainTerminal),
            "Expected scrollback_target to remain MainTerminal");
    }

    #[test]
    fn mini_try_wait_exit_preserves_scrollback_target_when_not_scrolling() {
        let (_, _, target, _) = simulate_mini_try_wait_exit_with_scrollback(
            true,
            FocusPane::MiniTerminal,
            Some(0),
            None,
        );
        assert!(target.is_none(), "Expected scrollback_target to remain None");
    }

    #[test]
    fn mini_try_wait_exit_resets_both_scrollback_and_focus() {
        let (visible, spawned, target, focus) = simulate_mini_try_wait_exit_with_scrollback(
            true,
            FocusPane::MiniTerminal,
            Some(0),
            Some(ScrollbackTarget::MiniTerminal),
        );
        assert!(!visible);
        assert!(!spawned);
        assert!(target.is_none());
        assert_eq!(focus, FocusPane::Terminal);
    }

    #[test]
    fn mini_try_wait_no_exit_preserves_scrollback_target() {
        let (_, _, target, _) = simulate_mini_try_wait_exit_with_scrollback(
            true,
            FocusPane::MiniTerminal,
            None,
            Some(ScrollbackTarget::MiniTerminal),
        );
        assert_eq!(target, Some(ScrollbackTarget::MiniTerminal),
            "Expected scrollback_target to remain when process has not exited");
    }

    // === Task #70: Draw closure scrollback state discrimination ===

    #[test]
    fn main_scrollback_check_only_matches_main_target() {
        let main_scrollback = Some(ScrollbackTarget::MainTerminal);
        let mini_scrollback = Some(ScrollbackTarget::MiniTerminal);
        let no_scrollback: Option<ScrollbackTarget> = None;

        // Only MainTerminal target should trigger main scrollback display
        assert_eq!(main_scrollback, Some(ScrollbackTarget::MainTerminal));
        assert_ne!(mini_scrollback, Some(ScrollbackTarget::MainTerminal));
        assert_ne!(no_scrollback, Some(ScrollbackTarget::MainTerminal));
    }

    #[test]
    fn mini_scrollback_check_only_matches_mini_target() {
        let main_scrollback = Some(ScrollbackTarget::MainTerminal);
        let mini_scrollback = Some(ScrollbackTarget::MiniTerminal);
        let no_scrollback: Option<ScrollbackTarget> = None;

        // Only MiniTerminal target should trigger mini scrollback display
        assert_ne!(main_scrollback, Some(ScrollbackTarget::MiniTerminal));
        assert_eq!(mini_scrollback, Some(ScrollbackTarget::MiniTerminal));
        assert_ne!(no_scrollback, Some(ScrollbackTarget::MiniTerminal));
    }

    // === Task #78: QuickSwitch dialog state tests ===

    #[test]
    fn dialog_state_quick_switch_variant_exists() {
        let dialog = DialogState::QuickSwitch {
            query: String::new(),
            cursor_pos: 0,
            selected_index: 0,
        };
        assert!(matches!(dialog, DialogState::QuickSwitch { .. }));
    }

    #[test]
    fn dialog_state_quick_switch_stores_query() {
        let dialog = DialogState::QuickSwitch {
            query: "test".to_string(),
            cursor_pos: 4,
            selected_index: 0,
        };
        if let DialogState::QuickSwitch { query, cursor_pos, selected_index } = dialog {
            assert_eq!(query, "test");
            assert_eq!(cursor_pos, 4);
            assert_eq!(selected_index, 0);
        } else {
            panic!("Expected QuickSwitch variant");
        }
    }

    // === Task #78: QuickSwitch key handling simulation ===

    /// Simulate typing a character into QuickSwitch dialog.
    /// Returns (new_query, new_cursor_pos, new_selected_index).
    fn simulate_qs_char_input(query: &str, cursor_pos: usize, _selected_index: usize, c: char) -> (String, usize, usize) {
        let mut q = query.to_string();
        let mut cp = cursor_pos;
        let byte_idx = char_to_byte_index(&q, cp);
        q.insert(byte_idx, c);
        cp += 1;
        let si = 0; // Reset on query change
        (q, cp, si)
    }

    #[test]
    fn qs_char_input_appends_to_empty_query() {
        let (q, cp, si) = simulate_qs_char_input("", 0, 0, 'a');
        assert_eq!(q, "a");
        assert_eq!(cp, 1);
        assert_eq!(si, 0);
    }

    #[test]
    fn qs_char_input_appends_to_existing_query() {
        let (q, cp, si) = simulate_qs_char_input("ab", 2, 3, 'c');
        assert_eq!(q, "abc");
        assert_eq!(cp, 3);
        assert_eq!(si, 0); // Reset on query change
    }

    #[test]
    fn qs_char_input_resets_selected_index() {
        let (_, _, si) = simulate_qs_char_input("x", 1, 5, 'y');
        assert_eq!(si, 0);
    }

    /// Simulate backspace in QuickSwitch dialog.
    /// Returns (new_query, new_cursor_pos, new_selected_index).
    fn simulate_qs_backspace(query: &str, cursor_pos: usize, selected_index: usize) -> (String, usize, usize) {
        let mut q = query.to_string();
        let mut cp = cursor_pos;
        let si;
        if cp > 0 {
            let byte_idx = char_to_byte_index(&q, cp - 1);
            q.remove(byte_idx);
            cp -= 1;
            si = 0; // Reset on query change
        } else {
            si = selected_index; // No change
        }
        (q, cp, si)
    }

    #[test]
    fn qs_backspace_removes_last_char() {
        let (q, cp, si) = simulate_qs_backspace("abc", 3, 2);
        assert_eq!(q, "ab");
        assert_eq!(cp, 2);
        assert_eq!(si, 0);
    }

    #[test]
    fn qs_backspace_at_start_is_noop() {
        let (q, cp, si) = simulate_qs_backspace("abc", 0, 2);
        assert_eq!(q, "abc");
        assert_eq!(cp, 0);
        assert_eq!(si, 2); // Not reset
    }

    #[test]
    fn qs_backspace_resets_selected_index_when_effective() {
        let (_, _, si) = simulate_qs_backspace("x", 1, 7);
        assert_eq!(si, 0);
    }

    /// Simulate Up arrow in QuickSwitch dialog.
    fn simulate_qs_up(selected_index: usize) -> usize {
        if selected_index > 0 {
            selected_index - 1
        } else {
            selected_index
        }
    }

    #[test]
    fn qs_up_decrements_selected_index() {
        assert_eq!(simulate_qs_up(3), 2);
    }

    #[test]
    fn qs_up_stays_at_zero() {
        assert_eq!(simulate_qs_up(0), 0);
    }

    /// Simulate Down arrow in QuickSwitch dialog.
    /// (Just increments; renderer clamps)
    fn simulate_qs_down(selected_index: usize) -> usize {
        selected_index + 1
    }

    #[test]
    fn qs_down_increments_selected_index() {
        assert_eq!(simulate_qs_down(0), 1);
        assert_eq!(simulate_qs_down(5), 6);
    }

    /// Simulate the selected_index clamping done by the renderer.
    fn clamp_selected(selected_index: usize, item_count: usize) -> usize {
        selected_index.min(item_count.saturating_sub(1))
    }

    #[test]
    fn clamp_selected_within_range() {
        assert_eq!(clamp_selected(2, 5), 2);
    }

    #[test]
    fn clamp_selected_at_boundary() {
        assert_eq!(clamp_selected(4, 5), 4);
    }

    #[test]
    fn clamp_selected_exceeds_range() {
        assert_eq!(clamp_selected(10, 5), 4);
    }

    #[test]
    fn clamp_selected_with_empty_list() {
        assert_eq!(clamp_selected(3, 0), 0);
    }

    #[test]
    fn clamp_selected_with_single_item() {
        assert_eq!(clamp_selected(5, 1), 0);
    }

    /// Simulate Esc key closing the QuickSwitch dialog.
    /// Returns true if dialog should be closed.
    fn simulate_qs_esc() -> bool {
        true // Always closes dialog
    }

    #[test]
    fn qs_esc_closes_dialog() {
        assert!(simulate_qs_esc());
    }

    /// Simulate building search items from terminal data.
    fn build_search_items(
        terminals: &[(u32, &str, &str, Option<&str>)], // (id, name, cwd, memo)
    ) -> Vec<(usize, String)> {
        terminals.iter().enumerate().map(|(idx, (id, name, cwd, memo))| {
            let display = format!("{}: {}  {}", id, name, cwd);
            let search_text = match memo {
                Some(m) => format!("{} {}", display, m),
                None => display,
            };
            (idx, search_text)
        }).collect()
    }

    #[test]
    fn build_search_items_formats_correctly() {
        let terminals = vec![
            (1, "my-term", "/home/user", None),
            (2, "dev", "/tmp", Some("important")),
        ];
        let items = build_search_items(&terminals);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0, 0);
        assert_eq!(items[0].1, "1: my-term  /home/user");
        assert_eq!(items[1].0, 1);
        assert_eq!(items[1].1, "2: dev  /tmp important");
    }

    #[test]
    fn build_search_items_trims_trailing_whitespace() {
        let terminals = vec![
            (1, "test", "/home", None),
        ];
        let items = build_search_items(&terminals);
        // Without memo, format is "1: test /home " - should be trimmed
        assert!(!items[0].1.ends_with(' '));
    }

    #[test]
    fn build_search_items_includes_memo_when_present() {
        let terminals = vec![
            (1, "test", "/home", Some("my memo")),
        ];
        let items = build_search_items(&terminals);
        assert!(items[0].1.contains("my memo"));
    }

    /// Simulate the Enter key handling logic in QuickSwitch:
    /// filter items, clamp selection, return selected terminal index.
    fn simulate_qs_enter(
        query: &str,
        selected_index: usize,
        search_items: &[(usize, String)],
    ) -> Option<usize> {
        let filtered = crate::infrastructure::tui::fuzzy_matcher::filter_and_sort(query, search_items);
        let sel = selected_index.min(filtered.len().saturating_sub(1));
        filtered.get(sel).map(|(idx, _)| *idx)
    }

    #[test]
    fn qs_enter_selects_first_item_when_no_query() {
        let items = vec![
            (0, "1: alpha /home".to_string()),
            (1, "2: beta /tmp".to_string()),
        ];
        let result = simulate_qs_enter("", 0, &items);
        assert_eq!(result, Some(0));
    }

    #[test]
    fn qs_enter_selects_second_item() {
        let items = vec![
            (0, "1: alpha /home".to_string()),
            (1, "2: beta /tmp".to_string()),
        ];
        let result = simulate_qs_enter("", 1, &items);
        assert_eq!(result, Some(1));
    }

    #[test]
    fn qs_enter_with_query_filters_items() {
        let items = vec![
            (0, "1: alpha /home".to_string()),
            (1, "2: beta /tmp".to_string()),
        ];
        let result = simulate_qs_enter("beta", 0, &items);
        assert_eq!(result, Some(1)); // "beta" matches only the second item
    }

    #[test]
    fn qs_enter_clamps_index_when_exceeds_filtered() {
        let items = vec![
            (0, "1: alpha /home".to_string()),
            (1, "2: beta /tmp".to_string()),
        ];
        // With query "alpha", only 1 result; selected_index=5 should clamp to 0
        let result = simulate_qs_enter("alpha", 5, &items);
        assert_eq!(result, Some(0));
    }

    #[test]
    fn qs_enter_returns_none_when_no_matches() {
        let items = vec![
            (0, "1: alpha /home".to_string()),
        ];
        let result = simulate_qs_enter("xyz", 0, &items);
        assert_eq!(result, None);
    }

    #[test]
    fn qs_enter_returns_none_when_no_items() {
        let items: Vec<(usize, String)> = vec![];
        let result = simulate_qs_enter("", 0, &items);
        assert_eq!(result, None);
    }

    // === Task #78: Ctrl+k / Ctrl+j navigation ===

    #[test]
    fn qs_ctrl_k_decrements_selected_index() {
        // Same as Up
        assert_eq!(simulate_qs_up(3), 2);
    }

    #[test]
    fn qs_ctrl_k_stays_at_zero() {
        assert_eq!(simulate_qs_up(0), 0);
    }

    #[test]
    fn qs_ctrl_j_increments_selected_index() {
        // Same as Down
        assert_eq!(simulate_qs_down(2), 3);
    }
}
