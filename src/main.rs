#![allow(unused)]
use std::io::stdout;

use anyhow::Result;
use clap::{command, Parser};
use flexi_logger::{detailed_format, FileSpec};
use log::{debug, info};
use rtail::tui::Tui;
use rtail::{ifile::IFile, view::View};
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

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    path: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    flexi_logger::Logger::try_with_env()?
        .log_to_file(FileSpec::default().suffix("log").use_timestamp(false))
        .append()
        .format(detailed_format)
        .start()?;

    let args = Args::parse();

    info!("rtail starting: {:?}", args);

    let mut ifile = IFile::new(&args.path);

    let (content_ifr_send, content_ifr_recv) = mpsc::channel(10);
    let mut content_view = View::new(
        "Content".to_owned(),
        args.path.clone(),
        ifile.get_view_sender(),
        content_ifr_send,
    );

    // TODO: Switch to a real filtered View
    let (filter_ifr_send, filter_ifr_recv) = mpsc::channel(10);
    let filter_view = View::new(
        "Filter".to_owned(),
        args.path.clone(),
        ifile.get_view_sender(),
        filter_ifr_send,
    );

    let tui = Tui::new(
        args.path.clone(),
        content_view,
        filter_view,
        content_ifr_recv,
        filter_ifr_recv,
    );

    let ifh = tokio::spawn(async move {
        ifile.run().await;
    });

    XXX Here
    // TODO: Arrange for view senders to be sent to the ifile.

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    tui.run(terminal).await;

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    debug!("Waiting for ifile to finish");
    //ifh.await;

    Ok(())
}
