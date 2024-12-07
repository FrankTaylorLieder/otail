#![allow(unused)]
use std::io::stdout;

use anyhow::Result;
use clap::{command, Parser};
use log::info;
use rtail::console_view::ConsoleView;
use rtail::ifile::IFile;
use rtail::tui::Tui;
use rtail::tui_view::TuiView;
use tokio::sync::mpsc;

use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    crossterm::{
        event::{self, Event, KeyCode},
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Style, Stylize},
    symbols,
    text::{Line, Span, Text},
    widgets::{
        block::BlockExt, Block, BorderType, Borders, Cell, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, StatefulWidget, Table, TableState, Widget,
    },
    DefaultTerminal, Frame, Terminal,
};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    path: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let args = Args::parse();

    let content_view = TuiView::new("Content".to_owned(), args.path.clone());

    // TODO: Switch to a real filtered View
    let filter_view = TuiView::new("Filter".to_owned(), args.path.clone());

    let app = Tui::new(content_view, filter_view);

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    app.run(terminal).await;

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}
