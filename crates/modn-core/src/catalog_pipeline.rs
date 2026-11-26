//! Typed catalog install plans.
//!
//! A plan records what one catalog installation would place, in which order,
//! with which archives, and what still blocks it. Every value here is data:
//! the plan holds no consent, grants no approval, writes nothing and can never
//! authorize a commit. Nothing in this module prompts, touches the network or
//! runs a game module.
//!
//! The planning steps are pure. Order resolution reads one catalog snapshot;
//! archive resolution reads supplied archives, the archive cache and each
//! release's planned sources.

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "t2 calls these building blocks from plan_install")
)]
mod archives;
mod input;
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "t2 calls these building blocks from plan_install")
)]
mod order;
mod plan;

pub use input::{ConsentInputs, InstallInput, InstallTarget};
pub use plan::{
    CatalogPlan, PendingDownload, PlanConflict, PlanKind, PlanRefusal, PlannedRelease,
    ServedArchive, ServedOrigin,
};

#[cfg(test)]
mod tests;
