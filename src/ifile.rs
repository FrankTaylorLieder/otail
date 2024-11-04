use anyhow::Result;
use log::{debug, error, info, trace};
use std::path::PathBuf;
use std::time::Duration;
use std::{thread, usize};
use tokio::fs::File;
use tokio::select;
use tokio::sync::{mpsc, oneshot};

use crate::reader::Reader;

pub type ViewCommandsSender = mpsc::Sender<ViewCommand>;
pub type ViewCommandsReceiver = mpsc::Receiver<ViewCommand>;

pub type ViewUpdateSender = mpsc::Sender<ViewUpdate>;
pub type ViewUpdateReceiver = mpsc::Receiver<ViewUpdate>;

pub type ResultResponder<T> = oneshot::Sender<T>;

pub type ReaderUpdateSender = mpsc::Sender<ReaderUpdate>;
pub type ReaderUpdateReceiver = mpsc::Receiver<ReaderUpdate>;

#[derive(Debug)]
pub enum ViewCommand {
    GetLine {
        index: u32,
        resp: ResultResponder<Option<String>>,
    },
    RegisterUpdates {
        updater: ViewUpdateSender,
    },
    RegisterTail {
        index: u32,
        resp: ResultResponder<String>,
    },
}

#[derive(Debug)]
pub enum ViewUpdate {
    Line {
        line_no: u32,
        line: String,
        line_bytes: u32,
        partial: bool,
        file_bytes: u32,
    },
    Truncated,
    FileError {
        reason: String,
    },
}

#[derive(Debug)]
pub enum ReaderUpdate {
    Line {
        line: String,
        line_bytes: u32,
        partial: bool,
        file_bytes: u32,
    },
    Truncated,
    FileError {
        reason: String,
    },
}

#[derive(Debug)]
struct SLine {
    content: String,
    index: u32,
    length: u32,
}

#[derive(Debug)]
pub struct IFile {
    view_receiver: ViewCommandsReceiver,
    view_sender: ViewCommandsSender,
    reader_receiver: ReaderUpdateReceiver,
    reader_sender: ReaderUpdateSender,
    path: PathBuf,
    lines: Vec<SLine>,
    line_count: u32,
    byte_count: u32,
    view_update_senders: Vec<ViewUpdateSender>,
    tailers: Vec<ResultResponder<String>>,
}

impl IFile {
    pub fn new(path: &str) -> IFile {
        let mut pb = PathBuf::new();
        pb.push(path);

        let (view_sender, view_receiver) = mpsc::channel(10);
        let (reader_sender, reader_receiver) = mpsc::channel(10);

        IFile {
            path: pb,
            view_receiver,
            view_sender,
            reader_receiver,
            reader_sender,
            lines: vec![],
            line_count: 0,
            byte_count: 0,
            view_update_senders: vec![],
            tailers: vec![],
        }
    }

    fn run_reader(&mut self, cs: ReaderUpdateSender) {
        let cs = cs.clone();
        let path = self.path.clone();
        tokio::spawn(async move {
            Reader::run(path, cs).await;
        });
    }

    pub fn get_view_sender(&self) -> ViewCommandsSender {
        self.view_sender.clone()
    }

    pub async fn run(&mut self) -> Result<()> {
        debug!("Ifile starting: {:?}", self.path);

        self.run_reader(self.reader_sender.clone());

        trace!("Waiting on commands/updates...");

        // TODO: Deal with closing command channels to signal shutting down
        loop {
            select! {
                cmd = self.view_receiver.recv() => {
                    match cmd {
                        Some(cmd) => {
                            self.handle_view_command(cmd).await;
                        },
                        None => {
                            todo!()
                        }
                    }
                }
                update = self.reader_receiver.recv() => {
                    match update {
                        Some(update) => {
                            self.handle_reader_update(update).await;
                        },
                        None => {
                            todo!()
                        }
                    }
                }
            }
        }

        trace!("IFile finished");

        Ok(())
    }

    async fn handle_reader_update(&mut self, update: ReaderUpdate) {
        trace!("Handle reader update: {:?}", update);
        match update {
            ReaderUpdate::Line {
                line,
                line_bytes,
                partial,
                file_bytes,
            } => {
                trace!("Adding line: {} / {:?}", partial, self.line_count);

                if partial {
                    self.lines[self.line_count as usize - 1] = SLine {
                        content: line.clone(),
                        index: self.line_count,
                        length: line_bytes,
                    }
                } else {
                    self.lines.push(SLine {
                        content: line.clone(),
                        index: self.line_count,
                        length: line_bytes,
                    });

                    self.line_count += 1;
                }
                self.byte_count = file_bytes;

                for updater in self.view_update_senders.iter() {
                    trace!("Sending update");
                    // TODO: Deal with unwrap
                    updater
                        .send(ViewUpdate::Line {
                            line_no: self.line_count - 1,
                            line: line.clone(),
                            line_bytes,
                            partial,
                            file_bytes,
                        })
                        .await
                        .unwrap();
                }

                while let Some(t) = self.tailers.pop() {
                    trace!("Sending line to tailer");
                    // TODO: Remove unwrap
                    t.send(line.clone()).unwrap();
                }
            }
            ReaderUpdate::Truncated => {
                trace!("File truncated... resetting ifile");
                self.line_count = 0;
                self.lines = vec![];
                self.byte_count = 0;

                for updater in self.view_update_senders.iter() {
                    trace!("Sending truncate");
                    // TODO: Deal with unwrap
                    updater.send(ViewUpdate::Truncated).await.unwrap();
                }
            }
            ReaderUpdate::FileError { reason } => {
                error!("File error: {:?}", reason);

                for updater in self.view_update_senders.iter() {
                    trace!("Forwarding error");
                    // TODO: Deal with unwrap
                    updater
                        .send(ViewUpdate::FileError {
                            reason: reason.clone(),
                        })
                        .await
                        .unwrap();
                }
            }
        }
    }

    async fn handle_view_command(&mut self, cmd: ViewCommand) {
        trace!("Handle view command: {:?}", cmd);
        match cmd {
            ViewCommand::GetLine { index, resp } => {
                trace!("Getting line: {}", index);
                let sl = self.lines.get(index as usize);
                resp.send(sl.map(|sl| sl.content.clone()));
            }
            ViewCommand::RegisterUpdates { updater } => {
                trace!("Registering an updater");
                self.view_update_senders.push(updater);
            }
            ViewCommand::RegisterTail { index, resp } => {
                trace!("Tail: {}", index);
                let sl = self.lines.get(index as usize);
                let Some(sl) = sl else {
                    trace!("Waiting for next line...");
                    self.tailers.push(resp);
                    return;
                };
                resp.send(sl.content.clone());
            }
        }
    }
}
