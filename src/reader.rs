use anyhow::Result;
use log::{error, trace};
use notify::event::{MetadataKind, ModifyKind};
use notify::{Config, Event, EventKind, RecommendedWatcher, Watcher};
use std::fs::File;
use std::path::PathBuf;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self, Receiver};

use crate::backing_file::BackingFile;

#[derive(Debug)]
pub enum ReaderUpdate {
    Line {
        line_content: String,
        offset: u64,
        line_bytes: usize,
        partial: bool,
        file_bytes: u64,
    },
    Truncated,
    FileError {
        reason: String,
    },
}

pub type ReaderUpdateSender = mpsc::Sender<ReaderUpdate>;
pub type ReaderUpdateReceiver = mpsc::Receiver<ReaderUpdate>;

pub struct Reader {}

impl Reader {
    pub async fn run(path: PathBuf, sender: ReaderUpdateSender) -> Result<()> {
        let metadata_file = File::open(&path)?;

        let mut bf = BackingFile::new(&path)?;

        trace!("Opened file: {:?}", path);

        // Start by spooling the file
        let mut pos = 0;
        let mut line = String::new();
        let mut line_bytes = 0;
        let mut partial = false;
        let mut file_lines: usize = 0;
        let mut line_offset = 0;

        trace!("Spooling file: {:?}", path);
        loop {
            if !partial {
                line.clear();
                line_bytes = 0;
                line_offset = pos;
            }

            let (bytes, partial) = bf.incremental_read(&mut line)?;

            trace!("Read line: {} @{} / {}", bytes, file_lines, line);

            if bytes == 0 {
                break;
            }

            line_bytes += bytes;
            pos += bytes as u64;

            if !partial {
                file_lines += 1;
            }

            sender
                .send(ReaderUpdate::Line {
                    // Deliver the whole line each time we send the line.
                    line_content: line.clone(),
                    offset: line_offset,
                    line_bytes,
                    partial,
                    file_bytes: pos,
                })
                .await?;
        }

        // Now tail the file.
        trace!("Tailing file: {:?} {} lines", path, file_lines);
        let (mut watcher, mut rx) = async_watcher()?;
        watcher.watch(path.as_ref(), notify::RecursiveMode::Recursive)?;

        while let Some(m) = rx.recv().await {
            match m {
                Ok(event) => {
                    // TODO: Should this be a match to only work with the cases we want?
                    if let EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)) = event.kind {
                        trace!("File truncated: {:?}", path);

                        sender.send(ReaderUpdate::Truncated).await?;

                        line.clear();
                        line_bytes = 0;
                        partial = false;
                        line_offset = 0;
                        pos = 0;

                        bf = BackingFile::new(&path)?;

                        // TODO: Test tuncation... does this properly continue reading? Or do we
                        // need to restart spooling?
                    }

                    if let EventKind::Remove(_) = event.kind {
                        trace!("File or directory removed: {:?}", path);

                        sender
                            .send(ReaderUpdate::FileError {
                                reason: "File removed".to_owned(),
                            })
                            .await?;

                        return Ok(());
                    }

                    let fmd = metadata_file.metadata()?;
                    let new_len = fmd.len();
                    if new_len == pos {
                        continue;
                    }

                    bf.seek(pos)?;

                    loop {
                        if !partial {
                            line.clear();
                            line_bytes = 0;
                            line_offset = pos;
                        }

                        let (bytes, partial) = bf.incremental_read(&mut line)?;

                        if bytes == 0 {
                            break;
                        }

                        line_bytes += bytes;
                        pos += bytes as u64;

                        sender
                            .send(ReaderUpdate::Line {
                                // Deliver the whole line each time we send the line.
                                line_content: line.clone(),
                                offset: line_offset,
                                line_bytes,
                                partial,
                                file_bytes: pos,
                            })
                            .await?;
                    }
                }
                Err(e) => {
                    let reason = format!("Watcher failed: {:?} - {:?}", path, e);
                    error!("{}", reason);
                    sender
                        .send(ReaderUpdate::FileError {
                            reason: reason.clone(),
                        })
                        .await?;

                    return Err(anyhow::anyhow!(reason));
                }
            };
        }

        Ok(())
    }
}

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (tx, rx) = mpsc::channel(1);

    let watcher = RecommendedWatcher::new(
        move |res| {
            let runtime = Runtime::new().expect("Cannot create Tokio runtime for watcher");
            let tx = tx.clone();
            runtime.block_on(async move {
                tx.send(res).await.expect("Failed to send watch event");
            });
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}
