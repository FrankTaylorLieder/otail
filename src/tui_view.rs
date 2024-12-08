use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek};

use anyhow::{anyhow, Result};
use log::{debug, error, trace};
use tokio::select;
use tokio::sync::{mpsc, oneshot};

use crate::ifile::{
    ViewCommand, ViewCommandsSender, ViewUpdate, ViewUpdateReceiver, ViewUpdateSender,
};
use crate::view::{LineRange, View};

#[derive(Debug)]
pub enum TUICallbackCommand {
    ChangeSelected { line: u32 },
    Truncated,
    Error { message: String },
}

#[derive(Debug)]
pub struct TuiView {
    id: String,
    path: String,
    br: BufReader<File>,

    commands_sender: ViewCommandsSender,
    update_sender: ViewUpdateSender,
    update_receiver: ViewUpdateReceiver,

    file_lines: u32,
    file_bytes: u32,

    offsets: Vec<u32>,

    requested_first_line: u32,
    requested_lines: u32,

    current_first_line: u32,
    lines: VecDeque<String>,
    tailing: bool,

    pending_line: Option<u32>,
}

fn clamped_sub(a: u32, b: u32) -> u32 {
    if b > a {
        0
    } else {
        a - b
    }
}

impl View for TuiView {
    fn set_tail(&mut self, tail: bool) {
        self.tailing = tail;

        self.current_first_line = 0;
        self.lines = VecDeque::new();
        self.requested_first_line = clamped_sub(self.file_lines, self.requested_lines) + 1;
    }

    fn get_line(&mut self, line: u32) -> Option<String> {
        if line >= self.file_lines {
            return None;
        }

        self.fetch_line(line)
    }

    fn set_line_range(&mut self, range: LineRange) {
        todo!()
    }

    fn num_lines(&self) -> u32 {
        self.file_lines
    }

    // async fn run(&mut self, commands_sender: ViewCommandsSender) -> Result<()> {
    //     commands_sender
    //         .send(ViewCommand::RegisterUpdater {
    //             id: self.id.clone(),
    //             updater: self.update_sender.clone(),
    //         })
    //         .await?;
    //
    //     tokio::spawn(async move {
    //         debug!("{}: Console View starting: {:?}", self.id, self.path);
    //
    //         loop {
    //             let nl = self.determine_next_line();
    //             trace!("{}: determine_next_line: {:?}", self.id, nl);
    //
    //             if nl != self.pending_line {
    //                 trace!("{}: Requesting next line: {:?}", self.id, nl);
    //
    //                 commands_sender
    //                     .send(ViewCommand::GetLine {
    //                         id: self.id.clone(),
    //                         line_no: nl,
    //                     })
    //                     .await
    //                     .unwrap();
    //
    //                 self.pending_line = nl;
    //             }
    //
    //             select! {
    //                 update = self.update_receiver.recv() => {
    //                     match update {
    //                         Some(update) => {
    //                             self.handle_update(update);
    //                         },
    //                         None => {
    //                             todo!()
    //                         }
    //                     }
    //                 }
    //             }
    //         }
    //     });
    //
    //     Ok(())
    // }
}

impl TuiView {
    pub fn new(id: String, path: String, commands_sender: ViewCommandsSender) -> Self {
        let mut f = std::fs::File::open(&path).unwrap();
        let mut br = BufReader::new(std::fs::File::open(&path).unwrap());

        let (update_sender, update_receiver) = mpsc::channel(10);
        TuiView {
            id,
            path,
            br,

            commands_sender,
            update_sender,
            update_receiver,

            file_lines: 0,
            file_bytes: 0,

            offsets: Vec::new(),

            requested_first_line: 1,
            requested_lines: 4,

            current_first_line: 0,
            lines: VecDeque::new(),
            tailing: false,

            pending_line: None,
        }
    }

    fn fetch_line(&mut self, line: u32) -> Option<String> {
        let Some(offset) = self.offsets.get(line as usize) else {
            return None;
        };

        self.br
            .seek(std::io::SeekFrom::Start(*offset as u64))
            .unwrap();

        let mut line = String::new();
        let len = self.br.read_line(&mut line).unwrap();

        Some(line)
    }

    fn determine_next_line(&self) -> Option<u32> {
        let nl = self.lines.len() as u32;
        trace!("{}: nl: {nl} / {}", self.id, self.current_first_line);

        if self.current_first_line == 0 {
            return Some(self.requested_first_line);
        }

        let last_line = self.requested_first_line + self.requested_lines;

        let next_line = self.current_first_line + nl;
        if next_line < last_line {
            return Some(next_line);
        }

        if self.tailing {
            return Some(last_line);
        }

        None
    }

    fn handle_update(&mut self, update: ViewUpdate) -> Option<TUICallbackCommand> {
        match update {
            ViewUpdate::Change {
                line_no,
                line_chars,
                line_bytes,
                file_bytes,
                partial,
            } => {
                debug!(
                    "{}: Change: {line_no} / len: {line_chars} / file bytes: {file_bytes}",
                    self.id
                );

                self.file_lines = line_no;
                self.file_bytes = file_bytes;

                None
            }
            ViewUpdate::Line {
                line_no,
                line,
                line_chars,
                line_bytes,
                partial,
            } => {
                debug!(
                    "{}: View line: {line_no} {} => {line}",
                    self.id,
                    if partial { "PARTIAL" } else { "COMPLETE" }
                );
                if self.current_first_line == 0 {
                    self.current_first_line = line_no;
                }
                self.lines.push_back(line);

                if self.lines.len() as u32 > self.requested_lines {
                    trace!(
                        "{}: Dropping first line ({})",
                        self.id,
                        self.current_first_line
                    );
                    self.lines.pop_front();
                    self.current_first_line += 1;
                    self.requested_first_line += 1;

                    // TODO: Handle new line when tailing
                    None
                } else {
                    None
                }
            }
            ViewUpdate::Truncated => {
                debug!("{}: File truncated", self.id);

                Some(TUICallbackCommand::Truncated)
            }
            ViewUpdate::FileError { reason } => {
                error!("{}: File error: {reason}", self.id);

                Some(TUICallbackCommand::Error { message: reason })
            }
        }
    }
}
