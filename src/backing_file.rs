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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_new_creates_backing_file() {
        let temp_file = create_test_file("test content\n");
        let path = temp_file.path().to_path_buf();
        
        let backing_file = BackingFile::new(&path);
        assert!(backing_file.is_ok());
    }

    #[test]
    fn test_new_fails_for_nonexistent_file() {
        let path = PathBuf::from("/nonexistent/file.txt");
        let backing_file = BackingFile::new(&path);
        assert!(backing_file.is_err());
    }

    #[test]
    fn test_read_line_without_offset() {
        let temp_file = create_test_file("first line\nsecond line\nthird line\n");
        let path = temp_file.path().to_path_buf();
        let mut backing_file = BackingFile::new(&path).unwrap();

        let line = backing_file.read_line(None).unwrap();
        assert_eq!(line, "first line");

        let line = backing_file.read_line(None).unwrap();
        assert_eq!(line, "second line");
    }

    #[test]
    fn test_read_line_with_offset() {
        let temp_file = create_test_file("first line\nsecond line\nthird line\n");
        let path = temp_file.path().to_path_buf();
        let mut backing_file = BackingFile::new(&path).unwrap();

        // Read from offset 11 (start of "second line")
        let line = backing_file.read_line(Some(11)).unwrap();
        assert_eq!(line, "second line");
    }

    #[test]
    fn test_incremental_read_complete_line() {
        let temp_file = create_test_file("complete line\npartial");
        let path = temp_file.path().to_path_buf();
        let mut backing_file = BackingFile::new(&path).unwrap();

        let mut line = String::new();
        let (bytes, partial) = backing_file.incremental_read(&mut line).unwrap();
        
        assert_eq!(line, "complete line");
        assert_eq!(bytes, 14); // "complete line\n"
        assert!(!partial);
    }

    #[test]
    fn test_incremental_read_partial_line() {
        let temp_file = create_test_file("partial line without newline");
        let path = temp_file.path().to_path_buf();
        let mut backing_file = BackingFile::new(&path).unwrap();

        let mut line = String::new();
        let (bytes, partial) = backing_file.incremental_read(&mut line).unwrap();
        
        assert_eq!(line, "partial line without newline");
        assert_eq!(bytes, 28);
        assert!(partial);
    }

    #[test]
    fn test_trim_line_end_with_unix_newline() {
        let mut line = String::from("test line\n");
        let partial = BackingFile::trim_line_end(&mut line);
        
        assert_eq!(line, "test line");
        assert!(!partial);
    }

    #[test]
    fn test_trim_line_end_with_windows_newline() {
        let mut line = String::from("test line\r\n");
        let partial = BackingFile::trim_line_end(&mut line);
        
        assert_eq!(line, "test line");
        assert!(!partial);
    }

    #[test]
    fn test_trim_line_end_no_newline() {
        let mut line = String::from("test line");
        let partial = BackingFile::trim_line_end(&mut line);
        
        assert_eq!(line, "test line");
        assert!(partial);
    }

    #[test]
    fn test_seek_changes_position() {
        let temp_file = create_test_file("first line\nsecond line\nthird line\n");
        let path = temp_file.path().to_path_buf();
        let mut backing_file = BackingFile::new(&path).unwrap();

        backing_file.seek(11).unwrap(); // Position at "second line"
        let line = backing_file.read_line(None).unwrap();
        assert_eq!(line, "second line");
    }

    #[test]
    fn test_empty_file() {
        let temp_file = create_test_file("");
        let path = temp_file.path().to_path_buf();
        let mut backing_file = BackingFile::new(&path).unwrap();

        let line = backing_file.read_line(None).unwrap();
        assert_eq!(line, "");
    }
}
