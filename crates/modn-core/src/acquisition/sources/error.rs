//! One error that names every artifact source that failed.

use super::SourceAttempt;
use std::fmt;

/// Every source failed; the message names each one and why.
#[derive(Debug)]
pub struct SourcesError {
    pub(super) reference: String,
    pub(super) attempts: Vec<SourceAttempt>,
}

impl fmt::Display for SourcesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "no source supplied the archive for {}; every source that was tried failed:",
            self.reference
        )?;
        let mut index = 0;
        while index < self.attempts.len() {
            let source = &self.attempts[index].source;
            let mut end = index + 1;
            while end < self.attempts.len() && &self.attempts[end].source == source {
                end += 1;
            }
            let count = end - index;
            let last = &self.attempts[end - 1];
            let reason = last
                .failure
                .as_ref()
                .map_or_else(|| "unknown".to_owned(), ToString::to_string);
            if count == 1 {
                writeln!(f, "  download failed: {source}: {reason}")?;
            } else {
                writeln!(
                    f,
                    "  download failed: {source}: {reason} ({count} attempts)"
                )?;
            }
            index = end;
        }
        Ok(())
    }
}

impl std::error::Error for SourcesError {}
