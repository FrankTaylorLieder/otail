#![allow(unused)]
use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, Watcher};
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
        while let Some(m) = rx.recv().await {
            println!("Received update: {:?}", m);
            // TODO: Interpret the response to show new data vs truncation.
            // TODO: Debounce events.
        }
    });

    rh.await?;

    Ok(())
}
