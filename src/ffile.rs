use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::sync::oneshot;

use anyhow::{anyhow, Result};
use log::{debug, trace, warn};
use regex::Regex;
use tokio::select;
use tokio::sync::mpsc;

use crate::common::{LineContent, CHANNEL_BUFFER, FILTER_SPOOLING_BATCH_SIZE};
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
    ViewUpdate { update: FileResp<FilterLine> },
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

impl FilterSpec {
    pub fn render(&self) -> String {
        format!(
            "\"{}\" ({})",
            self.filter,
            match self.mode {
                FilterMode::SimpleCaseSensitive => "Sensitive",
                FilterMode::SimpleCaseInsensitive => "Insensitive",
                FilterMode::Regex => "Regex",
            }
        )
    }
}

#[derive(Debug, Clone)]
pub enum FFReqResp {
    Ok,
    Err { message: String },
}

#[derive(Debug)]
pub enum FFReq {
    SetFilter { filter_spec: Option<FilterSpec> },
}

#[derive(Debug)]
struct Client {
    id: String,
    channel: FFRespSender,
    tailing: bool,
    interested: HashSet<usize>,
}

type LineNo = usize;

struct FilterState {
    filter_spec: FilterSpec,
    filter_re: Regex,
    matches: Vec<LineNo>,
    num_matches: usize,
    line_to_match: HashMap<usize, usize>,
    next_line_expected: LineNo,
    next_line_to_request: LineNo,
}

impl FilterState {
    fn make(filter_spec: FilterSpec) -> Result<Self> {
        let filter_re = Regex::new(&filter_spec.filter)?;
        Ok(FilterState {
            filter_spec,
            filter_re,
            matches: Vec::new(),
            line_to_match: HashMap::new(),
            num_matches: 0,
            next_line_expected: 0,
            next_line_to_request: 0,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct FilterLine {
    pub line_no: usize,
    pub line: String,
}

impl LineContent for FilterLine {
    fn len(&self) -> usize {
        self.line.len()
    }

    fn render(&self) -> String {
        self.line.clone()
    }
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
    if_resp_receiver: FileRespReceiver<IFResp<String>>,
    if_resp_sender: FileRespSender<IFResp<String>>,
    if_req_sender: FileReqSender<IFResp<String>>,

    clients: HashMap<String, Client>,

    filter_state: Option<FilterState>,
}

impl FFile {
    pub fn new(id: String, path: &str, if_req_sender: FileReqSender<IFResp<String>>) -> FFile {
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
        }

        trace!("FFile finished");

        Ok(())
    }

    async fn handle_ff_command(&mut self, cmd: FFReq) -> Result<()> {
        match cmd {
            FFReq::SetFilter { filter_spec } => {
                trace!("Setting filter: {:?}", filter_spec);

                let Some(filter_spec) = filter_spec else {
                    trace!("Removing filter");
                    return self.set_filter_state(None).await;
                };

                if let Some(filter_state) = &self.filter_state {
                    if filter_state.filter_spec == filter_spec {
                        trace!("Filter unchanged, no change.");
                        return Ok(());
                    }
                }

                self.set_filter_state(Some(FilterState::make(filter_spec)?))
                    .await
            }
        }
    }

    async fn set_filter_state(&mut self, filter_state: Option<FilterState>) -> Result<()> {
        self.filter_state = filter_state;

        for (_, client) in self.clients.iter() {
            client.channel.send(FFResp::Clear).await?;
        }

        if self.filter_state.is_some() {
            self.start_spooling().await?;
        }
        return Ok(());
    }

    async fn handle_client_command(&mut self, cmd: FileReq<FFResp>) -> Result<()> {
        match cmd {
            FileReq::GetLine {
                id,
                line_no: match_no,
            } => {
                trace!("Client {} requested match {}", id, match_no);
                let Some(client) = self.clients.get_mut(&id) else {
                    warn!("Unknown client, ignoring request: {}", id);
                    return Ok(());
                };

                let Some(filter_state) = &mut self.filter_state else {
                    warn!("No current filter applied. Ignoring. {}", id);
                    return Ok(());
                };

                let maybe_line_no = filter_state.matches.get(match_no);
                match maybe_line_no {
                    None => {
                        trace!("Registering interest in: {} / {}", id, match_no);
                        client.interested.insert(match_no);
                        Ok(())
                    }
                    Some(line_no) => {
                        trace!("Requesting match line: {} / {}", line_no, match_no);

                        self.if_req_sender
                            .send(crate::ifile::FileReq::GetLine {
                                id: self.id.clone(),
                                line_no: *line_no,
                            })
                            .await?;

                        filter_state.line_to_match.insert(*line_no, match_no);

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
            FileReq::EnableTailing { id, last_seen_line } => {
                self.enable_tailing(id, last_seen_line).await
            }
            FileReq::DisableTailing { id } => self.disable_tailing(id).await,
        }
    }

    async fn disable_tailing(&mut self, id: String) -> Result<()> {
        trace!("Disable tailing: {}", id);

        let Some(client) = self.clients.get_mut(&id) else {
            warn!("Unknown client, ignoring request: {}", id);
            return Ok(());
        };

        client.tailing = false;

        Ok(())
    }

    async fn enable_tailing(&mut self, id: String, last_seen_line: usize) -> Result<()> {
        trace!("Enable tailing: {}", id);
        let Some(client) = self.clients.get_mut(&id) else {
            warn!("Unknown client, ignoring request: {}", id);
            return Ok(());
        };

        client.tailing = true;

        let Some(filter_state) = &mut self.filter_state else {
            trace!("No filter set, nothing more to do.");
            return Ok(());
        };

        // Determine which lines the client will not know about.
        for match_no in last_seen_line..filter_state.num_matches {
            let sl = filter_state.matches.get(match_no);
            if sl.is_none() {
                warn!(
                    "Unknown line whilst sending missing tailing lines: {}",
                    match_no
                );
                continue;
            };

            let Some(line_no) = filter_state.matches.get(match_no) else {
                warn!(
                    "Attempted for fetch match that does not exist: match_no: {}",
                    match_no
                );
                return Ok(());
            };

            trace!("Requesting match line: {} / {}", line_no, match_no);

            self.if_req_sender
                .send(crate::ifile::FileReq::GetLine {
                    id: self.id.clone(),
                    line_no: *line_no,
                })
                .await?;

            filter_state.line_to_match.insert(*line_no, match_no);
        }

        Ok(())
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

            filter_state.next_line_to_request += 1;
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

        if line_no != filter_state.next_line_expected {
            warn!(
                "Next spooled line {} is not expected {}",
                line_no, filter_state.next_line_expected
            );
        }

        filter_state.next_line_expected += 1;

        if filter_state.filter_re.find(&line_content).is_some() {
            trace!("Line matches...");
            // TODO: Can we be sure that the updates come in order?
            filter_state.matches.push(line_no);

            let match_no = filter_state.num_matches;
            filter_state.num_matches += 1;

            for (id, client) in self.clients.iter_mut() {
                trace!("Sending stats update to: {} - match {}", id, match_no);
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
                    trace!(
                        "Sending match to client: {} - match {}",
                        client.id,
                        match_no
                    );
                    client
                        .channel
                        .send(FFResp::ViewUpdate {
                            update: FileResp::Line {
                                line_no: match_no,
                                line_content: FilterLine {
                                    line_no,
                                    line: line_content.clone(),
                                },
                                partial,
                            },
                        })
                        .await?;
                }
            }
        } else {
            trace!("Line does not match");
        }

        self.if_req_sender
            .send(FileReq::GetLine {
                id: self.id.clone(),
                line_no: filter_state.next_line_to_request,
            })
            .await?;

        filter_state.next_line_to_request += 1;

        Ok(())
    }

    async fn handle_ifile_update(&mut self, update: IFResp<String>) -> Result<()> {
        match update {
            IFResp::ViewUpdate {
                update:
                    FileResp::Line {
                        line_no,
                        line_content,
                        partial,
                    },
            } => {
                let Some(filter_state) = &mut self.filter_state else {
                    // No current filter, so not expecting data... just ignore this.
                    trace!("Ingoring data when no filter set.");
                    return Ok(());
                };

                if line_no < filter_state.next_line_expected {
                    let Some(match_no) = filter_state.line_to_match.remove(&line_no) else {
                        trace!(
                            "Line delivered without a corresponding waiting match: {}",
                            line_no
                        );
                        return Ok(());
                    };

                    for (id, client) in self.clients.iter() {
                        trace!("Forwarding matched filter line: {} - {}", id, match_no);
                        client
                            .channel
                            .send(FFResp::ViewUpdate {
                                update: FileResp::Line {
                                    line_no: match_no,
                                    line_content: FilterLine {
                                        line_no,
                                        line: line_content.clone(),
                                    },
                                    partial,
                                },
                            })
                            .await?;
                    }
                } else {
                    self.next_spooling(line_no, line_content, partial).await?;
                }
            }
            IFResp::Truncated => {
                let Some(filter_state) = &mut self.filter_state else {
                    // No current filter, ignore truncation.
                    trace!("Ignoring truncation, no current filter.");
                    return Ok(());
                };

                self.filter_state = Some(FilterState::make(filter_state.filter_spec.clone())?);
            }
            _ => {
                trace!("Ignoring unimportant message: {:?}", update);
            }
        }

        Ok(())
    }
}
