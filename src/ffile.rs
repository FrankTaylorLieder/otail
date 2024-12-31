use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use log::{debug, trace, warn};
use regex::Regex;
use tokio::select;
use tokio::sync::mpsc;

use crate::common::{CHANNEL_BUFFER, FILTER_SPOOLING_BATCH_SIZE};
use crate::ifile::{
    FileReq, FileReqReceiver, FileReqSender, FileResp, FileRespReceiver, FileRespSender, IFResp,
};

pub type FFRespSender = mpsc::Sender<FFResp>;
pub type FFRespReceiver = mpsc::Receiver<FFResp>;

#[derive(Debug)]
pub enum FFResp {
    ViewUpdate { update: FileResp },
    Clear,
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
    filter_text: String,
    filter_re: Regex,
    matches: Vec<LineNo>,
    next_line: LineNo,
}

pub struct FFile {
    id: String,
    path: PathBuf,

    view_receiver: FileReqReceiver<FFResp>,
    view_sender: FileReqSender<FFResp>,
    ifile_resp_receiver: FileRespReceiver<IFResp>,
    ifile_resp_sender: FileRespSender<IFResp>,
    ifile_req_sender: FileReqSender<IFResp>,
    clients: HashMap<String, Client>,

    filter_state: Option<FilterState>,
}

impl FFile {
    pub fn new(id: String, path: &str, ifile_req_sender: FileReqSender<IFResp>) -> FFile {
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

            clients: HashMap::new(),

            filter_state: None,
        }
    }

    pub fn get_view_sender(&self) -> FileReqSender<FFResp> {
        self.view_sender.clone()
    }

    pub async fn run(&mut self) -> Result<()> {
        debug!("Ffile starting: {:?}", self.path);

        self.ifile_req_sender
            .send(crate::ifile::FileReq::RegisterClient {
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

                        self.ifile_req_sender
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
            // FileReq::SetFilter { id, maybe_filter } => {
            //     trace!("Setting filter: {}, filter: {:?}", id, maybe_filter);
            //     let Some(client) = self.clients.get_mut(&id) else {
            //         warn!("Unknown client, ignoring request: {}", id);
            //         return Ok(());
            //     };
            //
            //     let Some(filter_text) = maybe_filter else {
            //         trace!("Removing filter: {}", id);
            //         self.filter_state = None;
            //         client.channel.send(FFResp::Clear).await?;
            //         return Ok(());
            //     };
            //
            //     if let Some(filter_state) = &self.filter_state {
            //         if filter_state.filter_text == filter_text {
            //             trace!("Filter unchanged, no change.");
            //             return Ok(());
            //         }
            //     }
            //
            //     // TODO: Return errors of the filter is malformed
            //     let filter_re = Regex::new(&filter_text)?;
            //
            //     self.filter_state = Some(FilterState {
            //         filter_text,
            //         filter_re,
            //         matches: Vec::new(),
            //         next_line: 0,
            //     });
            //
            //     client.channel.send(FFResp::Clear).await?;
            //
            //     self.start_spooling().await?;
            //
            //     Ok(())
            // }
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
            self.ifile_req_sender
                .send(FileReq::GetLine {
                    id: self.id.clone(),
                    line_no: i,
                })
                .await?;

            filter_state.next_line += 1;
        }

        Ok(())
    }

    async fn next_spooling(&mut self, line_no: LineNo, line_content: String) -> Result<()> {
        trace!("Next spooling: {} / {}", self.id, line_no);
        let Some(filter_state) = &mut self.filter_state else {
            trace!("Not spooling, ignore line. {} / {}", self.id, line_no);
            return Ok(());
        };

        if filter_state.filter_re.find(&line_content).is_some() {
            trace!("Line matches...");
            // TODO: Can we be sure that the updates come in order?
            filter_state.matches.push(line_no);
        }

        self.ifile_req_sender
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
                        line_chars,
                        line_bytes,
                        partial,
                    },
            } => {
                self.next_spooling(line_no, line_content).await?;
            }
            _ => {
                trace!("Ignoring unimportant message: {:?}", update);
            }
        }

        Ok(())
    }
}
