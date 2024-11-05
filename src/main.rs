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

    let mut ifile = IFile::new(&args.path);

    let mut view = ConsoleView::new(
        "ConsoleViewTop".to_owned(),
        args.path.to_owned(),
        ifile.get_view_sender(),
    );

    let mut view_tail = ConsoleView::new(
        "ConsoleViewTail".to_owned(),
        args.path.to_owned(),
        ifile.get_view_sender(),
    );
    view_tail.set_tail(true);

    info!("Starting rtail: {}", &args.path);

    let vh = tokio::spawn(async move {
        view.run().await;
    });
    let vht = tokio::spawn(async move {
        view_tail.run().await;
    });
    let ifh = tokio::spawn(async move {
        ifile.run().await;
    });

    ifh.await?;
    vht.await?;
    vh.await?;

    Ok(())
}
