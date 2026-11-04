use crate::planning::InstallPlan;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

pub(super) fn validate_staged_content(plan: &InstallPlan, staging: &Path) -> Result<()> {
    for (ordinal, operation) in plan.operations.iter().enumerate() {
        if operation.kind != "json-object" {
            continue;
        }
        if operation.bytes == 0 || operation.bytes > 65_536 {
            bail!("JSON object payload must contain 1 through 65536 bytes");
        }
        let path = staging.join(format!("{ordinal}.bin"));
        let bytes = fs::read(&path)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid JSON object payload: {}", operation.source))?;
        if !value.is_object() {
            bail!("JSON payload must be an object: {}", operation.source);
        }
    }
    Ok(())
}
