use anyhow::Result;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek};
use std::path::PathBuf;

#[derive(Debug)]
pub struct BackingFile {
    br: BufReader<File>,
}

impl BackingFile {
    pub fn new(path: PathBuf) -> Result<Self> {
        let file = File::open(path.clone())?;
        let bf = Self {
            br: BufReader::new(file),
        };

        Ok(bf)
    }

    pub fn read_line(&mut self, offset: u64) -> Result<String> {
        self.br.seek(io::SeekFrom::Start(offset))?;

        let mut line = String::new();
        self.br.read_line(&mut line)?;

        let mut replaced_line = line.replace("\t", " ");

        // Remove trailing newline if present
        BackingFile::trim_line_end(&mut replaced_line);

        Ok(replaced_line)
    }

    fn trim_line_end(line: &mut String) -> bool {
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }

            false
        } else {
            true
        }
    }
}
