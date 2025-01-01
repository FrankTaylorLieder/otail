use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use log::{debug, trace, warn};
use regex::Regex;
use tokio::select;
use tokio::sync::{mpsc, oneshot};

use crate::common::{CHANNEL_BUFFER, FILTER_SPOOLING_BATCH_SIZE};
use crate::ifile::{
    FileReq, FileReqReceiver, FileReqSender, FileResp, FileRespReceiver, FileRespSender, IFResp,
};

pub type FFRespSender = mpsc::Sender<FFResp>;
pub type FFRespReceiver = mpsc::Receiver<FFResp>;

pub type FFReqSender = mpsc::Sender<FFReq>;
pub type FFReqReceiver = mpsc::Receiver<FFReq>;

pub type FilterReqRespSender = oneshot::Sender<FFReqResp>;
pub type FilterReqRespReceiver = oneshot::Receiver<FFReqResp>;

#[derive(Debug)]
pub enum FFResp {
    ViewUpdate { update: FileResp },
    Clear,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum FilterMode {
    SimpleCaseSensitive,
    SimpleCaseInsensitive,
    Regex,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FilterSpec {
    pub filter: String,
    pub mode: FilterMode,
}

#[derive(Debug, Clone)]
pub enum FFReqResp {
    Ok,
    Err { message: String },
}

#[derive(Debug)]
pub enum FFReq {
    SetFilter {
        filter_spec: Option<FilterSpec>,
        response: Option<FilterReqRespSender>,
    },
}

#[derive(Debug)]
struct Client {
    id: String,
    channel: FFRespSender,
    tailing: bool,
    interested: HashSet<usize>,
    pending: HashSet<usize>,
}

type LineNo = usize;

struct FilterState {
    filter_spec: FilterSpec,
    filter_re: Regex,
    matches: Vec<LineNo>,
    num_matches: usize,
    next_line: LineNo,
}

pub struct FFile {
    id: String,
    path: PathBuf,

    // Interact with the View
    view_req_receiver: FileReqReceiver<FFResp>,
    view_req_sender: FileReqSender<FFResp>,

    // Interact with the TUI
    ff_req_sender: FFReqSender,
    ff_req_receiver: FFReqReceiver,

    // Interact with the downstream IFile
    if_resp_receiver: FileRespReceiver<IFResp>,
    if_resp_sender: FileRespSender<IFResp>,
    if_req_sender: FileReqSender<IFResp>,

    clients: HashMap<String, Client>,

    filter_state: Option<FilterState>,
}

impl FFile {
    pub fn new(id: String, path: &str, if_req_sender: FileReqSender<IFResp>) -> FFile {
        let mut pb = PathBuf::new();
        pb.push(path);

        let (view_req_sender, view_req_receiver) = mpsc::channel(CHANNEL_BUFFER);
        let (ff_req_sender, ff_req_receiver) = mpsc::channel(CHANNEL_BUFFER);
        let (if_resp_sender, if_resp_receiver) = mpsc::channel(CHANNEL_BUFFER);

        FFile {
            id,
            path: pb,

            view_req_sender,
            view_req_receiver,

            ff_req_sender,
            ff_req_receiver,

            if_resp_sender,
            if_resp_receiver,

            if_req_sender,

            clients: HashMap::new(),

            filter_state: None,
        }
    }

    pub fn get_view_sender(&self) -> FileReqSender<FFResp> {
        self.view_req_sender.clone()
    }

    pub fn get_ff_sender(&self) -> FFReqSender {
        self.ff_req_sender.clone()
    }

    pub async fn run(&mut self) -> Result<()> {
        debug!("FFile starting: {:?}", self.path);

        self.if_req_sender
            .send(crate::ifile::FileReq::RegisterClient {
                id: self.id.clone(),
                client_sender: self.if_resp_sender.clone(),
            })
            .await?;

        trace!("Waiting on commands/updates...");

        loop {
            trace!("Select...");
            select! {
                cmd = self.view_req_receiver.recv() => {
                    match cmd {
                        Some(cmd) => {
                            self.handle_client_command(cmd).await?;
                        },
                        None => {
                            debug!("View channel closed");
                            break;
                        }
                    }
                }
                cmd = self.ff_req_receiver.recv() => {
                    match cmd {
                        Some(cmd) => {
                            self.handle_ff_command(cmd).await?;
                        },
                        None => {
                            debug!("FF channel closed");
                            break;
                        }
                    }
                }
                update = self.if_resp_receiver.recv() => {
                    match update {
                        Some(update) => {
                            self.handle_ifile_update(update).await?;
                        },
                        None => {
                            debug!("IFile update channel closed");
                            break;
                        }
                    }
                }
            }
            trace!("Looping...");
        }

        trace!("FFile finished");

        Ok(())
    }

    async fn handle_ff_command(&mut self, cmd: FFReq) -> Result<()> {
        // TODO: Send response if a response channel is provided.
        // TODO: Handle errors properly.
        match cmd {
            FFReq::SetFilter {
                filter_spec,
                response,
            } => {
                trace!("Setting filter: {:?}", filter_spec);

                let Some(filter_spec) = filter_spec else {
                    trace!("Removing filter");
                    self.filter_state = None;

                    for (name, client) in self.clients.iter() {
                        client.channel.send(FFResp::Clear).await?;
                    }
                    return Ok(());
                };

                if let Some(filter_state) = &self.filter_state {
                    if filter_state.filter_spec == filter_spec {
                        trace!("Filter unchanged, no change.");
                        return Ok(());
                    }
                }

                let filter_re = Regex::new(&filter_spec.filter)?;

                self.filter_state = Some(FilterState {
                    filter_spec,
                    filter_re,
                    matches: Vec::new(),
                    num_matches: 0,
                    next_line: 0,
                });

                for (name, client) in self.clients.iter() {
                    client.channel.send(FFResp::Clear).await?;
                }

                self.start_spooling().await?;

                Ok(())
            }
        }
    }

    async fn handle_client_command(&mut self, cmd: FileReq<FFResp>) -> Result<()> {
        match cmd {
            FileReq::GetLine { id, line_no } => {
                trace!("Client {} requested match {}", id, line_no);
                let Some(client) = self.clients.get_mut(&id) else {
                    warn!("Unknown client, ignoring request: {}", id);
                    return Ok(());
                };

                let Some(filter_state) = &self.filter_state else {
                    warn!("No current filter applied. Ignoring. {}", id);
                    return Ok(());
                };

                let maybe_line_no = filter_state.matches.get(line_no);
                match maybe_line_no {
                    None => {
                        trace!("Registering interest in: {} / {}", id, line_no);
                        client.interested.insert(line_no);
                        Ok(())
                    }
                    Some(line_no) => {
                        trace!("Requesting match line: {} / {}", line_no, line_no);

                        self.if_req_sender
                            .send(crate::ifile::FileReq::GetLine {
                                id: self.id.clone(),
                                line_no: *line_no,
                            })
                            .await?;

                        client.pending.insert(*line_no);

                        Ok(())
                    }
                }
            }
            FileReq::CancelLine { id, line_no } => {
                trace!("Cancel match: {} / {:?}", id, line_no);
                let Some(client) = self.clients.get_mut(&id) else {
                    warn!("Unknown client, ignoring request: {}", id);
                    return Ok(());
                };

                if !client.interested.remove(&line_no) {
                    warn!("Client cancelled match that was not registered for interest: client {}, line {}", id, line_no);
                }
                Ok(())
            }
            FileReq::RegisterClient { id, client_sender } => {
                trace!("Registering ffile client: {}", id);
                self.clients.insert(
                    id.clone(),
                    Client {
                        id,
                        channel: client_sender.clone(),
                        tailing: false,
                        interested: HashSet::new(),
                        pending: HashSet::new(),
                    },
                );

                client_sender
                    .send(FFResp::ViewUpdate {
                        update: FileResp::Stats {
                            file_lines: 0,
                            file_bytes: 0,
                        },
                    })
                    .await?;

                trace!("Finished register");
                Ok(())
            }
            FileReq::EnableTailing { id, last_seen_line } => todo!(),
            FileReq::DisableTailing { id } => todo!(),
        }
    }

    async fn start_spooling(&mut self) -> Result<()> {
        trace!("Start spooling: {}", self.id);
        let Some(filter_state) = &mut self.filter_state else {
            warn!(
                "Attempted to start spooling without a filter set: {}",
                self.id
            );
            return Err(anyhow!("Spooling without filter"));
        };

        for i in 0..FILTER_SPOOLING_BATCH_SIZE {
            self.if_req_sender
                .send(FileReq::GetLine {
                    id: self.id.clone(),
                    line_no: i,
                })
                .await?;

            filter_state.next_line += 1;
        }

        Ok(())
    }

    async fn next_spooling(
        &mut self,
        line_no: LineNo,
        line_content: String,
        partial: bool,
    ) -> Result<()> {
        trace!("Next spooling: {} / {}", self.id, line_no);
        let Some(filter_state) = &mut self.filter_state else {
            trace!("Not spooling, ignore line. {} / {}", self.id, line_no);
            return Ok(());
        };

        if filter_state.filter_re.find(&line_content).is_some() {
            trace!("Line matches...");
            // TODO: Can we be sure that the updates come in order?
            filter_state.matches.push(line_no);

            let match_no = filter_state.num_matches;
            filter_state.num_matches += 1;

            for (_, client) in self.clients.iter_mut() {
                client
                    .channel
                    .send(FFResp::ViewUpdate {
                        update: FileResp::Stats {
                            file_lines: filter_state.num_matches,
                            file_bytes: 0,
                        },
                    })
                    .await?;

                if client.interested.remove(&match_no) {
                    client
                        .channel
                        .send(FFResp::ViewUpdate {
                            update: FileResp::Line {
                                line_no: match_no,
                                line_content: line_content.clone(),
                                partial,
                            },
                        })
                        .await?;
                }
            }
        } else {
            trace!("Line does not match");
            trace!(
                "XXX Missed: {} / {}$",
                filter_state.filter_spec.filter,
                line_content
            );
        }

        self.if_req_sender
            .send(FileReq::GetLine {
                id: self.id.clone(),
                line_no: filter_state.next_line,
            })
            .await?;

        filter_state.next_line += 1;

        Ok(())
    }

    async fn handle_ifile_update(&mut self, update: IFResp) -> Result<()> {
        match update {
            IFResp::ViewUpdate {
                update:
                    FileResp::Line {
                        line_no,
                        line_content,
                        partial,
                    },
            } => {
                self.next_spooling(line_no, line_content, partial).await?;
            }
            _ => {
                trace!("Ignoring unimportant message: {:?}", update);
            }
        }

        Ok(())
    }
}
