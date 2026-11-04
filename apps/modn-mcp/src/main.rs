mod backend;
mod config;
mod protocol;
#[cfg(test)]
mod tests;
mod tools;

use anyhow::Result;
use clap::Parser;
use config::{Config, Options};
use protocol::{error, Session, MESSAGE_LIMIT};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

enum Input {
    Line(Vec<u8>),
    Oversize,
}

async fn read_input(sender: mpsc::Sender<Input>) -> Result<()> {
    let mut reader = BufReader::new(tokio::io::stdin());
    let mut line = Vec::new();
    let mut oversize = false;
    loop {
        let bytes = reader.fill_buf().await?;
        if bytes.is_empty() {
            if oversize {
                let _ = sender.send(Input::Oversize).await;
            } else if !line.is_empty() {
                let _ = sender.send(Input::Line(line)).await;
            }
            return Ok(());
        }
        let end = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1);
        let consumed = end.unwrap_or(bytes.len());
        if !oversize {
            if line.len() + consumed > MESSAGE_LIMIT {
                oversize = true;
                line.clear();
            } else {
                line.extend_from_slice(&bytes[..consumed]);
            }
        }
        reader.consume(consumed);
        if end.is_some() {
            let input = if oversize {
                Input::Oversize
            } else {
                Input::Line(std::mem::take(&mut line))
            };
            if sender.send(input).await.is_err() {
                return Ok(());
            }
            oversize = false;
        }
    }
}

async fn run() -> Result<()> {
    let config = Config::new(Options::parse())?;
    let (sender, mut receiver) = mpsc::channel(1);
    let input = tokio::spawn(read_input(sender));
    let mut session = Session::default();
    let mut stdout = tokio::io::stdout();
    while let Some(message) = receiver.recv().await {
        let response = match message {
            Input::Oversize => Some(error(Value::Null, -32600, "Message exceeds 65536 bytes")),
            Input::Line(bytes) => session.handle(&config, &bytes).await,
        };
        if let Some(response) = response {
            let mut bytes = serde_json::to_vec(&response)?;
            if bytes.len() > backend::OUTPUT_LIMIT * 2 {
                bytes = serde_json::to_vec(&error(
                    response["id"].clone(),
                    -32603,
                    "Response exceeds limit",
                ))?;
            }
            bytes.push(b'\n');
            stdout.write_all(&bytes).await?;
            stdout.flush().await?;
        }
    }
    input.await??;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("modn-mcp: {error}");
        std::process::exit(1);
    }
}
