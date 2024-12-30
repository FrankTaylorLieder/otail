use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use log::{debug, trace, warn};
use tokio::select;
use tokio::sync::mpsc;

use crate::common::CHANNEL_BUFFER;
use crate::ifile::{IFReqReceiver, IFReqSender, IFResp, IFRespReceiver, IFRespSender};

pub type FFReqSender = mpsc::Sender<FFReq>;
pub type FFReqReceiver = mpsc::Receiver<FFReq>;

pub type FFRespSender = mpsc::Sender<FFResp>;
pub type FFRespReceiver = mpsc::Receiver<FFResp>;

#[derive(Debug)]
pub enum FFReq {
    GetMatch {
        id: String,
        match_no: usize,
    },
    CancelMatch {
        id: String,
        match_no: usize,
    },
    RegisterClient {
        id: String,
        client_sender: FFRespSender,
    },
    EnableTailing {
        id: String,
        last_seen_line: usize,
    },
    DisableTailing {
        id: String,
    },
    SetFilter {
        maybe_filter: Option<String>,
    },
}

#[derive(Debug)]
pub enum FFResp {
    Stats { matches: usize },
    Line { line_no: usize, line_content: usize },
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

pub struct FFile {
    id: String,
    view_receiver: FFReqReceiver,
    view_sender: FFReqSender,
    ifile_resp_receiver: IFRespReceiver,
    ifile_resp_sender: IFRespSender,
    ifile_req_sender: IFReqSender,
    path: PathBuf,
    matches: Vec<LineNo>,
    clients: HashMap<String, Client>,
}

impl FFile {
    pub fn new(id: String, path: &str, ifile_req_sender: IFReqSender) -> FFile {
        let mut pb = PathBuf::new();
        pb.push(path);

        let (view_sender, view_receiver) = mpsc::channel(CHANNEL_BUFFER);
        let (ifile_resp_sender, ifile_resp_receiver) = mpsc::channel(CHANNEL_BUFFER);

        FFile {
            id,
            path: pb,

            view_sender,
            view_receiver,

            ifile_resp_sender,
            ifile_resp_receiver,

            ifile_req_sender,

            matches: Vec::new(),
            clients: HashMap::new(),
        }
    }

    pub fn get_view_sender(&self) -> FFReqSender {
        self.view_sender.clone()
    }

    pub async fn run(&mut self) -> Result<()> {
        debug!("Ffile starting: {:?}", self.path);

        self.ifile_req_sender
            .send(crate::ifile::IFReq::RegisterClient {
                id: self.id.clone(),
                client_sender: self.ifile_resp_sender.clone(),
            })
            .await?;

        trace!("Waiting on commands/updates...");

        loop {
            trace!("Select...");
            select! {
                cmd = self.view_receiver.recv() => {
                    match cmd {
                        Some(cmd) => {
                            self.handle_client_command(cmd).await?;
                        },
                        None => {
                            debug!("Client FFR closed");
                            break;
                        }
                    }
                }
                update = self.ifile_resp_receiver.recv() => {
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

    async fn handle_client_command(&mut self, cmd: FFReq) -> Result<()> {
        match cmd {
            FFReq::GetMatch { id, match_no } => {
                trace!("Client {} requested match {}", id, match_no);
                let Some(client) = self.clients.get_mut(&id) else {
                    warn!("Unknown client, ignoring request: {}", id);
                    return Ok(());
                };

                let maybe_line_no = self.matches.get(match_no);
                match maybe_line_no {
                    None => {
                        trace!("Registering interest in: {} / {}", id, match_no);
                        client.interested.insert(match_no);
                        Ok(())
                    }
                    Some(line_no) => {
                        trace!("Requesting match line: {} / {}", match_no, line_no);

                        self.ifile_req_sender
                            .send(crate::ifile::IFReq::GetLine {
                                id: self.id.clone(),
                                line_no: *line_no,
                            })
                            .await?;

                        client.pending.insert(*line_no);

                        Ok(())
                    }
                }
            }
            FFReq::CancelMatch { id, match_no } => {
                trace!("Cancel match: {} / {:?}", id, match_no);
                let Some(client) = self.clients.get_mut(&id) else {
                    warn!("Unknown client, ignoring request: {}", id);
                    return Ok(());
                };

                if !client.interested.remove(&match_no) {
                    warn!("Client cancelled match that was not registered for interest: client {}, line {}", id, match_no);
                }
                Ok(())
            }
            FFReq::RegisterClient { id, client_sender } => {
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

                client_sender.send(FFResp::Stats { matches: 0 }).await?;

                trace!("Finished register");
                Ok(())
            }
            FFReq::EnableTailing { id, last_seen_line } => todo!(),
            FFReq::DisableTailing { id } => todo!(),
            FFReq::SetFilter { maybe_filter } => todo!(),
        }
    }

    async fn handle_ifile_update(&mut self, update: IFResp) -> Result<()> {
        todo!()
    }
}
