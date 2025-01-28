use anyhow::Result;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek};

#[derive(Debug)]
pub struct BackingFile {
    pub file: BufReader<File>,
}

impl BackingFile {
    pub fn read_line(&mut self, offset: u64) -> Result<String> {
        self.file.seek(io::SeekFrom::Start(offset))?;

        let mut line = String::new();
        self.file.read_line(&mut line)?;

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
