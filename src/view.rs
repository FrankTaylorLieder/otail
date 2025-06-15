use std::cmp::{max, min};
use std::ops::Range;

use anyhow::Result;
use log::{debug, trace, warn};

use crate::common::{self, clamped_add, LineContent};
use crate::ifile::{FileReq, FileReqSender, FileResp, FileRespSender};

#[derive(Debug, Default, Eq, PartialEq, Clone)]
pub struct LinesSlice {
    pub first_line: usize,
    pub num_lines: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub file_lines: usize,
    pub file_bytes: u64,
}

#[derive(Debug, Default)]
struct LineCache<L> {
    range: LinesSlice,
    lines: Vec<Option<L>>,
}

#[derive(Debug)]
pub struct View<T, L> {
    id: String,

    viewport: LinesSlice,
    current: usize,
    start_point: usize,
    longest_line_length: usize,

    file_req_sender: FileReqSender<T>,
    file_resp_sender: FileRespSender<T>,

    stats: Stats,

    line_cache: LineCache<L>,

    tailing: bool,
}

impl LinesSlice {
    pub fn range(&self) -> Range<usize> {
        self.first_line..(self.first_line + self.num_lines)
    }
}

impl<L: Clone + LineContent> LineCache<L> {
    pub fn reset(&mut self) -> Vec<usize> {
        self.lines = vec![None; self.range.num_lines];

        self.missing_lines()
    }

    // Set the viewport and report on this lines need to be fetched.
    pub fn set_viewport(&mut self, viewport: LinesSlice) -> Vec<usize> {
        trace!("New viewport: {:?}", viewport);
        let mut new_lines = vec![None; viewport.num_lines];

        let or = self.range.range();
        let nr = viewport.range();

        if or.start <= nr.end && nr.start <= or.end {
            let ofl = self.range.first_line;
            let nfl = viewport.first_line;
            for i in max(or.start, nr.start)..min(or.end, nr.end) {
                // TODO: Can we avoid the clone here? swap?
                new_lines[i - nfl] = self.lines[i - ofl].clone();
            }
        }

        self.lines = new_lines;
        self.range = viewport;

        self.missing_lines()
    }

    fn missing_lines(&self) -> Vec<usize> {
        let first_line = self.range.first_line;

        let missing_lines = self
            .lines
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                if v.is_none() {
                    Some(i + first_line)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        trace!("Missing lines: {:?}", missing_lines);
        missing_lines
    }

    pub fn set_line(&mut self, line_no: usize, line: L, tailing: bool) -> bool {
        if !self.range.range().contains(&line_no) {
            // Determine the next line after the current buffer if we were tailing.
            let tail_line = self.range.first_line + self.range.num_lines;
            if tailing && line_no == tail_line {
                self.add_tail(line_no, line);
                return true;
            }

            trace!(
                "set_line() outside viewport: {} not in {:?}",
                line_no,
                self.range
            );
            return false;
        }

        self.lines[line_no - self.range.first_line] = Some(line);
        true
    }

    fn add_tail(&mut self, line_no: usize, line: L) {
        trace!("Adding line whilst tailing: {}", line_no);
        self.lines.remove(0);
        self.range.first_line += 1;
        self.lines.push(Some(line));
    }

    pub fn get_line(&self, line_no: usize) -> Option<L> {
        if !self.range.range().contains(&line_no) {
            warn!(
                "Requested line outside the current ViewPort: line: {}, viewport: {:?}",
                line_no, self.range
            );
            return None;
        }

        let s = self.lines[line_no - self.range.first_line].clone();

        s
    }
}

impl<T: std::marker::Send + 'static, L: Clone + Default + LineContent> View<T, L> {
    pub fn new(
        id: String,
        ifile_req_sender: FileReqSender<T>,
        ifile_resp_sender: FileRespSender<T>,
    ) -> Self {
        View {
            id,

            viewport: LinesSlice::default(),
            current: 0,
            start_point: 0,
            longest_line_length: 0,

            file_req_sender: ifile_req_sender,
            file_resp_sender: ifile_resp_sender,

            stats: Stats::default(),

            line_cache: LineCache::default(),

            tailing: false,
        }
    }

    pub async fn init(&self) -> Result<()> {
        self.file_req_sender
            .send(FileReq::RegisterClient {
                id: self.id.clone(),
                client_sender: self.file_resp_sender.clone(),
            })
            .await?;

        Ok(())
    }

    pub async fn reset(&mut self) -> Result<()> {
        trace!("Reset view");

        self.current = 0;
        self.start_point = 0;
        self.set_viewport(LinesSlice {
            first_line: 0,
            num_lines: self.get_viewport_height(),
        })
        .await?;

        self.stats.file_lines = 0;
        self.stats.file_bytes = 0;
        let missing = self.line_cache.reset();

        self.request_missing(missing).await?;

        Ok(())
    }

    // Sync methods... callable from the TUI render function.
    //
    pub fn get_line(&self, line_no: usize) -> Option<L> {
        self.line_cache.get_line(line_no)
    }

    pub fn get_stats(&self) -> Stats {
        self.stats.clone()
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn current_line_length(&self) -> usize {
        if let Some(line) = self.get_line(self.current) {
            return line.len();
        }

        0
    }

    pub fn range(&self) -> Range<usize> {
        self.viewport.range()
    }

    pub fn get_start_point(&self) -> usize {
        self.start_point
    }

    pub fn pan(&mut self, delta: isize, width: usize) {
        let max = clamped_add(
            self.longest_line_length,
            (width as isize) * -1,
            0,
            self.longest_line_length,
        );

        self.start_point = clamped_add(self.start_point, delta, 0, max);
    }

    pub fn pan_start(&mut self) {
        self.start_point = 0;
    }

    pub fn pan_end(&mut self, width: usize) {
        let current_line_len = self.current_line_length();
        self.start_point =
            clamped_add(current_line_len, (width as isize) * -1, 0, current_line_len);
    }

    // Async methods... callable from the TUI event loop.
    //
    pub async fn set_tail(&mut self, tail: bool) -> Result<()> {
        self.tailing = tail;

        if !tail {
            self.file_req_sender
                .send(FileReq::DisableTailing {
                    id: self.id.clone(),
                })
                .await?;

            return Ok(());
        }

        let last_line = common::clamped_sub(self.get_stats().file_lines, 1);
        self.set_current(last_line).await?;

        self.file_req_sender
            .send(FileReq::EnableTailing {
                id: self.id.clone(),
                last_seen_line: last_line,
            })
            .await?;

        Ok(())
    }

    pub async fn set_current(&mut self, line_no: usize) -> Result<()> {
        self.current = line_no;

        // Whilst the current line is in the viewport, do not scroll.
        // Only scroll to keep the current in the viewport.

        if self.viewport.range().contains(&line_no) {
            return Ok(());
        }

        let num_lines = self.viewport.num_lines;
        if line_no < self.viewport.first_line {
            trace!("Moving viewport up to keep the current line on screen");
            return self
                .set_viewport(LinesSlice {
                    first_line: line_no,
                    num_lines,
                })
                .await;
        }

        // Move the viewport so the current line is at the end. Be careful to avoid a negative
        // first line.
        if line_no < num_lines {
            trace!("Moving to start to keep screen full");
            return self
                .set_viewport(LinesSlice {
                    first_line: 0,
                    num_lines,
                })
                .await;
        }

        trace!("Move viewport down to keep current line on screen");
        self.set_viewport(LinesSlice {
            first_line: line_no - num_lines + 1,
            num_lines,
        })
        .await
    }

    pub async fn center_current_line(&mut self) -> Result<()> {
        let height = self.get_viewport_height();
        let bottom_half = height / 2;

        let first_line = common::clamped_sub(self.current, bottom_half);

        // TODO If there are too few lines below, move current down the screen.

        self.set_viewport(LinesSlice {
            first_line,
            num_lines: height,
        })
        .await
    }

    pub async fn set_height(&mut self, height: usize) -> Result<()> {
        // Change the height of the viewport, ensuring the current line is still on screen.
        // TODO: For the filter pane we want to expand the top of the window, not the bottom

        let old_height = self.viewport.num_lines;
        let first_line = self.viewport.first_line;
        let current = self.current;

        if height >= old_height || current < first_line + height {
            return self
                .set_viewport(LinesSlice {
                    first_line,
                    num_lines: height,
                })
                .await;
        }

        self.set_viewport(LinesSlice {
            first_line: current - height + 1,
            num_lines: height,
        })
        .await
    }

    async fn set_viewport(&mut self, viewport: LinesSlice) -> Result<()> {
        if self.viewport == viewport {
            return Ok(());
        }

        let missing = self.line_cache.set_viewport(viewport.clone());
        self.viewport = viewport;

        // Recalculate the longest line
        self.longest_line_length = 0;
        for l in &self.line_cache.lines {
            if let Some(l) = l {
                let len = l.len();
                if len > self.longest_line_length {
                    self.longest_line_length = len;
                }
            }
        }
        trace!("New longest known line: {}", self.longest_line_length);

        // TODO: Cancel missing lines no longer needed.

        self.request_missing(missing).await?;

        Ok(())
    }

    pub fn get_viewport_height(&self) -> usize {
        self.viewport.num_lines
    }

    async fn request_missing(&self, missing: Vec<usize>) -> Result<()> {
        // Request the lines we don't have.
        for line_no in missing {
            trace!(
                "Client {} sending missing line request {}",
                self.id,
                line_no
            );
            self.file_req_sender
                .send(FileReq::GetLine {
                    id: self.id.clone(),
                    line_no,
                })
                .await?
        }
        Ok(())
    }

    pub async fn handle_update(&mut self, update: FileResp<L>) {
        match update {
            FileResp::Line {
                line_no,
                line_content,
                partial,
            } => {
                debug!(
                    "{}: View line: {line_no} {} => {}",
                    self.id,
                    if partial { "PARTIAL" } else { "COMPLETE" },
                    line_content.render(),
                );

                let len = line_content.len();
                if self
                    .line_cache
                    .set_line(line_no, line_content, self.tailing)
                {
                    trace!("Set line {} for {}", line_no, self.id);
                    if len > self.longest_line_length {
                        trace!("New longest line: {}", len);
                        self.longest_line_length = len;
                    }
                }

                if self.tailing {
                    if let Err(err) = self
                        .set_current(common::clamped_sub(self.stats.file_lines, 1))
                        .await
                    {
                        warn!("Failed to set current to last line during tail: {:?}", err);
                    }
                }
            }
            FileResp::Stats {
                file_lines,
                file_bytes,
            } => {
                self.stats.file_lines = file_lines;
                self.stats.file_bytes = file_bytes;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[derive(Debug, Clone, Default)]
    struct TestLineContent(String);

    impl LineContent for TestLineContent {
        fn len(&self) -> usize {
            self.0.len()
        }

        fn render(&self) -> String {
            self.0.clone()
        }
    }

    fn create_test_channels() -> (FileReqSender<String>, FileRespSender<String>) {
        let (req_sender, _req_receiver) = mpsc::channel(10);
        let (resp_sender, _resp_receiver) = mpsc::channel(10);
        (req_sender, resp_sender)
    }

    #[test]
    fn test_lines_slice_range() {
        let slice = LinesSlice {
            first_line: 5,
            num_lines: 10,
        };
        assert_eq!(slice.range(), 5..15);
    }

    #[test]
    fn test_lines_slice_default() {
        let slice = LinesSlice::default();
        assert_eq!(slice.first_line, 0);
        assert_eq!(slice.num_lines, 0);
        assert_eq!(slice.range(), 0..0);
    }

    #[test]
    fn test_line_cache_reset() {
        let mut cache: LineCache<TestLineContent> = LineCache {
            range: LinesSlice { first_line: 0, num_lines: 3 },
            lines: vec![
                Some(TestLineContent("line1".to_string())),
                Some(TestLineContent("line2".to_string())),
                Some(TestLineContent("line3".to_string())),
            ],
        };

        let missing = cache.reset();
        assert_eq!(missing, vec![0, 1, 2]);
        assert_eq!(cache.lines.len(), 3);
        assert!(cache.lines.iter().all(|l| l.is_none()));
    }

    #[test]
    fn test_line_cache_set_viewport_no_overlap() {
        let mut cache: LineCache<TestLineContent> = LineCache {
            range: LinesSlice { first_line: 0, num_lines: 3 },
            lines: vec![
                Some(TestLineContent("line0".to_string())),
                Some(TestLineContent("line1".to_string())),
                Some(TestLineContent("line2".to_string())),
            ],
        };

        let new_viewport = LinesSlice { first_line: 10, num_lines: 3 };
        let missing = cache.set_viewport(new_viewport);
        
        assert_eq!(missing, vec![10, 11, 12]);
        assert_eq!(cache.range.first_line, 10);
        assert_eq!(cache.range.num_lines, 3);
        assert!(cache.lines.iter().all(|l| l.is_none()));
    }

    #[test]
    fn test_line_cache_set_viewport_with_overlap() {
        let mut cache: LineCache<TestLineContent> = LineCache {
            range: LinesSlice { first_line: 0, num_lines: 3 },
            lines: vec![
                Some(TestLineContent("line0".to_string())),
                Some(TestLineContent("line1".to_string())),
                Some(TestLineContent("line2".to_string())),
            ],
        };

        let new_viewport = LinesSlice { first_line: 1, num_lines: 3 };
        let missing = cache.set_viewport(new_viewport);
        
        assert_eq!(missing, vec![3]);
        assert_eq!(cache.range.first_line, 1);
        assert_eq!(cache.range.num_lines, 3);
        
        // line1 and line2 should be preserved in new positions
        assert_eq!(cache.lines[0].as_ref().unwrap().0, "line1");
        assert_eq!(cache.lines[1].as_ref().unwrap().0, "line2");
        assert!(cache.lines[2].is_none()); // line 3 is missing
    }

    #[test]
    fn test_line_cache_set_line_in_range() {
        let mut cache: LineCache<TestLineContent> = LineCache {
            range: LinesSlice { first_line: 5, num_lines: 3 },
            lines: vec![None, None, None],
        };

        let result = cache.set_line(6, TestLineContent("test line".to_string()), false);
        assert!(result);
        assert_eq!(cache.lines[1].as_ref().unwrap().0, "test line");
    }

    #[test]
    fn test_line_cache_set_line_out_of_range() {
        let mut cache: LineCache<TestLineContent> = LineCache {
            range: LinesSlice { first_line: 5, num_lines: 3 },
            lines: vec![None, None, None],
        };

        let result = cache.set_line(10, TestLineContent("test line".to_string()), false);
        assert!(!result);
        assert!(cache.lines.iter().all(|l| l.is_none()));
    }

    #[test]
    fn test_line_cache_set_line_tailing() {
        let mut cache: LineCache<TestLineContent> = LineCache {
            range: LinesSlice { first_line: 5, num_lines: 3 },
            lines: vec![
                Some(TestLineContent("line5".to_string())),
                Some(TestLineContent("line6".to_string())),
                Some(TestLineContent("line7".to_string())),
            ],
        };

        // Line 8 is the next line after current buffer (5+3=8)
        let result = cache.set_line(8, TestLineContent("line8".to_string()), true);
        assert!(result);
        
        // Should have shifted the buffer
        assert_eq!(cache.range.first_line, 6);
        assert_eq!(cache.lines[0].as_ref().unwrap().0, "line6");
        assert_eq!(cache.lines[1].as_ref().unwrap().0, "line7");
        assert_eq!(cache.lines[2].as_ref().unwrap().0, "line8");
    }

    #[test]
    fn test_line_cache_get_line() {
        let mut cache: LineCache<TestLineContent> = LineCache {
            range: LinesSlice { first_line: 5, num_lines: 3 },
            lines: vec![
                Some(TestLineContent("line5".to_string())),
                None,
                Some(TestLineContent("line7".to_string())),
            ],
        };

        assert_eq!(cache.get_line(5).unwrap().0, "line5");
        assert!(cache.get_line(6).is_none());
        assert_eq!(cache.get_line(7).unwrap().0, "line7");
        assert!(cache.get_line(10).is_none()); // Out of range
    }

    #[tokio::test]
    async fn test_view_new() {
        let (req_sender, resp_sender) = create_test_channels();
        let view: View<String, TestLineContent> = View::new(
            "test_view".to_string(),
            req_sender,
            resp_sender,
        );

        assert_eq!(view.id, "test_view");
        assert_eq!(view.current, 0);
        assert_eq!(view.start_point, 0);
        assert!(!view.tailing);
    }

    #[test]
    fn test_view_pan() {
        let (req_sender, resp_sender) = create_test_channels();
        let mut view: View<String, TestLineContent> = View::new(
            "test_view".to_string(),
            req_sender,
            resp_sender,
        );
        view.longest_line_length = 100;

        // Pan right by 10
        view.pan(10, 50);
        assert_eq!(view.start_point, 10);

        // Pan left by 5
        view.pan(-5, 50);
        assert_eq!(view.start_point, 5);

        // Try to pan beyond limits
        view.pan(1000, 50);
        assert_eq!(view.start_point, 50); // Should clamp to max (100 - 50)
    }

    #[test]
    fn test_view_pan_start_and_end() {
        let (req_sender, resp_sender) = create_test_channels();
        let mut view: View<String, TestLineContent> = View::new(
            "test_view".to_string(),
            req_sender,
            resp_sender,
        );
        view.start_point = 25;

        view.pan_start();
        assert_eq!(view.start_point, 0);

        // Set up a current line and test pan_end
        view.line_cache.range = LinesSlice { first_line: 0, num_lines: 1 };
        view.line_cache.lines = vec![Some(TestLineContent("a".repeat(100)))];
        view.current = 0;
        view.longest_line_length = 100;

        view.pan_end(20);
        assert_eq!(view.start_point, 80); // 100 - 20
    }

    #[test]
    fn test_stats_default() {
        let stats = Stats::default();
        assert_eq!(stats.file_lines, 0);
        assert_eq!(stats.file_bytes, 0);
    }
}
