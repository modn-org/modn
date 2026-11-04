use crate::backend::Review;
use std::fmt::Write;

pub(crate) fn review_text(review: &Review) -> String {
    let mut text = format!(
        "Archive: {}\nInstallation: {}\n\n",
        review.source.display(),
        review.game.path.display()
    );
    if let Some(operations) = review.plan["operations"].as_array() {
        let _ = writeln!(text, "Proposed files ({})", operations.len());
        for operation in operations {
            let source = operation["source"].as_str().unwrap_or("(not supplied)");
            let destination = operation["destination"]
                .as_str()
                .unwrap_or("(not supplied)");
            let bytes = operation["bytes"].as_u64().unwrap_or_default();
            let _ = writeln!(text, "\n{source}\n  -> {destination} ({bytes} bytes)");
        }
    }
    if let Some(blockers) = review.plan["blockers"].as_array() {
        let _ = writeln!(text, "\nBlockers ({})", blockers.len());
        for blocker in blockers {
            let message = blocker["message"]
                .as_str()
                .unwrap_or("Backend supplied no explanation");
            let _ = writeln!(text, "\n{message}");
        }
        if blockers.is_empty() {
            text.push_str("\nNo blockers reported. Review the destinations before confirming.\n");
        }
    }
    text.push_str("\nThis preview reads archive metadata. It does not install files or certify game compatibility.");
    text
}
