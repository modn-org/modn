mod backend;
mod config;
mod progress;
mod protocol;
#[cfg(test)]
mod tests;
mod tools;

use anyhow::Result;
use clap::Parser;
use config::{Config, Options};
use protocol::{error, Action, Session, MESSAGE_LIMIT};
use serde_json::{json, Value};
use std::collections::VecDeque;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// Requests buffered while one call runs. The input loop keeps reading when
/// this queue is full so a cancellation always reaches the active call;
/// ordinary requests above the limit are refused, while malformed and
/// oversized lines still receive their protocol error.
const PENDING_LIMIT: usize = 4;

enum Input {
    Line(Vec<u8>),
    Oversize,
}

impl Input {
    fn cancelled_id(&self) -> Option<Value> {
        match self {
            Self::Line(bytes) => protocol::cancelled_id(bytes),
            Self::Oversize => None,
        }
    }

    fn request_id(&self) -> Option<Value> {
        match self {
            Self::Line(bytes) => protocol::request_id(bytes),
            Self::Oversize => None,
        }
    }
}

/// Removes the queued request `id` so it never starts. A cancelled request
/// gets no response.
fn remove_pending(pending: &mut VecDeque<Input>, id: &Value) -> bool {
    let Some(index) = pending
        .iter()
        .position(|input| input.request_id().as_ref() == Some(id))
    else {
        return false;
    };
    pending.remove(index);
    true
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

async fn write_message(stdout: &mut tokio::io::Stdout, response: Value) -> Result<()> {
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
    Ok(())
}

async fn run() -> Result<()> {
    let config = Config::new(Options::parse())?;
    let (sender, mut receiver) = mpsc::channel(1);
    let input = tokio::spawn(read_input(sender));
    let mut session = Session::default();
    let mut stdout = tokio::io::stdout();
    let mut pending: VecDeque<Input> = VecDeque::new();
    let mut eof = false;
    loop {
        let message = match pending.pop_front() {
            Some(message) => Some(message),
            None if eof => None,
            None => receiver.recv().await,
        };
        let Some(message) = message else {
            break;
        };
        match message {
            Input::Oversize => {
                write_message(
                    &mut stdout,
                    error(Value::Null, -32600, "Message exceeds 65536 bytes"),
                )
                .await?;
            }
            Input::Line(bytes) => match session.handle(&config, &bytes).await {
                Action::Silent => {}
                Action::Reply(response) => write_message(&mut stdout, response).await?,
                Action::Call {
                    id,
                    args,
                    progress_token,
                } => {
                    let (notifications, mut progress) = mpsc::unbounded_channel();
                    let (cancellation, signals) = mpsc::unbounded_channel();
                    let call = tools::call(&config, &args, progress_token, notifications, signals);
                    tokio::pin!(call);
                    let outcome = loop {
                        tokio::select! {
                            reply = &mut call => break reply,
                            Some(notification) = progress.recv() => {
                                write_message(&mut stdout, notification).await?;
                            }
                            incoming = receiver.recv(), if !eof => {
                                match incoming {
                                    None => eof = true,
                                    Some(input) => {
                                        if let Some(cancelled) = input.cancelled_id() {
                                            if cancelled == id {
                                                let _ = cancellation.send(());
                                            } else {
                                                remove_pending(&mut pending, &cancelled);
                                            }
                                        } else if pending.len() < PENDING_LIMIT {
                                            pending.push_back(input);
                                        } else if let Some(request_id) = input.request_id() {
                                            write_message(
                                                &mut stdout,
                                                error(
                                                    request_id,
                                                    -32000,
                                                    "Pending request limit reached",
                                                ),
                                            )
                                            .await?;
                                        } else {
                                            // A full queue must not swallow a
                                            // protocol reply: answer as the
                                            // normal path would.
                                            match input {
                                                Input::Oversize => {
                                                    write_message(
                                                        &mut stdout,
                                                        error(
                                                            Value::Null,
                                                            -32600,
                                                            "Message exceeds 65536 bytes",
                                                        ),
                                                    )
                                                    .await?;
                                                }
                                                Input::Line(bytes) => {
                                                    if let Action::Reply(response) =
                                                        session.handle(&config, &bytes).await
                                                    {
                                                        write_message(&mut stdout, response)
                                                            .await?;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    };
                    // The backend sends its final result event before it
                    // returns, so drain every queued notification before the
                    // response that supersedes it.
                    while let Ok(notification) = progress.try_recv() {
                        write_message(&mut stdout, notification).await?;
                    }
                    if let tools::Reply::Response(result) = outcome {
                        write_message(
                            &mut stdout,
                            json!({"jsonrpc":"2.0","id":id,"result":result}),
                        )
                        .await?;
                    }
                }
            },
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
