#![allow(unused)]
use std::io::{BufRead, BufReader, Seek};

use anyhow::Result;
use notify::event::{MetadataKind, ModifyKind};
use notify::{Config, Event, EventKind, RecommendedWatcher, Watcher};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self, Receiver};

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (tx, mut rx) = mpsc::channel(1);

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let runtime = Runtime::new().unwrap();
            let tx = tx.clone();
            runtime.block_on(async move {
                tx.send(res).await.unwrap();
            });
        },
        Config::default(),
    )
    .unwrap();

    Ok((watcher, rx))
}

#[tokio::main]
pub async fn main() -> Result<()> {
    let path = std::env::args().nth(1).expect("Missing filename");

    let (mut watcher, mut rx) = async_watcher()?;
    watcher.watch(path.as_ref(), notify::RecursiveMode::Recursive);

    let rh = tokio::spawn(async move {
        let mut f = std::fs::File::open(&path).unwrap();
        let mut br = BufReader::new(std::fs::File::open(&path).unwrap());
        let mut pos = std::fs::metadata(&path).unwrap().len();

        println!("Opened file: {}, len: {}", path, pos);

        let mut partial = false;
        while let Some(m) = rx.recv().await {
            println!("Received update: {:#?}", m);

            match m {
                Ok(event) => {
                    if let EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)) = event.kind {
                        println!("File truncated... reloading");

                        br = BufReader::new(std::fs::File::open(&path).unwrap());
                        pos = 0;
                        continue;
                    }

                    let fmd = f.metadata().unwrap();
                    let new_len = fmd.len();
                    println!("New length: {}", new_len);
                    if new_len == pos {
                        continue;
                    }

                    br.seek(std::io::SeekFrom::Start(pos)).unwrap();

                    let mut line = String::new();
                    loop {
                        let len = br.read_line(&mut line).unwrap();

                        if len == 0 {
                            break;
                        }

                        pos += len as u64;

                        if !line.as_str().ends_with('\n') {
                            partial = true;
                        } else {
                            partial = false;
                        }
                        println!(
                            "Next line: {} {}",
                            if partial { "PARTIAL" } else { "COMPLETE" },
                            line
                        );

                        line.clear();
                    }
                }
                Err(e) => {
                    println!("Error: {:?}", e);
                }
            };

            // TODO: Debounce events?
        }
    });

    rh.await?;

    Ok(())
}
