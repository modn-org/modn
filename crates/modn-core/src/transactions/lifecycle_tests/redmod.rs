use super::Fixture;
use crate::planning::{preview_install_zip, PlanStatus};
use crate::transactions::AdoptionApproval;
use std::fs;

#[test]
fn redmod_lifecycle_reports_individual_files_as_managed() {
    crate::test_support::seeded_roots(|roots| {
        let mut fixture = Fixture::new(roots);
        let archive = fixture.archive(
            "redmod.zip",
            &[
                (
                    "Wrapper/mods/My Mod/info.json",
                    br#"{"name":"My Mod","version":"1.0"}"#,
                ),
                ("Wrapper/mods/My Mod/archives/content.archive", b"content"),
            ],
        );
        let package = fixture.install(&archive);
        assert_eq!(package.files.len(), 2);
        assert_eq!(
            fs::read(fixture.game.join("mods/My Mod/archives/content.archive")).unwrap(),
            b"content"
        );
        let report = crate::inventory::scan_with_ownership_for(
            roots,
            &fixture.store,
            "cyberpunk-2077",
            &fixture.game,
        )
        .unwrap();
        assert_eq!(report.owned_files.len(), 2);
        assert!(report
            .owned_files
            .iter()
            .all(|item| item.package_id == package.id
                && item.status == crate::scan::IntegrityStatus::Intact));
        assert_eq!(report.mods.len(), 1);
        assert!(!report.mods[0].managed);
        let doctor = crate::inventory::doctor_with_ownership_for(
            roots,
            &fixture.store,
            "cyberpunk-2077",
            &fixture.game,
        )
        .unwrap();
        assert_eq!(doctor.scan.owned_files.len(), 2);
        fixture.store.uninstall(&fixture.game, &package.id).unwrap();
        assert!(!fixture.game.join("mods/My Mod").exists());
        assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
    });
}

#[test]
fn redmod_invalid_mixed_ambiguous_and_occupied_layouts_refuse_without_writes() {
    crate::test_support::seeded_roots(|roots| {
        let mut fixture = Fixture::new(roots);
        let cases: Vec<Vec<(&str, &[u8])>> = vec![
            vec![("mods/Example/archives/a.archive", b"a")],
            vec![("mods/Example/info.json", b"")],
            vec![("Mods/Example/info.json", b"{}")],
            vec![("mods/Example/INFO.JSON", b"{}")],
            vec![
                ("mods/Example/info.json", b"{}"),
                ("archive/pc/mod/a.archive", b"a"),
            ],
            vec![
                ("mods/Example/info.json", b"{}"),
                ("mods/Other/info.json", b"{}"),
            ],
            vec![
                ("mods/Example/info.json", b"{}"),
                ("mods/Example/unknown.bin", b"a"),
            ],
        ];
        for entries in cases {
            let archive = fixture.archive("invalid.zip", &entries);
            let plan =
                preview_install_zip(roots, "cyberpunk-2077", &fixture.game, &archive).unwrap();
            assert_eq!(plan.status, PlanStatus::Blocked, "{entries:?}");
            assert!(fixture
                .store
                .install_operations_zip(&fixture.game, &archive, &plan)
                .is_err());
            assert!(!fixture.game.join("mods").exists());
            assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
        }
        fixture.write("mods/Example/unmanaged.bin", b"keep");
        let archive = fixture.archive("occupied.zip", &[("mods/Example/info.json", b"{}")]);
        let plan = preview_install_zip(roots, "cyberpunk-2077", &fixture.game, &archive).unwrap();
        assert_eq!(plan.status, PlanStatus::Blocked);
        assert!(fixture
            .store
            .install_operations_zip(&fixture.game, &archive, &plan)
            .is_err());
        assert_eq!(
            fs::read(fixture.game.join("mods/Example/unmanaged.bin")).unwrap(),
            b"keep"
        );
        eprintln!("REDmod refusal: 8 layouts checked");
    });
}

#[test]
fn redmod_invalid_json_never_reaches_the_game_or_ownership() {
    crate::test_support::seeded_roots(|roots| {
        for bytes in [b"not JSON".as_slice(), b"[]", b"null", b"{\"broken\":}"] {
            let mut fixture = Fixture::new(roots);
            let archive = fixture.archive("invalid-json.zip", &[("mods/Example/info.json", bytes)]);
            let plan =
                preview_install_zip(roots, "cyberpunk-2077", &fixture.game, &archive).unwrap();
            assert_eq!(plan.status, PlanStatus::Reviewable);
            assert!(fixture
                .store
                .install_operations_zip(&fixture.game, &archive, &plan)
                .is_err());
            assert!(!fixture.game.join("mods").exists());
            assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
            assert!(fixture.store.pending().unwrap().is_empty());
            fixture.write("mods/Example/info.json", bytes);
            assert!(fixture
                .adopt(&archive, AdoptionApproval::Acknowledged)
                .is_err());
            assert_eq!(
                fs::read(fixture.game.join("mods/Example/info.json")).unwrap(),
                bytes
            );
        }
        eprintln!("REDmod JSON: 4 invalid contents rejected by install and adoption");
    });
}
