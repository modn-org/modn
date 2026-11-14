use super::Fixture;
use crate::transactions::AdoptionApproval;
use std::fs;

#[test]
fn manual_redmod_adoption_claims_files_without_unknown_descendants() {
    crate::test_support::seeded(|| {
        let mut fixture = Fixture::new();
        let entries: &[(&str, &[u8])] = &[
            (
                "mods/Manual/info.json",
                br#"{"name":"Manual","version":"1"}"#,
            ),
            ("mods/Manual/archives/a.archive", b"manual"),
        ];
        let archive = fixture.archive("manual-redmod.zip", entries);
        for (relative, bytes) in entries {
            fixture.write(relative, bytes);
        }
        fixture.write("mods/Manual/unknown.txt", b"preserve");
        let package = fixture
            .adopt(&archive, AdoptionApproval::Acknowledged)
            .unwrap();
        assert_eq!(package.files.len(), 2);
        assert!(package.directories.is_empty());
        assert_eq!(
            package.archive_blake3,
            crate::archive::inspect(&archive).unwrap().blake3
        );
        for file in &package.files {
            let bytes = fs::read(fixture.game.join(&file.destination)).unwrap();
            assert_eq!(
                file.blake3,
                blake3::hash(&bytes).to_hex().to_string(),
                "adoption must record the BLAKE3 digest of each owned file"
            );
        }
        fixture.store.uninstall(&fixture.game, &package.id).unwrap();
        assert_eq!(
            fs::read(fixture.game.join("mods/Manual/unknown.txt")).unwrap(),
            b"preserve"
        );
    });
}

#[test]
fn adoption_refuses_size_mismatch_and_files_owned_by_another_package() {
    crate::test_support::seeded(|| {
        let mut fixture = Fixture::new();
        let path = "archive/pc/mod/a.archive";
        let archive = fixture.archive("owned.zip", &[(path, b"one")]);
        let package = fixture.install(&archive);
        let incoming = fixture.archive(
            "incoming.zip",
            &[(path, b"one"), ("archive/pc/mod/b.archive", b"two")],
        );
        fixture.write("archive/pc/mod/b.archive", b"two");
        assert!(fixture
            .adopt(&incoming, AdoptionApproval::Acknowledged)
            .is_err());
        assert_eq!(fixture.store.installed(&fixture.game).unwrap().len(), 1);
        fixture.store.uninstall(&fixture.game, &package.id).unwrap();
        fixture.write(path, b"too long");
        assert!(fixture
            .adopt(&incoming, AdoptionApproval::Acknowledged)
            .is_err());
        assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
        assert_eq!(fs::read(fixture.game.join(path)).unwrap(), b"too long");
        assert_eq!(
            fs::read(fixture.game.join("archive/pc/mod/b.archive")).unwrap(),
            b"two"
        );
    });
}
