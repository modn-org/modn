//! Translation of CLI progress events into MCP progress notifications.
//!
//! The companion always starts the CLI with `--progress json`, so it can also
//! observe the commit point when the client sends no progress token. A token
//! controls only whether a notification is emitted.

use serde::Deserialize;
use serde_json::{json, Value};

/// One JSON line that the CLI writes to stderr under `--progress json`.
#[derive(Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum Event {
    Resolve,
    Download {
        #[serde(default)]
        package: Option<String>,
        bytes: u64,
        total: Option<u64>,
    },
    Extract {
        #[serde(default)]
        package: Option<String>,
    },
    Stage {
        #[serde(default)]
        package: Option<String>,
    },
    Commit {
        #[serde(default)]
        package: Option<String>,
    },
    Result {
        outcome: Outcome,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
}

/// Tracks phases and builds one notification per observable phase.
pub struct Reporter {
    token: Option<Value>,
    base: u64,
    last: u64,
    committed: bool,
    cancelled: bool,
}

impl Reporter {
    pub fn new(token: Option<Value>) -> Self {
        Self {
            token,
            base: 0,
            last: 0,
            committed: false,
            cancelled: false,
        }
    }

    /// Whether the CLI reported the commit point.
    pub fn is_committed(&self) -> bool {
        self.committed
    }

    /// Whether the CLI reported a cancelled result.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Parses one stderr line and returns its notification, when a token was
    /// requested. Phase and commit state advance with or without a token.
    pub fn event(&mut self, line: &str) -> Option<Value> {
        let event: Event = serde_json::from_str(line).ok()?;
        if matches!(event, Event::Commit { .. }) {
            self.committed = true;
        }
        if let Event::Result { outcome } = &event {
            if matches!(outcome, Outcome::Cancelled) {
                self.cancelled = true;
            }
        }
        let (progress, total) = self.advance(&event);
        let message = message(&event);
        let token = self.token.clone()?;
        let mut params = json!({
            "progressToken": token,
            "progress": progress,
            "message": message,
        });
        if let Some(total) = total {
            params["total"] = json!(total);
        }
        Some(json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": params,
        }))
    }

    /// Returns the next strictly increasing progress value and an optional
    /// total. Download bytes share one scale with the phases around them, so
    /// a later phase never reports a smaller value than the download.
    fn advance(&mut self, event: &Event) -> (u64, Option<u64>) {
        match event {
            Event::Download { bytes, total, .. } => {
                let progress = (self.base + bytes).max(self.last + 1);
                let total = total
                    .and_then(|value| self.base.checked_add(value))
                    .filter(|value| *value >= progress);
                self.last = progress;
                (progress, total)
            }
            _ => {
                self.base = self.last;
                self.last += 1;
                (self.last, None)
            }
        }
    }
}

fn message(event: &Event) -> String {
    let named = |verb: &str, package: &Option<String>| match package {
        Some(package) => format!("{verb} {package}"),
        None => verb.to_owned(),
    };
    match event {
        Event::Resolve => "resolving".to_owned(),
        Event::Download { package, .. } => named("downloading", package),
        Event::Extract { package } => named("extracting", package),
        Event::Stage { package } => named("staging", package),
        Event::Commit { package } => named("committing", package),
        Event::Result { outcome } => match outcome {
            Outcome::Succeeded => "succeeded".to_owned(),
            Outcome::Failed => "failed".to_owned(),
            Outcome::Cancelled => "cancelled".to_owned(),
        },
    }
}
