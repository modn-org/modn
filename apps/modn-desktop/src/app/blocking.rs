//! Runs one backend call off the UI thread.
//!
//! The job keeps its own error type. A command that calls a typed backend
//! method returns [`BackendError`](crate::backend::BackendError), so the
//! refusal code or inactivity timeout reaches the webview through its typed
//! [`CommandError`] conversion instead of being flattened into `anyhow::Error`
//! text.

use super::error::CommandError;
use super::state::Busy;
use std::sync::Arc;

/// Runs one backend call off the UI thread while holding the busy guard that
/// vetoes a window close.
pub(crate) async fn run_blocking<T, E>(
    busy: &Arc<Busy>,
    job: impl FnOnce() -> Result<T, E> + Send + 'static,
) -> Result<T, CommandError>
where
    T: Send + 'static,
    E: Into<CommandError> + Send + 'static,
{
    let _guard = busy.begin();
    match tauri::async_runtime::spawn_blocking(job).await {
        Ok(result) => result.map_err(Into::into),
        Err(_) => Err(CommandError::backend_failed()),
    }
}
