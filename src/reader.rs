use crate::backing_file::BackingFile;
use anyhow::Result;
use log::{error, trace};
use notify::{Config, Event, EventKind, RecommendedWatcher, Watcher};
use std::fs::{self, File};
use std::path::PathBuf;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self, Receiver};

use crate::backing_file::FileBackingFile;

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

        let mut bf = FileBackingFile::new(&path)?;

        trace!("Opened file: {:?}", path);

        // Start by spooling the file
        let mut pos = 0;
        let mut line = String::new();
        let mut line_bytes = 0;
        let mut previous_partial = false;
        let mut file_lines: usize = 0;
        let mut line_offset = 0;

        trace!("Spooling file: {:?}", path);
        loop {
            if !previous_partial {
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

            if !previous_partial {
                file_lines += 1;
            }

            trace!("Sending ReaderUpdate::Line (spooling) - line_bytes: {}, partial: {}, file_bytes: {}", line_bytes, partial, pos);
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

            previous_partial = partial;
        }

        // Now tail the file.
        trace!("Tailing file: {:?} {} lines", path, file_lines);
        let (mut watcher, mut rx) = async_watcher()?;
        watcher.watch(&path, notify::RecursiveMode::Recursive)?;

        trace!("Waiting to receive file system events for path: {:?}", path);
        while let Some(m) = rx.recv().await {
            trace!("Received file system event: {:?}", m);
            match m {
                Ok(event) => {
                    if let EventKind::Remove(_) = event.kind {
                        trace!("File or directory removed: {:?}", path);

                        trace!("Sending ReaderUpdate::FileError - reason: File removed");
                        sender
                            .send(ReaderUpdate::FileError {
                                reason: "File removed".to_owned(),
                            })
                            .await?;

                        return Ok(());
                    }

                    let new_size = fs::metadata(&path)?.len();

                    if new_size < pos {
                        // TODO: Is there a way to detect file truncation where the new content is
                        // longer than the old content?
                        trace!(
                            "File truncated: {:?}, old size: {}, new size: {}",
                            path,
                            pos,
                            new_size
                        );

                        trace!("Sending ReaderUpdate::Truncated - old_size: {}, new_size: {}", pos, new_size);
                        sender.send(ReaderUpdate::Truncated).await?;

                        line.clear();
                        line_bytes = 0;
                        previous_partial = false;
                        line_offset = 0;
                        pos = 0;

                        bf = FileBackingFile::new(&path)?;
                    }

                    let fmd = metadata_file.metadata()?;
                    let new_len = fmd.len();
                    if new_len == pos {
                        continue;
                    }

                    bf.seek(pos)?;

                    loop {
                        if !previous_partial {
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

                        trace!("Sending ReaderUpdate::Line (tailing) - line_bytes: {}, partial: {}, file_bytes: {}, content_preview: {:?}", line_bytes, partial, pos, line.chars().take(50).collect::<String>());
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

                        previous_partial = partial;
                    }
                }
                Err(e) => {
                    let reason = format!("Watcher failed: {:?} - {:?}", path, e);
                    error!("{}", reason);
                    trace!("Sending ReaderUpdate::FileError - reason: {}", reason);
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
                trace!("Forwarding watch event: {:?}", res);
                tx.send(res).await.expect("Failed to send watch event");
            });
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}
