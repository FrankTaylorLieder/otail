# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

otail is a TUI-based two-pane log file viewer written in Rust using ratatui. It provides real-time log file viewing with filtering capabilities in a vim-like interface.

## Common Commands

### Building and Running
- `cargo build` - Build the project
- `cargo run <file>` - Run otail with a log file
- `cargo run --release <file>` - Run optimized build
- `cargo install --path .` - Install locally
- `RUST_LOG=trace cargo run <file>` - Run with debug logging

### Development Commands
- `cargo check` - Quick syntax/type check
- `cargo clippy` - Lint the codebase
- `cargo fmt` - Format code
- `cargo test` - Run tests (note: unit tests are in backlog)

### Installation
- `cargo install --path .` - Install from local clone
- `cargo install --git https://github.com/FrankTaylorLieder/otail.git` - Install from git

## Architecture

The application follows an async actor-like pattern with three main components:

### Core Components
- **main.rs**: Entry point, initializes logging and spawns core tasks
- **tui.rs**: Terminal user interface handling, renders two-pane view
- **ifile.rs**: Input file handler, manages file reading and tailing
- **ffile.rs**: Filtered file handler, applies regex filters to content
- **view.rs**: Shared view abstraction for both panes

### Supporting Modules
- **backing_file.rs**: File system abstraction layer
- **reader.rs**: Low-level file reading with change notification
- **common.rs**: Shared types and utilities
- **panic.rs**: Panic handler initialization

### Communication Pattern
The architecture uses tokio channels for async communication:
- `IFile` reads from disk and sends content to views
- `FFile` receives content from `IFile`, applies filters, sends to filtered view
- `Tui` coordinates between components and handles user input

### Key Data Flow
1. File changes detected by `Reader` using filesystem notifications
2. `IFile` processes changes and updates line cache
3. Content sent to both main view and `FFile` for filtering  
4. `FFile` applies regex filters and maintains filtered line cache
5. `Tui` renders both panes with synchronized scrolling

## Dependencies

Key external dependencies:
- **ratatui**: Terminal UI framework
- **tokio**: Async runtime
- **crossterm**: Cross-platform terminal handling
- **notify**: File system change notifications
- **regex**: Pattern matching for filters
- **clap**: Command line argument parsing

## Development Guidelines
- We'll keep a log of all changes made to the project: the request, the plan and the changes. This will be accumulated in DEVELOPMENT.md. Always add to the log for every change we make to the project.
