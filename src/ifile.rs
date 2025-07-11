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
        view_lines: usize,
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

/// Provide access to a file on disk, for random access and watching for changes.
#[derive(Debug)]
pub struct IFile<BF: BackingFile> {
    view_receiver: FileReqReceiver<IFResp<String>>,
    view_sender: FileReqSender<IFResp<String>>,
    path: PathBuf,
    backing_file: BF,
    lines: Vec<SLine>,
    file_lines: usize,
    file_bytes: u64,
    previous_partial: bool,
    clients: Clients,
}

impl<BF: BackingFile> IFile<BF> {
    pub fn new(path: &str, backing_file: BF) -> IFile<BF> {
        let mut pb = PathBuf::new();
        pb.push(path);

        let (view_sender, view_receiver) = mpsc::channel(CHANNEL_BUFFER);

        IFile {
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
        }
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
                            trace!("Received client command: {:?}", cmd);
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
                            trace!("Received reader update: {:?}", update);
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

    /// Handle an update from the reader.
    ///
    /// Returns boolean indicating if the file should be closed
    async fn handle_reader_update(&mut self, update: ReaderUpdate) -> Result<()> {
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
                        "Sending stats to client: {} - line {}, file_lines: {}, file_bytes: {}",
                        id,
                        file_line_updated,
                        self.file_lines,
                        file_bytes
                    );
                    let send_result = client
                        .channel
                        .send(IFResp::ViewUpdate {
                            update: FileResp::Stats {
                                view_lines: self.file_lines,
                                file_lines: self.file_lines,
                                file_bytes,
                            },
                        })
                        .await;
                    if let Err(e) = &send_result {
                        trace!("Failed to send stats to client {}: {:?}", id, e);
                    }
                    send_result?;
                    let was_interested = client.interested.remove(&file_line_updated);
                    if was_interested || client.tailing {
                        let reason = if was_interested {
                            "interested"
                        } else {
                            "tailing"
                        };
                        trace!(
                            "Sending line to client {} ({}): line_no={}, partial={}, content_len={}",
                            id, reason, file_line_updated, partial, line_content.len()
                        );
                        let send_result = client
                            .channel
                            .send(IFResp::ViewUpdate {
                                update: FileResp::Line {
                                    line_no: file_line_updated,
                                    line_content: line_content.clone(),
                                    partial,
                                },
                            })
                            .await;
                        if let Err(e) = &send_result {
                            trace!("Failed to send line to client {}: {:?}", id, e);
                        }
                        send_result?;
                    }
                }
                Ok(())
            }
            ReaderUpdate::Truncated => {
                trace!("File truncated... resetting ifile");
                self.file_lines = 0;
                self.lines = vec![];
                self.file_bytes = 0;

                for (id, client) in self.clients.clients.iter_mut() {
                    trace!("Sending truncate to client: {}", id);
                    client.interested = HashSet::new();
                    let send_result = client.channel.send(IFResp::Truncated).await;
                    if let Err(e) = &send_result {
                        trace!("Failed to send truncate to client {}: {:?}", id, e);
                    }
                    send_result?;
                }
                Ok(())
            }
            ReaderUpdate::FileError { reason } => {
                error!("File error: {:?}", reason);

                for (id, updater) in self.clients.clients.iter_mut() {
                    trace!("Forwarding error to client {}: {}", id, reason);
                    updater.interested = HashSet::new();
                    let send_result = updater
                        .channel
                        .send(IFResp::FileError {
                            reason: reason.clone(),
                        })
                        .await;
                    if let Err(e) = &send_result {
                        trace!("Failed to send error to client {}: {:?}", id, e);
                    }
                    send_result?;
                }
                Ok(())
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

                        trace!(
                            "Sending requested line to client {}: line_no={}, partial={}, content_len={}",
                            id, line_no, sl.partial, line_content.len()
                        );
                        let send_result = client
                            .channel
                            .send(IFResp::ViewUpdate {
                                update: FileResp::Line {
                                    line_no,
                                    line_content,
                                    partial: sl.partial,
                                },
                            })
                            .await;
                        if let Err(e) = &send_result {
                            trace!("Failed to send requested line to client {}: {:?}", id, e);
                        }
                        send_result?;
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
                        _id: id.clone(),
                        channel: client_sender.clone(),
                        tailing: false,
                        interested: HashSet::new(),
                    },
                );

                trace!(
                    "Sending initial stats to new client {}: file_lines={}, file_bytes={}",
                    id,
                    self.file_lines,
                    self.file_bytes
                );
                let send_result = client_sender
                    .send(IFResp::ViewUpdate {
                        update: FileResp::Stats {
                            view_lines: self.file_lines,
                            file_lines: self.file_lines,
                            file_bytes: self.file_bytes,
                        },
                    })
                    .await;
                if let Err(e) = &send_result {
                    trace!("Failed to send initial stats to client {}: {:?}", id, e);
                }
                send_result?;
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
                let missing_lines_count = self.file_lines.saturating_sub(last_seen_line);
                trace!(
                    "Sending {} missing lines to client {} (last_seen_line={}, file_lines={})",
                    missing_lines_count,
                    id,
                    last_seen_line,
                    self.file_lines
                );
                for i in last_seen_line..self.file_lines {
                    let sl = self.lines.get(i);
                    let Some(l) = sl else {
                        warn!("Unknown line whilst sending missing tailing lines: {}", i);
                        continue;
                    };

                    let backing_file = &mut self.backing_file;
                    let line_content = backing_file.read_line(Some(l.offset as u64))?.clone();

                    trace!(
                        "Sending missing line to client {}: line_no={}, partial={}, content_len={}",
                        id,
                        i,
                        l.partial,
                        line_content.len()
                    );
                    let send_result = client
                        .channel
                        .send(IFResp::ViewUpdate {
                            update: FileResp::Line {
                                line_no: i,
                                line_content,
                                partial: l.partial,
                            },
                        })
                        .await;
                    if let Err(e) = &send_result {
                        trace!("Failed to send missing line to client {}: {:?}", id, e);
                    }
                    send_result?;
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
    //use mockall::{mock, predicate::*};

    use super::*;
    use crate::backing_file::MockBackingFile;
    use flexi_logger::{detailed_format, FileSpec};
    use tokio::sync::mpsc::Receiver;

    fn init_test_logging() {
        let _ = flexi_logger::Logger::try_with_env()
            .unwrap()
            .log_to_file(FileSpec::default().suffix("test-log").use_timestamp(false))
            .append()
            .format(detailed_format)
            .start();
    }

    #[tokio::test]
    async fn test_ifile_single_line() {
        init_test_logging();

        let line0 = "FirstFullLine".to_owned();
        let line0_len = line0.len();

        let mut backing_file = MockBackingFile::new();
        backing_file
            .expect_read_line()
            .with(mockall::predicate::eq(Some(0u64)))
            .times(1)
            .returning({
                let line0 = line0.clone();
                move |_| Ok(line0.clone())
            });

        let mut ifile = IFile::new("test", backing_file);

        let client_id = "test_client".to_owned();
        let (client_sender, mut client_receiver) = mpsc::channel(CHANNEL_BUFFER);

        let register_result = ifile
            .handle_client_command(FileReq::RegisterClient {
                id: client_id.clone(),
                client_sender,
            })
            .await;

        assert!(
            register_result.is_ok(),
            "Failed to register client of file: {:?}",
            register_result
        );

        let tailing_result = ifile
            .handle_client_command(FileReq::EnableTailing {
                id: client_id.clone(),
                last_seen_line: 0,
            })
            .await;

        assert!(
            tailing_result.is_ok(),
            "Failed to register tail for client: {:?}",
            tailing_result
        );

        let mut file_bytes = 0_u64;
        let mut file_lines = 0_usize;

        file_lines += 1;
        file_bytes += line0_len as u64;
        let r = ifile
            .handle_reader_update(ReaderUpdate::Line {
                line_content: line0.clone(),
                offset: 0,
                line_bytes: line0_len,
                partial: false,
                file_bytes,
            })
            .await;

        assert!(r.is_ok(), "Expected Ok for line0, got {:?}", r);

        check_viewupdate_fileresp_stats(
            &mut client_receiver,
            Some(0),
            Some(0),
            "Initial stats response",
        );

        check_viewupdate_fileresp_stats(
            &mut client_receiver,
            Some(file_lines),
            Some(file_bytes),
            "First stats update when new content arrives",
        );

        check_viewupdate_fileresp_line(
            &mut client_receiver,
            Some(0),
            Some(&line0),
            Some(false),
            "First line update",
        );

        // Test GetLine request to trigger mock call
        let getline_result = ifile
            .handle_client_command(FileReq::GetLine {
                id: client_id.clone(),
                line_no: 0,
            })
            .await;

        assert!(
            getline_result.is_ok(),
            "Failed to get line from client: {:?}",
            getline_result
        );

        check_viewupdate_fileresp_line(
            &mut client_receiver,
            Some(0),
            Some(&line0),
            Some(false),
            "GetLine response from mock",
        );

        // The mock expectations will be verified automatically when the mock is dropped
    }

    fn check_viewupdate_fileresp_line(
        client_receiver: &mut Receiver<IFResp<String>>,
        expected_line_no: Option<usize>,
        expected_content: Option<&str>,
        expected_partial: Option<bool>,
        context: &str,
    ) {
        let message = client_receiver.try_recv();
        assert!(
            message.is_ok(),
            "{}: Failed to receive client message: {:?}",
            context,
            message
        );

        let Ok(message) = message else {
            panic!("{}: Error whilst fetching message: {:?}", context, message);
        };

        if let IFResp::ViewUpdate {
            update:
                FileResp::Line {
                    line_no,
                    line_content,
                    partial,
                },
        } = message
        {
            if let Some(expected_line_no) = expected_line_no {
                assert_eq!(line_no, expected_line_no, "Wrong line number");
            }
            if let Some(expected_content) = expected_content {
                assert_eq!(line_content, expected_content, "Wrong content");
            }
            if let Some(expected_partial) = expected_partial {
                assert_eq!(partial, expected_partial, "Wrong partial");
            }
        } else {
            panic!("Incorrect response");
        }
    }

    // Additional tests:
    // - Partial lines
    // - Truncation
    // - Tailing

    // Check that the next message on this receiver is a Stats message with the relevant values.
    // Supply `None` if the specific attribute should not be checked.
    fn check_viewupdate_fileresp_stats(
        client_receiver: &mut Receiver<IFResp<String>>,
        expected_lines: Option<usize>,
        expected_bytes: Option<u64>,
        context: &str,
    ) {
        let message = client_receiver.try_recv();
        assert!(
            message.is_ok(),
            "{}: Failed to receive client message: {:?}",
            context,
            message
        );

        let Ok(message) = message else {
            panic!("{}: Error whilst fetching message: {:?}", context, message);
        };

        if let IFResp::ViewUpdate {
            update:
                FileResp::Stats {
                    view_lines,
                    file_lines,
                    file_bytes,
                },
        } = message
        {
            if let Some(expected_lines) = expected_lines {
                assert_eq!(file_lines, expected_lines, "{}: Wrong file_lines", context);
                assert_eq!(view_lines, expected_lines, "{}: Wrong view_lines", context);
            }
            if let Some(expected_bytes) = expected_bytes {
                assert_eq!(
                    file_bytes, expected_bytes,
                    "{}: Wrong bytes",
                    expected_bytes
                );
            }
        } else {
            panic!("{}: Unexpected message type: {:?}", context, message);
        }
    }
}
