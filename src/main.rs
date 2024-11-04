#![allow(unused)]
use anyhow::Result;
use clap::{command, Parser};
use log::info;
use rtail::console_view::ConsoleView;
use rtail::ifile::IFile;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    path: String,
}

#[tokio::main]
pub async fn main() -> Result<()> {
    env_logger::init();

    let args = Args::parse();

    let (update_sender, update_receiver) = mpsc::channel(10);
    let mut ifile = IFile::new(&args.path);

    let mut view = ConsoleView::new(
        args.path.to_owned(),
        ifile.get_view_sender(),
        update_sender,
        update_receiver,
    );

    info!("Starting rtail: {}", &args.path);

    let vh = tokio::spawn(async move {
        view.run().await;
    });
    let ifh = tokio::spawn(async move {
        ifile.run().await;
    });

    ifh.await?;
    vh.await?;

    Ok(())
}
