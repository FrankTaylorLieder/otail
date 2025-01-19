use std::cmp::{max, min};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek};
use std::ops::{Range, RangeBounds};

use anyhow::{anyhow, Result};
use log::{debug, error, trace, warn};
use ratatui::symbols::line;
use tokio::select;
use tokio::sync::{mpsc, oneshot};

use crate::common::{clamped_add, LineContent};
use crate::ifile::{FileReq, FileReqSender, FileResp, FileRespSender, IFResp};

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
    path: String,

    viewport: LinesSlice,
    current: usize,
    start_point: usize,
    longest_line_length: usize,

    ifile_req_sender: FileReqSender<T>,
    ifile_resp_sender: FileRespSender<T>,

    stats: Stats,

    line_cache: LineCache<L>,

    tailing: bool,
}

#[derive(Debug)]
pub enum UpdateAction {
    Truncated,
    Error { msg: String },
}

fn clamped_sub(a: usize, b: usize) -> usize {
    if b > a {
        0
    } else {
        a - b
    }
}

impl LinesSlice {
    pub fn range(&self) -> Range<usize> {
        self.first_line..(self.first_line + self.num_lines)
    }
}

impl<L: Clone + LineContent> LineCache<L> {
    pub fn set_range(&mut self, range: LinesSlice) {
        self.range = range;
    }

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

    pub fn get_viewport(&self) -> &LinesSlice {
        &self.range
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
        path: String,
        ifile_req_sender: FileReqSender<T>,
        ifile_resp_sender: FileRespSender<T>,
    ) -> Self {
        View {
            id,
            path,

            viewport: LinesSlice::default(),
            current: 0,
            start_point: 0,
            longest_line_length: 0,

            ifile_req_sender,
            ifile_resp_sender,

            stats: Stats::default(),

            line_cache: LineCache::default(),

            tailing: false,
        }
    }

    pub async fn init(&self) -> Result<()> {
        let r = self
            .ifile_req_sender
            .send(FileReq::RegisterClient {
                id: self.id.clone(),
                client_sender: self.ifile_resp_sender.clone(),
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
    pub fn get_line(&mut self, line_no: usize) -> Option<L> {
        self.line_cache.get_line(line_no)
    }

    pub fn get_stats(&self) -> Stats {
        self.stats.clone()
    }

    pub fn current(&self) -> usize {
        self.current
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
        self.start_point = clamped_add(
            self.longest_line_length,
            (width as isize) * -1,
            0,
            self.longest_line_length,
        );
    }

    // Async methods... callable from the TUI event loop.
    //
    pub async fn set_tail(&mut self, tail: bool) -> Result<()> {
        self.tailing = tail;

        if !tail {
            self.ifile_req_sender
                .send(FileReq::DisableTailing {
                    id: self.id.clone(),
                })
                .await?;

            return Ok(());
        }

        let last_line = clamped_sub(self.get_stats().file_lines, 1);
        self.set_current(last_line).await?;

        self.ifile_req_sender
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
            self.ifile_req_sender
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
                        .set_current(clamped_sub(self.stats.file_lines, 1))
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
