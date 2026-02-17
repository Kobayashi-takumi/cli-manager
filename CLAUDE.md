# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

CLI Manager is a TUI-based terminal multiplexer written in Rust. It manages multiple interactive CLI processes (e.g., Claude Code) via pseudo-terminals (pty), allowing users to switch between them in a 2-pane TUI interface. Think of it as a "window manager for CLI processes."

**Status:** Pre-implementation. Only design docs exist under `docs/`. No source code yet.

## Tech Stack

- Rust edition 2024
- ratatui 0.30 + crossterm 0.29 (TUI)
- portable-pty 0.9 (pty management)
- vte 0.15 (ANSI escape parser)
- thiserror 2.0 + anyhow 1.0 (error handling)
- Target OS: macOS (Linux later)

## Build & Run Commands

```bash
cargo check          # Type check
cargo build          # Build
cargo run            # Run the TUI app
cargo test           # Run tests
cargo test <name>    # Run a single test by name
cargo clippy         # Lint
```

## Architecture

Clean Architecture with strict dependency direction: `infrastructure → interface_adapter → usecase → domain`.

### Layer Responsibilities

| Layer | Purpose | External crate deps |
|-------|---------|-------------------|
| `domain/` | Entities (`ManagedTerminal`), value objects (`TerminalId`, `TerminalStatus`, `TerminalSize`, `Cell`, `CursorPos`) | None (pure Rust) |
| `usecase/` | Terminal management logic (`TerminalUsecase<P: PtyPort, S: ScreenPort>`) | None (depends on port traits) |
| `interface_adapter/` | Port traits (`PtyPort`, `ScreenPort`), adapter factories, TUI controller (`AppAction` dispatch) | None |
| `infrastructure/` | Concrete implementations: `PortablePtyAdapter`, `VteScreenAdapter`, ratatui TUI (app_runner, input handler, widgets), `main.rs` DI | ratatui, crossterm, portable-pty, vte, anyhow |
| `shared/` | `AppError` enum | thiserror |

### Key Design Decisions

- **Ports in `interface_adapter/port/`**: `PtyPort` and `ScreenPort` traits define the boundary between usecase and infrastructure. Usecase depends only on these traits via generics.
- **DI in `main.rs`**: Concrete adapters are assembled and injected at the entry point.
- **Prefix key model**: `Ctrl+t` is the prefix key (like tmux). `InputHandler` manages `Normal → PrefixWait → Normal` state machine with 1-second timeout.
- **Domain purity**: `domain/` and `usecase/` must never depend on external crates.

### Data Flow

```
KeyEvent → InputHandler → AppAction → TuiController → TerminalUsecase → PtyPort/ScreenPort
PTY stdout → poll_all() → ScreenPort.process() → cells grid → ratatui render
```

## Implementation Phases

Implementation follows inside-out ordering defined in `docs/tasks/implementation-tasks.md`:

1. **Phase 0**: Project init (Cargo.toml, directory structure, mod.rs)
2. **Phase 1**: domain + shared (value objects, entities, error types)
3. **Phase 2**: port traits + usecase
4. **Phase 3**: infrastructure concrete (pty adapter, vte screen adapter)
5. **Phase 4**: controller + TUI (input handler, widgets, app_runner)
6. **Phase 5**: DI assembly + integration testing

Detailed per-phase task breakdowns are in `docs/tasks/phase-{0..5}-*.md`.

## Design Docs

- `docs/requirements.md` — Functional requirements, UI layout spec, keybindings
- `docs/detailed-design.md` — Full architecture, code signatures for all layers, component diagrams
- `docs/tasks/implementation-tasks.md` — Task dependency graph and progress tracking

Always consult the detailed design doc before implementing any component.
