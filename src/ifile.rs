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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn test_file_req_variants() {
        let req = FileReq::<String>::GetLine {
            id: "test".to_string(),
            line_no: 42,
        };
        
        match req {
            FileReq::GetLine { id, line_no } => {
                assert_eq!(id, "test");
                assert_eq!(line_no, 42);
            },
            _ => panic!("Should be GetLine variant"),
        }

        let req = FileReq::<String>::CancelLine {
            id: "test".to_string(),
            line_no: 5,
        };
        
        match req {
            FileReq::CancelLine { id, line_no } => {
                assert_eq!(id, "test");
                assert_eq!(line_no, 5);
            },
            _ => panic!("Should be CancelLine variant"),
        }

        let (sender, _receiver) = mpsc::channel(10);
        let req = FileReq::<String>::RegisterClient {
            id: "client".to_string(),
            client_sender: sender,
        };
        
        match req {
            FileReq::RegisterClient { id, .. } => {
                assert_eq!(id, "client");
            },
            _ => panic!("Should be RegisterClient variant"),
        }

        let req = FileReq::<String>::EnableTailing {
            id: "test".to_string(),
            last_seen_line: 100,
        };
        
        match req {
            FileReq::EnableTailing { id, last_seen_line } => {
                assert_eq!(id, "test");
                assert_eq!(last_seen_line, 100);
            },
            _ => panic!("Should be EnableTailing variant"),
        }

        let req = FileReq::<String>::DisableTailing {
            id: "test".to_string(),
        };
        
        match req {
            FileReq::DisableTailing { id } => {
                assert_eq!(id, "test");
            },
            _ => panic!("Should be DisableTailing variant"),
        }
    }

    #[test]
    fn test_file_resp_variants() {
        let resp = FileResp::<String>::Stats {
            file_lines: 100,
            file_bytes: 1024,
        };
        
        match resp {
            FileResp::Stats { file_lines, file_bytes } => {
                assert_eq!(file_lines, 100);
                assert_eq!(file_bytes, 1024);
            },
            _ => panic!("Should be Stats variant"),
        }

        let resp = FileResp::<String>::Line {
            line_no: 42,
            line_content: "test line".to_string(),
            partial: true,
        };
        
        match resp {
            FileResp::Line { line_no, line_content, partial } => {
                assert_eq!(line_no, 42);
                assert_eq!(line_content, "test line");
                assert!(partial);
            },
            _ => panic!("Should be Line variant"),
        }
    }

    #[test]
    fn test_ifresp_variants() {
        let resp = IFResp::<String>::ViewUpdate { 
            update: FileResp::Stats { file_lines: 50, file_bytes: 512 }
        };
        
        match resp {
            IFResp::ViewUpdate { update } => {
                match update {
                    FileResp::Stats { file_lines, file_bytes } => {
                        assert_eq!(file_lines, 50);
                        assert_eq!(file_bytes, 512);
                    },
                    _ => panic!("Should be Stats update"),
                }
            },
            _ => panic!("Should be ViewUpdate variant"),
        }

        let resp = IFResp::<String>::Truncated;
        match resp {
            IFResp::Truncated => assert!(true),
            _ => panic!("Should be Truncated variant"),
        }
    }

    // Note: Client struct is private and doesn't have a public `new` method
    // LineReq is also not exposed in the public API
    // These tests focus on the public API instead
}
