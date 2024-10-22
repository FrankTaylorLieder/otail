#![allow(unused)]
use anyhow::Result;
use clap::{command, Parser};
use log::info;
use rtail::ifile::IFile;

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

    info!("Starting rtail: {}", &args.path);

    let ifh = tokio::spawn(async move {
        ifile.run().await;
    });

    ifh.await?;

    Ok(())
}
