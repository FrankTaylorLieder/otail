use anyhow::Result;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek};
use std::path::PathBuf;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait BackingFile {
    fn read_line(&mut self, offset: Option<u64>) -> Result<String>;
    fn seek(&mut self, offset: u64) -> Result<()>;
    fn incremental_read(&mut self, line: &mut String) -> Result<(usize, bool)>;
}

/// Provide random access to a file on disk.
#[derive(Debug)]
pub struct FileBackingFile {
    br: BufReader<File>,
}

impl FileBackingFile {
    pub fn new_from_path(path: &str) -> Result<Self> {
        let mut pb = PathBuf::new();
        pb.push(path);

        Self::new(&pb)
    }

    pub fn new(path: &PathBuf) -> Result<Self> {
        let file = File::open(path.clone())?;
        let bf = Self {
            br: BufReader::new(file),
        };

        Ok(bf)
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

impl BackingFile for FileBackingFile {
    fn read_line(&mut self, offset: Option<u64>) -> Result<String> {
        if let Some(offset) = offset {
            self.seek(offset)?;
        }

        let mut line = String::new();
        self.br.read_line(&mut line)?;

        // Remove trailing newline if present
        FileBackingFile::trim_line_end(&mut line);

        Ok(line)
    }

    fn seek(&mut self, offset: u64) -> Result<()> {
        self.br.seek(io::SeekFrom::Start(offset))?;

        Ok(())
    }

    fn incremental_read(&mut self, line: &mut String) -> Result<(usize, bool)> {
        let bytes = self.br.read_line(line)?;

        let partial = FileBackingFile::trim_line_end(line);

        Ok((bytes, partial))
    }
}
