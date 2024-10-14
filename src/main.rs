#![allow(unused)]
use clap::{command, Parser};
use rlog::sfile::SFile;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    path: String,
}

pub fn main() {
    let args = Args::parse();
    let mut sfile = SFile::new(&args.path);
    sfile.survey();
}
