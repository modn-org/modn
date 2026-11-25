use serde::Deserialize;
use serde_json::{json, Value};

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

    pub fn is_committed(&self) -> bool {
        self.committed
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn is_event(line: &str) -> bool {
        serde_json::from_str::<Event>(line).is_ok()
    }

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
