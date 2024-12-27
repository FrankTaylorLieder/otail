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

#[derive(Debug, Default)]
pub struct ViewPort {
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
    viewport: ViewPort,
    lines: Vec<Option<String>>,
}

#[derive(Debug)]
pub struct View {
    id: String,
    path: String,

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

impl ViewPort {
    pub fn range(&self) -> Range<usize> {
        (self.first_line..(self.first_line + self.num_lines))
    }
}

impl LineCache {
    // Set the viewport and report on this lines need to be fetched.
    pub fn set_viewport(&mut self, viewport: ViewPort) -> Vec<usize> {
        trace!("New viewport: {:?}", viewport);
        let mut new_lines = vec![None; viewport.num_lines];

        let or = self.viewport.range();
        let nr = self.viewport.range();

        // TODO: Reinstate code to capture lines.
        // if or.start <= nr.end && nr.start <= or.start {
        //     let ofl = self.viewport.first_line;
        //     let nfl = viewport.first_line;
        //     for i in (max(or.start, nr.start)..min(or.end, nr.end)) {
        //         // TODO: Can we avoid the clone here?
        //         new_lines.insert(i - nfl, self.lines[i - ofl].clone());
        //     }
        // }

        self.lines = new_lines;
        self.viewport = viewport;

        let missing_lines = self
            .lines
            .iter()
            .enumerate()
            .filter_map(|(i, v)| if v.is_none() { Some(i) } else { None })
            .collect::<Vec<_>>();

        trace!("Missing lines: {:?}", missing_lines);
        missing_lines
    }

    pub fn get_viewport(&self) -> &ViewPort {
        &self.viewport
    }

    pub fn set_line(&mut self, line_no: usize, line: String) -> bool {
        if !self.viewport.range().contains(&line_no) {
            trace!(
                "set_line() outside viewport: {} not in {:?}",
                line_no,
                self.viewport
            );
            return false;
        }

        trace!("Setting line: {}", line_no);
        self.lines
            .insert(line_no - self.viewport.first_line, Some(line));
        true
    }

    pub fn get_line(&self, line_no: usize) -> Option<String> {
        if !self.viewport.range().contains(&line_no) {
            warn!(
                "Requested line outside the current ViewPort: line: {}, viewport: {:?}",
                line_no, self.viewport
            );
            return None;
        }

        self.lines[line_no - self.viewport.first_line].clone()
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

    // Sync menthods... callable from the TUI render function.
    //
    pub fn get_line(&mut self, line_no: usize) -> Option<String> {
        self.line_cache.get_line(line_no)
    }

    pub fn get_stats(&self) -> Stats {
        self.stats.clone()
    }

    // Async methods... callable from the TUI event loop.
    //
    pub async fn set_tail(&mut self, tail: bool) {
        self.tailing = tail;

        todo!()
    }

    pub async fn set_viewport(&mut self, viewport: ViewPort) -> Result<()> {
        let missing = self.line_cache.set_viewport(viewport);

        // TODO: Cancel missing lines no longer needed.

        // Request the lines we don't have.
        for line_no in missing {
            trace!("Client {} sending line request {}", self.id, line_no);
            // TODO: Fix unwrap
            self.ifile_req_sender.try_send(IFReq::GetLine {
                id: self.id.clone(),
                line_no,
            })?
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

                self.line_cache.set_line(line_no, line_content);
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
