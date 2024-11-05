use std::collections::VecDeque;

use anyhow::Result;
use log::{debug, error, trace};
use tokio::select;
use tokio::sync::{mpsc, oneshot};

use crate::ifile::{
    ViewCommand, ViewCommandsSender, ViewUpdate, ViewUpdateReceiver, ViewUpdateSender,
};

pub struct ConsoleView {
    id: String,
    path: String,
    commands_sender: ViewCommandsSender,
    update_sender: ViewUpdateSender,
    update_receiver: ViewUpdateReceiver,

    file_lines: u32,
    file_bytes: u32,

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

impl ConsoleView {
    pub fn new(id: String, path: String, commands_sender: ViewCommandsSender) -> Self {
        let (update_sender, update_receiver) = mpsc::channel(10);
        ConsoleView {
            id,
            path,
            commands_sender,
            update_sender,
            update_receiver,

            file_lines: 0,
            file_bytes: 0,

            requested_first_line: 1,
            requested_lines: 4,

            current_first_line: 0,
            lines: VecDeque::new(),
            tailing: false,

            pending_line: None,
        }
    }

    pub fn set_tail(&mut self, tail: bool) {
        self.tailing = tail;

        self.current_first_line = 0;
        self.lines = VecDeque::new();
        self.requested_first_line = clamped_sub(self.file_lines, self.requested_lines) + 1;
    }

    pub async fn run(&mut self) -> Result<()> {
        debug!("{}: Console View starting: {:?}", self.id, self.path);

        self.commands_sender
            .send(ViewCommand::RegisterUpdater {
                id: self.id.clone(),
                updater: self.update_sender.clone(),
            })
            .await?;

        loop {
            let nl = self.determine_next_line();
            trace!("{}: determine_next_line: {:?}", self.id, nl);

            if nl != self.pending_line {
                trace!("{}: Requesting next line: {:?}", self.id, nl);

                self.commands_sender
                    .send(ViewCommand::GetLine {
                        id: self.id.clone(),
                        line_no: nl,
                    })
                    .await?;

                self.pending_line = nl;
            }

            select! {
                update = self.update_receiver.recv() => {
                    match update {
                        Some(update) => {
                            self.handle_update(update);
                        },
                        None => {
                            todo!()
                        }
                    }
                }
            }
        }

        Ok(())
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

    fn handle_update(&mut self, update: ViewUpdate) {
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
                }
            }
            ViewUpdate::Truncated => {
                debug!("{}: File truncated", self.id);
            }
            ViewUpdate::FileError { reason } => {
                error!("{}: File error: {reason}", self.id);
            }
        }
    }
}
