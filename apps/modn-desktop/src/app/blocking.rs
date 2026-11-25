use super::error::CommandError;
use super::state::Busy;
use std::sync::Arc;

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
