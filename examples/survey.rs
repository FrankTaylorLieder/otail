#![allow(unused)]

use anyhow::Result;
use log::{debug, error, info, trace};
use std::path::PathBuf;
use std::time::Duration;
use std::{thread, usize};
use tokio::fs::File;
use tokio::sync::{mpsc, oneshot};

pub type CommandsSender = mpsc::Sender<IFileCommand>;
pub type CommandsReceiver = mpsc::Receiver<IFileCommand>;
pub type ResultResponder<T> = oneshot::Sender<T>;

#[derive(Debug)]
pub enum IFileCommand {
    GetLine {
        index: u32,
        resp: ResultResponder<Option<String>>,
    },
    Tail {
        index: u32,
        resp: ResultResponder<String>,
    },
    AddLine {
        line: String,
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
    path: PathBuf,
    file: Option<File>,
    lines: Vec<SLine>,
    line_count: u32,
    byte_count: u32,
    tailers: Vec<ResultResponder<String>>,
}

impl IFile {
    pub fn new(path: &str, commands_receiver: CommandsReceiver) -> IFile {
        let mut pb = PathBuf::new();
        pb.push(path);

        IFile {
            path: pb,
            commands_receiver,
            file: None,
            lines: vec![],
            line_count: 0,
            byte_count: 0,
            tailers: vec![],
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        debug!("Ifile starting: {:?}", self.path);
        while let Some(cmd) = self.commands_receiver.recv().await {
            trace!("IFile received command: {:?}", cmd);

            match cmd {
                IFileCommand::GetLine { index, resp } => {
                    trace!("Getting line: {}", index);
                    let sl = self.lines.get(index as usize);
                    resp.send(sl.map(|sl| sl.content.clone()));
                }
                IFileCommand::Tail { index, resp } => {
                    trace!("Tail: {}", index);
                    let sl = self.lines.get(index as usize);
                    let Some(sl) = sl else {
                        trace!("Waiting for next line...");
                        self.tailers.push(resp);
                        continue;
                    };
                    resp.send(sl.content.clone());
                }
                IFileCommand::AddLine { line } => {
                    trace!("Adding line: {:?}", self.line_count);
                    let len = line.as_bytes().len() as u32;
                    self.lines.push(SLine {
                        content: line.clone(),
                        index: self.line_count,
                        length: len,
                    });

                    self.line_count += 1;
                    self.byte_count += len;

                    while let Some(t) = self.tailers.pop() {
                        trace!("Sending line to tailer");
                        t.send(line.clone()).unwrap();
                    }
                }
            }
        }

        Ok(())
    }
}

#[tokio::main]
pub async fn main() -> Result<()> {
    env_logger::init();

    info!("Survey starting...");

    let (sender_shared, mut rx) = mpsc::channel(100);
    let mut ifile = IFile::new("test.data", rx);

    let ifh = tokio::spawn(async move {
        ifile.run().await.unwrap();
    });

    let sender = sender_shared.clone();
    let ch = tokio::spawn(async move {
        trace!("Client starting...");

        let mut index = 0;
        loop {
            let (resp, rx) = oneshot::channel();
            sender
                .send(IFileCommand::Tail { index, resp })
                .await
                .unwrap();

            let line = rx.await.unwrap();

            debug!("Received line {}: {}", index, line);
            index += 1;
        }
    });

    let sender = sender_shared.clone();
    let rh = tokio::spawn(async move {
        debug!("Reader starting...");

        let mut index = 0;
        loop {
            sender
                .send(IFileCommand::AddLine {
                    line: format!("Generated line {}", index),
                })
                .await
                .unwrap();
            index += 1;

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    ch.await?;
    rh.await?;
    ifh.await?;

    Ok(())
}
