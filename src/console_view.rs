use anyhow::Result;
use log::{debug, error};
use tokio::select;

use crate::ifile::{ViewCommandsSender, ViewUpdate, ViewUpdateReceiver, ViewUpdateSender};

pub struct ConsoleView {
    path: String,
    commands_sender: ViewCommandsSender,
    update_sender: ViewUpdateSender,
    update_receiver: ViewUpdateReceiver,
    tailing: bool,
}

impl ConsoleView {
    pub fn new(
        path: String,
        commands_sender: ViewCommandsSender,
        update_sender: ViewUpdateSender,
        update_receiver: ViewUpdateReceiver,
    ) -> Self {
        ConsoleView {
            path,
            commands_sender,
            update_sender,
            update_receiver,
            tailing: false,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        debug!("Console View starting: {:?}", self.path);

        self.commands_sender
            .send(crate::ifile::ViewCommand::RegisterUpdates {
                updater: self.update_sender.clone(),
            })
            .await?;

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
            ViewUpdate::Line {
                line_no,
                line,
                line_bytes,
                partial,
                file_bytes,
            } => {
                debug!("View line: {line_no} = {line} ({file_bytes})");
            }
            ViewUpdate::Truncated => {
                todo!()
            }
            ViewUpdate::FileError { reason } => {
                error!("File error: {reason}");
            }
        }
    }
}
