use anyhow::Result;
use log::{debug, error, info, trace, warn};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::select;
use tokio::sync::mpsc;

use crate::backing_file::BackingFile;
use crate::common::CHANNEL_BUFFER;
use crate::reader::{Reader, ReaderUpdate, ReaderUpdateReceiver};

pub type FileReqSender<T> = mpsc::Sender<FileReq<T>>;
pub type FileReqReceiver<T> = mpsc::Receiver<FileReq<T>>;

pub type FileRespSender<T> = mpsc::Sender<T>;
pub type FileRespReceiver<T> = mpsc::Receiver<T>;

#[derive(Debug)]
pub enum FileReq<T> {
    GetLine {
        id: String,
        line_no: usize,
    },
    CancelLine {
        id: String,
        line_no: usize,
    },
    RegisterClient {
        id: String,
        client_sender: mpsc::Sender<T>,
    },
    EnableTailing {
        id: String,
        last_seen_line: usize,
    },
    DisableTailing {
        id: String,
    },
}

#[derive(Debug)]
pub enum FileResp<L> {
    Stats {
        file_lines: usize,
        file_bytes: u64,
    },
    Line {
        line_no: usize,
        line_content: L,
        partial: bool,
    },
}

#[derive(Debug)]
pub enum IFResp<L> {
    ViewUpdate { update: FileResp<L> },
    Truncated,
    FileError { reason: String },
}

#[derive(Debug)]
struct SLine {
    offset: u64,
    _line_no: usize,
    _line_chars: usize,
    _line_bytes: usize,
    partial: bool,
}

#[derive(Debug)]
struct Client<L> {
    _id: String,
    channel: FileRespSender<IFResp<L>>,
    tailing: bool,
    interested: HashSet<usize>,
}

// Separate Clients from BackingFile to avoid overlapping references to &mut self.
#[derive(Debug)]
struct Clients {
    clients: HashMap<String, Client<String>>,
}

#[derive(Debug)]
pub struct IFile {
    view_receiver: FileReqReceiver<IFResp<String>>,
    view_sender: FileReqSender<IFResp<String>>,
    path: PathBuf,
    backing_file: BackingFile,
    lines: Vec<SLine>,
    file_lines: usize,
    file_bytes: u64,
    previous_partial: bool,
    clients: Clients,
}

impl IFile {
    pub fn new(path: &str) -> Result<IFile> {
        let mut pb = PathBuf::new();
        pb.push(path);

        let (view_sender, view_receiver) = mpsc::channel(CHANNEL_BUFFER);

        let backing_file = BackingFile::new(&pb)?;

        let ifile = IFile {
            path: pb,
            backing_file,
            view_receiver,
            view_sender,
            lines: vec![],
            file_lines: 0,
            file_bytes: 0,
            previous_partial: false,
            clients: Clients {
                clients: HashMap::new(),
            },
        };

        Ok(ifile)
    }

    fn run_reader(&mut self) -> ReaderUpdateReceiver {
        let (reader_sender, reader_receiver) = mpsc::channel(CHANNEL_BUFFER);
        let path = self.path.clone();
        tokio::spawn(async move {
            match Reader::run(path, reader_sender).await {
                Err(err) => {
                    error!("Reader failed: {:?}", err);
                }
                Ok(_) => {
                    info!("Reader finished normally");
                }
            }
        });

        reader_receiver
    }

    pub fn get_view_sender(&self) -> FileReqSender<IFResp<String>> {
        self.view_sender.clone()
    }

    pub async fn run(&mut self) -> Result<()> {
        debug!("Ifile starting: {:?}", self.path);

        let mut reader_receiver = self.run_reader();

        loop {
            trace!("Select...");
            select! {
                cmd = self.view_receiver.recv() => {
                    match cmd {
                        Some(cmd) => {
                            self.handle_client_command(cmd).await?;
                        },
                        None => {
                            debug!("Client IFR closed");
                            break;
                        }
                    }
                }
                update = reader_receiver.recv() => {
                    match update {
                        Some(update) => {
                            self.handle_reader_update(update).await?;
                        },
                        None => {
                            debug!("Reader update channel closed");
                            break;
                        }
                    }
                }
            }
        }

        trace!("IFile finished");

        Ok(())
    }

    async fn handle_reader_update(&mut self, update: ReaderUpdate) -> Result<bool> {
        match update {
            ReaderUpdate::Line {
                line_content,
                offset,
                line_bytes,
                partial,
                file_bytes,
            } => {
                let line_chars = line_content.len();

                let file_line_updated = if self.previous_partial {
                    // We know updated_line_no >= 1, as we cannot have a previous_partial before
                    // the first line comes in.
                    let file_line_updated = self.file_lines - 1;
                    self.lines[file_line_updated] = SLine {
                        offset,
                        _line_no: file_line_updated,
                        _line_chars: line_content.len(),
                        _line_bytes: line_bytes,
                        partial,
                    };

                    file_line_updated
                } else {
                    let file_line_updated = self.file_lines;
                    self.lines.push(SLine {
                        offset,
                        _line_no: file_line_updated,
                        _line_chars: line_content.len(),
                        _line_bytes: line_bytes,
                        partial,
                    });
                    self.file_lines += 1;

                    file_line_updated
                };

                self.previous_partial = partial;
                self.file_bytes = file_bytes;

                trace!(
                    "Adding/updating line: {} / partial: {} / len: {}",
                    file_line_updated,
                    partial,
                    line_chars
                );

                for (id, client) in self.clients.clients.iter_mut() {
                    trace!(
                        "Sending stats to client: {} - line {}",
                        id,
                        file_line_updated,
                    );
                    client
                        .channel
                        .send(IFResp::ViewUpdate {
                            update: FileResp::Stats {
                                file_lines: self.file_lines,
                                file_bytes,
                            },
                        })
                        .await?;
                    if client.interested.remove(&file_line_updated) || client.tailing {
                        trace!("Sending line to client: {}", id);
                        client
                            .channel
                            .send(IFResp::ViewUpdate {
                                update: FileResp::Line {
                                    line_no: file_line_updated,
                                    line_content: line_content.clone(),
                                    partial,
                                },
                            })
                            .await?;
                    }
                }
                Ok(false)
            }
            ReaderUpdate::Truncated => {
                trace!("File truncated... resetting ifile");
                self.file_lines = 0;
                self.lines = vec![];
                self.file_bytes = 0;

                for (id, client) in self.clients.clients.iter_mut() {
                    trace!("Sending truncate to client: {}", id);
                    client.interested = HashSet::new();
                    client.channel.send(IFResp::Truncated).await?;
                }
                Ok(true)
            }
            ReaderUpdate::FileError { reason } => {
                error!("File error: {:?}", reason);

                for (id, updater) in self.clients.clients.iter_mut() {
                    trace!("Forwarding error to client: {}", id);
                    updater.interested = HashSet::new();
                    updater
                        .channel
                        .send(IFResp::FileError {
                            reason: reason.clone(),
                        })
                        .await?;
                }
                Ok(false)
            }
        }
    }

    async fn handle_client_command(&mut self, cmd: FileReq<IFResp<String>>) -> Result<()> {
        match cmd {
            FileReq::GetLine { id, line_no } => {
                trace!("Client {} requested line {}", id, line_no);

                let clients = &mut self.clients;
                let Some(client) = clients.clients.get_mut(&id) else {
                    warn!("Unknown client, ignoring request: {}", id);
                    return Ok(());
                };

                let sl = self.lines.get_mut(line_no);
                match sl {
                    None => {
                        trace!("Registering interest in: {} / {:?}", id, line_no);
                        client.interested.insert(line_no);
                        Ok(())
                    }
                    Some(sl) => {
                        let backing_file = &mut self.backing_file;
                        let line_content = backing_file.read_line(Some(sl.offset as u64))?.clone();

                        trace!("Returning line: {}", line_no);
                        client
                            .channel
                            .send(IFResp::ViewUpdate {
                                update: FileResp::Line {
                                    line_no,
                                    line_content,
                                    partial: sl.partial,
                                },
                            })
                            .await?;
                        Ok(())
                    }
                }
            }
            FileReq::CancelLine { id, line_no } => {
                trace!("Cancel line: {} / {:?}", id, line_no);
                let Some(client) = self.clients.clients.get_mut(&id) else {
                    warn!("Unknown client, ignoring request: {}", id);
                    return Ok(());
                };

                if !client.interested.remove(&line_no) {
                    warn!("Client cancelled line that was not registered for interest: client {}, line {}", id, line_no);
                }
                Ok(())
            }
            FileReq::RegisterClient { id, client_sender } => {
                trace!("Registering client: {}", id);
                self.clients.clients.insert(
                    id.clone(),
                    Client {
                        _id: id,
                        channel: client_sender.clone(),
                        tailing: false,
                        interested: HashSet::new(),
                    },
                );

                client_sender
                    .send(IFResp::ViewUpdate {
                        update: FileResp::Stats {
                            file_lines: self.file_lines,
                            file_bytes: self.file_bytes,
                        },
                    })
                    .await?;
                Ok(())
            }
            FileReq::EnableTailing { id, last_seen_line } => {
                trace!("Enable tailing: {}", id);
                let clients = &mut self.clients;
                let Some(client) = clients.clients.get_mut(&id) else {
                    warn!("Unknown client, ignoring request: {}", id);
                    return Ok(());
                };

                client.tailing = true;

                // Determine which lines the client will not know about.
                for i in last_seen_line..self.file_lines {
                    let sl = self.lines.get(i);
                    let Some(l) = sl else {
                        warn!("Unknown line whilst sending missing tailing lines: {}", i);
                        continue;
                    };

                    let backing_file = &mut self.backing_file;
                    let line_content = backing_file.read_line(Some(l.offset as u64))?.clone();

                    trace!("Forwaring missing line: {}", i);
                    client
                        .channel
                        .send(IFResp::ViewUpdate {
                            update: FileResp::Line {
                                line_no: i,
                                line_content,
                                partial: l.partial,
                            },
                        })
                        .await?;
                }
                Ok(())
            }
            FileReq::DisableTailing { id } => {
                trace!("Disable tailing: {}", id);

                let Some(client) = self.clients.clients.get_mut(&id) else {
                    warn!("Unknown client, ignoring request: {}", id);
                    return Ok(());
                };

                client.tailing = false;
                Ok(())
            }
        }
    }
}
