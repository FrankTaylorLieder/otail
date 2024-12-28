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

use crate::ifile::{IFReq, IFReqSender, IFResp, IFRespReceiver, IFRespSender};

#[derive(Debug, Default, Eq, PartialEq, Clone)]
pub struct LinesSlice {
    pub first_line: usize,
    pub num_lines: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub file_lines: usize,
    pub file_bytes: usize,
}

#[derive(Debug, Default)]
struct LineCache {
    range: LinesSlice,
    lines: Vec<Option<String>>,
}

#[derive(Debug)]
pub struct View {
    id: String,
    path: String,

    viewport: LinesSlice,
    current: usize,

    ifile_req_sender: IFReqSender,
    ifile_resp_sender: IFRespSender,

    stats: Stats,

    line_cache: LineCache,

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

impl LineCache {
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

        let first_line = viewport.first_line;

        self.lines = new_lines;
        self.range = viewport;

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

    pub fn set_line(&mut self, line_no: usize, line: String) -> bool {
        if !self.range.range().contains(&line_no) {
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

    pub fn get_line(&self, line_no: usize) -> Option<String> {
        if !self.range.range().contains(&line_no) {
            warn!(
                "Requested line outside the current ViewPort: line: {}, viewport: {:?}",
                line_no, self.range
            );
            return None;
        }

        let s = self.lines[line_no - self.range.first_line].clone();
        // trace!(
        //     "XXX Getting {} {} = {:?}",
        //     line_no,
        //     line_no - self.range.first_line,
        //     s
        // );

        s
    }
}

impl View {
    pub fn new(
        id: String,
        path: String,
        ifile_req_sender: IFReqSender,
        ifile_resp_sender: IFRespSender,
    ) -> Self {
        View {
            id,
            path,

            viewport: LinesSlice::default(),
            current: 0,

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
            .send(IFReq::RegisterClient {
                id: self.id.clone(),
                client_sender: self.ifile_resp_sender.clone(),
            })
            .await?;

        Ok(())
    }

    pub fn reset(&mut self) {
        self.stats.file_lines = 0;
        self.stats.file_bytes = 0;
        self.line_cache = LineCache::default();
    }

    // Sync methods... callable from the TUI render function.
    //
    pub fn get_line(&mut self, line_no: usize) -> Option<String> {
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

    // Async methods... callable from the TUI event loop.
    //
    pub async fn set_tail(&mut self, tail: bool) {
        self.tailing = tail;

        todo!()
    }

    pub async fn set_current(&mut self, line_no: usize) -> Result<()> {
        // trace!(
        //     "XXX set current: {}, range: {:?}",
        //     line_no,
        //     self.viewport.range()
        // );
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
        // trace!(
        //     "XXX Set viewport old: {:?} new: {:?}",
        //     self.viewport,
        //     viewport
        // );
        if self.viewport == viewport {
            return Ok(());
        }

        let missing = self.line_cache.set_viewport(viewport.clone());
        self.viewport = viewport;

        // TODO: Cancel missing lines no longer needed.

        // Request the lines we don't have.
        for line_no in missing {
            trace!("Client {} sending line request {}", self.id, line_no);
            self.ifile_req_sender
                .send(IFReq::GetLine {
                    id: self.id.clone(),
                    line_no,
                })
                .await?
        }
        Ok(())
    }

    pub async fn handle_update(&mut self, update: IFResp) -> Option<UpdateAction> {
        match update {
            IFResp::Line {
                line_no,
                line_content,
                line_chars,
                line_bytes,
                partial,
            } => {
                debug!(
                    "{}: View line: {line_no} {} => {line_content}",
                    self.id,
                    if partial { "PARTIAL" } else { "COMPLETE" }
                );

                if self.line_cache.set_line(line_no, line_content) {
                    trace!("Set line {} for {}", line_no, self.id);
                }
                None
            }
            IFResp::Stats {
                file_lines,
                file_bytes,
            } => {
                self.stats.file_lines = file_lines;
                self.stats.file_bytes = file_bytes;

                None
            }
            IFResp::Truncated => {
                debug!("{}: File truncated", self.id);

                self.stats = Stats::default();
                self.line_cache = LineCache::default();

                Some(UpdateAction::Truncated)
            }
            IFResp::FileError { reason } => {
                error!("{}: File error: {reason}", self.id);

                Some(UpdateAction::Error { msg: reason })
            }
        }
    }
}
