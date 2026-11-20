"""Failure refusals for rehash_main.py: invalid records, tampering, hosts.

Import rehash_test_support for the disposable fixtures. Run the suite with
`python .github/scripts/test_rehash_main.py`, or this file alone with
`python .github/scripts/test_rehash_main_safety_failures.py`.
"""

from rehash_test_support import *  # noqa: F401,F403


class FailureTests(RehashTestCase):
    @NEEDS_MODN
    def test_invalid_record_aborts_with_nothing_written(self) -> None:
        games = self.fixture.bucket / "games" / GAME
        (games / "aaa-invalid.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "id": "aaa-invalid",
                    "game": GAME,
                    "name": "Invalid Record",
                    "version": "1.0",
                    "archive_format": "zip",
                    "source": {"kind": "manual", "filename": "aaa-invalid.zip"},
                }
            ),
            encoding="utf-8",
        )
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code,
            stderr,
            [f"{GAME}/aaa-invalid", "sha256 must be 64 lowercase hexadecimal characters"],
            before,
        )

    @NEEDS_MODN
    def test_tampered_archive_aborts_with_expected_and_actual_sha256(self) -> None:
        tampered = self.fixture.archives / "Manual Mod 1.0.zip"
        with tampered.open("ab") as handle:
            handle.write(b"tampered")
        actual = sha256(tampered)
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code,
            stderr,
            [
                f"{GAME}/manual-mod",
                f"expected {self.fixture.expected['manual-mod']['sha256']}",
                f"actual {actual}",
                "wrote nothing",
            ],
            before,
        )

    @NEEDS_MODN
    def test_missing_manual_archive_aborts(self) -> None:
        (self.fixture.archives / "Manual Mod 1.0.zip").unlink()
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code,
            stderr,
            [f"{GAME}/manual-mod", "missing archive Manual Mod 1.0.zip"],
            before,
        )

    @NEEDS_MODN
    def test_failing_download_aborts_and_names_the_package(self) -> None:
        self.fail_urls.add("https://mods.example.org/https-mod.zip")
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code,
            stderr,
            [f"{GAME}/https-mod", "download failed", "connection refused"],
            before,
        )

    def test_missing_modn_binary_aborts_before_any_download(self) -> None:
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(
            self.fixture.bucket,
            "--out",
            self.out,
            "--archive-dir",
            self.fixture.archives,
            "--modn",
            self.root / "missing-modn",
        )
        self.assert_clean_failure(code, stderr, ["modn binary not found"], before)
        self.assertEqual(self.download_calls, [])

    @NEEDS_MODN
    def test_http_source_is_refused(self) -> None:
        games = self.fixture.bucket / "games" / GAME
        (games / "bad-http.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "id": "bad-http",
                    "game": GAME,
                    "name": "Bad HTTP",
                    "version": "1.0",
                    "archive_format": "zip",
                    "sha256": "0" * 64,
                    "source": {"kind": "https", "url": "http://example.org/bad-http.zip"},
                }
            ),
            encoding="utf-8",
        )
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code, stderr, [f"{GAME}/bad-http", "refused source", "http is refused"], before
        )

    @NEEDS_MODN
    def test_modn_org_host_is_refused(self) -> None:
        games = self.fixture.bucket / "games" / GAME
        (games / "bad-host.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "id": "bad-host",
                    "game": GAME,
                    "name": "Bad Host",
                    "version": "1.0",
                    "archive_format": "zip",
                    "sha256": "0" * 64,
                    "source": {
                        "kind": "https",
                        "url": "https://modn-org.example.org/bad-host.zip",
                    },
                }
            ),
            encoding="utf-8",
        )
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code,
            stderr,
            [f"{GAME}/bad-host", "refused source", "modn-org hosts no package copies"],
            before,
        )

    @NEEDS_MODN
    def test_non_empty_out_is_refused_before_any_download(self) -> None:
        self.out.mkdir()
        marker = self.out / "keep.txt"
        marker.write_text("keep", encoding="utf-8")
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assertNotEqual(code, 0)
        self.assertIn("must be missing or empty", stderr)
        self.assertEqual(self.download_calls, [])
        self.assertEqual(marker.read_text(encoding="utf-8"), "keep")
        self.assertEqual(before, snapshot(self.fixture.bucket))
        self.assert_no_staging()


if __name__ == "__main__":
    unittest.main()
