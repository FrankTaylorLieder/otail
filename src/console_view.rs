use anyhow::Result;
use log::{debug, error, trace};
use tokio::select;

use crate::ifile::{
    ViewCommand, ViewCommandsSender, ViewUpdate, ViewUpdateReceiver, ViewUpdateSender,
};

pub struct ConsoleView {
    id: String,
    path: String,
    commands_sender: ViewCommandsSender,
    update_sender: ViewUpdateSender,
    update_receiver: ViewUpdateReceiver,

    file_lines: u32,
    file_bytes: u32,

    start_line: u32,
    num_lines: u32,
    lines: Vec<String>,
    tailing: bool,
}

impl ConsoleView {
    pub fn new(
        id: String,
        path: String,
        commands_sender: ViewCommandsSender,
        update_sender: ViewUpdateSender,
        update_receiver: ViewUpdateReceiver,
    ) -> Self {
        ConsoleView {
            id,
            path,
            commands_sender,
            update_sender,
            update_receiver,

            file_lines: 0,
            file_bytes: 0,

            start_line: 0,
            num_lines: 10,
            lines: vec![],
            tailing: false,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        debug!("Console View starting: {:?}", self.path);

        self.commands_sender
            .send(ViewCommand::RegisterUpdater {
                id: self.id.clone(),
                updater: self.update_sender.clone(),
            })
            .await?;

        // NEXT: continuously attempt to get next line we want.

        loop {
            select! {
                update = self.update_receiver.recv() => {
                    match update {
                        Some(update) => {
                            self.handle_update(update);
                        },
                        None => {
                            todo!()
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn handle_update(&mut self, update: ViewUpdate) {
        match update {
            ViewUpdate::Change {
                line_no,
                line_chars,
                line_bytes,
                file_bytes,
                partial,
            } => {
                debug!("Change: {line_no} / len: {line_chars} / file bytes: {file_bytes}");

                self.file_lines = line_no;
                self.file_bytes = file_bytes;
            }
            ViewUpdate::Line {
                line_no,
                line,
                line_chars,
                line_bytes,
                partial,
            } => {
                debug!(
                    "View line: {line_no} {} = {line}",
                    if partial { "PARTIAL" } else { "COMPLETE" }
                );
            }
            ViewUpdate::Truncated => {
                debug!("File truncated");
            }
            ViewUpdate::FileError { reason } => {
                error!("File error: {reason}");
            }
        }
    }
}
