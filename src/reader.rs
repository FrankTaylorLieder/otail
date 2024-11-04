use anyhow::Result;
use log::{debug, trace};
use notify::event::{MetadataKind, ModifyKind};
use notify::{Config, Event, EventKind, RecommendedWatcher, Watcher};
use std::io::{BufRead, BufReader, Seek};
use std::path::PathBuf;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self, Receiver};

use crate::ifile::{ReaderUpdate, ReaderUpdateSender, ViewCommand, ViewCommandsSender};

pub struct Reader {}

impl Reader {
    pub async fn run(path: PathBuf, sender: ReaderUpdateSender) -> Result<()> {
        let mut f = std::fs::File::open(&path).unwrap();
        let mut br = BufReader::new(std::fs::File::open(&path).unwrap());
        let mut pos = 0;

        debug!("Opened file: {:?}", path);

        // Start by spooling the file
        trace!("Spooling file: {:?}", path);
        let mut line = String::new();
        let mut line_bytes = 0;
        let mut partial = false;
        loop {
            if !partial {
                line.clear();
                line_bytes = 0;
            }

            // TODO: Remove unwrap
            let len = br.read_line(&mut line).unwrap();

            trace!("Read line: {}/{}", len, line);

            if len == 0 {
                break;
            }

            line_bytes += len as u32;
            pos += len as u64;
            partial = !line.as_str().ends_with('\n');

            // TODO: Also check for '\r\n'
            trace!("Sending update...");
            sender
                .send(ReaderUpdate::Line {
                    // Deliver the whole line each time we send the line.
                    line: line.clone(),
                    line_bytes,
                    partial,
                    file_bytes: pos as u32,
                })
                .await;
        }

        // Now tail the file.
        trace!("Starting tail: {:?}", path);
        let (mut watcher, mut rx) = async_watcher()?;
        watcher.watch(path.as_ref(), notify::RecursiveMode::Recursive);

        while let Some(m) = rx.recv().await {
            trace!("Received update: {:#?}", m);

            match m {
                Ok(event) => {
                    // TODO: Should this be a match to only work with the cases we want?
                    if let EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)) = event.kind {
                        debug!("File truncated... reloading");

                        // TODO: Remove unwrap
                        br = BufReader::new(std::fs::File::open(&path).unwrap());
                        pos = 0;

                        sender.send(ReaderUpdate::Truncated).await;

                        continue;
                    }

                    // TODO: Remove unwrap
                    let fmd = f.metadata().unwrap();
                    let new_len = fmd.len();
                    trace!("New length: {}", new_len);
                    if new_len == pos {
                        trace!("File not modified");
                        continue;
                    }

                    trace!("Seek to: {}", pos);
                    // TODO: Remove unwrap
                    br.seek(std::io::SeekFrom::Start(pos)).unwrap();

                    loop {
                        if !partial {
                            line.clear();
                            line_bytes = 0;
                        }

                        // TODO: Remove unwrap
                        let len = br.read_line(&mut line).unwrap();

                        trace!("Read line: {} / {}", line, len);

                        if len == 0 {
                            break;
                        }

                        line_bytes += len as u32;
                        pos += len as u64;

                        // TODO: Also check for '\r\n'
                        partial = !line.as_str().ends_with('\n');

                        // TODO: Send message to consumer
                        trace!(
                            "Next line: {} {}",
                            if partial { "PARTIAL" } else { "COMPLETE" },
                            line
                        );

                        sender
                            .send(ReaderUpdate::Line {
                                // Deliver the whole line each time we send the line.
                                line: line.clone(),
                                line_bytes,
                                partial,
                                file_bytes: pos as u32,
                            })
                            .await;
                    }
                }
                Err(e) => {
                    println!("Error: {:?}", e);
                }
            };

            // TODO: Debounce events?
        }

        Ok(())
    }
}

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (tx, mut rx) = mpsc::channel(1);

    // TODO: Remove unwrap
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            // TODO: Remove unwrap
            let runtime = Runtime::new().unwrap();
            let tx = tx.clone();
            runtime.block_on(async move {
                // TODO: Remove unwrap
                tx.send(res).await.unwrap();
            });
        },
        Config::default(),
    )
    .unwrap();

    Ok((watcher, rx))
}
