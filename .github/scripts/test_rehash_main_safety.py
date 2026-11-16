"""Safety tests for rehash_main.py: containment, staging, types, refusals.

Import rehash_test_support for the disposable fixtures. Run the suite with
`python .github/scripts/test_rehash_main.py`.
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


class CheckCleanupTests(RehashTestCase):
    """`--check` downloads into a temporary directory and must remove it."""

    def check_staging(self) -> set[Path]:
        return set(Path(tempfile.gettempdir()).glob("modn-rehash-check-*"))

    def test_check_removes_its_staging_archives_on_success(self) -> None:
        before = self.check_staging()
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--archive-dir", self.fixture.archives, "--check"
        )
        self.assertEqual(code, 0, stderr)
        self.assertEqual(
            before,
            self.check_staging(),
            "a successful --check left its downloaded archives behind",
        )

    def test_check_removes_its_staging_archives_on_failure(self) -> None:
        tampered = self.fixture.archives / "Manual Mod 1.0.zip"
        with tampered.open("ab") as handle:
            handle.write(b"tampered")
        before = self.check_staging()
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--archive-dir", self.fixture.archives, "--check"
        )
        self.assertNotEqual(code, 0, stderr)
        self.assertEqual(
            before,
            self.check_staging(),
            "a failed --check left its downloaded archives behind",
        )


class ModnResolutionTests(RehashTestCase):
    def test_resolve_modn_returns_an_absolute_path(self) -> None:
        binary = self.root / "fake-modn"
        binary.write_text("", encoding="utf-8")
        previous = Path.cwd()
        os.chdir(self.root)
        try:
            resolved = rehash_main.resolve_modn("fake-modn")
        finally:
            os.chdir(previous)
        self.assertTrue(resolved.is_absolute(), f"relative path returned: {resolved}")
        self.assertEqual(resolved, binary.resolve())


class NexusSourceIdTests(unittest.TestCase):
    def test_nexus_ids_outside_the_u64_range_are_refused(self) -> None:
        for value in (-1, 2**64):
            with self.subTest(value=value):
                with self.assertRaises(rehash_bucket.RehashError):
                    rehash_bucket.validate_source(
                        {
                            "kind": "nexus",
                            "game_domain": "fixture-game",
                            "mod_id": value,
                            "file_id": 1,
                            "file_name": "archive.zip",
                        }
                    )


class OutputContainmentTests(RehashTestCase):
    """`--out` and archive names must resolve before they are compared."""

    def fake_modn(self) -> Path:
        path = self.root / "fake-modn"
        path.write_text("", encoding="utf-8")
        return path

    def test_out_inside_a_relative_bucket_alias_is_refused(self) -> None:
        previous = Path.cwd()
        os.chdir(self.root)
        try:
            code, _, stderr = self.run_main(
                "main-v1",
                "--out",
                self.fixture.bucket / "v2",
                "--modn",
                self.fake_modn(),
            )
        finally:
            os.chdir(previous)
        self.assertNotEqual(code, 0)
        self.assertIn("--out must not be inside the v1 bucket directory", stderr)
        self.assertFalse((self.fixture.bucket / "v2").exists())
        self.assertEqual(self.download_calls, [])

    def test_out_that_contains_the_bucket_is_refused(self) -> None:
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--out", self.root, "--modn", self.fake_modn()
        )
        self.assertNotEqual(code, 0)
        self.assertIn("--out must not contain the v1 bucket directory", stderr)
        self.assertEqual(before, snapshot(self.fixture.bucket))
        self.assertEqual(self.download_calls, [])

    def test_out_equal_to_the_archive_directory_is_refused(self) -> None:
        code, _, stderr = self.run_main(
            self.fixture.bucket,
            "--out",
            self.fixture.archives,
            "--archive-dir",
            self.fixture.archives,
            "--modn",
            self.fake_modn(),
        )
        self.assertNotEqual(code, 0)
        self.assertIn("--out must not be inside the archive directory", stderr)
        self.assertEqual(self.download_calls, [])

    @NEEDS_MODN
    def test_manual_archive_symlink_out_of_the_archive_dir_is_refused(self) -> None:
        outside = self.root / "outside.zip"
        make_archive(outside, "outside archive")
        link = self.fixture.archives / "linked.zip"
        try:
            os.symlink(outside, link)
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"cannot create a symlink: {error}")
        self.fixture.record(
            "linked",
            {
                "schema_version": 1,
                "id": "linked",
                "game": GAME,
                "name": "Linked Mod",
                "version": "1.0",
                "archive_format": "zip",
                "sha256": sha256(outside),
                "source": {"kind": "manual", "filename": "linked.zip"},
            },
        )
        code, _, stderr = self.run_main(
            self.fixture.bucket,
            "--out",
            self.out,
            "--archive-dir",
            self.fixture.archives,
            "--modn",
            MODN,
        )
        self.assertNotEqual(code, 0)
        self.assertIn("archive file name escapes --archive-dir", stderr)
        self.assertFalse(self.out.exists())

    def test_manual_archive_parent_escape_is_refused(self) -> None:
        games = self.fixture.bucket / "games" / GAME
        (games / "escape.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "id": "escape",
                    "game": GAME,
                    "name": "Escape Mod",
                    "version": "1.0",
                    "archive_format": "zip",
                    "sha256": "0" * 64,
                    "source": {"kind": "manual", "filename": "../escape.zip"},
                }
            ),
            encoding="utf-8",
        )
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--out", self.out, "--archive-dir", self.fixture.archives,
            "--modn", self.fake_modn(),
        )
        self.assertNotEqual(code, 0)
        self.assertIn("invalid manual archive file name", stderr)

    def test_manual_archive_absolute_path_is_refused(self) -> None:
        games = self.fixture.bucket / "games" / GAME
        absolute = str(Path(self.root, "escape.zip").resolve())
        (games / "escape.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "id": "escape",
                    "game": GAME,
                    "name": "Escape Mod",
                    "version": "1.0",
                    "archive_format": "zip",
                    "sha256": "0" * 64,
                    "source": {"kind": "manual", "filename": absolute},
                }
            ),
            encoding="utf-8",
        )
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--out", self.out, "--archive-dir", self.fixture.archives,
            "--modn", self.fake_modn(),
        )
        self.assertNotEqual(code, 0)
        self.assertIn("invalid manual archive file name", stderr)


class FieldTypeTests(RehashTestCase):
    """Converted fields must hold the type that the v2 manifest requires."""

    @NEEDS_MODN
    def test_wrong_field_types_name_the_package_file_and_field(self) -> None:
        cases = [
            ("type-id", {"id": 7}, "field id must be a string"),
            ("type-game", {"game": 7}, "field game must be a string"),
            ("type-name", {"name": 7}, "field name must be a string"),
            ("type-version", {"version": 1}, "field version must be a string"),
            ("type-format", {"archive_format": 1}, "field archive_format must be a string"),
            ("type-sha", {"sha256": 7}, "field sha256 must be a string"),
            (
                "type-dependencies",
                {"dependencies": ["https-mod", 1]},
                "field dependencies must be a list of strings",
            ),
            (
                "type-injector",
                {"native_injector": "false"},
                "field native_injector must be a boolean",
            ),
            ("type-source", {"source": "https://example.org/type.zip"}, "field source must be an object"),
            (
                "type-executables",
                {"executables": [42]},
                "field executables must be a list of strings",
            ),
            (
                "type-executables-not-list",
                {"executables": "tool.exe"},
                "field executables must be a list of strings",
            ),
        ]
        records = self.fixture.bucket / "games" / GAME
        for name, override, expected in cases:
            with self.subTest(field=name):
                value = {
                    "schema_version": 1,
                    "id": name,
                    "game": GAME,
                    "name": "Typed Mod",
                    "version": "1.0",
                    "archive_format": "zip",
                    "sha256": self.fixture.expected["https-mod"]["sha256"],
                    "source": {"kind": "manual", "filename": "Manual Mod 1.0.zip"},
                }
                value.update(override)
                path = records / f"{name}.json"
                path.write_text(json.dumps(value), encoding="utf-8")
                before = snapshot(self.fixture.bucket)
                code, _, stderr = self.run_main(
                    *self.convert_arguments(),
                )
                try:
                    self.assert_clean_failure(
                        code,
                        stderr,
                        [f"{GAME}/{name}", f"{name}.json", expected],
                        before,
                    )
                finally:
                    path.unlink()

if __name__ == "__main__":
    unittest.main()
