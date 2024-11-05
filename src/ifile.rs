use anyhow::Result;
use log::{debug, error, info, trace};
use std::collections::HashMap;
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
        id: String,
        line_no: Option<u32>,
    },
    RegisterUpdater {
        id: String,
        updater: ViewUpdateSender,
    },
}

#[derive(Debug)]
pub enum ViewUpdate {
    Change {
        line_no: u32, // 1-based line numbers.
        line_chars: u32,
        line_bytes: u32,
        file_bytes: u32,
        partial: bool,
    },
    Line {
        line_no: u32,
        line: String,
        line_chars: u32,
        line_bytes: u32,
        partial: bool,
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
    line_no: u32,
    line_chars: u32,
    line_bytes: u32,
    partial: bool,
}

#[derive(Debug)]
struct Updater {
    id: String,
    channel: ViewUpdateSender,
    line_no: Option<u32>,
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
    view_updaters: HashMap<String, Updater>,
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
            view_updaters: HashMap::new(),
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
        match update {
            ReaderUpdate::Line {
                line,
                line_bytes,
                partial,
                file_bytes,
            } => {
                let line_chars = line.len() as u32;

                if partial {
                    self.lines[self.line_count as usize - 1] = SLine {
                        content: line.clone(),
                        line_no: self.line_count,
                        line_chars: line.len() as u32,
                        line_bytes,
                        partial: true,
                    }
                } else {
                    self.line_count += 1;

                    self.lines.push(SLine {
                        content: line.clone(),
                        line_no: self.line_count,
                        line_chars: line.len() as u32,
                        line_bytes,
                        partial: false,
                    });
                }
                self.byte_count = file_bytes;

                trace!(
                    "Adding/updating line: {} / partial: {} / len: {}",
                    self.line_count,
                    partial,
                    line_chars
                );

                for (id, updater) in self.view_updaters.iter_mut() {
                    trace!("Sending update to view: {}", id);
                    // TODO: Deal with unwrap
                    updater
                        .channel
                        .send(ViewUpdate::Change {
                            line_no: self.line_count,
                            line_chars,
                            line_bytes,
                            file_bytes,
                            partial,
                        })
                        .await
                        .unwrap();
                    if (updater.line_no == Some(self.line_count)) {
                        trace!("Sending line to: {}", id);
                        updater
                            .channel
                            .send(ViewUpdate::Line {
                                line_no: self.line_count,
                                line: line.clone(),
                                line_chars,
                                line_bytes,
                                partial,
                            })
                            .await
                            .unwrap();
                        if !partial {
                            updater.line_no = None;
                        }
                    }
                }
            }
            ReaderUpdate::Truncated => {
                trace!("File truncated... resetting ifile");
                self.line_count = 0;
                self.lines = vec![];
                self.byte_count = 0;

                for (id, updater) in self.view_updaters.iter_mut() {
                    trace!("Sending truncate");
                    // TODO: Deal with unwrap
                    updater.line_no = None;
                    updater.channel.send(ViewUpdate::Truncated).await.unwrap();
                }
            }
            ReaderUpdate::FileError { reason } => {
                error!("File error: {:?}", reason);

                for (id, updater) in self.view_updaters.iter_mut() {
                    trace!("Forwarding error");
                    // TODO: Deal with unwrap
                    updater.line_no = None;
                    updater
                        .channel
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
        match cmd {
            ViewCommand::GetLine { id, line_no } => {
                trace!("Getting line: {} / {:?}", id, line_no);
                let Some(updater) = self.view_updaters.get_mut(&id) else {
                    error!("Unknown updater, ignoring request: {}", id);
                    return;
                };

                let Some(line_no) = line_no else {
                    trace!("Unregistering interest: {}", id);
                    updater.line_no = None;
                    return;
                };

                let sl = self.lines.get((line_no - 1) as usize);
                match sl {
                    None => {
                        trace!("Registering interest in: {} / {:?}", id, line_no);
                        updater.line_no = Some(line_no);
                    }
                    Some(sl) => {
                        updater
                            .channel
                            .send(ViewUpdate::Line {
                                line_no,
                                line: sl.content.clone(),
                                line_chars: sl.line_chars,
                                line_bytes: sl.line_bytes,
                                partial: sl.partial,
                            })
                            .await;
                    }
                }
            }
            ViewCommand::RegisterUpdater { id, updater } => {
                trace!("Registering an updater: {}", id);
                self.view_updaters.insert(
                    id.clone(),
                    Updater {
                        id,
                        channel: updater,
                        line_no: None,
                    },
                );
            }
        }
    }
}
