use super::{BucketPackage, VersionReq};
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
/// result is the same for the same manifests. A cycle, an ID that the
/// bucket does not publish, a release that misses its dependent's version
/// requirement, or two releases that conflict are errors, never a partial
/// order.
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
    for release in &order {
        if let Some(conflict) = order
            .iter()
            .find(|other| release.package.conflicts.contains(&other.package.id))
        {
            bail!(
                "{}/{}/{} conflicts with {}, which this installation also needs",
                release.bucket,
                release.package.game,
                release.package.id,
                conflict.package.id
            );
        }
    }
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
        if let Some(requirement) = package.package.dependency_versions.get(dependency) {
            let requirement = VersionReq::parse(requirement)?;
            if !requirement.matches(&release.package.version)? {
                bail!(
                    "{id} requires {dependency} {requirement}, but bucket {} publishes {dependency} {}",
                    package.bucket,
                    release.package.version
                );
            }
        }
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
                schema_version: 2,
                id: id.into(),
                game: "test-game".into(),
                name: id.into(),
                version: "1.0".into(),
                archive_format: Some(ArchiveFormat::Zip),
                blake3: Some("0".repeat(64)),
                sources: vec![PackageSource::Manual {
                    filename: format!("{id}.zip"),
                }],
                redistributable: false,
                upstream_sha256: None,
                dependencies: dependencies.iter().map(|id| (*id).into()).collect(),
                homepage: None,
                executables: Vec::new(),
                dependency_versions: BTreeMap::new(),
                conflicts: Vec::new(),
                native_injector: false,
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

    #[test]
    fn a_dependency_release_must_meet_its_version_requirement() {
        let mut top = release("main", "top", &["base"]);
        top.package
            .dependency_versions
            .insert("base".into(), ">=1.31.0".into());
        let mut base = release("main", "base", &[]);
        base.package.version = "1.30.2".into();
        let catalog = vec![top.clone(), base.clone()];
        let error = format!("{:#}", install_order(&top, &catalog).unwrap_err());
        assert!(
            error.contains("top requires base >=1.31.0, but bucket main publishes base 1.30.2"),
            "{error}"
        );
        base.package.version = "1.31.0".into();
        let order = install_order(&top, &[top.clone(), base]).expect("satisfied");
        assert_eq!(ids(&order), ["base", "top"]);
    }

    #[test]
    fn conflicting_releases_cannot_share_an_installation() {
        let top = release("main", "top", &["a", "b"]);
        let mut a = release("main", "a", &[]);
        a.package.conflicts.push("b".into());
        let catalog = vec![top.clone(), a, release("main", "b", &[])];
        let error = install_order(&top, &catalog).unwrap_err().to_string();
        assert!(
            error.contains("main/test-game/a conflicts with b, which this installation also needs"),
            "{error}"
        );
    }
}
