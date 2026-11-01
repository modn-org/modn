//! Deterministic install order for a mod release and its declared dependencies.
//!
//! Resolution reads package manifests only. It never decides readiness or
//! authorizes writes; callers check installed state separately.
use super::BucketPackage;
use anyhow::{bail, Result};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    InProgress,
    Done,
}

/// Return `selected` and every release it depends on, dependencies first.
///
/// Dependencies resolve within the selected package's bucket and game, as
/// catalog installation does. Siblings are visited in declared order, so the
/// result is the same for the same manifests. A cycle or an ID that the
/// bucket does not publish is an error, never a partial order.
pub fn install_order(
    selected: &BucketPackage,
    catalog: &[BucketPackage],
) -> Result<Vec<BucketPackage>> {
    let releases: BTreeMap<&str, &BucketPackage> = catalog
        .iter()
        .filter(|candidate| {
            candidate.bucket == selected.bucket && candidate.package.game == selected.package.game
        })
        .map(|candidate| (candidate.package.id.as_str(), candidate))
        .collect();
    let mut visits = BTreeMap::new();
    let mut path = Vec::new();
    let mut order = Vec::new();
    visit(selected, &releases, &mut visits, &mut path, &mut order)?;
    Ok(order)
}

fn visit<'a>(
    package: &'a BucketPackage,
    releases: &BTreeMap<&str, &'a BucketPackage>,
    visits: &mut BTreeMap<&'a str, Visit>,
    path: &mut Vec<&'a str>,
    order: &mut Vec<BucketPackage>,
) -> Result<()> {
    let id = package.package.id.as_str();
    match visits.get(id) {
        Some(Visit::Done) => return Ok(()),
        Some(Visit::InProgress) => {
            let start = path
                .iter()
                .position(|step| *step == id)
                .expect("an in-progress release is on the current path");
            let mut cycle = path[start..].to_vec();
            cycle.push(id);
            bail!(
                "dependency cycle in {}/{}: {}",
                package.bucket,
                package.package.game,
                cycle.join(" -> ")
            );
        }
        None => {}
    }
    visits.insert(id, Visit::InProgress);
    path.push(id);
    for dependency in &package.package.dependencies {
        let Some(release) = releases.get(dependency.as_str()) else {
            bail!(
                "dependency unavailable in bucket: {}/{}/{} (required by {id})",
                package.bucket,
                package.package.game,
                dependency
            );
        };
        visit(release, releases, visits, path, order)?;
    }
    path.pop();
    visits.insert(id, Visit::Done);
    order.push(package.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buckets::{ArchiveFormat, PackageManifest, PackageSource};

    fn release(bucket: &str, id: &str, dependencies: &[&str]) -> BucketPackage {
        BucketPackage {
            bucket: bucket.into(),
            package: PackageManifest {
                schema_version: 1,
                id: id.into(),
                game: "test-game".into(),
                name: id.into(),
                version: "1.0".into(),
                archive_format: ArchiveFormat::Zip,
                sha256: "0".repeat(64),
                source: PackageSource::Manual {
                    filename: format!("{id}.zip"),
                },
                dependencies: dependencies.iter().map(|id| (*id).into()).collect(),
                homepage: None,
                executables: Vec::new(),
            },
        }
    }

    fn ids(order: &[BucketPackage]) -> Vec<&str> {
        order.iter().map(|p| p.package.id.as_str()).collect()
    }

    #[test]
    fn shared_dependencies_appear_once_before_every_dependent() {
        let catalog = vec![
            release("main", "top", &["settings", "xl", "base"]),
            release("main", "settings", &["xl", "base"]),
            release("main", "xl", &["base"]),
            release("main", "base", &[]),
            release("main", "unrelated", &[]),
        ];
        let order = install_order(&catalog[0], &catalog).expect("order");
        assert_eq!(ids(&order), ["base", "xl", "settings", "top"]);
    }

    #[test]
    fn dependencies_resolve_only_in_the_selected_bucket() {
        let catalog = vec![
            release("community", "top", &["base"]),
            release("main", "base", &[]),
        ];
        let error = install_order(&catalog[0], &catalog).expect_err("other bucket");
        assert!(
            error
                .to_string()
                .contains("dependency unavailable in bucket: community/test-game/base"),
            "{error}"
        );
    }

    #[test]
    fn a_cycle_names_its_path_instead_of_returning_an_order() {
        let catalog = vec![
            release("main", "top", &["a"]),
            release("main", "a", &["b"]),
            release("main", "b", &["a"]),
        ];
        let error = install_order(&catalog[0], &catalog).expect_err("cycle");
        assert!(
            error
                .to_string()
                .contains("dependency cycle in main/test-game: a -> b -> a"),
            "{error}"
        );
    }
}
