use std::{fs::File, io::stdout};

use clap::{command, Parser};
use flexi_logger::{detailed_format, FileSpec};
use log::{error, info};
use otail::ffile::FFile;
use otail::ifile::IFile;
use otail::panic::init_panic_handler;
use otail::tui::Tui;

use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    Terminal,
};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    path: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_panic_handler();

    flexi_logger::Logger::try_with_env()?
        .log_to_file(FileSpec::default().suffix("log").use_timestamp(false))
        .append()
        .format(detailed_format)
        .start()?;

    let args = Args::parse();

    info!("otail starting: {:?}", args);

    // Quickly check the file before starting... can produce a better error.
    if let Err(e) = File::open(&args.path) {
        let message = format!("Failed to open: {} - {:?}", &args.path, e);
        error!("{}", message);
        eprintln!("{}", message);
        return Ok(());
    }
    let mut ifile = IFile::new(&args.path)?;
    let mut ffile = FFile::new("ff".to_owned(), &args.path, ifile.get_view_sender());

    let tui = Tui::new(
        args.path.clone(),
        ifile.get_view_sender(),
        ffile.get_view_sender(),
        ffile.get_ff_sender(),
    );

    tokio::spawn(async move {
        let result = ifile.run().await;
        info!("IFile finished: {:?}", result);
    });

    tokio::spawn(async move {
        let result = ffile.run().await;
        info!("FFile finished: {:?}", result);
    });

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    tui.run(terminal).await?;

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}
