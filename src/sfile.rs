use std::fs::File;
use std::io::{BufRead, BufReader};
use std::iter::once;
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct SLine {
    index: u32,
    length: u32,
}

#[derive(Default, Debug)]
pub struct SFile {
    path: PathBuf,
    file: Option<File>,
    lines: Vec<SLine>,
    line_count: u32,
    byte_count: u32,
}

pub enum SFileError {
    General(String),
}

type Result<T> = std::result::Result<T, SFileError>;

impl SFile {
    pub fn new(path: &str) -> SFile {
        let mut pb = PathBuf::new();
        pb.push(path);
        SFile {
            path: pb,
            ..Default::default()
        }
    }

    pub fn survey(&mut self) -> Result<()> {
        let file = File::open(&self.path).map_err(|e| SFileError::General(e.to_string()))?;

        let mut br = BufReader::new(file);
        let mut buf = String::new();

        loop {
            let bytes = br
                .read_line(&mut buf)
                .map_err(|e| SFileError::General(e.to_string()))?;

            if bytes == 0 {
                break;
            }

            self.byte_count += bytes as u32;
            self.line_count += 1;
            self.lines.push(SLine {
                index: self.line_count,
                length: bytes as u32,
            });
            buf.clear();
        }

        println!("Survey: {:#?}", self);

        Ok(())
    }
}
