use std::{fs::File, io::stdout};

use clap::{command, Parser};
use flexi_logger::{detailed_format, FileSpec};
use log::{error, info};
use otail::config::load_config_from;
use otail::ifile::IFile;
use otail::panic::init_panic_handler;
use otail::tui::Tui;
use otail::{backing_file::FileBackingFile, ffile::FFile};

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

    #[arg(
        short = 'c',
        long = "config",
        help = "Specify a custom config file path"
    )]
    config: Option<String>,
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

    // Load config first, exit if specified config file doesn't exist
    let config = match load_config_from(args.config) {
        Ok(config) => config,
        Err(e) => {
            error!("{}", e);
            eprintln!("{}", e);
            return Ok(());
        }
    };

    // Quickly check the file before starting... can produce a better error.
    if let Err(e) = File::open(&args.path) {
        let message = format!("Failed to open: {} - {:?}", &args.path, e);
        error!("{}", message);
        eprintln!("{}", message);
        return Ok(());
    }
    let mut ifile = IFile::new(
        &args.path,
        FileBackingFile::new_from_path(&args.path.clone())?,
    );
    let mut ffile = FFile::new("ff".to_owned(), &args.path, ifile.get_view_sender());

    let tui = Tui::new(
        args.path.clone(),
        ifile.get_view_sender(),
        ffile.get_view_sender(),
        ffile.get_ff_sender(),
        config,
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
