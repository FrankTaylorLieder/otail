use anyhow::Result;
use log::{debug, error, info, trace, warn};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;
use std::{thread, usize};
use tokio::fs::File;
use tokio::select;
use tokio::sync::{mpsc, oneshot};

use crate::reader::{Reader, ReaderUpdate, ReaderUpdateReceiver, ReaderUpdateSender};

pub type IFReqSender = mpsc::Sender<IFReq>;
pub type IFReqReceiver = mpsc::Receiver<IFReq>;

pub type IFRespSender = mpsc::Sender<IFResp>;
pub type IFRespReceiver = mpsc::Receiver<IFResp>;

#[derive(Debug)]
pub enum IFReq {
    GetLine {
        id: String,
        line_no: u32, // 0-based
    },
    CancelLine {
        id: String,
        line_no: u32, // 0-based
    },
    RegisterClient {
        id: String,
        client_sender: IFRespSender,
    },
}

#[derive(Debug)]
pub enum IFResp {
    Stats {
        file_lines: u32,
        file_bytes: u32,
    },
    Line {
        line_no: u32,
        line_content: String,
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
struct SLine {
    content: String,
    line_no: u32,
    line_chars: u32,
    line_bytes: u32,
    partial: bool,
}

#[derive(Debug)]
struct Client {
    id: String,
    channel: IFRespSender,
    tailing: bool,
    interested: HashSet<u32>,
}

#[derive(Debug)]
pub struct IFile {
    view_receiver: IFReqReceiver,
    view_sender: IFReqSender,
    reader_receiver: ReaderUpdateReceiver,
    reader_sender: ReaderUpdateSender,
    path: PathBuf,
    lines: Vec<SLine>,
    file_lines: u32,
    file_bytes: u32,
    clients: HashMap<String, Client>,
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
            file_lines: 0,
            file_bytes: 0,
            clients: HashMap::new(),
        }
    }

    fn run_reader(&mut self, cs: ReaderUpdateSender) {
        let cs = cs.clone();
        let path = self.path.clone();
        tokio::spawn(async move {
            Reader::run(path, cs).await;
        });
    }

    pub fn get_view_sender(&self) -> IFReqSender {
        self.view_sender.clone()
    }

    pub async fn run(&mut self) -> Result<()> {
        debug!("Ifile starting: {:?}", self.path);

        self.run_reader(self.reader_sender.clone());

        trace!("Waiting on commands/updates...");

        // TODO: Deal with closing command channels to signal shutting down
        loop {
            trace!("Select...");
            select! {
                cmd = self.view_receiver.recv() => {
                    match cmd {
                        Some(cmd) => {
                            self.handle_client_command(cmd).await;
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
                line_content,
                line_bytes,
                partial,
                file_bytes,
            } => {
                let line_chars = line_content.len() as u32;

                let updated_line_no = self.file_lines;
                if partial {
                    self.lines[self.file_lines as usize] = SLine {
                        content: line_content.clone(),
                        line_no: self.file_lines,
                        line_chars: line_content.len() as u32,
                        line_bytes,
                        partial: true,
                    }
                } else {
                    self.lines.push(SLine {
                        content: line_content.clone(),
                        line_no: self.file_lines,
                        line_chars: line_content.len() as u32,
                        line_bytes,
                        partial: false,
                    });
                    self.file_lines += 1;
                }

                self.file_bytes = file_bytes;

                trace!(
                    "Adding/updating line: {} / partial: {} / len: {}",
                    updated_line_no,
                    partial,
                    line_chars
                );

                for (id, client) in self.clients.iter_mut() {
                    trace!("Sending update to client: {}", id);
                    // TODO: Deal with unwrap
                    client
                        .channel
                        .send(IFResp::Stats {
                            file_lines: self.file_lines,
                            file_bytes,
                        })
                        .await
                        .unwrap();
                    if (client.interested.remove(&updated_line_no)) {
                        trace!("Sending line to: {}", id);
                        client
                            .channel
                            .send(IFResp::Line {
                                line_no: self.file_lines,
                                line_content: line_content.clone(),
                                line_chars,
                                line_bytes,
                                partial,
                            })
                            .await
                            .unwrap();
                    }
                }
            }
            ReaderUpdate::Truncated => {
                trace!("File truncated... resetting ifile");
                self.file_lines = 0;
                self.lines = vec![];
                self.file_bytes = 0;

                for (id, client) in self.clients.iter_mut() {
                    trace!("Sending truncate");
                    // TODO: Deal with unwrap
                    client.interested = HashSet::new();
                    client.channel.send(IFResp::Truncated).await.unwrap();
                }
            }
            ReaderUpdate::FileError { reason } => {
                error!("File error: {:?}", reason);

                for (id, updater) in self.clients.iter_mut() {
                    trace!("Forwarding error");
                    // TODO: Deal with unwrap
                    updater.interested = HashSet::new();
                    updater
                        .channel
                        .send(IFResp::FileError {
                            reason: reason.clone(),
                        })
                        .await
                        .unwrap();
                }
            }
        }
    }

    async fn handle_client_command(&mut self, cmd: IFReq) {
        match cmd {
            IFReq::GetLine { id, line_no } => {
                trace!("Client {} requested line {}", id, line_no);
                let Some(client) = self.clients.get_mut(&id) else {
                    warn!("Unknown client, ignoring request: {}", id);
                    return;
                };

                let sl = self.lines.get((line_no - 1) as usize);
                match sl {
                    None => {
                        trace!("Registering interest in: {} / {:?}", id, line_no);
                        client.interested.insert(line_no);
                    }
                    Some(sl) => {
                        // TODO: Fetch the data from the file rather than locally stored data.
                        client
                            .channel
                            .send(IFResp::Line {
                                line_no,
                                line_content: sl.content.clone(),
                                line_chars: sl.line_chars,
                                line_bytes: sl.line_bytes,
                                partial: sl.partial,
                            })
                            .await;
                    }
                }
            }
            IFReq::CancelLine { id, line_no } => {
                trace!("Cancel line: {} / {:?}", id, line_no);
                let Some(client) = self.clients.get_mut(&id) else {
                    warn!("Unknown client, ignoring request: {}", id);
                    return;
                };

                if !client.interested.remove(&line_no) {
                    warn!("Client cancelled line that was not registered for interest: client {}, line {}", id, line_no);
                }
            }
            IFReq::RegisterClient {
                id,
                client_sender: updater,
            } => {
                trace!("Registering client: {}", id);
                self.clients.insert(
                    id.clone(),
                    Client {
                        id,
                        channel: updater,
                        tailing: false,
                        interested: HashSet::new(),
                    },
                );
            }
        }
    }
}
