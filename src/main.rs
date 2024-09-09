use anyhow::Result;
use automerge::{Automerge, ReadDoc};
use clap::Parser;
use iroh::node::{Node, ProtocolHandler};
use protocol::IrohAutomergeProtocol;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, Duration};
use tracing::error;

mod protocol;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[clap(long)]
    remote_id: Option<iroh::net::NodeId>,
}

fn print_document_state(doc: &Automerge) -> Result<()> {
    println!("Current Document State:");
    let keys: Vec<_> = doc.keys(automerge::ROOT).collect();
    for key in keys {
        match doc.get(automerge::ROOT, &key)? {
            Some((value, _)) => println!("  {} = {}", key, value),
            None => println!("  {} = <no value>", key),
        }
    }
    println!();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let opts = Cli::parse();

    let (sync_sender, mut sync_finished) = mpsc::channel(10);
    let (doc_update_tx, mut doc_update_rx) = watch::channel(Automerge::new());
    let (automerge, mut user_input_rx) =
        IrohAutomergeProtocol::new(Automerge::new(), sync_sender, doc_update_tx);
    let iroh = Node::memory()
        .disable_docs()
        .build()
        .await?
        .accept(
            IrohAutomergeProtocol::ALPN,
            Arc::clone(&automerge) as Arc<dyn ProtocolHandler>,
        )
        .spawn()
        .await?;

    let addr = iroh.net().node_addr().await?;
    println!("Node started with ID: {}", addr.node_id);

    if let Some(remote_id) = opts.remote_id {
        println!("Connecting to remote node: {}", remote_id);
        let node_addr = iroh::net::NodeAddr::new(remote_id);
        match iroh
            .endpoint()
            .connect(node_addr, IrohAutomergeProtocol::ALPN)
            .await
        {
            Ok(conn) => {
                println!("Connected successfully to remote node");
                let (send, recv) = conn.open_bi().await?;
                tokio::spawn(Arc::clone(&automerge).handle_connection(send, recv));
            }
            Err(e) => {
                error!("Failed to connect: {:?}", e);
            }
        }
    } else {
        println!("Waiting for incoming connections...");
    }

    println!("Enter key-value pairs in the format 'key = value' to update the document.");
    println!("The document state will be displayed after each update.");

    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut input_line = String::new();

    // 定期的にドキュメントの状態を確認するタスク
    let mut doc_update_rx_clone = doc_update_rx.clone();
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            let doc = doc_update_rx_clone.borrow_and_update();
            if let Err(e) = print_document_state(&*doc) {
                eprintln!("Error printing document state: {:?}", e);
            }
        }
    });
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("Shutting down...");
                break;
            }
            doc = sync_finished.recv() => {
                if let Some(doc) = doc {
                    println!("\nDocument synchronized:");
                    if let Err(e) = print_document_state(&doc) {
                        eprintln!("Error printing document state: {:?}", e);
                    }
                }
            }
            result = stdin.read_line(&mut input_line) => {
                if let Ok(0) = result {
                    // EOF reached
                    break;
                }
                if let Ok(_) = result {
                    let line = input_line.trim();
                    let parts: Vec<&str> = line.split('=').collect();
                    if parts.len() == 2 {
                        let key = parts[0].trim().to_string();
                        let value = parts[1].trim().to_string();
                        println!("Sending update: {} = {}", key, value);
                        automerge.send_user_input(key, value).await?;
                    } else {
                        println!("Invalid input. Please use format: key = value");
                    }
                    input_line.clear();
                }
            }
            Ok((key, value)) = user_input_rx.recv() => {
                println!("\nReceived update from remote node: {} = {}", key, value);
                let doc = doc_update_rx.borrow_and_update();
                if let Err(e) = print_document_state(&*doc) {
                    eprintln!("Error printing document state: {:?}", e);
                }
            }
        }
    }

    iroh.shutdown().await?;
    Ok(())
}
