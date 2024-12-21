use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek};

use anyhow::{anyhow, Result};
use log::{debug, error, trace, warn};
use tokio::select;
use tokio::sync::{mpsc, oneshot};

use crate::ifile::{IFReq, IFReqSender, IFResp, IFRespReceiver, IFRespSender};

#[derive(Debug, Default)]
pub struct ViewPort {
    pub first_line: u32,
    pub num_lines: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub file_lines: u32,
    pub file_bytes: u32,
}

#[derive(Debug)]
pub struct View {
    id: String,
    path: String,

    ifile_req_sender: IFReqSender,
    ifile_resp_sender: IFRespSender,

    stats: Stats,

    viewport: ViewPort,
    cached_lines: VecDeque<String>,

    tailing: bool,
}

#[derive(Debug)]
pub enum UpdateAction {
    Truncated,
    Error { msg: String },
}

fn clamped_sub(a: u32, b: u32) -> u32 {
    if b > a {
        0
    } else {
        a - b
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

            viewport: ViewPort::default(),
            cached_lines: VecDeque::new(),

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

    // Sync menthods... callable from the TUI render function.
    //
    pub fn get_line(&mut self, line_no: u32) -> Option<&String> {
        if line_no < self.viewport.first_line
            || line_no >= self.viewport.first_line + self.viewport.num_lines
        {
            warn!(
                "Requested line outside the current ViewPort: line: {}, viewport: {:?}",
                line_no, self.viewport
            );
            return None;
        }

        let cache_index = line_no - self.viewport.first_line;

        self.cached_lines.get(cache_index as usize)
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
        self.viewport = viewport;

        // TODO: Try to remember overlapping elements of the cache. Just clear for now to avoid
        // nasty cache inconsistencies.
        self.cached_lines.clear();

        // Request the lines we don't have.
        for i in 0..self.viewport.num_lines {
            let line_no = self.viewport.first_line + i;
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

                self.cached_lines
                    .insert((line_no - self.viewport.first_line) as usize, line_content);

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
                self.viewport = ViewPort::default();
                self.cached_lines.clear();

                Some(UpdateAction::Truncated)
            }
            IFResp::FileError { reason } => {
                error!("{}: File error: {reason}", self.id);

                Some(UpdateAction::Error { msg: reason })
            }
        }
    }
}
