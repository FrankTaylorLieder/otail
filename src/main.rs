use std::io::stdout;

use clap::{command, Parser};
use flexi_logger::{detailed_format, FileSpec};
use log::info;
use rtail::ffile::FFile;
use rtail::ifile::IFile;
use rtail::panic::init_panic_handler;
use rtail::tui::Tui;

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

    info!("rtail starting: {:?}", args);

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
