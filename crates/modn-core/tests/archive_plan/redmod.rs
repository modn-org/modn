use super::{inspect, main_bucket, plan, setup_game, tempdir, zip_fixture, PlanStatus};

#[test]
fn redmod_package_requires_info_but_not_deployment_tools() {
    main_bucket::seeded(|| {
        let dir = tempdir().unwrap();
        let game = dir.path().join("game");
        setup_game(&game, false);
        let archive = dir.path().join("redmod.zip");
        zip_fixture(
            &archive,
            &[
                ("mods/Sample/info.json", b"{}"),
                ("mods/Sample/archives/sample.archive", b"data"),
            ],
        );
        let inspection = inspect(&archive).unwrap();
        for tools_present in [false, true] {
            setup_game(&game, tools_present);
            let report = plan(&inspection, &game);
            assert_eq!(
                report.status,
                PlanStatus::Reviewable,
                "{:?}",
                report.blockers
            );
            assert!(report.blockers.is_empty());
            assert!(report.metadata_only);
            let operations: Vec<_> = report
                .operations
                .iter()
                .map(|file| {
                    (
                        file.source.as_str(),
                        file.destination.as_str(),
                        file.kind.as_str(),
                        file.bytes,
                        file.owned_directory.as_deref(),
                    )
                })
                .collect();
            assert_eq!(
                operations,
                vec![
                    (
                        "mods/Sample/archives/sample.archive",
                        "mods/Sample/archives/sample.archive",
                        "redmod",
                        4,
                        None
                    ),
                    (
                        "mods/Sample/info.json",
                        "mods/Sample/info.json",
                        "json-object",
                        2,
                        None
                    ),
                ]
            );
            assert!(!game.join("mods").exists());
        }

        let missing = dir.path().join("missing-info.zip");
        zip_fixture(&missing, &[("mods/Sample/archives/test.archive", b"data")]);
        let report = plan(&inspect(&missing).unwrap(), &game);
        assert_eq!(report.status, PlanStatus::Blocked);
        assert!(report
            .blockers
            .iter()
            .any(|issue| issue.code == "REDMOD_INFO_MISSING"));
    });
}
