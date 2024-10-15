#![allow(unused)]
use clap::{command, Parser};
use rtail::ifile::IFile;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    path: String,
}

pub fn main() {
    let args = Args::parse();
    let mut ifile = IFile::new(&args.path);
    ifile.survey();
}
