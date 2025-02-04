use anyhow::Result;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek};
use std::path::PathBuf;

#[derive(Debug)]
pub struct BackingFile {
    br: BufReader<File>,
}

impl BackingFile {
    pub fn new(path: &PathBuf) -> Result<Self> {
        let file = File::open(path.clone())?;
        let bf = Self {
            br: BufReader::new(file),
        };

        Ok(bf)
    }

    pub fn read_line(&mut self, offset: Option<u64>) -> Result<String> {
        if let Some(offset) = offset {
            self.seek(offset)?;
        }

        let mut line = String::new();
        self.br.read_line(&mut line)?;

        // Remove trailing newline if present
        BackingFile::trim_line_end(&mut line);

        Ok(line)
    }

    pub fn seek(&mut self, offset: u64) -> Result<()> {
        self.br.seek(io::SeekFrom::Start(offset))?;

        Ok(())
    }

    pub fn incremental_read(&mut self, line: &mut String) -> Result<(usize, bool)> {
        let bytes = self.br.read_line(line)?;

        let partial = BackingFile::trim_line_end(line);

        Ok((bytes, partial))
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
