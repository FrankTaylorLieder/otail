# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

otail is a TUI-based two-pane log file viewer written in Rust. It displays a full log file in the top pane and a filtered view in the bottom pane, with real-time tailing capabilities and VIM-like key bindings.

## Development Commands

### Building and Running
- `cargo build` - Build the project
- `cargo run -- <file>` - Run with a log file
- `cargo install --path .` - Install locally
- `RUST_LOG=trace cargo run -- <file>` - Run with debug logging

### Testing
- `cargo test` - Run all tests
- `cargo test --lib` - Run library tests only

### Code Quality
- `cargo clippy` - Run linter
- `cargo fmt` - Format code
- `cargo check` - Quick compile check

## Architecture

The application follows an actor-based architecture with async message passing:

### Core Components

- **main.rs**: Entry point that sets up logging, parses CLI args, and orchestrates the three main components
- **IFile** (`ifile.rs`): Manages the input file, handles file watching, line caching, and serves content to views
- **FFile** (`ffile.rs`): Manages filtered content, applies regex filters, and maintains filtered line mappings
- **Tui** (`tui.rs`): Handles terminal UI rendering, user input, and coordinates between file managers

### Key Patterns

- Each component runs in its own async task communicating via mpsc channels
- **View** (`view.rs`): Shared abstraction for pane state management (scrolling, tailing, line tracking)
- **BackingFile** (`backing_file.rs`): Low-level file operations with change detection via notify crate
- **Reader** (`reader.rs`): Async file reading with line-based chunking and partial line handling

### Message Flow

1. IFile watches the source file and caches lines on demand
2. FFile subscribes to IFile updates and applies filters
3. Tui receives updates from both and renders the dual-pane interface
4. User input in Tui triggers filter changes or view commands via message channels

## Dependencies

- **ratatui**: Terminal UI framework
- **tokio**: Async runtime
- **notify**: File system event monitoring
- **regex**: Pattern matching for filters
- **crossterm**: Cross-platform terminal control