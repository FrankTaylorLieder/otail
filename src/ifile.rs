use anyhow::Result;
use log::{debug, error, info, trace};
use std::path::PathBuf;
use std::time::Duration;
use std::{thread, usize};
use tokio::fs::File;
use tokio::sync::{mpsc, oneshot};

use crate::reader::Reader;

pub type CommandsSender = mpsc::Sender<IFileCommand>;
pub type CommandsReceiver = mpsc::Receiver<IFileCommand>;
pub type ResultResponder<T> = oneshot::Sender<T>;

// TODO: Split these commands into Reader and Consumer commands.
#[derive(Debug)]
pub enum IFileCommand {
    GetLine {
        index: u32,
        resp: ResultResponder<Option<String>>,
    },
    RegisterTail {
        index: u32,
        resp: ResultResponder<String>,
    },
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
    commands_receiver: CommandsReceiver,
    commands_sender: CommandsSender,
    path: PathBuf,
    lines: Vec<SLine>,
    line_count: u32,
    byte_count: u32,
    tailers: Vec<ResultResponder<String>>,
}

impl IFile {
    pub fn new(path: &str) -> IFile {
        let mut pb = PathBuf::new();
        pb.push(path);

        let (commands_sender, commands_receiver) = mpsc::channel(10);

        IFile {
            path: pb,
            commands_receiver,
            commands_sender,
            lines: vec![],
            line_count: 0,
            byte_count: 0,
            tailers: vec![],
        }
    }

    fn run_reader(&mut self, cs: CommandsSender) {
        let cs = cs.clone();
        let path = self.path.clone();
        tokio::spawn(async move {
            Reader::run(path, cs).await;
        });
    }

    pub async fn run(&mut self) -> Result<()> {
        debug!("Ifile starting: {:?}", self.path);

        self.run_reader(self.commands_sender.clone());

        trace!("Waiting on commands");

        // TODO: Have different channels for reader and command input... use select! to handle
        // them.
        while let Some(cmd) = self.commands_receiver.recv().await {
            trace!("IFile received command: {:?}", cmd);

            match cmd {
                IFileCommand::GetLine { index, resp } => {
                    trace!("Getting line: {}", index);
                    let sl = self.lines.get(index as usize);
                    resp.send(sl.map(|sl| sl.content.clone()));
                }
                IFileCommand::RegisterTail { index, resp } => {
                    trace!("Tail: {}", index);
                    let sl = self.lines.get(index as usize);
                    let Some(sl) = sl else {
                        trace!("Waiting for next line...");
                        self.tailers.push(resp);
                        continue;
                    };
                    resp.send(sl.content.clone());
                }
                IFileCommand::Line {
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

                    while let Some(t) = self.tailers.pop() {
                        trace!("Sending line to tailer");
                        t.send(line.clone()).unwrap();
                    }
                }
                IFileCommand::Truncated => {
                    self.lines.clear();
                    self.line_count = 0;
                    self.byte_count = 0;

                    // TODO: Inform tailers we've been truncated.
                }
                IFileCommand::FileError { reason } => {
                    error!("File error: {}", reason);
                    return Err(anyhow::anyhow!("File error: {}", reason));
                }
            }
        }

        trace!("IFile finished");

        Ok(())
    }
}
